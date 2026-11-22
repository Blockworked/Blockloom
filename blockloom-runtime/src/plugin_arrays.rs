//! Texture arrays for plugin meshes: one image per stack of `texture_layers`,
//! drawn by a standard material whose base color reads the vertex's layer.
use bevy::asset::{LoadState, RenderAssetUsages};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor,
    TextureViewDimension,
};
use bevy::shader::ShaderRef;
use std::collections::HashMap;

pub type ArrayMaterial = ExtendedMaterial<StandardMaterial, ArraySurface>;

pub fn register(app: &mut App) {
    bevy::asset::embedded_asset!(app, "shaders/plugin_arrays.wesl");
    app.add_plugins(MaterialPlugin::<ArrayMaterial>::default())
        .init_resource::<TextureArrays>()
        .add_systems(Update, fill_arrays);
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct ArraySurface {
    #[texture(100, dimension = "2d_array")]
    #[sampler(101)]
    pub array: Handle<Image>,
}

impl MaterialExtension for ArraySurface {
    fn fragment_shader() -> ShaderRef {
        shader()
    }
    fn prepass_fragment_shader() -> ShaderRef {
        shader()
    }
    fn deferred_fragment_shader() -> ShaderRef {
        shader()
    }
}

fn shader() -> ShaderRef {
    ShaderRef::Path(
        bevy::asset::AssetPath::from_path_buf(bevy::asset::embedded_path!(
            "shaders/plugin_arrays.wesl"
        ))
        .with_source("embedded"),
    )
}

/// The arrays made so far, one per distinct list of layer paths.
#[derive(Resource, Default)]
pub struct TextureArrays {
    stacks: HashMap<Vec<String>, Stack>,
}

struct Stack {
    sources: Vec<Handle<Image>>,
    array: Handle<Image>,
    filled: bool,
}

impl TextureArrays {
    /// The array for these layers: white until every source has loaded (or
    /// failed, which leaves that layer white).
    pub fn array(
        &mut self,
        layers: &[String],
        server: Option<&AssetServer>,
        images: &mut Assets<Image>,
    ) -> Handle<Image> {
        if let Some(stack) = self.stacks.get(layers) {
            return stack.array.clone();
        }
        let sources = server.map_or_else(Vec::new, |server| {
            layers
                .iter()
                .map(|path| server.load(path.clone()))
                .collect()
        });
        let array = images.add(stacked(1, 1, vec![[255; 4]; layers.len()]));
        self.stacks.insert(
            layers.to_vec(),
            Stack {
                sources,
                array: array.clone(),
                filled: false,
            },
        );
        array
    }
}

/// Stack one image per layer, each `w` by `h` RGBA texels (or a single
/// color per layer when `w` and `h` are 1).
fn stacked(w: u32, h: u32, layers: Vec<[u8; 4]>) -> Image {
    let bytes = layers
        .iter()
        .flat_map(|c| std::iter::repeat_n(*c, (w * h) as usize).flatten())
        .collect();
    from_bytes(w, h, layers.len() as u32, bytes)
}

fn from_bytes(w: u32, h: u32, count: u32, bytes: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: count.max(1),
        },
        TextureDimension::D2,
        bytes,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    // Block textures are pixel art that repeats across merged faces.
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Nearest,
        min_filter: ImageFilterMode::Nearest,
        ..default()
    });
    image
}

/// One layer's RGBA texels at `w` by `h`, scaled nearest from the source.
fn layer_texels(source: Option<&Image>, w: u32, h: u32) -> Vec<u8> {
    let rgba = source.and_then(|image| {
        let size = image.size();
        let rgba = image.clone().try_into_dynamic().ok()?.to_rgba8().into_raw();
        Some((size.x, size.y, rgba))
    });
    let Some((sw, sh, rgba)) = rgba.filter(|(sw, sh, _)| *sw > 0 && *sh > 0) else {
        return vec![255; (w * h * 4) as usize];
    };
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let i = (((y * sh / h) * sw + x * sw / w) * 4) as usize;
            out.extend_from_slice(&rgba[i..i + 4]);
        }
    }
    out
}

/// Fill each array once its sources have all settled, at the largest
/// source's size.
fn fill_arrays(
    mut arrays: ResMut<TextureArrays>,
    server: Option<Res<AssetServer>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(server) = server else {
        return;
    };
    for stack in arrays.stacks.values_mut().filter(|s| !s.filled) {
        let settled = stack.sources.iter().all(|h| {
            matches!(
                server.load_state(h.id()),
                LoadState::Loaded | LoadState::Failed(_)
            )
        });
        if !settled {
            continue;
        }
        let (w, h) = stack
            .sources
            .iter()
            .filter_map(|s| images.get(s.id()))
            .map(|i| (i.width(), i.height()))
            .max()
            .unwrap_or((1, 1));
        let bytes = stack
            .sources
            .iter()
            .flat_map(|s| layer_texels(images.get(s.id()), w, h))
            .collect();
        let image = from_bytes(w, h, stack.sources.len() as u32, bytes);
        if let Some(mut target) = images.get_mut(stack.array.id()) {
            *target = image;
        }
        stack.filled = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_scale_to_the_stack_and_missing_ones_are_white() {
        let mut small = Image::new(
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            vec![10, 20, 30, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        small.sampler = ImageSampler::Default;
        assert_eq!(
            layer_texels(Some(&small), 2, 2),
            [10, 20, 30, 255].repeat(4)
        );
        assert_eq!(layer_texels(None, 1, 2), vec![255; 8]);
        let image = stacked(1, 1, vec![[255; 4]; 3]);
        assert_eq!(image.texture_descriptor.size.depth_or_array_layers, 3);
        assert_eq!(
            image.texture_view_descriptor.unwrap().dimension,
            Some(TextureViewDimension::D2Array)
        );
    }
}
