//! What running a script asks the host to do. Every effect names the actor it
//! applies to by id, and carries already-evaluated numbers - the host never
//! evaluates a [`crate::value::Value`] itself.

use crate::components::CameraView;
use crate::scene::{Axis, BodyKind};
use crate::sound::SoundBus;
use crate::ui::{UiElement, UiProp, UiTheme};
use crate::value::Evaluated;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "effect")]
pub enum Effect {
    /// Forward along the actor's own facing.
    Move {
        actor: String,
        steps: f32,
    },
    GoTo {
        actor: String,
        position: [f32; 3],
    },
    /// One step toward `target` along the navmesh, at `speed` units per
    /// second. The host resolves the path and moves at most one step.
    NavigateTo {
        actor: String,
        target: [f32; 3],
        speed: f32,
    },
    ChangePosition {
        actor: String,
        axis: Axis,
        by: f32,
    },
    /// Starts a slide to `target`; the host interpolates over `seconds` while
    /// the script sleeps for exactly as long.
    Glide {
        actor: String,
        seconds: f32,
        target: [f32; 3],
    },
    Turn {
        actor: String,
        axis: Axis,
        degrees: f32,
    },
    SetRotation {
        actor: String,
        axis: Axis,
        degrees: f32,
    },
    PointTowards {
        actor: String,
        target: String,
    },
    SetScale {
        actor: String,
        factor: f32,
    },
    /// The camera's exposure in EV100 for the rest of the run. World-global,
    /// like gravity: no actor.
    SetExposure {
        ev: f32,
    },
    /// The actor's light, in lumens.
    SetLightIntensity {
        actor: String,
        intensity: f32,
    },
    SetBody {
        actor: String,
        body: BodyKind,
    },
    ApplyImpulse {
        actor: String,
        impulse: [f32; 3],
    },
    SetVelocity {
        actor: String,
        velocity: [f32; 3],
    },
    SetGravity {
        gravity: [f32; 3],
    },
    /// How heavy a unit of this actor's collider area or volume is - the dial
    /// between a balloon and a lead ball of the same size.
    SetDensity {
        actor: String,
        density: f32,
    },
    /// An explicit body mass, overriding whatever `density` would derive.
    SetMass {
        actor: String,
        mass: f32,
    },
    /// Whether the actor's collider pushes back or only senses overlap.
    SetTrigger {
        actor: String,
        trigger: bool,
    },
    /// Which layer the actor lives on, 1-8.
    SetCollisionLayer {
        actor: String,
        layer: u8,
    },
    /// Bitmask of the layers the actor pairs with.
    SetCollisionMask {
        actor: String,
        mask: u8,
    },
    BurstParticles {
        actor: String,
        count: u32,
    },
    SetEmitterDial {
        actor: String,
        dial: crate::blocks::EmitterDial,
        value: f32,
    },
    SetTrailEnabled {
        actor: String,
        enabled: bool,
    },
    /// A speech bubble over the actor; an empty text clears it.
    Say {
        actor: String,
        text: String,
    },
    SetVisible {
        actor: String,
        visible: bool,
    },
    SetColor {
        actor: String,
        color: String,
    },
    /// Starts a voice for `sound` (a project-relative asset path, already
    /// evaluated). `volume` is a linear gain, `pitch` a speed factor, `loop_`
    /// whether it repeats. `at` is `None` for a global voice or the actor id
    /// a positional one follows - resolved by the VM, so the host never
    /// evaluates a `Value` itself.
    PlaySound {
        actor: String,
        sound: String,
        volume: f32,
        pitch: f32,
        loop_: bool,
        bus: SoundBus,
        at: Option<String>,
    },
    /// Stops the voices playing `sound`. Empty stops every voice at once.
    StopSound {
        actor: String,
        sound: String,
    },
    /// Retunes the live voices playing `sound`.
    SetSoundVolume {
        actor: String,
        sound: String,
        volume: f32,
    },
    /// Rebends the live voices playing `sound`.
    SetSoundPitch {
        actor: String,
        sound: String,
        pitch: f32,
    },
    /// Moves a whole mixing bus. Window-global, like gravity: no actor.
    SetBusVolume {
        bus: SoundBus,
        volume: f32,
    },
    /// Writes one field of one of the actor's custom components.
    SetComponentField {
        actor: String,
        component: String,
        field: String,
        value: Evaluated,
    },
    /// Switches the actor's camera component between first person, third
    /// person and plain follow.
    SetCameraView {
        actor: String,
        view: CameraView,
    },
    /// Tilts a first-person camera up or down, in degrees. Positive looks up.
    SetCameraPitch {
        actor: String,
        degrees: f32,
    },
    /// Sets the camera's vertical field of view, in degrees.
    SetCameraFov {
        actor: String,
        fov: f32,
    },
    /// Gives the actor a component mid-run, with whatever the project
    /// authored for it or that component's defaults.
    AttachComponent {
        actor: String,
        component: String,
    },
    /// Takes a component off the actor mid-run.
    DetachComponent {
        actor: String,
        component: String,
    },
    /// Hangs `actor` off `parent`, so the two move together. An empty
    /// `parent` takes it off whatever it was on. `parent` is whatever the
    /// block said - an id or a name - which the host resolves.
    SetParent {
        actor: String,
        parent: String,
    },
    /// A running copy of `of`, already registered with the scheduler under
    /// `clone`: the host's job is the entity. `actor` is whoever asked, so
    /// "the actor I just made" can answer.
    CreateClone {
        actor: String,
        clone: String,
        of: String,
    },
    /// A brand-new actor the document never had, at `position`.
    CreateActor {
        actor: String,
        id: String,
        name: String,
        position: [f32; 3],
    },
    /// Takes an actor out of the world for the rest of the run.
    DeleteActor {
        actor: String,
    },
    /// Every script stopped, by a `stop all` block.
    Stopped,
    /// Grabs the pointer for first-person play, or frees it. Window-global,
    /// like gravity: no actor.
    SetMouseLocked {
        locked: bool,
    },
    /// Rumbles connected gamepads. Window-global: no actor.
    RumbleGamepad {
        strength: f32,
        duration: f32,
    },
    /// Adds one binding to an action for the rest of the run.
    BindAction {
        actor: String,
        action: String,
        binding: String,
    },
    /// Forgets every binding an action has for the rest of the run.
    ClearActionBindings {
        actor: String,
        action: String,
    },
    /// Makes an interface element, or updates the one that id already names.
    /// Screen-space, so no actor - the block's owner is only ever who to
    /// blame in the log.
    ShowElement {
        element: UiElement,
    },
    /// Takes an element off the screen, children and all, without forgetting
    /// it. `all` hides everything and drops keyboard focus with it.
    HideElement {
        id: String,
        all: bool,
    },
    /// Forgets an element entirely, children and all.
    DeleteElement {
        id: String,
    },
    SetUiProp {
        id: String,
        prop: UiProp,
        value: Evaluated,
    },
    /// Gives the keyboard to a text input, or takes it back when the id is
    /// empty or names something that can't hold it.
    SetFocus {
        id: String,
    },
    SetUiTheme {
        theme: UiTheme,
    },
    /// Freezes or thaws the world. Strands a UI event started carry on
    /// either way, which is what keeps a pause menu alive.
    SetPaused {
        paused: bool,
    },
    SaveVariable {
        actor: String,
        name: String,
        clear: bool,
    },
    /// A block couldn't be evaluated. The script carries on with a zero, and
    /// the editor shows this in its log.
    Error {
        actor: String,
        message: String,
    },
}
