//! What plugins cost and what they say about themselves.
//!
//! The host times every call it makes into a module ([`Diagnostics::record_call`]),
//! and a plugin adds its own counters, gauges, timings and markers through the
//! `diag.*` services. [`Diagnostics::snapshot`] is the JSON the Plugin Manager,
//! the shell and the profiler read; [`Diagnostics::metrics`] flattens it to
//! `plugins/<id>/...` numbers.

use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

/// How many markers a plugin keeps; older ones fall off.
pub const MAX_MARKERS: usize = 64;
/// How many distinct names of each kind a plugin may create.
pub const MAX_NAMES: usize = 256;

#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Timing {
    count: u64,
    total_ms: f64,
    max_ms: f64,
    last_ms: f64,
}

impl Timing {
    fn add(&mut self, ms: f64) {
        self.count += 1;
        self.total_ms += ms;
        self.max_ms = self.max_ms.max(ms);
        self.last_ms = ms;
    }

    fn json(&self) -> Value {
        json!({
            "count": self.count,
            "totalMs": self.total_ms,
            "maxMs": self.max_ms,
            "lastMs": self.last_ms,
            "avgMs": if self.count == 0 { 0.0 } else { self.total_ms / self.count as f64 },
        })
    }
}

#[derive(Default)]
struct Stats {
    calls: BTreeMap<String, Timing>,
    counters: BTreeMap<String, i64>,
    gauges: BTreeMap<String, f64>,
    timings: BTreeMap<String, Timing>,
    markers: VecDeque<(u64, String)>,
    errors: u64,
}

#[derive(Default)]
struct Inner {
    tick: u64,
    plugins: BTreeMap<String, Stats>,
}

#[derive(Default)]
pub struct Diagnostics {
    inner: Mutex<Inner>,
}

fn room<V>(map: &BTreeMap<String, V>, name: &str) -> Result<(), String> {
    if map.contains_key(name) || map.len() < MAX_NAMES {
        Ok(())
    } else {
        Err(format!("over the limit of {MAX_NAMES} names"))
    }
}

impl Diagnostics {
    fn with<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard)
    }

    /// The fixed tick markers are stamped with.
    pub fn set_tick(&self, tick: u64) {
        self.with(|i| i.tick = tick);
    }

    /// One call into a module's op took `took`.
    pub fn record_call(&self, plugin: &str, op: &str, took: Duration) {
        let ms = took.as_secs_f64() * 1000.0;
        self.with(|i| {
            let stats = i.plugins.entry(plugin.to_string()).or_default();
            // Hook and block names are bounded by the schema; stay safe regardless.
            if room(&stats.calls, op).is_ok() {
                stats.calls.entry(op.to_string()).or_default().add(ms);
            }
        });
    }

    pub fn error(&self, plugin: &str) {
        self.with(|i| i.plugins.entry(plugin.to_string()).or_default().errors += 1);
    }

    pub fn count(&self, plugin: &str, name: &str, delta: i64) -> Result<(), String> {
        self.with(|i| {
            let stats = i.plugins.entry(plugin.to_string()).or_default();
            room(&stats.counters, name)?;
            let c = stats.counters.entry(name.to_string()).or_default();
            *c = c.saturating_add(delta);
            Ok(())
        })
    }

    pub fn gauge(&self, plugin: &str, name: &str, value: f64) -> Result<(), String> {
        if !value.is_finite() {
            return Err("a gauge must be a finite number".to_string());
        }
        self.with(|i| {
            let stats = i.plugins.entry(plugin.to_string()).or_default();
            room(&stats.gauges, name)?;
            stats.gauges.insert(name.to_string(), value);
            Ok(())
        })
    }

    pub fn timing(&self, plugin: &str, name: &str, ms: f64) -> Result<(), String> {
        if !ms.is_finite() || ms < 0.0 {
            return Err("a timing must be a non-negative number of milliseconds".to_string());
        }
        self.with(|i| {
            let stats = i.plugins.entry(plugin.to_string()).or_default();
            room(&stats.timings, name)?;
            stats.timings.entry(name.to_string()).or_default().add(ms);
            Ok(())
        })
    }

    pub fn marker(&self, plugin: &str, name: &str) {
        self.with(|i| {
            let tick = i.tick;
            let stats = i.plugins.entry(plugin.to_string()).or_default();
            if stats.markers.len() == MAX_MARKERS {
                stats.markers.pop_front();
            }
            stats
                .markers
                .push_back((tick, name.chars().take(80).collect()));
        });
    }

    /// Forgets everything, as a new run does.
    pub fn reset(&self) {
        self.with(|i| *i = Inner::default());
    }

    pub fn snapshot(&self) -> Value {
        self.with(|i| {
            let plugins: serde_json::Map<String, Value> = i
                .plugins
                .iter()
                .map(|(id, s)| {
                    let calls: serde_json::Map<String, Value> =
                        s.calls.iter().map(|(k, v)| (k.clone(), v.json())).collect();
                    let timings: serde_json::Map<String, Value> = s
                        .timings
                        .iter()
                        .map(|(k, v)| (k.clone(), v.json()))
                        .collect();
                    let total: f64 = s.calls.values().map(|t| t.total_ms).sum();
                    (
                        id.clone(),
                        json!({
                            "totalMs": total,
                            "errors": s.errors,
                            "calls": calls,
                            "counters": s.counters,
                            "gauges": s.gauges,
                            "timings": timings,
                            "markers": s.markers.iter()
                                .map(|(tick, name)| json!({"tick": tick, "name": name}))
                                .collect::<Vec<_>>(),
                        }),
                    )
                })
                .collect();
            json!({"tick": i.tick, "plugins": plugins})
        })
    }

    /// Flat `(name, value)` readings for a profiler: per plugin, the time its
    /// calls took in total and last, its errors, and its own counters and gauges.
    pub fn metrics(&self) -> Vec<(String, f64)> {
        self.with(|i| {
            let mut out = Vec::new();
            for (id, s) in &i.plugins {
                out.push((
                    format!("plugins/{id}/call_ms"),
                    s.calls.values().map(|t| t.last_ms).sum(),
                ));
                out.push((format!("plugins/{id}/errors"), s.errors as f64));
                for (name, v) in &s.counters {
                    out.push((format!("plugins/{id}/{name}"), *v as f64));
                }
                for (name, v) in &s.gauges {
                    out.push((format!("plugins/{id}/{name}"), *v));
                }
            }
            out
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_counters_gauges_and_markers_are_kept_per_plugin() {
        let d = Diagnostics::default();
        d.record_call("a", "hook.tick", Duration::from_millis(2));
        d.record_call("a", "hook.tick", Duration::from_millis(4));
        d.count("a", "meshes", 3).unwrap();
        d.count("a", "meshes", -1).unwrap();
        d.gauge("a", "chunks", 12.0).unwrap();
        d.timing("a", "mesh", 1.5).unwrap();
        d.set_tick(7);
        d.marker("a", "regen");
        d.error("b");
        let s = d.snapshot();
        assert_eq!(s["plugins"]["a"]["counters"]["meshes"], 2);
        assert_eq!(s["plugins"]["a"]["gauges"]["chunks"], 12.0);
        assert_eq!(s["plugins"]["a"]["calls"]["hook.tick"]["count"], 2);
        assert!(
            s["plugins"]["a"]["calls"]["hook.tick"]["maxMs"]
                .as_f64()
                .unwrap()
                >= 4.0
        );
        assert_eq!(s["plugins"]["a"]["markers"][0]["tick"], 7);
        assert_eq!(s["plugins"]["b"]["errors"], 1);
        assert!(
            d.metrics()
                .iter()
                .any(|(k, v)| k == "plugins/a/meshes" && *v == 2.0)
        );
    }

    #[test]
    fn bad_numbers_and_too_many_names_are_refused() {
        let d = Diagnostics::default();
        assert!(d.gauge("a", "x", f64::NAN).is_err());
        assert!(d.timing("a", "x", -1.0).is_err());
        for i in 0..MAX_NAMES {
            d.count("a", &format!("c{i}"), 1).unwrap();
        }
        assert!(d.count("a", "one-too-many", 1).is_err());
        d.count("a", "c0", 1).unwrap();
        for i in 0..MAX_MARKERS + 10 {
            d.marker("a", &i.to_string());
        }
        assert_eq!(
            d.snapshot()["plugins"]["a"]["markers"]
                .as_array()
                .unwrap()
                .len(),
            MAX_MARKERS
        );
        d.reset();
        assert_eq!(d.snapshot()["plugins"].as_object().unwrap().len(), 0);
    }
}
