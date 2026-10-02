//! The services a module can ask the host for with `blockloom.call`.
//!
//! A service is named `area.verb`, takes JSON and answers JSON. A refusal the
//! plugin can act on (a bad key, a store over its limit) comes back as
//! `{"error": "..."}`; an unknown service or one the plugin's capabilities
//! do not allow fails the call itself (see [`crate::native::CapabilityGate`]).
//!
//! Built in: `host.version`; `rng.*` (a seeded, replayable generator);
//! `storage.*` and `save.*` (see [`crate::storage`]); `diag.*` (see
//! [`crate::diagnostics`]). A host adds more, such as `physics.*` and `nav.*`
//! in a running world, with a [`ServiceProvider`].

use crate::diagnostics::Diagnostics;
use crate::jobs::JobTable;
use crate::native::ServiceFn;
use crate::storage::{BlobOp, BlobStore, StoreLimits, check_key, commit_for, hash_of, is_hash};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use blockloom_plugin_api::abi::{ABI_VERSION, Status};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

/// Answers services a host knows and the built-ins do not.
pub trait ServiceProvider: Send + Sync {
    /// `None` when the provider does not know the service.
    fn call(&self, plugin: &str, service: &str, input: &Value) -> Option<Result<Value, String>>;
}

#[derive(Clone)]
pub struct HostServices {
    engine: String,
    project: Option<Arc<dyn BlobStore>>,
    saves: Option<Arc<dyn BlobStore>>,
    limits: StoreLimits,
    providers: Vec<Arc<dyn ServiceProvider>>,
    pub diagnostics: Arc<Diagnostics>,
    /// The jobs plugins have started; the world steps them (see [`crate::jobs`]).
    pub jobs: Arc<Mutex<JobTable>>,
}

impl HostServices {
    pub fn new(engine: &str) -> Self {
        Self {
            engine: engine.to_string(),
            project: None,
            saves: None,
            limits: StoreLimits::default(),
            providers: Vec::new(),
            diagnostics: Arc::new(Diagnostics::default()),
            jobs: Arc::new(Mutex::new(JobTable::default())),
        }
    }

    /// The store for project blobs.
    pub fn with_project_store(mut self, store: Arc<dyn BlobStore>) -> Self {
        self.project = Some(store);
        self
    }

    /// The store for player saves.
    pub fn with_save_store(mut self, store: Arc<dyn BlobStore>) -> Self {
        self.saves = Some(store);
        self
    }

    pub fn with_limits(mut self, limits: StoreLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn with_provider(mut self, provider: Arc<dyn ServiceProvider>) -> Self {
        self.providers.push(provider);
        self
    }

    pub fn with_diagnostics(mut self, diagnostics: Arc<Diagnostics>) -> Self {
        self.diagnostics = diagnostics;
        self
    }

    pub fn engine(&self) -> &str {
        &self.engine
    }

    pub fn project_store(&self) -> Option<&Arc<dyn BlobStore>> {
        self.project.as_ref()
    }

    pub fn save_store(&self) -> Option<&Arc<dyn BlobStore>> {
        self.saves.as_ref()
    }

    /// Answers one call as `plugin`. Errors a plugin can act on are `Err`.
    pub fn call(
        &self,
        plugin: &str,
        service: &str,
        input: &Value,
    ) -> Option<Result<Value, String>> {
        let (area, verb) = service.split_once('.')?;
        let answer = match area {
            "host" => match verb {
                "version" => Ok(json!({"engine": self.engine, "abi": ABI_VERSION})),
                _ => return None,
            },
            "rng" => rng(verb, input),
            "storage" => store_call(
                self.project.as_deref(),
                "project storage",
                plugin,
                verb,
                input,
                &self.limits,
            ),
            "save" => store_call(
                self.saves.as_deref(),
                "player saves",
                plugin,
                verb,
                input,
                &self.limits,
            ),
            "diag" => diag(&self.diagnostics, plugin, verb, input),
            "jobs" => return crate::jobs::call(&self.jobs, plugin, verb, input),
            _ => {
                return self
                    .providers
                    .iter()
                    .find_map(|p| p.call(plugin, service, input));
            }
        };
        match answer {
            Err(e) if e == UNKNOWN => None,
            other => Some(other),
        }
    }

    /// The service function a module for `plugin` is opened with.
    pub fn for_plugin(&self, plugin: &str) -> Box<ServiceFn> {
        let this = self.clone();
        let plugin = plugin.to_string();
        Box::new(move |name, input| {
            let input: Value = if input.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(input).map_err(|_| Status::BadArgument)?
            };
            match this.call(&plugin, name, &input) {
                None => Err(Status::Unsupported),
                Some(Ok(v)) => serde_json::to_vec(&v).map_err(|_| Status::Error),
                Some(Err(message)) => {
                    this.diagnostics.error(&plugin);
                    serde_json::to_vec(&json!({"error": message})).map_err(|_| Status::Error)
                }
            }
        })
    }
}

const UNKNOWN: &str = "\u{0}unknown";

// ─── rng ────────────────────────────────────────────────────────────────────

fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A number in `[0, 1)` for a seed and an index, the same on every machine.
pub fn unit(seed: u64, index: u64) -> f64 {
    (splitmix(seed ^ splitmix(index)) >> 11) as f64 / (1u64 << 53) as f64
}

fn rng(verb: &str, input: &Value) -> Result<Value, String> {
    let seed = input.get("seed").and_then(Value::as_u64).unwrap_or(0);
    let index = input.get("index").and_then(Value::as_u64).unwrap_or(0);
    match verb {
        // `{seed, index}` -> one value; `count` gives that many from `index` on.
        "unit" => {
            let count = input.get("count").and_then(Value::as_u64);
            match count {
                None => Ok(json!({"value": unit(seed, index)})),
                Some(n) if n <= 65_536 => Ok(json!({
                    "values": (0..n).map(|i| unit(seed, index + i)).collect::<Vec<_>>()
                })),
                Some(_) => Err("count is over 65536".to_string()),
            }
        }
        // `{seed, index, min, max}` -> an integer in `[min, max]`.
        "range" => {
            let min = input
                .get("min")
                .and_then(Value::as_i64)
                .ok_or("min is a whole number")?;
            let max = input
                .get("max")
                .and_then(Value::as_i64)
                .ok_or("max is a whole number")?;
            if max < min {
                return Err("max is below min".to_string());
            }
            let span = (max as i128 - min as i128 + 1) as f64;
            let v = (unit(seed, index) * span).floor() as i128;
            Ok(json!({"value": (min as i128 + v).min(max as i128) as i64}))
        }
        _ => Err(UNKNOWN.to_string()),
    }
}

// ─── storage and saves ──────────────────────────────────────────────────────

fn payload(input: &Value) -> Result<Vec<u8>, String> {
    if let Some(text) = input.get("text").and_then(Value::as_str) {
        return Ok(text.as_bytes().to_vec());
    }
    match input.get("data").and_then(Value::as_str) {
        Some(data) => B64
            .decode(data)
            .map_err(|e| format!("data is not base64: {e}")),
        None => Err("give `text` or base64 `data`".to_string()),
    }
}

fn encode(data: &[u8], as_text: bool) -> Result<Value, String> {
    let hash = hash_of(data);
    if as_text {
        let text =
            String::from_utf8(data.to_vec()).map_err(|_| "the blob is not text".to_string())?;
        Ok(json!({"found": true, "size": data.len(), "hash": hash, "text": text}))
    } else {
        Ok(json!({"found": true, "size": data.len(), "hash": hash, "data": B64.encode(data)}))
    }
}

fn key_of(input: &Value) -> Result<&str, String> {
    let key = input
        .get("key")
        .and_then(Value::as_str)
        .ok_or("give a `key`")?;
    check_key(key)?;
    Ok(key)
}

fn store_call(
    store: Option<&dyn BlobStore>,
    what: &str,
    plugin: &str,
    verb: &str,
    input: &Value,
    limits: &StoreLimits,
) -> Result<Value, String> {
    if !matches!(
        verb,
        "read" | "write" | "delete" | "list" | "commit" | "put" | "get" | "exists"
    ) {
        return Err(UNKNOWN.to_string());
    }
    let store = store.ok_or_else(|| format!("no {what} here"))?;
    let full = |key: &str| format!("{plugin}/{key}");
    let as_text = input.get("as").and_then(Value::as_str) == Some("text");
    match verb {
        "read" => match store.read(&full(key_of(input)?))? {
            Some(data) => encode(&data, as_text),
            None => Ok(json!({"found": false})),
        },
        "exists" => Ok(json!({"found": store.read(&full(key_of(input)?))?.is_some()})),
        "write" => {
            let key = key_of(input)?.to_string();
            let data = payload(input)?;
            let hash = hash_of(&data);
            let size = data.len();
            commit_for(store, plugin, &[BlobOp::Write { key, data }], limits)?;
            Ok(json!({"hash": hash, "size": size}))
        }
        "delete" => {
            commit_for(
                store,
                plugin,
                &[BlobOp::Delete {
                    key: key_of(input)?.to_string(),
                }],
                limits,
            )?;
            Ok(json!({}))
        }
        "list" => {
            let prefix = input.get("prefix").and_then(Value::as_str).unwrap_or("");
            let base = full("");
            let keys: Vec<Value> = store
                .list(&format!("{base}{prefix}"))?
                .into_iter()
                .filter_map(|(key, size)| {
                    key.strip_prefix(&base)
                        .map(|k| json!({"key": k, "size": size}))
                })
                .collect();
            Ok(json!({"keys": keys}))
        }
        // All of `ops` or none: `[{"op": "write", "key", "text"|"data"}, {"op": "delete", "key"}]`.
        "commit" => {
            let ops = input
                .get("ops")
                .and_then(Value::as_array)
                .ok_or("give `ops`")?;
            if ops.len() > 1024 {
                return Err("over 1024 ops in one commit".to_string());
            }
            let mut parsed = Vec::new();
            for op in ops {
                let key = key_of(op)?.to_string();
                match op.get("op").and_then(Value::as_str) {
                    Some("write") => parsed.push(BlobOp::Write {
                        key,
                        data: payload(op)?,
                    }),
                    Some("delete") => parsed.push(BlobOp::Delete { key }),
                    _ => return Err("an op is `write` or `delete`".to_string()),
                }
            }
            commit_for(store, plugin, &parsed, limits)?;
            Ok(json!({"applied": parsed.len()}))
        }
        // Content-addressed: the key is the sha256 of the bytes.
        "put" => {
            let data = payload(input)?;
            let hash = hash_of(&data);
            let key = format!("blobs/{hash}");
            if store.read(&full(&key))?.is_none() {
                commit_for(
                    store,
                    plugin,
                    &[BlobOp::Write {
                        key,
                        data: data.clone(),
                    }],
                    limits,
                )?;
            }
            Ok(json!({"hash": hash, "size": data.len()}))
        }
        "get" => {
            let hash = input
                .get("hash")
                .and_then(Value::as_str)
                .ok_or("give a `hash`")?;
            if !is_hash(hash) {
                return Err("a hash is 64 hex digits".to_string());
            }
            match store.read(&full(&format!("blobs/{hash}")))? {
                Some(data) if hash_of(&data) == hash => encode(&data, as_text),
                Some(_) => Err(format!("blob {hash} does not match its hash")),
                None => Ok(json!({"found": false})),
            }
        }
        _ => Err(UNKNOWN.to_string()),
    }
}

// ─── diagnostics ────────────────────────────────────────────────────────────

fn diag(d: &Diagnostics, plugin: &str, verb: &str, input: &Value) -> Result<Value, String> {
    let name = || {
        input
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty() && n.len() <= 80)
            .ok_or_else(|| "give a `name` of 1 to 80 characters".to_string())
    };
    let number = |field: &str| {
        input
            .get(field)
            .and_then(Value::as_f64)
            .ok_or_else(|| format!("give a number `{field}`"))
    };
    match verb {
        "count" => {
            let delta = input.get("delta").and_then(Value::as_i64).unwrap_or(1);
            d.count(plugin, name()?, delta)?;
        }
        "gauge" => d.gauge(plugin, name()?, number("value")?)?,
        "time" => d.timing(plugin, name()?, number("ms")?)?,
        "marker" => d.marker(plugin, name()?),
        _ => return Err(UNKNOWN.to_string()),
    }
    Ok(json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStore;

    fn services() -> HostServices {
        HostServices::new("9.9.9")
            .with_project_store(Arc::new(MemoryStore::new()))
            .with_save_store(Arc::new(MemoryStore::new()))
    }

    fn call(s: &HostServices, plugin: &str, name: &str, input: Value) -> Result<Value, String> {
        s.call(plugin, name, &input).expect("known service")
    }

    #[test]
    fn storage_round_trips_text_and_bytes_per_plugin() {
        let s = services();
        let wrote = call(
            &s,
            "a",
            "storage.write",
            json!({"key": "n/x", "text": "hello"}),
        )
        .unwrap();
        assert_eq!(wrote["size"], 5);
        let read = call(&s, "a", "storage.read", json!({"key": "n/x", "as": "text"})).unwrap();
        assert_eq!(read["text"], "hello");
        assert_eq!(read["hash"], wrote["hash"]);
        let bytes = call(&s, "a", "storage.read", json!({"key": "n/x"})).unwrap();
        assert_eq!(
            B64.decode(bytes["data"].as_str().unwrap()).unwrap(),
            b"hello"
        );
        // Another plugin sees nothing, and the project and saves are separate.
        assert_eq!(
            call(&s, "b", "storage.read", json!({"key": "n/x"})).unwrap()["found"],
            false
        );
        assert_eq!(
            call(&s, "a", "save.read", json!({"key": "n/x"})).unwrap()["found"],
            false
        );
        let listed = call(&s, "a", "storage.list", json!({"prefix": "n/"})).unwrap();
        assert_eq!(listed["keys"][0]["key"], "n/x");
        call(&s, "a", "storage.delete", json!({"key": "n/x"})).unwrap();
        assert_eq!(
            call(&s, "a", "storage.exists", json!({"key": "n/x"})).unwrap()["found"],
            false
        );
    }

    #[test]
    fn a_commit_is_all_or_nothing_and_content_blobs_are_addressed_by_hash() {
        let s = services();
        let bad = call(
            &s,
            "a",
            "storage.commit",
            json!({"ops": [{"op": "write", "key": "ok", "text": "1"}, {"op": "write", "key": "../no", "text": "2"}]}),
        );
        assert!(bad.is_err());
        assert_eq!(
            call(&s, "a", "storage.exists", json!({"key": "ok"})).unwrap()["found"],
            false
        );
        let put = call(&s, "a", "storage.put", json!({"text": "chunk"})).unwrap();
        let hash = put["hash"].as_str().unwrap();
        let got = call(&s, "a", "storage.get", json!({"hash": hash, "as": "text"})).unwrap();
        assert_eq!(got["text"], "chunk");
        assert!(call(&s, "a", "storage.get", json!({"hash": "nope"})).is_err());
        call(
            &s,
            "a",
            "storage.commit",
            json!({"ops": [{"op": "write", "key": "ok", "text": "1"}, {"op": "delete", "key": "missing"}]}),
        )
        .unwrap();
    }

    #[test]
    fn a_missing_store_and_an_unknown_service_are_told_apart() {
        let s = HostServices::new("1");
        assert!(call(&s, "a", "storage.read", json!({"key": "k"})).is_err());
        assert!(s.call("a", "nonsense.verb", &Value::Null).is_none());
        assert!(s.call("a", "rng.nonsense", &Value::Null).is_none());
        assert_eq!(
            call(&s, "a", "host.version", Value::Null).unwrap()["engine"],
            "1"
        );
    }

    #[test]
    fn the_generator_replays_and_stays_in_range() {
        let s = services();
        let a = call(
            &s,
            "a",
            "rng.unit",
            json!({"seed": 7, "index": 3, "count": 4}),
        )
        .unwrap();
        let b = call(
            &s,
            "b",
            "rng.unit",
            json!({"seed": 7, "index": 3, "count": 4}),
        )
        .unwrap();
        assert_eq!(a, b);
        assert!(
            a["values"]
                .as_array()
                .unwrap()
                .iter()
                .all(|v| (0.0..1.0).contains(&v.as_f64().unwrap()))
        );
        let one = call(&s, "a", "rng.unit", json!({"seed": 7, "index": 4})).unwrap();
        assert_eq!(one["value"], a["values"][1]);
        for i in 0..200 {
            let v = call(
                &s,
                "a",
                "rng.range",
                json!({"seed": 1, "index": i, "min": -2, "max": 3}),
            )
            .unwrap();
            assert!((-2..=3).contains(&v["value"].as_i64().unwrap()));
        }
        assert!(call(&s, "a", "rng.range", json!({"min": 3, "max": 2})).is_err());
    }

    #[test]
    fn plugins_report_into_the_diagnostics() {
        let s = services();
        call(&s, "a", "diag.count", json!({"name": "meshes", "delta": 4})).unwrap();
        call(
            &s,
            "a",
            "diag.gauge",
            json!({"name": "chunks", "value": 9.5}),
        )
        .unwrap();
        call(&s, "a", "diag.time", json!({"name": "mesh", "ms": 2.0})).unwrap();
        call(&s, "a", "diag.marker", json!({"name": "regen"})).unwrap();
        assert!(call(&s, "a", "diag.count", json!({})).is_err());
        let snap = s.diagnostics.snapshot();
        assert_eq!(snap["plugins"]["a"]["counters"]["meshes"], 4);
        assert_eq!(snap["plugins"]["a"]["timings"]["mesh"]["count"], 1);
    }

    #[test]
    fn a_module_reaches_services_through_the_capability_gate() {
        use crate::portable::{PortableModule, fixture};
        use blockloom_plugin_api::manifest::{Capability, PortableEntry};
        use std::collections::BTreeSet;
        let entry = PortableEntry {
            module: "m.wasm".to_string(),
            memory_limit_mib: 16,
            call_limit_ms: 100,
        };
        let s = services();
        let open = |caps: BTreeSet<Capability>| {
            PortableModule::load(&fixture::wasm(), &entry, caps, s.for_plugin("p")).unwrap()
        };
        // Unsaved or ungated services answer without any capability.
        let mut plain = open(BTreeSet::new());
        let version: Value =
            serde_json::from_slice(&plain.call("service", b"host.version").unwrap()).unwrap();
        assert_eq!(version["engine"], "9.9.9");
        assert!(plain.call("service", b"rng.unit").is_ok());
        assert!(plain.call("service", b"storage.list").is_err());
        let mut stored = open(BTreeSet::from([Capability::ProjectStorage]));
        let listed: Value =
            serde_json::from_slice(&stored.call("service", b"storage.list").unwrap()).unwrap();
        assert_eq!(listed["keys"], json!([]));
        assert!(stored.call("service", b"save.list").is_ok());
    }

    #[test]
    fn a_provider_answers_what_the_built_ins_do_not() {
        struct Echo;
        impl ServiceProvider for Echo {
            fn call(
                &self,
                plugin: &str,
                service: &str,
                input: &Value,
            ) -> Option<Result<Value, String>> {
                (service == "echo.say").then(|| Ok(json!({"plugin": plugin, "input": input})))
            }
        }
        let s = services().with_provider(Arc::new(Echo));
        let f = s.for_plugin("p");
        let out = f("echo.say", br#"{"x":1}"#).unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["plugin"], "p");
        assert_eq!(f("echo.other", b"{}").unwrap_err(), Status::Unsupported);
        // A refusal reaches the plugin as an error object.
        let refused = f("storage.read", br#"{"key": "../x"}"#).unwrap();
        assert!(String::from_utf8(refused).unwrap().contains("error"));
    }
}
