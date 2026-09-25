//! The 3D sky: the project's HDR panorama or strip as a cube, drawn behind the
//! world and filtered into the environment light. A build ships it baked to
//! BC6H (see `blockloom_core::build`); the editor decodes the source file.
//! Both load on a background task, and the scene stands in its clear color
//! until the cube lands.

use crate::engine::Engine;
use crate::world::WorldCamera;
use bevy::asset::RenderAssetUsages;
use bevy::light::{GeneratedEnvironmentMapLight, Skybox};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
    WgpuFeatures,
};
use bevy::render::renderer::RenderDevice;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use blockloom_core::pipeline::{self, bc6h, hdr};
use blockloom_protocol::RuntimeMessage;
use std::path::{Path, PathBuf};

pub fn register(app: &mut App) {
    app.init_resource::<SkyState>()
        .add_systems(Update, (load_sky, apply_sky).chain());
}

/// What a loaded cube was made from; a change starts a new load.
#[derive(Clone, Debug, PartialEq)]
struct SkyKey {
    dir: PathBuf,
    path: String,
    bias: f32,
}

#[derive(Resource, Default)]
pub struct SkyState {
    key: Option<SkyKey>,
    task: Option<Task<Result<Image, String>>>,
    image: Option<Handle<Image>>,
}

impl SkyKey {
    fn of(engine: &Engine) -> Option<SkyKey> {
        let dir = engine.project_dir.clone()?;
        let path = blockloom_core::assets::normalize(&engine.project.world.lighting.sky)?;
        let bias = pipeline::load_manifest(&dir).bias_of(&path);
        Some(SkyKey { dir, path, bias })
    }
}

/// Starts a load when a rebuild's camera finds the sky changed, and picks
/// the cube up when its task lands.
fn load_sky(
    engine: NonSend<Engine>,
    mut state: ResMut<SkyState>,
    mut images: ResMut<Assets<Image>>,
    device: Option<Res<RenderDevice>>,
    fresh: Query<(), Added<WorldCamera>>,
) {
    if !fresh.is_empty() {
        let key = SkyKey::of(&engine);
        if key != state.key {
            state.image = None;
            state.task = key.clone().map(|key| {
                let compressed = device.as_ref().is_some_and(|device| {
                    device
                        .features()
                        .contains(WgpuFeatures::TEXTURE_COMPRESSION_BC)
                });
                AsyncComputeTaskPool::get().spawn(async move { load(&key, compressed) })
            });
            state.key = key;
        }
    }
    let Some(task) = state.task.as_mut() else {
        return;
    };
    let Some(result) = check_ready(task) else {
        return;
    };
    state.task = None;
    match result {
        Ok(image) => state.image = Some(images.add(image)),
        Err(error) => crate::bridge::send(&RuntimeMessage::Error {
            actor: "Blockloom".into(),
            message: format!("The sky didn't load: {error}"),
        }),
    }
}

/// Keeps the world camera's skybox and environment light matching the sky.
fn apply_sky(
    engine: NonSend<Engine>,
    state: Res<SkyState>,
    mut commands: Commands,
    cameras: Query<(Entity, Option<&Skybox>), With<WorldCamera>>,
) {
    let brightness = engine.project.world.lighting.sky_brightness;
    for (camera, skybox) in &cameras {
        let Some(image) = state.image.clone() else {
            if skybox.is_some() {
                commands
                    .entity(camera)
                    .remove::<(Skybox, GeneratedEnvironmentMapLight)>();
            }
            continue;
        };
        let current = skybox.is_some_and(|skybox| {
            skybox.image.as_ref() == Some(&image) && skybox.brightness == brightness
        });
        if current {
            continue;
        }
        commands.entity(camera).insert((
            Skybox {
                image: Some(image.clone()),
                brightness,
                ..default()
            },
            GeneratedEnvironmentMapLight {
                environment_map: image,
                intensity: brightness,
                ..default()
            },
        ));
    }
}

/// The baked BC6H cube where a build left one, else the source file.
fn load(key: &SkyKey, compressed: bool) -> Result<Image, String> {
    let baked = key.dir.join(pipeline::baked_sky_path(&key.path));
    if let Ok(bytes) = std::fs::read(&baked) {
        let (size, blocks) = bc6h::read_dds_cube(&bytes)?;
        if compressed {
            return Ok(cube_image(
                size,
                blocks.to_vec(),
                TextureFormat::Bc6hRgbUfloat,
            ));
        }
        let face = blocks.len() / 6;
        let mut texels = Vec::with_capacity((size * size * 8 * 6) as usize);
        for blocks in blocks.chunks_exact(face) {
            texels.extend(bc6h::decode_face(blocks, size)?);
        }
        return Ok(cube_image(size, texels, TextureFormat::Rgba16Float));
    }
    source_cube(&key.dir, &key.path, key.bias)
}

fn source_cube(dir: &Path, path: &str, bias: f32) -> Result<Image, String> {
    let mut image = hdr::load_hdr(dir, path)?;
    image.bias(bias);
    let cube = hdr::HdrCube::from_image(&image, hdr::HdrCube::MAX_FACE);
    let one = half::f16::ONE.to_le_bytes();
    let mut texels = Vec::with_capacity((cube.size * cube.size * 8 * 6) as usize);
    for texel in cube.faces.iter().flatten() {
        for channel in texel {
            texels.extend_from_slice(&half::f16::from_f32(*channel).to_le_bytes());
        }
        texels.extend_from_slice(&one);
    }
    Ok(cube_image(cube.size, texels, TextureFormat::Rgba16Float))
}

fn cube_image(size: u32, data: Vec<u8>, format: TextureFormat) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        data,
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_panorama(dir: &Path) {
        let (width, height) = (64u32, 32u32);
        let pixels: Vec<image::Rgb<f32>> = (0..width * height)
            .map(|i| image::Rgb([1.0 + (i % width) as f32 / 8.0, 2.0, 0.5]))
            .collect();
        let file = std::fs::File::create(dir.join("assets/sky.hdr")).unwrap();
        image::codecs::hdr::HdrEncoder::new(file)
            .encode(&pixels, width as usize, height as usize)
            .unwrap();
    }

    fn project() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "blockloom-sky-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        write_panorama(&dir);
        dir
    }

    #[test]
    fn a_source_panorama_becomes_an_fp16_cube() {
        let dir = project();
        let key = SkyKey {
            dir: dir.clone(),
            path: "assets/sky.hdr".into(),
            bias: 1.0,
        };
        let image = load(&key, true).unwrap();
        assert_eq!(image.texture_descriptor.format, TextureFormat::Rgba16Float);
        assert_eq!(image.texture_descriptor.size.depth_or_array_layers, 6);
        let size = image.texture_descriptor.size.width;
        assert!(size.is_power_of_two());
        // The bias doubled a green channel of 2.
        let data = image.data.as_ref().unwrap();
        let green = half::f16::from_le_bytes([data[2], data[3]]).to_f32();
        assert_eq!(green, 4.0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_baked_cube_loads_compressed_or_decoded() {
        let dir = project();
        let image = hdr::load_hdr(&dir, "assets/sky.hdr").unwrap();
        let cube = hdr::HdrCube::from_image(&image, 16);
        let baked = dir.join(pipeline::baked_sky_path("assets/sky.hdr"));
        std::fs::create_dir_all(baked.parent().unwrap()).unwrap();
        std::fs::write(&baked, bc6h::write_dds_cube(&cube)).unwrap();
        let key = SkyKey {
            dir: dir.clone(),
            path: "assets/sky.hdr".into(),
            bias: 0.0,
        };

        let compressed = load(&key, true).unwrap();
        assert_eq!(
            compressed.texture_descriptor.format,
            TextureFormat::Bc6hRgbUfloat
        );
        assert_eq!(compressed.data.as_ref().unwrap().len(), 16 * 16 * 6);

        let decoded = load(&key, false).unwrap();
        assert_eq!(
            decoded.texture_descriptor.format,
            TextureFormat::Rgba16Float
        );
        assert_eq!(decoded.data.as_ref().unwrap().len(), 16 * 16 * 8 * 6);
        std::fs::remove_dir_all(dir).ok();
    }
}
