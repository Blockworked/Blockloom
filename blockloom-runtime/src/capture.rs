//! EXR screenshots: a second camera renders the world camera's view, linear
//! and untonemapped, into an FP16 image that is read back and written as
//! OpenEXR off the main thread. The HUD is left out, as it is display-referred.

use crate::bridge;
use crate::environment::Environment;
use crate::world::WorldCamera;
use bevy::camera::{Hdr, RenderTarget};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::gpu_readback::{ReadbackComplete, ReadbackOnce};
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::tasks::IoTaskPool;
use blockloom_protocol::RuntimeMessage;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Frames the capture camera renders before its image is read.
const SETTLE_FRAMES: u32 = 3;
/// The same under realtime ray tracing.
const TRACED_SETTLE_FRAMES: u32 = 30;
/// Frames a request waits for a world camera with a size, as one asked for
/// while the world is still coming up.
const CAMERA_WAIT_FRAMES: u32 = 120;

pub fn register(app: &mut App) {
    app.init_resource::<ExrCaptures>()
        .add_systems(PostUpdate, run_exr_captures);
}

/// Paths the editor asked for, and the one capture in flight.
#[derive(Resource, Default)]
pub struct ExrCaptures {
    queued: Vec<PathBuf>,
    live: Option<Live>,
    waited: u32,
}

impl ExrCaptures {
    pub fn request(&mut self, path: impl Into<PathBuf>) {
        self.queued.push(path.into());
    }
}

struct Live {
    path: PathBuf,
    camera: Entity,
    size: UVec2,
    frames: u32,
    /// Frames to render before reading: more while tracing, so the image
    /// has converged.
    settle: u32,
    /// Seconds a path traced capture may take, 0 for no limit.
    deadline: f32,
    elapsed: f32,
    /// Filled by the readback observer.
    pixels: Option<Arc<Mutex<Option<Vec<u8>>>>>,
}

/// What the capture camera copies off the world camera.
type CapturedView = (
    Entity,
    &'static Camera,
    &'static Projection,
    &'static GlobalTransform,
    Has<Camera3d>,
    Option<&'static crate::sky::SkyView>,
    Option<&'static bevy::light::EnvironmentMapLight>,
    Option<&'static Bloom>,
);

fn run_exr_captures(
    mut commands: Commands,
    mut captures: ResMut<ExrCaptures>,
    mut images: ResMut<Assets<Image>>,
    environment: Res<Environment>,
    tracing: Option<Res<crate::ray_tracing::RayTracingState>>,
    editor: Option<Res<crate::edit::SceneEditor>>,
    time: Res<Time<Real>>,
    world_cameras: Query<CapturedView, With<WorldCamera>>,
) {
    let captures = &mut *captures;
    if captures.live.is_none() && !captures.queued.is_empty() {
        let ready = world_cameras
            .iter()
            .filter(|(_, camera, ..)| camera.is_active)
            .find_map(|found| {
                let size = found.1.physical_target_size()?;
                (size.min_element() > 0).then_some((found, size))
            });
        let Some(((source, camera, projection, transform, is_3d, sky, sky_light, bloom), size)) =
            ready
        else {
            captures.waited += 1;
            if captures.waited > CAMERA_WAIT_FRAMES {
                captures.waited = 0;
                fail(
                    &captures.queued.remove(0),
                    "no world camera to capture from",
                );
            }
            return;
        };
        captures.waited = 0;
        let path = captures.queued.remove(0);
        let mut image = Image::new_target_texture(size.x, size.y, TextureFormat::Rgba16Float, None);
        image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
        image.texture_descriptor.label = Some("capture_exr");
        let target = images.add(image);
        let mut entity = commands.spawn((
            Camera {
                // Ahead of the world camera, so the frame it captures is the
                // one the world camera draws next.
                order: -20,
                clear_color: camera.clear_color,
                ..default()
            },
            RenderTarget::from(target.clone()),
            projection.clone(),
            transform.compute_transform(),
            Hdr,
            Tonemapping::None,
            bevy::camera::Exposure {
                ev100: environment.exposure,
            },
            CaptureCamera(target),
            Name::new("exr capture"),
        ));
        if is_3d {
            entity.insert(Camera3d::default());
            if let Some(sky) = sky {
                entity.insert(*sky);
            }
            if let Some(light) = sky_light {
                entity.insert(light.clone());
            }
        } else {
            entity.insert(Camera2d);
        }
        if let Some(bloom) = bloom {
            entity.insert(bloom.clone());
        }
        let capture = entity.id();
        commands.queue(move |world: &mut World| {
            crate::ray_tracing::trace_like(world, source, capture);
        });
        let budget = editor
            .map(|editor| editor.view.path_tracer)
            .unwrap_or_default();
        let (settle, deadline) = match tracing.as_deref() {
            Some(state) if state.path_tracing => (budget.samples.clamp(1, 1 << 16), budget.seconds),
            // Solari's temporal reuse needs a few frames of history.
            Some(state) if state.active => (TRACED_SETTLE_FRAMES, 0.0),
            _ => (SETTLE_FRAMES, 0.0),
        };
        captures.live = Some(Live {
            path,
            camera: capture,
            size,
            frames: 0,
            settle,
            deadline,
            elapsed: 0.0,
            pixels: None,
        });
        return;
    }

    let Some(live) = captures.live.as_mut() else {
        return;
    };
    live.frames += 1;
    live.elapsed += time.delta_secs();
    let settled = live.frames >= live.settle
        || (live.deadline > 0.0 && live.elapsed >= live.deadline && live.frames >= SETTLE_FRAMES);
    match &live.pixels {
        None if settled => {
            let slot = Arc::new(Mutex::new(None));
            let sink = slot.clone();
            let camera = live.camera;
            commands.queue(move |world: &mut World| {
                let Some(target) = world.get::<CaptureCamera>(camera).map(|c| c.0.clone()) else {
                    return;
                };
                world.spawn(ReadbackOnce::texture(target)).observe(
                    move |event: On<ReadbackComplete>| {
                        if let Ok(mut slot) = sink.lock() {
                            *slot = Some(event.data.clone());
                        }
                    },
                );
            });
            live.pixels = Some(slot);
        }
        Some(slot) => {
            let Some(data) = slot.lock().ok().and_then(|mut slot| slot.take()) else {
                return;
            };
            let live = captures.live.take().expect("checked above");
            commands.entity(live.camera).despawn();
            write_exr(live.path, live.size, data);
        }
        None => {}
    }
}

#[derive(Component)]
struct CaptureCamera(Handle<Image>);

fn write_exr(path: PathBuf, size: UVec2, data: Vec<u8>) {
    IoTaskPool::get()
        .spawn(async move {
            match encode_exr(&path, size, &data) {
                Ok(()) => bridge::send(&RuntimeMessage::Say {
                    actor: "Blockloom".into(),
                    text: format!("Saved an EXR screenshot to {}", path.display()),
                }),
                Err(error) => fail(&path, &error),
            }
        })
        .detach();
}

/// Read-back FP16 rows, padded to 256 bytes, as a float EXR file.
fn encode_exr(path: &std::path::Path, size: UVec2, data: &[u8]) -> Result<(), String> {
    let pixels = crate::probes::unpad_rows(data, size.x, size.y);
    let floats: Vec<f32> = pixels
        .as_chunks::<2>()
        .0
        .iter()
        .map(|half| half::f16::from_le_bytes(*half).to_f32())
        .collect();
    let image = image::Rgba32FImage::from_raw(size.x, size.y, floats)
        .ok_or_else(|| "the captured frame came back the wrong size".to_string())?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    image::DynamicImage::ImageRgba32F(image)
        .save_with_format(path, image::ImageFormat::OpenExr)
        .map_err(|error| error.to_string())
}

fn fail(path: &std::path::Path, why: &str) {
    bridge::send(&RuntimeMessage::Error {
        actor: "Blockloom".into(),
        message: format!("Couldn't save {}: {why}", path.display()),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_of_halves_becomes_an_exr_file() {
        let dir = std::env::temp_dir().join(format!("blockloom-exr-{}", std::process::id()));
        let path = dir.join("shots/shot.exr");
        // A 2x1 frame, one texel past paper white, padded like a readback.
        let mut data = Vec::new();
        for value in [2.5f32, 0.25] {
            for channel in [value, value, value, 1.0] {
                data.extend_from_slice(&half::f16::from_f32(channel).to_le_bytes());
            }
        }
        data.resize(256, 0);
        encode_exr(&path, UVec2::new(2, 1), &data).unwrap();
        let back = image::open(&path).unwrap().into_rgba32f();
        assert_eq!(back.get_pixel(0, 0).0[0], 2.5);
        assert_eq!(back.get_pixel(1, 0).0[0], 0.25);
        std::fs::remove_dir_all(dir).ok();
    }
}
