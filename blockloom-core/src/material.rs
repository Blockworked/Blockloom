//! Surface materials, shader graphs, particles, trails and tilemaps.
//!
//! The look beyond flat colors: what a surface is made of, what moves on it,
//! and what the ground is built from.
//!
//! - [`SurfaceMaterial`] is PBR properties, texture maps and mapping controls
//!   plus an optional [`ShaderGraph`] custom effect. The 3D
//!   runtime applies the PBR half to its `StandardMaterial`; both dimensions
//!   render a custom graph through the shared ubershader (`GraphMaterial2d` /
//!   `GraphMaterial3d`), driven by the authored [`GraphEffect`] params.
//! - [`ShaderGraph`] is shader-graph lite: a few nodes that emit a real WESL
//!   `graph_main(uv, time)` function. [`GraphEffect::starter_graph`] is the
//!   uniform path spelled as nodes, so exporting one to a `.wesl` asset draws
//!   exactly what the inspector showed; pointing [`GraphEffect::source`] at a
//!   `.wesl` file (exported or hand-written) makes it drive the live material.
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
    #[serde(default)]
    pub normal_texture: String,
    #[serde(default)]
    pub roughness_texture: String,
    #[serde(default = "default_tiling")]
    pub tiling: [f32; 2],
    #[serde(default)]
    pub offset: [f32; 2],
    #[serde(default)]
    pub rotation: f32,
    #[serde(default)]
    pub sampler: TextureSampler,
    #[serde(default)]
    pub anisotropy: u8,
    #[serde(default)]
    pub box_projection: bool,
    #[serde(default = "default_texel_density")]
    pub texel_density: f32,
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

fn default_tiling() -> [f32; 2] {
    [1.0, 1.0]
}
fn default_texel_density() -> f32 {
    1.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TextureSampler {
    Repeat,
    Mirror,
    #[default]
    Clamp,
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
            normal_texture: String::new(),
            roughness_texture: String::new(),
            tiling: default_tiling(),
            offset: [0.0; 2],
            rotation: 0.0,
            sampler: TextureSampler::Clamp,
            anisotropy: 0,
            box_projection: false,
            texel_density: default_texel_density(),
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
        self.normal_texture = self.normal_texture.trim().to_string();
        self.roughness_texture = self.roughness_texture.trim().to_string();
        self.tiling = self.tiling.map(|v| {
            if v.is_finite() {
                v.clamp(0.001, 1024.0)
            } else {
                1.0
            }
        });
        self.offset = self.offset.map(|v| if v.is_finite() { v } else { 0.0 });
        if !self.rotation.is_finite() {
            self.rotation = 0.0;
        }
        if !self.texel_density.is_finite() {
            self.texel_density = 1.0;
        }
        self.texel_density = self.texel_density.clamp(0.001, 1024.0);
        self.anisotropy = self.anisotropy.min(16);
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
            || !self.normal_texture.is_empty()
            || !self.roughness_texture.is_empty()
            || self.tiling != default_tiling()
            || self.offset != [0.0; 2]
            || self.rotation != 0.0
            || self.sampler != TextureSampler::Clamp
            || self.anisotropy != 0
            || self.box_projection
            || self.texel_density != default_texel_density()
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
    /// A `.wesl` asset defining `graph_main(uv, time)`, relative to the
    /// project folder. Set, it draws the surface instead of `mode`; empty
    /// keeps the built-in uniform path.
    #[serde(default)]
    pub source: String,
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
            source: String::new(),
        }
    }
}

impl GraphEffect {
    pub fn normalize(&mut self) {
        self.speed = self.speed.max(0.0);
        self.strength = self.strength.clamp(0.0, 1.0);
        self.source = self.source.trim().to_string();
    }

    /// The graph the uniform path draws for this effect, node for node: the
    /// same math as `graph_2d.wesl`/`graph_3d.wesl`, reading the same tint,
    /// second color, speed and strength. Exporting it therefore changes
    /// nothing on screen, and the file is a working start for a hand edit.
    pub fn starter_graph(&self) -> ShaderGraph {
        let mut g = GraphBuilder::default();
        let base = g.push(GraphNode::Base);
        let out = match self.mode {
            EffectMode::Solid => base,
            EffectMode::Wave => {
                // mix(base, secondary, (sin(uv.x * 8 + t * speed) * 0.5 + 0.5) * strength)
                let x = g.uv_x();
                let phase = g.scaled_time();
                let eight = g.splat(8.0);
                let at = g.push(GraphNode::Mul { a: x, b: eight });
                let at = g.push(GraphNode::Add { a: at, b: phase });
                let bands = g.unit_sin(at);
                g.mix_by_strength(base, bands)
            }
            EffectMode::Plasma => {
                // sin(uv.x * 6 + t * speed) * sin(uv.y * 6 - t * speed * 1.3)
                let uv = g.push(GraphNode::Uv);
                let x = g.push(GraphNode::X { x: uv });
                let y = g.push(GraphNode::Y { x: uv });
                let six = g.splat(6.0);
                let phase = g.scaled_time();
                let back = g.splat(-1.3);
                let back = g.push(GraphNode::Mul { a: phase, b: back });
                let at_x = g.push(GraphNode::Mul { a: x, b: six });
                let at_x = g.push(GraphNode::Add { a: at_x, b: phase });
                let at_y = g.push(GraphNode::Mul { a: y, b: six });
                let at_y = g.push(GraphNode::Add { a: at_y, b: back });
                let sx = g.push(GraphNode::Sin { x: at_x });
                let sy = g.push(GraphNode::Sin { x: at_y });
                let v = g.push(GraphNode::Mul { a: sx, b: sy });
                let half = g.splat(0.5);
                let m = g.push(GraphNode::Mul { a: v, b: half });
                let m = g.push(GraphNode::Add { a: m, b: half });
                g.mix_by_strength(base, m)
            }
            EffectMode::Pulse => {
                // p = sin(t * speed) * 0.5 + 0.5, then brighten rgb by p * strength
                let phase = g.scaled_time();
                let p = g.unit_sin(phase);
                let strength = g.push(GraphNode::Strength);
                let ps = g.push(GraphNode::Mul { a: p, b: strength });
                let color = g.mix(base, ps);
                let one = g.splat(1.0);
                let rgb = g.push(GraphNode::Const {
                    color: [1.0, 1.0, 1.0, 0.0],
                });
                let lift = g.push(GraphNode::Mul { a: ps, b: rgb });
                let gain = g.push(GraphNode::Add { a: one, b: lift });
                g.push(GraphNode::Mul { a: color, b: gain })
            }
            EffectMode::Dissolve => {
                // Hashed noise against a pulsing threshold; eaten pixels go
                // clear, with a hot rim where they are being eaten.
                let uv = g.push(GraphNode::Uv);
                let seed = g.push(GraphNode::Const {
                    color: [12.9898, 78.233, 0.0, 0.0],
                });
                let d = g.push(GraphNode::Dot { a: uv, b: seed });
                let d = g.push(GraphNode::Sin { x: d });
                let big = g.splat(43758.55);
                let d = g.push(GraphNode::Mul { a: d, b: big });
                let noise = g.push(GraphNode::Fract { x: d });
                let phase = g.scaled_time();
                let pulse = g.unit_sin(phase);
                let strength = g.push(GraphNode::Strength);
                let threshold = g.push(GraphNode::Mul {
                    a: pulse,
                    b: strength,
                });
                let keep = g.push(GraphNode::Step {
                    edge: threshold,
                    x: noise,
                });
                let width = g.splat(0.15);
                let hi = g.push(GraphNode::Add {
                    a: threshold,
                    b: width,
                });
                let rim = g.push(GraphNode::Smoothstep {
                    lo: threshold,
                    hi,
                    x: noise,
                });
                let secondary = g.push(GraphNode::Secondary);
                let two = g.splat(2.0);
                let hot = g.push(GraphNode::Mul {
                    a: secondary,
                    b: two,
                });
                let color = g.push(GraphNode::Mix {
                    a: hot,
                    b: base,
                    t: rim,
                });
                let rgb = g.push(GraphNode::Const {
                    color: [1.0, 1.0, 1.0, 0.0],
                });
                let alpha = g.push(GraphNode::Const {
                    color: [0.0, 0.0, 0.0, 1.0],
                });
                let alpha = g.push(GraphNode::Mul { a: keep, b: alpha });
                let mask = g.push(GraphNode::Add { a: rgb, b: alpha });
                g.push(GraphNode::Mul { a: color, b: mask })
            }
        };
        ShaderGraph {
            nodes: g.nodes,
            output: out,
        }
    }
}

/// Appends nodes and hands back their index, so a graph reads top-down.
#[derive(Default)]
struct GraphBuilder {
    nodes: Vec<GraphNode>,
}

impl GraphBuilder {
    fn push(&mut self, node: GraphNode) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    fn splat(&mut self, value: f32) -> usize {
        self.push(GraphNode::Const { color: [value; 4] })
    }

    fn uv_x(&mut self) -> usize {
        let uv = self.push(GraphNode::Uv);
        self.push(GraphNode::X { x: uv })
    }

    /// `time * speed`.
    fn scaled_time(&mut self) -> usize {
        let time = self.push(GraphNode::Time);
        let speed = self.push(GraphNode::Speed);
        self.push(GraphNode::Mul { a: time, b: speed })
    }

    /// `sin(x) * 0.5 + 0.5`.
    fn unit_sin(&mut self, x: usize) -> usize {
        let s = self.push(GraphNode::Sin { x });
        let half = self.splat(0.5);
        let s = self.push(GraphNode::Mul { a: s, b: half });
        self.push(GraphNode::Add { a: s, b: half })
    }

    fn mix(&mut self, base: usize, t: usize) -> usize {
        let secondary = self.push(GraphNode::Secondary);
        self.push(GraphNode::Mix {
            a: base,
            b: secondary,
            t,
        })
    }

    /// `mix(base, secondary, t * strength)`.
    fn mix_by_strength(&mut self, base: usize, t: usize) -> usize {
        let strength = self.push(GraphNode::Strength);
        let t = self.push(GraphNode::Mul { a: t, b: strength });
        self.mix(base, t)
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
/// the clock, `Uv` carries `(u, v, 0, 1)`, `X`/`Y`/`Dot` broadcast a scalar,
/// and every op is component-wise, so no type checker is needed - a graph
/// that validates always emits. `Base` is the look's tint times its image,
/// and `Secondary`/`Speed`/`Strength` read the effect's live uniforms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "node")]
pub enum GraphNode {
    Const { color: [f32; 4] },
    Time,
    Uv,
    Base,
    Secondary,
    Speed,
    Strength,
    X { x: usize },
    Y { x: usize },
    Add { a: usize, b: usize },
    Mul { a: usize, b: usize },
    Mix { a: usize, b: usize, t: usize },
    Sin { x: usize },
    Fract { x: usize },
    Step { edge: usize, x: usize },
    Dot { a: usize, b: usize },
    Smoothstep { lo: usize, hi: usize, x: usize },
}

impl GraphNode {
    /// The node indices this one reads, for validation and ordering.
    pub fn refs(&self) -> Vec<usize> {
        match *self {
            GraphNode::Const { .. }
            | GraphNode::Time
            | GraphNode::Uv
            | GraphNode::Base
            | GraphNode::Secondary
            | GraphNode::Speed
            | GraphNode::Strength => Vec::new(),
            GraphNode::Add { a, b } | GraphNode::Mul { a, b } | GraphNode::Dot { a, b } => {
                vec![a, b]
            }
            GraphNode::Mix { a, b, t } => vec![a, b, t],
            GraphNode::Sin { x }
            | GraphNode::Fract { x }
            | GraphNode::X { x }
            | GraphNode::Y { x } => {
                vec![x]
            }
            GraphNode::Step { edge, x } => vec![edge, x],
            GraphNode::Smoothstep { lo, hi, x } => vec![lo, hi, x],
        }
    }

    fn wesl(&self) -> String {
        match *self {
            GraphNode::Const { color } => format!(
                "vec4<f32>({:.6}, {:.6}, {:.6}, {:.6})",
                color[0], color[1], color[2], color[3]
            ),
            GraphNode::Time => "vec4<f32>(time, time, time, time)".to_string(),
            GraphNode::Uv => "vec4<f32>(uv.x, uv.y, 0.0, 1.0)".to_string(),
            GraphNode::Base => "base_color(uv)".to_string(),
            GraphNode::Secondary => "secondary".to_string(),
            GraphNode::Speed => "vec4<f32>(params.y)".to_string(),
            GraphNode::Strength => "vec4<f32>(params.z)".to_string(),
            GraphNode::X { x } => format!("vec4<f32>(n{x}.x)"),
            GraphNode::Y { x } => format!("vec4<f32>(n{x}.y)"),
            GraphNode::Dot { a, b } => format!("vec4<f32>(dot(n{a}, n{b}))"),
            GraphNode::Smoothstep { lo, hi, x } => format!("smoothstep(n{lo}, n{hi}, n{x})"),
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
    pub fn to_wesl(&self) -> Result<String, String> {
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
                self.nodes[index].wesl()
            ));
        }
        body.push_str(&format!("    return n{};\n}}", self.output));
        Ok(body)
    }

    /// The graph as a `.wesl` asset: the contract as a comment, then the
    /// function. What [`GraphEffect::source`] points at after an export.
    pub fn to_wesl_asset(&self) -> Result<String, String> {
        Ok(format!("{SURFACE_CONTRACT}\n{}\n", self.to_wesl()?))
    }
}

/// What a surface `.wesl` asset has to provide, and what it can read. Heads
/// every exported file, so a hand edit starts from the rules.
pub const SURFACE_CONTRACT: &str = "\
// Blockloom surface shader (WESL).
// Define `fn graph_main(uv: vec2<f32>, time: f32) -> vec4<f32>`, returning the
// surface's color (alpha 0 is see-through). `import` statements may open the
// file, for Blockloom's library (blockloom::hash, noise, fbm, scattering) and
// Bevy's own modules: bevy_pbr in 3D, bevy_sprite_render in 2D. Other files
// in the project aren't modules and can't be imported.
// In scope: `tint` and `secondary` (vec4 colors), `params` (mode, speed,
// strength, time), and `base_color(uv)`, the look's tint times its image.
// The runtime passes box-projected UVs when the material enables projection.
";

/// The uniforms and helper every surface shader is compiled against. The
/// runtime's template and [`check_surface_wesl`] share this text, so what the
/// editor accepts is what the GPU gets. `constants::MATERIAL_BIND_GROUP` is Bevy's.
pub const SURFACE_BINDINGS: &str = "\
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> tint: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<uniform> secondary: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<uniform> params: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var<uniform> flags: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var texture_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var<uniform> uv_scale_offset: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var<uniform> uv_options: vec4<f32>;
var<private> surface_position: vec3<f32>;
var<private> surface_normal: vec3<f32>;

fn surface_uv(uv: vec2<f32>) -> vec2<f32> {
    let scaled = uv * uv_scale_offset.xy;
    let angle = uv_options.x;
    return vec2<f32>(scaled.x * cos(angle) - scaled.y * sin(angle),
                     scaled.x * sin(angle) + scaled.y * cos(angle)) + uv_scale_offset.zw;
}

fn base_color(uv: vec2<f32>) -> vec4<f32> {
    var color = tint;
    if (flags.x > 0.5) {
        if (uv_options.y > 0.5) {
            let p = surface_position * uv_options.z;
            let weight = pow(abs(normalize(surface_normal)), vec3<f32>(4.0));
            let w = weight / max(dot(weight, vec3<f32>(1.0)), 0.0001);
            color = color * (textureSample(texture, texture_sampler, surface_uv(p.yz)) * w.x
                + textureSample(texture, texture_sampler, surface_uv(p.xz)) * w.y
                + textureSample(texture, texture_sampler, surface_uv(p.xy)) * w.z);
        } else {
            color = color * textureSample(texture, texture_sampler, surface_uv(uv));
        }
    }
    return color;
}
";

/// The imports and fragment entry a surface shader is wrapped in, per
/// dimension. [`surface_module`] puts the bindings and the file between them.
pub const SURFACE_2D_HEAD: &str = "\
import bevy_sprite_render::mesh2d::{
    vertex_output::VertexOutput,
    view_bindings::view,
};
@if(TONEMAP_IN_SHADER)
import bevy_core_pipeline::tonemapping;
@if(SRGB_OUTPUT)
import bevy_render::color_operations::linear_to_srgb;
@if(OKLAB_OUTPUT)
import bevy_render::color_operations::linear_rgb_to_oklab;
";

pub const SURFACE_2D_TAIL: &str = "\
@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    if (flags.y > 0.5 && length((mesh.uv - 0.5) * 2.0) > 1.0) {
        discard;
    }
    var output_color = graph_main(mesh.uv, params.w);
    @if(TONEMAP_IN_SHADER)
    output_color = tonemapping::tone_mapping(output_color, view.color_grading);
    @if(SRGB_OUTPUT)
    output_color = vec4(linear_to_srgb(output_color.rgb), output_color.a);
    @if(OKLAB_OUTPUT)
    output_color = vec4(linear_rgb_to_oklab(output_color.rgb), output_color.a);
    return output_color;
}
";

pub const SURFACE_3D_HEAD: &str = "\
import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::view,
};
@if(TONEMAP_IN_SHADER)
import bevy_core_pipeline::tonemapping;
";

pub const SURFACE_3D_TAIL: &str = "\
@fragment
fn fragment(mesh: VertexOutput) -> FragmentOutput {
    surface_position = mesh.world_position.xyz;
    surface_normal = mesh.world_normal;
    var uv = mesh.uv;
    if (uv_options.y > 0.5) {
        let p = mesh.world_position.xyz * uv_options.z;
        let n = abs(mesh.world_normal);
        if (n.x >= n.y && n.x >= n.z) {
            uv = p.yz;
        } else if (n.y >= n.z) {
            uv = p.xz;
        } else {
            uv = p.xy;
        }
    }
    var output_color = graph_main(uv, params.w);
    @if(TONEMAP_IN_SHADER)
    output_color = tonemapping::tone_mapping(output_color, view.color_grading);
    var out: FragmentOutput;
    out.color = output_color;
    return out;
}
";

/// The whole module the GPU compiles for a surface file: its imports join
/// the wrapper's at the top, since WESL wants every import before the first
/// declaration, then its directives, the bindings, the rest of it, and the
/// fragment entry. The file's lines keep their numbers inside the body.
pub fn surface_module(source: &str, dim3: bool) -> String {
    let (head, tail) = if dim3 {
        (SURFACE_3D_HEAD, SURFACE_3D_TAIL)
    } else {
        (SURFACE_2D_HEAD, SURFACE_2D_TAIL)
    };
    let header = split_header(source);
    format!(
        "{}\n{head}\n{}\n{SURFACE_BINDINGS}\n{}\n{tail}",
        header.imports, header.directives, header.body
    )
}

/// A surface file split at its first declaration.
struct Header {
    imports: String,
    directives: String,
    /// The file with the import and directive statements blanked out, so its
    /// line numbers still match.
    body: String,
}

/// Picks the leading `import` and directive statements (`enable`, `requires`,
/// `diagnostic`) off a file, attributes such as `@if(...)` included.
fn split_header(source: &str) -> Header {
    let bytes = source.as_bytes();
    let trivia = |mut i: usize| loop {
        if bytes.get(i).is_some_and(|b| b.is_ascii_whitespace()) {
            i += 1;
        } else if source[i..].starts_with("//") {
            i = source[i..].find('\n').map_or(bytes.len(), |n| i + n);
        } else if source[i..].starts_with("/*") {
            let mut depth = 0;
            while i < bytes.len() {
                if source[i..].starts_with("/*") {
                    depth += 1;
                    i += 2;
                } else if source[i..].starts_with("*/") {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else {
            return i;
        }
    };
    let word_end = |i: usize| {
        i + source[i..]
            .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
            .unwrap_or(source.len() - i)
    };
    let mut header = Header {
        imports: String::new(),
        directives: String::new(),
        body: String::new(),
    };
    let mut blank = Vec::new();
    let mut pos = 0;
    loop {
        let start = trivia(pos);
        let mut i = start;
        // Attributes: `@name` and an optional parenthesised argument list.
        while bytes.get(i) == Some(&b'@') {
            i = trivia(word_end(i + 1));
            if bytes.get(i) == Some(&b'(') {
                let mut depth = 0;
                while i < bytes.len() {
                    match bytes[i] {
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        _ => {}
                    }
                    i += 1;
                    if depth == 0 {
                        break;
                    }
                }
                i = trivia(i);
            }
        }
        let keyword = &source[i..word_end(i)];
        if !matches!(keyword, "import" | "enable" | "requires" | "diagnostic") {
            break;
        }
        let mut depth = 0i32;
        let mut end = i;
        while end < bytes.len() {
            match bytes[end] {
                b'{' | b'(' => depth += 1,
                b'}' | b')' => depth -= 1,
                b';' if depth <= 0 => break,
                _ => {}
            }
            end += 1;
        }
        end = (end + 1).min(bytes.len());
        let statement = &source[start..end];
        let into = if keyword == "import" {
            &mut header.imports
        } else {
            &mut header.directives
        };
        into.push_str(statement);
        into.push('\n');
        blank.push(start..end);
        pos = end;
    }
    let mut last = 0;
    for range in blank {
        header.body.push_str(&source[last..range.start]);
        header
            .body
            .extend(source[range.clone()].chars().filter(|&ch| ch == '\n'));
        last = range.end;
    }
    header.body.push_str(&source[last..]);
    header
}

/// Parse and validate a surface shader the way the GPU will see it. Syntax
/// errors and the naga check carry line numbers that match the file: naga
/// sees the bindings after the source, which WGSL allows. A file without
/// `import` statements gets naga's full type check, and so does one that
/// only imports Blockloom's library, linked in first (its errors then point
/// into the linked module). One importing Bevy gets WESL syntax, the contract
/// and a clash check against the wrapper, and its types are checked when Play
/// compiles it on the GPU, since Bevy's modules only resolve there. An error
/// comes back ready to log.
pub fn check_surface_wesl(source: &str, dim3: bool) -> Result<(), String> {
    use std::borrow::Cow;
    use std::collections::HashSet;
    use wesl::syntax::{
        GlobalDeclaration, ImportContent, ImportStatement, PathOrigin, TranslationUnit,
    };
    use wgsl_parse::SyntaxNode;
    use wgsl_parse::syntax::AttributeNode;

    fn condcomp(attrs: &[AttributeNode]) -> bool {
        attrs
            .iter()
            .any(|attr| attr.is_if() || attr.is_elif() || attr.is_else())
    }
    fn import_names(content: &ImportContent, names: &mut Vec<String>) {
        match content {
            ImportContent::Item(item) => {
                names.push(item.rename.as_ref().unwrap_or(&item.ident).to_string())
            }
            ImportContent::Collection(items) => {
                for item in items {
                    import_names(&item.content, names);
                }
            }
        }
    }
    // Every global name a unit declares, `@if` branches included or not.
    fn global_names(unit: &TranslationUnit, branches: bool) -> Vec<String> {
        let mut names = Vec::new();
        for import in &unit.imports {
            if branches || !condcomp(import.attributes()) {
                import_names(&import.content, &mut names);
            }
        }
        for item in &unit.global_declarations {
            if branches || !condcomp(item.attributes()) {
                names.extend(item.ident().map(|ident| ident.to_string()));
            }
        }
        names
    }
    // The package an import reaches into, or an error for one that can't
    // resolve from a surface file.
    fn package(import: &ImportStatement) -> Result<String, String> {
        match import.path.as_ref().map(|path| &path.origin) {
            Some(PathOrigin::Package(name)) => Ok(name.clone()),
            Some(PathOrigin::Absolute | PathOrigin::Relative(_)) => Err(
                "`package::` and `super::` imports don't resolve: a surface file can only import Blockloom's and Bevy's shader modules"
                    .to_string(),
            ),
            None => Ok(match &import.content {
                ImportContent::Item(item) => item.ident.to_string(),
                ImportContent::Collection(items) => {
                    items.first().and_then(|item| item.path.first()).cloned().unwrap_or_default()
                }
            }),
        }
    }
    let parse = |text: &str| -> Result<TranslationUnit, String> {
        text.parse()
            .map_err(|error: wgsl_parse::Error| error.with_source(Cow::Borrowed(text)).to_string())
    };

    if source
        .lines()
        .any(|line| line.trim_start().starts_with("#import"))
    {
        return Err(
            "`#import` is the old syntax: WESL spells it `import bevy_pbr::forward_io::VertexOutput;` and it has to come before any declaration"
                .to_string(),
        );
    }
    let unit = parse(source)?;
    let graph_main = unit
        .global_declarations
        .iter()
        .find_map(|item| match item.node() {
            GlobalDeclaration::Function(function) if function.ident.to_string() == "graph_main" => {
                Some(function)
            }
            _ => None,
        })
        .ok_or("the shader has no graph_main(uv, time) function")?;
    if graph_main.parameters.len() != 2 {
        return Err("graph_main takes exactly (uv: vec2<f32>, time: f32)".to_string());
    }

    // Imports have to name a module this dimension's pipeline has loaded.
    let (other, here) = if dim3 {
        ("bevy_sprite_render", "3D")
    } else {
        ("bevy_pbr", "2D")
    };
    let mut library_only = true;
    for import in &unit.imports {
        let package = package(import)?;
        if package == other || (dim3 && package == "bevy_sprite") {
            return Err(format!(
                "`{package}` isn't available to a {here} surface shader"
            ));
        }
        library_only &= package == crate::shader_lib::PACKAGE;
    }

    // Names can't clash with each other or with anything the wrapper brings
    // in. The file's own `@if` branches may reuse a name, since only one lands.
    let wrapper: HashSet<String> = global_names(&parse(&surface_module("", dim3))?, true)
        .into_iter()
        .collect();
    let mut seen = HashSet::new();
    for name in global_names(&unit, false) {
        if !seen.insert(name.clone()) {
            return Err(format!("duplicate declaration of `{name}`"));
        }
    }
    for name in global_names(&unit, true) {
        if wrapper.contains(&name) {
            return Err(format!(
                "`{name}` is already declared by the surface wrapper; pick another name"
            ));
        }
    }
    // The module the GPU gets has to parse too, hoisted imports and all.
    parse(&surface_module(source, dim3))?;

    if !library_only {
        return Ok(());
    }
    let full = format!(
        "{source}\n{}\n@fragment\nfn blockloom_check(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {{\n    return graph_main(uv, params.w);\n}}\n",
        SURFACE_BINDINGS.replace("constants::MATERIAL_BIND_GROUP", "2")
    );
    if !unit.imports.is_empty() {
        return crate::shader_lib::validate(&full, &[]);
    }
    let module =
        naga::front::wgsl::parse_str(&full).map_err(|error| error.emit_to_string(&full))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .map_err(|error| error.emit_to_string(&full))?;
    Ok(())
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
    /// How much of the wind carries particles along, 0-1. Calm air carries
    /// nothing, so it only shows once the project has wind.
    #[serde(default = "default_wind")]
    pub wind: f32,
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
fn default_wind() -> f32 {
    1.0
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
            wind: default_wind(),
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
        self.wind = if self.wind.is_finite() {
            self.wind.clamp(0.0, 4.0)
        } else {
            default_wind()
        };
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
/// In 2D it lies flat; in 3D it stands as a wall, one tile thick. A solid
/// map collides tile by tile (see [`Tilemap::solid_rects`]), and a cell
/// painted with an animated tile cycles through that animation's frames.
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
    /// When solid every filled tile collides, bar the `passable` ones.
    #[serde(default)]
    pub solid: bool,
    /// Sheet indices a solid map still lets bodies through: grass, signs.
    #[serde(default)]
    pub passable: Vec<i32>,
    /// Animated tiles: water, torches, conveyor belts.
    #[serde(default)]
    pub animations: Vec<TileAnimation>,
}

/// One animated tile. Every cell painted `tile` shows `frames` in turn, at
/// `fps` frames a second, all in step with each other.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TileAnimation {
    pub tile: i32,
    #[serde(default)]
    pub frames: Vec<i32>,
    #[serde(default = "default_tile_fps")]
    pub fps: f32,
}

fn default_tile_fps() -> f32 {
    8.0
}

impl TileAnimation {
    /// Which frame shows at `time` seconds, as an index into `frames`.
    pub fn frame_index(&self, time: f32) -> usize {
        if self.frames.is_empty() {
            return 0;
        }
        let step = (time.max(0.0) * self.fps.max(0.0)).floor() as u64;
        (step % self.frames.len() as u64) as usize
    }
}

/// One solid rectangle of a tilemap in the actor's own frame: the same
/// centered, y-up coordinates [`Tilemap::build_mesh`] uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileRect {
    pub center: [f32; 2],
    pub half: [f32; 2],
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
            passable: Vec::new(),
            animations: Vec::new(),
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
        let in_sheet = |tile: &i32| (0..cells).contains(tile);
        self.passable.retain(in_sheet);
        self.passable.sort_unstable();
        self.passable.dedup();
        let mut seen = std::collections::HashSet::new();
        self.animations.retain_mut(|animation| {
            animation.frames.retain(in_sheet);
            animation.fps = animation.fps.clamp(0.1, 60.0);
            in_sheet(&animation.tile) && !animation.frames.is_empty() && seen.insert(animation.tile)
        });
    }

    /// Whether any tile animates, which is what asks the runtime to keep
    /// rewriting the map's UVs.
    pub fn is_animated(&self) -> bool {
        self.animations
            .iter()
            .any(|animation| animation.frames.len() > 1)
    }

    /// Which frame every animation is on at `time`: equal keys draw equal
    /// meshes, so the runtime only rewrites UVs when this changes.
    pub fn frame_key(&self, time: f32) -> Vec<usize> {
        self.animations
            .iter()
            .map(|animation| animation.frame_index(time))
            .collect()
    }

    /// The sheet cell a painted tile shows at `time`.
    pub fn shown_tile(&self, tile: i32, time: f32) -> i32 {
        self.animations
            .iter()
            .find(|animation| animation.tile == tile && !animation.frames.is_empty())
            .map(|animation| animation.frames[animation.frame_index(time)])
            .unwrap_or(tile)
    }

    fn collides(&self, tile: i32) -> bool {
        tile >= 0 && !self.passable.contains(&tile)
    }

    /// The map's collision as few rectangles as a greedy merge finds: runs
    /// along each row, then runs of the same span stacked down the rows. An
    /// empty list for a map that isn't solid.
    pub fn solid_rects(&self) -> Vec<TileRect> {
        if !self.solid {
            return Vec::new();
        }
        let [tw, th] = self.tile_size;
        let [w, h] = self.size();
        // (x0, x1) span -> (first row, last row) of the rectangle growing down.
        let mut open: std::collections::HashMap<(u32, u32), (u32, u32)> =
            std::collections::HashMap::new();
        let mut spans: Vec<(u32, u32, u32, u32)> = Vec::new();
        for gy in 0..self.height {
            let mut row = Vec::new();
            let mut gx = 0;
            while gx < self.width {
                if !self.collides(self.tile_at(gx, gy).unwrap_or(-1)) {
                    gx += 1;
                    continue;
                }
                let start = gx;
                while gx < self.width && self.collides(self.tile_at(gx, gy).unwrap_or(-1)) {
                    gx += 1;
                }
                row.push((start, gx));
            }
            let mut next = std::collections::HashMap::new();
            for span in row {
                let rows = match open.remove(&span) {
                    Some((first, _)) => (first, gy),
                    None => (gy, gy),
                };
                next.insert(span, rows);
            }
            spans.extend(open.drain().map(|((x0, x1), (y0, y1))| (x0, x1, y0, y1)));
            open = next;
        }
        spans.extend(open.drain().map(|((x0, x1), (y0, y1))| (x0, x1, y0, y1)));
        // A stable order, so two builds of one map collide identically.
        spans.sort_unstable_by_key(|&(x0, _, y0, _)| (y0, x0));
        spans
            .into_iter()
            .map(|(x0, x1, y0, y1)| {
                let left = x0 as f32 * tw - w / 2.0;
                let right = x1 as f32 * tw - w / 2.0;
                let top = h / 2.0 - y0 as f32 * th;
                let bottom = h / 2.0 - (y1 + 1) as f32 * th;
                TileRect {
                    center: [(left + right) / 2.0, (top + bottom) / 2.0],
                    half: [(right - left) / 2.0, (top - bottom) / 2.0],
                }
            })
            .collect()
    }

    /// Turn the grid into one mesh: a quad per filled tile with tileset UVs.
    /// Tile `(0, 0)` sits at the top-left; the map is centered on the actor.
    /// Animated tiles show their first frame; see [`Tilemap::build_mesh_at`].
    pub fn build_mesh(&self) -> TileMesh {
        self.build_mesh_at(0.0)
    }

    /// [`Tilemap::build_mesh`] with every animated tile on its frame for
    /// `time`. Which cells are filled never changes, so the quads line up
    /// one for one with any other time's and only the UVs differ.
    pub fn build_mesh_at(&self, time: f32) -> TileMesh {
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
                let tile = self.shown_tile(tile, time).max(0);
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
            tiling: [-2.0, f32::NAN],
            texel_density: 0.0,
            ..SurfaceMaterial::default()
        };
        material.normalize();
        assert_eq!(material.metallic, 1.0);
        assert_eq!(material.roughness, 0.0);
        assert_eq!(material.tiling, [0.001, 1.0]);
        assert_eq!(material.texel_density, 0.001);
        assert!(material.is_active());
    }

    #[test]
    fn older_materials_keep_clamped_unit_uvs() {
        let material: SurfaceMaterial =
            serde_json::from_str(r#"{"albedo_texture":"stone.png"}"#).unwrap();
        assert_eq!(material.tiling, [1.0, 1.0]);
        assert_eq!(material.offset, [0.0, 0.0]);
        assert_eq!(material.sampler, TextureSampler::Clamp);
        assert!(!material.box_projection);
        assert_eq!(material.texel_density, 1.0);
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
        for mode in EffectMode::ALL {
            let effect = GraphEffect {
                mode: *mode,
                ..GraphEffect::default()
            };
            let graph = effect.starter_graph();
            graph.validate().unwrap();
            let wesl = graph.to_wesl().unwrap();
            assert!(
                wesl.contains("fn graph_main(uv: vec2<f32>, time: f32) -> vec4<f32>"),
                "{mode:?}"
            );
            assert!(wesl.contains("return n"), "{mode:?}");
            // The exported asset is one the live material will take.
            for dim3 in [false, true] {
                check_surface_wesl(&graph.to_wesl_asset().unwrap(), dim3)
                    .unwrap_or_else(|error| panic!("{mode:?}: {error}"));
            }
        }
        let wave = GraphEffect {
            mode: EffectMode::Wave,
            ..GraphEffect::default()
        }
        .starter_graph()
        .to_wesl()
        .unwrap();
        assert!(wave.contains("sin(") && wave.contains("params.y"));
        let dissolve = GraphEffect {
            mode: EffectMode::Dissolve,
            ..GraphEffect::default()
        }
        .starter_graph()
        .to_wesl()
        .unwrap();
        assert!(dissolve.contains("fract(") && dissolve.contains("step("));
        assert!(dissolve.contains("smoothstep("));
    }

    #[test]
    fn hand_written_surface_shaders_are_checked_against_the_file() {
        let good = "fn graph_main(uv: vec2<f32>, time: f32) -> vec4<f32> {\n    \
                    return mix(base_color(uv), secondary, fract(time));\n}\n";
        check_surface_wesl(good, true).unwrap();
        let missing = "fn other(uv: vec2<f32>) -> vec4<f32> { return tint; }\n";
        assert!(check_surface_wesl(missing, true).is_err());
        let typo = "fn graph_main(uv: vec2<f32>, time: f32) -> vec4<f32> {\n    return tnit;\n}\n";
        let error = check_surface_wesl(typo, true).unwrap_err();
        // The report points at the user's own line 2, not the appended stub.
        assert!(error.contains(":2:"), "{error}");
        let wrong = "fn graph_main(uv: vec2<f32>) -> vec4<f32> { return tint; }\n";
        assert!(check_surface_wesl(wrong, true).is_err());
        let undeclared = "fn graph_main(uv: vec2<f32>, time: f32) -> vec4<f32> {\n    \
                    return no_such_helper(uv);\n}\n";
        assert!(check_surface_wesl(undeclared, true).is_err());
        let old = "#import bevy_pbr::forward_io\n";
        assert!(check_surface_wesl(old, true).unwrap_err().contains("WESL"));
        // The wrapper's own names are taken.
        let clash = "fn fragment() {}\n\
                     fn graph_main(uv: vec2<f32>, time: f32) -> vec4<f32> { return tint; }\n";
        assert!(check_surface_wesl(clash, true).is_err());
    }

    #[test]
    fn surface_imports_join_the_wrappers() {
        let imported = "// noise helpers\n\
                        @if(TONEMAP_IN_SHADER)\n\
                        import bevy_pbr::utils::{\n    PI,\n    rand_f,\n};\n\
                        import bevy_render::maths::PI_2;\n\
                        fn graph_main(uv: vec2<f32>, time: f32) -> vec4<f32> {\n    \
                        return base_color(uv) * PI_2;\n}\n";
        check_surface_wesl(imported, true).unwrap();
        // What the GPU gets parses, with every import ahead of the bindings.
        let module = surface_module(imported, true);
        let unit: wesl::syntax::TranslationUnit = module.parse().unwrap();
        assert_eq!(unit.imports.len(), 4);
        // The file's lines keep their numbers inside the body.
        let header = split_header(imported);
        assert_eq!(header.body.lines().count(), imported.lines().count());
        assert!(
            header
                .body
                .lines()
                .nth(7)
                .unwrap()
                .starts_with("fn graph_main")
        );

        // Importing what the wrapper already imports clashes on the GPU.
        let twice = "import bevy_pbr::forward_io::VertexOutput;\n\
                     fn graph_main(uv: vec2<f32>, time: f32) -> vec4<f32> { return tint; }\n";
        assert!(check_surface_wesl(twice, true).is_err());
        // Each dimension only has its own pipeline's modules.
        let pbr = "import bevy_pbr::utils::PI;\n\
                   fn graph_main(uv: vec2<f32>, time: f32) -> vec4<f32> { return tint; }\n";
        check_surface_wesl(pbr, true).unwrap();
        assert!(check_surface_wesl(pbr, false).is_err());
        // A project's own files aren't modules.
        let local = "import package::noise::hash;\n\
                     fn graph_main(uv: vec2<f32>, time: f32) -> vec4<f32> { return tint; }\n";
        assert!(check_surface_wesl(local, true).is_err());
        // Directives follow every import.
        let directive = "enable f16;\n\
                         fn graph_main(uv: vec2<f32>, time: f32) -> vec4<f32> { return tint; }\n";
        let module = surface_module(directive, false);
        assert!(module.find("enable f16;").unwrap() > module.rfind("import ").unwrap());
        let _: wesl::syntax::TranslationUnit = module.parse().unwrap();
    }

    #[test]
    fn surface_files_can_use_the_shader_library() {
        let clouds = "import blockloom::fbm::fbm2;\n\
                      fn graph_main(uv: vec2<f32>, time: f32) -> vec4<f32> {\n    \
                      let n = fbm2(uv * 4.0 + vec2<f32>(time, 0.0), 5u, 2.0, 0.5);\n    \
                      return mix(base_color(uv), secondary, n * 0.5 + 0.5);\n}\n";
        check_surface_wesl(clouds, true).unwrap();
        check_surface_wesl(clouds, false).unwrap();
        // Linked, the library still type-checks the file's own body.
        let wrong = "import blockloom::fbm::fbm2;\n\
                     fn graph_main(uv: vec2<f32>, time: f32) -> vec4<f32> {\n    \
                     return fbm2(uv, 5u, 2.0, 0.5);\n}\n";
        assert!(check_surface_wesl(wrong, true).is_err());
        let missing = "import blockloom::fbm::fbm9;\n\
                       fn graph_main(uv: vec2<f32>, time: f32) -> vec4<f32> {\n    \
                       return tint * fbm9(uv);\n}\n";
        assert!(check_surface_wesl(missing, true).is_err());
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
        let wesl = forward.to_wesl().unwrap();
        assert!(wesl.find("let n1").unwrap() < wesl.find("let n0").unwrap());
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

    #[test]
    fn animated_tiles_cycle_their_frames_in_step() {
        let mut map = Tilemap {
            width: 2,
            height: 1,
            sheet_columns: 4,
            sheet_rows: 1,
            tiles: vec![1, 0],
            animations: vec![
                TileAnimation {
                    tile: 1,
                    frames: vec![1, 2, 3],
                    fps: 2.0,
                },
                // Out of the sheet: dropped on normalize.
                TileAnimation {
                    tile: 9,
                    frames: vec![0],
                    fps: 2.0,
                },
            ],
            ..Tilemap::default()
        };
        map.normalize();
        assert_eq!(map.animations.len(), 1);
        assert!(map.is_animated());
        assert_eq!(map.shown_tile(1, 0.0), 1);
        assert_eq!(map.shown_tile(1, 0.5), 2);
        assert_eq!(map.shown_tile(1, 1.2), 3);
        assert_eq!(map.shown_tile(1, 1.5), 1);
        // A tile nobody animates stays put.
        assert_eq!(map.shown_tile(0, 1.2), 0);
        assert_ne!(map.frame_key(0.0), map.frame_key(0.5));
        // Same quads, different UVs.
        let first = map.build_mesh_at(0.0);
        let later = map.build_mesh_at(0.5);
        assert_eq!(first.positions, later.positions);
        assert_ne!(first.uvs, later.uvs);
        assert_eq!(first.uvs[4..], later.uvs[4..]);
    }

    #[test]
    fn a_solid_map_collides_per_tile_in_merged_rects() {
        // X X .
        // X X .
        // . P X      (P is passable)
        let mut map = Tilemap {
            width: 3,
            height: 3,
            tile_size: [10.0, 10.0],
            sheet_columns: 4,
            sheet_rows: 1,
            tiles: vec![0, 0, -1, 0, 0, -1, -1, 3, 1],
            solid: true,
            passable: vec![3],
            ..Tilemap::default()
        };
        map.normalize();
        let rects = map.solid_rects();
        assert_eq!(rects.len(), 2, "{rects:?}");
        // The 2x2 block, top-left, merged into one rect.
        assert_eq!(rects[0].center, [-5.0, 5.0]);
        assert_eq!(rects[0].half, [10.0, 10.0]);
        // The lone bottom-right tile.
        assert_eq!(rects[1].center, [10.0, -10.0]);
        assert_eq!(rects[1].half, [5.0, 5.0]);
        map.solid = false;
        assert!(map.solid_rects().is_empty());
    }
}
