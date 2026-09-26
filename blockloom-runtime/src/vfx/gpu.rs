//! The render-world half: CPU pools' bytes written into their buffers, and
//! the GPU sim dispatched once a frame, after the depth prepass (which
//! `Collide` reads) and before the main pass (which draws the result).

use super::SimParams;
use bevy::core_pipeline::prepass::ViewPrepassTextures;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{
    storage_buffer_read_only_sized, storage_buffer_sized, texture_depth_2d,
    texture_depth_2d_multisampled, uniform_buffer,
};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::storage::{GpuShaderBuffer, ShaderBuffer};
use bevy::render::view::{ExtractedView, Msaa};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::Shader;
use std::sync::Arc;

pub fn register(app: &mut App) {
    app.init_resource::<VfxFrame>()
        .add_plugins(ExtractResourcePlugin::<VfxFrame>::default());
    let Some(render) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render.add_systems(Render, upload.in_set(RenderSystems::PrepareResources));
}

/// The sim's own registration: 3D only, since the depth buffer it collides
/// with and the views it runs for are 3D's.
pub fn register_sim(app: &mut App, shader: Handle<Shader>) {
    app.add_plugins(ExtractComponentPlugin::<VfxView>::default());
    let Some(render) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render
        .insert_resource(SimShader(shader))
        .init_resource::<SimUniforms>()
        .add_systems(RenderStartup, init_pipelines)
        .add_systems(Render, prepare_sims.in_set(RenderSystems::PrepareResources))
        .add_systems(
            Core3d,
            simulate
                .after(Core3dSystems::Prepass)
                .before(Core3dSystems::MainPass),
        );
}

/// Marks the camera the sim collides against and meters for.
#[derive(Component, ExtractComponent, Clone, Copy, Default)]
#[extract_app(RenderApp)]
pub struct VfxView;

/// A CPU pool's bytes for this frame.
#[derive(Clone)]
pub struct Upload {
    pub buffer: Handle<ShaderBuffer>,
    pub bytes: Arc<Vec<u8>>,
}

/// One GPU emitter to step this frame. The camera half of `params` is filled
/// in the render world.
#[derive(Clone)]
pub struct GpuStep {
    pub params: SimParams,
    pub particles: Handle<ShaderBuffer>,
    pub trail: Handle<ShaderBuffer>,
    pub state: Handle<ShaderBuffer>,
    pub surface: Handle<ShaderBuffer>,
    pub capacity: u32,
}

/// Everything the particle half hands the render world this frame.
#[derive(Resource, Default, Clone)]
pub struct VfxFrame {
    pub uploads: Vec<Upload>,
    pub steps: Vec<GpuStep>,
    /// The actor shapes every GPU step collides with.
    pub shapes: Option<Handle<ShaderBuffer>>,
}

impl ExtractResource<RenderApp> for VfxFrame {
    type Source = VfxFrame;

    fn extract_resource(source: &Self::Source) -> Self {
        source.clone()
    }
}

fn upload(
    frame: Option<Res<VfxFrame>>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    queue: Res<RenderQueue>,
) {
    let Some(frame) = frame else {
        return;
    };
    for upload in &frame.uploads {
        let Some(gpu) = buffers.get(&upload.buffer) else {
            continue;
        };
        let len = (upload.bytes.len() as u64).min(gpu.buffer.size()) as usize;
        if len > 0 {
            queue.write_buffer(&gpu.buffer, 0, &upload.bytes[..len]);
        }
    }
}

#[derive(Resource)]
struct SimShader(Handle<Shader>);

#[derive(Resource)]
struct SimPipelines {
    /// Indexed by whether the depth is multisampled.
    layouts: [BindGroupLayoutDescriptor; 2],
    begin: [CachedComputePipelineId; 2],
    simulate: [CachedComputePipelineId; 2],
    /// Stands in for a view with no depth prepass: all sky, so nothing hits.
    blank_depth: TextureView,
}

fn init_pipelines(
    mut commands: Commands,
    shader: Res<SimShader>,
    device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
) {
    // No compute on this device: the main world already put every emitter
    // on the CPU pool (`VfxSupport`).
    if device.limits().max_compute_workgroup_size_x == 0 {
        return;
    }
    let layout = |multisampled: bool| {
        let depth = if multisampled {
            texture_depth_2d_multisampled()
        } else {
            texture_depth_2d()
        };
        BindGroupLayoutDescriptor::new(
            "vfx_sim_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    uniform_buffer::<SimParams>(true),
                    storage_buffer_sized(false, None),
                    storage_buffer_sized(false, None),
                    storage_buffer_sized(false, None),
                    storage_buffer_read_only_sized(false, None),
                    depth,
                    storage_buffer_read_only_sized(false, None),
                ),
            ),
        )
    };
    let layouts = [layout(false), layout(true)];
    let pipeline = |entry: &'static str, multisampled: bool| {
        pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some(format!("vfx_{entry}").into()),
            layout: vec![layouts[multisampled as usize].clone()],
            shader: shader.0.clone(),
            shader_defs: if multisampled {
                vec!["MULTISAMPLED".into()]
            } else {
                vec![]
            },
            entry_point: Some(entry.into()),
            ..default()
        })
    };
    let begin = [pipeline("begin", false), pipeline("begin", true)];
    let simulate = [pipeline("simulate", false), pipeline("simulate", true)];
    let texture = device.create_texture(&TextureDescriptor {
        label: Some("vfx_blank_depth"),
        size: Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Depth32Float,
        usage: TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let blank_depth = texture.create_view(&TextureViewDescriptor::default());
    commands.insert_resource(SimPipelines {
        layouts,
        begin,
        simulate,
        blank_depth,
    });
}

#[derive(Resource, Default)]
struct SimUniforms {
    buffer: DynamicUniformBuffer<SimParams>,
    offsets: Vec<u32>,
}

/// Every step's uniforms, with the collision camera's matrices in.
fn prepare_sims(
    frame: Option<Res<VfxFrame>>,
    views: Query<&ExtractedView, With<VfxView>>,
    mut uniforms: ResMut<SimUniforms>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let uniforms = &mut *uniforms;
    uniforms.buffer.clear();
    uniforms.offsets.clear();
    let Some(frame) = frame else {
        return;
    };
    let view = views.iter().next();
    for step in &frame.steps {
        let mut params = step.params;
        if let Some(view) = view {
            let clip_from_world = view.clip_from_world.unwrap_or_else(|| {
                view.clip_from_view * view.world_from_view.to_matrix().inverse()
            });
            params.clip_from_world = clip_from_world;
            params.world_from_clip = clip_from_world.inverse();
            let size = view.viewport.zw().max(UVec2::ONE).as_vec2();
            params.screen.x = size.x;
            params.screen.y = size.y;
            params.screen.z = 1.0;
            params.size.z = view.clip_from_view.col(1).y;
            params.size.w = size.x / size.y;
        }
        uniforms.offsets.push(uniforms.buffer.push(&params));
    }
    uniforms.buffer.write_buffer(&device, &queue);
}

fn simulate(
    view: ViewQuery<(Option<&ViewPrepassTextures>, Option<&Msaa>), With<VfxView>>,
    frame: Option<Res<VfxFrame>>,
    pipelines: Option<Res<SimPipelines>>,
    uniforms: Res<SimUniforms>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    cache: Res<PipelineCache>,
    count: Res<FrameCount>,
    mut last: Local<Option<u32>>,
    mut ctx: RenderContext,
) {
    let (prepass, msaa) = view.into_inner();
    let (Some(frame), Some(pipelines), Some(binding)) =
        (frame, pipelines, uniforms.buffer.binding())
    else {
        return;
    };
    // Once a frame, whichever view comes first.
    if *last == Some(count.0) || frame.steps.is_empty() {
        return;
    }
    *last = Some(count.0);
    let depth = prepass.and_then(ViewPrepassTextures::depth_only_view);
    let multisampled = depth.is_some() && msaa.is_some_and(|msaa| msaa.samples() > 1);
    let depth = depth.unwrap_or(&pipelines.blank_depth);
    let (Some(begin), Some(step_pipeline)) = (
        cache.get_compute_pipeline(pipelines.begin[multisampled as usize]),
        cache.get_compute_pipeline(pipelines.simulate[multisampled as usize]),
    ) else {
        return;
    };
    let Some(shapes) = frame.shapes.as_ref().and_then(|shapes| buffers.get(shapes)) else {
        return;
    };
    let layout = cache.get_bind_group_layout(&pipelines.layouts[multisampled as usize]);
    let device = ctx.render_device().clone();
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let mut pass = ctx
        .command_encoder()
        .begin_compute_pass(&ComputePassDescriptor {
            label: Some("vfx_sim"),
            timestamp_writes: None,
        });
    let span = diagnostics.pass_span(&mut pass, "vfx_sim");
    for (step, offset) in frame.steps.iter().zip(&uniforms.offsets) {
        let (Some(particles), Some(trail), Some(state), Some(surface)) = (
            buffers.get(&step.particles),
            buffers.get(&step.trail),
            buffers.get(&step.state),
            buffers.get(&step.surface),
        ) else {
            continue;
        };
        let bind_group = device.create_bind_group(
            "vfx_sim",
            &layout,
            &BindGroupEntries::sequential((
                binding.clone(),
                particles.buffer.as_entire_buffer_binding(),
                trail.buffer.as_entire_buffer_binding(),
                state.buffer.as_entire_buffer_binding(),
                surface.buffer.as_entire_buffer_binding(),
                depth,
                shapes.buffer.as_entire_buffer_binding(),
            )),
        );
        pass.set_bind_group(0, &bind_group, &[*offset]);
        pass.set_pipeline(begin);
        pass.dispatch_workgroups(1, 1, 1);
        pass.set_pipeline(step_pipeline);
        pass.dispatch_workgroups(step.capacity.div_ceil(64), 1, 1);
    }
    span.end(&mut pass);
}
