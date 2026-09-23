//! A shell onto the backend: a text command line is parsed into the same
//! (name, JSON args) shape the frontend's Tauri `invoke` sends, so an AI agent
//! can create and edit projects exactly as a user can - every command the
//! editor offers is here, and `help` documents each one.
//!
//! The `blockloom-shell` binary hosts a backend and reads these lines from
//! stdin, writing one JSON response per line to stdout:
//!
//! ```text
//! create-project name=MyGame mode=TwoD
//! add-actor shape=Circle name=Ball
//! ```
//!
//! Each response carries `ok`, `result`, and `state` - the same fresh full
//! snapshot the editor window gives the frontend after every edit, so an agent
//! can read the world straight back after each step.

use crate::Backend;
use serde::Serialize;
use serde_json::{Map, Value, json};

/// One argument a command takes.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct ArgSpec {
    pub name: &'static str,
    pub ty: &'static str,
    pub required: bool,
}

/// One command an agent can issue. `name` is the shell spelling (kebab-case),
/// `cmd` is the name `Backend::dispatch` knows it by (camelCase) - so the two
/// halves can drift without either side noticing.
#[derive(Debug, Serialize)]
pub struct CommandSpec {
    pub name: &'static str,
    pub cmd: &'static str,
    pub aliases: &'static [&'static str],
    pub summary: &'static str,
    pub args: &'static [ArgSpec],
}

const A: ArgSpec = ArgSpec {
    name: "actorId",
    ty: "id",
    required: true,
};

/// Everything the shell can say, in the order `help` lists it.
pub(crate) const COMMANDS: &[CommandSpec] = &[
    // ── State ─────────────────────────────────────────────────────────────
    CommandSpec {
        name: "get-state",
        cmd: "get_state",
        aliases: &["state"],
        summary: "The whole snapshot: Dashboard list, project, actors, log.",
        args: &[],
    },
    CommandSpec {
        name: "block-vocabulary",
        cmd: "block_vocabulary",
        aliases: &["blocks", "vocabulary"],
        summary: "Every block with its slots, dropdowns and bodies, as JSON.",
        args: &[],
    },
    // ── Projects ──────────────────────────────────────────────────────────
    CommandSpec {
        name: "open-project",
        cmd: "open_project",
        aliases: &["open_project"],
        summary: "Open the project in a folder, replacing whatever was open.",
        args: &[ArgSpec {
            name: "path",
            ty: "folder path",
            required: true,
        }],
    },
    CommandSpec {
        name: "create-project",
        cmd: "create_project",
        aliases: &["create_project"],
        summary: "Make a project folder under location and open it.",
        args: &[
            ArgSpec {
                name: "name",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "location",
                ty: "parent folder",
                required: false,
            },
            ArgSpec {
                name: "mode",
                ty: "TwoD|ThreeD",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "close-project",
        cmd: "close_project",
        aliases: &["close_project"],
        summary: "Save and go back to the Dashboard.",
        args: &[],
    },
    CommandSpec {
        name: "forget-project",
        cmd: "forget_project",
        aliases: &["forget_project"],
        summary: "Drop a project from the Dashboard without touching its folder.",
        args: &[ArgSpec {
            name: "path",
            ty: "folder path",
            required: true,
        }],
    },
    CommandSpec {
        name: "delete-project",
        cmd: "delete_project",
        aliases: &["delete_project"],
        summary: "Delete a project folder outright.",
        args: &[ArgSpec {
            name: "path",
            ty: "folder path",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-project-name",
        cmd: "set_project_name",
        aliases: &["set_project_name"],
        summary: "Rename the open project, and its folder with it.",
        args: &[ArgSpec {
            name: "name",
            ty: "string",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-project-icon",
        cmd: "set_project_icon",
        aliases: &["set_project_icon"],
        summary: "Choose an image asset to brand packaged builds.",
        args: &[ArgSpec {
            name: "path",
            ty: "image asset path (blank clears)",
            required: true,
        }],
    },
    CommandSpec {
        name: "save-project",
        cmd: "save_project",
        aliases: &["save_project"],
        summary: "Write the open project to disk now.",
        args: &[],
    },
    CommandSpec {
        name: "export-project",
        cmd: "export_project",
        aliases: &["export_project"],
        summary: "Write the open project to a .blockloom file.",
        args: &[ArgSpec {
            name: "path",
            ty: "file path",
            required: true,
        }],
    },
    CommandSpec {
        name: "import-project",
        cmd: "import_project",
        aliases: &["import_project"],
        summary: "Read a .blockloom file into a folder of its own and open it.",
        args: &[ArgSpec {
            name: "path",
            ty: "file path",
            required: true,
        }],
    },
    CommandSpec {
        name: "list-build-targets",
        cmd: "list_build_targets",
        aliases: &["list_build_targets", "build-targets"],
        summary: "Every platform a build can be made for, and whether it can be right now.",
        args: &[],
    },
    CommandSpec {
        name: "build-game",
        cmd: "build_game",
        aliases: &["build_game", "build"],
        summary: "Build the open project into a folder under path that runs on its own.",
        args: &[
            ArgSpec {
                name: "path",
                ty: "folder path",
                required: true,
            },
            ArgSpec {
                name: "target",
                ty: "target triple (default: this machine)",
                required: false,
            },
            ArgSpec {
                name: "fast",
                ty: "boolean (default: on when available)",
                required: false,
            },
        ],
    },
    // ── The world ─────────────────────────────────────────────────────────
    CommandSpec {
        name: "set-mode",
        cmd: "set_mode",
        aliases: &["set_mode"],
        summary: "Switch the project between 2D and 3D.",
        args: &[ArgSpec {
            name: "mode",
            ty: "TwoD|ThreeD",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-background",
        cmd: "set_background",
        aliases: &["set_background"],
        summary: "Set the world's background color.",
        args: &[ArgSpec {
            name: "color",
            ty: "#RRGGBB",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-gravity",
        cmd: "set_gravity",
        aliases: &["set_gravity"],
        summary: "Set world gravity, in units per second squared.",
        args: &[ArgSpec {
            name: "gravity",
            ty: "[x, y, z]",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-fixed-rate",
        cmd: "set_fixed_rate",
        aliases: &["set_fixed_rate"],
        summary: "How many times a second the world's blocks and physics advance.",
        args: &[ArgSpec {
            name: "fixedRate",
            ty: "number",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-camera",
        cmd: "set_camera",
        aliases: &["set_camera"],
        summary: "Set the world camera's position, look_at and zoom.",
        args: &[ArgSpec {
            name: "camera",
            ty: "object",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-lighting",
        cmd: "set_lighting",
        aliases: &["set_lighting"],
        summary: "Set the 3D world's light direction, colors, brightness and AO.",
        args: &[ArgSpec {
            name: "lighting",
            ty: "object",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-sound-mixer",
        cmd: "set_sound_mixer",
        aliases: &["set_sound_mixer"],
        summary: "Set the saved mix: master, music and effects gains, linear 0-2.",
        args: &[ArgSpec {
            name: "mixer",
            ty: "object",
            required: true,
        }],
    },
    // ── Actors ────────────────────────────────────────────────────────────
    CommandSpec {
        name: "select-actor",
        cmd: "select_actor",
        aliases: &["select_actor"],
        summary: "Choose whose canvas the editor shows.",
        args: &[A],
    },
    CommandSpec {
        name: "add-actor",
        cmd: "add_actor",
        aliases: &["add_actor"],
        summary: "Add an actor with a default look for the shape.",
        args: &[
            ArgSpec {
                name: "shape",
                ty: "Rect|Circle|Image|Cuboid|Sphere|Capsule|Plane",
                required: true,
            },
            ArgSpec {
                name: "name",
                ty: "string",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "duplicate-actor",
        cmd: "duplicate_actor",
        aliases: &["duplicate_actor"],
        summary: "Copy an actor (blocks, components) under a fresh id.",
        args: &[A],
    },
    CommandSpec {
        name: "remove-actor",
        cmd: "remove_actor",
        aliases: &["remove_actor"],
        summary: "Delete an actor and everything on its canvas.",
        args: &[A],
    },
    CommandSpec {
        name: "move-actor",
        cmd: "move_actor",
        aliases: &["move_actor", "reparent-actor", "reorder-actor"],
        summary: "Move an actor within the list and optionally under another one: parent is the id it hangs off afterwards (blank for the top level) and before the level-mate it lands in front of (blank for the end). Resolves to whether anything changed.",
        args: &[
            A,
            ArgSpec {
                name: "parent",
                ty: "id",
                required: false,
            },
            ArgSpec {
                name: "before",
                ty: "id",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "rename-actor",
        cmd: "rename_actor",
        aliases: &["rename_actor"],
        summary: "Give an actor a new name.",
        args: &[
            A,
            ArgSpec {
                name: "name",
                ty: "string",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "set-actor-visual",
        cmd: "set_actor_visual",
        aliases: &["set_actor_visual"],
        summary: "Replace an actor's look, and with it its collider shape.",
        args: &[
            A,
            ArgSpec {
                name: "visual",
                ty: "object {\"shape\", ...}",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "set-actor-placement",
        cmd: "set_actor_placement",
        aliases: &["set_actor_placement"],
        summary: "Set an actor's position, rotation and scale.",
        args: &[
            A,
            ArgSpec {
                name: "placement",
                ty: "object {position, rotation, scale}",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "set-actor-physics",
        cmd: "set_actor_physics",
        aliases: &["set_actor_physics"],
        summary: "Set an actor's body kind and physics dials.",
        args: &[
            A,
            ArgSpec {
                name: "physics",
                ty: "object {body, gravity_scale, lock_rotation, restitution, friction}",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "set-actor-visible",
        cmd: "set_actor_visible",
        aliases: &["set_actor_visible"],
        summary: "Show or hide an actor.",
        args: &[
            A,
            ArgSpec {
                name: "visible",
                ty: "bool",
                required: true,
            },
        ],
    },
    // ── Components ────────────────────────────────────────────────────────
    CommandSpec {
        name: "add-actor-component",
        cmd: "add_actor_component",
        aliases: &["add_actor_component"],
        summary: "Add a component to an actor, or replace the one of the same name.",
        args: &[
            A,
            ArgSpec {
                name: "component",
                ty: "object {\"component\", ...}",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "set-actor-component",
        cmd: "set_actor_component",
        aliases: &["set_actor_component"],
        summary: "Replace the component called name in place.",
        args: &[
            A,
            ArgSpec {
                name: "name",
                ty: "component name",
                required: true,
            },
            ArgSpec {
                name: "component",
                ty: "object {\"component\", ...}",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "remove-actor-component",
        cmd: "remove_actor_component",
        aliases: &["remove_actor_component"],
        summary: "Take a component off an actor. Place can't go.",
        args: &[
            A,
            ArgSpec {
                name: "name",
                ty: "component name",
                required: true,
            },
        ],
    },
    // ── Assets ────────────────────────────────────────────────────────────
    CommandSpec {
        name: "list-assets",
        cmd: "list_assets",
        aliases: &["list_assets"],
        summary: "List the files in one folder of the project.",
        args: &[ArgSpec {
            name: "path",
            ty: "folder, default the root",
            required: false,
        }],
    },
    CommandSpec {
        name: "create-asset-folder",
        cmd: "create_asset_folder",
        aliases: &["create_asset_folder"],
        summary: "Make a folder for assets.",
        args: &[
            ArgSpec {
                name: "parent",
                ty: "folder path",
                required: false,
            },
            ArgSpec {
                name: "name",
                ty: "string",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "create-asset",
        cmd: "create_asset",
        aliases: &["create_asset"],
        summary: "Make an empty asset (a text file, or a starter script).",
        args: &[
            ArgSpec {
                name: "parent",
                ty: "folder path",
                required: false,
            },
            ArgSpec {
                name: "name",
                ty: "file name",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "import-assets",
        cmd: "import_assets",
        aliases: &["import_assets"],
        summary: "Copy files from anywhere on the machine into the project.",
        args: &[
            ArgSpec {
                name: "parent",
                ty: "folder path",
                required: false,
            },
            ArgSpec {
                name: "paths",
                ty: "[source paths]",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "rename-asset",
        cmd: "rename_asset",
        aliases: &["rename_asset"],
        summary: "Rename a file or folder, repointing the document at it.",
        args: &[
            ArgSpec {
                name: "path",
                ty: "asset path",
                required: true,
            },
            ArgSpec {
                name: "name",
                ty: "new name",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "move-asset",
        cmd: "move_asset",
        aliases: &["move_asset"],
        summary: "Move a file or folder under another, repointing the document.",
        args: &[
            ArgSpec {
                name: "path",
                ty: "asset path",
                required: true,
            },
            ArgSpec {
                name: "parent",
                ty: "folder path",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "delete-asset",
        cmd: "delete_asset",
        aliases: &["delete_asset"],
        summary: "Delete a file or folder from the project.",
        args: &[ArgSpec {
            name: "path",
            ty: "asset path",
            required: true,
        }],
    },
    CommandSpec {
        name: "read-asset",
        cmd: "read_asset",
        aliases: &["read_asset"],
        summary: "A file's bytes as a data: URL (for thumbnails, not editing).",
        args: &[ArgSpec {
            name: "path",
            ty: "asset path",
            required: true,
        }],
    },
    CommandSpec {
        name: "inspect-asset",
        cmd: "inspect_asset",
        aliases: &["inspect_asset"],
        summary: "What the pipeline makes of one asset (rig counts, texture/audio plan, dirt).",
        args: &[ArgSpec {
            name: "path",
            ty: "asset path",
            required: true,
        }],
    },
    CommandSpec {
        name: "pipeline-status",
        cmd: "pipeline_status",
        aliases: &["pipeline_status"],
        summary: "Every asset with its pipeline report and reimport dirt.",
        args: &[],
    },
    CommandSpec {
        name: "reimport-assets",
        cmd: "reimport_assets",
        aliases: &["reimport_assets"],
        summary: "Re-inspect assets and refresh fingerprints (empty paths means everything dirty).",
        args: &[ArgSpec {
            name: "paths",
            ty: "[source paths]",
            required: false,
        }],
    },
    CommandSpec {
        name: "pack-atlas",
        cmd: "pack_atlas",
        aliases: &["pack_atlas"],
        summary: "Lay images into one atlas sheet plan.",
        args: &[ArgSpec {
            name: "paths",
            ty: "[source paths]",
            required: true,
        }],
    },
    // ── Scripts ───────────────────────────────────────────────────────────
    CommandSpec {
        name: "create-script",
        cmd: "create_script",
        aliases: &["create_script"],
        summary: "Give an actor a Rust script, made from the starter template.",
        args: &[A],
    },
    CommandSpec {
        name: "check-script",
        cmd: "check_script",
        aliases: &["check_script"],
        summary: "Compile one actor's script and log what rustc says.",
        args: &[A],
    },
    CommandSpec {
        name: "read-script",
        cmd: "read_script",
        aliases: &["read_script"],
        summary: "An actor's script source.",
        args: &[A],
    },
    CommandSpec {
        name: "write-script",
        cmd: "write_script",
        aliases: &["write_script"],
        summary: "Replace an actor's script source.",
        args: &[
            A,
            ArgSpec {
                name: "source",
                ty: "string",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "script-toolchain",
        cmd: "script_toolchain",
        aliases: &["script_toolchain", "toolchain"],
        summary: "Whether this machine can compile scripts, and what it would use.",
        args: &[],
    },
    CommandSpec {
        name: "script-diagnostics",
        cmd: "script_diagnostics",
        aliases: &["script_diagnostics", "diagnostics"],
        summary: "One actor's script errors pinned to their lines, for inline display.",
        args: &[A],
    },
    CommandSpec {
        name: "sync-script-ide",
        cmd: "sync_script_ide",
        aliases: &["sync_script_ide", "sync-ide"],
        summary: "Regenerate the Cargo project rust-analyzer opens for this project's scripts.",
        args: &[],
    },
    CommandSpec {
        name: "open-script-ide",
        cmd: "open_script_ide",
        aliases: &["open_script_ide", "open-ide"],
        summary: "Point the user's own editor at the project folder (VS Code, Zed, or the file manager).",
        args: &[],
    },
    // ── Running ───────────────────────────────────────────────────────────
    CommandSpec {
        name: "run-project",
        cmd: "run_project",
        aliases: &["run_project"],
        summary: "Press Play: open the game window and run the project.",
        args: &[],
    },
    CommandSpec {
        name: "stop-project",
        cmd: "stop_project",
        aliases: &["stop_project"],
        summary: "Stop the run, keeping the game window.",
        args: &[],
    },
    CommandSpec {
        name: "pause-project",
        cmd: "pause_project",
        aliases: &["pause_project"],
        summary: "Pause or resume the run.",
        args: &[ArgSpec {
            name: "paused",
            ty: "bool",
            required: true,
        }],
    },
    CommandSpec {
        name: "close-runtime",
        cmd: "close_runtime",
        aliases: &["close_runtime"],
        summary: "Close the game window without touching the project.",
        args: &[],
    },
    // ── Canvas: instructions ──────────────────────────────────────────────
    CommandSpec {
        name: "add-instruction",
        cmd: "add_instruction",
        aliases: &["add_instruction"],
        summary: "Add a block to a strand at a path.",
        args: &[
            ArgSpec {
                name: "strandId",
                ty: "id",
                required: true,
            },
            ArgSpec {
                name: "path",
                ty: "[{index, slot?}] address",
                required: true,
            },
            ArgSpec {
                name: "instruction",
                ty: "block object {\"id\", \"type\", ...}",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "edit-instruction",
        cmd: "edit_instruction",
        aliases: &["edit_instruction"],
        summary: "Replace a block's whole value (its dropdown choice, say).",
        args: &[
            ArgSpec {
                name: "strandId",
                ty: "id",
                required: true,
            },
            ArgSpec {
                name: "path",
                ty: "[{index, slot?}] address",
                required: true,
            },
            ArgSpec {
                name: "instruction",
                ty: "block object {\"id\", \"type\", ...}",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "remove-instruction",
        cmd: "remove_instruction",
        aliases: &["remove_instruction"],
        summary: "Take one block out, closing the gap behind whatever was below it.",
        args: &[
            ArgSpec {
                name: "strandId",
                ty: "id",
                required: true,
            },
            ArgSpec {
                name: "path",
                ty: "[{index, slot?}] address",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "delete-instruction",
        cmd: "delete_instruction",
        aliases: &["delete_instruction"],
        summary: "Delete one block, splitting the rest off into its own strand.",
        args: &[
            ArgSpec {
                name: "strandId",
                ty: "id",
                required: true,
            },
            ArgSpec {
                name: "path",
                ty: "[{index, slot?}] address",
                required: true,
            },
            ArgSpec {
                name: "x",
                ty: "number",
                required: true,
            },
            ArgSpec {
                name: "y",
                ty: "number",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "paste-instructions",
        cmd: "paste_instructions",
        aliases: &["paste_instructions"],
        summary: "Drop a stack of blocks at (x, y) as a new strand.",
        args: &[
            ArgSpec {
                name: "x",
                ty: "number",
                required: true,
            },
            ArgSpec {
                name: "y",
                ty: "number",
                required: true,
            },
            ArgSpec {
                name: "instructions",
                ty: "[block objects]",
                required: true,
            },
        ],
    },
    // ── Canvas: strands ───────────────────────────────────────────────────
    CommandSpec {
        name: "add-strand",
        cmd: "add_strand",
        aliases: &["add_strand"],
        summary: "Start a fresh stack on the canvas, optionally with one block.",
        args: &[
            ArgSpec {
                name: "x",
                ty: "number",
                required: false,
            },
            ArgSpec {
                name: "y",
                ty: "number",
                required: false,
            },
            ArgSpec {
                name: "instruction",
                ty: "block object",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "remove-strand",
        cmd: "remove_strand",
        aliases: &["remove_strand"],
        summary: "Delete a whole stack from the canvas.",
        args: &[ArgSpec {
            name: "strandId",
            ty: "id",
            required: true,
        }],
    },
    CommandSpec {
        name: "move-strand",
        cmd: "move_strand",
        aliases: &["move_strand"],
        summary: "Move a stack to a new spot on the canvas.",
        args: &[
            ArgSpec {
                name: "strandId",
                ty: "id",
                required: true,
            },
            ArgSpec {
                name: "x",
                ty: "number",
                required: true,
            },
            ArgSpec {
                name: "y",
                ty: "number",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "split-strand",
        cmd: "split_strand",
        aliases: &["split_strand"],
        summary: "Split a stack from the block at path into its own strand.",
        args: &[
            ArgSpec {
                name: "strandId",
                ty: "id",
                required: true,
            },
            ArgSpec {
                name: "path",
                ty: "[{index, slot?}] address",
                required: true,
            },
            ArgSpec {
                name: "x",
                ty: "number",
                required: true,
            },
            ArgSpec {
                name: "y",
                ty: "number",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "merge-strand",
        cmd: "merge_strand",
        aliases: &["merge_strand"],
        summary: "Snap one stack onto another at a path.",
        args: &[
            ArgSpec {
                name: "draggedId",
                ty: "id",
                required: true,
            },
            ArgSpec {
                name: "targetId",
                ty: "id",
                required: true,
            },
            ArgSpec {
                name: "path",
                ty: "[{index, slot?}] address",
                required: true,
            },
        ],
    },
    // ── Canvas: values ────────────────────────────────────────────────────
    CommandSpec {
        name: "edit-value-field",
        cmd: "edit_value_field",
        aliases: &["edit_value_field"],
        summary: "Type into a value slot (numbers can arrive half-written).",
        args: &[
            ArgSpec {
                name: "location",
                ty: "value location object",
                required: true,
            },
            ArgSpec {
                name: "text",
                ty: "string",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "set-value-kind",
        cmd: "set_value_kind",
        aliases: &["set_value_kind"],
        summary: "What occupies a slot: a variable, reporter, or a plain value.",
        args: &[
            ArgSpec {
                name: "location",
                ty: "value location object",
                required: true,
            },
            ArgSpec {
                name: "kind",
                ty: "kind name",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "take-value",
        cmd: "take_value",
        aliases: &["take_value"],
        summary: "Unplug whatever is in a slot, returning it.",
        args: &[ArgSpec {
            name: "location",
            ty: "value location object",
            required: true,
        }],
    },
    CommandSpec {
        name: "put-value",
        cmd: "put_value",
        aliases: &["put_value"],
        summary: "Plug a value node into a slot.",
        args: &[
            ArgSpec {
                name: "location",
                ty: "value location object",
                required: true,
            },
            ArgSpec {
                name: "value",
                ty: "value object {\"kind\", ...}",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "preview-value",
        cmd: "preview_value",
        aliases: &["preview_value"],
        summary: "What a value block would report right now.",
        args: &[ArgSpec {
            name: "value",
            ty: "value object",
            required: true,
        }],
    },
    CommandSpec {
        name: "create-floating-value",
        cmd: "create_floating_value",
        aliases: &["create_floating_value"],
        summary: "Park a value block on the canvas on its own.",
        args: &[
            ArgSpec {
                name: "x",
                ty: "number",
                required: true,
            },
            ArgSpec {
                name: "y",
                ty: "number",
                required: true,
            },
            ArgSpec {
                name: "value",
                ty: "value object",
                required: true,
            },
            ArgSpec {
                name: "originBlockId",
                ty: "id",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "move-floating-value",
        cmd: "move_floating_value",
        aliases: &["move_floating_value"],
        summary: "Move a parked value block.",
        args: &[
            ArgSpec {
                name: "floatingId",
                ty: "id",
                required: true,
            },
            ArgSpec {
                name: "x",
                ty: "number",
                required: true,
            },
            ArgSpec {
                name: "y",
                ty: "number",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "remove-floating-value",
        cmd: "remove_floating_value",
        aliases: &["remove_floating_value"],
        summary: "Delete a parked value block.",
        args: &[ArgSpec {
            name: "floatingId",
            ty: "id",
            required: true,
        }],
    },
    // ── Canvas: comments ──────────────────────────────────────────────────
    CommandSpec {
        name: "create-comment",
        cmd: "create_comment",
        aliases: &["create_comment"],
        summary: "Add a sticky note to the canvas.",
        args: &[
            ArgSpec {
                name: "x",
                ty: "number",
                required: true,
            },
            ArgSpec {
                name: "y",
                ty: "number",
                required: true,
            },
            ArgSpec {
                name: "text",
                ty: "string",
                required: false,
            },
            ArgSpec {
                name: "attachedTo",
                ty: "id",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "move-comment",
        cmd: "move_comment",
        aliases: &["move_comment"],
        summary: "Move a sticky note.",
        args: &[
            ArgSpec {
                name: "commentId",
                ty: "id",
                required: true,
            },
            ArgSpec {
                name: "x",
                ty: "number",
                required: true,
            },
            ArgSpec {
                name: "y",
                ty: "number",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "edit-comment-text",
        cmd: "edit_comment_text",
        aliases: &["edit_comment_text"],
        summary: "Rewrite a sticky note's text.",
        args: &[
            ArgSpec {
                name: "commentId",
                ty: "id",
                required: true,
            },
            ArgSpec {
                name: "text",
                ty: "string",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "set-comment-collapsed",
        cmd: "set_comment_collapsed",
        aliases: &["set_comment_collapsed"],
        summary: "Collapse or expand a sticky note.",
        args: &[
            ArgSpec {
                name: "commentId",
                ty: "id",
                required: true,
            },
            ArgSpec {
                name: "collapsed",
                ty: "bool",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "remove-comment",
        cmd: "remove_comment",
        aliases: &["remove_comment"],
        summary: "Delete a sticky note.",
        args: &[ArgSpec {
            name: "commentId",
            ty: "id",
            required: true,
        }],
    },
    // ── Variables and custom blocks ───────────────────────────────────────
    CommandSpec {
        name: "create-variable",
        cmd: "create_variable",
        aliases: &["create_variable"],
        summary: "Make a global or actor-scoped variable.",
        args: &[
            ArgSpec {
                name: "name",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "scope",
                ty: "actor|global",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "rename-variable",
        cmd: "rename_variable",
        aliases: &["rename_variable"],
        summary: "Rename a variable, wherever it lives.",
        args: &[
            ArgSpec {
                name: "oldName",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "newName",
                ty: "string",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "delete-variable",
        cmd: "delete_variable",
        aliases: &["delete_variable"],
        summary: "Delete a variable, wherever it lives.",
        args: &[ArgSpec {
            name: "name",
            ty: "string",
            required: true,
        }],
    },
    CommandSpec {
        name: "create-list",
        cmd: "create_list",
        aliases: &["create_list"],
        summary: "Make a global or actor-scoped list.",
        args: &[
            ArgSpec {
                name: "name",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "scope",
                ty: "actor|global",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "rename-list",
        cmd: "rename_list",
        aliases: &["rename_list"],
        summary: "Rename a list, wherever it lives.",
        args: &[
            ArgSpec {
                name: "oldName",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "newName",
                ty: "string",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "delete-list",
        cmd: "delete_list",
        aliases: &["delete_list"],
        summary: "Delete a list, wherever it lives.",
        args: &[ArgSpec {
            name: "name",
            ty: "string",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-list-items",
        cmd: "set_list_items",
        aliases: &["set_list_items"],
        summary: "Replace a list's items with literal numbers/text.",
        args: &[
            ArgSpec {
                name: "name",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "items",
                ty: "[{kind: Number|Text, value}]",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "set-list-editor-state",
        cmd: "set_list_editor_state",
        aliases: &["set_list_editor_state"],
        summary: "Show or hide a list's canvas editor and where it sits.",
        args: &[
            ArgSpec {
                name: "name",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "visible",
                ty: "bool",
                required: true,
            },
            ArgSpec {
                name: "x",
                ty: "number",
                required: true,
            },
            ArgSpec {
                name: "y",
                ty: "number",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "create-dict",
        cmd: "create_dict",
        aliases: &["create_dict"],
        summary: "Make a global or actor-scoped dict.",
        args: &[
            ArgSpec {
                name: "name",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "scope",
                ty: "actor|global",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "rename-dict",
        cmd: "rename_dict",
        aliases: &["rename_dict"],
        summary: "Rename a dict, wherever it lives.",
        args: &[
            ArgSpec {
                name: "oldName",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "newName",
                ty: "string",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "delete-dict",
        cmd: "delete_dict",
        aliases: &["delete_dict"],
        summary: "Delete a dict, wherever it lives.",
        args: &[ArgSpec {
            name: "name",
            ty: "string",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-dict-entries",
        cmd: "set_dict_entries",
        aliases: &["set_dict_entries"],
        summary: "Replace a dict's entries with literal keys and number/text values.",
        args: &[
            ArgSpec {
                name: "name",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "entries",
                ty: "[{key, value: {kind: Number|Text, value}}]",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "set-dict-editor-state",
        cmd: "set_dict_editor_state",
        aliases: &["set_dict_editor_state"],
        summary: "Show or hide a dict's canvas editor and where it sits.",
        args: &[
            ArgSpec {
                name: "name",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "visible",
                ty: "bool",
                required: true,
            },
            ArgSpec {
                name: "x",
                ty: "number",
                required: true,
            },
            ArgSpec {
                name: "y",
                ty: "number",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "create-block",
        cmd: "create_block",
        aliases: &["create_block"],
        summary: "Make a custom block (My Blocks) from pieces.",
        args: &[
            ArgSpec {
                name: "pieces",
                ty: "[{kind: Label|Input, id, ...}]",
                required: true,
            },
            ArgSpec {
                name: "shape",
                ty: "Normal|Ending|ReturnsValue|ReturnsBool",
                required: true,
            },
            ArgSpec {
                name: "color",
                ty: "#RRGGBB",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "edit-block",
        cmd: "edit_block",
        aliases: &["edit_block"],
        summary: "Redefine a custom block's prototype.",
        args: &[
            ArgSpec {
                name: "blockId",
                ty: "id",
                required: true,
            },
            ArgSpec {
                name: "pieces",
                ty: "[{kind: Label|Input, id, ...}]",
                required: true,
            },
            ArgSpec {
                name: "shape",
                ty: "Normal|Ending|ReturnsValue|ReturnsBool",
                required: true,
            },
            ArgSpec {
                name: "color",
                ty: "#RRGGBB",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "delete-block",
        cmd: "delete_block",
        aliases: &["delete_block"],
        summary: "Delete a custom block and every call of it.",
        args: &[ArgSpec {
            name: "blockId",
            ty: "id",
            required: true,
        }],
    },
    // ── Undo, log ─────────────────────────────────────────────────────────
    CommandSpec {
        name: "undo",
        cmd: "undo",
        aliases: &[],
        summary: "Undo the last edit.",
        args: &[],
    },
    CommandSpec {
        name: "redo",
        cmd: "redo",
        aliases: &[],
        summary: "Redo the undone edit.",
        args: &[],
    },
    CommandSpec {
        name: "push-log",
        cmd: "push_log",
        aliases: &["push_log"],
        summary: "Add a line to the run log yourself.",
        args: &[
            ArgSpec {
                name: "kind",
                ty: "say|error",
                required: true,
            },
            ArgSpec {
                name: "text",
                ty: "string",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "clear-log",
        cmd: "clear_log",
        aliases: &["clear_log"],
        summary: "Empty the run log.",
        args: &[],
    },
];

fn command(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS
        .iter()
        .find(|spec| spec.name == name || spec.cmd == name || spec.aliases.contains(&name))
}

// ─── Parsing ─────────────────────────────────────────────────────────────

/// What a parsed line wants the shell to do.
#[derive(Debug)]
pub enum Action {
    Exit,
    Help(Option<String>),
    /// Run `spec`'s backend command with these arguments.
    Run {
        spec: &'static CommandSpec,
        args: Map<String, Value>,
    },
}

/// Splits a line into (name, rest) at the first run of whitespace.
fn split_command(line: &str) -> (&str, &str) {
    let line = line.trim();
    match line.find(char::is_whitespace) {
        Some(i) => (&line[..i], line[i..].trim()),
        None => (line, ""),
    }
}

/// Splits the rest into `key=value` tokens. Quotes and brackets are tracked
/// so whitespace inside them doesn't split the token, but the raw text is
/// preserved untouched - a value that is JSON stays JSON, whatever it contains.
fn tokenize(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        let start = i;
        let mut quote = None;
        let mut depth: i32 = 0;
        while i < chars.len() {
            let c = chars[i];
            match quote {
                Some(q) => {
                    if c == q {
                        quote = None;
                    }
                }
                None => match c {
                    '"' | '\'' => quote = Some(c),
                    '{' | '[' => depth += 1,
                    '}' | ']' => depth -= 1,
                    c if c.is_whitespace() && depth == 0 => break,
                    _ => {}
                },
            }
            i += 1;
        }
        if i > start {
            tokens.push(chars[start..i].iter().collect());
        }
    }
    tokens
}

/// A value that parses as JSON keeps its type; anything else is a string,
/// with a pair of matching quotes that were just grouping stripped off.
fn coerce_value(raw: &str) -> Value {
    if let Ok(value) = serde_json::from_str(raw) {
        return value;
    }
    let raw = raw.trim();
    let bare = if raw.len() >= 2
        && ((raw.starts_with('"') && raw.ends_with('"'))
            || (raw.starts_with('\'') && raw.ends_with('\'')))
    {
        &raw[1..raw.len() - 1]
    } else {
        raw
    };
    Value::String(bare.to_string())
}

/// Parses the argument half of a command line into a JSON object.
fn parse_args(rest: &str) -> Result<Map<String, Value>, String> {
    if rest.is_empty() {
        return Ok(Map::new());
    }
    if rest.starts_with('{') {
        let value: Value =
            serde_json::from_str(rest).map_err(|e| format!("Bad JSON arguments: {e}"))?;
        return match value {
            Value::Object(map) => Ok(map),
            _ => Err("The JSON arguments must be an object".to_string()),
        };
    }
    let mut args = Map::new();
    for token in tokenize(rest) {
        let Some(eq) = token.find('=') else {
            return Err(format!(
                "Expected \"key=value\", got \"{token}\". See \"help <command>\"."
            ));
        };
        let name = &token[..eq];
        let value = coerce_value(&token[eq + 1..]);
        if name.is_empty() {
            return Err("Expected \"key=value\", got an empty key".to_string());
        }
        args.insert(name.to_string(), value);
    }
    Ok(args)
}

/// Parses one shell line. `#` comments and blank lines come back as nothing
/// for the caller to skip.
pub fn parse(line: &str) -> Result<Option<Action>, String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }
    let (name, rest) = split_command(line);
    match name {
        "exit" | "quit" | "q" => return Ok(Some(Action::Exit)),
        "help" | "commands" => {
            let topic = if rest.is_empty() {
                None
            } else {
                Some(rest.to_string())
            };
            return Ok(Some(Action::Help(topic)));
        }
        _ => {}
    }
    let Some(spec) = command(name) else {
        return Err(format!(
            "Unknown command \"{name}\". Type \"help\" for the list."
        ));
    };
    Ok(Some(Action::Run {
        spec,
        args: parse_args(rest)?,
    }))
}

// ─── Help ────────────────────────────────────────────────────────────────

/// The one-line documentation every command gets in the `help` list.
fn help_line(spec: &CommandSpec) -> String {
    format!("{}: {}", spec.name, spec.summary)
}

/// Lists every command with its summary.
pub(crate) fn help() -> String {
    let mut lines = vec![
        "A shell onto Blockloom's backend. Each line is a command; each has".to_string(),
        "key=value arguments (or one JSON object), and a fresh full state snapshot".to_string(),
        "comes back with every response. \"help <command>\" shows one command's".to_string(),
        "arguments.".to_string(),
        String::new(),
    ];
    lines.extend(COMMANDS.iter().map(help_line));
    lines.push(String::new());
    lines.push(
        "Shell words: help, commands, exit, quit. Lines starting with # are ignored.".to_string(),
    );
    lines.join("\n")
}

/// One command's full documentation.
fn help_for(topic: &str) -> Result<String, String> {
    let spec = command(topic)
        .ok_or_else(|| format!("\"{topic}\" isn't a command. Type \"help\" for the list."))?;
    let mut lines = vec![format!("{} - {}", spec.name, spec.summary)];
    if !spec.args.is_empty() {
        lines.push(String::from("Arguments:"));
        for arg in spec.args {
            let optional = if arg.required { "" } else { " (optional)" };
            lines.push(format!("  {}: {}{}", arg.name, arg.ty, optional));
        }
    } else {
        lines.push("No arguments.".to_string());
    }
    lines.push(format!("Dispatcher name: \"{}\".", spec.cmd));
    Ok(lines.join("\n"))
}

// ─── Specs ────────────────────────────────────────────────────────────────

/// The whole command registry as JSON, keyed so a machine can build a tool
/// list straight from it. Printed by the shell's `--specs` flag, and what an
/// MCP host reads at startup so its tools can't drift from the commands.
pub fn specs_json() -> String {
    let payload = serde_json::json!({ "commands": COMMANDS });
    serde_json::to_string(&payload).unwrap_or_else(|e| e.to_string())
}

// ─── Running ─────────────────────────────────────────────────────────────

/// Runs one shell line and returns the JSON response object. `with_state`
/// false leaves `state` null, for callers who only want the result.
pub fn run(backend: &Backend, line: &str, with_state: bool) -> Value {
    let state = || {
        if with_state {
            backend
                .dispatch("get_state", json!({}))
                .unwrap_or(Value::Null)
        } else {
            Value::Null
        }
    };
    let action = match parse(line) {
        Ok(Some(action)) => action,
        Ok(None) => return json!({"ok": true, "result": null, "error": null, "state": state()}),
        Err(error) => {
            return json!({"ok": false, "result": null, "error": error, "state": state()});
        }
    };
    match action {
        Action::Exit | Action::Help(_) => {
            // The binary reads these itself; this is only reached if it didn't.
            let result = match action {
                Action::Help(None) => Value::String(help()),
                Action::Help(Some(topic)) => match help_for(&topic) {
                    Ok(text) => Value::String(text),
                    Err(error) => {
                        return json!({"ok": false, "result": null, "error": error, "state":
                            state()});
                    }
                },
                _ => Value::Null,
            };
            json!({"ok": true, "result": result, "error": null, "state": state()})
        }
        Action::Run { spec, args } => match backend.dispatch(spec.cmd, Value::Object(args)) {
            Ok(result) => json!({"ok": true, "result": result, "error": null, "state": state()}),
            Err(error) => json!({"ok": false, "result": null, "error": error, "state": state()}),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args_ok(rest: &str) -> Map<String, Value> {
        parse_args(rest).unwrap()
    }

    #[test]
    fn kebab_names_map_onto_dispatch_commands() {
        assert_eq!(command("create-project").unwrap().cmd, "create_project");
        assert_eq!(command("add-actor").unwrap().cmd, "add_actor");
        assert_eq!(command("set-fixed-rate").unwrap().cmd, "set_fixed_rate");
    }

    #[test]
    fn camel_case_and_aliases_work_too() {
        assert_eq!(command("create_project").unwrap().name, "create-project");
        assert_eq!(command("state").unwrap().cmd, "get_state");
        assert_eq!(command("get-state").unwrap().cmd, "get_state");
    }

    #[test]
    fn every_registry_name_is_unique() {
        let mut names: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
        names.sort();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len());
    }

    #[test]
    fn every_command_has_a_summary() {
        assert!(COMMANDS.iter().all(|c| !c.summary.is_empty()));
    }

    #[test]
    fn simple_key_value_args_parse() {
        let args = parse_args_ok("shape=Circle name=Ball");
        assert_eq!(args["shape"], "Circle");
        assert_eq!(args["name"], "Ball");
    }

    #[test]
    fn numbers_keep_their_type() {
        let args = parse_args_ok("x=10 y=-3.5");
        assert_eq!(args["x"], 10);
        assert_eq!(args["y"], -3.5);
    }

    #[test]
    fn quoted_numbers_stay_text() {
        let args = parse_args_ok("name=\"123\"");
        assert_eq!(args["name"], "123");
    }

    #[test]
    fn quoted_values_keep_the_spaces() {
        let args = parse_args_ok("name=\"two words\" source='a b c'");
        assert_eq!(args["name"], "two words");
        assert_eq!(args["source"], "a b c");
    }

    #[test]
    fn arrays_and_objects_parse_as_json() {
        let args = parse_args_ok("gravity=[0,-981,0] value={\"kind\":\"Number\",\"value\":5}");
        assert_eq!(args["gravity"][1], -981.0);
        assert_eq!(args["value"]["kind"], "Number");
    }

    #[test]
    fn json_arguments_with_spaces_stay_one_token() {
        let args = parse_args_ok("placement={\"position\": [1, 2, 3], \"rotation\": [0, 0, 0]}");
        assert_eq!(args["placement"]["position"], json!([1, 2, 3]));
    }

    #[test]
    fn a_whole_json_object_takes_over_the_arguments() {
        let (name, rest) = split_command("set-camera {\"zoom\": 2}");
        assert_eq!(name, "set-camera");
        let args = parse_args_ok(rest);
        assert_eq!(args["zoom"], 2);
    }

    #[test]
    fn missing_equals_is_an_error() {
        assert!(parse_args("just a token").is_err());
    }

    #[test]
    fn blank_lines_and_comments_are_skipped() {
        assert!(parse("").unwrap().is_none());
        assert!(parse("  # a comment").unwrap().is_none());
        assert!(matches!(parse("help").unwrap(), Some(Action::Help(None))));
    }

    #[test]
    fn unknown_commands_error_with_a_hint() {
        assert!(
            parse("nonsense x=1")
                .unwrap_err()
                .contains("Unknown command")
        );
    }

    #[test]
    fn help_reaches_every_command() {
        for spec in COMMANDS {
            assert!(help_for(spec.name).is_ok(), "no help for {}", spec.name);
            assert!(help_for(spec.cmd).is_ok(), "no help for {}", spec.cmd);
        }
    }

    #[test]
    fn a_block_object_survives_the_wire_fold() {
        use blockloom_core::blocks::{Instruction, InstructionKind};
        use blockloom_core::wire;
        let args = parse_args_ok("instruction={\"id\":\"e1\",\"type\":\"WhenStarted\"}");
        let block: Instruction = wire::from_wire(args["instruction"].clone()).unwrap();
        assert_eq!(block.id, "e1");
        assert_eq!(block.kind, InstructionKind::WhenStarted);
    }
}
