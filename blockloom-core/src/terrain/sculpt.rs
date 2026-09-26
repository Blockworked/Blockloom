//! Brushes and erosion: every edit a terrain takes, as plain functions over
//! its grids.
//!
//! The scene view applies a stroke's stamps to its own copy for live
//! feedback and hands the same stroke to the editor, which applies it again
//! to the saved grids. Both run these functions, so the two agree.

use super::store::{Grid, GridKind};
use super::{Heightfield, Shape};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum BrushOp {
    #[default]
    Raise,
    Lower,
    Smooth,
    /// Towards the brush's `level`, which the stroke's first stamp picks up.
    Flatten,
    Noise,
    /// Steps `step` metres tall.
    Terrace,
    /// More of the target: a layer, a hole, grass or trees.
    Paint,
    /// Less of it.
    Erase,
}

impl BrushOp {
    pub fn shapes_heights(self) -> bool {
        !matches!(self, BrushOp::Paint | BrushOp::Erase)
    }
}

/// What a brush works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(tag = "kind")]
pub enum BrushTarget {
    #[default]
    Heights,
    /// A paint layer, by index.
    Layer { layer: u8 },
    Holes,
    /// A grass layer's density map.
    Grass { layer: u8 },
    /// A scatter layer's density map.
    Scatter { layer: u8 },
}

/// A brush's settings. Distances are metres.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Brush {
    pub op: BrushOp,
    pub target: BrushTarget,
    pub radius: f32,
    /// How much one stamp does, 0 to 1.
    pub strength: f32,
    /// 0 is a hard edge, 1 fades from the centre out.
    pub falloff: f32,
    /// Flatten's height, 0-1 of the terrain's height. `None` takes it from
    /// under the stroke's first stamp.
    pub level: Option<f32>,
    /// Terrace step height.
    pub step: f32,
    /// Noise feature size.
    pub scale: f32,
    pub seed: u32,
}

impl Default for Brush {
    fn default() -> Self {
        Self {
            op: BrushOp::Raise,
            target: BrushTarget::Heights,
            radius: 8.0,
            strength: 0.5,
            falloff: 0.6,
            level: None,
            step: 4.0,
            scale: 12.0,
            seed: 1,
        }
    }
}

impl Brush {
    pub fn normalize(&mut self) {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        self.radius = finite(self.radius, 8.0).clamp(0.05, 10_000.0);
        self.strength = finite(self.strength, 0.5).clamp(0.0, 1.0);
        self.falloff = finite(self.falloff, 0.6).clamp(0.0, 1.0);
        self.level = self.level.filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0));
        self.step = finite(self.step, 4.0).clamp(0.01, 10_000.0);
        self.scale = finite(self.scale, 12.0).clamp(0.01, 100_000.0);
    }

    /// How much of a stamp lands `distance` metres from its centre.
    pub fn weight(&self, distance: f32) -> f32 {
        let u = distance / self.radius;
        if u >= 1.0 {
            return 0.0;
        }
        let inner = 1.0 - self.falloff;
        let shape = if u <= inner {
            1.0
        } else {
            let t = 1.0 - (u - inner) / (1.0 - inner).max(1e-6);
            t * t * (3.0 - 2.0 * t)
        };
        shape * self.strength
    }
}

/// One stroke: a brush dragged through local X, Z points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Stroke {
    pub brush: Brush,
    pub stamps: Vec<[f32; 2]>,
}

/// Samples a stroke touched, inclusive, for re-meshing only what changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dirty {
    pub min: [u32; 2],
    pub max: [u32; 2],
}

impl Dirty {
    pub fn union(self, other: Dirty) -> Dirty {
        Dirty {
            min: [self.min[0].min(other.min[0]), self.min[1].min(other.min[1])],
            max: [self.max[0].max(other.max[0]), self.max[1].max(other.max[1])],
        }
    }
}

/// The samples one stamp can reach.
fn reach(shape: &Shape, brush: &Brush, at: [f32; 2]) -> Option<Dirty> {
    let spacing = shape.spacing();
    let [ci, cj] = shape.sample(at[0], at[1]);
    let (ri, rj) = (brush.radius / spacing[0], brush.radius / spacing[1]);
    let last = (shape.side - 1) as f32;
    let (i0, i1) = ((ci - ri).floor().max(0.0), (ci + ri).ceil().min(last));
    let (j0, j1) = ((cj - rj).floor().max(0.0), (cj + rj).ceil().min(last));
    (i0 <= i1 && j0 <= j1).then(|| Dirty {
        min: [i0 as u32, j0 as u32],
        max: [i1 as u32, j1 as u32],
    })
}

/// Runs `visit(index, i, j, weight)` for every sample under a stamp.
fn under(shape: &Shape, brush: &Brush, at: [f32; 2], mut visit: impl FnMut(u32, u32, f32)) -> Option<Dirty> {
    let dirty = reach(shape, brush, at)?;
    for j in dirty.min[1]..=dirty.max[1] {
        for i in dirty.min[0]..=dirty.max[0] {
            let [x, z] = shape.local(i as f32, j as f32);
            let w = brush.weight(((x - at[0]).powi(2) + (z - at[1]).powi(2)).sqrt());
            if w > 0.0 {
                visit(i, j, w);
            }
        }
    }
    Some(dirty)
}

/// Applies a height stroke. Flatten's level is fixed from the first stamp
/// when the brush doesn't carry one. Answers what changed.
pub fn apply_heights(field: &mut Heightfield, shape: &Shape, stroke: &Stroke) -> Option<Dirty> {
    let mut brush = stroke.brush;
    brush.normalize();
    if !brush.op.shapes_heights() {
        return None;
    }
    if brush.op == BrushOp::Flatten && brush.level.is_none() {
        let first = stroke.stamps.first()?;
        let [i, j] = shape.sample(first[0], first[1]);
        brush.level = Some(field.bilinear(i, j));
    }
    let mut dirty: Option<Dirty> = None;
    for &at in &stroke.stamps {
        if let Some(d) = stamp_heights(field, shape, &brush, at) {
            dirty = Some(dirty.map_or(d, |all| all.union(d)));
        }
    }
    dirty
}

fn stamp_heights(field: &mut Heightfield, shape: &Shape, brush: &Brush, at: [f32; 2]) -> Option<Dirty> {
    let height = shape.height.max(1e-3);
    // A full-strength stamp moves the ground a twentieth of the radius.
    let push = brush.radius * 0.05 / height;
    match brush.op {
        BrushOp::Raise | BrushOp::Lower => {
            let sign = if brush.op == BrushOp::Raise { 1.0 } else { -1.0 };
            under(shape, brush, at, |i, j, w| {
                let k = field.index(i, j);
                field.samples[k] = (field.samples[k] + sign * push * w).clamp(0.0, 1.0);
            })
        }
        BrushOp::Flatten => {
            let level = brush.level.unwrap_or(0.0);
            under(shape, brush, at, |i, j, w| {
                let k = field.index(i, j);
                field.samples[k] += (level - field.samples[k]) * w;
            })
        }
        BrushOp::Noise => under(shape, brush, at, |i, j, w| {
            let [x, z] = shape.local(i as f32, j as f32);
            let n = fbm(x / brush.scale, z / brush.scale, brush.seed) - 0.5;
            let k = field.index(i, j);
            field.samples[k] = (field.samples[k] + n * push * 2.0 * w).clamp(0.0, 1.0);
        }),
        BrushOp::Terrace => {
            let step = brush.step / height;
            under(shape, brush, at, |i, j, w| {
                let k = field.index(i, j);
                let t = field.samples[k] / step;
                let f = t.fract();
                let rise = ((f - 0.75) / 0.25).clamp(0.0, 1.0);
                let target = (t.floor() + rise * rise * (3.0 - 2.0 * rise)) * step;
                field.samples[k] += (target.clamp(0.0, 1.0) - field.samples[k]) * w;
            })
        }
        BrushOp::Smooth => {
            let dirty = reach(shape, brush, at)?;
            let spacing = shape.spacing()[0].min(shape.spacing()[1]);
            let kernel = ((brush.radius * 0.15 / spacing).round() as i64).clamp(1, 4);
            // Blur from a copy of just the stamp's reach, so samples don't
            // smear into each other.
            let last = field.side as i64 - 1;
            let (i0, j0) = ((dirty.min[0] as i64 - kernel).max(0), (dirty.min[1] as i64 - kernel).max(0));
            let (i1, j1) = ((dirty.max[0] as i64 + kernel).min(last), (dirty.max[1] as i64 + kernel).min(last));
            let width = i1 - i0 + 1;
            let mut before = Vec::with_capacity((width * (j1 - j0 + 1)) as usize);
            for j in j0..=j1 {
                for i in i0..=i1 {
                    before.push(field.at(i as u32, j as u32));
                }
            }
            let copied = |i: i64, j: i64| {
                let (i, j) = (i.clamp(0, last), j.clamp(0, last));
                before[((j - j0) * width + (i - i0)) as usize]
            };
            under(shape, brush, at, |i, j, w| {
                let (ii, jj) = (i as i64, j as i64);
                let mut sum = 0.0;
                let mut count = 0.0;
                for dj in -kernel..=kernel {
                    for di in -kernel..=kernel {
                        sum += copied(ii + di, jj + dj);
                        count += 1.0;
                    }
                }
                let k = field.index(i, j);
                field.samples[k] += (sum / count - field.samples[k]) * w;
            });
            Some(dirty)
        }
        BrushOp::Paint | BrushOp::Erase => None,
    }
}

/// Applies a paint stroke to a layer weight grid (RGBA8): paint pulls a
/// sample towards all `layer`, erase lets the rules back in.
pub fn apply_paint(splat: &mut Grid, shape: &Shape, stroke: &Stroke) -> Option<Dirty> {
    let mut brush = stroke.brush;
    brush.normalize();
    let BrushTarget::Layer { layer } = brush.target else {
        return None;
    };
    if splat.kind != GridKind::Rgba8 || layer as usize >= super::MAX_LAYERS {
        return None;
    }
    let side = splat.side;
    let mut dirty: Option<Dirty> = None;
    for &at in &stroke.stamps {
        let d = under(shape, &brush, at, |i, j, w| {
            let k = (j * side + i) as usize * 4;
            for c in 0..4 {
                let p = splat.bytes[k + c] as f32;
                let target = if brush.op == BrushOp::Paint && c == layer as usize {
                    255.0
                } else {
                    0.0
                };
                splat.bytes[k + c] = (p + (target - p) * w).round().clamp(0.0, 255.0) as u8;
            }
        });
        if let Some(d) = d {
            dirty = Some(dirty.map_or(d, |all| all.union(d)));
        }
    }
    dirty
}

/// Applies a stroke to a one-byte grid (holes or a density map): paint
/// raises it, erase lowers it.
pub fn apply_mask(mask: &mut Grid, shape: &Shape, stroke: &Stroke) -> Option<Dirty> {
    let mut brush = stroke.brush;
    brush.normalize();
    let side = mask.side;
    let bytes = mask.mask_mut()?;
    let mut dirty: Option<Dirty> = None;
    for &at in &stroke.stamps {
        let d = under(shape, &brush, at, |i, j, w| {
            let k = (j * side + i) as usize;
            let v = bytes[k] as f32;
            let target = if brush.op == BrushOp::Paint { 255.0 } else { 0.0 };
            bytes[k] = (v + (target - v) * w).round().clamp(0.0, 255.0) as u8;
        });
        if let Some(d) = d {
            dirty = Some(dirty.map_or(d, |all| all.union(d)));
        }
    }
    dirty
}

/// A density map's first stroke decides its starting fill: painting starts
/// from nothing, so trees go only where painted; erasing starts from full.
pub fn fresh_density(side: u32, op: BrushOp) -> Grid {
    let mut grid = Grid::new(GridKind::Mask8, side);
    if op == BrushOp::Erase {
        grid.bytes.fill(255);
    }
    grid
}

// ─── Strokes on a terrain ──────────────────────────────────────────────────

/// Every grid a stroke can edit, loaded (or flat) at the terrain's
/// resolution. The scene view keeps one while a stroke is live; the editor
/// loads one per stroke.
#[derive(Debug, Clone)]
pub struct Editable {
    pub heights: Heightfield,
    pub splat: Option<Grid>,
    pub holes: Option<Grid>,
    pub grass: Vec<Option<Grid>>,
    pub scatter: Vec<Option<Grid>>,
}

impl Editable {
    pub fn load(project: Option<&std::path::Path>, spec: &super::TerrainSpec) -> Editable {
        use super::store::{grid_for, heights_for};
        let side = spec.resolution;
        Editable {
            heights: heights_for(project, spec),
            splat: grid_for(project, &spec.splat, side),
            holes: grid_for(project, &spec.holes, side),
            grass: spec.grass.iter().map(|g| grid_for(project, &g.density_map, side)).collect(),
            scatter: spec.scatter.iter().map(|s| grid_for(project, &s.density_map, side)).collect(),
        }
    }

    /// Applies a stroke to the grid it targets, making that grid first if
    /// the terrain has none yet. Answers what changed.
    pub fn apply(&mut self, shape: &Shape, stroke: &Stroke) -> Option<Dirty> {
        let side = shape.side;
        let op = stroke.brush.op;
        match stroke.brush.target {
            BrushTarget::Heights => apply_heights(&mut self.heights, shape, stroke),
            BrushTarget::Layer { .. } => apply_paint(
                self.splat.get_or_insert_with(|| Grid::new(GridKind::Rgba8, side)),
                shape,
                stroke,
            ),
            BrushTarget::Holes => apply_mask(
                self.holes.get_or_insert_with(|| Grid::new(GridKind::Mask8, side)),
                shape,
                stroke,
            ),
            BrushTarget::Grass { layer } => {
                let slot = self.grass.get_mut(layer as usize)?;
                apply_mask(slot.get_or_insert_with(|| fresh_density(side, op)), shape, stroke)
            }
            BrushTarget::Scatter { layer } => {
                let slot = self.scatter.get_mut(layer as usize)?;
                apply_mask(slot.get_or_insert_with(|| fresh_density(side, op)), shape, stroke)
            }
        }
    }

    /// The grid a target now holds, as it will be stored.
    pub fn grid(&self, target: BrushTarget) -> Option<Grid> {
        match target {
            BrushTarget::Heights => Some(Grid::from_heights(&self.heights)),
            BrushTarget::Layer { .. } => self.splat.clone(),
            BrushTarget::Holes => self.holes.clone(),
            BrushTarget::Grass { layer } => self.grass.get(layer as usize)?.clone(),
            BrushTarget::Scatter { layer } => self.scatter.get(layer as usize)?.clone(),
        }
    }
}

/// Points a terrain at a target's newly stored grid.
pub fn set_target(spec: &mut super::TerrainSpec, target: BrushTarget, name: String) {
    match target {
        BrushTarget::Heights => spec.heights = name,
        BrushTarget::Layer { .. } => spec.splat = name,
        BrushTarget::Holes => spec.holes = name,
        BrushTarget::Grass { layer } => {
            if let Some(grass) = spec.grass.get_mut(layer as usize) {
                grass.density_map = name;
            }
        }
        BrushTarget::Scatter { layer } => {
            if let Some(scatter) = spec.scatter.get_mut(layer as usize) {
                scatter.density_map = name;
            }
        }
    }
}

/// Fixes a flatten stroke's level from where it starts, so applying its
/// stamps one at a time lands where applying them together does.
pub fn settle_level(stroke: &mut Stroke, field: &Heightfield, shape: &Shape) {
    if stroke.brush.op == BrushOp::Flatten
        && stroke.brush.level.is_none()
        && let Some(first) = stroke.stamps.first()
    {
        let [i, j] = shape.sample(first[0], first[1]);
        stroke.brush.level = Some(field.bilinear(i, j));
    }
}

// ─── Erosion ───────────────────────────────────────────────────────────────

/// Thermal erosion: material slides off anything steeper than `talus`
/// degrees, `iterations` times over.
pub fn thermal(field: &mut Heightfield, shape: &Shape, iterations: u32, talus: f32) {
    let side = field.side as i64;
    let spacing = shape.spacing()[0].min(shape.spacing()[1]);
    let height = shape.height.max(1e-3);
    // The steepest drop per neighbour that stays put, in 0-1 units.
    let limit = talus.clamp(1.0, 89.0).to_radians().tan() * spacing / height;
    let neighbours = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    for _ in 0..iterations.min(1000) {
        let before = field.samples.clone();
        for j in 0..side {
            for i in 0..side {
                let k = (j * side + i) as usize;
                let h = before[k];
                let mut total = 0.0;
                let mut drops = [0.0f32; 4];
                for (n, (di, dj)) in neighbours.iter().enumerate() {
                    let (ni, nj) = (i + di, j + dj);
                    if ni < 0 || nj < 0 || ni >= side || nj >= side {
                        continue;
                    }
                    let d = h - before[(nj * side + ni) as usize];
                    if d > limit {
                        drops[n] = d - limit;
                        total += d - limit;
                    }
                }
                if total <= 0.0 {
                    continue;
                }
                let moved = total * 0.25;
                field.samples[k] -= moved;
                for (n, (di, dj)) in neighbours.iter().enumerate() {
                    if drops[n] > 0.0 {
                        let nk = ((j + dj) * side + (i + di)) as usize;
                        field.samples[nk] += moved * drops[n] / total;
                    }
                }
            }
        }
    }
}

/// An erosion filter, as the editor previews and applies it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Erosion {
    Thermal { iterations: u32, talus: f32 },
    Hydraulic(Hydraulic),
}

impl Erosion {
    pub fn apply(&self, field: &mut Heightfield, shape: &Shape) {
        match self {
            Erosion::Thermal { iterations, talus } => thermal(field, shape, *iterations, *talus),
            Erosion::Hydraulic(params) => hydraulic(field, shape, params),
        }
    }
}

/// Hydraulic erosion's dials.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hydraulic {
    /// Raindrops simulated. Zero picks a count for the resolution.
    pub droplets: u32,
    pub seed: u32,
    /// How fast a drop picks up sediment, 0 to 1.
    pub erosion: f32,
    /// How fast it lets sediment go, 0 to 1.
    pub deposition: f32,
    /// How much a drop keeps its heading over the slope, 0 to 1.
    pub inertia: f32,
}

impl Default for Hydraulic {
    fn default() -> Self {
        Self {
            droplets: 0,
            seed: 7,
            erosion: 0.3,
            deposition: 0.3,
            inertia: 0.05,
        }
    }
}

/// Hydraulic erosion: raindrops run downhill, carving where they speed up
/// and dropping silt where they slow, which cuts gullies and fills valleys.
pub fn hydraulic(field: &mut Heightfield, shape: &Shape, params: &Hydraulic) {
    let side = field.side as usize;
    let droplets = if params.droplets == 0 {
        (side * side / 8) as u32
    } else {
        params.droplets.min(5_000_000)
    };
    let height = shape.height.max(1e-3);
    // Drops think in samples and metres-ish heights; the grid stores 0-1.
    let scale = height / shape.spacing()[0].max(1e-3);
    let erosion = params.erosion.clamp(0.0, 1.0);
    let deposition = params.deposition.clamp(0.0, 1.0);
    let inertia = params.inertia.clamp(0.0, 1.0);
    let last = (side - 1) as f32;
    let gradient = |field: &Heightfield, x: f32, y: f32| {
        let (i, j) = (x.floor() as i64, y.floor() as i64);
        let (u, v) = (x - i as f32, y - j as f32);
        let a = field.clamped(i, j) * scale;
        let b = field.clamped(i + 1, j) * scale;
        let c = field.clamped(i, j + 1) * scale;
        let d = field.clamped(i + 1, j + 1) * scale;
        let gx = (b - a) * (1.0 - v) + (d - c) * v;
        let gy = (c - a) * (1.0 - u) + (d - b) * u;
        let h = a * (1.0 - u) * (1.0 - v) + b * u * (1.0 - v) + c * (1.0 - u) * v + d * u * v;
        (gx, gy, h)
    };
    for n in 0..droplets {
        let mut x = unit(hash(params.seed, n, 0)) * last;
        let mut y = unit(hash(params.seed, n, 1)) * last;
        let (mut dx, mut dy) = (0.0f32, 0.0f32);
        let mut speed = 1.0f32;
        let mut water = 1.0f32;
        let mut sediment = 0.0f32;
        for _ in 0..64 {
            let (i, j) = (x.floor() as i64, y.floor() as i64);
            let (u, v) = (x - i as f32, y - j as f32);
            let (gx, gy, h) = gradient(field, x, y);
            dx = dx * inertia - gx * (1.0 - inertia);
            dy = dy * inertia - gy * (1.0 - inertia);
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1e-6 {
                break;
            }
            dx /= len;
            dy /= len;
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0.0 || ny < 0.0 || nx > last || ny > last {
                break;
            }
            let (_, _, nh) = gradient(field, nx, ny);
            let drop = nh - h;
            let capacity = (-drop).max(0.01) * speed * water * 4.0;
            // Spread a change over the cell's four corners.
            let mut deposit = |amount: f32| {
                for (di, dj, w) in [
                    (0, 0, (1.0 - u) * (1.0 - v)),
                    (1, 0, u * (1.0 - v)),
                    (0, 1, (1.0 - u) * v),
                    (1, 1, u * v),
                ] {
                    let (ci, cj) = ((i + di) as usize, (j + dj) as usize);
                    if ci < side && cj < side {
                        let k = cj * side + ci;
                        field.samples[k] = (field.samples[k] + amount * w / scale).clamp(0.0, 1.0);
                    }
                }
            };
            if sediment > capacity || drop > 0.0 {
                let amount = if drop > 0.0 {
                    drop.min(sediment)
                } else {
                    (sediment - capacity) * deposition
                };
                sediment -= amount;
                deposit(amount);
            } else {
                let amount = ((capacity - sediment) * erosion).min(-drop);
                sediment += amount;
                deposit(-amount);
            }
            speed = (speed * speed + drop.abs().min(4.0) * if drop < 0.0 { 1.0 } else { -1.0 })
                .max(0.0)
                .sqrt();
            water *= 0.97;
            x = nx;
            y = ny;
        }
    }
}

// ─── Noise ─────────────────────────────────────────────────────────────────

pub(crate) fn hash(seed: u32, n: u32, lane: u32) -> u32 {
    // PCG, as `blockloom::hash` does it on the GPU.
    let v = seed
        .wrapping_mul(0x9E37_79B9)
        .wrapping_add(n.wrapping_mul(0x85EB_CA6B))
        .wrapping_add(lane.wrapping_mul(0xC2B2_AE35));
    let state = v.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
    (word >> 22) ^ word
}

/// 0 to just under 1.
pub(crate) fn unit(x: u32) -> f32 {
    (x >> 8) as f32 / 16_777_216.0
}

fn lattice(seed: u32, x: i32, z: i32) -> f32 {
    unit(hash(seed, (x as u32).wrapping_mul(73_856_093) ^ (z as u32).wrapping_mul(19_349_663), 2))
}

/// Smooth value noise, 0-1.
pub(crate) fn value_noise(x: f32, z: f32, seed: u32) -> f32 {
    let (x0, z0) = (x.floor(), z.floor());
    let (fx, fz) = (x - x0, z - z0);
    let (sx, sz) = (fx * fx * (3.0 - 2.0 * fx), fz * fz * (3.0 - 2.0 * fz));
    let (x0, z0) = (x0 as i32, z0 as i32);
    let a = lattice(seed, x0, z0);
    let b = lattice(seed, x0 + 1, z0);
    let c = lattice(seed, x0, z0 + 1);
    let d = lattice(seed, x0 + 1, z0 + 1);
    let top = a + (b - a) * sx;
    let bottom = c + (d - c) * sx;
    top + (bottom - top) * sz
}

/// Four octaves of value noise, 0-1.
pub(crate) fn fbm(x: f32, z: f32, seed: u32) -> f32 {
    let mut sum = 0.0;
    let mut amplitude = 0.5;
    let mut frequency = 1.0;
    let mut total = 0.0;
    for octave in 0..4 {
        sum += value_noise(x * frequency, z * frequency, seed.wrapping_add(octave)) * amplitude;
        total += amplitude;
        amplitude *= 0.5;
        frequency *= 2.0;
    }
    sum / total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape() -> Shape {
        Shape {
            size: [128.0, 128.0],
            height: 50.0,
            side: 129,
        }
    }

    fn stroke(op: BrushOp, target: BrushTarget, stamps: Vec<[f32; 2]>) -> Stroke {
        Stroke {
            brush: Brush {
                op,
                target,
                radius: 10.0,
                strength: 1.0,
                ..Brush::default()
            },
            stamps,
        }
    }

    #[test]
    fn raise_lifts_the_centre_most_and_leaves_the_rest() {
        let mut field = Heightfield::flat(129, 0.2);
        let dirty = apply_heights(
            &mut field,
            &shape(),
            &stroke(BrushOp::Raise, BrushTarget::Heights, vec![[0.0, 0.0]]),
        )
        .unwrap();
        assert!(field.at(64, 64) > 0.2);
        assert!(field.at(64, 64) > field.at(70, 64));
        assert_eq!(field.at(0, 0), 0.2);
        assert!(dirty.min[0] >= 53 && dirty.max[0] <= 75);
    }

    #[test]
    fn a_stroke_replays_exactly() {
        let mut a = Heightfield::flat(129, 0.3);
        let mut b = a.clone();
        let s = stroke(BrushOp::Noise, BrushTarget::Heights, vec![[1.0, 2.0], [4.0, 3.0]]);
        apply_heights(&mut a, &shape(), &s);
        apply_heights(&mut b, &shape(), &s);
        assert_eq!(a, b);
        assert_ne!(a, Heightfield::flat(129, 0.3));
    }

    #[test]
    fn flatten_takes_its_level_from_the_first_stamp() {
        let mut field = Heightfield::flat(129, 0.2);
        let k = field.index(64, 64);
        field.samples[k] = 0.6;
        let k = field.index(80, 64);
        field.samples[k] = 0.9;
        apply_heights(
            &mut field,
            &shape(),
            &stroke(BrushOp::Flatten, BrushTarget::Heights, vec![[16.0, 0.0]]),
        );
        // The stroke started on the 0.9 peak, so its neighbourhood rose.
        assert!((field.at(80, 64) - 0.9).abs() < 1e-5);
        assert!(field.at(81, 64) > 0.5);
    }

    #[test]
    fn smoothing_shrinks_a_spike() {
        let mut field = Heightfield::flat(129, 0.2);
        let k = field.index(64, 64);
        field.samples[k] = 1.0;
        apply_heights(
            &mut field,
            &shape(),
            &stroke(BrushOp::Smooth, BrushTarget::Heights, vec![[0.0, 0.0]]),
        );
        assert!(field.at(64, 64) < 0.5);
    }

    #[test]
    fn terraces_make_flat_steps() {
        let mut field = Heightfield::flat(129, 0.0);
        for (k, s) in field.samples.iter_mut().enumerate() {
            *s = (k % 129) as f32 / 128.0 * 0.5;
        }
        let mut s = stroke(BrushOp::Terrace, BrushTarget::Heights, vec![[0.0, 0.0]]);
        s.brush.radius = 100.0;
        s.brush.falloff = 0.0;
        s.brush.step = 5.0;
        apply_heights(&mut field, &shape(), &s);
        // Two samples low on one 5 m step land on the same height.
        let (a, b) = (field.at(60, 64), field.at(62, 64));
        assert!((a - b).abs() < 1e-4, "{a} {b}");
    }

    #[test]
    fn paint_pulls_towards_a_layer_and_erase_lets_go() {
        let mut splat = Grid::new(GridKind::Rgba8, 129);
        apply_paint(
            &mut splat,
            &shape(),
            &stroke(BrushOp::Paint, BrushTarget::Layer { layer: 2 }, vec![[0.0, 0.0]]),
        )
        .unwrap();
        let k = (64 * 129 + 64) * 4;
        assert_eq!(&splat.bytes[k..k + 4], &[0, 0, 255, 0]);
        apply_paint(
            &mut splat,
            &shape(),
            &stroke(BrushOp::Erase, BrushTarget::Layer { layer: 2 }, vec![[0.0, 0.0]]),
        );
        assert_eq!(&splat.bytes[k..k + 4], &[0, 0, 0, 0]);
    }

    #[test]
    fn holes_and_densities_paint_one_byte() {
        let mut holes = Grid::new(GridKind::Mask8, 129);
        apply_mask(
            &mut holes,
            &shape(),
            &stroke(BrushOp::Paint, BrushTarget::Holes, vec![[0.0, 0.0]]),
        );
        assert_eq!(holes.mask().unwrap()[64 * 129 + 64], 255);
        assert_eq!(fresh_density(129, BrushOp::Erase).bytes[0], 255);
        assert_eq!(fresh_density(129, BrushOp::Paint).bytes[0], 0);
    }

    #[test]
    fn thermal_erosion_slumps_a_cliff() {
        let mut field = Heightfield::flat(129, 0.0);
        for j in 0..129 {
            for i in 64..129 {
                let k = field.index(i, j);
                field.samples[k] = 0.8;
            }
        }
        let before = field.slope(&shape(), 64, 64);
        thermal(&mut field, &shape(), 50, 30.0);
        assert!(field.slope(&shape(), 64, 64) < before);
        let total: f32 = field.samples.iter().sum();
        assert!((total - 0.8 * 65.0 * 129.0).abs() < 1.0, "mass is kept: {total}");
    }

    #[test]
    fn hydraulic_erosion_is_deterministic_and_changes_a_slope() {
        let mut field = Heightfield::flat(129, 0.0);
        for j in 0..129 {
            for i in 0..129 {
                let k = field.index(i, j);
                field.samples[k] = 0.3 + 0.4 * fbm(i as f32 / 20.0, j as f32 / 20.0, 3);
            }
        }
        let mut again = field.clone();
        let params = Hydraulic {
            droplets: 2000,
            ..Hydraulic::default()
        };
        let original = field.clone();
        hydraulic(&mut field, &shape(), &params);
        hydraulic(&mut again, &shape(), &params);
        assert_eq!(field, again);
        assert_ne!(field, original);
        assert!(field.samples.iter().all(|s| (0.0..=1.0).contains(s)));
    }
}
