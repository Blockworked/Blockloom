//! Tally: named counters that live as long as the module does.
//!
//! The whole plugin is one struct and one macro call. The same source builds
//! as a native library and as a WebAssembly module.
//!
//! Ops: `add` (`name`, `by`), `get` (`name`), `all`, `reset` and `engine`
//! (what the host says about itself, through a host service). When a running
//! game hosts the module it also hears `world.start` (a fresh run) and
//! `world.stop`, whose answer carries an effect: a line in the run log. In a
//! run, `add` also fires the `changed` event (its `name` and new count) for the
//! "when tally changes" hat, and `get` answers `value` for the "tally of"
//! reporter.

use blockloom_plugin_sdk::{Error, Host, Plugin, Value, export_plugin, json};
use std::collections::BTreeMap;

#[derive(Default)]
struct Tally {
    counts: BTreeMap<String, i64>,
    /// A running game hosts this module, so it has hats to start.
    hosted: bool,
}

impl Tally {
    fn name(args: &Value) -> Result<String, Error> {
        match args["name"].as_str() {
            Some(name) if !name.is_empty() => Ok(name.to_string()),
            _ => Err(Error::bad_argument("name must be some text")),
        }
    }
}

impl Plugin for Tally {
    fn start(host: &Host) -> Result<Self, Error> {
        host.info("tally started");
        Ok(Tally::default())
    }

    fn call_json(&mut self, host: &Host, op: &str, args: Value) -> Result<Value, Error> {
        match op {
            "add" => {
                let name = Tally::name(&args)?;
                let by = args["by"].as_i64().unwrap_or(1);
                let count = self.counts.entry(name.clone()).or_insert(0);
                *count = count.saturating_add(by);
                let count = *count;
                if !self.hosted {
                    return Ok(json!({ "count": count }));
                }
                Ok(json!({
                    "count": count,
                    "effects": [{"effect": "event", "name": "changed", "args": [name, count]}],
                }))
            }
            "get" => {
                let name = Tally::name(&args)?;
                let count = self.counts.get(&name).copied().unwrap_or(0);
                Ok(json!({ "count": count, "value": count }))
            }
            "all" => Ok(json!({ "counts": self.counts })),
            "reset" => {
                self.counts.clear();
                Ok(Value::Null)
            }
            "engine" => host.call_json("host.version", &Value::Null),
            "world.start" => {
                self.counts.clear();
                self.hosted = true;
                Ok(Value::Null)
            }
            "world.stop" if self.counts.is_empty() => Ok(Value::Null),
            "world.stop" => {
                let totals: Vec<String> = self
                    .counts
                    .iter()
                    .map(|(name, count)| format!("{name} = {count}"))
                    .collect();
                Ok(json!({"effects": [{
                    "effect": "say",
                    "text": format!("final tally: {}", totals.join(", ")),
                }]}))
            }
            _ => Err(Error::unsupported(op)),
        }
    }
}

export_plugin!(Tally);
