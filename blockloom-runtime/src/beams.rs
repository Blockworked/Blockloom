//! Light beams as geometry, 3D only: a shaft cone for a spot whose beam the
//! froxels don't draw, dust motes drifting in any beam, and the fog's height
//! dust around the camera. Both materials are unlit and additive, and read
//! the camera's exposure themselves.
//!
//! Geometry is kept apart from the light's own child entity and follows its
//! pose each frame without the actor's scale, so a stretched actor doesn't
//! stretch its beam.

use crate::engine::Engine;
use crate::environment::Environment;
use crate::lights::Lit;
use crate::world::{WorldCamera, parse_color};
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use blockloom_core::components::{LightKind, LightSpec};
use blockloom_core::fog::{Beam, Motes, VolumetricFog};
use std::collections::{HashMap, HashSet};
use std::f32::consts::PI;

/// Metres across the height dust's box around the camera.
pub const DUST_BOX: f32 = 12.0;

/// How sharply a shaft fades towards its silhouette.
const SHAFT_FRESNEL: f32 = 1.5;

/// Metres across one feature of a shaft's noise.
const SHAFT_NOISE_SCALE: f32 = 2.0;

const CONE_SEGMENTS: u32 = 32;
const CONE_RINGS: u32 = 8;

pub fn register(app: &mut App) {
    bevy::asset::embedded_asset!(app, "shaders/beam_shaft.wesl");
    bevy::asset::embedded_asset!(app, "shaders/beam_motes.wesl");
    app.add_plugins((
        MaterialPlugin::<ShaftMaterial>::default(),
        MaterialPlugin::<MoteMaterial>::default(),
    ))
    .add_systems(
        Update,
        (sync_beams, sync_dust)
            .after(crate::lights::sync_lights)
            .after(crate::environment::blend_environment),
    );
}

macro_rules! embedded {
    ($path:literal) => {
        ShaderRef::Path(
            bevy::asset::AssetPath::from_path_buf(bevy::asset::embedded_path!($path))
                .with_source("embedded"),
        )
    };
}

/// On every entity this module draws, so passes that copy the world's
/// meshes (ray tracing) leave them out.
#[derive(Component)]
pub struct Additive;

/// A spot's beam as an open cone.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, PartialEq)]
pub struct ShaftMaterial {
    /// rgb light per metre of distance from the lamp, w fresnel power.
    #[uniform(0)]
    pub color: Vec4,
    /// x range, y near fade, z far fade, w falloff curve.
    #[uniform(1)]
    pub beam: Vec4,
    /// x noise amount, y scroll m/s, z 1 / noise scale.
    #[uniform(2)]
    pub noise: Vec4,
}

impl Material for ShaftMaterial {
    fn fragment_shader() -> ShaderRef {
        embedded!("shaders/beam_shaft.wesl")
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Both sides, so the far wall of the cone glows too.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Billboard motes, placed on the GPU. `shaders/beam_motes.wesl`.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, PartialEq)]
pub struct MoteMaterial {
    /// rgb radiance (a lamp's at one metre, or the sun's), w alpha.
    #[uniform(0)]
    pub color: Vec4,
    /// x size, y twinkle, z drift, w mode: 0 spot, 1 point, 2 dust box.
    #[uniform(1)]
    pub shape: Vec4,
    /// x range (box size for dust), y tan of the outer angle, z near fade,
    /// w far fade.
    #[uniform(2)]
    pub beam: Vec4,
    /// x cos outer (-2 for a point), y cos inner, z falloff curve, w g.
    #[uniform(3)]
    pub cone: Vec4,
    /// Dust only: xyz towards the sun, w top height.
    #[uniform(4)]
    pub sun: Vec4,
    /// Dust only: rgb ambient radiance, w base height.
    #[uniform(5)]
    pub ambient: Vec4,
}

impl Material for MoteMaterial {
    fn vertex_shader() -> ShaderRef {
        embedded!("shaders/beam_motes.wesl")
    }

    fn fragment_shader() -> ShaderRef {
        embedded!("shaders/beam_motes.wesl")
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

fn linear(color: Color) -> Vec3 {
    let c = color.to_linear();
    Vec3::new(c.red, c.green, c.blue)
}

/// Which geometry a light's beam needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Shaft,
    Motes,
}

/// The parts a light wants, given the project's volumetric fog.
pub fn parts(spec: &LightSpec, fog: &VolumetricFog) -> Vec<Part> {
    let mut out = Vec::new();
    let beam = normalized(&spec.beam);
    if spec.kind == LightKind::Spot && beam.density > 0.0 && beam.uses_shaft(fog) {
        out.push(Part::Shaft);
    }
    let lamp = matches!(spec.kind, LightKind::Spot | LightKind::Point);
    if lamp && beam.motes.enabled && beam.motes.count > 0 {
        out.push(Part::Motes);
    }
    out
}

fn normalized(beam: &Beam) -> Beam {
    let mut beam = beam.clone();
    beam.normalize();
    beam
}

/// A light's candela per channel.
fn candela(spec: &LightSpec) -> Vec3 {
    linear(parse_color(&spec.color)) * spec.lumens() / (4.0 * PI)
}

/// The shaft for a spot: single scattering through the beam's width,
/// `density * I * 2 tan(outer) / (4 pi d)` nits, `scale` the air's.
pub fn shaft_material(spec: &LightSpec, scale: f32) -> ShaftMaterial {
    let beam = normalized(&spec.beam);
    let (_, outer) = spec.cone();
    let across = 2.0 * outer.tan() / (4.0 * PI);
    let strength = beam.density * scale.max(0.0) * across * beam.shaft_intensity;
    ShaftMaterial {
        color: (candela(spec) * strength).extend(SHAFT_FRESNEL),
        beam: Vec4::new(
            spec.range.max(0.01),
            beam.near_fade,
            beam.far_fade,
            beam.falloff,
        ),
        noise: Vec4::new(
            beam.shaft_noise,
            beam.shaft_scroll,
            1.0 / SHAFT_NOISE_SCALE,
            0.0,
        ),
    }
}

/// How far a light's motes reach: a spot's whole cone, or half a point's
/// range, where most of its light is.
fn mote_range(spec: &LightSpec) -> f32 {
    let range = spec.range.max(0.01);
    if spec.kind == LightKind::Point {
        range * 0.5
    } else {
        range
    }
}

/// Motes lit by their own lamp: a small diffuse grain, `E / pi`.
pub fn mote_material(spec: &LightSpec, fog: &VolumetricFog) -> MoteMaterial {
    let beam = normalized(&spec.beam);
    let motes = &beam.motes;
    let point = spec.kind == LightKind::Point;
    let (inner, outer) = spec.cone();
    let (cos_outer, cos_inner, tan_outer) = if point {
        (-2.0, 0.0, -1.0)
    } else {
        (outer.cos(), inner.cos(), outer.tan())
    };
    MoteMaterial {
        color: (candela(spec) / PI).extend(motes.alpha),
        shape: Vec4::new(motes.size, motes.twinkle, motes.drift, point as u32 as f32),
        beam: Vec4::new(mote_range(spec), tan_outer, beam.near_fade, beam.far_fade),
        cone: Vec4::new(
            cos_outer,
            cos_inner,
            beam.falloff,
            beam.anisotropy.unwrap_or(fog.anisotropy),
        ),
        sun: Vec4::ZERO,
        ambient: Vec4::ZERO,
    }
}

/// Height dust lit by the sun (through the fog's phase) and the ambient.
pub fn dust_material(fog: &VolumetricFog, env: &Environment) -> MoteMaterial {
    let mut motes = fog.dust.clone();
    motes.normalize();
    let albedo = linear(env.volumetric_albedo);
    let sun = linear(env.sun.color) * env.sun.illuminance / PI;
    let ambient = linear(env.ambient_color) * env.ambient_brightness;
    MoteMaterial {
        color: (albedo * sun).extend(motes.alpha),
        shape: Vec4::new(motes.size, motes.twinkle, motes.drift, 2.0),
        beam: Vec4::new(DUST_BOX, 0.0, 0.0, 0.0),
        cone: Vec4::new(-2.0, 0.0, 0.0, fog.anisotropy),
        sun: env
            .sun
            .direction
            .extend(fog.base_height + fog.dust_height.max(0.01)),
        ambient: (albedo * ambient).extend(fog.base_height),
    }
}

/// An open unit cone down -Z, apex at the origin and radius 1 at z = -1.
/// v runs from the apex (0) to the base (1).
pub fn cone_mesh() -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    for j in 0..=CONE_RINGS {
        let t = j as f32 / CONE_RINGS as f32;
        for i in 0..=CONE_SEGMENTS {
            let u = i as f32 / CONE_SEGMENTS as f32;
            let (sin, cos) = (u * 2.0 * PI).sin_cos();
            positions.push([cos * t, sin * t, -t]);
            normals.push((Vec3::new(cos, sin, 1.0) / 2f32.sqrt()).to_array());
            uvs.push([u, t]);
        }
    }
    let row = CONE_SEGMENTS + 1;
    let mut indices = Vec::new();
    for j in 0..CONE_RINGS {
        for i in 0..CONE_SEGMENTS {
            let a = j * row + i;
            let b = a + row;
            indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

/// Mote `index`'s seed, each component in [0, 1).
pub fn mote_seed(index: u32, salt: u32) -> Vec3 {
    let mut state = index.wrapping_mul(747_796_405).wrapping_add(salt | 1);
    let mut next = || {
        state = state.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
        let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
        ((word >> 22) ^ word) as f32 / 4_294_967_296.0
    };
    Vec3::new(next(), next(), next())
}

/// `count` camera-facing quads. The seed rides in the vertex color for the
/// shader; `place` puts each where it starts, so the bounds are right.
pub fn motes_mesh(count: u32, salt: u32, place: impl Fn(Vec3) -> Vec3) -> Mesh {
    let count = count.min(blockloom_core::fog::MAX_MOTES);
    let mut positions = Vec::with_capacity(count as usize * 4);
    let mut uvs = Vec::with_capacity(count as usize * 4);
    let mut colors = Vec::with_capacity(count as usize * 4);
    let mut indices = Vec::with_capacity(count as usize * 6);
    for k in 0..count {
        let seed = mote_seed(k, salt);
        let at = place(seed).to_array();
        let base = k * 4;
        for corner in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
            positions.push(at);
            uvs.push(corner);
            colors.push([seed.x, seed.y, seed.z, 1.0]);
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices))
}

/// Where a beam's mote starts, as `mote_in_beam` places it at time 0.
fn mote_start(seed: Vec3, spec: &LightSpec) -> Vec3 {
    let range = mote_range(spec);
    let angle = seed.y * 2.0 * PI;
    let spread = seed.z.sqrt();
    if spec.kind == LightKind::Point {
        let up = spread * 2.0 - 1.0;
        let flat = (1.0 - up * up).max(0.0).sqrt();
        return Vec3::new(angle.cos() * flat, up, angle.sin() * flat) * range * seed.x.cbrt();
    }
    let near = spec.beam.near_fade.clamp(0.0, range * 0.5);
    let along = near + (range - near) * seed.x.cbrt();
    let radius = spec.cone().1.tan() * along * spread;
    Vec3::new(angle.cos() * radius, angle.sin() * radius, -along)
}

/// One piece of a light's beam, following the light it was built from.
#[derive(Component)]
pub struct BeamGeometry {
    owner: Entity,
    child: Entity,
    spec: LightSpec,
    part: Part,
}

/// Builds, follows and takes away every light's shaft and motes.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_beams(
    mut commands: Commands,
    engine: NonSend<Engine>,
    environment: Res<Environment>,
    lit: Query<(Entity, &Lit)>,
    transforms: Query<&GlobalTransform>,
    mut parts_query: Query<(
        Entity,
        &BeamGeometry,
        &mut Transform,
        &mut Visibility,
        Option<&MeshMaterial3d<ShaftMaterial>>,
        Option<&MeshMaterial3d<MoteMaterial>>,
    )>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut shafts: ResMut<Assets<ShaftMaterial>>,
    mut motes: ResMut<Assets<MoteMaterial>>,
) {
    let fog = &engine.project.world.fog.volumetric;
    let lights: HashMap<Entity, &Lit> = lit.iter().collect();
    let mut have = HashSet::new();
    for (entity, geometry, mut transform, mut visibility, shaft, mote) in &mut parts_query {
        let current = lights.get(&geometry.owner).filter(|lit| {
            lit.child == geometry.child
                && lit.spec == geometry.spec
                && parts(&lit.spec, fog).contains(&geometry.part)
        });
        let (Some(lit), Ok(at)) = (current, transforms.get(geometry.child)) else {
            commands.entity(entity).despawn();
            continue;
        };
        have.insert((geometry.owner, geometry.part));
        let (_, rotation, translation) = at.to_scale_rotation_translation();
        transform.translation = translation;
        transform.rotation = rotation;
        match geometry.part {
            Part::Shaft => {
                let range = lit.spec.range.max(0.01);
                let wide = lit.spec.cone().1.tan() * range;
                transform.scale = Vec3::new(wide, wide, range);
                let next = shaft_material(&lit.spec, environment.beams);
                let lit_up = next.color.truncate().max_element() > 0.0;
                visibility.set_if_neq(if lit_up {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                });
                if let Some(handle) = shaft
                    && shafts.get(&handle.0) != Some(&next)
                    && let Some(mut material) = shafts.get_mut(&handle.0)
                {
                    *material = next;
                }
            }
            Part::Motes => {
                visibility.set_if_neq(Visibility::Inherited);
                let next = mote_material(&lit.spec, fog);
                if let Some(handle) = mote
                    && motes.get(&handle.0) != Some(&next)
                    && let Some(mut material) = motes.get_mut(&handle.0)
                {
                    *material = next;
                }
            }
        }
    }
    for (owner, lit) in lights {
        for part in parts(&lit.spec, fog) {
            if have.contains(&(owner, part)) {
                continue;
            }
            let geometry = BeamGeometry {
                owner,
                child: lit.child,
                spec: lit.spec.clone(),
                part,
            };
            // Hidden until the next frame puts it where its light is.
            let common = (
                geometry,
                Additive,
                Transform::default(),
                Visibility::Hidden,
                NotShadowCaster,
                NotShadowReceiver,
            );
            match part {
                Part::Shaft => {
                    commands.spawn((
                        common,
                        Mesh3d(meshes.add(cone_mesh())),
                        MeshMaterial3d(shafts.add(shaft_material(&lit.spec, environment.beams))),
                    ));
                }
                Part::Motes => {
                    let spec = lit.spec.clone();
                    let salt = owner.index_u32();
                    let mesh =
                        motes_mesh(spec.beam.motes.count, salt, |seed| mote_start(seed, &spec));
                    commands.spawn((
                        common,
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(motes.add(mote_material(&lit.spec, fog))),
                        // Motes wander past where they started.
                        NoFrustumCulling,
                    ));
                }
            }
        }
    }
}

/// The fog's height dust around the world camera.
#[derive(Component)]
pub struct HeightDust {
    motes: Motes,
}

#[allow(clippy::type_complexity)]
fn sync_dust(
    mut commands: Commands,
    engine: NonSend<Engine>,
    environment: Res<Environment>,
    cameras: Query<&GlobalTransform, With<WorldCamera>>,
    mut dust: Query<(
        Entity,
        &HeightDust,
        &MeshMaterial3d<MoteMaterial>,
        &mut Transform,
    )>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<MoteMaterial>>,
) {
    let fog = &engine.project.world.fog.volumetric;
    let wanted = fog.dust.enabled && fog.dust.count > 0;
    let camera = cameras
        .iter()
        .next()
        .map_or(Vec3::ZERO, GlobalTransform::translation);
    let next = dust_material(fog, &environment);
    let mut kept = false;
    for (entity, field, handle, mut transform) in &mut dust {
        if !wanted || kept || field.motes != fog.dust {
            commands.entity(entity).despawn();
            continue;
        }
        kept = true;
        // The shader wraps motes round the eye; this keeps the bounds there.
        transform.translation = camera;
        if materials.get(&handle.0) != Some(&next)
            && let Some(mut material) = materials.get_mut(&handle.0)
        {
            *material = next.clone();
        }
    }
    if wanted && !kept {
        let mesh = motes_mesh(fog.dust.count, 0x5eed, |seed| {
            (seed - Vec3::splat(0.5)) * DUST_BOX
        });
        commands.spawn((
            HeightDust {
                motes: fog.dust.clone(),
            },
            Additive,
            Transform::from_translation(camera),
            Visibility::Inherited,
            NotShadowCaster,
            NotShadowReceiver,
            NoFrustumCulling,
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(next)),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::fog::BeamMode;
    use blockloom_core::shader_lib;

    fn spot(density: f32) -> LightSpec {
        let mut spec = LightSpec {
            kind: LightKind::Spot,
            ..default()
        };
        spec.beam.density = density;
        spec
    }

    #[test]
    fn a_shaft_stands_in_only_without_good_froxels() {
        let mut fog = VolumetricFog::default();
        assert_eq!(parts(&spot(0.1), &fog), vec![Part::Shaft]);
        assert!(parts(&spot(0.0), &fog).is_empty());
        fog.enabled = true;
        assert!(parts(&spot(0.1), &fog).is_empty());
        let mut forced = spot(0.1);
        forced.beam.mode = BeamMode::Shaft;
        assert_eq!(parts(&forced, &fog), vec![Part::Shaft]);
        // A point light has no cone to draw.
        let point = LightSpec {
            beam: forced.beam.clone(),
            ..default()
        };
        assert!(parts(&point, &fog).is_empty());
        let mut dusty = point;
        dusty.beam.motes.enabled = true;
        assert_eq!(parts(&dusty, &fog), vec![Part::Motes]);
    }

    #[test]
    fn a_shaft_vanishes_with_the_air() {
        let spec = spot(0.1);
        let lit = shaft_material(&spec, 1.0);
        let thick = shaft_material(&spec, 2.0);
        assert!(lit.color.x > 0.0);
        assert!((thick.color.x - 2.0 * lit.color.x).abs() < 1e-4);
        assert_eq!(shaft_material(&spec, 0.0).color.truncate(), Vec3::ZERO);
        assert_eq!(lit.beam.x, spec.range);
    }

    #[test]
    fn motes_start_inside_their_beam() {
        let mut spec = spot(0.1);
        spec.range = 10.0;
        let tan = spec.cone().1.tan();
        for k in 0..200 {
            let p = mote_start(mote_seed(k, 7), &spec);
            let along = -p.z;
            assert!((0.0..=10.0).contains(&along), "{p}");
            assert!(p.truncate().length() <= tan * along + 1e-4, "{p}");
        }
        let seeds: HashSet<u32> = (0..100).map(|k| mote_seed(k, 1).x.to_bits()).collect();
        assert!(seeds.len() > 95);
    }

    #[test]
    fn meshes_hold_what_the_shaders_read() {
        let mesh = motes_mesh(10, 1, |seed| seed);
        assert_eq!(mesh.count_vertices(), 40);
        assert!(mesh.attribute(Mesh::ATTRIBUTE_COLOR).is_some());
        assert_eq!(motes_mesh(1_000_000, 1, |s| s).count_vertices(), 4 * 4096);
        let cone = cone_mesh();
        assert!(cone.attribute(Mesh::ATTRIBUTE_NORMAL).is_some());
        assert_eq!(
            cone.count_vertices() as u32,
            (CONE_RINGS + 1) * (CONE_SEGMENTS + 1)
        );
    }

    #[test]
    fn dust_follows_the_sun_and_its_height() {
        let fog = VolumetricFog {
            base_height: 2.0,
            dust_height: 3.0,
            ..VolumetricFog::default()
        };
        let env = Environment::default();
        let dust = dust_material(&fog, &env);
        assert_eq!(dust.shape.w, 2.0);
        assert_eq!(dust.sun.w, 5.0);
        assert_eq!(dust.ambient.w, 2.0);
        assert!(dust.color.x > 0.0);
    }

    // Bevy's modules only exist on the GPU side, so stand in for what the
    // two materials take from them.
    const BEVY_STUB: &str = "\
struct View { world_from_view: mat4x4<f32>, world_position: vec3<f32>, exposure: f32 }\n\
struct Globals { time: f32 }\n\
@group(0) @binding(0) var<uniform> view: View;\n\
@group(0) @binding(11) var<uniform> globals: Globals;\n\
struct Vertex { @builtin(instance_index) instance_index: u32, @location(0) position: vec3<f32>, \
@location(2) uv: vec2<f32>, @location(5) color: vec4<f32> }\n\
struct VertexOutput { @builtin(position) position: vec4<f32>, @location(0) world_position: vec4<f32>, \
@location(1) world_normal: vec3<f32>, @location(2) uv: vec2<f32>, @location(5) color: vec4<f32> }\n\
fn decompress_vertex(v: Vertex, i: u32) -> Vertex { return v; }\n\
fn get_world_from_local(i: u32) -> mat4x4<f32> { return mat4x4<f32>(); }\n\
fn position_world_to_clip(p: vec3<f32>) -> vec4<f32> { return vec4<f32>(p, 1.0); }\n";

    fn stubbed(source: &str) -> String {
        let start = source.find("import bevy_pbr::").unwrap();
        let end = start + source[start..].find("};").unwrap() + 2;
        let mut out = source.to_string();
        out.replace_range(start..end, "");
        out.replace("constants::MATERIAL_BIND_GROUP", "2") + BEVY_STUB
    }

    #[test]
    fn the_beam_shaders_compile() {
        for source in [
            include_str!("shaders/beam_shaft.wesl"),
            include_str!("shaders/beam_motes.wesl"),
        ] {
            shader_lib::validate(&stubbed(source), &[]).unwrap_or_else(|error| panic!("{error}"));
        }
    }
}
