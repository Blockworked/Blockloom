//! A plugin's loaded code, whichever tier it is: one handle for the editor's
//! command calls and a running world's hooks.

use crate::native::{NativeModule, ServiceFn};
use crate::portable::PortableModule;
use crate::services::HostServices;
use blockloom_plugin_api::loadout::CodeRuntime;
use serde_json::Value;

pub enum CodeModule {
    Native(NativeModule),
    Portable(Box<PortableModule>),
}

impl CodeModule {
    /// Opens the library or module `runtime` names, under its capabilities.
    pub fn load(runtime: &CodeRuntime, engine: &str) -> Result<CodeModule, String> {
        Self::load_with(runtime, "", &HostServices::new(engine))
    }

    /// [`CodeModule::load`] for `plugin`, with the services `host` gives it.
    pub fn load_with(
        runtime: &CodeRuntime,
        plugin: &str,
        host: &HostServices,
    ) -> Result<CodeModule, String> {
        Self::open(runtime, host.for_plugin(plugin))
    }

    /// Opens the module with exactly these services.
    pub fn open(runtime: &CodeRuntime, services: Box<ServiceFn>) -> Result<CodeModule, String> {
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

    /// Calls `op` over raw bytes. `limit_ms` is the work a portable module may
    /// spend on this one call; a native library is trusted and not timed.
    pub fn call_bytes(&mut self, op: &str, input: &[u8], limit_ms: u32) -> Result<Vec<u8>, String> {
        match self {
            CodeModule::Native(m) => m
                .call(op, input)
                .map_err(|status| format!("{op}: {status:?}")),
            CodeModule::Portable(m) => m.call_limited(op, input, limit_ms),
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
