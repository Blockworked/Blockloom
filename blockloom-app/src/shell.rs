//! A shell onto the backend: a text command line is parsed into the same
//! (name, JSON args) shape the editor's `invokeCommand` sends, so an AI agent
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
        summary: "Open the project in a folder, replacing whatever was open. Attaches to a live owner's copy unless force takes it over.",
        args: &[
            ArgSpec {
                name: "path",
                ty: "folder path",
                required: true,
            },
            ArgSpec {
                name: "force",
                ty: "bool",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "create-project",
        cmd: "create_project",
        aliases: &["create_project"],
        summary: "Make a project folder under location and open it, empty or from a sample (physics-playground: a player, crates, a pendulum, a motor wheel, a door, a breakable joint and a trigger zone).",
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
            ArgSpec {
                name: "sample",
                ty: "physics-playground",
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
        name: "sync-status",
        cmd: "sync_status",
        aliases: &["sync_status", "sync"],
        summary: "Where the open project stands against its folder: revisions, staleness, lock owner, attach mode.",
        args: &[],
    },
    CommandSpec {
        name: "reload-project",
        cmd: "reload_project",
        aliases: &["reload_project", "reload"],
        summary: "Load the open project back off disk. Refuses when unsaved in-memory edits would be lost, unless take_theirs takes the folder's side.",
        args: &[ArgSpec {
            name: "take_theirs",
            ty: "bool",
            required: false,
        }],
    },
    CommandSpec {
        name: "take-over-lock",
        cmd: "take_over_lock",
        aliases: &["take_over_lock", "take-over"],
        summary: "Take the open folder's owner lock explicitly, so headless edits stop deferring to whoever held it.",
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
            ArgSpec {
                name: "hdr",
                ty: "bool",
                required: false,
            },
            ArgSpec {
                name: "storePass",
                ty: "string",
                required: false,
            },
            ArgSpec {
                name: "keyPass",
                ty: "string",
                required: false,
            },
            ArgSpec {
                name: "rememberPasswords",
                ty: "boolean (default: off)",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "start-build-game",
        cmd: "start_build_game",
        aliases: &["start_build_game"],
        summary: "Start a background build. Poll build-job-status or cancel-build-job using its id. Optional device installs and launches an Android build. Build the open project into a folder under path that runs on its own.",
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
            ArgSpec {
                name: "hdr",
                ty: "bool",
                required: false,
            },
            ArgSpec {
                name: "storePass",
                ty: "string",
                required: false,
            },
            ArgSpec {
                name: "keyPass",
                ty: "string",
                required: false,
            },
            ArgSpec {
                name: "rememberPasswords",
                ty: "boolean (default: off)",
                required: false,
            },
            ArgSpec {
                name: "device",
                ty: "string",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "build-job-status",
        cmd: "build_job_status",
        aliases: &["build_job_status"],
        summary: "Read a background build's stage, tool output and final result.",
        args: &[ArgSpec {
            name: "id",
            ty: "number",
            required: true,
        }],
    },
    CommandSpec {
        name: "cancel-build-job",
        cmd: "cancel_build_job",
        aliases: &["cancel_build_job"],
        summary: "Cancel a background build and its active tools. Poll until cancelled before starting another.",
        args: &[ArgSpec {
            name: "id",
            ty: "number",
            required: true,
        }],
    },
    CommandSpec {
        name: "android-status",
        cmd: "android_status",
        aliases: &["android_status", "android-check"],
        summary: "The Android toolchain as it stands: SDK/NDK paths, license stamp, JDK and Rust target probes. No device needed.",
        args: &[],
    },
    CommandSpec {
        name: "android-device-status",
        cmd: "android_device_status",
        aliases: &["android_device_status", "android-devices"],
        summary: "What adb devices sees through the installed platform-tools, or why there is no adb to ask.",
        args: &[],
    },
    CommandSpec {
        name: "android-install-sdk",
        cmd: "android_install_sdk",
        aliases: &["android_install_sdk", "android-sdk-install"],
        summary: "Download the cmdline-tools bootstrap when missing and install the pinned platform, build-tools, platform-tools and NDK. Licenses stay unaccepted until android-accept-licenses.",
        args: &[],
    },
    CommandSpec {
        name: "android-accept-licenses",
        cmd: "android_accept_licenses",
        aliases: &["android_accept_licenses"],
        summary: "Show the SDK license texts, or accept them when accept is true and record the stamp in the app config.",
        args: &[ArgSpec {
            name: "accept",
            ty: "bool",
            required: false,
        }],
    },
    CommandSpec {
        name: "android-install",
        cmd: "android_install",
        aliases: &["android_install"],
        summary: "Install an APK on a connected device or emulator and launch it.",
        args: &[
            ArgSpec {
                name: "apk",
                ty: "file path",
                required: true,
            },
            ArgSpec {
                name: "app",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "device",
                ty: "string",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "android-logcat",
        cmd: "android_logcat",
        aliases: &["android_logcat"],
        summary: "Dump the device log, keeping the runtime's blockloom markers and any Rust panic. One shot for the dev loop and smoke tests.",
        args: &[ArgSpec {
            name: "device",
            ty: "string",
            required: false,
        }],
    },
    CommandSpec {
        name: "android-logcat-tail",
        cmd: "android_logcat_tail",
        aliases: &["android_logcat_tail"],
        summary: "Poll the device log the way the Build dialog streams it: dump, clear the buffer for the next poll, and append every kept line to the RunLog. Each call reads only what arrived since the last.",
        args: &[ArgSpec {
            name: "device",
            ty: "string",
            required: false,
        }],
    },
    CommandSpec {
        name: "android-keyring-status",
        cmd: "android_keyring_status",
        aliases: &["android_keyring_status"],
        summary: "What the OS keyring keeps for the open project's release key: whether this machine has a scriptable store, and which passwords it holds. Needs no device.",
        args: &[],
    },
    CommandSpec {
        name: "android-forget-passwords",
        cmd: "android_forget_passwords",
        aliases: &["android_forget_passwords"],
        summary: "Forget whatever the OS keyring keeps for the open project's release key. Needs no device.",
        args: &[],
    },
    CommandSpec {
        name: "android-emulator-status",
        cmd: "android_emulator_status",
        aliases: &["android_emulator_status"],
        summary: "The emulator rows as they stand: whether this machine can boot anything, and every AVD with its run state. Needs no device.",
        args: &[],
    },
    CommandSpec {
        name: "android-create-avd",
        cmd: "android_create_avd",
        aliases: &["android_create_avd"],
        summary: "Make an AVD on the pinned Android 35 x86_64 image. Empty names the managed default. Needs no device.",
        args: &[ArgSpec {
            name: "name",
            ty: "string",
            required: false,
        }],
    },
    CommandSpec {
        name: "android-rename-avd",
        cmd: "android_rename_avd",
        aliases: &["android_rename_avd"],
        summary: "Rename a stopped virtual device.",
        args: &[
            ArgSpec {
                name: "name",
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
        name: "android-delete-avd",
        cmd: "android_delete_avd",
        aliases: &["android_delete_avd"],
        summary: "Delete a stopped virtual device and its saved data.",
        args: &[ArgSpec {
            name: "name",
            ty: "string",
            required: true,
        }],
    },
    CommandSpec {
        name: "android-start-emulator",
        cmd: "android_start_emulator",
        aliases: &["android_start_emulator"],
        summary: "Boot an AVD (the managed default when unset, created on the spot when no AVDs exist at all) and wait up to waitSecs for adb to see it booted: 5 minutes when unset, 0 to return right after spawning. headless hides the host window for the embedded view. Needs no device.",
        args: &[
            ArgSpec {
                name: "avd",
                ty: "string",
                required: false,
            },
            ArgSpec {
                name: "waitSecs",
                ty: "number",
                required: false,
            },
            ArgSpec {
                name: "headless",
                ty: "bool",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "android-stop-emulator",
        cmd: "android_stop_emulator",
        aliases: &["android_stop_emulator"],
        summary: "Stop the running emulator on serial. Empty stops the only running emulator; a physical serial is refused. Needs no device.",
        args: &[ArgSpec {
            name: "serial",
            ty: "string",
            required: false,
        }],
    },
    CommandSpec {
        name: "android-mirror-frame",
        cmd: "android_mirror_frame",
        aliases: &["android_mirror_frame"],
        summary: "Grab the device's screen as a downscaled PNG data URL for the embedded emulator view. Empty device means the only device; maxWidth caps the frame width (default 360).",
        args: &[
            ArgSpec {
                name: "device",
                ty: "string",
                required: false,
            },
            ArgSpec {
                name: "maxWidth",
                ty: "number",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "android-mirror-tap",
        cmd: "android_mirror_tap",
        aliases: &["android_mirror_tap"],
        summary: "Tap the device at the fractional point x, y (0..1 across the mirror image). What a click on the embedded screen becomes.",
        args: &[
            ArgSpec {
                name: "device",
                ty: "string",
                required: false,
            },
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
        ],
    },
    CommandSpec {
        name: "android-mirror-swipe",
        cmd: "android_mirror_swipe",
        aliases: &["android_mirror_swipe"],
        summary: "Swipe the device from one fractional point to another over durationMs (default 300). What a drag on the embedded screen becomes.",
        args: &[
            ArgSpec {
                name: "device",
                ty: "string",
                required: false,
            },
            ArgSpec {
                name: "x1",
                ty: "number",
                required: false,
            },
            ArgSpec {
                name: "y1",
                ty: "number",
                required: false,
            },
            ArgSpec {
                name: "x2",
                ty: "number",
                required: false,
            },
            ArgSpec {
                name: "y2",
                ty: "number",
                required: false,
            },
            ArgSpec {
                name: "durationMs",
                ty: "number",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "android-mirror-key",
        cmd: "android_mirror_key",
        aliases: &["android_mirror_key"],
        summary: "Press a named key on the device: back, home, recents, enter, delete, tab, power, volume_up, volume_down, volume_mute. The embedded screen's hardware buttons.",
        args: &[
            ArgSpec {
                name: "device",
                ty: "string",
                required: false,
            },
            ArgSpec {
                name: "code",
                ty: "string",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "android-set-sdk-path",
        cmd: "android_set_sdk_path",
        aliases: &["android_set_sdk_path"],
        summary: "Point the SDK row at a folder (empty clears back to the default).",
        args: &[ArgSpec {
            name: "path",
            ty: "folder path",
            required: false,
        }],
    },
    CommandSpec {
        name: "android-set-ndk-path",
        cmd: "android_set_ndk_path",
        aliases: &["android_set_ndk_path"],
        summary: "Point the NDK row at a folder (empty clears back to the pinned NDK inside the SDK).",
        args: &[ArgSpec {
            name: "path",
            ty: "folder path",
            required: false,
        }],
    },
    CommandSpec {
        name: "set-android-settings",
        cmd: "set_android_settings",
        aliases: &["set_android_settings"],
        summary: "Write the open project's Android rows: applicationId override (empty for the default), version code and name, release keystore file plus key alias (empty signs debug). Passwords are never stored.",
        args: &[
            ArgSpec {
                name: "applicationId",
                ty: "string",
                required: false,
            },
            ArgSpec {
                name: "versionCode",
                ty: "number",
                required: false,
            },
            ArgSpec {
                name: "versionName",
                ty: "string",
                required: false,
            },
            ArgSpec {
                name: "keystore",
                ty: "file path",
                required: false,
            },
            ArgSpec {
                name: "keyAlias",
                ty: "string",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "android-create-keystore",
        cmd: "android_create_keystore",
        aliases: &["android_create_keystore"],
        summary: "Make a release key: a new RSA keypair under alias in the key file at path, creating the file when needed. Passwords come from storePass/keyPass, the env, then the OS keyring, and are kept there only with rememberPasswords.",
        args: &[
            ArgSpec {
                name: "path",
                ty: "file path",
                required: true,
            },
            ArgSpec {
                name: "alias",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "storePass",
                ty: "string",
                required: false,
            },
            ArgSpec {
                name: "keyPass",
                ty: "string",
                required: false,
            },
            ArgSpec {
                name: "rememberPasswords",
                ty: "boolean (default: off)",
                required: false,
            },
        ],
    },
    // ── The world ─────────────────────────────────────────────────────────
    CommandSpec {
        name: "set-mode",
        cmd: "set_mode",
        aliases: &["set_mode"],
        summary: "Switch the active scene between 2D and 3D.",
        args: &[ArgSpec {
            name: "mode",
            ty: "TwoD|ThreeD",
            required: true,
        }],
    },
    CommandSpec {
        name: "add-scene",
        cmd: "add_scene",
        aliases: &["add_scene"],
        summary: "Add an empty scene (active scene's dimension unless mode is given) and make it active.",
        args: &[
            ArgSpec {
                name: "name",
                ty: "string",
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
        name: "duplicate-scene",
        cmd: "duplicate_scene",
        aliases: &["duplicate_scene"],
        summary: "Copy a scene with fresh ids and make the copy active.",
        args: &[ArgSpec {
            name: "sceneId",
            ty: "id",
            required: true,
        }],
    },
    CommandSpec {
        name: "rename-scene",
        cmd: "rename_scene",
        aliases: &["rename_scene"],
        summary: "Rename a scene, keeping names unique.",
        args: &[
            ArgSpec {
                name: "sceneId",
                ty: "id",
                required: true,
            },
            ArgSpec {
                name: "name",
                ty: "string",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "remove-scene",
        cmd: "remove_scene",
        aliases: &["remove_scene"],
        summary: "Delete a scene. The last one stays.",
        args: &[ArgSpec {
            name: "sceneId",
            ty: "id",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-active-scene",
        cmd: "set_active_scene",
        aliases: &["set_active_scene", "switch-scene"],
        summary: "Make a scene the edited one. A switch across dimensions rebuilds the runtime like set-mode.",
        args: &[ArgSpec {
            name: "sceneId",
            ty: "id",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-default-scene",
        cmd: "set_default_scene",
        aliases: &["set_default_scene"],
        summary: "Set which scene a fresh open - and a built game - boots into.",
        args: &[ArgSpec {
            name: "sceneId",
            ty: "id",
            required: true,
        }],
    },
    CommandSpec {
        name: "scene-components",
        cmd: "scene_components",
        aliases: &["scene_components"],
        summary: "List a scene's settings as components (active scene by default).",
        args: &[ArgSpec {
            name: "sceneId",
            ty: "id",
            required: false,
        }],
    },
    CommandSpec {
        name: "set-scene-component",
        cmd: "set_scene_component",
        aliases: &["set_scene_component"],
        summary: "Set one scene component (lighting, sky, fog, wind, post, physics and the rest).",
        args: &[
            ArgSpec {
                name: "sceneId",
                ty: "id",
                required: false,
            },
            ArgSpec {
                name: "component",
                ty: "object",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "remove-scene-component",
        cmd: "remove_scene_component",
        aliases: &["remove_scene_component"],
        summary: "Drop one scene component; reads of it fall back to its default.",
        args: &[
            ArgSpec {
                name: "sceneId",
                ty: "id",
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
        name: "import-scene",
        cmd: "import_scene",
        aliases: &["import_scene"],
        summary: "Bring a .blockscene file into this project (absolute or project-relative) and make it active.",
        args: &[ArgSpec {
            name: "path",
            ty: "file path",
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
        name: "set-navigation",
        cmd: "set_navigation",
        aliases: &["set_navigation"],
        summary: "Set navigation cost areas and off-mesh links in XY or XZ coordinates.",
        args: &[ArgSpec {
            name: "navigation",
            ty: "object",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-lighting",
        cmd: "set_lighting",
        aliases: &["set_lighting"],
        summary: "Set the 3D world's light direction, colors, brightness, AO, shadows and ray tracing.",
        args: &[ArgSpec {
            name: "lighting",
            ty: "object",
            required: true,
        }],
    },
    CommandSpec {
        name: "save-interface-asset",
        cmd: "save_interface_asset",
        aliases: &["save_interface_asset"],
        summary: "Save the interface to a reusable JSON asset in assets/ui.",
        args: &[ArgSpec {
            name: "name",
            ty: "string",
            required: true,
        }],
    },
    CommandSpec {
        name: "load-interface-asset",
        cmd: "load_interface_asset",
        aliases: &["load_interface_asset"],
        summary: "Load an interface JSON asset as one undoable edit.",
        args: &[ArgSpec {
            name: "path",
            ty: "string",
            required: true,
        }],
    },
    CommandSpec {
        name: "preview-interface",
        cmd: "preview_interface",
        aliases: &["preview_interface"],
        summary: "Preview a temporary {revision, generation, document, viewport?, screen?} without saving or starting gameplay. Screen names a top-level widget tree. Omit design to close it.",
        args: &[ArgSpec {
            name: "design",
            ty: "object",
            required: false,
        }],
    },
    CommandSpec {
        name: "interface-layout",
        cmd: "interface_layout",
        aliases: &["interface_layout"],
        summary: "Read the latest design geometry in physical viewport pixels, tagged by revision and generation.",
        args: &[],
    },
    CommandSpec {
        name: "begin-interface-edit",
        cmd: "begin_interface_edit",
        aliases: &["begin_interface_edit"],
        summary: "Begin a move/resize draft at the saved project revision; returns a token.",
        args: &[ArgSpec {
            name: "revision",
            ty: "number",
            required: true,
        }],
    },
    CommandSpec {
        name: "update-interface-edit",
        cmd: "update_interface_edit",
        aliases: &["update_interface_edit"],
        summary: "Replace the draft with Move, Resize, SetProperty {id, property: {path, value}}, Reorder {id, index: zero-based sibling position} or Reparent {id, parent, placement: {mode: Free, offset, size} or {mode: Flow}}. Property paths: element.kind/content/anchor/modal, layout (object or null). Does not save.",
        args: &[
            ArgSpec {
                name: "token",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "edit",
                ty: "object",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "commit-interface-edit",
        cmd: "commit_interface_edit",
        aliases: &["commit_interface_edit"],
        summary: "Commit a current draft as one saved undo step. Rejects stale transactions.",
        args: &[ArgSpec {
            name: "token",
            ty: "string",
            required: true,
        }],
    },
    CommandSpec {
        name: "cancel-interface-edit",
        cmd: "cancel_interface_edit",
        aliases: &["cancel_interface_edit"],
        summary: "Discard a draft without saving. Also accepts a stale draft token.",
        args: &[ArgSpec {
            name: "token",
            ty: "string",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-interface",
        cmd: "set_interface",
        aliases: &["set_interface"],
        summary: "Save the interface designer document, including widgets, styles, bindings and prefabs.",
        args: &[ArgSpec {
            name: "document",
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
    CommandSpec {
        name: "set-sky",
        cmd: "set_sky",
        aliases: &["set_sky"],
        summary: "Set the 3D sky: kind (Flat, Physical, Gradient, Hdri), sun placement, each kind's settings, background/reflections/lighting, stars and aurora.",
        args: &[ArgSpec {
            name: "sky",
            ty: "object",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-fog",
        cmd: "set_fog",
        aliases: &["set_fog"],
        summary: "Set the 3D air: height fog, volumetric fog (froxels lit by sun, moon and lights) and aerial perspective.",
        args: &[ArgSpec {
            name: "fog",
            ty: "object",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-clouds",
        cmd: "set_clouds",
        aliases: &["set_clouds"],
        summary: "Set volumetric clouds: shape, altitude, erosion, lighting, shadows and quality.",
        args: &[ArgSpec {
            name: "clouds",
            ty: "object",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-cloud-layers",
        cmd: "set_cloud_layers",
        aliases: &["set_cloud_layers"],
        summary: "Set the planar cloud layers (up to 4): coverage texture or seeded FBM, coverage, contrast, tiling, opacity, altitude, parallax, tints and ramps, horizon fade, scroll, wind, flow map and spin.",
        args: &[ArgSpec {
            name: "layers",
            ty: "[layer objects]",
            required: true,
        }],
    },
    CommandSpec {
        name: "paint-cloud-layer",
        cmd: "paint_cloud_layer",
        aliases: &["paint_cloud_layer"],
        summary: "Paint a stroke into a cloud layer's coverage (assets/clouds/layer-N.png): layer index from 0, brush {tool: Cloud|Eraser|Blur|Advect, radius, strength, falloff}, points [[u,v],...] across the tile.",
        args: &[
            ArgSpec {
                name: "layer",
                ty: "number",
                required: true,
            },
            ArgSpec {
                name: "brush",
                ty: "object {tool, radius, strength, falloff}",
                required: true,
            },
            ArgSpec {
                name: "points",
                ty: "[[u, v]]",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "bake-cloud-noise",
        cmd: "bake_cloud_noise",
        aliases: &["bake_cloud_noise"],
        summary: "Bake the clouds' shape and erosion noise from the cloud seed into assets/clouds/*.png volume strips and point the clouds at them.",
        args: &[],
    },
    CommandSpec {
        name: "set-lightning",
        cmd: "set_lightning",
        aliases: &["set_lightning"],
        summary: "Set lightning: flash light, sky pulse, thunder, and the random-strike storm (rate a minute, region box).",
        args: &[ArgSpec {
            name: "lightning",
            ty: "object",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-wind",
        cmd: "set_wind",
        aliases: &["set_wind"],
        summary: "Set the wind: direction (degrees clockwise from north), speed, gusts, log-law profile, storm 0-1, and the clouds' drift, erosion, layer scroll, time-lapse and seed.",
        args: &[ArgSpec {
            name: "wind",
            ty: "object",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-director",
        cmd: "set_director",
        aliases: &["set_director"],
        summary: "Set the time-of-day and weather director: enabled, time_of_day (0-24), day_length seconds per day (0 freezes), loop_enabled, one 24h Bezier track per dial (sun/moon azimuth and elevation, exposure, temperature, fog_density, cloud_coverage, cloud_type, precipitation, wetness, wind_speed, wind_direction, aurora_kp, lut_weight; each {keys: [{time, value, in_tangent, out_tangent}], loop_enabled}) and presets (Clear, Overcast, Storm, Sunset, Night built in, plus project ones).",
        args: &[ArgSpec {
            name: "director",
            ty: "object",
            required: true,
        }],
    },
    CommandSpec {
        name: "apply-director-preset",
        cmd: "apply_director_preset",
        aliases: &["apply_director_preset"],
        summary: "Replace the director with a keyframe preset (dawn, noon, dusk or midnight): a sun track through that moment plus matching exposure, fog and cloud tracks.",
        args: &[ArgSpec {
            name: "preset",
            ty: "dawn|noon|dusk|midnight",
            required: true,
        }],
    },
    CommandSpec {
        name: "save-director-preset",
        cmd: "save_director_preset",
        aliases: &["save_director_preset"],
        summary: "Copy a weather preset (a project one, or a built-in like Storm) to a project preset under a new name, replacing the project preset of that name when one exists.",
        args: &[
            ArgSpec {
                name: "name",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "from",
                ty: "string",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "set-vfx",
        cmd: "set_vfx",
        aliases: &["set_vfx"],
        summary: "Set the particle budget (live particles across every emitter) and cpu_only, which keeps every emitter off the GPU.",
        args: &[ArgSpec {
            name: "vfx",
            ty: "object",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-quality",
        cmd: "set_quality",
        aliases: &["set_quality"],
        summary: "Set rendering quality: preset (Low, Medium, High, Ultra), resolution_scale, dynamic_resolution, min_scale, target_ms, auto_drop, over_budget_frames, upscaler (Spatial, Taa, Dlss), dlss_mode and sharpness.",
        args: &[ArgSpec {
            name: "quality",
            ty: "object",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-post-process",
        cmd: "set_post_process",
        aliases: &["set_post_process"],
        summary: "Set the camera's post chain: exposure_ev and auto_exposure, tonemapping and tone (toe, shoulder), bloom (threshold, knee, scatter, dirt), grading (white balance, lift/gamma/gain, saturation, contrast, lut), vignette, depth_of_field, motion_blur, ao (radius, intensity), ssr, chromatic_aberration, grain and sharpen.",
        args: &[ArgSpec {
            name: "post",
            ty: "object",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-display-output",
        cmd: "set_display_output",
        aliases: &["set_display_output"],
        summary: "Set the output signal (Sdr, Hdr10, Scrgb), peak_nits and paper_white_nits.",
        args: &[ArgSpec {
            name: "display",
            ty: "object",
            required: true,
        }],
    },
    // ── Actors ────────────────────────────────────────────────────────────
    CommandSpec {
        name: "select-actor",
        cmd: "select_actor",
        aliases: &["select_actor"],
        summary: "Choose whose canvas the editor shows; an empty id clears selection.",
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
                ty: "object {position, rotation, scale, stretch}",
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
    // ── Physics ───────────────────────────────────────────────────────────
    CommandSpec {
        name: "add-collider",
        cmd: "add_collider",
        aliases: &[],
        summary: "Add a collider to an actor and answer its id. The object is a collider spec: {geometry: {kind: \"Shape\", shape: {...}} or {kind: \"FromLook\"}, center, rotation, material, trigger, layer, ...}. Refused when it would leave the actor invalid.",
        args: &[
            A,
            ArgSpec {
                name: "collider",
                ty: "object {\"geometry\", ...}",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "set-collider",
        cmd: "set_collider",
        aliases: &[],
        summary: "Replace the collider whose id the object carries, in place. The id never changes.",
        args: &[ArgSpec {
            name: "collider",
            ty: "object {\"id\", \"geometry\", ...}",
            required: true,
        }],
    },
    CommandSpec {
        name: "remove-collider",
        cmd: "remove_collider",
        aliases: &[],
        summary: "Remove a collider by id. Answers the actor it was on.",
        args: &[ArgSpec {
            name: "colliderId",
            ty: "id",
            required: true,
        }],
    },
    CommandSpec {
        name: "add-constraint",
        cmd: "add_constraint",
        aliases: &[],
        summary: "Add a joint to an actor and answer its id. The object is a constraint spec: {kind: Fixed|Hinge|Ball|Slider|Spring|Distance|Wheel|Configurable, name, target (an actor id; empty anchors to the world), anchor, axis, limit: {enabled, min, max}, motor: {mode: Off|Velocity|Position, target, max_force}, spring, min_distance, max_distance, break_force, break_torque, break_message, enable_collision}. The actor and its target need Rigidbodies.",
        args: &[
            A,
            ArgSpec {
                name: "constraint",
                ty: "object {\"kind\", ...}",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "set-constraint",
        cmd: "set_constraint",
        aliases: &[],
        summary: "Replace the constraint whose id the object carries, in place. The id never changes.",
        args: &[ArgSpec {
            name: "constraint",
            ty: "object {\"id\", \"kind\", ...}",
            required: true,
        }],
    },
    CommandSpec {
        name: "remove-constraint",
        cmd: "remove_constraint",
        aliases: &[],
        summary: "Remove a constraint by id. Answers the actor it was on.",
        args: &[ArgSpec {
            name: "constraintId",
            ty: "id",
            required: true,
        }],
    },
    CommandSpec {
        name: "list-constraints",
        cmd: "list_constraints",
        aliases: &[],
        summary: "List the active scene's constraints with the name blocks use for each, optionally for one actor.",
        args: &[ArgSpec {
            name: "actorId",
            ty: "id",
            required: false,
        }],
    },
    CommandSpec {
        name: "fit-collider-to-look",
        cmd: "fit_collider_to_look",
        aliases: &[],
        summary: "Save a collider's shape from its actor's Look, so it stops following the Look.",
        args: &[ArgSpec {
            name: "colliderId",
            ty: "id",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-rigidbody",
        cmd: "set_rigidbody",
        aliases: &[],
        summary: "Give an actor a Rigidbody, or replace the one it has (its id stays). The object is a rigidbody spec: {body_type, mass, use_gravity, constraints, ...}. Answers the body's id.",
        args: &[
            A,
            ArgSpec {
                name: "rigidbody",
                ty: "object {...}",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "remove-rigidbody",
        cmd: "remove_rigidbody",
        aliases: &[],
        summary: "Take the Rigidbody off an actor. Its colliders stay, as static scenery or on an ancestor's body.",
        args: &[A],
    },
    CommandSpec {
        name: "set-character-controller",
        cmd: "set_character_controller",
        aliases: &[],
        summary: "Give an actor a CharacterController, or replace the one it has (its id stays). The object is {radius, height, slope_limit, step_offset, skin_width, min_move_distance, detect_collisions, overlap_recovery, layer, center, up}; omitted fields take Unity's defaults. Answers the component's id.",
        args: &[
            A,
            ArgSpec {
                name: "controller",
                ty: "object {...}",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "remove-character-controller",
        cmd: "remove_character_controller",
        aliases: &[],
        summary: "Take the CharacterController off an actor.",
        args: &[A],
    },
    CommandSpec {
        name: "set-character-motor",
        cmd: "set_character_motor",
        aliases: &[],
        summary: "Give an actor a CharacterMotor (walking, sprinting, jumping, crouching on top of its CharacterController), or replace the one it has. The object is {owner, space, walk_speed, sprint_speed, crouch_speed, ground_acceleration, ground_braking, air_acceleration, air_control, turn_speed, gravity_scale, terminal_fall_speed, ground_snap_distance, slide_on_steep, jump_height, max_jumps, jump_cut, coyote_time, jump_buffer, crouch_height, top_down}; omitted fields take the 3D defaults and the whole object replaces the old one. Answers the component's id.",
        args: &[
            A,
            ArgSpec {
                name: "motor",
                ty: "object {...}",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "remove-character-motor",
        cmd: "remove_character_motor",
        aliases: &[],
        summary: "Take the CharacterMotor off an actor.",
        args: &[A],
    },
    CommandSpec {
        name: "preview-player-preset",
        cmd: "preview_player_preset",
        aliases: &[],
        summary: "What a player preset would add, replace or convert on an actor, and why it might be refused. Changes nothing. Presets: first-person-3d, third-person-3d, top-down-3d, platformer-2d, top-down-2d.",
        args: &[
            A,
            ArgSpec {
                name: "preset",
                ty: "first-person-3d|third-person-3d|top-down-3d|platformer-2d|top-down-2d",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "apply-player-preset",
        cmd: "apply_player_preset",
        aliases: &[],
        summary: "Make an actor a playable character in one undoable step: a visual if it has none, a CharacterController, a CharacterMotor, the Camera and PlayerCamera, and the standard Move, Look, Jump, Sprint, Crouch and Interact actions. An actor that already moves another way (a dynamic Rigidbody, a legacy Body) needs convert=true.",
        args: &[
            A,
            ArgSpec {
                name: "preset",
                ty: "first-person-3d|third-person-3d|top-down-3d|platformer-2d|top-down-2d",
                required: true,
            },
            ArgSpec {
                name: "convert",
                ty: "bool",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "save-player-profile",
        cmd: "save_player_profile",
        aliases: &[],
        summary: "Save an actor's controller, motor, camera and input actions as assets/profiles/<name>.profile.json, to reuse here or in another project. Answers the file's project path.",
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
        name: "list-player-profiles",
        cmd: "list_player_profiles",
        aliases: &[],
        summary: "The player profiles saved in this project.",
        args: &[],
    },
    CommandSpec {
        name: "apply-player-profile",
        cmd: "apply_player_profile",
        aliases: &[],
        summary: "Install a saved player profile on an actor as one undoable step. Input actions the project already has keep their own bindings.",
        args: &[
            A,
            ArgSpec {
                name: "name",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "convert",
                ty: "bool",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "import-player-profile",
        cmd: "import_player_profile",
        aliases: &[],
        summary: "Copy a profile file from another project into this one. Answers the profile's name.",
        args: &[ArgSpec {
            name: "path",
            ty: "file path",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-physics-profile",
        cmd: "set_physics_profile",
        aliases: &[],
        summary: "Choose how the project's collisions behave: Legacy (as before this system) or Unity (the documented matrix).",
        args: &[ArgSpec {
            name: "profile",
            ty: "Legacy|Unity",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-physics-layer-name",
        cmd: "set_physics_layer_name",
        aliases: &[],
        summary: "Name one of the 32 collision layers; an empty name goes back to \"Layer N\".",
        args: &[
            ArgSpec {
                name: "layer",
                ty: "number",
                required: true,
            },
            ArgSpec {
                name: "name",
                ty: "string",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "set-layer-collision",
        cmd: "set_layer_collision",
        aliases: &[],
        summary: "Switch collisions between two layers on or off in the 2D or 3D matrix. Layers are 1 to 32; everything collides until a pair is switched off.",
        args: &[
            ArgSpec {
                name: "mode",
                ty: "TwoD|ThreeD",
                required: true,
            },
            ArgSpec {
                name: "a",
                ty: "number",
                required: true,
            },
            ArgSpec {
                name: "b",
                ty: "number",
                required: true,
            },
            ArgSpec {
                name: "collides",
                ty: "bool",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "physics-plan",
        cmd: "physics_plan",
        aliases: &[],
        summary: "What Play would install for the active scene: each body's mass split, each shape's pose, material and filter groups, and every problem (errors stop Play and Build).",
        args: &[],
    },
    CommandSpec {
        name: "add-physics-material",
        cmd: "add_physics_material",
        aliases: &[],
        summary: "Store a reusable surface material and answer its id. 3D: {dimension: \"Three\", material: {static_friction, dynamic_friction, bounciness, friction_combine, bounce_combine}}; 2D: {dimension: \"Two\", material: {friction, bounciness}}.",
        args: &[
            ArgSpec {
                name: "name",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "material",
                ty: "object {\"dimension\", \"material\"}",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "set-physics-material",
        cmd: "set_physics_material",
        aliases: &[],
        summary: "Change a stored material's values (and name, when given). Colliders using it follow.",
        args: &[
            ArgSpec {
                name: "id",
                ty: "id",
                required: true,
            },
            ArgSpec {
                name: "name",
                ty: "string",
                required: false,
            },
            ArgSpec {
                name: "material",
                ty: "object {\"dimension\", \"material\"}",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "remove-physics-material",
        cmd: "remove_physics_material",
        aliases: &[],
        summary: "Forget a stored material. Refused while a collider still uses it.",
        args: &[ArgSpec {
            name: "id",
            ty: "id",
            required: true,
        }],
    },
    CommandSpec {
        name: "physics-check",
        cmd: "physics_check",
        aliases: &[],
        summary: "Every physics problem in the project's scenes: errors block Play, warnings do not.",
        args: &[],
    },
    CommandSpec {
        name: "physics-cook",
        cmd: "physics_cook",
        aliases: &[],
        summary: "Make (or reuse) the collision data of every mesh collider in the project and answer each mesh's kind and statistics (vertices, triangles, hulls, worst error). A mesh that cannot cook is an error here, in Play and in a build.",
        args: &[],
    },
    CommandSpec {
        name: "set-physics-cooking",
        cmd: "set_physics_cooking",
        aliases: &[],
        summary: "Change how meshes become collision: the weld distance, the most points a hull keeps and, with mesh, a decomposition ({max_hulls, max_hull_vertices, concavity, resolution}, or null to forget it) so a concave mesh can be a solid shape on a dynamic body.",
        args: &[
            ArgSpec {
                name: "weld",
                ty: "number",
                required: false,
            },
            ArgSpec {
                name: "maxHullVertices",
                ty: "number",
                required: false,
            },
            ArgSpec {
                name: "mesh",
                ty: "asset path",
                required: false,
            },
            ArgSpec {
                name: "decompose",
                ty: "object {...}",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "physics-ownership",
        cmd: "physics_ownership",
        aliases: &[],
        summary: "Which body carries each collider in the active scene (itself or the nearest ancestor with a Rigidbody), the shape's pose in the body's frame, static colliders, bodies with no shape and actors still on the legacy Body.",
        args: &[],
    },
    CommandSpec {
        name: "physics-properties",
        cmd: "physics_properties",
        aliases: &[],
        summary: "The units, bounds and visibility of every Rigidbody, Collider and material field.",
        args: &[],
    },
    CommandSpec {
        name: "physics-migration-preview",
        cmd: "physics_migration_preview",
        aliases: &[],
        summary: "What converting each legacy Body to a Rigidbody and Collider would store, in every scene or for one actor. Changes nothing.",
        args: &[ArgSpec {
            name: "actorId",
            ty: "id",
            required: false,
        }],
    },
    CommandSpec {
        name: "migrate-physics",
        cmd: "migrate_physics",
        aliases: &[],
        summary: "Convert legacy Body components to Rigidbody and Collider in every scene, or on one actor. One undo step, and the project file is copied to .blockloom/backups first. Running it again converts nothing.",
        args: &[ArgSpec {
            name: "actorId",
            ty: "id",
            required: false,
        }],
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
        summary: "Make an empty asset (a text file, a starter script, or a scene with .blockscene).",
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
        name: "read-lighting-asset",
        cmd: "read_lighting_asset",
        aliases: &["read_lighting_asset"],
        summary: "Read a reusable Lighting asset.",
        args: &[ArgSpec {
            name: "path",
            ty: "asset path",
            required: true,
        }],
    },
    CommandSpec {
        name: "write-lighting-asset",
        cmd: "write_lighting_asset",
        aliases: &["write_lighting_asset"],
        summary: "Update a Lighting asset and all scenes using it.",
        args: &[
            ArgSpec {
                name: "path",
                ty: "asset path",
                required: true,
            },
            ArgSpec {
                name: "lighting",
                ty: "JSON object",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "set-scene-lighting-asset",
        cmd: "set_scene_lighting_asset",
        aliases: &["set_scene_lighting_asset"],
        summary: "Assign a Lighting asset to a scene (active by default); empty detaches it.",
        args: &[
            ArgSpec {
                name: "path",
                ty: "asset path",
                required: true,
            },
            ArgSpec {
                name: "sceneId",
                ty: "id",
                required: false,
            },
        ],
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
        name: "set-import-role",
        cmd: "set_import_role",
        aliases: &["set_import_role"],
        summary: "What an asset imports as (a PNG can be a heightmap, cookie or volume strip); auto follows the extension.",
        args: &[
            ArgSpec {
                name: "path",
                ty: "asset path",
                required: true,
            },
            ArgSpec {
                name: "role",
                ty: "auto|texture|hdr|volume|heightmap|ies|cookie",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "set-exposure-bias",
        cmd: "set_exposure_bias",
        aliases: &["set_exposure_bias"],
        summary: "Scale an HDR image by some stops (ev, -16 to 16) wherever it is decoded.",
        args: &[
            ArgSpec {
                name: "path",
                ty: "asset path",
                required: true,
            },
            ArgSpec {
                name: "ev",
                ty: "number",
                required: true,
            },
        ],
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
        summary: "Lay images into one atlas sheet; with output, bake it to <output>.png and .json.",
        args: &[
            ArgSpec {
                name: "paths",
                ty: "[source paths]",
                required: true,
            },
            ArgSpec {
                name: "output",
                ty: "asset path",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "export-shader",
        cmd: "export_shader",
        aliases: &["export_shader"],
        summary: "Write an actor's custom effect out as a .wesl asset and draw with that file from now on.",
        args: &[
            A,
            ArgSpec {
                name: "path",
                ty: "asset path",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "check-shader",
        cmd: "check_shader",
        aliases: &["check_shader"],
        summary: "Check the .wesl file an actor's effect draws with, and log the verdict.",
        args: &[A],
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
        name: "step-project",
        cmd: "step_project",
        aliases: &["step_project", "step"],
        summary: "Advance a paused run by one fixed tick.",
        args: &[],
    },
    CommandSpec {
        name: "set-preview-enabled",
        cmd: "set_preview_enabled",
        aliases: &["set_preview_enabled", "preview"],
        summary: "Turn the embedded preview sidecar on or off.",
        args: &[ArgSpec {
            name: "enabled",
            ty: "bool",
            required: true,
        }],
    },
    CommandSpec {
        name: "set-preview-headless",
        cmd: "set_preview_headless",
        aliases: &["set_preview_headless", "preview-headless"],
        summary: "Hide the game window while the preview stream runs.",
        args: &[ArgSpec {
            name: "headless",
            ty: "bool",
            required: true,
        }],
    },
    CommandSpec {
        name: "preview-input",
        cmd: "preview_input",
        aliases: &["preview_input"],
        summary: "Forward one viewport input event to the run.",
        args: &[ArgSpec {
            name: "input",
            ty: "object {\"kind\": ...}",
            required: true,
        }],
    },
    CommandSpec {
        name: "open-world",
        cmd: "open_world",
        aliases: &["open_world", "scene"],
        summary: "Bring up the world with the project loaded but not running, for the scene view.",
        args: &[],
    },
    CommandSpec {
        name: "set-scene-view",
        cmd: "set_scene_view",
        aliases: &["set_scene_view"],
        summary: "How the scene view edits: {enabled, tool: move|rotate|scale, local, snap, grid, angle, scale, show_grid, debug_view: lit|false_color|clipping|histogram|waveform|calibration|hdr_preview, volumes: {bounds, heatmap, freeze}}.",
        args: &[ArgSpec {
            name: "view",
            ty: "object {\"tool\": ...}",
            required: true,
        }],
    },
    CommandSpec {
        name: "frame-selected",
        cmd: "frame_selected",
        aliases: &["frame_selected", "frame"],
        summary: "Point the scene view's camera at the selected actor.",
        args: &[],
    },
    CommandSpec {
        name: "capture-exr",
        cmd: "capture_exr",
        aliases: &["capture_exr", "exr"],
        summary: "Save the Game view's next frame, linear and before tonemapping, as an OpenEXR file under the project's screenshots/. Returns its path.",
        args: &[],
    },
    CommandSpec {
        name: "bake-probes",
        cmd: "bake_probes",
        aliases: &["bake_probes"],
        summary: "Bake light probes (reflection cubemaps, irradiance grids) from where they stand into .blockloom/probes. Empty actors bakes every probe. Needs the world open; each bake says so in the run log.",
        args: &[ArgSpec {
            name: "actors",
            ty: "[actor ids]",
            required: false,
        }],
    },
    CommandSpec {
        name: "probe-status",
        cmd: "probe_status",
        aliases: &["probe_status"],
        summary: "Every light probe's bake: whether one is on disk and whether the probe or the scene around it changed since.",
        args: &[],
    },
    CommandSpec {
        name: "paint-tiles",
        cmd: "paint_tiles",
        aliases: &["paint_tiles", "tile-stroke"],
        summary: "Run one tile brush stroke on a tilemap, as one undo step. brush is {\"tool\": paint, erase, fill, line, rect or scatter, \"tiles\": [sheet indices], \"autotile\": set name, \"size\", \"density\", \"jitter\", \"seed\"}; segments are [x0, y0, x1, y1] grid cells (y down from the top-left), run in order. Answers how many cells changed.",
        args: &[
            A,
            ArgSpec {
                name: "brush",
                ty: "object {\"tool\", \"tiles\", ...}",
                required: true,
            },
            ArgSpec {
                name: "segments",
                ty: "[[x0, y0, x1, y1]]",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "import-tileset",
        cmd: "import_tileset",
        aliases: &["import_tileset"],
        summary: "Take a Tiled JSON tileset (.tsj/.json asset) onto a tilemap: image, tile size, collision and passable tiles, animations, `region` tiles and edge or mixed wang sets as autotiles. Painted cells stay. Answers what didn't come across.",
        args: &[
            A,
            ArgSpec {
                name: "path",
                ty: "asset path",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "tilemap-stats",
        cmd: "tilemap_stats",
        aliases: &["tilemap_stats"],
        summary: "A tilemap's tiles, draw batches, colliding rectangles, animated and region tiles.",
        args: &[A],
    },
    CommandSpec {
        name: "add-autotile",
        cmd: "add_autotile",
        aliases: &["add_autotile"],
        summary: "Add (or replace) a tilemap's autotile set laid out as consecutive sheet cells from `first`: 16 for Edge (mask N=1 E=2 S=4 W=8 added to first), 47 for Blob (edges and corners, ascending mask order).",
        args: &[
            A,
            ArgSpec {
                name: "name",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "mode",
                ty: "Edge|Blob",
                required: true,
            },
            ArgSpec {
                name: "first",
                ty: "number",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "paint-terrain",
        cmd: "paint_terrain",
        aliases: &["paint_terrain", "sculpt-terrain"],
        summary: "Apply one brush stroke to a terrain, as one undo step. stroke is {\"brush\": {\"op\": Raise, Lower, Smooth, Flatten, Noise, Terrace, Paint or Erase, \"target\": {\"kind\": \"Heights\"} or {\"kind\": \"Layer\", \"layer\": 1} (also Holes, Grass, Scatter), \"radius\", \"strength\", \"falloff\"}, \"stamps\": [[x, z], ...]} in metres from the terrain's centre.",
        args: &[
            A,
            ArgSpec {
                name: "stroke",
                ty: "object {\"brush\", \"stamps\"}",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "import-terrain-heightmap",
        cmd: "import_terrain_heightmap",
        aliases: &["import_terrain_heightmap"],
        summary: "Replace a terrain's heights with a heightmap asset (16-bit PNG, .r16/.r32 RAW, or an image marked as a heightmap), stretched to its resolution.",
        args: &[
            A,
            ArgSpec {
                name: "path",
                ty: "asset path",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "erode-terrain",
        cmd: "erode_terrain",
        aliases: &["erode_terrain"],
        summary: "Run an erosion filter over a terrain's heights, as one undo step: {\"kind\": \"thermal\", \"iterations\", \"talus\"} or {\"kind\": \"hydraulic\", \"droplets\", \"seed\", \"erosion\", \"deposition\", \"inertia\"}.",
        args: &[
            A,
            ArgSpec {
                name: "erosion",
                ty: "object {\"kind\", ...}",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "preview-terrain-erosion",
        cmd: "preview_terrain_erosion",
        aliases: &["preview_terrain_erosion"],
        summary: "Show an erosion filter on a terrain in the Game view without saving it (same shape as erode-terrain); leave erosion out to put the saved ground back.",
        args: &[
            A,
            ArgSpec {
                name: "erosion",
                ty: "object {\"kind\", ...}",
                required: false,
            },
        ],
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
    CommandSpec {
        name: "merge-tail",
        cmd: "merge_tail",
        aliases: &["merge_tail"],
        summary: "Move a stack's tail from a path onto another stack at a path.",
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
                name: "targetId",
                ty: "id",
                required: true,
            },
            ArgSpec {
                name: "targetPath",
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
        name: "create-input-action",
        cmd: "create_input_action",
        aliases: &["create_input_action"],
        summary: "Declare a named input action with no bindings.",
        args: &[ArgSpec {
            name: "name",
            ty: "string",
            required: true,
        }],
    },
    CommandSpec {
        name: "rename-input-action",
        cmd: "rename_input_action",
        aliases: &["rename_input_action"],
        summary: "Rename an input action and every block that names it.",
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
        name: "delete-input-action",
        cmd: "delete_input_action",
        aliases: &["delete_input_action"],
        summary: "Delete an input action. Blocks naming it read as unheld.",
        args: &[ArgSpec {
            name: "name",
            ty: "string",
            required: true,
        }],
    },
    CommandSpec {
        name: "add-input-binding",
        cmd: "add_input_binding",
        aliases: &["add_input_binding"],
        summary: "Add one binding (space, mouse:left, gamepad:south) to an action.",
        args: &[
            ArgSpec {
                name: "name",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "binding",
                ty: "string",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "remove-input-binding",
        cmd: "remove_input_binding",
        aliases: &["remove_input_binding"],
        summary: "Remove one binding from an action.",
        args: &[
            ArgSpec {
                name: "name",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "binding",
                ty: "string",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "clear-input-bindings",
        cmd: "clear_input_bindings",
        aliases: &["clear_input_bindings"],
        summary: "Forget every binding an action has.",
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
    // ── Plugins ───────────────────────────────────────────────────────────
    CommandSpec {
        name: "plugin-list",
        cmd: "plugin_list",
        aliases: &["plugins"],
        summary: "The open project's plugins: what is installed, what failed to load, direct dependencies and the dependency tree.",
        args: &[],
    },
    CommandSpec {
        name: "plugin-check",
        cmd: "plugin_check",
        aliases: &[],
        summary: "Every plugin record that needs attention (missing plugin, unknown type, newer or older schema, invalid payload) and whether the project may run.",
        args: &[],
    },
    CommandSpec {
        name: "plugin-diagnostics",
        cmd: "plugin_diagnostics",
        aliases: &[],
        summary: "What plugin calls cost and report: per-op call counts and timings, counters, gauges, markers and recent errors, for the editor's modules and for the running world.",
        args: &[],
    },
    CommandSpec {
        name: "plugin-commands",
        cmd: "plugin_commands",
        aliases: &[],
        summary: "The commands installed plugins contribute, as plugin-id/name with their arguments. Run one as a shell command by that name, or through plugin-call.",
        args: &[],
    },
    CommandSpec {
        name: "plugin-install",
        cmd: "plugin_install",
        aliases: &["plugin-add"],
        summary: "Install a plugin and its dependencies into the project. The whole graph is resolved first and nothing changes if it fails. Sources: path:<dir>, archive:<zip>, git:<url>#<commit>, registry:<name>; none means the project's registries.",
        args: &[
            ArgSpec {
                name: "id",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "version",
                ty: "string",
                required: false,
            },
            ArgSpec {
                name: "source",
                ty: "string",
                required: false,
            },
            ArgSpec {
                name: "features",
                ty: "[plugin ids]",
                required: false,
            },
            ArgSpec {
                name: "dryRun",
                ty: "bool",
                required: false,
            },
            ArgSpec {
                name: "offline",
                ty: "bool",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "plugin-remove",
        cmd: "plugin_remove",
        aliases: &[],
        summary: "Remove a direct plugin dependency. Records the plugin owned stay in the project; the answer lists them.",
        args: &[
            ArgSpec {
                name: "id",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "dryRun",
                ty: "bool",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "plugin-update",
        cmd: "plugin_update",
        aliases: &[],
        summary: "Move plugins (all when ids is empty) to the newest versions their requirements allow. Nothing else ever changes a locked version.",
        args: &[
            ArgSpec {
                name: "ids",
                ty: "[plugin ids]",
                required: false,
            },
            ArgSpec {
                name: "dryRun",
                ty: "bool",
                required: false,
            },
            ArgSpec {
                name: "offline",
                ty: "bool",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "plugin-pin",
        cmd: "plugin_pin",
        aliases: &[],
        summary: "Hold a direct dependency at exactly one version.",
        args: &[
            ArgSpec {
                name: "id",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "version",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "offline",
                ty: "bool",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "plugin-sync",
        cmd: "plugin_sync",
        aliases: &[],
        summary: "Install exactly what plugins.lock says, fetching what the cache lacks and verifying every hash. Never upgrades.",
        args: &[
            ArgSpec {
                name: "dryRun",
                ty: "bool",
                required: false,
            },
            ArgSpec {
                name: "offline",
                ty: "bool",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "plugin-rollback",
        cmd: "plugin_rollback",
        aliases: &[],
        summary: "Undo the last plugin install, update or removal, restoring plugins.json and plugins.lock. Works offline.",
        args: &[],
    },
    CommandSpec {
        name: "plugin-registry",
        cmd: "plugin_registry",
        aliases: &[],
        summary: "Add a registry to plugins.json under a name: a folder (relative to the project) or an https:// URL serving a published registry folder (index.json and archives/).",
        args: &[
            ArgSpec {
                name: "name",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "path",
                ty: "string",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "plugin-data-gc",
        cmd: "plugin_data_gc",
        aliases: &[],
        summary: "Remove content blobs (storage put/get) that no plugin record, stored key or other blob names, from the project's plugin data and saves. dryRun=true only reports. Refused while a game runs.",
        args: &[ArgSpec {
            name: "dryRun",
            ty: "bool",
            required: false,
        }],
    },
    CommandSpec {
        name: "plugin-gc",
        cmd: "plugin_gc",
        aliases: &[],
        summary: "Delete cached packages no project on the Dashboard (or its plugin history) still needs.",
        args: &[],
    },
    CommandSpec {
        name: "plugin-migrate",
        cmd: "plugin_migrate",
        aliases: &[],
        summary: "Upgrade every record of a plugin to its schema's current version. All records or none; a snapshot of the old ones is kept.",
        args: &[
            ArgSpec {
                name: "id",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "dryRun",
                ty: "bool",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "plugin-call",
        cmd: "plugin_call",
        aliases: &[],
        summary: "Run a command a plugin contributes, by plugin-id/name, with its arguments as an object.",
        args: &[
            ArgSpec {
                name: "command",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "args",
                ty: "object {...}",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "plugin-importers",
        cmd: "plugin_importers",
        aliases: &[],
        summary: "List the importers and build hooks installed plugins add, and how each imported file stands against its source.",
        args: &[],
    },
    CommandSpec {
        name: "plugin-imports",
        cmd: "plugin_imports",
        aliases: &[],
        summary: "Every remembered import and whether it is current (fresh, source changed, dependency changed, output missing or edited, source missing).",
        args: &[],
    },
    CommandSpec {
        name: "plugin-import",
        cmd: "plugin_import",
        aliases: &[],
        summary: "Run a plugin's importer over a project file under assets/ and write what it makes to <file>.imported/. Replaces what the last import of that file wrote.",
        args: &[
            ArgSpec {
                name: "path",
                ty: "asset path",
                required: true,
            },
            ArgSpec {
                name: "importer",
                ty: "string",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "plugin-reimport",
        cmd: "plugin_reimport",
        aliases: &[],
        summary: "Import again every file whose source, dependency or output changed since its last import (Play and Build do this too). With a path, imports that one file again even if its output was edited by hand.",
        args: &[ArgSpec {
            name: "path",
            ty: "asset path",
            required: false,
        }],
    },
    CommandSpec {
        name: "plugin-run-block",
        cmd: "plugin_run_block",
        aliases: &[],
        summary: "Run a plugin block as a game strand would: its command with the slot values in the schema's slot order. The actor is who a command that wants one defaults to.",
        args: &[
            ArgSpec {
                name: "plugin",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "block",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "args",
                ty: "[slot values]",
                required: false,
            },
            ArgSpec {
                name: "actor",
                ty: "id",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "plugin-run-tool",
        cmd: "plugin_run_tool",
        aliases: &[],
        summary: "Run a plugin's scene-view tool as a click or a stroke would: the tool's command with its arguments read from `hit` (the cast's answer), or from each of `hits`, and `options`. A stroke is one undo step.",
        args: &[
            ArgSpec {
                name: "plugin",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "tool",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "hit",
                ty: "object",
                required: false,
            },
            ArgSpec {
                name: "hits",
                ty: "[{cast answers}]",
                required: false,
            },
            ArgSpec {
                name: "options",
                ty: "object",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "plugin-inspect",
        cmd: "plugin_inspect",
        aliases: &[],
        summary: "Verify a package folder and say what it is (id, version, tier, capabilities, what it contributes) without installing it.",
        args: &[ArgSpec {
            name: "path",
            ty: "folder path",
            required: true,
        }],
    },
    CommandSpec {
        name: "plugin-new",
        cmd: "plugin_new",
        aliases: &[],
        summary: "Author tool: write a starter package folder (template declarative, sealed and installable; portable, a Rust crate with a test harness and build.sh; or native, the same crate plus native libraries).",
        args: &[
            ArgSpec {
                name: "path",
                ty: "folder path",
                required: true,
            },
            ArgSpec {
                name: "id",
                ty: "id",
                required: true,
            },
            ArgSpec {
                name: "name",
                ty: "string",
                required: false,
            },
            ArgSpec {
                name: "template",
                ty: "declarative | portable | native",
                required: false,
            },
            ArgSpec {
                name: "sdk",
                ty: "folder path",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "plugin-add-native",
        cmd: "plugin_add_native",
        aliases: &[],
        summary: "Author tool: record a built native library (a file in the package) under a target triple in a package folder's plugin.json, mark it native and seal it.",
        args: &[
            ArgSpec {
                name: "path",
                ty: "folder path",
                required: true,
            },
            ArgSpec {
                name: "target",
                ty: "string",
                required: true,
            },
            ArgSpec {
                name: "library",
                ty: "file path",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "plugin-seal",
        cmd: "plugin_seal",
        aliases: &[],
        summary: "Author tool: write the sha256 of every file into a package folder's plugin.json and verify the package.",
        args: &[ArgSpec {
            name: "path",
            ty: "folder path",
            required: true,
        }],
    },
    CommandSpec {
        name: "plugin-publish",
        cmd: "plugin_publish",
        aliases: &[],
        summary: "Author tool: publish a sealed package folder to a folder registry. A published version is immutable.",
        args: &[
            ArgSpec {
                name: "path",
                ty: "folder path",
                required: true,
            },
            ArgSpec {
                name: "registry",
                ty: "folder path",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "add-plugin-component",
        cmd: "add_plugin_component",
        aliases: &[],
        summary: "Add a plugin's component (plugin-id/Type) to an actor, validated against the plugin's schema and starting from its defaults.",
        args: &[
            A,
            ArgSpec {
                name: "component",
                ty: "component name",
                required: true,
            },
            ArgSpec {
                name: "payload",
                ty: "object {...}",
                required: false,
            },
        ],
    },
    CommandSpec {
        name: "set-plugin-component",
        cmd: "set_plugin_component",
        aliases: &[],
        summary: "Replace the payload of a plugin component an actor already has.",
        args: &[
            A,
            ArgSpec {
                name: "component",
                ty: "component name",
                required: true,
            },
            ArgSpec {
                name: "payload",
                ty: "object {...}",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "set-plugin-resource",
        cmd: "set_plugin_resource",
        aliases: &[],
        summary: "Set a project resource a plugin owns (plugin-id/Type).",
        args: &[
            ArgSpec {
                name: "resource",
                ty: "component name",
                required: true,
            },
            ArgSpec {
                name: "payload",
                ty: "object {...}",
                required: true,
            },
        ],
    },
    CommandSpec {
        name: "remove-plugin-resource",
        cmd: "remove_plugin_resource",
        aliases: &[],
        summary: "Remove a project resource a plugin owns.",
        args: &[ArgSpec {
            name: "resource",
            ty: "component name",
            required: true,
        }],
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
    /// Run a command an installed plugin contributes, named `plugin-id/name`.
    /// Its arguments are checked against the plugin's own schema.
    Plugin {
        command: String,
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
    if name.contains('/') {
        return Ok(Some(Action::Plugin {
            command: name.to_string(),
            args: parse_args(rest)?,
        }));
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
        Action::Plugin { command, args } => {
            match backend.dispatch("plugin_call", plugin_call_args(command, args)) {
                Ok(result) => {
                    json!({"ok": true, "result": result, "error": null, "state": state()})
                }
                Err(error) => {
                    json!({"ok": false, "result": null, "error": error, "state": state()})
                }
            }
        }
    }
}

/// Arguments of a `plugin_call` made from a `plugin-id/name` shell line.
fn plugin_call_args(command: String, args: Map<String, Value>) -> Value {
    json!({"command": command, "args": Value::Object(args)})
}

/// Runs one shell line against an attached editor and returns the same JSON
/// response object as [`run`]. Blank lines, comments and `help` are answered
/// locally; every real command goes through `forward`, which should send
/// `(cmd, args)` down the attach socket and answer the server's whole
/// response object.
pub fn run_forwarded(
    forward: &mut dyn FnMut(&str, Value) -> Result<Value, String>,
    line: &str,
    with_state: bool,
) -> Value {
    let state = |forward: &mut dyn FnMut(&str, Value) -> Result<Value, String>| {
        if !with_state {
            return Value::Null;
        }
        match forward("get_state", json!({})) {
            Ok(response) => response.get("state").cloned().unwrap_or(Value::Null),
            Err(_) => Value::Null,
        }
    };
    let action = match parse(line) {
        Ok(Some(action)) => action,
        Ok(None) => {
            return json!({"ok": true, "result": null, "error": null, "state": state(forward)});
        }
        Err(error) => {
            return json!({"ok": false, "result": null, "error": error, "state": state(forward)});
        }
    };
    match action {
        Action::Exit => {
            json!({"ok": true, "result": null, "error": null, "state": Value::Null})
        }
        Action::Help(topic) => {
            let result = match topic {
                None => Value::String(help()),
                Some(topic) => match help_for(&topic) {
                    Ok(text) => Value::String(text),
                    Err(error) => {
                        return json!({"ok": false, "result": null, "error": error, "state":
                            state(forward)});
                    }
                },
            };
            json!({"ok": true, "result": result, "error": null, "state": state(forward)})
        }
        action @ (Action::Run { .. } | Action::Plugin { .. }) => {
            let (cmd, args) = match action {
                Action::Run { spec, args } => (spec.cmd, Value::Object(args)),
                Action::Plugin { command, args } => {
                    ("plugin_call", plugin_call_args(command, args))
                }
                _ => unreachable!(),
            };
            match forward(cmd, args) {
                Ok(response) => {
                    let state = if with_state {
                        response.get("state").cloned().unwrap_or(Value::Null)
                    } else {
                        Value::Null
                    };
                    json!({
                        "ok": response.get("ok").cloned().unwrap_or(Value::Bool(false)),
                        "result": response.get("result").cloned().unwrap_or(Value::Null),
                        "error": response.get("error").cloned().unwrap_or(Value::Null),
                        "state": state,
                    })
                }
                Err(error) => {
                    json!({"ok": false, "result": null, "error": error, "state": state(forward)})
                }
            }
        }
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
    fn forwarded_commands_keep_the_response_shape() {
        let mut forward = |cmd: &str, _args: Value| -> Result<Value, String> {
            assert_eq!(cmd, "add_actor");
            Ok(json!({
                "ok": true,
                "result": "a1",
                "error": null,
                "state": {"project": null},
            }))
        };
        let response = run_forwarded(&mut forward, "add-actor shape=Circle", true);
        assert_eq!(response["ok"], Value::Bool(true));
        assert_eq!(response["result"], "a1");
        assert_eq!(response["state"]["project"], Value::Null);
    }

    #[test]
    fn forwarded_help_is_answered_locally() {
        let mut forward = |_: &str, _: Value| -> Result<Value, String> {
            panic!("help must not reach the editor");
        };
        let response = run_forwarded(&mut forward, "help sync-status", false);
        assert_eq!(response["ok"], Value::Bool(true));
        assert!(response["result"].as_str().unwrap().contains("sync-status"));
    }

    #[test]
    fn a_dead_attach_server_errors_like_a_failed_command() {
        let mut forward = |_: &str, _: Value| -> Result<Value, String> {
            Err("The editor closed the attach connection".to_string())
        };
        let response = run_forwarded(&mut forward, "get-state", false);
        assert_eq!(response["ok"], Value::Bool(false));
        assert!(response["error"].as_str().unwrap().contains("attach"));
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
    fn a_slash_names_a_plugin_command() {
        let Ok(Some(Action::Plugin { command, args })) =
            parse("com.example.health/set_hp actor=a value=3")
        else {
            panic!("not a plugin command");
        };
        assert_eq!(command, "com.example.health/set_hp");
        assert_eq!(args["value"], 3);
        assert_eq!(
            plugin_call_args(command, args)["args"]["actor"],
            Value::String("a".into())
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
