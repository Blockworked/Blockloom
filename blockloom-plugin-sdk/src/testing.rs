//! Test a plugin without an editor: [`Harness`] starts it in-process through
//! the host's own loader, so ops, errors, logs and host services behave as
//! they do in a real run.
//!
//! ```ignore
//! let plugin = blockloom_plugin_sdk::harness!(Counter).unwrap();
//! assert_eq!(plugin.call("bump", json!({"by": 2})).unwrap()["count"], 2);
//! ```

use blockloom_plugin_api::abi::{EntryFn, Status};
use blockloom_plugin_api::manifest::Capability;
use blockloom_plugin_host::native::NativeModule;
use blockloom_plugin_host::services::HostServices;
use blockloom_plugin_host::storage::MemoryStore;
use serde_json::Value;
use std::sync::Arc;

/// A running plugin. Each harness has its own memory stores, so tests do not
/// see each other's saves.
pub struct Harness {
    module: NativeModule,
    services: HostServices,
}

impl Harness {
    /// Starts the plugin behind `entry` with no capabilities.
    pub fn from_entry(entry: EntryFn) -> Result<Harness, String> {
        Self::with_capabilities(entry, &[])
    }

    /// Starts it with the capabilities its manifest would be granted.
    pub fn with_capabilities(
        entry: EntryFn,
        capabilities: &[Capability],
    ) -> Result<Harness, String> {
        let services = HostServices::new("test")
            .with_project_store(Arc::new(MemoryStore::new()))
            .with_save_store(Arc::new(MemoryStore::new()));
        let module = unsafe {
            NativeModule::from_entry(
                entry,
                capabilities.iter().copied().collect(),
                services.for_plugin("test"),
            )
        }?;
        Ok(Harness { module, services })
    }

    /// Calls a JSON op. A failure names the op and the status the host saw.
    pub fn call(&self, op: &str, args: Value) -> Result<Value, String> {
        self.module.call_json(op, &args)
    }

    /// Calls an op over raw bytes and keeps the status.
    pub fn call_bytes(&self, op: &str, input: &[u8]) -> Result<Vec<u8>, Status> {
        self.module.call(op, input)
    }

    /// The lines the plugin has logged since the last call here.
    pub fn logs(&self) -> Vec<String> {
        self.module
            .take_logs()
            .into_iter()
            .map(|(_, line)| line)
            .collect()
    }

    /// The services the plugin sees, for a test that asks them directly.
    pub fn services(&self) -> &HostServices {
        &self.services
    }
}

/// Starts a [`Plugin`](crate::Plugin) type under a [`Harness`]. It defines the
/// entry function here, so it can be used any number of times in one test
/// binary (the exported symbol of `export_plugin!` can not). A second form
/// grants capabilities: `harness!(Counter, [Capability::ProjectStorage])`.
#[macro_export]
macro_rules! harness {
    ($plugin:ty) => {
        $crate::harness!($plugin, [])
    };
    ($plugin:ty, [$($capability:expr),* $(,)?]) => {{
        unsafe extern "C" fn entry(
            host: *const $crate::__private::abi::HostApi,
            out: *mut $crate::__private::abi::PluginApi,
        ) -> i32 {
            unsafe { $crate::__private::entry::<$plugin>(host, out) }
        }
        $crate::testing::Harness::with_capabilities(entry, &[$($capability),*])
    }};
}
