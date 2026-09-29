//! The Android player (Phase 6.5): a built game as its APK's native library.
//!
//! A build packs the staged game folder into the APK's assets and the runtime
//! plus scripts into `lib/<abi>/` (see `blockloom_core::android`). There is
//! no Java: the manifest names NativeActivity with this library, and the
//! entry below boots the world straight from the APK.
//!
//! What Android changes versus a desktop player:
//! - The pack comes from the asset manager, never `std::fs` beside a binary.
//!   Game files read through the `vfs` hook registered here, while images and
//!   the like load through Bevy's own Android asset reader.
//! - SDR only, no Solari: the shipped `.so` builds with neither (see
//!   `blockloom_core::android::build_runtime_so`), and `Launch::Android`
//!   refuses HDR the way the web launch does.
//! - Saves and caches live under the app's internal data dir: an APK's assets
//!   are read-only, and probe bakes ship pre-baked and are never rewritten.
//! - The world pauses on focus loss and resumes on return, the back button
//!   arrives as the `back` key (a visible modal swallows it, like clicks),
//!   and a focused text input raises the soft keyboard. Touch and gamepads
//!   are Bevy's own mobile input; there is no second stack.
//! - No safe-area API exists in this Bevy line, so notched displays are not
//!   inset: feeding the activity's content rect into the UI layout is a later
//!   step, not this one.

use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowFocused};
use blockloom_core::pack::GamePack;
use std::path::{Path, PathBuf};

/// This `.so`'s own file name. The installer unpacks it into the app's lib
/// dir, so scripts ride beside it and resolve the same way.
const OWN_LIB: &str = "libblockloom_runtime.so";

/// The NativeActivity entry. `bevy_main` sets `ANDROID_APP` and calls this
/// module's `main`, which boots the game; winit picks the app handle up from
/// there. The generated `android_main` is unmangled, which is the symbol the
/// activity loads.
#[bevy::prelude::bevy_main]
fn main() {
    crate::run_android();
}

/// Reads one APK asset by its path in the game folder (`game.pack`,
/// `assets/sprites/ball.png`): what `vfs` calls through its hook. Paths are
/// already forward-slash relative, which is what the asset manager takes.
fn read_asset(key: &str) -> Option<Vec<u8>> {
    let app = bevy_android::ANDROID_APP.get()?;
    let name = std::ffi::CString::new(key).ok()?;
    let mut asset = app.asset_manager().open(&name)?;
    asset.buffer().ok().map(|bytes| bytes.to_vec())
}

/// Registers the APK asset reads with `vfs`, then loads the pack. Runs before
/// the Bevy app exists: the asset manager is usable as soon as `ANDROID_APP`
/// is set, which the entry above did.
pub fn load_pack() -> Result<GamePack, String> {
    blockloom_core::vfs::set_asset_reader(read_asset);
    let bytes = blockloom_core::vfs::read(Path::new(blockloom_core::pack::PACK_FILE))
        .map_err(|e| format!("couldn't read the game out of the APK: {e}"))?;
    let text =
        String::from_utf8(bytes).map_err(|e| format!("the APK's gamepack isn't text: {e}"))?;
    GamePack::from_json(&text, blockloom_core::pack::PACK_FILE)
}

/// Where saves and caches live: the app's internal data dir, which survives
/// updates and needs no permission. Falls back to the temp dir when the app
/// handle isn't up yet, so path helpers stay total.
fn data_root() -> PathBuf {
    if let Some(app) = bevy_android::ANDROID_APP.get()
        && let Some(dir) = app.internal_data_path()
    {
        return dir.join("blockloom");
    }
    std::env::temp_dir().join("blockloom")
}

/// This game's save file. Same shape as `save::path`, rooted on device.
pub fn save_path(project_id: &str) -> PathBuf {
    data_root().join("saves").join(format!("{project_id}.json"))
}

/// A cache file under the data dir, e.g. `surfaces/<key>.json`.
pub fn data_file(relative: &str) -> PathBuf {
    let mut path = data_root();
    for part in relative.split('/').filter(|part| !part.is_empty()) {
        path.push(part);
    }
    path
}

/// Finds a shipped `.so` by its file name: beside this library in the app's
/// lib dir when `/proc/self/maps` names it, else the bare name, which the
/// linker resolves through the app's own library path.
pub fn native_lib_path(file_name: &str) -> PathBuf {
    if let Some(dir) = own_lib_dir() {
        let candidate = dir.join(file_name);
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from(file_name)
}

fn own_lib_dir() -> Option<PathBuf> {
    let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
    for line in maps.lines() {
        let path = line.split_whitespace().last()?;
        if path.ends_with(&format!("/{OWN_LIB}")) {
            return Path::new(path).parent().map(Path::to_path_buf);
        }
    }
    None
}

/// Whether the last focus loss paused us. A game the player paused stays
/// paused across a suspend; only our own pause lifts on return.
#[derive(Resource, Default)]
struct AutoPaused(bool);

/// At most two logcat snapshots per run: the world-built marker plus actor
/// positions now and a few seconds later, so `just android-smoke` can tell
/// the world started and things moved. Then quiet, however long the run is.
#[derive(Resource)]
struct SmokeLog {
    stage: u8,
    at: f64,
}

impl Default for SmokeLog {
    fn default() -> Self {
        Self { stage: 0, at: 0.0 }
    }
}

pub fn register(app: &mut App) {
    app.init_resource::<AutoPaused>()
        .init_resource::<SmokeLog>()
        .add_systems(Update, (auto_pause, sync_soft_keyboard, smoke_log));
}

/// Freezes the world when the activity loses focus (home button, task
/// switcher) and thaws it on return. `set_paused` shifts the clock, so
/// timers resume where they froze instead of jumping.
fn auto_pause(
    mut engine: NonSendMut<crate::engine::Engine>,
    time: Res<Time>,
    mut focus: MessageReader<WindowFocused>,
    mut state: ResMut<AutoPaused>,
) {
    for event in focus.read() {
        if event.focused {
            if state.0 && engine.running {
                crate::world::set_paused(&mut engine, false, time.elapsed_secs() as f64);
                state.0 = false;
            }
        } else if engine.running && !engine.paused {
            crate::world::set_paused(&mut engine, true, time.elapsed_secs() as f64);
            state.0 = true;
        }
    }
}

/// Raises the soft keyboard while a text input holds it, and lowers it
/// otherwise. Bevy forwards the flag to winit, which shows or hides the
/// input method; commits arrive as IME text (see
/// `world::type_into_focused_input`).
fn sync_soft_keyboard(
    manager: Res<crate::ui::UiManager>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    let typing = manager.focus().is_some();
    for mut window in &mut windows {
        if window.ime_enabled != typing {
            window.ime_enabled = typing;
        }
    }
}

fn smoke_log(
    engine: NonSendMut<crate::engine::Engine>,
    time: Res<Time<Real>>,
    mut state: ResMut<SmokeLog>,
) {
    if state.stage >= 2 || !engine.running {
        return;
    }
    let now = time.elapsed_secs() as f64;
    if state.stage == 0 {
        eprintln!("blockloom: run started");
        eprintln!("blockloom: actors {}", actor_snapshot());
        state.stage = 1;
        state.at = now + 5.0;
    } else if now >= state.at {
        eprintln!("blockloom: actors {}", actor_snapshot());
        state.stage = 2;
    }
}

/// Every actor the blocks last sensed, as `{id: [x, y]}`. Same shape as the
/// web player's `game_actors`, on stderr where logcat keeps it.
fn actor_snapshot() -> String {
    let actors: serde_json::Map<String, serde_json::Value> =
        blockloom_core::sense::read(|sensors| {
            sensors
                .actors
                .iter()
                .map(|(id, actor)| {
                    (
                        id.clone(),
                        serde_json::json!([actor.position[0], actor.position[1]]),
                    )
                })
                .collect()
        });
    serde_json::Value::Object(actors).to_string()
}
