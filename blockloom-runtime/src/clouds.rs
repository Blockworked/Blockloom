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
}
#[derive(Resource, Clone, Default)]
struct CloudRender {
    uniforms: Option<CloudUniforms>,
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

pub fn register(app: &mut App) {
    app.init_resource::<CloudStats>()
        .init_resource::<CloudRender>()
        .add_systems(Update, resolve.after(crate::environment::apply_environment));
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
            draw.after(Core3dSystems::MainPass)
                .in_set(CloudPass)
                .before(Core3dSystems::EarlyPostProcess),
        );
}
fn resolve(
    mut commands: Commands,
    env: Res<Environment>,
    wind: Res<WindField>,
    engine: NonSend<Engine>,
    moon: Option<Res<crate::space::MoonState>>,
    mut render: ResMut<CloudRender>,
    mut sources: ResMut<crate::atmosphere::AtmosphereSources>,
    views: Query<(Entity, Has<CloudView>), With<WorldCamera>>,
) {
    let mut c = env.clouds.clone();
    c.normalize();
    let on = c.enabled && c.coverage > 0.0 && c.density > 0.0;
    sources.cloud_cover = if on { c.coverage } else { 0.0 };
    render.uniforms = on.then(|| {
        let (primary, light) = c.quality.steps();
        let sun = env.sun.color.to_linear().to_vec3() * env.sun.illuminance;
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
                    texture_3d(TextureSampleType::Float { filterable: true }),
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
                texture_3d(TextureSampleType::Float { filterable: true }),
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
                TextureUsages::STORAGE_BINDING
            } else {
                TextureUsages::RENDER_ATTACHMENT | TextureUsages::STORAGE_BINDING
            },
        view_formats: &[],
    });
    let v = t.create_view(&default());
    (t, v)
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
    shape: (Texture, TextureView),
    detail: (Texture, TextureView),
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
            Some(mut h) if h.size == half && h.base.steps.w == base.steps.w => {
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
                    shape: texture(&device, UVec3::splat(128), true, "working_cloud_shape"),
                    detail: texture(&device, UVec3::splat(32), true, "working_cloud_detail"),
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
        for (tex, size) in [(&h.shape.1, 128u32), (&h.detail.1, 32u32)] {
            let group = device.create_bind_group(
                "cloud_bake",
                &cache.get_bind_group_layout(&pipeline.bake_layout),
                &BindGroupEntries::sequential((uniform.clone(), tex)),
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
                &h.shape.1,
                &h.detail.1,
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
            &h.shape.1,
            &h.detail.1,
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
        assert_eq!(CloudUniforms::min_size().get(), 128 + 19 * 16);
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
