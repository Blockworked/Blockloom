//! Probe capture as a service: six 90° cameras at a point render the world
//! into FP16 faces, and whoever asked gets the faces and, when they asked for
//! a readback, the assembled cubemap. HDRI baking, reflection probes and water
//! reflections are all requests to this; none of them owns a camera rig.
//!
//! 3D only. A one-shot capture tears its cameras down once delivered; a
//! refreshing one keeps them and re-renders every `n` frames into the same
//! faces, so the handles a consumer holds stay good.

use crate::environment::Environment;
use bevy::asset::RenderAssetUsages;
use bevy::camera::{Hdr, RenderTarget};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::gpu_readback::{ReadbackComplete, ReadbackOnce};
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureUsages, TextureViewDescriptor,
    TextureViewDimension,
};
use std::collections::HashMap;

pub fn register(app: &mut App) {
    app.init_resource::<ProbeService>()
        .add_message::<ProbeCaptured>();
}

/// Frames a face renders before it is read or announced, so shadows and
/// anything that settles over a frame are in.
const SETTLE_FRAMES: u32 = 2;
/// Every face renders HDR and linear: a sky can be brighter than 1.
const FACE_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
const FACE_TEXEL_BYTES: u32 = 8;

/// Who a capture is for. Decides nothing here beyond the defaults below; it
/// rides along so one message stream serves every consumer.
// The Phase 5 sky, reflection and water passes are the callers.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProbeUse {
    /// Baked to disk as a sky or IBL source.
    Hdri,
    /// A reflection probe's cubemap.
    Reflection,
    /// A cubemap at the water surface, refreshed as the scene moves.
    Water,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeRefresh {
    /// Render, deliver, tear down.
    Once,
    /// Keep rendering into the same faces every this many frames.
    Every(u32),
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub struct ProbeRequest {
    pub position: Vec3,
    /// Texels per face side. Rounded up to a multiple of 32 so readback rows
    /// need no padding.
    pub resolution: u32,
    pub near: f32,
    pub far: f32,
    pub purpose: ProbeUse,
    pub refresh: ProbeRefresh,
    /// Copy the faces back to the CPU and assemble a cubemap image.
    pub readback: bool,
}

#[allow(dead_code)]
impl ProbeRequest {
    fn new(purpose: ProbeUse, position: Vec3, resolution: u32) -> Self {
        Self {
            position,
            resolution,
            near: 0.1,
            far: 1000.0,
            purpose,
            refresh: ProbeRefresh::Once,
            readback: false,
        }
    }

    /// A one-shot capture with the cube read back, for writing to disk.
    pub fn hdri(position: Vec3, resolution: u32) -> Self {
        Self {
            readback: true,
            ..Self::new(ProbeUse::Hdri, position, resolution)
        }
    }

    /// A baked reflection probe: captured once, assembled into a cube.
    pub fn reflection(position: Vec3, resolution: u32) -> Self {
        Self {
            readback: true,
            ..Self::new(ProbeUse::Reflection, position, resolution)
        }
    }

    /// A live cube at the water surface. GPU faces only, refreshed every
    /// `every` frames.
    pub fn water(position: Vec3, resolution: u32, every: u32) -> Self {
        Self {
            refresh: ProbeRefresh::Every(every.max(1)),
            ..Self::new(ProbeUse::Water, position, resolution)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProbeId(u32);

/// A capture is ready. Sent once per request; a refreshing capture keeps
/// drawing into the same `faces` after that.
#[allow(dead_code)]
#[derive(Message, Clone, Debug)]
pub struct ProbeCaptured {
    pub id: ProbeId,
    pub purpose: ProbeUse,
    /// +X, -X, +Y, -Y, +Z, -Z in cubemap order (see [`FACES`]).
    pub faces: [Handle<Image>; 6],
    /// The faces as one cube image, when the request asked for a readback.
    pub cube: Option<Handle<Image>>,
    /// The exposure the faces were rendered at, so a consumer lighting with
    /// them can undo it.
    pub ev100: f32,
}

/// Every capture in flight, and the queue of ones asked for.
#[derive(Resource, Default)]
pub struct ProbeService {
    next: u32,
    queued: Vec<(ProbeId, ProbeRequest)>,
    live: HashMap<ProbeId, Capture>,
    cancelled: Vec<ProbeId>,
}

#[allow(dead_code)]
impl ProbeService {
    /// Queues a capture; it starts next frame.
    pub fn request(&mut self, request: ProbeRequest) -> ProbeId {
        self.next += 1;
        let id = ProbeId(self.next);
        self.queued.push((id, request));
        id
    }

    /// Stops a capture, refreshing or not, and drops its cameras.
    pub fn cancel(&mut self, id: ProbeId) {
        self.queued.retain(|(queued, _)| *queued != id);
        self.cancelled.push(id);
    }

    /// The faces a capture renders into, once it has started.
    pub fn faces(&self, id: ProbeId) -> Option<&[Handle<Image>; 6]> {
        self.live.get(&id).map(|capture| &capture.faces)
    }

    pub fn in_flight(&self) -> usize {
        self.queued.len() + self.live.len()
    }

    fn deliver(&mut self, id: ProbeId, face: usize, data: Vec<u8>) {
        if let Some(capture) = self.live.get_mut(&id)
            && let Stage::Reading(read) = &mut capture.stage
        {
            read[face] = Some(data);
        }
    }
}

struct Capture {
    request: ProbeRequest,
    faces: [Handle<Image>; 6],
    cameras: [Entity; 6],
    stage: Stage,
    /// Frames since the cameras last rendered a refresh.
    frames: u32,
    announced: bool,
}

enum Stage {
    Rendering,
    Reading([Option<Vec<u8>>; 6]),
    Done,
}

/// Where each face camera looks and which way is up, in cubemap order. Bevy
/// samples cubes with z negated (they are left-handed), so the cube's +Z face
/// looks down world -Z.
pub const FACES: [(Vec3, Vec3); 6] = [
    (Vec3::X, Vec3::Y),
    (Vec3::NEG_X, Vec3::Y),
    (Vec3::Y, Vec3::Z),
    (Vec3::NEG_Y, Vec3::NEG_Z),
    (Vec3::NEG_Z, Vec3::Y),
    (Vec3::Z, Vec3::Y),
];

fn face_resolution(requested: u32) -> u32 {
    requested.clamp(32, 4096).next_multiple_of(32)
}

fn face_image(resolution: u32) -> Image {
    let mut image = Image::new_target_texture(resolution, resolution, FACE_FORMAT, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    image.texture_descriptor.label = Some("probe_face");
    image
}

/// Starts queued captures, ticks live ones, tears down what is finished.
pub fn run_captures(
    mut commands: Commands,
    mut service: ResMut<ProbeService>,
    mut images: ResMut<Assets<Image>>,
    mut cameras: Query<&mut Camera>,
    environment: Res<Environment>,
    mut captured: MessageWriter<ProbeCaptured>,
) {
    let service = &mut *service;
    for id in std::mem::take(&mut service.cancelled) {
        if let Some(capture) = service.live.remove(&id) {
            despawn(&mut commands, &capture.cameras);
        }
    }
    for (id, request) in std::mem::take(&mut service.queued) {
        let resolution = face_resolution(request.resolution);
        let faces = std::array::from_fn(|_| images.add(face_image(resolution)));
        let cameras = std::array::from_fn(|face| {
            let (forward, up) = FACES[face];
            commands
                .spawn((
                    Camera3d::default(),
                    Camera {
                        // Before the world camera, so a frame's faces are
                        // ready for anything that samples them this frame.
                        order: -10 - face as isize,
                        ..default()
                    },
                    RenderTarget::from(faces[face].clone()),
                    Hdr,
                    // Scene-referred: tonemapping belongs to whoever shows it.
                    Tonemapping::None,
                    bevy::camera::Exposure {
                        ev100: environment.exposure,
                    },
                    Projection::Perspective(PerspectiveProjection {
                        fov: std::f32::consts::FRAC_PI_2,
                        aspect_ratio: 1.0,
                        near: request.near,
                        far: request.far,
                        ..default()
                    }),
                    Transform::from_translation(request.position).looking_to(forward, up),
                    Name::new(format!("probe {} face {face}", id.0)),
                ))
                .id()
        });
        service.live.insert(
            id,
            Capture {
                request,
                faces,
                cameras,
                stage: Stage::Rendering,
                frames: 0,
                announced: false,
            },
        );
    }

    let mut finished = Vec::new();
    for (&id, capture) in service.live.iter_mut() {
        capture.frames += 1;
        match &capture.stage {
            Stage::Rendering if capture.frames >= SETTLE_FRAMES => {
                if capture.request.readback {
                    for (face, handle) in capture.faces.iter().enumerate() {
                        commands
                            .spawn(ReadbackOnce::texture(handle.clone()))
                            .observe(
                            move |event: On<ReadbackComplete>,
                                  mut service: ResMut<ProbeService>| {
                                service.deliver(id, face, event.data.clone());
                            },
                        );
                    }
                    capture.stage = Stage::Reading(Default::default());
                } else {
                    capture.stage = Stage::Done;
                }
            }
            Stage::Reading(read) if read.iter().all(Option::is_some) => {
                let resolution = face_resolution(capture.request.resolution);
                let faces: Vec<&[u8]> = read.iter().flatten().map(Vec::as_slice).collect();
                let cube = images.add(assemble_cube(resolution, &faces));
                captured.write(ProbeCaptured {
                    id,
                    purpose: capture.request.purpose,
                    faces: capture.faces.clone(),
                    cube: Some(cube),
                    ev100: environment.exposure,
                });
                capture.announced = true;
                capture.stage = Stage::Done;
            }
            _ => {}
        }
        if !matches!(capture.stage, Stage::Done) {
            continue;
        }
        if !capture.announced {
            captured.write(ProbeCaptured {
                id,
                purpose: capture.request.purpose,
                faces: capture.faces.clone(),
                cube: None,
                ev100: environment.exposure,
            });
            capture.announced = true;
        }
        match capture.request.refresh {
            ProbeRefresh::Once => finished.push(id),
            ProbeRefresh::Every(every) => {
                // Render on refresh frames only; the faces hold in between.
                let due = capture.frames % every.max(1) == 0;
                for camera in capture.cameras {
                    if let Ok(mut camera) = cameras.get_mut(camera) {
                        camera.is_active = due;
                    }
                }
            }
        }
    }
    for id in finished {
        if let Some(capture) = service.live.remove(&id) {
            despawn(&mut commands, &capture.cameras);
        }
    }
}

fn despawn(commands: &mut Commands, cameras: &[Entity; 6]) {
    for camera in cameras {
        commands.entity(*camera).try_despawn();
    }
}

/// Strips readback row padding: wgpu copies rows 256-byte aligned.
pub(crate) fn unpad_rows(data: &[u8], width: u32, height: u32) -> Vec<u8> {
    let row = (width * FACE_TEXEL_BYTES) as usize;
    let padded = row.next_multiple_of(256);
    if padded == row {
        return data[..row * height as usize].to_vec();
    }
    data.chunks(padded)
        .take(height as usize)
        .flat_map(|chunk| &chunk[..row])
        .copied()
        .collect()
}

/// Six read-back faces as one cube image, layers in cubemap order.
fn assemble_cube(resolution: u32, faces: &[&[u8]]) -> Image {
    let mut data = Vec::with_capacity((resolution * resolution * FACE_TEXEL_BYTES * 6) as usize);
    for face in faces {
        data.extend(unpad_rows(face, resolution, resolution));
    }
    let mut cube = Image::new(
        Extent3d {
            width: resolution,
            height: resolution,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        data,
        FACE_FORMAT,
        RenderAssetUsages::all(),
    );
    cube.texture_descriptor.label = Some("probe_cube");
    cube.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    cube
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut app = App::new();
        app.init_resource::<Assets<Image>>()
            .init_resource::<Environment>();
        register(&mut app);
        app.add_systems(Update, run_captures);
        app
    }

    fn captured(app: &mut App) -> Vec<ProbeCaptured> {
        app.world_mut()
            .resource_mut::<Messages<ProbeCaptured>>()
            .drain()
            .collect()
    }

    fn probe_cameras(app: &mut App) -> usize {
        app.world_mut()
            .query_filtered::<(), With<Camera3d>>()
            .iter(app.world())
            .count()
    }

    #[test]
    fn faces_match_the_left_handed_cube_axes() {
        // The cube face's u axis, in world space, after Bevy's z flip.
        let u_axes = [Vec3::Z, Vec3::NEG_Z, Vec3::X, Vec3::X, Vec3::X, Vec3::NEG_X];
        for (face, (forward, up)) in FACES.iter().enumerate() {
            let transform = Transform::IDENTITY.looking_to(*forward, *up);
            assert!(
                transform.right().abs_diff_eq(u_axes[face], 1e-6),
                "face {face} has right {:?}",
                transform.right()
            );
        }
    }

    #[test]
    fn a_one_shot_capture_announces_once_and_leaves() {
        let mut app = app();
        let id = app
            .world_mut()
            .resource_mut::<ProbeService>()
            .request(ProbeRequest::new(ProbeUse::Water, Vec3::ZERO, 50));
        app.update();
        assert_eq!(probe_cameras(&mut app), 6);
        let faces = app
            .world()
            .resource::<ProbeService>()
            .faces(id)
            .unwrap()
            .clone();
        let size = app
            .world()
            .resource::<Assets<Image>>()
            .get(&faces[0])
            .unwrap()
            .size();
        assert_eq!(size, UVec2::splat(64));

        app.update();
        let messages = captured(&mut app);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, id);
        assert!(messages[0].cube.is_none());
        app.update();
        assert_eq!(probe_cameras(&mut app), 0);
        assert_eq!(app.world().resource::<ProbeService>().in_flight(), 0);
    }

    #[test]
    fn a_refreshing_capture_keeps_its_cameras_until_cancelled() {
        let mut app = app();
        let id = app
            .world_mut()
            .resource_mut::<ProbeService>()
            .request(ProbeRequest::water(Vec3::Y, 32, 3));
        app.update();
        app.update();
        assert_eq!(captured(&mut app).len(), 1);
        for _ in 0..4 {
            app.update();
        }
        assert!(captured(&mut app).is_empty());
        assert_eq!(probe_cameras(&mut app), 6);
        app.world_mut().resource_mut::<ProbeService>().cancel(id);
        app.update();
        app.update();
        assert_eq!(probe_cameras(&mut app), 0);
    }

    #[test]
    fn read_back_faces_assemble_into_a_cube() {
        let mut app = app();
        let id = app
            .world_mut()
            .resource_mut::<ProbeService>()
            .request(ProbeRequest::reflection(Vec3::ZERO, 32));
        app.update();
        app.update();
        // No GPU here: hand the service what the readback observers would.
        let face_bytes = (32 * 32 * FACE_TEXEL_BYTES) as usize;
        for face in 0..6 {
            app.world_mut().resource_mut::<ProbeService>().deliver(
                id,
                face,
                vec![face as u8; face_bytes],
            );
        }
        app.update();
        let messages = captured(&mut app);
        let cube = messages[0].cube.clone().unwrap();
        let images = app.world().resource::<Assets<Image>>();
        let cube = images.get(&cube).unwrap();
        assert_eq!(cube.texture_descriptor.size.depth_or_array_layers, 6);
        let data = cube.data.as_ref().unwrap();
        assert_eq!(data[face_bytes * 5], 5);
    }

    #[test]
    fn padded_rows_lose_their_padding() {
        // 4 texels of 8 bytes = 32 bytes, padded to 256.
        let mut data = vec![0u8; 256 * 2];
        data[0..32].fill(1);
        data[256..288].fill(2);
        let rows = unpad_rows(&data, 4, 2);
        assert_eq!(rows.len(), 64);
        assert!(rows[..32].iter().all(|b| *b == 1));
        assert!(rows[32..].iter().all(|b| *b == 2));
    }
}
