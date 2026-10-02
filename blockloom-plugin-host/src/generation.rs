//! Evaluating a plugin's node graph over a tile.
//!
//! A [`GraphRun`] walks a checked [`Plan`] one node at a time, so a world can
//! spread a graph over many ticks as a job. A node whose inputs, parameters,
//! version, tile and seed match an earlier run is answered from the
//! [`GraphCache`] and its module is not called. The cache key is a hash of
//! those, and each output's own hash derives from the key, so a downstream
//! node never hashes a large value and an upstream edit changes every key
//! below it.

use blockloom_plugin_api::generation::{GraphDef, NodeSchema, Plan, Tile};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// What the cache holds by default.
pub const DEFAULT_CACHE_BYTES: u64 = 64 * 1024 * 1024;

/// Evaluated nodes by key, least recently used out first.
pub struct GraphCache {
    entries: BTreeMap<String, (Value, u64, u64)>,
    bytes: u64,
    limit: u64,
    clock: u64,
    pub hits: u64,
    pub misses: u64,
}

impl Default for GraphCache {
    fn default() -> Self {
        Self::with_limit(DEFAULT_CACHE_BYTES)
    }
}

impl GraphCache {
    pub fn with_limit(limit: u64) -> Self {
        Self {
            entries: BTreeMap::new(),
            bytes: 0,
            limit,
            clock: 0,
            hits: 0,
            misses: 0,
        }
    }

    pub fn get(&mut self, key: &str) -> Option<Value> {
        self.clock += 1;
        let clock = self.clock;
        match self.entries.get_mut(key) {
            Some(entry) => {
                entry.2 = clock;
                self.hits += 1;
                Some(entry.0.clone())
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    pub fn put(&mut self, key: String, value: Value) {
        let size = value.to_string().len() as u64;
        if size > self.limit {
            return;
        }
        self.clock += 1;
        if let Some((_, old, _)) = self.entries.insert(key, (value, size, self.clock)) {
            self.bytes -= old;
        }
        self.bytes += size;
        while self.bytes > self.limit {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, _, used))| *used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some((_, size, _)) = self.entries.remove(&oldest) {
                self.bytes -= size;
            }
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

fn hash_hex(parts: &Value) -> String {
    Sha256::digest(parts.to_string().as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// One graph evaluation in progress.
pub struct GraphRun {
    graph: GraphDef,
    schemas: Vec<NodeSchema>,
    plan: Plan,
    tile: Tile,
    seed: u64,
    extra: Value,
    next: usize,
    /// `node.port` to its value and the hash it stands for.
    values: BTreeMap<String, (Value, String)>,
    pub evaluated: u64,
    pub cached: u64,
}

impl GraphRun {
    /// Starts a run from a job's arguments: `{graph, tile, seed?, extra?}`.
    /// `extra` is mixed into every key, so a caller can invalidate on a
    /// revision of whatever else the nodes read.
    pub fn new(schemas: &[NodeSchema], args: &Value) -> Result<GraphRun, String> {
        let graph: GraphDef =
            serde_json::from_value(args.get("graph").cloned().ok_or("give a `graph`")?)
                .map_err(|e| format!("graph: {e}"))?;
        let tile: Tile = serde_json::from_value(args.get("tile").cloned().ok_or("give a `tile`")?)
            .map_err(|e| format!("tile: {e}"))?;
        if tile.size.contains(&0) {
            return Err("a tile has no size".to_string());
        }
        let plan = graph.plan(schemas)?;
        Ok(GraphRun {
            graph,
            schemas: schemas.to_vec(),
            plan,
            tile,
            seed: args.get("seed").and_then(Value::as_u64).unwrap_or(0),
            extra: args.get("extra").cloned().unwrap_or(Value::Null),
            next: 0,
            values: BTreeMap::new(),
            evaluated: 0,
            cached: 0,
        })
    }

    pub fn is_done(&self) -> bool {
        self.next >= self.plan.order.len()
    }

    pub fn progress(&self) -> f64 {
        if self.plan.order.is_empty() {
            1.0
        } else {
            self.next as f64 / self.plan.order.len() as f64
        }
    }

    /// Evaluates the next node. `call` runs a module op and answers its JSON.
    pub fn step(
        &mut self,
        cache: &mut GraphCache,
        call: &mut dyn FnMut(&str, &Value) -> Result<Value, String>,
    ) -> Result<(), String> {
        let i = self.plan.order[self.next];
        let node = &self.graph.nodes[i];
        let schema = self
            .schemas
            .iter()
            .find(|s| s.name == node.node)
            .expect("planned against these schemas");
        let params = schema.normalize_params(&node.params);
        let tile = self.tile.grown(self.plan.margins[i]);
        let mut input_hashes = Map::new();
        let mut inputs = Map::new();
        for (port, source) in &self.plan.sources[i] {
            let (value, hash) = self
                .values
                .get(source)
                .ok_or_else(|| format!("{}: {source} was not computed", node.id))?;
            input_hashes.insert(port.clone(), json!(hash));
            inputs.insert(port.clone(), value.clone());
        }
        let key = hash_hex(&json!([
            schema.name,
            schema.version,
            params,
            input_hashes,
            tile,
            self.seed,
            self.extra
        ]));
        let outputs = match schema.cacheable.then(|| cache.get(&key)).flatten() {
            Some(hit) => {
                self.cached += 1;
                hit
            }
            None => {
                let answer = call(
                    &schema.op(),
                    &json!({
                        "node": node.id,
                        "params": params,
                        "inputs": inputs,
                        "tile": tile,
                        "margin": self.plan.margins[i],
                        "seed": self.seed,
                    }),
                )
                .map_err(|e| format!("{}: {e}", node.id))?;
                let outputs = answer
                    .get("outputs")
                    .and_then(Value::as_object)
                    .ok_or_else(|| format!("{}: the answer has no `outputs`", node.id))?;
                if let Some(missing) = schema
                    .outputs
                    .iter()
                    .find(|p| !outputs.contains_key(&p.name))
                {
                    return Err(format!(
                        "{}: the answer has no output {}",
                        node.id, missing.name
                    ));
                }
                let outputs = Value::Object(outputs.clone());
                self.evaluated += 1;
                if schema.cacheable {
                    cache.put(key.clone(), outputs.clone());
                }
                outputs
            }
        };
        for port in &schema.outputs {
            self.values.insert(
                format!("{}.{}", node.id, port.name),
                (
                    outputs[&port.name].clone(),
                    hash_hex(&json!([key, port.name])),
                ),
            );
        }
        self.next += 1;
        Ok(())
    }

    /// The endpoints the graph asked for, once done.
    pub fn result(&self) -> Value {
        let outputs: Map<String, Value> = self
            .plan
            .outputs
            .iter()
            .filter_map(|e| self.values.get(e).map(|(v, _)| (e.clone(), v.clone())))
            .collect();
        json!({"outputs": outputs, "evaluated": self.evaluated, "cached": self.cached})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schemas() -> Vec<NodeSchema> {
        serde_json::from_value(json!([
            {"name": "noise", "title": "Noise",
             "outputs": [{"name": "height", "type": "number"}],
             "params": [{"name": "scale", "type": "number", "default": 1.0}]},
            {"name": "double", "title": "Double", "margin": 2,
             "inputs": [{"name": "in", "type": "number"}],
             "outputs": [{"name": "out", "type": "number"}]},
            {"name": "now", "title": "Now", "cacheable": false,
             "outputs": [{"name": "n", "type": "number"}]}
        ]))
        .unwrap()
    }

    fn args(scale: f64, seed: u64) -> Value {
        json!({
            "graph": {
                "nodes": [
                    {"id": "d", "node": "double"},
                    {"id": "n", "node": "noise", "params": {"scale": scale}}
                ],
                "edges": [{"from": "n.height", "to": "d.in"}],
                "outputs": ["d.out"]
            },
            "tile": {"origin": [0, 0, 0], "size": [8, 8, 8]},
            "seed": seed
        })
    }

    /// A module: `node.noise` answers scale + seed; `node.double` doubles its input.
    fn run(a: &Value, cache: &mut GraphCache, calls: &mut Vec<(String, Value)>) -> GraphRun {
        let mut run = GraphRun::new(&schemas(), a).unwrap();
        let mut module = |op: &str, input: &Value| -> Result<Value, String> {
            calls.push((op.to_string(), input.clone()));
            match op {
                "node.noise" => Ok(json!({"outputs": {"height":
                    input["params"]["scale"].as_f64().unwrap() + input["seed"].as_f64().unwrap()}})),
                "node.double" => {
                    Ok(json!({"outputs": {"out": input["inputs"]["in"].as_f64().unwrap() * 2.0}}))
                }
                _ => Err("no such op".to_string()),
            }
        };
        while !run.is_done() {
            run.step(cache, &mut module).unwrap();
        }
        run
    }

    #[test]
    fn a_graph_is_evaluated_in_order_with_margins_on_the_upstream_tile() {
        let mut cache = GraphCache::default();
        let mut calls = Vec::new();
        let r = run(&args(3.0, 1), &mut cache, &mut calls);
        assert_eq!(r.result()["outputs"]["d.out"], 8.0);
        assert_eq!(
            calls.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
            ["node.noise", "node.double"]
        );
        // double needs 2 extra cells of noise around its tile.
        assert_eq!(calls[0].1["tile"]["size"], json!([12, 12, 12]));
        assert_eq!(calls[0].1["tile"]["origin"], json!([-2, -2, -2]));
        assert_eq!(calls[1].1["tile"]["size"], json!([8, 8, 8]));
        assert_eq!((r.evaluated, r.cached), (2, 0));
    }

    #[test]
    fn an_unchanged_graph_is_answered_from_the_cache_and_an_edit_only_redoes_what_it_touches() {
        let mut cache = GraphCache::default();
        let mut calls = Vec::new();
        run(&args(3.0, 1), &mut cache, &mut calls);
        calls.clear();
        let again = run(&args(3.0, 1), &mut cache, &mut calls);
        assert!(calls.is_empty());
        assert_eq!((again.evaluated, again.cached), (0, 2));
        // A new scale changes the noise key and so the one below it.
        let edited = run(&args(5.0, 1), &mut cache, &mut calls);
        assert_eq!(edited.result()["outputs"]["d.out"], 12.0);
        assert_eq!((edited.evaluated, edited.cached), (2, 0));
        // A new seed does too; the old entries are still there for the old graph.
        calls.clear();
        run(&args(3.0, 1), &mut cache, &mut calls);
        assert!(calls.is_empty());
        assert!(cache.hits >= 4);
    }

    #[test]
    fn extra_state_invalidates_and_uncacheable_nodes_always_run() {
        let mut cache = GraphCache::default();
        let mut calls = Vec::new();
        let mut a = args(1.0, 0);
        run(&a, &mut cache, &mut calls);
        calls.clear();
        a["extra"] = json!({"revision": 2});
        run(&a, &mut cache, &mut calls);
        assert_eq!(calls.len(), 2);
        let live = json!({
            "graph": {"nodes": [{"id": "t", "node": "now"}], "outputs": ["t.n"]},
            "tile": {"origin": [0, 0, 0], "size": [1, 1, 1]}
        });
        let mut ticks = 0.0;
        for _ in 0..2 {
            let mut r = GraphRun::new(&schemas(), &live).unwrap();
            r.step(&mut cache, &mut |_, _| {
                ticks += 1.0;
                Ok(json!({"outputs": {"n": ticks}}))
            })
            .unwrap();
        }
        assert_eq!(ticks, 2.0);
    }

    #[test]
    fn a_node_that_fails_or_answers_short_stops_the_run() {
        let mut cache = GraphCache::default();
        let mut r = GraphRun::new(&schemas(), &args(1.0, 0)).unwrap();
        let err = r
            .step(&mut cache, &mut |_, _| Err("boom".into()))
            .unwrap_err();
        assert_eq!(err, "n: boom");
        let mut r = GraphRun::new(&schemas(), &args(1.0, 0)).unwrap();
        let err = r
            .step(&mut cache, &mut |_, _| Ok(json!({"outputs": {}})))
            .unwrap_err();
        assert!(err.contains("no output height"), "{err}");
        assert!(
            GraphRun::new(
                &schemas(),
                &json!({"tile": {"origin": [0,0,0], "size": [1,1,1]}})
            )
            .is_err()
        );
        let mut zero = args(1.0, 0);
        zero["tile"]["size"] = json!([0, 1, 1]);
        assert!(GraphRun::new(&schemas(), &zero).is_err());
    }

    #[test]
    fn the_cache_evicts_the_least_recently_used_over_its_limit() {
        let mut c = GraphCache::with_limit(30);
        c.put("a".into(), json!("0123456789"));
        c.put("b".into(), json!("0123456789"));
        assert!(c.get("a").is_some());
        c.put("c".into(), json!("0123456789"));
        assert!(c.get("b").is_none(), "b was the oldest unused");
        assert!(c.get("a").is_some() && c.get("c").is_some());
        assert!(c.bytes() <= 30);
        c.put("huge".into(), json!("x".repeat(100)));
        assert!(c.get("huge").is_none());
        c.clear();
        assert!(c.is_empty());
    }
}
