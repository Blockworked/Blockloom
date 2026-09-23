//! The embedded-preview sidecar: an MJPEG server on loopback plus the frame
//! capture and input injection behind it.
//!
//! The editor's viewport reads `http://127.0.0.1:{port}/preview.mjpg`
//! directly, so video never clogs the stdin/stdout control pipe. Control
//! (on/off, resize, input, step) still travels as [`EditorMessage`]s, and the
//! port travels back as [`RuntimeMessage::PreviewReady`]. Windowed mode keeps
//! the OS window up beside the viewport; headless mode hides it and the
//! hidden window keeps rendering the same stream.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use blockloom_protocol::PreviewInput;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How often a frame is captured while previewing.
const CAPTURE_INTERVAL: Duration = Duration::from_millis(66);
/// JPEG quality for preview frames: small enough to stream, clear enough to
/// read the game. Keeps the pipe at a few hundred KB/s.
const JPEG_QUALITY: u8 = 60;

/// The latest encoded frame, shared with the HTTP thread.
#[derive(Resource, Clone, Default)]
pub struct LatestFrame(pub Arc<Mutex<Option<Vec<u8>>>>);

/// Preview streamer state on the main world.
#[derive(Resource)]
pub struct PreviewState {
    pub enabled: bool,
    /// Hides the OS window while the stream keeps rendering it. The window
    /// stays hidden only while the sidecar is enabled; turning preview off
    /// shows it again.
    pub headless: bool,
    pub port: Option<u16>,
    pub next_capture: Instant,
    pub frames: LatestFrame,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self {
            enabled: false,
            headless: false,
            port: None,
            next_capture: Instant::now(),
            frames: LatestFrame::default(),
        }
    }
}

/// Pointer forwarded from the embedded viewport, in window pixels. Freshness
/// is tracked so a stale position never steers the game after the user looks
/// away.
#[derive(Resource, Default)]
pub struct PreviewPointer {
    pub pos: Option<Vec2>,
    pub seen: Option<Instant>,
    /// Movement since `publish_sensors` last consumed it, for the mouse-delta
    /// reporter. Drained once a frame.
    pub delta: Vec2,
}

/// Buttons held from the embedded viewport. Applied onto Bevy's own
/// [`ButtonInput`]s each frame, so downstream systems read one truth.
#[derive(Resource, Default)]
pub struct PreviewButtons {
    pub left: bool,
    pub right: bool,
    pub middle: bool,
    /// A left press that hasn't been consumed by `detect_clicks` yet. Edges
    /// are levelled by the round trip, so this rebuilds the edge.
    pub left_edge: bool,
}

/// Key codes held from the embedded viewport, by Bevy [`KeyCode`] name.
#[derive(Resource, Default)]
pub struct PreviewKeys {
    pub held: std::collections::HashSet<String>,
}

/// Starts the sidecar: binds loopback on an ephemeral port, serves MJPEG on a
/// thread, and answers the port. Idempotent while already serving.
pub fn start_preview(state: &mut PreviewState) -> Option<u16> {
    if state.enabled && state.port.is_some() {
        return state.port;
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").ok()?;
    let port = listener.local_addr().ok()?.port();
    listener.set_nonblocking(false).ok()?;
    state.enabled = true;
    state.port = Some(port);
    state.next_capture = Instant::now();
    let frames = state.frames.clone();
    std::thread::Builder::new()
        .name("preview-http".to_string())
        .spawn(move || serve(listener, frames))
        .ok()?;
    Some(port)
}

/// Stops advertising the sidecar. The serving thread ends on its own once its
/// listener is dropped; frames simply stop refreshing.
pub fn stop_preview(state: &mut PreviewState) {
    state.enabled = false;
    state.headless = false;
    state.port = None;
}

/// Minimal HTTP server: `GET /preview.mjpg` streams multipart JPEG,
/// `GET /frame.jpg` serves one frame. Single-threaded and blocking per
/// connection; the editor holds at most one stream plus polls.
fn serve(listener: std::net::TcpListener, frames: LatestFrame) {
    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(stream) => stream,
            Err(_) => continue,
        };
        let mut head = [0u8; 1024];
        let Ok(n) = std::io::Read::read(&mut stream, &mut head) else {
            continue;
        };
        let head = String::from_utf8_lossy(&head[..n]);
        let first = head.lines().next().unwrap_or_default();
        if first.starts_with("GET /frame.jpg") {
            serve_single(&mut stream, &frames);
        } else {
            serve_stream(&mut stream, &frames);
        }
    }
}

fn serve_single(stream: &mut std::net::TcpStream, frames: &LatestFrame) {
    let body = frames.0.lock().ok().and_then(|guard| (*guard).clone());
    match body {
        Some(jpeg) => {
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                jpeg.len()
            );
            let _ = stream.write_all(&jpeg);
        }
        None => {
            let _ = stream.write_all(
                b"HTTP/1.1 503 No frame yet\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    }
}

fn serve_stream(stream: &mut std::net::TcpStream, frames: &LatestFrame) {
    let _ = stream.write_all(
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
            blockloom_protocol::PREVIEW_MIME
        )
        .as_bytes(),
    );
    let mut last_len = 0usize;
    let mut last_hash = 0u64;
    loop {
        let jpeg: Option<Vec<u8>> = frames.0.lock().ok().and_then(|guard| (*guard).clone());
        if let Some(jpeg) = jpeg {
            // Skip re-sending an identical frame: a paused world is still.
            let hash = hash_bytes(&jpeg);
            if jpeg.len() != last_len || hash != last_hash {
                last_len = jpeg.len();
                last_hash = hash;
                if write_frame(stream, &jpeg).is_err() {
                    return;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(66));
    }
}

fn write_frame(stream: &mut std::net::TcpStream, jpeg: &[u8]) -> std::io::Result<()> {
    stream.write_all(
        format!(
            "--blockloom-frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
            jpeg.len()
        )
        .as_bytes(),
    )?;
    stream.write_all(jpeg)?;
    stream.write_all(b"\r\n")?;
    stream.flush()
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    // FNV-1a over a sample: full frames are ~50KB, and equality only skips a
    // redundant send, so a cheap hash is fine.
    let mut hash: u64 = 0xcbf29ce484222325;
    for (i, &b) in bytes.iter().enumerate() {
        if i % 7 == 0 {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    hash
}

/// Whether a frame is due. Throttled so capture costs ~15fps, not every frame.
pub fn capture_due(state: &PreviewState) -> bool {
    state.enabled && Instant::now() >= state.next_capture
}

/// Records one captured screenshot as the latest JPEG frame.
pub fn publish_frame(state: &mut PreviewState, jpeg: Vec<u8>) {
    if let Ok(mut guard) = state.frames.0.lock() {
        *guard = Some(jpeg);
    }
    state.next_capture = Instant::now() + CAPTURE_INTERVAL;
}

/// Encodes a Bevy image as JPEG for the stream.
pub fn encode_jpeg(image: &Image) -> Option<Vec<u8>> {
    let dynamic = image.clone().try_into_dynamic().ok()?;
    let rgb = dynamic.to_rgb8();
    let (width, height) = (rgb.width(), rgb.height());
    let mut out = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY);
    use image::ImageEncoder;
    encoder
        .write_image(rgb.as_raw(), width, height, image::ExtendedColorType::Rgb8)
        .ok()?;
    Some(out)
}

/// Folds one forwarded input event into the preview input resources. Window
/// scaling maps preview pixels onto window pixels.
pub fn apply_input(
    input: &PreviewInput,
    pointer: &mut PreviewPointer,
    buttons: &mut PreviewButtons,
    keys: &mut PreviewKeys,
    window_size: Vec2,
) {
    let now = Instant::now();
    match input {
        PreviewInput::MouseMove { x, y, w, h } => {
            move_pointer(pointer, *x, *y, *w, *h, window_size, now);
        }
        PreviewInput::MouseButton {
            button,
            down,
            x,
            y,
            w,
            h,
        } => {
            move_pointer(pointer, *x, *y, *w, *h, window_size, now);
            match button {
                0 => {
                    if *down && !buttons.left {
                        buttons.left_edge = true;
                    }
                    buttons.left = *down;
                }
                1 => buttons.right = *down,
                2 => buttons.middle = *down,
                _ => {}
            }
        }
        PreviewInput::Key { code, down } => {
            if *down {
                keys.held.insert(code.clone());
            } else {
                keys.held.remove(code);
            }
        }
        PreviewInput::Text { .. } => {}
    }
}

fn move_pointer(
    pointer: &mut PreviewPointer,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    window: Vec2,
    now: Instant,
) {
    let next = preview_to_window(x, y, w, h, window);
    if let Some(prev) = pointer.pos {
        pointer.delta += next - prev;
    }
    pointer.pos = Some(next);
    pointer.seen = Some(now);
}

fn preview_to_window(x: f32, y: f32, w: f32, h: f32, window: Vec2) -> Vec2 {
    if w <= 0.0 || h <= 0.0 {
        return Vec2::new(x, y);
    }
    Vec2::new(x / w * window.x, y / h * window.y)
}

/// Whether the preview pointer is fresh enough to steer the game.
pub fn pointer_live(pointer: &PreviewPointer) -> bool {
    pointer.pos.is_some()
        && pointer
            .seen
            .is_some_and(|seen| seen.elapsed() < Duration::from_secs(5))
}

/// Parses a forwarded key name into a [`KeyCode`]. The viewport sends
/// `KeyboardEvent.code` spellings (`KeyW`, `Space`, `ArrowLeft`, `Digit0`).
pub fn parse_key(code: &str) -> Option<KeyCode> {
    Some(match code {
        "KeyA" => KeyCode::KeyA,
        "KeyB" => KeyCode::KeyB,
        "KeyC" => KeyCode::KeyC,
        "KeyD" => KeyCode::KeyD,
        "KeyE" => KeyCode::KeyE,
        "KeyF" => KeyCode::KeyF,
        "KeyG" => KeyCode::KeyG,
        "KeyH" => KeyCode::KeyH,
        "KeyI" => KeyCode::KeyI,
        "KeyJ" => KeyCode::KeyJ,
        "KeyK" => KeyCode::KeyK,
        "KeyL" => KeyCode::KeyL,
        "KeyM" => KeyCode::KeyM,
        "KeyN" => KeyCode::KeyN,
        "KeyO" => KeyCode::KeyO,
        "KeyP" => KeyCode::KeyP,
        "KeyQ" => KeyCode::KeyQ,
        "KeyR" => KeyCode::KeyR,
        "KeyS" => KeyCode::KeyS,
        "KeyT" => KeyCode::KeyT,
        "KeyU" => KeyCode::KeyU,
        "KeyV" => KeyCode::KeyV,
        "KeyW" => KeyCode::KeyW,
        "KeyX" => KeyCode::KeyX,
        "KeyY" => KeyCode::KeyY,
        "KeyZ" => KeyCode::KeyZ,
        "Digit0" | "Numpad0" => KeyCode::Digit0,
        "Digit1" | "Numpad1" => KeyCode::Digit1,
        "Digit2" | "Numpad2" => KeyCode::Digit2,
        "Digit3" | "Numpad3" => KeyCode::Digit3,
        "Digit4" | "Numpad4" => KeyCode::Digit4,
        "Digit5" | "Numpad5" => KeyCode::Digit5,
        "Digit6" | "Numpad6" => KeyCode::Digit6,
        "Digit7" | "Numpad7" => KeyCode::Digit7,
        "Digit8" | "Numpad8" => KeyCode::Digit8,
        "Digit9" | "Numpad9" => KeyCode::Digit9,
        "Space" => KeyCode::Space,
        "ArrowUp" => KeyCode::ArrowUp,
        "ArrowDown" => KeyCode::ArrowDown,
        "ArrowLeft" => KeyCode::ArrowLeft,
        "ArrowRight" => KeyCode::ArrowRight,
        "Enter" | "NumpadEnter" => KeyCode::Enter,
        "Escape" => KeyCode::Escape,
        "Tab" => KeyCode::Tab,
        "Backspace" => KeyCode::Backspace,
        "ShiftLeft" => KeyCode::ShiftLeft,
        "ShiftRight" => KeyCode::ShiftRight,
        "ControlLeft" => KeyCode::ControlLeft,
        "ControlRight" => KeyCode::ControlRight,
        "AltLeft" => KeyCode::AltLeft,
        "AltRight" => KeyCode::AltRight,
        _ => return None,
    })
}

// ─── Systems ─────────────────────────────────────────────────────────────────

/// Drains the editor-forwarded input queue onto the preview input resources
/// and Bevy's own button inputs, so the unchanged downstream systems
/// (`detect_clicks`, `publish_sensors`) read one truth. Runs first in the
/// Update chain, right after `pump_editor`.
pub fn drain_preview_inputs(
    mut engine: NonSendMut<crate::engine::Engine>,
    mut pointer: ResMut<PreviewPointer>,
    mut buttons: ResMut<PreviewButtons>,
    mut keys: ResMut<PreviewKeys>,
    mut manager: ResMut<crate::ui::UiManager>,
    mut mouse_buttons: ResMut<ButtonInput<MouseButton>>,
    mut key_buttons: ResMut<ButtonInput<KeyCode>>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    let inputs = std::mem::take(&mut engine.preview_inputs);
    if inputs.is_empty() {
        return;
    }
    let window_size = windows
        .single()
        .map(|window| Vec2::new(window.width(), window.height()))
        .unwrap_or(Vec2::new(960.0, 720.0));
    // Forwarded input means attention on the preview, which the OS window
    // behind the editor would otherwise never report as focused.
    engine.window_focused = true;
    for input in inputs {
        match &input {
            PreviewInput::Text { text } => {
                type_preview_text(&mut manager, &mut engine, text);
            }
            _ => apply_input(&input, &mut pointer, &mut buttons, &mut keys, window_size),
        }
    }
    // Mirror onto Bevy's inputs. Repeated presses are transition-only inside
    // `ButtonInput`, so holding doesn't retrigger an edge.
    set_button(&mut mouse_buttons, MouseButton::Left, buttons.left);
    set_button(&mut mouse_buttons, MouseButton::Right, buttons.right);
    set_button(&mut mouse_buttons, MouseButton::Middle, buttons.middle);
    let mut wanted: std::collections::HashSet<KeyCode> = std::collections::HashSet::new();
    for code in &keys.held {
        if let Some(key) = parse_key(code) {
            wanted.insert(key);
        }
    }
    // Release whatever the preview no longer holds, press what it does.
    for held in key_buttons.get_pressed().copied().collect::<Vec<_>>() {
        if !wanted.contains(&held) {
            key_buttons.release(held);
        }
    }
    for key in wanted {
        key_buttons.press(key);
    }
}

fn set_button(input: &mut ButtonInput<MouseButton>, button: MouseButton, down: bool) {
    if down {
        input.press(button);
    } else {
        input.release(button);
    }
}

/// Types forwarded text into the focused in-game input, through the same
/// per-character rules a physical keystroke goes through.
fn type_preview_text(
    manager: &mut crate::ui::UiManager,
    engine: &mut crate::engine::Engine,
    text: &str,
) {
    if !engine.running {
        return;
    }
    let Some(focused) = manager.focus().map(str::to_string) else {
        return;
    };
    let Some(node) = manager.get(&focused) else {
        return;
    };
    let (allow, ceiling) = (node.allow(), node.max_length());
    let mut current = node.value.as_text();
    let before = current.clone();
    for ch in text.chars() {
        if let Some(next) = blockloom_core::ui::typed(&current, ch, allow, ceiling) {
            current = next;
        }
    }
    if current != before
        && let Some(value) =
            manager.changed(&focused, blockloom_core::value::Evaluated::Text(current))
    {
        engine.fire(blockloom_core::vm::Event::UiChanged { id: focused, value });
    }
}

/// Captures a frame when due. Runs in Update, after the world has been drawn.
pub fn capture_preview_frame(mut state: ResMut<PreviewState>, mut commands: Commands) {
    if !capture_due(&state) {
        return;
    }
    // Reserve the slot up front so at most one capture is in flight: the
    // observer publishes on completion, and screenshots lag a frame behind.
    state.next_capture = Instant::now() + CAPTURE_INTERVAL;
    commands
        .spawn(bevy::render::view::window::screenshot::Screenshot::primary_window())
        .observe(on_screenshot);
}

fn on_screenshot(
    trigger: On<bevy::render::view::window::screenshot::ScreenshotCaptured>,
    mut state: ResMut<PreviewState>,
) {
    if !state.enabled {
        return;
    }
    if let Some(jpeg) = encode_jpeg(&trigger.image) {
        publish_frame(&mut state, jpeg);
        // `publish_frame` stamps its own throttle; the reservation above
        // only covered the flight time.
    }
}

/// Applies a viewport-requested resize to the window, so the stream is 1:1
/// with the viewport. A hidden headless window resizes freely: nobody sees
/// it change.
pub fn apply_preview_resize(
    mut engine: NonSendMut<crate::engine::Engine>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    let Some((width, height)) = engine.preview_resize.take() else {
        return;
    };
    if let Ok(mut window) = windows.single_mut() {
        window.resolution.set(width as f32, height as f32);
    }
}

/// Hides the OS window while headless previewing, shows it otherwise. The
/// hidden window keeps rendering, so screenshots keep flowing; on platforms
/// where hiding is unsupported (Wayland) the window simply stays up.
pub fn apply_preview_visibility(
    state: Res<PreviewState>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    let wanted = !(state.enabled && state.headless);
    if let Ok(mut window) = windows.single_mut()
        && window.visible != wanted
    {
        window.visible = wanted;
    }
}
