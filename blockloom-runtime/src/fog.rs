//! The air (`blockloom_core::fog`), 3D only: one pass over the frame after
//! the main passes and before any post, which lays aerial haze on far
//! surfaces, height fog along every ray and volumetric fog nearest the
//! camera.
//!
//! Volumetric fog lives in a froxel grid (`shaders/fog_froxels.wesl`): a
//! compute pass works out each froxel's medium and in-scattered light -
//! Bevy's directional lights through their own shadow cascades, plus up to
//! `MAX_FOG_LIGHTS` point and spot lights unshadowed - blends it with last
//! frame's grid reprojected, and a second pass integrates each column front
//! to back. The composite (`shaders/fog_composite.wesl`) then reads each
//! pixel's distance from the depth prepass and fetches once.
//!
//! Everything the passes read is worked out here in the main world from the
//! blended `Environment` into `FogRender`, so volumes, `set fog density` and
//! the time of day reach the fog the way they reach the sun.

use crate::engine::{ActorId, Engine};
use crate::environment::Environment;
use crate::lights::Lit;
use crate::sky::SkyRender;
use crate::world::{WorldCamera, WorldLight, parse_color};
use bevy::core_pipeline::prepass::{DepthPrepass, ViewPrepassTextures};
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::diagnostic::FrameCount;
use bevy::light::VolumetricLight;
use bevy::pbr::{
    GpuLights, LightMeta, ShadowSamplers, ViewLightsUniformOffset, ViewShadowBindings,
};
use bevy::prelude::*;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{
    self, texture_2d, texture_2d_array, texture_3d, texture_cube, texture_depth_2d,
    texture_depth_2d_multisampled, texture_storage_3d, uniform_buffer,
};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::{ExtractedView, Msaa, ViewTarget};
use bevy::render::{GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::Shader;
use blockloom_core::components::LightKind;
use blockloom_core::fog::{Fog, color_weights};
use blockloom_core::volume::VolumeShape;

/// As many local fog volumes and lights as the froxels read; the rest are
/// left out. `MAX_LOCAL_FOG` and `MAX_FOG_LIGHTS` in `shaders/fog.wesl`.
pub const MAX_LOCAL_FOG: usize = 16;
pub const MAX_FOG_LIGHTS: usize = 16;

pub const FLAG_HEIGHT: u32 = 1;
pub const FLAG_VOLUMETRIC: u32 = 2;
pub const FLAG_AERIAL: u32 = 4;
pub const FLAG_SKY: u32 = 8;
pub const FLAG_HISTORY: u32 = 16;
pub const FLAG_SUN_VOLUMETRIC: u32 = 32;

/// How much of each froxel is this frame's rather than history's.
const HISTORY_BLEND: f32 = 0.12;

pub fn register(app: &mut App) {
    app.init_resource::<FogRender>().add_systems(
        Update,
        (resolve_fog, apply_fog_views)
            .chain()
            .after(crate::environment::apply_environment),
    );
    if app.get_sub_app(RenderApp).is_none() {
        return;
    }
    bevy::asset::embedded_asset!(app, "shaders/fog_froxels.wesl");
    bevy::asset::embedded_asset!(app, "shaders/fog_composite.wesl");
    app.add_plugins((
        ExtractComponentPlugin::<FogView>::default(),
        ExtractResourcePlugin::<FogRender>::default(),
    ));
    let render = app.get_sub_app_mut(RenderApp).unwrap();
    render
        .init_gpu_resource::<SpecializedRenderPipelines<FogComposite>>()
        .init_resource::<FogUniformBuffer>()
        .add_systems(RenderStartup, init_pipelines)
        .add_systems(
            Render,
            prepare_fog_views.in_set(RenderSystems::PrepareResources),
        )
        .add_systems(
            Core3d,
            draw_fog
                .after(Core3dSystems::MainPass)
                .before(Core3dSystems::EarlyPostProcess),
        );
}

/// Asks for the air on a camera.
#[derive(Component, Clone, Copy, Default, Debug, ExtractComponent)]
#[extract_app(RenderApp)]
#[extract_component_filter(With<Camera>)]
pub struct FogView;

/// `LocalFogVolume` in `shaders/fog.wesl`.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct LocalFogVolume {
    pub centre: Vec4,
    pub rotation: Vec4,
    pub extent: Vec4,
    pub albedo: Vec4,
    pub emissive: Vec4,
}

/// `FogLight` in `shaders/fog.wesl`.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct FogLight {
    pub position: Vec4,
    pub color: Vec4,
    pub direction: Vec4,
    pub cone: Vec4,
}

/// `FogUniforms` in `shaders/fog.wesl`, field for field. The per-view half
/// (the matrices, camera and target) is filled in the render world.
#[derive(Clone, Copy, Debug, PartialEq, ShaderType)]
pub struct FogUniforms {
    pub world_from_clip: Mat4,
    pub previous_clip_from_world: Mat4,
    pub previous_camera: Vec4,
    pub camera: Vec4,
    pub forward: Vec4,
    pub screen_size: Vec4,
    pub grid: UVec4,
    pub counts: UVec4,
    pub height: Vec4,
    pub height_ambient: Vec4,
    pub height_sun: Vec4,
    pub height_moon: Vec4,
    pub height_albedo: Vec4,
    pub sun_dir: Vec4,
    pub moon_dir: Vec4,
    pub aerial: Vec4,
    pub aerial_color: Vec4,
    pub aerial_tint: Vec4,
    pub volumetric: Vec4,
    pub volumetric_albedo: Vec4,
    pub volumetric_emissive: Vec4,
    pub noise: Vec4,
    pub noise_scale: Vec4,
    pub ambient: Vec4,
    pub locals: [LocalFogVolume; MAX_LOCAL_FOG],
    pub lights: [FogLight; MAX_FOG_LIGHTS],
}

impl Default for FogUniforms {
    fn default() -> Self {
        Self {
            world_from_clip: Mat4::IDENTITY,
            previous_clip_from_world: Mat4::IDENTITY,
            previous_camera: Vec4::ZERO,
            camera: Vec4::ZERO,
            forward: Vec4::ZERO,
            screen_size: Vec4::ZERO,
            grid: UVec4::ZERO,
            counts: UVec4::ZERO,
            height: Vec4::ZERO,
            height_ambient: Vec4::ZERO,
            height_sun: Vec4::ZERO,
            height_moon: Vec4::ZERO,
            height_albedo: Vec4::ZERO,
            sun_dir: Vec4::Y,
            moon_dir: Vec4::Y,
            aerial: Vec4::ZERO,
            aerial_color: Vec4::ZERO,
            aerial_tint: Vec4::ONE,
            volumetric: Vec4::ZERO,
            volumetric_albedo: Vec4::ONE,
            volumetric_emissive: Vec4::ZERO,
            noise: Vec4::ZERO,
            noise_scale: Vec4::ZERO,
            ambient: Vec4::ZERO,
            locals: [LocalFogVolume::default(); MAX_LOCAL_FOG],
            lights: [FogLight::default(); MAX_FOG_LIGHTS],
        }
    }
}

/// This frame's air, for the render world. `None` when there is none.
#[derive(Resource, Clone, Default, Debug, PartialEq)]
pub struct FogRender {
    pub uniforms: Option<FogUniforms>,
}

impl ExtractResource<RenderApp> for FogRender {
    type Source = FogRender;

    fn extract_resource(source: &Self::Source) -> Self {
        source.clone()
    }
}

fn linear(color: Color) -> Vec3 {
    let c = color.to_linear();
    Vec3::new(c.red, c.green, c.blue)
}

/// What the world's lights and volumes add to the air this frame.
#[derive(Clone, Debug, Default)]
pub struct FogScene {
    pub locals: Vec<LocalFogVolume>,
    pub lights: Vec<FogLight>,
    /// Where the moon's light comes from and its lux per channel, or zero.
    pub moon: (Vec3, Vec3),
    /// Whether the sky lights the world, so fog glows with its horizon.
    pub sky: bool,
    /// Seconds, for the noise's drift.
    pub time: f32,
}

impl FogUniforms {
    /// The air as the passes read it, `None` for clear air. `env` carries
    /// what volumes and blocks made of the project's fog.
    pub fn resolve(fog: &Fog, env: &Environment, scene: &FogScene) -> Option<FogUniforms> {
        let mut flags = 0;
        let mut u = FogUniforms {
            camera: Vec4::new(0.0, 0.0, 0.0, exposure_scale(env.exposure)),
            sun_dir: env.sun.direction.extend(0.0),
            moon_dir: scene.moon.0.try_normalize().unwrap_or(Vec3::Y).extend(0.0),
            ..default()
        };
        let ambient = linear(env.ambient_color) * env.ambient_brightness;
        let sun = linear(env.sun.color) * env.sun.illuminance;
        u.ambient = ambient.extend(0.0);

        if env.fog_density > 0.0 {
            flags |= FLAG_HEIGHT;
            let h = &fog.height;
            let [day, dusk, night] = color_weights(env.sun.direction.y);
            let albedo = linear(env.fog_colors[0]) * day
                + linear(env.fog_colors[1]) * dusk
                + linear(env.fog_colors[2]) * night;
            u.height = Vec4::new(env.fog_density, h.falloff, env.fog_height, h.start);
            u.height_ambient = (albedo * ambient).extend(0.0);
            u.height_sun = (albedo * sun).extend(h.sun_boost);
            u.height_moon = (albedo * scene.moon.1).extend(h.sun_boost_g);
            u.height_albedo = albedo.extend(0.0);
        }

        if env.haze > 0.0 {
            flags |= FLAG_AERIAL;
            let a = &fog.aerial;
            let tint = linear(parse_color(&a.tint));
            u.aerial = Vec4::new(env.haze, a.height_scale, a.desaturation, a.blue_shift);
            // With no sky to take it from, haze glows like white fog would.
            let lit = ambient + sun / (4.0 * std::f32::consts::PI);
            u.aerial_color = (tint * lit).extend(0.0);
            u.aerial_tint = tint.extend(0.0);
        }

        let v = &fog.volumetric;
        let locals = scene.locals.len().min(MAX_LOCAL_FOG);
        let lights = scene.lights.len().min(MAX_FOG_LIGHTS);
        if env.volumetric_density > 0.0 || locals > 0 {
            flags |= FLAG_VOLUMETRIC;
            if v.sun {
                flags |= FLAG_SUN_VOLUMETRIC;
            }
            let grid = v.quality.grid();
            u.grid = UVec4::new(grid[0], grid[1], grid[2], 0);
            u.volumetric = Vec4::new(
                env.volumetric_density,
                v.falloff,
                v.base_height,
                v.anisotropy,
            );
            u.volumetric_albedo = linear(env.volumetric_albedo).extend(v.ambient);
            u.volumetric_emissive =
                (linear(parse_color(&v.emissive)) * v.emissive_strength).extend(v.range);
            u.noise = (Vec3::from(v.noise_wind) * -scene.time).extend(v.noise);
            u.noise_scale = Vec4::new(1.0 / v.noise_scale, HISTORY_BLEND, 1.0, 0.0);
            u.locals[..locals].copy_from_slice(&scene.locals[..locals]);
            u.lights[..lights].copy_from_slice(&scene.lights[..lights]);
        } else {
            u.grid = UVec4::new(1, 1, 1, 0);
            u.volumetric_emissive.w = 1.0;
        }
        if flags == 0 {
            return None;
        }
        if scene.sky {
            flags |= FLAG_SKY;
        }
        u.grid.w = flags;
        u.counts = UVec4::new(locals as u32, lights as u32, 1, 0);
        Some(u)
    }
}

/// What the camera multiplies scene light by at `ev100`, as Bevy does.
pub fn exposure_scale(ev100: f32) -> f32 {
    1.0 / (1.2 * ev100.exp2())
}

/// A `Volume`'s local fog as the froxels read it.
fn local_fog(spec: &blockloom_core::volume::VolumeSpec, transform: &Transform) -> LocalFogVolume {
    let scale = transform.scale.abs();
    let rotation = transform.rotation.normalize();
    let rotation = if rotation.is_finite() {
        rotation
    } else {
        Quat::IDENTITY
    };
    let (shape, half) = match spec.shape {
        VolumeShape::Sphere => (1.0, Vec3::splat(spec.radius.max(0.0) * scale.max_element())),
        _ => (0.0, Vec3::from(spec.half_extents).max(Vec3::ZERO) * scale),
    };
    let fog = &spec.fog;
    let weight = spec.weight.clamp(0.0, 1.0);
    LocalFogVolume {
        centre: transform.translation.extend(shape),
        rotation: Vec4::from(rotation.inverse()),
        extent: half.extend(spec.blend_distance.max(0.0)),
        albedo: linear(parse_color(&fog.albedo)).extend(fog.density.max(0.0) * weight),
        emissive: (linear(parse_color(&fog.emissive)) * fog.emissive_strength).extend(0.0),
    }
}

/// A light as the froxels read it: candela, where it stands and points.
fn fog_light(spec: &blockloom_core::components::LightSpec, at: &GlobalTransform) -> FogLight {
    let candela = spec.lumens() / (4.0 * std::f32::consts::PI);
    let color = linear(parse_color(&spec.color)) * candela;
    let (direction, cone) = match spec.kind {
        LightKind::Spot => {
            let (inner, outer) = spec.cone();
            (
                at.forward().extend(outer.cos()),
                Vec4::new(inner.cos(), 0.0, 0.0, 0.0),
            )
        }
        _ => (Vec4::new(0.0, -1.0, 0.0, -2.0), Vec4::ZERO),
    };
    FogLight {
        position: at.translation().extend(spec.range.max(0.01)),
        color: color.extend(0.0),
        direction,
        cone,
    }
}

/// Works out this frame's air from the blended environment, the volumes
/// carrying local fog and the lights that light it.
#[allow(clippy::too_many_arguments)]
fn resolve_fog(
    engine: NonSend<Engine>,
    environment: Res<Environment>,
    time: Res<Time>,
    sky: Option<Res<SkyRender>>,
    moon: Option<Res<crate::space::MoonState>>,
    actors: Query<(&ActorId, &Transform)>,
    lit: Query<&Lit>,
    transforms: Query<&GlobalTransform>,
    mut sources: Option<ResMut<crate::atmosphere::AtmosphereSources>>,
    mut render: ResMut<FogRender>,
) {
    let world = &engine.project.world;
    let locals: Vec<_> = crate::volumes::placed(&engine, actors.iter())
        .into_iter()
        .filter(|(_, spec, _)| spec.fog.enabled && spec.fog.density > 0.0)
        .filter_map(|(id, spec, _)| {
            let (_, transform) = actors.iter().find(|(actor, _)| actor.0 == id)?;
            Some(local_fog(&spec, transform))
        })
        .take(MAX_LOCAL_FOG)
        .collect();
    let lights = lit
        .iter()
        .filter(|lit| lit.spec.volumetric && lit.spec.lumens() > 0.0)
        .filter_map(|lit| Some(fog_light(&lit.spec, transforms.get(lit.child).ok()?)))
        .take(MAX_FOG_LIGHTS)
        .collect();
    let scene = FogScene {
        locals,
        lights,
        moon: moon.map_or((Vec3::Y, Vec3::ZERO), |moon| (moon.direction, moon.lux)),
        sky: sky.is_some_and(|sky| sky.params.is_some()) && world.sky.lighting,
        time: time.elapsed_secs_wrapped(),
    };
    let uniforms = FogUniforms::resolve(&world.fog, &environment, &scene);
    if let Some(sources) = sources.as_mut() {
        sources.fog_density = environment.fog_density;
        let [day, dusk, night] = color_weights(environment.sun.direction.y);
        let color = linear(environment.fog_colors[0]) * day
            + linear(environment.fog_colors[1]) * dusk
            + linear(environment.fog_colors[2]) * night;
        sources.fog_color = LinearRgba::rgb(color.x, color.y, color.z);
    }
    if render.uniforms != uniforms {
        render.uniforms = uniforms;
    }
}

/// Puts the pass on every world camera while there is air, and lets the
/// sun and moon light volumetric fog when it asks for them.
#[allow(clippy::type_complexity)]
fn apply_fog_views(
    mut commands: Commands,
    engine: NonSend<Engine>,
    render: Res<FogRender>,
    cameras: Query<(Entity, Has<FogView>, Has<DepthPrepass>), With<WorldCamera>>,
    suns: Query<
        (Entity, Has<VolumetricLight>),
        Or<(With<WorldLight>, With<crate::space::MoonLight>)>,
    >,
) {
    let on = render.uniforms.is_some();
    for (camera, has, depth) in &cameras {
        match (on, has) {
            (true, false) => {
                let mut camera = commands.entity(camera);
                camera.insert(FogView);
                if !depth {
                    camera.insert(DepthPrepass);
                }
            }
            (false, true) => {
                commands.entity(camera).remove::<FogView>();
            }
            _ => {}
        }
    }
    let volumetric = &engine.project.world.fog.volumetric;
    let lit = on && volumetric.sun;
    for (sun, has) in &suns {
        match (lit, has) {
            (true, false) => {
                commands.entity(sun).insert(VolumetricLight);
            }
            (false, true) => {
                commands.entity(sun).remove::<VolumetricLight>();
            }
            _ => {}
        }
    }
}

#[derive(Resource, Default)]
struct FogUniformBuffer(DynamicUniformBuffer<FogUniforms>);

/// The froxel grid a view keeps from frame to frame: two injected grids
/// (this frame's and last) and the integrated one.
#[derive(Component)]
struct FogGrid {
    size: UVec3,
    injected: [(Texture, TextureView); 2],
    integrated: (Texture, TextureView),
    /// Which injected grid this frame writes.
    current: usize,
    previous_clip_from_world: Mat4,
    previous_camera: Vec3,
    /// Whether last frame's grid holds anything to reproject.
    warm: bool,
}

/// Each fog view's uniforms and pipeline.
#[derive(Component)]
struct ViewFog {
    offset: u32,
    pipeline: CachedRenderPipelineId,
    multisampled: bool,
    volumetric: bool,
}

#[derive(Resource)]
struct FogPipelines {
    froxel_layout: BindGroupLayoutDescriptor,
    integrate_layout: BindGroupLayoutDescriptor,
    inject: CachedComputePipelineId,
    integrate: CachedComputePipelineId,
    linear: Sampler,
}

#[derive(Resource)]
struct FogComposite {
    layouts: [BindGroupLayoutDescriptor; 2],
    shader: Handle<Shader>,
    fullscreen: FullscreenShader,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FogCompositeKey {
    format: TextureFormat,
    multisampled: bool,
}

impl SpecializedRenderPipeline for FogComposite {
    type Key = FogCompositeKey;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        let mut shader_defs = Vec::new();
        if key.multisampled {
            shader_defs.push("MULTISAMPLED".into());
        }
        RenderPipelineDescriptor {
            label: Some("fog_composite".into()),
            layout: vec![self.layouts[key.multisampled as usize].clone()],
            vertex: self.fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs,
                targets: vec![Some(ColorTargetState {
                    format: key.format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            ..default()
        }
    }
}

fn init_pipelines(
    mut commands: Commands,
    assets: Res<AssetServer>,
    device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
    fullscreen: Res<FullscreenShader>,
) {
    let froxel_layout = BindGroupLayoutDescriptor::new(
        "fog_froxel_layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::COMPUTE,
            (
                (0, uniform_buffer::<FogUniforms>(true)),
                (1, uniform_buffer::<GpuLights>(true)),
                (2, texture_2d_array(TextureSampleType::Depth)),
                (3, binding_types::sampler(SamplerBindingType::Comparison)),
                (4, texture_3d(TextureSampleType::Float { filterable: true })),
                (5, binding_types::sampler(SamplerBindingType::Filtering)),
                (
                    6,
                    texture_storage_3d(TextureFormat::Rgba16Float, StorageTextureAccess::WriteOnly),
                ),
            ),
        ),
    );
    let integrate_layout = BindGroupLayoutDescriptor::new(
        "fog_integrate_layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::COMPUTE,
            (
                (0, uniform_buffer::<FogUniforms>(true)),
                (
                    7,
                    texture_3d(TextureSampleType::Float { filterable: false }),
                ),
                (
                    8,
                    texture_storage_3d(TextureFormat::Rgba16Float, StorageTextureAccess::WriteOnly),
                ),
            ),
        ),
    );
    let froxels: Handle<Shader> =
        bevy::asset::load_embedded_asset!(assets.as_ref(), "shaders/fog_froxels.wesl");
    let inject = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("fog_inject".into()),
        layout: vec![froxel_layout.clone()],
        shader: froxels.clone(),
        entry_point: Some("inject".into()),
        ..default()
    });
    let integrate = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("fog_integrate".into()),
        layout: vec![integrate_layout.clone()],
        shader: froxels,
        entry_point: Some("integrate".into()),
        ..default()
    });
    let linear = device.create_sampler(&SamplerDescriptor {
        label: Some("fog_sampler"),
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Linear,
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        address_mode_w: AddressMode::ClampToEdge,
        ..default()
    });
    let composite_layout = |multisampled: bool| {
        let depth = if multisampled {
            texture_depth_2d_multisampled()
        } else {
            texture_depth_2d()
        };
        BindGroupLayoutDescriptor::new(
            "fog_composite_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    uniform_buffer::<FogUniforms>(true),
                    texture_2d(TextureSampleType::Float { filterable: false }),
                    depth,
                    texture_3d(TextureSampleType::Float { filterable: true }),
                    binding_types::sampler(SamplerBindingType::Filtering),
                    texture_cube(TextureSampleType::Float { filterable: true }),
                ),
            ),
        )
    };
    commands.insert_resource(FogPipelines {
        froxel_layout,
        integrate_layout,
        inject,
        integrate,
        linear,
    });
    commands.insert_resource(FogComposite {
        layouts: [composite_layout(false), composite_layout(true)],
        shader: bevy::asset::load_embedded_asset!(assets.as_ref(), "shaders/fog_composite.wesl"),
        fullscreen: fullscreen.clone(),
    });
}

fn grid_texture(device: &RenderDevice, size: UVec3, label: &'static str) -> (Texture, TextureView) {
    let texture = device.create_texture(&TextureDescriptor {
        label: Some(label),
        size: Extent3d {
            width: size.x,
            height: size.y,
            depth_or_array_layers: size.z,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D3,
        format: TextureFormat::Rgba16Float,
        usage: TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&TextureViewDescriptor::default());
    (texture, view)
}

/// Each fog view's grid, uniforms and composite pipeline. A view that
/// stopped asking loses them, since render world views outlive their
/// components.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn prepare_fog_views(
    mut commands: Commands,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
    frame: Res<FrameCount>,
    fog: Option<Res<FogRender>>,
    composite: Option<Res<FogComposite>>,
    mut pipelines: ResMut<SpecializedRenderPipelines<FogComposite>>,
    mut buffer: ResMut<FogUniformBuffer>,
    mut views: Query<
        (
            Entity,
            &ExtractedView,
            Option<&Msaa>,
            Option<&mut FogGrid>,
            Has<ViewShadowBindings>,
        ),
        With<FogView>,
    >,
    stale: Query<Entity, (With<ViewFog>, Without<FogView>)>,
) {
    for entity in &stale {
        commands.entity(entity).remove::<(ViewFog, FogGrid)>();
    }
    buffer.0.clear();
    let (Some(fog), Some(composite)) = (fog, composite) else {
        return;
    };
    let Some(base) = fog.uniforms else {
        for (entity, ..) in &views {
            commands.entity(entity).remove::<ViewFog>();
        }
        return;
    };
    for (entity, view, msaa, grid, shadows) in &mut views {
        let size = base.grid.truncate().max(UVec3::ONE);
        let clip_from_world = view
            .clip_from_world
            .unwrap_or_else(|| view.clip_from_view * view.world_from_view.to_matrix().inverse());
        let world_from_clip = clip_from_world.inverse();
        let camera = view.world_from_view.translation();
        let forward = view.world_from_view.forward().as_vec3();
        let mut uniforms = base;
        uniforms.world_from_clip = world_from_clip;
        uniforms.camera = camera.extend(base.camera.w);
        uniforms.forward = forward.extend((frame.0 % 65_536) as f32);
        let target = view.viewport.zw().max(UVec2::ONE).as_vec2();
        uniforms.screen_size = Vec4::new(target.x, target.y, 1.0 / target.x, 1.0 / target.y);
        let mut volumetric = base.grid.w & FLAG_VOLUMETRIC != 0;
        if !shadows {
            // No shadow bindings yet: the froxels can't be lit this frame.
            volumetric = false;
            uniforms.grid.w &= !FLAG_VOLUMETRIC;
        }
        match grid {
            Some(mut grid) if grid.size == size => {
                grid.current ^= 1;
                uniforms.previous_clip_from_world = grid.previous_clip_from_world;
                uniforms.previous_camera = grid.previous_camera.extend(0.0);
                if grid.warm {
                    uniforms.grid.w |= FLAG_HISTORY;
                }
                grid.previous_clip_from_world = clip_from_world;
                grid.previous_camera = camera;
                grid.warm = volumetric;
            }
            _ => {
                commands.entity(entity).insert(FogGrid {
                    size,
                    injected: [
                        grid_texture(&device, size, "fog_froxels_a"),
                        grid_texture(&device, size, "fog_froxels_b"),
                    ],
                    integrated: grid_texture(&device, size, "fog_froxels_integrated"),
                    current: 0,
                    previous_clip_from_world: clip_from_world,
                    previous_camera: camera,
                    warm: volumetric,
                });
            }
        }
        let multisampled = msaa.is_some_and(|msaa| msaa.samples() > 1);
        let pipeline = pipelines.specialize(
            &cache,
            &composite,
            FogCompositeKey {
                format: view.target_format,
                multisampled,
            },
        );
        let offset = buffer.0.push(&uniforms);
        commands.entity(entity).insert(ViewFog {
            offset,
            pipeline,
            multisampled,
            volumetric,
        });
    }
    buffer.0.write_buffer(&device, &queue);
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn draw_fog(
    view: ViewQuery<(
        &ViewTarget,
        &ViewFog,
        &FogGrid,
        Option<&ViewPrepassTextures>,
        Option<&ViewLightsUniformOffset>,
        Option<&ViewShadowBindings>,
    )>,
    froxel: Option<Res<FogPipelines>>,
    composite: Option<Res<FogComposite>>,
    buffer: Res<FogUniformBuffer>,
    cache: Res<PipelineCache>,
    light_meta: Option<Res<LightMeta>>,
    shadow_samplers: Option<Res<ShadowSamplers>>,
    sky: Option<Res<SkyRender>>,
    images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    let (target, view_fog, grid, prepass, lights_offset, shadows) = view.into_inner();
    let (Some(froxel), Some(composite)) = (froxel, composite) else {
        return;
    };
    let (Some(pipeline), Some(uniforms), Some(depth)) = (
        cache.get_render_pipeline(view_fog.pipeline),
        buffer.0.binding(),
        prepass.and_then(ViewPrepassTextures::depth_only_view),
    ) else {
        return;
    };
    let Some(sky_cube) = sky
        .as_ref()
        .and_then(|sky| images.get(&sky.diffuse))
        .map(|image| image.texture_view.clone())
    else {
        return;
    };
    let device = ctx.render_device().clone();
    let current = &grid.injected[grid.current];
    let previous = &grid.injected[grid.current ^ 1];

    if view_fog.volumetric
        && let (
            Some(inject),
            Some(integrate),
            Some(lights),
            Some(lights_offset),
            Some(shadows),
            Some(samplers),
        ) = (
            cache.get_compute_pipeline(froxel.inject),
            cache.get_compute_pipeline(froxel.integrate),
            light_meta
                .as_ref()
                .and_then(|meta| meta.view_gpu_lights.binding()),
            lights_offset,
            shadows,
            shadow_samplers.as_ref(),
        )
    {
        let inject_group = device.create_bind_group(
            "fog_inject",
            &cache.get_bind_group_layout(&froxel.froxel_layout),
            &BindGroupEntries::with_indices((
                (0, uniforms.clone()),
                (1, lights),
                (2, &shadows.directional_light_depth_texture_view),
                (3, &samplers.directional_light_comparison_sampler),
                (4, &previous.1),
                (5, &froxel.linear),
                (6, &current.1),
            )),
        );
        let integrate_group = device.create_bind_group(
            "fog_integrate",
            &cache.get_bind_group_layout(&froxel.integrate_layout),
            &BindGroupEntries::with_indices((
                (0, uniforms.clone()),
                (7, &current.1),
                (8, &grid.integrated.1),
            )),
        );
        let diagnostics = ctx.diagnostic_recorder();
        let diagnostics = diagnostics.as_deref();
        let mut pass = ctx
            .command_encoder()
            .begin_compute_pass(&ComputePassDescriptor {
                label: Some("fog_froxels"),
                timestamp_writes: None,
            });
        let span = diagnostics.pass_span(&mut pass, "fog_froxels");
        let size = grid.size;
        pass.set_pipeline(inject);
        pass.set_bind_group(0, &inject_group, &[view_fog.offset, lights_offset.offset]);
        pass.dispatch_workgroups(size.x.div_ceil(4), size.y.div_ceil(4), size.z.div_ceil(4));
        pass.set_pipeline(integrate);
        pass.set_bind_group(0, &integrate_group, &[view_fog.offset]);
        pass.dispatch_workgroups(size.x.div_ceil(8), size.y.div_ceil(8), 1);
        span.end(&mut pass);
    }

    let layout = cache.get_bind_group_layout(&composite.layouts[view_fog.multisampled as usize]);
    let post = target.post_process_write();
    let bind_group = device.create_bind_group(
        "fog_composite",
        &layout,
        &BindGroupEntries::sequential((
            uniforms,
            post.source,
            depth,
            &grid.integrated.1,
            &froxel.linear,
            &sky_cube,
        )),
    );
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let mut pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("fog_composite"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: post.destination,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    let span = diagnostics.pass_span(&mut pass, "fog_composite");
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &bind_group, &[view_fog.offset]);
    pass.draw(0..3, 0..1);
    span.end(&mut pass);
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::scene::{Mode, World};
    use blockloom_core::shader_lib;

    fn env(world: &World) -> Environment {
        let mut env = Environment::from_world(world);
        env.through_air(&world.sky);
        env
    }

    fn world() -> World {
        World {
            mode: Mode::ThreeD,
            ..default()
        }
    }

    #[test]
    fn clear_air_is_nothing() {
        let world = world();
        assert!(FogUniforms::resolve(&world.fog, &env(&world), &FogScene::default()).is_none());
    }

    #[test]
    fn height_fog_turns_on_with_its_density() {
        let mut world = world();
        world.fog.height.enabled = true;
        world.fog.height.distance = 300.0;
        let u = FogUniforms::resolve(&world.fog, &env(&world), &FogScene::default()).unwrap();
        assert_eq!(u.grid.w & FLAG_HEIGHT, FLAG_HEIGHT);
        assert_eq!(u.grid.w & FLAG_VOLUMETRIC, 0);
        assert!((u.height.x - 0.01).abs() < 1e-6);
        // Lit by the sun far more than by the ambient at noon.
        assert!(u.height_sun.x > u.height_ambient.x);
        // A one-texel grid stands in while volumetrics are off.
        assert_eq!(u.grid.truncate(), UVec3::ONE);
    }

    #[test]
    fn a_blocks_density_outlasts_the_project_switch() {
        let mut world = world();
        let mut e = env(&world);
        e.fog_density = 0.05;
        let u = FogUniforms::resolve(&world.fog, &e, &FogScene::default()).unwrap();
        assert!((u.height.x - 0.05).abs() < 1e-6);
        world.fog.height.enabled = true;
        e.fog_density = 0.0;
        assert!(FogUniforms::resolve(&world.fog, &e, &FogScene::default()).is_none());
    }

    #[test]
    fn volumetric_fog_carries_its_grid_locals_and_lights() {
        let mut world = world();
        world.fog.volumetric.enabled = true;
        let scene = FogScene {
            locals: vec![LocalFogVolume::default(); 20],
            lights: vec![FogLight::default(); 3],
            sky: true,
            ..default()
        };
        let u = FogUniforms::resolve(&world.fog, &env(&world), &scene).unwrap();
        assert_eq!(u.grid.truncate(), UVec3::new(128, 72, 64));
        assert_eq!(u.counts.x, MAX_LOCAL_FOG as u32);
        assert_eq!(u.counts.y, 3);
        assert_ne!(u.grid.w & FLAG_SUN_VOLUMETRIC, 0);
        assert_ne!(u.grid.w & FLAG_SKY, 0);
        assert_eq!(u.volumetric_emissive.w, world.fog.volumetric.range);
    }

    #[test]
    fn a_local_fog_volume_alone_makes_volumetric_fog() {
        let world = world();
        let scene = FogScene {
            locals: vec![LocalFogVolume::default()],
            ..default()
        };
        let u = FogUniforms::resolve(&world.fog, &env(&world), &scene).unwrap();
        assert_ne!(u.grid.w & FLAG_VOLUMETRIC, 0);
    }

    #[test]
    fn a_spot_light_points_its_cone_down_its_forward() {
        let spec = blockloom_core::components::LightSpec {
            kind: LightKind::Spot,
            intensity: 4.0 * std::f32::consts::PI * 100.0,
            ..default()
        };
        let at = GlobalTransform::from(
            Transform::from_xyz(1.0, 2.0, 3.0).looking_to(Vec3::NEG_Y, Vec3::X),
        );
        let light = fog_light(&spec, &at);
        assert!((light.color.x - 100.0).abs() < 1e-3);
        assert!((light.direction.truncate() - Vec3::NEG_Y).length() < 1e-5);
        assert!(light.direction.w > 0.0 && light.cone.x > light.direction.w);
        let point = fog_light(&default(), &at);
        assert_eq!(point.direction.w, -2.0);
    }

    #[test]
    fn fog_uniforms_match_the_wesl_layout() {
        // Three matrices' worth of vec4s, then the locals and lights.
        let vec4s = 2 * 4 + 22;
        assert_eq!(
            FogUniforms::min_size().get(),
            (vec4s * 16 + MAX_LOCAL_FOG * 80 + MAX_FOG_LIGHTS * 64) as u64
        );
        let source = shader_lib::module("fog").unwrap();
        let body = source.split("struct FogUniforms {").nth(1).unwrap();
        let body = &body[..body.find("\n}").unwrap()];
        let fields = body.lines().filter(|l| l.contains(": vec4<")).count();
        assert_eq!(fields, 22);
    }

    // Bevy's modules only exist on the GPU side, so the tests stand in for
    // the one struct the froxels take from them.
    const LIGHTS_STUB: &str = "struct DirectionalCascade { clip_from_world: mat4x4<f32>, texel_size: f32, far_bound: f32 }\n\
        struct DirectionalLight { cascades: array<DirectionalCascade, 4>, color: vec4<f32>, direction_to_light: vec3<f32>, flags: u32, \
        soft_shadow_size: f32, shadow_depth_bias: f32, shadow_normal_bias: f32, num_cascades: u32, cascades_overlap_proportion: f32, \
        depth_texture_base_index: u32, decal_index: u32, sun_disk_angular_size: f32, sun_disk_intensity: f32 }\n\
        struct Lights { directional_lights: array<DirectionalLight, 10>, ambient_color: vec4<f32>, cluster_dimensions: vec4<u32>, \
        cluster_factors: vec4<f32>, n_directional_lights: u32 }\n\
        const DIRECTIONAL_LIGHT_FLAGS_VOLUMETRIC_BIT: u32 = 2u;\n";

    #[test]
    fn the_fog_shaders_compile() {
        let froxels = include_str!("shaders/fog_froxels.wesl").replace(
            "import bevy_pbr::render::mesh_view_types::{Lights, DIRECTIONAL_LIGHT_FLAGS_VOLUMETRIC_BIT};",
            "",
        ) + LIGHTS_STUB;
        shader_lib::validate(&froxels, &[]).unwrap_or_else(|error| panic!("{error}"));
        let composite = include_str!("shaders/fog_composite.wesl");
        for multisampled in [false, true] {
            shader_lib::validate(composite, &[("MULTISAMPLED", multisampled)])
                .unwrap_or_else(|error| panic!("{error}"));
        }
    }
}
