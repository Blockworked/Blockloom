//! The 3D surface: one mesh per body, displaced by `water_surface.wesl`
//! with the very waves `sample_water` resolved. A lake or a river is a grid
//! over its rectangle; an ocean is a disc of rings round the camera out to
//! `OCEAN_REACH`. Each body also gets a floor at its depth, which is what
//! the depth buffer and every pass after the water see where the surface is
//! (the surface itself draws in Bevy's transmissive pass, after the depth
//! prepass), and a probe or a mirror camera for reflections. The world
//! camera's depth becomes bindable while water exists, so the fog pass can
//! measure to the surface rather than the floor under it.

use super::mirror::{Mirrors, Want};
use super::{LiveBody, WaterState, rgb};
use crate::engine::Engine;
use crate::environment::Environment;
use crate::probes::{ProbeId, ProbeRequest, ProbeService};
use crate::world::WorldCamera;
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, ScreenSpaceTransmission};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType, TextureUsages};
use bevy::shader::ShaderRef;
use blockloom_core::material::{SurfaceMaterial, TextureSampler};
use blockloom_core::water::{DETAIL_WAVES, MAX_WAVES, OCEAN_REACH, WaterKind};
use std::collections::HashMap;

pub type WaterMaterial = ExtendedMaterial<StandardMaterial, WaterExtension>;

pub fn register(app: &mut App) {
    app.add_plugins(MaterialPlugin::<WaterMaterial>::default())
        .init_resource::<Surfaces>()
        .init_resource::<Mirrors>()
        .add_systems(
            Update,
            (drive_mirrors, sync_surfaces)
                .chain()
                .after(crate::world::drive_camera)
                .after(crate::edit::apply_view),
        );
}

/// On a body's drawn surface, which is its own entity rather than the
/// actor's child so an ocean can follow the camera.
#[derive(Component, Debug, Clone)]
pub struct WaterSurface;

/// `WaterWaves` in `blockloom::water`, field for field.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct WavesUniform {
    pub shape: [Vec4; MAX_WAVES],
    pub size: [Vec4; MAX_WAVES],
    pub detail_shape: [Vec4; DETAIL_WAVES],
    pub detail_size: [Vec4; DETAIL_WAVES],
    pub counts: Vec4,
    pub flow: Vec4,
    pub field: Vec4,
}

impl WavesUniform {
    /// A body's waves at render time `time`, its ripples `between` their
    /// last two ticks.
    pub fn of(live: &LiveBody, time: f32, between: f32) -> Self {
        let body = &live.body;
        let mut out = WavesUniform::default();
        for (k, wave) in body.waves.iter().take(MAX_WAVES).enumerate() {
            out.shape[k] = Vec4::new(wave.dir[0], wave.dir[1], wave.k, wave.omega);
            out.size[k] = Vec4::new(wave.phase, wave.amplitude, wave.horizontal, 0.0);
        }
        for (k, wave) in live.detail.iter().take(DETAIL_WAVES).enumerate() {
            out.detail_shape[k] = Vec4::new(wave.dir[0], wave.dir[1], wave.k, wave.omega);
            out.detail_size[k] = Vec4::new(wave.phase, wave.slope, 0.0, 0.0);
        }
        out.counts = Vec4::new(
            body.waves.len().min(MAX_WAVES) as f32,
            if body.ripples.is_some() { 1.0 } else { 0.0 },
            time,
            if body.flat { 1.0 } else { 0.0 },
        );
        out.flow = Vec4::new(body.flow[0], body.flow[1], body.center[1], 0.0);
        if let Some(field) = &body.ripples {
            out.field = Vec4::new(field.origin[0], field.origin[1], field.cell, between);
        }
        out
    }
}

/// `WaterLook` in `water_surface.wesl`, field for field.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct LookUniform {
    pub shallow: Vec4,
    pub deep: Vec4,
    pub foam_color: Vec4,
    pub foam: Vec4,
    pub optics: Vec4,
    pub modes: Vec4,
    pub probe: Vec4,
    pub sky: Vec4,
    pub under: Vec4,
    pub mirror: Vec4,
}

impl LookUniform {
    pub fn of(
        live: &LiveBody,
        env: &Environment,
        probe: Option<Vec3>,
        caustics: bool,
        mirror: bool,
    ) -> Self {
        let spec = &live.spec;
        let look = &spec.look;
        let foam = &spec.foam;
        let reflections = &spec.reflections;
        let sky = Vec4::from(env.background.to_linear().to_f32_array()).truncate();
        LookUniform {
            shallow: rgb(&look.shallow).extend(look.clarity),
            deep: rgb(&look.deep).extend(look.absorption),
            foam_color: rgb(&foam.color).extend(live.foam()),
            foam: Vec4::new(foam.shore, foam.crest, foam.scale, foam.drift),
            optics: Vec4::new(
                look.refraction,
                look.roughness,
                look.glint,
                reflections.strength,
            ),
            modes: Vec4::new(
                if reflections.mode.screen_space() {
                    1.0
                } else {
                    0.0
                },
                if probe.is_some() { 1.0 } else { 0.0 },
                spec.underwater.caustics,
                spec.underwater.caustics_scale,
            ),
            probe: probe
                .unwrap_or_default()
                .extend(if caustics { 1.0 } else { 0.0 }),
            sky: sky.extend(spec.waves.wavelength * 4.0),
            under: (rgb(&spec.underwater.fog) * super::under::water_light(env))
                .extend(spec.underwater.distance),
            mirror: Vec4::new(
                if mirror { 1.0 } else { 0.0 },
                // Rough water smears its reflection.
                look.roughness * look.roughness * 0.25,
                0.0,
                0.0,
            ),
        }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct WaterExtension {
    #[uniform(100)]
    pub waves: WavesUniform,
    #[uniform(101)]
    pub look: LookUniform,
    #[texture(102)]
    #[sampler(103)]
    pub face_px: Option<Handle<Image>>,
    #[texture(104)]
    pub face_nx: Option<Handle<Image>>,
    #[texture(105)]
    pub face_py: Option<Handle<Image>>,
    #[texture(106)]
    pub face_ny: Option<Handle<Image>>,
    #[texture(107)]
    pub face_pz: Option<Handle<Image>>,
    #[texture(108)]
    pub face_nz: Option<Handle<Image>>,
    #[texture(109)]
    #[sampler(110)]
    pub caustics: Option<Handle<Image>>,
    #[texture(111, filterable = false)]
    pub ripples: Option<Handle<Image>>,
    #[texture(112)]
    #[sampler(113)]
    pub mirror: Option<Handle<Image>>,
}

impl MaterialExtension for WaterExtension {
    fn vertex_shader() -> ShaderRef {
        crate::materials::water_shader()
    }

    fn fragment_shader() -> ShaderRef {
        crate::materials::water_shader()
    }

    // The surface draws after the prepass by design: what is under it has
    // to be in the depth buffer, not the water.
    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }
}

/// Every drawn body, by actor id.
#[derive(Resource, Default)]
pub struct Surfaces(HashMap<String, Surface>);

/// Keeps a mirror camera for every body that reflects with one.
#[allow(clippy::type_complexity)]
fn drive_mirrors(
    scaling: Option<Res<crate::quality::Scaling>>,
    mut commands: Commands,
    state: Res<WaterState>,
    surfaces: Res<Surfaces>,
    mut mirrors: ResMut<Mirrors>,
    environment: Res<Environment>,
    mut images: ResMut<Assets<Image>>,
    world: Query<(&Camera, &Transform, &Projection, Has<crate::sky::SkyView>), With<WorldCamera>>,
    mut cameras: Query<
        (
            &mut Camera,
            &mut Transform,
            &mut Projection,
            &mut bevy::camera::Exposure,
        ),
        Without<WorldCamera>,
    >,
) {
    let max_scale = world
        .iter()
        .next()
        .and_then(|(camera, ..)| camera.physical_viewport_size())
        .map_or(1.0, |size| {
            scaling.as_ref().map_or(1.0, |s| {
                s.budget().reflection as f32 / size.max_element().max(1) as f32
            })
        });
    let wants = state
        .bodies
        .iter()
        .filter(|live| live.spec.reflections.mode.planar())
        .filter_map(|live| {
            Some(Want {
                id: live.body.id.clone(),
                level: live.body.center[1],
                scale: live.spec.reflections.planar_scale.min(max_scale),
                hide: surfaces.0.get(&live.body.id)?.entity,
            })
        })
        .collect();
    super::mirror::sync_mirrors(
        &mut commands,
        &mut mirrors,
        wants,
        world.iter().next(),
        &environment,
        &mut images,
        &mut cameras,
    );
}

struct Surface {
    entity: Entity,
    floor: Entity,
    floor_material: Handle<StandardMaterial>,
    material: Handle<WaterMaterial>,
    shape: Shape,
    probe: Option<(ProbeId, ProbeRequest)>,
    caustics: (String, Option<Handle<Image>>),
    ripples: Option<(u64, Handle<Image>)>,
}

/// What a body's mesh was built for.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    Grid { half: [f32; 2], cells: [u32; 2] },
    Ocean { spacing: f32 },
}

impl Shape {
    fn of(live: &LiveBody) -> Self {
        // Around ten vertices along the longest wave.
        let spacing = (live.spec.waves.wavelength / 10.0).clamp(0.05, 4.0);
        match live.body.kind {
            WaterKind::Ocean => Shape::Ocean { spacing },
            _ => {
                let half = live.body.half;
                let cells = half.map(|h| ((2.0 * h / spacing).ceil() as u32).clamp(16, 256));
                Shape::Grid { half, cells }
            }
        }
    }

    fn mesh(self) -> Mesh {
        match self {
            Shape::Grid { half, cells } => grid_mesh(half, cells),
            Shape::Ocean { spacing } => ocean_mesh(spacing, 256),
        }
    }

    fn floor(self) -> Mesh {
        let half = match self {
            Shape::Grid { half, .. } => Vec2::from(half),
            Shape::Ocean { .. } => Vec2::splat(OCEAN_REACH),
        };
        Plane3d::new(Vec3::Y, half).mesh().build()
    }
}

fn mesh_from(positions: Vec<[f32; 3]>, indices: Vec<u32>) -> Mesh {
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    let uvs: Vec<[f32; 2]> = positions.iter().map(|p| [p[0], p[2]]).collect();
    // The second uv set carries how calm the far waves are (vertex shader).
    let calm = vec![[1.0, 0.0]; positions.len()];
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, calm)
    .with_inserted_indices(Indices::U32(indices))
}

/// A flat grid `cells` across a rectangle of half extents `half`, in the
/// body's own frame.
fn grid_mesh(half: [f32; 2], cells: [u32; 2]) -> Mesh {
    let [nx, nz] = cells;
    let mut positions = Vec::with_capacity(((nx + 1) * (nz + 1)) as usize);
    for j in 0..=nz {
        for i in 0..=nx {
            positions.push([
                -half[0] + 2.0 * half[0] * i as f32 / nx as f32,
                0.0,
                -half[1] + 2.0 * half[1] * j as f32 / nz as f32,
            ]);
        }
    }
    let mut indices = Vec::with_capacity((nx * nz * 6) as usize);
    for j in 0..nz {
        for i in 0..nx {
            let a = j * (nx + 1) + i;
            let b = a + nx + 1;
            indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    mesh_from(positions, indices)
}

/// Rings round the origin: evenly `spacing` apart out to where a ring's
/// segments are that long, then growing so every cell stays about square,
/// out to `OCEAN_REACH`.
fn ocean_mesh(spacing: f32, segments: u32) -> Mesh {
    let step = std::f32::consts::TAU / segments as f32;
    let inner = spacing / step;
    let mut radii = Vec::new();
    let mut r = spacing;
    while r < OCEAN_REACH {
        radii.push(r);
        r = if r < inner {
            r + spacing
        } else {
            r * (1.0 + step)
        };
    }
    radii.push(OCEAN_REACH);
    let mut positions = vec![[0.0, 0.0, 0.0]];
    for &r in &radii {
        for s in 0..segments {
            let a = s as f32 * step;
            positions.push([r * a.cos(), 0.0, r * a.sin()]);
        }
    }
    let ring = |k: u32, s: u32| 1 + k * segments + s % segments;
    let mut indices = Vec::new();
    for s in 0..segments {
        indices.extend([0, ring(0, s + 1), ring(0, s)]);
    }
    for k in 0..radii.len() as u32 - 1 {
        for s in 0..segments {
            let (a, b) = (ring(k, s), ring(k, s + 1));
            let (c, d) = (ring(k + 1, s), ring(k + 1, s + 1));
            indices.extend([a, b, c, b, d, c]);
        }
    }
    mesh_from(positions, indices)
}

/// Where an ocean's mesh centres for a camera at `eye`: snapped so the
/// rings don't swim under a moving camera.
fn ocean_centre(eye: Vec3, spacing: f32) -> Vec2 {
    let snap = spacing * 4.0;
    (Vec2::new(eye.x, eye.z) / snap).round() * snap
}

/// The time the surfaces are drawn at: the fixed tick's water time plus
/// how far the frame is into the next tick, so waves move smoothly on any
/// display. A stopped world's water still moves in the editor.
fn render_time(engine: &Engine, state: &WaterState, fixed: &Time<Fixed>, time: &Time) -> f32 {
    match (engine.running, engine.paused) {
        (true, false) => state.time + fixed.overstep_fraction() * fixed.delta_secs(),
        (true, true) => state.time,
        (false, _) => time.elapsed_secs() % 3600.0,
    }
}

/// Spawns, reshapes, moves and feeds every body's surface, and turns on the
/// camera's transmission pass while there is any.
#[allow(clippy::too_many_arguments)]
fn sync_surfaces(
    scaling: Option<Res<crate::quality::Scaling>>,
    mut commands: Commands,
    engine: NonSend<Engine>,
    state: Res<WaterState>,
    (fixed, time): (Res<Time<Fixed>>, Res<Time>),
    environment: Res<Environment>,
    mut cameras: Query<
        (
            Entity,
            &GlobalTransform,
            Has<ScreenSpaceTransmission>,
            &mut Camera3d,
        ),
        With<WorldCamera>,
    >,
    (mut surfaces, mirrors): (ResMut<Surfaces>, Res<Mirrors>),
    mut transforms: Query<&mut Transform, With<WaterSurface>>,
    mut meshes: ResMut<Assets<Mesh>>,
    (mut materials, mut plain, mut images): (
        ResMut<Assets<WaterMaterial>>,
        ResMut<Assets<StandardMaterial>>,
        ResMut<Assets<Image>>,
    ),
    mut probes: Option<ResMut<ProbeService>>,
    assets: Res<AssetServer>,
) {
    let eye = cameras
        .iter()
        .next()
        .map_or(Vec3::ZERO, |(_, camera, ..)| camera.translation());
    let any = !state.bodies.is_empty();
    for (camera, _, has, mut camera3d) in &mut cameras {
        if any && !has {
            commands
                .entity(camera)
                .insert(ScreenSpaceTransmission::default());
        } else if !any && has {
            commands.entity(camera).remove::<ScreenSpaceTransmission>();
        }
        // The fog pass reads this depth, which has the surface in it.
        let usages = TextureUsages::from(camera3d.depth_texture_usages);
        let wanted = if any {
            usages | TextureUsages::TEXTURE_BINDING
        } else {
            usages - TextureUsages::TEXTURE_BINDING
        };
        if wanted != usages {
            camera3d.depth_texture_usages = wanted.into();
        }
    }
    let now = render_time(&engine, &state, &fixed, &time);
    let surfaces = &mut surfaces.0;
    surfaces.retain(|id, surface| {
        let keep = state.body(id).is_some();
        if !keep {
            commands.entity(surface.entity).despawn();
            if let (Some(probes), Some((probe, _))) = (probes.as_deref_mut(), &surface.probe) {
                probes.cancel(*probe);
            }
        }
        keep
    });
    for live in &state.bodies {
        let body = &live.body;
        let shape = Shape::of(live);
        let deep = rgb(&live.spec.look.deep);
        let surface = surfaces.entry(body.id.clone()).or_insert_with(|| {
            let material = materials.add(new_material());
            let floor_material = plain.add(floor_material(deep));
            let floor = commands
                .spawn((
                    Mesh3d(meshes.add(shape.floor())),
                    MeshMaterial3d(floor_material.clone()),
                    Transform::default(),
                    NotShadowCaster,
                ))
                .id();
            let entity = commands
                .spawn((
                    WaterSurface,
                    Mesh3d(meshes.add(shape.mesh())),
                    MeshMaterial3d(material.clone()),
                    Transform::default(),
                    Visibility::default(),
                    NoFrustumCulling,
                    NotShadowCaster,
                    Name::new(format!("water {}", body.id)),
                ))
                .add_child(floor)
                .id();
            Surface {
                entity,
                floor,
                floor_material,
                material,
                shape,
                probe: None,
                caustics: (String::new(), None),
                ripples: None,
            }
        });
        if surface.shape != shape {
            surface.shape = shape;
            commands
                .entity(surface.entity)
                .insert(Mesh3d(meshes.add(shape.mesh())));
            commands
                .entity(surface.floor)
                .insert(Mesh3d(meshes.add(shape.floor())));
        }
        let level = body.center[1];
        let (centre, yaw) = match shape {
            Shape::Ocean { spacing } => (ocean_centre(eye, spacing), 0.0),
            Shape::Grid { .. } => (
                Vec2::new(body.center[0], body.center[2]),
                (-body.axis[1]).atan2(body.axis[0]),
            ),
        };
        if let Ok(mut transform) = transforms.get_mut(surface.entity) {
            *transform = Transform::from_xyz(centre.x, level, centre.y)
                .with_rotation(Quat::from_rotation_y(yaw));
        }
        commands
            .entity(surface.floor)
            .insert(Transform::from_xyz(0.0, -body.depth, 0.0));

        // Reflections: a probe hovering over the surface, following the
        // camera over an ocean.
        let reflections = &live.spec.reflections;
        let wants_probe = reflections.mode.probe();
        let above = live.spec.waves.amplitude * 2.0 + 0.5;
        let spot = Vec3::new(centre.x, level + above, centre.y);
        let mut faces = None;
        if let Some(probes) = probes.as_deref_mut() {
            let request = ProbeRequest {
                hide: Some(surface.entity),
                ..ProbeRequest::water(
                    spot,
                    reflections
                        .probe_resolution
                        .min(scaling.as_ref().map_or(u32::MAX, |s| s.budget().reflection)),
                    reflections.probe_refresh.max(
                        scaling
                            .as_ref()
                            .map_or(1, |s| (8.0 / s.budget().distance) as u32),
                    ),
                )
            };
            let stale = surface.probe.as_ref().is_some_and(|(_, asked)| {
                asked.resolution != request.resolution || asked.refresh != request.refresh
            });
            if (stale || !wants_probe)
                && let Some((probe, _)) = surface.probe.take()
            {
                probes.cancel(probe);
            }
            if wants_probe {
                let (probe, _) = surface
                    .probe
                    .get_or_insert_with(|| (probes.request(request.clone()), request));
                probes.relocate(*probe, spot);
                faces = probes.faces(*probe).cloned();
            }
        }

        let path = live.spec.underwater.caustics_texture.trim();
        if surface.caustics.0 != path {
            let tiled = SurfaceMaterial {
                sampler: TextureSampler::Repeat,
                ..default()
            };
            let handle = crate::materials::load_surface_image(
                &mut commands,
                path,
                &tiled,
                engine.project_dir.as_deref(),
                &assets,
                false,
            );
            surface.caustics = (path.to_string(), handle);
        }
        let caustics = surface.caustics.1.clone();
        let ripples = super::sync_ripple_image(&mut surface.ripples, live, state.tick, &mut images);
        let mirror = (live.spec.reflections.mode.planar()
            && super::mirror::showing(&mirrors, &body.id, eye, level))
        .then(|| mirrors.image(&body.id))
        .flatten();
        let Some(mut material) = materials.get_mut(&surface.material) else {
            continue;
        };
        let extension = &mut material.extension;
        extension.waves = WavesUniform::of(live, now, super::between_ticks(&engine, &fixed));
        extension.look = LookUniform::of(
            live,
            &environment,
            faces.as_ref().map(|_| spot),
            caustics.is_some(),
            mirror.is_some(),
        );
        if extension.ripples != ripples {
            extension.ripples = ripples;
        }
        if extension.mirror != mirror {
            extension.mirror = mirror;
        }
        let [px, nx, py, ny, pz, nz] = faces.map_or(Default::default(), |faces| faces.map(Some));
        extension.face_px = px;
        extension.face_nx = nx;
        extension.face_py = py;
        extension.face_ny = ny;
        extension.face_pz = pz;
        extension.face_nz = nz;
        if extension.caustics != caustics {
            extension.caustics = caustics;
        }
        let floor_color = Color::linear_rgb(deep.x, deep.y, deep.z);
        if let Some(mut floor) = plain.get_mut(&surface.floor_material)
            && floor.base_color != floor_color
        {
            floor.base_color = floor_color;
        }
    }
}

fn new_material() -> WaterMaterial {
    WaterMaterial {
        base: StandardMaterial {
            // Any transmission puts it in the transmissive pass, which is
            // what hands the shader the opaque frame to refract.
            specular_transmission: 1.0,
            double_sided: true,
            cull_mode: None,
            perceptual_roughness: 0.1,
            reflectance: 0.0,
            opaque_render_method: bevy::material::OpaqueRendererMethod::Forward,
            ..default()
        },
        extension: WaterExtension {
            waves: WavesUniform::default(),
            look: LookUniform::default(),
            face_px: None,
            face_nx: None,
            face_py: None,
            face_ny: None,
            face_pz: None,
            face_nz: None,
            caustics: None,
            ripples: None,
            mirror: None,
        },
    }
}

fn floor_material(deep: Vec3) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::linear_rgb(deep.x, deep.y, deep.z),
        perceptual_roughness: 1.0,
        reflectance: 0.1,
        ..default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::shader_lib;

    #[test]
    fn a_grid_spans_its_rectangle() {
        let mesh = grid_mesh([10.0, 5.0], [4, 2]);
        assert_eq!(mesh.count_vertices(), 15);
        assert_eq!(mesh.indices().unwrap().len(), 4 * 2 * 6);
    }

    #[test]
    fn the_ocean_reaches_the_horizon() {
        let mesh = ocean_mesh(1.0, 64);
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("positions");
        };
        let far = positions
            .iter()
            .map(|p| Vec2::new(p[0], p[2]).length())
            .fold(0.0, f32::max);
        assert!((far - OCEAN_REACH).abs() < 1.0);
        let max_index = mesh.indices().unwrap().iter().max().unwrap();
        assert!(max_index < positions.len());
    }

    #[test]
    fn water_uniforms_match_the_wesl_layout() {
        assert_eq!(
            WavesUniform::min_size().get() as usize,
            (2 * MAX_WAVES + 2 * DETAIL_WAVES + 3) * 16
        );
        assert_eq!(LookUniform::min_size().get(), 10 * 16);
    }

    // Enough of Bevy's modules for naga to check the surface shader.
    const STUB: &str = "struct View { world_position: vec3<f32>, viewport: vec4<f32>, clip_from_view: mat4x4<f32>, exposure: f32 }\n\
@group(0) @binding(0) var<uniform> view: View;\n\
struct Lights { ambient_color: vec4<f32> }\n\
@group(0) @binding(1) var<uniform> lights: Lights;\n\
@group(0) @binding(20) var depth_prepass_texture: texture_depth_2d;\n\
@group(0) @binding(24) var view_transmission_texture: texture_2d<f32>;\n\
@group(0) @binding(25) var view_transmission_sampler: sampler;\n\
struct Vertex { @builtin(instance_index) instance_index: u32, @location(0) position: vec3<f32>, \
@location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>, @location(3) uv_b: vec2<f32> }\n\
struct VertexOutput { @builtin(position) position: vec4<f32>, @location(0) world_position: vec4<f32>, \
@location(1) world_normal: vec3<f32>, @location(2) uv: vec2<f32>, @location(3) uv_b: vec2<f32>, \
@location(6) @interpolate(flat) instance_index: u32 }\n\
struct FragmentOutput { @location(0) color: vec4<f32> }\n\
struct StandardMaterial { base_color: vec4<f32>, perceptual_roughness: f32, metallic: f32, reflectance: vec3<f32> }\n\
struct PbrInput { material: StandardMaterial, frag_coord: vec4<f32>, world_position: vec4<f32>, world_normal: vec3<f32>, N: vec3<f32>, V: vec3<f32>, is_orthographic: bool }\n\
fn pbr_input_new() -> PbrInput { var p: PbrInput; return p; }\n\
fn apply_pbr_lighting(p: PbrInput) -> vec4<f32> { return p.material.base_color; }\n\
fn main_pass_post_lighting_processing(p: PbrInput, c: vec4<f32>) -> vec4<f32> { return c; }\n\
fn decompress_vertex(v: Vertex, i: u32) -> Vertex { return v; }\n\
fn get_world_from_local(i: u32) -> mat4x4<f32> { return mat4x4<f32>(); }\n\
fn position_world_to_clip(p: vec3<f32>) -> vec4<f32> { return vec4<f32>(p, 1.0); }\n\
fn position_world_to_ndc(p: vec3<f32>) -> vec3<f32> { return p; }\n\
fn position_ndc_to_world(p: vec3<f32>) -> vec3<f32> { return p; }\n\
fn ndc_to_uv(p: vec2<f32>) -> vec2<f32> { return p; }\n\
fn uv_to_ndc(p: vec2<f32>) -> vec2<f32> { return p; }\n";

    #[test]
    fn the_surface_shader_compiles() {
        let source =
            crate::materials::tests::stubbed(include_str!("../shaders/water_surface.wesl"), STUB)
                .replace("view_bindings::", "")
                .replace(
                    "import bevy_pbr::render::mesh_view_bindings as view_bindings;",
                    "",
                );
        for (transmission, depth) in [(true, true), (false, false)] {
            shader_lib::validate(
                &source,
                &[
                    ("VERTEX_UVS_A", true),
                    ("VERTEX_UVS_B", true),
                    ("VERTEX_NORMALS", true),
                    ("VERTEX_OUTPUT_INSTANCE_INDEX", true),
                    ("VIEW_TRANSMISSION_TEXTURE", transmission),
                    ("DEPTH_PREPASS", depth),
                    ("MULTISAMPLED", false),
                ],
            )
            .unwrap_or_else(|error| panic!("{error}"));
        }
    }
}
