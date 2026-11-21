//! The editor's default keys. A binding names a dispatch command (`undo`) or
//! one of the window's own actions (`editor.*`, which open a dialog or a
//! panel); the menu bar, the command palette and the window's shortcuts all
//! read this one table.

use serde::Serialize;

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

/// `editor-commands`: every dispatch command the shell knows plus the keymap.
pub fn editor_commands() -> serde_json::Value {
    serde_json::json!({ "commands": crate::shell::COMMANDS, "keymap": KEYMAP })
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
}
