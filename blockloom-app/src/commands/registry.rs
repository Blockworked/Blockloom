//! The type registry over the open project: listing every component and
//! setting type, built-in and plugin, and the generic edits over them.

use super::{EditSession, auto_save, emit, lock, push_undo_for, sync_runtime};
use crate::AppHandle;
use crate::state::SharedState;
use blockloom_core::components::ActorComponent;
use blockloom_core::registry;
use blockloom_core::scene::{Mode, World};
use blockloom_core::settings::GameSettings;
use serde_json::{Value, json};

/// `type-registry`: every built-in component kind and world setting with its
/// fields and defaults, plus the open project's plugin component and
/// resource types, in one listing.
pub(crate) fn type_registry(state: &SharedState) -> Result<Value, String> {
    let s = lock(state)?;
    let mode = s.project().map(|p| p.world.mode).unwrap_or(Mode::ThreeD);
    let mut out = registry::to_json(mode);
    let plugins = s
        .open
        .as_ref()
        .map(|open| super::plugins::types_json(&open.plugins))
        .unwrap_or_default();
    out["plugins"] = Value::Array(plugins);
    out["mode"] = json!(mode);
    Ok(out)
}

/// What a `set-property` target names.
enum Target {
    World,
    Settings,
    Component { actor: String, name: String },
}

fn parse_target(target: &str) -> Result<Target, String> {
    match target {
        "world" => return Ok(Target::World),
        "settings" | "project" => return Ok(Target::Settings),
        _ => {}
    }
    let rest = target
        .strip_prefix("actor:")
        .ok_or("A target is \"world\", \"settings\" or \"actor:<id>/<Component>\"")?;
    let (actor, name) = rest
        .split_once('/')
        .ok_or("An actor target names its component: actor:<id>/<Component>")?;
    Ok(Target::Component {
        actor: actor.to_string(),
        name: name.to_string(),
    })
}

/// Runs `edit` on the open scene's world settings as one undo step; with a
/// `session` key, further writes to the same key coalesce into it.
pub(crate) fn write_world(
    state: &SharedState,
    app: &AppHandle,
    session: Option<&str>,
    edit: impl FnOnce(&World) -> Result<World, String>,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let next = edit(&s.project().ok_or("No project is open")?.world)?;
    push_undo_for(
        &mut s,
        session.map(|key| EditSession::Comment {
            comment_id: format!("world:{key}"),
        }),
    );
    if let Some(project) = s.project_mut() {
        project.world = next;
    }
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(())
}

/// One checked write to the project's game settings, coalesced per path.
fn write_settings(
    state: &SharedState,
    app: &AppHandle,
    path: &str,
    value: Value,
) -> Result<(), String> {
    settings_edit(state, app, Some(path), |current| {
        registry::set_settings_property(current, path, value)
    })
}

/// Runs `edit` on the project's game settings as one undo step.
pub(crate) fn write_settings_with(
    state: &SharedState,
    app: &AppHandle,
    edit: impl FnOnce(&GameSettings) -> Result<GameSettings, String>,
) -> Result<(), String> {
    settings_edit(state, app, None, edit)
}

fn settings_edit(
    state: &SharedState,
    app: &AppHandle,
    session: Option<&str>,
    edit: impl FnOnce(&GameSettings) -> Result<GameSettings, String>,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let next = edit(&s.project().ok_or("No project is open")?.settings)?;
    push_undo_for(
        &mut s,
        session.map(|path| EditSession::Comment {
            comment_id: format!("settings:{path}"),
        }),
    );
    if let Some(project) = s.project_mut() {
        project.settings = next;
    }
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(())
}

/// `set-property`: one checked write anywhere the registry describes. A
/// built-in component goes through `set_actor_component`, so it gets that
/// command's checks, normalization and undo; a plugin component through
/// `set_plugin_component`, validated by its plugin's schema.
pub(crate) fn set_property(
    state: &SharedState,
    app: &AppHandle,
    target: String,
    path: String,
    value: Value,
) -> Result<(), String> {
    match parse_target(&target)? {
        Target::World => write_world(state, app, Some(&path), |world| {
            registry::set_world_property(world, &path, value)
        }),
        Target::Settings => write_settings(state, app, &path, value),
        Target::Component { actor, name } => {
            if let Some(qualified) = name.strip_prefix("plugin:") {
                let payload = {
                    let s = lock(state)?;
                    let record = s
                        .project()
                        .and_then(|p| p.actor(&actor))
                        .ok_or("Actor not found")?
                        .components
                        .plugin_record(qualified)
                        .ok_or_else(|| format!("This actor has no \"{qualified}\" component"))?;
                    // The plugin's own schema checks the whole payload.
                    let mut payload = record.payload.clone();
                    *registry::slot_at(&mut payload, &registry::split_path(&path))? = value;
                    payload
                };
                return super::plugins::set_plugin_component(
                    state,
                    app,
                    actor,
                    qualified.to_string(),
                    payload,
                );
            }
            let next = {
                let s = lock(state)?;
                let component = s
                    .project()
                    .and_then(|p| p.actor(&actor))
                    .ok_or("Actor not found")?
                    .components
                    .get(&name)
                    .ok_or_else(|| format!("This actor has no \"{name}\" component"))?
                    .clone();
                registry::set_component_property(&component, &path, value)?
            };
            super::set_actor_component(state, app, actor, name, next)
        }
    }
}

/// `add-component`: a fresh component of any registered kind, its defaults
/// from the registry. `plugin:<plugin>/<Type>` adds a plugin component.
pub(crate) fn add_component(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    kind: String,
) -> Result<String, String> {
    if let Some(qualified) = kind.strip_prefix("plugin:") {
        return super::plugins::add_plugin_component(
            state,
            app,
            actor_id,
            qualified.to_string(),
            None,
        );
    }
    let mode = {
        let s = lock(state)?;
        s.project().ok_or("No project is open")?.world.mode
    };
    let component: ActorComponent = match kind.as_str() {
        "Script" => return Err("A script needs a file: use add-script".into()),
        "Collider" => return Err("A collider needs a shape: use add-collider".into()),
        _ => registry::default_component(&kind, mode)
            .ok_or_else(|| format!("\"{kind}\" isn't a component kind"))?,
    };
    super::add_actor_component(state, app, actor_id, component)
}
