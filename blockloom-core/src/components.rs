//! An actor's properties, as components.
//!
//! Everything an actor *is* - where it stands, what it looks like, whether
//! physics owns it, whether it is drawn - is one entry in its
//! [`Components`] list rather than a fixed field, and the runtime turns each
//! entry into the Bevy component it names. Removing `Body` really does leave
//! the actor without a rigid body; removing `Look` leaves a positioned,
//! scriptable actor with nothing to draw.
//!
//! Four components have no fixed-field ancestor. [`ActorComponent::Camera`]
//! attaches the world camera to the actor, in first person, third person or
//! plain follow. [`ActorComponent::Script`] names a Rust file in the project's
//! `assets/scripts`, compiled and loaded by the runtime (see
//! [`crate::script`]). [`ActorComponent::Custom`] is a named bag of values the
//! project invents - `Health { hp, armour }` - which blocks, scripts and the
//! inspector all read and write by name.

use crate::ai::BrainSpec;
use crate::animation::AnimationSpec;
use crate::material::{ParticleSpec, SurfaceMaterial, TrailSpec};
use crate::probe::ProbeSpec;
use crate::scene::{Physics, Placement, Visual};
use crate::sprite2d::SpriteSpec;
use crate::terrain::TerrainSpec;
use crate::tilemap::{ParallaxSpec, RoomSpec};
use crate::value::Evaluated;
use crate::volume::VolumeSpec;
use crate::water::{BuoyancySpec, WaterSpec};
use serde::{Deserialize, Serialize};

/// The components every project knows about by name. A custom component
/// can't take one of these names.
pub const BUILT_IN_NAMES: &[&str] = &[
    "Place",
    "Look",
    "Render",
    "Body",
    "Joint",
    "Brain",
    "Camera",
    "Script",
    "Parent",
    "Material",
    "Emitter",
    "Trail",
    "Light",
    "Animation",
    "Volume",
    "Probe",
    "Terrain",
    "Sprite",
    "Fracture",
    "Water",
    "Buoyancy",
    "Parallax",
    "Room",
    "Persist",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum JointKind {
    #[default]
    Fixed,
    Hinge,
    Rope,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JointSpec {
    /// Actor id at the other end of the joint.
    pub target: String,
    #[serde(default)]
    pub kind: JointKind,
    /// Anchor in this actor's local frame.
    #[serde(default)]
    pub anchor: [f32; 3],
    /// Maximum rope length; ignored by fixed and hinge joints.
    #[serde(default = "default_rope_length")]
    pub length: f32,
}

fn default_rope_length() -> f32 {
    2.0
}

impl Default for JointSpec {
    fn default() -> Self {
        Self {
            target: String::new(),
            kind: JointKind::Fixed,
            anchor: [0.0; 3],
            length: default_rope_length(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LightKind {
    /// Shines every way from the actor.
    #[default]
    Point,
    /// A cone down the actor's forward (-Z) axis.
    Spot,
    /// A glowing rectangle facing the actor's forward (-Z) axis, `width` by
    /// `height`. Soft, LTC-shaded speculars; casts no shadow maps.
    Rect,
    /// A glowing disc facing forward, `width` across. Drawn as the square of
    /// the same area, which is what Bevy's area lights offer.
    Disk,
}

/// What a light's `intensity` is measured in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LightUnit {
    /// Luminous power, all directions together.
    #[default]
    Lumens,
    /// Luminous intensity down the brightest direction - what an IES file
    /// and a fixture's datasheet quote.
    Candela,
}

/// A light in physical units. 3D only: a 2D world has no lights.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LightSpec {
    #[serde(default)]
    pub kind: LightKind,
    #[serde(default = "default_light_color")]
    pub color: String,
    /// Luminous power in lumens. A household bulb is about 800. A spot's
    /// power isn't gathered into its cone, so a narrow one is no brighter.
    #[serde(default = "default_lumens")]
    pub intensity: f32,
    /// Metres past which the light no longer reaches.
    #[serde(default = "default_light_range")]
    pub range: f32,
    /// Metres. A bigger emitter softens highlights and shadow edges.
    #[serde(default)]
    pub radius: f32,
    /// Degrees from the spot's axis where the falloff starts.
    #[serde(default = "default_inner_angle")]
    pub inner_angle: f32,
    /// Degrees from the spot's axis where the light ends.
    #[serde(default = "default_outer_angle")]
    pub outer_angle: f32,
    /// Whether the light casts shadow maps. An area light casts a point
    /// light's, from its centre.
    #[serde(default)]
    pub shadows: bool,
    #[serde(default)]
    pub unit: LightUnit,
    /// Metres across a rect light (or a disk's diameter), and a rect's height.
    #[serde(default = "default_area_side")]
    pub width: f32,
    #[serde(default = "default_area_side")]
    pub height: f32,
    /// An image asset projected through the light like a gobo; only its red
    /// channel counts. A point light repeats it on each of its six sides.
    /// Empty for none.
    #[serde(default)]
    pub cookie: String,
    /// How many times the cookie repeats across the beam.
    #[serde(default = "default_cookie_tiling")]
    pub cookie_tiling: f32,
    /// An `.ies` profile shaping a spot's beam. With [`LightUnit::Candela`],
    /// `intensity` is the profile's peak.
    #[serde(default)]
    pub ies: String,
    /// Screen-space shadows under feet and small clutter, which shadow maps
    /// are too coarse to catch. Needs the project's contact shadows on.
    #[serde(default)]
    pub contact_shadows: bool,
    /// Shadows that soften with distance from their caster (PCSS). Needs the
    /// light's `radius` above zero to have a size to soften by.
    #[serde(default)]
    pub soft_shadows: bool,
    /// Overrides Bevy's own shadow biases when set.
    #[serde(default)]
    pub shadow_depth_bias: Option<f32>,
    #[serde(default)]
    pub shadow_normal_bias: Option<f32>,
    /// Whether ray-traced lighting traces this light. Off leaves it out of
    /// the traced world, lighting only what the raster rig still draws.
    #[serde(default = "default_true")]
    pub ray_traced: bool,
    /// Whether it lights volumetric fog.
    #[serde(default = "default_true")]
    pub volumetric: bool,
    /// A visible beam of its own, and the dust drifting in it.
    #[serde(default)]
    pub beam: crate::fog::Beam,
}

fn default_true() -> bool {
    true
}

fn default_area_side() -> f32 {
    1.0
}

fn default_cookie_tiling() -> f32 {
    1.0
}

fn default_light_color() -> String {
    "#FFFFFF".to_string()
}

fn default_lumens() -> f32 {
    800.0
}

fn default_light_range() -> f32 {
    20.0
}

fn default_inner_angle() -> f32 {
    30.0
}

fn default_outer_angle() -> f32 {
    45.0
}

impl Default for LightSpec {
    fn default() -> Self {
        Self {
            kind: LightKind::Point,
            color: default_light_color(),
            intensity: default_lumens(),
            range: default_light_range(),
            radius: 0.0,
            inner_angle: default_inner_angle(),
            outer_angle: default_outer_angle(),
            shadows: false,
            unit: LightUnit::Lumens,
            width: default_area_side(),
            height: default_area_side(),
            cookie: String::new(),
            cookie_tiling: default_cookie_tiling(),
            ies: String::new(),
            contact_shadows: false,
            soft_shadows: false,
            shadow_depth_bias: None,
            shadow_normal_bias: None,
            ray_traced: true,
            volumetric: true,
            beam: crate::fog::Beam::default(),
        }
    }
}

impl LightSpec {
    /// The spot cone in radians, inner never past outer and outer short of
    /// a half turn, which is as wide as a cone gets.
    pub fn cone(&self) -> (f32, f32) {
        let outer = self.outer_angle.clamp(0.1, 89.9).to_radians();
        let inner = self.inner_angle.clamp(0.0, 89.9).to_radians().min(outer);
        (inner, outer)
    }

    /// The intensity as lumens, which is what every Bevy light takes. A
    /// candela figure spreads over the whole sphere, the way Bevy's point
    /// and spot lights do; `peak` is the brightest direction's share of it
    /// (an IES profile's max over its own normalised table is 1).
    pub fn lumens(&self) -> f32 {
        let intensity = self.intensity.max(0.0);
        match self.unit {
            LightUnit::Lumens => intensity,
            LightUnit::Candela => intensity * 4.0 * std::f32::consts::PI,
        }
    }

    /// A rect's sides in metres, or for a disk the side of the square of
    /// its area (which the renderer turns back into the disk).
    pub fn area_size(&self) -> (f32, f32) {
        let width = self.width.max(0.01);
        match self.kind {
            LightKind::Disk => {
                let side = width * std::f32::consts::PI.sqrt() / 2.0;
                (side, side)
            }
            _ => (width, self.height.max(0.01)),
        }
    }

    pub fn is_area(&self) -> bool {
        matches!(self.kind, LightKind::Rect | LightKind::Disk)
    }
}

/// One component on an actor. Serialized internally-tagged, so a component
/// reads as `{"component": "Body", "physics": {...}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "component")]
#[allow(clippy::large_enum_variant)]
pub enum ActorComponent {
    /// Where the actor stands - its Bevy `Transform`. Every actor has one:
    /// there is nowhere for an entity without a transform to be drawn.
    Place { placement: Placement },
    /// What the actor looks like, and so what shape it collides with.
    Look { visual: Visual },
    /// Whether the actor is drawn, and how it sorts. Absent means visible
    /// on layer 0. In 2D a higher layer draws on top: the final depth is
    /// the placement's z plus the layer.
    Render {
        visible: bool,
        #[serde(default)]
        layer: i32,
    },
    /// A rigid body and collider. Absent means blocks move the actor and
    /// nothing else does.
    Body { physics: Physics },
    /// Constrains this body to another actor. A chain of hinged bodies makes
    /// a ragdoll while the usual parent hierarchy remains independent.
    Joint { joint: JointSpec },
    /// A behavior tree with a target, sight cone and crowd spacing.
    Brain { brain: BrainSpec },
    /// Puts the world camera on this actor.
    Camera { camera: CameraAttach },
    /// A Rust file under the project's `assets/scripts`, compiled to a shared
    /// library the runtime loads and ticks. `path` is relative to the project
    /// folder - see [`crate::script`].
    Script { path: String },
    /// Hangs this actor off another one: every move the parent makes is
    /// made to it too. `parent` is the other actor's id.
    ///
    /// `offset` is where the child stands in its parent's frame. `None` -
    /// which is what a document written before offsets says - means the
    /// child keeps the world position its `Place` gives it, both when the
    /// world is built and when `set my parent to` hangs it off someone new.
    Parent {
        parent: String,
        #[serde(default)]
        offset: Option<[f32; 3]>,
    },
    /// A named set of values this project invented, readable and writable
    /// from blocks.
    Custom {
        name: String,
        #[serde(default)]
        fields: Vec<ComponentField>,
    },
    /// What the surface is made of: PBR properties and optional custom
    /// shader effect. Absent means flat color or plain image.
    Material { material: SurfaceMaterial },
    Fracture {
        fracture: crate::destruction::FractureSpec,
    },
    /// A particle emitter: sparks, smoke, splash. Runs while attached, so
    /// `detach` doubles as the stop button for a burst.
    Emitter { emitter: ParticleSpec },
    /// A motion trail: fading snapshots of where the actor just was.
    Trail { trail: TrailSpec },
    /// A point or spot light riding the actor.
    Light { light: LightSpec },
    /// Sprite flipbooks and the named states over them. The runtime's player
    /// swaps the displayed frame; `play clip` changes state and `when
    /// animation ends` fires the transition.
    Animation { animation: AnimationSpec },
    /// An environment volume: a region that lays its own look over the
    /// project's, blended by where the camera stands.
    Volume { volume: VolumeSpec },
    /// A light probe: a box whose reflections (a cubemap) or bounced light
    /// (an irradiance grid) are baked from where it stands.
    Probe { probe: ProbeSpec },
    /// Heightmap ground centred on the actor, with painted layers, grass
    /// and scattered trees and rocks. 3D only.
    Terrain { terrain: TerrainSpec },
    /// 2D dials over the look: flips, 9-slice, stacking, palette swap,
    /// outline, and the order and Y-sort inside the Render layer.
    Sprite { sprite: SpriteSpec },
    /// An ocean, a lake or a river whose surface rests at the actor.
    Water { water: WaterSpec },
    /// Floats a body on whatever water it is in.
    Buoyancy { buoyancy: BuoyancySpec },
    /// A 2D layer that scrolls at its own rate against the camera.
    Parallax { parallax: ParallaxSpec },
    /// A 2D room: bounds the camera keeps inside, and `when actor enters
    /// room` fires across.
    Room { room: RoomSpec },
    /// Keeps this actor across scene switches: a `switch scene to` carries it,
    /// live position, variables and attached components included, into the
    /// new scene instead of unloading it with the old one. Opt-in per actor,
    /// the DontDestroyOnLoad half of multi-scene support.
    Persist,
}

impl ActorComponent {
    /// What the editor and the blocks call this component.
    pub fn name(&self) -> &str {
        match self {
            ActorComponent::Place { .. } => "Place",
            ActorComponent::Look { .. } => "Look",
            ActorComponent::Render { .. } => "Render",
            ActorComponent::Body { .. } => "Body",
            ActorComponent::Joint { .. } => "Joint",
            ActorComponent::Brain { .. } => "Brain",
            ActorComponent::Camera { .. } => "Camera",
            ActorComponent::Script { .. } => "Script",
            ActorComponent::Parent { .. } => "Parent",
            ActorComponent::Material { .. } => "Material",
            ActorComponent::Emitter { .. } => "Emitter",
            ActorComponent::Trail { .. } => "Trail",
            ActorComponent::Light { .. } => "Light",
            ActorComponent::Animation { .. } => "Animation",
            ActorComponent::Volume { .. } => "Volume",
            ActorComponent::Probe { .. } => "Probe",
            ActorComponent::Terrain { .. } => "Terrain",
            ActorComponent::Sprite { .. } => "Sprite",
            ActorComponent::Fracture { .. } => "Fracture",
            ActorComponent::Water { .. } => "Water",
            ActorComponent::Buoyancy { .. } => "Buoyancy",
            ActorComponent::Parallax { .. } => "Parallax",
            ActorComponent::Room { .. } => "Room",
            ActorComponent::Persist => "Persist",
            ActorComponent::Custom { name, .. } => name,
        }
    }

    /// True for the one component an actor can't be without.
    pub fn is_required(&self) -> bool {
        matches!(self, ActorComponent::Place { .. })
    }
}

/// How the camera sits on the actor it is attached to. In a 2D project all
/// three views centre the camera on the actor: there is no depth to stand in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum CameraView {
    /// Keeps the world camera's own offset, centred on the actor.
    #[default]
    Follow,
    /// Sits at the actor's eye: yaw from the actor, `pitch` from this rig.
    /// `set camera pitch` drives that angle mid-run.
    FirstPerson,
    /// Sits `distance` behind and `pitch` degrees above the actor, looking
    /// back at it.
    ThirdPerson,
}

/// The camera component's settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraAttach {
    #[serde(default)]
    pub view: CameraView,
    /// Offset from the actor's origin, in the actor's own frame: the eye in
    /// first person, the point the boom looks at in third.
    #[serde(default = "default_eye")]
    pub offset: [f32; 3],
    /// How far behind the actor a third-person camera sits.
    #[serde(default = "default_distance")]
    pub distance: f32,
    /// How far above the actor a third-person camera looks down from, in
    /// degrees; a first-person camera's look-up angle, also in degrees.
    #[serde(default = "default_pitch")]
    pub pitch: f32,
    /// Vertical field of view, in degrees. What a settings slider drives.
    #[serde(default = "default_fov")]
    pub fov: f32,
}

fn default_eye() -> [f32; 3] {
    [0.0, 0.6, 0.0]
}

fn default_distance() -> f32 {
    6.0
}

fn default_pitch() -> f32 {
    15.0
}

fn default_fov() -> f32 {
    75.0
}

impl Default for CameraAttach {
    fn default() -> Self {
        Self {
            view: CameraView::default(),
            offset: default_eye(),
            distance: default_distance(),
            pitch: default_pitch(),
            fov: default_fov(),
        }
    }
}

/// One value on a custom component.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentField {
    pub name: String,
    pub value: Evaluated,
}

impl ComponentField {
    pub fn number(name: impl Into<String>, value: f64) -> Self {
        Self {
            name: name.into(),
            value: Evaluated::Number(value),
        }
    }
}

/// An actor's components, in the order the editor shows them. Kept as a list
/// rather than a map so the inspector has a stable order to draw, and so a
/// component's name can live on the component itself.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Components(pub Vec<ActorComponent>);

impl Components {
    /// The components a freshly made actor has: somewhere to stand, something
    /// to draw, and drawn. A body is added when the actor needs one.
    pub fn new(visual: Visual) -> Self {
        Self(vec![
            ActorComponent::Place {
                placement: Placement::default(),
            },
            ActorComponent::Look { visual },
            ActorComponent::Render {
                visible: true,
                layer: 0,
            },
        ])
    }

    pub fn iter(&self) -> impl Iterator<Item = &ActorComponent> {
        self.0.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut ActorComponent> {
        self.0.iter_mut()
    }

    pub fn get(&self, name: &str) -> Option<&ActorComponent> {
        self.0.iter().find(|component| component.name() == name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut ActorComponent> {
        self.0.iter_mut().find(|component| component.name() == name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Adds `component`, replacing one of the same name. Returns whether it
    /// was new.
    pub fn insert(&mut self, component: ActorComponent) -> bool {
        match self.get_mut(component.name()) {
            Some(slot) => {
                *slot = component;
                false
            }
            None => {
                self.0.push(component);
                true
            }
        }
    }

    /// Drops the named component. The required one stays; everything else
    /// goes, and reads of it fall back to its default.
    pub fn remove(&mut self, name: &str) -> bool {
        let Some(index) = self
            .0
            .iter()
            .position(|component| component.name() == name && !component.is_required())
        else {
            return false;
        };
        self.0.remove(index);
        true
    }

    // ─── The built-ins, as the rest of the engine reads them ───────────────

    /// What the actor looks like, or `None` when it has no `Look` - a
    /// positioned actor its blocks can still drive, with nothing to draw.
    pub fn visual(&self) -> Option<&Visual> {
        match self.get("Look") {
            Some(ActorComponent::Look { visual }) => Some(visual),
            _ => None,
        }
    }

    pub fn set_visual(&mut self, visual: Visual) {
        self.insert(ActorComponent::Look { visual });
    }

    pub fn placement(&self) -> Placement {
        match self.get("Place") {
            Some(ActorComponent::Place { placement }) => *placement,
            _ => Placement::default(),
        }
    }

    /// The actor's placement, adding a default `Place` first if it somehow
    /// has none.
    pub fn placement_mut(&mut self) -> &mut Placement {
        if !self.contains("Place") {
            self.0.insert(
                0,
                ActorComponent::Place {
                    placement: Placement::default(),
                },
            );
        }
        match self.get_mut("Place") {
            Some(ActorComponent::Place { placement }) => placement,
            _ => unreachable!("just inserted"),
        }
    }

    pub fn set_placement(&mut self, placement: Placement) {
        *self.placement_mut() = placement;
    }

    /// The actor's physics, or the inert default when it has no `Body`.
    pub fn physics(&self) -> Physics {
        match self.get("Body") {
            Some(ActorComponent::Body { physics }) => *physics,
            _ => Physics::default(),
        }
    }

    pub fn set_physics(&mut self, physics: Physics) {
        self.insert(ActorComponent::Body { physics });
    }

    pub fn joint(&self) -> Option<&JointSpec> {
        match self.get("Joint") {
            Some(ActorComponent::Joint { joint }) => Some(joint),
            _ => None,
        }
    }

    pub fn brain(&self) -> Option<&BrainSpec> {
        match self.get("Brain") {
            Some(ActorComponent::Brain { brain }) => Some(brain),
            _ => None,
        }
    }

    /// Whether the actor is drawn. An actor with no `Render` is visible.
    pub fn visible(&self) -> bool {
        match self.get("Render") {
            Some(ActorComponent::Render { visible, .. }) => *visible,
            _ => true,
        }
    }

    pub fn set_visible(&mut self, visible: bool) {
        let layer = self.layer();
        self.insert(ActorComponent::Render { visible, layer });
    }

    /// The 2D sort layer. An actor with no `Render` sorts on layer 0.
    pub fn layer(&self) -> i32 {
        match self.get("Render") {
            Some(ActorComponent::Render { layer, .. }) => *layer,
            _ => 0,
        }
    }

    pub fn set_layer(&mut self, layer: i32) {
        let visible = self.visible();
        self.insert(ActorComponent::Render { visible, layer });
    }

    /// The surface material, if the actor carries one.
    pub fn material(&self) -> Option<&SurfaceMaterial> {
        match self.get("Material") {
            Some(ActorComponent::Material { material }) => Some(material),
            _ => None,
        }
    }

    /// The particle emitter, if the actor carries one.
    pub fn emitter(&self) -> Option<&ParticleSpec> {
        match self.get("Emitter") {
            Some(ActorComponent::Emitter { emitter }) => Some(emitter),
            _ => None,
        }
    }

    /// The motion trail, if the actor carries one.
    pub fn trail(&self) -> Option<&TrailSpec> {
        match self.get("Trail") {
            Some(ActorComponent::Trail { trail }) => Some(trail),
            _ => None,
        }
    }

    /// The volume, if the actor carries one.
    pub fn volume(&self) -> Option<&VolumeSpec> {
        match self.get("Volume") {
            Some(ActorComponent::Volume { volume }) => Some(volume),
            _ => None,
        }
    }

    /// The light probe, if the actor carries one.
    pub fn probe(&self) -> Option<&ProbeSpec> {
        match self.get("Probe") {
            Some(ActorComponent::Probe { probe }) => Some(probe),
            _ => None,
        }
    }

    /// The terrain, if the actor carries one.
    pub fn terrain(&self) -> Option<&TerrainSpec> {
        match self.get("Terrain") {
            Some(ActorComponent::Terrain { terrain }) => Some(terrain),
            _ => None,
        }
    }

    /// The water, if the actor carries some.
    pub fn water(&self) -> Option<&WaterSpec> {
        match self.get("Water") {
            Some(ActorComponent::Water { water }) => Some(water),
            _ => None,
        }
    }

    /// How the actor floats, if it does.
    pub fn buoyancy(&self) -> Option<&BuoyancySpec> {
        match self.get("Buoyancy") {
            Some(ActorComponent::Buoyancy { buoyancy }) => Some(buoyancy),
            _ => None,
        }
    }

    /// The parallax layer, if the actor is one.
    pub fn parallax(&self) -> Option<&ParallaxSpec> {
        match self.get("Parallax") {
            Some(ActorComponent::Parallax { parallax }) => Some(parallax),
            _ => None,
        }
    }

    /// Whether this actor opted into surviving scene switches: a `Persist`
    /// component, authored or attached mid-run.
    pub fn persists(&self) -> bool {
        self.contains("Persist")
    }

    /// The room, if the actor is one.
    pub fn room(&self) -> Option<&RoomSpec> {
        match self.get("Room") {
            Some(ActorComponent::Room { room }) => Some(room),
            _ => None,
        }
    }

    /// The light, if the actor carries one.
    pub fn light(&self) -> Option<&LightSpec> {
        match self.get("Light") {
            Some(ActorComponent::Light { light }) => Some(light),
            _ => None,
        }
    }

    /// The authored flipbooks and states, if the actor carries them.
    pub fn animation(&self) -> Option<&AnimationSpec> {
        match self.get("Animation") {
            Some(ActorComponent::Animation { animation }) => Some(animation),
            _ => None,
        }
    }

    /// The 2D sprite dials, if the actor carries them.
    pub fn sprite(&self) -> Option<&SpriteSpec> {
        match self.get("Sprite") {
            Some(ActorComponent::Sprite { sprite }) => Some(sprite),
            _ => None,
        }
    }

    pub fn camera(&self) -> Option<&CameraAttach> {
        match self.get("Camera") {
            Some(ActorComponent::Camera { camera }) => Some(camera),
            _ => None,
        }
    }

    /// The script file this actor runs, if any.
    pub fn script(&self) -> Option<&str> {
        match self.get("Script") {
            Some(ActorComponent::Script { path }) => Some(path),
            _ => None,
        }
    }

    /// The actor this one hangs off, by id, if any.
    pub fn parent(&self) -> Option<&str> {
        match self.get("Parent") {
            Some(ActorComponent::Parent { parent, .. }) if !parent.is_empty() => Some(parent),
            _ => None,
        }
    }

    /// Where this actor stands in its parent's frame, if it was authored
    /// that way. Nothing reads it without a parent to read it against.
    pub fn parent_offset(&self) -> Option<[f32; 3]> {
        match self.get("Parent") {
            Some(ActorComponent::Parent { parent, offset }) if !parent.is_empty() => *offset,
            _ => None,
        }
    }

    /// Hangs the actor off `parent`, or takes it off whatever it was on when
    /// `parent` is empty. An offset already authored stays: it is about where
    /// the child stands, not about which actor it hangs off.
    pub fn set_parent(&mut self, parent: &str) {
        if parent.is_empty() {
            self.remove("Parent");
            return;
        }
        let offset = self.parent_offset();
        self.insert(ActorComponent::Parent {
            parent: parent.to_string(),
            offset,
        });
    }

    /// Places the actor in its parent's frame, or puts it back in world
    /// coordinates with `None`. Silent without a parent to stand against.
    pub fn set_parent_offset(&mut self, offset: Option<[f32; 3]>) {
        let Some(parent) = self.parent().map(str::to_string) else {
            return;
        };
        self.insert(ActorComponent::Parent { parent, offset });
    }

    // ─── Custom components ─────────────────────────────────────────────────

    /// Every custom component, by name, with its fields - what the runtime
    /// puts on the entity and the blocks read back.
    pub fn custom(&self) -> impl Iterator<Item = (&str, &[ComponentField])> {
        self.0.iter().filter_map(|component| match component {
            ActorComponent::Custom { name, fields } => Some((name.as_str(), fields.as_slice())),
            _ => None,
        })
    }

    /// One field of one custom component.
    pub fn field(&self, component: &str, field: &str) -> Option<&Evaluated> {
        match self.get(component) {
            Some(ActorComponent::Custom { fields, .. }) => fields
                .iter()
                .find(|candidate| candidate.name == field)
                .map(|candidate| &candidate.value),
            _ => None,
        }
    }

    /// Writes a field of an existing custom component, adding the field if it
    /// isn't declared yet. Returns false when there's no such component -
    /// only the editor creates components, so a block can't conjure one.
    pub fn set_field(&mut self, component: &str, field: &str, value: Evaluated) -> bool {
        let Some(ActorComponent::Custom { fields, .. }) = self.get_mut(component) else {
            return false;
        };
        match fields.iter_mut().find(|candidate| candidate.name == field) {
            Some(slot) => slot.value = value,
            None => fields.push(ComponentField {
                name: field.to_string(),
                value,
            }),
        }
        true
    }

    /// An unused custom-component name based on `base`, never colliding with
    /// a built-in - `"Health"`, then `"Health 2"`.
    pub fn unique_custom_name(&self, base: &str) -> String {
        let base = base.trim();
        let base = if base.is_empty() { "Component" } else { base };
        let taken =
            |name: &str| BUILT_IN_NAMES.contains(&name) || self.0.iter().any(|c| c.name() == name);
        if !taken(base) {
            return base.to_string();
        }
        (2..)
            .map(|n| format!("{base} {n}"))
            .find(|candidate| !taken(candidate))
            .expect("an unused name always exists")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::BodyKind;

    fn rect() -> Visual {
        Visual::Rect {
            color: "#4C97FF".to_string(),
            size: [10.0, 10.0],
        }
    }

    #[test]
    fn a_missing_component_reads_as_its_default() {
        let mut components = Components::new(rect());
        assert_eq!(components.physics().body, BodyKind::None);
        assert!(components.visible());
        assert!(components.visual().is_some());

        components.remove("Look");
        components.remove("Render");
        assert!(components.visual().is_none());
        // Nothing to draw isn't the same as hidden: `Render` is about a
        // visual that exists.
        assert!(components.visible());
    }

    #[test]
    fn the_place_component_cant_be_removed() {
        let mut components = Components::new(rect());
        assert!(!components.remove("Place"));
        assert!(components.contains("Place"));
    }

    #[test]
    fn inserting_replaces_a_component_of_the_same_name() {
        let mut components = Components::new(rect());
        assert!(components.insert(ActorComponent::Body {
            physics: Physics {
                body: BodyKind::Dynamic,
                ..Physics::default()
            },
        }));
        assert!(!components.insert(ActorComponent::Body {
            physics: Physics {
                body: BodyKind::Static,
                ..Physics::default()
            },
        }));
        assert_eq!(components.physics().body, BodyKind::Static);
        assert_eq!(components.0.len(), 4);
    }

    #[test]
    fn a_block_writes_an_existing_custom_component_and_no_other() {
        let mut components = Components::new(rect());
        components.insert(ActorComponent::Custom {
            name: "Health".to_string(),
            fields: vec![ComponentField::number("hp", 3.0)],
        });

        assert!(components.set_field("Health", "hp", Evaluated::Number(1.0)));
        assert_eq!(
            components.field("Health", "hp"),
            Some(&Evaluated::Number(1.0))
        );
        // A field nobody declared is added; a component nobody declared isn't.
        assert!(components.set_field("Health", "armour", Evaluated::Number(2.0)));
        assert!(!components.set_field("Mana", "amount", Evaluated::Number(2.0)));
        assert!(components.field("Mana", "amount").is_none());
    }

    #[test]
    fn a_custom_name_never_collides_with_a_built_in_or_another_component() {
        let mut components = Components::new(rect());
        assert_eq!(components.unique_custom_name("Body"), "Body 2");
        components.insert(ActorComponent::Custom {
            name: "Health".to_string(),
            fields: Vec::new(),
        });
        assert_eq!(components.unique_custom_name("Health"), "Health 2");
        assert_eq!(components.unique_custom_name("  "), "Component");
    }

    #[test]
    fn a_component_reads_as_its_tagged_json_shape() {
        let json = serde_json::to_value(ActorComponent::Camera {
            camera: CameraAttach {
                view: CameraView::ThirdPerson,
                ..CameraAttach::default()
            },
        })
        .unwrap();
        assert_eq!(json["component"], "Camera");
        assert_eq!(json["camera"]["view"], "ThirdPerson");
    }

    #[test]
    fn a_bare_light_reads_as_a_bulb_and_its_cone_stays_a_cone() {
        let component: ActorComponent =
            serde_json::from_str(r#"{"component":"Light","light":{"kind":"Spot"}}"#).unwrap();
        let ActorComponent::Light { light } = component else {
            panic!("not a light");
        };
        assert_eq!(light.kind, LightKind::Spot);
        assert_eq!(light.intensity, 800.0);
        let wide = LightSpec {
            inner_angle: 120.0,
            outer_angle: 10.0,
            ..LightSpec::default()
        };
        let (inner, outer) = wide.cone();
        assert!(inner <= outer);
        assert!(outer < std::f32::consts::FRAC_PI_2);
    }
}
