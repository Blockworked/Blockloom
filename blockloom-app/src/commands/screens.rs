//! Screen commands: make, read and write `.blockui` files, and list them
//! on the open scene or the whole game.

use super::{auto_save, emit, lock, project_dir, push_undo, reindex, runtime_stale, sync_runtime};
use crate::AppHandle;
use crate::state::{AppState, SharedState};
use blockloom_core::assets;
use blockloom_core::ui_layers::{self as layers, UiLayer};
use blockloom_core::ui_tree::UiScreen;
use std::path::Path;

fn checked_path(path: &str) -> Result<String, String> {
    let path = assets::normalize(path).ok_or("That isn't a path in this project")?;
    if !layers::is_screen(&path) {
        return Err(format!("A screen's file ends in .{}", layers::EXTENSION));
    }
    Ok(path)
}

fn new_screen_path(dir: &Path, name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stem = match stem.trim() {
        "" => "Screen",
        stem => stem,
    };
    (1..)
        .map(|n| match n {
            1 => format!("assets/ui/{stem}.{}", layers::EXTENSION),
            n => format!("assets/ui/{stem} {n}.{}", layers::EXTENSION),
        })
        .find(|path| !dir.join(path).exists())
        .expect("an unused name always exists")
}

/// Which list a layer command edits: the open scene's or the game's.
fn list<'a>(s: &'a mut AppState, scope: Option<&str>) -> Result<&'a mut Vec<UiLayer>, String> {
    let project = s.project_mut().ok_or("No project is open")?;
    match scope.unwrap_or("scene") {
        "scene" => Ok(&mut project.world.screens),
        "game" => Ok(&mut project.settings.screens),
        other => Err(format!(
            "A screen layer is on the scene or the game, not \"{other}\""
        )),
    }
}

/// The world draws the file afresh: it isn't part of the document, so a
/// patch wouldn't carry it.
fn redraw(s: &mut AppState) {
    runtime_stale(s);
    sync_runtime(s);
}

/// `create-screen`: writes a new `.blockui` and answers its path. With
/// `fromInterface`, the open scene's own interface moves into it and the
/// scene draws the file instead.
pub(crate) fn create_screen(
    state: &SharedState,
    app: &AppHandle,
    path: Option<String>,
    name: Option<String>,
    from_interface: bool,
) -> Result<String, String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let path = match path.filter(|p| !p.trim().is_empty()) {
        Some(path) => checked_path(&path)?,
        None => new_screen_path(&dir, name.as_deref().unwrap_or("Screen")),
    };
    if dir.join(&path).exists() {
        return Err(format!("{path} already exists"));
    }
    let project = s.project().ok_or("No project is open")?;
    let screen = if from_interface {
        UiScreen::from_document(&project.world.interface)
    } else {
        UiScreen::default()
    };
    screen.lower()?;
    layers::write(&dir, &path, &screen)?;
    reindex(&dir);
    if from_interface {
        push_undo(&mut s);
        let project = s.project_mut().ok_or("No project is open")?;
        project.world.interface = Default::default();
        project.world.screens.retain(|l| l.screen != path);
        project.world.screens.insert(0, UiLayer::new(path.clone()));
        auto_save(&mut s);
        sync_runtime(&mut s);
    }
    emit(app, &mut s);
    Ok(path)
}

/// `read-screen`: a `.blockui` file's tree.
pub(crate) fn read_screen(state: &SharedState, path: String) -> Result<UiScreen, String> {
    let s = lock(state)?;
    let dir = project_dir(&s)?;
    layers::read(&dir, &checked_path(&path)?)
}

/// `write-screen`: replaces a `.blockui` file's tree, refusing one the
/// runtime can't draw. Every scene that lists it redraws.
pub(crate) fn write_screen(
    state: &SharedState,
    app: &AppHandle,
    path: String,
    screen: UiScreen,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let path = checked_path(&path)?;
    screen.lower()?;
    layers::write(&dir, &path, &screen)?;
    reindex(&dir);
    redraw(&mut s);
    emit(app, &mut s);
    Ok(())
}

/// `add-screen-layer`: draws a screen over the open scene (`scope=scene`)
/// or every scene (`scope=game`). Listing it again only moves it to the top
/// and sets `open`.
pub(crate) fn add_screen_layer(
    state: &SharedState,
    app: &AppHandle,
    path: String,
    scope: Option<String>,
    open: Option<bool>,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let path = checked_path(&path)?;
    layers::read(&dir, &path)?.lower()?;
    push_undo(&mut s);
    let list = list(&mut s, scope.as_deref())?;
    list.retain(|l| l.screen != path);
    list.push(UiLayer {
        open: open.unwrap_or(true),
        ..UiLayer::new(path)
    });
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(())
}

/// `remove-screen-layer`: stops drawing a screen; the file stays.
pub(crate) fn remove_screen_layer(
    state: &SharedState,
    app: &AppHandle,
    path: String,
    scope: Option<String>,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let path = checked_path(&path)?;
    if !list(&mut s, scope.as_deref())?
        .iter()
        .any(|l| l.screen == path)
    {
        return Err(format!("{path} isn't drawn there"));
    }
    push_undo(&mut s);
    list(&mut s, scope.as_deref())?.retain(|l| l.screen != path);
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(())
}
