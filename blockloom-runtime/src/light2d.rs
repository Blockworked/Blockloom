//! 2D lighting in the world (`blockloom_core::light2d`). One full-screen
//! layer, sorted just above the last lit sorting layer, multiplies what is
//! under it by ambient plus every light; `sync_lighting` gathers the frame's
//! lights, builds their shadow maps from solid tiles and shape actors, writes
//! them into the layer's material and publishes the same numbers for
//! `light level at` and `is night?`.

use crate::director::DirectorClock;
use crate::engine::{ActorId, Dimension, Engine, PendingEffects};
use crate::tiles::Level;
use crate::world::WorldCamera;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, RenderPipelineDescriptor,
    ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPlugin};
use blockloom_core::components::ActorComponent;
use blockloom_core::light2d::{
    Light2dSense, LightSample, MAX_LIGHTS, MAX_SHADOW_LIGHTS, Occluder, SHADOW_SAMPLES,
    distance_map,
};
use blockloom_core::scene::{Mode, Visual};
use blockloom_core::sense;
use blockloom_core::vm::Effect;
use std::collections::HashMap;

/// Vec4s of one light's shadow map in the uniform.
const SHADOW_VEC4S: usize = SHADOW_SAMPLES / 4;

pub fn register(app: &mut App) {
    bevy::asset::embedded_asset!(app, "shaders/light2d.wesl");
    app.add_plugins(Material2dPlugin::<LightMapMaterial>::default())
        .init_resource::<ShadowCache>()
        .add_systems(
            Update,
            sync_lighting
                .after(crate::world::interpolate_poses)
                .after(crate::world::publish_sensors),
        );
}

/// What the blocks have set the lit look to this run. Cleared on a rebuild
/// with the rest of the run's live state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Look2d {
    /// `set ambient light to`: the multiplier over the project's ambient
    /// colour, or `None` to leave it to the project and the clock.
    pub ambient: Option<f32>,
    /// A colour that replaces the project's ambient colour.
    pub ambient_color: Option<String>,
    /// 2D post dials set this run, one per dial in the order first set.
    pub post: Vec<(blockloom_core::blocks::Look2dDial, String)>,
    /// Camera trauma added since the camera system last looked.
    pub shake: f32,
    /// Seconds of hitstop asked for since the camera system last looked.
    pub hitstop: f32,
    /// Screen flash and cover dials not yet taken by `screenfx`, in order.
    pub fx: Vec<(blockloom_core::blocks::Look2dDial, String)>,
}

/// `set [look] to` blocks, applied on the fixed tick.
pub fn apply_look2d_effects(effects: Res<PendingEffects>, mut engine: NonSendMut<Engine>) {
    if !engine.running || engine.paused {
        return;
    }
    for effect in &effects.0 {
        if let Effect::SetLook2d { dial, value } = effect {
            engine.look2d.set(*dial, value);
        }
    }
}

impl Look2d {
    /// Applies one dial. A value that doesn't read is ignored.
    pub fn set(&mut self, dial: blockloom_core::blocks::Look2dDial, value: &str) {
        use blockloom_core::blocks::Look2dDial as D;
        let number = value.trim().parse::<f32>().ok().filter(|n| n.is_finite());
        match dial {
            D::AmbientLight => {
                if let Some(n) = number {
                    self.ambient = Some(n.clamp(0.0, 16.0));
                }
            }
            D::AmbientColor => {
                let color = value.trim();
                self.ambient_color = (!color.is_empty()).then(|| color.to_string());
            }
            D::Shake => {
                if let Some(n) = number {
                    self.shake = (self.shake + n).clamp(0.0, 1.0);
                }
            }
            D::Hitstop => {
                if let Some(n) = number {
                    self.hitstop = self.hitstop.max(n.clamp(0.0, 5.0));
                }
            }
            D::Flash | D::FlashColor | D::Cover | D::CoverKind | D::CoverColor | D::CoverTime => {
                self.fx.push((dial, value.to_string()));
            }
            _ => match self.post.iter_mut().find(|(d, _)| *d == dial) {
                Some(entry) => entry.1 = value.to_string(),
                None => self.post.push((dial, value.to_string())),
            },
        }
    }
}

/// One light as the shader takes it; matches `Light` in `light2d.wesl`.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct LightGpu {
    pos_range: Vec4,
    color_soft: Vec4,
    dir_cone: Vec4,
    flags: Vec4,
}

/// Matches `LightMap` in `light2d.wesl`.
#[derive(Clone, Debug, PartialEq, ShaderType)]
pub struct LightMapUniform {
    ambient: Vec4,
    lights: [LightGpu; MAX_LIGHTS],
    shadows: [Vec4; MAX_SHADOW_LIGHTS * SHADOW_VEC4S],
}

impl Default for LightMapUniform {
    fn default() -> Self {
        Self {
            ambient: Vec4::ONE,
            lights: [LightGpu::default(); MAX_LIGHTS],
            shadows: [Vec4::ZERO; MAX_SHADOW_LIGHTS * SHADOW_VEC4S],
        }
    }
}

impl LightMapUniform {
    /// The uniform for a frame's lights and shadow maps.
    pub fn of(sense: &Light2dSense) -> Self {
        let mut uniform = Self {
            ambient: Vec4::new(
                sense.ambient[0],
                sense.ambient[1],
                sense.ambient[2],
                sense.lights.len().min(MAX_LIGHTS) as f32,
            ),
            ..Self::default()
        };
        let mut slot = 0usize;
        for (i, (light, map)) in sense
            .lights
            .iter()
            .zip(&sense.maps)
            .take(MAX_LIGHTS)
            .enumerate()
        {
            let shadow = match map {
                Some(map) if slot < MAX_SHADOW_LIGHTS && map.len() == SHADOW_SAMPLES => {
                    for (k, quad) in map.as_chunks::<4>().0.iter().enumerate() {
                        uniform.shadows[slot * SHADOW_VEC4S + k] =
                            Vec4::new(quad[0], quad[1], quad[2], quad[3]);
                    }
                    slot += 1;
                    (slot - 1) as f32
                }
                _ => -1.0,
            };
            uniform.lights[i] = LightGpu {
                pos_range: Vec4::new(
                    light.position[0],
                    light.position[1],
                    light.range,
                    light.falloff,
                ),
                color_soft: Vec4::new(
                    light.color[0],
                    light.color[1],
                    light.color[2],
                    light.softness,
                ),
                dir_cone: Vec4::new(
                    light.direction[0],
                    light.direction[1],
                    light.cos_inner,
                    light.cos_outer,
                ),
                flags: Vec4::new(shadow, 0.0, 0.0, 0.0),
            };
        }
        uniform
    }
}

/// The multiply layer. Its mesh is a 2 by 2 quad the vertex shader turns
/// into the screen; the transform only gives it a depth.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct LightMapMaterial {
    #[uniform(0)]
    pub map: LightMapUniform,
}

impl Material2d for LightMapMaterial {
    fn vertex_shader() -> ShaderRef {
        shader()
    }

    fn fragment_shader() -> ShaderRef {
        shader()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn specialize(
        _pipeline: &bevy::sprite_render::Material2dPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Destination times light: the layer darkens or brightens what is
        // under it and leaves its alpha alone.
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(BlendState {
                    color: BlendComponent {
                        src_factor: BlendFactor::Dst,
                        dst_factor: BlendFactor::Zero,
                        operation: BlendOperation::Add,
                    },
                    alpha: BlendComponent {
                        src_factor: BlendFactor::Zero,
                        dst_factor: BlendFactor::One,
                        operation: BlendOperation::Add,
                    },
                });
            }
        }
        Ok(())
    }
}

fn shader() -> ShaderRef {
    ShaderRef::Path(
        bevy::asset::AssetPath::from_path_buf(bevy::asset::embedded_path!("shaders/light2d.wesl"))
            .with_source("embedded"),
    )
}

/// The layer in the world, so it can be moved and taken away.
#[derive(Component)]
pub struct LightLayer;

/// A shadow map per light actor, and what it was built from.
#[derive(Resource, Default)]
pub struct ShadowCache {
    maps: HashMap<String, (u64, Vec<f32>)>,
}

/// The frame's light state, for tests and the profiler.
#[derive(Resource, Default, Clone, Debug)]
#[allow(dead_code)]
pub struct LightFrame {
    pub lights: usize,
    pub shadowed: usize,
    pub occluders: usize,
}

fn mix(hash: u64, bits: u32) -> u64 {
    (hash ^ u64::from(bits)).wrapping_mul(0x0000_0100_0000_01B3)
}

/// A fingerprint of a light's position, range and the occluders it can see,
/// so a map is rebuilt only when something near it moved.
fn fingerprint(light: &LightSample, occluders: &[Occluder]) -> u64 {
    let mut hash = 0xCBF2_9CE4_8422_2325u64;
    for bits in [
        light.position[0].to_bits(),
        light.position[1].to_bits(),
        light.range.to_bits(),
    ] {
        hash = mix(hash, bits);
    }
    for occluder in occluders
        .iter()
        .filter(|o| o.near(light.position, light.range))
    {
        match *occluder {
            Occluder::Circle { center, radius } => {
                for bits in [
                    1,
                    center[0].to_bits(),
                    center[1].to_bits(),
                    radius.to_bits(),
                ] {
                    hash = mix(hash, bits);
                }
            }
            Occluder::Box {
                center,
                half,
                angle,
            } => {
                for bits in [
                    2,
                    center[0].to_bits(),
                    center[1].to_bits(),
                    half[0].to_bits(),
                    half[1].to_bits(),
                    angle.to_bits(),
                ] {
                    hash = mix(hash, bits);
                }
            }
        }
    }
    hash
}

/// The rotation about the view axis, radians.
fn spin(transform: &Transform) -> f32 {
    transform.rotation.to_euler(EulerRot::ZYX).0
}

/// Everything that blocks light right now: solid tiles and the shapes of
/// actors that opted in.
fn occluders(
    engine: &Engine,
    level: &Level,
    placed: &Query<(&ActorId, &Transform, &Visibility)>,
) -> Vec<Occluder> {
    let mut out = Vec::new();
    for (id, transform, visibility) in placed {
        if *visibility == Visibility::Hidden {
            continue;
        }
        if let Some(map) = level.maps.get(&id.0) {
            let scale = transform.scale.truncate().abs();
            let angle = spin(transform);
            for rect in map.solid_rects() {
                let center = transform
                    .transform_point(Vec3::new(rect.center[0], rect.center[1], 0.0))
                    .truncate();
                out.push(Occluder::Box {
                    center: center.to_array(),
                    half: [rect.half[0] * scale.x, rect.half[1] * scale.y],
                    angle,
                });
            }
            continue;
        }
        let Some(actor) = engine.actor(&id.0) else {
            continue;
        };
        let casts = matches!(
            actor.components.get("Sprite"),
            Some(ActorComponent::Sprite { sprite }) if sprite.casts_shadow
        );
        if !casts {
            continue;
        }
        let scale = transform.scale.truncate().abs();
        let center = transform.translation.truncate().to_array();
        match actor.visual() {
            Some(Visual::Rect { size, .. }) | Some(Visual::Image { size, .. }) => {
                out.push(Occluder::Box {
                    center,
                    half: [size[0] * scale.x * 0.5, size[1] * scale.y * 0.5],
                    angle: spin(transform),
                });
            }
            Some(Visual::Circle { radius, .. }) => out.push(Occluder::Circle {
                center,
                radius: radius * scale.x.max(scale.y),
            }),
            _ => {}
        }
    }
    out
}

/// What each light actor adds this frame, nearest the camera first.
fn gather(
    engine: &Engine,
    placed: &Query<(&ActorId, &Transform, &Visibility)>,
    camera: Vec2,
    time: f32,
) -> Vec<LightSample> {
    let mut found: Vec<(f32, LightSample)> = Vec::new();
    for (id, transform, visibility) in placed {
        if *visibility == Visibility::Hidden || !engine.has_component(&id.0, "Light2D") {
            continue;
        }
        let Some(spec) = engine.actor(&id.0).and_then(|a| a.components.light2d()) else {
            continue;
        };
        let intensity = engine
            .light_intensity
            .get(&id.0)
            .copied()
            .unwrap_or(spec.intensity);
        let position = transform.translation.truncate();
        let sample = spec.sample(position.to_array(), spin(transform), intensity, time);
        found.push((position.distance_squared(camera), sample));
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.into_iter().take(MAX_LIGHTS).map(|(_, l)| l).collect()
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn sync_lighting(
    mut commands: Commands,
    engine: NonSend<Engine>,
    dimension: Res<Dimension>,
    level: Option<Res<Level>>,
    clock: Option<Res<DirectorClock>>,
    fixed: Res<Time<Fixed>>,
    placed: Query<(&ActorId, &Transform, &Visibility)>,
    cameras: Query<&Transform, (With<WorldCamera>, Without<ActorId>)>,
    mut cache: ResMut<ShadowCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LightMapMaterial>>,
    mut layer: Local<Option<(Entity, Handle<LightMapMaterial>)>>,
    mut last: Local<Option<LightMapUniform>>,
    mut frame: Local<LightFrame>,
) {
    let settings = &engine.project.world.lighting2d;
    let live = dimension.0 == Mode::TwoD && settings.enabled;
    if !live {
        if let Some((entity, _)) = layer.take() {
            commands.entity(entity).despawn();
            *last = None;
        }
        cache.maps.clear();
        sense::publish_light2d(Light2dSense::default());
        return;
    }
    let director = &engine.project.world.director;
    let hour = clock
        .as_deref()
        .map_or(director.time_of_day, |clock| clock.time);
    // A stopped world doesn't flicker: the scene view shows the steady light.
    let time = if engine.running {
        fixed.elapsed_secs()
    } else {
        0.0
    };
    let camera = cameras
        .iter()
        .next()
        .map_or(Vec2::ZERO, |t| t.translation.truncate());
    let lights = gather(&engine, &placed, camera, time);
    let blockers = match level.as_deref() {
        Some(level) => occluders(&engine, level, &placed),
        None => Vec::new(),
    };
    let mut maps: Vec<Option<Vec<f32>>> = Vec::with_capacity(lights.len());
    let mut shadowed = 0usize;
    let mut keep: Vec<String> = Vec::new();
    // Lights come back in distance order, not by actor, so a map is looked
    // up by where the light stands.
    for light in &lights {
        if !light.shadows || shadowed >= MAX_SHADOW_LIGHTS {
            maps.push(None);
            continue;
        }
        shadowed += 1;
        let key = format!(
            "{:x}:{:x}",
            light.position[0].to_bits(),
            light.position[1].to_bits()
        );
        let print = fingerprint(light, &blockers);
        let map = match cache.maps.get(&key) {
            Some((held, map)) if *held == print => map.clone(),
            _ => {
                let map = distance_map(light.position, light.range, &blockers);
                cache.maps.insert(key.clone(), (print, map.clone()));
                map
            }
        };
        keep.push(key);
        maps.push(Some(map));
    }
    cache.maps.retain(|key, _| keep.contains(key));

    let overrides = &engine.look2d;
    let mut ambient = settings.ambient_at(hour, director.enabled);
    if let Some(color) = &overrides.ambient_color {
        let c = blockloom_core::material::hex_to_linear(color);
        let k = overrides.ambient.unwrap_or(settings.ambient);
        ambient = [c[0] * k, c[1] * k, c[2] * k];
    } else if let Some(k) = overrides.ambient {
        let base = blockloom_core::material::hex_to_linear(&settings.ambient_color);
        ambient = [base[0] * k, base[1] * k, base[2] * k];
    }
    let light_sense = Light2dSense {
        enabled: true,
        ambient,
        night: settings.is_night(hour),
        lights,
        maps,
    };
    *frame = LightFrame {
        lights: light_sense.lights.len(),
        shadowed,
        occluders: blockers.len(),
    };
    let uniform = LightMapUniform::of(&light_sense);
    sense::publish_light2d(light_sense);

    let depth = settings.unlit_above as f32 + 0.5;
    match layer.as_ref() {
        Some((entity, handle)) => {
            if last.as_ref() != Some(&uniform)
                && let Some(mut material) = materials.get_mut(handle)
            {
                material.map = uniform.clone();
            }
            commands
                .entity(*entity)
                .insert(Transform::from_xyz(0.0, 0.0, depth));
        }
        None => {
            let handle = materials.add(LightMapMaterial {
                map: uniform.clone(),
            });
            let entity = commands
                .spawn((
                    LightLayer,
                    Mesh2d(meshes.add(Rectangle::new(2.0, 2.0))),
                    MeshMaterial2d(handle.clone()),
                    Transform::from_xyz(0.0, 0.0, depth),
                    Visibility::default(),
                    bevy::camera::visibility::NoFrustumCulling,
                    Name::new("2D light layer"),
                ))
                .id();
            *layer = Some((entity, handle));
        }
    }
    *last = Some(uniform);
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::blocks::Look2dDial;
    use blockloom_core::light2d::Light2dSpec;

    fn lamp_sense(shadow: bool) -> Light2dSense {
        let spec = Light2dSpec {
            color: "#FFFFFF".to_string(),
            range: 100.0,
            shadows: shadow,
            ..Light2dSpec::default()
        };
        let light = spec.sample([10.0, 20.0], 0.0, 1.0, 0.0);
        let map = shadow.then(|| {
            distance_map(
                light.position,
                light.range,
                &[Occluder::Circle {
                    center: [50.0, 20.0],
                    radius: 5.0,
                }],
            )
        });
        Light2dSense {
            enabled: true,
            ambient: [0.1, 0.2, 0.3],
            night: false,
            lights: vec![light],
            maps: vec![map],
        }
    }

    #[test]
    fn the_uniform_carries_each_light_and_its_shadow_slot() {
        let sense = lamp_sense(true);
        let uniform = LightMapUniform::of(&sense);
        assert_eq!(uniform.ambient, Vec4::new(0.1, 0.2, 0.3, 1.0));
        assert_eq!(
            uniform.lights[0].pos_range.truncate(),
            Vec3::new(10.0, 20.0, 100.0)
        );
        assert_eq!(uniform.lights[0].flags.x, 0.0);
        let map = sense.maps[0].as_ref().unwrap();
        assert_eq!(uniform.shadows[0].x, map[0]);
        assert_eq!(uniform.shadows[SHADOW_VEC4S - 1].w, map[SHADOW_SAMPLES - 1]);
        let plain = LightMapUniform::of(&lamp_sense(false));
        assert_eq!(plain.lights[0].flags.x, -1.0);
    }

    #[test]
    fn only_the_first_sixteen_shadow_casters_get_a_slot() {
        let mut sense = lamp_sense(true);
        let (light, map) = (sense.lights[0].clone(), sense.maps[0].clone());
        for _ in 0..(MAX_SHADOW_LIGHTS + 3) {
            sense.lights.push(light.clone());
            sense.maps.push(map.clone());
        }
        let uniform = LightMapUniform::of(&sense);
        assert_eq!(
            uniform.lights[MAX_SHADOW_LIGHTS - 1].flags.x,
            (MAX_SHADOW_LIGHTS - 1) as f32
        );
        assert_eq!(uniform.lights[MAX_SHADOW_LIGHTS].flags.x, -1.0);
        assert_eq!(
            uniform.ambient.w as usize,
            MAX_LIGHTS.min(sense.lights.len())
        );
    }

    #[test]
    fn a_map_is_rebuilt_only_when_something_near_moves() {
        let light = lamp_sense(true).lights[0].clone();
        let wall = |x: f32| Occluder::Circle {
            center: [x, 20.0],
            radius: 5.0,
        };
        let a = fingerprint(&light, &[wall(50.0)]);
        assert_eq!(a, fingerprint(&light, &[wall(50.0)]));
        assert_ne!(a, fingerprint(&light, &[wall(60.0)]));
        // Far past the light's reach changes nothing.
        assert_eq!(
            fingerprint(&light, &[wall(50.0)]),
            fingerprint(&light, &[wall(50.0), wall(5_000.0)])
        );
    }

    #[test]
    fn blocks_set_the_ambient() {
        let mut look = Look2d::default();
        look.set(Look2dDial::AmbientLight, "0.5");
        look.set(Look2dDial::AmbientColor, "#112233");
        assert_eq!(look.ambient, Some(0.5));
        assert_eq!(look.ambient_color.as_deref(), Some("#112233"));
        look.set(Look2dDial::AmbientLight, "bright");
        assert_eq!(look.ambient, Some(0.5));
        look.set(Look2dDial::AmbientLight, "99");
        assert_eq!(look.ambient, Some(16.0));
        look.set(Look2dDial::AmbientColor, "");
        assert_eq!(look.ambient_color, None);
    }

    /// The shader's source, with Bevy's imports stubbed so naga can check
    /// everything else.
    #[test]
    fn the_light_shader_compiles() {
        let stub = "struct View { world_from_clip: mat4x4<f32> }\n\
@group(0) @binding(0) var<uniform> view: View;\n\
struct Vertex { @builtin(instance_index) instance_index: u32, @location(0) position: vec3<f32> }\n\
struct VertexOutput { @builtin(position) position: vec4<f32>, @location(0) world_position: vec4<f32>, \
@location(1) world_normal: vec3<f32>, @location(2) uv: vec2<f32>, @location(5) @interpolate(flat) instance_index: u32 }\n\
fn decompress_vertex(v: Vertex, i: u32) -> Vertex { return v; }\n\
fn get_world_from_local(i: u32) -> mat4x4<f32> { return mat4x4<f32>(); }\n\
fn mesh2d_position_world_to_clip(p: vec4<f32>) -> vec4<f32> { return p; }\n";
        let source = crate::materials::tests::stubbed(include_str!("shaders/light2d.wesl"), stub)
            .replace("mesh_functions::", "");
        blockloom_core::shader_lib::validate(&source, &[])
            .unwrap_or_else(|error| panic!("{error}"));
    }
}
