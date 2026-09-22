//! What running a script asks the host to do. Every effect names the actor it
//! applies to by id, and carries already-evaluated numbers - the host never
//! evaluates a [`crate::value::Value`] itself.

use crate::components::CameraView;
use crate::scene::{Axis, BodyKind};
use crate::ui::{UiElement, UiProp};
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
    /// Freezes or thaws the world. Strands a UI event started carry on
    /// either way, which is what keeps a pause menu alive.
    SetPaused {
        paused: bool,
    },
    /// A block couldn't be evaluated. The script carries on with a zero, and
    /// the editor shows this in its log.
    Error {
        actor: String,
        message: String,
    },
}
