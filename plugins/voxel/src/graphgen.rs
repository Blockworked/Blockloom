//! Worldgen graphs: `.blockgen` assets shaped after Hytale's Worldgen V2.
//!
//! A world structure picks biomes by ranges of a `biome_map` density and
//! blends their terrain densities across `transition` cells; each biome is
//! a terrain density (positive is solid), a material provider and props.
//! Densities are typed node trees with `inputs`, and `exports` are named
//! trees any node can `import`. Every cell asks only its own coordinates,
//! the seed and the graph, so streamed pages never change the world.

use crate::grid::Block;
use blockloom_plugin_sdk::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

/// Nodes one graph may hold after its imports are expanded.
pub const MAX_NODES: usize = 4096;
/// The farthest a prop may reach from its anchor, in cells.
const MAX_EXTENT: i32 = 16;
/// Columns kept per cache.
const CACHED_COLUMNS: usize = 4096;

type Id = usize;

#[derive(Debug, Clone)]
enum D {
    Constant(f64),
    X,
    Y,
    Z,
    /// `y - base`, or the base itself.
    BaseHeight(f64, bool),
    Noise2(Fbm),
    Noise3(Fbm),
    /// Distance to the nearest jittered cell point, or that cell's value.
    Cell2 {
        scale: f64,
        jitter: f64,
        seed: u32,
        value: bool,
    },
    Sum(Vec<Id>),
    Multiply(Vec<Id>),
    Min(Vec<Id>),
    Max(Vec<Id>),
    Abs(Id),
    Pow(Id, f64),
    Clamp(Id, f64, f64),
    Normalize(Id, [f64; 4]),
    Curve(Id, Vec<(f64, f64)>),
    /// Then when the first input is not 0, else otherwise.
    Select(Id, Id, Id),
    Less(Id, Id),
    Greater(Id, Id),
    /// `a + (b - a) * t`.
    Mix(Id, Id, Id),
    Scale(Id, [f64; 3]),
    Offset(Id, [f64; 3]),
    YOverride(Id, f64),
    /// Moves x and z by 2D noise before asking the input.
    Warp(Id, Fbm, f64),
}

#[derive(Debug, Clone, Copy)]
struct Fbm {
    scale: f64,
    octaves: u32,
    persistence: f64,
    lacunarity: f64,
    seed: u32,
}

#[derive(Debug, Clone)]
enum M {
    /// A block, or air for `air`/`empty`.
    Constant(Block),
    /// The first provider that answers.
    Queue(Vec<usize>),
    Solidity(Option<usize>, Option<usize>),
    /// Layers counted down from the surface (or up from a ceiling).
    Depth {
        layers: Vec<(i32, usize)>,
        min_space: i32,
        ceiling: bool,
    },
    Range(Id, Vec<(f64, f64, usize)>),
    Horizontal(f64, f64, usize),
    Weighted(u32, Vec<(f64, usize)>),
}

#[derive(Debug, Clone)]
enum P {
    Cuboid([i32; 3], [i32; 3], Block),
    Sphere(f64, [i32; 3], Block),
    Tree {
        trunk: Block,
        leaves: Block,
        height: (i32, i32),
        radius: i32,
    },
    Column(Block, (i32, i32)),
    Weighted(Vec<(f64, usize)>),
}

#[derive(Debug, Clone)]
enum Positions {
    Grid {
        spacing: i32,
        jitter: f64,
        seed: u32,
        triangular: bool,
    },
    List(Vec<[i32; 2]>),
}

#[derive(Debug, Clone)]
struct PropRule {
    positions: Positions,
    chance: f64,
    on: BTreeSet<Block>,
    gate: Option<Id>,
    prop: usize,
    extent: i32,
}

#[derive(Debug, Clone)]
struct Biome {
    name: String,
    min: f64,
    max: f64,
    terrain: Id,
    material: usize,
    props: Vec<PropRule>,
}

/// Seed, x, z and height.
type ColumnKey = (u32, i32, i32, i32);

struct Column {
    blocks: Vec<Block>,
    top: i32,
}

/// A compiled `.blockgen`.
pub struct Graph {
    d: Vec<D>,
    depends_y: Vec<bool>,
    m: Vec<M>,
    p: Vec<P>,
    biomes: Vec<Biome>,
    default_biome: usize,
    biome_map: Id,
    transition: i32,
    terrain: Mutex<BTreeMap<ColumnKey, Arc<Column>>>,
    columns: Mutex<BTreeMap<ColumnKey, Arc<Vec<Block>>>>,
}

impl std::fmt::Debug for Graph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Graph({} nodes, {} biomes)",
            self.d.len(),
            self.biomes.len()
        )
    }
}

fn name_seed(seed: u32, name: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5 ^ seed;
    for b in name.bytes() {
        h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    h ^ (h >> 15)
}

fn hash(seed: u32, a: i32, b: i32, c: i32) -> u32 {
    let mut h = seed
        ^ (a as u32).wrapping_mul(0x8da6_b343)
        ^ (b as u32).wrapping_mul(0xd816_3841)
        ^ (c as u32).wrapping_mul(0xcb1a_b31f);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^ (h >> 16)
}

fn unit(h: u32) -> f64 {
    f64::from(h) / f64::from(u32::MAX)
}

fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

const R: f64 = std::f64::consts::FRAC_1_SQRT_2;

const G2: [[f64; 2]; 8] = [
    [1.0, 0.0],
    [-1.0, 0.0],
    [0.0, 1.0],
    [0.0, -1.0],
    [R, R],
    [-R, R],
    [R, -R],
    [-R, -R],
];

const G3: [[f64; 3]; 12] = [
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
    [1.0, -1.0, 0.0],
    [-1.0, -1.0, 0.0],
    [1.0, 0.0, 1.0],
    [-1.0, 0.0, 1.0],
    [1.0, 0.0, -1.0],
    [-1.0, 0.0, -1.0],
    [0.0, 1.0, 1.0],
    [0.0, -1.0, 1.0],
    [0.0, 1.0, -1.0],
    [0.0, -1.0, -1.0],
];

/// Gradient noise in about -1..1.
fn gradient2(seed: u32, x: f64, z: f64) -> f64 {
    let (x0, z0) = (x.floor(), z.floor());
    let (fx, fz) = (x - x0, z - z0);
    let (ix, iz) = (x0 as i32, z0 as i32);
    let dot = |cx: i32, cz: i32, dx: f64, dz: f64| {
        let g = G2[(hash(seed, ix + cx, iz + cz, 0) & 7) as usize];
        g[0] * dx + g[1] * dz
    };
    let (u, v) = (fade(fx), fade(fz));
    let a = lerp(dot(0, 0, fx, fz), dot(1, 0, fx - 1.0, fz), u);
    let b = lerp(dot(0, 1, fx, fz - 1.0), dot(1, 1, fx - 1.0, fz - 1.0), u);
    (lerp(a, b, v) * 1.414).clamp(-1.0, 1.0)
}

fn gradient3(seed: u32, x: f64, y: f64, z: f64) -> f64 {
    let (x0, y0, z0) = (x.floor(), y.floor(), z.floor());
    let (fx, fy, fz) = (x - x0, y - y0, z - z0);
    let (ix, iy, iz) = (x0 as i32, y0 as i32, z0 as i32);
    let dot = |cx: i32, cy: i32, cz: i32| {
        let g = G3[(hash(seed, ix + cx, iy + cy, iz + cz) % 12) as usize];
        g[0] * (fx - f64::from(cx)) + g[1] * (fy - f64::from(cy)) + g[2] * (fz - f64::from(cz))
    };
    let (u, v, w) = (fade(fx), fade(fy), fade(fz));
    let x00 = lerp(dot(0, 0, 0), dot(1, 0, 0), u);
    let x10 = lerp(dot(0, 1, 0), dot(1, 1, 0), u);
    let x01 = lerp(dot(0, 0, 1), dot(1, 0, 1), u);
    let x11 = lerp(dot(0, 1, 1), dot(1, 1, 1), u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w).clamp(-1.0, 1.0)
}

impl Fbm {
    fn sample(&self, x: f64, y: Option<f64>, z: f64) -> f64 {
        let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0 / self.scale, 0.0);
        for o in 0..self.octaves {
            let s = self.seed.wrapping_add(o.wrapping_mul(0x9e37_79b9));
            sum += amp
                * match y {
                    None => gradient2(s, x * freq, z * freq),
                    Some(y) => gradient3(s, x * freq, y * freq, z * freq),
                };
            norm += amp;
            amp *= self.persistence;
            freq *= self.lacunarity;
        }
        sum / norm.max(1e-9)
    }
}

/// Per-column values of nodes that don't depend on y.
struct Cache(Vec<f64>);

impl Cache {
    fn new(n: usize) -> Cache {
        Cache(vec![f64::NAN; n])
    }
}

impl Graph {
    fn eval(&self, id: Id, p: [f64; 3], cache: &mut Cache) -> f64 {
        if !self.depends_y[id] {
            let v = cache.0[id];
            if !v.is_nan() {
                return v;
            }
        }
        let v = self.eval_raw(id, p, cache);
        if !self.depends_y[id] {
            cache.0[id] = v;
        }
        v
    }

    fn eval_raw(&self, id: Id, p: [f64; 3], c: &mut Cache) -> f64 {
        let [x, y, z] = p;
        match &self.d[id] {
            D::Constant(v) => *v,
            D::X => x,
            D::Y => y,
            D::Z => z,
            D::BaseHeight(base, distance) => {
                if *distance {
                    y - base
                } else {
                    *base
                }
            }
            D::Noise2(f) => f.sample(x, None, z),
            D::Noise3(f) => f.sample(x, Some(y), z),
            D::Cell2 {
                scale,
                jitter,
                seed,
                value,
            } => {
                let (cx, cz) = (x / scale, z / scale);
                let (ix, iz) = (cx.floor() as i32, cz.floor() as i32);
                let mut best = (f64::MAX, 0u32);
                for dx in -1..=1 {
                    for dz in -1..=1 {
                        let h = hash(*seed, ix + dx, iz + dz, 1);
                        let px = f64::from(ix + dx) + 0.5 + (unit(h) - 0.5) * jitter;
                        let pz =
                            f64::from(iz + dz) + 0.5 + (unit(h.rotate_left(16)) - 0.5) * jitter;
                        let d = ((px - cx).powi(2) + (pz - cz).powi(2)).sqrt();
                        if d < best.0 {
                            best = (d, h);
                        }
                    }
                }
                if *value {
                    unit(best.1.wrapping_mul(0x2c1b_3c6d)) * 2.0 - 1.0
                } else {
                    best.0
                }
            }
            D::Sum(i) => i.iter().map(|i| self.eval(*i, p, c)).sum(),
            D::Multiply(i) => i.iter().map(|i| self.eval(*i, p, c)).product(),
            D::Min(i) => i
                .iter()
                .map(|i| self.eval(*i, p, c))
                .fold(f64::MAX, f64::min),
            D::Max(i) => i
                .iter()
                .map(|i| self.eval(*i, p, c))
                .fold(f64::MIN, f64::max),
            D::Abs(i) => self.eval(*i, p, c).abs(),
            D::Pow(i, e) => {
                let v = self.eval(*i, p, c);
                v.signum() * v.abs().powf(*e)
            }
            D::Clamp(i, lo, hi) => self.eval(*i, p, c).clamp(*lo, *hi),
            D::Normalize(i, [a, b, to_a, to_b]) => {
                let v = self.eval(*i, p, c);
                to_a + (v - a) / (b - a) * (to_b - to_a)
            }
            D::Curve(i, points) => {
                let v = self.eval(*i, p, c);
                curve(points, v)
            }
            D::Select(cond, then, other) => {
                if self.eval(*cond, p, c) != 0.0 {
                    self.eval(*then, p, c)
                } else {
                    self.eval(*other, p, c)
                }
            }
            D::Less(a, b) => f64::from(u8::from(self.eval(*a, p, c) < self.eval(*b, p, c))),
            D::Greater(a, b) => f64::from(u8::from(self.eval(*a, p, c) > self.eval(*b, p, c))),
            D::Mix(a, b, t) => {
                let t = self.eval(*t, p, c);
                lerp(self.eval(*a, p, c), self.eval(*b, p, c), t)
            }
            D::Scale(i, s) => self.eval(*i, [x * s[0], y * s[1], z * s[2]], c),
            D::Offset(i, o) => self.eval(*i, [x + o[0], y + o[1], z + o[2]], c),
            D::YOverride(i, v) => self.eval(*i, [x, *v, z], c),
            D::Warp(i, f, strength) => {
                let dx = f.sample(x, None, z) * strength;
                let dz = f.sample(x + 517.0, None, z - 211.0) * strength;
                self.eval(*i, [x + dx, y, z + dz], c)
            }
        }
    }

    /// The biome weights at a column: the centre's biome plus blending.
    fn biome_weights(&self, x: i32, z: i32) -> (usize, Vec<(usize, f64)>) {
        let at = |x: f64, z: f64| {
            let v = self.eval(self.biome_map, [x, 0.0, z], &mut Cache::new(self.d.len()));
            self.biomes
                .iter()
                .position(|b| v >= b.min && v < b.max)
                .unwrap_or(self.default_biome)
        };
        let (fx, fz) = (f64::from(x), f64::from(z));
        let centre = at(fx, fz);
        if self.transition == 0 || self.biomes.len() < 2 {
            return (centre, vec![(centre, 1.0)]);
        }
        let r = f64::from(self.transition);
        let mut weights: BTreeMap<usize, f64> = BTreeMap::new();
        *weights.entry(centre).or_default() += 1.0;
        for k in 0..8 {
            for ring in [0.5, 1.0] {
                let a = f64::from(k) * std::f64::consts::FRAC_PI_4;
                let b = at(fx + a.cos() * r * ring, fz + a.sin() * r * ring);
                *weights.entry(b).or_default() += 1.0 - ring * 0.5;
            }
        }
        let total: f64 = weights.values().sum();
        (
            centre,
            weights.into_iter().map(|(b, w)| (b, w / total)).collect(),
        )
    }

    fn material(&self, id: usize, ctx: &Here, cache: &mut Cache) -> Option<Block> {
        match &self.m[id] {
            M::Constant(b) => Some(*b),
            M::Queue(list) => list.iter().find_map(|m| self.material(*m, ctx, cache)),
            M::Solidity(solid, empty) => {
                let pick = if ctx.solid { solid } else { empty };
                pick.and_then(|m| self.material(m, ctx, cache))
            }
            M::Depth {
                layers,
                min_space,
                ceiling,
            } => {
                if !ctx.solid || (!*ceiling && ctx.space < *min_space) {
                    return None;
                }
                let mut depth = if *ceiling { ctx.depth_up } else { ctx.depth };
                for (thickness, m) in layers {
                    if depth <= *thickness {
                        return self.material(*m, ctx, cache);
                    }
                    depth -= thickness;
                }
                None
            }
            M::Range(d, ranges) => {
                let v = self.eval(*d, ctx.p, cache);
                ranges
                    .iter()
                    .find(|(a, b, _)| v >= *a && v < *b)
                    .and_then(|(_, _, m)| self.material(*m, ctx, cache))
            }
            M::Horizontal(bottom, top, m) => {
                (ctx.p[1] >= *bottom && ctx.p[1] < *top).then_some(())?;
                self.material(*m, ctx, cache)
            }
            M::Weighted(seed, entries) => {
                let total: f64 = entries.iter().map(|(w, _)| w).sum();
                let [x, y, z] = ctx.p.map(|v| v as i32);
                let mut roll = unit(hash(*seed, x, y, z)) * total;
                for (w, m) in entries {
                    if roll < *w {
                        return self.material(*m, ctx, cache);
                    }
                    roll -= w;
                }
                None
            }
        }
    }

    fn terrain_column(&self, seed: u32, x: i32, z: i32, height: i32) -> Arc<Column> {
        let key = (seed, x, z, height);
        if let Some(c) = self.terrain.lock().unwrap().get(&key) {
            return c.clone();
        }
        let (centre, weights) = self.biome_weights(x, z);
        let mut cache = Cache::new(self.d.len());
        let h = height.max(0) as usize;
        let solid: Vec<bool> = (0..h)
            .map(|y| {
                let p = [f64::from(x), y as f64, f64::from(z)];
                weights
                    .iter()
                    .map(|(b, w)| w * self.eval(self.biomes[*b].terrain, p, &mut cache))
                    .sum::<f64>()
                    > 0.0
            })
            .collect();
        // Depth into the floor counts down from each run's top; space is
        // the air above that run.
        let mut depth = vec![0; h];
        let mut space = vec![0; h];
        let (mut run, mut air) = (0, i32::MAX / 2);
        for y in (0..h).rev() {
            if solid[y] {
                run += 1;
                depth[y] = run;
                space[y] = air;
            } else {
                if run > 0 {
                    air = 0;
                }
                run = 0;
                air += 1;
            }
        }
        let mut depth_up = vec![0; h];
        let mut run = 0;
        for y in 0..h {
            run = if solid[y] { run + 1 } else { 0 };
            depth_up[y] = run;
        }
        let material = self.biomes[centre].material;
        let blocks: Vec<Block> = (0..h)
            .map(|y| {
                let here = Here {
                    p: [f64::from(x), y as f64, f64::from(z)],
                    solid: solid[y],
                    depth: depth[y],
                    depth_up: depth_up[y],
                    space: space[y],
                };
                self.material(material, &here, &mut cache).unwrap_or(0)
            })
            .collect();
        let top = (0..h).rev().find(|y| solid[*y]).map_or(-1, |y| y as i32);
        let column = Arc::new(Column { blocks, top });
        let mut map = self.terrain.lock().unwrap();
        if map.len() >= CACHED_COLUMNS {
            map.pop_first();
        }
        map.insert(key, column.clone());
        column
    }

    /// Every cell of one column, props included.
    pub fn column(&self, seed: u32, x: i32, z: i32, height: i32) -> Arc<Vec<Block>> {
        let key = (seed, x, z, height);
        if let Some(c) = self.columns.lock().unwrap().get(&key) {
            return c.clone();
        }
        let mut blocks = self.terrain_column(seed, x, z, height).blocks.clone();
        let mut write = |cell: [i32; 3], block: Block| {
            if cell[0] == x && cell[2] == z && (0..height).contains(&cell[1]) {
                let at = cell[1] as usize;
                if blocks[at] == 0 {
                    blocks[at] = block;
                }
            }
        };
        let reach = self
            .biomes
            .iter()
            .flat_map(|b| &b.props)
            .map(|r| (r, r.extent))
            .collect::<Vec<_>>();
        for (n, (rule, extent)) in reach.into_iter().enumerate() {
            let rule_seed = name_seed(seed, &format!("prop{n}"));
            for anchor in rule.anchors(rule_seed, x - extent, z - extent, x + extent, z + extent) {
                self.place(seed, rule, n, anchor, height, &mut write);
            }
        }
        let column = Arc::new(blocks);
        let mut map = self.columns.lock().unwrap();
        if map.len() >= CACHED_COLUMNS {
            map.pop_first();
        }
        map.insert(key, column.clone());
        column
    }

    fn place(
        &self,
        seed: u32,
        rule: &PropRule,
        n: usize,
        [ax, az]: [i32; 2],
        height: i32,
        write: &mut impl FnMut([i32; 3], Block),
    ) {
        // A prop belongs to the biome at its anchor.
        let (biome, _) = self.biome_weights(ax, az);
        if !self.biomes[biome]
            .props
            .iter()
            .any(|r| std::ptr::eq(r, rule))
        {
            return;
        }
        let h = hash(name_seed(seed, "props"), ax, n as i32, az);
        if unit(h) >= rule.chance {
            return;
        }
        let ground = self.terrain_column(seed, ax, az, height);
        if ground.top < 0 || ground.top + 1 >= height {
            return;
        }
        if !rule.on.is_empty() && !rule.on.contains(&ground.blocks[ground.top as usize]) {
            return;
        }
        let anchor = [ax, ground.top + 1, az];
        if let Some(gate) = rule.gate {
            let p = anchor.map(f64::from);
            if self.eval(gate, p, &mut Cache::new(self.d.len())) <= 0.0 {
                return;
            }
        }
        self.prop(rule.prop, anchor, h, write);
    }

    fn prop(&self, id: usize, a: [i32; 3], h: u32, write: &mut impl FnMut([i32; 3], Block)) {
        let span = |(lo, hi): (i32, i32), h: u32| lo + (h % (hi - lo + 1).max(1) as u32) as i32;
        match &self.p[id] {
            P::Cuboid(from, to, b) => {
                for x in from[0]..=to[0] {
                    for y in from[1]..=to[1] {
                        for z in from[2]..=to[2] {
                            write([a[0] + x, a[1] + y, a[2] + z], *b);
                        }
                    }
                }
            }
            P::Sphere(r, offset, b) => {
                let c = [0, 1, 2].map(|i| a[i] + offset[i]);
                let n = r.ceil() as i32;
                for x in -n..=n {
                    for y in -n..=n {
                        for z in -n..=n {
                            if f64::from(x * x + y * y + z * z) <= r * r {
                                write([c[0] + x, c[1] + y, c[2] + z], *b);
                            }
                        }
                    }
                }
            }
            P::Tree {
                trunk,
                leaves,
                height,
                radius,
            } => {
                let tall = span(*height, h);
                for y in 0..tall {
                    write([a[0], a[1] + y, a[2]], *trunk);
                }
                let top = a[1] + tall;
                for y in -2..=1 {
                    let r = if y == 1 { radius - 1 } else { *radius };
                    for x in -r..=r {
                        for z in -r..=r {
                            let corner = x.abs() == r && z.abs() == r;
                            if !corner || hash(h, x, y, z) & 1 == 0 {
                                write([a[0] + x, top + y, a[2] + z], *leaves);
                            }
                        }
                    }
                }
            }
            P::Column(b, height) => {
                for y in 0..span(*height, h) {
                    write([a[0], a[1] + y, a[2]], *b);
                }
            }
            P::Weighted(entries) => {
                let total: f64 = entries.iter().map(|(w, _)| w).sum();
                let mut roll = unit(h.rotate_left(7)) * total;
                for (w, p) in entries {
                    if roll < *w {
                        return self.prop(*p, a, h.rotate_left(13), write);
                    }
                    roll -= w;
                }
            }
        }
    }

    /// The highest solid terrain cell in a column, before props.
    pub fn surface(&self, seed: u32, x: i32, z: i32, height: i32) -> i32 {
        self.terrain_column(seed, x, z, height).top
    }

    #[cfg(test)]
    pub fn biome_at(&self, x: i32, z: i32) -> &str {
        &self.biomes[self.biome_weights(x, z).0].name
    }
}

struct Here {
    p: [f64; 3],
    solid: bool,
    depth: i32,
    depth_up: i32,
    space: i32,
}

fn curve(points: &[(f64, f64)], v: f64) -> f64 {
    if v <= points[0].0 {
        return points[0].1;
    }
    for w in points.windows(2) {
        if v <= w[1].0 {
            let t = (v - w[0].0) / (w[1].0 - w[0].0).max(1e-12);
            return lerp(w[0].1, w[1].1, t);
        }
    }
    points[points.len() - 1].1
}

impl PropRule {
    fn anchors(&self, seed: u32, x0: i32, z0: i32, x1: i32, z1: i32) -> Vec<[i32; 2]> {
        match &self.positions {
            Positions::List(points) => points
                .iter()
                .copied()
                .filter(|[x, z]| (x0..=x1).contains(x) && (z0..=z1).contains(z))
                .collect(),
            Positions::Grid {
                spacing,
                jitter,
                seed: own,
                triangular,
            } => {
                let s = *spacing;
                let seed = seed ^ own;
                let mut out = Vec::new();
                for j in (z0.div_euclid(s) - 1)..=(z1.div_euclid(s) + 1) {
                    let shift = if *triangular && j % 2 != 0 { s / 2 } else { 0 };
                    for i in ((x0 - shift).div_euclid(s) - 1)..=((x1 - shift).div_euclid(s) + 1) {
                        let h = hash(seed, i, j, 7);
                        let wobble =
                            |h: u32| ((unit(h) - 0.5) * jitter * f64::from(s)).round() as i32;
                        let x = i * s + shift + wobble(h);
                        let z = j * s + wobble(h.rotate_left(16));
                        if (x0..=x1).contains(&x) && (z0..=z1).contains(&z) {
                            out.push([x, z]);
                        }
                    }
                }
                out.sort();
                out.dedup();
                out
            }
        }
    }
}

/// Reads a `.blockgen` document, resolving block names through `block`.
pub fn parse(
    text: &str,
    seed: i64,
    block: &dyn Fn(&str) -> Result<Block, String>,
) -> Result<Graph, String> {
    let doc: Value = blockloom_plugin_sdk::serde_json::from_str(text)
        .map_err(|e| format!("not a worldgen graph ({e})"))?;
    if doc["version"].as_u64().unwrap_or(1) != 1 {
        return Err("this worldgen graph is a newer version".into());
    }
    let mut b = Builder {
        seed: seed as u32 ^ (seed >> 32) as u32,
        constants: BTreeMap::new(),
        exports: doc["exports"].as_object().cloned().unwrap_or_default(),
        importing: Vec::new(),
        d: Vec::new(),
        m: Vec::new(),
        p: Vec::new(),
        block,
    };
    if let Some(constants) = doc["constants"].as_object() {
        for (name, v) in constants {
            let v = v
                .as_f64()
                .ok_or_else(|| format!("constant {name} is a number"))?;
            b.constants.insert(name.clone(), v);
        }
    }
    let biome_map = match doc.get("biome_map") {
        Some(v) if !v.is_null() => b.density(v)?,
        _ => b.push(D::Constant(0.0))?,
    };
    let list = doc["biomes"]
        .as_array()
        .filter(|l| !l.is_empty() && l.len() <= 64)
        .ok_or("a worldgen graph lists 1 to 64 biomes")?;
    let mut biomes = Vec::new();
    for (n, v) in list.iter().enumerate() {
        let name = v["name"].as_str().unwrap_or("").to_string();
        if name.is_empty() {
            return Err(format!("biome {} needs a name", n + 1));
        }
        let at = |e: String| format!("biome {name}: {e}");
        let terrain = b.density(&v["terrain"]).map_err(at)?;
        let material = b.material(&v["materials"]).map_err(at)?;
        let mut props = Vec::new();
        for rule in v["props"].as_array().into_iter().flatten() {
            props.push(b.prop_rule(rule).map_err(at)?);
        }
        biomes.push(Biome {
            min: v["min"].as_f64().unwrap_or(f64::MIN),
            max: v["max"].as_f64().unwrap_or(f64::MAX),
            name,
            terrain,
            material,
            props,
        });
    }
    let default_biome = match doc["default_biome"].as_str() {
        None => 0,
        Some(name) => biomes
            .iter()
            .position(|bm| bm.name == name)
            .ok_or_else(|| format!("default biome {name} is not listed"))?,
    };
    let transition = doc["transition"].as_i64().unwrap_or(8);
    if !(0..=64).contains(&transition) {
        return Err("transition is 0 to 64 cells".into());
    }
    let depends_y = (0..b.d.len()).map(|i| depends_y(&b.d, i)).collect();
    Ok(Graph {
        depends_y,
        d: b.d,
        m: b.m,
        p: b.p,
        biomes,
        default_biome,
        biome_map,
        transition: transition as i32,
        terrain: Mutex::new(BTreeMap::new()),
        columns: Mutex::new(BTreeMap::new()),
    })
}

fn depends_y(d: &[D], i: Id) -> bool {
    match &d[i] {
        D::Y | D::Noise3(_) => true,
        D::BaseHeight(_, distance) => *distance,
        D::YOverride(..) => false,
        D::Constant(_) | D::X | D::Z | D::Noise2(_) | D::Cell2 { .. } => false,
        D::Sum(v) | D::Multiply(v) | D::Min(v) | D::Max(v) => v.iter().any(|c| depends_y(d, *c)),
        D::Abs(c)
        | D::Pow(c, _)
        | D::Clamp(c, ..)
        | D::Normalize(c, _)
        | D::Curve(c, _)
        | D::Scale(c, _)
        | D::Offset(c, _)
        | D::Warp(c, ..) => depends_y(d, *c),
        D::Select(a, b, c) | D::Mix(a, b, c) => [a, b, c].iter().any(|c| depends_y(d, **c)),
        D::Less(a, b) | D::Greater(a, b) => depends_y(d, *a) || depends_y(d, *b),
    }
}

struct Builder<'a> {
    seed: u32,
    constants: BTreeMap<String, f64>,
    exports: blockloom_plugin_sdk::serde_json::Map<String, Value>,
    importing: Vec<String>,
    d: Vec<D>,
    m: Vec<M>,
    p: Vec<P>,
    block: &'a dyn Fn(&str) -> Result<Block, String>,
}

fn num(v: &Value, key: &str, default: f64) -> Result<f64, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(n) => n
            .as_f64()
            .filter(|n| n.is_finite())
            .ok_or_else(|| format!("{key} is a number")),
    }
}

fn triple(v: &Value, key: &str, default: f64) -> Result<[f64; 3], String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok([default; 3]),
        Some(Value::Number(n)) => Ok([n.as_f64().unwrap_or(default); 3]),
        Some(Value::Array(a)) if a.len() == 3 => {
            let mut out = [0.0; 3];
            for (o, n) in out.iter_mut().zip(a) {
                *o = n.as_f64().ok_or_else(|| format!("{key} holds numbers"))?;
            }
            Ok(out)
        }
        _ => Err(format!("{key} is a number or [x, y, z]")),
    }
}

fn cells(v: &Value, key: &str) -> Result<[i32; 3], String> {
    Ok(triple(v, key, 0.0)?.map(|n| n.round() as i32))
}

fn range(v: &Value, key: &str, default: (i32, i32)) -> Result<(i32, i32), String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Number(n)) => {
            let n = n.as_i64().unwrap_or(0) as i32;
            Ok((n, n))
        }
        Some(Value::Array(a)) if a.len() == 2 => {
            let lo = a[0].as_i64().ok_or("a range is two whole numbers")? as i32;
            let hi = a[1].as_i64().ok_or("a range is two whole numbers")? as i32;
            Ok((lo.min(hi), lo.max(hi)))
        }
        _ => Err(format!("{key} is a whole number or [min, max]")),
    }
}

impl Builder<'_> {
    fn push(&mut self, node: D) -> Result<Id, String> {
        if self.d.len() >= MAX_NODES {
            return Err(format!("a worldgen graph holds at most {MAX_NODES} nodes"));
        }
        self.d.push(node);
        Ok(self.d.len() - 1)
    }

    fn seed_of(&self, v: &Value) -> u32 {
        match v.get("seed") {
            Some(Value::String(s)) => name_seed(self.seed, s),
            Some(Value::Number(n)) => name_seed(self.seed, &n.to_string()),
            _ => name_seed(self.seed, "default"),
        }
    }

    fn fbm(&self, v: &Value) -> Result<Fbm, String> {
        let scale = num(v, "scale", 64.0)?;
        let octaves = num(v, "octaves", 1.0)?;
        if scale <= 0.0 || !(1.0..=8.0).contains(&octaves) {
            return Err("noise needs a positive scale and 1 to 8 octaves".into());
        }
        Ok(Fbm {
            scale,
            octaves: octaves as u32,
            persistence: num(v, "persistence", 0.5)?,
            lacunarity: num(v, "lacunarity", 2.0)?,
            seed: self.seed_of(v),
        })
    }

    fn inputs(&mut self, v: &Value, want: usize) -> Result<Vec<Id>, String> {
        let list: Vec<Value> = match (v.get("inputs"), v.get("input")) {
            (Some(Value::Array(list)), _) => list.clone(),
            (_, Some(one)) if !one.is_null() => vec![one.clone()],
            _ => Vec::new(),
        };
        if want > 0 && list.len() != want {
            let kind = v["type"].as_str().unwrap_or("");
            return Err(format!("{kind} takes {want} input(s)"));
        }
        if want == 0 && list.is_empty() {
            let kind = v["type"].as_str().unwrap_or("");
            return Err(format!("{kind} needs inputs"));
        }
        list.iter().map(|i| self.density(i)).collect()
    }

    fn density(&mut self, v: &Value) -> Result<Id, String> {
        if let Some(n) = v.as_f64() {
            return self.push(D::Constant(n));
        }
        let kind = v["type"]
            .as_str()
            .ok_or("a density node is a number or an object with a type")?;
        let one = |b: &mut Self| b.inputs(v, 1).map(|i| i[0]);
        let node = match kind {
            "constant" => D::Constant(num(v, "value", 0.0)?),
            "x" => D::X,
            "y" => D::Y,
            "z" => D::Z,
            "base_height" => {
                let name = v["name"].as_str().unwrap_or("base");
                let base = *self
                    .constants
                    .get(name)
                    .ok_or_else(|| format!("no constant called {name}"))?;
                D::BaseHeight(base, v["distance"].as_bool().unwrap_or(true))
            }
            "noise2d" => D::Noise2(self.fbm(v)?),
            "noise3d" => D::Noise3(self.fbm(v)?),
            "cell2d" => D::Cell2 {
                scale: num(v, "scale", 64.0)?.max(1e-6),
                jitter: num(v, "jitter", 1.0)?.clamp(0.0, 1.0),
                seed: self.seed_of(v),
                value: v["returns"].as_str() == Some("value"),
            },
            "sum" => D::Sum(self.inputs(v, 0)?),
            "multiply" => D::Multiply(self.inputs(v, 0)?),
            "min" => D::Min(self.inputs(v, 0)?),
            "max" => D::Max(self.inputs(v, 0)?),
            "abs" => D::Abs(one(self)?),
            "pow" => D::Pow(one(self)?, num(v, "exponent", 2.0)?),
            "clamp" => D::Clamp(one(self)?, num(v, "min", -1.0)?, num(v, "max", 1.0)?),
            "normalize" => {
                let [a, b] = [num(v, "from_min", -1.0)?, num(v, "from_max", 1.0)?];
                if a == b {
                    return Err("normalize needs from_min and from_max to differ".into());
                }
                D::Normalize(
                    one(self)?,
                    [a, b, num(v, "to_min", 0.0)?, num(v, "to_max", 1.0)?],
                )
            }
            "curve" => {
                let points = v["points"]
                    .as_array()
                    .filter(|p| (2..=32).contains(&p.len()))
                    .ok_or("a curve has 2 to 32 points")?
                    .iter()
                    .map(|p| {
                        match (
                            p.get(0).and_then(Value::as_f64),
                            p.get(1).and_then(Value::as_f64),
                        ) {
                            (Some(i), Some(o)) => Ok((i, o)),
                            _ => Err("a curve point is [in, out]".to_string()),
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let mut points = points;
                points.sort_by(|a, b| a.0.total_cmp(&b.0));
                D::Curve(one(self)?, points)
            }
            "select" => {
                let i = self.inputs(v, 3)?;
                D::Select(i[0], i[1], i[2])
            }
            "less" | "greater" => {
                let i = self.inputs(v, 2)?;
                if kind == "less" {
                    D::Less(i[0], i[1])
                } else {
                    D::Greater(i[0], i[1])
                }
            }
            "mix" => {
                let i = self.inputs(v, 3)?;
                D::Mix(i[0], i[1], i[2])
            }
            "scale" => {
                let s = triple(v, "scale", 1.0)?;
                D::Scale(one(self)?, s)
            }
            "offset" => {
                let o = triple(v, "offset", 0.0)?;
                D::Offset(one(self)?, o)
            }
            "y_override" => D::YOverride(one(self)?, num(v, "value", 0.0)?),
            "warp" => {
                let f = self.fbm(v)?;
                D::Warp(one(self)?, f, num(v, "strength", 8.0)?)
            }
            "import" => {
                let name = v["name"].as_str().unwrap_or("").to_string();
                if self.importing.contains(&name) {
                    return Err(format!("export {name} imports itself"));
                }
                let tree = self
                    .exports
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| format!("no export called {name}"))?;
                // Each import is its own copy, so every node has one path.
                self.importing.push(name);
                let id = self.density(&tree);
                self.importing.pop();
                return id;
            }
            other => return Err(format!("no density node called {other}")),
        };
        self.push(node)
    }

    fn block(&self, v: &Value) -> Result<Block, String> {
        let name = v.as_str().ok_or("a material names a block")?;
        if name.eq_ignore_ascii_case("air") || name.eq_ignore_ascii_case("empty") {
            return Ok(0);
        }
        (self.block)(name)
    }

    fn material(&mut self, v: &Value) -> Result<usize, String> {
        let node = if v.is_string() {
            M::Constant(self.block(v)?)
        } else {
            let kind = v["type"]
                .as_str()
                .ok_or("a material is a block name or a typed provider")?;
            let sub = |b: &mut Self, key: &str| -> Result<Option<usize>, String> {
                match v.get(key) {
                    None | Some(Value::Null) => Ok(None),
                    Some(m) => b.material(m).map(Some),
                }
            };
            match kind {
                "constant" => M::Constant(self.block(&v["material"])?),
                "queue" => {
                    let list = v["queue"]
                        .as_array()
                        .ok_or("a queue lists providers")?
                        .clone();
                    M::Queue(
                        list.iter()
                            .map(|m| self.material(m))
                            .collect::<Result<_, _>>()?,
                    )
                }
                "solidity" => M::Solidity(sub(self, "solid")?, sub(self, "empty")?),
                "depth" => {
                    let mut layers = Vec::new();
                    for layer in v["layers"].as_array().ok_or("depth lists its layers")? {
                        let t = layer["thickness"]
                            .as_i64()
                            .filter(|t| (1..=256).contains(t));
                        let t = t.ok_or("a layer is 1 to 256 cells thick")? as i32;
                        layers.push((t, self.material(&layer["material"])?));
                    }
                    M::Depth {
                        layers,
                        min_space: num(v, "min_space", 0.0)? as i32,
                        ceiling: v["from"].as_str() == Some("ceiling"),
                    }
                }
                "range" => {
                    let d = self.density(&v["density"])?;
                    let mut ranges = Vec::new();
                    for r in v["ranges"].as_array().ok_or("range lists its ranges")? {
                        ranges.push((
                            num(r, "from", f64::MIN)?,
                            num(r, "to", f64::MAX)?,
                            self.material(&r["material"])?,
                        ));
                    }
                    M::Range(d, ranges)
                }
                "horizontal" => {
                    let base = match v["base"].as_str() {
                        None => 0.0,
                        Some(name) => *self
                            .constants
                            .get(name)
                            .ok_or_else(|| format!("no constant called {name}"))?,
                    };
                    let m = self.material(&v["material"])?;
                    M::Horizontal(
                        base + num(v, "bottom", f64::MIN)?,
                        base + num(v, "top", f64::MAX)?,
                        m,
                    )
                }
                "weighted" => {
                    let seed = self.seed_of(v);
                    let mut entries = Vec::new();
                    for e in v["entries"]
                        .as_array()
                        .ok_or("weighted lists its entries")?
                    {
                        entries.push((
                            num(e, "weight", 1.0)?.max(0.0),
                            self.material(&e["material"])?,
                        ));
                    }
                    M::Weighted(seed, entries)
                }
                other => return Err(format!("no material provider called {other}")),
            }
        };
        if self.m.len() >= MAX_NODES {
            return Err(format!(
                "a worldgen graph holds at most {MAX_NODES} providers"
            ));
        }
        self.m.push(node);
        Ok(self.m.len() - 1)
    }

    /// A prop and how far it reaches from its anchor.
    fn prop(&mut self, v: &Value) -> Result<(usize, i32), String> {
        let kind = v["type"].as_str().ok_or("a prop has a type")?;
        let (node, extent) = match kind {
            "cuboid" => {
                let (from, to) = (cells(v, "from")?, cells(v, "to")?);
                let lo = [0, 1, 2].map(|a| from[a].min(to[a]));
                let hi = [0, 1, 2].map(|a| from[a].max(to[a]));
                let reach = [lo[0], hi[0], lo[2], hi[2]]
                    .iter()
                    .map(|c| c.abs())
                    .max()
                    .unwrap();
                (P::Cuboid(lo, hi, self.block(&v["material"])?), reach)
            }
            "sphere" => {
                let r = num(v, "radius", 2.0)?;
                let offset = cells(v, "offset")?;
                let reach = r.ceil() as i32 + offset[0].abs().max(offset[2].abs());
                (P::Sphere(r, offset, self.block(&v["material"])?), reach)
            }
            "tree" => {
                let radius = num(v, "radius", 2.0)? as i32;
                (
                    P::Tree {
                        trunk: self.block(&v["trunk"])?,
                        leaves: self.block(&v["leaves"])?,
                        height: range(v, "height", (4, 6))?,
                        radius: radius.max(1),
                    },
                    radius.max(1),
                )
            }
            "column" => (
                P::Column(self.block(&v["material"])?, range(v, "height", (1, 3))?),
                0,
            ),
            "weighted" => {
                let mut entries = Vec::new();
                let mut reach = 0;
                for e in v["entries"]
                    .as_array()
                    .ok_or("weighted lists its entries")?
                {
                    let (p, r) = self.prop(&e["prop"])?;
                    reach = reach.max(r);
                    entries.push((num(e, "weight", 1.0)?.max(0.0), p));
                }
                (P::Weighted(entries), reach)
            }
            other => return Err(format!("no prop called {other}")),
        };
        if extent > MAX_EXTENT {
            return Err(format!(
                "a prop reaches at most {MAX_EXTENT} cells from its anchor"
            ));
        }
        if self.p.len() >= MAX_NODES {
            return Err(format!("a worldgen graph holds at most {MAX_NODES} props"));
        }
        self.p.push(node);
        Ok((self.p.len() - 1, extent))
    }

    fn prop_rule(&mut self, v: &Value) -> Result<PropRule, String> {
        let pos = &v["positions"];
        let positions = match pos["type"].as_str().unwrap_or("grid") {
            kind @ ("grid" | "triangular") => {
                let spacing = num(pos, "spacing", 16.0)? as i32;
                if !(1..=512).contains(&spacing) {
                    return Err("positions are 1 to 512 cells apart".into());
                }
                Positions::Grid {
                    spacing,
                    jitter: num(pos, "jitter", 0.0)?.clamp(0.0, 1.0),
                    seed: self.seed_of(pos),
                    triangular: kind == "triangular",
                }
            }
            "list" => Positions::List(
                pos["points"]
                    .as_array()
                    .ok_or("a list names its points")?
                    .iter()
                    .map(|p| {
                        match (
                            p.get(0).and_then(Value::as_i64),
                            p.get(1).and_then(Value::as_i64),
                        ) {
                            (Some(x), Some(z)) => Ok([x as i32, z as i32]),
                            _ => Err("a point is [x, z]".to_string()),
                        }
                    })
                    .collect::<Result<_, _>>()?,
            ),
            other => return Err(format!("no positions called {other}")),
        };
        let on = v["on"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|b| self.block(b))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let gate = match v.get("where") {
            None | Some(Value::Null) => None,
            Some(d) => Some(self.density(d)?),
        };
        let (prop, extent) = self.prop(&v["prop"])?;
        Ok(PropRule {
            positions,
            chance: num(v, "chance", 1.0)?.clamp(0.0, 1.0),
            on,
            gate,
            prop,
            extent,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(name: &str) -> Result<Block, String> {
        match name {
            "stone" => Ok(1),
            "dirt" => Ok(2),
            "grass" => Ok(3),
            "sand" => Ok(4),
            "wood" => Ok(5),
            "leaves" => Ok(6),
            "water" => Ok(20),
            other => Err(format!("no block called {other}")),
        }
    }

    const PLAINS: &str = r#"{
      "constants": {"base": 20, "water": 18},
      "exports": {"hills": {"type": "noise2d", "scale": 40, "octaves": 3, "seed": "hills"}},
      "biome_map": {"type": "noise2d", "scale": 200, "seed": "biomes"},
      "default_biome": "plains",
      "transition": 6,
      "biomes": [
        {"name": "plains", "min": -2, "max": 0.1,
         "terrain": {"type": "sum", "inputs": [
            {"type": "multiply", "inputs": [{"type": "import", "name": "hills"}, 4]},
            {"type": "curve", "points": [[-10, 10], [10, -10]],
             "input": {"type": "base_height", "name": "base"}}]},
         "materials": {"type": "solidity",
            "solid": {"type": "queue", "queue": [
               {"type": "depth", "layers": [{"thickness": 1, "material": "grass"}, {"thickness": 3, "material": "dirt"}]},
               "stone"]},
            "empty": {"type": "horizontal", "base": "water", "bottom": 0, "top": 1, "material": "water"}},
         "props": [{"positions": {"type": "triangular", "spacing": 9, "jitter": 0.6, "seed": "trees"},
                    "on": ["grass"], "chance": 0.8,
                    "prop": {"type": "tree", "trunk": "wood", "leaves": "leaves", "height": [4, 5], "radius": 2}}]},
        {"name": "desert", "min": 0.1, "max": 2,
         "terrain": {"type": "curve", "points": [[-10, 10], [10, -10]],
             "input": {"type": "base_height", "name": "base"}},
         "materials": {"type": "solidity", "solid": "sand"}}
      ]
    }"#;

    #[test]
    fn a_graph_layers_its_terrain_and_grows_props_on_grass() {
        let g = parse(PLAINS, 42, &blocks).unwrap();
        let mut grass = 0;
        let mut trees = 0;
        for x in 0..64 {
            for z in 0..64 {
                let col = g.column(1, x, z, 48);
                let top = g.surface(1, x, z, 48);
                assert!((10..30).contains(&top), "{top}");
                assert_eq!(col[0], if g.biome_at(x, z) == "plains" { 1 } else { 4 });
                if col[top as usize] == 3 {
                    grass += 1;
                    assert_eq!(col[top as usize - 1], 2, "dirt under grass");
                }
                trees += usize::from(col.contains(&5));
            }
        }
        assert!(grass > 100, "{grass}");
        assert!(trees > 5, "{trees}");
    }

    #[test]
    fn columns_are_the_same_whatever_order_they_are_asked_in() {
        let a = parse(PLAINS, 7, &blocks).unwrap();
        let b = parse(PLAINS, 7, &blocks).unwrap();
        let mut first = Vec::new();
        for x in 0..24 {
            for z in 0..24 {
                first.push(a.column(3, x, z, 40));
            }
        }
        let mut n = first.len();
        for x in (0..24).rev() {
            for z in (0..24).rev() {
                n -= 1;
                assert_eq!(*b.column(3, x, z, 40), *first[n]);
            }
        }
    }

    #[test]
    fn biomes_split_the_map_and_water_fills_low_ground() {
        let g = parse(
            r#"{"constants": {"base": 10, "sea": 12},
                "biome_map": {"type": "x"}, "transition": 0,
                "biomes": [
                  {"name": "low", "max": 16, "terrain": {"type": "curve", "points": [[0, 1], [1, -1]],
                     "input": {"type": "base_height", "name": "base"}},
                   "materials": {"type": "solidity", "solid": "stone",
                     "empty": {"type": "horizontal", "top": 12, "material": "water"}}},
                  {"name": "high", "min": 16, "terrain": {"type": "less", "inputs": [{"type": "y"}, 14]},
                   "materials": {"type": "solidity", "solid": "dirt"}}]}"#,
            1,
            &blocks,
        )
        .unwrap();
        assert_eq!(g.biome_at(3, 0), "low");
        assert_eq!(g.biome_at(20, 0), "high");
        let low = g.column(1, 3, 0, 20);
        assert_eq!(low[10], 1);
        assert_eq!(low[11], 20, "water up to the sea");
        assert_eq!(low[12], 0);
        let high = g.column(1, 20, 0, 20);
        assert_eq!(high[13], 2);
        assert_eq!(high[14], 0);
    }

    #[test]
    fn bad_graphs_say_what_is_wrong() {
        let bad = |text: &str| parse(text, 1, &blocks).err().unwrap();
        assert!(bad("{}").contains("biomes"));
        assert!(
            bad(r#"{"biomes":[{"name":"a","terrain":{"type":"wobble"},"materials":"stone"}]}"#)
                .contains("wobble")
        );
        assert!(bad(r#"{"biomes":[{"name":"a","terrain":1,"materials":"mud"}]}"#).contains("mud"));
        assert!(bad(r#"{"exports":{"a":{"type":"import","name":"a"}},"biomes":[{"name":"a","terrain":{"type":"import","name":"a"},"materials":"stone"}]}"#).contains("itself"));
        assert!(bad(r#"{"biomes":[{"name":"a","terrain":1,"materials":"stone","props":[{"prop":{"type":"sphere","radius":30,"material":"stone"}}]}]}"#).contains("reaches"));
    }
}
