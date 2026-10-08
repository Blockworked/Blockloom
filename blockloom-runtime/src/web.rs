//! The browser player (Phase 8): a built game as a wasm module.
//!
//! A web build is one `.html` file (see `blockloom_core::build`): its page
//! unpacks the player, the pack, every game file and each script's wasm
//! module, then calls [`start_game`] once the player clicks to play (audio
//! needs that gesture). The files are mounted in [`blockloom_core::vfs`] and
//! served to Bevy's asset server from memory, so nothing is fetched; a page
//! that passes none falls back to fetching them from its server (a folder
//! deploy). Saves live in localStorage under the pack's id, pointer lock goes
//! through the browser's own API on a click, and a fatal error lands on the
//! page as an overlay - there is no process to exit. Blocks run on the VM.

use crate::player::Launch;
use bevy::app::PluginGroup;
use bevy::asset::io::wasm::HttpWasmAssetReader;
use bevy::asset::io::{
    AssetReader, AssetReaderError, AssetSourceBuilder, AssetSourceId, PathStream, Reader, VecReader,
};
use bevy::prelude::AssetApp;
use blockloom_core::pack::GamePack;
use blockloom_core::save::SaveData;
use js_sys::{Object, Uint8Array, WebAssembly};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

thread_local! {
    /// Each script's compiled module, keyed by its library's path in the
    /// game folder (`.blockloom/build/player.wasm`).
    static SCRIPTS: RefCell<HashMap<String, WebAssembly::Module>> = RefCell::default();
    /// The canvas the game draws into, for pointer lock.
    static CANVAS: RefCell<Option<web_sys::Element>> = const { RefCell::new(None) };
    /// Whether the game wants the pointer locked; a click on the canvas asks
    /// the browser for it, since only a gesture can.
    static WANTS_LOCK: Cell<bool> = const { Cell::new(false) };
}

/// Boots the game `pack_json` describes onto `canvas` (a CSS selector, or
/// nothing for a canvas Bevy appends itself). `files` maps each game file's
/// path to its bytes and `scripts` each script library's path to its compiled
/// `WebAssembly.Module`; either may be left out. Reports a bad pack back to
/// the page rather than starting half a world.
#[wasm_bindgen]
pub fn start_game(
    pack_json: &str,
    canvas: Option<String>,
    files: JsValue,
    scripts: JsValue,
) -> Result<(), JsValue> {
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
    if let Some(files) = files.dyn_ref::<Object>() {
        blockloom_core::vfs::mount(entries(files).filter_map(|(path, bytes)| {
            Some((path, bytes.dyn_into::<Uint8Array>().ok()?.to_vec()))
        }));
    }
    // Shipped plugins read their files from what the page mounted.
    #[cfg(feature = "plugins")]
    blockloom_plugin_host::files::set_reader(blockloom_core::vfs::read);
    if let Some(scripts) = scripts.dyn_ref::<Object>() {
        SCRIPTS.with_borrow_mut(|known| {
            known.extend(entries(scripts).filter_map(|(path, module)| {
                Some((
                    blockloom_core::vfs::key(Path::new(&path)),
                    module.dyn_into::<WebAssembly::Module>().ok()?,
                ))
            }));
        });
    }
    if let Some(selector) = &canvas {
        watch_canvas(selector);
    }
    let launch = Launch::from_pack(pack);
    let mode = launch.mode();
    let title = launch.title();

    let mut app = bevy::prelude::App::new();
    // Registered ahead of the asset plugin, which builds its sources then.
    app.register_asset_source(
        AssetSourceId::Default,
        AssetSourceBuilder::new(|| Box::new(GameFiles::default())),
    );
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
                    // The page sizes the canvas's parent; the game follows it.
                    fit_canvas_to_parent: true,
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

/// Every actor in the running world as the blocks last sensed it, as JSON
/// `{id: {name, position}}`: what the page (or a smoke test) can ask of a
/// game without reading pixels.
#[wasm_bindgen]
pub fn game_actors() -> String {
    let actors: serde_json::Map<String, serde_json::Value> =
        blockloom_core::sense::read(|sensors| {
            sensors
                .actors
                .iter()
                .map(|(id, actor)| {
                    (
                        id.clone(),
                        serde_json::json!({ "name": actor.name, "position": actor.position }),
                    )
                })
                .collect()
        });
    serde_json::Value::Object(actors).to_string()
}

/// A JS object's own `[key, value]` pairs.
fn entries(object: &Object) -> impl Iterator<Item = (String, JsValue)> {
    Object::entries(object).into_iter().filter_map(|entry| {
        let pair = entry.dyn_into::<js_sys::Array>().ok()?;
        Some((pair.get(0).as_string()?, pair.get(1)))
    })
}

/// The compiled module the page handed over for the script at `relative`
/// (its source path), if the build made one.
pub(crate) fn script_module(relative: &str) -> Option<WebAssembly::Module> {
    let library = blockloom_core::script::library_path(Path::new(""), relative);
    let key = blockloom_core::vfs::key(&library);
    SCRIPTS.with_borrow(|known| known.get(&key).cloned())
}

/// Game files for the asset server: the mounted ones, or with none mounted,
/// whatever the page's server has at the same path.
struct GameFiles {
    server: HttpWasmAssetReader,
}

impl Default for GameFiles {
    fn default() -> Self {
        Self {
            server: HttpWasmAssetReader::new(""),
        }
    }
}

impl AssetReader for GameFiles {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<VecReader, AssetReaderError> {
        if let Some(bytes) = blockloom_core::vfs::mounted(path) {
            return Ok(VecReader::new(bytes.to_vec()));
        }
        if blockloom_core::vfs::is_mounted() {
            return Err(AssetReaderError::NotFound(path.to_path_buf()));
        }
        let mut reader = self.server.read(path).await?;
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Ok(VecReader::new(bytes))
    }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<VecReader, AssetReaderError> {
        // Meta files are never shipped (see `asset_plugin`).
        Err(AssetReaderError::NotFound(path.to_path_buf()))
    }

    async fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        Err(AssetReaderError::NotFound(path.to_path_buf()))
    }

    async fn is_directory<'a>(&'a self, _path: &'a Path) -> Result<bool, AssetReaderError> {
        Ok(false)
    }
}

/// Remembers the canvas and asks for pointer lock on a click while the game
/// wants it: the browser only grants a lock inside a user gesture.
fn watch_canvas(selector: &str) {
    let Some(canvas) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.query_selector(selector).ok().flatten())
    else {
        return;
    };
    let target = canvas.clone();
    let click = Closure::<dyn FnMut()>::new(move || {
        if WANTS_LOCK.get() && !pointer_locked() {
            target.request_pointer_lock();
        }
    });
    let _ = canvas.add_event_listener_with_callback("pointerdown", click.as_ref().unchecked_ref());
    // Listens for the whole page's life.
    click.forget();
    CANVAS.set(Some(canvas));
}

fn pointer_locked() -> bool {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.pointer_lock_element())
        .is_some()
}

/// What `lock mouse` is in a browser. Locking waits for the next click on
/// the canvas (tried now too, which a browser allows after a recent
/// gesture); unlocking lets go at once.
pub(crate) fn want_pointer_lock(wanted: bool) {
    if WANTS_LOCK.replace(wanted) == wanted {
        return;
    }
    if wanted {
        CANVAS.with_borrow(|canvas| {
            if let Some(canvas) = canvas {
                canvas.request_pointer_lock();
            }
        });
    } else if pointer_locked()
        && let Some(document) = web_sys::window().and_then(|window| window.document())
    {
        document.exit_pointer_lock();
    }
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
    load_save_slot(project_id, blockloom_core::save::DEFAULT_SLOT)
}

/// Reads one named save slot from localStorage. The default slot is the
/// legacy key above, so an old browser save loads unchanged.
pub(crate) fn load_save_slot(project_id: &str, slot: &str) -> SaveData {
    let text = storage()
        .and_then(|store| store.get_item(&save_key_slot(project_id, slot)).ok())
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
    store_save_slot(project_id, blockloom_core::save::DEFAULT_SLOT, data)
}

/// Writes one named save slot to localStorage.
pub(crate) fn store_save_slot(project_id: &str, slot: &str, data: &SaveData) -> Result<(), String> {
    let store = storage().ok_or_else(|| "this browser won't keep saves".to_string())?;
    let text = serde_json::to_string(data).map_err(|e| e.to_string())?;
    store
        .set_item(&save_key_slot(project_id, slot), &text)
        .map_err(|_| "this browser won't keep saves".to_string())
}

/// Deletes one named save slot from localStorage. Quiet when nothing by
/// that name was saved: removing a missing key is not an error.
pub(crate) fn delete_save_slot(project_id: &str, slot: &str) -> Result<(), String> {
    let store = storage().ok_or_else(|| "this browser won't keep saves".to_string())?;
    store
        .remove_item(&save_key_slot(project_id, slot))
        .map_err(|_| "could not remove a save".to_string())
}

/// Every slot with an entry in localStorage, default first. A browser with
/// no storage reads as none rather than failing the run.
pub(crate) fn list_save_slots(project_id: &str) -> Vec<String> {
    use blockloom_core::save::DEFAULT_SLOT;
    let Some(store) = storage() else {
        return Vec::new();
    };
    let plain = save_key(project_id);
    let prefix = format!("{plain}__");
    let Ok(len) = store.length() else {
        return Vec::new();
    };
    let mut slots = Vec::new();
    for i in 0..len {
        let Ok(Some(key)) = store.key(i) else {
            continue;
        };
        if key == plain {
            slots.push(DEFAULT_SLOT.to_string());
        } else if let Some(slot) = key.strip_prefix(&prefix)
            && !slot.is_empty()
            && slot == blockloom_core::save::normalize_slot(slot)
        {
            slots.push(slot.to_string());
        }
    }
    slots.sort();
    slots.dedup();
    if let Some(at) = slots.iter().position(|s| s == DEFAULT_SLOT) {
        let default = slots.remove(at);
        slots.insert(0, default);
    }
    slots
}

/// localStorage as the plain table plugin saves are kept in.
pub(crate) struct LocalKv;

impl blockloom_plugin_host::storage::KvBackend for LocalKv {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        let store = storage().ok_or("this browser won't keep saves")?;
        store
            .get_item(key)
            .map_err(|_| "could not read a save".to_string())
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        let store = storage().ok_or("this browser won't keep saves")?;
        store
            .set_item(key, value)
            .map_err(|_| "this browser has no room for that save".to_string())
    }

    fn remove(&self, key: &str) -> Result<(), String> {
        let store = storage().ok_or("this browser won't keep saves")?;
        store
            .remove_item(key)
            .map_err(|_| "could not remove a save".to_string())
    }

    fn keys(&self) -> Result<Vec<String>, String> {
        let store = storage().ok_or("this browser won't keep saves")?;
        let count = store
            .length()
            .map_err(|_| "could not list saves".to_string())?;
        Ok((0..count)
            .filter_map(|i| store.key(i).ok().flatten())
            .collect())
    }
}

/// A project's plugin saves, kept beside its variable saves.
pub(crate) fn plugin_save_prefix(project_id: &str) -> String {
    format!("blockloom-plugin-save:{project_id}/")
}

fn save_key(project_id: &str) -> String {
    format!("blockloom-save:{project_id}")
}

/// The localStorage key for one named save slot. The default slot is the
/// legacy key above, so an old browser save loads unchanged.
fn save_key_slot(project_id: &str, slot: &str) -> String {
    let slot = blockloom_core::save::normalize_slot(slot);
    if slot == blockloom_core::save::DEFAULT_SLOT {
        save_key(project_id)
    } else {
        format!("blockloom-save:{project_id}__{slot}")
    }
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}
