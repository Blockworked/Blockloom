//! The 3D sky (`blockloom_core::sky`): the background, the ambient light and
//! the reflections, all from one source.
//!
//! Whenever the sky changes, a compute pass (`shaders/sky_cube.wesl`) writes
//! its light into two cubes: a full one and a small one scaled by the ambient
//! dimmer. Bevy's generated environment light filters them on two helper
//! entities - reflections from the full one, diffuse from the small one - and
//! the world camera takes one map from each, or a black cube for whichever
//! the sky doesn't give. Filtering stops once the cubes have settled, so a
//! still sky is filtered once rather than every frame.
//!
//! The background is drawn per pixel in Bevy's opaque pass, where its skybox
//! would go (`shaders/sky_background.wesl`), so gradients dither, the sun and
//! moon stay sharp and an HDRI can be blurred by the filtered light's mips.
//! An HDRI's file loads on a background task; a build ships it baked to a
//! BC6H cube with its mips (see `blockloom_core::build`).

use crate::engine::Engine;
use crate::environment::Environment;
use crate::space::{SpaceParams, SpaceRender};
use crate::world::{WorldCamera, parse_color};
use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::core_3d::CORE_3D_DEPTH_FORMAT;
use bevy::core_pipeline::skybox::{SkyboxBindGroup, SkyboxPipelineId};
use bevy::light::{EnvironmentMapLight, GeneratedEnvironmentMapLight};
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{
    self, texture_2d, texture_cube, texture_storage_2d_array, uniform_buffer,
};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue};
use bevy::render::sync_component::{SyncComponent, SyncComponentPlugin};
use bevy::render::sync_world::RenderEntity;
use bevy::render::texture::{FallbackImage, GpuImage};
use bevy::render::view::{ExtractedView, Msaa, ViewUniform, ViewUniforms};
use bevy::render::{
    Extract, ExtractSchedule, GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems,
};
use bevy::shader::Shader;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use blockloom_core::pipeline::{self, bc6h, hdr};
use blockloom_core::sky::{PhysicalSky, Sky, SkyKind};
use blockloom_protocol::RuntimeMessage;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Cubes hold radiance over this many nits; `SKY_UNIT` in `shaders/sky.wesl`.
pub const SKY_UNIT: f32 = 1000.0;
/// The light cube's face.
const LIGHT_FACE: u32 = 256;
/// The diffuse cube's, `DIFFUSE_RATIO` smaller in `shaders/sky_cube.wesl`.
const DIFFUSE_FACE: u32 = LIGHT_FACE / 8;
/// Frames the filter keeps running after the cubes were last written.
const SETTLE_FRAMES: u32 = 4;

const KIND_PHYSICAL: u32 = 1;
const KIND_GRADIENT: u32 = 2;
const KIND_HDRI: u32 = 3;
const FLAG_DISKS: u32 = 1;
const FLAG_MOON: u32 = 2;
const FLAG_DITHER: u32 = 4;

pub fn register(app: &mut App) {
    app.init_resource::<SkyState>().add_systems(
        Update,
        (load_hdri, resolve_sky, apply_sky)
            .chain()
            .after(crate::environment::blend_environment),
    );
    if app.get_sub_app(RenderApp).is_none() {
        return;
    }
    bevy::asset::embedded_asset!(app, "shaders/sky_cube.wesl");
    bevy::asset::embedded_asset!(app, "shaders/sky_background.wesl");
    app.add_plugins((
        ExtractResourcePlugin::<SkyRender>::default(),
        SyncComponentPlugin::<SkyView, SkyPlugin>::default(),
    ));
    let render = app.get_sub_app_mut(RenderApp).unwrap();
    render
        .init_gpu_resource::<SpecializedRenderPipelines<SkyBackground>>()
        .init_resource::<SkyUniforms>()
        .add_systems(ExtractSchedule, extract_sky_views)
        .add_systems(RenderStartup, init_pipelines)
        .add_systems(
            Render,
            (
                prepare_sky_views.in_set(RenderSystems::PrepareBindGroups),
                write_cubes
                    .after(RenderSystems::PrepareBindGroups)
                    .before(bevy::pbr::generate::downsampling_system)
                    .before(RenderSystems::Render),
            ),
        );
}

/// Marker for the sky's render-world plumbing.
pub struct SkyPlugin;

/// Draws the sky behind a camera. Probe faces leave the sun and moon out,
/// since the sun already lights what they capture.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct SkyView {
    pub disks: bool,
}

impl SyncComponent<RenderApp, SkyPlugin> for SkyView {
    type Target = (SkyView, SkyboxPipelineId, SkyboxBindGroup);
}

/// Which of the two filtered cubes a helper entity holds.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
enum SkyProbe {
    Light,
    Diffuse,
}

/// `SkyParams` in `shaders/sky.wesl`, field for field.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct SkyParams {
    pub info: UVec4,
    pub sun: Vec4,
    pub sun_dir: Vec4,
    pub sun_disk: Vec4,
    pub moon_dir: Vec4,
    pub moon: Vec4,
    pub rayleigh: Vec4,
    pub mie: Vec4,
    pub ozone: Vec4,
    pub planet: Vec4,
    pub ground: Vec4,
    pub night: Vec4,
    pub top: Vec4,
    pub middle: Vec4,
    pub bottom: Vec4,
    pub warm: Vec4,
    pub horizon: Vec4,
    pub hdri_x: Vec4,
    pub hdri_y: Vec4,
    pub hdri_z: Vec4,
    pub hdri: Vec4,
    pub scale: Vec4,
}

fn linear(hex: &str) -> Vec3 {
    let color = parse_color(hex).to_linear();
    Vec3::new(color.red, color.green, color.blue)
}

impl SkyParams {
    /// The sky as the passes read it, `None` for no sky. `hdri_mips` is the
    /// loaded image's mip count.
    pub fn resolve(sky: &Sky, env: &Environment, hdri_mips: u32) -> Option<SkyParams> {
        let kind = match sky.active_kind() {
            SkyKind::Flat => return None,
            SkyKind::Physical => KIND_PHYSICAL,
            SkyKind::Gradient => KIND_GRADIENT,
            SkyKind::Hdri => KIND_HDRI,
        };
        let sun = env.sun.direction;
        let mut params = SkyParams {
            sun: env.sun.above_air.extend(0.0),
            sun_dir: sun.extend(0.0),
            scale: Vec4::new(
                env.sky_exposure.exp2(),
                env.ambient_dimmer.max(0.0),
                0.0,
                0.0,
            ),
            hdri_x: Vec4::X,
            hdri_y: Vec4::Y,
            hdri_z: Vec4::Z,
            ..default()
        };
        let mut flags = 0;
        match kind {
            KIND_PHYSICAL => {
                let p = &sky.physical;
                let radius = (p.sun_size.to_radians() * 0.5).sin();
                let solid_angle = std::f32::consts::TAU * (1.0 - radius.asin().cos());
                let [r, g, b] = p.air_transmittance(sun.to_array());
                let disk = env.sun.above_air * Vec3::new(r, g, b) * p.sun_intensity
                    / solid_angle.max(1e-9);
                params.sun_dir.w = radius;
                params.sun_disk = disk.extend(p.limb_darkening);
                params.moon_dir = Vec3::from_array(sky.moon_direction())
                    .extend((p.moon_size.to_radians() * 0.5).sin());
                params.moon = Vec4::new(
                    p.moon_brightness,
                    p.moon_phase,
                    p.moon_halo,
                    p.moon_halo_power,
                );
                params.rayleigh = Vec3::from_array(p.rayleigh).extend(p.rayleigh_height);
                params.mie = Vec4::new(p.mie, p.mie_g, p.mie_height, p.horizon_curve);
                params.ozone = Vec3::from_array(p.ozone).extend(0.0);
                params.planet = Vec4::new(
                    p.planet_radius,
                    p.planet_radius + p.atmosphere_height,
                    PhysicalSky::EYE_HEIGHT,
                    p.horizon_sine(),
                );
                params.ground = linear(&p.ground_albedo).extend(0.0);
                params.night =
                    (linear(&p.night_color) * p.night_brightness * p.night(sun.to_array()))
                        .extend(0.0);
                if p.moon {
                    flags |= FLAG_MOON;
                }
            }
            KIND_GRADIENT => {
                let g = &sky.gradient;
                let nits = |hex: &str| (linear(hex) * g.brightness).extend(0.0);
                params.top = nits(&g.top);
                params.middle = nits(&g.middle);
                params.bottom = nits(&g.bottom);
                // Warmest with the sun low, gone by day and by night.
                let low = (1.0 - smoothstep(0.1, 0.5, sun.y)) * smoothstep(-0.2, 0.0, sun.y);
                params.warm = (linear(&g.warm_color) * g.brightness).extend(g.warmth * low);
                params.horizon = Vec4::new(g.horizon_offset, g.softness, 0.0, 0.0);
                if g.dither {
                    flags |= FLAG_DITHER;
                }
            }
            _ => {
                let h = &sky.hdri;
                // The image turned into the world, so the world is turned back
                // to look it up.
                let placed = Quat::from_rotation_y(-h.rotation.to_radians())
                    * Quat::from_rotation_x(h.tilt.to_radians());
                let lookup = Mat3::from_quat(placed.inverse());
                params.hdri_x = lookup.x_axis.extend(0.0);
                params.hdri_y = lookup.y_axis.extend(0.0);
                params.hdri_z = lookup.z_axis.extend(0.0);
                let light_mips = LIGHT_FACE.ilog2() as f32;
                params.hdri = (linear(&h.tint) * h.brightness).extend(h.blur * light_mips);
                params.scale.z = if h.blur > 0.0 { 1.0 } else { 0.0 };
            }
        }
        params.info = UVec4::new(kind, flags, LIGHT_FACE, hdri_mips.max(1));
        Some(params)
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// What an HDRI's cube was made from; a change starts a new load.
#[derive(Clone, Debug, PartialEq)]
struct HdriKey {
    dir: PathBuf,
    path: String,
    bias: f32,
    seam: f32,
}

impl HdriKey {
    fn of(engine: &Engine) -> Option<HdriKey> {
        let sky = &engine.project.world.sky;
        if sky.active_kind() != SkyKind::Hdri {
            return None;
        }
        let dir = engine.project_dir.clone()?;
        let path =
            blockloom_core::assets::normalize(&sky.hdri.path).filter(|path| !path.is_empty())?;
        let bias = pipeline::load_manifest(&dir).bias_of(&path);
        Some(HdriKey {
            dir,
            path,
            bias,
            seam: sky.hdri.seam_fix,
        })
    }
}

/// The cubes the sky is written into, and the helpers that filter them.
struct SkyCubes {
    light: Handle<Image>,
    diffuse: Handle<Image>,
    /// Stands in for a missing image and for light the sky doesn't give.
    black: Handle<Image>,
    light_probe: Entity,
    diffuse_probe: Entity,
}

#[derive(Resource, Default)]
pub struct SkyState {
    key: Option<HdriKey>,
    task: Option<Task<Result<Image, String>>>,
    hdri: Option<Handle<Image>>,
    cubes: Option<SkyCubes>,
    generation: u64,
    /// Frames since the render world caught up with `generation`.
    quiet: u32,
}

/// What the render world draws the sky from.
#[derive(Resource, Clone)]
pub struct SkyRender {
    pub params: Option<SkyParams>,
    pub light: Handle<Image>,
    pub diffuse: Handle<Image>,
    /// The HDRI, or black.
    pub source: Handle<Image>,
    /// The light cube's filtered reflections, once Bevy has made them.
    pub blurred: Option<Handle<Image>>,
    /// Bumped on every change of the above; the cubes are rewritten for it.
    pub generation: u64,
    /// The generation the cubes last were written for, set by the render world.
    pub written: Arc<AtomicU64>,
}

impl ExtractResource<RenderApp> for SkyRender {
    type Source = SkyRender;

    fn extract_resource(source: &Self::Source) -> Self {
        source.clone()
    }
}

/// Starts loading an HDRI when the sky names a new one, and picks the cube
/// up when its task lands.
fn load_hdri(
    engine: NonSend<Engine>,
    mut state: ResMut<SkyState>,
    mut images: ResMut<Assets<Image>>,
    device: Option<Res<RenderDevice>>,
) {
    let key = HdriKey::of(&engine);
    if key != state.key {
        state.hdri = None;
        state.task = key.clone().map(|key| {
            let compressed = device.as_ref().is_some_and(|device| {
                device
                    .features()
                    .contains(WgpuFeatures::TEXTURE_COMPRESSION_BC)
            });
            AsyncComputeTaskPool::get().spawn(async move { load(&key, compressed) })
        });
        state.key = key;
    }
    let Some(task) = state.task.as_mut() else {
        return;
    };
    let Some(result) = check_ready(task) else {
        return;
    };
    state.task = None;
    match result {
        Ok(image) => state.hdri = Some(images.add(image)),
        Err(error) => crate::bridge::send(&RuntimeMessage::Error {
            actor: "Blockloom".into(),
            message: format!("The sky didn't load: {error}"),
        }),
    }
}

/// Works out this frame's sky, bumps the generation when it changed, and
/// keeps the filter running only until the cubes have settled.
#[allow(clippy::too_many_arguments)]
fn resolve_sky(
    mut commands: Commands,
    engine: NonSend<Engine>,
    environment: Res<Environment>,
    mut state: ResMut<SkyState>,
    mut images: ResMut<Assets<Image>>,
    render: Option<ResMut<SkyRender>>,
    backlog: Option<Res<crate::streaming::PipelineBacklog>>,
    probes: Query<(
        &SkyProbe,
        Option<&EnvironmentMapLight>,
        Has<GeneratedEnvironmentMapLight>,
    )>,
) {
    let state = &mut *state;
    let cubes = state.cubes.get_or_insert_with(|| {
        let light = images.add(storage_cube(LIGHT_FACE, "sky_light"));
        let diffuse = images.add(storage_cube(DIFFUSE_FACE, "sky_diffuse"));
        let black = images.add(black_cube());
        let light_probe = commands
            .spawn((SkyProbe::Light, Name::new("sky light")))
            .id();
        let diffuse_probe = commands
            .spawn((SkyProbe::Diffuse, Name::new("sky diffuse")))
            .id();
        SkyCubes {
            light,
            diffuse,
            black,
            light_probe,
            diffuse_probe,
        }
    });
    let hdri_mips = state
        .hdri
        .as_ref()
        .and_then(|hdri| images.get(hdri))
        .map_or(1, |image| image.texture_descriptor.mip_level_count);
    let mut params = SkyParams::resolve(&engine.project.world.sky, &environment, hdri_mips);
    // An HDRI still loading is no sky yet.
    if params.is_some_and(|p| p.info.x == KIND_HDRI) && state.hdri.is_none() {
        params = None;
    }
    let source = state.hdri.clone().unwrap_or_else(|| cubes.black.clone());
    let blurred = probes
        .iter()
        .find(|(probe, ..)| **probe == SkyProbe::Light)
        .and_then(|(_, filtered, _)| filtered.map(|f| f.specular_map.clone()));

    let Some(mut render) = render else {
        let written = Arc::new(AtomicU64::new(0));
        commands.insert_resource(SkyRender {
            params,
            light: cubes.light.clone(),
            diffuse: cubes.diffuse.clone(),
            source,
            blurred,
            generation: 1,
            written,
        });
        state.generation = 1;
        return;
    };
    let changed = render.params != params || render.source != source;
    if changed {
        state.generation += 1;
        render.params = params;
        render.source = source;
        render.generation = state.generation;
    }
    if render.blurred != blurred {
        render.blurred = blurred;
    }

    let caught_up = render.written.load(Ordering::Relaxed) == state.generation;
    state.quiet = if caught_up && !changed {
        state.quiet.saturating_add(1)
    } else {
        0
    };
    let compiling = backlog.is_some_and(|backlog| backlog.get() > 0);
    let filtering = render.params.is_some() && (state.quiet < SETTLE_FRAMES || compiling);
    for (entity, cube) in [
        (cubes.light_probe, &cubes.light),
        (cubes.diffuse_probe, &cubes.diffuse),
    ] {
        let has = probes.get(entity).is_ok_and(|(_, _, generated)| generated);
        if filtering && !has {
            commands
                .entity(entity)
                .insert(GeneratedEnvironmentMapLight {
                    environment_map: cube.clone(),
                    intensity: SKY_UNIT,
                    ..default()
                });
        } else if !filtering && has {
            commands
                .entity(entity)
                .remove::<GeneratedEnvironmentMapLight>();
        }
    }
}

/// Keeps every world camera's sky and environment light matching the sky,
/// the light scaled by the volumes' `reflections`.
#[allow(clippy::type_complexity)]
fn apply_sky(
    mut commands: Commands,
    engine: NonSend<Engine>,
    state: Res<SkyState>,
    render: Option<Res<SkyRender>>,
    environment: Res<Environment>,
    traced_ambient: Option<Res<crate::ray_tracing::TracedAmbient>>,
    probes: Query<(&SkyProbe, &EnvironmentMapLight)>,
    cameras: Query<
        (Entity, Option<&SkyView>, Option<&EnvironmentMapLight>),
        (With<WorldCamera>, Without<SkyProbe>),
    >,
) {
    let (Some(render), Some(cubes)) = (render, state.cubes.as_ref()) else {
        return;
    };
    let sky = &engine.project.world.sky;
    let active = render.params.is_some();
    let wanted_view = (active && sky.background).then_some(SkyView { disks: true });
    let filtered = |which: SkyProbe| {
        probes
            .iter()
            .find(|(probe, _)| **probe == which)
            .map(|(_, light)| light)
    };
    let tracing = traced_ambient
        .as_ref()
        .is_some_and(|ambient| ambient.light.is_some());
    let wanted_light = match (filtered(SkyProbe::Light), filtered(SkyProbe::Diffuse)) {
        (Some(light), Some(diffuse)) if active && (sky.lighting || sky.reflections) => {
            Some(EnvironmentMapLight {
                diffuse_map: if sky.lighting {
                    diffuse.diffuse_map.clone()
                } else {
                    cubes.black.clone()
                },
                // Traced rays read only this one; a sky that lights but
                // doesn't reflect shows them its blurred light.
                specular_map: if sky.reflections {
                    light.specular_map.clone()
                } else if tracing && sky.lighting {
                    diffuse.diffuse_map.clone()
                } else {
                    cubes.black.clone()
                },
                intensity: SKY_UNIT * environment.reflections.max(0.0),
                ..default()
            })
        }
        _ => None,
    };
    // Traced rays that escape a skyless world see its flat ambient.
    let wanted_light = wanted_light.or_else(|| traced_ambient.and_then(|a| a.light.clone()));
    for (camera, view, light) in &cameras {
        let mut camera = commands.entity(camera);
        match wanted_view {
            Some(wanted) if view != Some(&wanted) => {
                camera.insert(wanted);
            }
            None if view.is_some() => {
                camera.remove::<SkyView>();
            }
            _ => {}
        }
        match &wanted_light {
            Some(wanted) if !same_light(light, wanted) => {
                camera.insert(wanted.clone());
            }
            None if light.is_some() => {
                camera.remove::<EnvironmentMapLight>();
            }
            _ => {}
        }
    }
}

fn same_light(have: Option<&EnvironmentMapLight>, wanted: &EnvironmentMapLight) -> bool {
    have.is_some_and(|have| {
        have.diffuse_map == wanted.diffuse_map
            && have.specular_map == wanted.specular_map
            && have.intensity == wanted.intensity
    })
}

/// An uninitialised FP16 cube the compute pass writes.
fn storage_cube(size: u32, label: &'static str) -> Image {
    let mut image = Image::new_uninit(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        TextureFormat::Rgba16Float,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage |= TextureUsages::STORAGE_BINDING;
    image.texture_descriptor.label = Some(label);
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    image
}

fn black_cube() -> Image {
    cube_image(1, 1, vec![0; 8 * 6], TextureFormat::Rgba16Float)
}

/// The baked BC6H cube where a build left one, else the source file.
fn load(key: &HdriKey, compressed: bool) -> Result<Image, String> {
    let baked = key.dir.join(pipeline::baked_sky_path(&key.path));
    if let Ok(bytes) = blockloom_core::vfs::read(&baked) {
        let cube = bc6h::read_dds_cube_levels(&bytes)?;
        if compressed {
            return Ok(cube_image(
                cube.size,
                cube.mips,
                cube.blocks.to_vec(),
                TextureFormat::Bc6hRgbUfloat,
            ));
        }
        let mut texels = Vec::with_capacity(cube.blocks.len() * 8);
        let mut at = 0;
        for _face in 0..6 {
            for mip in 0..cube.mips {
                let bytes = cube.level_bytes(mip);
                texels.extend(bc6h::decode_face(
                    &cube.blocks[at..at + bytes],
                    cube.level_size(mip),
                )?);
                at += bytes;
            }
        }
        return Ok(cube_image(
            cube.size,
            cube.mips,
            texels,
            TextureFormat::Rgba16Float,
        ));
    }
    source_cube(&key.dir, &key.path, key.bias, key.seam)
}

/// The file itself as an FP16 cube with its whole mip chain.
fn source_cube(dir: &Path, path: &str, bias: f32, seam: f32) -> Result<Image, String> {
    let mut image = hdr::load_hdr(dir, path)?;
    image.bias(bias);
    image.fix_seam(seam);
    let levels = hdr::HdrCube::from_image(&image, hdr::HdrCube::MAX_FACE).mip_chain(1);
    let one = half::f16::ONE.to_le_bytes();
    let size = levels[0].size;
    let mut texels = Vec::with_capacity((size * size * 8 * 8) as usize);
    for face in 0..6 {
        for level in &levels {
            for texel in &level.faces[face] {
                for channel in texel {
                    texels.extend_from_slice(&half::f16::from_f32(*channel).to_le_bytes());
                }
                texels.extend_from_slice(&one);
            }
        }
    }
    Ok(cube_image(
        size,
        levels.len() as u32,
        texels,
        TextureFormat::Rgba16Float,
    ))
}

/// A cube from data laid out face by face, each face's mips in turn.
pub(crate) fn cube_image(size: u32, mips: u32, data: Vec<u8>, format: TextureFormat) -> Image {
    let mut image = Image::new_uninit(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = mips.max(1);
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    image
}

fn extract_sky_views(
    mut commands: Commands,
    mut previous: Local<usize>,
    views: Extract<Query<(RenderEntity, &SkyView)>>,
) {
    let mut values = Vec::with_capacity(*previous);
    for (entity, view) in &views {
        values.push((entity, *view));
    }
    *previous = values.len();
    commands.try_insert_batch(values);
}

#[derive(Resource)]
struct SkyPipelines {
    cube_layout: BindGroupLayoutDescriptor,
    cube: CachedComputePipelineId,
    sampler: Sampler,
}

/// The background pipeline, specialized per view like Bevy's skybox.
#[derive(Resource)]
struct SkyBackground {
    layout: BindGroupLayoutDescriptor,
    shader: Handle<Shader>,
}

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
struct SkyBackgroundKey {
    target_format: TextureFormat,
    samples: u32,
    depth_format: TextureFormat,
}

impl SpecializedRenderPipeline for SkyBackground {
    type Key = SkyBackgroundKey;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("sky_background".into()),
            layout: vec![self.layout.clone()],
            vertex: VertexState {
                shader: self.shader.clone(),
                entry_point: Some("vertex".into()),
                ..default()
            },
            // Only where nothing was drawn: reverse Z puts the far plane at 0.
            depth_stencil: Some(DepthStencilState {
                format: key.depth_format,
                depth_write_enabled: Some(false),
                depth_compare: Some(CompareFunction::GreaterEqual),
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            }),
            multisample: MultisampleState {
                count: key.samples,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                entry_point: Some("fragment".into()),
                targets: vec![Some(ColorTargetState {
                    format: key.target_format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            ..default()
        }
    }
}

/// The background's parameters: one entry with the sun and moon, one
/// without, and each view binds the one it wants.
#[derive(Resource, Default)]
struct SkyUniforms {
    buffer: DynamicUniformBuffer<SkyParams>,
    offsets: [u32; 2],
    /// The compute pass's own copy.
    cube: UniformBuffer<SkyParams>,
    /// Stars, aurora and flash, which move every frame.
    space: UniformBuffer<SpaceParams>,
}

fn init_pipelines(
    mut commands: Commands,
    assets: Res<AssetServer>,
    device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
) {
    let cube_layout = BindGroupLayoutDescriptor::new(
        "sky_cube_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<SkyParams>(false),
                texture_cube(TextureSampleType::Float { filterable: true }),
                binding_types::sampler(SamplerBindingType::Filtering),
                texture_storage_2d_array(
                    TextureFormat::Rgba16Float,
                    StorageTextureAccess::WriteOnly,
                ),
                texture_storage_2d_array(
                    TextureFormat::Rgba16Float,
                    StorageTextureAccess::WriteOnly,
                ),
            ),
        ),
    );
    let cube = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("sky_cube".into()),
        layout: vec![cube_layout.clone()],
        shader: bevy::asset::load_embedded_asset!(assets.as_ref(), "shaders/sky_cube.wesl"),
        entry_point: Some("write_sky".into()),
        ..default()
    });
    let sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("sky_sampler"),
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Linear,
        ..default()
    });
    let background = BindGroupLayoutDescriptor::new(
        "sky_background_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                uniform_buffer::<ViewUniform>(true).visibility(ShaderStages::VERTEX_FRAGMENT),
                uniform_buffer::<SkyParams>(true),
                texture_cube(TextureSampleType::Float { filterable: true }),
                texture_cube(TextureSampleType::Float { filterable: true }),
                texture_cube(TextureSampleType::Float { filterable: true }),
                binding_types::sampler(SamplerBindingType::Filtering),
                uniform_buffer::<SpaceParams>(false),
                texture_2d(TextureSampleType::Float { filterable: true }),
            ),
        ),
    );
    commands.insert_resource(SkyPipelines {
        cube_layout,
        cube,
        sampler,
    });
    commands.insert_resource(SkyBackground {
        layout: background,
        shader: bevy::asset::load_embedded_asset!(assets.as_ref(), "shaders/sky_background.wesl"),
    });
}

/// Gives every sky view the background pipeline and bind group, in the
/// components Bevy's opaque pass draws its skybox from.
#[allow(clippy::too_many_arguments)]
fn prepare_sky_views(
    mut commands: Commands,
    sky: Option<Res<SkyRender>>,
    pipelines: Option<Res<SkyPipelines>>,
    background: Option<Res<SkyBackground>>,
    mut specialized: ResMut<SpecializedRenderPipelines<SkyBackground>>,
    pipeline_cache: Res<PipelineCache>,
    view_uniforms: Res<ViewUniforms>,
    images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut uniforms: ResMut<SkyUniforms>,
    space: Option<Res<SpaceRender>>,
    fallback: Res<FallbackImage>,
    views: Query<(Entity, &SkyView, &ExtractedView, &Msaa)>,
) {
    let ready = (|| {
        let sky = sky.as_ref()?;
        let params = sky.params?;
        let light = images.get(&sky.light)?;
        let source = images.get(&sky.source)?;
        let blurred = sky
            .blurred
            .as_ref()
            .and_then(|blurred| images.get(blurred))
            .unwrap_or(light);
        Some((params, light, source, blurred))
    })();
    let (
        Some((params, light, source, blurred)),
        Some(pipelines),
        Some(background),
        Some(view_binding),
    ) = (
        ready,
        pipelines,
        background,
        view_uniforms.uniforms.binding(),
    )
    else {
        for (entity, ..) in &views {
            commands
                .entity(entity)
                .remove::<(SkyboxPipelineId, SkyboxBindGroup)>();
        }
        return;
    };
    let uniforms = &mut *uniforms;
    uniforms.buffer.clear();
    let with_disks = SkyParams {
        info: params.info | UVec4::new(0, FLAG_DISKS, 0, 0),
        ..params
    };
    uniforms.offsets = [
        uniforms.buffer.push(&params),
        uniforms.buffer.push(&with_disks),
    ];
    uniforms.buffer.write_buffer(&device, &queue);
    let space = space.map(|space| space.clone()).unwrap_or_default();
    uniforms.space.set(space.params);
    uniforms.space.write_buffer(&device, &queue);
    let (Some(sky_binding), Some(space_binding)) =
        (uniforms.buffer.binding(), uniforms.space.binding())
    else {
        return;
    };
    let milky_way = space
        .milky_way
        .as_ref()
        .and_then(|handle| images.get(handle))
        .map_or(&fallback.d2.texture_view, |image| &image.texture_view);
    let bind_group = device.create_bind_group(
        "sky_background",
        &pipeline_cache.get_bind_group_layout(&background.layout),
        &BindGroupEntries::sequential((
            view_binding,
            sky_binding,
            &light.texture_view,
            &source.texture_view,
            &blurred.texture_view,
            &pipelines.sampler,
            space_binding,
            milky_way,
        )),
    );
    for (entity, view, extracted, msaa) in &views {
        let pipeline = specialized.specialize(
            &pipeline_cache,
            &background,
            SkyBackgroundKey {
                target_format: extracted.target_format,
                samples: msaa.samples(),
                depth_format: CORE_3D_DEPTH_FORMAT,
            },
        );
        let offset = uniforms.offsets[view.disks as usize];
        commands.entity(entity).insert((
            SkyboxPipelineId(pipeline),
            SkyboxBindGroup((bind_group.clone(), offset)),
        ));
    }
}

/// Writes the sky into its cubes when the main world has moved it on,
/// ahead of Bevy filtering them.
#[allow(clippy::too_many_arguments)]
fn write_cubes(
    sky: Option<Res<SkyRender>>,
    pipelines: Option<Res<SkyPipelines>>,
    pipeline_cache: Res<PipelineCache>,
    images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut uniforms: ResMut<SkyUniforms>,
    mut ctx: RenderContext,
) {
    let (Some(sky), Some(pipelines)) = (sky, pipelines) else {
        return;
    };
    let Some(params) = sky.params else {
        return;
    };
    if sky.written.load(Ordering::Relaxed) == sky.generation {
        return;
    }
    let (Some(pipeline), Some(light), Some(diffuse), Some(source)) = (
        pipeline_cache.get_compute_pipeline(pipelines.cube),
        images.get(&sky.light),
        images.get(&sky.diffuse),
        images.get(&sky.source),
    ) else {
        return;
    };
    uniforms.cube.set(params);
    uniforms.cube.write_buffer(&device, &queue);
    let Some(binding) = uniforms.cube.binding() else {
        return;
    };
    let layers = |image: &GpuImage| {
        image.texture.create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
            mip_level_count: Some(1),
            ..default()
        })
    };
    let bind_group = device.create_bind_group(
        "sky_cube",
        &pipeline_cache.get_bind_group_layout(&pipelines.cube_layout),
        &BindGroupEntries::sequential((
            binding,
            &source.texture_view,
            &pipelines.sampler,
            &layers(light),
            &layers(diffuse),
        )),
    );
    {
        let mut pass = ctx
            .command_encoder()
            .begin_compute_pass(&ComputePassDescriptor {
                label: Some("sky_cube"),
                timestamp_writes: None,
            });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        let groups = LIGHT_FACE.div_ceil(8);
        pass.dispatch_workgroups(groups, groups, 6);
    }
    sky.written.store(sky.generation, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::sky::SunMode;

    fn write_panorama(dir: &Path) {
        let (width, height) = (64u32, 32u32);
        let pixels: Vec<image::Rgb<f32>> = (0..width * height)
            .map(|i| image::Rgb([1.0 + (i % width) as f32 / 8.0, 2.0, 0.5]))
            .collect();
        let file = std::fs::File::create(dir.join("assets/sky.hdr")).unwrap();
        image::codecs::hdr::HdrEncoder::new(file)
            .encode(&pixels, width as usize, height as usize)
            .unwrap();
    }

    fn project() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "blockloom-sky-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        write_panorama(&dir);
        dir
    }

    fn key(dir: &Path, bias: f32) -> HdriKey {
        HdriKey {
            dir: dir.to_path_buf(),
            path: "assets/sky.hdr".into(),
            bias,
            seam: 0.0,
        }
    }

    #[test]
    fn a_source_panorama_becomes_an_fp16_cube_with_mips() {
        let dir = project();
        let image = load(&key(&dir, 1.0), true).unwrap();
        assert_eq!(image.texture_descriptor.format, TextureFormat::Rgba16Float);
        assert_eq!(image.texture_descriptor.size.depth_or_array_layers, 6);
        let size = image.texture_descriptor.size.width;
        assert!(size.is_power_of_two());
        assert_eq!(image.texture_descriptor.mip_level_count, size.ilog2() + 1);
        // Every face's whole chain is there.
        let texels: u32 = (0..=size.ilog2()).map(|mip| (size >> mip).pow(2)).sum();
        assert_eq!(
            image.data.as_ref().unwrap().len(),
            (texels * 8 * 6) as usize
        );
        // The bias doubled a green channel of 2.
        let data = image.data.as_ref().unwrap();
        let green = half::f16::from_le_bytes([data[2], data[3]]).to_f32();
        assert_eq!(green, 4.0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_baked_cube_loads_compressed_or_decoded() {
        let dir = project();
        let image = hdr::load_hdr(&dir, "assets/sky.hdr").unwrap();
        let cube = hdr::HdrCube::from_image(&image, 16);
        let baked = dir.join(pipeline::baked_sky_path("assets/sky.hdr"));
        std::fs::create_dir_all(baked.parent().unwrap()).unwrap();
        std::fs::write(&baked, bc6h::write_dds_cube_levels(&cube.mip_chain(4))).unwrap();

        let compressed = load(&key(&dir, 0.0), true).unwrap();
        assert_eq!(
            compressed.texture_descriptor.format,
            TextureFormat::Bc6hRgbUfloat
        );
        assert_eq!(compressed.texture_descriptor.mip_level_count, 3);
        // A byte a texel: 16x16, 8x8 and 4x4 on each face.
        assert_eq!(compressed.data.as_ref().unwrap().len(), (256 + 64 + 16) * 6);

        let decoded = load(&key(&dir, 0.0), false).unwrap();
        assert_eq!(
            decoded.texture_descriptor.format,
            TextureFormat::Rgba16Float
        );
        assert_eq!(
            decoded.data.as_ref().unwrap().len(),
            (256 + 64 + 16) * 8 * 6
        );
        std::fs::remove_dir_all(dir).ok();
    }

    fn environment(sky: &Sky) -> Environment {
        let mut world = blockloom_core::scene::World {
            sky: sky.clone(),
            ..default()
        };
        world.mode = blockloom_core::scene::Mode::ThreeD;
        let mut env = Environment::from_world(&world);
        env.through_air(sky);
        env
    }

    #[test]
    fn a_flat_sky_or_an_hdri_without_a_file_is_nothing() {
        let sky = Sky::default();
        assert!(SkyParams::resolve(&sky, &environment(&sky), 1).is_none());
        let hdri = Sky {
            kind: SkyKind::Hdri,
            ..default()
        };
        assert!(SkyParams::resolve(&hdri, &environment(&hdri), 1).is_none());
    }

    #[test]
    fn the_physical_sky_follows_the_blended_sun() {
        let mut sky = Sky {
            kind: SkyKind::Physical,
            ..default()
        };
        sky.sun.mode = SunMode::Manual;
        sky.sun.elevation = 30.0;
        sky.exposure = 1.0;
        let env = environment(&sky);
        let params = SkyParams::resolve(&sky, &env, 1).unwrap();
        assert_eq!(params.info.x, KIND_PHYSICAL);
        assert!((params.sun_dir.y - 0.5).abs() < 1e-4);
        assert_eq!(params.scale.x, 2.0);
        // The sky scatters the sun above the air; the light keeps what's left.
        assert!(params.sun.x > env.sun.illuminance * 0.5);
        assert!(env.sun.above_air.z > env.sun.illuminance * env.sun.color.to_linear().blue);
        // The disk is far brighter than the sky's lux spread over the sky.
        assert!(params.sun_disk.x > params.sun.x * 1000.0);
        assert_eq!(params.info.y & FLAG_MOON, 0);
        // Day has no night glow.
        assert_eq!(params.night.x, 0.0);
    }

    #[test]
    fn night_glows_once_the_sun_is_down() {
        let mut sky = Sky {
            kind: SkyKind::Physical,
            ..default()
        };
        sky.sun.mode = SunMode::Manual;
        sky.sun.elevation = -20.0;
        sky.physical.moon = true;
        let env = environment(&sky);
        let params = SkyParams::resolve(&sky, &env, 1).unwrap();
        assert!(params.night.z > 0.0);
        assert_ne!(params.info.y & FLAG_MOON, 0);
        // And the sun no longer lights the world.
        assert_eq!(env.sun.illuminance, 0.0);
    }

    #[test]
    fn a_gradient_warms_only_with_the_sun_low() {
        let mut sky = Sky {
            kind: SkyKind::Gradient,
            ..default()
        };
        sky.sun.mode = SunMode::Manual;
        let warmth = |elevation: f32, sky: &mut Sky| {
            sky.sun.elevation = elevation;
            SkyParams::resolve(sky, &environment(sky), 1)
                .unwrap()
                .warm
                .w
        };
        assert_eq!(warmth(60.0, &mut sky), 0.0);
        assert!(warmth(3.0, &mut sky) > 0.3);
        assert_eq!(warmth(-30.0, &mut sky), 0.0);
        let params = SkyParams::resolve(&sky, &environment(&sky), 1).unwrap();
        assert_ne!(params.info.y & FLAG_DITHER, 0);
        // Colors are nits: white at 1000 nits is 1000.
        sky.gradient.top = "#FFFFFF".into();
        let params = SkyParams::resolve(&sky, &environment(&sky), 1).unwrap();
        assert!((params.top.x - 1000.0).abs() < 0.5);
    }

    #[test]
    fn an_hdri_turns_the_world_back_to_look_itself_up() {
        let mut sky = Sky {
            kind: SkyKind::Hdri,
            ..default()
        };
        sky.hdri.path = "assets/sky.hdr".into();
        sky.hdri.rotation = 90.0;
        sky.hdri.blur = 0.5;
        let params = SkyParams::resolve(&sky, &environment(&sky), 5).unwrap();
        assert_eq!(params.info.w, 5);
        let lookup = Mat3::from_cols(
            params.hdri_x.truncate(),
            params.hdri_y.truncate(),
            params.hdri_z.truncate(),
        );
        // Turned a quarter, what was straight ahead in the image is now
        // off to one side.
        let ahead = lookup * Vec3::NEG_Z;
        assert!(ahead.z.abs() < 1e-5 && ahead.x.abs() > 0.99, "{ahead}");
        assert_eq!(params.scale.z, 1.0);
        assert_eq!(params.hdri.w, 4.0);
    }

    #[test]
    fn sky_params_match_the_wesl_layout() {
        let source = blockloom_core::shader_lib::module("sky").unwrap();
        let body = source.split("struct SkyParams {").nth(1).unwrap();
        let body = &body[..body.find("\n}").unwrap()];
        let fields = body.lines().filter(|line| line.contains(": vec4<")).count();
        // Every field is one vec4.
        assert_eq!(SkyParams::min_size().get(), fields as u64 * 16);
        assert_eq!(fields, 22);
    }

    #[test]
    fn the_sky_shaders_compile() {
        blockloom_core::shader_lib::validate(include_str!("shaders/sky_cube.wesl"), &[])
            .unwrap_or_else(|error| panic!("{error}"));
        // Bevy's modules only exist on the GPU side, so stand in for the two
        // things the background takes from them.
        let background = include_str!("shaders/sky_background.wesl")
            .replace("import bevy_render::view::View;", "")
            .replace("import bevy_pbr::render::utils::coords_to_viewport_uv;", "")
            + "struct View { view_from_clip: mat4x4<f32>, world_from_view: mat4x4<f32>, \
               main_pass_viewport: vec4<f32>, exposure: f32 }\n\
               fn coords_to_viewport_uv(p: vec2<f32>, v: vec4<f32>) -> vec2<f32> \
               { return (p - v.xy) / v.zw; }\n";
        blockloom_core::shader_lib::validate(&background, &[])
            .unwrap_or_else(|error| panic!("{error}"));
    }
}
