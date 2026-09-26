//! Under the water, 3D only: while the world camera is below a body's
//! surface, one pass after the fog absorbs every ray over the part of it
//! that is under the surface and lays caustics on what it lands on.

use super::{WaterSample, WaterState};
use crate::environment::Environment;
use crate::fog::exposure_scale;
use crate::world::WorldCamera;
use bevy::core_pipeline::prepass::ViewPrepassTextures;
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
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

pub fn register(app: &mut App) {
    app.init_resource::<UnderwaterRender>().add_systems(
        Update,
        apply_underwater.after(crate::environment::apply_environment),
    );
    if app.get_sub_app(RenderApp).is_none() {
        return;
    }
    app.add_plugins((
        ExtractComponentPlugin::<UnderwaterView>::default(),
        ExtractResourcePlugin::<UnderwaterRender>::default(),
    ));
    let render = app.get_sub_app_mut(RenderApp).unwrap();
    render
        .init_gpu_resource::<SpecializedRenderPipelines<UnderPipeline>>()
        .init_resource::<UnderUniformBuffer>()
        .add_systems(RenderStartup, init_pipeline)
        .add_systems(
            Render,
            prepare_under.in_set(RenderSystems::PrepareResources),
        )
        .add_systems(
            Core3d,
            // After the fog, which would otherwise fog the water's own
            // color by the air's rules, and strictly before post.
            draw_under
                .after(crate::fog::FogPass)
                .after(Core3dSystems::MainPass)
                .before(Core3dSystems::EarlyPostProcess)
                .before(Core3dSystems::PostProcess),
        );
}

/// On the world camera while it is under a surface.
#[derive(Component, Clone, Copy, Default, Debug, ExtractComponent)]
#[extract_app(RenderApp)]
#[extract_component_filter(With<Camera>)]
pub struct UnderwaterView;

/// The water the camera is in, `None` above every surface.
#[derive(Resource, Clone, Default, Debug, PartialEq)]
pub struct UnderwaterRender(pub Option<UnderUniforms>);

impl ExtractResource<RenderApp> for UnderwaterRender {
    type Source = UnderwaterRender;

    fn extract_resource(source: &Self::Source) -> Self {
        source.clone()
    }
}

/// `Underwater` in `underwater.wesl`, field for field.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct UnderUniforms {
    world_from_clip: Mat4,
    camera: Vec4,
    fog: Vec4,
    tint: Vec4,
    caustics: Vec4,
    size: Vec4,
}

/// How brightly the water's own color is lit, exposed: the sky's ambient
/// plus what of the sun comes down through the surface.
pub fn water_light(env: &Environment) -> Vec3 {
    let ambient = Vec3::from_slice(&env.ambient_color.to_linear().to_f32_array()[..3])
        * env.ambient_brightness;
    let sun = Vec3::from_slice(&env.sun.color.to_linear().to_f32_array()[..3])
        * env.sun.illuminance
        * env.sun.direction.y.max(0.0);
    (ambient + sun / std::f32::consts::PI) * exposure_scale(env.exposure)
}

/// Works out whether the camera is under a surface and, if so, what the
/// pass needs to fog it.
fn apply_underwater(
    mut commands: Commands,
    sample: Res<WaterSample>,
    state: Res<WaterState>,
    environment: Res<Environment>,
    cameras: Query<(Entity, &GlobalTransform, Has<UnderwaterView>), With<WorldCamera>>,
    mut render: ResMut<UnderwaterRender>,
) {
    let mut next = None;
    for (camera, transform, has) in &cameras {
        let eye = transform.translation();
        let inside = sample
            .0
            .surface_at(eye.x, eye.z)
            .filter(|(body, water)| eye.y < water.height && eye.y > body.floor());
        let under = inside.and_then(|(body, water)| {
            let live = state.body(&body.id)?;
            let spec = &live.spec;
            let shallow = super::rgb(&spec.look.shallow);
            let fog = super::rgb(&spec.underwater.fog);
            let extinction = (Vec3::ONE - shallow * 0.75) * (2.0 - spec.look.clarity);
            Some(UnderUniforms {
                camera: eye.extend(water.height),
                fog: (fog * water_light(&environment)).extend(spec.underwater.distance),
                tint: extinction.extend(spec.underwater.caustics),
                caustics: Vec4::new(
                    spec.underwater.caustics_scale,
                    sample.0.time,
                    spec.look.absorption,
                    0.0,
                ),
                ..default()
            })
        });
        match (under.is_some(), has) {
            (true, false) => {
                commands.entity(camera).insert(UnderwaterView);
            }
            (false, true) => {
                commands.entity(camera).remove::<UnderwaterView>();
            }
            _ => {}
        }
        next = next.or(under);
    }
    render.set_if_neq(UnderwaterRender(next));
}

#[derive(Resource, Default)]
struct UnderUniformBuffer(DynamicUniformBuffer<UnderUniforms>);

#[derive(Component)]
struct ViewUnder {
    offset: u32,
    multisampled: bool,
    pipeline: CachedRenderPipelineId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct UnderKey {
    multisampled: bool,
    format: TextureFormat,
}

#[derive(Resource)]
struct UnderPipeline {
    layouts: [BindGroupLayoutDescriptor; 2],
    shader: Handle<bevy::shader::Shader>,
    fullscreen: FullscreenShader,
}

fn init_pipeline(
    mut commands: Commands,
    fullscreen: Res<FullscreenShader>,
    assets: Res<AssetServer>,
) {
    let layouts = [false, true].map(|multisampled| {
        let uniforms = uniform_buffer::<UnderUniforms>(true);
        let screen = texture_2d(TextureSampleType::Float { filterable: false });
        let entries = if multisampled {
            BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (uniforms, screen, texture_depth_2d_multisampled()),
            )
            .to_vec()
        } else {
            BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (uniforms, screen, texture_depth_2d()),
            )
            .to_vec()
        };
        BindGroupLayoutDescriptor::new("underwater_layout", &entries)
    });
    commands.insert_resource(UnderPipeline {
        layouts,
        shader: assets.load(crate::materials::underwater_shader()),
        fullscreen: fullscreen.clone(),
    });
}

impl SpecializedRenderPipeline for UnderPipeline {
    type Key = UnderKey;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        let shader_defs = if key.multisampled {
            vec!["MULTISAMPLED".into()]
        } else {
            Vec::new()
        };
        RenderPipelineDescriptor {
            label: Some("underwater".into()),
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

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn prepare_under(
    mut commands: Commands,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<UnderPipeline>>,
    under_pipeline: Res<UnderPipeline>,
    mut buffer: ResMut<UnderUniformBuffer>,
    render: Option<Res<UnderwaterRender>>,
    views: Query<(Entity, &ExtractedView, &ExtractedCamera, Option<&Msaa>), With<UnderwaterView>>,
    stale: Query<Entity, (With<ViewUnder>, Without<UnderwaterView>)>,
) {
    for entity in &stale {
        commands.entity(entity).remove::<ViewUnder>();
    }
    buffer.0.clear();
    let Some(base) = render.and_then(|render| render.0) else {
        for (entity, ..) in &views {
            commands.entity(entity).remove::<ViewUnder>();
        }
        return;
    };
    for (entity, view, camera, msaa) in &views {
        let multisampled = msaa.is_some_and(|msaa| msaa.samples() > 1);
        let pipeline = pipelines.specialize(
            &cache,
            &under_pipeline,
            UnderKey {
                multisampled,
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
        let offset = buffer.0.push(&UnderUniforms {
            world_from_clip,
            size: size.as_vec2().extend(0.0).extend(0.0),
            ..base
        });
        commands.entity(entity).insert(ViewUnder {
            offset,
            multisampled,
            pipeline,
        });
    }
    buffer.0.write_buffer(&device, &queue);
}

fn draw_under(
    view: ViewQuery<(&ViewTarget, &ViewUnder, Option<&ViewPrepassTextures>), With<UnderwaterView>>,
    under: Option<Res<UnderPipeline>>,
    buffer: Res<UnderUniformBuffer>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (target, view_under, prepass) = view.into_inner();
    let Some(under) = under else {
        return;
    };
    let (Some(pipeline), Some(uniforms), Some(depth)) = (
        cache.get_render_pipeline(view_under.pipeline),
        buffer.0.binding(),
        prepass.and_then(ViewPrepassTextures::depth_only_view),
    ) else {
        return;
    };
    let layout = cache.get_bind_group_layout(&under.layouts[view_under.multisampled as usize]);
    let post = target.post_process_write();
    let bind_group = ctx.render_device().create_bind_group(
        "underwater",
        &layout,
        &BindGroupEntries::sequential((uniforms, post.source, depth)),
    );
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let mut pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("underwater"),
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
    let span = diagnostics.pass_span(&mut pass, "underwater");
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &bind_group, &[view_under.offset]);
    pass.draw(0..3, 0..1);
    span.end(&mut pass);
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::shader_lib;

    #[test]
    fn the_underwater_shader_compiles() {
        let source = include_str!("../shaders/underwater.wesl");
        for multisampled in [false, true] {
            shader_lib::validate(source, &[("MULTISAMPLED", multisampled)])
                .unwrap_or_else(|error| panic!("{error}"));
        }
    }

    #[test]
    fn underwater_uniforms_match_the_wesl_layout() {
        // A mat4 and five vec4s.
        assert_eq!(UnderUniforms::min_size().get(), 64 + 5 * 16);
    }
}
