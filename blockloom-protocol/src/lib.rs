//! The wire format between the editor and the game world.
//!
//! The world runs in its own process (`blockloom-runtime`) because Bevy needs
//! its own window and event loop, which the editor's CEF runtime already owns.
//! The editor spawns it as a child and the two talk newline-delimited JSON over
//! its stdin and stdout - no sockets, no ports, and the pipe closing is all the
//! shutdown handshake either side needs.

pub mod keys;

use blockloom_core::project::Project;
use blockloom_core::scene::Placement;
use blockloom_core::value::Evaluated;
use serde::{Deserialize, Serialize};

/// Bumped when a message changes shape. The runtime reports the version it
/// was built with in [`RuntimeMessage::Ready`]; a mismatch means a stale
/// binary next to a fresh editor.
pub const PROTOCOL_VERSION: u32 = 11;

/// The size a game's window opens at, in pixels - and so the size the
/// editor's Game view draws it at, scaled to fit, so it shows exactly what a
/// player would see.
pub const GAME_SIZE: (u32, u32) = (960, 720);

/// Where the embedded preview streams: the runtime's MJPEG sidecar, which
/// the editor's viewport reads directly so frames never clog the control
/// pipe. Always loopback.
pub const PREVIEW_MIME: &str = "multipart/x-mixed-replace; boundary=blockloom-frame";

/// One forwarded input event for the embedded preview, in preview-pixel
/// coordinates. The runtime maps it onto its own window before injecting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreviewInput {
    /// Pointer moved. `x`/`y` are in preview pixels, `w`/`h` the preview's
    /// size so the runtime can scale onto its window.
    MouseMove { x: f32, y: f32, w: f32, h: f32 },
    /// Button went down (`down` true) or up. `button` is 0/1/2 for
    /// left/right/middle.
    MouseButton {
        button: u8,
        down: bool,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    /// Physical key went down or up, by Bevy [`KeyCode`](https://docs.rs/bevy/latest/bevy/prelude/struct.KeyCode.html) name (`"KeyW"`, `"Space"`, ...).
    Key { code: String, down: bool },
    /// Wheel or touchpad scroll over the view. `line` says `dx`/`dy` count
    /// notches rather than pixels; positive `dy` scrolls up.
    Scroll {
        dx: f32,
        dy: f32,
        line: bool,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    /// One finger on a touch screen, `id` stable from `start` to `end`.
    Touch {
        id: u64,
        phase: TouchPhase,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    /// Text typed into a focused in-game input while the preview has focus.
    Text { text: String },
    /// Raw pointer motion while the view holds the pointer locked.
    MouseDelta { dx: f32, dy: f32 },
    /// The view gained or lost the keyboard. Once sent, it decides focus.
    Focus { focused: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TouchPhase {
    Start,
    Move,
    End,
    Cancel,
}

/// Editor -> runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum EditorMessage {
    /// Builds (or rebuilds) the world from this project. Any running scripts
    /// are dropped.
    Load {
        project: Box<Project>,
        /// The project's folder, so the runtime can find its assets and the
        /// script libraries the editor built into `.blockloom/build`.
        #[serde(default)]
        dir: Option<String>,
    },
    /// The green flag.
    Start,
    /// Stops every script and puts each actor back where the project says.
    Stop,
    Pause {
        paused: bool,
    },
    /// Advances a paused world by one fixed tick. Ignored while running.
    Step,
    /// Turns the embedded-preview sidecar on or off. When on, the runtime
    /// serves MJPEG on loopback and reports the port with
    /// [`RuntimeMessage::PreviewReady`]; the editor's viewport reads it
    /// directly. Additive: the OS window stays up unless `headless` hides
    /// it, in which case the hidden window keeps rendering the stream.
    /// `headless` defaults to false so older editors still decode.
    Preview {
        enabled: bool,
        #[serde(default)]
        headless: bool,
    },
    /// A pointer or keyboard event from the embedded viewport. While nothing
    /// runs, these drive the scene view instead of the game.
    PreviewInput {
        input: PreviewInput,
    },
    /// How the scene view edits while nothing runs.
    SceneView(SceneView),
    /// The actor the editor has selected, which the scene view outlines.
    Select {
        #[serde(default)]
        actor: Option<String>,
    },
    /// Points the scene view's camera at the selected actor.
    FrameSelected,
    /// Close the window and exit.
    Shutdown,
}

/// Runtime -> editor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum RuntimeMessage {
    /// Sent once, as soon as the window is up.
    Ready { protocol: u32 },
    /// A `say` block, or anything else worth showing in the editor's log.
    Say { actor: String, text: String },
    /// A block failed to evaluate. The script carried on regardless.
    Error { actor: String, message: String },
    /// Where everything is, a few times a second - what the editor's actor
    /// inspector and variable watchers display while a project runs.
    Status(Status),
    /// The play session ended through Stop or a `stop all` block.
    Stopped,
    /// The preview sidecar is serving MJPEG on this loopback port. The
    /// editor's viewport reads `http://127.0.0.1:{port}/preview.mjpg`.
    PreviewReady { port: u16 },
    /// The preview sidecar stopped (turned off or failed to bind).
    PreviewStopped,
    /// The game wants the pointer locked (or free). A windowless world asks
    /// the view to hold it instead.
    PointerLock { locked: bool },
    /// The runtime is giving up (a fatal renderer or physics error).
    Fatal { message: String },
    /// An actor was clicked in the scene view.
    Picked { actor: String },
    /// A scene view drag ended: where the actor now stands. `offset` is set
    /// for a child placed in its parent's frame, whose `Place` position the
    /// world ignores.
    Placed {
        actor: String,
        placement: Placement,
        #[serde(default)]
        offset: Option<[f32; 3]>,
    },
}

/// Which handle the scene view's gizmo shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneTool {
    #[default]
    Move,
    Rotate,
    Scale,
}

/// What the Game view shows in place of the lit image, for judging exposure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DebugView {
    #[default]
    Lit,
    /// Exposed luminance as bands of stops around middle grey.
    FalseColor,
    /// Stripes over whatever is brighter than paper white.
    Clipping,
}

/// The scene view's settings, which are the editor's preferences rather than
/// the project's. Steps are in world units: pixels in 2D, metres in 3D.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SceneView {
    /// Off shows the idle world through the game's own camera.
    pub enabled: bool,
    pub tool: SceneTool,
    /// Move and rotate along the actor's own axes rather than the world's.
    pub local: bool,
    pub snap: bool,
    pub grid: f32,
    /// Degrees.
    pub angle: f32,
    pub scale: f32,
    pub show_grid: bool,
    /// Applies while a game runs too, since exposure is judged in play.
    pub debug_view: DebugView,
}

impl Default for SceneView {
    fn default() -> Self {
        Self {
            enabled: true,
            tool: SceneTool::Move,
            local: false,
            snap: false,
            grid: 1.0,
            angle: 15.0,
            scale: 0.1,
            show_grid: true,
            debug_view: DebugView::Lit,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub running: bool,
    pub paused: bool,
    /// Seconds since the green flag.
    pub time: f64,
    pub fps: f32,
    /// Renderer timings and mesh allocation, sampled with the live status.
    pub render_metrics: Vec<RenderMetric>,
    pub actors: Vec<ActorStatus>,
    /// Project-wide variables, by name.
    pub globals: Vec<VariableValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderMetric {
    pub name: String,
    pub value: f64,
    pub unit: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActorStatus {
    pub id: String,
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub visible: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariableValue {
    pub name: String,
    pub value: Evaluated,
}

/// One message as a line on the wire, newline included.
pub fn encode<T: Serialize>(message: &T) -> String {
    match serde_json::to_string(message) {
        Ok(json) => format!("{json}\n"),
        // A message that can't be serialized is a bug, not a wire error; the
        // other side would rather see a parse failure than a dropped line.
        Err(e) => format!("{{\"event\":\"fatal\",\"message\":\"{e}\"}}\n"),
    }
}

/// Parses one line. Blank lines are ignored by returning `None`.
pub fn decode<T: for<'de> Deserialize<'de>>(line: &str) -> Option<Result<T, String>> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    Some(serde_json::from_str(line).map_err(|e| format!("{e}: {line}")))
}

/// Name of the runtime binary, which every install ships next to the editor.
pub const RUNTIME_BINARY: &str = if cfg!(windows) {
    "blockloom-runtime.exe"
} else {
    "blockloom-runtime"
};

/// The runtime binary that belongs to this build: the one sitting next to the
/// running executable.
pub fn runtime_path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(RUNTIME_BINARY)))
        .unwrap_or_else(|| std::path::PathBuf::from(RUNTIME_BINARY))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_round_trips_as_one_line() {
        let message = RuntimeMessage::Say {
            actor: "a1".to_string(),
            text: "hello".to_string(),
        };
        let line = encode(&message);
        assert!(line.ends_with('\n'));
        assert!(!line[..line.len() - 1].contains('\n'));
        assert_eq!(decode::<RuntimeMessage>(&line), Some(Ok(message)));
    }

    #[test]
    fn a_blank_line_is_not_an_error() {
        assert_eq!(decode::<EditorMessage>("  \n"), None);
        assert!(matches!(decode::<EditorMessage>("{oops}"), Some(Err(_))));
    }

    #[test]
    fn preview_messages_round_trip_as_one_line() {
        let input = EditorMessage::PreviewInput {
            input: PreviewInput::MouseButton {
                button: 0,
                down: true,
                x: 10.0,
                y: 20.0,
                w: 480.0,
                h: 270.0,
            },
        };
        let line = encode(&input);
        assert_eq!(decode::<EditorMessage>(&line), Some(Ok(input)));
        let ready = RuntimeMessage::PreviewReady { port: 4129 };
        let line = encode(&ready);
        assert_eq!(decode::<RuntimeMessage>(&line), Some(Ok(ready)));
    }

    #[test]
    fn scene_messages_round_trip_as_one_line() {
        let view = EditorMessage::SceneView(SceneView {
            tool: SceneTool::Rotate,
            snap: true,
            debug_view: DebugView::FalseColor,
            ..SceneView::default()
        });
        assert_eq!(decode::<EditorMessage>(&encode(&view)), Some(Ok(view)));
        let placed = RuntimeMessage::Placed {
            actor: "a1".to_string(),
            placement: Placement::default(),
            offset: Some([1.0, 2.0, 3.0]),
        };
        assert_eq!(decode::<RuntimeMessage>(&encode(&placed)), Some(Ok(placed)));
    }

    #[test]
    fn preview_without_headless_defaults_to_windowed() {
        // Editors from before the headless flag send no `headless` key.
        let line = "{\"cmd\":\"preview\",\"enabled\":true}\n";
        assert_eq!(
            decode::<EditorMessage>(line),
            Some(Ok(EditorMessage::Preview {
                enabled: true,
                headless: false,
            }))
        );
    }
}
