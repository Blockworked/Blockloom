//! The world a project describes: actors, how they look, how they collide,
//! and how it's viewed. Shared verbatim between the editor (which edits it)
//! and the runtime (which renders it).

use serde::{Deserialize, Serialize};

use crate::input::InputConfig;
use crate::sound::SoundMixer;

/// Which dimension a project runs in. Blocks are written once and mean the
/// obvious thing in both: a Z coordinate is simply ignored in `TwoD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Mode {
    #[default]
    TwoD,
    ThreeD,
}

impl Mode {
    pub fn is_3d(self) -> bool {
        matches!(self, Mode::ThreeD)
    }
}

/// One axis of a position, rotation or velocity. Travels as its name, which
/// is what an in-place dropdown on the block writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Axis {
    #[default]
    X,
    Y,
    Z,
}

impl Axis {
    pub fn index(self) -> usize {
        match self {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        }
    }
}

/// What an actor looks like. The first three are 2D; the rest are 3D. An
/// actor's visual also decides the collider its body gets, so a project
/// never has to describe the same shape twice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape")]
pub enum Visual {
    Rect {
        color: String,
        size: [f32; 2],
    },
    Circle {
        color: String,
        radius: f32,
    },
    /// A 2D image loaded from `path`, relative to the project's folder.
    Image {
        path: String,
        size: [f32; 2],
    },
    Cuboid {
        color: String,
        size: [f32; 3],
    },
    Sphere {
        color: String,
        radius: f32,
    },
    Capsule {
        color: String,
        radius: f32,
        height: f32,
    },
    /// A flat ground plane, the one 3D visual meant to be left static.
    Plane {
        color: String,
        size: [f32; 2],
    },
    /// A rigged model loaded from `path` (glTF/GLB/OBJ/FBX, relative to the
    /// project folder). The runtime draws a glTF's first scene, scaled by
    /// `scale`, and falls back to a tinted box while it loads or when it
    /// won't; the collider is that same box.
    Model {
        path: String,
        tint: String,
        scale: [f32; 3],
        /// The glTF animation that loops on the rig, by name. Empty plays
        /// the first one the file has; a rig without any stands still.
        #[serde(default)]
        animation: String,
    },
    /// A tilemap: a grid of tiles over one tileset image (see
    /// [`crate::material::Tilemap`]). Flat in 2D, a standing wall in 3D.
    Tilemap {
        tilemap: crate::material::Tilemap,
    },
}

impl Visual {
    /// True for the 3D visuals - a project in the wrong mode still loads,
    /// and the runtime just won't have anything to draw it with. A tilemap
    /// renders in both, so it counts as neither for mode switches.
    pub fn is_3d(&self) -> bool {
        matches!(
            self,
            Visual::Cuboid { .. }
                | Visual::Sphere { .. }
                | Visual::Capsule { .. }
                | Visual::Plane { .. }
                | Visual::Model { .. }
        )
    }

    pub fn color(&self) -> Option<&str> {
        match self {
            Visual::Rect { color, .. }
            | Visual::Circle { color, .. }
            | Visual::Cuboid { color, .. }
            | Visual::Sphere { color, .. }
            | Visual::Capsule { color, .. }
            | Visual::Plane { color, .. } => Some(color),
            Visual::Model { tint, .. } => Some(tint),
            Visual::Image { .. } | Visual::Tilemap { .. } => None,
        }
    }

    pub fn set_color(&mut self, next: String) {
        match self {
            Visual::Rect { color, .. }
            | Visual::Circle { color, .. }
            | Visual::Cuboid { color, .. }
            | Visual::Sphere { color, .. }
            | Visual::Capsule { color, .. }
            | Visual::Plane { color, .. } => *color = next,
            Visual::Model { tint, .. } => *tint = next,
            Visual::Image { .. } | Visual::Tilemap { .. } => {}
        }
    }
}

/// How the physics engine treats an actor. `None` means the actor is moved
/// only by its own blocks - no collider, no gravity, nothing to push it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum BodyKind {
    #[default]
    None,
    /// Immovable, but everything else collides with it - ground and walls.
    Static,
    /// Simulated: gravity, impulses, and collision response all apply.
    Dynamic,
    /// Moved by blocks, but still pushes dynamic bodies out of the way.
    Kinematic,
}

/// An actor's physics settings. The collider shape isn't here: it's derived
/// from [`Visual`], so the two can't disagree.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Physics {
    #[serde(default)]
    pub body: BodyKind,
    /// Multiplies the world's gravity for this actor alone - `0` for a
    /// floating dynamic body, `2` for a heavy one.
    #[serde(default = "one")]
    pub gravity_scale: f32,
    /// Stops a dynamic body from tipping over, the usual want for a
    /// platformer character.
    #[serde(default)]
    pub lock_rotation: bool,
    #[serde(default = "default_restitution")]
    pub restitution: f32,
    #[serde(default = "default_friction")]
    pub friction: f32,
    /// Mass per unit of collider area (2D) or volume (3D): the dial between a
    /// balloon and a lead ball of the same size.
    #[serde(default = "one")]
    pub density: f32,
    /// An explicit body mass, beating `density` when set - the collider
    /// weighs this however big it is. `None` lets shape and density decide.
    #[serde(default)]
    pub mass: Option<f32>,
    /// A trigger collider senses overlap without pushing back: it still fires
    /// `when I touch` and answers `touching?`, but the solver never resolves
    /// a contact against it. What a coin, a goal zone or a vision cone wants.
    #[serde(default)]
    pub trigger: bool,
    /// In 2D, a static platform supports actors from above while letting
    /// them pass through its underside and sides.
    #[serde(default)]
    pub one_way: bool,
    /// Use Rapier's collision-aware movement for a kinematic actor.
    #[serde(default)]
    pub character_controller: bool,
    /// Which collision layer the actor lives on, 1-8. The solver only pairs
    /// two bodies when each one's mask names the other's layer, so walls can
    /// ignore the player while the player's feet still raycast against them.
    #[serde(default = "default_layer")]
    pub collision_layer: u8,
    /// Bitmask of the layers this actor collides (and raycasts) with, bit
    /// `n - 1` for layer `n`. `255` is every layer.
    #[serde(default = "default_mask")]
    pub collision_mask: u8,
}

fn default_layer() -> u8 {
    1
}

fn default_mask() -> u8 {
    0xFF
}

fn one() -> f32 {
    1.0
}

fn default_restitution() -> f32 {
    0.0
}

fn default_friction() -> f32 {
    0.5
}

impl Default for Physics {
    fn default() -> Self {
        Self {
            body: BodyKind::None,
            gravity_scale: 1.0,
            lock_rotation: false,
            restitution: 0.0,
            friction: 0.5,
            density: 1.0,
            mass: None,
            trigger: false,
            one_way: false,
            character_controller: false,
            collision_layer: 1,
            collision_mask: 0xFF,
        }
    }
}

/// Eight layers, Godot-style: an actor lives on exactly one, and its mask
/// names which ones it pairs with.
pub const COLLISION_LAYERS: u8 = 8;

impl Physics {
    /// The actor's layer clamped to 1-8, so a stray document value can't
    /// shift a bit out of the mask.
    pub fn layer(self) -> u8 {
        self.collision_layer.clamp(1, COLLISION_LAYERS)
    }

    /// The rapier membership bit for [`Physics::layer`].
    pub fn layer_bits(self) -> u32 {
        1 << (self.layer() - 1)
    }

    /// Whether this actor's mask names `layer`.
    pub fn sees_layer(self, layer: u8) -> bool {
        let layer = layer.clamp(1, COLLISION_LAYERS);
        self.collision_mask & (1 << (layer - 1)) != 0
    }

    /// Whether two physics settings would interact: each one's mask names
    /// the other's layer. Raycasts use the querier's half; the solver uses
    /// both.
    pub fn interacts(self, other_layer: u8) -> bool {
        self.sees_layer(other_layer)
    }
}

/// An actor's place in the world. Rotation is Euler degrees per axis, which
/// is what the blocks talk in; the runtime converts once on the way to Bevy.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    #[serde(default)]
    pub position: [f32; 3],
    #[serde(default)]
    pub rotation: [f32; 3],
    /// The uniform size the `size` blocks read and write.
    #[serde(default = "unit_scale")]
    pub scale: f32,
    /// Per-axis stretch in the actor's own frame, on top of `scale`.
    #[serde(default = "unit_stretch", skip_serializing_if = "is_unit_stretch")]
    pub stretch: [f32; 3],
}

fn unit_scale() -> f32 {
    1.0
}

fn unit_stretch() -> [f32; 3] {
    [1.0; 3]
}

fn is_unit_stretch(stretch: &[f32; 3]) -> bool {
    *stretch == [1.0; 3]
}

impl Placement {
    /// The scale on each axis: the size times the stretch.
    pub fn scale3(&self) -> [f32; 3] {
        self.stretch.map(|s| s * self.scale)
    }
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            rotation: [0.0; 3],
            scale: 1.0,
            stretch: [1.0; 3],
        }
    }
}

/// Where the camera stands when nothing is holding it. In `TwoD` only `zoom`
/// matters; in `ThreeD` the camera sits at `position` looking at `look_at`.
/// An actor carrying a [`crate::components::ActorComponent::Camera`] takes it
/// over from there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    #[serde(default = "default_camera_position")]
    pub position: [f32; 3],
    #[serde(default)]
    pub look_at: [f32; 3],
    #[serde(default = "unit_scale")]
    pub zoom: f32,
    /// Pre-component projects named the followed actor here.
    /// [`crate::project::Project::normalize`] moves it onto that actor as a
    /// camera component and clears it, so nothing else ever reads it.
    #[serde(default, rename = "follow", skip_serializing)]
    pub legacy_follow: Option<String>,
}

fn default_camera_position() -> [f32; 3] {
    [0.0, 6.0, 14.0]
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: default_camera_position(),
            look_at: [0.0; 3],
            zoom: 1.0,
            legacy_follow: None,
        }
    }
}

/// The project-wide presentation of an actor's `say` bubble. Keeping this in
/// the saved scene, rather than in the VM or renderer, lets a future asset UI
/// expose themes and imported fonts without changing how the block executes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeechBubbleStyle {
    #[serde(default = "default_bubble_background")]
    pub background: String,
    #[serde(default = "default_bubble_border")]
    pub border: String,
    #[serde(default = "default_bubble_text")]
    pub text: String,
    #[serde(default = "default_bubble_font_size")]
    pub font_size: f32,
    #[serde(default = "default_bubble_max_width")]
    pub max_width: f32,
    #[serde(default = "default_bubble_padding")]
    pub padding: [f32; 2],
    /// Screen-space offset from the top of the speaking actor.
    #[serde(default = "default_bubble_offset")]
    pub offset: [f32; 2],
    /// A font in the project folder, spelled the way [`crate::assets`] does.
    /// An absent value uses Bevy's built-in sans-serif font.
    #[serde(default)]
    pub font_asset: Option<String>,
}

fn default_bubble_background() -> String {
    "#FFFFFF".to_string()
}

fn default_bubble_border() -> String {
    "#B8C0CC".to_string()
}

fn default_bubble_text() -> String {
    "#172033".to_string()
}

fn default_bubble_font_size() -> f32 {
    18.0
}

fn default_bubble_max_width() -> f32 {
    260.0
}

fn default_bubble_padding() -> [f32; 2] {
    [12.0, 8.0]
}

fn default_bubble_offset() -> [f32; 2] {
    [0.0, -12.0]
}

impl Default for SpeechBubbleStyle {
    fn default() -> Self {
        Self {
            background: default_bubble_background(),
            border: default_bubble_border(),
            text: default_bubble_text(),
            font_size: default_bubble_font_size(),
            max_width: default_bubble_max_width(),
            padding: default_bubble_padding(),
            offset: default_bubble_offset(),
            font_asset: None,
        }
    }
}

/// How a 3D project is lit. Stored on every project but only read by the
/// 3D runtime; 2D sprites ignore it. Defaults reproduce the old hardcoded
/// scenery (one bright directional light, modest ambient, AO off).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lighting {
    /// Where the directional light shines from, aimed at the origin.
    #[serde(default = "default_light_direction")]
    pub light_direction: [f32; 3],
    #[serde(default = "default_light_color")]
    pub light_color: String,
    /// Lux the directional light emits - 10_000 is bright daylight.
    #[serde(default = "default_illuminance")]
    pub illuminance: f32,
    #[serde(default = "default_light_color")]
    pub ambient_color: String,
    #[serde(default = "default_ambient_brightness")]
    pub ambient_brightness: f32,
    /// Screen-space ambient occlusion on the 3D camera. Off by default:
    /// it costs GPU time and changes the look of existing projects.
    #[serde(default)]
    pub ao_enabled: bool,
    /// Shadow map size per cascade, in pixels. Must be a power of two;
    /// larger is crisper and hungrier. Bevy's own default is 2048.
    #[serde(default = "default_shadow_map_size")]
    pub shadow_map_size: u32,
    /// Depth bias fighting shadow acne. Small positive values lift the
    /// shadow off the surface; too much makes shadows detach. The normal
    /// bias stays at Bevy's own default.
    #[serde(default = "default_shadow_bias")]
    pub shadow_bias: f32,
    /// Where an HDR sky lived before `World::sky`: read from old documents,
    /// moved there by `Project::normalize`, never written.
    #[serde(default, skip_serializing)]
    pub sky: String,
    #[serde(default = "default_sky_brightness", skip_serializing)]
    pub sky_brightness: f32,
    #[serde(default)]
    pub shadows: ShadowSettings,
    /// An image the sun shines through, tiled across the world like cloud
    /// shadows or a canopy. Only its red channel counts. Empty for none.
    #[serde(default)]
    pub sun_cookie: String,
    /// Metres one tile of the sun's cookie covers.
    #[serde(default = "default_sun_cookie_size")]
    pub sun_cookie_size: f32,
    #[serde(default)]
    pub ray_tracing: RayTracingSettings,
}

/// What cleans up ray-traced lighting's noise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Denoiser {
    /// The best this build has: ReSTIR's reuse (hybrid tracing) under
    /// Blockloom's spatiotemporal filter. No build carries DLSS Ray
    /// Reconstruction yet.
    #[default]
    Auto,
    /// ReSTIR's temporal and spatial reuse only.
    Restir,
    /// The spatiotemporal filter alone, without ReSTIR's reuse.
    Filter,
    /// The raw samples, for judging what a denoiser is working from.
    None,
}

impl Denoiser {
    /// Whether the spatiotemporal filter runs.
    pub fn filters(self) -> bool {
        matches!(self, Self::Auto | Self::Filter)
    }

    /// Whether hybrid tracing reuses samples through ReSTIR.
    pub fn reuses(self) -> bool {
        matches!(self, Self::Auto | Self::Restir)
    }
}

/// How ray-traced lighting is worked out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TracingMode {
    /// Solari: ReSTIR direct light and shadows, a world cache for bounced
    /// light, traced reflections. The fast one.
    #[default]
    Hybrid,
    /// Fresh paths from every pixel each frame, bounced to the end with no
    /// cache, then denoised. Closer to the truth and heavier.
    PathTraced,
}

/// Ray-traced lighting for a 3D world (Bevy Solari): traced direct light
/// and shadows, bounced light and reflections, in place of the raster rig
/// on a GPU that can trace rays. Elsewhere the raster rig carries on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RayTracingSettings {
    /// Off by default: it needs a ray-tracing GPU and costs a lot of it.
    pub enabled: bool,
    /// Most bounces a light path takes, 1-8. `set GI bounces to` overrides
    /// it for a run.
    pub bounces: u32,
    /// Light samples per pixel at the first hit, 1-32. `set GI samples to`
    /// overrides it for a run.
    pub samples: u32,
    pub denoiser: Denoiser,
    /// Metres a bounced-light ray reaches before it gives up.
    pub gi_distance: f32,
    pub mode: TracingMode,
    /// Paths per pixel each frame when path traced, 1-16.
    pub paths: u32,
}

impl Default for RayTracingSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            bounces: 3,
            samples: 8,
            denoiser: Denoiser::Auto,
            gi_distance: 50.0,
            mode: TracingMode::Hybrid,
            paths: 1,
        }
    }
}

impl RayTracingSettings {
    pub const MAX_BOUNCES: u32 = 8;
    pub const MAX_SAMPLES: u32 = 32;
    pub const MAX_PATHS: u32 = 16;

    /// The same settings pulled into the ranges the renderer accepts.
    pub fn sanitized(self) -> Self {
        let gi_distance = if self.gi_distance.is_finite() {
            self.gi_distance.clamp(1.0, 10_000.0)
        } else {
            50.0
        };
        Self {
            bounces: self.bounces.clamp(1, Self::MAX_BOUNCES),
            samples: self.samples.clamp(1, Self::MAX_SAMPLES),
            paths: self.paths.clamp(1, Self::MAX_PATHS),
            gi_distance,
            ..self
        }
    }
}

fn default_sun_cookie_size() -> f32 {
    20.0
}

/// How shadow maps are filtered on the world camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ShadowFilter {
    /// Hardware 2x2 PCF: fastest, hardest edges.
    Hardware,
    /// A 9-tap Gaussian PCF. Bevy's own default.
    #[default]
    Gaussian,
    /// A rotating spiral that TAA smooths out.
    Temporal,
}

/// Shadow tuning for the 3D world: the sun's cascades and biases, the
/// filter every shadow map is read through, soft (PCSS) sun shadows and
/// screen-space contact shadows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowSettings {
    #[serde(default)]
    pub filter: ShadowFilter,
    /// Metres from the camera the sun's shadows reach. `set shadow distance
    /// to` overrides it for a run.
    #[serde(default = "default_shadow_distance")]
    pub distance: f32,
    /// Sun shadow cascades, 1-4. More keeps near shadows sharp over a long
    /// distance.
    #[serde(default = "default_cascades")]
    pub cascades: u32,
    /// Where the first cascade ends, in metres. The rest split the remaining
    /// distance logarithmically.
    #[serde(default = "default_first_cascade")]
    pub first_cascade: f32,
    /// How much neighbouring cascades overlap, 0-0.5, so the step between
    /// them fades rather than snaps.
    #[serde(default = "default_cascade_blend")]
    pub cascade_blend: f32,
    /// The last part of the distance, 0-0.5 of it, over which the sun's
    /// shadows fade out instead of ending at a line.
    #[serde(default = "default_shadow_fade")]
    pub fade: f32,
    /// Normal bias for the sun, pushing the lookup out along the surface to
    /// fight acne on slopes. The depth bias is `Lighting::shadow_bias`.
    #[serde(default = "default_normal_bias")]
    pub normal_bias: f32,
    /// Soft sun shadows (PCSS) as if the sun were this many degrees across;
    /// 0 keeps them hard. The real sun is about 0.5.
    #[serde(default)]
    pub sun_size: f32,
    /// Screen-space contact shadows: a short raymarch against the depth
    /// buffer under feet and clutter. The sun takes part; each light opts in.
    #[serde(default)]
    pub contact: bool,
    /// Metres a contact shadow ray travels.
    #[serde(default = "default_contact_length")]
    pub contact_length: f32,
    /// How thick the depth buffer is taken to be, in metres.
    #[serde(default = "default_contact_thickness")]
    pub contact_thickness: f32,
}

fn default_shadow_distance() -> f32 {
    150.0
}

fn default_cascades() -> u32 {
    4
}

fn default_first_cascade() -> f32 {
    5.0
}

fn default_cascade_blend() -> f32 {
    0.2
}

fn default_shadow_fade() -> f32 {
    0.1
}

fn default_normal_bias() -> f32 {
    1.8
}

fn default_contact_length() -> f32 {
    0.3
}

fn default_contact_thickness() -> f32 {
    0.1
}

impl ShadowSettings {
    /// The same settings pulled into the ranges the renderer accepts.
    pub fn sanitized(self) -> Self {
        let finite = |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };
        let distance = finite(self.distance, default_shadow_distance()).clamp(1.0, 10_000.0);
        Self {
            distance,
            cascades: self.cascades.clamp(1, 4),
            first_cascade: finite(self.first_cascade, default_first_cascade()).clamp(0.1, distance),
            cascade_blend: finite(self.cascade_blend, default_cascade_blend()).clamp(0.0, 0.5),
            fade: finite(self.fade, default_shadow_fade()).clamp(0.0, 0.5),
            normal_bias: finite(self.normal_bias, default_normal_bias()).clamp(0.0, 10.0),
            sun_size: finite(self.sun_size, 0.0).clamp(0.0, 10.0),
            contact_length: finite(self.contact_length, default_contact_length()).clamp(0.01, 10.0),
            contact_thickness: finite(self.contact_thickness, default_contact_thickness())
                .clamp(0.001, 2.0),
            ..self
        }
    }
}

impl Default for ShadowSettings {
    fn default() -> Self {
        Self {
            filter: ShadowFilter::Gaussian,
            distance: default_shadow_distance(),
            cascades: default_cascades(),
            first_cascade: default_first_cascade(),
            cascade_blend: default_cascade_blend(),
            fade: default_shadow_fade(),
            normal_bias: default_normal_bias(),
            sun_size: 0.0,
            contact: false,
            contact_length: default_contact_length(),
            contact_thickness: default_contact_thickness(),
        }
    }
}

fn default_light_direction() -> [f32; 3] {
    [8.0, 16.0, 8.0]
}

fn default_light_color() -> String {
    "#FFFFFF".to_string()
}

fn default_illuminance() -> f32 {
    10_000.0
}

/// Bevy's own default ambient brightness - keeping it means an old project
/// with no lighting saved looks exactly like it used to.
fn default_ambient_brightness() -> f32 {
    80.0
}

fn default_shadow_map_size() -> u32 {
    2048
}

fn default_shadow_bias() -> f32 {
    0.02
}

fn default_sky_brightness() -> f32 {
    1000.0
}

impl Default for Lighting {
    fn default() -> Self {
        Self {
            light_direction: default_light_direction(),
            light_color: default_light_color(),
            illuminance: default_illuminance(),
            ambient_color: default_light_color(),
            ambient_brightness: default_ambient_brightness(),
            ao_enabled: false,
            shadow_map_size: default_shadow_map_size(),
            shadow_bias: default_shadow_bias(),
            sky: String::new(),
            sky_brightness: default_sky_brightness(),
            shadows: ShadowSettings::default(),
            sun_cookie: String::new(),
            sun_cookie_size: default_sun_cookie_size(),
            ray_tracing: RayTracingSettings::default(),
        }
    }
}

/// Which tonemapper the camera ends with. Bevy's default is TonyMcMapface,
/// so keeping that default means an old project looks exactly like it used
/// to; the rest are looks to choose on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TonemapName {
    None,
    Reinhard,
    ReinhardLuminance,
    AcesFitted,
    #[default]
    TonyMcMapface,
    Filmic,
    AgX,
    /// Khronos PBR Neutral: keeps base colors true up to the highlights.
    Neutral,
}

impl TonemapName {
    /// Every mapper, in the order the project settings dialog lists them.
    pub const ALL: &[TonemapName] = &[
        TonemapName::TonyMcMapface,
        TonemapName::None,
        TonemapName::Reinhard,
        TonemapName::ReinhardLuminance,
        TonemapName::AcesFitted,
        TonemapName::Filmic,
        TonemapName::AgX,
        TonemapName::Neutral,
    ];

    pub fn name(self) -> &'static str {
        match self {
            TonemapName::None => "None",
            TonemapName::Reinhard => "Reinhard",
            TonemapName::ReinhardLuminance => "ReinhardLuminance",
            TonemapName::AcesFitted => "AcesFitted",
            TonemapName::TonyMcMapface => "TonyMcMapface",
            TonemapName::Filmic => "Filmic",
            TonemapName::AgX => "AgX",
            TonemapName::Neutral => "Neutral",
        }
    }

    pub fn parse(name: &str) -> Option<TonemapName> {
        match name.trim() {
            "None" => Some(TonemapName::None),
            "Reinhard" => Some(TonemapName::Reinhard),
            "ReinhardLuminance" => Some(TonemapName::ReinhardLuminance),
            "AcesFitted" => Some(TonemapName::AcesFitted),
            "TonyMcMapface" => Some(TonemapName::TonyMcMapface),
            "Filmic" => Some(TonemapName::Filmic),
            "AgX" => Some(TonemapName::AgX),
            "Neutral" => Some(TonemapName::Neutral),
            _ => None,
        }
    }
}

/// Post-process on the world camera, in both dimensions. Everything off or
/// neutral reads exactly like no post pass, so old projects don't change.
/// The chain runs in a fixed order, HDR first: motion blur, depth of field,
/// chromatic aberration and vignette, then bloom, white balance, grading
/// and the tone shape, then the tonemapper, sharpening, the LUT and grain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PostProcess {
    /// Camera exposure in EV100. 9.7 is Bevy's own default; lower is brighter.
    #[serde(default = "default_exposure")]
    pub exposure_ev: f32,
    /// Meters the frame and moves the exposure itself, 3D only.
    #[serde(default)]
    pub auto_exposure: AutoExposure,
    #[serde(default)]
    pub tonemapping: TonemapName,
    /// Bloom: emissive surfaces and bright lights glow.
    #[serde(default)]
    pub bloom_enabled: bool,
    #[serde(default = "default_bloom_threshold")]
    pub bloom_threshold: f32,
    #[serde(default = "default_bloom_intensity")]
    pub bloom_intensity: f32,
    /// 0-1, how softly the threshold lets light in: 0 is a hard cut.
    #[serde(default = "default_bloom_knee")]
    pub bloom_knee: f32,
    /// 0-1, how far the glow spreads out through the mip chain.
    #[serde(default = "default_bloom_scatter")]
    pub bloom_scatter: f32,
    /// A lens dirt image the glow lights up, empty for none.
    #[serde(default)]
    pub bloom_dirt: String,
    #[serde(default)]
    pub bloom_dirt_intensity: f32,
    /// Vignette: darkened corners, 0 for off.
    #[serde(default)]
    pub vignette_strength: f32,
    #[serde(default)]
    pub tone: ToneShape,
    #[serde(default)]
    pub grading: Grading,
    #[serde(default)]
    pub depth_of_field: DepthOfField,
    #[serde(default)]
    pub motion_blur: MotionBlur,
    /// Ambient occlusion's reach and strength; the switch is the lighting's.
    #[serde(default)]
    pub ao: AmbientOcclusion,
    #[serde(default)]
    pub ssr: Reflections,
    /// 0-1, color fringes towards the frame's edges.
    #[serde(default)]
    pub chromatic_aberration: f32,
    #[serde(default)]
    pub grain: Grain,
    /// 0-1, contrast adaptive sharpening after the tonemapper.
    #[serde(default)]
    pub sharpen: f32,
}

fn default_exposure() -> f32 {
    9.7
}

fn default_bloom_threshold() -> f32 {
    1.0
}

fn default_bloom_intensity() -> f32 {
    0.15
}

fn default_bloom_knee() -> f32 {
    0.5
}

fn default_bloom_scatter() -> f32 {
    0.7
}

impl Default for PostProcess {
    fn default() -> Self {
        Self {
            exposure_ev: default_exposure(),
            auto_exposure: AutoExposure::default(),
            tonemapping: TonemapName::default(),
            bloom_enabled: false,
            bloom_threshold: default_bloom_threshold(),
            bloom_intensity: default_bloom_intensity(),
            bloom_knee: default_bloom_knee(),
            bloom_scatter: default_bloom_scatter(),
            bloom_dirt: String::new(),
            bloom_dirt_intensity: 0.0,
            vignette_strength: 0.0,
            tone: ToneShape::default(),
            grading: Grading::default(),
            depth_of_field: DepthOfField::default(),
            motion_blur: MotionBlur::default(),
            ao: AmbientOcclusion::default(),
            ssr: Reflections::default(),
            chromatic_aberration: 0.0,
            grain: Grain::default(),
            sharpen: 0.0,
        }
    }
}

impl PostProcess {
    pub fn normalize(&mut self) {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        self.exposure_ev = finite(self.exposure_ev, default_exposure()).clamp(0.0, 20.0);
        self.auto_exposure.normalize();
        self.bloom_threshold = finite(self.bloom_threshold, 1.0).max(0.0);
        self.bloom_intensity = finite(self.bloom_intensity, 0.15).clamp(0.0, 1.0);
        self.bloom_knee = finite(self.bloom_knee, default_bloom_knee()).clamp(0.0, 1.0);
        self.bloom_scatter = finite(self.bloom_scatter, default_bloom_scatter()).clamp(0.0, 1.0);
        self.bloom_dirt_intensity = finite(self.bloom_dirt_intensity, 0.0).clamp(0.0, 10.0);
        self.vignette_strength = finite(self.vignette_strength, 0.0).clamp(0.0, 1.0);
        self.tone.normalize();
        self.grading.normalize();
        self.depth_of_field.normalize();
        self.motion_blur.normalize();
        self.ao.normalize();
        self.ssr.normalize();
        self.chromatic_aberration = finite(self.chromatic_aberration, 0.0).clamp(0.0, 1.0);
        self.grain.normalize();
        self.sharpen = finite(self.sharpen, 0.0).clamp(0.0, 1.0);
    }

    /// True when any pass would change a pixel.
    pub fn is_active(&self) -> bool {
        *self != PostProcess::default()
    }
}

/// Which part of the frame auto-exposure reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Metering {
    /// The whole frame, evenly.
    Average,
    /// Mostly the middle of the frame, fading out towards the edges.
    CenterWeighted,
    /// A small spot in the middle, which is what the player looks at.
    #[default]
    Spot,
}

impl Metering {
    pub const ALL: &[Metering] = &[Metering::Spot, Metering::CenterWeighted, Metering::Average];
}

/// Auto-exposure: the meter's log-average steers the EV towards middle grey
/// at `speed` stops a second, inside `min_ev`..`max_ev`. A `set exposure to`
/// block still wins over it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoExposure {
    pub enabled: bool,
    pub metering: Metering,
    /// EV100 bounds: the darkest and brightest scene it adapts to.
    pub min_ev: f32,
    pub max_ev: f32,
    /// Stops a second towards a brighter scene, then a darker one.
    pub speed_up: f32,
    pub speed_down: f32,
    /// Stops added after metering: positive brightens the result.
    pub compensation: f32,
}

impl Default for AutoExposure {
    fn default() -> Self {
        Self {
            enabled: false,
            metering: Metering::Spot,
            min_ev: 2.0,
            max_ev: 16.0,
            speed_up: 3.0,
            speed_down: 1.0,
            compensation: 0.0,
        }
    }
}

impl AutoExposure {
    pub fn normalize(&mut self) {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        self.min_ev = finite(self.min_ev, 2.0).clamp(-10.0, 30.0);
        self.max_ev = finite(self.max_ev, 16.0).clamp(self.min_ev, 30.0);
        self.speed_up = finite(self.speed_up, 3.0).clamp(0.01, 100.0);
        self.speed_down = finite(self.speed_down, 1.0).clamp(0.01, 100.0);
        self.compensation = finite(self.compensation, 0.0).clamp(-10.0, 10.0);
    }

    /// The EV the metered frame asks for. `measured` is the log-average of
    /// the frame as it was exposed at `current`, where 0.18 is middle grey.
    pub fn target(&self, current: f32, measured: f32) -> f32 {
        let measured = measured.max(1.0e-4);
        let target = current + (measured / 0.18).log2() - self.compensation;
        target.clamp(self.min_ev, self.max_ev.max(self.min_ev))
    }

    /// One step from `current` towards `target` over `dt` seconds. A higher
    /// EV darkens, so moving up is adapting to a brighter scene.
    pub fn step(&self, current: f32, target: f32, dt: f32) -> f32 {
        let speed = if target > current {
            self.speed_up
        } else {
            self.speed_down
        };
        let most = speed * dt.max(0.0);
        current + (target - current).clamp(-most, most)
    }
}

/// Shapes the exposed signal ahead of the tonemapper: the toe deepens (or,
/// below 0, lifts) what is under middle grey, the shoulder rolls off what is
/// over it. Both 0 leave the tonemapper's own curve alone.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ToneShape {
    /// -1 to 1.
    pub toe: f32,
    /// 0 to 1.
    pub shoulder: f32,
}

impl ToneShape {
    pub fn normalize(&mut self) {
        self.toe = if self.toe.is_finite() { self.toe } else { 0.0 }.clamp(-1.0, 1.0);
        self.shoulder = if self.shoulder.is_finite() {
            self.shoulder
        } else {
            0.0
        }
        .clamp(0.0, 1.0);
    }

    /// The curve on one channel, as `post_hdr.wesl` applies it.
    pub fn apply(&self, x: f32) -> f32 {
        const GREY: f32 = 0.18;
        let x = x.max(0.0);
        if x < GREY {
            let t = x / GREY;
            // Blends out towards grey so the curve meets itself there.
            let weight = 1.0 - t * t * (3.0 - 2.0 * t);
            GREY * t.powf(1.0 + self.toe * weight)
        } else {
            let over = x - GREY;
            GREY + over / (1.0 + self.shoulder * over)
        }
    }
}

/// White balance and grading, applied in linear light before the
/// tonemapper, then an optional LUT after it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Grading {
    /// -1 cool to 1 warm.
    pub temperature: f32,
    /// -1 green to 1 magenta.
    pub tint: f32,
    /// Added to the shadows, per channel.
    pub lift: [f32; 3],
    /// Midtone power, per channel; 1 leaves it.
    pub gamma: [f32; 3],
    /// Multiplies everything, per channel.
    pub gain: [f32; 3],
    /// 0 grey, 1 as is, 2 doubled.
    pub saturation: f32,
    /// Around middle grey: 1 as is.
    pub contrast: f32,
    /// A `.cube` LUT or an image strip, applied to the display values.
    pub lut: String,
    /// 0-1, how much of the LUT shows.
    pub lut_contribution: f32,
}

impl Default for Grading {
    fn default() -> Self {
        Self {
            temperature: 0.0,
            tint: 0.0,
            lift: [0.0; 3],
            gamma: [1.0; 3],
            gain: [1.0; 3],
            saturation: 1.0,
            contrast: 1.0,
            lut: String::new(),
            lut_contribution: 1.0,
        }
    }
}

impl Grading {
    pub fn normalize(&mut self) {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        self.temperature = finite(self.temperature, 0.0).clamp(-1.0, 1.0);
        self.tint = finite(self.tint, 0.0).clamp(-1.0, 1.0);
        for v in &mut self.lift {
            *v = finite(*v, 0.0).clamp(-1.0, 1.0);
        }
        for v in &mut self.gamma {
            *v = finite(*v, 1.0).clamp(0.1, 10.0);
        }
        for v in &mut self.gain {
            *v = finite(*v, 1.0).clamp(0.0, 10.0);
        }
        self.saturation = finite(self.saturation, 1.0).clamp(0.0, 2.0);
        self.contrast = finite(self.contrast, 1.0).clamp(0.0, 2.0);
        self.lut_contribution = finite(self.lut_contribution, 1.0).clamp(0.0, 1.0);
    }

    /// True when the grade before the tonemapper would change a pixel.
    pub fn is_active(&self) -> bool {
        let neutral = Grading {
            lut: self.lut.clone(),
            lut_contribution: self.lut_contribution,
            ..Grading::default()
        };
        *self != neutral
    }
}

/// How out-of-focus highlights spread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Bokeh {
    /// Six-bladed bokeh: bright spots blur into hexagons.
    #[default]
    Hexagonal,
    /// A soft round blur, cheaper and without shaped highlights.
    Circular,
}

/// Depth of field, 3D only. Focus sits on the named actor when it has one,
/// or at `focus_distance` metres.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DepthOfField {
    pub enabled: bool,
    /// An actor's name to keep in focus, empty for the fixed distance.
    pub target: String,
    /// Metres.
    pub focus_distance: f32,
    pub f_stops: f32,
    pub bokeh: Bokeh,
    /// Blur things nearer than the focus.
    pub near: bool,
    /// Metres past which the blur stops growing, 0 for no limit.
    pub far_limit: f32,
    /// The widest blur, in pixels.
    pub max_blur: f32,
}

impl Default for DepthOfField {
    fn default() -> Self {
        Self {
            enabled: false,
            target: String::new(),
            focus_distance: 10.0,
            f_stops: 2.8,
            bokeh: Bokeh::Hexagonal,
            near: true,
            far_limit: 0.0,
            max_blur: 32.0,
        }
    }
}

impl DepthOfField {
    pub fn normalize(&mut self) {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        self.focus_distance = finite(self.focus_distance, 10.0).clamp(0.05, 100_000.0);
        self.f_stops = finite(self.f_stops, 2.8).clamp(0.5, 64.0);
        self.far_limit = finite(self.far_limit, 0.0).max(0.0);
        self.max_blur = finite(self.max_blur, 32.0).clamp(1.0, 128.0);
    }
}

/// Camera motion blur along the motion vectors, 3D only.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MotionBlur {
    pub enabled: bool,
    /// Degrees: 180 is half a frame of exposure, 360 the whole frame.
    pub shutter_angle: f32,
    pub samples: u32,
}

impl Default for MotionBlur {
    fn default() -> Self {
        Self {
            enabled: false,
            shutter_angle: 180.0,
            samples: 4,
        }
    }
}

impl MotionBlur {
    pub fn normalize(&mut self) {
        self.shutter_angle = if self.shutter_angle.is_finite() {
            self.shutter_angle
        } else {
            180.0
        }
        .clamp(0.0, 360.0);
        self.samples = self.samples.clamp(1, 32);
    }
}

/// Screen-space ambient occlusion, 3D only, turned on by `Lighting::ao_enabled`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AmbientOcclusion {
    /// How far to look for occluders, in metres.
    pub radius: f32,
    /// 0-4: how dark the occluded corners get; 1 is the default.
    pub intensity: f32,
}

impl Default for AmbientOcclusion {
    fn default() -> Self {
        Self {
            radius: 0.7285,
            intensity: 1.0,
        }
    }
}

impl AmbientOcclusion {
    pub fn normalize(&mut self) {
        self.radius = if self.radius.is_finite() {
            self.radius
        } else {
            0.7285
        }
        .clamp(0.01, 10.0);
        self.intensity = if self.intensity.is_finite() {
            self.intensity
        } else {
            1.0
        }
        .clamp(0.0, 4.0);
    }
}

/// Screen-space reflections, 3D only. Turning them on draws opaque surfaces
/// deferred, since the reflections read the G-buffer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Reflections {
    pub enabled: bool,
    /// Surfaces rougher than this reflect nothing on screen.
    pub roughness_cutoff: f32,
    /// How thick a depth sample counts as, in metres.
    pub thickness: f32,
}

impl Default for Reflections {
    fn default() -> Self {
        Self {
            enabled: false,
            roughness_cutoff: 0.4,
            thickness: 0.25,
        }
    }
}

impl Reflections {
    pub fn normalize(&mut self) {
        self.roughness_cutoff = if self.roughness_cutoff.is_finite() {
            self.roughness_cutoff
        } else {
            0.4
        }
        .clamp(0.05, 1.0);
        self.thickness = if self.thickness.is_finite() {
            self.thickness
        } else {
            0.25
        }
        .clamp(0.001, 10.0);
    }
}

/// Film grain over the display image.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Grain {
    /// 0-1, 0 for off.
    pub intensity: f32,
    /// Grain size in pixels, 1 to 4.
    pub size: f32,
    /// 0-1: how much the grain fades out of the highlights.
    pub response: f32,
}

impl Default for Grain {
    fn default() -> Self {
        Self {
            intensity: 0.0,
            size: 1.5,
            response: 0.8,
        }
    }
}

impl Grain {
    pub fn normalize(&mut self) {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        self.intensity = finite(self.intensity, 0.0).clamp(0.0, 1.0);
        self.size = finite(self.size, 1.5).clamp(1.0, 4.0);
        self.response = finite(self.response, 0.8).clamp(0.0, 1.0);
    }
}

/// The signal a window is sent in. HDR only takes effect where the display
/// offers it; anywhere else the game falls back to SDR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum OutputSpace {
    /// Rec.709 primaries, sRGB transfer, 0 to 1.
    #[default]
    Sdr,
    /// Rec.2020 primaries with the ST 2084 (PQ) curve.
    Hdr10,
    /// Extended linear sRGB, 1.0 at 80 nits.
    Scrgb,
}

impl OutputSpace {
    pub const ALL: &[OutputSpace] = &[OutputSpace::Sdr, OutputSpace::Hdr10, OutputSpace::Scrgb];

    pub fn is_hdr(self) -> bool {
        self != OutputSpace::Sdr
    }
}

/// How the finished frame meets the display: the output signal, how bright
/// the display can go and where SDR white (and the HUD) sits under HDR.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DisplayOutput {
    #[serde(default)]
    pub space: OutputSpace,
    /// The brightest the display shows, in nits.
    #[serde(default = "default_peak_nits")]
    pub peak_nits: f32,
    /// Where 1.0 (paper white, and the HUD) sits under HDR, in nits.
    #[serde(default = "default_paper_white_nits")]
    pub paper_white_nits: f32,
}

fn default_peak_nits() -> f32 {
    1000.0
}

fn default_paper_white_nits() -> f32 {
    200.0
}

impl Default for DisplayOutput {
    fn default() -> Self {
        Self {
            space: OutputSpace::Sdr,
            peak_nits: default_peak_nits(),
            paper_white_nits: default_paper_white_nits(),
        }
    }
}

impl DisplayOutput {
    pub const MIN_NITS: f32 = 80.0;
    pub const MAX_NITS: f32 = 10_000.0;

    pub fn normalize(&mut self) {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        self.peak_nits = finite(self.peak_nits, default_peak_nits()).clamp(100.0, Self::MAX_NITS);
        self.paper_white_nits = finite(self.paper_white_nits, default_paper_white_nits())
            .clamp(Self::MIN_NITS, self.peak_nits);
    }

    /// How far past paper white the display reaches, as a multiple of it.
    pub fn headroom(&self) -> f32 {
        (self.peak_nits / self.paper_white_nits.max(1.0)).max(1.0)
    }
}

/// Everything about the world that isn't an actor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct World {
    #[serde(default)]
    pub quality: crate::quality::Settings,
    #[serde(default)]
    pub interface: crate::ui::UiDocument,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default = "default_background")]
    pub background: String,
    /// Metres per second squared. The default is 2D-friendly: -981 units,
    /// since a 2D unit is a pixel.
    #[serde(default = "default_gravity_2d")]
    pub gravity: [f32; 3],
    /// How many times a second the world's physics and blocks advance,
    /// whatever the display rate is. 2D and 3D share it.
    #[serde(default = "default_fixed_rate")]
    pub fixed_rate: f32,
    #[serde(default)]
    pub camera: Camera,
    #[serde(default)]
    pub speech_bubble: SpeechBubbleStyle,
    #[serde(default)]
    pub lighting: Lighting,
    /// The saved mix: one gain per bus. What a `play sound` block's volumes
    /// scale against, and what the project settings dialog edits.
    #[serde(default)]
    pub sound: SoundMixer,
    /// Named input actions and the bindings that drive them. What the
    /// `action` reporters read and `when action pressed` fires on.
    #[serde(default)]
    pub input: InputConfig,
    /// Post-process on the world camera: exposure, tonemapping, bloom and
    /// vignette. Neutral by default, so old projects look the same.
    #[serde(default)]
    pub post: PostProcess,
    /// The output signal, peak brightness and paper white.
    #[serde(default)]
    pub display: DisplayOutput,
    /// Cost regions and explicit links in the navigation plane.
    #[serde(default)]
    pub navigation: crate::nav::NavSettings,
    /// The 3D sky: background, ambient light and reflections.
    #[serde(default)]
    pub sky: crate::sky::Sky,
    /// Height fog, volumetric fog and aerial perspective, 3D only.
    #[serde(default)]
    pub fog: crate::fog::Fog,
    #[serde(default)]
    pub clouds: crate::clouds::Clouds,
    /// Planar cloud layers, at most `cloud_layers::MAX_LAYERS`, 3D only.
    #[serde(default)]
    pub cloud_layers: Vec<crate::cloud_layers::CloudLayer>,
    /// Strikes and the storm that throws them.
    #[serde(default)]
    pub lightning: crate::lightning::Lightning,
    /// The wind everything that moves with the air reads, and the clouds'
    /// drift on it.
    #[serde(default)]
    pub wind: crate::wind::Wind,
    /// Time-of-day clock, 24h curve tracks and weather presets. Off by
    /// default, so old projects keep exactly the look they shipped.
    #[serde(default)]
    pub director: crate::director::Director,
    /// Snow cover and wetness, which surface masks scale by.
    #[serde(default)]
    pub surface: crate::material::SurfaceWeather,
    /// The particle budget and where emitters simulate.
    #[serde(default)]
    pub vfx: crate::vfx::VfxSettings,
    /// Named camera reels that `play cutscene` runs on the wall clock.
    #[serde(default)]
    pub cutscenes: Vec<crate::cinematic::Cutscene>,
}

fn default_background() -> String {
    "#1B2431".to_string()
}

fn default_gravity_2d() -> [f32; 3] {
    [0.0, -981.0, 0.0]
}

/// The project's default fixed step rate - the same 60 as the runtime's.
pub const DEFAULT_FIXED_RATE: f32 = 60.0;

fn default_fixed_rate() -> f32 {
    DEFAULT_FIXED_RATE
}

impl Default for World {
    fn default() -> Self {
        Self {
            quality: crate::quality::Settings::default(),
            interface: crate::ui::UiDocument::default(),
            mode: Mode::TwoD,
            background: default_background(),
            gravity: default_gravity_2d(),
            fixed_rate: default_fixed_rate(),
            camera: Camera::default(),
            speech_bubble: SpeechBubbleStyle::default(),
            lighting: Lighting::default(),
            sound: SoundMixer::default(),
            input: InputConfig::default(),
            post: PostProcess::default(),
            display: DisplayOutput::default(),
            navigation: crate::nav::NavSettings::default(),
            sky: crate::sky::Sky::default(),
            fog: crate::fog::Fog::default(),
            clouds: crate::clouds::Clouds::default(),
            cloud_layers: Vec::new(),
            lightning: crate::lightning::Lightning::default(),
            wind: crate::wind::Wind::default(),
            director: crate::director::Director::default(),
            surface: crate::material::SurfaceWeather::default(),
            vfx: crate::vfx::VfxSettings::default(),
            cutscenes: Vec::new(),
        }
    }
}

impl World {
    /// The gravity a fresh project of this mode starts with - 2D works in
    /// pixels, 3D in metres, so the two differ by roughly a hundredfold.
    pub fn default_gravity(mode: Mode) -> [f32; 3] {
        match mode {
            Mode::TwoD => default_gravity_2d(),
            Mode::ThreeD => [0.0, -9.81, 0.0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_older_world_gets_the_default_speech_bubble_style() {
        let world: World = serde_json::from_str("{}").unwrap();
        assert_eq!(world.speech_bubble, SpeechBubbleStyle::default());
    }

    #[test]
    fn display_output_keeps_paper_white_under_the_peak() {
        let mut display = DisplayOutput {
            space: OutputSpace::Hdr10,
            peak_nits: 50_000.0,
            paper_white_nits: f32::NAN,
        };
        display.normalize();
        assert_eq!(display.peak_nits, 10_000.0);
        assert_eq!(display.paper_white_nits, 200.0);
        display.peak_nits = 150.0;
        display.paper_white_nits = 400.0;
        display.normalize();
        assert_eq!(display.paper_white_nits, 150.0);
        assert_eq!(display.headroom(), 1.0);
        let old: World = serde_json::from_str("{}").unwrap();
        assert_eq!(old.display, DisplayOutput::default());
    }

    #[test]
    fn an_older_world_gets_the_default_lighting() {
        let world: World = serde_json::from_str("{}").unwrap();
        assert_eq!(world.lighting, Lighting::default());
        assert!(!world.lighting.ao_enabled);
    }

    #[test]
    fn lighting_round_trip() {
        let mut world = World::default();
        world.lighting.light_direction = [4.0, 10.0, -6.0];
        world.lighting.illuminance = 5000.0;
        world.lighting.ao_enabled = true;
        world.lighting.shadow_map_size = 1024;
        world.lighting.shadow_bias = 0.05;

        let json = serde_json::to_string(&world).unwrap();
        assert_eq!(serde_json::from_str::<World>(&json).unwrap(), world);
    }

    #[test]
    fn an_older_world_gets_neutral_post_and_shadow_defaults() {
        let world: World = serde_json::from_str("{}").unwrap();
        assert_eq!(world.post, PostProcess::default());
        assert!(!world.post.is_active());
        assert_eq!(world.lighting.shadow_map_size, 2048);
        for name in ["None", "Reinhard", "AcesFitted", "TonyMcMapface", "Filmic"] {
            assert!(TonemapName::parse(name).is_some(), "{name}");
        }
        assert_eq!(TonemapName::parse("nope"), None);
    }

    #[test]
    fn older_ray_tracing_settings_load_as_hybrid_with_one_path() {
        let old: RayTracingSettings =
            serde_json::from_str(r#"{"enabled":true,"bounces":2,"denoiser":"Restir"}"#).unwrap();
        assert_eq!(old.mode, TracingMode::Hybrid);
        assert_eq!(old.paths, 1);
        assert!(old.denoiser.reuses() && !old.denoiser.filters());
        let wild = RayTracingSettings {
            paths: 99,
            ..RayTracingSettings::default()
        };
        assert_eq!(wild.sanitized().paths, RayTracingSettings::MAX_PATHS);
        assert!(Denoiser::Filter.filters() && !Denoiser::Filter.reuses());
    }

    #[test]
    fn post_process_round_trip_and_clamps() {
        let mut post = PostProcess {
            exposure_ev: 99.0,
            bloom_enabled: true,
            vignette_strength: 2.0,
            ..PostProcess::default()
        };
        post.normalize();
        assert_eq!(post.exposure_ev, 20.0);
        assert_eq!(post.vignette_strength, 1.0);
        assert!(post.is_active());
        let json = serde_json::to_string(&post).unwrap();
        assert_eq!(serde_json::from_str::<PostProcess>(&json).unwrap(), post);
    }

    #[test]
    fn speech_bubble_asset_and_style_round_trip() {
        let mut world = World::default();
        world.speech_bubble.background = "#102030".to_string();
        world.speech_bubble.font_asset = Some("fonts/dialogue.ttf".to_string());

        let json = serde_json::to_string(&world).unwrap();
        assert_eq!(serde_json::from_str::<World>(&json).unwrap(), world);
    }

    #[test]
    fn an_old_physics_defaults_its_mass_and_density() {
        let physics: Physics =
            serde_json::from_str(r#"{"body": "Dynamic", "friction": 0.2}"#).unwrap();
        assert_eq!(physics.body, BodyKind::Dynamic);
        assert_eq!(physics.friction, 0.2);
        assert_eq!(physics.density, 1.0);
        assert_eq!(physics.mass, None);
    }

    #[test]
    fn an_old_physics_is_a_solid_on_layer_one_seeing_everything() {
        let physics: Physics = serde_json::from_str(r#"{"body": "Dynamic"}"#).unwrap();
        assert!(!physics.trigger);
        assert_eq!(physics.collision_layer, 1);
        assert_eq!(physics.collision_mask, 0xFF);
        assert_eq!(physics.layer(), 1);
        assert!(physics.sees_layer(8));
    }

    #[test]
    fn layers_clamp_and_masks_filter() {
        let physics = Physics {
            collision_layer: 99,
            collision_mask: 0b0000_0010,
            ..Physics::default()
        };
        assert_eq!(physics.layer(), 8);
        assert!(!physics.sees_layer(1));
        assert!(physics.sees_layer(2));
    }

    #[test]
    fn an_explicit_mass_beats_density_in_the_document() {
        let physics: Physics =
            serde_json::from_str(r#"{"body": "Dynamic", "density": 2.0, "mass": 50.0}"#).unwrap();
        assert_eq!(physics.density, 2.0);
        assert_eq!(physics.mass, Some(50.0));
        let json = serde_json::to_string(&physics).unwrap();
        assert_eq!(serde_json::from_str::<Physics>(&json).unwrap(), physics);
    }
}
