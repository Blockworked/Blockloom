//! Typed graph nodes a plugin offers, and the graphs built from them.
//!
//! A package lists [`NodeSchema`]s: typed input and output ports, parameters
//! and the module op (`node.<name>`) that computes one. A graph is data,
//! usually in one of the plugin's own resources: [`GraphDef`] names nodes,
//! their parameters and the edges between ports. [`GraphDef::plan`] checks a
//! graph against the schemas (types, one source per input, no cycles) and
//! orders it; the host evaluates a plan tile by tile with a cache (see
//! `blockloom_plugin_host::generation`).
//!
//! A node's `margin` is how many cells of extra input it needs around the
//! tile it makes (a blur's radius). A plan carries, for each node, the
//! margin its consumers need of it, so a tile is computed a little wider
//! upstream and the seams line up.

use crate::id::validate_type_id;
use crate::schema::{FieldSchema, check_fields, validate_object};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// The most nodes one graph may hold.
pub const MAX_NODES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortType {
    Number,
    Bool,
    Text,
    /// A scalar value per cell of the tile.
    Field,
    /// A yes/no per cell of the tile.
    Mask,
    /// A list of positions.
    Points,
    /// Anything; connects to every other type.
    Json,
}

impl PortType {
    /// Whether an output of type `from` may feed an input of this type.
    pub fn accepts(self, from: PortType) -> bool {
        self == from || self == PortType::Json || from == PortType::Json
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortSchema {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: PortType,
    /// An input that may stay unconnected.
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeSchema {
    pub name: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub category: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default)]
    pub inputs: Vec<PortSchema>,
    #[serde(default)]
    pub outputs: Vec<PortSchema>,
    #[serde(default)]
    pub params: Vec<FieldSchema>,
    /// Bump when the node computes something different, so cached tiles stop matching.
    #[serde(default = "one")]
    pub version: u32,
    /// Cells of extra input needed around the tile this node makes.
    #[serde(default)]
    pub margin: u32,
    /// False for a node whose answer depends on more than its inputs.
    #[serde(default = "yes")]
    pub cacheable: bool,
}

fn one() -> u32 {
    1
}

fn yes() -> bool {
    true
}

impl NodeSchema {
    /// The module op that computes this node.
    pub fn op(&self) -> String {
        format!("node.{}", self.name)
    }

    pub fn input(&self, name: &str) -> Option<&PortSchema> {
        self.inputs.iter().find(|p| p.name == name)
    }

    pub fn output(&self, name: &str) -> Option<&PortSchema> {
        self.outputs.iter().find(|p| p.name == name)
    }

    /// `given` with every absent parameter at its default.
    pub fn normalize_params(&self, given: &BTreeMap<String, Value>) -> Value {
        let mut object: serde_json::Map<String, Value> =
            given.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        for field in &self.params {
            object
                .entry(field.name.clone())
                .or_insert_with(|| field.default_value());
        }
        Value::Object(object)
    }

    pub fn check_definition(&self) -> Result<(), String> {
        validate_type_id(&self.name)?;
        if self.title.trim().is_empty() {
            return Err(format!("node {}: a title is needed", self.name));
        }
        if self.version == 0 {
            return Err(format!("node {}: versions start at 1", self.name));
        }
        if self.outputs.is_empty() {
            return Err(format!("node {}: it has no outputs", self.name));
        }
        let mut ports = BTreeSet::new();
        for port in self.inputs.iter().chain(&self.outputs) {
            validate_type_id(&port.name)?;
            if !ports.insert(port.name.as_str()) {
                return Err(format!("node {}: two ports named {}", self.name, port.name));
            }
        }
        if self.outputs.iter().any(|p| p.optional) {
            return Err(format!("node {}: only an input can be optional", self.name));
        }
        check_fields(&self.params, &format!("node {}", self.name))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub node: String,
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
}

/// A connection from `from` (`node.port`, an output) to `to` (`node.port`, an input).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphDef {
    pub nodes: Vec<GraphNode>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    /// The endpoints (`node.port`) whose values the caller wants.
    #[serde(default)]
    pub outputs: Vec<String>,
}

/// The region a graph is evaluated over: cell origin, size and level of detail.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tile {
    pub origin: [i64; 3],
    pub size: [u32; 3],
    #[serde(default)]
    pub lod: u32,
}

impl Tile {
    /// The tile grown by `margin` cells on every side.
    pub fn grown(&self, margin: u32) -> Tile {
        let m = i64::from(margin);
        Tile {
            origin: self.origin.map(|o| o - m),
            size: self
                .size
                .map(|s| s.saturating_add(margin.saturating_mul(2))),
            lod: self.lod,
        }
    }
}

/// A checked graph: the order to evaluate it in and what each node needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// Indexes into the graph's nodes, every node after the nodes it reads.
    pub order: Vec<usize>,
    /// Per node (by index): its inputs, port to source endpoint.
    pub sources: Vec<BTreeMap<String, String>>,
    /// Per node (by index): the margin its consumers need of it.
    pub margins: Vec<u32>,
    /// The endpoints the caller asked for, checked to exist.
    pub outputs: Vec<String>,
}

fn split_endpoint(endpoint: &str) -> Result<(&str, &str), String> {
    endpoint
        .split_once('.')
        .filter(|(n, p)| !n.is_empty() && !p.is_empty())
        .ok_or_else(|| format!("\"{endpoint}\" is not an endpoint like node.port"))
}

impl GraphDef {
    /// Checks the graph against the plugin's node schemas.
    pub fn plan(&self, schemas: &[NodeSchema]) -> Result<Plan, String> {
        if self.nodes.len() > MAX_NODES {
            return Err(format!("a graph may hold {MAX_NODES} nodes at most"));
        }
        let mut index = BTreeMap::new();
        let mut schema_of = Vec::new();
        for (i, node) in self.nodes.iter().enumerate() {
            let ok = !node.id.is_empty()
                && node.id.len() <= 64
                && node
                    .id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if !ok {
                return Err(format!("\"{}\" is not a node id", node.id));
            }
            if index.insert(node.id.as_str(), i).is_some() {
                return Err(format!("two nodes are called {}", node.id));
            }
            let schema = schemas
                .iter()
                .find(|s| s.name == node.node)
                .ok_or_else(|| format!("{}: the plugin has no node {}", node.id, node.node))?;
            // Parameters: nothing unknown, everything valid, defaults filled in at run time.
            if let Some(extra) = node
                .params
                .keys()
                .find(|k| !schema.params.iter().any(|p| &p.name == *k))
            {
                return Err(format!(
                    "{}: {} has no parameter {extra}",
                    node.id, schema.name
                ));
            }
            let given: serde_json::Map<String, Value> = node.params.clone().into_iter().collect();
            let errors = validate_object(&schema.params, &Value::Object(given));
            if let Some(e) = errors.first() {
                return Err(format!("{}: {e}", node.id));
            }
            schema_of.push(schema);
        }

        let mut sources: Vec<BTreeMap<String, String>> = vec![BTreeMap::new(); self.nodes.len()];
        let mut reads: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); self.nodes.len()];
        let mut consumers: Vec<Vec<usize>> = vec![Vec::new(); self.nodes.len()];
        for edge in &self.edges {
            let (from_node, from_port) = split_endpoint(&edge.from)?;
            let (to_node, to_port) = split_endpoint(&edge.to)?;
            let &from = index
                .get(from_node)
                .ok_or_else(|| format!("{}: there is no node {from_node}", edge.from))?;
            let &to = index
                .get(to_node)
                .ok_or_else(|| format!("{}: there is no node {to_node}", edge.to))?;
            let out = schema_of[from]
                .output(from_port)
                .ok_or_else(|| format!("{}: {from_node} has no output {from_port}", edge.from))?;
            let input = schema_of[to]
                .input(to_port)
                .ok_or_else(|| format!("{}: {to_node} has no input {to_port}", edge.to))?;
            if !input.ty.accepts(out.ty) {
                return Err(format!(
                    "{} -> {}: a {:?} output cannot feed a {:?} input",
                    edge.from, edge.to, out.ty, input.ty
                ));
            }
            if sources[to]
                .insert(to_port.to_string(), edge.from.clone())
                .is_some()
            {
                return Err(format!("{}: more than one edge feeds it", edge.to));
            }
            reads[to].insert(from);
            consumers[from].push(to);
        }
        for (i, node) in self.nodes.iter().enumerate() {
            if let Some(missing) = schema_of[i]
                .inputs
                .iter()
                .find(|p| !p.optional && !sources[i].contains_key(&p.name))
            {
                return Err(format!(
                    "{}.{}: nothing feeds this input",
                    node.id, missing.name
                ));
            }
        }

        // Kahn's order, by index so the result is the same however edges are listed.
        let mut waiting: Vec<usize> = reads.iter().map(BTreeSet::len).collect();
        let mut ready: BTreeSet<usize> =
            (0..self.nodes.len()).filter(|i| waiting[*i] == 0).collect();
        let mut order = Vec::new();
        while let Some(next) = ready.pop_first() {
            order.push(next);
            for &c in &consumers[next] {
                // An edge may repeat a node pair on different ports; count once per pair.
                if reads[c].contains(&next) {
                    reads[c].remove(&next);
                    waiting[c] -= 1;
                    if waiting[c] == 0 {
                        ready.insert(c);
                    }
                }
            }
        }
        if order.len() != self.nodes.len() {
            let stuck = (0..self.nodes.len())
                .find(|i| !order.contains(i))
                .map(|i| self.nodes[i].id.as_str())
                .unwrap_or("");
            return Err(format!("the graph has a cycle through {stuck}"));
        }

        // Margins flow upstream: a node makes what its consumers need, plus
        // what they need extra around it.
        let mut margins = vec![0u32; self.nodes.len()];
        for &i in order.iter().rev() {
            let need = consumers[i]
                .iter()
                .map(|&c| margins[c].saturating_add(schema_of[c].margin))
                .max()
                .unwrap_or(0);
            margins[i] = need;
        }

        let mut outputs = Vec::new();
        for endpoint in &self.outputs {
            let (node, port) = split_endpoint(endpoint)?;
            let &i = index
                .get(node)
                .ok_or_else(|| format!("{endpoint}: there is no node {node}"))?;
            if schema_of[i].output(port).is_none() {
                return Err(format!("{endpoint}: {node} has no output {port}"));
            }
            outputs.push(endpoint.clone());
        }
        Ok(Plan {
            order,
            sources,
            margins,
            outputs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schemas() -> Vec<NodeSchema> {
        serde_json::from_value(json!([
            {"name": "noise", "title": "Noise",
             "outputs": [{"name": "height", "type": "field"}],
             "params": [{"name": "scale", "type": "number", "default": 1.0, "min": 0.0}]},
            {"name": "blur", "title": "Blur", "margin": 2,
             "inputs": [{"name": "in", "type": "field"}],
             "outputs": [{"name": "out", "type": "field"}]},
            {"name": "threshold", "title": "Threshold", "margin": 1,
             "inputs": [{"name": "in", "type": "field"}, {"name": "level", "type": "number", "optional": true}],
             "outputs": [{"name": "mask", "type": "mask"}]},
            {"name": "label", "title": "Label",
             "inputs": [{"name": "text", "type": "text"}],
             "outputs": [{"name": "shown", "type": "text"}]}
        ]))
        .unwrap()
    }

    fn graph(value: Value) -> GraphDef {
        serde_json::from_value(value).unwrap()
    }

    fn chain() -> GraphDef {
        graph(json!({
            "nodes": [
                {"id": "t", "node": "threshold"},
                {"id": "b", "node": "blur"},
                {"id": "n", "node": "noise", "params": {"scale": 4.0}}
            ],
            "edges": [{"from": "n.height", "to": "b.in"}, {"from": "b.out", "to": "t.in"}],
            "outputs": ["t.mask"]
        }))
    }

    #[test]
    fn a_chain_is_ordered_and_margins_flow_upstream() {
        for s in schemas() {
            s.check_definition().unwrap();
        }
        let plan = chain().plan(&schemas()).unwrap();
        // Indexes: t=0, b=1, n=2. Sources first.
        assert_eq!(plan.order, vec![2, 1, 0]);
        // t needs nothing extra; b must make 1 extra for t; n 1 + 2 for b.
        assert_eq!(plan.margins, vec![0, 1, 3]);
        assert_eq!(plan.sources[1]["in"], "n.height");
        assert_eq!(plan.outputs, ["t.mask"]);
    }

    #[test]
    fn a_tile_grows_by_a_margin_on_every_side() {
        let tile = Tile {
            origin: [10, 0, -4],
            size: [16, 16, 16],
            lod: 1,
        };
        let grown = tile.grown(3);
        assert_eq!(grown.origin, [7, -3, -7]);
        assert_eq!(grown.size, [22, 22, 22]);
        assert_eq!(grown.lod, 1);
    }

    #[test]
    fn bad_graphs_are_refused_with_the_reason() {
        let s = schemas();
        let err = |g: Value| graph(g).plan(&s).unwrap_err();
        assert!(err(json!({"nodes": [{"id": "a", "node": "nope"}]})).contains("no node nope"));
        assert!(
            err(json!({"nodes": [{"id": "a", "node": "noise"}, {"id": "a", "node": "noise"}]}))
                .contains("two nodes")
        );
        assert!(err(json!({"nodes": [{"id": "a b", "node": "noise"}]})).contains("not a node id"));
        assert!(
            err(json!({"nodes": [{"id": "n", "node": "noise", "params": {"bogus": 1}}]}))
                .contains("no parameter")
        );
        assert!(
            err(json!({"nodes": [{"id": "n", "node": "noise", "params": {"scale": -1.0}}]}))
                .contains("scale")
        );
        assert!(err(json!({"nodes": [{"id": "b", "node": "blur"}]})).contains("nothing feeds"));
        // A mask cannot feed a field input.
        assert!(err(json!({
            "nodes": [{"id": "n", "node": "noise"}, {"id": "t", "node": "threshold"}, {"id": "b", "node": "blur"}],
            "edges": [{"from": "n.height", "to": "t.in"}, {"from": "t.mask", "to": "b.in"}]
        })).contains("cannot feed"));
        assert!(err(json!({
            "nodes": [{"id": "n", "node": "noise"}, {"id": "m", "node": "noise"}, {"id": "b", "node": "blur"}],
            "edges": [{"from": "n.height", "to": "b.in"}, {"from": "m.height", "to": "b.in"}]
        })).contains("more than one edge"));
        assert!(
            err(json!({
                "nodes": [{"id": "n", "node": "noise"}],
                "edges": [{"from": "n.nothing", "to": "n.in"}]
            }))
            .contains("no output nothing")
        );
        assert!(
            err(json!({"nodes": [{"id": "n", "node": "noise"}], "outputs": ["n.nope"]}))
                .contains("no output nope")
        );
    }

    #[test]
    fn a_cycle_is_found() {
        let s = schemas();
        let cyclic = graph(json!({
            "nodes": [{"id": "a", "node": "blur"}, {"id": "b", "node": "blur"}],
            "edges": [{"from": "a.out", "to": "b.in"}, {"from": "b.out", "to": "a.in"}]
        }));
        assert!(cyclic.plan(&s).unwrap_err().contains("cycle"));
    }

    #[test]
    fn json_ports_connect_to_anything_and_node_definitions_are_checked() {
        assert!(PortType::Json.accepts(PortType::Mask));
        assert!(PortType::Field.accepts(PortType::Json));
        assert!(!PortType::Field.accepts(PortType::Mask));
        let mut bad = schemas()[0].clone();
        bad.outputs.clear();
        assert!(bad.check_definition().is_err());
        let mut dup = schemas()[1].clone();
        dup.outputs[0].name = "in".to_string();
        assert!(dup.check_definition().unwrap_err().contains("two ports"));
        let mut optional_out = schemas()[0].clone();
        optional_out.outputs[0].optional = true;
        assert!(optional_out.check_definition().is_err());
    }
}
