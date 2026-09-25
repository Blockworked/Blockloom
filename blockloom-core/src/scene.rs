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
    ];

    pub fn name(self) -> &'static str {
        match self {
            TonemapName::None => "None",
            TonemapName::Reinhard => "Reinhard",
            TonemapName::ReinhardLuminance => "ReinhardLuminance",
            TonemapName::AcesFitted => "AcesFitted",
            TonemapName::TonyMcMapface => "TonyMcMapface",
            TonemapName::Filmic => "Filmic",
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
            _ => None,
        }
    }
}

/// Post-process on the world camera, in both dimensions. Everything off or
/// neutral reads exactly like no post pass, so old projects don't change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PostProcess {
    /// Camera exposure in EV100. 9.7 is Bevy's own default; lower is brighter.
    #[serde(default = "default_exposure")]
    pub exposure_ev: f32,
    #[serde(default)]
    pub tonemapping: TonemapName,
    /// Bloom: emissive surfaces and bright lights glow.
    #[serde(default)]
    pub bloom_enabled: bool,
    #[serde(default = "default_bloom_threshold")]
    pub bloom_threshold: f32,
    #[serde(default = "default_bloom_intensity")]
    pub bloom_intensity: f32,
    /// Vignette: darkened corners, 0 for off.
    #[serde(default)]
    pub vignette_strength: f32,
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

impl Default for PostProcess {
    fn default() -> Self {
        Self {
            exposure_ev: default_exposure(),
            tonemapping: TonemapName::default(),
            bloom_enabled: false,
            bloom_threshold: default_bloom_threshold(),
            bloom_intensity: default_bloom_intensity(),
            vignette_strength: 0.0,
        }
    }
}

impl PostProcess {
    pub fn normalize(&mut self) {
        self.exposure_ev = self.exposure_ev.clamp(0.0, 20.0);
        self.bloom_threshold = self.bloom_threshold.max(0.0);
        self.bloom_intensity = self.bloom_intensity.clamp(0.0, 1.0);
        self.vignette_strength = self.vignette_strength.clamp(0.0, 1.0);
    }

    /// True when any pass would change a pixel.
    pub fn is_active(&self) -> bool {
        *self != PostProcess::default()
    }
}

/// Everything about the world that isn't an actor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct World {
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
