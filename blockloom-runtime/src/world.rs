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
use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow, WindowFocused};
use blockloom_core::components::CameraView;
use blockloom_core::project::Actor;
use blockloom_core::scene::{Axis, BodyKind, Mode, Visual};
use blockloom_core::sense::{ActorSense, Sensors, normalize_key};
use blockloom_core::vm::{Effect, Event};
use blockloom_protocol::{ActorStatus, EditorMessage, RuntimeMessage, Status, VariableValue};
use std::collections::{HashMap, HashSet};

/// The one camera the project controls.
#[derive(Component)]
pub struct WorldCamera;

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
        scale: Vec3::splat(placement.scale),
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
                open_logic(&mut engine);
                engine.speech.clear();
                engine.running = false;
                engine.paused = false;
                engine.pause_began = None;
                engine.rebuild = true;
            }
            EditorMessage::Start => {
                let project = engine.project.clone();
                engine.vm.load(&project);
                if let Some(logic) = &mut engine.logic {
                    logic.reset();
                }
                engine.touching.clear();
                engine.speech.clear();
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
                engine.running = false;
                engine.paused = false;
                engine.pause_began = None;
                engine.rebuild = true;
                bridge::send(&RuntimeMessage::Stopped);
            }
            EditorMessage::Pause { paused } => {
                if paused && !engine.paused {
                    engine.paused = true;
                    engine.pause_began = Some(now);
                } else if !paused && engine.paused {
                    engine.paused = false;
                    if let Some(began) = engine.pause_began.take() {
                        // Shift the start forward by the paused span so the
                        // timer resumes where it froze instead of jumping.
                        engine.started_at += now - began;
                    }
                }
            }
            EditorMessage::Shutdown => {
                exit.write(AppExit::Success);
                return;
            }
        }
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
    cameras: Query<Entity, With<WorldCamera>>,
) {
    if !engine.rebuild {
        return;
    }
    engine.rebuild = false;

    for entity in &actors {
        commands.entity(entity).despawn();
    }
    for entity in &cameras {
        commands.entity(entity).despawn();
    }
    engine.entities.clear();
    engine.touching.clear();
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

    let mut project = engine.project.clone();
    // A child authored in its parent's frame is put where that works out to,
    // once, before anything is spawned from these placements.
    place_authored_children(&mut project);
    let dir = engine.project_dir.clone();
    clear_color.0 = parse_color(&project.world.background);
    match dimension.0 {
        Mode::TwoD => {
            commands.spawn((
                Camera2d,
                Projection::Orthographic(OrthographicProjection {
                    scale: 1.0 / project.world.camera.zoom.max(0.05),
                    ..OrthographicProjection::default_2d()
                }),
                WorldCamera,
            ));
            for actor in &project.actors {
                let entity =
                    dim2::spawn_actor(&mut commands, actor, dir.as_deref(), &assets, &mut textures)
                        .unwrap_or_else(|| spawn_unseen(&mut commands, actor, Mode::TwoD));
                attach_camera(&mut commands, actor, entity);
                engine.entities.insert(actor.id.clone(), entity);
            }
        }
        Mode::ThreeD => {
            dim3::spawn_scenery(&mut commands, &project.world.camera);
            for actor in &project.actors {
                let entity = dim3::spawn_actor(&mut commands, actor, &mut meshes, &mut materials)
                    .unwrap_or_else(|| spawn_unseen(&mut commands, actor, Mode::ThreeD));
                attach_camera(&mut commands, actor, entity);
                engine.entities.insert(actor.id.clone(), entity);
            }
        }
    }
    // The dimension's own effect system owns the physics pipeline, so gravity
    // is set the same way a `set gravity` block would set it.
    effects.0.push(Effect::SetGravity {
        gravity: project.world.gravity,
    });
    open_scripts(&mut engine, &project);
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
pub fn publish_sensors(
    mut engine: NonSendMut<Engine>,
    time: Res<Time>,
    dimension: Res<Dimension>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut motion: MessageReader<MouseMotion>,
    mut focus: MessageReader<WindowFocused>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    actors: Query<(&ActorId, &Transform, &Visibility, Option<&CustomComponents>)>,
) {
    let now = time.elapsed_secs() as f64;
    let held: HashSet<String> = keys.get_pressed().filter_map(key_name).collect();
    // OS-confirmed focus, not the component: `Window::focused` defaults to
    // true and winit only reports changes, so a game opened behind the
    // editor would otherwise read focused until the heat death of the run.
    for event in focus.read() {
        engine.window_focused = event.focused;
    }
    let focused = engine.window_focused;
    let mouse = mouse_world_position(dimension.0, &windows, &cameras).unwrap_or_default();
    // The pointer travels a few pixels a frame, not a teleport: sum the
    // motion events into one delta so a reporter reads what moved since last
    // frame, whichever half of the window it crossed.
    let mut mouse_delta = [0.0f32; 2];
    for moved in motion.read() {
        mouse_delta[0] += moved.delta.x;
        mouse_delta[1] += moved.delta.y;
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
    for (id, transform, visibility, custom) in &actors {
        let (rx, ry, rz) = transform.rotation.to_euler(EulerRot::XYZ);
        senses.insert(
            id.0.clone(),
            ActorSense {
                name: engine
                    .actor(&id.0)
                    .map(|actor| actor.name.clone())
                    .unwrap_or_default(),
                position: transform.translation.to_array(),
                rotation: [rx.to_degrees(), ry.to_degrees(), rz.to_degrees()],
                scale: transform.scale.x,
                visible: *visibility != Visibility::Hidden,
                parent: engine.parents.get(&id.0).cloned().unwrap_or_default(),
                is_clone: engine.clones.contains_key(&id.0),
                last_created: engine.last_created.get(&id.0).cloned().unwrap_or_default(),
                touching: engine.touching.get(&id.0).cloned().unwrap_or_default(),
                attached: engine.attached.get(&id.0).cloned().unwrap_or_default(),
                components: custom.map(|custom| custom.0.clone()).unwrap_or_default(),
            },
        );
    }

    // Wanted and focused reads as held: the component alone would still say
    // Locked after an unfocused request the backend silently dropped, and
    // reading the window back can't see that drop either.
    let mouse_locked = engine.wants_cursor_locked && focused;

    blockloom_core::sense::publish(Sensors {
        time: engine.run_time(now),
        keys: held,
        mouse,
        mouse_delta,
        mouse_locked,
        mouse_down: focused && buttons.pressed(MouseButton::Left),
        actors: senses,
    });

    if engine.running && !engine.paused {
        for key in keys.get_just_pressed().filter_map(key_name) {
            engine.fire(Event::Key(key));
        }
    }
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
    let (camera, camera_transform) = cameras.iter().next()?;
    match mode {
        Mode::TwoD => {
            let point = camera.viewport_to_world_2d(camera_transform, cursor).ok()?;
            Some([point.x, point.y])
        }
        Mode::ThreeD => {
            let ray = camera.viewport_to_world(camera_transform, cursor).ok()?;
            let distance = ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y))?;
            let point = ray.get_point(distance);
            Some([point.x, point.z])
        }
    }
}

/// Starts `when I am clicked` strands for whatever the pointer hit.
pub fn detect_clicks(
    mut engine: NonSendMut<Engine>,
    buttons: Res<ButtonInput<MouseButton>>,
    dimension: Res<Dimension>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    actors: Query<(&ActorId, &Transform)>,
) {
    if !engine.running || engine.paused || !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    let Some(window) = windows.iter().next() else {
        return;
    };
    // Clicks land in the focused window, so a click in the editor beside a
    // running game must never start its click strands. Event truth, same as
    // the motion gate above: the component defaults to focused.
    if !engine.window_focused {
        return;
    }
    let Some(cursor) = window.cursor_position() else {
        return;
    };
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

fn half_extents(visual: &Visual) -> Vec2 {
    match visual {
        Visual::Rect { size, .. } | Visual::Image { size, .. } => {
            Vec2::new(size[0] / 2.0, size[1] / 2.0)
        }
        Visual::Circle { radius, .. } => Vec2::splat(*radius),
        _ => Vec2::ZERO,
    }
}

fn half_extents3(visual: &Visual) -> Vec3 {
    match visual {
        Visual::Cuboid { size, .. } => Vec3::new(size[0] / 2.0, size[1] / 2.0, size[2] / 2.0),
        Visual::Sphere { radius, .. } => Vec3::splat(*radius),
        Visual::Capsule { radius, height, .. } => {
            Vec3::new(*radius, height / 2.0 + radius, *radius)
        }
        Visual::Plane { size, .. } => Vec3::new(size[0] / 2.0, 0.1, size[1] / 2.0),
        _ => Vec3::ZERO,
    }
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
    if !engine.running || engine.paused {
        return;
    }
    let now = engine.run_time(time.elapsed_secs() as f64);
    let mut produced = Vec::new();
    let mut messages = Vec::new();
    if engine.logic.is_some() {
        let variables = engine.variables.clone();
        engine.logic.as_mut().expect("checked above").tick(
            now,
            variables,
            &mut produced,
            &mut messages,
        );
    } else {
        engine.vm.tick(now, &mut produced);
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
    mut exit: MessageWriter<AppExit>,
    mut transforms: Query<(&mut Transform, &mut Visibility)>,
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
                if is_dynamic(&engine, actor) {
                    continue;
                }
                let forward = forward_of(&transform, dimension.0);
                transform.translation += forward * *steps;
            }
            Effect::GoTo { position, .. } => {
                transform.translation = vec3_in(dimension.0, *position, transform.translation);
            }
            Effect::ChangePosition { axis, by, .. } => {
                if is_dynamic(&engine, actor) {
                    continue;
                }
                if let Some(index) = position_axis(dimension.0, *axis) {
                    transform.translation[index] += *by;
                }
            }
            Effect::Turn { axis, degrees, .. } => {
                if let Some(index) = rotation_axis(dimension.0, *axis) {
                    let mut euler = euler_of(&transform);
                    euler[index] += degrees.to_radians();
                    transform.rotation = quat_of(euler);
                }
            }
            Effect::SetRotation { axis, degrees, .. } => {
                if let Some(index) = rotation_axis(dimension.0, *axis) {
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
            Effect::SetScale { factor, .. } => transform.scale = Vec3::splat(*factor),
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
    mut cameras: Query<&mut Transform, With<WorldCamera>>,
) {
    let Some((rig, target)) = rigs.iter().next() else {
        return;
    };
    let Ok(mut camera) = cameras.single_mut() else {
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
            camera.rotation =
                target.rotation * Quat::from_rotation_x(rig.pitch.to_radians());
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
) {
    if !engine.running || engine.paused {
        return;
    }
    for effect in &effects.0 {
        match effect {
            Effect::AttachComponent { actor, component } => attach(
                &mut commands,
                &mut engine,
                &mut customs,
                &mut visibilities,
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

/// Components that aren't this module's to attach: a body and a look need the
/// dimension's own pipeline, so `dim2`/`dim3` pick those up from the same
/// effect list.
fn is_dimensions_own(component: &str) -> bool {
    matches!(component, "Body" | "Look")
}

fn attach(
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
        // The hierarchy is `engine.parents`, not anything on the entity, and
        // the actor stays where it stands - as `set my parent to` leaves it.
        "Parent" => {
            let Some(parent) = engine
                .actor(actor)
                .and_then(|actor| actor.parent())
                .map(str::to_string)
            else {
                return;
            };
            set_parent(engine, actor, &parent);
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
    live: Query<(&Transform, &Visibility, Option<&CustomComponents>)>,
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
                if let Some(entity) = engine.entities.get(of).copied()
                    && let Ok((transform, visibility, custom)) = live.get(entity)
                {
                    copy.components.set_placement(placement_of(transform));
                    copy.components
                        .set_visible(*visibility != Visibility::Hidden);
                    if let Some(custom) = custom {
                        carry_custom(&mut copy, custom);
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
                    made,
                );
            }
            Effect::DeleteActor { actor } => delete_actor(&mut commands, &mut engine, actor),
            Effect::SetParent { actor, parent } => set_parent(&mut engine, actor, parent),
            _ => {}
        }
    }
}

/// A transform, as the `Place` component spells one.
fn placement_of(transform: &Transform) -> blockloom_core::scene::Placement {
    let (rx, ry, rz) = transform.rotation.to_euler(EulerRot::XYZ);
    blockloom_core::scene::Placement {
        position: transform.translation.to_array(),
        rotation: [rx.to_degrees(), ry.to_degrees(), rz.to_degrees()],
        scale: transform.scale.x,
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
    actor: Actor,
) {
    let dir = engine.project_dir.clone();
    let entity = match mode {
        Mode::TwoD => dim2::spawn_actor(commands, &actor, dir.as_deref(), assets, textures),
        Mode::ThreeD => dim3::spawn_actor(commands, &actor, meshes, materials),
    }
    .unwrap_or_else(|| spawn_unseen(commands, &actor, mode));
    attach_camera(commands, &actor, entity);
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

/// Hangs `actor` off `wanted`, or takes it off when that names nothing. The
/// actor stays exactly where it is: a parent moves a child from here on, it
/// doesn't place it.
fn set_parent(engine: &mut Engine, actor: &str, wanted: &str) {
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
    engine.parents.insert(actor.to_string(), parent);
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

/// Resolves every authored local offset into the world placement the actor
/// is spawned at.
///
/// `Place` stays what the world is built from, so this is the one moment an
/// offset is read: a child that carries one stands at
/// `parent placement * offset`, and one that doesn't stays exactly where its
/// own `Place` puts it. Parents are laid out before their children, so an
/// offset down a chain is measured against a parent that has already moved.
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
            .map(Vec3::from)
        else {
            continue;
        };
        let Some(parent) = parents.get(&child).and_then(|id| placed.get(id)).copied() else {
            continue;
        };
        let Some(mine) = placed.get_mut(&child) else {
            continue;
        };
        mine.translation = parent.transform_point(offset);
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
        scale: Vec3::splat(placement.scale),
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
    bridge::send(&RuntimeMessage::Status(Status {
        running: engine.running,
        paused: engine.paused,
        time: engine.run_time(now),
        fps: 1.0 / time.delta_secs().max(f32::EPSILON),
        actors: statuses,
        globals,
    }));
}

/// Empties the effect list once every apply system has seen it.
pub fn clear_effects(mut effects: ResMut<PendingEffects>) {
    effects.0.clear();
}

/// Carries out pointer lock requests, and guarantees a stopped run never
/// keeps the user's mouse: locking only means anything while running, so a
/// fresh Play always starts unlocked.
pub fn apply_cursor_lock(
    mut engine: NonSendMut<Engine>,
    effects: Res<PendingEffects>,
    mut targets: Query<(&mut Window, &mut CursorOptions), With<PrimaryWindow>>,
) {
    let Ok((mut window, mut cursor)) = targets.single_mut() else {
        return;
    };
    if !engine.running {
        engine.wants_cursor_locked = false;
        set_cursor_locked(&mut cursor, false);
        return;
    }
    for effect in &effects.0 {
        if let Effect::SetMouseLocked { locked } = effect {
            engine.wants_cursor_locked = *locked;
        }
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
        | Effect::Say { actor, .. }
        | Effect::SetVisible { actor, .. }
        | Effect::SetColor { actor, .. }
        | Effect::SetComponentField { actor, .. }
        |         Effect::SetCameraView { actor, .. }
        | Effect::SetCameraPitch { actor, .. }
        | Effect::AttachComponent { actor, .. }
        | Effect::DetachComponent { actor, .. }
        | Effect::Error { actor, .. } => Some(actor),
        // Making, deleting and re-parenting an actor are `apply_lifetimes`'s
        // to carry out, and none of them is a change to a transform.
        Effect::SetGravity { .. }
        | Effect::Stopped
        | Effect::SetMouseLocked { .. }
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
        app.world_mut()
            .resource_mut::<PendingEffects>()
            .0
            .clear();
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
                        scale: 1.0,
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
}
