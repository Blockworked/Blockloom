//! Persistent quad records decoded by the PBR, depth and shadow vertex stages.
use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::mesh::{Indices, MeshVertexAttribute, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{
    ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError, VertexFormat,
};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use blockloom_plugin_api::mesh::{CompactQuads, MeshData};

pub type QuadMaterial = ExtendedMaterial<StandardMaterial, QuadSurface>;
const QUAD_VERTEX: MeshVertexAttribute =
    MeshVertexAttribute::new("QuadVertex", 921617335, VertexFormat::Uint32);

pub fn register(app: &mut App) {
    bevy::asset::embedded_asset!(app, "shaders/plugin_quads.wesl");
    app.add_plugins(MaterialPlugin::<QuadMaterial>::default());
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct QuadSurface {
    #[storage(100, read_only)]
    pub records: Handle<ShaderBuffer>,
    #[storage(101, read_only)]
    pub palette: Handle<ShaderBuffer>,
    #[uniform(102)]
    pub scale: Vec4,
}

impl MaterialExtension for QuadSurface {
    fn vertex_shader() -> ShaderRef {
        shader()
    }
    fn prepass_vertex_shader() -> ShaderRef {
        shader()
    }
    fn deferred_vertex_shader() -> ShaderRef {
        shader()
    }
    fn specialize(
        _: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.vertex.buffers =
            vec![layout.0.get_layout(&[QUAD_VERTEX.at_shader_location(0)])?];
        for def in ["VERTEX_POSITIONS", "VERTEX_NORMALS", "VERTEX_COLORS"] {
            descriptor.vertex.shader_defs.push(def.into());
            if let Some(fragment) = &mut descriptor.fragment {
                fragment.shader_defs.push(def.into());
            }
        }
        Ok(())
    }
}
fn shader() -> ShaderRef {
    ShaderRef::Path(
        bevy::asset::AssetPath::from_path_buf(bevy::asset::embedded_path!(
            "shaders/plugin_quads.wesl"
        ))
        .with_source("embedded"),
    )
}

pub fn mesh(quads: &CompactQuads) -> Mesh {
    let mut indices = Vec::with_capacity(quads.records.len() * 6);
    for (i, [record, _]) in quads.records.iter().enumerate() {
        let order = if (record >> 24) & 1 == 1 {
            [0, 1, 2, 0, 2, 3]
        } else {
            [0, 2, 1, 0, 3, 2]
        };
        indices.extend(order.map(|v| i as u32 * 4 + v));
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        QUAD_VERTEX,
        (0..quads.records.len() as u32 * 4).collect::<Vec<_>>(),
    )
    .with_inserted_indices(Indices::U32(indices))
}

pub fn bounds(data: &MeshData) -> Aabb {
    let (lo, hi) = data.positions.as_chunks::<3>().0.iter().fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(Vec3::from(*p)), hi.max(Vec3::from(*p))),
    );
    Aabb::from_min_max(lo, hi)
}

pub fn surface(quads: &CompactQuads, buffers: &mut Assets<ShaderBuffer>) -> QuadSurface {
    QuadSurface {
        records: buffers.add(ShaderBuffer::from(
            quads
                .records
                .iter()
                .map(|r| UVec2::from(*r))
                .collect::<Vec<_>>(),
        )),
        palette: buffers.add(ShaderBuffer::from(
            quads
                .palette
                .iter()
                .map(|c| Vec4::from(*c))
                .collect::<Vec<_>>(),
        )),
        scale: Vec4::new(quads.voxel, 0.0, 0.0, 0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::{
        RenderApp,
        gpu_readback::{Readback, ReadbackComplete},
    };
    use std::sync::{Arc, Mutex};

    #[test]
    #[ignore = "needs a GPU or lavapipe"]
    fn compact_quads_render_like_cpu_geometry_with_prepasses() {
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(bevy::window::WindowPlugin {
                    primary_window: None,
                    ..default()
                })
                .set(bevy::render::RenderPlugin {
                    synchronous_pipeline_compilation: true,
                    ..default()
                })
                .disable::<bevy::winit::WinitPlugin>()
                .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>()
                .disable::<bevy::log::LogPlugin>()
                .disable::<bevy::audio::AudioPlugin>(),
        );
        register(&mut app);
        while app.plugins_state() == bevy::app::PluginsState::Adding {
            bevy::tasks::tick_global_task_pools_on_main_thread();
        }
        app.finish();
        app.cleanup();
        let quads = CompactQuads {
            records: (0..6u32)
                .map(|face| [(face % 2) | (face << 24), 1 | 1 << 8])
                .collect(),
            palette: vec![[0.5, 0.25, 0.125, 1.0]],
            voxel: 1.0,
        };
        let compact = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(mesh(&quads));
        let surface = surface(
            &quads,
            &mut app.world_mut().resource_mut::<Assets<ShaderBuffer>>(),
        );
        let material = app
            .world_mut()
            .resource_mut::<Assets<QuadMaterial>>()
            .add(QuadMaterial {
                base: StandardMaterial {
                    unlit: true,
                    ..default()
                },
                extension: surface,
            });
        let compact_entity = app
            .world_mut()
            .spawn((
                Mesh3d(compact),
                MeshMaterial3d(material),
                Aabb::from_min_max(Vec3::ZERO, Vec3::ONE),
                Transform::from_xyz(-1.3, -0.5, 0.0),
            ))
            .id();
        let reference = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(Cuboid::from_size(Vec3::ONE));
        let material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color: Color::linear_rgb(0.5, 0.25, 0.125),
                unlit: true,
                ..default()
            });
        app.world_mut().spawn((
            Mesh3d(reference.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_xyz(0.8, 0.0, 0.5),
        ));
        let mut image = Image::new_target_texture(
            128,
            64,
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            None,
        );
        image.texture_descriptor.usage |= bevy::render::render_resource::TextureUsages::COPY_SRC;
        let image = app.world_mut().resource_mut::<Assets<Image>>().add(image);
        app.world_mut().spawn((
            Camera3d::default(),
            bevy::camera::RenderTarget::from(image.clone()),
            bevy::core_pipeline::prepass::DepthPrepass,
            bevy::core_pipeline::prepass::NormalPrepass,
            bevy::core_pipeline::prepass::MotionVectorPrepass,
            bevy::render::occlusion_culling::OcclusionCulling,
            Msaa::Off,
            Transform::from_xyz(0.0, 0.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
        ));
        let pixels = Arc::new(Mutex::new(Vec::new()));
        let result = pixels.clone();
        app.world_mut().spawn(Readback::texture(image)).observe(
            move |event: On<ReadbackComplete>| {
                *result.lock().unwrap() = event.data.clone();
            },
        );
        for _ in 0..40 {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // Halfway through a cut switch, complementary masks preserve coverage.
        let camera = Vec3::new(0.0, 0.0, 5.0);
        let distance = camera.distance(Vec3::new(-1.3, -0.5, 0.0));
        app.world_mut().entity_mut(compact_entity).insert(
            bevy::camera::visibility::VisibilityRange {
                start_margin: distance - 0.5..distance + 0.5,
                end_margin: f32::MAX..f32::MAX,
                use_aabb: false,
            },
        );
        let distance = camera.distance(Vec3::new(-0.8, 0.0, 0.5));
        app.world_mut().spawn((
            Mesh3d(reference),
            MeshMaterial3d(material),
            Transform::from_xyz(-0.8, 0.0, 0.5),
            bevy::camera::visibility::VisibilityRange {
                start_margin: 0.0..0.0,
                end_margin: distance - 0.5..distance + 0.5,
                use_aabb: false,
            },
        ));
        for _ in 0..40 {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let cache = app
            .get_sub_app(RenderApp)
            .unwrap()
            .world()
            .resource::<bevy::render::render_resource::PipelineCache>();
        for pipeline in cache.pipelines() {
            if let bevy::render::render_resource::CachedPipelineState::Err(error) = &pipeline.state
            {
                panic!("quad pipeline: {error:?}");
            }
        }
        let pixels = pixels.lock().unwrap();
        assert_eq!(pixels.len(), 128 * 64 * 4);
        let mut colored = 0;
        for y in 12..52 {
            for x in 8..64 {
                let a = (y * 128 + x) * 4;
                let b = (y * 128 + 127 - x) * 4;
                assert!(
                    pixels[a..a + 3]
                        .iter()
                        .zip(&pixels[b..b + 3])
                        .all(|(a, b)| a.abs_diff(*b) <= 2),
                    "compact and reference differ at {x},{y}: {:?} vs {:?}",
                    &pixels[a..a + 4],
                    &pixels[b..b + 4]
                );
                if pixels[a] > 100 && pixels[a] > pixels[a + 1] {
                    colored += 1;
                }
            }
        }
        assert!(colored > 50, "compact quads must draw visible pixels");
        let render = app.get_sub_app(RenderApp).unwrap();
        let cache = render
            .world()
            .resource::<bevy::render::render_resource::PipelineCache>();
        assert!(
            cache.waiting_pipelines().next().is_none(),
            "pipelines must finish compiling"
        );
    }
}
