//! Blockloom's block vocabulary - the instructions its canvas speaks - and
//! its `BlockKind` impl, the hinge onto blockstitch's document model.
//!
//! Every numeric, text or boolean slot is a [`Value`], so any of them can
//! hold an expression or a reporter block. Everything else on an instruction
//! (an axis, a key name, a body kind) is a fixed in-place dropdown the
//! frontend rewrites with `edit_instruction`.

use crate::components::CameraView;
use crate::scene::{Axis, BodyKind};
use crate::value::Value;
use serde::{Deserialize, Serialize};

pub use blockstitch_core::graph::{
    BlockDef, BlockGraph, BlockKind, BlockPiece, BlockShape, Comment, FloatingValue,
    InputValueType, VariableDef, default_block_color, normalize_block_color,
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
    /// Runs every time `key` goes down.
    WhenKeyPressed {
        key: String,
    },
    /// Runs when this actor is clicked.
    WhenClicked,
    /// Runs when this actor starts touching `with` (an actor name, or an
    /// empty string for "anything").
    WhenCollision {
        with: String,
    },
    /// Runs when any actor broadcasts `name`.
    WhenMessage {
        name: String,
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
    ChangePosition {
        axis: Axis,
        by: Value,
    },
    /// Slides to a position over `seconds`, one step per rendered frame.
    Glide {
        seconds: Value,
        x: Value,
        y: Value,
        z: Value,
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
    /// Gives this actor a component mid-run. One the editor authored comes
    /// back as it was left; anything else arrives with its defaults.
    AttachComponent {
        component: String,
    },
    /// Takes a component away mid-run. `Place` can't go.
    DetachComponent {
        component: String,
    },

    // ─── Control ────────────────────────────────────────────────────────────
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
            | K::SetDensity { density: v }
            | K::SetMass { mass: v }
            | K::Say { text: v }
            | K::SetColor { color: v }
            | K::Wait { duration: v }
            | K::SetVariable { value: v, .. }
            | K::ChangeVariable { value: v, .. }
            | K::Return { value: v }
            | K::SetComponentField { value: v, .. }
            | K::Repeat { count: v, .. } => f(v, InputValueType::Any),
            K::GoTo { x, y, z }
            | K::ApplyImpulse { x, y, z }
            | K::SetVelocity { x, y, z }
            | K::SetGravity { x, y, z } => {
                f(x, InputValueType::Any);
                f(y, InputValueType::Any);
                f(z, InputValueType::Any);
            }
            K::Glide { seconds, x, y, z } => {
                f(seconds, InputValueType::Any);
                f(x, InputValueType::Any);
                f(y, InputValueType::Any);
                f(z, InputValueType::Any);
            }
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
            K::WhenStarted
            | K::WhenKeyPressed { .. }
            | K::WhenClicked
            | K::WhenCollision { .. }
            | K::WhenMessage { .. }
            | K::BlockHeader { .. }
            | K::PointTowards { .. }
            | K::SetBody { .. }
            | K::SetCameraView { .. }
            | K::AttachComponent { .. }
            | K::DetachComponent { .. }
            | K::SetVisible { .. }
            | K::Forever { .. }
            | K::EscapeLoop
            | K::ContinueLoop
            | K::Broadcast { .. }
            | K::StopAll => {}
        }
    }

    fn is_header(&self) -> bool {
        matches!(
            self,
            InstructionKind::WhenStarted
                | InstructionKind::WhenKeyPressed { .. }
                | InstructionKind::WhenClicked
                | InstructionKind::WhenCollision { .. }
                | InstructionKind::WhenMessage { .. }
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
