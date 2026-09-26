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
pub const PROTOCOL_VERSION: u32 = 16;

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
    /// Saves the world camera's next frame, linear and before tonemapping,
    /// as an OpenEXR file at `path`. Answered with a `say` or an `error`.
    CaptureExr {
        path: String,
    },
    /// Bakes these actors' light probes from where they stand and writes
    /// them under the project's `.blockloom/probes`; empty bakes every one.
    /// Each bake is announced with a `say`.
    BakeProbes {
        #[serde(default)]
        actors: Vec<String>,
    },
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
    /// world ignores. `volume` is set when a volume's handles resized it.
    Placed {
        actor: String,
        placement: Placement,
        #[serde(default)]
        offset: Option<[f32; 3]>,
        #[serde(default)]
        volume: Option<VolumeBounds>,
    },
    /// What ray tracing can do and is doing, whenever that changes, and a
    /// few times a second while the path tracer converges.
    RayTracing(RayTracingStatus),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RayTracingStatus {
    /// Whether this GPU and build can trace rays.
    pub available: bool,
    /// Why not, when not.
    pub reason: String,
    /// Realtime ray-traced lighting is on the world camera.
    pub active: bool,
    /// The reference path tracer is drawing the view.
    pub path_tracing: bool,
    /// Samples per pixel since the path tracer's image last started over.
    pub samples: u32,
    pub seconds: f32,
    /// The path tracer met its sample or time budget.
    pub converged: bool,
}

/// A volume's size as its scene view handles left it, before the actor's
/// scale.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VolumeBounds {
    pub half_extents: [f32; 3],
    pub radius: f32,
    pub blend_distance: f32,
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
    /// Stripes over whatever is brighter than the display can show.
    Clipping,
    /// The lit image with a luminance histogram over its corner.
    Histogram,
    /// The lit image with a luminance waveform over its lower part.
    Waveform,
    /// Test patches at known levels, for setting peak brightness and paper
    /// white against the display.
    Calibration,
    /// The project's HDR output as an HDR display would show it, paper white
    /// at SDR white and anything brighter clipped.
    HdrPreview,
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
    pub volumes: VolumeDebug,
    pub path_tracer: PathTracerView,
}

/// The reference path tracer in place of the lit image, for checking a
/// scene's lighting against ground truth. Not for gameplay.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PathTracerView {
    pub enabled: bool,
    /// Samples per pixel that count as converged. The image keeps refining
    /// past it; an EXR capture waits for it.
    pub samples: u32,
    /// Seconds that count as converged too; 0 for no limit.
    pub seconds: f32,
}

impl Default for PathTracerView {
    fn default() -> Self {
        Self {
            enabled: false,
            samples: 256,
            seconds: 60.0,
        }
    }
}

/// The environment volumes' debug views.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VolumeDebug {
    /// Each volume's shape and blend feather, in the scene view.
    pub bounds: bool,
    /// How much volume each pixel's surface is under, drawn over the
    /// frame. Shows in play too.
    pub heatmap: bool,
    /// Holds the blend where it is so it can be inspected: the camera moves
    /// but the look doesn't, and the status carries each property's lerp.
    pub freeze: bool,
}

impl Default for VolumeDebug {
    fn default() -> Self {
        Self {
            bounds: true,
            heatmap: false,
            freeze: false,
        }
    }
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
            volumes: VolumeDebug::default(),
            path_tracer: PathTracerView::default(),
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
    /// The environment volumes showing at the camera, in blend order.
    #[serde(default)]
    pub volumes: Vec<VolumeStatus>,
    /// Each blended property's lerp, only while the blend is frozen.
    #[serde(default)]
    pub volume_trace: Vec<VolumeTraceRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VolumeStatus {
    pub actor: String,
    pub name: String,
    pub priority: f32,
    /// How much of the camera the shape covers, 0-1.
    pub coverage: f32,
    /// What it blends at: coverage times its weight.
    pub weight: f32,
    /// The properties it overrides.
    pub overrides: Vec<String>,
}

/// One property's way from the project's value through every volume that
/// touches it. Values are display text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VolumeTraceRow {
    pub property: String,
    pub base: String,
    pub steps: Vec<VolumeTraceStep>,
    pub result: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VolumeTraceStep {
    pub volume: String,
    pub weight: f32,
    /// What the volume asks for.
    pub target: String,
    /// The value once this volume has blended in.
    pub after: String,
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
            volume: Some(VolumeBounds {
                half_extents: [1.0, 2.0, 3.0],
                radius: 4.0,
                blend_distance: 0.5,
            }),
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
