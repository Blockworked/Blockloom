//! The browser player (Phase 8): a built game as a wasm module.
//!
//! The page inlines its `game.pack` JSON and calls [`start_game`] with it,
//! so the pack arrives as a string rather than a file beside a binary. Assets
//! resolve against the server root through Bevy's own web reader, saves live
//! in localStorage under the pack's id, and a fatal error lands on the page
//! as an overlay - there is no process to exit. Blocks run on the VM;
//! scripts and native logic stay unloaded (see `script.rs` and `logic.rs`).

use crate::player::Launch;
use bevy::app::PluginGroup;
use blockloom_core::pack::GamePack;
use blockloom_core::save::SaveData;
use wasm_bindgen::prelude::*;

/// Boots the game `pack_json` describes onto `canvas` (a CSS selector, or
/// nothing for a canvas Bevy appends itself). Reports a bad pack back to the
/// page rather than starting half a world.
#[wasm_bindgen]
pub fn start_game(pack_json: &str, canvas: Option<String>) -> Result<(), JsValue> {
    // A bare wasm trap says nothing: forward panics to the devtools console
    // with their message, the way a native run prints them to stderr.
    #[cfg(target_arch = "wasm32")]
    std::panic::set_hook(Box::new(|info| {
        console_error(&format!("blockloom: panic: {info}"));
    }));
    // Before the pack is read: a saved document names Blockloom's own
    // reporter blocks, which have to be registered to evaluate.
    blockloom_core::init();
    let pack = match GamePack::from_json(pack_json, "game.pack") {
        Ok(pack) => pack,
        Err(message) => {
            show_error(&message);
            return Err(JsValue::from_str(&message));
        }
    };
    let launch = Launch::from_pack(pack);
    let mode = launch.mode();
    let title = launch.title();

    let mut app = bevy::prelude::App::new();
    app.add_plugins(
        bevy::prelude::DefaultPlugins
            .set(bevy::prelude::WindowPlugin {
                primary_window: Some(bevy::prelude::Window {
                    title,
                    resolution: bevy::window::WindowResolution::new(
                        blockloom_protocol::GAME_SIZE.0,
                        blockloom_protocol::GAME_SIZE.1,
                    ),
                    canvas,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .set(crate::asset_plugin()),
    );
    crate::add_world(&mut app, mode, launch.into_engine());
    app.run();
    Ok(())
}

/// Pins a fatal message onto the page. Does nothing where there is no
/// document (a headless smoke test), so the caller still returns the error.
pub fn show_error(message: &str) {
    let _ = overlay(message);
}

fn overlay(message: &str) -> Option<()> {
    let document = web_sys::window()?.document()?;
    let body = document.body()?;
    let element = document.create_element("div").ok()?;
    element
        .set_attribute(
            "style",
            "position:fixed;left:16px;right:16px;bottom:16px;padding:12px 16px;\
             background:#3a1414;color:#ffd7d7;font:14px sans-serif;white-space:pre-wrap;\
             border:1px solid #a33;border-radius:8px;z-index:2147483647;",
        )
        .ok()?;
    element.set_text_content(Some(&format!("Blockloom: {message}")));
    body.append_child(&element).ok()?;
    Some(())
}

/// A player-side error where `eprintln!` would go on native. Standard output
/// is nowhere on wasm (printing panics), so it goes to the devtools console.
pub(crate) fn console_error(message: &str) {
    web_sys::console::error_1(&JsValue::from_str(message));
}

/// Reads this game's saves from localStorage: what `save::read` is on
/// native. Missing or corrupt entries read as empty rather than failing the
/// run - a renamed project simply starts fresh.
pub(crate) fn load_save(project_id: &str) -> SaveData {
    let text = storage()
        .and_then(|store| store.get_item(&save_key(project_id)).ok())
        .flatten();
    match text {
        Some(text) => serde_json::from_str(&text).unwrap_or_default(),
        None => SaveData::default(),
    }
}

/// Writes this game's saves to localStorage: what `save::write` is on
/// native. Private browsing can refuse the write, which surfaces as a block
/// error rather than a lost run.
pub(crate) fn store_save(project_id: &str, data: &SaveData) -> Result<(), String> {
    let store = storage().ok_or_else(|| "this browser won't keep saves".to_string())?;
    let text = serde_json::to_string(data).map_err(|e| e.to_string())?;
    store
        .set_item(&save_key(project_id), &text)
        .map_err(|_| "this browser won't keep saves".to_string())
}

fn save_key(project_id: &str) -> String {
    format!("blockloom-save:{project_id}")
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}
