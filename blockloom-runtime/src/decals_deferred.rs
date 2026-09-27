//! Writes projected marks into the G-buffer before deferred lighting reads it.

use bevy::core_pipeline::deferred::DEFERRED_PREPASS_FORMAT;
use bevy::core_pipeline::prepass::ViewPrepassTextures;
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::pbr::deferred::DeferredLightingLayout;
use bevy::pbr::{MeshPipelineKey, MeshViewBindGroup, ViewKeyCache};
use bevy::prelude::*;
use bevy::render::render_resource::binding_types::texture_2d;
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::ExtractedView;
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems};

pub fn register(app: &mut App) {
    if app.get_sub_app(RenderApp).is_none() {
        return;
    }
    bevy::asset::embedded_asset!(app, "shaders/decals_deferred.wesl");
    bevy::asset::embedded_asset!(app, "shaders/decals_copy.wesl");
    app.sub_app_mut(RenderApp)
        .init_resource::<Active>()
        .add_systems(ExtractSchedule, extract)
        .add_systems(RenderStartup, init)
        .add_systems(Render, prepare.in_set(RenderSystems::PrepareBindGroups))
        .add_systems(
            Core3d,
            draw.after(Core3dSystems::Prepass)
                .before(Core3dSystems::MainPass),
        );
}

#[derive(Resource, Default)]
struct Active(bool);

fn extract(mut commands: Commands, decals: Extract<Res<super::decals::Decals>>) {
    commands.insert_resource(Active(!decals.pool.marks.is_empty()));
}

#[derive(Resource)]
struct Pipelines {
    shader: Handle<Shader>,
    fullscreen: FullscreenShader,
    copy_layout: BindGroupLayoutDescriptor,
    copy: CachedRenderPipelineId,
    apply: std::collections::HashMap<MeshPipelineKey, CachedRenderPipelineId>,
}

#[derive(Component)]
struct Target {
    scratch: CachedTexture,
    pipeline: CachedRenderPipelineId,
}

fn init(
    mut commands: Commands,
    assets: Res<AssetServer>,
    fullscreen: Res<FullscreenShader>,
    cache: Res<PipelineCache>,
) {
    let copy_layout = BindGroupLayoutDescriptor::new(
        "decal_copy",
        &BindGroupLayoutEntries::single(
            ShaderStages::FRAGMENT,
            texture_2d(TextureSampleType::Uint),
        ),
    );
    let copy = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("decal_copy".into()),
        layout: vec![copy_layout.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: bevy::asset::load_embedded_asset!(assets.as_ref(), "shaders/decals_copy.wesl"),
            targets: vec![Some(DEFERRED_PREPASS_FORMAT.into())],
            ..default()
        }),
        ..default()
    });
    commands.insert_resource(Pipelines {
        shader: bevy::asset::load_embedded_asset!(assets.as_ref(), "shaders/decals_deferred.wesl"),
        fullscreen: fullscreen.clone(),
        copy_layout,
        copy,
        apply: default(),
    });
}

fn prepare(
    mut commands: Commands,
    active: Res<Active>,
    mut pipelines: ResMut<Pipelines>,
    lighting: Res<DeferredLightingLayout>,
    keys: Res<ViewKeyCache>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    mut textures: ResMut<TextureCache>,
    views: Query<(Entity, &ExtractedView, &ViewPrepassTextures)>,
) {
    for (entity, view, prepass) in &views {
        if !active.0 || prepass.deferred.is_none() {
            commands.entity(entity).remove::<Target>();
            continue;
        }
        let Some(key) = keys.get(&view.retained_view_entity) else {
            continue;
        };
        let key = *key;
        let pipeline = if let Some(id) = pipelines.apply.get(&key) {
            *id
        } else {
            // Keep Bevy's view layout and feature defines in lockstep with its lighting pass.
            let mut descriptor = lighting.specialize(key);
            descriptor.label = Some("decal_gbuffer".into());
            descriptor.layout.truncate(2);
            descriptor.vertex = pipelines.fullscreen.to_vertex_state();
            descriptor.depth_stencil = None;
            let fragment = descriptor.fragment.as_mut().unwrap();
            fragment.shader = pipelines.shader.clone();
            fragment.targets = vec![Some(DEFERRED_PREPASS_FORMAT.into())];
            let id = cache.queue_render_pipeline(descriptor);
            pipelines.apply.insert(key, id);
            id
        };
        let scratch = textures.get(
            &device,
            TextureDescriptor {
                label: Some("decal_gbuffer_scratch"),
                size: prepass.size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: DEFERRED_PREPASS_FORMAT,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
        );
        commands.entity(entity).insert(Target { scratch, pipeline });
    }
}

fn draw(
    view: ViewQuery<(&MeshViewBindGroup, &ViewPrepassTextures, &Target)>,
    active: Res<Active>,
    pipelines: Res<Pipelines>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    if !active.0 {
        return;
    }
    let (bindings, prepass, target) = view.into_inner();
    let Some(deferred) = &prepass.deferred else {
        return;
    };
    let (Some(apply), Some(copy)) = (
        cache.get_render_pipeline(target.pipeline),
        cache.get_render_pipeline(pipelines.copy),
    ) else {
        return;
    };
    let copy_bindings = ctx.render_device().create_bind_group(
        "decal_copy",
        &cache.get_bind_group_layout(&pipelines.copy_layout),
        &BindGroupEntries::single(&target.scratch.default_view),
    );
    {
        let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("decal_gbuffer"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &target.scratch.default_view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(default()),
                    store: StoreOp::Store,
                },
            })],
            ..default()
        });
        pass.set_render_pipeline(apply);
        pass.set_bind_group(0, &bindings.main, &bindings.main_offsets);
        pass.set_bind_group(1, &bindings.binding_array, &[]);
        pass.draw(0..3, 0..1);
    }
    // The source G-buffer cannot also be an attachment in the first pass.
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("decal_copy"),
        color_attachments: &[Some(deferred.get_attachment())],
        ..default()
    });
    pass.set_render_pipeline(copy);
    pass.set_bind_group(0, &copy_bindings, &[]);
    pass.draw(0..3, 0..1);
}
