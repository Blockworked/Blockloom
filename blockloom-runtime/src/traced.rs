//! The render passes Blockloom adds to ray-traced lighting, 3D only: the
//! realtime path tracer and the denoiser. `ray_tracing.rs` decides which a
//! world camera carries; this is the render-world half.
//!
//! `TracedPaths` lights the G-buffer with fresh paths every frame
//! (`shaders/traced_paths.wesl`), in place of Solari and the deferred
//! lighting pass. `TracedDenoiser` then filters whichever of the two drew the
//! frame (`shaders/denoise.wesl`): a temporal pass, five a-trous passes and a
//! resolve. It runs once the opaque pass is done, so it only ever sees the
//! traced pixels plus forward surfaces and the sky, which it tells apart by
//! an empty G-buffer and an empty depth.

use bevy::camera::Hdr;
use bevy::core_pipeline::core_3d::{main_opaque_pass_3d, main_transparent_pass_3d};
use bevy::core_pipeline::prepass::{
    DeferredPrepass, DepthPrepass, MotionVectorPrepass, PreviousViewData,
    PreviousViewUniformOffset, PreviousViewUniforms, ViewPrepassTextures,
};
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::diagnostic::FrameCount;
use bevy::pbr::deferred::SkipDeferredLighting;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::render_resource::binding_types::{
    texture_2d, texture_depth_2d, texture_storage_2d, uniform_buffer, uniform_buffer_sized,
};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::sync_world::RenderEntity;
use bevy::render::view::{ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms};
use bevy::render::{
    ExtractSchedule, MainWorld, Render, RenderApp, RenderStartup, RenderSystems, init_gpu_resource,
};
use bevy::shader::{ShaderCacheError, ShaderDefVal};
use bevy::solari::SolariPlugins;
use bevy::solari::scene::RaytracingSceneBindings;
use std::num::NonZero;

/// Wavelet passes, each twice as wide as the last: 1, 2, 4, 8 and 16 pixels.
const ATROUS_PASSES: u32 = 5;
const HISTORY_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

/// Realtime path tracing on a world camera.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[require(Hdr, DeferredPrepass, DepthPrepass, MotionVectorPrepass)]
pub struct TracedPaths {
    /// Paths per pixel each frame.
    pub paths: u32,
    /// Most bounces a path takes.
    pub bounces: u32,
}

/// The spatiotemporal filter over a camera's traced lighting.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[require(DeferredPrepass, DepthPrepass, MotionVectorPrepass)]
pub struct TracedDenoiser {
    /// Frames a rough surface's history holds at most.
    pub max_history: f32,
    /// Throw the history away next frame, as after a camera cut. Cleared
    /// once extracted.
    pub reset: bool,
}

impl Default for TracedDenoiser {
    fn default() -> Self {
        Self {
            max_history: 32.0,
            reset: true,
        }
    }
}

pub struct TracedPlugin;

impl Plugin for TracedPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "shaders/traced_paths.wesl");
        bevy::asset::embedded_asset!(app, "shaders/denoise.wesl");
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        let features = render_app.world().resource::<RenderDevice>().features();
        if !features.contains(SolariPlugins::required_wgpu_features()) {
            return;
        }
        render_app
            .add_systems(
                RenderStartup,
                init_pipelines.after(init_gpu_resource::<RaytracingSceneBindings>),
            )
            .add_systems(ExtractSchedule, extract_traced)
            .add_systems(
                Render,
                (
                    skip_deferred_lighting.in_set(RenderSystems::PrepareAssets),
                    (prepare_paths, prepare_denoiser).in_set(RenderSystems::PrepareResources),
                    report_broken_pipelines,
                ),
            )
            .add_systems(
                Core3d,
                (
                    trace_paths
                        .before(main_opaque_pass_3d)
                        .in_set(Core3dSystems::MainPass),
                    denoise
                        .after(main_opaque_pass_3d)
                        .before(main_transparent_pass_3d)
                        .in_set(Core3dSystems::MainPass),
                ),
            );
    }
}

fn extract_traced(mut main_world: ResMut<MainWorld>, mut commands: Commands) {
    let mut cameras = main_world.query::<(
        RenderEntity,
        &Camera,
        Option<&TracedPaths>,
        Option<&mut TracedDenoiser>,
    )>();
    for (entity, camera, paths, denoiser) in cameras.iter_mut(&mut main_world) {
        let Ok(mut target) = commands.get_entity(entity) else {
            continue;
        };
        match paths.filter(|_| camera.is_active) {
            Some(paths) => {
                target.insert(*paths);
            }
            None => {
                target.remove::<(TracedPaths, PathUniforms)>();
            }
        }
        match denoiser {
            Some(mut denoiser) if camera.is_active => {
                target.insert(*denoiser);
                if denoiser.reset {
                    denoiser.reset = false;
                }
            }
            _ => {
                target.remove::<(TracedDenoiser, DenoiserTextures)>();
            }
        }
    }
}

/// Solari's own extraction takes this off every camera it doesn't light, so
/// it goes back on after the extracted commands have landed.
fn skip_deferred_lighting(
    mut commands: Commands,
    views: Query<Entity, (With<TracedPaths>, Without<SkipDeferredLighting>)>,
) {
    for view in &views {
        commands.entity(view).insert(SkipDeferredLighting);
    }
}

#[derive(Resource)]
struct TracedPipelines {
    paths_layout: BindGroupLayoutDescriptor,
    paths: CachedComputePipelineId,
    denoise_layout: BindGroupLayoutDescriptor,
    temporal: CachedComputePipelineId,
    atrous: Vec<CachedComputePipelineId>,
    resolve: CachedComputePipelineId,
}

/// Tells the editor, once, about a pass whose shader won't compile, which
/// otherwise just leaves the frame unfiltered or unlit.
fn report_broken_pipelines(
    pipelines: Option<Res<TracedPipelines>>,
    pipeline_cache: Res<PipelineCache>,
    mut reported: Local<bool>,
) {
    let Some(pipelines) = pipelines else {
        return;
    };
    if *reported {
        return;
    }
    let ids = [pipelines.paths, pipelines.temporal, pipelines.resolve];
    for id in ids.iter().chain(&pipelines.atrous) {
        // Not loaded yet is only a wait.
        if let CachedPipelineState::Err(
            error @ (ShaderCacheError::ProcessShaderError(_)
            | ShaderCacheError::CreateShaderModule(_)),
        ) = pipeline_cache.get_compute_pipeline_state(*id)
        {
            *reported = true;
            crate::bridge::send(&blockloom_protocol::RuntimeMessage::Error {
                actor: "Blockloom".into(),
                message: format!("A ray tracing shader didn't compile: {error}"),
            });
            return;
        }
    }
}

/// `PathUniforms` in `shaders/traced_paths.wesl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PathSettings {
    paths: u32,
    bounces: u32,
    frame: u32,
    _padding: u32,
}

/// `DenoiseUniforms` in `shaders/denoise.wesl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DenoiseSettings {
    reset: u32,
    max_history: f32,
    min_history: f32,
    color_sigma: f32,
}

fn init_pipelines(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>,
    scene: Res<RaytracingSceneBindings>,
    assets: Res<AssetServer>,
) {
    let paths_layout = BindGroupLayoutDescriptor::new(
        "traced_paths_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                texture_storage_2d(TextureFormat::Rgba16Float, StorageTextureAccess::WriteOnly),
                texture_2d(TextureSampleType::Uint),
                texture_depth_2d(),
                uniform_buffer::<ViewUniform>(true),
                uniform_buffer_sized(false, NonZero::new(size_of::<PathSettings>() as u64)),
            ),
        ),
    );
    let paths = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("traced_paths".into()),
        layout: vec![scene.bind_group_layout.clone(), paths_layout.clone()],
        shader: bevy::asset::load_embedded_asset!(assets.as_ref(), "shaders/traced_paths.wesl"),
        entry_point: Some("trace_paths".into()),
        ..default()
    });

    let history = || texture_2d(TextureSampleType::Float { filterable: false });
    let written = || texture_storage_2d(HISTORY_FORMAT, StorageTextureAccess::WriteOnly);
    let denoise_layout = BindGroupLayoutDescriptor::new(
        "traced_denoise_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                texture_storage_2d(TextureFormat::Rgba16Float, StorageTextureAccess::ReadWrite),
                texture_2d(TextureSampleType::Uint),
                texture_depth_2d(),
                history(),
                uniform_buffer::<ViewUniform>(true),
                uniform_buffer::<PreviousViewData>(true),
                uniform_buffer_sized(false, NonZero::new(size_of::<DenoiseSettings>() as u64)),
                history(),
                history(),
                history(),
                written(),
                written(),
                written(),
                history(),
                written(),
            ),
        ),
    );
    let shader = bevy::asset::load_embedded_asset!(assets.as_ref(), "shaders/denoise.wesl");
    let denoise = |label: String, entry: &'static str, step: u32, feedback: bool| {
        let mut shader_defs = vec![ShaderDefVal::UInt("STEP".into(), step)];
        if feedback {
            shader_defs.push("FEEDBACK".into());
        }
        pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some(label.into()),
            layout: vec![denoise_layout.clone()],
            shader: shader.clone(),
            shader_defs,
            entry_point: Some(entry.into()),
            ..default()
        })
    };
    let temporal = denoise("traced_denoise_temporal".into(), "temporal", 1, false);
    let atrous = (0..ATROUS_PASSES)
        .map(|pass| {
            denoise(
                format!("traced_denoise_atrous_{pass}"),
                "atrous",
                1 << pass,
                pass == 0,
            )
        })
        .collect();
    let resolve = denoise("traced_denoise_resolve".into(), "resolve", 1, false);
    commands.insert_resource(TracedPipelines {
        paths_layout,
        paths,
        denoise_layout,
        temporal,
        atrous,
        resolve,
    });
}

/// A path traced view's settings on the GPU.
#[derive(Component)]
struct PathUniforms(Buffer);

fn prepare_paths(
    mut commands: Commands,
    views: Query<(Entity, &TracedPaths, Option<&PathUniforms>)>,
    frame: Res<FrameCount>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    for (view, paths, uniforms) in &views {
        let settings = PathSettings {
            paths: paths.paths.max(1),
            bounces: paths.bounces.max(1),
            frame: frame.0,
            _padding: 0,
        };
        match uniforms {
            Some(uniforms) => queue.write_buffer(&uniforms.0, 0, bytemuck::bytes_of(&settings)),
            None => {
                let buffer = device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("traced_paths_uniforms"),
                    contents: bytemuck::bytes_of(&settings),
                    usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                });
                commands.entity(view).insert(PathUniforms(buffer));
            }
        }
    }
}

fn trace_paths(
    view: ViewQuery<(
        &PathUniforms,
        &ExtractedCamera,
        &ViewTarget,
        &ViewPrepassTextures,
        &ViewUniformOffset,
    )>,
    pipelines: Option<Res<TracedPipelines>>,
    pipeline_cache: Res<PipelineCache>,
    scene: Res<RaytracingSceneBindings>,
    view_uniforms: Res<ViewUniforms>,
    mut ctx: RenderContext,
) {
    let (uniforms, camera, target, prepass, offset) = view.into_inner();
    let Some(pipelines) = pipelines else {
        return;
    };
    let (Some(pipeline), Some(scene_group), Some(size), Some(gbuffer), Some(depth), Some(views)) = (
        pipeline_cache.get_compute_pipeline(pipelines.paths),
        &scene.bind_group,
        camera.physical_viewport_size,
        prepass.deferred_view(),
        prepass.depth_only_view(),
        view_uniforms.uniforms.binding(),
    ) else {
        return;
    };
    let attachment = target.get_unsampled_color_attachment();
    let group = ctx.render_device().create_bind_group(
        "traced_paths_group",
        &pipeline_cache.get_bind_group_layout(&pipelines.paths_layout),
        &BindGroupEntries::sequential((
            attachment.view,
            gbuffer,
            depth,
            views,
            uniforms.0.as_entire_binding(),
        )),
    );
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let encoder = ctx.command_encoder();
    // The first to draw into the view clears it, as Solari does.
    if matches!(attachment.ops.load, LoadOp::Clear(_)) {
        encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("traced_paths_clear"),
            color_attachments: &[Some(attachment)],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
        label: Some("traced_paths"),
        timestamp_writes: None,
    });
    let span = diagnostics.time_span(&mut pass, "traced/paths");
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, scene_group, &[]);
    pass.set_bind_group(1, &group, &[offset.offset]);
    pass.dispatch_workgroups(size.x.div_ceil(8), size.y.div_ceil(8), 1);
    span.end(&mut pass);
}

/// One history image and the pair it ping-pongs with. The textures are held
/// only to keep them alive.
struct Pair(#[allow(dead_code)] [Texture; 2], [TextureView; 2]);

impl Pair {
    fn new(device: &RenderDevice, label: &'static str, size: UVec2) -> Self {
        let make = || {
            let texture = device.create_texture(&TextureDescriptor {
                label: Some(label),
                size: Extent3d {
                    width: size.x,
                    height: size.y,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: HISTORY_FORMAT,
                usage: TextureUsages::TEXTURE_BINDING | TextureUsages::STORAGE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&TextureViewDescriptor::default());
            (texture, view)
        };
        let (a, b) = (make(), make());
        Self([a.0, b.0], [a.1, b.1])
    }
}

/// A denoised view's history and scratch, render world. Rebuilt, with the
/// history thrown away, whenever the view changes size.
#[derive(Component)]
struct DenoiserTextures {
    size: UVec2,
    color: Pair,
    moments: Pair,
    guide: Pair,
    filter: Pair,
    uniforms: Buffer,
    /// Which of each history pair this frame writes.
    current: usize,
}

fn prepare_denoiser(
    mut commands: Commands,
    mut views: Query<(
        Entity,
        &TracedDenoiser,
        &ExtractedCamera,
        Option<&mut DenoiserTextures>,
    )>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    for (view, denoiser, camera, textures) in &mut views {
        let Some(size) = camera.physical_viewport_size else {
            continue;
        };
        let mut settings = DenoiseSettings {
            reset: u32::from(denoiser.reset),
            max_history: denoiser.max_history.max(1.0),
            min_history: 4.0_f32.min(denoiser.max_history.max(1.0)),
            color_sigma: 4.0,
        };
        match textures {
            Some(mut textures) if textures.size == size => {
                textures.current = 1 - textures.current;
                queue.write_buffer(&textures.uniforms, 0, bytemuck::bytes_of(&settings));
            }
            _ => {
                settings.reset = 1;
                let uniforms = device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("traced_denoise_uniforms"),
                    contents: bytemuck::bytes_of(&settings),
                    usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                });
                commands.entity(view).insert(DenoiserTextures {
                    size,
                    color: Pair::new(&device, "traced_denoise_color", size),
                    moments: Pair::new(&device, "traced_denoise_moments", size),
                    guide: Pair::new(&device, "traced_denoise_guide", size),
                    filter: Pair::new(&device, "traced_denoise_filter", size),
                    uniforms,
                    current: 0,
                });
            }
        }
    }
}

#[allow(clippy::type_complexity)]
fn denoise(
    view: ViewQuery<(
        &DenoiserTextures,
        &ViewTarget,
        &ViewPrepassTextures,
        &ViewUniformOffset,
        &PreviousViewUniformOffset,
    )>,
    pipelines: Option<Res<TracedPipelines>>,
    pipeline_cache: Res<PipelineCache>,
    view_uniforms: Res<ViewUniforms>,
    previous_uniforms: Res<PreviousViewUniforms>,
    mut ctx: RenderContext,
) {
    let (textures, target, prepass, offset, previous_offset) = view.into_inner();
    let Some(pipelines) = pipelines else {
        return;
    };
    let (Some(gbuffer), Some(depth), Some(motion), Some(views), Some(previous_views)) = (
        prepass.deferred_view(),
        prepass.depth_only_view(),
        prepass.motion_vectors_view(),
        view_uniforms.uniforms.binding(),
        previous_uniforms.uniforms.binding(),
    ) else {
        return;
    };
    let Some(temporal) = pipeline_cache.get_compute_pipeline(pipelines.temporal) else {
        return;
    };
    let Some(resolve) = pipeline_cache.get_compute_pipeline(pipelines.resolve) else {
        return;
    };
    let Some(atrous) = pipelines
        .atrous
        .iter()
        .map(|id| pipeline_cache.get_compute_pipeline(*id))
        .collect::<Option<Vec<_>>>()
    else {
        return;
    };

    let output = target.get_unsampled_color_attachment().view;
    let layout = pipeline_cache.get_bind_group_layout(&pipelines.denoise_layout);
    let (current, previous) = (textures.current, 1 - textures.current);
    let t = textures;
    // Every pass binds the lot; which filter image is read and which written
    // is all that changes.
    let group = |read: usize, write: usize| {
        ctx.render_device().create_bind_group(
            "traced_denoise_group",
            &layout,
            &BindGroupEntries::sequential((
                output,
                gbuffer,
                depth,
                motion,
                views.clone(),
                previous_views.clone(),
                t.uniforms.as_entire_binding(),
                &t.color.1[previous],
                &t.moments.1[previous],
                &t.guide.1[previous],
                &t.color.1[current],
                &t.moments.1[current],
                &t.guide.1[current],
                &t.filter.1[read],
                &t.filter.1[write],
            )),
        )
    };
    // The temporal pass writes filter 0; each wavelet pass flips.
    let mut groups = vec![group(1, 0)];
    let mut read = 0;
    for _ in 0..ATROUS_PASSES {
        groups.push(group(read, 1 - read));
        read = 1 - read;
    }
    groups.push(group(read, 1 - read));

    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let mut pass = ctx
        .command_encoder()
        .begin_compute_pass(&ComputePassDescriptor {
            label: Some("traced_denoise"),
            timestamp_writes: None,
        });
    let span = diagnostics.time_span(&mut pass, "traced/denoise");
    let (x, y) = (t.size.x.div_ceil(8), t.size.y.div_ceil(8));
    let offsets = [offset.offset, previous_offset.offset];
    let stages = std::iter::once(temporal)
        .chain(atrous)
        .chain(std::iter::once(resolve));
    for (pipeline, group) in stages.zip(&groups) {
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &offsets);
        pass.dispatch_workgroups(x, y, 1);
    }
    span.end(&mut pass);
}
