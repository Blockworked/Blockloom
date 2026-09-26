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
//! sprite or mesh, with a private fading material for each ghost.

use bevy::mesh::Mesh2d;
use bevy::prelude::*;
use bevy::sprite_render::{ColorMaterial, MeshMaterial2d};
use blockloom_core::blocks::EmitterDial;
use blockloom_core::material::ParticleSpec;
use blockloom_core::vm::Effect;

use crate::engine::ActorId;
use crate::engine::{Dimension, Engine};
use crate::materials::{GraphMaterial2d, GraphMaterial3d, TilemapMesh};
use crate::world::{forward_of, parse_color};

/// Live emission bookkeeping on an actor carrying an emitter.
#[derive(Component)]
pub struct EmitterState {
    acc: f32,
    spec: Option<ParticleSpec>,
    burst: u32,
}

impl EmitterState {
    pub fn fresh() -> Self {
        Self {
            acc: 0.0,
            spec: None,
            burst: 0,
        }
    }
}

/// Trail bookkeeping on an actor carrying a trail.
#[derive(Component)]
pub struct TrailState {
    timer: f32,
    enabled: bool,
}

impl TrailState {
    pub fn fresh() -> Self {
        Self {
            timer: 0.0,
            enabled: true,
        }
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
    /// How much of the wind carries it, and the air speed it has picked up.
    wind: f32,
    air: Vec3,
}

/// Seconds the wind takes to mostly carry a new particle along.
const WIND_CATCH: f32 = 0.25;

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

/// Despawn every particle and ghost when the world rebuilds: they belong to
/// the last run, not the document. Split out of `rebuild_world`, which is
/// already at the system's sixteen-param limit. Runs just before it.
pub fn despawn_fx(
    mut commands: Commands,
    engine: NonSend<Engine>,
    particles: Query<Entity, With<Particle>>,
    ghosts: Query<Entity, With<Ghost>>,
) {
    if !engine.rebuild {
        return;
    }
    for entity in particles.iter().chain(ghosts.iter()) {
        commands.entity(entity).despawn();
    }
}

/// At most this many particles and ghosts combined. Past it the world stops
/// emitting rather than spawning unbounded entities.
pub const MAX_FX: usize = 1024;

/// Apply block changes before the pending effects are cleared this step.
pub fn apply_fx_effects(
    effects: Res<crate::engine::PendingEffects>,
    engine: NonSend<Engine>,
    mut emitters: Query<&mut EmitterState>,
    mut trails: Query<&mut TrailState>,
) {
    if !engine.running || engine.paused {
        return;
    }
    for effect in &effects.0 {
        match effect {
            Effect::BurstParticles { actor, count } => {
                if let Some(entity) = engine.entities.get(actor) {
                    if let Ok(mut state) = emitters.get_mut(*entity) {
                        state.burst = state.burst.saturating_add(*count).min(512);
                    }
                }
            }
            Effect::SetEmitterDial { actor, dial, value } => {
                if !value.is_finite() {
                    continue;
                }
                let Some(entity) = engine.entities.get(actor) else {
                    continue;
                };
                let Ok(mut state) = emitters.get_mut(*entity) else {
                    continue;
                };
                let Some(base) = engine.actor(actor).and_then(|a| a.components.emitter()) else {
                    continue;
                };
                let spec = state.spec.get_or_insert_with(|| base.clone());
                match dial {
                    EmitterDial::Rate => spec.rate = *value,
                    EmitterDial::Lifetime => spec.lifetime = *value,
                    EmitterDial::Speed => spec.speed = *value,
                    EmitterDial::Spread => spec.spread = *value,
                    EmitterDial::Gravity => spec.gravity_scale = *value,
                    EmitterDial::SizeStart => spec.size_start = *value,
                    EmitterDial::SizeEnd => spec.size_end = *value,
                    EmitterDial::Max => spec.max = (*value as i64).clamp(1, 512) as u32,
                }
                spec.normalize();
            }
            Effect::SetTrailEnabled { actor, enabled } => {
                if let Some(entity) = engine.entities.get(actor) {
                    if let Ok(mut state) = trails.get_mut(*entity) {
                        state.enabled = *enabled;
                        state.timer = 0.0;
                    }
                }
            }
            _ => {}
        }
    }
}

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
    ghosts: Query<&Ghost>,
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
    let mut living = particles.iter().count() + ghosts.iter().count();
    if living >= MAX_FX {
        return;
    }
    // One sphere for every 3D particle, scaled per entity.
    let sphere = if dimension.0.is_3d() {
        Some(cache.sphere.clone().unwrap_or_else(|| {
            let handle = meshes.add(Mesh::from(Sphere::new(0.5)));
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
        let Some(authored) = engine
            .actor(&id.0)
            .and_then(|actor| actor.components.emitter())
            .cloned()
        else {
            continue;
        };
        let spec = state.spec.as_ref().unwrap_or(&authored).clone();
        state.acc += spec.rate * dt;
        let due = state.acc.floor() as usize;
        state.acc -= due as f32;
        let mut count = due.saturating_add(state.burst as usize);
        state.burst = 0;
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
        living += count;
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
        wind: spec.wind,
        air: Vec3::ZERO,
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
            Mesh3d(
                sphere
                    .clone()
                    .expect("the 3D branch always builds the sphere"),
            ),
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
    wind: Option<Res<crate::wind::WindField>>,
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
        if let Some(wind) = wind.as_deref().filter(|_| particle.wind > 0.0) {
            // The air eases in rather than kicking, and launch speed keeps.
            let carried = wind.at(transform.translation) * particle.wind;
            let catch = (dt / WIND_CATCH).min(1.0);
            particle.air = particle.air.lerp(carried, catch);
        }
        transform.translation += (particle.vel + particle.air) * dt;
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
            if let Some(mut material) = materials.get_mut(&handle.0) {
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
        Option<&Mesh2d>,
        Option<&MeshMaterial2d<ColorMaterial>>,
        Option<&MeshMaterial2d<GraphMaterial2d>>,
        Option<&Mesh3d>,
        Option<&MeshMaterial3d<StandardMaterial>>,
        Option<&MeshMaterial3d<GraphMaterial3d>>,
        Option<&MeshMaterial3d<crate::materials::BoxMaterial>>,
        Option<&TilemapMesh>,
        Option<&MeshMaterial3d<crate::batching::InstancedMaterial>>,
    )>,
    tile_children: Query<(&Mesh2d, &MeshMaterial2d<ColorMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut tiles: ResMut<Assets<ColorMaterial>>,
    mut graphs_2d: ResMut<Assets<GraphMaterial2d>>,
    mut graphs_3d: ResMut<Assets<GraphMaterial3d>>,
    mut boxes: ResMut<Assets<crate::materials::BoxMaterial>>,
    instanced: Option<Res<Assets<crate::batching::InstancedMaterial>>>,
    ghosts: Query<&Ghost>,
    particles: Query<&Particle>,
) {
    if !engine.running || engine.paused {
        return;
    }
    let mut living = ghosts.iter().count() + particles.iter().count();
    if living >= MAX_FX {
        return;
    }
    let dt = time.delta_secs();
    for (
        id,
        transform,
        mut state,
        sprite,
        mesh_2d,
        tile,
        graph_2d,
        mesh,
        handle,
        graph_3d,
        box_3d,
        child,
        instanced_3d,
    ) in &mut trailed
    {
        if !state.enabled || !engine.has_component(&id.0, "Trail") {
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
        if living >= MAX_FX {
            break;
        }
        let mut tint = parse_color(&spec.color);
        tint.set_alpha(0.5);
        let tile_child = child.and_then(|child| tile_children.get(child.0).ok());
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
            living += 1;
        } else if let Some((mesh, handle)) = tile_child.or_else(|| mesh_2d.zip(tile)) {
            let mut faded = tiles.get(&handle.0).cloned().unwrap_or_default();
            faded.color = tint;
            commands.spawn((
                Ghost {
                    age: 0.0,
                    life: spec.life,
                    base: 0.5,
                },
                *transform,
                Mesh2d(mesh.0.clone()),
                MeshMaterial2d(tiles.add(faded)),
            ));
            living += 1;
        } else if let (Some(mesh), Some(handle)) = (mesh_2d, graph_2d) {
            if let Some(mut faded) = graphs_2d.get(&handle.0).cloned() {
                faded.tint = Vec4::from_array(tint.to_linear().to_f32_array());
                commands.spawn((
                    Ghost {
                        age: 0.0,
                        life: spec.life,
                        base: 0.5,
                    },
                    *transform,
                    Mesh2d(mesh.0.clone()),
                    MeshMaterial2d(graphs_2d.add(faded)),
                ));
                living += 1;
            }
        } else if let (Some(mesh), Some(handle)) = (mesh, graph_3d) {
            if let Some(mut faded) = graphs_3d.get(&handle.0).cloned() {
                faded.tint = Vec4::from_array(tint.to_linear().to_f32_array());
                commands.spawn((
                    Ghost {
                        age: 0.0,
                        life: spec.life,
                        base: 0.5,
                    },
                    *transform,
                    Mesh3d(mesh.0.clone()),
                    MeshMaterial3d(graphs_3d.add(faded)),
                ));
                living += 1;
            }
        } else if let (Some(mesh), Some(handle)) = (mesh, box_3d) {
            if let Some(mut faded) = boxes.get(&handle.0).cloned() {
                faded.base.base_color = tint;
                faded.base.alpha_mode = AlphaMode::Blend;
                commands.spawn((
                    Ghost {
                        age: 0.0,
                        life: spec.life,
                        base: 0.5,
                    },
                    *transform,
                    Mesh3d(mesh.0.clone()),
                    MeshMaterial3d(boxes.add(faded)),
                ));
                living += 1;
            }
        } else if let Some(mesh) = mesh
            && (handle.is_some() || instanced_3d.is_some())
        {
            // An instanced actor's ghost fades on its own, so it gets a plain
            // copy of the shared surface.
            let mut faded = match (handle, instanced_3d, instanced.as_ref()) {
                (Some(handle), _, _) => materials.get(&handle.0).cloned(),
                (None, Some(handle), Some(instanced)) => instanced
                    .get(&handle.0)
                    .map(|material| material.base.clone()),
                _ => None,
            }
            .unwrap_or_default();
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
            living += 1;
        }
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
        Option<&MeshMaterial2d<ColorMaterial>>,
        Option<&MeshMaterial2d<GraphMaterial2d>>,
        Option<&MeshMaterial3d<StandardMaterial>>,
        Option<&MeshMaterial3d<GraphMaterial3d>>,
        Option<&MeshMaterial3d<crate::materials::BoxMaterial>>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut tiles: ResMut<Assets<ColorMaterial>>,
    mut graphs_2d: ResMut<Assets<GraphMaterial2d>>,
    mut graphs_3d: ResMut<Assets<GraphMaterial3d>>,
    mut boxes: ResMut<Assets<crate::materials::BoxMaterial>>,
) {
    if !engine.running || engine.paused {
        return;
    }
    let dt = time.delta_secs();
    for (entity, mut ghost, sprite, tile, graph_2d, handle, graph_3d, box_3d) in &mut ghosts {
        ghost.age += dt;
        if ghost.age >= ghost.life {
            commands.entity(entity).despawn();
            continue;
        }
        let alpha = ghost.base * (1.0 - (ghost.age / ghost.life).clamp(0.0, 1.0));
        if let Some(mut sprite) = sprite {
            sprite.color.set_alpha(alpha);
        } else if let Some(handle) = tile {
            if let Some(mut material) = tiles.get_mut(&handle.0) {
                material.color.set_alpha(alpha);
            }
        } else if let Some(handle) = graph_2d {
            if let Some(mut material) = graphs_2d.get_mut(&handle.0) {
                material.tint.w = alpha;
            }
        } else if let Some(handle) = graph_3d {
            if let Some(mut material) = graphs_3d.get_mut(&handle.0) {
                material.tint.w = alpha;
            }
        } else if let Some(handle) = box_3d {
            if let Some(mut material) = boxes.get_mut(&handle.0) {
                material.base.base_color.set_alpha(alpha);
            }
        } else if let Some(handle) = handle {
            if let Some(mut material) = materials.get_mut(&handle.0) {
                material.base_color.set_alpha(alpha);
            }
        }
    }
}
