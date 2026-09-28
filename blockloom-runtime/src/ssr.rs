//! Trace specular light at half size, then add it at scene resolution.
//! Bevy still owns SSR settings, view layouts and the full-size fallback.
use crate::passes::{self, WorkingTargets};
use bevy::core_pipeline::core_3d::main_opaque_pass_3d;
use bevy::core_pipeline::prepass::ViewPrepassTextures;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::pbr::{Bluenoise, MeshViewBindGroup, ScreenSpaceReflections};
use bevy::pbr::{
    ScreenSpaceReflectionsPipelineId, prepare_ssr_pipelines, screen_space_reflections,
};
use bevy::prelude::*;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::ViewTarget;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use std::collections::HashMap;

#[derive(Component, Clone, Copy, ExtractComponent)]
#[extract_app(RenderApp)]
struct HalfSsr;

#[derive(Component)]
struct HalfPipeline(CachedRenderPipelineId);

#[derive(Resource)]
struct HalfPipelines {
    ids: HashMap<CachedRenderPipelineId, CachedRenderPipelineId>,
    samplers: [Sampler; 3],
}

pub fn register(app: &mut App) {
    app.add_plugins(ExtractComponentPlugin::<HalfSsr>::default())
        .add_systems(
            PostUpdate,
            configure.after(crate::environment::apply_environment),
        );
    let Some(render) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render
        .add_systems(RenderStartup, init)
        .add_systems(
            Render,
            prepare
                .in_set(RenderSystems::PrepareBindGroups)
                .after(prepare_ssr_pipelines),
        )
        .add_systems(
            Core3d,
            trace
                .in_set(Core3dSystems::MainPass)
                .after(screen_space_reflections)
                .before(main_opaque_pass_3d),
        );
}

fn configure(
    mut commands: Commands,
    patches: Option<Res<crate::pbr_patch::PatchedShaders>>,
    cameras: Query<
        (
            Entity,
            Has<ScreenSpaceReflections>,
            Has<HalfSsr>,
            Option<&Camera>,
        ),
        With<crate::world::WorldCamera>,
    >,
) {
    let patched = patches.is_some_and(|p| p.applied.contains(&"bevy_pbr/ssr.wesl"));
    for (entity, ssr, half, camera) in &cameras {
        let wanted = ssr && patched && camera.is_none_or(|camera| camera.viewport.is_none());
        if wanted && !half {
            commands.entity(entity).insert((HalfSsr, WorkingTargets));
        } else if !wanted && half {
            commands.entity(entity).remove::<HalfSsr>();
        }
    }
}

fn init(mut commands: Commands, device: Res<RenderDevice>) {
    let sampler = |filter| {
        device.create_sampler(&SamplerDescriptor {
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            mag_filter: filter,
            min_filter: filter,
            ..default()
        })
    };
    commands.insert_resource(HalfPipelines {
        ids: HashMap::new(),
        samplers: [
            sampler(FilterMode::Linear),
            sampler(FilterMode::Linear),
            sampler(FilterMode::Nearest),
        ],
    });
}

fn prepare(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    mut pipelines: ResMut<HalfPipelines>,
    views: Query<
        (
            Entity,
            &ScreenSpaceReflectionsPipelineId,
            &passes::ViewUpsample,
        ),
        With<HalfSsr>,
    >,
) {
    for (entity, original, upsample) in &views {
        // The descriptor isn't readable until Bevy has processed its queue.
        if cache.get_render_pipeline(original.0).is_none() {
            continue;
        }
        let half = *pipelines.ids.entry(original.0).or_insert_with(|| {
            let mut descriptor = cache.get_render_pipeline_descriptor(original.0).clone();
            descriptor.label = Some("ssr_half".into());
            let fragment = descriptor.fragment.as_mut().expect("SSR fragment shader");
            fragment.shader_defs.push("BLOCKLOOM_HALF_SSR".into());
            fragment.targets[0].as_mut().unwrap().format = passes::WORKING_FORMAT;
            cache.queue_render_pipeline(descriptor)
        });
        // Keep the stock pass until both replacement pipelines are ready.
        if cache.get_render_pipeline(half).is_some()
            && cache.get_render_pipeline(upsample.add_to_view).is_some()
        {
            commands
                .entity(entity)
                .insert(HalfPipeline(half))
                .remove::<ScreenSpaceReflectionsPipelineId>();
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn trace(
    view: ViewQuery<
        (
            &ViewTarget,
            &MeshViewBindGroup,
            &HalfPipeline,
            &passes::WorkingTargetSet,
            &passes::ViewUpsample,
            &passes::ViewFrameUniforms,
            &ViewPrepassTextures,
        ),
        (With<HalfSsr>, Without<ScreenSpaceReflectionsPipelineId>),
    >,
    cache: Res<PipelineCache>,
    pipelines: Res<HalfPipelines>,
    upsampler: Res<passes::BilateralUpsample>,
    frame: Res<passes::FrameUniformBuffer>,
    bluenoise: Res<Bluenoise>,
    images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    let (target, mesh_view, half, working, up, uniforms, depth) = view.into_inner();
    let Some(pipeline) = cache.get_render_pipeline(half.0) else {
        return;
    };
    if cache.get_render_pipeline(up.add_to_view).is_none()
        || frame.0.binding().is_none()
        || depth.depth_only_view().is_none()
    {
        return;
    }
    let Some(noise) = images.get(&bluenoise.texture) else {
        return;
    };
    let noise = noise.texture.create_view(&TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    let descriptor = cache.get_render_pipeline_descriptor(half.0);
    let binding = ctx.render_device().create_bind_group(
        "ssr_half",
        &cache.get_bind_group_layout(&descriptor.layout[2]),
        &BindGroupEntries::sequential((
            target.main_texture_view(),
            &pipelines.samplers[0],
            &pipelines.samplers[1],
            &pipelines.samplers[2],
            &noise,
        )),
    );
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    {
        let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("ssr_half"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &working.scratch[0].default_view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let span = diagnostics.pass_span(&mut pass, "ssr_half");
        pass.set_render_pipeline(pipeline);
        pass.set_bind_group(0, &mesh_view.main, &mesh_view.main_offsets);
        pass.set_bind_group(1, &mesh_view.binding_array, &[]);
        pass.set_bind_group(2, &binding, &[]);
        pass.draw(0..3, 0..1);
        span.end(&mut pass);
    }
    // Only specular light is filtered, so the scene keeps its native detail.
    passes::upsample(
        &mut ctx,
        &cache,
        &upsampler,
        &frame,
        (up, uniforms, Some(depth)),
        up.add_to_view,
        &working.scratch[0].default_view,
        target.main_texture_view(),
    );
}

pub fn patch_shader(
    source: &str,
    _: &crate::pbr_patch::PbrPatches,
) -> Result<String, &'static str> {
    let signature = "fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {";
    let output = "return vec4(fragment.rgb + indirect_light, fragment.a);";
    let depth = "frag_coord.z = prepass_utils::prepass_depth(in.position, 0u);";
    if !source.contains(signature) || !source.contains(output) || !source.contains(depth) {
        return Err("SSR fragment signature or output");
    }
    Ok(source
        .replace(
            signature,
            "fn fragment(input: FullscreenVertexOutput) -> @location(0) vec4<f32> {\n\
             var in = input;\n\
             @if(BLOCKLOOM_HALF_SSR) {\n\
             in.position = vec4(min(floor(input.position.xy) * 2.0,\n\
                 view.viewport.zw - 1.0) + 0.5, input.position.zw);\n\
             }",
        )
        .replace(
            depth,
            &format!(
                "{depth}\n@if(BLOCKLOOM_HALF_SSR) {{\n\
                if (frag_coord.z == 0.0) {{ return vec4(0.0); }}\n}}"
            ),
        )
        .replace(
            output,
            "@if(BLOCKLOOM_HALF_SSR) return vec4(indirect_light, 0.0);\n\
             @else return vec4(fragment.rgb + indirect_light, fragment.a);",
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_ssr_requires_a_patched_shader_and_follows_the_camera_setting() {
        let mut app = App::new();
        app.init_resource::<crate::pbr_patch::PatchedShaders>()
            .add_systems(Update, configure);
        let camera = app
            .world_mut()
            .spawn((crate::world::WorldCamera, ScreenSpaceReflections::default()))
            .id();
        app.update();
        assert!(!app.world().entity(camera).contains::<HalfSsr>());
        assert!(!app.world().entity(camera).contains::<WorkingTargets>());
        app.world_mut()
            .resource_mut::<crate::pbr_patch::PatchedShaders>()
            .applied
            .push("bevy_pbr/ssr.wesl");
        app.update();
        assert!(app.world().entity(camera).contains::<HalfSsr>());
        assert!(app.world().entity(camera).contains::<WorkingTargets>());
        app.world_mut()
            .entity_mut(camera)
            .remove::<ScreenSpaceReflections>();
        app.update();
        assert!(!app.world().entity(camera).contains::<HalfSsr>());
    }

    #[test]
    fn a_changed_upstream_shader_is_refused() {
        assert!(patch_shader("fn fragment() {}", &default()).is_err());
    }
}
