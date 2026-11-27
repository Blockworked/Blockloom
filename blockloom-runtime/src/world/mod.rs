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
//! other at the project's own rate. Actor sensing runs before each logical step.
//! Input and rendering stay on per-frame `Update`: events fire at most once
//! there, so several fixed steps in one frame don't triple a keypress.

use crate::engine::{
    ActorId, AnimationPlayer, CameraRig, CustomComponents, Dimension, Engine, Gliding,
    PendingEffects, PhysicsPose, PrevPose, TweeningColor, TweeningRotation, TweeningScale,
};
use crate::{bridge, dim2, dim3};
use bevy::ecs::message::MessageCursor;
use bevy::input::ButtonState;
use bevy::input::gamepad::{
    Gamepad, GamepadAxis, GamepadButton, GamepadRumbleIntensity, GamepadRumbleRequest,
};
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::{MouseMotion, MouseScrollUnit, MouseWheel};
use bevy::input::touch::Touches;
use bevy::prelude::*;
#[cfg(not(target_arch = "wasm32"))]
use bevy::window::CursorGrabMode;
use bevy::window::{CursorOptions, Ime, PrimaryWindow, WindowFocused};
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
use std::sync::Arc;

mod effects;
mod geometry;
mod helpers;
mod lifetimes;
mod rebuild;
mod run;
mod scripts;
mod sensors;
mod status;
mod ui_input;
mod vm;
pub use effects::*;
pub use geometry::*;
pub use helpers::*;
pub use lifetimes::*;
pub use rebuild::*;
pub use run::*;
pub use scripts::*;
pub use sensors::*;
pub use status::*;
pub use ui_input::*;
pub use vm::*;

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
    pub mesh: Option<Arc<polyanya::Mesh>>,
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
    // Borrowed, so a tick where nothing moved clones nothing.
    let mut signature: Vec<(&String, [f32; 3], &Visual)> = Vec::new();
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
        let Some(visual) = actor.visual() else {
            continue;
        };
        signature.push((id, transform.translation.to_array(), visual));
    }
    signature.sort_by(|a, b| a.0.cmp(b.0));
    // A plugin that changed the ground asks for a rebake whatever the actors say.
    let dirty = engine.plugins.nav_dirty.replace(false);
    let unchanged = !dirty
        && signature.len() == navmesh.signature.len()
        && signature
            .iter()
            .zip(&navmesh.signature)
            .all(|(now, then)| *now.0 == then.0 && now.1 == then.1 && *now.2 == then.2);
    if unchanged {
        return;
    }
    let mut actors = Vec::with_capacity(signature.len());
    for (id, position, _) in &signature {
        let Some(actor) = engine.actor(id) else {
            continue;
        };
        let mut live = actor.clone();
        live.components.placement_mut().position = *position;
        actors.push(live);
    }
    navmesh.signature = signature
        .into_iter()
        .map(|(id, position, visual)| (id.clone(), position, visual.clone()))
        .collect();
    let mut project = engine.project.clone();
    project.actors = actors;
    navmesh.mesh = nav::build_mesh(&project, nav::DEFAULT_AGENT_RADIUS)
        .map_err(|error| tracing::warn!("navmesh rebake failed: {error}"))
        .ok()
        .map(Arc::new);
    navmesh.mode = project.world.mode;
    navmesh.settings = project.world.navigation.clone();
    crate::plugins::publish_nav(&navmesh);
}

/// The systems that advance the simulation itself, kept apart from the input
/// and rendering systems in `Update` so the runtime can order them before the
/// physics pipeline's own fixed systems.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct SimulationSet;

/// The per-frame world rebuild, registered by the simulation. Presentation
/// gates it (a 2D world waits for its sprite shaders) and draws after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct RebuildSet;

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
/// An actor the simulation spawned whose look presentation hasn't drawn yet,
/// with whether a room streams its map in later. A headless world never
/// draws one, so there the marker just stays.
#[derive(Component)]
pub struct PendingLook {
    pub actor: Box<Actor>,
    pub streamed: bool,
    /// Only the surface changed (a material came or went), so the mesh's
    /// LOD, model and extras stay as they are.
    pub surface_only: bool,
}

impl PendingLook {
    pub fn of(actor: &Actor, streamed: bool) -> Self {
        Self {
            actor: Box::new(actor.clone()),
            streamed,
            surface_only: false,
        }
    }

    pub fn surface(actor: &Actor) -> Self {
        Self {
            surface_only: true,
            ..Self::of(actor, false)
        }
    }
}

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
        crate::engine::PluginComponents(actor.components.plugin_records().cloned().collect()),
    )
}

/// Hands the VM the contact events the last closed tick made, for both actors
/// of each pair. Only the actor-level edge of a pair starts a `when I touch`
/// strand, so a compound wall is still one Enter and one Exit.
pub fn deliver_contacts(engine: &mut Engine) {
    let tick = engine.contact_ticks + 1;
    for event in engine.contacts.deliver(tick) {
        if !event.edge {
            continue;
        }
        let (impulse, speed) = event.payload.as_ref().map_or((0.0, 0.0), |payload| {
            let v = payload.relative_velocity;
            (
                payload.impulse,
                (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt(),
            )
        });
        let sides = [
            (event.a.actor.clone(), event.b.actor.clone()),
            (event.b.actor.clone(), event.a.actor.clone()),
        ];
        for (actor, with) in sides {
            engine.fire(Event::Collision {
                actor,
                with,
                phase: event.phase,
                kind: event.kind,
                impulse,
                speed,
            });
        }
    }
}

/// Mirrors the tracker's actor-level answer into `touching`, which the
/// reporters read, when a tick changed it.
pub fn sync_touching(engine: &mut Engine) {
    if !engine.contacts.take_changed() {
        return;
    }
    let mut touching: HashMap<String, HashSet<String>> = HashMap::new();
    for (a, b, _) in engine.contacts.active_pairs() {
        if a.actor != b.actor {
            touching
                .entry(a.actor.clone())
                .or_default()
                .insert(b.actor.clone());
            touching
                .entry(b.actor.clone())
                .or_default()
                .insert(a.actor.clone());
        }
    }
    engine.touching = touching;
}

// ─── Editor messages ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
