//! Modules compiled into this process by engine-tier plugins: a project
//! runtime's engine crate registers its entry here, and a loadout naming the
//! plugin as `Linked` starts it through the same C ABI a library would.

use std::collections::BTreeMap;
use std::sync::Mutex;

/// A module's entry function, as `export_plugin!` defines it.
pub type Entry = blockloom_plugin_api::abi::EntryFn;

static LINKED: Mutex<BTreeMap<String, Entry>> = Mutex::new(BTreeMap::new());

/// Makes `entry` the module of `plugin` for every world this process hosts.
pub fn register(plugin: &str, entry: Entry) {
    if let Ok(mut linked) = LINKED.lock() {
        linked.insert(plugin.to_string(), entry);
    }
}

/// The entry `plugin` registered, if this process links it.
pub fn entry(plugin: &str) -> Option<Entry> {
    LINKED.lock().ok()?.get(plugin).copied()
}
