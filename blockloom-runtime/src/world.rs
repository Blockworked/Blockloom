//! Everything about the running world that doesn't depend on which dimension
//! it is: rebuilding it from a project, publishing what the sensing blocks
//! read, stepping the VM, and applying the effects that are pure transform or
//! visibility changes.
//!
//! Every system here touches the `!Send` [`Engine`], which is what pins them
//! all to the main thread - the same thread the VM's thread-local sensor
//! snapshot lives on.
//!
//! Simulation runs on `FixedUpdate`, Bevy's constant-rate step that catches up
//! whatever the display does, so blocks and physics advance in step with each
//! other at the project's own rate. Input, sensing and rendering stay on the
//! per-frame `Update`: events fire at most once there, so a slow machine that
//! sinks several fixed steps into one frame doesn't triple a keypress.

use crate::engine::{
    ActorId, CameraRig, CustomComponents, Dimension, Engine, Gliding, PendingEffects, PhysicsPose,
    PrevPose,
};
use crate::{bridge, dim2, dim3};
use bevy::input::ButtonState;
use bevy::input::gamepad::{
    Gamepad, GamepadAxis, GamepadButton, GamepadRumbleIntensity, GamepadRumbleRequest,
};
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::{MouseMotion, MouseScrollUnit, MouseWheel};
use bevy::input::touch::Touches;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow, WindowFocused};
use blockloom_core::components::CameraView;
use blockloom_core::input::{ActionSense, LiveInput, normalize_pad_axis, normalize_pad_button};
use blockloom_core::nav;
use blockloom_core::project::Actor;
use blockloom_core::scene::{Axis, BodyKind, Mode, Visual};
use blockloom_core::sense::{ActorSense, Sensors, TouchSense, normalize_key};
use blockloom_core::ui::UiKind;
use blockloom_core::value::Evaluated;
use blockloom_core::vm::{Effect, Event};
use blockloom_protocol::{
    ActorStatus, EditorMessage, RenderMetric, RuntimeMessage, Status, VariableValue,
};
use std::collections::{HashMap, HashSet};

/// The one camera the project controls.
#[derive(Component)]
pub struct WorldCamera;

/// The project's sun, which a rebuild replaces rather than adds to.
#[derive(Component)]
pub struct WorldLight;

/// The navmesh the `navigate to` block walks. It is rebaked when static
/// geometry changes. `None` falls back to a straight step at the target.
#[derive(Resource, Default)]
pub struct NavMesh {
    pub mesh: Option<polyanya::Mesh>,
    pub mode: Mode,
    pub settings: nav::NavSettings,
    signature: Vec<(String, [f32; 3], Visual)>,
}

/// Re-bake when a running static collider changes. Dynamic actors stay out of
/// the mesh, so moving crowds do not trigger expensive bakes.
pub fn sync_navmesh(
    engine: NonSend<Engine>,
    mut navmesh: ResMut<NavMesh>,
    transforms: Query<&Transform, With<ActorId>>,
) {
    if !engine.running || engine.paused {
        return;
    }
    let mut signature = Vec::new();
    let mut actors = Vec::new();
    for (id, entity) in &engine.entities {
        if !engine.has_component(id, "Body") || !engine.has_component(id, "Look") {
            continue;
        }
        let Some(actor) = engine.actor(id) else {
            continue;
        };
        if actor.physics().body != BodyKind::Static {
            continue;
        }
        let Ok(transform) = transforms.get(*entity) else {
            continue;
        };
        let Some(visual) = actor.visual().cloned() else {
            continue;
        };
        let position = transform.translation.to_array();
        signature.push((id.clone(), position, visual));
        let mut live = actor.clone();
        live.components.placement_mut().position = position;
        actors.push(live);
    }
    signature.sort_by(|a, b| a.0.cmp(&b.0));
    if signature == navmesh.signature {
        return;
    }
    navmesh.signature = signature;
    let mut project = engine.project.clone();
    project.actors = actors;
    navmesh.mesh = nav::build_mesh(&project, nav::DEFAULT_AGENT_RADIUS)
        .map_err(|error| tracing::warn!("navmesh rebake failed: {error}"))
        .ok();
    navmesh.mode = project.world.mode;
    navmesh.settings = project.world.navigation.clone();
}

/// The systems that advance the simulation itself, kept apart from the input
/// and rendering systems in `Update` so the runtime can order them before the
/// physics pipeline's own fixed systems.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct SimulationSet;

/// `#RRGGBB` as the renderer wants it. An unparseable color reads as magenta,
/// which is easier to notice than a silent black.
pub fn parse_color(hex: &str) -> Color {
    match Srgba::hex(hex) {
        Ok(color) => color.into(),
        Err(_) => Color::srgb(1.0, 0.0, 1.0),
    }
}

/// The project's tonemapper as the camera component. TonyMcMapface is
/// Bevy's own default, so spelling it out changes nothing for old projects.
pub fn tonemapping_of(
    name: blockloom_core::scene::TonemapName,
) -> bevy::core_pipeline::tonemapping::Tonemapping {
    use bevy::core_pipeline::tonemapping::Tonemapping;
    use blockloom_core::scene::TonemapName;
    match name {
        TonemapName::None => Tonemapping::None,
        TonemapName::Reinhard => Tonemapping::Reinhard,
        TonemapName::ReinhardLuminance => Tonemapping::ReinhardLuminance,
        TonemapName::AcesFitted => Tonemapping::AcesFitted,
        TonemapName::TonyMcMapface => Tonemapping::TonyMcMapface,
        TonemapName::Filmic => Tonemapping::BlenderFilmic,
    }
}

/// Bloom from the project's post settings: threshold and intensity are the
/// two dials a game usefully turns.
pub fn bloom_of(post: &blockloom_core::scene::PostProcess) -> bevy::post_process::bloom::Bloom {
    bevy::post_process::bloom::Bloom {
        intensity: post.bloom_intensity,
        prefilter: bevy::post_process::bloom::BloomPrefilter {
            threshold: post.bloom_threshold,
            ..default()
        },
        ..default()
    }
}

/// Vignette from a single strength dial. Radius and softness stay at
/// Bevy's defaults; games tune how dark the corners get.
pub fn vignette_of(strength: f32) -> bevy::post_process::effect_stack::Vignette {
    bevy::post_process::effect_stack::Vignette {
        intensity: strength.clamp(0.0, 1.0),
        ..default()
    }
}

/// Where an asset the project names - an image, a font - actually sits. Bevy's
/// asset root is this process's own folder, not the project's, so a path the
/// editor stores (`assets/player.png`) is resolved against the project folder
/// and handed to the asset server whole.
pub fn asset_path(dir: Option<&std::path::Path>, relative: &str) -> std::path::PathBuf {
    dir.and_then(|dir| blockloom_core::assets::resolve(dir, relative))
        .unwrap_or_else(|| std::path::PathBuf::from(relative))
}

pub fn transform_for(actor: &Actor) -> Transform {
    let placement = actor.placement();
    let [x, y, z] = placement.position;
    let [rx, ry, rz] = placement.rotation;
    Transform {
        translation: Vec3::new(x, y, z),
        rotation: Quat::from_euler(
            EulerRot::XYZ,
            rx.to_radians(),
            ry.to_radians(),
            rz.to_radians(),
        ),
        scale: Vec3::from(placement.scale3()),
    }
}

pub fn visibility_for(actor: &Actor) -> Visibility {
    if actor.visible() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    }
}

/// The actor's custom components, as the entity carries them.
pub fn custom_for(actor: &Actor) -> CustomComponents {
    CustomComponents(
        actor
            .components
            .custom()
            .map(|(name, fields)| {
                let values = fields
                    .iter()
                    .map(|field| (field.name.clone(), field.value.clone()))
                    .collect();
                (name.to_string(), values)
            })
            .collect(),
    )
}

/// What every actor entity gets whatever it looks like: who it is, where it
/// stands, whether it's drawn, and its custom components. The dimension's own
/// spawner adds the sprite or mesh on top.
///
/// The pose pair every actor carries is what lets the renderer draw it between
/// fixed steps - for physics bodies the step overwrites it again (`record_poses`
/// after rapier's writeback), and for actors moved straight by step effects it
/// is captured the same way.
pub fn actor_bundle(actor: &Actor) -> impl Bundle {
    let transform = transform_for(actor);
    (
        Name::new(actor.name.clone()),
        ActorId(actor.id.clone()),
        transform,
        visibility_for(actor),
        custom_for(actor),
        PhysicsPose(transform),
        PrevPose(transform),
    )
}

/// Records (or clears) a contact between two entities, and starts any
/// `when I touch` strand it satisfies - in both directions, since either
/// actor may be the one listening.
pub fn note_contact(engine: &mut Engine, a: Entity, b: Entity, started: bool) {
    let Some(first) = engine.actor_id_of(a).map(str::to_string) else {
        return;
    };
    let Some(second) = engine.actor_id_of(b).map(str::to_string) else {
        return;
    };
    for (actor, other) in [(&first, &second), (&second, &first)] {
        let contacts = engine.touching.entry(actor.clone()).or_default();
        if started {
            contacts.insert(other.clone());
        } else {
            contacts.remove(other);
        }
    }
    if started {
        engine.fire(Event::Collision {
            actor: first.clone(),
            with: second.clone(),
        });
        engine.fire(Event::Collision {
            actor: second,
            with: first,
        });
    }
}

// ─── Editor messages ───────────────────────────────────────────────────────

pub fn pump_editor(
    mut engine: NonSendMut<Engine>,
    mut preview: Option<ResMut<crate::preview::PreviewState>>,
    mut manager: ResMut<crate::ui::UiManager>,
    mut scene: Option<ResMut<crate::edit::SceneEditor>>,
    time: Res<Time>,
    mut fixed: ResMut<Time<Fixed>>,
    mut exit: MessageWriter<AppExit>,
) {
    // The project names its own fixed rate. Step-sized here, once a frame, so
    // the next FixedUpdate runs at whatever the loaded project asked for.
    let rate = engine.project.world.fixed_rate;
    if rate.is_finite() {
        fixed.set_timestep_hz(rate.clamp(1.0, 1000.0) as f64);
    }
    let now = time.elapsed_secs() as f64;
    loop {
        let message = match engine.incoming.try_recv() {
            Ok(message) => message,
            Err(std::sync::mpsc::TryRecvError::Empty) => break,
            // The editor closed the pipe: nothing left to render for.
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                exit.write(AppExit::Success);
                return;
            }
        };
        match message {
            EditorMessage::Load { project, dir } => {
                engine.project = *project;
                engine.project_dir = dir.map(std::path::PathBuf::from);
                let loaded = engine.project.clone();
                engine.vm.load(&loaded);
                load_saved_data(&mut engine);
                open_logic(&mut engine);
                engine.speech.clear();
                // The interface goes with the world it belonged to. Cleared
                // here rather than in `rebuild_world`, which runs after the
                // fixed step that builds this run's interface.
                manager.clear();
                engine.running = false;
                engine.paused = false;
                engine.pause_began = None;
                engine.rebuild = true;
                if let Some(scene) = scene.as_mut() {
                    scene.loaded = true;
                }
            }
            EditorMessage::Start => {
                let project = engine.project.clone();
                engine.vm.load(&project);
                load_saved_data(&mut engine);
                if let Some(logic) = &mut engine.logic {
                    logic.reset();
                }
                engine.touching.clear();
                engine.speech.clear();
                manager.clear();
                engine.rebuild = true;
                engine.running = true;
                engine.paused = false;
                engine.pause_began = None;
                engine.started_at = now;
                engine.fire(Event::Started);
            }
            EditorMessage::Stop => {
                engine.stop_program();
                engine.speech.clear();
                manager.clear();
                engine.running = false;
                engine.paused = false;
                engine.pause_began = None;
                engine.rebuild = true;
                bridge::send(&RuntimeMessage::Stopped);
            }
            EditorMessage::Pause { paused } => set_paused(&mut engine, paused, now),
            EditorMessage::Step => {
                // One fixed tick while paused, then re-pause at the end of
                // the fixed chain (`finish_step`). Ignored unless paused:
                // a running world is already stepping.
                if engine.running && engine.paused && !engine.pause_after_tick {
                    engine.paused = false;
                    engine.pause_began.take();
                    engine.vm.set_paused(false);
                    if let Some(logic) = &mut engine.logic {
                        logic.set_paused(false);
                    }
                    engine.pause_after_tick = true;
                }
            }
            EditorMessage::Preview { enabled, headless } => {
                let Some(preview) = preview.as_mut() else {
                    continue;
                };
                if enabled {
                    // Recorded before serving so a re-sent toggle applies
                    // while the sidecar stays up; visibility follows next
                    // frame in `apply_preview_visibility`.
                    preview.headless = headless;
                    match crate::preview::start_preview(preview) {
                        Some(port) => bridge::send(&RuntimeMessage::PreviewReady { port }),
                        None => bridge::send(&RuntimeMessage::PreviewStopped),
                    }
                } else {
                    crate::preview::stop_preview(preview);
                    bridge::send(&RuntimeMessage::PreviewStopped);
                }
            }
            EditorMessage::PreviewInput { input } => {
                engine.preview_inputs.push(input);
            }
            EditorMessage::SceneView(view) => {
                let Some(scene) = scene.as_mut() else {
                    continue;
                };
                // Turning the scene view off shows the game's own camera,
                // which only a rebuild puts back where the project says.
                if scene.view.enabled && !view.enabled && !engine.running {
                    engine.rebuild = true;
                }
                scene.view = view;
            }
            EditorMessage::Select { actor } => {
                if let Some(scene) = scene.as_mut() {
                    scene.selected = actor;
                }
            }
            EditorMessage::FrameSelected => {
                if let Some(scene) = scene.as_mut() {
                    scene.frame = true;
                }
            }
            EditorMessage::Shutdown => {
                exit.write(AppExit::Success);
                return;
            }
        }
    }
}

fn load_saved_data(engine: &mut Engine) {
    engine.save_path = blockloom_core::save::path(&engine.project.id);
    match blockloom_core::save::read(&engine.save_path) {
        Ok(data) => {
            data.apply(&engine.project, &engine.variables);
            engine.save_data = data;
        }
        Err(message) => {
            engine.save_data = Default::default();
            bridge::send(&RuntimeMessage::Error {
                actor: String::new(),
                message: format!("couldn't load saved variables: {message}"),
            });
        }
    }
}

/// Persists the variable slots named by this step's save-data effects. This
/// runs while paused too, since a settings menu is the common caller.
pub fn apply_saved_data(effects: Res<PendingEffects>, mut engine: NonSendMut<Engine>) {
    if !engine.running {
        return;
    }
    for effect in &effects.0 {
        let Effect::SaveVariable { actor, name, clear } = effect else {
            continue;
        };
        let owner = engine
            .clones
            .get(actor)
            .cloned()
            .unwrap_or_else(|| actor.clone());
        let project = engine.project.clone();
        let snapshot = engine.variables.snapshot();
        let changed = if *clear {
            engine.save_data.clear(&project, &owner, name)
        } else {
            engine
                .save_data
                .capture(&project, &snapshot, &owner, actor, name)
        };
        if !changed && !clear {
            bridge::send(&RuntimeMessage::Error {
                actor: actor.clone(),
                message: format!("there's no variable called \"{name}\" to save"),
            });
            continue;
        }
        if changed
            && let Err(message) = blockloom_core::save::write(&engine.save_path, &engine.save_data)
        {
            bridge::send(&RuntimeMessage::Error {
                actor: actor.clone(),
                message: format!("couldn't save variable \"{name}\": {message}"),
            });
        }
    }
}

/// Freezes or thaws the world. The editor's Pause button and the `pause
/// game` block both land here, so the timer is shifted forward by the paused
/// span exactly once however the pause was asked for.
pub fn set_paused(engine: &mut Engine, paused: bool, now: f64) {
    if paused == engine.paused {
        return;
    }
    engine.paused = paused;
    if paused {
        engine.pause_began = Some(now);
    } else if let Some(began) = engine.pause_began.take() {
        // Shift the start forward by the paused span so the timer resumes
        // where it froze instead of jumping.
        engine.started_at += now - began;
    }
    // The schedulers keep their own copy: a paused world still gives a slice
    // to strands the interface started, so a pause menu's buttons work.
    engine.vm.set_paused(paused);
    if let Some(logic) = &mut engine.logic {
        logic.set_paused(paused);
    }
}

/// Re-pauses after a stepped tick (`EditorMessage::Step`). Runs last in the
/// fixed chain, so the one tick simulated fully before freezing again.
pub fn finish_step(mut engine: NonSendMut<Engine>, time: Res<Time>) {
    if !engine.pause_after_tick {
        return;
    }
    engine.pause_after_tick = false;
    if engine.running && !engine.paused {
        set_paused(&mut engine, true, time.elapsed_secs() as f64);
    }
}

// ─── Building the world ────────────────────────────────────────────────────
/// Despawns everything and spawns it again from the project. Called on load,
/// on every Start (so actors go back where they were authored), and on stop.
pub fn rebuild_world(
    mut commands: Commands,
    mut engine: NonSendMut<Engine>,
    dimension: Res<Dimension>,
    mut effects: ResMut<PendingEffects>,
    mut clear_color: ResMut<ClearColor>,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut textures: ResMut<Assets<Image>>,
    actors: Query<Entity, With<ActorId>>,
    scenery: Query<Entity, Or<(With<WorldCamera>, With<WorldLight>)>>,
    voices: Query<Entity, With<crate::sound::VoiceTag>>,
    mut sound: ResMut<crate::sound::SoundState>,
    mut navmesh: Option<ResMut<NavMesh>>,
    mut stores: crate::materials::MaterialStores,
    mut performance: crate::performance::PerformanceStores,
) {
    if !engine.rebuild {
        return;
    }
    engine.rebuild = false;
    performance.cache.clear();
    performance.cells.clear();

    for entity in &actors {
        commands.entity(entity).despawn();
    }
    for entity in &scenery {
        commands.entity(entity).despawn();
    }
    // Voices are neither actors nor cameras, so the passes above miss them:
    // a rebuild starts from silence at the saved mix.
    for entity in &voices {
        commands.entity(entity).despawn();
    }
    // Particles and ghosts go in `fx::despawn_fx`, just before this system.
    sound.reset(project_sound(&engine));
    engine.entities.clear();
    engine.touching.clear();
    // Remaps last exactly as long as the run, like everything else live.
    engine.reset_input_run();
    // Everything the last run made goes with it: Play starts from the
    // document, which is the one thing a clone was never in.
    engine.spawned.clear();
    engine.clones.clear();
    engine.last_created.clear();
    engine.parents = engine
        .project
        .actors
        .iter()
        .filter_map(|actor| Some((actor.id.clone(), actor.parent()?.to_string())))
        .collect();
    // Dropped here rather than left open: a rebuild follows a fresh build of
    // the libraries, and the old ones must be closed before the new ones open.
    engine.scripts.clear();
    engine.scripts_started.clear();
    engine.attached = engine
        .project
        .actors
        .iter()
        .map(|actor| {
            let held = actor
                .components
                .iter()
                .map(|component| component.name().to_string())
                .collect();
            (actor.id.clone(), held)
        })
        .collect();
    // Live collision filters start as the document authored them; the `set
    // trigger` / `set collision` blocks move them from there.
    engine.physics_filter = engine
        .project
        .actors
        .iter()
        .map(|actor| {
            let physics = actor.physics();
            (
                actor.id.clone(),
                (physics.layer(), physics.collision_mask, physics.trigger),
            )
        })
        .collect();

    let mut project = engine.project.clone();
    // A child authored in its parent's frame is put where that works out to,
    // once, before anything is spawned from these placements.
    place_authored_children(&mut project);
    let dir = engine.project_dir.clone();
    clear_color.0 = parse_color(&project.world.background);
    match dimension.0 {
        Mode::TwoD => {
            let post = project.world.post.clone();
            let mut camera_entity = commands.spawn((
                Camera2d,
                Projection::Orthographic(OrthographicProjection {
                    scale: 1.0 / project.world.camera.zoom.max(0.05),
                    ..OrthographicProjection::default_2d()
                }),
                WorldCamera,
                // The one listener positional voices pan against. It rides
                // the camera, so what the player sees is what they hear.
                bevy::audio::SpatialListener::default(),
                bevy::camera::Exposure {
                    ev100: post.exposure_ev,
                },
                tonemapping_of(post.tonemapping),
            ));
            if post.bloom_enabled {
                camera_entity.insert(bloom_of(&post));
            }
            if post.vignette_strength > 0.0 {
                camera_entity.insert(vignette_of(post.vignette_strength));
            }
            for actor in &project.actors {
                let entity = dim2::spawn_actor(
                    &mut commands,
                    actor,
                    dir.as_deref(),
                    &assets,
                    &mut textures,
                    &mut meshes,
                    &mut stores.graph_2d,
                    &mut stores.tiles,
                )
                .unwrap_or_else(|| spawn_unseen(&mut commands, actor, Mode::TwoD));
                attach_camera(&mut commands, actor, entity);
                crate::fx::insert_fx_state(&mut commands, actor, entity);
                engine.entities.insert(actor.id.clone(), entity);
            }
        }
        Mode::ThreeD => {
            dim3::spawn_scenery(
                &mut commands,
                &project.world.camera,
                &project.world.lighting,
                &project.world.post,
            );
            for actor in &project.actors {
                let entity = dim3::spawn_actor(
                    &mut commands,
                    actor,
                    dir.as_deref(),
                    &assets,
                    &mut meshes,
                    &mut materials,
                    &mut stores.graph_3d,
                    &mut performance.cache,
                )
                .unwrap_or_else(|| spawn_unseen(&mut commands, actor, Mode::ThreeD));
                attach_camera(&mut commands, actor, entity);
                crate::fx::insert_fx_state(&mut commands, actor, entity);
                engine.entities.insert(actor.id.clone(), entity);
            }
        }
    }
    // The dimension's own effect system owns the physics pipeline, so gravity
    // is set the same way a `set gravity` block would set it.
    effects.0.push(Effect::SetGravity {
        gravity: project.world.gravity,
    });
    // The navmesh walks what the editor shows: static ground as the boundary,
    // every other static solid as a hole. A bake that finds nothing to stand
    // on leaves `None`, and navigation steps straight at its target.
    if let Some(nav) = navmesh.as_deref_mut() {
        nav.mesh = nav::build_mesh(&project, nav::DEFAULT_AGENT_RADIUS)
            .map_err(|error| tracing::warn!("navmesh bake failed: {error}"))
            .ok();
        nav.mode = project.world.mode;
        nav.settings = project.world.navigation.clone();
    }
    open_scripts(&mut engine, &project);
}

/// The saved mix, for reseeding the runtime's live gains on a rebuild.
fn project_sound(engine: &Engine) -> blockloom_core::sound::SoundMixer {
    engine.project.world.sound
}

/// Opens every actor's compiled script. The editor builds them before Play,
/// so a script with no library yet is simply one that hasn't been played -
/// which happens on every edit and is nothing to report. Anything else here
/// is a library that won't load, which the actor is told about.
fn open_scripts(engine: &mut Engine, project: &blockloom_core::project::Project) {
    let Some(dir) = engine.project_dir.clone() else {
        return;
    };
    for actor in &project.actors {
        let Some(path) = actor.components.script() else {
            continue;
        };
        if !crate::script::LoadedScript::is_built(&dir, path) {
            continue;
        }
        match crate::script::LoadedScript::load(&dir, path) {
            Ok(script) => {
                engine.scripts.insert(actor.id.clone(), script);
            }
            Err(message) => bridge::send(&RuntimeMessage::Error {
                actor: actor.id.clone(),
                message,
            }),
        }
    }
}

/// Opens native block logic only in a shipped player. Editor Play remains the
/// reference VM, including while an older compiled library is still present.
fn open_logic(engine: &mut Engine) {
    engine.logic = None;
    if bridge::attached() {
        return;
    }
    let Some(dir) = engine.project_dir.as_deref() else {
        return;
    };
    if !crate::logic::LoadedLogic::is_built(dir) {
        return;
    }
    match crate::logic::LoadedLogic::load(dir) {
        Ok(logic) => engine.logic = Some(logic),
        Err(message) => bridge::send(&RuntimeMessage::Error {
            actor: String::new(),
            message,
        }),
    }
}

/// Runs every scripted for this fixed step: `start` once per run, then `tick`
/// with the step's delta. Their effects join the VM's in the same list, so a
/// script and a canvas driving one actor are applied together, in order.
pub fn step_scripts(
    mut engine: NonSendMut<Engine>,
    time: Res<Time>,
    mut effects: ResMut<PendingEffects>,
) {
    if !engine.running || engine.paused || engine.scripts.is_empty() {
        return;
    }
    let dt = time.delta_secs();
    let actors: Vec<String> = engine.scripts.keys().cloned().collect();
    // `start` runs once per actor rather than once per run, so a clone made
    // half way through gets its own, on the first step it exists for.
    let fresh: Vec<String> = actors
        .iter()
        .filter(|actor| !engine.scripts_started.contains(*actor))
        .cloned()
        .collect();

    let mut asked = crate::script::Asked::default();
    for actor in &actors {
        let Some(script) = engine.scripts.get(actor) else {
            continue;
        };
        if fresh.contains(actor) {
            script.start(actor, &mut asked);
        }
        script.tick(actor, &mut asked, dt);
    }
    engine.scripts_started.extend(fresh);
    for message in asked.messages.drain(..) {
        engine.fire(Event::Message(message));
    }
    script_lifetimes(&mut engine, &mut asked);
    for effect in &asked.effects {
        // Says and errors go to the editor the same way the VM's do.
        if let Effect::Say { actor, text } = effect {
            engine.note_say(actor, text);
            bridge::send(&RuntimeMessage::Say {
                actor: actor.clone(),
                text: text.clone(),
            });
        }
    }
    effects.0.append(&mut asked.effects);
}

/// The actor a script asked to make or unmake. Clones and fresh actors need
/// a scheduler slot before they need an entity, so they are registered with
/// whichever scheduler this run has and come back out as the same effects.
///
/// The VM mints its own ids; a compiled program mints its own too, so when
/// that is the scheduler the host mints a differently shaped one and hands
/// it over, which is what keeps the two from ever picking the same id.
fn script_lifetimes(engine: &mut Engine, asked: &mut crate::script::Asked) {
    for (actor, wanted) in asked.clones.drain(..) {
        let made = if engine.logic.is_some() {
            clone_for_logic(engine, &actor, &wanted)
        } else {
            engine.vm.clone_actor(&actor, &wanted)
        };
        match made {
            Some((clone, of)) => asked.effects.push(Effect::CreateClone { actor, clone, of }),
            None => bridge::send(&RuntimeMessage::Error {
                actor,
                message: format!("there's no actor named \"{wanted}\" to clone"),
            }),
        }
    }
    for (actor, name, position) in asked.created.drain(..) {
        let id = if engine.logic.is_some() {
            let id = engine.new_actor_id();
            if let Some(logic) = &mut engine.logic {
                logic.created(&id, &name);
            }
            id
        } else {
            engine.vm.create_actor(&name)
        };
        asked.effects.push(Effect::CreateActor {
            actor,
            id,
            name,
            position,
        });
    }
    for (actor, wanted) in asked.deleted.drain(..) {
        let gone = if engine.logic.is_some() {
            let gone = named_or_self(engine, &actor, &wanted);
            if let Some(gone) = gone.as_deref() {
                engine.variables.forget_actor(gone);
                engine.lists.forget_actor(gone);
                engine.dicts.forget_actor(gone);
                if let Some(logic) = &mut engine.logic {
                    logic.deleted(gone);
                }
            }
            gone
        } else {
            engine.vm.delete_actor(&actor, &wanted)
        };
        match gone {
            Some(gone) => asked.effects.push(Effect::DeleteActor { actor: gone }),
            None => bridge::send(&RuntimeMessage::Error {
                actor,
                message: format!("there's no actor named \"{wanted}\" to delete"),
            }),
        }
    }
}

/// Makes a clone on a script's behalf while a compiled program is the
/// scheduler: the host names the copy and the program adopts it, so the
/// copy's `when I start as a clone` strands run like any other clone's.
fn clone_for_logic(engine: &mut Engine, running: &str, wanted: &str) -> Option<(String, String)> {
    let of = named_or_self(engine, running, wanted)?;
    let clone = engine.new_actor_id();
    engine.variables.copy_actor(&of, &clone);
    engine.lists.copy_actor(&of, &clone);
    engine.dicts.copy_actor(&of, &clone);
    if let Some(logic) = &mut engine.logic {
        logic.cloned(&clone, &of);
    }
    Some((clone, of))
}

/// Which actor a script means, with an empty slot meaning itself - the rule
/// `Vm::find_actor` uses, and the one thing [`resolve_actor`] leaves out
/// because an empty slot elsewhere means nothing at all.
fn named_or_self(engine: &Engine, running: &str, wanted: &str) -> Option<String> {
    if wanted.trim().is_empty() {
        return Some(running.to_string());
    }
    resolve_actor(engine, running, wanted)
}

/// An actor with nothing to draw: it has no `Look` component, or one for the
/// other dimension. It still gets an entity, so its blocks can move it, other
/// actors can sense it, and adding a `Look` later is all it takes to see it.
fn spawn_unseen(commands: &mut Commands, actor: &Actor, mode: Mode) -> Entity {
    if actor
        .visual()
        .is_some_and(|visual| visual.is_3d() != mode.is_3d())
    {
        let wanted = if mode.is_3d() { "2D" } else { "3D" };
        warn!("{} has a {wanted} shape in a {mode:?} project", actor.name);
    }
    commands.spawn(actor_bundle(actor)).id()
}

/// Gives the actor its camera component, if the project put one on it.
fn attach_camera(commands: &mut Commands, actor: &Actor, entity: Entity) {
    if let Some(camera) = actor.camera() {
        commands.entity(entity).insert(CameraRig(*camera));
    }
}

// ─── Sensing ───────────────────────────────────────────────────────────────

/// Publishes the snapshot reporter blocks read, and starts `when key pressed`
/// strands for keys that went down this frame.
///
/// Runs after the frame's input has landed (`type_into_focused_input`,
/// `detect_clicks`), so the snapshot reflects the click that fired a strand
/// rather than the frame before it - a one-shot `when input changed` strand
/// would otherwise read the value the click just moved away from.
pub fn publish_sensors(
    mut engine: NonSendMut<Engine>,
    manager: Res<crate::ui::UiManager>,
    time: Res<Time>,
    dimension: Res<Dimension>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    pads: Query<&Gamepad>,
    touches: Res<Touches>,
    mut motion: MessageReader<MouseMotion>,
    mut focus: MessageReader<WindowFocused>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    actors: Query<(&ActorId, &Transform, &Visibility, Option<&CustomComponents>)>,
    sound: Res<crate::sound::SoundState>,
    preview_pointer: Option<ResMut<crate::preview::PreviewPointer>>,
) {
    let now = time.elapsed_secs() as f64;
    let held: HashSet<String> = keys.get_pressed().filter_map(key_name).collect();
    // OS-confirmed focus, not the component: `Window::focused` defaults to
    // true and winit only reports changes, so a game opened behind the
    // editor would otherwise read focused until the heat death of the run.
    for event in focus.read() {
        engine.window_focused = event.focused;
    }
    // A live preview pointer counts as attention: the OS window sits behind
    // the editor while the viewport is used, so it would otherwise never
    // report focus.
    let preview_live = preview_pointer
        .as_ref()
        .is_some_and(|pointer| crate::preview::pointer_live(pointer));
    let focused = engine.window_focused || preview_live;
    let mouse = preview_pointer
        .as_ref()
        .and_then(|pointer| {
            let pos = pointer.pos.filter(|_| preview_live)?;
            screen_to_world(dimension.0, pos, &cameras)
        })
        .or_else(|| mouse_world_position(dimension.0, &windows, &cameras))
        .unwrap_or_default();
    // The pointer travels a few pixels a frame, not a teleport: sum the
    // motion events into one delta so a reporter reads what moved since last
    // frame, whichever half of the window it crossed.
    let mut mouse_delta = [0.0f32; 2];
    for moved in motion.read() {
        mouse_delta[0] += moved.delta.x;
        mouse_delta[1] += moved.delta.y;
    }
    if let Some(mut pointer) = preview_pointer {
        mouse_delta[0] += pointer.delta.x;
        mouse_delta[1] += pointer.delta.y;
        pointer.delta = Vec2::ZERO;
    }
    // Motion events are raw device input: they arrive whichever window holds
    // the cursor, including the editor beside this one. A game that answered
    // those would turn under a cursor it can't see, so only a focused window
    // feeds its game. The drain above still runs, so nothing stale bursts
    // out the moment focus lands.
    if !focused {
        mouse_delta = [0.0; 2];
    }

    let mut senses: HashMap<String, ActorSense> = HashMap::new();
    // Every actor's world transform, so a child can be answered about its
    // place in its parent's frame below.
    let posed: HashMap<&str, Transform> = actors
        .iter()
        .map(|(id, transform, _, _)| (id.0.as_str(), *transform))
        .collect();
    for (id, transform, visibility, custom) in &actors {
        // The parent's world transform inverted onto this actor's own: the
        // world position itself when it hangs off nothing, or its parent is
        // gone. The inverse of `world_of`, which places an offset.
        let local_position = engine
            .parents
            .get(&id.0)
            .and_then(|parent| posed.get(parent.as_str()))
            .map(|parent| local_of(parent, transform.translation))
            .unwrap_or(transform.translation.to_array());
        let (rx, ry, rz) = transform.rotation.to_euler(EulerRot::XYZ);
        let (layer, mask, trigger) = engine.filter_of(&id.0);
        let has_body = engine.has_component(&id.0, "Body");
        let shape = collider_shape(&engine, &id.0, dimension.0, transform);
        senses.insert(
            id.0.clone(),
            ActorSense {
                name: engine
                    .actor(&id.0)
                    .map(|actor| actor.name.clone())
                    .unwrap_or_default(),
                position: transform.translation.to_array(),
                local_position,
                rotation: [rx.to_degrees(), ry.to_degrees(), rz.to_degrees()],
                scale: size_of(transform, engine.stretch_of(&id.0)),
                visible: *visibility != Visibility::Hidden,
                parent: engine.parents.get(&id.0).cloned().unwrap_or_default(),
                is_clone: engine.clones.contains_key(&id.0),
                last_created: engine.last_created.get(&id.0).cloned().unwrap_or_default(),
                touching: engine.touching.get(&id.0).cloned().unwrap_or_default(),
                attached: engine.attached.get(&id.0).cloned().unwrap_or_default(),
                components: custom.map(|custom| custom.0.clone()).unwrap_or_default(),
                has_body,
                trigger,
                layer,
                mask,
                shape,
            },
        );
    }

    // Wanted and focused reads as held: the component alone would still say
    // Locked after an unfocused request the backend silently dropped, and
    // reading the window back can't see that drop either.
    let mouse_locked = engine.wants_cursor_locked && focused;

    // A focused text input owns the keyboard: while one does, game strands
    // see no keys at all, so typing a name never also fires the gun.
    let typing = manager.focus().is_some();
    let keys_live = if typing { HashSet::new() } else { held };

    // Gamepads: every connected pad feeds one shared state, strongest wins.
    let mut pad_buttons: HashSet<String> = HashSet::new();
    let mut pad_axes: HashMap<String, f32> = HashMap::new();
    let mut pad_count = 0;
    for pad in &pads {
        pad_count += 1;
        for button in GamepadButton::all() {
            if pad.pressed(button) {
                pad_buttons.insert(pad_button_name(button));
            }
        }
        for axis in GamepadAxis::all() {
            if let Some(value) = pad.get(axis) {
                let name = pad_axis_name(axis);
                let kept = pad_axes.get(&name).copied().unwrap_or(0.0);
                if value.abs() > kept.abs() {
                    pad_axes.insert(name, value);
                }
            }
        }
    }
    // Pad buttons and sticks keep working while typing: only the keyboard
    // belongs to the text input, so a gamepad pause button still pauses.
    let mut mouse_buttons: HashSet<String> = HashSet::new();
    if focused && buttons.pressed(MouseButton::Left) {
        mouse_buttons.insert("left".to_string());
    }
    if focused && buttons.pressed(MouseButton::Right) {
        mouse_buttons.insert("right".to_string());
    }
    if focused && buttons.pressed(MouseButton::Middle) {
        mouse_buttons.insert("middle".to_string());
    }
    let live = LiveInput {
        keys: keys_live.clone(),
        mouse: mouse_buttons.clone(),
        pad_buttons: pad_buttons.clone(),
        axes: pad_axes.clone(),
    };

    // Named actions, with run-scoped remaps winning over the document.
    // Pressed/released are edges against last frame's held.
    let mut actions: HashMap<String, ActionSense> = HashMap::new();
    let mut fired_actions: Vec<String> = Vec::new();
    let action_defs = engine.project.world.input.actions.clone();
    for action in &action_defs {
        let bindings = engine
            .input_overrides
            .get(&action.name.to_lowercase())
            .cloned()
            .unwrap_or_else(|| action.bindings.clone());
        let scoped = blockloom_core::input::InputAction {
            name: action.name.clone(),
            bindings,
        };
        let held_now = live.action_held(&scoped);
        let value = live.action_value(&scoped);
        let was = engine
            .prev_action_held
            .get(&action.name.to_lowercase())
            .copied()
            .unwrap_or(false);
        let pressed = held_now && !was;
        let released = !held_now && was;
        engine
            .prev_action_held
            .insert(action.name.to_lowercase(), held_now);
        actions.insert(
            action.name.clone(),
            ActionSense {
                held: held_now,
                pressed,
                released,
                value,
            },
        );
        if pressed {
            fired_actions.push(action.name.to_lowercase());
        }
    }
    // Remapped-away actions leave no stale edge behind.
    let valid_actions: HashSet<String> = engine
        .project
        .world
        .input
        .actions
        .iter()
        .map(|action| action.name.to_lowercase())
        .collect();
    engine
        .prev_action_held
        .retain(|name, _| valid_actions.contains(name));

    // Touches in world units, press order. Unfocused windows read none, the
    // same gate the pointer delta keeps.
    let mut touch_points: Vec<TouchSense> = Vec::new();
    let mut touch_started = false;
    if focused {
        for touch in touches.iter() {
            if let Some(point) = screen_to_world(dimension.0, touch.position(), &cameras) {
                touch_points.push(TouchSense {
                    id: touch.id(),
                    position: point,
                });
            }
        }
        touch_started = touches.iter_just_pressed().next().is_some();
    }

    blockloom_core::sense::publish(Sensors {
        time: engine.run_time(now),
        // Never frozen: what a strand the interface started reads, so a
        // clock on a pause menu keeps ticking.
        wall_time: (now - engine.started_at).max(0.0),
        paused: engine.paused,
        keys: keys_live,
        mouse,
        mouse_delta,
        mouse_locked,
        mouse_down: focused && buttons.pressed(MouseButton::Left),
        mouse_buttons,
        actions,
        touches: touch_points,
        gamepad_connected: pad_count > 0,
        gamepad_axes: pad_axes,
        gamepad_buttons: pad_buttons,
        actors: senses,
        ui: manager.senses(),
        ui_focus: manager.focus().unwrap_or_default().to_string(),
        sounds: sound.playing(),
        bus_volumes: sound.bus_volumes(),
    });

    // No world event queues while paused, so resuming never bursts.
    // Escape is the exception: a pause menu toggles on it, and the schedulers
    // run that strand as an interface one while the world stands still.
    if engine.running && !typing {
        for key in keys.get_just_pressed().filter_map(key_name) {
            if engine.paused && key != "escape" {
                continue;
            }
            engine.fire(Event::Key(key));
        }
        if !engine.paused {
            for action in fired_actions {
                engine.fire(Event::Action(action));
            }
            if touch_started {
                engine.fire(Event::Touched);
            }
        }
    }
}

/// Canonical sensor spelling of a pad button.
fn pad_button_name(button: GamepadButton) -> String {
    normalize_pad_button(match button {
        GamepadButton::South => "south",
        GamepadButton::East => "east",
        GamepadButton::North => "north",
        GamepadButton::West => "west",
        GamepadButton::C => "c",
        GamepadButton::Z => "z",
        GamepadButton::LeftTrigger => "lefttrigger",
        GamepadButton::LeftTrigger2 => "lefttrigger2",
        GamepadButton::RightTrigger => "righttrigger",
        GamepadButton::RightTrigger2 => "righttrigger2",
        GamepadButton::Select => "select",
        GamepadButton::Start => "start",
        GamepadButton::Mode => "mode",
        GamepadButton::LeftThumb => "leftthumb",
        GamepadButton::RightThumb => "rightthumb",
        GamepadButton::DPadUp => "dpadup",
        GamepadButton::DPadDown => "dpaddown",
        GamepadButton::DPadLeft => "dpadleft",
        GamepadButton::DPadRight => "dpadright",
        GamepadButton::Other(n) => return format!("other{n}"),
    })
}

/// Canonical sensor spelling of a pad axis.
fn pad_axis_name(axis: GamepadAxis) -> String {
    normalize_pad_axis(match axis {
        GamepadAxis::LeftStickX => "leftstickx",
        GamepadAxis::LeftStickY => "leftsticky",
        GamepadAxis::LeftZ => "leftz",
        GamepadAxis::RightStickX => "rightstickx",
        GamepadAxis::RightStickY => "rightsticky",
        GamepadAxis::RightZ => "rightz",
        GamepadAxis::Other(n) => return format!("other{n}"),
    })
}

/// Where the pointer is in world units: straight out in 2D, and where it meets
/// the ground plane in 3D (so `mouse x`/`mouse y` name a place an actor can
/// actually stand).
fn mouse_world_position(
    mode: Mode,
    windows: &Query<&Window, With<PrimaryWindow>>,
    cameras: &Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) -> Option<[f32; 2]> {
    let window = windows.iter().next()?;
    let cursor = window.cursor_position()?;
    screen_to_world(mode, cursor, cameras)
}

/// The same projection for a touch point: a finger names the same place the
/// pointer would at those window coordinates.
fn screen_to_world(
    mode: Mode,
    screen: Vec2,
    cameras: &Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) -> Option<[f32; 2]> {
    let (camera, camera_transform) = cameras.iter().next()?;
    match mode {
        Mode::TwoD => {
            let point = camera.viewport_to_world_2d(camera_transform, screen).ok()?;
            Some([point.x, point.y])
        }
        Mode::ThreeD => {
            let ray = camera.viewport_to_world(camera_transform, screen).ok()?;
            let distance = ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y))?;
            let point = ray.get_point(distance);
            Some([point.x, point.z])
        }
    }
}

/// Types into whichever text input holds the keyboard. Escape lets go of
/// it, Backspace rubs a character out, and every other key that means a
/// character adds one. `changed` fires per keystroke, as the spec has it.
///
/// While an input holds the keyboard, `publish_sensors` hands the game no
/// keys at all, so nothing else sees this typing.
pub fn type_into_focused_input(
    mut engine: NonSendMut<Engine>,
    mut manager: ResMut<crate::ui::UiManager>,
    mut typed: MessageReader<KeyboardInput>,
) {
    let Some(focused) = manager.focus().map(str::to_string) else {
        // Nothing has the keyboard, but the queue still has to be drained:
        // otherwise a burst arrives the moment an input is clicked.
        typed.clear();
        return;
    };
    if !engine.running {
        return;
    }
    let Some(node) = manager.get(&focused) else {
        typed.clear();
        return;
    };
    let (allow, ceiling) = (node.allow(), node.max_length());
    let mut text = node.value.as_text();
    let before = text.clone();
    let mut release = false;
    // Every character goes through the input's own rules, one at a time:
    // a full field takes no more, and a numeric one takes no letters. What
    // is refused is simply not there - a rubbed-out keystroke rather than
    // an error, since a person holding a key down means no harm by it.
    let write = |text: &mut String, ch: char| {
        if let Some(next) = blockloom_core::ui::typed(text, ch, allow, ceiling) {
            *text = next;
        }
    };
    for key in typed.read() {
        if key.state != ButtonState::Pressed {
            continue;
        }
        match &key.logical_key {
            Key::Escape => release = true,
            Key::Backspace => {
                text.pop();
            }
            Key::Space => write(&mut text, ' '),
            Key::Enter => release = true,
            Key::Character(written) => {
                for ch in written.chars() {
                    write(&mut text, ch);
                }
            }
            _ => {}
        }
    }
    if text != before
        && let Some(value) = manager.changed(&focused, Evaluated::Text(text))
    {
        engine.fire(Event::UiChanged {
            id: focused.clone(),
            value,
        });
    }
    if release {
        manager.focus_on(None);
    }
}

/// Routes a click: the interface is asked first, and only what it doesn't
/// want reaches the world.
///
/// The interface is asked whether the game is running or paused - a pause
/// menu's own buttons are the whole point - while world picks need a running,
/// unpaused game with no modal element up. That is why this can't return
/// early on `paused` the way it used to.
pub fn detect_clicks(
    mut engine: NonSendMut<Engine>,
    mut manager: ResMut<crate::ui::UiManager>,
    buttons: Res<ButtonInput<MouseButton>>,
    dimension: Res<Dimension>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    actors: Query<(&ActorId, &Transform)>,
    laid_out: Query<(&ComputedNode, &UiGlobalTransform)>,
    preview_pointer: Option<Res<crate::preview::PreviewPointer>>,
) {
    if !engine.running || !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    // Embedded in the editor there is no window, only the preview pointer.
    let window = windows.iter().next();
    // Clicks land in the focused window, so a click in the editor beside a
    // running game must never start its click strands. Event truth, same as
    // the motion gate above: the component defaults to focused. A live
    // preview pointer counts as attention for the same reason.
    let preview_live = preview_pointer
        .as_ref()
        .is_some_and(|pointer| crate::preview::pointer_live(pointer));
    if !engine.window_focused && !preview_live {
        return;
    }
    let Some(cursor) = preview_pointer
        .as_ref()
        .and_then(|pointer| pointer.pos.filter(|_| preview_live))
        .or_else(|| window.and_then(Window::cursor_position))
    else {
        return;
    };
    // Boxes are laid out in physical pixels while the cursor reads logical:
    // scale it up the way Bevy's own picking does, or every click falls
    // through on a scaled display.
    let cursor = cursor * window.map_or(1.0, Window::scale_factor);

    // The interface first, topmost-first, over the rectangles Bevy laid out
    // this frame - so a panel and a label are as clickable as a button.
    let hit = manager
        .hit(cursor, |node| screen_rect(&laid_out, node.entity))
        .map(|node| {
            (
                node.spec.id.clone(),
                node.kind,
                node.range,
                node.step(),
                node.entity,
            )
        });
    if let Some((id, kind, range, step, entity)) = hit {
        // Clicking a text input hands it the keyboard; clicking anything
        // else takes it back, which is how clicking away releases the keys.
        manager.focus_on(if kind == UiKind::Input {
            Some(id.as_str())
        } else {
            None
        });
        // A drag on a slider and a press on a toggle are changes, not
        // clicks - though both also count as a click on the element.
        let changed = match kind {
            UiKind::Slider => screen_rect(&laid_out, entity).and_then(|rect| {
                let width = rect.width().max(1.0);
                let fraction = (cursor.x - rect.min.x) / width;
                let at = crate::ui::slider_at(range, step, fraction);
                manager.changed(&id, Evaluated::Number(at))
            }),
            UiKind::Toggle => {
                let was = manager.get(&id).is_some_and(|node| node.value.as_bool());
                manager.changed(&id, Evaluated::Bool(!was))
            }
            _ => None,
        };
        engine.fire(Event::UiClicked { id: id.clone() });
        if let Some(value) = changed {
            engine.fire(Event::UiChanged { id, value });
        }
        return;
    }
    manager.focus_on(None);

    // Nothing in the interface wanted it, so the world picks are next - if
    // the world is taking clicks at all.
    if !world_takes_clicks(&engine, &manager) {
        return;
    }
    let Some((camera, camera_transform)) = cameras.iter().next() else {
        return;
    };

    let mut hits: Vec<String> = Vec::new();
    match dimension.0 {
        Mode::TwoD => {
            let Ok(point) = camera.viewport_to_world_2d(camera_transform, cursor) else {
                return;
            };
            for (id, transform) in &actors {
                let Some(visual) = engine.actor(&id.0).and_then(|a| a.visual()).cloned() else {
                    continue;
                };
                let delta = point - transform.translation.truncate();
                // A circle is picked by distance, not by box: the corners of
                // its bounding square aren't part of the actor.
                if let Visual::Circle { radius, .. } = &visual {
                    let scale = transform.scale.truncate().abs().max_element();
                    if delta.length() <= radius * scale {
                        hits.push(id.0.clone());
                    }
                    continue;
                }
                let half = half_extents(&visual) * transform.scale.truncate();
                if delta.x.abs() <= half.x && delta.y.abs() <= half.y {
                    hits.push(id.0.clone());
                }
            }
        }
        Mode::ThreeD => {
            let Ok(ray) = camera.viewport_to_world(camera_transform, cursor) else {
                return;
            };
            for (id, transform) in &actors {
                let Some(visual) = engine.actor(&id.0).and_then(|a| a.visual()).cloned() else {
                    continue;
                };
                // A bounding sphere is close enough to pick with, and needs no
                // collider - an actor with no body is still clickable.
                let radius = (half_extents3(&visual) * transform.scale).length();
                let to_center = transform.translation - ray.origin;
                let along = to_center.dot(*ray.direction);
                if along < 0.0 {
                    continue;
                }
                let closest = ray.origin + *ray.direction * along;
                if closest.distance(transform.translation) <= radius {
                    hits.push(id.0.clone());
                }
            }
        }
    }
    for actor in hits {
        engine.fire(Event::Click { actor });
    }
}

/// Scrolls the nearest list under the pointer. Children count as part of the
/// list, and clipped children cannot receive clicks or wheel input outside its
/// viewport because `UiManager::hit` checks list ancestors.
pub fn scroll_ui_lists(
    mut wheels: MessageReader<MouseWheel>,
    windows: Query<&Window, With<PrimaryWindow>>,
    manager: Res<crate::ui::UiManager>,
    laid_out: Query<(&ComputedNode, &UiGlobalTransform)>,
    mut scrolls: Query<(&mut ScrollPosition, &Node, &ComputedNode)>,
    preview_pointer: Option<Res<crate::preview::PreviewPointer>>,
) {
    // Drained whatever happens, so a wheel over nothing never lands later.
    let wheels: Vec<MouseWheel> = wheels.read().copied().collect();
    if wheels.is_empty() {
        return;
    }
    // Embedded there is no window, only the preview pointer, as for clicks.
    let window = windows.iter().next();
    let Some(point) = preview_pointer
        .as_ref()
        .filter(|pointer| crate::preview::pointer_live(pointer))
        .and_then(|pointer| pointer.pos)
        .or_else(|| window.and_then(Window::cursor_position))
    else {
        return;
    };
    // Physical boxes, logical cursor: scaled up like a click is.
    let point = point * window.map_or(1.0, Window::scale_factor);
    let Some(hit) = manager.hit(point, |node| screen_rect(&laid_out, node.entity)) else {
        return;
    };
    let Some(list) = manager.scroll_owner(&hit.spec.id) else {
        return;
    };
    let Ok((mut position, node, computed)) = scrolls.get_mut(list.entity) else {
        return;
    };
    for wheel in wheels {
        let scale = if wheel.unit == MouseScrollUnit::Line {
            24.0
        } else {
            1.0
        };
        let max = (computed.content_size() - computed.size()) * computed.inverse_scale_factor();
        if node.overflow.y == OverflowAxis::Scroll {
            position.y = (position.y - wheel.y * scale).clamp(0.0, max.y.max(0.0));
        }
    }
}

/// Whether a click nothing in the interface wanted should reach the world.
/// A frozen world has no world clicks at all, and any visible modal element
/// swallows what lands beside it - so clicking next to a pause menu never
/// fires the gun behind it.
fn world_takes_clicks(engine: &Engine, manager: &crate::ui::UiManager) -> bool {
    !engine.paused && !manager.swallows_world_clicks()
}

/// Where one interface element's box ended up on screen, from the layout
/// Bevy computed this frame. `None` for an element nothing has drawn yet.
fn screen_rect(
    laid_out: &Query<(&ComputedNode, &UiGlobalTransform)>,
    entity: Entity,
) -> Option<Rect> {
    let (node, transform) = laid_out.get(entity).ok()?;
    let size = node.size();
    if size.x <= 0.0 || size.y <= 0.0 {
        return None;
    }
    let centre = transform.translation;
    Some(Rect::from_center_size(centre, size))
}

pub(crate) fn half_extents(visual: &Visual) -> Vec2 {
    match visual {
        Visual::Rect { size, .. } | Visual::Image { size, .. } => {
            Vec2::new(size[0] / 2.0, size[1] / 2.0)
        }
        Visual::Circle { radius, .. } => Vec2::splat(*radius),
        Visual::Tilemap { tilemap } => {
            let size = tilemap.size();
            Vec2::new(size[0] / 2.0, size[1] / 2.0)
        }
        _ => Vec2::ZERO,
    }
}

pub(crate) fn half_extents3(visual: &Visual) -> Vec3 {
    match visual {
        Visual::Cuboid { size, .. } => Vec3::new(size[0] / 2.0, size[1] / 2.0, size[2] / 2.0),
        Visual::Sphere { radius, .. } => Vec3::splat(*radius),
        Visual::Capsule { radius, height, .. } => {
            Vec3::new(*radius, height / 2.0 + radius, *radius)
        }
        Visual::Plane { size, .. } => Vec3::new(size[0] / 2.0, 0.1, size[1] / 2.0),
        // A model's extents are its authored scale until the glTF scene
        // loads and reports its own bounds.
        Visual::Model { scale, .. } => Vec3::new(scale[0] / 2.0, scale[1] / 2.0, scale[2] / 2.0),
        // A wall map's face: width, height, no depth.
        Visual::Tilemap { tilemap } => {
            let size = tilemap.size();
            Vec3::new(size[0] / 2.0, size[1] / 2.0, 0.1)
        }
        _ => Vec3::ZERO,
    }
}

/// What a physics query sees: the actor's collider in world units, or
/// nothing for an actor with no body. Scale is folded in; rotation is not,
/// so a spun actor still queries against its unrotated box.
fn collider_shape(
    engine: &Engine,
    id: &str,
    mode: Mode,
    transform: &Transform,
) -> blockloom_core::sense::ColliderShape {
    use blockloom_core::sense::ColliderShape;
    if !engine.has_component(id, "Body") {
        return ColliderShape::None;
    }
    let Some(actor) = engine.actor(id) else {
        return ColliderShape::None;
    };
    let Some(visual) = actor.visual() else {
        return ColliderShape::None;
    };
    let scale = transform.scale.abs();
    match (visual, mode) {
        (Visual::Circle { radius, .. }, Mode::TwoD) => {
            let radius = radius * scale.max_element().max(0.0);
            (radius > 0.0).then_some(ColliderShape::Ball { radius })
        }
        (Visual::Sphere { radius, .. }, Mode::ThreeD) => {
            let radius = radius * scale.max_element().max(0.0);
            (radius > 0.0).then_some(ColliderShape::Ball { radius })
        }
        (Visual::Rect { size, .. } | Visual::Image { size, .. }, Mode::TwoD) => {
            let half = [
                size[0] / 2.0 * scale.x.max(0.0),
                size[1] / 2.0 * scale.y.max(0.0),
                0.0,
            ];
            (half[0] > 0.0 && half[1] > 0.0).then_some(ColliderShape::Box { half })
        }
        (Visual::Cuboid { size, .. }, Mode::ThreeD) => {
            let half = [
                size[0] / 2.0 * scale.x.max(0.0),
                size[1] / 2.0 * scale.y.max(0.0),
                size[2] / 2.0 * scale.z.max(0.0),
            ];
            (half[0] > 0.0 && half[1] > 0.0 && half[2] > 0.0).then_some(ColliderShape::Box { half })
        }
        (Visual::Capsule { radius, height, .. }, Mode::ThreeD) => {
            let half = [
                radius * scale.x.max(0.0),
                (height / 2.0 + radius) * scale.y.max(0.0),
                radius * scale.z.max(0.0),
            ];
            (half[0] > 0.0 && half[1] > 0.0).then_some(ColliderShape::Box { half })
        }
        (Visual::Plane { size, .. }, Mode::ThreeD) => {
            let half = [
                size[0] / 2.0 * scale.x.max(0.0),
                0.1,
                size[1] / 2.0 * scale.z.max(0.0),
            ];
            (half[0] > 0.0 && half[2] > 0.0).then_some(ColliderShape::Box { half })
        }
        (Visual::Model { scale: size, .. }, Mode::ThreeD) => {
            let half = [
                size[0] / 2.0 * scale.x.max(0.0),
                size[1] / 2.0 * scale.y.max(0.0),
                size[2] / 2.0 * scale.z.max(0.0),
            ];
            (half[0] > 0.0 && half[1] > 0.0 && half[2] > 0.0).then_some(ColliderShape::Box { half })
        }
        (Visual::Tilemap { tilemap }, Mode::TwoD) if tilemap.solid => {
            let size = tilemap.size();
            let half = [
                size[0] / 2.0 * scale.x.max(0.0),
                size[1] / 2.0 * scale.y.max(0.0),
                0.0,
            ];
            (half[0] > 0.0 && half[1] > 0.0).then_some(ColliderShape::Box { half })
        }
        _ => None,
    }
    .unwrap_or(ColliderShape::None)
}

/// The world-space point a speech bubble should sit over. The bubble itself is
/// screen-space UI, but measuring the visual here keeps it attached to the top
/// of both 2D and 3D actors as they move and scale.
pub fn actor_top(actor: &Actor, transform: &Transform, mode: Mode) -> Vec3 {
    let half_height = match (actor.visual(), mode) {
        (Some(visual), Mode::TwoD) => half_extents(visual).y,
        (Some(visual), Mode::ThreeD) => half_extents3(visual).y,
        (None, _) => 0.0,
    } * transform.scale.y.abs();
    transform.translation + Vec3::Y * half_height
}

// ─── Running blocks ────────────────────────────────────────────────────────

/// One block-program tick per fixed step, through native logic in a fast build
/// and the VM otherwise. Effects take the same path after either scheduler.
pub fn step_vm(
    mut engine: NonSendMut<Engine>,
    time: Res<Time>,
    mut effects: ResMut<PendingEffects>,
) {
    // A paused world is still stepped: the scheduler gives a slice to the
    // strands the interface started and skips everything else, which is what
    // keeps a pause menu's own buttons alive.
    if !engine.running {
        return;
    }
    let elapsed = time.elapsed_secs() as f64;
    let now = engine.run_time(elapsed);
    let wall = (elapsed - engine.started_at).max(0.0);
    let mut produced = Vec::new();
    let mut messages = Vec::new();
    if engine.logic.is_some() {
        let variables = engine.variables.clone();
        let lists = engine.lists.clone();
        let dicts = engine.dicts.clone();
        engine.logic.as_mut().expect("checked above").tick(
            now,
            wall,
            variables,
            lists,
            dicts,
            &mut produced,
            &mut messages,
        );
    } else {
        engine.vm.tick_at(now, wall, &mut produced);
    }
    for message in messages {
        engine.fire(Event::Message(message));
    }
    for effect in &produced {
        match effect {
            Effect::Say { actor, text } => {
                engine.note_say(actor, text);
                bridge::send(&RuntimeMessage::Say {
                    actor: actor.clone(),
                    text: text.clone(),
                });
            }
            Effect::Error { actor, message } => bridge::send(&RuntimeMessage::Error {
                actor: actor.clone(),
                message: message.clone(),
            }),
            _ => {}
        }
    }
    effects.0.extend(produced);

    // An idle VM is still a live play session. It must keep accepting key,
    // click, collision, and broadcast events until Stop or `stop all` ends it.
}

/// The dimension-agnostic effects: anything that's a transform, a scale or a
/// visibility change.
pub fn apply_common(
    mut commands: Commands,
    effects: Res<PendingEffects>,
    mut engine: NonSendMut<Engine>,
    dimension: Res<Dimension>,
    navmesh: Option<Res<NavMesh>>,
    mut exit: MessageWriter<AppExit>,
    mut transforms: Query<(&mut Transform, &mut Visibility)>,
    mut controllers_2d: Query<&mut bevy_rapier2d::prelude::KinematicCharacterController>,
    mut controllers_3d: Query<&mut bevy_rapier3d::prelude::KinematicCharacterController>,
    mut velocities_2d: Query<&mut bevy_rapier2d::prelude::Velocity>,
    mut velocities_3d: Query<&mut bevy_rapier3d::prelude::Velocity>,
) {
    if !engine.running || engine.paused {
        return;
    }
    let positions: HashMap<String, Vec3> = engine
        .entities
        .iter()
        .filter_map(|(id, entity)| {
            transforms
                .get(*entity)
                .ok()
                .map(|(transform, _)| (id.clone(), transform.translation))
        })
        .collect();

    for effect in &effects.0 {
        let Some(actor) = effect_actor(effect) else {
            if let Effect::Stopped = effect {
                engine.running = false;
                engine.speech.clear();
                bridge::send(&RuntimeMessage::Stopped);
                // Nothing can press Play again in a built game, so a stopped
                // world is a finished one: `stop all` is how a game quits.
                if !bridge::attached() {
                    exit.write(AppExit::Success);
                }
            }
            continue;
        };
        let Some(entity) = engine.entities.get(actor).copied() else {
            continue;
        };
        let Ok((mut transform, mut visibility)) = transforms.get_mut(entity) else {
            continue;
        };
        match effect {
            Effect::Move { steps, .. } => {
                // A dynamic body is moved through its velocity instead, by the
                // dimension's own system - teleporting one every frame stops
                // the solver ever resolving a contact, and it walks through
                // walls and floors. See `dim2::apply_effects`.
                if is_dynamic(&engine, actor) || is_character(&engine, actor) {
                    continue;
                }
                let forward = forward_of(&transform, dimension.0);
                transform.translation += forward * *steps;
            }
            Effect::GoTo { position, .. } => {
                transform.translation = vec3_in(dimension.0, *position, transform.translation);
            }
            Effect::NavigateTo { target, speed, .. } => {
                // One step along the baked mesh at `speed` units per second.
                // No route, or no mesh at all, steps straight at the target,
                // so the actor never idles where `go to` would have moved.
                let mode = dimension.0;
                let from3 = transform.translation;
                let from = nav::plane_coords(mode, [from3.x, from3.y, from3.z]);
                let to = nav::plane_coords(mode, *target);
                let rate = engine.project.world.fixed_rate.clamp(1.0, 1000.0);
                let max_step = speed.max(0.0) / rate;
                let nav = navmesh.as_deref();
                let layer = engine
                    .actor(actor)
                    .and_then(|a| a.components.brain())
                    .map(|brain| brain.layer)
                    .unwrap_or(1);
                let route = nav.and_then(|nav| {
                    nav.mesh.as_ref().and_then(|mesh| {
                        nav::find_route(mesh, from, to, &nav.settings, layer)
                            .map(|path| (path, &nav.settings))
                    })
                });
                let (mut next, linked) = match route {
                    Some((path, settings)) => {
                        nav::next_route_step(&path, from, max_step, settings, layer)
                    }
                    None => (nav::next_step(&[to], from, max_step), false),
                };
                if !linked {
                    if let Some(radius) = engine
                        .actor(actor)
                        .and_then(|a| a.components.brain())
                        .map(|brain| brain.separation.max(0.0))
                        .filter(|radius| *radius > 0.0)
                    {
                        let mut away = Vec2::ZERO;
                        for (other, position) in &positions {
                            if other == actor || !engine.has_component(other, "Brain") {
                                continue;
                            }
                            let point = nav::plane_coords(mode, position.to_array());
                            if (point[0] - to[0]).hypot(point[1] - to[1]) < radius * 0.5 {
                                continue;
                            }
                            let offset = Vec2::new(from[0] - point[0], from[1] - point[1]);
                            let distance = offset.length();
                            if distance > 1e-4 && distance < radius {
                                away += offset / distance * (1.0 - distance / radius);
                            }
                        }
                        let desired = Vec2::new(next[0] - from[0], next[1] - from[1]);
                        let step = (desired + away * max_step).clamp_length_max(max_step);
                        next = [from[0] + step.x, from[1] + step.y];
                    }
                }
                let destination = match mode {
                    Mode::TwoD => Vec3::new(next[0], next[1], from3.z),
                    Mode::ThreeD => Vec3::new(next[0], from3.y, next[1]),
                };
                if linked {
                    transform.translation = destination;
                } else if is_character(&engine, actor) {
                    let delta = destination - from3;
                    match mode {
                        Mode::TwoD => {
                            if let Ok(mut controller) = controllers_2d.get_mut(entity) {
                                controller.translation = Some(
                                    controller.translation.unwrap_or(Vec2::ZERO) + delta.truncate(),
                                );
                            }
                        }
                        Mode::ThreeD => {
                            if let Ok(mut controller) = controllers_3d.get_mut(entity) {
                                controller.translation =
                                    Some(controller.translation.unwrap_or(Vec3::ZERO) + delta);
                            }
                        }
                    }
                } else if is_dynamic(&engine, actor) {
                    let delta = (destination - from3) * rate;
                    match mode {
                        Mode::TwoD => {
                            if let Ok(mut velocity) = velocities_2d.get_mut(entity) {
                                velocity.linear = delta.truncate();
                            }
                        }
                        Mode::ThreeD => {
                            if let Ok(mut velocity) = velocities_3d.get_mut(entity) {
                                velocity.linear.x = delta.x;
                                velocity.linear.z = delta.z;
                            }
                        }
                    }
                } else {
                    transform.translation = destination;
                }
            }
            Effect::ChangePosition { axis, by, .. } => {
                if is_dynamic(&engine, actor) || is_character(&engine, actor) {
                    continue;
                }
                if let Some(index) = position_axis(dimension.0, *axis) {
                    transform.translation[index] += *by;
                }
            }
            Effect::Turn { axis, degrees, .. } => {
                // A dynamic body turns in its dimension's own system, in
                // effect order with its deferred `move`. Turning here would
                // net a turn-move-turn sandwich (how strafe is spelled) back
                // to zero before the move ever sees the turn, so both strafe
                // keys would walk forward. See `dim2|3::apply_effects`.
                if is_dynamic(&engine, actor) {
                    continue;
                }
                if dimension.0 == Mode::ThreeD {
                    turn_3d(&mut transform, *axis, degrees.to_radians());
                } else if rotation_axis(dimension.0, *axis).is_some() {
                    turn_2d(&mut transform, degrees.to_radians());
                }
            }
            Effect::SetRotation { axis, degrees, .. } => {
                if dimension.0 == Mode::ThreeD {
                    set_rotation_3d(&mut transform, *axis, degrees.to_radians());
                } else if let Some(index) = rotation_axis(dimension.0, *axis) {
                    let mut euler = euler_of(&transform);
                    euler[index] = degrees.to_radians();
                    transform.rotation = quat_of(euler);
                }
            }
            Effect::PointTowards { target, .. } => {
                let here = transform.translation;
                let Some(to) = target_position(&engine, &positions, target, here) else {
                    continue;
                };
                match dimension.0 {
                    Mode::TwoD => {
                        let delta = to - here;
                        let mut euler = euler_of(&transform);
                        euler[2] = delta.y.atan2(delta.x);
                        transform.rotation = quat_of(euler);
                    }
                    Mode::ThreeD => transform.look_at(to, Vec3::Y),
                }
            }
            Effect::SetScale { factor, .. } => {
                transform.scale = Vec3::from(engine.stretch_of(actor)) * *factor
            }
            Effect::SetVisible { visible, .. } => {
                *visibility = if *visible {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
            Effect::Glide {
                seconds, target, ..
            } => {
                let to = vec3_in(dimension.0, *target, transform.translation);
                if *seconds <= 0.0 {
                    transform.translation = to;
                } else {
                    commands.entity(entity).insert(Gliding {
                        from: transform.translation,
                        to,
                        elapsed: 0.0,
                        duration: *seconds,
                    });
                }
            }
            // Physics, colors and speech are somebody else's job.
            _ => {}
        }
    }
}

/// Advances every glide, and drops the component when it arrives.
/// Frozen while paused or stopped, like the VM and physics.
pub fn step_glides(
    mut commands: Commands,
    time: Res<Time>,
    engine: NonSend<Engine>,
    mut gliding: Query<(Entity, &mut Transform, &mut Gliding)>,
) {
    if !engine.running || engine.paused {
        return;
    }
    for (entity, mut transform, mut glide) in &mut gliding {
        glide.elapsed += time.delta_secs();
        let progress = (glide.elapsed / glide.duration).clamp(0.0, 1.0);
        transform.translation = glide.from.lerp(glide.to, progress);
        if progress >= 1.0 {
            commands.entity(entity).remove::<Gliding>();
        }
    }
}

/// Drives the world camera from the actor carrying a camera component.
/// Nothing attached leaves the camera where `rebuild_world` put it.
///
/// A 2D project has no depth to stand in, so all three views mean the same
/// thing there: centre on the actor, offset by the rig's x and y.
pub fn drive_camera(
    engine: NonSend<Engine>,
    dimension: Res<Dimension>,
    rigs: Query<(&CameraRig, &Transform), Without<WorldCamera>>,
    mut cameras: Query<(&mut Transform, Option<&mut Projection>), With<WorldCamera>>,
) {
    let Some((rig, target)) = rigs.iter().next() else {
        return;
    };
    let Ok((mut camera, projection)) = cameras.single_mut() else {
        return;
    };
    let rig = rig.0;
    let offset = Vec3::from(rig.offset);
    if let Mode::TwoD = dimension.0 {
        camera.translation = Vec3::new(
            target.translation.x + offset.x,
            target.translation.y + offset.y,
            camera.translation.z,
        );
        return;
    }
    // The offset is in the actor's own frame, so an eye stays on its head
    // however the actor is turned.
    let pivot = target.translation + target.rotation * offset;
    match rig.view {
        CameraView::FirstPerson => {
            camera.translation = pivot;
            // Yaw from the body, pitch from the rig: the FPS composition,
            // where turning the body never weakens looking up and down.
            camera.rotation = target.rotation * Quat::from_rotation_x(rig.pitch.to_radians());
        }
        CameraView::ThirdPerson => {
            let pitch = rig.pitch.to_radians();
            let back = -forward_of(target, Mode::ThreeD) * rig.distance * pitch.cos();
            camera.translation = pivot + back + Vec3::Y * rig.distance * pitch.sin();
            camera.look_at(pivot, Vec3::Y);
        }
        CameraView::Follow => {
            let settings = &engine.project.world.camera;
            let boom = Vec3::from(settings.position) - Vec3::from(settings.look_at);
            camera.translation = target.translation + boom;
            camera.look_at(target.translation, Vec3::Y);
        }
    }
    if dimension.0 == Mode::ThreeD
        && let Some(mut projection) = projection
        && let Projection::Perspective(perspective) = projection.as_mut()
    {
        perspective.fov = rig.fov.clamp(30.0, 110.0).to_radians();
    }
}

/// Puts every actor back at the pose its last fixed step left it at, before
/// this one starts.
///
/// [`interpolate_poses`] writes a transform part-way between two steps so the
/// display is smooth, and that drawn pose is what a fixed step would
/// otherwise find in `Transform` and build on: a `move` would add to it, and
/// `apply_parenting` would read the difference as motion the parent never
/// made. Simulation starts from the settled pose; the renderer's is only for
/// the frames in between.
pub fn restore_poses(engine: NonSend<Engine>, mut posed: Query<(&mut Transform, &PhysicsPose)>) {
    if !engine.running || engine.paused {
        return;
    }
    for (mut transform, pose) in &mut posed {
        if *transform != pose.0 {
            *transform = pose.0;
        }
    }
}

/// Renders actors between the poses they settled at, so nothing on screen
/// marches along at the fixed step rate - whether physics wrote the pose or a
/// step's effects did. On a high-refresh display that step pattern would
/// otherwise be visible as stutter.
pub fn interpolate_poses(
    engine: NonSend<Engine>,
    fixed: Res<Time<Fixed>>,
    mut posed: Query<(&mut Transform, &PhysicsPose, &PrevPose)>,
) {
    if !engine.running || engine.paused {
        // Frozen: put each actor back exactly where its last step left it.
        for (mut transform, current, _) in &mut posed {
            *transform = current.0;
        }
        return;
    }
    let alpha = fixed.overstep_fraction();
    for (mut transform, current, previous) in &mut posed {
        transform.translation = previous.0.translation.lerp(current.0.translation, alpha);
        transform.rotation = previous.0.rotation.slerp(current.0.rotation, alpha);
        transform.scale = previous.0.scale.lerp(current.0.scale, alpha);
    }
}

/// The effects that are about an actor's components rather than about the
/// world: writing a custom component's field, switching the camera's view,
/// and attaching or detaching a whole component mid-run.
///
/// Camera tweaks apply while paused too, so a settings menu can preview
/// them. Everything structural waits for the thaw, like the rest of the
/// simulation.
///
/// The dimension-specific halves of attach/detach - the ones that need a
/// sprite, a mesh or a physics body - are in `dim2`/`dim3`; this owns the
/// bookkeeping, so `engine.attached` is written in exactly one place.
pub fn apply_component_effects(
    mut commands: Commands,
    effects: Res<PendingEffects>,
    mut engine: NonSendMut<Engine>,
    mut customs: Query<&mut CustomComponents>,
    mut visibilities: Query<&mut Visibility>,
    mut rigs: Query<&mut CameraRig>,
    mut transforms: Query<&mut Transform>,
) {
    if !engine.running {
        return;
    }
    let paused = engine.paused;
    for effect in &effects.0 {
        // While paused only interface-started strands run, and those are a
        // settings menu: its camera tweaks preview live, while structural
        // changes wait for the world to thaw like everything else does.
        if paused
            && !matches!(
                effect,
                Effect::SetCameraView { .. }
                    | Effect::SetCameraPitch { .. }
                    | Effect::SetCameraFov { .. }
            )
        {
            continue;
        }
        match effect {
            Effect::AttachComponent { actor, component } => attach(
                &mut commands,
                &mut engine,
                &mut customs,
                &mut visibilities,
                &mut transforms,
                actor,
                component,
            ),
            Effect::DetachComponent { actor, component } => detach(
                &mut commands,
                &mut engine,
                &mut customs,
                &mut visibilities,
                actor,
                component,
            ),
            Effect::SetComponentField {
                actor,
                component,
                field,
                value,
            } => {
                let Some(mut custom) = engine
                    .entities
                    .get(actor)
                    .and_then(|entity| customs.get_mut(*entity).ok())
                else {
                    continue;
                };
                // Only the editor declares components, so a block can write a
                // new field but not conjure the component holding it.
                if let Some(fields) = custom.0.get_mut(component) {
                    fields.insert(field.clone(), value.clone());
                }
            }
            Effect::SetCameraView { actor, view } => {
                let Some(entity) = engine.entities.get(actor).copied() else {
                    continue;
                };
                // Only an actor holding the camera has a view to change.
                let Some(mut rig) = camera_of(&engine, actor) else {
                    continue;
                };
                rig.view = *view;
                commands.entity(entity).insert(CameraRig(rig));
            }
            Effect::SetCameraPitch { actor, degrees } => {
                let Some(entity) = engine.entities.get(actor).copied() else {
                    continue;
                };
                // The live rig, not the authored one, so a pitch a view
                // change just reset can be set again straight after.
                let Ok(mut rig) = rigs.get_mut(entity) else {
                    continue;
                };
                // Just short of vertical either way: at the pole the view
                // flips over instead of stopping.
                rig.0.pitch = degrees.clamp(-89.0, 89.0);
            }
            Effect::SetCameraFov { actor, fov } => {
                let Some(entity) = engine.entities.get(actor).copied() else {
                    continue;
                };
                let Ok(mut rig) = rigs.get_mut(entity) else {
                    continue;
                };
                rig.0.fov = fov.clamp(30.0, 110.0);
            }
            _ => {}
        }
    }
}

/// The camera settings an actor is running with: whatever the project gave
/// it, or the defaults if a script attached the camera mid-run.
fn camera_of(engine: &Engine, actor: &str) -> Option<blockloom_core::components::CameraAttach> {
    if !engine.has_component(actor, "Camera") {
        return None;
    }
    Some(
        engine
            .actor(actor)
            .and_then(|actor| actor.camera())
            .copied()
            .unwrap_or_default(),
    )
}

/// Components that aren't this module's to attach: bodies, looks and
/// materials need the dimension's own pipeline, so `dim2`/`dim3` pick those
/// up from the same effect list.
fn is_dimensions_own(component: &str) -> bool {
    matches!(component, "Body" | "Joint" | "Brain" | "Look" | "Material")
}

fn attach(
    commands: &mut Commands,
    engine: &mut Engine,
    customs: &mut Query<&mut CustomComponents>,
    visibilities: &mut Query<&mut Visibility>,
    transforms: &mut Query<&mut Transform>,
    actor: &str,
    component: &str,
) {
    let Some(entity) = engine.entities.get(actor).copied() else {
        return;
    };
    if engine.has_component(actor, component) {
        return;
    }
    // A script can't be loaded mid-frame: its library is opened when the
    // world is built, which is where a rebuilt one is picked up.
    if component == "Script" {
        bridge::send(&RuntimeMessage::Error {
            actor: actor.to_string(),
            message: "a script can only be attached in the editor, not mid-run".to_string(),
        });
        return;
    }
    engine
        .attached
        .entry(actor.to_string())
        .or_default()
        .insert(component.to_string());
    if is_dimensions_own(component) {
        return;
    }
    match component {
        "Render" => {
            // The project's own answer if it has one; a fresh Render shows.
            let visible = engine
                .actor(actor)
                .map(|actor| actor.visible())
                .unwrap_or(true);
            if let Ok(mut slot) = visibilities.get_mut(entity) {
                *slot = if visible {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
        }
        "Camera" => {
            // One camera: taking it means taking it off whoever had it.
            for (other, held) in engine.attached.iter_mut() {
                if other != actor {
                    held.remove("Camera");
                }
            }
            for (id, other) in &engine.entities {
                if id != actor {
                    commands.entity(*other).remove::<CameraRig>();
                }
            }
            let rig = camera_of(engine, actor).unwrap_or_default();
            commands.entity(entity).insert(CameraRig(rig));
        }
        // The hierarchy is `engine.parents`, not anything on the entity. Like
        // `set my parent to`, hanging it off its authored parent puts it at
        // its authored offset when it carries one.
        "Parent" => {
            let Some(parent) = engine
                .actor(actor)
                .and_then(|actor| actor.parent())
                .map(str::to_string)
            else {
                return;
            };
            set_parent(engine, actor, &parent, transforms);
        }
        // Emitters and trails run while attached: hanging one starts the
        // spray, and the state is what the FX systems read.
        "Emitter" => {
            commands
                .entity(entity)
                .insert(crate::fx::EmitterState::fresh());
        }
        "Trail" => {
            commands
                .entity(entity)
                .insert(crate::fx::TrailState::fresh());
        }
        // Anything else is a custom component: it comes back with the fields
        // the editor gave it, or empty if the project never had one.
        name => {
            let fields = engine
                .actor(actor)
                .and_then(|actor| match actor.components.get(name) {
                    Some(blockloom_core::components::ActorComponent::Custom { fields, .. }) => {
                        Some(fields.clone())
                    }
                    _ => None,
                })
                .unwrap_or_default();
            if let Ok(mut custom) = customs.get_mut(entity) {
                custom.0.insert(
                    name.to_string(),
                    fields
                        .into_iter()
                        .map(|field| (field.name, field.value))
                        .collect(),
                );
            }
        }
    }
}

fn detach(
    commands: &mut Commands,
    engine: &mut Engine,
    customs: &mut Query<&mut CustomComponents>,
    visibilities: &mut Query<&mut Visibility>,
    actor: &str,
    component: &str,
) {
    let Some(entity) = engine.entities.get(actor).copied() else {
        return;
    };
    // There would be nowhere left for the actor to be.
    if component == "Place" || !engine.has_component(actor, component) {
        return;
    }
    if let Some(held) = engine.attached.get_mut(actor) {
        held.remove(component);
    }
    if is_dimensions_own(component) {
        return;
    }
    match component {
        // No Render component means visible, the same as it does in the
        // document - hiding an actor is `hide`, not detaching anything.
        "Render" => {
            if let Ok(mut slot) = visibilities.get_mut(entity) {
                *slot = Visibility::Inherited;
            }
        }
        "Camera" => {
            commands.entity(entity).remove::<CameraRig>();
        }
        "Script" => {
            engine.scripts.remove(actor);
        }
        "Parent" => {
            engine.parents.remove(actor);
        }
        // Taking the spray or the trail away stops it: live particles and
        // ghosts fade out on their own rather than vanishing mid-flight.
        "Emitter" => {
            commands.entity(entity).remove::<crate::fx::EmitterState>();
        }
        "Trail" => {
            commands.entity(entity).remove::<crate::fx::TrailState>();
        }
        name => {
            if let Ok(mut custom) = customs.get_mut(entity) {
                custom.0.remove(name);
            }
        }
    }
}

// ─── Actors that come and go ───────────────────────────────────────────────

/// Making, deleting and re-parenting actors, before anything else this step
/// is applied - so a clone made now is already somewhere for the rest of the
/// step's effects to land.
///
/// A clone is the actor it was copied from as the editor authored it,
/// standing where that actor stands at this moment, carrying its live custom
/// component values. What `attach`/`detach` did to the template since Play
/// doesn't carry over: re-attaching a component has always meant the
/// authored one, and a clone is no different.
pub fn apply_lifetimes(
    mut commands: Commands,
    effects: Res<PendingEffects>,
    mut engine: NonSendMut<Engine>,
    dimension: Res<Dimension>,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut textures: ResMut<Assets<Image>>,
    mut graph_materials_2d: ResMut<Assets<crate::materials::GraphMaterial2d>>,
    mut graph_materials_3d: ResMut<Assets<crate::materials::GraphMaterial3d>>,
    mut tile_materials: ResMut<Assets<bevy::sprite_render::ColorMaterial>>,
    mut render_cache: ResMut<crate::performance::RenderCache>,
    live: Query<(&Visibility, Option<&CustomComponents>)>,
    mut transforms: Query<&mut Transform>,
) {
    if !engine.running || engine.paused {
        return;
    }
    for effect in &effects.0 {
        match effect {
            Effect::CreateClone { actor, clone, of } => {
                let Some(mut copy) = engine.actor(of).cloned() else {
                    continue;
                };
                copy.id = clone.clone();
                if let Some(entity) = engine.entities.get(of).copied() {
                    if let Ok(transform) = transforms.get(entity) {
                        let stretch = copy.placement().stretch;
                        copy.components
                            .set_placement(placement_of(transform, stretch));
                        // The live z carries the sort layer, and the spawner
                        // re-adds it: take it back off so a clone of a layered
                        // actor doesn't sort twice as high.
                        copy.components.placement_mut().position[2] -=
                            copy.components.layer() as f32;
                    }
                    if let Ok((visibility, custom)) = live.get(entity) {
                        copy.components
                            .set_visible(*visibility != Visibility::Hidden);
                        if let Some(custom) = custom {
                            carry_custom(&mut copy, custom);
                        }
                    }
                }
                // A clone hangs where its template hangs. Its own blocks can
                // move it off, the same as anything else about it.
                let parent = engine.parents.get(of).cloned();
                copy.components.set_parent(parent.as_deref().unwrap_or(""));
                engine.clones.insert(clone.clone(), of.clone());
                engine.last_created.insert(actor.clone(), clone.clone());
                spawn_runtime_actor(
                    &mut commands,
                    &mut engine,
                    dimension.0,
                    &assets,
                    &mut meshes,
                    &mut materials,
                    &mut textures,
                    &mut graph_materials_2d,
                    &mut graph_materials_3d,
                    &mut tile_materials,
                    &mut render_cache,
                    copy,
                );
            }
            Effect::CreateActor {
                actor,
                id,
                name,
                position,
            } => {
                let mut made = Actor::blank(name.clone(), dimension.0);
                made.id = id.clone();
                made.components.placement_mut().position = *position;
                engine.last_created.insert(actor.clone(), id.clone());
                spawn_runtime_actor(
                    &mut commands,
                    &mut engine,
                    dimension.0,
                    &assets,
                    &mut meshes,
                    &mut materials,
                    &mut textures,
                    &mut graph_materials_2d,
                    &mut graph_materials_3d,
                    &mut tile_materials,
                    &mut render_cache,
                    made,
                );
            }
            Effect::DeleteActor { actor } => delete_actor(&mut commands, &mut engine, actor),
            Effect::SetParent { actor, parent } => {
                set_parent(&mut engine, actor, parent, &mut transforms)
            }
            _ => {}
        }
    }
}

/// A transform, as the `Place` component spells one.
/// `stretch` is the actor's own, since a transform can't tell it from size.
pub(crate) fn placement_of(
    transform: &Transform,
    stretch: [f32; 3],
) -> blockloom_core::scene::Placement {
    let (rx, ry, rz) = transform.rotation.to_euler(EulerRot::XYZ);
    blockloom_core::scene::Placement {
        position: transform.translation.to_array(),
        rotation: [rx.to_degrees(), ry.to_degrees(), rz.to_degrees()],
        scale: size_of(transform, stretch),
        stretch,
    }
}

/// The uniform size a transform stands at, with the actor's stretch taken out.
pub(crate) fn size_of(transform: &Transform, stretch: [f32; 3]) -> f32 {
    let x = stretch[0];
    if x.abs() < 1e-6 {
        transform.scale.x
    } else {
        transform.scale.x / x
    }
}

/// Writes the template's live custom-component values onto the copy, so a
/// clone starts with the numbers its template is carrying rather than the
/// ones the editor typed.
fn carry_custom(copy: &mut Actor, custom: &CustomComponents) {
    for (name, fields) in &custom.0 {
        for (field, value) in fields {
            copy.components.set_field(name, field, value.clone());
        }
    }
}

/// Puts an actor the run made into the world and into every map that answers
/// a question about it.
#[allow(clippy::too_many_arguments)]
fn spawn_runtime_actor(
    commands: &mut Commands,
    engine: &mut Engine,
    mode: Mode,
    assets: &AssetServer,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    textures: &mut Assets<Image>,
    graph_materials_2d: &mut Assets<crate::materials::GraphMaterial2d>,
    graph_materials_3d: &mut Assets<crate::materials::GraphMaterial3d>,
    tile_materials: &mut Assets<bevy::sprite_render::ColorMaterial>,
    render_cache: &mut crate::performance::RenderCache,
    actor: Actor,
) {
    let dir = engine.project_dir.clone();
    let entity = match mode {
        Mode::TwoD => dim2::spawn_actor(
            commands,
            &actor,
            dir.as_deref(),
            assets,
            textures,
            meshes,
            graph_materials_2d,
            tile_materials,
        ),
        Mode::ThreeD => dim3::spawn_actor(
            commands,
            &actor,
            dir.as_deref(),
            assets,
            meshes,
            materials,
            graph_materials_3d,
            render_cache,
        ),
    }
    .unwrap_or_else(|| spawn_unseen(commands, &actor, mode));
    attach_camera(commands, &actor, entity);
    crate::fx::insert_fx_state(commands, &actor, entity);
    // One camera in the world: a clone of the actor holding it takes it.
    if actor.camera().is_some() {
        claim_camera(commands, engine, &actor.id);
    }
    let held = actor
        .components
        .iter()
        .map(|component| component.name().to_string())
        .collect();
    engine.attached.insert(actor.id.clone(), held);
    let physics = actor.physics();
    engine.physics_filter.insert(
        actor.id.clone(),
        (physics.layer(), physics.collision_mask, physics.trigger),
    );
    if let Some(parent) = actor.parent() {
        engine.parents.insert(actor.id.clone(), parent.to_string());
    }
    engine.entities.insert(actor.id.clone(), entity);
    // A scripted actor's library is opened here rather than when the world
    // was built, because this one didn't exist then.
    open_script_for(engine, &actor);
    engine.spawned.insert(actor.id.clone(), actor);
}

/// Takes the camera off everyone but `actor`.
fn claim_camera(commands: &mut Commands, engine: &mut Engine, actor: &str) {
    for (other, held) in engine.attached.iter_mut() {
        if other != actor {
            held.remove("Camera");
        }
    }
    for (id, entity) in &engine.entities {
        if id != actor {
            commands.entity(*entity).remove::<CameraRig>();
        }
    }
}

/// Opens one actor's compiled script, if it has one and the editor has built
/// it. Silent otherwise, the same bargain `open_scripts` makes.
fn open_script_for(engine: &mut Engine, actor: &Actor) {
    let Some(dir) = engine.project_dir.clone() else {
        return;
    };
    let Some(path) = actor.components.script() else {
        return;
    };
    if !crate::script::LoadedScript::is_built(&dir, path) {
        return;
    }
    match crate::script::LoadedScript::load(&dir, path) {
        Ok(script) => {
            engine.scripts.insert(actor.id.clone(), script);
        }
        Err(message) => bridge::send(&RuntimeMessage::Error {
            actor: actor.id.clone(),
            message,
        }),
    }
}

/// Takes an actor out of the world for the rest of the run. An authored one
/// comes back on the next Play: the document was never touched.
fn delete_actor(commands: &mut Commands, engine: &mut Engine, actor: &str) {
    let Some(entity) = engine.entities.remove(actor) else {
        return;
    };
    commands.entity(entity).despawn();
    engine.spawned.remove(actor);
    engine.clones.remove(actor);
    engine.attached.remove(actor);
    engine.physics_filter.remove(actor);
    engine.touching.remove(actor);
    engine.speech.remove(actor);
    engine.scripts.remove(actor);
    engine.scripts_started.remove(actor);
    engine.parents.remove(actor);
    // Its children are let go rather than deleted with it, and nobody is
    // left pointing at it as the actor they just made.
    engine.parents.retain(|_, parent| parent != actor);
    engine.last_created.retain(|_, made| made != actor);
    for touching in engine.touching.values_mut() {
        touching.remove(actor);
    }
}

/// Hangs `actor` off `wanted`, or takes it off when that names nothing. A
/// child carrying an authored offset is placed at it - that far from its new
/// parent, in the parent's own frame, the same `parent * offset` the world is
/// built from - while one without an offset stays exactly where it is.
fn set_parent(
    engine: &mut Engine,
    actor: &str,
    wanted: &str,
    transforms: &mut Query<&mut Transform>,
) {
    if !engine.entities.contains_key(actor) {
        return;
    }
    if wanted.trim().is_empty() {
        engine.parents.remove(actor);
        return;
    }
    let Some(parent) = resolve_actor(engine, actor, wanted) else {
        bridge::send(&RuntimeMessage::Error {
            actor: actor.to_string(),
            message: format!("there's no actor named \"{wanted}\" to hang off"),
        });
        return;
    };
    if parent == actor {
        engine.parents.remove(actor);
        return;
    }
    if engine.would_loop(actor, &parent) {
        let name = engine
            .actor(&parent)
            .map(|other| other.name.clone())
            .unwrap_or_else(|| parent.clone());
        bridge::send(&RuntimeMessage::Error {
            actor: actor.to_string(),
            message: format!("{name} already hangs off this actor, so it can't be its parent"),
        });
        return;
    }
    engine.parents.insert(actor.to_string(), parent.clone());
    // The offset is where the child stands in its parent's frame, so hanging
    // it off someone new puts it there rather than leaving it where it
    // stood. Without one there is nowhere to put it, and it keeps its world
    // place, as before.
    let offset = engine.actor(actor).and_then(|actor| actor.parent_offset());
    let Some(offset) = offset else {
        return;
    };
    let parent_pose = engine
        .entities
        .get(&parent)
        .and_then(|entity| transforms.get(*entity).ok())
        .copied();
    let (Some(parent), Some(child)) = (parent_pose, engine.entities.get(actor).copied()) else {
        return;
    };
    if let Ok(mut mine) = transforms.get_mut(child) {
        mine.translation = world_of(&parent, offset);
    }
}

/// Which actor a name or an id means right now. Clones share their
/// template's name, so a name answers with whichever one comes to hand -
/// `the actor I made` is how a block names one in particular.
pub fn resolve_actor(engine: &Engine, running: &str, wanted: &str) -> Option<String> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return None;
    }
    if wanted.eq_ignore_ascii_case("myself") || wanted.eq_ignore_ascii_case("me") {
        return Some(running.to_string());
    }
    if engine.entities.contains_key(wanted) {
        return Some(wanted.to_string());
    }
    engine
        .actor_ids()
        .find(|id| {
            engine
                .actor(id)
                .is_some_and(|actor| actor.name.eq_ignore_ascii_case(wanted))
        })
        .cloned()
}

/// Where `child` stands in its parent's frame: the parent's world transform
/// inverted onto the child's world position. What the local-position
/// reporters read, and the inverse of [`world_of`].
pub(crate) fn local_of(parent: &Transform, child: Vec3) -> [f32; 3] {
    parent
        .compute_affine()
        .inverse()
        .transform_point3(child)
        .to_array()
}

/// Where an offset in a parent's frame lands in the world: the same
/// `parent * offset` the world is built from, which is also where `set my
/// parent to` puts a child carrying an authored offset.
pub(crate) fn world_of(parent: &Transform, offset: [f32; 3]) -> Vec3 {
    parent.transform_point(Vec3::from(offset))
}

/// Resolves every authored local offset into the world placement the actor
/// is spawned at.
///
/// `Place` stays what the world is built from, so this is the first moment an
/// offset is read: a child that carries one stands at
/// `parent placement * offset`, and one that doesn't stays exactly where its
/// own `Place` puts it. `set my parent to` reads it again at run time, when
/// it hangs the actor off someone new. Parents are laid out before their
/// children, so an offset down a chain is measured against a parent that has
/// already moved.
fn place_authored_children(project: &mut blockloom_core::project::Project) {
    let parents: HashMap<String, String> = project
        .actors
        .iter()
        .filter_map(|actor| Some((actor.id.clone(), actor.parent()?.to_string())))
        .collect();
    if parents.is_empty() {
        return;
    }
    let mut placed: HashMap<String, Transform> = project
        .actors
        .iter()
        .map(|actor| (actor.id.clone(), transform_of(&actor.placement())))
        .collect();
    let mut order: Vec<&String> = parents.keys().collect();
    order.sort_by_key(|id| depth_of(&parents, id));
    let order: Vec<String> = order.into_iter().cloned().collect();

    for child in order {
        let Some(offset) = project
            .actor(&child)
            .and_then(|actor| actor.parent_offset())
        else {
            continue;
        };
        let Some(parent) = parents.get(&child).and_then(|id| placed.get(id)).copied() else {
            continue;
        };
        let Some(mine) = placed.get_mut(&child) else {
            continue;
        };
        mine.translation = world_of(&parent, offset);
        let world = *mine;
        if let Some(actor) = project.actor_mut(&child) {
            actor.components.placement_mut().position = world.translation.to_array();
        }
    }
}

/// A `Place` as the transform the world is built with.
fn transform_of(placement: &blockloom_core::scene::Placement) -> Transform {
    let [rx, ry, rz] = placement.rotation;
    Transform {
        translation: Vec3::from(placement.position),
        rotation: Quat::from_euler(
            EulerRot::XYZ,
            rx.to_radians(),
            ry.to_radians(),
            rz.to_radians(),
        ),
        scale: Vec3::from(placement.scale3()),
    }
}

/// Carries every parent's motion this step onto everything hanging off it.
///
/// Rather than reparenting Bevy's own transforms - which would make every
/// position in the engine relative to somebody and leave rapier owning half
/// of them - a child is moved by exactly the change its parent underwent
/// since the last step: `child = (parent now / parent then) * child`. A child
/// that moved itself this step keeps that motion, and one that is a parent in
/// turn passes the whole of it on, which is what the root-first order is for.
///
/// It runs in `FixedPostUpdate` before `record_poses`, so the parent's change
/// includes whatever physics wrote this step, and the pose the renderer
/// interpolates towards is the one the child ends up at.
pub fn apply_parenting(engine: NonSend<Engine>, mut posed: Query<(&mut Transform, &PhysicsPose)>) {
    if !engine.running || engine.paused || engine.parents.is_empty() {
        return;
    }
    let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
    for (child, parent) in &engine.parents {
        children
            .entry(parent.as_str())
            .or_default()
            .push(child.as_str());
    }
    for parent in roots_first(&engine.parents) {
        let Some(mine) = children.get(parent.as_str()) else {
            continue;
        };
        let Some(entity) = engine.entities.get(&parent).copied() else {
            continue;
        };
        let Ok((transform, pose)) = posed.get(entity) else {
            continue;
        };
        if transform.translation.abs_diff_eq(pose.0.translation, 1e-6)
            && transform.rotation.abs_diff_eq(pose.0.rotation, 1e-6)
            && transform.scale.abs_diff_eq(pose.0.scale, 1e-6)
        {
            continue;
        }
        let delta = transform.compute_affine() * pose.0.compute_affine().inverse();
        for child in mine {
            let Some(entity) = engine.entities.get(*child).copied() else {
                continue;
            };
            if let Ok((mut transform, _)) = posed.get_mut(entity) {
                *transform = Transform::from_matrix(Mat4::from(delta * transform.compute_affine()));
            }
        }
    }
}

/// Every actor that is somebody's parent, parents before their own children,
/// so one pass carries a move all the way down a chain. `prune_parents` and
/// `set_parent` between them rule loops out, and the depth cap is what keeps
/// a hand-made one from hanging this.
fn roots_first(parents: &HashMap<String, String>) -> Vec<String> {
    let mut depths: Vec<(usize, &String)> = parents
        .values()
        .collect::<HashSet<&String>>()
        .into_iter()
        .map(|id| (depth_of(parents, id), id))
        .collect();
    depths.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    depths.into_iter().map(|(_, id)| id.clone()).collect()
}

/// How many parents deep `id` sits. The cap is the whole map, so a loop that
/// somehow got in answers rather than spinning.
fn depth_of(parents: &HashMap<String, String>, id: &str) -> usize {
    let mut at = id;
    for depth in 0..parents.len() {
        match parents.get(at) {
            Some(parent) => at = parent.as_str(),
            None => return depth,
        }
    }
    parents.len()
}

/// Tells the editor where everything is, a few times a second.
pub fn report_status(
    mut engine: NonSendMut<Engine>,
    time: Res<Time>,
    diagnostics: Option<Res<bevy::diagnostic::DiagnosticsStore>>,
    target_bytes: Option<Res<crate::performance::GameViewTargetBytes>>,
    actors: Query<(&ActorId, &Transform, &Visibility)>,
) {
    let now = time.elapsed_secs() as f64;
    if now < engine.next_report {
        return;
    }
    engine.next_report = now + 0.2;
    let statuses = actors
        .iter()
        .map(|(id, transform, visibility)| {
            let (rx, ry, rz) = transform.rotation.to_euler(EulerRot::XYZ);
            ActorStatus {
                id: id.0.clone(),
                position: transform.translation.to_array(),
                rotation: [rx.to_degrees(), ry.to_degrees(), rz.to_degrees()],
                visible: *visibility != Visibility::Hidden,
            }
        })
        .collect();
    let variables = engine.variables.snapshot();
    let globals = variables
        .globals
        .iter()
        .map(|(name, value)| VariableValue {
            name: name.clone(),
            value: value.clone(),
        })
        .collect();
    let mut render_metrics: Vec<RenderMetric> = diagnostics
        .iter()
        .flat_map(|store| store.iter())
        .filter_map(|diagnostic| {
            let name = diagnostic.path().as_str();
            let measured = name.starts_with("render/")
                && (name.ends_with("/elapsed_gpu") || name.ends_with("/elapsed_cpu"));
            if !measured && name != "mesh_allocator_slabs_size" {
                return None;
            }
            Some(RenderMetric {
                name: name.to_string(),
                value: diagnostic.smoothed()?,
                unit: if name == "mesh_allocator_slabs_size" {
                    "bytes"
                } else {
                    "ms"
                }
                .into(),
            })
        })
        .collect();
    render_metrics.sort_by(|a, b| a.name.cmp(&b.name));
    if let Some(bytes) = target_bytes.filter(|bytes| bytes.0 > 0) {
        render_metrics.push(RenderMetric {
            name: "game_view_target_minimum".into(),
            value: bytes.0 as f64,
            unit: "bytes".into(),
        });
    }
    bridge::send(&RuntimeMessage::Status(Status {
        running: engine.running,
        paused: engine.paused,
        time: engine.run_time(now),
        fps: 1.0 / time.delta_secs().max(f32::EPSILON),
        render_metrics,
        actors: statuses,
        globals,
    }));
}

/// Empties the effect list once every apply system has seen it.
pub fn clear_effects(mut effects: ResMut<PendingEffects>) {
    effects.0.clear();
}

/// Carries out run-scoped input remaps. A `bind` adds one binding to an
/// action's override, a `clear` empties it; both last exactly as long as the
/// run. Unknown actions and unparseable bindings are reported, not applied.
pub fn apply_input_effects(mut engine: NonSendMut<Engine>, effects: Res<PendingEffects>) {
    if !engine.running {
        return;
    }
    for effect in &effects.0 {
        match effect {
            Effect::BindAction {
                actor,
                action,
                binding,
            } => {
                let Some(found) = engine
                    .project
                    .world
                    .input
                    .find(action)
                    .map(|found| found.name.clone())
                else {
                    crate::bridge::send(&RuntimeMessage::Error {
                        actor: actor.clone(),
                        message: format!("there's no input action named \"{action}\""),
                    });
                    continue;
                };
                match blockloom_core::input::parse_binding(binding) {
                    Some(parsed) => {
                        let key = found.to_lowercase();
                        if !engine.input_overrides.contains_key(&key) {
                            let defaults = engine
                                .project
                                .world
                                .input
                                .find(&found)
                                .map(|found| found.bindings.clone())
                                .unwrap_or_default();
                            engine.input_overrides.insert(key.clone(), defaults);
                        }
                        let entry = engine.input_overrides.entry(key).or_default();
                        let text = parsed.text();
                        if !entry.iter().any(|held| held.text() == text) {
                            entry.push(parsed);
                        }
                    }
                    None => {
                        crate::bridge::send(&RuntimeMessage::Error {
                            actor: actor.clone(),
                            message: format!("\"{binding}\" isn't a binding"),
                        });
                    }
                }
            }
            Effect::ClearActionBindings { action, actor } => {
                let Some(found) = engine
                    .project
                    .world
                    .input
                    .find(action)
                    .map(|found| found.name.clone())
                else {
                    crate::bridge::send(&RuntimeMessage::Error {
                        actor: actor.clone(),
                        message: format!("there's no input action named \"{action}\""),
                    });
                    continue;
                };
                engine
                    .input_overrides
                    .insert(found.to_lowercase(), Vec::new());
            }
            _ => {}
        }
    }
}

/// Rumbles every connected gamepad. A zero strength or duration stops
/// instead, so `rumble 0 for 0` is a stop block.
pub fn apply_rumble(
    engine: NonSend<Engine>,
    effects: Res<PendingEffects>,
    pads: Query<Entity, With<Gamepad>>,
    mut rumbles: MessageWriter<GamepadRumbleRequest>,
) {
    if !engine.running || engine.paused {
        return;
    }
    for effect in &effects.0 {
        if let Effect::RumbleGamepad { strength, duration } = effect {
            for entity in &pads {
                if *strength <= 0.0 || *duration <= 0.0 {
                    rumbles.write(GamepadRumbleRequest::Stop { gamepad: entity });
                } else {
                    let clamped = (strength / 100.0).clamp(0.0, 1.0);
                    rumbles.write(GamepadRumbleRequest::Add {
                        gamepad: entity,
                        duration: std::time::Duration::from_secs_f32(duration.max(0.0)),
                        intensity: GamepadRumbleIntensity {
                            strong_motor: clamped,
                            weak_motor: clamped,
                        },
                    });
                }
            }
        }
    }
}

/// Carries out pointer lock requests, and guarantees a stopped run never
/// keeps the user's mouse: locking only means anything while running, so a
/// fresh Play always starts unlocked.
pub fn apply_cursor_lock(
    mut engine: NonSendMut<Engine>,
    effects: Res<PendingEffects>,
    mut targets: Query<(&mut Window, &mut CursorOptions), With<PrimaryWindow>>,
) {
    let wanted = engine.wants_cursor_locked;
    if !engine.running {
        engine.wants_cursor_locked = false;
    } else {
        for effect in &effects.0 {
            if let Effect::SetMouseLocked { locked } = effect {
                engine.wants_cursor_locked = *locked;
            }
        }
    }
    let Ok((mut window, mut cursor)) = targets.single_mut() else {
        // Embedded there is no window: the editor's view holds the pointer.
        if engine.wants_cursor_locked != wanted {
            crate::bridge::send(&RuntimeMessage::PointerLock {
                locked: engine.wants_cursor_locked,
            });
        }
        return;
    };
    if !engine.running {
        set_cursor_locked(&mut cursor, false);
        return;
    }
    // Re-asserted every tick, not just when the block runs. The first request
    // usually lands before the window is focused, and the backend answers an
    // unfocused grab with a silent no-op while the component still says
    // Locked - so without this the pointer roams free with no way back.
    // Rewriting the same values re-triggers the backend attempt, so the real
    // lock sticks as soon as the window can hold it, and re-sticks after any
    // later drop. Skipped while unfocused, where no grab can succeed and
    // every attempt only logs another failure. A tab-out releases instead,
    // browser-style, and the return trip re-grabs.
    if engine.wants_cursor_locked && engine.window_focused {
        if cursor.grab_mode != CursorGrabMode::Locked {
            // Center first: a lock pins the cursor where it stands, which is
            // usually still over the editor from the Play click - freezing it
            // there, visible, outside this window. The backend applies the
            // warp before the clip, so the pin and the hidden cursor land
            // inside the game instead.
            let center = Vec2::new(window.width(), window.height()) / 2.0;
            window.set_cursor_position(Some(center));
        }
        set_cursor_locked(&mut cursor, true);
    } else if cursor.grab_mode != CursorGrabMode::None {
        set_cursor_locked(&mut cursor, false);
    }
}

/// Locked is grabbed and hidden, the first-person standard; unlocked is a
/// plain visible pointer again.
fn set_cursor_locked(cursor: &mut CursorOptions, locked: bool) {
    cursor.grab_mode = if locked {
        CursorGrabMode::Locked
    } else {
        CursorGrabMode::None
    };
    cursor.visible = !locked;
}

// ─── Small shared helpers ──────────────────────────────────────────────────

fn effect_actor(effect: &Effect) -> Option<&String> {
    match effect {
        Effect::Move { actor, .. }
        | Effect::GoTo { actor, .. }
        | Effect::NavigateTo { actor, .. }
        | Effect::BurstParticles { actor, .. }
        | Effect::SetEmitterDial { actor, .. }
        | Effect::SetTrailEnabled { actor, .. }
        | Effect::ChangePosition { actor, .. }
        | Effect::Glide { actor, .. }
        | Effect::Turn { actor, .. }
        | Effect::SetRotation { actor, .. }
        | Effect::PointTowards { actor, .. }
        | Effect::SetScale { actor, .. }
        | Effect::SetBody { actor, .. }
        | Effect::ApplyImpulse { actor, .. }
        | Effect::SetVelocity { actor, .. }
        | Effect::SetDensity { actor, .. }
        | Effect::SetMass { actor, .. }
        | Effect::SetTrigger { actor, .. }
        | Effect::SetCollisionLayer { actor, .. }
        | Effect::SetCollisionMask { actor, .. }
        | Effect::Say { actor, .. }
        | Effect::SetVisible { actor, .. }
        | Effect::SetColor { actor, .. }
        | Effect::SetComponentField { actor, .. }
        |         Effect::SetCameraView { actor, .. }
        | Effect::SetCameraPitch { actor, .. }
        | Effect::SetCameraFov { actor, .. }
        | Effect::AttachComponent { actor, .. }
        | Effect::DetachComponent { actor, .. }
        | Effect::BindAction { actor, .. }
        | Effect::ClearActionBindings { actor, .. }
        | Effect::Error { actor, .. } => Some(actor),
        // Making, deleting and re-parenting an actor are `apply_lifetimes`'s
        // to carry out, and none of them is a change to a transform.
        Effect::SetGravity { .. }
        | Effect::SetBusVolume { .. }
        | Effect::RumbleGamepad { .. }
        | Effect::Stopped
        // Sound is the sound module's to play, like UI is the overlay's.
        | Effect::PlaySound { .. }
        | Effect::StopSound { .. }
        | Effect::SetSoundVolume { .. }
        | Effect::SetSoundPitch { .. }
        | Effect::SetMouseLocked { .. }
        // Screen-space, so against no actor at all - `overlay` applies them.
        | Effect::ShowElement { .. }
        | Effect::HideElement { .. }
        | Effect::DeleteElement { .. }
        | Effect::SetUiProp { .. }
        | Effect::SetFocus { .. }
        | Effect::SetUiTheme { .. }
        | Effect::SetPaused { .. }
        | Effect::SaveVariable { .. }
        | Effect::SetParent { .. }
        | Effect::CreateClone { .. }
        | Effect::CreateActor { .. }
        | Effect::DeleteActor { .. } => None,
    }
}

/// A block's `z` is ignored in a 2D project, so the actor keeps its own.
fn vec3_in(mode: Mode, value: [f32; 3], current: Vec3) -> Vec3 {
    match mode {
        Mode::TwoD => Vec3::new(value[0], value[1], current.z),
        Mode::ThreeD => Vec3::new(value[0], value[1], value[2]),
    }
}

/// Which way an actor faces, as a unit vector. In 2D a rotation of zero faces
/// right, so turning anticlockwise raises the angle.
pub fn forward_of(transform: &Transform, mode: Mode) -> Vec3 {
    match mode {
        Mode::TwoD => {
            let angle = transform.rotation.to_euler(EulerRot::XYZ).2;
            Vec3::new(angle.cos(), angle.sin(), 0.0)
        }
        Mode::ThreeD => *transform.forward(),
    }
}

/// True when the physics engine owns this actor's movement.
pub fn is_dynamic(engine: &Engine, actor: &str) -> bool {
    engine
        .actor(actor)
        .is_some_and(|actor| actor.physics().body == BodyKind::Dynamic)
}

pub fn is_character(engine: &Engine, actor: &str) -> bool {
    engine.actor(actor).is_some_and(|actor| {
        let physics = actor.physics();
        physics.body == BodyKind::Kinematic && physics.character_controller
    })
}

/// Which coordinate an axis names. A 2D project has no depth to change, so Z
/// is ignored there rather than moving a sprite out of view.
pub fn position_axis(mode: Mode, axis: Axis) -> Option<usize> {
    match (mode, axis) {
        (Mode::TwoD, Axis::Z) => None,
        _ => Some(axis.index()),
    }
}

/// Which rotation an axis names. In 2D only Z is a rotation at all; X and Y
/// would tip a sprite out of the plane.
fn rotation_axis(mode: Mode, axis: Axis) -> Option<usize> {
    match (mode, axis) {
        (Mode::TwoD, Axis::Z) => Some(2),
        (Mode::TwoD, _) => None,
        (Mode::ThreeD, _) => Some(axis.index()),
    }
}

fn euler_of(transform: &Transform) -> [f32; 3] {
    let (x, y, z) = transform.rotation.to_euler(EulerRot::XYZ);
    [x, y, z]
}

fn quat_of(euler: [f32; 3]) -> Quat {
    Quat::from_euler(EulerRot::XYZ, euler[0], euler[1], euler[2])
}

/// A 2D turn raises the one rotation a sprite has.
pub fn turn_2d(transform: &mut Transform, radians: f32) {
    let mut euler = euler_of(transform);
    euler[2] += radians;
    transform.rotation = quat_of(euler);
}

/// A 3D turn spins about the actor's own axis, composed straight onto the
/// quaternion. Reading the yaw back out of XYZ euler instead folds it into
/// [-90, 90] past the first quarter turn, which walled every 3D game's
/// horizontal look at about 180 degrees of freedom.
pub fn turn_3d(transform: &mut Transform, axis: Axis, radians: f32) {
    let spin = Quat::from_axis_angle(axis_vector(axis), radians);
    transform.rotation = (transform.rotation * spin).normalize();
}

/// A 3D absolute rotation sets one component through YXZ euler, which spells
/// yaw first over the full circle. The XYZ spelling folds a yaw past 90
/// degrees into flipped pitch and roll, so zeroing X or Z through it snaps
/// the yaw back inside - the same 180-degree wall from the other side.
fn set_rotation_3d(transform: &mut Transform, axis: Axis, radians: f32) {
    let (yaw, pitch, roll) = transform.rotation.to_euler(EulerRot::YXZ);
    let mut angles = [pitch, yaw, roll];
    angles[axis.index()] = radians;
    transform.rotation = Quat::from_euler(EulerRot::YXZ, angles[1], angles[0], angles[2]);
}

fn axis_vector(axis: Axis) -> Vec3 {
    match axis {
        Axis::X => Vec3::X,
        Axis::Y => Vec3::Y,
        Axis::Z => Vec3::Z,
    }
}

/// Which linear axes a released walk brakes: every axis but the one gravity
/// pulls along, so a released key stops the run without hanging a fall - and
/// a gravity-free game stops dead on all of them.
pub fn stop_axes(gravity: Vec3) -> [bool; 3] {
    if gravity.length_squared() < 1e-6 {
        return [true, true, true];
    }
    let pull = [gravity.x.abs(), gravity.y.abs(), gravity.z.abs()];
    let mainly = if pull[0] >= pull[1] && pull[0] >= pull[2] {
        0
    } else if pull[1] >= pull[2] {
        1
    } else {
        2
    };
    [mainly != 0, mainly != 1, mainly != 2]
}

/// Where a `point towards` target is: another actor, or the mouse.
fn target_position(
    engine: &Engine,
    positions: &HashMap<String, Vec3>,
    target: &str,
    here: Vec3,
) -> Option<Vec3> {
    if target.eq_ignore_ascii_case("mouse") {
        return blockloom_core::sense::read(|sensors| {
            Some(Vec3::new(sensors.mouse[0], sensors.mouse[1], here.z))
        });
    }
    let id = resolve_actor(engine, target, target)?;
    positions.get(&id).copied()
}

/// Our name for a key, matching what the blocks are written with.
fn key_name(code: &KeyCode) -> Option<String> {
    let name = match code {
        KeyCode::Space => "space",
        KeyCode::ArrowUp => "up arrow",
        KeyCode::ArrowDown => "down arrow",
        KeyCode::ArrowLeft => "left arrow",
        KeyCode::ArrowRight => "right arrow",
        KeyCode::Enter | KeyCode::NumpadEnter => "enter",
        KeyCode::Escape => "escape",
        KeyCode::ShiftLeft | KeyCode::ShiftRight => "shift",
        KeyCode::ControlLeft | KeyCode::ControlRight => "control",
        KeyCode::AltLeft | KeyCode::AltRight => "alt",
        KeyCode::Tab => "tab",
        KeyCode::Backspace => "backspace",
        KeyCode::KeyA => "a",
        KeyCode::KeyB => "b",
        KeyCode::KeyC => "c",
        KeyCode::KeyD => "d",
        KeyCode::KeyE => "e",
        KeyCode::KeyF => "f",
        KeyCode::KeyG => "g",
        KeyCode::KeyH => "h",
        KeyCode::KeyI => "i",
        KeyCode::KeyJ => "j",
        KeyCode::KeyK => "k",
        KeyCode::KeyL => "l",
        KeyCode::KeyM => "m",
        KeyCode::KeyN => "n",
        KeyCode::KeyO => "o",
        KeyCode::KeyP => "p",
        KeyCode::KeyQ => "q",
        KeyCode::KeyR => "r",
        KeyCode::KeyS => "s",
        KeyCode::KeyT => "t",
        KeyCode::KeyU => "u",
        KeyCode::KeyV => "v",
        KeyCode::KeyW => "w",
        KeyCode::KeyX => "x",
        KeyCode::KeyY => "y",
        KeyCode::KeyZ => "z",
        KeyCode::Digit0 | KeyCode::Numpad0 => "0",
        KeyCode::Digit1 | KeyCode::Numpad1 => "1",
        KeyCode::Digit2 | KeyCode::Numpad2 => "2",
        KeyCode::Digit3 | KeyCode::Numpad3 => "3",
        KeyCode::Digit4 | KeyCode::Numpad4 => "4",
        KeyCode::Digit5 | KeyCode::Numpad5 => "5",
        KeyCode::Digit6 | KeyCode::Numpad6 => "6",
        KeyCode::Digit7 | KeyCode::Numpad7 => "7",
        KeyCode::Digit8 | KeyCode::Numpad8 => "8",
        KeyCode::Digit9 | KeyCode::Numpad9 => "9",
        // Punctuation answers to what it prints on a US layout.
        KeyCode::Minus | KeyCode::NumpadSubtract => "-",
        KeyCode::Equal => "=",
        KeyCode::NumpadAdd => "+",
        KeyCode::NumpadMultiply => "*",
        KeyCode::Slash | KeyCode::NumpadDivide => "/",
        KeyCode::Period | KeyCode::NumpadDecimal => ".",
        KeyCode::Comma => ",",
        KeyCode::Semicolon => ";",
        KeyCode::Quote => "'",
        KeyCode::Backquote => "`",
        KeyCode::BracketLeft => "[",
        KeyCode::BracketRight => "]",
        KeyCode::Backslash => "\\",
        _ => return None,
    };
    Some(normalize_key(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::{TimePlugin, TimeUpdateStrategy};

    #[test]
    fn an_idle_vm_keeps_the_play_session_running() {
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::TwoD);
        engine.running = true;

        let mut app = fixed_step_app(engine);
        app.update();

        assert!(app.world().non_send::<Engine>().running);
    }

    /// An app that drives the fixed schedule deterministically: one
    /// `app.update()` is exactly one fixed step at 60 Hz.
    fn fixed_step_app(engine: Engine) -> App {
        let mut app = App::new();
        app.add_plugins(TimePlugin);
        app.insert_resource(Time::<Fixed>::from_hz(60.0));
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        ));
        app.init_resource::<PendingEffects>();
        app.insert_non_send(engine);
        app.add_systems(FixedUpdate, step_vm);
        app
    }

    #[test]
    fn pause_freezes_the_timer_and_unpause_resumes_it() {
        let (sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::TwoD);
        engine.running = true;
        engine.started_at = 10.0;

        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(Time::<Fixed>::from_hz(60.0));
        app.init_resource::<PendingEffects>();
        app.init_resource::<crate::ui::UiManager>();
        app.insert_non_send(engine);
        app.add_systems(Update, pump_editor);
        app.update();

        // Pause at t=15: timer freezes at 5.
        sender.send(EditorMessage::Pause { paused: true }).unwrap();
        app.world_mut()
            .resource_mut::<Time>()
            .advance_to(std::time::Duration::from_secs(15));
        app.update();
        let engine = app.world().non_send::<Engine>();
        assert!(engine.paused);
        assert_eq!(engine.run_time(20.0), 5.0);
        assert_eq!(engine.run_time(15.0), 5.0);

        // Resume at t=25: timer continues from 5, not jumping to 15.
        sender.send(EditorMessage::Pause { paused: false }).unwrap();
        app.world_mut()
            .resource_mut::<Time>()
            .advance_to(std::time::Duration::from_secs(25));
        app.update();
        let engine = app.world().non_send::<Engine>();
        assert!(!engine.paused);
        assert_eq!(engine.run_time(25.0), 5.0);
        assert_eq!(engine.run_time(30.0), 10.0);
    }

    #[test]
    fn preview_step_ticks_once_then_repauses_and_queues_input() {
        use blockloom_protocol::PreviewInput;

        let (sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::TwoD);
        engine.running = true;
        engine.paused = true;
        engine.vm.set_paused(true);

        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(Time::<Fixed>::from_hz(60.0));
        app.init_resource::<PendingEffects>();
        app.init_resource::<crate::ui::UiManager>();
        app.insert_non_send(engine);
        // No PreviewState resource: the sidecar arms are skipped, nothing panics.
        app.add_systems(Update, (pump_editor, finish_step).chain());

        sender.send(EditorMessage::Step).unwrap();
        sender
            .send(EditorMessage::PreviewInput {
                input: PreviewInput::Key {
                    code: "Space".to_string(),
                    down: true,
                },
            })
            .unwrap();
        sender
            .send(EditorMessage::Preview {
                enabled: true,
                headless: false,
            })
            .unwrap();
        app.update();

        let engine = app.world().non_send::<Engine>();
        // Stepped, then re-paused by the end of the tick.
        assert!(engine.paused);
        assert!(!engine.pause_after_tick);
        assert_eq!(engine.preview_inputs.len(), 1);
    }

    /// Runs `drive_camera` once over a world holding one rigged actor and one
    /// camera, and hands back where the camera ended up.
    fn camera_after(
        mode: Mode,
        rig: blockloom_core::components::CameraAttach,
        actor: Transform,
    ) -> Transform {
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut app = App::new();
        app.insert_resource(Dimension(mode));
        app.insert_non_send(Engine::new(incoming, mode));
        let camera = app
            .world_mut()
            .spawn((WorldCamera, Transform::default()))
            .id();
        app.world_mut().spawn((CameraRig(rig), actor));
        app.add_systems(Update, drive_camera);
        app.update();
        *app.world().entity(camera).get::<Transform>().unwrap()
    }

    #[test]
    fn a_first_person_camera_sits_at_the_actors_eye_facing_where_it_faces() {
        use blockloom_core::components::{CameraAttach, CameraView};

        let facing = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let camera = camera_after(
            Mode::ThreeD,
            CameraAttach {
                view: CameraView::FirstPerson,
                offset: [0.0, 2.0, 0.0],
                pitch: 30.0,
                ..CameraAttach::default()
            },
            Transform::from_xyz(5.0, 0.0, 0.0).with_rotation(facing),
        );

        assert!(
            camera
                .translation
                .abs_diff_eq(Vec3::new(5.0, 2.0, 0.0), 0.001)
        );
        // Yaw from the body, pitch from the rig: the FPS composition, so a
        // level body turning never weakens looking up and down.
        let looking = facing * Quat::from_rotation_x(30.0f32.to_radians());
        assert!(camera.rotation.abs_diff_eq(looking, 0.001));
    }

    #[test]
    fn a_third_person_camera_sits_behind_and_above_what_it_looks_at() {
        use blockloom_core::components::{CameraAttach, CameraView};

        let camera = camera_after(
            Mode::ThreeD,
            CameraAttach {
                view: CameraView::ThirdPerson,
                offset: [0.0, 0.0, 0.0],
                distance: 10.0,
                pitch: 30.0,
                ..CameraAttach::default()
            },
            Transform::IDENTITY,
        );

        // Bevy's forward is -Z, so "behind" an unturned actor is +Z.
        assert!(camera.translation.z > 0.0);
        assert!((camera.translation.y - 5.0).abs() < 0.001);
        assert!(camera.translation.x.abs() < 0.001);
    }

    #[test]
    fn a_2d_camera_only_ever_centres_on_its_actor() {
        use blockloom_core::components::{CameraAttach, CameraView};

        let camera = camera_after(
            Mode::TwoD,
            CameraAttach {
                // First person means nothing in a plane, so it still centres.
                view: CameraView::FirstPerson,
                offset: [0.0, 20.0, 0.0],
                ..CameraAttach::default()
            },
            Transform::from_xyz(100.0, 40.0, 0.0),
        );

        assert_eq!(camera.translation.truncate(), Vec2::new(100.0, 60.0));
    }

    #[test]
    fn a_settings_slider_retunes_the_camera_while_paused() {
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::ThreeD);
        engine.running = true;
        engine.paused = true;
        engine.vm.set_paused(true);

        let mut app = App::new();
        app.insert_resource(PendingEffects(vec![
            Effect::SetCameraFov {
                actor: "player".to_string(),
                fov: 90.0,
            },
            Effect::AttachComponent {
                actor: "player".to_string(),
                component: "Body".to_string(),
            },
        ]));
        app.insert_non_send(engine);
        let actor = app
            .world_mut()
            .spawn((
                ActorId("player".to_string()),
                CameraRig(blockloom_core::components::CameraAttach::default()),
            ))
            .id();
        app.world_mut()
            .non_send_mut::<Engine>()
            .entities
            .insert("player".to_string(), actor);
        app.add_systems(Update, apply_component_effects);
        app.update();

        // The slider's tweak previews live, while structural changes wait
        // for the thaw like the rest of the simulation.
        let rig = app.world().entity(actor).get::<CameraRig>().unwrap();
        assert_eq!(rig.0.fov, 90.0);
        assert!(
            !app.world()
                .non_send::<Engine>()
                .has_component("player", "Body")
        );
    }

    /// Runs `apply_cursor_lock` once over one primary window and hands back
    /// what the engine wants plus what the window holds.
    fn lock_after(app: &mut App, window_entity: Entity) -> (bool, CursorGrabMode, bool) {
        app.update();
        let engine = app.world().non_send::<Engine>();
        let wants = engine.wants_cursor_locked;
        let cursor = app
            .world()
            .entity(window_entity)
            .get::<CursorOptions>()
            .expect("the window keeps its cursor options");
        (wants, cursor.grab_mode, cursor.visible)
    }

    #[test]
    fn a_lock_request_waits_for_focus_then_grabs() {
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::ThreeD);
        engine.running = true;
        // No focus event has arrived: the window a fresh Play opens behind.
        // The component still says focused - its default - which is exactly
        // why the engine tracks event truth instead of reading it.

        let mut app = App::new();
        app.insert_resource(PendingEffects(vec![Effect::SetMouseLocked {
            locked: true,
        }]));
        app.insert_non_send(engine);
        let window_entity = app
            .world_mut()
            .spawn((Window::default(), CursorOptions::default(), PrimaryWindow))
            .id();
        app.add_systems(Update, apply_cursor_lock);

        // Wanted, but unwritable: no grab while unfocused, where every
        // attempt only logs another failure.
        assert_eq!(
            lock_after(&mut app, window_entity),
            (true, CursorGrabMode::None, true)
        );

        // The effect is spent; a real focus event retries the same want
        // into a grab.
        app.world_mut().resource_mut::<PendingEffects>().0.clear();
        app.world_mut().non_send_mut::<Engine>().window_focused = true;
        assert_eq!(
            lock_after(&mut app, window_entity),
            (true, CursorGrabMode::Locked, false)
        );
        // ...centered first, so the pin lands inside the game instead of
        // freezing over whatever the cursor sat on. Default window,
        // scale one: middle pixel.
        let position = app
            .world()
            .entity(window_entity)
            .get::<Window>()
            .expect("the window")
            .physical_cursor_position();
        assert_eq!(position, Some(Vec2::new(640.0, 360.0)));
    }

    #[test]
    fn stopping_the_run_releases_the_pointer() {
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::ThreeD);
        engine.running = true;
        engine.window_focused = true;

        let mut app = App::new();
        app.insert_resource(PendingEffects(vec![Effect::SetMouseLocked {
            locked: true,
        }]));
        app.insert_non_send(engine);
        let window_entity = app
            .world_mut()
            .spawn((Window::default(), CursorOptions::default(), PrimaryWindow))
            .id();
        app.add_systems(Update, apply_cursor_lock);

        assert_eq!(
            lock_after(&mut app, window_entity),
            (true, CursorGrabMode::Locked, false)
        );

        // Stop: the want goes with the run, so a finished game never keeps
        // the user's mouse.
        app.world_mut().non_send_mut::<Engine>().running = false;
        assert_eq!(
            lock_after(&mut app, window_entity),
            (false, CursorGrabMode::None, true)
        );
    }

    #[test]
    fn a_tab_out_releases_and_the_return_trip_regrabs() {
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::ThreeD);
        engine.running = true;
        engine.window_focused = true;

        let mut app = App::new();
        app.insert_resource(PendingEffects(vec![Effect::SetMouseLocked {
            locked: true,
        }]));
        app.insert_non_send(engine);
        let window_entity = app
            .world_mut()
            .spawn((Window::default(), CursorOptions::default(), PrimaryWindow))
            .id();
        app.add_systems(Update, apply_cursor_lock);

        assert_eq!(
            lock_after(&mut app, window_entity),
            (true, CursorGrabMode::Locked, false)
        );

        // Tabbing out drops the OS grab behind the component's back, so the
        // runtime lets go too - browser-style - while keeping the want.
        app.world_mut().non_send_mut::<Engine>().window_focused = false;
        assert_eq!(
            lock_after(&mut app, window_entity),
            (true, CursorGrabMode::None, true)
        );

        // Back in: the same want re-grabs without another block running.
        app.world_mut().non_send_mut::<Engine>().window_focused = true;
        assert_eq!(
            lock_after(&mut app, window_entity),
            (true, CursorGrabMode::Locked, false)
        );
    }

    /// Runs `publish_sensors` once over a primary window, after feeding one
    /// motion event and a held left button, and hands back the published
    /// delta and button state. `focused` drives real focus events: `None`
    /// means none ever arrived, which is exactly what a window opened behind
    /// the editor looks like.
    fn motion_after(focused: Option<bool>) -> ([f32; 2], bool) {
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::ThreeD);
        engine.running = true;
        let mut buttons = ButtonInput::<MouseButton>::default();
        buttons.press(MouseButton::Left);

        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(Dimension(Mode::ThreeD));
        app.init_resource::<ButtonInput<KeyCode>>();
        app.insert_resource(buttons);
        app.init_resource::<Messages<MouseMotion>>();
        app.init_resource::<Messages<WindowFocused>>();
        app.init_resource::<crate::ui::UiManager>();
        app.init_resource::<crate::sound::SoundState>();
        app.init_resource::<bevy::input::touch::Touches>();
        app.insert_non_send(engine);
        let window_entity = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        app.world_mut()
            .resource_mut::<Messages<MouseMotion>>()
            .write(MouseMotion {
                delta: Vec2::new(5.0, -3.0),
            });
        if let Some(focused) = focused {
            app.world_mut()
                .resource_mut::<Messages<WindowFocused>>()
                .write(WindowFocused {
                    window: window_entity,
                    focused,
                });
        }
        app.add_systems(Update, publish_sensors);
        app.update();

        blockloom_core::sense::read(|sensors| (sensors.mouse_delta, sensors.mouse_down))
    }

    #[test]
    fn an_unfocused_window_feeds_the_game_no_pointer() {
        // Raw device motion arrives whichever window holds the cursor -
        // including the editor beside a running game - so only a focused
        // window passes it on. The drain still runs, so focus landing later
        // starts from zero rather than a stale burst. No event at all reads
        // as unfocused: the component defaults to focused.
        assert_eq!(motion_after(None), ([0.0, 0.0], false));
        assert_eq!(motion_after(Some(false)), ([0.0, 0.0], false));
        assert_eq!(motion_after(Some(true)), ([5.0, -3.0], true));
    }

    #[test]
    fn paused_vm_produces_no_effects() {
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::TwoD);
        engine.running = true;
        engine.paused = true;
        engine.pause_began = Some(0.0);

        let mut app = fixed_step_app(engine);
        app.update();

        assert!(app.world().resource::<PendingEffects>().0.is_empty());
    }

    /// A world of loose actors with poses, and the parenting pass run once
    /// over it. Each actor is `(id, pose at the start of the step, where it
    /// is now)`, and the answer is where each one ends up.
    fn after_parenting(
        actors: &[(&str, Transform, Transform)],
        parents: &[(&str, &str)],
    ) -> HashMap<String, Transform> {
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::TwoD);
        engine.running = true;

        let mut app = App::new();
        let mut ids = Vec::new();
        for (id, pose, now) in actors {
            let entity = app
                .world_mut()
                .spawn((*now, PhysicsPose(*pose), PrevPose(*pose)))
                .id();
            engine.entities.insert((*id).to_string(), entity);
            ids.push(((*id).to_string(), entity));
        }
        for (child, parent) in parents {
            engine
                .parents
                .insert((*child).to_string(), (*parent).to_string());
        }
        app.insert_non_send(engine);
        app.add_systems(Update, apply_parenting);
        app.update();

        ids.into_iter()
            .map(|(id, entity)| (id, *app.world().entity(entity).get::<Transform>().unwrap()))
            .collect()
    }

    #[test]
    fn a_child_is_carried_by_exactly_what_its_parent_moved() {
        let was = Transform::from_xyz(0.0, 0.0, 0.0);
        let now = Transform::from_xyz(10.0, 4.0, 0.0);
        let child = Transform::from_xyz(100.0, 0.0, 0.0);
        let after = after_parenting(
            &[("parent", was, now), ("child", child, child)],
            &[("child", "parent")],
        );

        assert!(
            after["child"]
                .translation
                .abs_diff_eq(Vec3::new(110.0, 4.0, 0.0), 0.001)
        );
        // The parent itself is left exactly where it got to.
        assert!(
            after["parent"]
                .translation
                .abs_diff_eq(now.translation, 0.001)
        );
    }

    #[test]
    fn a_turning_parent_swings_its_child_around_rather_than_sliding_it() {
        let was = Transform::IDENTITY;
        let now = Transform::from_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
        let child = Transform::from_xyz(10.0, 0.0, 0.0);
        let after = after_parenting(
            &[("parent", was, now), ("child", child, child)],
            &[("child", "parent")],
        );

        // A quarter turn about the origin takes (10, 0) to (0, 10)...
        assert!(
            after["child"]
                .translation
                .abs_diff_eq(Vec3::new(0.0, 10.0, 0.0), 0.001)
        );
        // ...and turns the child with it.
        assert!(after["child"].rotation.abs_diff_eq(now.rotation, 0.001));
    }

    #[test]
    fn a_chain_carries_the_whole_way_down_in_one_pass() {
        let was = Transform::from_xyz(0.0, 0.0, 0.0);
        let now = Transform::from_xyz(5.0, 0.0, 0.0);
        let middle = Transform::from_xyz(20.0, 0.0, 0.0);
        let leaf = Transform::from_xyz(30.0, 0.0, 0.0);
        let after = after_parenting(
            &[
                ("root", was, now),
                ("middle", middle, middle),
                ("leaf", leaf, leaf),
            ],
            &[("middle", "root"), ("leaf", "middle")],
        );

        assert!((after["middle"].translation.x - 25.0).abs() < 0.001);
        // The leaf gets the root's five once, not twice: the middle is done
        // before it is, and the delta it passes on is measured from its own
        // start-of-step pose.
        assert!((after["leaf"].translation.x - 35.0).abs() < 0.001);
    }

    #[test]
    fn a_child_that_moved_itself_keeps_that_move_and_its_parents() {
        let was = Transform::from_xyz(0.0, 0.0, 0.0);
        let now = Transform::from_xyz(5.0, 0.0, 0.0);
        let child_was = Transform::from_xyz(0.0, 0.0, 0.0);
        let child_now = Transform::from_xyz(0.0, 7.0, 0.0);
        let after = after_parenting(
            &[("parent", was, now), ("child", child_was, child_now)],
            &[("child", "parent")],
        );

        assert!(
            after["child"]
                .translation
                .abs_diff_eq(Vec3::new(5.0, 7.0, 0.0), 0.001)
        );
    }

    #[test]
    fn a_parent_that_did_not_move_leaves_its_child_alone() {
        let still = Transform::from_xyz(3.0, 3.0, 0.0);
        let child = Transform::from_xyz(9.0, 0.0, 0.0);
        let after = after_parenting(
            &[("parent", still, still), ("child", child, child)],
            &[("child", "parent")],
        );

        assert!(
            after["child"]
                .translation
                .abs_diff_eq(child.translation, 0.001)
        );
    }

    // ─── Authored local offsets ─────────────────────────────────────────────

    /// A project of plain actors, each at a placement, with the hierarchy
    /// and offsets the test asks for.
    fn project_of(
        actors: &[(&str, blockloom_core::scene::Placement)],
        parents: &[(&str, &str, Option<[f32; 3]>)],
    ) -> blockloom_core::project::Project {
        let mut project = blockloom_core::project::Project::starter("Offsets", Mode::TwoD);
        project.actors.clear();
        for (id, placement) in actors {
            let mut actor = Actor::new(
                *id,
                Visual::Rect {
                    color: "#fff".to_string(),
                    size: [10.0, 10.0],
                },
            );
            actor.id = (*id).to_string();
            actor.components.set_placement(*placement);
            project.actors.push(actor);
        }
        for (child, parent, offset) in parents {
            let actor = project.actor_mut(child).unwrap();
            actor.components.set_parent(parent);
            actor.components.set_parent_offset(*offset);
        }
        project
    }

    fn at(x: f32, y: f32) -> blockloom_core::scene::Placement {
        blockloom_core::scene::Placement {
            position: [x, y, 0.0],
            ..Default::default()
        }
    }

    fn position_of(project: &blockloom_core::project::Project, id: &str) -> [f32; 3] {
        project.actor(id).unwrap().placement().position
    }

    #[test]
    fn an_offset_child_is_placed_in_its_parents_frame() {
        let mut project = project_of(
            &[("parent", at(100.0, 50.0)), ("child", at(0.0, 0.0))],
            &[("child", "parent", Some([10.0, -5.0, 0.0]))],
        );
        place_authored_children(&mut project);

        assert_eq!(position_of(&project, "child"), [110.0, 45.0, 0.0]);
        assert_eq!(position_of(&project, "parent"), [100.0, 50.0, 0.0]);
    }

    #[test]
    fn a_child_without_an_offset_stays_in_world_coordinates() {
        let mut project = project_of(
            &[("parent", at(100.0, 50.0)), ("child", at(7.0, 7.0))],
            &[("child", "parent", None)],
        );
        place_authored_children(&mut project);

        assert_eq!(position_of(&project, "child"), [7.0, 7.0, 0.0]);
    }

    #[test]
    fn a_turned_parent_turns_the_offset_with_it() {
        let mut project = project_of(
            &[
                (
                    "parent",
                    blockloom_core::scene::Placement {
                        position: [0.0, 0.0, 0.0],
                        rotation: [0.0, 0.0, 90.0],
                        ..Default::default()
                    },
                ),
                ("child", at(0.0, 0.0)),
            ],
            &[("child", "parent", Some([10.0, 0.0, 0.0]))],
        );
        place_authored_children(&mut project);

        // A quarter turn puts ten to the right ten above instead.
        let position = position_of(&project, "child");
        assert!(position[0].abs() < 0.001, "{position:?}");
        assert!((position[1] - 10.0).abs() < 0.001, "{position:?}");
    }

    #[test]
    fn a_placed_child_reads_back_the_offset_it_was_placed_at() {
        let parent = transform_of(&at(100.0, 50.0));
        let child = world_of(&parent, [10.0, -5.0, 0.0]);

        assert_eq!(child.to_array(), [110.0, 45.0, 0.0]);
        // The reporters' read is the inverse of `set my parent to`'s write.
        assert_eq!(local_of(&parent, child), [10.0, -5.0, 0.0]);
    }

    #[test]
    fn a_turned_parent_turns_the_local_reading_with_it() {
        let parent = transform_of(&blockloom_core::scene::Placement {
            position: [0.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 90.0],
            ..Default::default()
        });
        let child = world_of(&parent, [10.0, 0.0, 0.0]);

        // Ten to the right in the parent's frame is ten above in the world.
        assert!(child.x.abs() < 0.001, "{child:?}");
        assert!((child.y - 10.0).abs() < 0.001, "{child:?}");
        let local = local_of(&parent, child);
        assert!((local[0] - 10.0).abs() < 0.001, "{local:?}");
        assert!(local[1].abs() < 0.001, "{local:?}");
    }

    #[test]
    fn navigation_drives_a_dynamic_body_without_teleporting_it() {
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::ThreeD);
        engine.running = true;
        let mut actor = Actor::new(
            "walker",
            Visual::Cuboid {
                color: "#fff".into(),
                size: [1.0; 3],
            },
        );
        actor.id = "walker".into();
        actor
            .components
            .set_physics(blockloom_core::scene::Physics {
                body: BodyKind::Dynamic,
                ..Default::default()
            });
        engine.project.actors = vec![actor];
        let mut app = App::new();
        app.add_message::<AppExit>();
        app.insert_resource(Dimension(Mode::ThreeD));
        app.insert_resource(PendingEffects(vec![Effect::NavigateTo {
            actor: "walker".into(),
            target: [10.0, 0.0, 0.0],
            speed: 4.0,
        }]));
        let entity = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                Visibility::Inherited,
                bevy_rapier3d::prelude::Velocity::zero(),
            ))
            .id();
        engine.entities.insert("walker".into(), entity);
        app.insert_non_send(engine);
        app.add_systems(Update, apply_common);
        app.update();
        let body = app.world().entity(entity);
        assert_eq!(body.get::<Transform>().unwrap().translation, Vec3::ZERO);
        assert!(
            (body
                .get::<bevy_rapier3d::prelude::Velocity>()
                .unwrap()
                .linear
                .x
                - 4.0)
                .abs()
                < 1e-4
        );
    }

    /// A running app with a parent at (100, 50) and a child at (7, 7)
    /// carrying `offset`, with `set my parent to parent` queued for it.
    fn reparent_app(offset: Option<[f32; 3]>) -> App {
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::TwoD);
        engine.running = true;
        // The offset only places the child once something hangs it: at build
        // time that is the authored parent, at run time `set my parent to`.
        engine.project = project_of(
            &[("parent", at(100.0, 50.0)), ("child", at(7.0, 7.0))],
            &[("child", "parent", offset)],
        );
        engine.parents.clear();

        let mut app = App::new();
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<crate::performance::RenderCache>();
        app.init_resource::<crate::performance::StreamingCells>();
        app.init_resource::<Assets<StandardMaterial>>();
        app.init_resource::<Assets<Image>>();
        app.init_resource::<Assets<crate::materials::GraphMaterial2d>>();
        app.init_resource::<Assets<crate::materials::GraphMaterial3d>>();
        app.init_resource::<Assets<bevy::sprite_render::ColorMaterial>>();
        app.insert_resource(Dimension(Mode::TwoD));
        app.insert_resource(PendingEffects(vec![Effect::SetParent {
            actor: "child".to_string(),
            parent: "parent".to_string(),
        }]));
        app.insert_non_send(engine);
        for (id, x, y) in [("parent", 100.0, 50.0), ("child", 7.0, 7.0)] {
            let entity = app
                .world_mut()
                .spawn((
                    ActorId(id.to_string()),
                    Transform::from_xyz(x, y, 0.0),
                    Visibility::Inherited,
                ))
                .id();
            app.world_mut()
                .non_send_mut::<Engine>()
                .entities
                .insert(id.to_string(), entity);
        }
        app.add_systems(Update, apply_lifetimes);
        app
    }

    #[test]
    fn setting_a_parent_at_run_time_places_an_offset_child_in_its_frame() {
        let mut app = reparent_app(Some([10.0, -5.0, 0.0]));
        app.update();

        let engine = app.world().non_send::<Engine>();
        assert_eq!(
            engine.parents.get("child").map(String::as_str),
            Some("parent")
        );
        let child = engine.entities["child"];
        let transform = app.world().entity(child).get::<Transform>().unwrap();
        assert_eq!(transform.translation.to_array(), [110.0, 45.0, 0.0]);
    }

    #[test]
    fn setting_a_parent_at_run_time_leaves_a_child_without_an_offset_where_it_stands() {
        let mut app = reparent_app(None);
        app.update();

        let engine = app.world().non_send::<Engine>();
        assert_eq!(
            engine.parents.get("child").map(String::as_str),
            Some("parent")
        );
        let child = engine.entities["child"];
        let transform = app.world().entity(child).get::<Transform>().unwrap();
        assert_eq!(transform.translation.to_array(), [7.0, 7.0, 0.0]);
    }

    #[test]
    fn an_offset_down_a_chain_is_measured_against_a_parent_already_placed() {
        let mut project = project_of(
            &[
                ("root", at(100.0, 0.0)),
                ("middle", at(0.0, 0.0)),
                ("leaf", at(0.0, 0.0)),
            ],
            &[
                ("middle", "root", Some([10.0, 0.0, 0.0])),
                ("leaf", "middle", Some([1.0, 0.0, 0.0])),
            ],
        );
        place_authored_children(&mut project);

        assert_eq!(position_of(&project, "middle"), [110.0, 0.0, 0.0]);
        assert_eq!(position_of(&project, "leaf"), [111.0, 0.0, 0.0]);
    }

    #[test]
    fn a_3d_yaw_spins_past_ninety_degrees_without_folding() {
        // Two hundred one-degree turns, the way a mouse-look strand
        // drives them. XYZ euler reads the yaw back folded into
        // [-90, 90] past the first quarter turn, which used to wall
        // every 3D game at about 180 degrees of horizontal look.
        let mut transform = Transform::IDENTITY;
        for _ in 0..200 {
            turn_3d(&mut transform, Axis::Y, 1f32.to_radians());
        }

        // 200 degrees wraps to -160, still level, and facing where a
        // full 200-degree yaw faces rather than folded back inside 90.
        let (yaw, pitch, roll) = transform.rotation.to_euler(EulerRot::YXZ);
        assert!((yaw.to_degrees() + 160.0).abs() < 0.5, "{yaw:?}");
        assert!(pitch.abs() < 0.001, "{pitch:?}");
        assert!(roll.abs() < 0.001, "{roll:?}");
        let facing = *transform.forward();
        let expect = Vec3::new(-200f32.to_radians().sin(), 0.0, -200f32.to_radians().cos());
        assert!(facing.dot(expect) > 0.999, "{facing:?} vs {expect:?}");
    }

    #[test]
    fn a_turn_move_turn_sandwich_strafes_a_dynamic_body() {
        use bevy_rapier3d::prelude::{ExternalImpulse, Velocity};

        // How strafe is spelled in blocks: face sideways, step, face back.
        // A dynamic body's `move` is deferred to the dimension pass as a
        // velocity, so turning in the common pass used to net the sandwich
        // to zero first and both strafe keys walked forward instead.
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::ThreeD);
        engine.running = true;
        let mut project = blockloom_core::project::Project::starter("Strafe", Mode::ThreeD);
        project.actors.clear();
        let mut actor = Actor::new(
            "player",
            Visual::Rect {
                color: "#fff".to_string(),
                size: [1.0, 1.0],
            },
        );
        actor.id = "player".to_string();
        actor
            .components
            .set_physics(blockloom_core::scene::Physics {
                body: BodyKind::Dynamic,
                ..Default::default()
            });
        project.actors.push(actor);
        engine.project = project;

        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(Dimension(Mode::ThreeD));
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.init_resource::<Assets<crate::materials::GraphMaterial3d>>();
        app.init_resource::<Assets<StandardMaterial>>();
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<crate::performance::RenderCache>();
        app.init_resource::<crate::performance::StreamingCells>();
        app.insert_resource(PendingEffects(vec![
            Effect::Turn {
                actor: "player".to_string(),
                axis: Axis::Y,
                degrees: 90.0,
            },
            Effect::Move {
                actor: "player".to_string(),
                steps: 0.18,
            },
            Effect::Turn {
                actor: "player".to_string(),
                axis: Axis::Y,
                degrees: -90.0,
            },
        ]));
        app.add_message::<AppExit>();
        let entity = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                Visibility::Inherited,
                Velocity::default(),
                ExternalImpulse::default(),
            ))
            .id();
        app.world_mut().insert_non_send(engine);
        app.world_mut()
            .non_send_mut::<Engine>()
            .entities
            .insert("player".to_string(), entity);
        app.add_systems(Update, (apply_common, dim3::apply_effects).chain());
        app.update();

        // Sideways velocity, not forward: facing -Z, a +90 yaw looks down
        // -X, and the sandwich faces back afterwards.
        let velocity = app
            .world()
            .entity(entity)
            .get::<Velocity>()
            .expect("the body");
        assert!(velocity.linear.x < -100.0, "{:?}", velocity.linear);
        assert!(velocity.linear.z.abs() < 1.0, "{:?}", velocity.linear);
        assert_eq!(velocity.linear.y, 0.0);
        let facing = app.world().entity(entity).get::<Transform>().expect("pose");
        assert!(
            facing.rotation.angle_between(Quat::IDENTITY) < 0.01,
            "{:?}",
            facing.rotation
        );
    }

    #[test]
    fn holding_forward_and_strafe_walks_diagonal() {
        use bevy_rapier3d::prelude::{ExternalImpulse, Velocity};

        // One tick of W+A in the First Person game: W's forward step and
        // A's turn-step-turn-back sandwich. Each deferred move writes only
        // its nonzero axes, so both survive into the velocity.
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::ThreeD);
        engine.running = true;
        let mut project = blockloom_core::project::Project::starter("Diagonal", Mode::ThreeD);
        project.actors.clear();
        let mut actor = Actor::new(
            "player",
            Visual::Rect {
                color: "#fff".to_string(),
                size: [1.0, 1.0],
            },
        );
        actor.id = "player".to_string();
        actor
            .components
            .set_physics(blockloom_core::scene::Physics {
                body: BodyKind::Dynamic,
                ..Default::default()
            });
        project.actors.push(actor);
        engine.project = project;

        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(Dimension(Mode::ThreeD));
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.init_resource::<Assets<crate::materials::GraphMaterial3d>>();
        app.init_resource::<Assets<StandardMaterial>>();
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<crate::performance::RenderCache>();
        app.init_resource::<crate::performance::StreamingCells>();
        let walk = |steps: f32| Effect::Move {
            actor: "player".to_string(),
            steps,
        };
        let level = Effect::SetRotation {
            actor: "player".to_string(),
            axis: Axis::X,
            degrees: 0.0,
        };
        let sideways = |degrees: f32| Effect::Turn {
            actor: "player".to_string(),
            axis: Axis::Y,
            degrees,
        };
        app.insert_resource(PendingEffects(vec![
            level.clone(),
            walk(0.22),
            level.clone(),
            sideways(90.0),
            walk(0.18),
            sideways(-90.0),
        ]));
        app.add_message::<AppExit>();
        let entity = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                Visibility::Inherited,
                Velocity::default(),
                ExternalImpulse::default(),
            ))
            .id();
        app.world_mut().insert_non_send(engine);
        app.world_mut()
            .non_send_mut::<Engine>()
            .entities
            .insert("player".to_string(), entity);
        app.add_systems(Update, (apply_common, dim3::apply_effects).chain());
        app.update();

        // Forward (-Z) from W and left (-X) from A, both present.
        let velocity = app
            .world()
            .entity(entity)
            .get::<Velocity>()
            .expect("walker")
            .linear;
        assert!(velocity.x < -100.0, "{velocity:?}");
        assert!(velocity.z < -100.0, "{velocity:?}");
        assert_eq!(velocity.y, 0.0);
    }

    #[test]
    fn holding_forward_and_strafe_walks_diagonal_after_a_real_turn_history() {
        use bevy_rapier3d::prelude::{ExternalImpulse, Velocity};

        // The live-game shape of W+A: the yaw came from ninety small mouse
        // turns, so the quaternion is inexact the way played yaw always is.
        // A turned facing reads back with float dust on its true-zero axes,
        // and each deferred move writing straight through let that dust -
        // past epsilon - clobber the other move's axis: W+A walked purely
        // sideways. Composing one tick's walks first keeps both.
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::ThreeD);
        engine.running = true;
        let mut project = blockloom_core::project::Project::starter("Turned", Mode::ThreeD);
        project.actors.clear();
        let mut actor = Actor::new(
            "player",
            Visual::Rect {
                color: "#fff".to_string(),
                size: [1.0, 1.0],
            },
        );
        actor.id = "player".to_string();
        actor
            .components
            .set_physics(blockloom_core::scene::Physics {
                body: BodyKind::Dynamic,
                ..Default::default()
            });
        project.actors.push(actor);
        engine.project = project;

        let mut start = Transform::IDENTITY;
        for _ in 0..90 {
            turn_3d(&mut start, Axis::Y, 1f32.to_radians());
        }
        let (yaw, _, _) = start.rotation.to_euler(EulerRot::YXZ);
        assert!((yaw.to_degrees() - 90.0).abs() < 1.0, "{yaw:?}");

        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(Dimension(Mode::ThreeD));
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.init_resource::<Assets<crate::materials::GraphMaterial3d>>();
        app.init_resource::<Assets<StandardMaterial>>();
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<crate::performance::RenderCache>();
        app.init_resource::<crate::performance::StreamingCells>();
        let walk = |steps: f32| Effect::Move {
            actor: "player".to_string(),
            steps,
        };
        let level = Effect::SetRotation {
            actor: "player".to_string(),
            axis: Axis::X,
            degrees: 0.0,
        };
        let sideways = |degrees: f32| Effect::Turn {
            actor: "player".to_string(),
            axis: Axis::Y,
            degrees,
        };
        app.insert_resource(PendingEffects(vec![
            level.clone(),
            walk(0.22),
            level.clone(),
            sideways(90.0),
            walk(0.18),
            sideways(-90.0),
        ]));
        app.add_message::<AppExit>();
        let entity = app
            .world_mut()
            .spawn((
                start,
                Visibility::Inherited,
                Velocity::default(),
                ExternalImpulse::default(),
            ))
            .id();
        app.world_mut().insert_non_send(engine);
        app.world_mut()
            .non_send_mut::<Engine>()
            .entities
            .insert("player".to_string(), entity);
        app.add_systems(Update, (apply_common, dim3::apply_effects).chain());
        app.update();

        // Facing ~90 degrees: W runs down -X, the sandwich steps up +Z.
        let velocity = app
            .world()
            .entity(entity)
            .get::<Velocity>()
            .expect("walker")
            .linear;
        assert!(velocity.x < -100.0, "{velocity:?}");
        assert!(velocity.z > 100.0, "{velocity:?}");
    }

    #[test]
    fn opposing_walks_cancel_instead_of_cruising_stale() {
        use bevy_rapier3d::prelude::{ExternalImpulse, Velocity};

        // W+S in one tick name Z with equal and opposite steps: the axis
        // is genuinely walked, so it writes zero and stops rather than
        // keeping whatever the last tick left there. Unwalked axes cruise.
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::ThreeD);
        engine.running = true;
        let mut project = blockloom_core::project::Project::starter("Cancel", Mode::ThreeD);
        project.actors.clear();
        let mut actor = Actor::new(
            "player",
            Visual::Rect {
                color: "#fff".to_string(),
                size: [1.0, 1.0],
            },
        );
        actor.id = "player".to_string();
        actor
            .components
            .set_physics(blockloom_core::scene::Physics {
                body: BodyKind::Dynamic,
                ..Default::default()
            });
        project.actors.push(actor);
        engine.project = project;

        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(Dimension(Mode::ThreeD));
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.init_resource::<Assets<crate::materials::GraphMaterial3d>>();
        app.init_resource::<Assets<StandardMaterial>>();
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<crate::performance::RenderCache>();
        app.init_resource::<crate::performance::StreamingCells>();
        app.insert_resource(PendingEffects(vec![
            Effect::Move {
                actor: "player".to_string(),
                steps: 0.22,
            },
            Effect::Move {
                actor: "player".to_string(),
                steps: -0.22,
            },
        ]));
        app.add_message::<AppExit>();
        let entity = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                Visibility::Inherited,
                Velocity::default(),
                ExternalImpulse::default(),
            ))
            .id();
        app.world_mut().insert_non_send(engine);
        app.world_mut()
            .non_send_mut::<Engine>()
            .entities
            .insert("player".to_string(), entity);
        app.world_mut()
            .entity_mut(entity)
            .get_mut::<Velocity>()
            .expect("walker")
            .linear = Vec3::new(9.0, -2.0, 7.0);
        app.add_systems(Update, (apply_common, dim3::apply_effects).chain());
        app.update();

        let velocity = app
            .world()
            .entity(entity)
            .get::<Velocity>()
            .expect("walker")
            .linear;
        assert_eq!(velocity, Vec3::new(9.0, -2.0, 0.0));
    }

    #[test]
    fn a_released_walk_brakes_but_leaves_falls_and_flying_balls_alone() {
        use bevy_rapier3d::prelude::{ExternalImpulse, Velocity};

        // A walk writes an absolute velocity every tick it runs, so the
        // tick after the strand goes quiet that speed used to glide on.
        // Braking only ever touches walk-driven actors, on every axis but
        // gravity's: falls, collisions and impulses keep their inertia.
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::ThreeD);
        engine.running = true;
        let mut project = blockloom_core::project::Project::starter("Brakes", Mode::ThreeD);
        project.actors.clear();
        for id in ["player", "ball"] {
            let mut actor = Actor::new(
                id,
                Visual::Rect {
                    color: "#fff".to_string(),
                    size: [1.0, 1.0],
                },
            );
            actor.id = id.to_string();
            actor
                .components
                .set_physics(blockloom_core::scene::Physics {
                    body: BodyKind::Dynamic,
                    ..Default::default()
                });
            project.actors.push(actor);
        }
        engine.project = project;

        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(Dimension(Mode::ThreeD));
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.init_resource::<Assets<crate::materials::GraphMaterial3d>>();
        app.init_resource::<Assets<StandardMaterial>>();
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<crate::performance::RenderCache>();
        app.init_resource::<crate::performance::StreamingCells>();
        app.insert_resource(PendingEffects(vec![
            Effect::Turn {
                actor: "player".to_string(),
                axis: Axis::Y,
                degrees: 90.0,
            },
            Effect::Move {
                actor: "player".to_string(),
                steps: 0.18,
            },
            Effect::Turn {
                actor: "player".to_string(),
                axis: Axis::Y,
                degrees: -90.0,
            },
        ]));
        app.add_message::<AppExit>();
        let player = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                Visibility::Inherited,
                Velocity::default(),
                ExternalImpulse::default(),
            ))
            .id();
        let ball = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                Visibility::Inherited,
                Velocity::default(),
                ExternalImpulse::default(),
            ))
            .id();
        app.world_mut().insert_non_send(engine);
        {
            let mut engine = app.world_mut().non_send_mut::<Engine>();
            engine.entities.insert("player".to_string(), player);
            engine.entities.insert("ball".to_string(), ball);
        }
        // A mid-fall walker and a steadily rolling ball.
        app.world_mut()
            .entity_mut(player)
            .get_mut::<Velocity>()
            .expect("walker")
            .linear
            .y = -2.0;
        app.world_mut()
            .entity_mut(ball)
            .get_mut::<Velocity>()
            .expect("ball")
            .linear = Vec3::new(5.0, -1.0, -3.0);
        app.add_systems(Update, (apply_common, dim3::apply_effects).chain());

        // Tick one: the sandwich strafes, and the fall is untouched.
        app.update();
        let walked = app
            .world()
            .entity(player)
            .get::<Velocity>()
            .expect("walker")
            .linear;
        assert!(walked.x < -100.0, "{walked:?}");
        assert_eq!(walked.y, -2.0);

        // Tick two: silence. The walker stops dead but keeps falling; the
        // never-walk-driven ball rolls on untouched.
        app.world_mut().resource_mut::<PendingEffects>().0.clear();
        app.update();
        let stopped = app
            .world()
            .entity(player)
            .get::<Velocity>()
            .expect("walker")
            .linear;
        assert_eq!(stopped, Vec3::new(0.0, -2.0, 0.0));
        let rolling = app
            .world()
            .entity(ball)
            .get::<Velocity>()
            .expect("ball")
            .linear;
        assert_eq!(rolling, Vec3::new(5.0, -1.0, -3.0));

        // An explicit impulse is a physics verb, not a walk: it persists
        // through the quiet ticks exactly like a collision would.
        app.world_mut()
            .resource_mut::<PendingEffects>()
            .0
            .push(Effect::ApplyImpulse {
                actor: "ball".to_string(),
                impulse: [1.0, 0.0, 0.0],
            });
        app.update();
        app.world_mut().resource_mut::<PendingEffects>().0.clear();
        app.update();
        let thrown = app
            .world()
            .entity(ball)
            .get::<Velocity>()
            .expect("ball")
            .linear;
        assert_eq!(thrown, Vec3::new(6.0, -1.0, -3.0));
    }

    #[test]
    fn zeroing_pitch_and_roll_leaves_a_far_3d_yaw_alone() {
        // The game's anti-capsize strand zeroes X and Z every tick.
        // Through XYZ euler that snaps a yaw past 90 back inside it;
        // through YXZ it is an identity write that changes nothing.
        let mut transform = Transform::IDENTITY;
        turn_3d(&mut transform, Axis::Y, (-130f32).to_radians());
        set_rotation_3d(&mut transform, Axis::X, 0.0);
        set_rotation_3d(&mut transform, Axis::Z, 0.0);

        let (yaw, pitch, roll) = transform.rotation.to_euler(EulerRot::YXZ);
        assert!((yaw.to_degrees() + 130.0).abs() < 0.5, "{yaw:?}");
        assert!(pitch.abs() < 0.001, "{pitch:?}");
        assert!(roll.abs() < 0.001, "{roll:?}");
    }

    // ─── Routing a click through the interface ─────────────────────────────

    /// One element, drawn where a layout pass would have put it: a box of
    /// `size` centred on `centre`. That rectangle is all the hit test reads.
    fn drawn_element(app: &mut App, spec: blockloom_core::ui::UiElement, centre: Vec2, size: Vec2) {
        let entity = app
            .world_mut()
            .spawn((
                ComputedNode {
                    size,
                    ..Default::default()
                },
                UiGlobalTransform::from_translation(centre),
            ))
            .id();
        let mut manager = app.world_mut().resource_mut::<crate::ui::UiManager>();
        let id = spec.id.clone();
        manager.show(spec);
        manager.take_pending();
        manager.attach(&id, entity);
    }

    fn element(id: &str, kind: blockloom_core::ui::UiKind) -> blockloom_core::ui::UiElement {
        blockloom_core::ui::UiElement {
            id: id.to_string(),
            kind,
            ..Default::default()
        }
    }

    /// An app with one clickable world actor at the origin whose canvas says
    /// which route a click took, a pointer parked at `cursor`, and the left
    /// button just pressed.
    fn click_harness(cursor: Vec2) -> App {
        use blockloom_core::blocks::{Instruction, InstructionKind as K, Strand};

        let headed = |header: K, text: &str| {
            Strand::with_instructions(
                0,
                0,
                vec![
                    Instruction::new(header),
                    Instruction::new(K::Say {
                        text: blockloom_core::value::Value::text(text),
                    }),
                ],
            )
        };
        let mut actor = Actor::new(
            "Target",
            Visual::Rect {
                color: "#FFFFFF".to_string(),
                size: [400.0, 400.0],
            },
        );
        actor.id = "a1".to_string();
        actor.graph.strands = vec![
            headed(K::WhenClicked, "world"),
            headed(
                K::WhenUiClicked {
                    element: "resume".to_string(),
                },
                "ui",
            ),
            headed(
                K::WhenUiChanged {
                    element: "shadows".to_string(),
                },
                "changed",
            ),
            headed(
                K::WhenUiChanged {
                    element: "volume".to_string(),
                },
                "moved",
            ),
        ];

        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::TwoD);
        engine.project.actors = vec![actor];
        let project = engine.project.clone();
        engine.vm.load(&project);
        engine.running = true;
        engine.window_focused = true;

        let mut buttons = ButtonInput::<MouseButton>::default();
        buttons.press(MouseButton::Left);

        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(Dimension(Mode::TwoD));
        app.insert_resource(buttons);
        app.init_resource::<crate::ui::UiManager>();

        let entity = app
            .world_mut()
            .spawn((ActorId("a1".to_string()), Transform::default()))
            .id();
        engine.entities.insert("a1".to_string(), entity);
        app.insert_non_send(engine);

        let mut window = Window::default();
        window.set_physical_cursor_position(Some(bevy::math::DVec2::new(
            cursor.x as f64,
            cursor.y as f64,
        )));
        app.world_mut().spawn((window, PrimaryWindow));
        // A camera the world pick projects through, parked so the origin is
        // under the middle of the window.
        app.world_mut().spawn((
            Camera2d,
            Camera::default(),
            Transform::default(),
            GlobalTransform::default(),
            WorldCamera,
        ));
        app.add_systems(Update, detect_clicks);
        app
    }

    /// Routes the click, then gives the VM one tick and answers with what
    /// the strands the click started had to say.
    fn routed(app: &mut App) -> Vec<String> {
        app.update();
        let mut effects = Vec::new();
        let mut engine = app.world_mut().non_send_mut::<Engine>();
        let paused = engine.paused;
        engine.vm.set_paused(paused);
        engine.vm.tick(0.0, &mut effects);
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Say { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_click_on_an_element_starts_its_strand_and_never_reaches_the_world() {
        let mut app = click_harness(Vec2::new(100.0, 100.0));
        drawn_element(
            &mut app,
            element("resume", blockloom_core::ui::UiKind::Button),
            Vec2::new(100.0, 100.0),
            Vec2::new(80.0, 30.0),
        );
        assert_eq!(routed(&mut app), vec!["ui".to_string()]);
    }

    /// Whether the world would be offered the click this app just routed.
    /// The pick itself needs a render target, which a headless app has none
    /// of, so the rule is checked where it is decided.
    fn world_offered(app: &mut App) -> bool {
        app.update();
        let engine = app.world().non_send::<Engine>();
        let manager = app.world().resource::<crate::ui::UiManager>();
        world_takes_clicks(engine, manager)
    }

    #[test]
    fn a_click_that_misses_every_element_is_offered_to_the_world() {
        let mut app = click_harness(Vec2::new(480.0, 360.0));
        drawn_element(
            &mut app,
            element("resume", blockloom_core::ui::UiKind::Button),
            Vec2::new(100.0, 100.0),
            Vec2::new(80.0, 30.0),
        );
        // Nothing in the interface answered, and nothing is swallowing.
        assert!(routed(&mut app).is_empty());
        assert!(world_offered(&mut app));
    }

    #[test]
    fn a_click_beside_a_modal_panel_is_swallowed_rather_than_hitting_the_world() {
        let mut app = click_harness(Vec2::new(480.0, 360.0));
        let mut menu = element("menu", blockloom_core::ui::UiKind::Panel);
        menu.modal = true;
        // Drawn somewhere the pointer isn't: the swallow is about the panel
        // being up, not about what the click landed on.
        drawn_element(&mut app, menu, Vec2::new(10.0, 10.0), Vec2::new(20.0, 20.0));
        assert!(routed(&mut app).is_empty());
        assert!(!world_offered(&mut app));
    }

    #[test]
    fn the_same_click_reaches_the_world_once_the_panel_is_hidden() {
        let mut app = click_harness(Vec2::new(480.0, 360.0));
        let mut menu = element("menu", blockloom_core::ui::UiKind::Panel);
        menu.modal = true;
        drawn_element(&mut app, menu, Vec2::new(10.0, 10.0), Vec2::new(20.0, 20.0));
        app.world_mut()
            .resource_mut::<crate::ui::UiManager>()
            .hide("menu", false);
        assert!(world_offered(&mut app));
    }

    #[test]
    fn a_world_click_is_dead_while_paused_but_an_element_still_answers() {
        let mut app = click_harness(Vec2::new(100.0, 100.0));
        drawn_element(
            &mut app,
            element("resume", blockloom_core::ui::UiKind::Button),
            Vec2::new(100.0, 100.0),
            Vec2::new(80.0, 30.0),
        );
        app.world_mut().non_send_mut::<Engine>().paused = true;
        // The strand a UI click started runs on, frozen world or not.
        assert_eq!(routed(&mut app), vec!["ui".to_string()]);

        let mut beside = click_harness(Vec2::new(480.0, 360.0));
        beside.world_mut().non_send_mut::<Engine>().paused = true;
        assert!(routed(&mut beside).is_empty());
        assert!(!world_offered(&mut beside));
    }

    #[test]
    fn clicking_a_text_input_hands_it_the_keyboard_and_clicking_away_takes_it_back() {
        let mut app = click_harness(Vec2::new(100.0, 100.0));
        drawn_element(
            &mut app,
            element("name", blockloom_core::ui::UiKind::Input),
            Vec2::new(100.0, 100.0),
            Vec2::new(180.0, 30.0),
        );
        app.update();
        assert_eq!(
            app.world().resource::<crate::ui::UiManager>().focus(),
            Some("name")
        );

        // The pointer moves off it; the next click releases the keyboard.
        let mut windows = app.world_mut().query::<&mut Window>();
        for mut window in windows.iter_mut(app.world_mut()) {
            window.set_physical_cursor_position(Some(bevy::math::DVec2::new(500.0, 400.0)));
        }
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        assert_eq!(app.world().resource::<crate::ui::UiManager>().focus(), None);
    }

    #[test]
    fn clicking_a_toggle_turns_it_over_and_reports_the_change() {
        let mut app = click_harness(Vec2::new(100.0, 100.0));
        let mut toggle = element("shadows", blockloom_core::ui::UiKind::Toggle);
        toggle.value = blockloom_core::value::Evaluated::Bool(false);
        drawn_element(
            &mut app,
            toggle,
            Vec2::new(100.0, 100.0),
            Vec2::new(120.0, 30.0),
        );
        // The click and the change both land, in that order.
        assert_eq!(routed(&mut app), vec!["changed".to_string()]);
        assert_eq!(
            app.world()
                .resource::<crate::ui::UiManager>()
                .get("shadows")
                .map(|node| node.value.clone()),
            Some(blockloom_core::value::Evaluated::Bool(true))
        );
    }

    #[test]
    fn dragging_a_slider_lands_on_the_number_its_own_ends_name() {
        // Two thirds of the way across a 90-wide track centred on 100.
        let mut app = click_harness(Vec2::new(115.0, 100.0));
        let mut slider = element("volume", blockloom_core::ui::UiKind::Slider);
        slider.range = [0.0, 90.0];
        slider.value = blockloom_core::value::Evaluated::Number(0.0);
        drawn_element(
            &mut app,
            slider,
            Vec2::new(100.0, 100.0),
            Vec2::new(90.0, 20.0),
        );
        assert_eq!(routed(&mut app), vec!["moved".to_string()]);

        let at = app
            .world()
            .resource::<crate::ui::UiManager>()
            .get("volume")
            .and_then(|node| node.value.as_number().ok());
        assert_eq!(at, Some(60.0));
    }

    #[test]
    fn a_slider_click_reaches_the_sensor_snapshot_the_same_frame() {
        // Two thirds of the way across a 90-wide track centred on 100. A
        // `when input changed` strand reads the new number through the
        // snapshot, so the snapshot has to hold 60 - the click's own value -
        // rather than the 0 it just moved away from. Input lands before
        // publishing: the chain below mirrors main.rs, where
        // `publish_sensors` follows `detect_clicks`.
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::TwoD);
        engine.running = true;
        engine.window_focused = true;

        let mut buttons = ButtonInput::<MouseButton>::default();
        buttons.press(MouseButton::Left);

        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(Dimension(Mode::TwoD));
        app.insert_resource(buttons);
        app.init_resource::<ButtonInput<KeyCode>>();
        app.init_resource::<Messages<MouseMotion>>();
        app.init_resource::<Messages<WindowFocused>>();
        app.init_resource::<crate::ui::UiManager>();
        app.init_resource::<crate::sound::SoundState>();
        app.init_resource::<bevy::input::touch::Touches>();
        app.insert_non_send(engine);

        let mut window = Window::default();
        window.set_physical_cursor_position(Some(bevy::math::DVec2::new(115.0, 100.0)));
        app.world_mut().spawn((window, PrimaryWindow));

        let mut slider = element("volume", blockloom_core::ui::UiKind::Slider);
        slider.range = [0.0, 90.0];
        slider.value = blockloom_core::value::Evaluated::Number(0.0);
        drawn_element(
            &mut app,
            slider,
            Vec2::new(100.0, 100.0),
            Vec2::new(90.0, 20.0),
        );
        app.add_systems(Update, (detect_clicks, publish_sensors).chain());
        app.update();

        let seen = blockloom_core::sense::read(|sensors| {
            sensors.ui.get("volume").map(|sense| sense.value.clone())
        });
        assert_eq!(seen.and_then(|value| value.as_number().ok()), Some(60.0));
    }

    /// The whole menu stack with nothing mocked: the blocks show a modal
    /// panel with a parented button, real Bevy layout draws it, and a click
    /// on the button's own drawn box starts its strand while paused.
    #[test]
    fn a_drawn_menu_button_answers_a_click_on_its_own_box() {
        use bevy::asset::AssetPlugin;
        use bevy::text::TextPlugin;
        use bevy::time::TimePlugin;
        use bevy::ui::UiPlugin;
        use blockloom_core::blocks::{Instruction, InstructionKind as K, Strand};
        use blockloom_core::ui::{UiAnchor, UiElement, UiKind};
        use blockloom_core::value::{Evaluated, Value};

        let mut actor = Actor::new(
            "Player",
            Visual::Rect {
                color: "#fff".to_string(),
                size: [10.0, 10.0],
            },
        );
        actor.id = "a1".to_string();
        actor.graph.strands = vec![Strand::with_instructions(
            0,
            0,
            vec![
                Instruction::new(K::WhenUiClicked {
                    element: "resume_btn".to_string(),
                }),
                Instruction::new(K::Say {
                    text: Value::text("ui"),
                }),
            ],
        )];

        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::TwoD);
        engine.project.actors = vec![actor];
        let project = engine.project.clone();
        engine.vm.load(&project);
        engine.running = true;
        engine.paused = true;
        engine.vm.set_paused(true);
        engine.window_focused = true;

        let mut manager = crate::ui::UiManager::default();
        manager.show(UiElement {
            id: "pause_menu".to_string(),
            kind: UiKind::Panel,
            content: "Paused".to_string(),
            anchor: UiAnchor::Center,
            offset: [0.0, 0.0],
            size: [320.0, 0.0],
            parent: String::new(),
            modal: true,
            range: [0.0, 100.0],
            value: Evaluated::Text(String::new()),
        });
        manager.show(UiElement {
            id: "resume_btn".to_string(),
            kind: UiKind::Button,
            content: "Resume".to_string(),
            anchor: UiAnchor::Center,
            offset: [0.0, 0.0],
            size: [0.0, 0.0],
            parent: "pause_menu".to_string(),
            modal: false,
            range: [0.0, 100.0],
            value: Evaluated::Bool(false),
        });

        let mut app = App::new();
        app.add_plugins((AssetPlugin::default(), TimePlugin, TextPlugin, UiPlugin));
        // Bevy's own viewport picking runs but has nothing to say here; it
        // only needs its two maps and its messages to exist.
        app.init_resource::<bevy::picking::hover::HoverMap>();
        app.init_resource::<bevy::picking::events::PointerState>();
        app.add_message::<bevy::picking::pointer::PointerInput>();
        app.add_message::<bevy::picking::backend::PointerHits>();
        app.init_asset::<bevy::image::Image>();
        app.init_asset::<bevy::image::TextureAtlasLayout>();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(Dimension(Mode::TwoD));
        app.init_resource::<ButtonInput<MouseButton>>();
        app.init_resource::<bevy::input::touch::Touches>();
        app.insert_non_send(engine);
        app.insert_resource(manager);
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app.world_mut().spawn((
            Camera2d,
            Camera::default(),
            Transform::default(),
            GlobalTransform::default(),
            WorldCamera,
        ));
        app.add_systems(Update, (crate::overlay::draw_ui, detect_clicks).chain());

        // Spawn, then let layout measure the button's real box.
        for _ in 0..5 {
            app.update();
        }
        let centre: Vec2 = {
            let entity = app
                .world()
                .resource::<crate::ui::UiManager>()
                .get("resume_btn")
                .expect("the button was drawn")
                .entity;
            let world = app.world_mut();
            let mut laid_out = world.query::<(&ComputedNode, &UiGlobalTransform)>();
            let (node, transform) = laid_out.get(world, entity).expect("a laid-out box");
            assert!(
                node.size().x > 0.0 && node.size().y > 0.0,
                "the button drew at no size: {node:?}"
            );
            transform.translation
        };

        let mut windows = app.world_mut().query::<&mut Window>();
        for mut window in windows.iter_mut(app.world_mut()) {
            window.set_physical_cursor_position(Some(bevy::math::DVec2::new(
                centre.x as f64,
                centre.y as f64,
            )));
        }
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();

        let mut effects = Vec::new();
        {
            let mut engine = app.world_mut().non_send_mut::<Engine>();
            engine.vm.tick(0.0, &mut effects);
        }
        let said: Vec<String> = effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Say { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(said, vec!["ui".to_string()]);
    }

    /// A click on a high-DPI display: layout boxes are physical pixels while
    /// the cursor reads logical ones, so the router has to scale the pointer
    /// up the way Bevy's own picking does. The box below is the live report
    /// that found this: a 200% menu the clicks fell straight through.
    #[test]
    fn a_click_at_double_scale_lands_on_the_physical_box() {
        // A 1920x1440 window at 200%: logical space is 960x720.
        let mut app = click_harness(Vec2::new(480.0, 360.0));
        drawn_element(
            &mut app,
            element("resume", blockloom_core::ui::UiKind::Button),
            Vec2::new(960.0, 720.0),
            Vec2::new(592.0, 72.0),
        );
        // The physical point above reads as half in logical pixels - which
        // is what the router is handed.
        let mut windows = app.world_mut().query::<&mut Window>();
        for mut window in windows.iter_mut(app.world_mut()) {
            window.resolution =
                bevy::window::WindowResolution::new(1920, 1440).with_scale_factor_override(2.0);
            // The OS cursor sits over the rendered button, in raw physical
            // pixels like a real pointer would.
            window.set_physical_cursor_position(Some(bevy::math::DVec2::new(960.0, 720.0)));
        }
        app.world_mut().non_send_mut::<Engine>().paused = true;
        assert_eq!(routed(&mut app), vec!["ui".to_string()]);
    }

    /// An app with one text input holding the keyboard, ready to be typed
    /// into, and an actor whose canvas says when that input changed. The
    /// element carries whatever `rules` writes to it.
    fn typing_harness(rules: impl FnOnce(&mut crate::ui::UiManager)) -> App {
        use blockloom_core::blocks::{Instruction, InstructionKind as K, Strand};

        let mut actor = Actor::new(
            "Target",
            Visual::Rect {
                color: "#FFFFFF".to_string(),
                size: [10.0, 10.0],
            },
        );
        actor.id = "a1".to_string();
        actor.graph.strands = vec![Strand::with_instructions(
            0,
            0,
            vec![
                Instruction::new(K::WhenUiChanged {
                    element: "name".to_string(),
                }),
                Instruction::new(K::Say {
                    text: blockloom_core::value::Value::text("typed"),
                }),
            ],
        )];

        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::TwoD);
        engine.project.actors = vec![actor];
        let project = engine.project.clone();
        engine.vm.load(&project);
        engine.running = true;
        engine.window_focused = true;

        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(Dimension(Mode::TwoD));
        app.init_resource::<Messages<KeyboardInput>>();
        app.init_resource::<crate::ui::UiManager>();
        app.insert_non_send(engine);
        app.add_systems(Update, type_into_focused_input);

        let mut manager = app.world_mut().resource_mut::<crate::ui::UiManager>();
        manager.show(element("name", blockloom_core::ui::UiKind::Input));
        manager.take_pending();
        manager.focus_on(Some("name"));
        rules(&mut manager);
        app
    }

    /// Queues one keypress per character, as a keyboard would.
    fn keys_in(app: &mut App, word: &str) {
        for ch in word.chars() {
            let key = if ch == ' ' {
                Key::Space
            } else {
                Key::Character(ch.to_string().into())
            };
            app.world_mut()
                .resource_mut::<Messages<KeyboardInput>>()
                .write(KeyboardInput {
                    key_code: KeyCode::KeyA,
                    logical_key: key,
                    state: ButtonState::Pressed,
                    text: None,
                    repeat: false,
                    window: Entity::PLACEHOLDER,
                });
        }
    }

    fn type_word(app: &mut App, word: &str) {
        keys_in(app, word);
        app.update();
    }

    fn typed_text(app: &App) -> String {
        app.world()
            .resource::<crate::ui::UiManager>()
            .get("name")
            .map(|node| node.value.as_text())
            .unwrap_or_default()
    }

    #[test]
    fn an_input_takes_every_character_until_its_rules_say_otherwise() {
        let mut app = typing_harness(|_| {});
        type_word(&mut app, "Ada 7!");
        assert_eq!(typed_text(&app), "Ada 7!");
    }

    #[test]
    fn a_numeric_input_turns_away_what_is_not_a_number() {
        let mut app = typing_harness(|manager| {
            manager.set(
                "name",
                blockloom_core::ui::UiProp::Allow,
                &Evaluated::Text("numbers".to_string()),
            );
        });
        // The letters are simply not there: a refused keystroke rubs itself
        // out rather than reporting, since a person holding a key down
        // means no harm by it.
        type_word(&mut app, "-12a.5b");
        assert_eq!(typed_text(&app), "-12.5");
    }

    #[test]
    fn a_full_input_takes_no_more_however_long_the_key_is_held() {
        let mut app = typing_harness(|manager| {
            manager.set(
                "name",
                blockloom_core::ui::UiProp::MaxLength,
                &Evaluated::Number(3.0),
            );
        });
        type_word(&mut app, "Adamant");
        assert_eq!(typed_text(&app), "Ada");

        // Rubbing one out makes room for exactly one more.
        app.world_mut()
            .resource_mut::<Messages<KeyboardInput>>()
            .write(KeyboardInput {
                key_code: KeyCode::Backspace,
                logical_key: Key::Backspace,
                state: ButtonState::Pressed,
                text: None,
                repeat: false,
                window: Entity::PLACEHOLDER,
            });
        app.update();
        type_word(&mut app, "ze");
        assert_eq!(typed_text(&app), "Adz");
    }

    #[test]
    fn a_keystroke_an_input_took_starts_its_changed_strand() {
        let mut app = typing_harness(|_| {});
        keys_in(&mut app, "h");
        assert_eq!(routed(&mut app), vec!["typed".to_string()]);
    }

    #[test]
    fn a_keystroke_an_input_refused_changes_nothing_and_starts_nothing() {
        let mut app = typing_harness(|manager| {
            manager.set(
                "name",
                blockloom_core::ui::UiProp::Allow,
                &Evaluated::Text("digits".to_string()),
            );
        });
        keys_in(&mut app, "h");
        assert!(routed(&mut app).is_empty());
        assert_eq!(typed_text(&app), "");
    }

    #[test]
    fn the_focus_block_hands_the_keyboard_over_without_a_click() {
        let mut app = typing_harness(|manager| {
            manager.show(element("other", blockloom_core::ui::UiKind::Input));
            manager.take_pending();
        });
        app.init_resource::<PendingEffects>();
        app.add_systems(Update, crate::overlay::apply_ui_effects);

        app.world_mut().resource_mut::<PendingEffects>().0 = vec![Effect::SetFocus {
            id: "other".to_string(),
        }];
        app.update();
        assert_eq!(
            app.world().resource::<crate::ui::UiManager>().focus(),
            Some("other")
        );

        // `clear focus` is the same effect naming nobody.
        app.world_mut().resource_mut::<PendingEffects>().0 =
            vec![Effect::SetFocus { id: String::new() }];
        app.update();
        assert_eq!(app.world().resource::<crate::ui::UiManager>().focus(), None);
    }

    #[test]
    fn dragging_a_slider_with_a_step_lands_on_one_of_its_stops() {
        // Two thirds of the way across a 90-wide track centred on 100.
        let mut app = click_harness(Vec2::new(115.0, 100.0));
        let mut slider = element("volume", blockloom_core::ui::UiKind::Slider);
        slider.range = [0.0, 90.0];
        slider.value = Evaluated::Number(0.0);
        drawn_element(
            &mut app,
            slider,
            Vec2::new(100.0, 100.0),
            Vec2::new(90.0, 20.0),
        );
        app.world_mut().resource_mut::<crate::ui::UiManager>().set(
            "volume",
            blockloom_core::ui::UiProp::Step,
            &Evaluated::Number(45.0),
        );
        assert_eq!(routed(&mut app), vec!["moved".to_string()]);

        // 60 along the track, rounded to the nearest stop.
        let at = app
            .world()
            .resource::<crate::ui::UiManager>()
            .get("volume")
            .and_then(|node| node.value.as_number().ok());
        assert_eq!(at, Some(45.0));
    }

    #[test]
    fn a_focused_input_keeps_every_key_from_the_game() {
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::TwoD);
        engine.running = true;
        engine.window_focused = true;

        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::KeyW);

        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(Dimension(Mode::TwoD));
        app.insert_resource(keys);
        app.init_resource::<ButtonInput<MouseButton>>();
        app.init_resource::<Messages<MouseMotion>>();
        app.init_resource::<Messages<WindowFocused>>();
        app.init_resource::<crate::ui::UiManager>();
        app.init_resource::<crate::sound::SoundState>();
        app.init_resource::<bevy::input::touch::Touches>();
        app.insert_non_send(engine);
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app.add_systems(Update, publish_sensors);

        app.update();
        assert!(blockloom_core::sense::read(|sensors| sensors
            .keys
            .contains("w")));

        let mut manager = app.world_mut().resource_mut::<crate::ui::UiManager>();
        manager.show(element("name", blockloom_core::ui::UiKind::Input));
        manager.take_pending();
        manager.focus_on(Some("name"));
        app.update();
        assert!(blockloom_core::sense::read(|sensors| sensors
            .keys
            .is_empty()));
    }
}
