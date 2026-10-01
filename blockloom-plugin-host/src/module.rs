//! A plugin's loaded code, whichever tier it is: one handle for the editor's
//! command calls and a running world's hooks.

use crate::native::{NativeModule, default_services};
use crate::portable::PortableModule;
use blockloom_plugin_api::loadout::CodeRuntime;
use serde_json::Value;

pub enum CodeModule {
    Native(NativeModule),
    Portable(Box<PortableModule>),
}

impl CodeModule {
    /// Opens the library or module `runtime` names, under its capabilities.
    pub fn load(runtime: &CodeRuntime, engine: &str) -> Result<CodeModule, String> {
        let services = default_services(engine.to_string());
        match runtime {
            CodeRuntime::Native(library) => {
                NativeModule::load(&library.path, library.capabilities.clone(), services)
                    .map(CodeModule::Native)
            }
            CodeRuntime::Portable(library) => crate::files::read(&library.path)
                .map_err(|e| format!("{}: {e}", library.path.display()))
                .and_then(|wasm| {
                    PortableModule::load(
                        &wasm,
                        &library.entry,
                        library.capabilities.clone(),
                        services,
                    )
                })
                .map(|m| CodeModule::Portable(Box::new(m))),
        }
    }

    /// Calls `op` with a JSON value and parses the JSON answer. A plugin
    /// that has no such op fails with [`is_unsupported`].
    pub fn call_json(&mut self, op: &str, input: &Value) -> Result<Value, String> {
        match self {
            CodeModule::Native(m) => m.call_json(op, input),
            CodeModule::Portable(m) => m.call_json(op, input),
        }
    }

    /// What the plugin has logged through the host since the last call here.
    pub fn take_logs(&mut self) -> Vec<(u32, String)> {
        match self {
            CodeModule::Native(m) => m.take_logs(),
            CodeModule::Portable(m) => m.take_logs(),
        }
    }

    /// A portable module that ran out of budget or trapped is not reused.
    pub fn is_stopped(&self) -> bool {
        matches!(self, CodeModule::Portable(m) if m.is_stopped())
    }
}

/// Whether a failed call is the module saying it has no such op.
pub fn is_unsupported(error: &str) -> bool {
    error.ends_with(": Unsupported")
}
