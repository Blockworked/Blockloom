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
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
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
    // Dropped here rather than left open: a rebuild follows a fresh build of
    // the libraries, and the old ones must be closed before the new ones open.
    engine.scripts.clear();
    engine.scripts_started = false;
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

    let project = engine.project.clone();
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
    let first = !engine.scripts_started;
    engine.scripts_started = true;

    let actors: Vec<String> = engine.scripts.keys().cloned().collect();
    let mut asked = crate::script::Asked::default();
    for actor in &actors {
        let Some(script) = engine.scripts.get(actor) else {
            continue;
        };
        if first {
            script.start(actor, &mut asked);
        }
        script.tick(actor, &mut asked, dt);
    }
    for message in asked.messages.drain(..) {
        engine.fire(Event::Message(message));
    }
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
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    actors: Query<(&ActorId, &Transform, &Visibility, Option<&CustomComponents>)>,
) {
    let now = time.elapsed_secs() as f64;
    let held: HashSet<String> = keys.get_pressed().filter_map(key_name).collect();
    let mouse = mouse_world_position(dimension.0, &windows, &cameras).unwrap_or_default();

    let mut senses: HashMap<String, ActorSense> = HashMap::new();
    for (id, transform, visibility, custom) in &actors {
        let (rx, ry, rz) = transform.rotation.to_euler(EulerRot::XYZ);
        senses.insert(
            id.0.clone(),
            ActorSense {
                name: engine
                    .project
                    .actor(&id.0)
                    .map(|actor| actor.name.clone())
                    .unwrap_or_default(),
                position: transform.translation.to_array(),
                rotation: [rx.to_degrees(), ry.to_degrees(), rz.to_degrees()],
                scale: transform.scale.x,
                visible: *visibility != Visibility::Hidden,
                touching: engine.touching.get(&id.0).cloned().unwrap_or_default(),
                attached: engine.attached.get(&id.0).cloned().unwrap_or_default(),
                components: custom.map(|custom| custom.0.clone()).unwrap_or_default(),
            },
        );
    }

    blockloom_core::sense::publish(Sensors {
        time: engine.run_time(now),
        keys: held,
        mouse,
        mouse_down: buttons.pressed(MouseButton::Left),
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
                let Some(visual) = engine
                    .project
                    .actor(&id.0)
                    .and_then(|a| a.visual())
                    .cloned()
                else {
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
                let Some(visual) = engine
                    .project
                    .actor(&id.0)
                    .and_then(|a| a.visual())
                    .cloned()
                else {
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
            camera.rotation = target.rotation;
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
            .project
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
                .project
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
        // Anything else is a custom component: it comes back with the fields
        // the editor gave it, or empty if the project never had one.
        name => {
            let fields = engine
                .project
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
        name => {
            if let Ok(mut custom) = customs.get_mut(entity) {
                custom.0.remove(name);
            }
        }
    }
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
        | Effect::SetCameraView { actor, .. }
        | Effect::AttachComponent { actor, .. }
        | Effect::DetachComponent { actor, .. }
        | Effect::Error { actor, .. } => Some(actor),
        Effect::SetGravity { .. } | Effect::Stopped => None,
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
        .project
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
    let id = engine
        .project
        .actors
        .iter()
        .find(|actor| actor.id == target || actor.name.eq_ignore_ascii_case(target))
        .map(|actor| actor.id.clone())?;
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
                ..CameraAttach::default()
            },
            Transform::from_xyz(5.0, 0.0, 0.0).with_rotation(facing),
        );

        assert!(
            camera
                .translation
                .abs_diff_eq(Vec3::new(5.0, 2.0, 0.0), 0.001)
        );
        assert!(camera.rotation.abs_diff_eq(facing, 0.001));
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
}
