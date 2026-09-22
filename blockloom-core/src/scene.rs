//! The world a project describes: actors, how they look, how they collide,
//! and how it's viewed. Shared verbatim between the editor (which edits it)
//! and the runtime (which renders it).

use serde::{Deserialize, Serialize};

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
}

impl Visual {
    /// True for the 3D visuals - a project in the wrong mode still loads,
    /// and the runtime just won't have anything to draw it with.
    pub fn is_3d(&self) -> bool {
        matches!(
            self,
            Visual::Cuboid { .. }
                | Visual::Sphere { .. }
                | Visual::Capsule { .. }
                | Visual::Plane { .. }
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
            Visual::Image { .. } => None,
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
            Visual::Image { .. } => {}
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
        }
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
    #[serde(default = "unit_scale")]
    pub scale: f32,
}

fn unit_scale() -> f32 {
    1.0
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            rotation: [0.0; 3],
            scale: 1.0,
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

impl Default for Lighting {
    fn default() -> Self {
        Self {
            light_direction: default_light_direction(),
            light_color: default_light_color(),
            illuminance: default_illuminance(),
            ambient_color: default_light_color(),
            ambient_brightness: default_ambient_brightness(),
            ao_enabled: false,
        }
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

        let json = serde_json::to_string(&world).unwrap();
        assert_eq!(serde_json::from_str::<World>(&json).unwrap(), world);
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
    fn an_explicit_mass_beats_density_in_the_document() {
        let physics: Physics =
            serde_json::from_str(r#"{"body": "Dynamic", "density": 2.0, "mass": 50.0}"#).unwrap();
        assert_eq!(physics.density, 2.0);
        assert_eq!(physics.mass, Some(50.0));
        let json = serde_json::to_string(&physics).unwrap();
        assert_eq!(serde_json::from_str::<Physics>(&json).unwrap(), physics);
    }
}
