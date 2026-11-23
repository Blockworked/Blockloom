//! The editor's default keys. A binding names a dispatch command (`undo`) or
//! one of the window's own actions (`editor.*`, which open a dialog or a
//! panel); the menu bar, the command palette and the window's shortcuts all
//! read this one table, with the user's own rebindings laid over it
//! (`<data dir>/blockloom/keymap.json`, per user, never in a project).

use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Binding {
    pub action: &'static str,
    /// Qt key sequences, the first one shown in menus.
    pub keys: &'static [&'static str],
}

const fn bind(action: &'static str, keys: &'static [&'static str]) -> Binding {
    Binding { action, keys }
}

pub const KEYMAP: &[Binding] = &[
    bind("save_project", &["Ctrl+S"]),
    bind("undo", &["Ctrl+Z"]),
    bind("redo", &["Ctrl+Shift+Z", "Ctrl+Y"]),
    bind("run_project", &["F5"]),
    bind("stop_project", &["Shift+F5"]),
    bind("step_project", &["F10"]),
    bind("close_project", &["Ctrl+Shift+W"]),
    bind("editor.palette", &["Ctrl+Shift+P", "Ctrl+K"]),
    bind("editor.build", &["Ctrl+B"]),
    bind("editor.projectSettings", &["Ctrl+,"]),
    bind("editor.appSettings", &["Ctrl+Alt+,"]),
    bind("editor.plugins", &["Ctrl+Shift+X"]),
    bind("editor.import", &["Ctrl+O"]),
    bind("editor.showCode", &["Ctrl+1"]),
    bind("editor.showGame", &["Ctrl+2"]),
    bind("editor.showInterface", &["Ctrl+3"]),
    bind("editor.showDevices", &["Ctrl+4"]),
    bind("editor.split", &["Ctrl+\\"]),
    bind("editor.problems", &["Ctrl+Shift+M"]),
    bind("editor.runLog", &["Ctrl+`"]),
    bind("editor.assets", &["Ctrl+Shift+A"]),
];

/// The user's rebindings: action to keys, an empty list unbinding it.
pub type Overrides = BTreeMap<String, Vec<String>>;

fn overrides_path() -> PathBuf {
    blockloom_core::project::data_dir()
        .join("blockloom")
        .join("keymap.json")
}

/// The saved rebindings; a missing or unreadable file is none.
pub fn overrides() -> Overrides {
    std::fs::read(overrides_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save(overrides: &Overrides) -> Result<(), String> {
    let path = overrides_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let text = serde_json::to_vec_pretty(overrides).map_err(|e| e.to_string())?;
    blockloom_core::files::write_atomic(&path, &text)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// The keymap in force: the defaults with `overrides` laid over them, plus
/// any action that only an override binds.
pub fn effective(overrides: &Overrides) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = KEYMAP
        .iter()
        .map(|b| {
            let keys = overrides
                .get(b.action)
                .cloned()
                .unwrap_or_else(|| b.keys.iter().map(|k| k.to_string()).collect());
            (b.action.to_string(), keys)
        })
        .collect();
    for (action, keys) in overrides {
        if !KEYMAP.iter().any(|b| b.action == action) {
            out.push((action.clone(), keys.clone()));
        }
    }
    out
}

fn known(action: &str) -> bool {
    action.starts_with("editor.") || crate::shell::COMMANDS.iter().any(|c| c.cmd == action)
}

/// A key sequence as Qt spells it: modifiers then one key, joined by `+`.
fn check_key(key: &str) -> Result<(), String> {
    let parts: Vec<&str> = key.split('+').collect();
    let bad = key.trim() != key || parts.iter().any(|p| p.trim().is_empty() || p.trim() != *p);
    // "Ctrl++" is the plus key.
    if bad && !key.ends_with("++") {
        return Err(format!("\"{key}\" isn't a key like Ctrl+Shift+K"));
    }
    Ok(())
}

/// Whether `action` may take `keys` over the bindings already in force.
fn check_binding(overrides: &Overrides, action: &str, keys: &[String]) -> Result<(), String> {
    for key in keys {
        check_key(key)?;
    }
    for (other, held) in effective(overrides) {
        if other != action
            && let Some(key) = held.iter().find(|k| keys.contains(k))
        {
            return Err(format!("{key} already runs {other}; rebind that first"));
        }
    }
    Ok(())
}

/// `set-keybinding`: binds `action` to `keys` (an empty list unbinds it, none
/// puts the default back). A key another action already holds is refused,
/// naming that action, so one press never means two things.
pub fn set_keybinding(
    action: &str,
    keys: Option<Vec<String>>,
) -> Result<serde_json::Value, String> {
    if !known(action) {
        return Err(format!(
            "\"{action}\" is neither a command nor an editor action"
        ));
    }
    let mut overrides = overrides();
    match keys {
        None => {
            overrides.remove(action);
        }
        Some(keys) => {
            check_binding(&overrides, action, &keys)?;
            overrides.insert(action.to_string(), keys);
        }
    }
    save(&overrides)?;
    Ok(editor_commands())
}

/// `reset-keymap`: forgets every rebinding.
pub fn reset_keymap() -> Result<serde_json::Value, String> {
    save(&Overrides::new())?;
    Ok(editor_commands())
}

/// `editor-commands`: every dispatch command the shell knows, the keymap in
/// force and the defaults it started from.
pub fn editor_commands() -> serde_json::Value {
    let keymap: Vec<serde_json::Value> = effective(&overrides())
        .into_iter()
        .map(|(action, keys)| serde_json::json!({ "action": action, "keys": keys }))
        .collect();
    serde_json::json!({ "commands": crate::shell::COMMANDS, "keymap": keymap, "defaults": KEYMAP })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_binding_names_a_command_and_no_key_is_bound_twice() {
        let mut seen = std::collections::HashSet::new();
        for b in KEYMAP {
            assert!(
                b.action.starts_with("editor.")
                    || crate::shell::COMMANDS.iter().any(|c| c.cmd == b.action),
                "{} is neither a command nor an editor action",
                b.action
            );
            for key in b.keys {
                assert!(seen.insert(*key), "{key} is bound twice");
            }
        }
    }

    #[test]
    fn an_override_replaces_the_default_and_a_taken_key_is_refused() {
        let mut overrides = Overrides::new();
        overrides.insert("undo".into(), vec!["Ctrl+U".into()]);
        overrides.insert("editor.split".into(), vec![]);
        let keys = effective(&overrides);
        let of = |a: &str| keys.iter().find(|(x, _)| x == a).unwrap().1.clone();
        assert_eq!(of("undo"), vec!["Ctrl+U".to_string()]);
        assert!(of("editor.split").is_empty());
        assert_eq!(
            of("redo"),
            vec!["Ctrl+Shift+Z".to_string(), "Ctrl+Y".to_string()]
        );
        let taken = check_binding(&overrides, "redo", &["Ctrl+S".into()]).unwrap_err();
        assert!(taken.contains("save_project"), "{taken}");
        // A key freed by an override is free again.
        assert!(check_binding(&overrides, "redo", &["Ctrl+Z".into()]).is_ok());
        assert!(check_binding(&overrides, "redo", &["Ctrl+".into()]).is_err());
        assert!(check_binding(&overrides, "redo", &["Ctrl++".into()]).is_ok());
    }
}
