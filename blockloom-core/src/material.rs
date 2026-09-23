//! Surface materials, shader graphs, particles, trails and tilemaps.
//!
//! The look beyond flat colors: what a surface is made of, what moves on it,
//! and what the ground is built from.
//!
//! - [`SurfaceMaterial`] is PBR properties (metallic, roughness, emissive,
//!   albedo texture) plus an optional [`ShaderGraph`] custom effect. The 3D
//!   runtime applies the PBR half to its `StandardMaterial`; both dimensions
//!   render a custom graph through the shared ubershader (`GraphMaterial2d` /
//!   `GraphMaterial3d`), driven by the authored [`GraphEffect`] params.
//! - [`ShaderGraph`] is shader-graph lite: a few nodes that emit a real WGSL
//!   `graph_main(uv, time)` function. The WGSL text is the portable artifact
//!   (saved as `.wgsl`, shipped with the build); the live preview uses the
//!   equivalent uniform path, so a graph means the same thing in both.
//! - [`ParticleSpec`] and [`TrailSpec`] describe emitters and motion trails.
//!   The runtime simulates them on the CPU with capped entity pools.
//! - [`Tilemap`] is a grid of tiles over one tileset image, with
//!   [`Tilemap::build_mesh`] turning it into vertices the runtime uploads.

use serde::{Deserialize, Serialize};

// ─── Surface materials ───────────────────────────────────────────────────

/// What a surface is made of, on top of the look's own color or image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceMaterial {
    /// How metallic the surface reads, 0 (plastic) to 1 (mirror).
    #[serde(default)]
    pub metallic: f32,
    /// How rough the surface reads, 0 (glossy) to 1 (matte).
    #[serde(default = "default_roughness")]
    pub roughness: f32,
    /// Emissive tint, added to the surface so it glows through bloom.
    #[serde(default = "default_emissive")]
    pub emissive: String,
    /// Emissive strength. 0 is off; 1 and up starts to bloom.
    #[serde(default)]
    pub emissive_energy: f32,
    /// An albedo texture in the project folder, multiplied over the look.
    /// Empty means none.
    #[serde(default)]
    pub albedo_texture: String,
    /// Draw both faces. Off culls back faces, the usual want.
    #[serde(default)]
    pub double_sided: bool,
    /// A custom shader-graph effect. `None` is the PBR path above.
    #[serde(default)]
    pub shader: Option<GraphEffect>,
}

fn default_roughness() -> f32 {
    0.6
}

fn default_emissive() -> String {
    "#000000".to_string()
}

impl Default for SurfaceMaterial {
    fn default() -> Self {
        Self {
            metallic: 0.0,
            roughness: default_roughness(),
            emissive: default_emissive(),
            emissive_energy: 0.0,
            albedo_texture: String::new(),
            double_sided: false,
            shader: None,
        }
    }
}

impl SurfaceMaterial {
    /// Clamp every dial into its live range, in place.
    pub fn normalize(&mut self) {
        self.metallic = self.metallic.clamp(0.0, 1.0);
        self.roughness = self.roughness.clamp(0.0, 1.0);
        self.emissive_energy = self.emissive_energy.max(0.0);
        self.albedo_texture = self.albedo_texture.trim().to_string();
        if let Some(effect) = self.shader.as_mut() {
            effect.normalize();
        }
    }

    /// True when the material changes anything the runtime must apply.
    pub fn is_active(&self) -> bool {
        self.metallic > 0.0
            || self.roughness != default_roughness()
            || self.emissive_energy > 0.0
            || !self.albedo_texture.is_empty()
            || self.double_sided
            || self.shader.is_some()
    }
}

// ─── Shader graphs ───────────────────────────────────────────────────────

/// The live effect a custom graph renders as. The graph is the portable
/// source; this is the authored preview path, so the two agree by
/// construction (see [`GraphEffect::starter_graph`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum EffectMode {
    /// Flat tint. The graph equivalent of no graph.
    #[default]
    Solid,
    /// Brightness bands sliding over the surface.
    Wave,
    /// Drifting two-color plasma.
    Plasma,
    /// Pulsing glow, for pickups and warnings.
    Pulse,
    /// Time-eaten edges that discard fragments.
    Dissolve,
}

impl EffectMode {
    /// Every mode, in the order the inspector lists them.
    pub const ALL: &[EffectMode] = &[
        EffectMode::Solid,
        EffectMode::Wave,
        EffectMode::Plasma,
        EffectMode::Pulse,
        EffectMode::Dissolve,
    ];

    /// What the editor dropdown and documents agree on.
    pub fn name(self) -> &'static str {
        match self {
            EffectMode::Solid => "Solid",
            EffectMode::Wave => "Wave",
            EffectMode::Plasma => "Plasma",
            EffectMode::Pulse => "Pulse",
            EffectMode::Dissolve => "Dissolve",
        }
    }

    pub fn parse(name: &str) -> Option<EffectMode> {
        match name.trim().to_lowercase().as_str() {
            "solid" => Some(EffectMode::Solid),
            "wave" => Some(EffectMode::Wave),
            "plasma" => Some(EffectMode::Plasma),
            "pulse" => Some(EffectMode::Pulse),
            "dissolve" => Some(EffectMode::Dissolve),
            _ => None,
        }
    }

    /// The uniform id the ubershader switches on.
    pub fn mode_id(self) -> u32 {
        match self {
            EffectMode::Solid => 0,
            EffectMode::Wave => 1,
            EffectMode::Plasma => 2,
            EffectMode::Pulse => 3,
            EffectMode::Dissolve => 4,
        }
    }
}

/// A custom surface effect: which motion, how fast, how strong, and the
/// second color it plays against the look's own tint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphEffect {
    #[serde(default)]
    pub mode: EffectMode,
    /// Cycles per second. 0 freezes the motion.
    #[serde(default = "default_speed")]
    pub speed: f32,
    /// Effect strength, 0 (barely there) to 1 (full).
    #[serde(default = "default_strength")]
    pub strength: f32,
    /// The second color: wave crests, plasma swirls, pulse peaks.
    #[serde(default = "default_effect_color")]
    pub color: String,
}

fn default_speed() -> f32 {
    1.0
}

fn default_strength() -> f32 {
    0.5
}

fn default_effect_color() -> String {
    "#FFFFFF".to_string()
}

impl Default for GraphEffect {
    fn default() -> Self {
        Self {
            mode: EffectMode::default(),
            speed: default_speed(),
            strength: default_strength(),
            color: default_effect_color(),
        }
    }
}

impl GraphEffect {
    pub fn normalize(&mut self) {
        self.speed = self.speed.max(0.0);
        self.strength = self.strength.clamp(0.0, 1.0);
    }

    /// A graph whose WGSL means the same thing as this effect's live path:
    /// same motion, same second color. Editing the graph afterwards changes
    /// the exported WGSL only; the live uniforms stay as authored.
    pub fn starter_graph(&self, tint: [f32; 4]) -> ShaderGraph {
        let second = hex_to_linear(&self.color);
        let speed = [self.speed, self.speed, self.speed, self.speed];
        let strength = [self.strength, self.strength, self.strength, self.strength];
        match self.mode {
            EffectMode::Solid => ShaderGraph {
                nodes: vec![GraphNode::Const { color: tint }],
                output: 0,
            },
            EffectMode::Wave => {
                // Bands sliding along x: mix(tint, second, sin(uv.x * 8 + t)).
                let uv = 0;
                let time = 1;
                let scaled_uv = 2;
                let phase = 3;
                let bands = 4;
                let out = 5;
                ShaderGraph {
                    nodes: vec![
                        GraphNode::Uv,
                        GraphNode::Time,
                        GraphNode::Mul { a: uv, b: 6 },
                        GraphNode::Add {
                            a: scaled_uv,
                            b: time,
                        },
                        GraphNode::Sin { x: phase },
                        GraphNode::Mix {
                            a: 7,
                            b: 8,
                            t: bands,
                        },
                        GraphNode::Const {
                            color: [8.0, 8.0, 8.0, 8.0],
                        },
                        GraphNode::Const { color: tint },
                        GraphNode::Const { color: second },
                    ],
                    output: out,
                }
            }
            EffectMode::Plasma => {
                // Two sines multiplied: mix(tint, second, plasma * strength).
                let out = 8;
                ShaderGraph {
                    nodes: vec![
                        GraphNode::Uv,
                        GraphNode::Time,
                        GraphNode::Mul { a: 0, b: 9 },
                        GraphNode::Add { a: 2, b: 1 },
                        GraphNode::Sin { x: 3 },
                        GraphNode::Mul { a: 0, b: 10 },
                        GraphNode::Add { a: 5, b: 1 },
                        GraphNode::Sin { x: 6 },
                        GraphNode::Mix {
                            a: 11,
                            b: 12,
                            t: 13,
                        },
                        GraphNode::Const {
                            color: [6.0, 6.0, 6.0, 6.0],
                        },
                        GraphNode::Const {
                            color: [3.0, 3.0, 3.0, 3.0],
                        },
                        GraphNode::Const { color: tint },
                        GraphNode::Const { color: second },
                        GraphNode::Mul { a: 4, b: 7 },
                    ],
                    output: out,
                }
            }
            EffectMode::Pulse => {
                // A 0..1 pulse from time: mix(tint, second, pulse * strength).
                let out = 8;
                ShaderGraph {
                    nodes: vec![
                        GraphNode::Time,
                        GraphNode::Mul { a: 0, b: 9 },
                        GraphNode::Sin { x: 1 },
                        GraphNode::Mul { a: 2, b: 10 },
                        GraphNode::Add { a: 3, b: 10 },
                        GraphNode::Mul { a: 4, b: 11 },
                        GraphNode::Const { color: tint },
                        GraphNode::Const { color: second },
                        GraphNode::Mix { a: 6, b: 7, t: 5 },
                        GraphNode::Const { color: speed },
                        GraphNode::Const {
                            color: [0.5, 0.5, 0.5, 0.5],
                        },
                        GraphNode::Const { color: strength },
                    ],
                    output: out,
                }
            }
            EffectMode::Dissolve => {
                // Hashed noise stepped against a pulsing threshold.
                let out = 12;
                ShaderGraph {
                    nodes: vec![
                        GraphNode::Uv,
                        GraphNode::Mul { a: 0, b: 13 },
                        GraphNode::Sin { x: 1 },
                        GraphNode::Mul { a: 2, b: 14 },
                        GraphNode::Fract { x: 3 },
                        GraphNode::Time,
                        GraphNode::Mul { a: 5, b: 15 },
                        GraphNode::Sin { x: 6 },
                        GraphNode::Mul { a: 7, b: 16 },
                        GraphNode::Add { a: 8, b: 16 },
                        GraphNode::Mul { a: 9, b: 17 },
                        GraphNode::Step { edge: 10, x: 4 },
                        GraphNode::Mix {
                            a: 18,
                            b: 19,
                            t: 11,
                        },
                        GraphNode::Const {
                            color: [12.9898, 12.9898, 12.9898, 12.9898],
                        },
                        GraphNode::Const {
                            color: [43758.55, 43758.55, 43758.55, 43758.55],
                        },
                        GraphNode::Const { color: speed },
                        GraphNode::Const {
                            color: [0.5, 0.5, 0.5, 0.5],
                        },
                        GraphNode::Const { color: strength },
                        GraphNode::Const {
                            color: [0.0, 0.0, 0.0, 0.0],
                        },
                        GraphNode::Const { color: tint },
                    ],
                    output: out,
                }
            }
        }
    }
}

/// `#RRGGBB` into linear RGBA. Unparseable reads as magenta, the way the
/// runtime's own color parser does.
pub fn hex_to_linear(hex: &str) -> [f32; 4] {
    let bytes = hex.trim_start_matches('#');
    let channel = |at: usize| {
        u8::from_str_radix(bytes.get(at..at + 2).unwrap_or("ff"), 16).unwrap_or(255) as f32 / 255.0
    };
    if bytes.len() < 6 {
        return [1.0, 0.0, 1.0, 1.0];
    }
    let srgb = [channel(0), channel(2), channel(4)];
    let linear = srgb.map(|c| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    });
    [linear[0], linear[1], linear[2], 1.0]
}

// ─── Shader graphs ───────────────────────────────────────────────────────

/// One node in a shader graph. Every value is a `vec4`: `Time` broadcasts
/// the clock, `Uv` carries `(u, v, 0, 1), and every op is component-wise,
/// so no type checker is needed - a graph that validates always emits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "node")]
pub enum GraphNode {
    Const { color: [f32; 4] },
    Time,
    Uv,
    Add { a: usize, b: usize },
    Mul { a: usize, b: usize },
    Mix { a: usize, b: usize, t: usize },
    Sin { x: usize },
    Fract { x: usize },
    Step { edge: usize, x: usize },
}

impl GraphNode {
    /// The node indices this one reads, for validation and ordering.
    pub fn refs(&self) -> Vec<usize> {
        match *self {
            GraphNode::Const { .. } | GraphNode::Time | GraphNode::Uv => Vec::new(),
            GraphNode::Add { a, b } | GraphNode::Mul { a, b } => vec![a, b],
            GraphNode::Mix { a, b, t } => vec![a, b, t],
            GraphNode::Sin { x } | GraphNode::Fract { x } => vec![x],
            GraphNode::Step { edge, x } => vec![edge, x],
        }
    }

    fn wgsl(&self) -> String {
        match *self {
            GraphNode::Const { color } => format!(
                "vec4<f32>({:.6}, {:.6}, {:.6}, {:.6})",
                color[0], color[1], color[2], color[3]
            ),
            GraphNode::Time => "vec4<f32>(time, time, time, time)".to_string(),
            GraphNode::Uv => "vec4<f32>(uv.x, uv.y, 0.0, 1.0)".to_string(),
            GraphNode::Add { a, b } => format!("(n{a} + n{b})"),
            GraphNode::Mul { a, b } => format!("(n{a} * n{b})"),
            GraphNode::Mix { a, b, t } => format!("mix(n{a}, n{b}, n{t})"),
            GraphNode::Sin { x } => format!("sin(n{x})"),
            GraphNode::Fract { x } => format!("fract(n{x})"),
            GraphNode::Step { edge, x } => format!("step(n{edge}, n{x})"),
        }
    }
}

/// A shader graph: nodes plus which one colors the surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShaderGraph {
    #[serde(default)]
    pub nodes: Vec<GraphNode>,
    #[serde(default)]
    pub output: usize,
}

impl ShaderGraph {
    /// A flat tint, the graph a fresh Custom effect starts from.
    pub fn solid(tint: [f32; 4]) -> Self {
        Self {
            nodes: vec![GraphNode::Const { color: tint }],
            output: 0,
        }
    }

    /// Check ranges and cycles. A graph that validates always emits.
    pub fn validate(&self) -> Result<(), String> {
        if self.nodes.is_empty() {
            return Err("the graph has no nodes".to_string());
        }
        if self.output >= self.nodes.len() {
            return Err(format!("output {} names no node", self.output));
        }
        for (index, node) in self.nodes.iter().enumerate() {
            for dep in node.refs() {
                if dep >= self.nodes.len() {
                    return Err(format!("node {index} reads missing node {dep}"));
                }
            }
        }
        // A cycle would hang the emitter, so walk for one first.
        let mut visiting = vec![false; self.nodes.len()];
        let mut done = vec![false; self.nodes.len()];
        for index in 0..self.nodes.len() {
            if !done[index] {
                check_acyclic(&self.nodes, index, &mut visiting, &mut done)?;
            }
        }
        Ok(())
    }

    /// Emit `graph_main(uv, time)`, the fragment body the runtime's
    /// ubershader shares its math with. Nodes come out topologically, so
    /// forward references still read correctly.
    pub fn to_wgsl(&self) -> Result<String, String> {
        self.validate()?;
        let mut order = Vec::new();
        let mut done = vec![false; self.nodes.len()];
        let mut visiting = vec![false; self.nodes.len()];
        for index in 0..self.nodes.len() {
            topo(&self.nodes, index, &mut visiting, &mut done, &mut order)?;
        }
        let mut body = String::from("fn graph_main(uv: vec2<f32>, time: f32) -> vec4<f32> {\n");
        for index in order {
            body.push_str(&format!(
                "    let n{index} = {};\n",
                self.nodes[index].wgsl()
            ));
        }
        body.push_str(&format!("    return n{};\n}}", self.output));
        Ok(body)
    }
}

fn check_acyclic(
    nodes: &[GraphNode],
    index: usize,
    visiting: &mut [bool],
    done: &mut [bool],
) -> Result<(), String> {
    if done[index] {
        return Ok(());
    }
    if visiting[index] {
        return Err(format!("node {index} feeds back into itself"));
    }
    visiting[index] = true;
    for dep in nodes[index].refs() {
        check_acyclic(nodes, dep, visiting, done)?;
    }
    visiting[index] = false;
    done[index] = true;
    Ok(())
}

fn topo(
    nodes: &[GraphNode],
    index: usize,
    visiting: &mut [bool],
    done: &mut [bool],
    order: &mut Vec<usize>,
) -> Result<(), String> {
    if done[index] {
        return Ok(());
    }
    if visiting[index] {
        return Err(format!("node {index} feeds back into itself"));
    }
    visiting[index] = true;
    for dep in nodes[index].refs() {
        topo(nodes, dep, visiting, done, order)?;
    }
    visiting[index] = false;
    done[index] = true;
    order.push(index);
    Ok(())
}

// ─── Particles and trails ────────────────────────────────────────────────

/// A particle emitter: sparks, smoke, splash. Particles spawn at the actor,
/// fly on their own, and fade out - moving the actor leaves them behind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleSpec {
    /// Particles per second. 0 emits nothing (a burst-only emitter).
    #[serde(default = "default_rate")]
    pub rate: f32,
    /// How long one particle lives, in seconds.
    #[serde(default = "default_lifetime")]
    pub lifetime: f32,
    /// Launch speed in world units per second.
    #[serde(default = "default_launch_speed")]
    pub speed: f32,
    /// Cone around the actor's facing particles launch into, in degrees.
    /// 360 sprays everywhere.
    #[serde(default = "default_spread")]
    pub spread: f32,
    /// How much of the world's gravity pulls particles down. 0 floats.
    #[serde(default = "default_gravity")]
    pub gravity_scale: f32,
    /// Particle size at birth and at death, in world units.
    #[serde(default = "default_size_start")]
    pub size_start: f32,
    #[serde(default = "default_size_end")]
    pub size_end: f32,
    /// Particle tint at birth and at death.
    #[serde(default = "default_color_start")]
    pub color_start: String,
    #[serde(default = "default_color_end")]
    pub color_end: String,
    /// Live particles per emitter before the oldest is reused.
    #[serde(default = "default_max")]
    pub max: u32,
}

fn default_rate() -> f32 {
    24.0
}
fn default_lifetime() -> f32 {
    0.8
}
fn default_launch_speed() -> f32 {
    120.0
}
fn default_spread() -> f32 {
    60.0
}
fn default_gravity() -> f32 {
    0.5
}
fn default_size_start() -> f32 {
    6.0
}
fn default_size_end() -> f32 {
    1.0
}
fn default_color_start() -> String {
    "#FFFFFF".to_string()
}
fn default_color_end() -> String {
    "#FFAB19".to_string()
}
fn default_max() -> u32 {
    128
}

impl Default for ParticleSpec {
    fn default() -> Self {
        Self {
            rate: default_rate(),
            lifetime: default_lifetime(),
            speed: default_launch_speed(),
            spread: default_spread(),
            gravity_scale: default_gravity(),
            size_start: default_size_start(),
            size_end: default_size_end(),
            color_start: default_color_start(),
            color_end: default_color_end(),
            max: default_max(),
        }
    }
}

impl ParticleSpec {
    /// Clamp every dial into its live range, in place.
    pub fn normalize(&mut self) {
        self.rate = self.rate.clamp(0.0, 240.0);
        self.lifetime = self.lifetime.clamp(0.05, 10.0);
        self.speed = self.speed.max(0.0);
        self.spread = self.spread.clamp(0.0, 360.0);
        self.gravity_scale = self.gravity_scale.clamp(0.0, 4.0);
        self.size_start = self.size_start.clamp(0.5, 256.0);
        self.size_end = self.size_end.clamp(0.0, 256.0);
        self.max = self.max.clamp(1, 512);
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.rate < 0.0 {
            return Err("emission rate can't be negative".to_string());
        }
        if self.lifetime <= 0.0 {
            return Err("particles must live longer than an instant".to_string());
        }
        if self.max == 0 {
            return Err("an emitter needs room for at least one particle".to_string());
        }
        Ok(())
    }
}

/// A motion trail: fading snapshots of where the actor just was, for
/// dashes, blades and comets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrailSpec {
    /// Seconds between snapshots. Smaller is smoother and hungrier.
    #[serde(default = "default_interval")]
    pub interval: f32,
    /// How long one ghost lasts, in seconds.
    #[serde(default = "default_ghost_life")]
    pub life: f32,
    /// Ghost tint. Alpha fades from half to nothing over its life.
    #[serde(default = "default_trail_color")]
    pub color: String,
}

fn default_interval() -> f32 {
    0.05
}
fn default_ghost_life() -> f32 {
    0.4
}
fn default_trail_color() -> String {
    "#FFFFFF".to_string()
}

impl Default for TrailSpec {
    fn default() -> Self {
        Self {
            interval: default_interval(),
            life: default_ghost_life(),
            color: default_trail_color(),
        }
    }
}

impl TrailSpec {
    pub fn normalize(&mut self) {
        self.interval = self.interval.clamp(0.016, 1.0);
        self.life = self.life.clamp(0.05, 5.0);
    }
}

// ─── Tilemaps ────────────────────────────────────────────────────────────

/// A tilemap: a grid of tiles over one tileset image. Tile `-1` is empty;
/// anything else indexes into the sheet, row-major from the top-left.
/// In 2D it lies flat; in 3D it stands as a wall, one tile thick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tilemap {
    /// The tileset image, relative to the project folder.
    #[serde(default)]
    pub tileset: String,
    /// One tile's size in world units.
    #[serde(default = "default_tile_size")]
    pub tile_size: [f32; 2],
    /// Tiles across and down.
    #[serde(default = "default_map_extent")]
    pub width: u32,
    #[serde(default = "default_map_extent")]
    pub height: u32,
    /// Tiles across and down in the tileset image.
    #[serde(default = "default_sheet_extent")]
    pub sheet_columns: u32,
    #[serde(default = "default_sheet_extent")]
    pub sheet_rows: u32,
    /// Row-major tile indices, `-1` for empty. Always `width * height`.
    #[serde(default)]
    pub tiles: Vec<i32>,
    /// When solid the whole map collides as one slab.
    #[serde(default)]
    pub solid: bool,
}

fn default_tile_size() -> [f32; 2] {
    [32.0, 32.0]
}
fn default_map_extent() -> u32 {
    8
}
fn default_sheet_extent() -> u32 {
    4
}

impl Default for Tilemap {
    fn default() -> Self {
        let (width, height) = (default_map_extent(), default_map_extent());
        Self {
            tileset: String::new(),
            tile_size: default_tile_size(),
            width,
            height,
            sheet_columns: default_sheet_extent(),
            sheet_rows: default_sheet_extent(),
            tiles: vec![-1; (width * height) as usize],
            solid: false,
        }
    }
}

impl Tilemap {
    /// The map's size in world units.
    pub fn size(&self) -> [f32; 2] {
        [
            self.width as f32 * self.tile_size[0],
            self.height as f32 * self.tile_size[1],
        ]
    }

    /// The tile at `(x, y)`, or `None` outside the map. `-1` is empty.
    pub fn tile_at(&self, x: u32, y: u32) -> Option<i32> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.tiles.get((y * self.width + x) as usize).copied()
    }

    /// Write one tile. Out-of-bounds writes are ignored, not errors, so a
    /// fill loop can overshoot freely.
    pub fn set_tile(&mut self, x: u32, y: u32, tile: i32) {
        if x >= self.width || y >= self.height {
            return;
        }
        if let Some(slot) = self.tiles.get_mut((y * self.width + x) as usize) {
            *slot = tile;
        }
    }

    /// Resize keeping whatever overlaps. New cells are empty.
    pub fn resize(&mut self, width: u32, height: u32) {
        let width = width.clamp(1, 256);
        let height = height.clamp(1, 256);
        let mut tiles = vec![-1; (width * height) as usize];
        for y in 0..height.min(self.height) {
            for x in 0..width.min(self.width) {
                tiles[(y * width + x) as usize] = self.tile_at(x, y).unwrap_or(-1);
            }
        }
        self.width = width;
        self.height = height;
        self.tiles = tiles;
    }

    /// Fix the shape after load or edit: exact length, clamped extents,
    /// indices inside the sheet (strays become empty).
    pub fn normalize(&mut self) {
        self.width = self.width.clamp(1, 256);
        self.height = self.height.clamp(1, 256);
        self.sheet_columns = self.sheet_columns.max(1);
        self.sheet_rows = self.sheet_rows.max(1);
        self.tile_size[0] = self.tile_size[0].max(1.0);
        self.tile_size[1] = self.tile_size[1].max(1.0);
        self.tileset = self.tileset.trim().to_string();
        self.tiles.resize((self.width * self.height) as usize, -1);
        let cells = (self.sheet_columns * self.sheet_rows) as i32;
        for tile in &mut self.tiles {
            if *tile < -1 || *tile >= cells {
                *tile = -1;
            }
        }
    }

    /// Turn the grid into one mesh: a quad per filled tile with tileset UVs.
    /// Tile `(0, 0)` sits at the top-left; the map is centered on the actor.
    pub fn build_mesh(&self) -> TileMesh {
        let [tw, th] = self.tile_size;
        let [w, h] = self.size();
        let (cols, rows) = (
            self.sheet_columns.max(1) as f32,
            self.sheet_rows.max(1) as f32,
        );
        let mut mesh = TileMesh {
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
            size: [w, h],
        };
        for gy in 0..self.height {
            for gx in 0..self.width {
                let tile = self.tile_at(gx, gy).unwrap_or(-1);
                if tile < 0 {
                    continue;
                }
                let (tx, ty) = (
                    tile as u32 % self.sheet_columns.max(1),
                    tile as u32 / self.sheet_columns.max(1),
                );
                // Bevy UVs start bottom-left; tileset rows start top-left.
                let (u0, v0) = (tx as f32 / cols, 1.0 - (ty + 1) as f32 / rows);
                let (u1, v1) = ((tx + 1) as f32 / cols, 1.0 - ty as f32 / rows);
                let (x0, y0) = (gx as f32 * tw - w / 2.0, h / 2.0 - (gy + 1) as f32 * th);
                let (x1, y1) = (x0 + tw, y0 + th);
                let base = mesh.positions.len() as u32;
                mesh.positions.extend_from_slice(&[
                    [x0, y0, 0.0],
                    [x1, y0, 0.0],
                    [x1, y1, 0.0],
                    [x0, y1, 0.0],
                ]);
                mesh.normals.extend_from_slice(&[[0.0, 0.0, 1.0]; 4]);
                mesh.uvs
                    .extend_from_slice(&[[u0, v0], [u1, v0], [u1, v1], [u0, v1]]);
                mesh.indices.extend_from_slice(&[
                    base,
                    base + 1,
                    base + 2,
                    base,
                    base + 2,
                    base + 3,
                ]);
            }
        }
        mesh
    }
}

/// One uploaded tilemap: positions, normals, UVs and triangles.
#[derive(Debug, Clone, PartialEq)]
pub struct TileMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub size: [f32; 2],
}

impl TileMesh {
    /// How many tiles made it in.
    pub fn quads(&self) -> usize {
        self.indices.len() / 6
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_material_changes_nothing() {
        assert!(!SurfaceMaterial::default().is_active());
        let mut material = SurfaceMaterial {
            metallic: 2.0,
            roughness: -1.0,
            ..SurfaceMaterial::default()
        };
        material.normalize();
        assert_eq!(material.metallic, 1.0);
        assert_eq!(material.roughness, 0.0);
        assert!(material.is_active());
    }

    #[test]
    fn effect_modes_round_trip_by_name() {
        for mode in EffectMode::ALL {
            assert_eq!(EffectMode::parse(mode.name()), Some(*mode));
        }
        assert_eq!(EffectMode::parse("nope"), None);
        // One id per mode, so the ubershader can switch on it.
        let mut ids: Vec<u32> = EffectMode::ALL.iter().map(|mode| mode.mode_id()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), EffectMode::ALL.len());
    }

    #[test]
    fn every_starter_graph_validates_and_emits_its_motion() {
        let tint = [0.3, 0.5, 1.0, 1.0];
        for mode in EffectMode::ALL {
            let effect = GraphEffect {
                mode: *mode,
                ..GraphEffect::default()
            };
            let graph = effect.starter_graph(tint);
            graph.validate().unwrap();
            let wgsl = graph.to_wgsl().unwrap();
            assert!(
                wgsl.contains("fn graph_main(uv: vec2<f32>, time: f32) -> vec4<f32>"),
                "{mode:?}"
            );
            assert!(wgsl.contains("return n"), "{mode:?}");
        }
        let wave = GraphEffect {
            mode: EffectMode::Wave,
            ..GraphEffect::default()
        }
        .starter_graph(tint)
        .to_wgsl()
        .unwrap();
        assert!(wave.contains("sin("));
        let dissolve = GraphEffect {
            mode: EffectMode::Dissolve,
            ..GraphEffect::default()
        }
        .starter_graph(tint)
        .to_wgsl()
        .unwrap();
        assert!(dissolve.contains("fract(") && dissolve.contains("step("));
    }

    #[test]
    fn graphs_refuse_missing_nodes_and_cycles() {
        let empty = ShaderGraph {
            nodes: Vec::new(),
            output: 0,
        };
        assert!(empty.validate().is_err());
        let dangling = ShaderGraph {
            nodes: vec![GraphNode::Sin { x: 3 }],
            output: 0,
        };
        assert!(dangling.validate().is_err());
        let looping = ShaderGraph {
            nodes: vec![GraphNode::Add { a: 1, b: 1 }, GraphNode::Mul { a: 0, b: 0 }],
            output: 0,
        };
        assert!(looping.validate().is_err());
        // A forward reference still emits: nodes come out topologically.
        let forward = ShaderGraph {
            nodes: vec![
                GraphNode::Add { a: 1, b: 2 },
                GraphNode::Const {
                    color: [1.0, 0.0, 0.0, 1.0],
                },
                GraphNode::Const {
                    color: [0.0, 0.0, 1.0, 1.0],
                },
            ],
            output: 0,
        };
        let wgsl = forward.to_wgsl().unwrap();
        assert!(wgsl.find("let n1").unwrap() < wgsl.find("let n0").unwrap());
    }

    #[test]
    fn hex_parses_the_colors_the_editor_writes() {
        let white = hex_to_linear("#FFFFFF");
        assert!((white[0] - 1.0).abs() < 0.01 && white[3] == 1.0);
        let magenta = hex_to_linear("nope");
        assert_eq!(magenta, [1.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn particle_specs_clamp_into_shape() {
        let mut spec = ParticleSpec {
            rate: 9999.0,
            lifetime: 0.0,
            max: 0,
            ..ParticleSpec::default()
        };
        spec.normalize();
        assert_eq!(spec.rate, 240.0);
        assert_eq!(spec.lifetime, 0.05);
        assert_eq!(spec.max, 1);
        assert!(ParticleSpec::default().validate().is_ok());
    }

    #[test]
    fn trails_clamp_into_shape() {
        let mut trail = TrailSpec {
            interval: 0.0,
            life: 99.0,
            ..TrailSpec::default()
        };
        trail.normalize();
        assert_eq!(trail.interval, 0.016);
        assert_eq!(trail.life, 5.0);
    }

    #[test]
    fn tilemaps_read_write_resize_and_mesh() {
        let mut map = Tilemap {
            width: 2,
            height: 2,
            tile_size: [32.0, 32.0],
            sheet_columns: 4,
            sheet_rows: 4,
            tiles: vec![0, -1, 5, 15],
            ..Tilemap::default()
        };
        assert_eq!(map.size(), [64.0, 64.0]);
        assert_eq!(map.tile_at(0, 0), Some(0));
        assert_eq!(map.tile_at(9, 9), None);
        map.set_tile(1, 0, 3);
        assert_eq!(map.tile_at(1, 0), Some(3));
        // Out-of-range indices become empty on normalize.
        map.tiles[3] = 99;
        map.normalize();
        assert_eq!(map.tile_at(1, 1), Some(-1));

        let mesh = map.build_mesh();
        assert_eq!(mesh.quads(), 3);
        assert_eq!(mesh.size, [64.0, 64.0]);
        // Tile 0 is the top-left sheet cell: u starts at 0, v near the top.
        assert_eq!(mesh.uvs[0], [0.0, 0.75]);
        assert_eq!(mesh.uvs[2], [0.25, 1.0]);
        // Quads are centered on the actor: first tile starts left and top.
        assert_eq!(mesh.positions[0], [-32.0, 0.0, 0.0]);

        map.resize(3, 1);
        assert_eq!(map.tile_at(0, 0), Some(0));
        assert_eq!(map.tile_at(2, 0), Some(-1));
    }
}
