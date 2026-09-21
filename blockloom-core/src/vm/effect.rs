//! What running a script asks the host to do. Every effect names the actor it
//! applies to by id, and carries already-evaluated numbers - the host never
//! evaluates a [`crate::value::Value`] itself.

use crate::components::CameraView;
use crate::scene::{Axis, BodyKind};
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
    /// Every script stopped, by a `stop all` block.
    Stopped,
    /// A block couldn't be evaluated. The script carries on with a zero, and
    /// the editor shows this in its log.
    Error {
        actor: String,
        message: String,
    },
}
