//! The embedded-preview sidecar: an MJPEG server on loopback plus the frame
//! capture and input injection behind it.
//!
//! The editor's viewport reads `http://127.0.0.1:{port}/preview.mjpg`
//! directly, so video never clogs the stdin/stdout control pipe. Control
//! (on/off, resize, input, step) still travels as [`EditorMessage`]s, and the
//! port travels back as [`RuntimeMessage::PreviewReady`]. Windowed mode keeps
//! the OS window up beside the viewport; headless mode hides it and the
//! hidden window keeps rendering the same stream.

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use blockloom_protocol::PreviewInput;
use std::io::Write;
use std::sync::{Arc, Mutex};
// `std::time::Instant` panics on wasm (no OS clock there); `web_time` reads
// the browser's clock instead and wraps std everywhere else.
use web_time::{Duration, Instant};

/// How often a frame is captured while previewing.
const CAPTURE_INTERVAL: Duration = Duration::from_millis(66);
/// JPEG quality for preview frames: small enough to stream, clear enough to
/// read the game. Keeps the pipe at a few hundred KB/s.
const JPEG_QUALITY: u8 = 60;

/// The latest encoded frame, shared with the HTTP thread.
#[derive(Resource, Clone, Default)]
pub struct LatestFrame(pub Arc<Mutex<Option<EncodedFrame>>>);

#[derive(Clone)]
pub struct EncodedFrame {
    jpeg: Vec<u8>,
    layout: String,
}

#[derive(Component)]
struct CaptureLayout(Option<blockloom_protocol::InterfaceLayout>);

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
    /// What the view last said about the keyboard. Once set, hovering no
    /// longer counts as attention.
    pub focus: Option<bool>,
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

/// Fingers down on the embedded viewport, in window pixels, so a lost focus
/// can lift every one of them.
#[derive(Resource, Default)]
pub struct PreviewTouches {
    pub held: std::collections::HashMap<u64, Vec2>,
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
        Some(frame) => {
            let jpeg = frame.jpeg;
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
    let mut last_layout = String::new();
    loop {
        let jpeg: Option<EncodedFrame> = frames.0.lock().ok().and_then(|guard| (*guard).clone());
        if let Some(frame) = jpeg {
            let jpeg = &frame.jpeg;
            // Skip re-sending an identical frame: a paused world is still.
            let hash = hash_bytes(jpeg);
            if jpeg.len() != last_len || hash != last_hash || frame.layout != last_layout {
                last_layout = frame.layout.clone();
                last_len = jpeg.len();
                last_hash = hash;
                if write_frame(stream, jpeg, &frame.layout).is_err() {
                    return;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(66));
    }
}

fn write_frame(stream: &mut impl Write, jpeg: &[u8], layout: &str) -> std::io::Result<()> {
    stream.write_all(
        format!(
            "--blockloom-frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nX-Blockloom-Interface: {}\r\n\r\n",
            jpeg.len(), layout
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
        PreviewInput::MouseDelta { dx, dy } => pointer.delta += Vec2::new(*dx, *dy),
        PreviewInput::Scroll { x, y, w, h, .. } => {
            move_pointer(pointer, *x, *y, *w, *h, window_size, now);
        }
        PreviewInput::Focus { focused } => {
            pointer.focus = Some(*focused);
            // No key-ups follow a lost focus, so nothing may stay held.
            if !focused {
                keys.held.clear();
                *buttons = PreviewButtons::default();
            }
        }
        PreviewInput::Text { .. } | PreviewInput::Touch { .. } => {}
    }
}

/// Turns a forwarded touch into Bevy's own, so `Touches` tracks it the way a
/// real touch screen's would. `None` for a finger nothing knows about.
pub fn touch_input(
    touches: &mut PreviewTouches,
    id: u64,
    phase: blockloom_protocol::TouchPhase,
    at: Vec2,
) -> Option<TouchInput> {
    use blockloom_protocol::TouchPhase as Phase;
    let phase = match phase {
        Phase::Start => {
            touches.held.insert(id, at);
            TouchPhase::Started
        }
        Phase::Move => {
            *touches.held.get_mut(&id)? = at;
            TouchPhase::Moved
        }
        Phase::End => {
            touches.held.remove(&id)?;
            TouchPhase::Ended
        }
        Phase::Cancel => {
            touches.held.remove(&id)?;
            TouchPhase::Canceled
        }
    };
    Some(TouchInput {
        phase,
        position: at,
        window: Entity::PLACEHOLDER,
        force: None,
        id,
    })
}

/// Cancels every finger still down, for a view that lost focus.
fn lift_touches(touches: &mut PreviewTouches) -> Vec<TouchInput> {
    touches
        .held
        .drain()
        .map(|(id, at)| TouchInput {
            phase: TouchPhase::Canceled,
            position: at,
            window: Entity::PLACEHOLDER,
            force: None,
            id,
        })
        .collect()
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

pub fn preview_to_window(x: f32, y: f32, w: f32, h: f32, window: Vec2) -> Vec2 {
    if w <= 0.0 || h <= 0.0 {
        return Vec2::new(x, y);
    }
    Vec2::new(x / w * window.x, y / h * window.y)
}

/// Whether the preview pointer is fresh enough to steer the game.
pub fn pointer_live(pointer: &PreviewPointer) -> bool {
    pointer.focus != Some(false)
        && pointer.pos.is_some()
        && pointer
            .seen
            .is_some_and(|seen| seen.elapsed() < Duration::from_secs(5))
}

/// Parses a forwarded key name into a [`KeyCode`]: Bevy's own variant names,
/// which are also `KeyboardEvent.code`'s (`KeyW`, `Space`, `ArrowLeft`).
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
        "Digit0" => KeyCode::Digit0,
        "Digit1" => KeyCode::Digit1,
        "Digit2" => KeyCode::Digit2,
        "Digit3" => KeyCode::Digit3,
        "Digit4" => KeyCode::Digit4,
        "Digit5" => KeyCode::Digit5,
        "Digit6" => KeyCode::Digit6,
        "Digit7" => KeyCode::Digit7,
        "Digit8" => KeyCode::Digit8,
        "Digit9" => KeyCode::Digit9,
        "Numpad0" => KeyCode::Numpad0,
        "Numpad1" => KeyCode::Numpad1,
        "Numpad2" => KeyCode::Numpad2,
        "Numpad3" => KeyCode::Numpad3,
        "Numpad4" => KeyCode::Numpad4,
        "Numpad5" => KeyCode::Numpad5,
        "Numpad6" => KeyCode::Numpad6,
        "Numpad7" => KeyCode::Numpad7,
        "Numpad8" => KeyCode::Numpad8,
        "Numpad9" => KeyCode::Numpad9,
        "NumpadAdd" => KeyCode::NumpadAdd,
        "NumpadSubtract" => KeyCode::NumpadSubtract,
        "NumpadMultiply" => KeyCode::NumpadMultiply,
        "NumpadDivide" => KeyCode::NumpadDivide,
        "NumpadDecimal" => KeyCode::NumpadDecimal,
        "NumpadEqual" => KeyCode::NumpadEqual,
        "NumpadComma" => KeyCode::NumpadComma,
        "NumpadEnter" => KeyCode::NumpadEnter,
        "NumLock" => KeyCode::NumLock,
        "Minus" => KeyCode::Minus,
        "Equal" => KeyCode::Equal,
        "BracketLeft" => KeyCode::BracketLeft,
        "BracketRight" => KeyCode::BracketRight,
        "Backslash" => KeyCode::Backslash,
        "IntlBackslash" => KeyCode::IntlBackslash,
        "IntlRo" => KeyCode::IntlRo,
        "IntlYen" => KeyCode::IntlYen,
        "Semicolon" => KeyCode::Semicolon,
        "Quote" => KeyCode::Quote,
        "Backquote" => KeyCode::Backquote,
        "Comma" => KeyCode::Comma,
        "Period" => KeyCode::Period,
        "Slash" => KeyCode::Slash,
        "Space" => KeyCode::Space,
        "ArrowUp" => KeyCode::ArrowUp,
        "ArrowDown" => KeyCode::ArrowDown,
        "ArrowLeft" => KeyCode::ArrowLeft,
        "ArrowRight" => KeyCode::ArrowRight,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        "Insert" => KeyCode::Insert,
        "Delete" => KeyCode::Delete,
        "Enter" => KeyCode::Enter,
        "Escape" => KeyCode::Escape,
        "Tab" => KeyCode::Tab,
        "Backspace" => KeyCode::Backspace,
        "CapsLock" => KeyCode::CapsLock,
        "ScrollLock" => KeyCode::ScrollLock,
        "PrintScreen" => KeyCode::PrintScreen,
        "Pause" => KeyCode::Pause,
        "ContextMenu" => KeyCode::ContextMenu,
        "ShiftLeft" => KeyCode::ShiftLeft,
        "ShiftRight" => KeyCode::ShiftRight,
        "ControlLeft" => KeyCode::ControlLeft,
        "ControlRight" => KeyCode::ControlRight,
        "AltLeft" => KeyCode::AltLeft,
        "AltRight" => KeyCode::AltRight,
        "SuperLeft" | "MetaLeft" => KeyCode::SuperLeft,
        "SuperRight" | "MetaRight" => KeyCode::SuperRight,
        "F1" => KeyCode::F1,
        "F2" => KeyCode::F2,
        "F3" => KeyCode::F3,
        "F4" => KeyCode::F4,
        "F5" => KeyCode::F5,
        "F6" => KeyCode::F6,
        "F7" => KeyCode::F7,
        "F8" => KeyCode::F8,
        "F9" => KeyCode::F9,
        "F10" => KeyCode::F10,
        "F11" => KeyCode::F11,
        "F12" => KeyCode::F12,
        "F13" => KeyCode::F13,
        "F14" => KeyCode::F14,
        "F15" => KeyCode::F15,
        "F16" => KeyCode::F16,
        "F17" => KeyCode::F17,
        "F18" => KeyCode::F18,
        "F19" => KeyCode::F19,
        "F20" => KeyCode::F20,
        "F21" => KeyCode::F21,
        "F22" => KeyCode::F22,
        "F23" => KeyCode::F23,
        "F24" => KeyCode::F24,
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
    mut touches: ResMut<PreviewTouches>,
    mut touch_out: MessageWriter<TouchInput>,
    mut wheel_out: MessageWriter<MouseWheel>,
    mut manager: ResMut<crate::ui::UiManager>,
    mut mouse_buttons: ResMut<ButtonInput<MouseButton>>,
    mut key_buttons: ResMut<ButtonInput<KeyCode>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    #[cfg(any(target_os = "linux", target_os = "windows"))] surface: Option<
        Res<crate::embed::GameSurface>,
    >,
) {
    let inputs = std::mem::take(&mut engine.preview_inputs);
    if inputs.is_empty() {
        return;
    }
    let window_size = windows
        .single()
        .map(|window| Vec2::new(window.width(), window.height()))
        .unwrap_or(Vec2::new(960.0, 720.0));
    // Embedded, the pointer lands on the shared image rather than a window.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let window_size = surface.map_or(window_size, |surface| surface.size());
    for input in inputs {
        match &input {
            PreviewInput::Text { text } => {
                type_preview_text(&mut manager, &mut engine, text);
            }
            PreviewInput::Touch {
                id,
                phase,
                x,
                y,
                w,
                h,
            } => {
                let at = preview_to_window(*x, *y, *w, *h, window_size);
                if let Some(touch) = touch_input(&mut touches, *id, *phase, at) {
                    touch_out.write(touch);
                }
            }
            _ => {
                apply_input(&input, &mut pointer, &mut buttons, &mut keys, window_size);
                match input {
                    // Read by `scroll_ui_lists` later this same frame.
                    PreviewInput::Scroll { dx, dy, line, .. } => {
                        wheel_out.write(MouseWheel {
                            unit: if line {
                                MouseScrollUnit::Line
                            } else {
                                MouseScrollUnit::Pixel
                            },
                            x: dx,
                            y: dy,
                            window: Entity::PLACEHOLDER,
                            phase: TouchPhase::Moved,
                        });
                    }
                    // No touch-ends follow a lost focus either.
                    PreviewInput::Focus { focused: false } => {
                        touch_out.write_batch(lift_touches(&mut touches));
                    }
                    _ => {}
                }
            }
        }
    }
    // Forwarded input means attention on the preview, which the OS window
    // behind the editor would otherwise never report, unless the view says.
    engine.window_focused = pointer.focus.unwrap_or(true);
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

/// Captures after layout, so geometry is frozen with the screenshot request.
pub fn capture_preview_frame(
    mut state: ResMut<PreviewState>,
    design: Res<crate::ui_design::DesignSession>,
    mut commands: Commands,
) {
    if !capture_due(&state) {
        return;
    }
    // Reserve the slot up front so at most one capture is in flight: the
    // observer publishes on completion, and screenshots lag a frame behind.
    state.next_capture = Instant::now() + CAPTURE_INTERVAL;
    commands
        .spawn((
            bevy::render::view::window::screenshot::Screenshot::primary_window(),
            CaptureLayout(design.last.clone()),
        ))
        .observe(on_screenshot);
}

fn on_screenshot(
    trigger: On<bevy::render::view::window::screenshot::ScreenshotCaptured>,
    mut state: ResMut<PreviewState>,
    captures: Query<&CaptureLayout>,
) {
    if !state.enabled {
        return;
    }
    if let Some(jpeg) = encode_jpeg(&trigger.image) {
        let layout = captures.get(trigger.entity).ok().and_then(|v| v.0.as_ref());
        if let Ok(mut guard) = state.frames.0.lock() {
            *guard = Some(EncodedFrame {
                jpeg,
                layout: layout
                    .and_then(|v| serde_json::to_string(v).ok())
                    .unwrap_or_default(),
            });
        }
        state.next_capture = Instant::now() + CAPTURE_INTERVAL;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_frame_carries_its_frozen_layout_even_for_identical_pixels() {
        let mut stream = Vec::new();
        let layout = serde_json::json!({"revision": 9, "generation": 3, "viewport": [960,720], "widgets": [{"id": "line\nbreak"}]}).to_string();
        write_frame(&mut stream, &[1, 2, 3], &layout).unwrap();
        let header_end = stream.windows(4).position(|p| p == b"\r\n\r\n").unwrap();
        let headers = std::str::from_utf8(&stream[..header_end]).unwrap();
        assert!(headers.contains("Content-Length: 3"));
        let metadata = headers
            .lines()
            .find_map(|l| l.strip_prefix("X-Blockloom-Interface: "))
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(metadata).unwrap()["revision"],
            9
        );
        assert_eq!(&stream[header_end + 4..header_end + 7], &[1, 2, 3]);
    }

    #[test]
    fn a_locked_view_moves_by_raw_motion_and_a_lost_focus_lets_go() {
        let mut pointer = PreviewPointer::default();
        let mut buttons = PreviewButtons::default();
        let mut keys = PreviewKeys::default();
        let size = Vec2::new(960.0, 720.0);
        let mut apply = |input: PreviewInput, pointer: &mut PreviewPointer| {
            apply_input(&input, pointer, &mut buttons, &mut keys, size)
        };
        apply(PreviewInput::Focus { focused: true }, &mut pointer);
        apply(
            PreviewInput::MouseMove {
                x: 10.0,
                y: 10.0,
                w: 960.0,
                h: 720.0,
            },
            &mut pointer,
        );
        apply(
            PreviewInput::Key {
                code: "KeyW".into(),
                down: true,
            },
            &mut pointer,
        );
        apply(PreviewInput::MouseDelta { dx: 3.5, dy: -2.0 }, &mut pointer);
        apply(PreviewInput::MouseDelta { dx: 1.5, dy: 0.0 }, &mut pointer);
        // Raw motion isn't scaled onto the window, and never moves the position.
        assert_eq!(pointer.delta, Vec2::new(5.0, -2.0));
        assert_eq!(pointer.pos, Some(Vec2::new(10.0, 10.0)));
        assert!(pointer_live(&pointer));

        apply(PreviewInput::Focus { focused: false }, &mut pointer);
        // No key-up ever comes for a key held while focus left.
        assert!(keys.held.is_empty());
        // Hovering an unfocused view steers nothing.
        assert!(!pointer_live(&pointer));
    }

    #[test]
    fn touches_track_their_finger_and_a_lost_focus_lifts_them() {
        use blockloom_protocol::TouchPhase as Phase;
        let mut touches = PreviewTouches::default();
        // A finger nothing started is not news.
        assert!(touch_input(&mut touches, 1, Phase::Move, Vec2::ZERO).is_none());
        let start = touch_input(&mut touches, 1, Phase::Start, Vec2::new(4.0, 5.0)).unwrap();
        assert_eq!(start.phase, TouchPhase::Started);
        touch_input(&mut touches, 2, Phase::Start, Vec2::ONE).unwrap();
        let moved = touch_input(&mut touches, 1, Phase::Move, Vec2::new(6.0, 5.0)).unwrap();
        assert_eq!(moved.position, Vec2::new(6.0, 5.0));
        touch_input(&mut touches, 2, Phase::End, Vec2::ONE).unwrap();
        let lifted = lift_touches(&mut touches);
        assert_eq!(lifted.len(), 1);
        assert_eq!(lifted[0].phase, TouchPhase::Canceled);
        assert_eq!(lifted[0].position, Vec2::new(6.0, 5.0));
        assert!(touches.held.is_empty());
    }

    #[test]
    fn keys_the_old_mapping_dropped_now_arrive() {
        assert_eq!(parse_key("Numpad1"), Some(KeyCode::Numpad1));
        assert_eq!(parse_key("BracketLeft"), Some(KeyCode::BracketLeft));
        assert_eq!(parse_key("Backquote"), Some(KeyCode::Backquote));
        assert_eq!(parse_key("ShiftRight"), Some(KeyCode::ShiftRight));
    }
}
