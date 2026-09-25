//! Level of detail and occlusion, 3D only. `LodGroup` is the one selector:
//! anything with levels (primitives, later terrain chunks, trees, VFX) hands
//! it thresholds and meshes rather than picking its own. Occlusion is a
//! software Hi-Z over solid `Occluder` boxes, pruning the world camera's
//! visible list after Bevy's frustum pass, beside Bevy's own GPU occlusion.

use crate::batching::InstancedMaterial;
use crate::materials::BoxMaterial;
use crate::world::WorldCamera;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::{NoCpuCulling, VisibleEntities};
use bevy::math::Affine3A;
use bevy::prelude::*;
use bevy::render::occlusion_culling::OcclusionCulling;
use std::any::TypeId;
use std::collections::HashSet;

pub fn register(app: &mut App) {
    app.init_resource::<LodPolicy>()
        .init_resource::<OcclusionPolicy>()
        .init_resource::<Culling>();
}

/// One level: used while the screen size is at least `min_screen`. A level
/// without a mesh only reports itself, for things that thin out rather than
/// swap geometry.
#[derive(Clone, Debug)]
pub struct LodLevel {
    pub min_screen: f32,
    pub mesh: Option<Handle<Mesh>>,
}

/// Screen size is the bounding sphere's diameter over the viewport height.
/// Below the last level's `min_screen` the entity is culled from the main view.
#[derive(Component, Clone, Debug)]
pub struct LodGroup {
    levels: Vec<LodLevel>,
    radius: f32,
    current: Option<usize>,
}

impl LodGroup {
    /// `radius` bounds the level-0 geometry in local space.
    pub fn new(radius: f32) -> Self {
        Self {
            levels: Vec::new(),
            radius,
            current: Some(0),
        }
    }

    pub fn level(mut self, min_screen: f32, mesh: Option<Handle<Mesh>>) -> Self {
        self.levels.push(LodLevel { min_screen, mesh });
        self.levels
            .sort_by(|a, b| b.min_screen.total_cmp(&a.min_screen));
        self
    }

    // Read by content that hooks in; nothing in the runtime does yet.
    #[allow(dead_code)]
    pub fn levels(&self) -> &[LodLevel] {
        &self.levels
    }

    /// The level drawn now, or `None` while culled.
    pub fn current(&self) -> Option<usize> {
        self.current
    }

    pub fn swaps_meshes(&self) -> bool {
        self.levels
            .iter()
            .filter(|level| level.mesh.is_some())
            .count()
            > 1
    }
}

/// Fired on an entity whose level changed, for consumers that react to it.
#[allow(dead_code)]
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct LodChanged {
    pub entity: Entity,
    pub level: Option<usize>,
    pub previous: Option<usize>,
}

/// The numbers LOD selection works to. Phase 5 tunes these.
#[derive(Resource, Clone, Debug)]
pub struct LodPolicy {
    pub enabled: bool,
    /// Multiplies every screen size: above 1 keeps detail further out.
    pub bias: f32,
    /// Relative band around each threshold, so a level never flickers.
    pub hysteresis: f32,
}

impl Default for LodPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            bias: 1.0,
            hysteresis: 0.1,
        }
    }
}

/// A solid box in the entity's local space that hides what is behind it.
/// It must be opaque and filled: a hollow shape would cull its contents.
#[derive(Component, Clone, Copy, Debug)]
pub struct Occluder(pub Aabb);

#[derive(Resource, Clone, Debug)]
pub struct OcclusionPolicy {
    /// The CPU Hi-Z pass over `Occluder`s.
    pub software: bool,
    /// Depth buffer width in texels; height follows the aspect.
    pub width: u32,
    pub max_occluders: usize,
    /// Occluders smaller than this on screen aren't drawn into the buffer.
    pub min_occluder_screen: f32,
    /// Bevy's two-phase GPU occlusion on the world camera.
    pub gpu_occlusion: bool,
    /// Frustum culling on the GPU instead of the CPU, for scenes with many
    /// thousands of instances.
    pub gpu_frustum: bool,
}

impl Default for OcclusionPolicy {
    fn default() -> Self {
        Self {
            software: true,
            width: 256,
            max_occluders: 64,
            min_occluder_screen: 0.05,
            gpu_occlusion: true,
            gpu_frustum: false,
        }
    }
}

#[derive(Clone, Copy, Default, Debug)]
pub struct CullStats {
    pub lod_groups: usize,
    pub lod_reduced: usize,
    pub lod_culled: usize,
    pub occluders: usize,
    pub tested: usize,
    pub occluded: usize,
}

#[derive(Resource, Default)]
pub struct Culling {
    hiz: HiZ,
    pub stats: CullStats,
}

/// The screen size of a sphere of `radius` at `distance`.
fn screen_size(projection: &Projection, distance: f32, radius: f32) -> f32 {
    match projection {
        Projection::Perspective(p) => {
            if distance <= radius {
                f32::INFINITY
            } else {
                radius / (distance * (p.fov * 0.5).tan())
            }
        }
        Projection::Orthographic(o) => 2.0 * radius / o.area.height().max(f32::EPSILON),
        _ => f32::INFINITY,
    }
}

fn pick(levels: &[LodLevel], screen: f32) -> Option<usize> {
    levels.iter().position(|level| screen >= level.min_screen)
}

/// The level for `screen`, only leaving `current` once past the band.
fn next_level(
    levels: &[LodLevel],
    current: Option<usize>,
    screen: f32,
    hysteresis: f32,
) -> Option<usize> {
    let rank = |level: Option<usize>| level.unwrap_or(levels.len());
    let finer = pick(levels, screen / (1.0 + hysteresis));
    if rank(finer) < rank(current) {
        return finer;
    }
    let coarser = pick(levels, screen * (1.0 + hysteresis));
    if rank(coarser) > rank(current) {
        return coarser;
    }
    current
}

fn max_scale(transform: &GlobalTransform) -> f32 {
    let m = transform.affine().matrix3;
    m.x_axis
        .length()
        .max(m.y_axis.length())
        .max(m.z_axis.length())
}

pub fn select_lod(
    mut commands: Commands,
    policy: Res<LodPolicy>,
    mut culling: ResMut<Culling>,
    cameras: Query<(&GlobalTransform, &Projection, &Camera), With<WorldCamera>>,
    mut groups: Query<(Entity, &GlobalTransform, &mut LodGroup, Option<&mut Mesh3d>)>,
) {
    let camera = cameras.iter().find(|(.., camera)| camera.is_active);
    let mut stats = CullStats::default();
    for (entity, transform, mut group, mesh) in &mut groups {
        if group.levels.is_empty() {
            continue;
        }
        stats.lod_groups += 1;
        let level = match camera {
            Some((eye, projection, _)) if policy.enabled => {
                let distance = eye.translation().distance(transform.translation());
                let radius = group.radius * max_scale(transform);
                let screen = screen_size(projection, distance, radius) * policy.bias;
                next_level(&group.levels, group.current, screen, policy.hysteresis)
            }
            _ => Some(0),
        };
        match level {
            None => stats.lod_culled += 1,
            Some(level) if level > 0 => stats.lod_reduced += 1,
            _ => {}
        }
        if level == group.current {
            continue;
        }
        let previous = group.current;
        group.current = level;
        if let (Some(level), Some(mut mesh)) = (level, mesh)
            && let Some(handle) = &group.levels[level].mesh
            && mesh.0 != *handle
        {
            mesh.0 = handle.clone();
        }
        commands.trigger(LodChanged {
            entity,
            level,
            previous,
        });
    }
    culling.stats.lod_groups = stats.lod_groups;
    culling.stats.lod_reduced = stats.lod_reduced;
    culling.stats.lod_culled = stats.lod_culled;
}

/// Keeps the world camera's GPU culling in step with the policy.
pub fn configure_cameras(
    mut commands: Commands,
    policy: Res<OcclusionPolicy>,
    cameras: Query<(Entity, Has<OcclusionCulling>, Has<NoCpuCulling>), With<WorldCamera>>,
) {
    for (entity, occlusion, gpu_frustum) in &cameras {
        let mut camera = commands.entity(entity);
        match (policy.gpu_occlusion, occlusion) {
            (true, false) => {
                camera.insert(OcclusionCulling);
            }
            (false, true) => {
                camera.remove::<OcclusionCulling>();
            }
            _ => {}
        }
        match (policy.gpu_frustum, gpu_frustum) {
            (true, false) => {
                camera.insert(NoCpuCulling);
            }
            (false, true) => {
                camera.remove::<NoCpuCulling>();
            }
            _ => {}
        }
    }
}

/// Takes LOD-culled and occluded meshes out of the world camera's visible
/// list. Shadow views keep their own lists, so hidden casters still cast.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn cull_views(
    policy: Res<OcclusionPolicy>,
    mut culling: ResMut<Culling>,
    mut cameras: Query<
        (&GlobalTransform, &Projection, &Camera, &mut VisibleEntities),
        With<WorldCamera>,
    >,
    groups: Query<(Entity, &LodGroup)>,
    occluders: Query<(
        &Occluder,
        &GlobalTransform,
        &InheritedVisibility,
        Option<&MeshMaterial3d<StandardMaterial>>,
        Option<&MeshMaterial3d<InstancedMaterial>>,
        Option<&MeshMaterial3d<BoxMaterial>>,
    )>,
    bounds: Query<(&Aabb, &GlobalTransform)>,
    standard: Res<Assets<StandardMaterial>>,
    instanced: Res<Assets<InstancedMaterial>>,
    boxes: Res<Assets<BoxMaterial>>,
) {
    let culling = &mut *culling;
    culling.stats.occluders = 0;
    culling.stats.tested = 0;
    culling.stats.occluded = 0;
    let Some((eye, projection, _, mut visible)) = cameras
        .iter_mut()
        .find(|(_, _, camera, _)| camera.is_active)
    else {
        return;
    };
    let mut removed: HashSet<Entity> = groups
        .iter()
        .filter(|(_, group)| !group.levels.is_empty() && group.current().is_none())
        .map(|(entity, _)| entity)
        .collect();

    if policy.software
        && let Projection::Perspective(perspective) = projection
    {
        let view = eye.affine().inverse();
        let tan_y = (perspective.fov * 0.5).tan();
        let aspect = perspective.aspect_ratio.max(0.01);
        let width = policy.width.clamp(16, 2048) as usize;
        let height = ((width as f32 / aspect).round() as usize).clamp(8, 2048);
        let lens = Lens {
            view,
            tan_x: tan_y * aspect,
            tan_y,
            near: perspective.near.max(1e-3),
        };
        let opaque = |alpha: AlphaMode| alpha == AlphaMode::Opaque;
        let mut chosen: Vec<(f32, Aabb, Affine3A)> = occluders
            .iter()
            .filter(|(_, _, visible, ..)| visible.get())
            .filter(|(_, _, _, s, i, b)| {
                s.and_then(|m| standard.get(&m.0))
                    .map(|m| opaque(m.alpha_mode))
                    .or_else(|| {
                        i.and_then(|m| instanced.get(&m.0))
                            .map(|m| opaque(m.base.alpha_mode))
                    })
                    .or_else(|| {
                        b.and_then(|m| boxes.get(&m.0))
                            .map(|m| opaque(m.base.alpha_mode))
                    })
                    .unwrap_or(false)
            })
            .filter_map(|(occluder, transform, ..)| {
                let affine = transform.affine();
                let center = affine.transform_point3a(occluder.0.center);
                let radius = (affine.matrix3 * occluder.0.half_extents).length();
                let distance = center.distance(eye.translation_vec3a());
                let screen = screen_size(projection, distance, radius);
                (screen >= policy.min_occluder_screen).then_some((screen, occluder.0, affine))
            })
            .collect();
        chosen.sort_by(|a, b| b.0.total_cmp(&a.0));
        chosen.truncate(policy.max_occluders);
        if !chosen.is_empty() {
            culling.stats.occluders = chosen.len();
            let hiz = &mut culling.hiz;
            hiz.begin(width, height, lens);
            for (_, aabb, affine) in &chosen {
                hiz.draw_box(aabb, affine);
            }
            hiz.finish();
            for entity in visible.get(TypeId::of::<Mesh3d>()) {
                if removed.contains(entity) {
                    continue;
                }
                let Ok((aabb, transform)) = bounds.get(*entity) else {
                    continue;
                };
                culling.stats.tested += 1;
                if hiz.occludes(aabb, &transform.affine()) {
                    culling.stats.occluded += 1;
                    removed.insert(*entity);
                }
            }
        }
    }
    if !removed.is_empty() {
        visible
            .get_mut(TypeId::of::<Mesh3d>())
            .retain(|entity| !removed.contains(entity));
    }
}

#[derive(Clone, Copy)]
struct Lens {
    view: Affine3A,
    tan_x: f32,
    tan_y: f32,
    near: f32,
}

impl Default for Lens {
    fn default() -> Self {
        Self {
            view: Affine3A::IDENTITY,
            tan_x: 1.0,
            tan_y: 1.0,
            near: 0.1,
        }
    }
}

/// A software depth pyramid. Level 0 holds the nearest occluder depth per
/// texel (view distance, infinity where nothing is), each level above the
/// farthest of four below, so one lookup bounds a whole screen rectangle.
#[derive(Default)]
struct HiZ {
    lens: Lens,
    levels: Vec<(usize, usize, Vec<f32>)>,
    raw: Vec<f32>,
}

impl HiZ {
    fn begin(&mut self, width: usize, height: usize, lens: Lens) {
        self.lens = lens;
        self.raw.clear();
        self.raw.resize(width * height, f32::INFINITY);
        self.levels.truncate(1);
        if self.levels.is_empty() {
            self.levels.push((0, 0, Vec::new()));
        }
        self.levels[0].0 = width;
        self.levels[0].1 = height;
    }

    fn size(&self) -> (usize, usize) {
        (self.levels[0].0, self.levels[0].1)
    }

    /// View-space point to pixel coordinates and view distance.
    fn project(&self, p: Vec3) -> Vec3 {
        let (w, h) = self.size();
        let d = -p.z;
        Vec3::new(
            (p.x / (d * self.lens.tan_x) * 0.5 + 0.5) * w as f32,
            (0.5 - p.y / (d * self.lens.tan_y) * 0.5) * h as f32,
            d,
        )
    }

    /// Draws the faces of a solid box that face the camera. From inside it
    /// every face points away, so it hides nothing.
    fn draw_box(&mut self, aabb: &Aabb, world: &Affine3A) {
        let to_view = self.lens.view * *world;
        let normals = to_view.matrix3.inverse().transpose();
        let center = Vec3::from(aabb.center);
        let half = Vec3::from(aabb.half_extents);
        for axis in 0..3 {
            let (b, c) = ((axis + 1) % 3, (axis + 2) % 3);
            for sign in [-1.0f32, 1.0] {
                let mut face = center;
                face[axis] += sign * half[axis];
                let mut normal = Vec3::ZERO;
                normal[axis] = sign;
                let normal = Vec3::from(normals * Vec3A::from(normal));
                let face_view = to_view.transform_point3(face);
                if normal.dot(face_view) >= 0.0 {
                    continue;
                }
                let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(u, v)| {
                    let mut p = face;
                    p[b] += u * half[b];
                    p[c] += v * half[c];
                    to_view.transform_point3(p)
                });
                self.draw_polygon(&corners);
            }
        }
    }

    fn draw_polygon(&mut self, points: &[Vec3]) {
        // Clip against the near plane, then fan into triangles.
        let near = self.lens.near;
        let inside = |p: &Vec3| -p.z >= near;
        let mut clipped: Vec<Vec3> = Vec::with_capacity(8);
        for (i, a) in points.iter().enumerate() {
            let b = &points[(i + 1) % points.len()];
            if inside(a) {
                clipped.push(*a);
            }
            if inside(a) != inside(b) {
                let t = (-near - a.z) / (b.z - a.z);
                clipped.push(a.lerp(*b, t));
            }
        }
        if clipped.len() < 3 {
            return;
        }
        let projected: Vec<Vec3> = clipped.iter().map(|p| self.project(*p)).collect();
        for i in 1..projected.len() - 1 {
            self.draw_triangle(projected[0], projected[i], projected[i + 1]);
        }
    }

    fn draw_triangle(&mut self, a: Vec3, b: Vec3, c: Vec3) {
        let (w, h) = self.size();
        let area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
        if area.abs() < 1e-8 {
            return;
        }
        let x0 = a.x.min(b.x).min(c.x).floor().max(0.0) as usize;
        let y0 = a.y.min(b.y).min(c.y).floor().max(0.0) as usize;
        let x1 = (a.x.max(b.x).max(c.x).ceil() as isize).min(w as isize - 1);
        let y1 = (a.y.max(b.y).max(c.y).ceil() as isize).min(h as isize - 1);
        if x1 < 0 || y1 < 0 {
            return;
        }
        // Depth is linear in screen space as its reciprocal.
        let inv = Vec3::new(1.0 / a.z, 1.0 / b.z, 1.0 / c.z);
        for y in y0..=y1 as usize {
            for x in x0..=x1 as usize {
                let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((b.x - p.x) * (c.y - p.y) - (b.y - p.y) * (c.x - p.x)) / area;
                let w1 = ((c.x - p.x) * (a.y - p.y) - (c.y - p.y) * (a.x - p.x)) / area;
                let w2 = 1.0 - w0 - w1;
                // A shared diagonal must not drop the centres on it.
                if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                    continue;
                }
                let depth = 1.0 / Vec3::new(w0, w1, w2).dot(inv);
                let texel = &mut self.raw[y * w + x];
                *texel = texel.min(depth);
            }
        }
    }

    /// Erodes coverage by a texel, since a sampled pixel centre says nothing
    /// about its edges, then builds the pyramid.
    fn finish(&mut self) {
        let (w, h) = self.size();
        let mut base = std::mem::take(&mut self.levels[0].2);
        base.clear();
        base.resize(w * h, 0.0);
        for y in 0..h {
            for x in 0..w {
                let mut far = 0.0f32;
                for ny in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                    for nx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                        far = far.max(self.raw[ny * w + nx]);
                    }
                }
                base[y * w + x] = far;
            }
        }
        self.levels[0].2 = base;
        let (mut lw, mut lh) = (w, h);
        let mut level = 0;
        while lw > 1 || lh > 1 {
            let (nw, nh) = (lw.div_ceil(2), lh.div_ceil(2));
            let mut next = match self.levels.get_mut(level + 1) {
                Some(slot) => std::mem::take(&mut slot.2),
                None => Vec::new(),
            };
            next.clear();
            next.resize(nw * nh, 0.0);
            let below = &self.levels[level].2;
            for y in 0..nh {
                for x in 0..nw {
                    let (x0, y0) = (x * 2, y * 2);
                    let (x1, y1) = ((x0 + 1).min(lw - 1), (y0 + 1).min(lh - 1));
                    next[y * nw + x] = below[y0 * lw + x0]
                        .max(below[y0 * lw + x1])
                        .max(below[y1 * lw + x0])
                        .max(below[y1 * lw + x1]);
                }
            }
            if self.levels.len() == level + 1 {
                self.levels.push((nw, nh, next));
            } else {
                self.levels[level + 1] = (nw, nh, next);
            }
            (lw, lh) = (nw, nh);
            level += 1;
        }
        self.levels.truncate(level + 1);
    }

    /// Whether every part of the box is behind drawn occluders. Anything
    /// crossing the near plane or off screen is left to the frustum.
    fn occludes(&self, aabb: &Aabb, world: &Affine3A) -> bool {
        let (w, h) = self.size();
        let to_view = self.lens.view * *world;
        let (mut min, mut max) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        let mut nearest = f32::MAX;
        for i in 0..8 {
            let corner = Vec3::from(aabb.center)
                + Vec3::from(aabb.half_extents)
                    * Vec3::new(
                        if i & 1 == 0 { -1.0 } else { 1.0 },
                        if i & 2 == 0 { -1.0 } else { 1.0 },
                        if i & 4 == 0 { -1.0 } else { 1.0 },
                    );
            let view = to_view.transform_point3(corner);
            if -view.z < self.lens.near {
                return false;
            }
            let p = self.project(view);
            min = min.min(p.truncate());
            max = max.max(p.truncate());
            nearest = nearest.min(p.z);
        }
        if max.x < 0.0 || max.y < 0.0 || min.x >= w as f32 || min.y >= h as f32 {
            return false;
        }
        let x0 = min.x.max(0.0) as usize;
        let y0 = min.y.max(0.0) as usize;
        let x1 = (max.x as usize).min(w - 1);
        let y1 = (max.y as usize).min(h - 1);
        let mut level = 0;
        while level + 1 < self.levels.len()
            && ((x1 >> level) - (x0 >> level) > 1 || (y1 >> level) - (y0 >> level) > 1)
        {
            level += 1;
        }
        let (lw, _, texels) = &self.levels[level];
        let mut far = 0.0f32;
        for y in (y0 >> level)..=(y1 >> level) {
            for x in (x0 >> level)..=(x1 >> level) {
                far = far.max(texels[y * lw + x]);
            }
        }
        nearest > far
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn levels(thresholds: &[f32]) -> Vec<LodLevel> {
        thresholds
            .iter()
            .map(|&min_screen| LodLevel {
                min_screen,
                mesh: None,
            })
            .collect()
    }

    #[test]
    fn levels_hold_inside_the_hysteresis_band() {
        let levels = levels(&[0.3, 0.1, 0.02]);
        assert_eq!(next_level(&levels, Some(0), 0.29, 0.1), Some(0));
        assert_eq!(next_level(&levels, Some(0), 0.25, 0.1), Some(1));
        assert_eq!(next_level(&levels, Some(1), 0.31, 0.1), Some(1));
        assert_eq!(next_level(&levels, Some(1), 0.34, 0.1), Some(0));
        assert_eq!(next_level(&levels, Some(2), 0.001, 0.1), None);
        assert_eq!(next_level(&levels, None, 0.021, 0.1), None);
        assert_eq!(next_level(&levels, None, 0.05, 0.1), Some(2));
    }

    #[test]
    fn group_levels_sort_finest_first() {
        let group = LodGroup::new(1.0)
            .level(0.0, None)
            .level(0.5, None)
            .level(0.1, None);
        let order: Vec<f32> = group.levels().iter().map(|l| l.min_screen).collect();
        assert_eq!(order, [0.5, 0.1, 0.0]);
        assert!(!group.swaps_meshes());
    }

    /// Camera at the origin looking down -Z, a wall 10 units out.
    fn scene() -> HiZ {
        let mut hiz = HiZ::default();
        let lens = Lens {
            view: Affine3A::IDENTITY,
            tan_x: 1.0,
            tan_y: 1.0,
            near: 0.1,
        };
        hiz.begin(128, 128, lens);
        let wall = Aabb::from_min_max(Vec3::new(-4.0, -4.0, -0.5), Vec3::new(4.0, 4.0, 0.5));
        hiz.draw_box(
            &wall,
            &Affine3A::from_translation(Vec3::new(0.0, 0.0, -10.0)),
        );
        hiz.finish();
        hiz
    }

    fn cube_at(position: Vec3) -> (Aabb, Affine3A) {
        (
            Aabb::from_min_max(Vec3::splat(-0.5), Vec3::splat(0.5)),
            Affine3A::from_translation(position),
        )
    }

    #[test]
    fn a_wall_hides_what_is_behind_it_only() {
        let hiz = scene();
        let (aabb, behind) = cube_at(Vec3::new(0.0, 0.0, -20.0));
        assert!(hiz.occludes(&aabb, &behind));
        let (aabb, front) = cube_at(Vec3::new(0.0, 0.0, -5.0));
        assert!(!hiz.occludes(&aabb, &front));
        let (aabb, beside) = cube_at(Vec3::new(12.0, 0.0, -20.0));
        assert!(!hiz.occludes(&aabb, &beside));
        // Peeking past the edge by a little still shows.
        let (aabb, edge) = cube_at(Vec3::new(8.2, 0.0, -20.0));
        assert!(!hiz.occludes(&aabb, &edge));
    }

    #[test]
    fn a_box_around_the_camera_hides_nothing() {
        let mut hiz = HiZ::default();
        hiz.begin(64, 64, Lens::default());
        let room = Aabb::from_min_max(Vec3::splat(-5.0), Vec3::splat(5.0));
        hiz.draw_box(&room, &Affine3A::IDENTITY);
        hiz.finish();
        let (aabb, far) = cube_at(Vec3::new(0.0, 0.0, -20.0));
        assert!(!hiz.occludes(&aabb, &far));
    }

    #[test]
    fn a_wall_through_the_near_plane_still_occludes() {
        let mut hiz = HiZ::default();
        hiz.begin(64, 64, Lens::default());
        let floor = Aabb::from_min_max(Vec3::new(-50.0, -1.0, -50.0), Vec3::new(50.0, 0.0, 50.0));
        hiz.draw_box(
            &floor,
            &Affine3A::from_translation(Vec3::new(0.0, -1.0, 0.0)),
        );
        hiz.finish();
        let (aabb, under) = cube_at(Vec3::new(0.0, -6.0, -12.0));
        assert!(hiz.occludes(&aabb, &under));
        let (aabb, above) = cube_at(Vec3::new(0.0, 1.0, -8.0));
        assert!(!hiz.occludes(&aabb, &above));
    }
}
