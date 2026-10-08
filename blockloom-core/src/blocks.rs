//! Blockloom's block vocabulary - the instructions its canvas speaks - and
//! its `BlockKind` impl, the hinge onto blockstitch's document model.
//!
//! Every numeric, text or boolean slot is a [`Value`], so any of them can
//! hold an expression or a reporter block. Everything else on an instruction
//! (an axis, a key name, a body kind) is a fixed in-place dropdown the
//! frontend rewrites with `edit_instruction`.

use crate::animation::TweenEasing;
use crate::components::CameraView;
use crate::scene::{Axis, BodyKind};
use crate::sound::SoundBus;
use crate::ui::{UiAnchor, UiKind, UiProp, UiTheme};
use crate::value::Value;
use serde::{Deserialize, Serialize};

/// The 2D sprite dials `set sprite [dial] to` can change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SpriteDial {
    FlipX,
    FlipY,
    Order,
    YSort,
    Palette,
    OutlineWidth,
}

impl SpriteDial {
    /// A dial by the name a script or compiled logic sends.
    pub fn parse(name: &str) -> Option<SpriteDial> {
        match name
            .trim()
            .to_lowercase()
            .replace(['_', '-', ' '], "")
            .as_str()
        {
            "flipx" => Some(SpriteDial::FlipX),
            "flipy" => Some(SpriteDial::FlipY),
            "order" => Some(SpriteDial::Order),
            "ysort" => Some(SpriteDial::YSort),
            "palette" => Some(SpriteDial::Palette),
            "outlinewidth" | "outline" => Some(SpriteDial::OutlineWidth),
            _ => None,
        }
    }
}

/// The 2D look `set [look] to` can change for the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Look2dDial {
    /// The ambient multiplier over the project's colour.
    AmbientLight,
    /// A colour that replaces the ambient colour; empty puts it back.
    AmbientColor,
    /// Pixels per block of the picture (0 or 1 is off).
    Pixelation,
    /// Colour levels per channel (0 is off).
    Levels,
    /// 0-1 ordered dither across the quantize step.
    Dither,
    /// 0-1 edge outline strength.
    Outline,
    /// 0-1 CRT scanline depth.
    Scanlines,
    /// 0-1 CRT screen bow.
    Curvature,
    /// 0-1 CRT darkened corners.
    Vignette,
    /// Camera zoom in pixels per world unit.
    Zoom,
    /// Camera view height in world units.
    ZoomHeight,
    /// Camera roll in degrees.
    Rotation,
    /// Adds trauma (0-1) to the camera shake.
    Shake,
    /// Freezes the world for this many seconds; the interface keeps going.
    Hitstop,
    BoundsLeft,
    BoundsRight,
    BoundsBottom,
    BoundsTop,
    /// 1 keeps the camera's x where it is.
    LockX,
    LockY,
    /// 1 snaps the camera to whole pixels.
    PixelSnap,
}

impl Look2dDial {
    /// A dial by the name a script or compiled logic sends.
    pub fn parse(name: &str) -> Option<Look2dDial> {
        match name
            .trim()
            .to_lowercase()
            .replace(['_', '-', ' '], "")
            .as_str()
        {
            "ambientlight" | "ambient" => Some(Look2dDial::AmbientLight),
            "ambientcolor" | "ambientcolour" => Some(Look2dDial::AmbientColor),
            "pixelation" | "pixelate" => Some(Look2dDial::Pixelation),
            "levels" => Some(Look2dDial::Levels),
            "dither" => Some(Look2dDial::Dither),
            "outline" => Some(Look2dDial::Outline),
            "scanlines" => Some(Look2dDial::Scanlines),
            "curvature" => Some(Look2dDial::Curvature),
            "vignette" => Some(Look2dDial::Vignette),
            "zoom" => Some(Look2dDial::Zoom),
            "zoomheight" | "viewheight" => Some(Look2dDial::ZoomHeight),
            "rotation" | "roll" => Some(Look2dDial::Rotation),
            "shake" => Some(Look2dDial::Shake),
            "hitstop" => Some(Look2dDial::Hitstop),
            "boundsleft" => Some(Look2dDial::BoundsLeft),
            "boundsright" => Some(Look2dDial::BoundsRight),
            "boundsbottom" => Some(Look2dDial::BoundsBottom),
            "boundstop" => Some(Look2dDial::BoundsTop),
            "lockx" => Some(Look2dDial::LockX),
            "locky" => Some(Look2dDial::LockY),
            "pixelsnap" => Some(Look2dDial::PixelSnap),
            _ => None,
        }
    }
}

/// Runtime emitter settings available to blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EmitterDial {
    Rate,
    Lifetime,
    Speed,
    Spread,
    Gravity,
    SizeStart,
    SizeEnd,
    Max,
}

impl EmitterDial {
    /// A dial by the name a script spells it, any case, spaces or not.
    pub fn parse(name: &str) -> Option<Self> {
        match name
            .trim()
            .to_ascii_lowercase()
            .replace([' ', '_'], "")
            .as_str()
        {
            "rate" => Some(Self::Rate),
            "lifetime" => Some(Self::Lifetime),
            "speed" => Some(Self::Speed),
            "spread" => Some(Self::Spread),
            "gravity" => Some(Self::Gravity),
            "sizestart" => Some(Self::SizeStart),
            "sizeend" => Some(Self::SizeEnd),
            "max" => Some(Self::Max),
            _ => None,
        }
    }
}

pub use blockstitch_core::graph::{
    BlockDef, BlockGraph, BlockKind, BlockPiece, BlockShape, Comment, DictDef, DictEntry, DictItem,
    FloatingValue, InputValueType, ListDef, ListItem, VariableDef, default_block_color,
    dict_lookup, dict_remove, dict_set, dict_to_json, is_dict_reporter, is_list_reporter,
    list_index, list_to_json, normalize_block_color, parse_json_array, parse_json_object,
    rename_dict_in_value, rename_list_in_value, resolve_dict_reporter, resolve_dict_reporters,
    resolve_list_reporter, resolve_list_reporters,
};

/// One instruction on a canvas: a [`InstructionKind`] plus the stable id
/// blockstitch tracks it by.
pub type Instruction = blockstitch_core::graph::Instruction<InstructionKind>;
/// One draggable stack of instructions.
pub type Strand = blockstitch_core::graph::Strand<InstructionKind>;
/// Everything on one actor's canvas - see [`crate::project::Actor`].
pub type ActorGraph = BlockGraph<InstructionKind>;

/// Serialized internally-tagged, so an instruction is the flat
/// `{"type": "Move", "steps": ...}` object the frontend already speaks (see
/// [`crate::wire`] for the `id` half).
#[derive(Debug, Clone, PartialEq, Hash, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum InstructionKind {
    // ─── Events (headers) ───────────────────────────────────────────────────
    /// Runs when the project starts. The green flag.
    WhenStarted,
    WhenQualityDrops,
    /// Runs every time `key` goes down.
    WhenKeyPressed {
        key: String,
    },
    /// Runs every time the named input action goes down. An action groups
    /// keys, mouse buttons and gamepad inputs under one name, so one strand
    /// answers a jump however the player says it.
    WhenActionPressed {
        action: String,
    },
    /// Runs when a finger touches the screen. The touch reporters say where.
    WhenTouched,
    /// Runs when this actor is clicked.
    WhenClicked,
    /// Runs when this actor starts (Enter), keeps (Stay) or stops (Exit)
    /// touching `with` (an actor name, or an empty string for "anything").
    /// `scope` narrows it to solid touches or trigger overlaps.
    WhenCollision {
        with: String,
        #[serde(default)]
        phase: crate::physics::ContactPhase,
        #[serde(default)]
        scope: crate::physics::ContactScope,
    },
    /// Runs when any actor broadcasts `name`.
    WhenMessage {
        name: String,
    },
    /// Runs on a fresh clone, in the clone itself, the moment it is made.
    WhenCloned,
    /// Runs when the named clip finishes a `Once` pass. An empty `clip`
    /// matches any clip ending, which is what a state machine transition
    /// wants when it doesn't care which state just left.
    WhenAnimationEnds {
        clip: String,
    },
    /// Runs in the emitting actor when its particles spawn, die or hit
    /// something: at most once a frame per event, however many there were.
    WhenParticles {
        #[serde(default)]
        event: crate::vfx::ParticleEvent,
    },
    /// Runs each time the playing clip reaches the named frame marker (or a
    /// rig animation's event). Empty matches any marker.
    WhenAnimationMarker {
        marker: String,
    },
    /// Runs in an actor each time it walks into a room (an actor with a
    /// Room component), named by the room's name. Empty matches any room.
    WhenEnterRoom {
        room: String,
    },
    /// Runs in the newly loaded scene's actors after a `switch scene to`
    /// finishes loading it, and once at the start of the first scene.
    WhenSceneStarts,
    /// Runs when the weather blend lands on the named preset (an empty name
    /// matches any weather arriving).
    WhenWeather {
        weather: String,
    },
    /// Runs in the outgoing scene's actors before a `switch scene to`
    /// unloads it.
    WhenSceneEnds,
    /// Runs when the playing cutscene passes the named signal marker.
    /// Empty matches any signal.
    WhenCutsceneSignal {
        signal: String,
    },
    /// Runs when the playing cutscene reaches its end marker, by skipping
    /// or by playing through.
    WhenCutsceneEnds,
    /// Runs when the interface element named `element` is clicked.
    ///
    /// Spelled `element` rather than `id` because a flattened instruction
    /// already carries its own `id` on the wire - see [`crate::wire`].
    WhenUiEvent {
        element: String,
        event: String,
    },
    WhenUiClicked {
        element: String,
    },
    /// Runs when the input element named `element` is changed - every
    /// keystroke in a text input, every drag step of a slider, a toggle
    /// going over.
    WhenUiChanged {
        element: String,
    },
    /// Runs when a plugin fires `event` (a hat its `BlockSchema` adds). An
    /// empty entry of `args` matches anything; the others must equal what the
    /// plugin fired, as text, in the schema's slot order.
    WhenPlugin {
        plugin: String,
        block: String,
        event: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// Marks a strand as a custom block's body; `block_id` is its
    /// [`BlockDef::id`]. Never runs on its own.
    BlockHeader {
        block_id: String,
    },

    // ─── Motion ─────────────────────────────────────────────────────────────
    /// Forward along the actor's own facing, by `steps` units.
    Move {
        steps: Value,
    },
    /// Jump straight to a position. `z` is ignored in a 2D project.
    GoTo {
        x: Value,
        y: Value,
        z: Value,
    },
    /// One step toward a position along the navmesh, at `speed` units per
    /// second. Steers around static obstacles; with no route it steps
    /// straight at the target instead.
    NavigateTo {
        x: Value,
        y: Value,
        z: Value,
        speed: Value,
    },
    ChangePosition {
        axis: Axis,
        by: Value,
    },
    /// Slides to a position over `seconds`, one step per rendered frame.
    /// `easing` shapes the motion: linear glides at one speed, the rest ease
    /// in, out, or bounce. Old documents without one read as linear.
    Glide {
        seconds: Value,
        x: Value,
        y: Value,
        z: Value,
        #[serde(default)]
        easing: TweenEasing,
    },
    /// Tweens the size towards `factor` over `seconds`, eased.
    TweenScale {
        factor: Value,
        seconds: Value,
        easing: TweenEasing,
    },
    /// Tweens one axis towards `degrees` over `seconds`, eased.
    TweenRotation {
        axis: Axis,
        degrees: Value,
        seconds: Value,
        easing: TweenEasing,
    },
    /// Tweens the tint towards `color` over `seconds`, eased. No-op on an
    /// image actor, like `set color`.
    TweenColor {
        color: Value,
        seconds: Value,
        easing: TweenEasing,
    },
    /// Stops every tween on this actor where it stands: glides included.
    StopTweens,
    /// Plays the named flipbook clip at `speed` (1 is as authored). A state
    /// whose clip this names changes state too; an unknown name is an error
    /// the run log shows.
    PlayAnimation {
        clip: Value,
        speed: Value,
    },
    /// Stops the animation player where it stands, keeping the frame.
    StopAnimation,
    /// Retunes the playing clip's speed. 1 is as authored, 0 freezes.
    SetAnimationSpeed {
        speed: Value,
    },
    /// Fires a named trigger into the animation state machine this tick.
    FireAnimationTrigger {
        name: Value,
    },
    /// Shows a different attachment in one of the rig's slots. Empty hides
    /// the slot; the animation's own swaps win again on its next key.
    SetRigSlot {
        slot: Value,
        attachment: Value,
    },
    /// Tints one rig slot, over its authored color.
    SetSlotTint {
        slot: Value,
        color: Value,
    },
    /// Points a rig IK constraint at a spot, relative to the actor.
    SetIkTarget {
        constraint: Value,
        x: Value,
        y: Value,
    },
    /// Changes one 2D sprite dial for the rest of the run.
    SetSpriteDial {
        dial: SpriteDial,
        value: Value,
    },
    Turn {
        axis: Axis,
        degrees: Value,
    },
    SetRotation {
        axis: Axis,
        degrees: Value,
    },
    /// Faces another actor by name, or the mouse pointer (`"mouse"`).
    PointTowards {
        target: String,
    },
    SetScale {
        factor: Value,
    },
    /// Sets the camera's exposure in EV100 for the rest of the run: lower is
    /// brighter. Outranks auto-exposure and the project's own value.
    SetRenderSetting {
        setting: crate::quality::Setting,
        value: Value,
    },
    /// Changes one dial of the 2D lit look for the rest of the run.
    SetLook2d {
        dial: Look2dDial,
        value: Value,
    },
    SetExposure {
        ev: Value,
    },
    /// Sets this actor's light, in lumens. Nothing happens without a Light.
    SetLightIntensity {
        intensity: Value,
    },
    /// Sets how strongly this actor's surface glows, as a multiple of its
    /// emissive tint (its look color when the tint is black). 3D only.
    SetEmissiveStrength {
        strength: Value,
    },
    /// Turns HDR output on or off for the rest of the run, where the
    /// display offers it. Window-global.
    SetHdrOutput {
        enabled: bool,
    },
    /// Sets the display's peak brightness in nits for the rest of the run.
    SetPeakBrightness {
        nits: Value,
    },
    /// Switches an environment volume on or off for the rest of the run.
    /// Names an actor or an id; empty means this actor.
    EnableVolume {
        enabled: bool,
        volume: Value,
    },
    /// Sets an environment volume's weight, 0-1, for the rest of the run.
    SetVolumeWeight {
        volume: Value,
        weight: Value,
    },
    /// Re-captures every light probe from where it stands now, for the rest
    /// of the run. Nothing is written to disk.
    CaptureProbes,
    /// How far the sun's shadows reach, in metres, for the rest of the run.
    SetShadowDistance {
        distance: Value,
    },
    /// Whether this actor's light casts shadows, for the rest of the run.
    SetLightShadows {
        enabled: bool,
    },
    /// Ray-traced lighting on or off for the rest of the run, where the GPU
    /// can trace rays. Window-global.
    SetRayTracing {
        enabled: bool,
    },
    /// Most bounces a ray-traced light path takes, for the rest of the run.
    SetGiBounces {
        bounces: Value,
    },
    /// Light samples per pixel for ray-traced lighting, for the rest of the
    /// run.
    SetGiSamples {
        samples: Value,
    },
    /// Height fog's extinction per metre at its base, for the rest of the
    /// run. 0 clears the air. Window-global.
    SetFogDensity {
        density: Value,
    },
    /// The aurora's KP index, 0-9, for the rest of the run. 0 puts it out.
    SetAurora {
        kp: Value,
    },
    /// Break an actor carrying Fracture into physical debris.
    Fracture {
        target: Value,
    },
    /// Disturb nearby water and wet the surface map.
    Splash {
        x: Value,
        y: Value,
        z: Value,
        radius: Value,
        strength: Value,
    },
    /// Inject a puff into a bounded smoke advection grid.
    PuffSmoke {
        x: Value,
        y: Value,
        z: Value,
        radius: Value,
        strength: Value,
    },
    /// A temporary 3D surface mark, with its outward normal and lifetime.
    SpawnDecal {
        preset: crate::decals::DecalPreset,
        x: Value,
        y: Value,
        z: Value,
        nx: Value,
        ny: Value,
        nz: Value,
        size: Value,
        lifetime: Value,
        fade: Value,
    },
    /// Fade marks whose centres lie inside a world-space sphere.
    FadeDecals {
        x: Value,
        y: Value,
        z: Value,
        radius: Value,
        seconds: Value,
    },
    /// A lightning strike landing at a point: a flash, a pulse of the sky
    /// and thunder late by the distance.
    StrikeLightning {
        x: Value,
        y: Value,
        z: Value,
    },
    /// Strikes a minute the storm throws, for the rest of the run. 0 calms it.
    SetLightningRate {
        rate: Value,
    },
    /// One of the wind's dials for the rest of the run: its direction in
    /// degrees, speed, gust strength or storm 0-1. Window-global.
    SetWind {
        property: crate::wind::WindProperty,
        value: Value,
    },
    /// Volumetric cloud coverage, density or type for the rest of the run.
    /// Window-global.
    SetClouds {
        property: crate::clouds::CloudProperty,
        value: Value,
    },
    /// The clock in hours, 0-24, for the rest of the run. Moves the
    /// director's sun and every track reading from it. Window-global.
    SetTimeOfDay {
        time: Value,
    },
    /// Moves the director's clock by hours (negative rewinds), for the rest
    /// of the run. Window-global.
    AdvanceTime {
        hours: Value,
    },
    /// Rain or snow intensity, 0-1, for the rest of the run. Window-global.
    SetPrecipitation {
        property: crate::director::PrecipitationKind,
        value: Value,
    },
    /// Blends the weather towards a preset (Clear, Overcast, Storm, Sunset,
    /// Night, or one the project authored) over seconds. Starting a new
    /// blend mid-blend carries on from where the air is, so rapid changes
    /// never pop. Window-global.
    BlendWeather {
        weather: Value,
        seconds: Value,
    },
    /// A water dial for the rest of the run: the running actor's own water
    /// when it has some, every body's otherwise.
    SetWater {
        property: crate::water::WaterProperty,
        value: Value,
    },
    /// Paints one cell of a tilemap for the rest of the run, at a world
    /// point (z only matters in 3D). `map` names the tilemap; empty means
    /// this actor if it is one, else whichever map covers the point. Tile -1
    /// erases.
    PaintTile {
        map: Value,
        tile: Value,
        x: Value,
        y: Value,
        #[serde(default = "zero")]
        z: Value,
    },
    /// A parallax layer's scroll factor (0-2) for the rest of the run.
    SetParallax {
        layer: Value,
        axis: crate::tilemap::ParallaxAxis,
        value: Value,
    },
    /// One cloud layer's dial (layer counted from 1) for the rest of the run.
    /// Window-global.
    SetCloudLayer {
        layer: Value,
        property: crate::cloud_layers::CloudLayerProperty,
        value: Value,
    },
    /// Extra drift of the clouds, world units per second, for the rest of
    /// the run.
    SetCloudDrift {
        x: Value,
        y: Value,
        z: Value,
    },

    // ─── Physics ────────────────────────────────────────────────────────────
    SetBody {
        body: BodyKind,
    },
    /// One-shot push, in units per second. Only a dynamic body responds.
    ApplyImpulse {
        x: Value,
        y: Value,
        z: Value,
    },
    SetVelocity {
        x: Value,
        y: Value,
        z: Value,
    },
    /// A force on this body for one fixed step, read per `mode`. Only a
    /// dynamic body responds.
    AddForce {
        mode: crate::physics::ForceMode,
        x: Value,
        y: Value,
        z: Value,
    },
    /// A torque on this body for one fixed step, read per `mode`. 2D bodies
    /// turn about z only.
    AddTorque {
        mode: crate::physics::ForceMode,
        x: Value,
        y: Value,
        z: Value,
    },
    /// Moves this actor's character controller: a displacement, or with
    /// `Simple` a speed with gravity. Needs a CharacterController component.
    ControllerMove {
        #[serde(default)]
        mode: crate::physics::controller::MoveMode,
        x: Value,
        y: Value,
        z: Value,
    },
    /// Steers this actor's character motor or fires one of its actions: a
    /// direction, jump, sprint, crouch, a knockback push or a stop. Needs a
    /// CharacterMotor component; a player-owned motor reads the keys itself.
    MotorAct {
        #[serde(default)]
        action: crate::physics::motor::MotorAction,
        x: Value,
        y: Value,
        z: Value,
    },
    /// Commands one of this actor's constraints by name (or place): switch it
    /// on or off, break it, or change its motor or spring for the run. The
    /// value is a speed, a target, a force, a stiffness or a damping.
    JointAct {
        #[serde(default)]
        action: crate::physics::joints::JointVerb,
        #[serde(default)]
        joint: String,
        value: Value,
    },
    /// Changes one setting of this actor's character motor for the run.
    SetMotor {
        property: crate::physics::motor::MotorProperty,
        value: Value,
    },
    /// Changes one setting of this actor's character controller for the run.
    SetController {
        property: crate::physics::controller::ControllerProperty,
        value: Value,
    },
    /// Casts a ray along a segment and keeps what it crossed as this actor's
    /// query result, read back by the `hit` reporters. The ray ignores this
    /// actor's own colliders.
    CastRay {
        #[serde(default)]
        hits: crate::physics::query::RayHits,
        #[serde(default)]
        triggers: crate::physics::query::TriggerPolicy,
        from_x: Value,
        from_y: Value,
        from_z: Value,
        to_x: Value,
        to_y: Value,
        to_z: Value,
    },
    /// Sweeps a ball along a segment and keeps the first thing it meets as
    /// this actor's query result.
    CastBall {
        #[serde(default)]
        triggers: crate::physics::query::TriggerPolicy,
        radius: Value,
        from_x: Value,
        from_y: Value,
        from_z: Value,
        to_x: Value,
        to_y: Value,
        to_z: Value,
    },
    /// Keeps everything a ball at a point overlaps as this actor's query
    /// result.
    OverlapBall {
        #[serde(default)]
        triggers: crate::physics::query::TriggerPolicy,
        radius: Value,
        x: Value,
        y: Value,
        z: Value,
    },
    /// Keeps the collider nearest a point, within `range`, as this actor's
    /// query result.
    FindClosest {
        #[serde(default)]
        triggers: crate::physics::query::TriggerPolicy,
        range: Value,
        x: Value,
        y: Value,
        z: Value,
    },
    /// World gravity, not this actor's - see
    /// [`crate::scene::Physics::gravity_scale`] for the per-actor dial.
    SetGravity {
        x: Value,
        y: Value,
        z: Value,
    },
    /// How heavy this actor is for its size - a `2` shoves a `1` aside and
    /// resists being shoved itself.
    SetDensity {
        density: Value,
    },
    /// An explicit body mass; set, it wins over [`SetDensity`] and the shape.
    SetMass {
        mass: Value,
    },
    /// Whether this actor's collider pushes back (solid) or only senses
    /// overlap (trigger). A trigger still fires `when I touch` and answers
    /// `touching?`, which is what a coin or a goal zone wants.
    SetTrigger {
        trigger: bool,
    },
    /// Which collision layer the actor lives on, 1-8. Two bodies only pair
    /// when each one's mask names the other's layer.
    SetCollisionLayer {
        layer: Value,
    },
    /// Bitmask of the layers this actor pairs with, 0-255 (bit `n - 1` for
    /// layer `n`). Raycasts use the running actor's own mask as their filter.
    SetCollisionMask {
        mask: Value,
    },

    // ─── Looks ──────────────────────────────────────────────────────────────
    /// Shows a speech bubble over the actor. An empty text clears it.
    Say {
        text: Value,
    },
    SetVisible {
        visible: bool,
    },
    /// A `#RRGGBB` string. No-op on an image actor.
    SetColor {
        color: Value,
    },
    /// Spawns particles immediately, including from a zero-rate emitter.
    BurstParticles {
        count: Value,
    },
    /// Changes one emitter setting for this run.
    SetEmitterDial {
        dial: EmitterDial,
        value: Value,
    },
    /// Starts or stops recording snapshots for this run.
    SetTrailEnabled {
        enabled: bool,
    },
    /// Starts or stops the emitter spawning. Live particles fly on either
    /// way; starting again restarts the burst clock.
    SetEmitterPlaying {
        playing: bool,
    },

    // ─── Sound ──────────────────────────────────────────────────────────────
    /// Plays a sound file from the project's assets (`assets/sounds/jump.wav`)
    /// as a global voice - no position, no panning. `volume` is 0-100,
    /// `pitch` is 1 for as recorded, and `bus` is which mixing bus it routes
    /// through. Every play is a new voice, so rapid replays overlap rather
    /// than cut each other off.
    PlaySound {
        sound: Value,
        volume: Value,
        pitch: Value,
        #[serde(rename = "loop")]
        loop_: bool,
        bus: SoundBus,
    },
    /// Plays a sound at an actor's place in the world and follows it around,
    /// panned by where it stands relative to the camera and quieter with
    /// distance. `target` names an actor by id or name; empty means here, at
    /// whoever ran the block.
    PlaySoundAt {
        sound: Value,
        volume: Value,
        pitch: Value,
        #[serde(rename = "loop")]
        loop_: bool,
        bus: SoundBus,
        target: Value,
    },
    /// Stops the voices playing a sound file. An empty slot stops every sound
    /// at once - the block equivalent of `stop all` for audio.
    StopSound {
        sound: Value,
    },
    /// Retunes the voices already playing a sound file - an engine hum that
    /// follows the speed, a loop that ducks under dialogue. Future plays
    /// still start at the volume their own block names.
    SetSoundVolume {
        sound: Value,
        volume: Value,
    },
    /// Rebends the voices already playing a sound file. `1` is as recorded.
    SetSoundPitch {
        sound: Value,
        pitch: Value,
    },
    /// Moves a whole mixing bus: every present and future voice routed through
    /// it. `volume` is 0-100, like a play block's.
    SetBusVolume {
        bus: SoundBus,
        volume: Value,
    },

    // ─── Components ─────────────────────────────────────────────────────────
    /// Writes one field of one of this actor's custom components. A field the
    /// component doesn't declare yet is added; a component it doesn't have is
    /// an error, since only the editor creates components.
    SetComponentField {
        component: String,
        field: String,
        value: Value,
    },
    /// Switches the camera component's view, so a game can go from third to
    /// first person mid-run. Does nothing on an actor with no camera.
    SetCameraView {
        view: CameraView,
    },
    /// Tilts a first-person camera up or down, in degrees. Positive looks up.
    /// Clamped to just short of vertical, so the view can never flip over.
    /// The body stays level, so this is what first-person mouse look drives.
    SetCameraPitch {
        degrees: Value,
    },
    /// Sets the camera's vertical field of view, in degrees. Clamped to a
    /// usable lens range, so a settings slider cannot break the projection.
    SetCameraFov {
        fov: Value,
    },
    /// Gives this actor a component mid-run. One the editor authored comes
    /// back as it was left; anything else arrives with its defaults.
    AttachComponent {
        component: String,
    },
    /// Takes a component away mid-run. `Place` can't go.
    DetachComponent {
        component: String,
    },
    /// Hangs this actor off another one, so the two move together. An empty
    /// target takes it off whatever it was on. Names an actor or an id. A
    /// child carrying an authored offset is placed at it - that far from its
    /// new parent, in the parent's own frame - and one without an offset
    /// keeps the place it is standing in.
    SetParent {
        parent: Value,
    },

    // ─── Actors ─────────────────────────────────────────────────────────────
    /// Makes a running copy of an actor - its components as they stand, its
    /// canvas, and its own variables - and starts its `when I start as a
    /// clone` strands. An empty `of` clones whoever ran the block.
    CreateClone {
        of: String,
    },
    /// Makes a brand-new actor the project never authored: somewhere to
    /// stand, something plain to see, and no blocks. It lasts as long as the
    /// run does.
    CreateActor {
        name: Value,
        x: Value,
        y: Value,
        z: Value,
    },
    /// Takes an actor out of the running world and stops its scripts. The
    /// document is untouched, so Play puts an authored one back. An empty
    /// target deletes whoever ran the block.
    DeleteActor {
        target: Value,
    },

    // ─── Interface ──────────────────────────────────────────────────────────
    /// Creates the element `id` names, or updates it in place when it is
    /// already there - so a HUD strand can rebuild itself every frame.
    ///
    /// One variant per kind, because the row differs: a panel has a modal
    /// dropdown, a slider its ends, a toggle its starting state. Everything
    /// they share - where it sits, how big it is, whose child it is - is
    /// spelled the same way on all of them.
    ShowPanel {
        element: Value,
        title: Value,
        modal: bool,
        anchor: UiAnchor,
        x: Value,
        y: Value,
        width: Value,
        height: Value,
        parent: Value,
    },
    ShowLabel {
        element: Value,
        text: Value,
        anchor: UiAnchor,
        x: Value,
        y: Value,
        width: Value,
        height: Value,
        parent: Value,
    },
    ShowButton {
        element: Value,
        label: Value,
        anchor: UiAnchor,
        x: Value,
        y: Value,
        width: Value,
        height: Value,
        parent: Value,
    },
    /// `asset` is a project-relative path, the same spelling a Look's image
    /// uses (`assets/sprites/logo.png`).
    ShowImage {
        element: Value,
        asset: Value,
        anchor: UiAnchor,
        x: Value,
        y: Value,
        width: Value,
        height: Value,
        parent: Value,
    },
    ShowInput {
        element: Value,
        placeholder: Value,
        anchor: UiAnchor,
        x: Value,
        y: Value,
        width: Value,
        height: Value,
        parent: Value,
    },
    ShowSlider {
        element: Value,
        min: Value,
        max: Value,
        value: Value,
        anchor: UiAnchor,
        x: Value,
        y: Value,
        width: Value,
        height: Value,
        parent: Value,
    },
    ShowToggle {
        element: Value,
        label: Value,
        on: bool,
        anchor: UiAnchor,
        x: Value,
        y: Value,
        width: Value,
        height: Value,
        parent: Value,
    },
    /// A vertical container with a clipped, wheel-scrollable viewport.
    ShowWidget {
        kind: UiKind,
        element: Value,
        text: Value,
        anchor: UiAnchor,
        x: Value,
        y: Value,
        width: Value,
        height: Value,
        parent: Value,
    },
    ShowList {
        element: Value,
        anchor: UiAnchor,
        x: Value,
        y: Value,
        width: Value,
        height: Value,
        parent: Value,
    },
    /// Changes the defaults for elements that have no explicit style.
    SetUiTheme {
        theme: UiTheme,
    },
    /// Writes one property of an existing element. A property that means
    /// nothing for that kind is ignored, and an id nothing answers to is an
    /// error the run log shows.
    BindUi {
        element: Value,
        value: Value,
    },
    SetUiItems {
        element: Value,
        value: Value,
    },
    ScrollUi {
        element: Value,
        value: Value,
    },
    SetElementTheme {
        element: Value,
        value: Value,
    },
    SetUiProp {
        prop: UiProp,
        element: Value,
        value: Value,
    },
    /// Takes an element off the screen without forgetting it, children and
    /// all. `show` or `set visible` brings it back.
    HideElement {
        element: Value,
    },
    /// Hides every element at once, and drops keyboard focus with them.
    HideAllUi,
    /// Forgets an element entirely, children and all.
    DeleteElement {
        element: Value,
    },
    /// Hands the keyboard to a text input without waiting for a click, so a
    /// menu can open with its field already live. An element that isn't a
    /// text input takes the keyboard off whoever had it, as clicking away
    /// does.
    FocusElement {
        element: Value,
    },
    /// Takes the keyboard back off whichever input holds it.
    ClearFocus,
    /// Freezes the world: no world strand advances, no physics steps, no key
    /// or collision event queues. Strands a UI click started keep running,
    /// which is what makes a pause menu's buttons work.
    PauseGame,
    ResumeGame,

    // ─── Saved data ────────────────────────────────────────────────────────
    /// Writes this actor's visible variable slot to per-player save data.
    SaveVariable {
        name: String,
    },
    /// Removes a slot from per-player save data without changing it live.
    ClearSavedVariable {
        name: String,
    },
    /// Switches which save slot this run writes to and loads that slot's
    /// saved variables into the run, so a slots menu can offer "Slot 1" or
    /// a player name. A name with no file yet starts fresh; the strand
    /// carries on, unlike `switch scene to`.
    SwitchSaveSlot {
        slot: Value,
    },
    /// Deletes one save slot's file without touching the live run. Quiet
    /// when nothing by that name was saved.
    DeleteSaveSlot {
        slot: Value,
    },
    /// Speaks the run's language for the rest of the run: what `text for
    /// key` answers in. Empty reads as the project's default language.
    SetLanguage {
        language: Value,
    },

    // ─── Control ────────────────────────────────────────────────────────────
    /// Loads another scene by name and continues the run there: the current
    /// world's actors stop, globals and save data carry over, and the new
    /// scene's `when scene starts` strands run once it is warm. `transition`
    /// is `none`, `fade`, `wipe` or `circle`; unknown spellings read as
    /// `none`. The strand that asked stops where it stands.
    SwitchScene {
        scene: Value,
        transition: Value,
    },
    /// Plays the named cutscene reel on the wall clock: camera cuts, dolly
    /// moves, signal markers, slow-motion and volume keys. The strand that
    /// asked carries on; `when cutscene ends` runs when the reel does.
    PlayCutscene {
        cutscene: Value,
    },
    /// Jumps the playing cutscene to its end marker: remaining signals fire
    /// in order, then `when cutscene ends` runs. Quiet with none playing.
    SkipCutscene,
    /// Kicks the camera trauma 0-1 higher: the view shakes on seeded noise
    /// and settles as the trauma decays. Adds to what is already shaking.
    CameraShake {
        amount: Value,
    },
    /// Scales world time for the rest of the run: 1 is normal speed, 0.5
    /// halves it, 0 freezes world strands while interface strands tick on.
    /// A cutscene's slow-motion keys only move it while no block has.
    SetTimeScale {
        scale: Value,
    },
    /// Freezes world strands for `frames` render frames - the punch landing
    /// - while interface strands and the cutscene clock tick on.
    Hitstop {
        frames: Value,
    },
    /// Shows the cinematic letterbox bars for a nonzero `on`, hides them
    /// for zero. They sit over the world but under the fade.
    SetLetterbox {
        on: Value,
    },
    /// Fades the screen to `black` or `white`, or clears the fade for
    /// `none`. Unknown spellings read as `none`.
    FadeScreen {
        color: Value,
    },
    /// Suspends this script for `duration` seconds.
    Wait {
        duration: Value,
    },
    /// Suspends this script until `condition` is true, re-checked each frame.
    WaitUntil {
        condition: Value,
    },
    If {
        condition: Value,
        body: Vec<Instruction>,
    },
    IfElse {
        condition: Value,
        then_body: Vec<Instruction>,
        else_body: Vec<Instruction>,
    },
    Repeat {
        count: Value,
        body: Vec<Instruction>,
    },
    Forever {
        body: Vec<Instruction>,
    },
    While {
        condition: Value,
        body: Vec<Instruction>,
    },
    /// Stops the nearest enclosing loop. A no-op outside one.
    EscapeLoop,
    /// Skips to the next iteration of the nearest enclosing loop.
    ContinueLoop,
    /// Fires every `WhenMessage` strand listening for `name`, in every actor.
    Broadcast {
        name: String,
    },
    /// Stops every running script, this one included.
    StopAll,
    /// Grabs the pointer for first-person play: locked to the window and
    /// hidden, so `mouse delta` never stalls at a screen edge. Unlocking
    /// shows it again. The run always ends unlocked.
    SetMouseLocked {
        locked: bool,
    },
    // ─── Input ────────────────────────────────────────────────────────────
    /// Rumbles connected gamepads at `strength` (0-100) for `duration`
    /// seconds. Does nothing with no gamepad attached.
    RumbleGamepad {
        strength: Value,
        duration: Value,
    },
    /// Adds one binding (`space`, `mouse:left`, `gamepad:south`,
    /// `gamepad:leftstickx-`) to the named input action for the rest of the
    /// run. What a settings screen calls to remap a key.
    BindAction {
        action: Value,
        binding: Value,
    },
    /// Forgets every binding an action has for the rest of the run. Pair
    /// with `bind` to replace a mapping rather than add to it.
    ClearActionBindings {
        action: Value,
    },

    // ─── Variables ──────────────────────────────────────────────────────────
    SetVariable {
        name: String,
        value: Value,
    },
    /// Adds to the named variable, coercing a non-numeric one to `0` first.
    ChangeVariable {
        name: String,
        value: Value,
    },

    // ─── Lists ──────────────────────────────────────────────────────────────
    /// Appends a number/text value to a named list. Boolean values are ignored.
    AddToList {
        value: Value,
        name: String,
    },
    /// Removes the 1-based item at `index` from a named list.
    DeleteOfList {
        index: Value,
        name: String,
    },
    /// Removes every item from a named list.
    DeleteAllOfList {
        name: String,
    },
    /// Rotates a named list by `amount` positions (positive is toward the end).
    ShiftList {
        name: String,
        amount: Value,
    },
    /// Inserts a number/text value at the 1-based `index` in a named list.
    InsertIntoList {
        value: Value,
        index: Value,
        name: String,
    },
    /// Replaces the 1-based item at `index` in a named list with a literal.
    ReplaceItemOfList {
        index: Value,
        name: String,
        value: Value,
    },
    /// Reverses a named list in place.
    ReverseList {
        name: String,
    },

    // ─── Dicts ──────────────────────────────────────────────────────────────
    /// Sets `key` in a named dict to a number/text value. Replaces the entry
    /// when the key exists, appends one when it does not. Bools are ignored.
    SetDictValue {
        key: Value,
        name: String,
        value: Value,
    },
    /// Removes `key` from a named dict. Missing keys are a no-op.
    DeleteDictKey {
        key: Value,
        name: String,
    },
    /// Removes every entry from a named dict.
    DeleteAllOfDict {
        name: String,
    },

    // ─── JSON ───────────────────────────────────────────────────────────────
    /// Parses `json` as a JSON object and loads it into a named dict,
    /// replacing its entries. A parse error is reported and leaves the dict.
    LoadJsonIntoDict {
        json: Value,
        name: String,
    },
    /// Parses `json` as a JSON array and loads it into a named list,
    /// replacing its items. A parse error is reported and leaves the list.
    LoadJsonIntoList {
        json: Value,
        name: String,
    },

    // ─── Custom blocks ──────────────────────────────────────────────────────
    /// Runs a custom block's body inline, with `args` bound to its inputs.
    CallBlock {
        block_id: String,
        args: Vec<Value>,
    },
    /// Only meaningful in a reporter block's body: evaluates and returns.
    Return {
        value: Value,
    },

    // ─── Plugin blocks ──────────────────────────────────────────────────────
    /// A statement a plugin's `BlockSchema`
    /// adds. `args` follow the schema's slot order; the editor runs the
    /// block's command with them when the instruction executes.
    PluginBlock {
        plugin: String,
        block: String,
        args: Vec<Value>,
    },
}

/// Blockloom's half of the block-editor contract: where its instructions
/// keep values and bodies, which are headers, and how field ids map onto
/// their slots. Every traversal built on this lives in blockstitch.
impl BlockKind for InstructionKind {
    const HEADER_LABEL: &'static str = "event/block definition";

    fn visit_values_mut(&mut self, f: &mut dyn FnMut(&mut Value, InputValueType)) {
        use InstructionKind as K;
        match self {
            K::Move { steps: v }
            | K::ChangePosition { by: v, .. }
            | K::Turn { degrees: v, .. }
            | K::SetRotation { degrees: v, .. }
            | K::SetScale { factor: v }
            | K::SetRenderSetting { value: v, .. }
            | K::SetLook2d { value: v, .. }
            | K::SetExposure { ev: v }
            | K::SetLightIntensity { intensity: v }
            | K::SetEmissiveStrength { strength: v }
            | K::SetPeakBrightness { nits: v }
            | K::SetShadowDistance { distance: v }
            | K::SetGiBounces { bounces: v }
            | K::SetGiSamples { samples: v }
            | K::SetFogDensity { density: v }
            | K::SetAurora { kp: v }
            | K::SetLightningRate { rate: v }
            | K::SetWind { value: v, .. }
            | K::SetClouds { value: v, .. }
            | K::SetTimeOfDay { time: v }
            | K::AdvanceTime { hours: v }
            | K::SetPrecipitation { value: v, .. }
            | K::SetWater { value: v, .. }
            | K::SetDensity { density: v }
            | K::SetMass { mass: v }
            | K::SetController { value: v, .. }
            | K::SetMotor { value: v, .. }
            | K::JointAct { value: v, .. }
            | K::SetCollisionLayer { layer: v }
            | K::SetCollisionMask { mask: v }
            | K::Say { text: v }
            | K::SetColor { color: v }
            | K::BurstParticles { count: v }
            | K::SetEmitterDial { value: v, .. }
            | K::SetSpriteDial { value: v, .. }
            | K::FireAnimationTrigger { name: v }
            | K::Wait { duration: v }
            | K::SetVariable { value: v, .. }
            | K::SetCameraPitch { degrees: v, .. }
            | K::SetCameraFov { fov: v, .. }
            | K::ChangeVariable { value: v, .. }
            | K::AddToList { value: v, .. }
            | K::Return { value: v }
            | K::SetComponentField { value: v, .. }
            | K::SetParent { parent: v }
            | K::Fracture { target: v }
            | K::DeleteActor { target: v }
            | K::StopSound { sound: v }
            | K::SetBusVolume { volume: v, .. }
            | K::SwitchSaveSlot { slot: v }
            | K::DeleteSaveSlot { slot: v }
            | K::SetLanguage { language: v }
            | K::Repeat { count: v, .. } => f(v, InputValueType::Any),
            K::GoTo { x, y, z }
            | K::StrikeLightning { x, y, z }
            | K::SetCloudDrift { x, y, z }
            | K::ApplyImpulse { x, y, z }
            | K::SetVelocity { x, y, z }
            | K::AddForce { x, y, z, .. }
            | K::AddTorque { x, y, z, .. }
            | K::ControllerMove { x, y, z, .. }
            | K::MotorAct { x, y, z, .. }
            | K::SetGravity { x, y, z } => {
                f(x, InputValueType::Any);
                f(y, InputValueType::Any);
                f(z, InputValueType::Any);
            }
            K::CastRay {
                from_x,
                from_y,
                from_z,
                to_x,
                to_y,
                to_z,
                ..
            } => {
                for v in [from_x, from_y, from_z, to_x, to_y, to_z] {
                    f(v, InputValueType::Any);
                }
            }
            K::CastBall {
                radius,
                from_x,
                from_y,
                from_z,
                to_x,
                to_y,
                to_z,
                ..
            } => {
                for v in [radius, from_x, from_y, from_z, to_x, to_y, to_z] {
                    f(v, InputValueType::Any);
                }
            }
            K::OverlapBall {
                radius, x, y, z, ..
            }
            | K::FindClosest {
                range: radius,
                x,
                y,
                z,
                ..
            } => {
                for v in [radius, x, y, z] {
                    f(v, InputValueType::Any);
                }
            }
            K::Splash {
                x,
                y,
                z,
                radius,
                strength,
            }
            | K::PuffSmoke {
                x,
                y,
                z,
                radius,
                strength,
            } => {
                for v in [x, y, z, radius, strength] {
                    f(v, InputValueType::Any);
                }
            }
            K::SpawnDecal {
                x,
                y,
                z,
                nx,
                ny,
                nz,
                size,
                lifetime,
                fade,
                ..
            } => {
                for v in [x, y, z, nx, ny, nz, size, lifetime, fade] {
                    f(v, InputValueType::Any);
                }
            }
            K::FadeDecals {
                x,
                y,
                z,
                radius,
                seconds,
            } => {
                for v in [x, y, z, radius, seconds] {
                    f(v, InputValueType::Any);
                }
            }
            K::NavigateTo { x, y, z, speed } => {
                f(x, InputValueType::Any);
                f(y, InputValueType::Any);
                f(z, InputValueType::Any);
                f(speed, InputValueType::Any);
            }
            K::Glide {
                seconds, x, y, z, ..
            } => {
                f(seconds, InputValueType::Any);
                f(x, InputValueType::Any);
                f(y, InputValueType::Any);
                f(z, InputValueType::Any);
            }
            K::TweenScale {
                factor, seconds, ..
            } => {
                f(factor, InputValueType::Any);
                f(seconds, InputValueType::Any);
            }
            K::TweenRotation {
                degrees, seconds, ..
            } => {
                f(degrees, InputValueType::Any);
                f(seconds, InputValueType::Any);
            }
            K::TweenColor { color, seconds, .. } => {
                f(color, InputValueType::Any);
                f(seconds, InputValueType::Any);
            }
            K::PlayAnimation { clip, speed } => {
                f(clip, InputValueType::Any);
                f(speed, InputValueType::Any);
            }
            K::SetAnimationSpeed { speed } => f(speed, InputValueType::Any),
            K::SetRigSlot { slot, attachment } => {
                f(slot, InputValueType::Any);
                f(attachment, InputValueType::Any);
            }
            K::SetSlotTint { slot, color } => {
                f(slot, InputValueType::Any);
                f(color, InputValueType::Any);
            }
            K::SetIkTarget { constraint, x, y } => {
                f(constraint, InputValueType::Any);
                f(x, InputValueType::Any);
                f(y, InputValueType::Any);
            }
            K::EnableVolume { volume, .. } => f(volume, InputValueType::Any),
            K::SetVolumeWeight { volume, weight } => {
                f(volume, InputValueType::Any);
                f(weight, InputValueType::Any);
            }
            K::SetCloudLayer { layer, value, .. } => {
                f(layer, InputValueType::Any);
                f(value, InputValueType::Any);
            }
            K::PaintTile { map, tile, x, y, z } => {
                f(map, InputValueType::Any);
                f(tile, InputValueType::Any);
                f(x, InputValueType::Any);
                f(y, InputValueType::Any);
                f(z, InputValueType::Any);
            }
            K::SetParallax { layer, value, .. } => {
                f(layer, InputValueType::Any);
                f(value, InputValueType::Any);
            }
            K::CreateActor { name, x, y, z } => {
                f(name, InputValueType::Any);
                f(x, InputValueType::Any);
                f(y, InputValueType::Any);
                f(z, InputValueType::Any);
            }
            K::BlendWeather { weather, seconds } => {
                f(weather, InputValueType::Any);
                f(seconds, InputValueType::Any);
            }
            K::SwitchScene { scene, transition } => {
                f(scene, InputValueType::Any);
                f(transition, InputValueType::Any);
            }
            K::PlayCutscene { cutscene } => {
                f(cutscene, InputValueType::Any);
            }
            K::CameraShake { amount } => {
                f(amount, InputValueType::Any);
            }
            K::SetTimeScale { scale } => {
                f(scale, InputValueType::Any);
            }
            K::Hitstop { frames } => {
                f(frames, InputValueType::Any);
            }
            K::SetLetterbox { on } => {
                f(on, InputValueType::Any);
            }
            K::FadeScreen { color } => {
                f(color, InputValueType::Any);
            }
            K::PlaySound {
                sound,
                volume,
                pitch,
                ..
            } => {
                f(sound, InputValueType::Any);
                f(volume, InputValueType::Any);
                f(pitch, InputValueType::Any);
            }
            K::PlaySoundAt {
                sound,
                volume,
                pitch,
                target,
                ..
            } => {
                f(sound, InputValueType::Any);
                f(volume, InputValueType::Any);
                f(pitch, InputValueType::Any);
                f(target, InputValueType::Any);
            }
            K::SetSoundVolume { sound, volume } => {
                f(sound, InputValueType::Any);
                f(volume, InputValueType::Any);
            }
            K::SetSoundPitch { sound, pitch } => {
                f(sound, InputValueType::Any);
                f(pitch, InputValueType::Any);
            }
            // Every `show` block reads the same five slots, plus whatever
            // its own kind adds. Order matters: it is the order the VM and
            // a compiled program evaluate them in.
            K::ShowPanel {
                element: id,
                title: content,
                x,
                y,
                width,
                height,
                parent,
                ..
            }
            | K::ShowLabel {
                element: id,
                text: content,
                x,
                y,
                width,
                height,
                parent,
                ..
            }
            | K::ShowWidget {
                element: id,
                text: content,
                x,
                y,
                width,
                height,
                parent,
                ..
            }
            | K::ShowButton {
                element: id,
                label: content,
                x,
                y,
                width,
                height,
                parent,
                ..
            }
            | K::ShowImage {
                element: id,
                asset: content,
                x,
                y,
                width,
                height,
                parent,
                ..
            }
            | K::ShowInput {
                element: id,
                placeholder: content,
                x,
                y,
                width,
                height,
                parent,
                ..
            }
            | K::ShowToggle {
                element: id,
                label: content,
                x,
                y,
                width,
                height,
                parent,
                ..
            } => {
                f(id, InputValueType::Any);
                f(content, InputValueType::Any);
                f(x, InputValueType::Any);
                f(y, InputValueType::Any);
                f(width, InputValueType::Any);
                f(height, InputValueType::Any);
                f(parent, InputValueType::Any);
            }
            K::ShowList {
                element: id,
                x,
                y,
                width,
                height,
                parent,
                ..
            } => {
                f(id, InputValueType::Any);
                f(x, InputValueType::Any);
                f(y, InputValueType::Any);
                f(width, InputValueType::Any);
                f(height, InputValueType::Any);
                f(parent, InputValueType::Any);
            }
            K::ShowSlider {
                element: id,
                min,
                max,
                value,
                x,
                y,
                width,
                height,
                parent,
                ..
            } => {
                f(id, InputValueType::Any);
                f(min, InputValueType::Any);
                f(max, InputValueType::Any);
                f(value, InputValueType::Any);
                f(x, InputValueType::Any);
                f(y, InputValueType::Any);
                f(width, InputValueType::Any);
                f(height, InputValueType::Any);
                f(parent, InputValueType::Any);
            }
            K::BindUi { element: id, value }
            | K::SetUiItems { element: id, value }
            | K::ScrollUi { element: id, value }
            | K::SetElementTheme { element: id, value }
            | K::SetUiProp {
                element: id, value, ..
            } => {
                f(id, InputValueType::Any);
                f(value, InputValueType::Any);
            }
            K::DeleteOfList { index, .. } => f(index, InputValueType::Any),
            K::ShiftList { amount, .. } => f(amount, InputValueType::Any),
            K::InsertIntoList { value, index, .. } => {
                f(value, InputValueType::Any);
                f(index, InputValueType::Any);
            }
            K::ReplaceItemOfList { index, value, .. } => {
                f(index, InputValueType::Any);
                f(value, InputValueType::Any);
            }
            K::DeleteDictKey { key, .. } => f(key, InputValueType::Any),
            K::SetDictValue { key, value, .. } => {
                f(key, InputValueType::Any);
                f(value, InputValueType::Any);
            }
            K::LoadJsonIntoDict { json, .. } | K::LoadJsonIntoList { json, .. } => {
                f(json, InputValueType::Any)
            }
            K::HideElement { element }
            | K::DeleteElement { element }
            | K::FocusElement { element } => f(element, InputValueType::Any),
            K::If { condition, .. } | K::IfElse { condition, .. } | K::While { condition, .. } => {
                f(condition, InputValueType::Bool)
            }
            K::WaitUntil { condition } => f(condition, InputValueType::Bool),
            // A call site's declared Boolean inputs live on the `BlockDef`,
            // not here - `BlockGraph::migrate_bool_slots` handles those.
            K::CallBlock { args, .. } => {
                for arg in args {
                    f(arg, InputValueType::Any);
                }
            }
            K::PluginBlock { args, .. } => {
                for arg in args {
                    f(arg, InputValueType::Any);
                }
            }
            K::RumbleGamepad { strength, duration } => {
                f(strength, InputValueType::Any);
                f(duration, InputValueType::Any);
            }
            K::BindAction { action, binding } => {
                f(action, InputValueType::Any);
                f(binding, InputValueType::Any);
            }
            K::ClearActionBindings { action } => f(action, InputValueType::Any),
            K::WhenStarted
            | K::WhenQualityDrops
            | K::WhenKeyPressed { .. }
            | K::WhenActionPressed { .. }
            | K::WhenTouched
            | K::WhenClicked
            | K::WhenCollision { .. }
            | K::WhenMessage { .. }
            | K::WhenCloned
            | K::WhenAnimationEnds { .. }
            | K::WhenParticles { .. }
            | K::WhenAnimationMarker { .. }
            | K::WhenEnterRoom { .. }
            | K::WhenSceneStarts
            | K::WhenWeather { .. }
            | K::WhenSceneEnds
            | K::WhenCutsceneSignal { .. }
            | K::WhenCutsceneEnds
            | K::WhenPlugin { .. }
            | K::SkipCutscene
            | K::BlockHeader { .. }
            | K::CreateClone { .. }
            | K::PointTowards { .. }
            | K::SetBody { .. }
            | K::SetTrigger { .. }
            | K::SetCameraView { .. }
            | K::AttachComponent { .. }
            | K::DetachComponent { .. }
            | K::SetVisible { .. }
            | K::SetTrailEnabled { .. }
            | K::SetEmitterPlaying { .. }
            | K::CaptureProbes
            | K::SetLightShadows { .. }
            | K::SetRayTracing { .. }
            | K::StopTweens
            | K::StopAnimation
            | K::SetHdrOutput { .. }
            | K::Forever { .. }
            | K::EscapeLoop
            | K::ContinueLoop
            | K::Broadcast { .. }
            | K::StopAll
            | K::SetMouseLocked { .. }
            | K::WhenUiEvent { .. }
            | K::WhenUiClicked { .. }
            | K::WhenUiChanged { .. }
            | K::HideAllUi
            | K::ClearFocus
            | K::PauseGame
            | K::ResumeGame
            | K::SetUiTheme { .. }
            | K::SaveVariable { .. }
            | K::ClearSavedVariable { .. }
            | K::DeleteAllOfList { .. }
            | K::DeleteAllOfDict { .. }
            | K::ReverseList { .. } => {}
        }
    }

    fn is_header(&self) -> bool {
        matches!(
            self,
            InstructionKind::WhenStarted
                | InstructionKind::WhenQualityDrops
                | InstructionKind::WhenKeyPressed { .. }
                | InstructionKind::WhenActionPressed { .. }
                | InstructionKind::WhenTouched
                | InstructionKind::WhenClicked
                | InstructionKind::WhenCollision { .. }
                | InstructionKind::WhenMessage { .. }
                | InstructionKind::WhenCloned
                | InstructionKind::WhenAnimationEnds { .. }
                | InstructionKind::WhenParticles { .. }
                | InstructionKind::WhenAnimationMarker { .. }
                | InstructionKind::WhenEnterRoom { .. }
                | InstructionKind::WhenSceneStarts
                | InstructionKind::WhenWeather { .. }
                | InstructionKind::WhenSceneEnds
                | InstructionKind::WhenCutsceneSignal { .. }
                | InstructionKind::WhenCutsceneEnds
                | InstructionKind::WhenPlugin { .. }
                | InstructionKind::WhenUiEvent { .. }
                | InstructionKind::WhenUiClicked { .. }
                | InstructionKind::WhenUiChanged { .. }
                | InstructionKind::BlockHeader { .. }
        )
    }

    fn body(&self, slot: u8) -> Option<&Vec<Instruction>> {
        use InstructionKind as K;
        match (self, slot) {
            (K::If { body, .. }, 0) => Some(body),
            (K::IfElse { then_body, .. }, 0) => Some(then_body),
            (K::IfElse { else_body, .. }, 1) => Some(else_body),
            (K::Repeat { body, .. }, 0) => Some(body),
            (K::Forever { body }, 0) => Some(body),
            (K::While { body, .. }, 0) => Some(body),
            _ => None,
        }
    }

    fn body_mut(&mut self, slot: u8) -> Option<&mut Vec<Instruction>> {
        use InstructionKind as K;
        match (self, slot) {
            (K::If { body, .. }, 0) => Some(body),
            (K::IfElse { then_body, .. }, 0) => Some(then_body),
            (K::IfElse { else_body, .. }, 1) => Some(else_body),
            (K::Repeat { body, .. }, 0) => Some(body),
            (K::Forever { body }, 0) => Some(body),
            (K::While { body, .. }, 0) => Some(body),
            _ => None,
        }
    }

    fn variable_target_mut(&mut self) -> Option<&mut String> {
        match self {
            InstructionKind::SetVariable { name, .. }
            | InstructionKind::ChangeVariable { name, .. } => Some(name),
            _ => None,
        }
    }

    fn list_target_mut(&mut self) -> Option<&mut String> {
        match self {
            InstructionKind::AddToList { name, .. }
            | InstructionKind::DeleteOfList { name, .. }
            | InstructionKind::DeleteAllOfList { name }
            | InstructionKind::ShiftList { name, .. }
            | InstructionKind::InsertIntoList { name, .. }
            | InstructionKind::ReplaceItemOfList { name, .. }
            | InstructionKind::ReverseList { name }
            | InstructionKind::LoadJsonIntoList { name, .. } => Some(name),
            _ => None,
        }
    }

    fn dict_target_mut(&mut self) -> Option<&mut String> {
        match self {
            InstructionKind::SetDictValue { name, .. }
            | InstructionKind::DeleteDictKey { name, .. }
            | InstructionKind::DeleteAllOfDict { name }
            | InstructionKind::LoadJsonIntoDict { name, .. } => Some(name),
            _ => None,
        }
    }

    fn calls_block(&self, block_id: &str) -> bool {
        matches!(self, InstructionKind::CallBlock { block_id: id, .. } if id == block_id)
    }

    fn call_args_mut(&mut self, block_id: &str) -> Option<&mut Vec<Value>> {
        match self {
            InstructionKind::CallBlock { block_id: id, args } if id == block_id => Some(args),
            _ => None,
        }
    }

    fn block_header_id(&self) -> Option<&str> {
        match self {
            InstructionKind::BlockHeader { block_id } => Some(block_id),
            _ => None,
        }
    }

    fn value_slot_mut(&mut self, field: &str) -> Option<&mut Value> {
        crate::fields::value_slot_mut(self, field.parse().ok()?)
    }

    fn blank_field_value(&self, field: &str, blocks: &[BlockDef]) -> Option<Value> {
        crate::fields::blank_field_value(self, field.parse().ok()?, blocks)
    }

    fn field_requires_integer(&self, field: &str) -> bool {
        field
            .parse()
            .is_ok_and(|field: crate::fields::FieldId| field.requires_integer())
    }
}

impl InstructionKind {
    /// True for a block that only makes sense in a 3D project - the editor
    /// greys these out in a 2D one rather than hiding them, so switching
    /// modes never silently changes what a script means.
    pub fn is_3d_only(&self) -> bool {
        matches!(
            self,
            InstructionKind::Turn {
                axis: Axis::X | Axis::Y,
                ..
            } | InstructionKind::SetRotation {
                axis: Axis::X | Axis::Y,
                ..
            }
        )
    }

    /// The custom block this instruction invokes in command position.
    pub fn called_block(&self) -> Option<&str> {
        match self {
            InstructionKind::CallBlock { block_id, .. } => Some(block_id),
            _ => None,
        }
    }
}

/// A slot older documents didn't have, read as 0.
fn zero() -> Value {
    Value::number(0.0)
}
