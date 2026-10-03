//! Bounded perspective traversal. Selection is independent of mesh readiness.

use crate::{
    lod::MAX_LEVEL,
    lod_mesh::{Key, TILE},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};

pub const MAX_ROOTS: usize = 512;
pub const MAX_LEAVES: usize = 512;
pub const MAX_VISITS: usize = 4096;

#[derive(Clone, Deserialize)]
pub struct View {
    pub position: [f64; 3],
    pub forward: [f64; 3],
    pub up: [f64; 3],
    pub viewport_width: f64,
    pub viewport_height: f64,
    pub fov_y: f64,
    pub render_distance: f64,
    pub split_pixels: f64,
    pub merge_pixels: f64,
}

#[derive(Serialize)]
pub struct Leaf {
    pub level: u8,
    pub tile: [i32; 3],
    pub base: [i32; 3],
    pub extent: [i32; 3],
    pub projected_area: f64,
}

#[derive(Serialize)]
pub struct Selection {
    pub leaves: Vec<Leaf>,
    pub visited: usize,
    pub budget_limited: bool,
}

#[derive(Default)]
pub struct Selector {
    split: BTreeSet<Key>,
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn unit(a: [f64; 3]) -> Result<[f64; 3], String> {
    let n = dot(a, a).sqrt();
    if !n.is_finite() || n < 1e-8 {
        return Err("LOD camera axes must be nonzero and nonparallel".into());
    }
    Ok(a.map(|v| v / n))
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

impl Selector {
    pub fn select(
        &mut self,
        size: [i32; 3],
        origin: [f32; 3],
        voxel: f32,
        view: View,
    ) -> Result<Selection, String> {
        let finite = view
            .position
            .iter()
            .chain(&view.forward)
            .chain(&view.up)
            .chain([&view.viewport_width, &view.viewport_height])
            .chain([
                &view.fov_y,
                &view.render_distance,
                &view.split_pixels,
                &view.merge_pixels,
            ]);
        if finite.into_iter().any(|v| !v.is_finite())
            || !(1.0..=179.0).contains(&view.fov_y)
            || [view.viewport_width, view.viewport_height]
                .iter()
                .any(|v| !(1.0..=32768.0).contains(v))
            || !(0.0..=1e9).contains(&view.render_distance)
            || view.render_distance == 0.0
            || view.merge_pixels <= 0.0
            || view.split_pixels <= view.merge_pixels
            || view.split_pixels > 1e9
        {
            return Err("invalid LOD camera projection, distance or hysteresis thresholds".into());
        }
        let forward = unit(view.forward)?;
        let right = unit(cross(forward, unit(view.up)?))?;
        let up = cross(right, forward);
        let ty = (view.fov_y.to_radians() * 0.5).tan();
        let tx = ty * view.viewport_width / view.viewport_height;
        let focal = view.viewport_height / (2.0 * ty);
        let root_width = TILE << MAX_LEVEL;
        let bounds = [0, 1, 2].map(|a| {
            let count = (size[a] + root_width - 1) / root_width;
            let cell = (view.position[a] - f64::from(origin[a])) / f64::from(voxel);
            let radius = view.render_distance / f64::from(voxel);
            let lo = (((cell - radius) / f64::from(root_width)).ceil() - 1.0)
                .clamp(0.0, f64::from(count)) as i32;
            let hi = ((cell + radius) / f64::from(root_width))
                .floor()
                .clamp(-1.0, f64::from(count - 1)) as i32;
            (lo, hi)
        });
        let roots: i64 = bounds
            .iter()
            .map(|(lo, hi)| i64::from((hi - lo + 1).max(0)))
            .product();
        if roots > MAX_ROOTS as i64 {
            return Err("LOD view exceeds the root budget; reduce render_distance".into());
        }
        let mut queue = VecDeque::new();
        for z in bounds[2].0..=bounds[2].1 {
            for y in bounds[1].0..=bounds[1].1 {
                for x in bounds[0].0..=bounds[0].1 {
                    queue.push_back(Key {
                        level: MAX_LEVEL,
                        tile: [x, y, z],
                    });
                }
            }
        }
        let mut next_split = BTreeSet::new();
        let mut result = Selection {
            leaves: Vec::new(),
            visited: 0,
            budget_limited: false,
        };
        while let Some(key) = queue.pop_front() {
            result.visited += 1;
            let width = TILE << key.level;
            let base = key.tile.map(|c| c * width);
            let extent = [0, 1, 2].map(|a| (size[a] - base[a]).clamp(0, width));
            if extent.contains(&0) {
                continue;
            }
            let lo =
                [0, 1, 2].map(|a| f64::from(origin[a]) + f64::from(base[a]) * f64::from(voxel));
            let hi = [0, 1, 2].map(|a| lo[a] + f64::from(extent[a]) * f64::from(voxel));
            let distance_squared: f64 = (0..3)
                .map(|a| (view.position[a] - view.position[a].clamp(lo[a], hi[a])).powi(2))
                .sum();
            if distance_squared > view.render_distance.powi(2) {
                continue;
            }
            let centre = [0, 1, 2].map(|a| (lo[a] + hi[a]) * 0.5 - view.position[a]);
            let radius = (0..3)
                .map(|a| ((hi[a] - lo[a]) * 0.5).powi(2))
                .sum::<f64>()
                .sqrt();
            let depth = dot(centre, forward);
            if depth + radius < 0.0
                || dot(centre, right).abs() - depth * tx > radius * (1.0 + tx * tx).sqrt()
                || dot(centre, up).abs() - depth * ty > radius * (1.0 + ty * ty).sqrt()
            {
                continue;
            }
            let diameter = 2.0 * radius * focal / (depth - radius).max(f64::from(voxel) * 1e-6);
            let area = diameter * diameter;
            let threshold = if self.split.contains(&key) {
                view.merge_pixels
            } else {
                view.split_pixels
            };
            if key.level > 0 && area > threshold * threshold {
                let children: Vec<_> = (0..8)
                    .map(|i| Key {
                        level: key.level - 1,
                        tile: [0, 1, 2].map(|a| key.tile[a] * 2 + ((i >> a) & 1)),
                    })
                    .filter(|c| (0..3).all(|a| c.tile[a] * (width / 2) < size[a]))
                    .collect();
                if result.leaves.len() + queue.len() + children.len() <= MAX_LEAVES
                    && result.visited + queue.len() + children.len() <= MAX_VISITS
                {
                    queue.extend(children);
                    next_split.insert(key);
                    continue;
                }
                result.budget_limited = true;
            }
            result.leaves.push(Leaf {
                level: key.level,
                tile: key.tile,
                base,
                extent,
                projected_area: (area * 1000.0).round() / 1000.0,
            });
        }
        self.split = next_split;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn view(z: f64) -> View {
        View {
            position: [64.0, 64.0, z],
            forward: [0.0, 0.0, 1.0],
            up: [0.0, 1.0, 0.0],
            viewport_width: 800.0,
            viewport_height: 800.0,
            fov_y: 90.0,
            render_distance: 10000.0,
            split_pixels: 160.0,
            merge_pixels: 120.0,
        }
    }
    #[test]
    fn jitter_retains_split_until_merge_threshold() {
        let mut selector = Selector::default();
        assert!(
            selector
                .select([128; 3], [0.0; 3], 1.0, view(-540.0))
                .unwrap()
                .leaves
                .iter()
                .all(|v| v.level < 4)
        );
        let split = selector
            .select([128; 3], [0.0; 3], 1.0, view(-640.0))
            .unwrap();
        assert!(split.leaves.iter().all(|v| v.level < 4));
        assert_eq!(
            Selector::default()
                .select([128; 3], [0.0; 3], 1.0, view(-640.0))
                .unwrap()
                .leaves[0]
                .level,
            4
        );
        assert_eq!(
            selector
                .select([128; 3], [0.0; 3], 1.0, view(-1000.0))
                .unwrap()
                .leaves[0]
                .level,
            4
        );
    }
    #[test]
    fn budgets_keep_a_nonoverlapping_cover_and_exact_bounds() {
        let mut camera = view(0.0);
        camera.position = [64.0; 3];
        camera.split_pixels = 0.02;
        camera.merge_pixels = 0.01;
        let result = Selector::default()
            .select([127; 3], [0.0; 3], 1.0, camera)
            .unwrap();
        assert!(result.budget_limited);
        assert!(result.leaves.len() <= MAX_LEAVES && result.visited <= MAX_VISITS);
        for (i, a) in result.leaves.iter().enumerate() {
            assert!((0..3).all(|axis| a.base[axis] + a.extent[axis] <= 127));
            for b in &result.leaves[i + 1..] {
                assert!(
                    (0..3).any(|axis| a.base[axis] + a.extent[axis] <= b.base[axis]
                        || b.base[axis] + b.extent[axis] <= a.base[axis])
                );
            }
        }
        // Frustum rejection is conservative; every cell on the forward centre ray is covered.
        for z in 64..127 {
            assert_eq!(
                result
                    .leaves
                    .iter()
                    .filter(|v| (0..3)
                        .all(|a| v.base[a] <= [64, 64, z][a]
                            && [64, 64, z][a] < v.base[a] + v.extent[a]))
                    .count(),
                1
            );
        }
    }
    #[test]
    fn budget_fallback_covers_the_whole_visible_world() {
        let mut camera = view(-1000.0);
        camera.split_pixels = 0.02;
        camera.merge_pixels = 0.01;
        let result = Selector::default()
            .select([127; 3], [0.0; 3], 1.0, camera)
            .unwrap();
        assert!(result.budget_limited);
        assert_eq!(
            result
                .leaves
                .iter()
                .map(|v| v.extent.iter().map(|&n| i64::from(n)).product::<i64>())
                .sum::<i64>(),
            127_i64.pow(3)
        );
        assert!(result.leaves.len() <= MAX_LEAVES);
    }
    #[test]
    fn selection_respects_world_units_and_viewport_scale() {
        let camera = view(-540.0);
        let a = Selector::default()
            .select([128; 3], [0.0; 3], 1.0, camera.clone())
            .unwrap();
        let mut moved = camera.clone();
        moved.position = [0, 1, 2].map(|i| camera.position[i] * 2.0 + [10.0, 20.0, 30.0][i]);
        moved.render_distance *= 2.0;
        moved.viewport_width *= 2.0;
        moved.viewport_height *= 2.0;
        moved.split_pixels *= 2.0;
        moved.merge_pixels *= 2.0;
        let b = Selector::default()
            .select([128; 3], [10.0, 20.0, 30.0], 2.0, moved)
            .unwrap();
        let keys = |s: &Selection| {
            s.leaves
                .iter()
                .map(|v| (v.level, v.tile))
                .collect::<Vec<_>>()
        };
        assert!(!a.leaves.is_empty());
        assert_eq!(keys(&a), keys(&b));
        let mut wider = camera;
        wider.viewport_width = 1600.0;
        wider.position[0] = -300.0;
        let wide = Selector::default()
            .select([128; 3], [0.0; 3], 1.0, wider.clone())
            .unwrap();
        wider.viewport_width = 100.0;
        let narrow = Selector::default()
            .select([128; 3], [0.0; 3], 1.0, wider)
            .unwrap();
        assert!(!wide.leaves.is_empty());
        assert!(narrow.leaves.is_empty());
    }
    #[test]
    fn frustum_distance_teleport_and_invalid_views() {
        let mut selector = Selector::default();
        assert!(
            selector
                .select([128; 3], [0.0; 3], 1.0, view(1000.0))
                .unwrap()
                .leaves
                .is_empty()
        );
        let mut camera = view(-1000.0);
        camera.render_distance = 100.0;
        assert!(
            selector
                .select([128; 3], [0.0; 3], 1.0, camera)
                .unwrap()
                .leaves
                .is_empty()
        );
        assert!(
            selector
                .select([1048576; 3], [0.0; 3], 1.0, view(0.0))
                .is_err()
        );
        let mut camera = view(0.0);
        camera.up = camera.forward;
        assert!(selector.select([128; 3], [0.0; 3], 1.0, camera).is_err());
        let mut camera = view(0.0);
        camera.position[0] = f64::NAN;
        assert!(selector.select([128; 3], [0.0; 3], 1.0, camera).is_err());
        assert!(
            !selector
                .select([128; 3], [0.0; 3], 1.0, view(-100.0))
                .unwrap()
                .leaves
                .is_empty()
        );
    }
}
