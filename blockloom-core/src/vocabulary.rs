//! The block vocabulary, machine-readable. One entry per `InstructionKind`
//! naming its value slots, dropdowns, string fields, bodies and category, so
//! an AI agent authoring a block object spells it exactly as the wire format
//! and the editor's commands expect.
//!
//! The table below is exhaustive: adding a variant to
//! [`crate::blocks::InstructionKind`] fails to compile until it is described
//! here, so the vocabulary can't silently fall behind the engine.

use serde::Serialize;

/// A value slot (a `Value` block or expression) on an instruction.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Slot {
    /// The JSON key on the instruction (`steps`, `x`, ...).
    pub field: &'static str,
    /// The [`crate::fields::FieldId`] string the commands address it by.
    pub id: &'static str,
    /// What the slot holds: `Any` (number/text/expression), `Bool`, `Integer`.
    pub value: &'static str,
}

/// An in-place dropdown on an instruction, with its choices.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Dropdown {
    /// The JSON key (`axis`, `body`, `view`, ...).
    pub field: &'static str,
    pub options: &'static [&'static str],
}

/// A nested stack of blocks an instruction wraps (`repeat`'s body, an `if`).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Body {
    /// The JSON key (`body`, `then_body`, `else_body`).
    pub field: &'static str,
}

/// One block an agent can put on a canvas, as a wire `instruction` starts it.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct BlockSpec {
    /// The wire `type` - `InstructionKind`'s serde tag, e.g. `"Move"`.
    pub r#type: &'static str,
    pub category: &'static str,
    pub purpose: &'static str,
    #[serde(skip_serializing_if = "is_false")]
    pub header: bool,
    #[serde(skip_serializing_if = "is_false", rename = "3dOnly")]
    pub three_d: bool,
    #[serde(skip_serializing_if = "slots_empty", default)]
    pub slots: &'static [Slot],
    #[serde(skip_serializing_if = "dropdowns_empty", default)]
    pub dropdowns: &'static [Dropdown],
    #[serde(skip_serializing_if = "strings_empty", default)]
    pub strings: &'static [&'static str],
    #[serde(skip_serializing_if = "bools_empty", default)]
    pub bools: &'static [&'static str],
    #[serde(skip_serializing_if = "bodies_empty", default)]
    pub bodies: &'static [Body],
}

fn is_false(b: &bool) -> bool {
    !b
}
fn slots_empty(s: &[Slot]) -> bool {
    s.is_empty()
}
fn dropdowns_empty(s: &[Dropdown]) -> bool {
    s.is_empty()
}
fn strings_empty(s: &[&'static str]) -> bool {
    s.is_empty()
}
fn bools_empty(s: &[&'static str]) -> bool {
    s.is_empty()
}
fn bodies_empty(s: &[Body]) -> bool {
    s.is_empty()
}

const AXES: &[&str] = &["X", "Y", "Z"];
const BODY_KINDS: &[&str] = &["None", "Static", "Dynamic", "Kinematic"];
const CAMERA_VIEWS: &[&str] = &["Follow", "FirstPerson", "ThirdPerson"];
const UI_ANCHORS: &[&str] = &[
    "TopLeft",
    "Top",
    "TopRight",
    "Left",
    "Center",
    "Right",
    "BottomLeft",
    "Bottom",
    "BottomRight",
];
const UI_PROPS: &[&str] = &[
    "Text",
    "TextColor",
    "TextSize",
    "Background",
    "Width",
    "Height",
    "Visible",
    "CornerRadius",
    "Padding",
    "Modal",
    "Min",
    "Max",
    "Value",
];

/// The dropdown every `show` block has: which corner of the window its
/// offset is measured from.
const UI_ANCHOR: &[Dropdown] = &[Dropdown {
    field: "anchor",
    options: UI_ANCHORS,
}];

/// The slot each `show` block names its element by.
const UI_ID: Slot = Slot {
    field: "element",
    id: "UiId",
    value: "Any",
};

/// Where it sits and how big it is - shared by all seven kinds, and ignored
/// for one with a parent, which flows after its siblings instead.
const UI_PLACE: &[Slot] = &[
    Slot {
        field: "x",
        id: "UiX",
        value: "Any",
    },
    Slot {
        field: "y",
        id: "UiY",
        value: "Any",
    },
    Slot {
        field: "width",
        id: "UiWidth",
        value: "Any",
    },
    Slot {
        field: "height",
        id: "UiHeight",
        value: "Any",
    },
    Slot {
        field: "parent",
        id: "UiParent",
        value: "Any",
    },
];

const NO_SLOTS: &[Slot] = &[];
const NO_DROPDOWNS: &[Dropdown] = &[];
const NO_STRINGS: &[&str] = &[];
const NO_BOOLS: &[&str] = &[];
const NO_BODIES: &[Body] = &[];

/// Every block, grouped and ordered like the editor's palette.
pub const BLOCKS: &[BlockSpec] = &[
    // ── Events ─────────────────────────────────────────────────────────────
    BlockSpec {
        r#type: "WhenStarted",
        category: "Events",
        purpose: "Runs when the project starts (the green flag).",
        header: true,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "WhenKeyPressed",
        category: "Events",
        purpose: "Runs every time key goes down.",
        header: true,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: &["key"],
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "WhenClicked",
        category: "Events",
        purpose: "Runs when this actor is clicked.",
        header: true,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "WhenCollision",
        category: "Events",
        purpose: "Runs when this actor starts touching with (an actor name; blank means anything).",
        header: true,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: &["with"],
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "WhenMessage",
        category: "Events",
        purpose: "Runs when any actor broadcasts name.",
        header: true,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: &["name"],
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "BlockHeader",
        category: "Events",
        purpose: "Marks a strand as a custom block's body; block_id names it. Never runs on its own.",
        header: true,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: &["block_id"],
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "WhenCloned",
        category: "Events",
        purpose: "Runs inside a fresh clone the moment it is made.",
        header: true,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    // ── Motion ─────────────────────────────────────────────────────────────
    BlockSpec {
        r#type: "Move",
        category: "Motion",
        purpose: "Moves forward along the actor's own facing by steps units.",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "steps",
            id: "MoveSteps",
            value: "Any",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "GoTo",
        category: "Motion",
        purpose: "Jumps straight to a position; z is ignored in a 2D project.",
        header: false,
        three_d: false,
        slots: &[
            Slot {
                field: "x",
                id: "GoToX",
                value: "Any",
            },
            Slot {
                field: "y",
                id: "GoToY",
                value: "Any",
            },
            Slot {
                field: "z",
                id: "GoToZ",
                value: "Any",
            },
        ],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "ChangePosition",
        category: "Motion",
        purpose: "Changes the position along one axis by by.",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "by",
            id: "ChangeByAmount",
            value: "Any",
        }],
        dropdowns: &[Dropdown {
            field: "axis",
            options: AXES,
        }],
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "Glide",
        category: "Motion",
        purpose: "Slides to a position over seconds, one step per rendered frame.",
        header: false,
        three_d: false,
        slots: &[
            Slot {
                field: "seconds",
                id: "GlideSeconds",
                value: "Any",
            },
            Slot {
                field: "x",
                id: "GlideX",
                value: "Any",
            },
            Slot {
                field: "y",
                id: "GlideY",
                value: "Any",
            },
            Slot {
                field: "z",
                id: "GlideZ",
                value: "Any",
            },
        ],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "Turn",
        category: "Motion",
        purpose: "Rotates by degrees around the chosen axis.",
        header: false,
        three_d: true,
        slots: &[Slot {
            field: "degrees",
            id: "TurnDegrees",
            value: "Any",
        }],
        dropdowns: &[Dropdown {
            field: "axis",
            options: AXES,
        }],
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "SetRotation",
        category: "Motion",
        purpose: "Sets the rotation to degrees around the chosen axis.",
        header: false,
        three_d: true,
        slots: &[Slot {
            field: "degrees",
            id: "RotationDegrees",
            value: "Any",
        }],
        dropdowns: &[Dropdown {
            field: "axis",
            options: AXES,
        }],
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "PointTowards",
        category: "Motion",
        purpose: "Faces another actor by name, or mouse.",
        header: false,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: &["target"],
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "SetScale",
        category: "Motion",
        purpose: "Sets the actor's scale to factor (1 is its authored size).",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "factor",
            id: "ScaleFactor",
            value: "Any",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    // ── Physics ────────────────────────────────────────────────────────────
    BlockSpec {
        r#type: "SetBody",
        category: "Physics",
        purpose: "Sets the actor's body kind: None, Static, Dynamic or Kinematic.",
        header: false,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: &[Dropdown {
            field: "body",
            options: BODY_KINDS,
        }],
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "ApplyImpulse",
        category: "Physics",
        purpose: "One-shot push in units per second; only a dynamic body responds.",
        header: false,
        three_d: false,
        slots: &[
            Slot {
                field: "x",
                id: "ImpulseX",
                value: "Any",
            },
            Slot {
                field: "y",
                id: "ImpulseY",
                value: "Any",
            },
            Slot {
                field: "z",
                id: "ImpulseZ",
                value: "Any",
            },
        ],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "SetVelocity",
        category: "Physics",
        purpose: "Sets the actor's velocity in units per second.",
        header: false,
        three_d: false,
        slots: &[
            Slot {
                field: "x",
                id: "VelocityX",
                value: "Any",
            },
            Slot {
                field: "y",
                id: "VelocityY",
                value: "Any",
            },
            Slot {
                field: "z",
                id: "VelocityZ",
                value: "Any",
            },
        ],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "SetGravity",
        category: "Physics",
        purpose: "Sets world gravity, not this actor's.",
        header: false,
        three_d: false,
        slots: &[
            Slot {
                field: "x",
                id: "GravityX",
                value: "Any",
            },
            Slot {
                field: "y",
                id: "GravityY",
                value: "Any",
            },
            Slot {
                field: "z",
                id: "GravityZ",
                value: "Any",
            },
        ],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "SetDensity",
        category: "Physics",
        purpose: "Sets how heavy the actor is for its size.",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "density",
            id: "Density",
            value: "Any",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "SetMass",
        category: "Physics",
        purpose: "Sets an explicit body mass, winning over density and the shape.",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "mass",
            id: "Mass",
            value: "Any",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    // ── Looks ──────────────────────────────────────────────────────────────
    BlockSpec {
        r#type: "Say",
        category: "Looks",
        purpose: "Shows a speech bubble over the actor; empty text clears it.",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "text",
            id: "SayText",
            value: "Any",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "SetVisible",
        category: "Looks",
        purpose: "Shows or hides the actor.",
        header: false,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: &["visible"],
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "SetColor",
        category: "Looks",
        purpose: "Tints the actor a #RRGGBB color; no-op on an image actor.",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "color",
            id: "ColorText",
            value: "Any",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    // ── Components ─────────────────────────────────────────────────────────
    BlockSpec {
        r#type: "SetComponentField",
        category: "Components",
        purpose: "Writes one field of one of this actor's custom components.",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "value",
            id: "ComponentFieldValue",
            value: "Any",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: &["component", "field"],
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "SetCameraView",
        category: "Components",
        purpose: "Switches the camera component's view mid-run.",
        header: false,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: &[Dropdown {
            field: "view",
            options: CAMERA_VIEWS,
        }],
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "SetCameraPitch",
        category: "Components",
        purpose: "Tilts a first-person camera up or down in degrees; positive looks up.",
        header: false,
        three_d: true,
        slots: &[Slot {
            field: "degrees",
            id: "CameraPitchDegrees",
            value: "Any",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "AttachComponent",
        category: "Components",
        purpose: "Gives this actor a component mid-run.",
        header: false,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: &["component"],
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "DetachComponent",
        category: "Components",
        purpose: "Takes a component away mid-run; Place can't go.",
        header: false,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: &["component"],
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "SetParent",
        category: "Components",
        purpose: "Hangs this actor off another one, so the two move together.                   Names an actor or an id; an empty slot takes it off.",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "parent",
            id: "ParentTarget",
            value: "Any",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    // ── Actors ─────────────────────────────────────────────────────────────
    BlockSpec {
        r#type: "CreateClone",
        category: "Actors",
        purpose: "Makes a running copy of an actor and starts its                   \"when I start as a clone\" strands. An empty `of` clones                   whoever ran the block.",
        header: false,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: &["of"],
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "CreateActor",
        category: "Actors",
        purpose: "Makes a brand-new actor with no blocks, for this run only.",
        header: false,
        three_d: false,
        slots: &[
            Slot {
                field: "name",
                id: "NewActorName",
                value: "Any",
            },
            Slot {
                field: "x",
                id: "NewActorX",
                value: "Any",
            },
            Slot {
                field: "y",
                id: "NewActorY",
                value: "Any",
            },
            Slot {
                field: "z",
                id: "NewActorZ",
                value: "Any",
            },
        ],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "DeleteActor",
        category: "Actors",
        purpose: "Takes an actor out of the running world and stops its                   scripts. An empty slot deletes whoever ran the block.",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "target",
            id: "DeleteTarget",
            value: "Any",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    // ── Control ────────────────────────────────────────────────────────────
    BlockSpec {
        r#type: "Wait",
        category: "Control",
        purpose: "Suspends this script for duration seconds.",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "duration",
            id: "WaitDuration",
            value: "Any",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "WaitUntil",
        category: "Control",
        purpose: "Suspends this script until condition is true.",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "condition",
            id: "WaitUntilCondition",
            value: "Bool",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "If",
        category: "Control",
        purpose: "Runs body once if condition is true.",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "condition",
            id: "Condition",
            value: "Bool",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: &[Body { field: "body" }],
    },
    BlockSpec {
        r#type: "IfElse",
        category: "Control",
        purpose: "Runs then_body if condition, else else_body.",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "condition",
            id: "Condition",
            value: "Bool",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: &[Body { field: "then_body" }, Body { field: "else_body" }],
    },
    BlockSpec {
        r#type: "Repeat",
        category: "Control",
        purpose: "Runs body count times.",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "count",
            id: "RepeatCount",
            value: "Integer",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: &[Body { field: "body" }],
    },
    BlockSpec {
        r#type: "Forever",
        category: "Control",
        purpose: "Runs body forever, yielding once per tick.",
        header: false,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: &[Body { field: "body" }],
    },
    BlockSpec {
        r#type: "While",
        category: "Control",
        purpose: "Runs body while condition is true.",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "condition",
            id: "Condition",
            value: "Bool",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: &[Body { field: "body" }],
    },
    BlockSpec {
        r#type: "EscapeLoop",
        category: "Control",
        purpose: "Stops the nearest enclosing loop; a no-op outside one.",
        header: false,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "ContinueLoop",
        category: "Control",
        purpose: "Skips to the next iteration of the nearest enclosing loop.",
        header: false,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "Broadcast",
        category: "Control",
        purpose: "Fires every WhenMessage strand listening for name, in every actor.",
        header: false,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: &["name"],
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "StopAll",
        category: "Control",
        purpose: "Stops every running script, this one included.",
        header: false,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "WhenUiClicked",
        category: "Events",
        purpose: "Runs when the interface element named element is clicked.",
        header: true,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: &["element"],
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "WhenUiChanged",
        category: "Events",
        purpose: "Runs when the input element named element is changed.",
        header: true,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: &["element"],
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "SetMouseLocked",
        category: "Control",
        purpose: "Grabs the pointer for first-person play, or shows it again.",
        header: false,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: &["locked"],
        bodies: NO_BODIES,
    },
    // ── Interface ─────────────────────────────────────────────────
    BlockSpec {
        r#type: "ShowPanel",
        category: "Interface",
        purpose: "Makes or updates a panel: a vertical stack its children flow down. An empty title means no header row; modal swallows world clicks behind it.",
        header: false,
        three_d: false,
        slots: &[
            UI_ID,
            Slot {
                field: "title",
                id: "UiContent",
                value: "Any",
            },
            UI_PLACE[0],
            UI_PLACE[1],
            UI_PLACE[2],
            UI_PLACE[3],
            UI_PLACE[4],
        ],
        dropdowns: UI_ANCHOR,
        strings: NO_STRINGS,
        bools: &["modal"],
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "ShowLabel",
        category: "Interface",
        purpose: "Makes or updates a text label - the HUD workhorse.",
        header: false,
        three_d: false,
        slots: &[
            UI_ID,
            Slot {
                field: "text",
                id: "UiContent",
                value: "Any",
            },
            UI_PLACE[0],
            UI_PLACE[1],
            UI_PLACE[2],
            UI_PLACE[3],
            UI_PLACE[4],
        ],
        dropdowns: UI_ANCHOR,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "ShowButton",
        category: "Interface",
        purpose: "Makes or updates a clickable button.",
        header: false,
        three_d: false,
        slots: &[
            UI_ID,
            Slot {
                field: "label",
                id: "UiContent",
                value: "Any",
            },
            UI_PLACE[0],
            UI_PLACE[1],
            UI_PLACE[2],
            UI_PLACE[3],
            UI_PLACE[4],
        ],
        dropdowns: UI_ANCHOR,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "ShowImage",
        category: "Interface",
        purpose: "Makes or updates an image, by project-relative asset path.",
        header: false,
        three_d: false,
        slots: &[
            UI_ID,
            Slot {
                field: "asset",
                id: "UiContent",
                value: "Any",
            },
            UI_PLACE[0],
            UI_PLACE[1],
            UI_PLACE[2],
            UI_PLACE[3],
            UI_PLACE[4],
        ],
        dropdowns: UI_ANCHOR,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "ShowInput",
        category: "Interface",
        purpose: "Makes or updates a single-line text input. The text is its placeholder.",
        header: false,
        three_d: false,
        slots: &[
            UI_ID,
            Slot {
                field: "placeholder",
                id: "UiContent",
                value: "Any",
            },
            UI_PLACE[0],
            UI_PLACE[1],
            UI_PLACE[2],
            UI_PLACE[3],
            UI_PLACE[4],
        ],
        dropdowns: UI_ANCHOR,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "ShowSlider",
        category: "Interface",
        purpose: "Makes or updates a slider between min and max.",
        header: false,
        three_d: false,
        slots: &[
            UI_ID,
            Slot {
                field: "min",
                id: "UiMin",
                value: "Any",
            },
            Slot {
                field: "max",
                id: "UiMax",
                value: "Any",
            },
            Slot {
                field: "value",
                id: "UiValue",
                value: "Any",
            },
            UI_PLACE[0],
            UI_PLACE[1],
            UI_PLACE[2],
            UI_PLACE[3],
            UI_PLACE[4],
        ],
        dropdowns: UI_ANCHOR,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "ShowToggle",
        category: "Interface",
        purpose: "Makes or updates an on/off toggle.",
        header: false,
        three_d: false,
        slots: &[
            UI_ID,
            Slot {
                field: "label",
                id: "UiContent",
                value: "Any",
            },
            UI_PLACE[0],
            UI_PLACE[1],
            UI_PLACE[2],
            UI_PLACE[3],
            UI_PLACE[4],
        ],
        dropdowns: UI_ANCHOR,
        strings: NO_STRINGS,
        bools: &["on"],
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "SetUiProp",
        category: "Interface",
        purpose: "Writes one property of an existing interface element.",
        header: false,
        three_d: false,
        slots: &[
            Slot {
                field: "element",
                id: "UiTarget",
                value: "Any",
            },
            Slot {
                field: "value",
                id: "UiPropValue",
                value: "Any",
            },
        ],
        dropdowns: &[Dropdown {
            field: "prop",
            options: UI_PROPS,
        }],
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "HideElement",
        category: "Interface",
        purpose: "Takes an element off the screen, children and all, without forgetting it.",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "element",
            id: "UiTarget",
            value: "Any",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "HideAllUi",
        category: "Interface",
        purpose: "Hides every interface element, and drops keyboard focus with them.",
        header: false,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "DeleteElement",
        category: "Interface",
        purpose: "Forgets an interface element entirely, children and all.",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "element",
            id: "UiTarget",
            value: "Any",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "PauseGame",
        category: "Interface",
        purpose: "Freezes the world. Strands a UI click started keep running.",
        header: false,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "ResumeGame",
        category: "Interface",
        purpose: "Thaws a world `pause game` froze.",
        header: false,
        three_d: false,
        slots: NO_SLOTS,
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    // ── Variables ──────────────────────────────────────────────────────────
    BlockSpec {
        r#type: "SetVariable",
        category: "Variables",
        purpose: "Sets a variable to value.",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "value",
            id: "SetVariableValue",
            value: "Any",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: &["name"],
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "ChangeVariable",
        category: "Variables",
        purpose: "Adds value to a variable, coercing a non-numeric one to 0 first.",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "value",
            id: "ChangeVariableValue",
            value: "Any",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: &["name"],
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    // ── Custom blocks ──────────────────────────────────────────────────────
    BlockSpec {
        r#type: "CallBlock",
        category: "Custom Blocks",
        purpose: "Runs a custom block's body inline, binding args to its inputs.",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "args",
            id: "CallArg:n",
            value: "Any",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: &["block_id"],
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
    BlockSpec {
        r#type: "Return",
        category: "Custom Blocks",
        purpose: "Evaluates and returns value; only meaningful in a reporter body.",
        header: false,
        three_d: false,
        slots: &[Slot {
            field: "value",
            id: "ReturnValue",
            value: "Any",
        }],
        dropdowns: NO_DROPDOWNS,
        strings: NO_STRINGS,
        bools: NO_BOOLS,
        bodies: NO_BODIES,
    },
];

/// The whole vocabulary as one JSON document: how a `Value` leaf is spelled,
/// the sensing reporters a slot can read, and every block. This is what an
/// agent paces its authoring against.
pub fn block_vocabulary() -> serde_json::Value {
    serde_json::json!({
        "value": {
            "Number": { "kind": "Number", "value": 5.0 },
            "Text": { "kind": "Text", "value": "hello" },
            "Bool": { "kind": "Bool" },
            "Var": { "kind": "Var", "name": "a variable's name" },
            "expression or reporter": {
                "kind": "Op",
                "op": "a name from reporters, or an operator like Add",
                "args": [{ "kind": "Number", "value": 1.0 }]
            }
        },
        "reporters": crate::value::reporter_specs(),
        "blocks": BLOCKS,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::FieldId;
    use crate::{blocks::InstructionKind, value::Value};
    use serde_json::json;

    #[test]
    fn every_block_describes_its_variant_truthfully() {
        for spec in BLOCKS {
            assert!(!spec.r#type.is_empty(), "a block lacks a type");
            assert!(!spec.category.is_empty());
            assert!(!spec.purpose.is_empty());
            // A wrong tag name is "unknown variant"; a real one with its
            // fields missing is "missing field" - so the error kind is the
            // drift check against the enum's own serialization.
            let tagged = json!({ "type": spec.r#type });
            match serde_json::from_value::<InstructionKind>(tagged) {
                Ok(_) => {}
                Err(e) => assert!(
                    e.to_string().contains("missing field"),
                    "{}: {e}",
                    spec.r#type
                ),
            }
        }
    }

    #[test]
    fn slot_ids_are_real_field_ids() {
        for spec in BLOCKS {
            for slot in spec.slots {
                let id = slot.id;
                if id == "CallArg:n" {
                    continue; // call sites carry one arg per declared input
                }
                assert!(
                    id.parse::<FieldId>().is_ok(),
                    "{} has a bogus slot id {id}",
                    spec.r#type
                );
            }
        }
    }

    #[test]
    fn a_move_wire_object_matches_its_spec() {
        let spec = BLOCKS.iter().find(|b| b.r#type == "Move").unwrap();
        assert_eq!(spec.slots[0].field, "steps");
        let instruction = InstructionKind::Move {
            steps: Value::number(10.0),
        };
        assert_eq!(serde_json::to_value(&instruction).unwrap()["type"], "Move");
        assert!(serde_json::to_value(spec).unwrap().get("id").is_none());
    }

    #[test]
    fn the_vocabulary_serializes_as_one_json_document() {
        let json = block_vocabulary();
        assert!(json["blocks"].is_array());
        assert!(json["blocks"].as_array().unwrap().len() >= 40);
        assert!(json["reporters"].is_array());
    }
}
