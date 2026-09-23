//! CPU particles and motion trails.
//!
//! Emitters spray short-lived entities that fly on their own; trails stamp
//! fading ghosts of where an actor just was. Both are capped entity pools in
//! the sound system's image: one pass emits, one pass steps, the dead are
//! reaped. They freeze with the world when paused: a paused fireball hangs
//! mid-air instead of piling up while nobody watches.
//!
//! 2D particles are sprites; 3D ones are small emissive spheres sharing one
//! mesh, each with its own material so it can fade alone. Ghosts copy a
//! sprite or a standard mesh; custom-shaded actors leave no ghosts, since
//! their material can't be cloned into a fade.

use bevy::prelude::*;
use blockloom_core::material::ParticleSpec;

use crate::engine::{Dimension, Engine};
use crate::world::{ActorId, forward_of, parse_color};

/// Live emission bookkeeping on an actor carrying an emitter.
#[derive(Component)]
pub struct EmitterState {
    acc: f32,
}

impl EmitterState {
    pub fn fresh() -> Self {
        Self { acc: 0.0 }
    }
}

/// Trail bookkeeping on an actor carrying a trail.
#[derive(Component)]
pub struct TrailState {
    timer: f32,
}

impl TrailState {
    pub fn fresh() -> Self {
        Self { timer: 0.0 }
    }
}

/// One live particle: who made it, how it flies, how it fades.
#[derive(Component)]
pub struct Particle {
    owner: String,
    vel: Vec3,
    age: f32,
    life: f32,
    size0: f32,
    size1: f32,
    color0: Color,
    color1: Color,
    gravity: f32,
}

/// One fading snapshot left by a trail, with the alpha it started at.
#[derive(Component)]
pub struct Ghost {
    age: f32,
    life: f32,
    base: f32,
}

/// The shape every 3D particle shares: a unit sphere, scaled per particle.
#[derive(Resource, Default)]
pub struct FxCache {
    sphere: Option<Handle<Mesh>>,
}

/// At most this many particles and ghosts combined. Past it the world stops
/// emitting rather than spawning unbounded entities.
pub const MAX_FX: usize = 1024;

/// Insert emission state for an actor that carries the matching component.
/// Called everywhere an actor entity is born: both rebuild loops and the
/// runtime spawner, so a mid-run attach and an authored component agree.
pub fn insert_fx_state(
    commands: &mut Commands,
    actor: &blockloom_core::project::Actor,
    entity: Entity,
) {
    if actor.components.emitter().is_some() {
        commands.entity(entity).insert(EmitterState::fresh());
    }
    if actor.components.trail().is_some() {
        commands.entity(entity).insert(TrailState::fresh());
    }
}

/// A cheap deterministic stream: the same run plays the same spray, and no
/// RNG plugin is needed for what is only a visual.
fn rand(state: &mut u64) -> f32 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    ((*state >> 33) as f32) / (u32::MAX as f32)
}

/// Spray new particles from every running emitter.
#[allow(clippy::too_many_arguments)]
pub fn emit_particles(
    mut commands: Commands,
    engine: NonSend<Engine>,
    dimension: Res<Dimension>,
    time: Res<Time>,
    mut cache: ResMut<FxCache>,
    mut emitters: Query<(&ActorId, &Transform, &mut EmitterState)>,
    particles: Query<&Particle>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut seed: Local<u64>,
) {
    if !engine.running || engine.paused {
        return;
    }
    if *seed == 0 {
        *seed = 0x9E3779B97F4A7C15;
    }
    let dt = time.delta_secs();
    let living = particles.iter().count();
    if living >= MAX_FX {
        return;
    }
    // One sphere for every 3D particle, scaled per entity.
    let sphere = if dimension.0.is_3d() {
        Some(cache.sphere.clone().unwrap_or_else(|| {
            let handle = meshes.add(Sphere::new(0.5).mesh());
            cache.sphere = Some(handle.clone());
            handle
        }))
    } else {
        None
    };
    for (id, transform, mut state) in &mut emitters {
        if !engine.has_component(&id.0, "Emitter") {
            continue;
        }
        let Some(spec) = engine
            .actor(&id.0)
            .and_then(|actor| actor.components.emitter())
            .cloned()
        else {
            continue;
        };
        if spec.rate <= 0.0 {
            continue;
        }
        state.acc += spec.rate * dt;
        let mut count = state.acc.floor() as usize;
        state.acc -= count as f32;
        // One emitter never takes more than its share of the pool.
        let owned = particles.iter().filter(|p| p.owner == id.0).count();
        count = count.min(spec.max.saturating_sub(owned as u32) as usize);
        count = count.min(MAX_FX.saturating_sub(living));
        if count == 0 {
            continue;
        }
        let facing = forward_of(transform, dimension.0);
        for _ in 0..count {
            spawn_particle(
                &mut commands,
                &spec,
                transform,
                facing,
                dimension.0,
                &sphere,
                &mut materials,
                &mut seed,
                &id.0,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_particle(
    commands: &mut Commands,
    spec: &ParticleSpec,
    from: &Transform,
    facing: Vec3,
    mode: blockloom_core::scene::Mode,
    sphere: &Option<Handle<Mesh>>,
    materials: &mut Assets<StandardMaterial>,
    seed: &mut u64,
    owner: &str,
) {
    let jitter = (rand(seed) - 0.5) * spec.spread.to_radians();
    let dir = if mode.is_3d() {
        Quat::from_rotation_y(jitter) * facing
    } else {
        Quat::from_rotation_z(jitter) * facing
    };
    let speed = spec.speed * (0.5 + rand(seed));
    let life = spec.lifetime * (0.6 + 0.8 * rand(seed));
    let color0 = parse_color(&spec.color_start);
    let particle = Particle {
        owner: owner.to_string(),
        vel: dir * speed,
        age: 0.0,
        life,
        size0: spec.size_start,
        size1: spec.size_end,
        color0,
        color1: parse_color(&spec.color_end),
        gravity: spec.gravity_scale,
    };
    if mode.is_3d() {
        let material = materials.add(StandardMaterial {
            base_color: color0,
            emissive: bevy::color::LinearRgba::from(color0) * 1.5,
            alpha_mode: AlphaMode::Blend,
            ..default()
        });
        commands.spawn((
            particle,
            Transform {
                translation: from.translation,
                scale: Vec3::splat(spec.size_start.max(0.01)),
                ..default()
            },
            Mesh3d(sphere.clone().expect("the 3D branch always builds the sphere")),
            MeshMaterial3d(material),
        ));
    } else {
        commands.spawn((
            particle,
            Transform::from_translation(from.translation),
            Sprite {
                color: color0,
                custom_size: Some(Vec2::splat(spec.size_start)),
                ..default()
            },
        ));
    }
}

/// Fly every particle one step: age, fall, drift, tint, shrink. The dead
/// despawn here rather than in a third pass, since stepping already visits
/// them all.
pub fn step_particles(
    mut commands: Commands,
    engine: NonSend<Engine>,
    time: Res<Time>,
    mut particles: Query<(
        Entity,
        &mut Particle,
        &mut Transform,
        Option<&mut Sprite>,
        Option<&MeshMaterial3d<StandardMaterial>>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !engine.running || engine.paused {
        return;
    }
    let dt = time.delta_secs();
    let gravity = engine.project.world.gravity[1];
    for (entity, mut particle, mut transform, sprite, handle) in &mut particles {
        particle.age += dt;
        if particle.age >= particle.life {
            commands.entity(entity).despawn();
            continue;
        }
        particle.vel.y += gravity * particle.gravity * dt;
        transform.translation += particle.vel * dt;
        let t = (particle.age / particle.life).clamp(0.0, 1.0);
        // Linear in sRGB reads fine for sparks and smoke; physical blending
        // would cost a round trip nobody sees at this size.
        let faded = particle.color0.mix(&particle.color1, t);
        let faded = faded.with_alpha(faded.alpha() * (1.0 - t));
        let size = particle.size0 + (particle.size1 - particle.size0) * t;
        if let Some(mut sprite) = sprite {
            sprite.color = faded;
            sprite.custom_size = Some(Vec2::splat(size.max(0.0)));
        }
        if let Some(handle) = handle {
            if let Some(material) = materials.get_mut(&handle.0) {
                material.base_color = faded;
                material.emissive = bevy::color::LinearRgba::from(faded) * 1.5;
            }
            transform.scale = Vec3::splat(size.max(0.01));
        }
    }
}

/// Stamp a ghost of every trailed actor on its interval.
pub fn snapshot_trails(
    mut commands: Commands,
    engine: NonSend<Engine>,
    time: Res<Time>,
    mut trailed: Query<(
        &ActorId,
        &Transform,
        &mut TrailState,
        Option<&Sprite>,
        Option<&Mesh3d>,
        Option<&MeshMaterial3d<StandardMaterial>>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    ghosts: Query<&Ghost>,
) {
    if !engine.running || engine.paused {
        return;
    }
    if ghosts.iter().count() >= MAX_FX {
        return;
    }
    let dt = time.delta_secs();
    for (id, transform, mut state, sprite, mesh, handle) in &mut trailed {
        if !engine.has_component(&id.0, "Trail") {
            continue;
        }
        let Some(spec) = engine
            .actor(&id.0)
            .and_then(|actor| actor.components.trail())
            .cloned()
        else {
            continue;
        };
        state.timer += dt;
        if state.timer < spec.interval {
            continue;
        }
        state.timer = 0.0;
        let mut tint = parse_color(&spec.color);
        tint.set_alpha(0.5);
        if let Some(sprite) = sprite {
            let mut ghost = sprite.clone();
            ghost.color = tint;
            commands.spawn((
                Ghost {
                    age: 0.0,
                    life: spec.life,
                    base: 0.5,
                },
                *transform,
                ghost,
            ));
        } else if let (Some(mesh), Some(handle)) = (mesh, handle) {
            let mut faded = materials.get(&handle.0).cloned().unwrap_or_default();
            faded.base_color = tint;
            faded.alpha_mode = AlphaMode::Blend;
            commands.spawn((
                Ghost {
                    age: 0.0,
                    life: spec.life,
                    base: 0.5,
                },
                *transform,
                Mesh3d(mesh.0.clone()),
                MeshMaterial3d(materials.add(faded)),
            ));
        }
        // Anything else (tilemap quads, custom shaders) leaves no ghost:
        // their material can't be faded by cloning.
    }
}

/// Fade every ghost from its starting alpha and reap it.
pub fn step_ghosts(
    mut commands: Commands,
    engine: NonSend<Engine>,
    time: Res<Time>,
    mut ghosts: Query<(
        Entity,
        &mut Ghost,
        Option<&mut Sprite>,
        Option<&MeshMaterial3d<StandardMaterial>>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !engine.running || engine.paused {
        return;
    }
    let dt = time.delta_secs();
    for (entity, mut ghost, sprite, handle) in &mut ghosts {
        ghost.age += dt;
        if ghost.age >= ghost.life {
            commands.entity(entity).despawn();
            continue;
        }
        let alpha = ghost.base * (1.0 - (ghost.age / ghost.life).clamp(0.0, 1.0));
        if let Some(mut sprite) = sprite {
            sprite.color.set_alpha(alpha);
        } else if let Some(handle) = handle {
            if let Some(material) = materials.get_mut(&handle.0) {
                material.base_color.set_alpha(alpha);
            }
        }
    }
}
