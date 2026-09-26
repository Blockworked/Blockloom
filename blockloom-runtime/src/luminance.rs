//! Scene luminance: a compute pass meters the world camera's exposed image
//! before tonemapping, and a GPU readback brings the answer home a frame or
//! two later. The atmosphere sample turns it into nits on the fixed tick, so
//! the VM and compiled logic read one number per tick.

use crate::hdr::{HdrDebugView2d, HdrDebugView3d, HdrTone2d, HdrTone3d};
use crate::world::WorldCamera;
use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::fullscreen_material::fullscreen_material_system;
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::{Core2d, Core2dSystems, Core3d, Core3dSystems};
use bevy::prelude::*;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{storage_buffer_sized, texture_2d};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::storage::{GpuShaderBuffer, ShaderBuffer};
use bevy::render::view::ViewTarget;
use bevy::render::{RenderApp, RenderStartup};
use blockloom_core::scene::Mode;
use std::num::NonZero;

/// Bytes the shader writes: log-average, peak, a measured flag, padding.
const RESULT_BYTES: u64 = 16;

pub fn register(app: &mut App) {
    app.init_resource::<SceneLuminance>();
    // A bare test world has no render assets to meter with.
    if !app.world().contains_resource::<Assets<ShaderBuffer>>() {
        return;
    }
    bevy::asset::embedded_asset!(app, "shaders/luminance.wesl");
    let buffer =
        app.world_mut()
            .resource_mut::<Assets<ShaderBuffer>>()
            .add(ShaderBuffer::with_size(
                RESULT_BYTES,
                RenderAssetUsages::RENDER_WORLD,
            ));
    app.insert_resource(LuminanceBuffer(buffer.clone()))
        .add_plugins((
            ExtractComponentPlugin::<LuminanceMeter>::default(),
            ExtractResourcePlugin::<LuminanceBuffer>::default(),
        ))
        .add_systems(Update, meter_world_cameras);
    app.world_mut()
        .spawn(Readback::buffer(buffer))
        .observe(read_meter);

    let Some(render) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render
        .add_systems(RenderStartup, init_pipeline)
        .add_systems(
            Core3d,
            meter
                .in_set(Core3dSystems::PostProcess)
                .before(fullscreen_material_system::<HdrDebugView3d>)
                .before(fullscreen_material_system::<HdrTone3d>)
                .before(tonemapping),
        )
        .add_systems(
            Core2d,
            meter
                .in_set(Core2dSystems::PostProcess)
                .before(fullscreen_material_system::<HdrDebugView2d>)
                .before(fullscreen_material_system::<HdrTone2d>)
                .before(tonemapping),
        );
}

/// The last metered frame, in exposed units (1.0 is paper white).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct SceneLuminance {
    pub average: f32,
    pub peak: f32,
    /// False until a metered frame has come back.
    pub measured: bool,
}

impl SceneLuminance {
    /// Nits. In 3D the scene is physically lit, so exposure is undone
    /// (Bevy exposes by 1 / (1.2 * 2^ev)); 2D colors count paper white as 1.
    pub fn nits(&self, mode: Mode, ev: f32, paper_white_nits: f32) -> f32 {
        if !self.measured {
            return 0.0;
        }
        match mode {
            Mode::ThreeD => self.average * 1.2 * ev.exp2(),
            Mode::TwoD => self.average * paper_white_nits,
        }
    }
}

/// Marks the camera whose image is metered.
#[derive(Component, ExtractComponent, Clone, Copy, Default)]
#[extract_app(RenderApp)]
pub struct LuminanceMeter;

#[derive(Resource, Clone)]
struct LuminanceBuffer(Handle<ShaderBuffer>);

impl ExtractResource<RenderApp> for LuminanceBuffer {
    type Source = LuminanceBuffer;

    fn extract_resource(source: &Self::Source) -> Self {
        source.clone()
    }
}

fn meter_world_cameras(
    mut commands: Commands,
    cameras: Query<Entity, (With<WorldCamera>, Without<LuminanceMeter>)>,
) {
    for camera in &cameras {
        commands.entity(camera).insert(LuminanceMeter);
    }
}

fn read_meter(event: On<ReadbackComplete>, mut luminance: ResMut<SceneLuminance>) {
    let Some(values) = event
        .data
        .get(..RESULT_BYTES as usize)
        .map(bytemuck::pod_read_unaligned::<[f32; 4]>)
    else {
        return;
    };
    if values[2] != 1.0 || !values[0].is_finite() {
        return;
    }
    luminance.set_if_neq(SceneLuminance {
        average: values[0],
        peak: values[1],
        measured: true,
    });
}

#[derive(Resource)]
struct MeterPipeline {
    layout: BindGroupLayoutDescriptor,
    pipeline: CachedComputePipelineId,
}

fn init_pipeline(
    mut commands: Commands,
    assets: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
    render_device: Res<RenderDevice>,
) {
    // No compute shaders on this device (WebGL, and the downlevel limits
    // Bevy simulates it with): queueing or dispatching the meter trips
    // validation and quits the run, so the reading stays unmeasured and the
    // atmosphere reports zero instead (see `SceneLuminance::nits`).
    if render_device.limits().max_compute_workgroup_size_x == 0 {
        return;
    }
    let layout = BindGroupLayoutDescriptor::new(
        "luminance_meter_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                texture_2d(TextureSampleType::Float { filterable: false }),
                storage_buffer_sized(false, NonZero::new(RESULT_BYTES)),
            ),
        ),
    );
    let pipeline = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("luminance_meter".into()),
        layout: vec![layout.clone()],
        shader: bevy::asset::load_embedded_asset!(assets.as_ref(), "shaders/luminance.wesl"),
        entry_point: Some("meter".into()),
        ..default()
    });
    commands.insert_resource(MeterPipeline { layout, pipeline });
}

fn meter(
    view: ViewQuery<&ViewTarget, With<LuminanceMeter>>,
    meter: Option<Res<MeterPipeline>>,
    buffer: Option<Res<LuminanceBuffer>>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    pipeline_cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let target = view.into_inner();
    let (Some(meter), Some(buffer)) = (meter, buffer) else {
        return;
    };
    let (Some(pipeline), Some(result)) = (
        pipeline_cache.get_compute_pipeline(meter.pipeline),
        buffers.get(&buffer.0),
    ) else {
        return;
    };
    let bind_group = ctx.render_device().create_bind_group(
        "luminance_meter",
        &pipeline_cache.get_bind_group_layout(&meter.layout),
        &BindGroupEntries::sequential((
            target.main_texture_view(),
            result.buffer.as_entire_buffer_binding(),
        )),
    );
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let span = diagnostics.time_span(ctx.command_encoder(), "luminance_meter");
    {
        let mut pass = ctx
            .command_encoder()
            .begin_compute_pass(&ComputePassDescriptor {
                label: Some("luminance_meter"),
                timestamp_writes: None,
            });
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_pipeline(pipeline);
        pass.dispatch_workgroups(1, 1, 1);
    }
    span.end(ctx.command_encoder());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_meter_shader_compiles() {
        blockloom_core::shader_lib::validate(include_str!("shaders/luminance.wesl"), &[])
            .unwrap_or_else(|error| panic!("{error}"));
    }

    #[test]
    fn nits_undo_exposure_in_3d_and_count_paper_white_in_2d() {
        let metered = SceneLuminance {
            average: 0.5,
            peak: 2.0,
            measured: true,
        };
        assert_eq!(metered.nits(Mode::ThreeD, 0.0, 200.0), 0.6);
        assert_eq!(metered.nits(Mode::ThreeD, 2.0, 200.0), 2.4);
        assert_eq!(metered.nits(Mode::TwoD, 5.0, 200.0), 100.0);
        assert_eq!(
            SceneLuminance::default().nits(Mode::ThreeD, 0.0, 200.0),
            0.0
        );
    }
}
