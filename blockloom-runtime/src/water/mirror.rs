//! Planar reflections: for a body in `ReflectionMode::Planar`, a second
//! camera stands where the world camera would be mirrored under the surface
//! and renders the world into a texture the surface samples. Its projection
//! is oblique, so its near plane is the water plane and nothing under the
//! surface gets in the way. It is turned upside down rather than mirrored,
//! which keeps every triangle's winding: the surface reads it with v flipped.

use crate::environment::Environment;
use crate::probes::CaptureHides;
use crate::world::WorldCamera;
use bevy::camera::primitives::Frustum;
use bevy::camera::{CameraProjection, Hdr, RenderTarget, SubCameraView};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::math::Vec3A;
use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use std::collections::HashMap;

/// A mirror's texture format: scene-referred, like the world camera's.
const MIRROR_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

/// How far under the rest level the clip plane sits, so troughs still find
/// something to reflect.
const CLIP_BELOW: f32 = 0.05;

/// A perspective projection whose near plane is replaced by a world plane,
/// in view space (Lengyel's oblique clipping, for reverse z).
#[derive(Debug, Clone)]
pub struct MirrorProjection {
    pub base: PerspectiveProjection,
    /// Kept where `plane . (view position, 1) >= 0`. Unit normal.
    pub plane: Vec4,
}

/// Replaces the near row of a reverse-z perspective matrix so clip z runs
/// from the plane (1) outwards. Half the plane keeps the far end from
/// clipping anything inside a sane field of view.
pub fn oblique(clip_from_view: Mat4, plane: Vec4) -> Mat4 {
    let rows = [0, 1, 2, 3].map(|i| clip_from_view.row(i));
    let near = rows[3] - plane * 0.5;
    Mat4::from_cols(rows[0], rows[1], near, rows[3]).transpose()
}

impl CameraProjection for MirrorProjection {
    fn get_clip_from_view(&self) -> Mat4 {
        oblique(self.base.get_clip_from_view(), self.plane)
    }

    fn get_clip_from_view_for_sub(&self, sub_view: &SubCameraView) -> Mat4 {
        oblique(self.base.get_clip_from_view_for_sub(sub_view), self.plane)
    }

    fn update(&mut self, width: f32, height: f32) {
        self.base.update(width, height);
    }

    fn far(&self) -> f32 {
        self.base.far
    }

    fn get_frustum_corners(&self, z_near: f32, z_far: f32) -> [Vec3A; 8] {
        self.base.get_frustum_corners(z_near, z_far)
    }

    // Culls with the plain frustum: a little more than it needs.
    fn compute_frustum(&self, camera_transform: &GlobalTransform) -> Frustum {
        self.base.compute_frustum(camera_transform)
    }
}

/// Where a camera would be seen in a mirror at height `level`, turned
/// upside down so its basis stays right-handed.
pub fn mirrored(camera: &Transform, level: f32) -> Transform {
    let flip = |v: Vec3| Vec3::new(v.x, -v.y, v.z);
    let right = flip(camera.rotation * Vec3::X);
    let up = -flip(camera.rotation * Vec3::Y);
    let back = flip(camera.rotation * Vec3::Z);
    let mut out = *camera;
    out.translation = Vec3::new(
        camera.translation.x,
        2.0 * level - camera.translation.y,
        camera.translation.z,
    );
    out.rotation = Quat::from_mat3(&Mat3::from_cols(right, up, back)).normalize();
    out
}

/// The world plane `y = level`, facing up, in `camera`'s view space.
pub fn view_plane(camera: &Transform, level: f32) -> Vec4 {
    let world = Vec4::new(0.0, 1.0, 0.0, -level);
    camera.to_matrix().transpose() * world
}

/// Every body's mirror, by actor id.
#[derive(Resource, Default)]
pub struct Mirrors(HashMap<String, Mirror>);

struct Mirror {
    camera: Entity,
    image: Handle<Image>,
    size: UVec2,
}

impl Mirrors {
    /// The texture a body's surface reads, while its mirror is rendering.
    pub fn image(&self, id: &str) -> Option<Handle<Image>> {
        self.0.get(id).map(|mirror| mirror.image.clone())
    }
}

fn mirror_image(size: UVec2) -> Image {
    let mut image = Image::new_target_texture(size.x, size.y, MIRROR_FORMAT, None);
    image.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING;
    image.texture_descriptor.label = Some("water_mirror");
    image
}

/// What one body wants of its mirror this frame.
pub struct Want {
    pub id: String,
    pub level: f32,
    pub scale: f32,
    /// The surface entity, which the mirror leaves out.
    pub hide: Entity,
}

/// Brings every wanted mirror up to date and removes the rest. Mirrors
/// stand down while the camera is under a surface or not perspective.
#[allow(clippy::type_complexity)]
pub fn sync_mirrors(
    commands: &mut Commands,
    mirrors: &mut Mirrors,
    wants: Vec<Want>,
    world: Option<(&Camera, &Transform, &Projection, bool)>,
    environment: &Environment,
    images: &mut Assets<Image>,
    cameras: &mut Query<
        (
            &mut Camera,
            &mut Transform,
            &mut Projection,
            &mut bevy::camera::Exposure,
        ),
        Without<WorldCamera>,
    >,
) {
    let keep: Vec<&str> = wants.iter().map(|want| want.id.as_str()).collect();
    mirrors.0.retain(|id, mirror| {
        let kept = keep.contains(&id.as_str());
        if !kept {
            commands.entity(mirror.camera).despawn();
        }
        kept
    });
    let Some((camera, pose, Projection::Perspective(base), sky)) = world else {
        for mirror in mirrors.0.values() {
            if let Ok((mut camera, ..)) = cameras.get_mut(mirror.camera) {
                camera.is_active = false;
            }
        }
        return;
    };
    let viewport = camera
        .physical_viewport_size()
        .unwrap_or(UVec2::new(640, 360));
    for want in wants {
        let size = (viewport.as_vec2() * want.scale)
            .round()
            .as_uvec2()
            .max(UVec2::ONE);
        let above = pose.translation.y > want.level;
        let mirror = mirrors.0.entry(want.id.clone()).or_insert_with(|| {
            let image = images.add(mirror_image(size));
            let mut spawned = commands.spawn((
                Camera3d::default(),
                Camera {
                    // Before the world camera, which samples it.
                    order: -2,
                    ..default()
                },
                RenderTarget::from(image.clone()),
                Hdr,
                // Scene-referred: the surface tonemaps it with everything else.
                Tonemapping::None,
                Msaa::Off,
                bevy::camera::Exposure {
                    ev100: environment.exposure,
                },
                Projection::custom(MirrorProjection {
                    base: base.clone(),
                    plane: Vec4::Y,
                }),
                Transform::default(),
                CaptureHides(want.hide),
                Name::new(format!("water mirror {}", want.id)),
            ));
            if sky {
                spawned.insert(crate::sky::SkyView { disks: true });
            }
            Mirror {
                camera: spawned.id(),
                image,
                size,
            }
        });
        if mirror.size != size {
            mirror.size = size;
            mirror.image = images.add(mirror_image(size));
            commands
                .entity(mirror.camera)
                .insert(RenderTarget::from(mirror.image.clone()));
        }
        commands
            .entity(mirror.camera)
            .insert(CaptureHides(want.hide));
        let Ok((mut camera, mut transform, mut projection, mut exposure)) =
            cameras.get_mut(mirror.camera)
        else {
            continue;
        };
        camera.is_active = above;
        if !above {
            continue;
        }
        let level = want.level - CLIP_BELOW;
        *transform = mirrored(pose, want.level);
        exposure.ev100 = environment.exposure;
        let plane = view_plane(&transform, level);
        if let Projection::Custom(custom) = &mut *projection
            && let Some(mirror) = custom.get_mut::<MirrorProjection>()
        {
            mirror.base.fov = base.fov;
            mirror.base.near = base.near;
            mirror.base.far = base.far;
            mirror.plane = plane;
        }
    }
}

/// Whether a body's mirror has something to show: it exists and the camera
/// is above its surface.
pub fn showing(mirrors: &Mirrors, id: &str, eye: Vec3, level: f32) -> bool {
    mirrors.0.contains_key(id) && eye.y > level
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mirror_sees_the_reflection_flipped_top_to_bottom() {
        let camera =
            Transform::from_xyz(3.0, 6.0, 10.0).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y);
        let mirror = mirrored(&camera, 1.0);
        assert!((mirror.translation - Vec3::new(3.0, -4.0, 10.0)).length() < 1e-5);
        // A point above the water, through the mirror camera, lands where
        // its reflection does through the real one, upside down.
        let point = Vec3::new(-2.0, 4.0, -5.0);
        let reflection = Vec3::new(point.x, 2.0 - point.y, point.z);
        let seen = |pose: &Transform, p: Vec3| {
            let v = pose.to_matrix().inverse().transform_point3(p);
            Vec2::new(v.x, v.y) / -v.z
        };
        let real = seen(&camera, reflection);
        let flipped = seen(&mirror, point);
        assert!((real.x - flipped.x).abs() < 1e-4 && (real.y + flipped.y).abs() < 1e-4);
        assert!((mirror.right().dot(*camera.right()) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn the_oblique_near_plane_is_the_water() {
        let camera = mirrored(
            &Transform::from_xyz(0.0, 5.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
            0.0,
        );
        let mut base = PerspectiveProjection::default();
        base.update(16.0, 9.0);
        let plane = view_plane(&camera, 0.0);
        let clip_from_world =
            oblique(base.get_clip_from_view(), plane) * camera.to_matrix().inverse();
        let depth = |p: Vec3| {
            let c = clip_from_world * p.extend(1.0);
            c.z / c.w
        };
        let eye = camera.translation;
        let ray = (Vec3::new(0.5, 0.0, -1.0) - eye).normalize();
        let along = |y: f32| eye + ray * ((y - eye.y) / ray.y);
        // Where the ray crosses the water: the near plane (reverse z 1).
        assert!((depth(along(0.0)) - 1.0).abs() < 1e-4);
        // Past it: in range, and further is smaller.
        let near = depth(along(1.0));
        let far = depth(along(20.0));
        assert!(near < 1.0 && far > 0.0 && near > far, "{near} {far}");
        // Short of it, under the water: clipped.
        assert!(depth(along(-1.0)) > 1.0);
    }
}
