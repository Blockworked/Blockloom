//! The volume heat map, per pixel. A pass after tonemapping puts each
//! pixel's surface back in the world (from the depth prepass in 3D, on the
//! screen plane in 2D), measures it against every placed volume and tints it
//! by how much volume it is under. Global volumes are left out, since they
//! would tint everything the same.

use crate::engine::{ActorId, Dimension, Engine};
use crate::volumes::{VolumeDebugView, placed};
use crate::world::WorldCamera;
use bevy::core_pipeline::prepass::{DepthPrepass, ViewPrepassTextures};
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::{Core2d, Core2dSystems, Core3d, Core3dSystems, FullscreenShader};
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_resource::binding_types::{
    texture_2d, texture_depth_2d, texture_depth_2d_multisampled, uniform_buffer,
};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::view::{ExtractedView, Msaa, ViewTarget};
use bevy::render::{GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems};
use blockloom_core::scene::Mode;
use blockloom_core::volume::VolumeShape;

/// As many volumes as the shader's array holds; the rest go unmeasured.
pub const MAX_VOLUMES: usize = 32;

pub fn register(app: &mut App) {
    app.init_resource::<HeatVolumes>();
    if app.get_sub_app(RenderApp).is_none() {
        return;
    }
    bevy::asset::embedded_asset!(app, "shaders/volume_heat.wesl");
    app.add_plugins((
        ExtractComponentPlugin::<VolumeHeat>::default(),
        ExtractResourcePlugin::<HeatVolumes>::default(),
    ));
    let Some(render) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render
        .init_gpu_resource::<SpecializedRenderPipelines<HeatPipeline>>()
        .init_resource::<HeatUniformBuffer>()
        .add_systems(RenderStartup, init_pipeline)
        .add_systems(Render, prepare_heat.in_set(RenderSystems::PrepareResources))
        .add_systems(
            Core3d,
            draw_heat
                .after(tonemapping)
                .in_set(Core3dSystems::PostProcess),
        )
        .add_systems(
            Core2d,
            draw_heat
                .after(tonemapping)
                .in_set(Core2dSystems::PostProcess),
        );
}

/// Asks for the heat map on a camera.
#[derive(Component, Clone, Copy, Default, Debug, ExtractComponent)]
#[extract_app(RenderApp)]
#[extract_component_filter(With<Camera>)]
pub struct VolumeHeat;

/// One volume as the shader measures it; matches `HeatVolume`.
#[derive(Clone, Copy, Default, Debug, PartialEq, ShaderType)]
pub struct HeatVolume {
    /// xyz the centre, w the shape: 0 box, 1 sphere.
    centre: Vec4,
    /// World to the volume's own frame.
    rotation: Vec4,
    /// Half size in world units (x the radius for a sphere), w the blend.
    extent: Vec4,
    weight: Vec4,
}

/// This frame's placed volumes, for the render world.
#[derive(Resource, Clone, Default, Debug, PartialEq)]
pub struct HeatVolumes {
    pub volumes: Vec<HeatVolume>,
    pub flat: bool,
}

impl ExtractResource<RenderApp> for HeatVolumes {
    type Source = HeatVolumes;

    fn extract_resource(source: &Self::Source) -> Self {
        source.clone()
    }
}

/// Puts the pass on the world camera while the heat map is on, and gathers
/// the volumes it measures.
pub fn collect_heat(
    mut commands: Commands,
    engine: NonSend<Engine>,
    dimension: Res<Dimension>,
    debug: Res<VolumeDebugView>,
    cameras: Query<(Entity, Has<VolumeHeat>), With<WorldCamera>>,
    actors: Query<(&ActorId, &Transform), Without<WorldCamera>>,
    mut heat: ResMut<HeatVolumes>,
) {
    let on = debug.0.heatmap;
    for (camera, has) in &cameras {
        match (on, has) {
            (true, false) => {
                commands.entity(camera).insert(VolumeHeat);
            }
            (false, true) => {
                commands.entity(camera).remove::<VolumeHeat>();
            }
            _ => {}
        }
    }
    let next = if on {
        heat_volumes(&engine, dimension.0, actors.iter())
    } else {
        HeatVolumes::default()
    };
    heat.set_if_neq(next);
}

fn heat_volumes<'a>(
    engine: &'a Engine,
    mode: Mode,
    actors: impl Iterator<Item = (&'a ActorId, &'a Transform)>,
) -> HeatVolumes {
    let volumes = placed(engine, actors)
        .into_iter()
        .filter(|(_, spec, _)| spec.shape != VolumeShape::Global)
        .take(MAX_VOLUMES)
        .map(|(_, spec, pose)| {
            let scale = Vec3::from(pose.scale).abs();
            let rotation = Quat::from_array(pose.rotation).normalize();
            let rotation = if rotation.is_finite() {
                rotation
            } else {
                Quat::IDENTITY
            };
            let (shape, half) = match spec.shape {
                VolumeShape::Sphere => {
                    (1.0, Vec3::splat(spec.radius.max(0.0) * scale.max_element()))
                }
                _ => (0.0, Vec3::from(spec.half_extents).max(Vec3::ZERO) * scale),
            };
            HeatVolume {
                centre: Vec3::from(pose.position).extend(shape),
                rotation: Vec4::from(rotation.inverse()),
                extent: half.extend(spec.blend_distance.max(0.0)),
                weight: Vec4::new(spec.weight.clamp(0.0, 1.0), 0.0, 0.0, 0.0),
            }
        })
        .collect();
    HeatVolumes {
        volumes,
        flat: mode == Mode::TwoD,
    }
}

/// `HeatUniforms`, field for field.
#[derive(Clone, Copy, Debug, ShaderType)]
struct HeatUniforms {
    world_from_clip: Mat4,
    size: Vec2,
    count: u32,
    flat: u32,
    volumes: [HeatVolume; MAX_VOLUMES],
}

#[derive(Resource, Default)]
struct HeatUniformBuffer(DynamicUniformBuffer<HeatUniforms>);

#[derive(Component)]
struct ViewHeat {
    offset: u32,
    guide: Guide,
    pipeline: CachedRenderPipelineId,
}

/// How a pixel finds its depth: none in 2D, the prepass in 3D.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Guide {
    Flat,
    Depth,
    DepthMultisampled,
}

impl Guide {
    const ALL: [Guide; 3] = [Self::Flat, Self::Depth, Self::DepthMultisampled];

    fn features(self) -> [(&'static str, bool); 2] {
        [
            ("DEPTH", self != Self::Flat),
            ("MULTISAMPLED", self == Self::DepthMultisampled),
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct HeatKey {
    guide: Guide,
    format: TextureFormat,
}

#[derive(Resource)]
struct HeatPipeline {
    layouts: [BindGroupLayoutDescriptor; 3],
    shader: Handle<bevy::shader::Shader>,
    fullscreen: FullscreenShader,
}

fn init_pipeline(
    mut commands: Commands,
    fullscreen: Res<FullscreenShader>,
    assets: Res<AssetServer>,
) {
    let layouts = Guide::ALL.map(|guide| {
        let uniforms = uniform_buffer::<HeatUniforms>(true);
        let screen = texture_2d(TextureSampleType::Float { filterable: false });
        let entries = match guide {
            Guide::Flat => {
                BindGroupLayoutEntries::sequential(ShaderStages::FRAGMENT, (uniforms, screen))
                    .to_vec()
            }
            Guide::Depth => BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (uniforms, screen, texture_depth_2d()),
            )
            .to_vec(),
            Guide::DepthMultisampled => BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (uniforms, screen, texture_depth_2d_multisampled()),
            )
            .to_vec(),
        };
        BindGroupLayoutDescriptor::new("volume_heat_layout", &entries)
    });
    commands.insert_resource(HeatPipeline {
        layouts,
        shader: bevy::asset::load_embedded_asset!(assets.as_ref(), "shaders/volume_heat.wesl"),
        fullscreen: fullscreen.clone(),
    });
}

impl SpecializedRenderPipeline for HeatPipeline {
    type Key = HeatKey;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        let shader_defs = key
            .guide
            .features()
            .into_iter()
            .filter(|(_, on)| *on)
            .map(|(name, _)| name.into())
            .collect();
        RenderPipelineDescriptor {
            label: Some("volume_heat".into()),
            layout: vec![self.layouts[key.guide as usize].clone()],
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

/// Each heat view's pipeline and uniforms. A view that stopped asking
/// loses its `ViewHeat`, since render world views outlive their components.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn prepare_heat(
    mut commands: Commands,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<HeatPipeline>>,
    heat_pipeline: Res<HeatPipeline>,
    mut buffer: ResMut<HeatUniformBuffer>,
    heat: Option<Res<HeatVolumes>>,
    views: Query<
        (
            Entity,
            &ExtractedView,
            &ExtractedCamera,
            Option<&Msaa>,
            Has<DepthPrepass>,
        ),
        With<VolumeHeat>,
    >,
    stale: Query<Entity, (With<ViewHeat>, Without<VolumeHeat>)>,
) {
    for entity in &stale {
        commands.entity(entity).remove::<ViewHeat>();
    }
    buffer.0.clear();
    let heat = heat.map(|heat| heat.clone()).unwrap_or_default();
    let mut volumes = [HeatVolume::default(); MAX_VOLUMES];
    for (slot, volume) in volumes.iter_mut().zip(&heat.volumes) {
        *slot = *volume;
    }
    for (entity, view, camera, msaa, depth) in &views {
        let guide = match (depth, msaa.is_some_and(|msaa| msaa.samples() > 1)) {
            (false, _) => Guide::Flat,
            (true, false) => Guide::Depth,
            (true, true) => Guide::DepthMultisampled,
        };
        let pipeline = pipelines.specialize(
            &cache,
            &heat_pipeline,
            HeatKey {
                guide,
                format: view.target_format,
            },
        );
        let size = camera
            .physical_viewport_size
            .unwrap_or(UVec2::ONE)
            .max(UVec2::ONE);
        let world_from_clip = view
            .clip_from_world
            .map(|clip_from_world| clip_from_world.inverse())
            .unwrap_or_else(|| view.world_from_view.to_matrix() * view.clip_from_view.inverse());
        let offset = buffer.0.push(&HeatUniforms {
            world_from_clip,
            size: size.as_vec2(),
            count: heat.volumes.len().min(MAX_VOLUMES) as u32,
            flat: heat.flat as u32,
            volumes,
        });
        commands.entity(entity).insert(ViewHeat {
            offset,
            guide,
            pipeline,
        });
    }
    buffer.0.write_buffer(&device, &queue);
}

fn draw_heat(
    view: ViewQuery<(&ViewTarget, &ViewHeat, Option<&ViewPrepassTextures>), With<VolumeHeat>>,
    heat: Option<Res<HeatPipeline>>,
    buffer: Res<HeatUniformBuffer>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (target, view_heat, prepass) = view.into_inner();
    let Some(heat) = heat else {
        return;
    };
    let (Some(pipeline), Some(uniforms)) = (
        cache.get_render_pipeline(view_heat.pipeline),
        buffer.0.binding(),
    ) else {
        return;
    };
    let layout = cache.get_bind_group_layout(&heat.layouts[view_heat.guide as usize]);
    let post = target.post_process_write();
    let bind_group = match view_heat.guide {
        Guide::Flat => ctx.render_device().create_bind_group(
            "volume_heat",
            &layout,
            &BindGroupEntries::sequential((uniforms, post.source)),
        ),
        Guide::Depth | Guide::DepthMultisampled => {
            let Some(depth) = prepass.and_then(ViewPrepassTextures::depth_only_view) else {
                return;
            };
            ctx.render_device().create_bind_group(
                "volume_heat",
                &layout,
                &BindGroupEntries::sequential((uniforms, post.source, depth)),
            )
        }
    };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let mut pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("volume_heat"),
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
    let span = diagnostics.pass_span(&mut pass, "volume_heat");
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &bind_group, &[view_heat.offset]);
    pass.draw(0..3, 0..1);
    span.end(&mut pass);
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::shader_lib;

    #[test]
    fn the_heat_shader_compiles_for_every_guide() {
        let source = include_str!("shaders/volume_heat.wesl");
        for guide in Guide::ALL {
            shader_lib::validate(source, &guide.features())
                .unwrap_or_else(|error| panic!("{guide:?}: {error}"));
        }
    }

    #[test]
    fn heat_uniforms_match_the_wesl_layout() {
        // The matrix, a size, two words, then 32 volumes of four vec4s.
        assert_eq!(
            HeatUniforms::min_size().get(),
            64 + 16 + MAX_VOLUMES as u64 * 64
        );
    }
}
