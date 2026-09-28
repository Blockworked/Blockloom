//! Half-resolution cloud scattering with persistent, per-view temporal history.
use crate::{
    engine::Engine, environment::Environment, passes::*, sky::SkyRender, wind::WindField,
    world::WorldCamera,
};
use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::{
    Core3d, Core3dSystems, FullscreenShader,
    prepass::{DepthPrepass, ViewPrepassTextures},
};
use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::*;
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::storage::{GpuShaderBuffer, ShaderBuffer};
use bevy::render::texture::GpuImage;
use bevy::render::view::{ExtractedView, Msaa, ViewTarget};
use bevy::render::{GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
struct CloudUniforms {
    world_from_clip: Mat4,
    previous_clip: Mat4,
    camera: Vec4,
    previous_camera: Vec4,
    layer: Vec4,
    shape: Vec4,
    detail: Vec4,
    profile: Vec4,
    phase: Vec4,
    lighting: Vec4,
    sun: Vec4,
    sun_color: Vec4,
    moon: Vec4,
    moon_color: Vec4,
    drift: Vec4,
    erosion_drift: Vec4,
    shear: Vec4,
    screen: Vec4,
    steps: UVec4,
    shadow: Vec4,
    previous_drift: Vec4,
    // Single-bind stacked noise atlas: each page's XY scale into the
    // shared image plus its depth slab (offset, extent) in 0-1.
    noise_shape: Vec4,
    noise_detail: Vec4,
}

/// Default atlas pages when nothing is authored: a 128^3 shape slab under
/// a 32^3 detail slab, matching the bake sizes below.
const BAKED_SHAPE: u32 = 128;
const BAKED_DETAIL: u32 = 32;

/// Atlas size and both page mappings for the current staging, or the baked
/// defaults when a slot has no authored volume. The atlas always holds
/// both pages: the authored size when loaded, else the bake size.
fn noise_atlas(staging: Option<&NoiseStaging>) -> (UVec3, Vec4, Vec4) {
    fn slot_size(staging: Option<&NoiseStaging>, slot: usize, baked: u32) -> UVec3 {
        staging
            .and_then(|s| s.page_of[slot])
            .and_then(|i| staging?.layout.pages.get(i))
            .map(|page| UVec3::new(page.size[0], page.size[1], page.size[2]))
            .unwrap_or(UVec3::splat(baked))
    }
    let shape_size = slot_size(staging, 0, BAKED_SHAPE);
    let detail_size = slot_size(staging, 1, BAKED_DETAIL);
    let atlas = UVec3::new(
        shape_size.x.max(detail_size.x),
        shape_size.y.max(detail_size.y),
        shape_size.z + detail_size.z,
    );
    let depth = atlas.z.max(1) as f32;
    let shape = Vec4::new(
        shape_size.x as f32 / atlas.x.max(1) as f32,
        shape_size.y as f32 / atlas.y.max(1) as f32,
        0.0,
        shape_size.z as f32 / depth,
    );
    let detail = Vec4::new(
        detail_size.x as f32 / atlas.x.max(1) as f32,
        detail_size.y as f32 / atlas.y.max(1) as f32,
        shape_size.z as f32 / depth,
        detail_size.z as f32 / depth,
    );
    (atlas, shape, detail)
}
/// An authored noise volume, as RGBA8 texels ready to upload.
#[derive(Clone)]
struct NoiseVolume {
    size: UVec3,
    bytes: Vec<u8>,
}

/// Authored noise pages sharing one staging allocation, laid out by
/// [`pack_volume_atlas`]. Both files load as one payload family with one
/// lifetime; the march samples them from one stacked 3D atlas, so the pass
/// binds once instead of once per volume.
#[derive(Clone)]
struct NoiseStaging {
    layout: blockloom_core::pipeline::volume::VolumeAtlasLayout,
    bytes: Vec<u8>,
    /// Which layout page each volume slot reads, when that slot is loaded.
    page_of: [Option<usize>; 2],
}

impl NoiseStaging {
    /// Packs the loaded slots back to back. `None` with no pages, so an
    /// empty atlas never uploads.
    fn build(slots: [&Option<NoiseVolume>; 2]) -> Option<Self> {
        let mut sizes = Vec::new();
        let mut page_of = [None, None];
        for (slot, volume) in slots.iter().enumerate() {
            if let Some(volume) = volume {
                page_of[slot] = Some(sizes.len());
                sizes.push([volume.size.x, volume.size.y, volume.size.z]);
            }
        }
        if sizes.is_empty() {
            return None;
        }
        let layout = blockloom_core::pipeline::volume::pack_volume_atlas(&sizes).ok()?;
        let mut bytes = Vec::with_capacity(layout.total_texels as usize * 4);
        for volume in slots.into_iter().flatten() {
            bytes.extend_from_slice(&volume.bytes);
        }
        Some(Self {
            layout,
            bytes,
            page_of,
        })
    }

    /// This slot's page, if loaded: its native size plus its staging slice,
    /// which uploads exactly like the old per-volume bytes did.
    fn page(&self, slot: usize) -> Option<(UVec3, &[u8])> {
        let index = self.page_of[slot]?;
        let size = self.layout.pages[index].size;
        let start = self.layout.page_offset(index) as usize * 4;
        let len = (size[0] as usize * size[1] as usize * size[2] as usize) * 4;
        Some((
            UVec3::new(size[0], size[1], size[2]),
            self.bytes.get(start..start + len)?,
        ))
    }
}
#[derive(Resource, Clone, Default)]
struct CloudRender {
    uniforms: Option<CloudUniforms>,
    /// Authored shape and detail pages in one staging buffer; `None` bakes
    /// from the seed.
    staging: Option<std::sync::Arc<NoiseStaging>>,
    /// Bumped whenever `staging` changes, so views re-upload.
    generation: u32,
}
/// Which volume files are loaded, so they are read once per change.
#[derive(Resource, Default)]
struct LoadedVolumes {
    jobs: crate::streaming::CellTasks<usize, Result<NoiseVolume, String>>,
    slots: [Option<NoiseVolume>; 2],
    key: Option<(Option<std::path::PathBuf>, String, String)>,
}
fn load_noise(dir: &std::path::Path, relative: &str) -> Result<NoiseVolume, String> {
    let volume = blockloom_core::pipeline::load_volume(dir, relative)?;
    let [x, y, z] = volume.info.size;
    let bytes = volume
        .texels
        .iter()
        .flat_map(|t| t.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8))
        .collect();
    Ok(NoiseVolume {
        size: UVec3::new(x, y, z),
        bytes,
    })
}
/// Reads the clouds' authored volumes when their paths change.
fn load_volumes(
    engine: NonSend<Engine>,
    mut cells: ResMut<crate::streaming::StreamingCells>,
    mut loaded: ResMut<LoadedVolumes>,
    mut render: ResMut<CloudRender>,
) {
    let clouds = &engine.project.world.clouds;
    let key = (
        engine.project_dir.clone(),
        if clouds.enabled {
            clouds.shape_volume.clone()
        } else {
            String::new()
        },
        if clouds.enabled {
            clouds.detail_volume.clone()
        } else {
            String::new()
        },
    );
    if loaded.key.as_ref() != Some(&key) {
        for index in 0..2 {
            loaded.jobs.cancel(&mut cells, &index);
        }
        loaded.slots = [None, None];
        render.staging = None;
        render.generation = render.generation.wrapping_add(1);
        if let Some(dir) = &key.0 {
            for (index, path) in [&key.1, &key.2].into_iter().enumerate() {
                if path.trim().is_empty() {
                    continue;
                }
                let (dir, path) = (dir.clone(), path.clone());
                loaded.jobs.spawn(
                    &mut cells,
                    index,
                    crate::streaming::GLOBAL_CELL,
                    move || load_noise(&dir, &path),
                );
            }
        }
        loaded.key = Some(key);
    }
    for (index, result) in loaded.jobs.poll(&mut cells) {
        match result {
            Ok(volume) => {
                loaded.slots[index] = Some(volume);
                render.staging = NoiseStaging::build([&loaded.slots[0], &loaded.slots[1]])
                    .map(std::sync::Arc::new);
                render.generation = render.generation.wrapping_add(1);
            }
            Err(error) => crate::bridge::send(&blockloom_protocol::RuntimeMessage::Error {
                actor: "Blockloom".into(),
                message: format!("The cloud noise didn't load, baking it instead: {error}"),
            }),
        }
    }
}

impl ExtractResource<RenderApp> for CloudRender {
    type Source = Self;
    fn extract_resource(source: &Self) -> Self {
        source.clone()
    }
}
#[derive(Component, Clone, Copy, ExtractComponent)]
#[extract_app(RenderApp)]
struct CloudView;
#[derive(Resource, Default)]
struct CloudBuffer(DynamicUniformBuffer<CloudUniforms>);

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CloudPass;
/// The march's own composite, inside `CloudPass`; cloud layers draw round it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CloudMarch;
/// Where the clouds' uniforms are resolved each frame.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CloudResolve;

pub fn register(app: &mut App) {
    app.init_resource::<CloudStats>()
        .init_resource::<CloudRender>()
        .init_resource::<LoadedVolumes>()
        .add_systems(
            Update,
            (load_volumes, resolve)
                .chain()
                .in_set(CloudResolve)
                .after(crate::environment::apply_environment),
        );
    if app.get_sub_app(RenderApp).is_none() {
        return;
    }
    let handle = app
        .world_mut()
        .resource_mut::<Assets<ShaderBuffer>>()
        .add(ShaderBuffer::with_size(16, RenderAssetUsages::RENDER_WORLD));
    app.insert_resource(StatsBuffer(handle.clone()))
        .add_plugins(ExtractResourcePlugin::<StatsBuffer>::default());
    app.world_mut()
        .spawn(Readback::buffer(handle))
        .observe(read_stats);
    bevy::asset::embedded_asset!(app, "shaders/cloud_stats.wesl");
    bevy::asset::embedded_asset!(app, "shaders/cloud_bake.wesl");
    bevy::asset::embedded_asset!(app, "shaders/cloud_march.wesl");
    bevy::asset::embedded_asset!(app, "shaders/cloud_shadow.wesl");
    bevy::asset::embedded_asset!(app, "shaders/cloud_shadow_composite.wesl");
    app.add_plugins((
        ExtractResourcePlugin::<CloudRender>::default(),
        ExtractComponentPlugin::<CloudView>::default(),
    ));
    app.get_sub_app_mut(RenderApp)
        .unwrap()
        .init_resource::<CloudBuffer>()
        .init_gpu_resource::<SpecializedRenderPipelines<CloudPipeline>>()
        .init_gpu_resource::<SpecializedRenderPipelines<ShadowComposite>>()
        .add_systems(RenderStartup, init)
        .add_systems(Render, prepare.in_set(RenderSystems::PrepareResources))
        .add_systems(
            Core3d,
            // Strictly before post, as fog is: the stages are weakly chained.
            draw.after(Core3dSystems::Prepass)
                .after(Core3dSystems::MainPass)
                .in_set(CloudPass)
                .in_set(CloudMarch)
                .before(Core3dSystems::EarlyPostProcess)
                .before(Core3dSystems::PostProcess),
        );
}
fn resolve(
    mut commands: Commands,
    env: Res<Environment>,
    wind: Res<WindField>,
    engine: NonSend<Engine>,
    moon: Option<Res<crate::space::MoonState>>,
    scaling: Option<Res<crate::quality::Scaling>>,
    mut render: ResMut<CloudRender>,
    mut sources: ResMut<crate::atmosphere::AtmosphereSources>,
    views: Query<(Entity, Has<CloudView>), With<WorldCamera>>,
) {
    let mut c = env.clouds.clone();
    c.normalize();
    // The preset caps authored march quality the way the fog pass caps its
    // grid: a Low world never marches Ultra steps.
    if let Some(scaling) = scaling {
        let capped = capped_quality(c.quality, scaling.controller.quality);
        c.quality = capped;
    }
    let on = c.enabled && c.coverage > 0.0 && c.density > 0.0;
    sources.cloud_cover = if on { c.coverage } else { 0.0 };
    render.uniforms = on.then(|| {
        let (primary, light) = c.quality.steps();
        let sun = env.sun.color.to_linear().to_vec3() * env.sun.illuminance;
        let (_, noise_shape, noise_detail) = noise_atlas(render.staging.as_deref());
        CloudUniforms {
            layer: Vec4::new(c.bottom, c.top, c.tiling_km * 1000.0, c.threshold),
            shape: Vec4::new(c.coverage, c.density, c.cloud_type, c.toe),
            detail: Vec4::new(c.detail_scale, c.erosion, c.detail_speed, c.shoulder),
            profile: Vec4::new(c.anvil, c.billow, c.feather, 0.0),
            phase: Vec4::new(c.forward, c.backward, c.back_blend, c.powder),
            lighting: Vec4::new(
                c.ambient,
                c.bottom_occlusion,
                c.lightbleed,
                crate::fog::exposure_scale(env.exposure),
            ),
            moon: moon
                .as_ref()
                .map_or(Vec3::Y, |m| m.direction)
                .extend(if c.moon_shadows { 1.0 } else { 0.0 }),
            moon_color: moon.as_ref().map_or(Vec3::ZERO, |m| m.lux).extend(0.0),
            sun: env
                .sun
                .direction
                .extend(if c.sun_shadows { 1.0 } else { 0.0 }),
            sun_color: sun.extend(0.0),
            drift: Vec3::from(wind.clouds.advection).extend(0.0),
            erosion_drift: (Vec3::from(wind.clouds.erosion) * c.detail_speed).extend(0.0),
            shear: Vec4::new(c.shear[0], c.shear[1], 0.0, 0.0),
            steps: UVec4::new(primary, light, 0, engine.project.world.wind.clouds.seed),
            shadow: Vec4::new(
                c.shadow_range,
                if c.shadows { c.shadow_strength } else { 0.0 },
                0.0,
                0.0,
            ),
            noise_shape,
            noise_detail,
            ..default()
        }
    });
    for (entity, has) in &views {
        if on && !has {
            commands
                .entity(entity)
                .insert((CloudView, WorkingTargets, DepthPrepass));
        } else if !on && has {
            commands.entity(entity).remove::<CloudView>();
        }
    }
}

/// The preset's cap on authored march quality, mirroring the fog grid cap:
/// a Low world never marches Ultra steps.
fn capped_quality(
    authored: blockloom_core::clouds::CloudQuality,
    preset: blockloom_core::quality::Quality,
) -> blockloom_core::clouds::CloudQuality {
    use blockloom_core::{clouds::CloudQuality, quality::Quality};
    let cap = match preset {
        Quality::Low => CloudQuality::Low,
        Quality::Medium => CloudQuality::Medium,
        Quality::High => CloudQuality::High,
        Quality::Ultra => CloudQuality::Ultra,
    };
    if authored.steps().0 > cap.steps().0 {
        cap
    } else {
        authored
    }
}
#[derive(Resource)]
struct CloudPipeline {
    stats_layout: BindGroupLayoutDescriptor,
    stats: CachedComputePipelineId,
    shadow_layout: BindGroupLayoutDescriptor,
    shadow: CachedComputePipelineId,
    layouts: [BindGroupLayoutDescriptor; 2],
    bake_layout: BindGroupLayoutDescriptor,
    bake: CachedComputePipelineId,
    shader: Handle<Shader>,
    fullscreen: FullscreenShader,
    repeat: Sampler,
    linear: Sampler,
}
impl SpecializedRenderPipeline for CloudPipeline {
    type Key = bool;
    fn specialize(&self, multi: bool) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("cloud_march".into()),
            layout: vec![self.layouts[multi as usize].clone()],
            vertex: self.fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs: if multi {
                    vec!["MULTISAMPLED".into()]
                } else {
                    vec![]
                },
                targets: vec![
                    Some(ColorTargetState {
                        format: WORKING_FORMAT,
                        blend: None,
                        write_mask: ColorWrites::ALL
                    });
                    2
                ],
                ..default()
            }),
            ..default()
        }
    }
}
fn init(
    mut commands: Commands,
    assets: Res<AssetServer>,
    device: Res<RenderDevice>,
    cache: Res<PipelineCache>,
    fullscreen: Res<FullscreenShader>,
) {
    if device.limits().max_compute_workgroup_size_x == 0 {
        return;
    }
    let layout = |multi| {
        BindGroupLayoutDescriptor::new(
            "cloud_march",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    uniform_buffer::<CloudUniforms>(true),
                    if multi {
                        texture_depth_2d_multisampled()
                    } else {
                        texture_depth_2d()
                    },
                    // One stacked noise atlas for shape and detail both.
                    texture_3d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    texture_cube(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        )
    };
    let bake_layout = BindGroupLayoutDescriptor::new(
        "cloud_bake",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<CloudUniforms>(true),
                texture_storage_3d(TextureFormat::Rgba8Unorm, StorageTextureAccess::WriteOnly),
            ),
        ),
    );
    let bake = cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("cloud_bake".into()),
        layout: vec![bake_layout.clone()],
        shader: bevy::asset::load_embedded_asset!(assets.as_ref(), "shaders/cloud_bake.wesl"),
        entry_point: Some("bake".into()),
        ..default()
    });
    let make_sampler = |address| {
        device.create_sampler(&SamplerDescriptor {
            address_mode_u: address,
            address_mode_v: address,
            address_mode_w: address,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..default()
        })
    };
    let stats_layout = BindGroupLayoutDescriptor::new(
        "cloud_stats",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                texture_2d(TextureSampleType::Float { filterable: false }),
                storage_buffer_sized(false, std::num::NonZero::new(16)),
            ),
        ),
    );
    let stats = cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("cloud_stats".into()),
        layout: vec![stats_layout.clone()],
        shader: bevy::asset::load_embedded_asset!(assets.as_ref(), "shaders/cloud_stats.wesl"),
        entry_point: Some("measure".into()),
        ..default()
    });
    let shadow_layout = BindGroupLayoutDescriptor::new(
        "cloud_shadow",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<CloudUniforms>(true),
                // One stacked noise atlas for shape and detail both.
                texture_3d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                texture_storage_2d(WORKING_FORMAT, StorageTextureAccess::WriteOnly),
            ),
        ),
    );
    let shadow = cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("cloud_shadow".into()),
        layout: vec![shadow_layout.clone()],
        shader: bevy::asset::load_embedded_asset!(assets.as_ref(), "shaders/cloud_shadow.wesl"),
        entry_point: Some("shadow".into()),
        ..default()
    });
    let shadow_layouts = [false, true].map(|multi| {
        BindGroupLayoutDescriptor::new(
            "cloud_shadow_composite",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    uniform_buffer::<CloudUniforms>(true),
                    texture_2d(TextureSampleType::Float { filterable: false }),
                    if multi {
                        texture_depth_2d_multisampled()
                    } else {
                        texture_depth_2d()
                    },
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        )
    });
    commands.insert_resource(ShadowComposite {
        layouts: shadow_layouts,
        shader: bevy::asset::load_embedded_asset!(
            assets.as_ref(),
            "shaders/cloud_shadow_composite.wesl"
        ),
        fullscreen: fullscreen.clone(),
    });
    commands.insert_resource(CloudPipeline {
        stats_layout,
        stats,
        shadow_layout,
        shadow,
        layouts: [layout(false), layout(true)],
        bake_layout,
        bake,
        shader: bevy::asset::load_embedded_asset!(assets.as_ref(), "shaders/cloud_march.wesl"),
        fullscreen: fullscreen.clone(),
        repeat: make_sampler(AddressMode::Repeat),
        linear: make_sampler(AddressMode::ClampToEdge),
    });
}
fn texture(
    device: &RenderDevice,
    size: UVec3,
    volume: bool,
    label: &'static str,
) -> (Texture, TextureView) {
    let t = device.create_texture(&TextureDescriptor {
        label: Some(label),
        size: Extent3d {
            width: size.x,
            height: size.y,
            depth_or_array_layers: size.z,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: if volume {
            TextureDimension::D3
        } else {
            TextureDimension::D2
        },
        format: if volume {
            TextureFormat::Rgba8Unorm
        } else {
            WORKING_FORMAT
        },
        usage: TextureUsages::TEXTURE_BINDING
            | if volume {
                // The stacked noise atlas takes authored uploads plus
                // bake-then-copy fills, so it needs both copy ends.
                TextureUsages::STORAGE_BINDING | TextureUsages::COPY_DST | TextureUsages::COPY_SRC
            } else {
                TextureUsages::RENDER_ATTACHMENT | TextureUsages::STORAGE_BINDING
            },
        view_formats: &[],
    });
    let v = t.create_view(&default());
    (t, v)
}
/// Both noise page sizes: the authored size when loaded, else the bake size.
fn noise_pages(staging: Option<&NoiseStaging>) -> [UVec3; 2] {
    fn slot(staging: Option<&NoiseStaging>, slot: usize, baked: u32) -> UVec3 {
        staging
            .and_then(|s| s.page_of[slot])
            .and_then(|i| staging?.layout.pages.get(i))
            .map(|page| UVec3::new(page.size[0], page.size[1], page.size[2]))
            .unwrap_or(UVec3::splat(baked))
    }
    [
        slot(staging, 0, BAKED_SHAPE),
        slot(staging, 1, BAKED_DETAIL),
    ]
}
/// One stacked 3D atlas for shape under detail, with authored pages
/// uploaded into their slabs. Missing pages stay zero until the bake pass
/// copies its scratch textures in.
fn noise_atlas_texture(
    device: &RenderDevice,
    queue: &RenderQueue,
    staging: Option<&NoiseStaging>,
) -> ((Texture, TextureView), [UVec3; 2]) {
    let pages = noise_pages(staging);
    let size = UVec3::new(
        pages[0].x.max(pages[1].x),
        pages[0].y.max(pages[1].y),
        pages[0].z + pages[1].z,
    );
    let (t, v) = texture(device, size, true, "working_cloud_noise");
    if let Some(staging) = staging {
        let mut z = 0;
        for (slot, page) in pages.iter().enumerate() {
            if let Some((_, bytes)) = staging.page(slot) {
                queue.write_texture(
                    TexelCopyTextureInfo {
                        texture: &t,
                        mip_level: 0,
                        origin: Origin3d { x: 0, y: 0, z },
                        aspect: TextureAspect::All,
                    },
                    bytes,
                    TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(page.x * 4),
                        rows_per_image: Some(page.y),
                    },
                    Extent3d {
                        width: page.x,
                        height: page.y,
                        depth_or_array_layers: page.z,
                    },
                );
            }
            z += page.z;
        }
    }
    ((t, v), pages)
}
fn compatible_history(mut old: CloudUniforms, next: CloudUniforms) -> bool {
    let limit = next.layer.z * 0.1;
    if old.drift.truncate().distance(next.drift.truncate()) > limit
        || old
            .erosion_drift
            .truncate()
            .distance(next.erosion_drift.truncate())
            > limit
    {
        return false;
    }
    old.drift = next.drift;
    old.erosion_drift = next.erosion_drift;
    old == next
}

#[derive(Component)]
struct History {
    shadow: (Texture, TextureView),
    size: UVec2,
    color: [(Texture, TextureView); 2],
    depth: [(Texture, TextureView); 2],
    /// One stacked atlas for shape under detail: a single bind for the
    /// march and the shadow pass both.
    noise: (Texture, TextureView),
    /// Bake targets for pages with no authored volume, copied into their
    /// atlas slab once the bake lands.
    scratch: [Option<(Texture, TextureView)>; 2],
    /// Shape and detail page sizes, in order.
    pages: [UVec3; 2],
    /// Which of shape and detail still need the GPU bake.
    bake: [bool; 2],
    generation: u32,
    baked: AtomicBool,
    drawn: AtomicBool,
    current: usize,
    clip: Mat4,
    camera: Vec3,
    base: CloudUniforms,
    frame: u32,
}
#[derive(Component)]
struct ViewCloud {
    shadow_pipeline: CachedRenderPipelineId,
    offset: u32,
    pipeline: CachedRenderPipelineId,
    multi: bool,
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn prepare(
    mut commands: Commands,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
    frame: Res<FrameCount>,
    render: Res<CloudRender>,
    pipeline: Option<Res<CloudPipeline>>,
    mut pipelines: ResMut<SpecializedRenderPipelines<CloudPipeline>>,
    mut buffer: ResMut<CloudBuffer>,
    shadow: Option<Res<ShadowComposite>>,
    mut shadow_pipelines: ResMut<SpecializedRenderPipelines<ShadowComposite>>,
    mut views: Query<
        (Entity, &ExtractedView, Option<&Msaa>, Option<&mut History>),
        With<CloudView>,
    >,
    stale: Query<Entity, (With<ViewCloud>, Without<CloudView>)>,
) {
    for e in &stale {
        commands.entity(e).remove::<(ViewCloud, History)>();
    }
    buffer.0.clear();
    let (Some(base), Some(pipeline), Some(shadow)) = (render.uniforms, pipeline, shadow) else {
        return;
    };
    for (entity, view, msaa, history) in &mut views {
        let size = view.viewport.zw().max(UVec2::ONE);
        let half = scratch_size(size);
        let clip = view
            .clip_from_world
            .unwrap_or_else(|| view.clip_from_view * view.world_from_view.to_matrix().inverse());
        let camera = view.world_from_view.translation();
        let mut u = base;
        u.world_from_clip = clip.inverse();
        u.camera = camera.extend((frame.0 % 65536) as f32);
        u.screen = Vec4::new(size.x as f32, size.y as f32, 0.0, 0.0);
        match history {
            Some(mut h)
                if h.size == half
                    && h.base.steps.w == base.steps.w
                    && h.generation == render.generation =>
            {
                u.previous_clip = h.clip;
                u.previous_camera = h.camera.extend(0.0);
                u.previous_drift = h.base.drift;
                if h.drawn.swap(false, Ordering::Relaxed)
                    && h.frame.wrapping_add(1) == frame.0
                    && compatible_history(h.base, base)
                    && camera.distance(h.camera) < 500.0
                {
                    u.previous_camera.w = 1.0;
                }
                h.current ^= 1;
                h.clip = clip;
                h.camera = camera;
                h.base = base;
                h.frame = frame.0;
            }
            _ => {
                let ((noise, noise_view), pages) =
                    noise_atlas_texture(&device, &queue, render.staging.as_deref());
                let scratch = [0, 1].map(|slot| {
                    let authored = render
                        .staging
                        .as_deref()
                        .and_then(|staging| staging.page(slot))
                        .is_some();
                    (!authored).then(|| {
                        texture(
                            &device,
                            pages[slot],
                            true,
                            if slot == 0 {
                                "working_cloud_shape"
                            } else {
                                "working_cloud_detail"
                            },
                        )
                    })
                });
                commands.entity(entity).insert(History {
                    shadow: texture(
                        &device,
                        UVec3::new(2048, 2048, 1),
                        false,
                        "working_cloud_shadow",
                    ),
                    size: half,
                    color: std::array::from_fn(|_| {
                        texture(&device, half.extend(1), false, "working_cloud_history")
                    }),
                    depth: std::array::from_fn(|_| {
                        texture(&device, half.extend(1), false, "working_cloud_depth")
                    }),
                    noise: (noise, noise_view),
                    scratch,
                    pages,
                    bake: [0, 1].map(|slot| {
                        render
                            .staging
                            .as_deref()
                            .and_then(|staging| staging.page(slot))
                            .is_none()
                    }),
                    generation: render.generation,
                    baked: AtomicBool::new(false),
                    drawn: AtomicBool::new(false),
                    current: 0,
                    clip,
                    camera,
                    base,
                    frame: frame.0,
                });
            }
        }
        let multi = msaa.is_some_and(|m| m.samples() > 1);
        let id = pipelines.specialize(&cache, &pipeline, multi);
        commands.entity(entity).insert(ViewCloud {
            shadow_pipeline: shadow_pipelines.specialize(
                &cache,
                &shadow,
                (multi, view.target_format),
            ),
            offset: buffer.0.push(&u),
            pipeline: id,
            multi,
        });
    }
    buffer.0.write_buffer(&device, &queue);
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn draw(
    view: ViewQuery<(
        &ViewTarget,
        &ViewCloud,
        &History,
        &ViewPrepassTextures,
        &ViewUpsample,
        &ViewFrameUniforms,
    )>,
    pipeline: Option<Res<CloudPipeline>>,
    buffer: Res<CloudBuffer>,
    cache: Res<PipelineCache>,
    sky: Res<SkyRender>,
    images: Res<RenderAssets<GpuImage>>,
    upsample_pipeline: Res<BilateralUpsample>,
    frame: Res<FrameUniformBuffer>,
    shadow: Option<Res<ShadowComposite>>,
    stats: Res<StatsBuffer>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    mut ctx: RenderContext,
) {
    let (Some(pipeline), Some(shadow)) = (pipeline, shadow) else {
        return;
    };
    let (target, v, h, prepass, up, fu) = view.into_inner();
    let (Some(march), Some(bake), Some(uniform), Some(depth), Some(sky)) = (
        cache.get_render_pipeline(v.pipeline),
        cache.get_compute_pipeline(pipeline.bake),
        buffer.0.binding(),
        prepass.depth_only_view(),
        images.get(&sky.diffuse),
    ) else {
        return;
    };
    if cache.get_render_pipeline(up.over_view).is_none() {
        return;
    }
    let device = ctx.render_device().clone();
    if !h.baked.load(Ordering::Relaxed) {
        for (slot, page) in h.pages.iter().enumerate() {
            let size = page.x;
            if h.bake[slot]
                && let Some((_, view)) = &h.scratch[slot]
            {
                let group = device.create_bind_group(
                    "cloud_bake",
                    &cache.get_bind_group_layout(&pipeline.bake_layout),
                    &BindGroupEntries::sequential((uniform.clone(), view)),
                );
                let mut pass = ctx
                    .command_encoder()
                    .begin_compute_pass(&ComputePassDescriptor {
                        label: Some("cloud_bake"),
                        timestamp_writes: None,
                    });
                pass.set_pipeline(bake);
                pass.set_bind_group(0, &group, &[v.offset]);
                pass.dispatch_workgroups(size / 4, size / 4, size / 4);
            }
        }
        // Bake targets are page-sized; the atlas is shared, so copy each
        // fresh page into its own slab before the march samples it.
        let mut base = 0;
        for (slot, page) in h.pages.iter().enumerate() {
            if h.bake[slot]
                && let Some((texture, _)) = &h.scratch[slot]
            {
                ctx.command_encoder().copy_texture_to_texture(
                    TexelCopyTextureInfo {
                        texture,
                        mip_level: 0,
                        origin: Origin3d::ZERO,
                        aspect: TextureAspect::All,
                    },
                    TexelCopyTextureInfo {
                        texture: &h.noise.0,
                        mip_level: 0,
                        origin: Origin3d {
                            x: 0,
                            y: 0,
                            z: base,
                        },
                        aspect: TextureAspect::All,
                    },
                    Extent3d {
                        width: page.x,
                        height: page.y,
                        depth_or_array_layers: page.z,
                    },
                );
            }
            base += page.z;
        }
        h.baked.store(true, Ordering::Relaxed);
    }
    if h.base.shadow.y > 0.0 {
        let (Some(compute), Some(composite)) = (
            cache.get_compute_pipeline(pipeline.shadow),
            cache.get_render_pipeline(v.shadow_pipeline),
        ) else {
            return;
        };
        let group = device.create_bind_group(
            "cloud_shadow",
            &cache.get_bind_group_layout(&pipeline.shadow_layout),
            &BindGroupEntries::sequential((
                uniform.clone(),
                &h.noise.1,
                &pipeline.repeat,
                &h.shadow.1,
            )),
        );
        {
            let mut pass = ctx
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor {
                    label: Some("cloud_shadow"),
                    timestamp_writes: None,
                });
            pass.set_pipeline(compute);
            pass.set_bind_group(0, &group, &[v.offset]);
            pass.dispatch_workgroups(256, 256, 1);
        }
        let post = target.post_process_write();
        let group = device.create_bind_group(
            "cloud_shadow_composite",
            &cache.get_bind_group_layout(&shadow.layouts[v.multi as usize]),
            &BindGroupEntries::sequential((
                uniform.clone(),
                post.source,
                depth,
                &h.shadow.1,
                &pipeline.linear,
            )),
        );
        let mut pass = ctx
            .command_encoder()
            .begin_render_pass(&RenderPassDescriptor {
                label: Some("cloud_shadow_composite"),
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
        pass.set_pipeline(composite);
        pass.set_bind_group(0, &group, &[v.offset]);
        pass.draw(0..3, 0..1);
    }
    let group = device.create_bind_group(
        "cloud_march",
        &cache.get_bind_group_layout(&pipeline.layouts[v.multi as usize]),
        &BindGroupEntries::sequential((
            uniform,
            depth,
            &h.noise.1,
            &pipeline.repeat,
            &h.color[h.current ^ 1].1,
            &h.depth[h.current ^ 1].1,
            &sky.texture_view,
            &pipeline.linear,
        )),
    );
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    {
        let attachments = [&h.color[h.current].1, &h.depth[h.current].1].map(|view| {
            Some(RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })
        });
        let mut pass = ctx
            .command_encoder()
            .begin_render_pass(&RenderPassDescriptor {
                label: Some("cloud_march"),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        let span = diagnostics.pass_span(&mut pass, "cloud_march");
        pass.set_pipeline(march);
        pass.set_bind_group(0, &group, &[v.offset]);
        pass.draw(0..3, 0..1);
        span.end(&mut pass);
    }
    if let (Some(measure), Some(output)) = (
        cache.get_compute_pipeline(pipeline.stats),
        buffers.get(&stats.0),
    ) {
        let group = device.create_bind_group(
            "cloud_stats",
            &cache.get_bind_group_layout(&pipeline.stats_layout),
            &BindGroupEntries::sequential((
                &h.depth[h.current].1,
                output.buffer.as_entire_buffer_binding(),
            )),
        );
        let mut pass = ctx
            .command_encoder()
            .begin_compute_pass(&ComputePassDescriptor {
                label: Some("cloud_stats"),
                timestamp_writes: None,
            });
        pass.set_pipeline(measure);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    if upsample(
        &mut ctx,
        &cache,
        &upsample_pipeline,
        &frame,
        (up, fu, Some(prepass)),
        up.over_view,
        &h.color[h.current].1,
        target.main_texture_view(),
    ) {
        h.drawn.store(true, Ordering::Relaxed);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shaders_validate() {
        for multi in [false, true] {
            blockloom_core::shader_lib::validate(
                include_str!("shaders/cloud_march.wesl"),
                &[("MULTISAMPLED", multi)],
            )
            .unwrap();
        }
        blockloom_core::shader_lib::validate(include_str!("shaders/cloud_stats.wesl"), &[])
            .unwrap();
        blockloom_core::shader_lib::validate(include_str!("shaders/cloud_bake.wesl"), &[]).unwrap();
        blockloom_core::shader_lib::validate(include_str!("shaders/cloud_shadow.wesl"), &[])
            .unwrap();
        for multi in [false, true] {
            blockloom_core::shader_lib::validate(
                include_str!("shaders/cloud_shadow_composite.wesl"),
                &[("MULTISAMPLED", multi)],
            )
            .unwrap();
        }
    }
    #[test]
    fn the_preset_caps_authored_march_quality() {
        use blockloom_core::{clouds::CloudQuality, quality::Quality};
        assert_eq!(
            capped_quality(CloudQuality::Ultra, Quality::Low),
            CloudQuality::Low
        );
        assert_eq!(
            capped_quality(CloudQuality::Ultra, Quality::Medium),
            CloudQuality::Medium
        );
        assert_eq!(
            capped_quality(CloudQuality::Low, Quality::Ultra),
            CloudQuality::Low
        );
        assert_eq!(
            capped_quality(CloudQuality::Ultra, Quality::Ultra),
            CloudQuality::Ultra
        );
    }
    #[test]
    fn history_rejects_edits_and_fast_drift() {
        let base = CloudUniforms {
            layer: Vec4::new(1500.0, 3500.0, 12000.0, 0.01),
            ..default()
        };
        let mut next = base;
        next.drift.x = 10.0;
        assert!(compatible_history(base, next));
        next.shape.x = 0.5;
        assert!(!compatible_history(base, next));
        next = base;
        next.drift.x = 2000.0;
        assert!(!compatible_history(base, next));
        next = base;
        next.steps.w = 4;
        assert!(!compatible_history(base, next));
    }

    #[test]
    fn uniform_layout() {
        assert_eq!(CloudUniforms::min_size().get(), 128 + 21 * 16);
    }

    #[test]
    fn stacked_atlas_maps_each_page_onto_its_own_slab() {
        // Baked defaults: shape fills the full XY, detail a corner slab after it.
        let (size, shape, detail) = noise_atlas(None);
        assert_eq!(size, UVec3::new(128, 128, 160));
        assert_eq!(shape, Vec4::new(1.0, 1.0, 0.0, 128.0 / 160.0));
        assert_eq!(detail, Vec4::new(0.25, 0.25, 128.0 / 160.0, 32.0 / 160.0));
        // Two authored pages: shape at the base, detail stacked after it.
        let staging =
            NoiseStaging::build([&Some(noise_volume(4, 7)), &Some(noise_volume(2, 9))]).unwrap();
        let (size, shape, detail) = noise_atlas(Some(&staging));
        assert_eq!(size, UVec3::new(4, 4, 6));
        assert_eq!(shape, Vec4::new(1.0, 1.0, 0.0, 4.0 / 6.0));
        assert_eq!(detail, Vec4::new(0.5, 0.5, 4.0 / 6.0, 2.0 / 6.0));
        // One authored page: the missing one falls back to its bake size.
        let staging = NoiseStaging::build([&Some(noise_volume(4, 7)), &None]).unwrap();
        let (size, shape, detail) = noise_atlas(Some(&staging));
        assert_eq!(size, UVec3::new(32, 32, 36));
        assert_eq!(shape.z, 0.0);
        assert!((shape.w + detail.w - 1.0).abs() < 1e-6);
    }

    fn noise_volume(size: u32, fill: u8) -> NoiseVolume {
        NoiseVolume {
            size: UVec3::splat(size),
            bytes: vec![fill; size as usize * size as usize * size as usize * 4],
        }
    }

    #[test]
    fn authored_volumes_share_one_staging_buffer() {
        assert!(NoiseStaging::build([&None, &None]).is_none());
        // One page is the identity: the upload reads its own bytes back.
        let shape = noise_volume(4, 7);
        let staging = NoiseStaging::build([&Some(shape), &None]).unwrap();
        assert_eq!(staging.page_of, [Some(0), None]);
        let (size, bytes) = staging.page(0).unwrap();
        assert_eq!(size, UVec3::splat(4));
        assert!(bytes.iter().all(|byte| *byte == 7));
        assert!(staging.page(1).is_none());
        // Two pages pack back to back with the layout's offsets.
        let detail = noise_volume(2, 9);
        let both = NoiseStaging::build([&Some(noise_volume(4, 7)), &Some(detail)]).unwrap();
        assert_eq!(both.layout.atlas, [4, 4, 6]);
        let (_, shape_bytes) = both.page(0).unwrap();
        let (detail_size, detail_bytes) = both.page(1).unwrap();
        assert_eq!(detail_size, UVec3::splat(2));
        assert_eq!(shape_bytes.len(), 4 * 4 * 4 * 4);
        assert_eq!(detail_bytes.len(), 2 * 2 * 2 * 4);
        assert!(shape_bytes.iter().all(|byte| *byte == 7));
        assert!(detail_bytes.iter().all(|byte| *byte == 9));
        // The second page starts where the first ends.
        let offset = both.layout.page_offset(1) as usize * 4;
        assert_eq!(offset, shape_bytes.len());
        assert_eq!(
            &both.bytes[offset..offset + detail_bytes.len()],
            detail_bytes
        );
    }
}

#[derive(Resource)]
struct ShadowComposite {
    layouts: [BindGroupLayoutDescriptor; 2],
    shader: Handle<Shader>,
    fullscreen: FullscreenShader,
}
impl SpecializedRenderPipeline for ShadowComposite {
    type Key = (bool, TextureFormat);
    fn specialize(&self, (multi, format): Self::Key) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("cloud_shadow_composite".into()),
            layout: vec![self.layouts[multi as usize].clone()],
            vertex: self.fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs: if multi {
                    vec!["MULTISAMPLED".into()]
                } else {
                    vec![]
                },
                targets: vec![Some(ColorTargetState {
                    format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            ..default()
        }
    }
}

#[derive(Resource, Default)]
pub(crate) struct CloudStats {
    pub steps: f32,
    pub overdraw: f32,
}
#[derive(Resource, Clone)]
struct StatsBuffer(Handle<ShaderBuffer>);
impl ExtractResource<RenderApp> for StatsBuffer {
    type Source = Self;
    fn extract_resource(s: &Self) -> Self {
        s.clone()
    }
}
fn read_stats(event: On<ReadbackComplete>, mut stats: ResMut<CloudStats>) {
    if let Some(data) = event.data.get(..16) {
        let values = bytemuck::pod_read_unaligned::<[f32; 4]>(data);
        if values[2] > 0.0 {
            stats.steps = values[0];
            stats.overdraw = values[1];
        }
    }
}
