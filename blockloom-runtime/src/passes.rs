//! What the screen-space passes share, in both dimensions: Blockloom's WESL
//! library (`blockloom::hash`, `noise`, `fbm`, `scattering`, `frame`), the
//! standard per-view `FrameUniforms`, one FP16 working target set per view
//! (a full-res target plus a half-res scratch pair) and a bilateral upsample
//! from scratch to full res. Volumetrics, fog and SSR build on these rather
//! than each rolling their own.
//!
//! A camera asks for the set with [`WorkingTargets`], the way it asks for a
//! depth prepass; nothing is allocated for one that doesn't.

use crate::environment::Environment;
use bevy::asset::uuid::Uuid;
use bevy::core_pipeline::FullscreenShader;
use bevy::core_pipeline::prepass::{DepthPrepass, ViewPrepassTextures};
use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_resource::binding_types::{
    texture_2d, texture_depth_2d, texture_depth_2d_multisampled, uniform_buffer,
};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue};
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::{ExtractedView, Msaa};
use bevy::render::{GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::Shader;
use blockloom_core::shader_lib;

/// The working targets' format: HDR, blendable and storage-writable.
pub const WORKING_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

pub fn register(app: &mut App) {
    register_library(app);
    bevy::asset::embedded_asset!(app, "shaders/bilateral_upsample.wesl");
    app.add_plugins(ExtractComponentPlugin::<WorkingTargets>::default());
    let Some(render) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render
        .init_gpu_resource::<SpecializedRenderPipelines<BilateralUpsample>>()
        .init_resource::<FrameUniformBuffer>()
        .add_systems(RenderStartup, init_upsample)
        .add_systems(
            Render,
            (
                prepare_upsample_pipelines.in_set(RenderSystems::Prepare),
                (prepare_working_targets, prepare_frame_uniforms)
                    .in_set(RenderSystems::PrepareResources),
            ),
        );
}

/// Each library module as a shader asset whose import path is
/// `blockloom::<name>`. A fixed id per module, so they are never unloaded.
fn register_library(app: &mut App) {
    let Some(mut shaders) = app.world_mut().get_resource_mut::<Assets<Shader>>() else {
        return;
    };
    for (index, (name, source)) in shader_lib::MODULES.iter().enumerate() {
        let id = Uuid::from_u64_pair(0x626c_6f63_6b6c_6f6f, index as u64);
        let path = format!("embedded://{}/{name}.wesl", shader_lib::PACKAGE);
        let _ = shaders.insert(id, Shader::from_wesl(*source, path));
    }
}

/// Asks for a view's working targets and frame uniforms.
#[derive(Component, Clone, Copy, Default, Debug, ExtractComponent)]
#[extract_app(RenderApp)]
#[extract_component_filter(With<Camera>)]
pub struct WorkingTargets;

/// A view's working set, render world. The scratch pair is half the size,
/// rounded up, for ping-ponging a half-res effect before it is upsampled.
#[derive(Component)]
pub struct WorkingTargetSet {
    pub full: CachedTexture,
    pub scratch: [CachedTexture; 2],
    pub size: UVec2,
    pub scratch_size: UVec2,
}

pub fn scratch_size(size: UVec2) -> UVec2 {
    ((size + 1) / 2).max(UVec2::ONE)
}

fn prepare_working_targets(
    mut commands: Commands,
    device: Res<RenderDevice>,
    mut cache: ResMut<TextureCache>,
    views: Query<(Entity, &ExtractedCamera), With<WorkingTargets>>,
) {
    for (entity, camera) in &views {
        let Some(size) = camera
            .physical_viewport_size
            .filter(|size| size.min_element() > 0)
        else {
            continue;
        };
        let half = scratch_size(size);
        // Labels start `working_` so GPU memory counts them as post.
        let mut target = |label: &'static str, size: UVec2| {
            cache.get(
                &device,
                TextureDescriptor {
                    label: Some(label),
                    size: Extent3d {
                        width: size.x,
                        height: size.y,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: WORKING_FORMAT,
                    usage: TextureUsages::RENDER_ATTACHMENT
                        | TextureUsages::TEXTURE_BINDING
                        | TextureUsages::STORAGE_BINDING,
                    view_formats: &[],
                },
            )
        };
        let set = WorkingTargetSet {
            full: target("working_target", size),
            scratch: [
                target("working_scratch_a", half),
                target("working_scratch_b", half),
            ],
            size,
            scratch_size: half,
        };
        commands.entity(entity).insert(set);
    }
}

/// `blockloom::frame::FrameUniforms`, field for field.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct FrameUniforms {
    pub time: f32,
    pub delta: f32,
    pub frame: u32,
    pub exposure: f32,
    pub sun_direction: Vec3,
    pub sun_illuminance: f32,
    pub sun_color: Vec4,
    pub ambient: Vec4,
    pub background: Vec4,
    pub target_size: Vec2,
    pub target_texel: Vec2,
    pub scratch_size: Vec2,
    pub scratch_texel: Vec2,
}

impl FrameUniforms {
    pub fn new(environment: &Environment, time: &Time, frame: u32, size: UVec2) -> Self {
        let size = size.max(UVec2::ONE);
        let half = scratch_size(size);
        let linear = |color: Color| color.to_linear().to_vec4();
        let mut ambient = linear(environment.ambient_color);
        ambient.w = environment.ambient_brightness;
        Self {
            // Wrapped the way Bevy's globals are, so f32 keeps its precision.
            time: time.elapsed_secs_wrapped(),
            delta: time.delta_secs(),
            frame,
            exposure: environment.exposure,
            sun_direction: environment.sun.direction,
            sun_illuminance: environment.sun.illuminance,
            sun_color: linear(environment.sun.color),
            ambient,
            background: linear(environment.background),
            target_size: size.as_vec2(),
            target_texel: size.as_vec2().recip(),
            scratch_size: half.as_vec2(),
            scratch_texel: half.as_vec2().recip(),
        }
    }
}

#[derive(Resource, Default)]
pub struct FrameUniformBuffer(pub DynamicUniformBuffer<FrameUniforms>);

/// Where a view's `FrameUniforms` sit in [`FrameUniformBuffer`].
#[derive(Component)]
pub struct ViewFrameUniforms {
    pub offset: u32,
}

fn prepare_frame_uniforms(
    mut commands: Commands,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut buffer: ResMut<FrameUniformBuffer>,
    environment: Option<Res<Environment>>,
    time: Res<Time>,
    frame: Res<FrameCount>,
    views: Query<(Entity, &ExtractedCamera), With<WorkingTargets>>,
) {
    buffer.0.clear();
    let environment = environment.map(|env| env.clone()).unwrap_or_default();
    for (entity, camera) in &views {
        let size = camera.physical_viewport_size.unwrap_or(UVec2::ONE);
        let offset = buffer
            .0
            .push(&FrameUniforms::new(&environment, &time, frame.0, size));
        commands.entity(entity).insert(ViewFrameUniforms { offset });
    }
    buffer.0.write_buffer(&device, &queue);
}

/// How the upsample tells an edge: a depth prepass in 3D, nothing in 2D
/// (where it is plain bilinear).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UpsampleGuide {
    None,
    Depth,
    DepthMultisampled,
}

impl UpsampleGuide {
    const ALL: [UpsampleGuide; 3] = [Self::None, Self::Depth, Self::DepthMultisampled];

    fn features(self) -> [(&'static str, bool); 2] {
        [
            ("DEPTH_GUIDE", self != Self::None),
            ("MULTISAMPLED", self == Self::DepthMultisampled),
        ]
    }
}

/// Replace writes the upsampled color as is; `Over` composites it,
/// premultiplied, over what the destination holds (alpha is coverage).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UpsampleBlend {
    Replace,
    Over,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UpsampleKey {
    pub guide: UpsampleGuide,
    pub blend: UpsampleBlend,
    pub format: TextureFormat,
}

#[derive(Resource)]
pub struct BilateralUpsample {
    layouts: [BindGroupLayoutDescriptor; 3],
    shader: Handle<Shader>,
    fullscreen: FullscreenShader,
}

impl BilateralUpsample {
    fn layout(&self, guide: UpsampleGuide) -> &BindGroupLayoutDescriptor {
        &self.layouts[guide as usize]
    }
}

fn init_upsample(
    mut commands: Commands,
    fullscreen: Res<FullscreenShader>,
    assets: Res<AssetServer>,
) {
    let layouts = UpsampleGuide::ALL.map(|guide| {
        let uniforms = uniform_buffer::<FrameUniforms>(true);
        let source = texture_2d(TextureSampleType::Float { filterable: false });
        let entries = match guide {
            UpsampleGuide::None => {
                BindGroupLayoutEntries::sequential(ShaderStages::FRAGMENT, (uniforms, source))
                    .to_vec()
            }
            UpsampleGuide::Depth => BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (uniforms, source, texture_depth_2d()),
            )
            .to_vec(),
            UpsampleGuide::DepthMultisampled => BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (uniforms, source, texture_depth_2d_multisampled()),
            )
            .to_vec(),
        };
        BindGroupLayoutDescriptor::new("bilateral_upsample_layout", &entries)
    });
    commands.insert_resource(BilateralUpsample {
        layouts,
        shader: bevy::asset::load_embedded_asset!(
            assets.as_ref(),
            "shaders/bilateral_upsample.wesl"
        ),
        fullscreen: fullscreen.clone(),
    });
}

impl SpecializedRenderPipeline for BilateralUpsample {
    type Key = UpsampleKey;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        let shader_defs = key
            .guide
            .features()
            .into_iter()
            .filter(|(_, on)| *on)
            .map(|(name, _)| name.into())
            .collect();
        RenderPipelineDescriptor {
            label: Some("bilateral_upsample".into()),
            layout: vec![self.layout(key.guide).clone()],
            vertex: self.fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs,
                targets: vec![Some(ColorTargetState {
                    format: key.format,
                    blend: match key.blend {
                        UpsampleBlend::Replace => None,
                        UpsampleBlend::Over => Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    },
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            ..default()
        }
    }
}

/// A view's upsample pipelines: scratch onto its own working target, and
/// scratch composited over the view's main texture. Queued as soon as the
/// view asks for working targets, so they compile during warm-up.
#[derive(Component)]
pub struct ViewUpsample {
    pub guide: UpsampleGuide,
    pub to_working: CachedRenderPipelineId,
    pub over_view: CachedRenderPipelineId,
}

fn prepare_upsample_pipelines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<BilateralUpsample>>,
    upsample: Res<BilateralUpsample>,
    views: Query<(Entity, &ExtractedView, Option<&Msaa>, Has<DepthPrepass>), With<WorkingTargets>>,
) {
    for (entity, view, msaa, depth) in &views {
        let guide = match (depth, msaa.is_some_and(|msaa| msaa.samples() > 1)) {
            (false, _) => UpsampleGuide::None,
            (true, false) => UpsampleGuide::Depth,
            (true, true) => UpsampleGuide::DepthMultisampled,
        };
        let mut specialize = |blend, format| {
            pipelines.specialize(
                &cache,
                &upsample,
                UpsampleKey {
                    guide,
                    blend,
                    format,
                },
            )
        };
        let to_working = specialize(UpsampleBlend::Replace, WORKING_FORMAT);
        let over_view = specialize(UpsampleBlend::Over, view.target_format);
        commands.entity(entity).insert(ViewUpsample {
            guide,
            to_working,
            over_view,
        });
    }
}

/// Upsamples `source` (one of the view's scratch pair) onto `destination`
/// with `pipeline`, one of the view's [`ViewUpsample`] ids. False while the
/// pipeline is still compiling or the view is missing what it needs, in
/// which case nothing was drawn.
#[allow(clippy::too_many_arguments)]
pub fn upsample(
    ctx: &mut RenderContext,
    cache: &PipelineCache,
    upsample: &BilateralUpsample,
    frame: &FrameUniformBuffer,
    view: (
        &ViewUpsample,
        &ViewFrameUniforms,
        Option<&ViewPrepassTextures>,
    ),
    pipeline: CachedRenderPipelineId,
    source: &TextureView,
    destination: &TextureView,
) -> bool {
    let (view_upsample, uniforms, prepass) = view;
    let Some(render_pipeline) = cache.get_render_pipeline(pipeline) else {
        return false;
    };
    let Some(frame_binding) = frame.0.binding() else {
        return false;
    };
    let layout = cache.get_bind_group_layout(upsample.layout(view_upsample.guide));
    let bind_group = match view_upsample.guide {
        UpsampleGuide::None => ctx.render_device().create_bind_group(
            "bilateral_upsample",
            &layout,
            &BindGroupEntries::sequential((frame_binding, source)),
        ),
        UpsampleGuide::Depth | UpsampleGuide::DepthMultisampled => {
            let Some(depth) = prepass.and_then(ViewPrepassTextures::depth_only_view) else {
                return false;
            };
            ctx.render_device().create_bind_group(
                "bilateral_upsample",
                &layout,
                &BindGroupEntries::sequential((frame_binding, source, depth)),
            )
        }
    };
    let mut pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("bilateral_upsample"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: destination,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    pass.set_pipeline(render_pipeline);
    pass.set_bind_group(0, &bind_group, &[uniforms.offset]);
    pass.draw(0..3, 0..1);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_upsample_shader_compiles_for_every_guide() {
        let source = include_str!("shaders/bilateral_upsample.wesl");
        for guide in UpsampleGuide::ALL {
            shader_lib::validate(source, &guide.features())
                .unwrap_or_else(|error| panic!("{guide:?}: {error}"));
        }
    }

    #[test]
    fn scratch_is_half_size_rounded_up() {
        assert_eq!(scratch_size(UVec2::new(1920, 1080)), UVec2::new(960, 540));
        assert_eq!(scratch_size(UVec2::new(1921, 1)), UVec2::new(961, 1));
        assert_eq!(scratch_size(UVec2::ZERO), UVec2::ONE);
    }

    #[test]
    fn frame_uniforms_carry_the_environment() {
        let mut environment = Environment::default();
        environment.exposure = 3.5;
        environment.ambient_brightness = 80.0;
        let uniforms = FrameUniforms::new(&environment, &Time::default(), 7, UVec2::new(640, 360));
        assert_eq!(uniforms.frame, 7);
        assert_eq!(uniforms.exposure, 3.5);
        assert_eq!(uniforms.ambient.w, 80.0);
        assert_eq!(uniforms.sun_direction, environment.sun.direction);
        assert_eq!(uniforms.target_texel, Vec2::new(1.0 / 640.0, 1.0 / 360.0));
        assert_eq!(uniforms.scratch_size, Vec2::new(320.0, 180.0));
    }

    #[test]
    fn frame_uniforms_match_the_wesl_layout() {
        // 16 bytes of scalars, the sun's direction and illuminance, three
        // colors and two size pairs.
        assert_eq!(FrameUniforms::min_size().get(), 16 + 16 + 3 * 16 + 2 * 16);
        let wgsl = shader_lib::link(
            "import blockloom::frame::FrameUniforms;\n\
             @group(0) @binding(0) var<uniform> frame: FrameUniforms;\n\
             @fragment fn main() -> @location(0) vec4<f32> { return frame.background; }\n",
            &[],
        )
        .unwrap();
        let module = naga::front::wgsl::parse_str(&wgsl).unwrap();
        let (_, ty) = module
            .types
            .iter()
            .find(|(_, ty)| {
                ty.name
                    .as_deref()
                    .is_some_and(|name| name.ends_with("FrameUniforms"))
            })
            .unwrap();
        let naga::TypeInner::Struct { span, .. } = ty.inner else {
            panic!("FrameUniforms isn't a struct");
        };
        assert_eq!(span as u64, FrameUniforms::min_size().get());
    }
}
