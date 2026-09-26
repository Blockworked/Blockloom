//! Planar cloud layers (`blockloom_core::cloud_layers`), 3D only. One
//! full-resolution pass per side of the volumetric clouds: layers beyond
//! them draw before the march is composited, layers between the camera and
//! them after it, and with volumetrics off every layer draws in the first.
use crate::{
    engine::Engine, environment::Environment, sky::SkyRender, wind::WindField, world::WorldCamera,
};
use bevy::core_pipeline::{
    Core3d, Core3dSystems, FullscreenShader,
    prepass::{DepthPrepass, ViewPrepassTextures},
};
use bevy::prelude::*;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::*;
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::{ExtractedView, Msaa, ViewTarget};
use bevy::render::{GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems};
use blockloom_core::cloud_layers::{COVERAGE_SIZE, CloudLayer, FLOW_SIZE, MAX_LAYERS};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
struct LayerGpu {
    /// Altitude, opacity, coverage, contrast.
    shape: Vec4,
    /// 1 / tiling in metres, parallax metres, aerial, flow push in tiles.
    map: Vec4,
    /// Scroll offset xz in metres, spin radians, flow phase.
    motion: Vec4,
    /// Pivot xz, sines of the horizon fade's start and end.
    pivot: Vec4,
    /// Tint times the time-of-day ramp; w 1 with a flow map.
    tint: Vec4,
    /// w the texture slice.
    sun_tint: Vec4,
    edge_tint: Vec4,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
struct LayerUniforms {
    world_from_clip: Mat4,
    /// w haze extinction per metre.
    camera: Vec4,
    sun: Vec4,
    sun_color: Vec4,
    moon: Vec4,
    moon_color: Vec4,
    /// Size, exposure scale, unused.
    screen: Vec4,
    /// First and end layer this draw composites.
    range: UVec4,
    layers: [LayerGpu; MAX_LAYERS],
}

/// Every layer's coverage and flow, ready to upload.
struct LayerPixels {
    coverage: Vec<u8>,
    flow: Vec<u8>,
}

#[derive(Resource, Clone, Default)]
struct LayerRender {
    /// Unsorted, in texture-slice order; empty draws nothing.
    layers: Vec<LayerGpu>,
    base: LayerUniforms,
    /// The volumetric clouds' bottom and top, when they draw.
    slab: Option<(f32, f32)>,
    pixels: Option<Arc<LayerPixels>>,
    /// Bumped whenever `pixels` changes, so the textures re-upload.
    generation: u32,
}

impl ExtractResource<RenderApp> for LayerRender {
    type Source = Self;
    fn extract_resource(source: &Self) -> Self {
        source.clone()
    }
}

type TextureKey = (
    Option<std::path::PathBuf>,
    Vec<((String, u32, u32, u32, u32), String)>,
);

/// Which textures are loaded, so files are read and noise baked once per change.
#[derive(Resource, Default)]
struct LoadedLayers {
    key: Option<TextureKey>,
}

#[derive(Component, Clone, Copy, ExtractComponent)]
#[extract_app(RenderApp)]
struct LayerView;

#[derive(Resource, Default)]
struct LayerBuffer(DynamicUniformBuffer<LayerUniforms>);

pub fn register(app: &mut App) {
    app.init_resource::<LayerRender>()
        .init_resource::<LoadedLayers>()
        .add_systems(
            Update,
            (load_layers, resolve)
                .chain()
                .after(crate::clouds::CloudResolve)
                .after(crate::environment::apply_environment),
        );
    if app.get_sub_app(RenderApp).is_none() {
        return;
    }
    bevy::asset::embedded_asset!(app, "shaders/cloud_layers.wesl");
    app.add_plugins((
        ExtractResourcePlugin::<LayerRender>::default(),
        ExtractComponentPlugin::<LayerView>::default(),
    ));
    app.get_sub_app_mut(RenderApp)
        .unwrap()
        .init_resource::<LayerBuffer>()
        .init_gpu_resource::<SpecializedRenderPipelines<LayerPipeline>>()
        .add_systems(RenderStartup, init)
        .add_systems(Render, prepare.in_set(RenderSystems::PrepareResources))
        .add_systems(
            Core3d,
            (
                draw_behind
                    .after(Core3dSystems::MainPass)
                    .before(crate::clouds::CloudMarch),
                draw_front.after(crate::clouds::CloudMarch),
            )
                .in_set(crate::clouds::CloudPass),
        );
}

/// Reads or bakes every layer's coverage and flow when what they name changes.
fn load_layers(
    engine: NonSend<Engine>,
    mut loaded: ResMut<LoadedLayers>,
    mut render: ResMut<LayerRender>,
) {
    let layers = &engine.project.world.cloud_layers;
    let key: TextureKey = (
        engine.project_dir.clone(),
        layers
            .iter()
            .take(MAX_LAYERS)
            .map(|l| (l.texture_key(), l.flow_map.clone()))
            .collect(),
    );
    if loaded.key.as_ref() == Some(&key) {
        return;
    }
    let dir = key.0.as_deref();
    let report = |message: String| {
        crate::bridge::send(&blockloom_protocol::RuntimeMessage::Error {
            actor: "Blockloom".into(),
            message,
        })
    };
    let cover = (COVERAGE_SIZE * COVERAGE_SIZE) as usize;
    let flow = (FLOW_SIZE * FLOW_SIZE * 4) as usize;
    let mut pixels = LayerPixels {
        coverage: Vec::with_capacity(cover * MAX_LAYERS),
        flow: Vec::with_capacity(flow * MAX_LAYERS),
    };
    for layer in layers.iter().take(MAX_LAYERS) {
        let coverage =
            blockloom_core::cloud_layers::load_coverage(dir, layer).unwrap_or_else(|e| {
                report(format!(
                    "A cloud layer's coverage didn't load, baking it instead: {e}"
                ));
                let baked = CloudLayer {
                    coverage_texture: String::new(),
                    ..layer.clone()
                };
                blockloom_core::cloud_layers::bake_coverage(&baked, COVERAGE_SIZE)
            });
        pixels.coverage.extend(coverage);
        let map = blockloom_core::cloud_layers::load_flow(dir, layer).unwrap_or_else(|e| {
            report(format!("A cloud layer's flow map didn't load: {e}"));
            None
        });
        pixels.flow.extend(map.unwrap_or_else(|| still_flow(flow)));
    }
    // Unused slices keep the array a fixed size.
    pixels.coverage.resize(cover * MAX_LAYERS, 0);
    while pixels.flow.len() < flow * MAX_LAYERS {
        pixels.flow.extend(still_flow(flow));
    }
    render.pixels = Some(Arc::new(pixels));
    render.generation = render.generation.wrapping_add(1);
    loaded.key = Some(key);
}

fn still_flow(len: usize) -> Vec<u8> {
    [128u8, 128, 0, 255].into_iter().cycle().take(len).collect()
}

fn linear(hex: &str) -> Vec3 {
    crate::world::parse_color(hex).to_linear().to_vec3()
}

/// A layer as the shader reads it, at cloud time `offsets`.
fn layer_gpu(
    layer: &CloudLayer,
    slice: usize,
    offsets: &blockloom_core::wind::CloudOffsets,
    ramp: [f32; 3],
) -> LayerGpu {
    let tiling = layer.tiling_km * 1000.0;
    let time = offsets.time;
    let offset = Vec2::from(offsets.layers) * layer.wind + Vec2::from(layer.scroll) * time;
    let ramp = linear(&layer.day_tint) * ramp[0]
        + linear(&layer.sunset_tint) * ramp[1]
        + linear(&layer.night_tint) * ramp[2];
    let has_flow = !layer.flow_map.trim().is_empty() && layer.flow_strength > 0.0;
    LayerGpu {
        shape: Vec4::new(
            layer.altitude,
            layer.opacity,
            layer.coverage,
            layer.contrast,
        ),
        map: Vec4::new(
            1.0 / tiling,
            layer.parallax,
            layer.aerial,
            layer.flow_strength / tiling,
        ),
        motion: Vec4::new(
            offset.x,
            offset.y,
            (layer.spin * time).to_radians(),
            time / layer.flow_period,
        ),
        pivot: Vec4::new(
            layer.pivot[0],
            layer.pivot[1],
            layer.horizon_fade[0].to_radians().sin(),
            layer.horizon_fade[1].to_radians().sin(),
        ),
        tint: (linear(&layer.tint) * ramp).extend(if has_flow { 1.0 } else { 0.0 }),
        sun_tint: linear(&layer.sun_tint).extend(slice as f32),
        edge_tint: linear(&layer.edge_tint).extend(0.0),
    }
}

fn resolve(
    mut commands: Commands,
    env: Res<Environment>,
    wind: Res<WindField>,
    engine: NonSend<Engine>,
    moon: Option<Res<crate::space::MoonState>>,
    mut render: ResMut<LayerRender>,
    mut sources: ResMut<crate::atmosphere::AtmosphereSources>,
    views: Query<(Entity, Has<LayerView>), With<WorldCamera>>,
) {
    let mut layers = engine.project.world.cloud_layers.clone();
    blockloom_core::cloud_layers::normalize(&mut layers);
    let ramp = blockloom_core::fog::color_weights(env.sun.direction.y);
    render.layers = layers
        .iter()
        .enumerate()
        .filter(|(_, l)| l.enabled && l.opacity > 0.0 && l.coverage > 0.0)
        .map(|(slice, l)| layer_gpu(l, slice, &wind.clouds, ramp))
        .collect();
    // With volumetrics off the layers are the whole sky's clouds.
    let cover = layers.iter().map(CloudLayer::cover).fold(0.0, f32::max);
    sources.cloud_cover = sources.cloud_cover.max(cover);
    let mut clouds = env.clouds.clone();
    clouds.normalize();
    render.slab = (clouds.enabled && clouds.coverage > 0.0 && clouds.density > 0.0)
        .then_some((clouds.bottom, clouds.top));
    let sun = env.sun.color.to_linear().to_vec3() * env.sun.illuminance;
    render.base = LayerUniforms {
        camera: Vec4::new(0.0, 0.0, 0.0, env.haze.max(0.0)),
        sun: env.sun.direction.normalize_or(Vec3::Y).extend(0.0),
        sun_color: sun.extend(0.0),
        moon: moon.as_ref().map_or(Vec3::Y, |m| m.direction).extend(0.0),
        moon_color: moon.as_ref().map_or(Vec3::ZERO, |m| m.lux).extend(0.0),
        screen: Vec4::new(0.0, 0.0, crate::fog::exposure_scale(env.exposure), 0.0),
        ..default()
    };
    let on = !render.layers.is_empty();
    for (entity, has) in &views {
        if on && !has {
            commands.entity(entity).insert((LayerView, DepthPrepass));
        } else if !on && has {
            commands.entity(entity).remove::<LayerView>();
        }
    }
}

/// Farthest first from a camera at height `eye`, split into the layers
/// beyond the volumetric `slab` and those between the camera and it.
fn order(layers: &[LayerGpu], eye: f32, slab: Option<(f32, f32)>) -> (Vec<LayerGpu>, usize) {
    let mut sorted = layers.to_vec();
    sorted.sort_by(|a, b| {
        let (a, b) = ((a.shape.x - eye).abs(), (b.shape.x - eye).abs());
        b.total_cmp(&a)
    });
    let front = |altitude: f32| match slab {
        Some((bottom, _)) if eye < bottom => altitude > eye && altitude < bottom,
        Some((_, top)) if eye > top => altitude < eye && altitude > top,
        _ => false,
    };
    // Sorting farthest first puts every front layer after the rest.
    sorted.sort_by_key(|l| front(l.shape.x));
    let behind = sorted.iter().filter(|l| !front(l.shape.x)).count();
    (sorted, behind)
}

#[derive(Resource)]
struct LayerPipeline {
    layouts: [BindGroupLayoutDescriptor; 2],
    shader: Handle<Shader>,
    fullscreen: FullscreenShader,
    repeat: Sampler,
    linear: Sampler,
}

impl SpecializedRenderPipeline for LayerPipeline {
    type Key = (bool, TextureFormat);
    fn specialize(&self, (multi, format): Self::Key) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("cloud_layers".into()),
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
                    blend: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
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
    fullscreen: Res<FullscreenShader>,
) {
    let layout = |multi| {
        BindGroupLayoutDescriptor::new(
            "cloud_layers",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    uniform_buffer::<LayerUniforms>(true),
                    if multi {
                        texture_depth_2d_multisampled()
                    } else {
                        texture_depth_2d()
                    },
                    texture_2d_array(TextureSampleType::Float { filterable: true }),
                    texture_2d_array(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    texture_cube(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        )
    };
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
    commands.insert_resource(LayerPipeline {
        layouts: [layout(false), layout(true)],
        shader: bevy::asset::load_embedded_asset!(assets.as_ref(), "shaders/cloud_layers.wesl"),
        fullscreen: fullscreen.clone(),
        repeat: make_sampler(AddressMode::Repeat),
        linear: make_sampler(AddressMode::ClampToEdge),
    });
}

/// The layer arrays on the GPU, re-uploaded when their pixels change.
#[derive(Resource)]
struct LayerTextures {
    generation: u32,
    coverage: TextureView,
    flow: TextureView,
}

fn array_texture(
    device: &RenderDevice,
    queue: &RenderQueue,
    size: u32,
    format: TextureFormat,
    bytes: &[u8],
    label: &'static str,
) -> TextureView {
    let texel = format.block_copy_size(None).unwrap_or(4);
    let extent = Extent3d {
        width: size,
        height: size,
        depth_or_array_layers: MAX_LAYERS as u32,
    };
    let texture = device.create_texture(&TextureDescriptor {
        label: Some(label),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        bytes,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size * texel),
            rows_per_image: Some(size),
        },
        extent,
    );
    texture.create_view(&TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    })
}

#[derive(Component)]
struct ViewLayers {
    pipeline: CachedRenderPipelineId,
    multi: bool,
    /// Uniform offsets for the draw behind the volumetrics and the one in
    /// front, `None` when it has no layers.
    offsets: [Option<u32>; 2],
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn prepare(
    mut commands: Commands,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
    render: Res<LayerRender>,
    pipeline: Option<Res<LayerPipeline>>,
    mut pipelines: ResMut<SpecializedRenderPipelines<LayerPipeline>>,
    mut buffer: ResMut<LayerBuffer>,
    textures: Option<Res<LayerTextures>>,
    views: Query<(Entity, &ExtractedView, Option<&Msaa>), With<LayerView>>,
    stale: Query<Entity, (With<ViewLayers>, Without<LayerView>)>,
) {
    for e in &stale {
        commands.entity(e).remove::<ViewLayers>();
    }
    buffer.0.clear();
    let (Some(pipeline), Some(pixels)) = (pipeline, render.pixels.as_ref()) else {
        return;
    };
    if render.layers.is_empty() {
        return;
    }
    if textures.is_none_or(|t| t.generation != render.generation) {
        commands.insert_resource(LayerTextures {
            generation: render.generation,
            coverage: array_texture(
                &device,
                &queue,
                COVERAGE_SIZE,
                TextureFormat::R8Unorm,
                &pixels.coverage,
                "cloud_layer_coverage",
            ),
            flow: array_texture(
                &device,
                &queue,
                FLOW_SIZE,
                TextureFormat::Rgba8Unorm,
                &pixels.flow,
                "cloud_layer_flow",
            ),
        });
    }
    for (entity, view, msaa) in &views {
        let size = view.viewport.zw().max(UVec2::ONE);
        let clip = view
            .clip_from_world
            .unwrap_or_else(|| view.clip_from_view * view.world_from_view.to_matrix().inverse());
        let camera = view.world_from_view.translation();
        let (sorted, behind) = order(&render.layers, camera.y, render.slab);
        let mut u = render.base;
        u.world_from_clip = clip.inverse();
        u.camera = camera.extend(render.base.camera.w);
        u.screen.x = size.x as f32;
        u.screen.y = size.y as f32;
        for (slot, layer) in u.layers.iter_mut().zip(&sorted) {
            *slot = *layer;
        }
        let mut offsets = [None; 2];
        for (i, range) in [(0, behind), (behind, sorted.len())]
            .into_iter()
            .enumerate()
        {
            if range.1 > range.0 {
                u.range = UVec4::new(range.0 as u32, range.1 as u32, 0, 0);
                offsets[i] = Some(buffer.0.push(&u));
            }
        }
        let multi = msaa.is_some_and(|m| m.samples() > 1);
        commands.entity(entity).insert(ViewLayers {
            pipeline: pipelines.specialize(&cache, &pipeline, (multi, view.target_format)),
            multi,
            offsets,
        });
    }
    buffer.0.write_buffer(&device, &queue);
}

#[allow(clippy::too_many_arguments)]
fn draw_layers(
    side: usize,
    view: ViewQuery<(&ViewTarget, &ViewLayers, &ViewPrepassTextures)>,
    pipeline: Option<Res<LayerPipeline>>,
    textures: Option<Res<LayerTextures>>,
    buffer: Res<LayerBuffer>,
    cache: Res<PipelineCache>,
    sky: Res<SkyRender>,
    images: Res<RenderAssets<GpuImage>>,
    ctx: &mut RenderContext,
) {
    let (target, v, prepass) = view.into_inner();
    let (Some(pipeline), Some(textures), Some(offset)) = (pipeline, textures, v.offsets[side])
    else {
        return;
    };
    let (Some(draw), Some(uniform), Some(depth), Some(sky)) = (
        cache.get_render_pipeline(v.pipeline),
        buffer.0.binding(),
        prepass.depth_only_view(),
        images.get(&sky.diffuse),
    ) else {
        return;
    };
    let group = ctx.render_device().create_bind_group(
        "cloud_layers",
        &cache.get_bind_group_layout(&pipeline.layouts[v.multi as usize]),
        &BindGroupEntries::sequential((
            uniform,
            depth,
            &textures.coverage,
            &textures.flow,
            &pipeline.repeat,
            &sky.texture_view,
            &pipeline.linear,
        )),
    );
    let mut pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("cloud_layers"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: target.main_texture_view(),
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    pass.set_pipeline(draw);
    pass.set_bind_group(0, &group, &[offset]);
    pass.draw(0..3, 0..1);
}

#[allow(clippy::too_many_arguments)]
fn draw_behind(
    view: ViewQuery<(&ViewTarget, &ViewLayers, &ViewPrepassTextures)>,
    pipeline: Option<Res<LayerPipeline>>,
    textures: Option<Res<LayerTextures>>,
    buffer: Res<LayerBuffer>,
    cache: Res<PipelineCache>,
    sky: Res<SkyRender>,
    images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    draw_layers(
        0, view, pipeline, textures, buffer, cache, sky, images, &mut ctx,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_front(
    view: ViewQuery<(&ViewTarget, &ViewLayers, &ViewPrepassTextures)>,
    pipeline: Option<Res<LayerPipeline>>,
    textures: Option<Res<LayerTextures>>,
    buffer: Res<LayerBuffer>,
    cache: Res<PipelineCache>,
    sky: Res<SkyRender>,
    images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    draw_layers(
        1, view, pipeline, textures, buffer, cache, sky, images, &mut ctx,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_validates() {
        for multi in [false, true] {
            blockloom_core::shader_lib::validate(
                include_str!("shaders/cloud_layers.wesl"),
                &[("MULTISAMPLED", multi)],
            )
            .unwrap();
        }
    }

    #[test]
    fn uniform_layout() {
        // The WESL struct is the same fields in the same order.
        assert_eq!(LayerGpu::min_size().get(), 7 * 16);
        assert_eq!(
            LayerUniforms::min_size().get(),
            64 + 7 * 16 + MAX_LAYERS as u64 * 7 * 16
        );
    }

    fn at(altitude: f32) -> LayerGpu {
        LayerGpu {
            shape: Vec4::new(altitude, 1.0, 0.5, 1.0),
            ..default()
        }
    }

    #[test]
    fn layers_split_round_the_volumetrics() {
        let layers = [at(500.0), at(9000.0), at(1000.0), at(-200.0)];
        let heights = |v: &[LayerGpu]| v.iter().map(|l| l.shape.x).collect::<Vec<_>>();
        // No volumetrics: one pass, farthest first.
        let (sorted, behind) = order(&layers, 0.0, None);
        assert_eq!(heights(&sorted), [9000.0, 1000.0, 500.0, -200.0]);
        assert_eq!(behind, 4);
        // Under a 1500-3500 slab, the low layers sit in front of it.
        let (sorted, behind) = order(&layers, 0.0, Some((1500.0, 3500.0)));
        assert_eq!(heights(&sorted), [9000.0, -200.0, 1000.0, 500.0]);
        assert_eq!(behind, 2);
        // Above it, only the cirrus between the camera and the slab.
        let (sorted, behind) = order(&layers, 10000.0, Some((1500.0, 3500.0)));
        assert_eq!(heights(&sorted), [-200.0, 500.0, 1000.0, 9000.0]);
        assert_eq!(behind, 3);
    }

    #[test]
    fn layers_move_with_cloud_time() {
        let layer = CloudLayer {
            scroll: [2.0, 0.0],
            wind: 0.5,
            spin: 10.0,
            flow_period: 4.0,
            ..default()
        };
        let offsets = blockloom_core::wind::CloudOffsets {
            layers: [100.0, 40.0],
            time: 8.0,
            ..default()
        };
        let gpu = layer_gpu(&layer, 2, &offsets, [1.0, 0.0, 0.0]);
        assert_eq!(
            gpu.motion.truncate(),
            Vec3::new(66.0, 20.0, 80f32.to_radians())
        );
        assert_eq!(gpu.motion.w, 2.0);
        assert_eq!(gpu.sun_tint.w, 2.0);
        assert_eq!(gpu.map.x, 1.0 / 20000.0);
        // No flow map, no flow.
        assert_eq!(gpu.tint.w, 0.0);
    }
}
