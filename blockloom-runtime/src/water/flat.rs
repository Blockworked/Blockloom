//! 2D water: a strip from the surface down to the body's depth, drawn over
//! what is behind it with the same waves the floating bodies ride.

use super::surface::WavesUniform;
use super::{LiveBody, WaterState, rgb};
use crate::engine::Engine;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dPlugin};
use std::collections::HashMap;

pub fn register(app: &mut App) {
    app.add_plugins(Material2dPlugin::<FlatWater>::default())
        .init_resource::<Strips>()
        .add_systems(Update, sync_strips.after(crate::world::interpolate_poses));
}

/// `FlatLook` in `water_2d.wesl`, field for field.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct FlatLook {
    pub shallow: Vec4,
    pub deep: Vec4,
    pub foam_color: Vec4,
    pub foam: Vec4,
    pub light: Vec4,
}

impl FlatLook {
    fn of(live: &LiveBody) -> Self {
        let spec = &live.spec;
        FlatLook {
            shallow: rgb(&spec.look.shallow).extend(spec.look.clarity),
            deep: rgb(&spec.look.deep).extend(spec.look.absorption),
            foam_color: rgb(&spec.foam.color).extend(live.foam()),
            foam: Vec4::new(
                spec.foam.shore,
                spec.foam.crest,
                spec.foam.scale,
                spec.foam.drift,
            ),
            light: Vec4::new(
                spec.underwater.caustics,
                spec.underwater.caustics_scale,
                0.0,
                0.0,
            ),
        }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct FlatWater {
    #[uniform(0)]
    pub waves: WavesUniform,
    #[uniform(1)]
    pub look: FlatLook,
    #[texture(2, filterable = false)]
    pub ripples: Option<Handle<Image>>,
}

impl Material2d for FlatWater {
    fn vertex_shader() -> ShaderRef {
        crate::materials::water_2d_shader()
    }

    fn fragment_shader() -> ShaderRef {
        crate::materials::water_2d_shader()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}

/// Every drawn body, by actor id.
#[derive(Resource, Default)]
struct Strips(HashMap<String, Strip>);

struct Strip {
    entity: Entity,
    material: Handle<FlatWater>,
    shape: [u32; 3],
    ripples: Option<(u64, Handle<Image>)>,
}

/// A strip `2 half` wide and `depth` deep, its top row at y 0 (uv y 0).
fn strip_mesh(half: f32, depth: f32, columns: u32) -> Mesh {
    let mut positions = Vec::with_capacity(2 * (columns as usize + 1));
    let mut uvs = Vec::with_capacity(positions.capacity());
    for i in 0..=columns {
        let x = -half + 2.0 * half * i as f32 / columns as f32;
        let u = i as f32 / columns as f32;
        positions.extend([[x, 0.0, 0.0], [x, -depth, 0.0]]);
        uvs.extend([[u, 0.0], [u, 1.0]]);
    }
    let mut indices = Vec::with_capacity(columns as usize * 6);
    for i in 0..columns {
        let (a, b) = (2 * i, 2 * i + 2);
        indices.extend([a, a + 1, b, b, a + 1, b + 1]);
    }
    let normals = vec![[0.0, 0.0, 1.0]; positions.len()];
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

fn render_time(engine: &Engine, state: &WaterState, fixed: &Time<Fixed>, time: &Time) -> f32 {
    match (engine.running, engine.paused) {
        (true, false) => state.time + fixed.overstep_fraction() * fixed.delta_secs(),
        (true, true) => state.time,
        (false, _) => time.elapsed_secs() % 3600.0,
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_strips(
    mut commands: Commands,
    engine: NonSend<Engine>,
    state: Res<WaterState>,
    fixed: Res<Time<Fixed>>,
    time: Res<Time>,
    mut strips: ResMut<Strips>,
    mut transforms: Query<&mut Transform, With<super::WaterSurface>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<FlatWater>>,
    mut images: ResMut<Assets<Image>>,
) {
    let now = render_time(&engine, &state, &fixed, &time);
    strips.0.retain(|id, strip| {
        let keep = state.body(id).is_some();
        if !keep {
            commands.entity(strip.entity).despawn();
        }
        keep
    });
    for live in &state.bodies {
        let body = &live.body;
        let half = body.half[0];
        let spacing = (live.spec.waves.wavelength / 12.0).max(1.0);
        let columns = ((2.0 * half / spacing).ceil() as u32).clamp(16, 512);
        let shape = [half.to_bits(), body.depth.to_bits(), columns];
        let strip = strips.0.entry(body.id.clone()).or_insert_with(|| {
            let material = materials.add(FlatWater {
                waves: WavesUniform::default(),
                look: FlatLook::default(),
                ripples: None,
            });
            let entity = commands
                .spawn((
                    super::WaterSurface,
                    Mesh2d(meshes.add(strip_mesh(half, body.depth, columns))),
                    MeshMaterial2d(material.clone()),
                    Transform::default(),
                    Visibility::default(),
                    bevy::camera::visibility::NoFrustumCulling,
                    Name::new(format!("water {}", body.id)),
                ))
                .id();
            Strip {
                entity,
                material,
                shape,
                ripples: None,
            }
        });
        if strip.shape != shape {
            strip.shape = shape;
            commands
                .entity(strip.entity)
                .insert(Mesh2d(meshes.add(strip_mesh(half, body.depth, columns))));
        }
        if let Ok(mut transform) = transforms.get_mut(strip.entity) {
            *transform = Transform::from_xyz(body.center[0], body.center[1], body.center[2]);
        }
        let ripples = super::sync_ripple_image(&mut strip.ripples, live, state.tick, &mut images);
        if let Some(mut material) = materials.get_mut(&strip.material) {
            material.waves = WavesUniform::of(live, now, super::between_ticks(&engine, &fixed));
            material.look = FlatLook::of(live);
            if material.ripples != ripples {
                material.ripples = ripples;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_strip_hangs_from_its_surface() {
        let mesh = strip_mesh(100.0, 50.0, 4);
        assert_eq!(mesh.count_vertices(), 10);
        assert_eq!(mesh.indices().unwrap().len(), 24);
    }

    #[test]
    fn the_flat_shader_compiles() {
        let stub = "struct View { world_position: vec3<f32> }\n\
@group(0) @binding(0) var<uniform> view: View;\n\
struct Vertex { @builtin(instance_index) instance_index: u32, @location(0) position: vec3<f32>, \
@location(2) uv: vec2<f32> }\n\
struct VertexOutput { @builtin(position) position: vec4<f32>, @location(0) world_position: vec4<f32>, \
@location(1) world_normal: vec3<f32>, @location(2) uv: vec2<f32>, @location(5) @interpolate(flat) instance_index: u32 }\n\
fn decompress_vertex(v: Vertex, i: u32) -> Vertex { return v; }\n\
fn get_world_from_local(i: u32) -> mat4x4<f32> { return mat4x4<f32>(); }\n\
fn mesh2d_position_local_to_world(m: mat4x4<f32>, p: vec4<f32>) -> vec4<f32> { return m * p; }\n\
fn mesh2d_position_world_to_clip(p: vec4<f32>) -> vec4<f32> { return p; }\n";
        let source =
            crate::materials::tests::stubbed(include_str!("../shaders/water_2d.wesl"), stub)
                .replace("mesh_functions::", "")
                // The imports these guarded are gone.
                .replace(
                    "@if(TONEMAP_IN_SHADER)\n@if(SRGB_OUTPUT)\n@if(OKLAB_OUTPUT)\n",
                    "",
                );
        blockloom_core::shader_lib::validate(&source, &[("VERTEX_UVS", true)])
            .unwrap_or_else(|error| panic!("{error}"));
    }
}
