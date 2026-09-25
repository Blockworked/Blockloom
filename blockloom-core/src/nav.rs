//! Polyanya navmesh pathfinding over a project's static geometry.
//!
//! A navmesh is baked once per project from its static colliders - ground
//! planes (or slabs) as the walkable boundary, static boxes and balls as
//! holes - and then queried any-angle at run time. The mesh lives in the
//! mover's plane: XZ in 3D, XY in 2D. Only `Static` bodies become obstacles,
//! so the player, the chaser and every clone are never baked in.

use glam::Vec2;
use polyanya::{Mesh, Triangulation};

use crate::project::Project;
use crate::scene::{BodyKind, Mode, Visual};

/// Agent radius the baked mesh keeps off walls when none is given.
pub const DEFAULT_AGENT_RADIUS: f32 = 0.4;

/// Fallback boundary half-extents when a project has no static ground to
/// read one from: 3D first, then 2D (pixels).
const FALLBACK_3D: [f32; 2] = [20.0, 15.0];
const FALLBACK_2D: [f32; 2] = [1000.0, 500.0];

/// A mover's position in its plane: XZ in 3D, XY in 2D.
pub fn plane_coords(mode: Mode, pos: [f32; 3]) -> [f32; 2] {
    if mode.is_3d() {
        [pos[0], pos[2]]
    } else {
        [pos[0], pos[1]]
    }
}

/// A solid footprint in the mover's plane: center plus half-extents.
struct Footprint {
    center: [f32; 2],
    half: [f32; 2],
}

impl Footprint {
    fn area(&self) -> f32 {
        (self.half[0] * 2.0).max(0.0) * (self.half[1] * 2.0).max(0.0)
    }

    /// The overlap with `bounds`, or `None` when fully outside or degenerate.
    fn clamped(&self, bounds: &Footprint) -> Option<Footprint> {
        let lo = [
            (self.center[0] - self.half[0]).max(bounds.center[0] - bounds.half[0]),
            (self.center[1] - self.half[1]).max(bounds.center[1] - bounds.half[1]),
        ];
        let hi = [
            (self.center[0] + self.half[0]).min(bounds.center[0] + bounds.half[0]),
            (self.center[1] + self.half[1]).min(bounds.center[1] + bounds.half[1]),
        ];
        if hi[0] - lo[0] < 1e-4 || hi[1] - lo[1] < 1e-4 {
            return None;
        }
        Some(Footprint {
            center: [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0],
            half: [(hi[0] - lo[0]) / 2.0, (hi[1] - lo[1]) / 2.0],
        })
    }

    /// Corners counter-clockwise, as the triangulation takes them.
    fn corners(&self) -> [Vec2; 4] {
        let [cx, cz] = self.center;
        let [hx, hz] = self.half;
        [
            Vec2::new(cx - hx, cz - hz),
            Vec2::new(cx + hx, cz - hz),
            Vec2::new(cx + hx, cz + hz),
            Vec2::new(cx - hx, cz + hz),
        ]
    }
}

/// The footprints an actor blocks in its project's plane: a static body
/// whose visual has extent in that plane. Most are one box; a solid tilemap
/// is one per merged run of tiles, the same rects it collides with.
fn obstacles_of(mode: Mode, actor: &crate::project::Actor) -> Vec<Footprint> {
    if actor.physics().body != BodyKind::Static {
        return Vec::new();
    }
    let Some(visual) = actor.visual() else {
        return Vec::new();
    };
    if let (false, Visual::Tilemap { tilemap }) = (mode.is_3d(), visual) {
        let pos = actor.placement().position;
        return tilemap
            .solid_rects()
            .into_iter()
            .map(|rect| Footprint {
                center: [pos[0] + rect.center[0], pos[1] + rect.center[1]],
                half: rect.half,
            })
            .collect();
    }
    obstacle_of(mode, visual, actor.placement().position)
        .into_iter()
        .collect()
}

fn obstacle_of(mode: Mode, visual: &Visual, pos: [f32; 3]) -> Option<Footprint> {
    if visual.is_3d() != mode.is_3d() {
        return None;
    }
    let (center, half) = match (mode.is_3d(), visual) {
        (true, Visual::Cuboid { size, .. }) => ([pos[0], pos[2]], [size[0] / 2.0, size[2] / 2.0]),
        (true, Visual::Sphere { radius, .. }) => ([pos[0], pos[2]], [*radius, *radius]),
        (true, Visual::Capsule { radius, .. }) => ([pos[0], pos[2]], [*radius, *radius]),
        (true, Visual::Model { scale, .. }) => ([pos[0], pos[2]], [scale[0] / 2.0, scale[2] / 2.0]),
        (false, Visual::Rect { size, .. }) => ([pos[0], pos[1]], [size[0] / 2.0, size[1] / 2.0]),
        (false, Visual::Circle { radius, .. }) => ([pos[0], pos[1]], [*radius, *radius]),
        (false, Visual::Image { size, .. }) => ([pos[0], pos[1]], [size[0] / 2.0, size[1] / 2.0]),
        _ => return None,
    };
    if half[0] < 1e-4 || half[1] < 1e-4 {
        return None;
    }
    Some(Footprint { center, half })
}

/// The walkable boundary: the largest static ground slab, or a fallback.
/// A 3D ground is a `Plane`; a 2D one a wide `Rect`.
fn boundary_of(project: &Project) -> Footprint {
    let mode = project.world.mode;
    let mut best: Option<Footprint> = None;
    for actor in &project.actors {
        if actor.physics().body != BodyKind::Static {
            continue;
        }
        let Some(visual) = actor.visual() else {
            continue;
        };
        let pos = actor.placement().position;
        let footprint = match (mode.is_3d(), visual) {
            (true, Visual::Plane { size, .. }) => Footprint {
                center: [pos[0], pos[2]],
                half: [size[0] / 2.0, size[1] / 2.0],
            },
            (false, Visual::Rect { size, .. }) => Footprint {
                center: [pos[0], pos[1]],
                half: [size[0] / 2.0, size[1] / 2.0],
            },
            _ => continue,
        };
        if best.as_ref().is_some_and(|b| b.area() >= footprint.area()) {
            continue;
        }
        best = Some(footprint);
    }
    best.unwrap_or_else(|| {
        let half = if mode.is_3d() {
            FALLBACK_3D
        } else {
            FALLBACK_2D
        };
        Footprint {
            center: [0.0, 0.0],
            half,
        }
    })
}

/// Bakes a navmesh from the project's static geometry. The ground slab is
/// the walkable area (never an obstacle); every other static solid is a
/// hole, inflated by `agent_radius`. Degenerate or out-of-bounds solids are
/// skipped rather than failing the bake.
pub fn build_mesh(project: &Project, agent_radius: f32) -> Result<Mesh, String> {
    let mode = project.world.mode;
    let bounds = boundary_of(project);
    let mut tri = Triangulation::from_outer_edges(&bounds.corners());
    tri.set_agent_radius(agent_radius.max(0.0));
    for print in project
        .actors
        .iter()
        .flat_map(|actor| obstacles_of(mode, actor))
    {
        // The ground slab itself is walkable, not a hole.
        if print.center == bounds.center && print.half == bounds.half {
            continue;
        }
        let Some(clipped) = print.clamped(&bounds) else {
            continue;
        };
        tri.add_obstacle(clipped.corners());
    }
    tri.simplify(0.01);
    let mut mesh = tri.as_navmesh();
    mesh.merge_polygons();
    mesh.bake();
    Ok(mesh)
}

/// The shortest any-angle path in plane coords, starting near `from` and
/// ending exactly at `to`. Endpoints off the mesh (inside an inflated wall,
/// outside the grounds) fall back to their closest on-mesh points. `None`
/// means no route exists even then, e.g. a sealed room.
pub fn find_path(mesh: &Mesh, from: [f32; 2], to: [f32; 2]) -> Option<Vec<[f32; 2]>> {
    let f = Vec2::new(from[0], from[1]);
    let t = Vec2::new(to[0], to[1]);
    if let Some(path) = mesh.path(f, t) {
        return Some(normalize(&path.path, from, to));
    }
    let fc = mesh.get_closest_point(f)?.position();
    let tc = mesh.get_closest_point(t)?.position();
    let path = mesh.path(fc, tc)?;
    Some(normalize(&path.path, from, to))
}

/// Drops the waypoint the mover already stands on and pins the last one to
/// the real goal, so following the list ends on the target.
fn normalize(raw: &[Vec2], from: [f32; 2], to: [f32; 2]) -> Vec<[f32; 2]> {
    let mut pts: Vec<[f32; 2]> = raw.iter().map(|v| [v.x, v.y]).collect();
    if pts
        .first()
        .is_some_and(|p| (p[0] - from[0]).hypot(p[1] - from[1]) < 1e-3)
    {
        pts.remove(0);
    }
    match pts.last_mut() {
        Some(last) => *last = to,
        None => pts.push(to),
    }
    pts
}

/// One step from `from` toward the head of `path`, capped at `max_step`.
/// An empty path (already there) answers the current position.
pub fn next_step(path: &[[f32; 2]], from: [f32; 2], max_step: f32) -> [f32; 2] {
    let Some([tx, tz]) = path.first().copied() else {
        return from;
    };
    let dx = tx - from[0];
    let dz = tz - from[1];
    let dist = dx.hypot(dz);
    if dist <= max_step || dist < 1e-6 {
        [tx, tz]
    } else {
        [
            from[0] + dx / dist * max_step,
            from[1] + dz / dist * max_step,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Actor;
    use crate::scene::{Physics, Visual};

    fn solid(name: &str, visual: Visual, pos: [f32; 3]) -> Actor {
        let mut actor = Actor::new(name, visual);
        actor.components.placement_mut().position = pos;
        actor.components.set_physics(Physics {
            body: BodyKind::Static,
            ..Physics::default()
        });
        actor
    }

    fn cuboid(name: &str, center: [f32; 2], size: [f32; 2]) -> Actor {
        solid(
            name,
            Visual::Cuboid {
                color: "#FFF".to_string(),
                size: [size[0], 2.0, size[1]],
            },
            [center[0], 1.0, center[1]],
        )
    }

    /// Ground plus one wall, the chaser's scenario in miniature.
    fn walled() -> Project {
        let mut project = Project::starter("nav", Mode::ThreeD);
        project.actors.push(solid(
            "Ground",
            Visual::Plane {
                color: "#000".to_string(),
                size: [40.0, 30.0],
            },
            [0.0, 0.0, 0.0],
        ));
        // A wall across z = 7, with a gap on the far left.
        project.actors.push(cuboid("Wall", [8.0, 7.0], [24.0, 1.0]));
        project
    }

    #[test]
    fn an_empty_room_is_a_straight_line() {
        let project = Project::starter("nav", Mode::ThreeD);
        let mesh = build_mesh(&project, 0.4).unwrap();
        let path = find_path(&mesh, [0.0, 10.0], [0.0, 0.0]).unwrap();
        assert_eq!(*path.last().unwrap(), [0.0, 0.0]);
        assert!(path.len() <= 2);
    }

    #[test]
    fn a_wall_forces_a_detour_around_its_end() {
        let project = walled();
        let mesh = build_mesh(&project, 0.4).unwrap();
        let path = find_path(&mesh, [0.0, 10.0], [0.0, 0.0]).unwrap();
        // Longer than straight, ends on the goal, and rounds the wall's end.
        assert!(path.len() > 2);
        assert_eq!(*path.last().unwrap(), [0.0, 0.0]);
        assert!(path.iter().any(|p| p[0] < -4.0));
        // No waypoint inside the wall itself (half 12 x 0.5): the path may
        // round its inflated corners, but never clips through it.
        for p in &path {
            let inside =
                p[0] > 8.0 - 12.0 + 0.15 && p[0] < 8.0 + 12.0 - 0.15 && (p[1] - 7.0).abs() < 0.35;
            assert!(!inside, "waypoint {p:?} inside the wall");
        }
    }

    #[test]
    fn a_sealed_room_has_no_route() {
        let mut project = Project::starter("nav", Mode::ThreeD);
        project.actors.push(solid(
            "Ground",
            Visual::Plane {
                color: "#000".to_string(),
                size: [40.0, 30.0],
            },
            [0.0, 0.0, 0.0],
        ));
        // Four walls sealing [9, 11] x [-1, 1].
        project.actors.push(cuboid("N", [10.0, 1.5], [3.0, 1.0]));
        project.actors.push(cuboid("S", [10.0, -1.5], [3.0, 1.0]));
        project.actors.push(cuboid("W", [8.0, 0.0], [1.0, 4.0]));
        project.actors.push(cuboid("E", [12.0, 0.0], [1.0, 4.0]));
        let mesh = build_mesh(&project, 0.4).unwrap();
        assert!(find_path(&mesh, [0.0, 0.0], [10.0, 0.0]).is_none());
    }

    #[test]
    fn a_solid_tilemap_blocks_per_tile_so_its_gaps_stay_open() {
        let mut project = Project::starter("nav", Mode::TwoD);
        project.actors.push(solid(
            "Ground",
            Visual::Rect {
                color: "#000".to_string(),
                size: [800.0, 600.0],
            },
            [0.0, 0.0, 0.0],
        ));
        // A row of five tiles with the middle one missing.
        project.actors.push(solid(
            "Wall",
            Visual::Tilemap {
                tilemap: crate::material::Tilemap {
                    width: 5,
                    height: 1,
                    tile_size: [40.0, 40.0],
                    tiles: vec![0, 0, -1, 0, 0],
                    solid: true,
                    ..crate::material::Tilemap::default()
                },
            },
            [0.0, 0.0, 0.0],
        ));
        let mesh = build_mesh(&project, 5.0).unwrap();
        let path = find_path(&mesh, [0.0, 100.0], [0.0, -100.0]).unwrap();
        // Straight through the gap, which a whole-slab wall would have shut.
        assert!(path.len() <= 2, "{path:?}");
        // A tile itself is still a wall.
        let around = find_path(&mesh, [-60.0, 100.0], [-60.0, -100.0]).unwrap();
        assert!(around.len() > 2, "{around:?}");
    }

    #[test]
    fn plane_coords_use_xz_in_3d_and_xy_in_2d() {
        assert_eq!(plane_coords(Mode::ThreeD, [1.0, 2.0, 3.0]), [1.0, 3.0]);
        assert_eq!(plane_coords(Mode::TwoD, [1.0, 2.0, 3.0]), [1.0, 2.0]);
    }

    #[test]
    fn a_step_is_capped_and_snaps_at_the_waypoint() {
        let path = [[3.0, 4.0]];
        assert_eq!(next_step(&path, [0.0, 0.0], 2.5), [1.5, 2.0]);
        assert_eq!(next_step(&path, [0.0, 0.0], 99.0), [3.0, 4.0]);
        assert_eq!(next_step(&[], [1.0, 1.0], 5.0), [1.0, 1.0]);
    }
}
