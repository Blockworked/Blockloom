//! The terrain's surface and its grass. The surface blends up to four
//! layers from texture arrays through `blockloom::texturing`, so stochastic
//! tiling, macro and detail maps and the mask stack mean what they mean on
//! any other projected surface.

use super::{Built, Terrained};
use crate::materials::{DetailUniforms, MaskUniforms, SurfaceGlobals, load_surface_image};
use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat, TextureViewDescriptor,
    TextureViewDimension,
};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use blockloom_core::material::{SurfaceMaterial, TextureSampler, hex_to_linear};
use blockloom_core::terrain::{GrassLayer, MAX_LAYERS, TerrainSpec};
use std::path::Path;

pub type TerrainMaterial = ExtendedMaterial<StandardMaterial, TerrainSurface>;
pub type GrassMaterial = ExtendedMaterial<StandardMaterial, GrassBlades>;

/// Largest side a layer texture is resampled to.
const MAX_LAYER_SIZE: u32 = 2048;

pub fn register(app: &mut App) {
    app.add_plugins(MaterialPlugin::<TerrainMaterial>::default());
    app.add_plugins(MaterialPlugin::<GrassMaterial>::default());
}

/// `TerrainUniforms` in `terrain.wesl`.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct TerrainUniforms {
    /// Per layer: linear rgb tint and roughness.
    pub colors: [Vec4; 4],
    /// Per layer texture repeats per metre.
    pub density: Vec4,
    /// Layer count, then albedo, normal and roughness map bits.
    pub maps: Vec4,
    /// The terrain frame's rotation, for its baked normals.
    pub rotation: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct TerrainSurface {
    #[uniform(100)]
    pub terrain: TerrainUniforms,
    /// Layer weights per sample.
    #[texture(101)]
    #[sampler(102)]
    pub weights: Handle<Image>,
    /// Normal and cavity per sample.
    #[texture(103)]
    #[sampler(104)]
    pub surface: Handle<Image>,
    #[texture(105, dimension = "2d_array")]
    #[sampler(106)]
    pub albedo: Handle<Image>,
    #[texture(107, dimension = "2d_array")]
    pub normal: Handle<Image>,
    #[texture(108, dimension = "2d_array")]
    pub roughness: Handle<Image>,
    #[uniform(109)]
    pub masks: MaskUniforms,
    #[uniform(110)]
    pub detail: DetailUniforms,
    #[texture(111)]
    #[sampler(112)]
    pub macro_map: Option<Handle<Image>>,
    #[texture(113)]
    #[sampler(114)]
    pub detail_map: Option<Handle<Image>>,
    #[storage(115, read_only)]
    pub globals: Handle<ShaderBuffer>,
    #[texture(116)]
    pub surface_state: Option<Handle<Image>>,
}

impl MaterialExtension for TerrainSurface {
    fn fragment_shader() -> ShaderRef {
        crate::materials::terrain_shader()
    }
}

/// `grass` in `grass.wesl`: fade start, cull distance, stiffness, wind.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct GrassBlades {
    #[uniform(100)]
    pub fade: Vec4,
    #[storage(101, read_only)]
    pub globals: Handle<ShaderBuffer>,
}

impl MaterialExtension for GrassBlades {
    fn vertex_shader() -> ShaderRef {
        crate::materials::grass_shader()
    }

    // Blades are too thin to shadow well and too many to draw twice.
    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }
}

/// On a terrain root until its layer images have loaded and been packed
/// into the arrays.
#[derive(Component)]
pub struct PendingLayers {
    material: Handle<TerrainMaterial>,
    albedo: [Option<Handle<Image>>; MAX_LAYERS],
    normal: [Option<Handle<Image>>; MAX_LAYERS],
    roughness: [Option<Handle<Image>>; MAX_LAYERS],
}

/// The terrain's material, with plain placeholder arrays until its layer
/// images are in.
pub fn terrain_material(
    world: &mut World,
    spec: &TerrainSpec,
    built: &Built,
    dir: Option<&Path>,
    rotation: Quat,
) -> (Handle<TerrainMaterial>, Option<PendingLayers>) {
    let side = built.geometry.shape.side;
    let mut images = world.resource_mut::<Assets<Image>>();
    let weights = images.add(grid_image(side, bytes(&built.weights)));
    let surface = images.add(grid_image(side, bytes(&built.geometry.surface)));
    let placeholder = |images: &mut Assets<Image>, fill: [u8; 4], srgb: bool| {
        images.add(layer_array(
            4,
            &std::array::from_fn(|_| flatten(vec![fill; 16])),
            srgb,
        ))
    };
    let albedo = placeholder(&mut images, [255; 4], true);
    let normal = placeholder(&mut images, [128, 128, 255, 255], false);
    let roughness = placeholder(&mut images, [255; 4], false);
    let globals = world.resource::<SurfaceGlobals>().buffer.clone();
    let detail = &spec.texturing;
    let tiled = SurfaceMaterial {
        sampler: TextureSampler::Repeat,
        anisotropy: 8,
        ..default()
    };
    let assets = world.resource::<AssetServer>().clone();
    let (pending, macro_map, detail_map) = {
        let mut commands = world.commands();
        let mut load = |name: &str, srgb: bool| {
            load_surface_image(&mut commands, name, &tiled, dir, &assets, srgb)
        };
        let mut layer = |pick: fn(&blockloom_core::terrain::TerrainLayer) -> &str, srgb: bool| {
            let mut out: [Option<Handle<Image>>; MAX_LAYERS] = Default::default();
            for (k, layer) in spec.layers.iter().take(MAX_LAYERS).enumerate() {
                out[k] = load(pick(layer), srgb);
            }
            out
        };
        let albedo = layer(|l| &l.albedo_texture, true);
        let normal = layer(|l| &l.normal_texture, false);
        let roughness = layer(|l| &l.roughness_texture, false);
        let macro_map = load(&detail.macro_texture, false);
        let detail_map = load(&detail.detail_texture, false);
        ((albedo, normal, roughness), macro_map, detail_map)
    };
    world.flush();
    let mut colors = [Vec4::ONE; 4];
    let mut density = Vec4::ONE;
    for (k, layer) in spec.layers.iter().take(MAX_LAYERS).enumerate() {
        let c = hex_to_linear(&layer.color);
        colors[k] = Vec4::new(c[0], c[1], c[2], layer.roughness);
        density[k] = layer.texel_density;
    }
    let material = TerrainMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 1.0,
            opaque_render_method: bevy::material::OpaqueRendererMethod::Forward,
            ..default()
        },
        extension: TerrainSurface {
            terrain: TerrainUniforms {
                colors,
                density,
                maps: Vec4::new(spec.layers.len().min(MAX_LAYERS) as f32, 0.0, 0.0, 0.0),
                rotation: Vec4::from(rotation),
            },
            weights,
            surface,
            albedo,
            normal,
            roughness,
            surface_state: None,
            masks: MaskUniforms::of(&detail.masks),
            detail: DetailUniforms::of(detail, macro_map.is_some()),
            macro_map,
            detail_map,
            globals,
        },
    };
    let handle = world
        .resource_mut::<Assets<TerrainMaterial>>()
        .add(material);
    let (albedo, normal, roughness) = pending;
    let any = albedo
        .iter()
        .chain(&normal)
        .chain(&roughness)
        .any(Option::is_some);
    let pending = any.then(|| PendingLayers {
        material: handle.clone(),
        albedo,
        normal,
        roughness,
    });
    (handle, pending)
}

fn bytes(texels: &[[u8; 4]]) -> Vec<u8> {
    texels.iter().flatten().copied().collect()
}

fn flatten(texels: Vec<[u8; 4]>) -> Vec<u8> {
    bytes(&texels)
}

/// A per-sample grid texture: linear, clamped, no mips.
pub fn grid_image(side: u32, data: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    image
}

/// A four-layer array from each layer's full mip chain, `size` on a side.
fn layer_array(size: u32, layers: &[Vec<u8>; MAX_LAYERS], srgb: bool) -> Image {
    let mips = size.max(1).ilog2() + 1;
    let data: Vec<u8> = layers.iter().flatten().copied().collect();
    // Every layer carries the same chain, so one layer's length says whether it has mips.
    let has_mips = layers[0].len() > (size * size * 4) as usize;
    let mut image = Image::new_uninit(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: MAX_LAYERS as u32,
        },
        TextureDimension::D2,
        if srgb {
            TextureFormat::Rgba8UnormSrgb
        } else {
            TextureFormat::Rgba8Unorm
        },
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = if has_mips { mips } else { 1 };
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    });
    image
}

/// One layer at `size`, with every mip after it, box-filtered. A layer
/// with no image is `fill`.
fn layer_mips(image: Option<&Image>, size: u32, fill: [u8; 4]) -> Vec<u8> {
    let base = image
        .and_then(|image| image.clone().try_into_dynamic().ok())
        .map(|dynamic| {
            dynamic
                .resize_exact(size, size, image::imageops::FilterType::Triangle)
                .to_rgba8()
                .into_raw()
        })
        .unwrap_or_else(|| flatten(vec![fill; (size * size) as usize]));
    let mut out = base.clone();
    let mut level = base;
    let mut side = size;
    while side > 1 {
        let half = side / 2;
        let mut next = vec![0u8; (half * half * 4) as usize];
        for y in 0..half {
            for x in 0..half {
                for c in 0..4 {
                    let at = |xx: u32, yy: u32| level[((yy * side + xx) * 4 + c) as usize] as u32;
                    let sum = at(2 * x, 2 * y)
                        + at(2 * x + 1, 2 * y)
                        + at(2 * x, 2 * y + 1)
                        + at(2 * x + 1, 2 * y + 1);
                    next[((y * half + x) * 4 + c) as usize] = ((sum + 2) / 4) as u8;
                }
            }
        }
        out.extend_from_slice(&next);
        level = next;
        side = half;
    }
    out
}

/// Packs each terrain's layer images into its arrays once they've loaded
/// (or failed, which leaves that layer plain).
pub fn build_layer_arrays(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
    pending: Query<(Entity, &PendingLayers)>,
) {
    for (entity, layers) in &pending {
        let all = layers
            .albedo
            .iter()
            .chain(&layers.normal)
            .chain(&layers.roughness)
            .flatten();
        let settled = all
            .clone()
            .all(|handle| images.get(handle).is_some() || assets.load_state(handle).is_failed());
        if !settled {
            continue;
        }
        let size = all
            .filter_map(|handle| images.get(handle))
            .map(|image| image.width().max(image.height()))
            .max()
            .unwrap_or(4)
            .clamp(4, MAX_LAYER_SIZE)
            .next_power_of_two()
            .min(MAX_LAYER_SIZE);
        let pack = |set: &[Option<Handle<Image>>; MAX_LAYERS], fill: [u8; 4]| {
            std::array::from_fn(|k| {
                layer_mips(set[k].as_ref().and_then(|h| images.get(h)), size, fill)
            })
        };
        let albedo = layer_array(size, &pack(&layers.albedo, [255; 4]), true);
        let normal = layer_array(size, &pack(&layers.normal, [128, 128, 255, 255]), false);
        let roughness = layer_array(size, &pack(&layers.roughness, [255; 4]), false);
        let bits = |set: &[Option<Handle<Image>>; MAX_LAYERS]| {
            set.iter()
                .enumerate()
                .filter(|(_, h)| h.as_ref().is_some_and(|h| images.get(h).is_some()))
                .fold(0u32, |bits, (k, _)| bits | 1 << k) as f32
        };
        let maps = (
            bits(&layers.albedo),
            bits(&layers.normal),
            bits(&layers.roughness),
        );
        let (albedo, normal, roughness) = (
            images.add(albedo),
            images.add(normal),
            images.add(roughness),
        );
        if let Some(mut material) = materials.get_mut(&layers.material) {
            let ext = &mut material.extension;
            ext.albedo = albedo;
            ext.normal = normal;
            ext.roughness = roughness;
            ext.terrain.maps.y = maps.0;
            ext.terrain.maps.z = maps.1;
            ext.terrain.maps.w = maps.2;
        }
        commands.entity(entity).remove::<PendingLayers>();
    }
}

/// Keeps a terrain's baked normals turned with its actor.
pub fn follow_rotation(
    roots: Query<(&GlobalTransform, &Terrained), Changed<GlobalTransform>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
) {
    for (transform, terrained) in &roots {
        let rotation = Vec4::from(transform.rotation());
        if materials
            .get(&terrained.material)
            .is_some_and(|m| m.extension.terrain.rotation != rotation)
            && let Some(mut material) = materials.get_mut(&terrained.material)
        {
            material.extension.terrain.rotation = rotation;
        }
    }
}

/// One grass material per layer.
pub fn grass_material(
    materials: &mut Assets<GrassMaterial>,
    globals: Handle<ShaderBuffer>,
    grass: &GrassLayer,
) -> Handle<GrassMaterial> {
    materials.add(GrassMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.8,
            reflectance: 0.3,
            double_sided: true,
            cull_mode: None,
            opaque_render_method: bevy::material::OpaqueRendererMethod::Forward,
            ..default()
        },
        extension: GrassBlades {
            fade: Vec4::new(
                grass.fade_start,
                grass.cull_distance,
                grass.stiffness,
                grass.wind,
            ),
            globals,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mip count has to match the bytes, or wgpu reads past the end on upload.
    #[test]
    fn a_layer_array_counts_mips_from_one_layer() {
        let plain = layer_array(4, &std::array::from_fn(|_| vec![0; 64]), false);
        assert_eq!(plain.texture_descriptor.mip_level_count, 1);
        let chained: [Vec<u8>; MAX_LAYERS] =
            std::array::from_fn(|_| layer_mips(None, 4, [1, 2, 3, 4]));
        let mipped = layer_array(4, &chained, false);
        assert_eq!(mipped.texture_descriptor.mip_level_count, 3);
    }
}
