//! Replays `fixtures/editor-baseline/trace.json` through `Backend::dispatch`
//! and holds every step's reply and resulting actor graph to `expected.json`.
//! It pins down today's canvas edit behavior so shared-editor refactors can
//! show what they change. `BLOCKLOOM_BLESS=1` rewrites the expected file.

use blockloom_app::{AppHandle, Backend};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/editor-baseline")
}

fn scratch(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "blockloom-baseline-{tag}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn is_generated_id(s: &str) -> bool {
    let hex = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit());
    (s.len() == 32 && hex(s))
        || (s.len() == 36
            && s.split('-').map(str::len).eq([8, 4, 4, 4, 12])
            && hex(&s.replace('-', "")))
}

/// Numbers generated ids by first appearance so runs compare equal.
#[derive(Default)]
struct Ids(HashMap<String, usize>);

impl Ids {
    fn normalize(&mut self, value: &Value) -> Value {
        match value {
            Value::String(s) if is_generated_id(s) => {
                let next = self.0.len() + 1;
                Value::String(format!("id#{}", self.0.entry(s.clone()).or_insert(next)))
            }
            Value::Array(items) => Value::Array(items.iter().map(|v| self.normalize(v)).collect()),
            Value::Object(map) => {
                let mut out = Map::new();
                for (k, v) in map {
                    let key = if is_generated_id(k) {
                        self.normalize(&Value::String(k.clone()))
                    } else {
                        Value::String(k.clone())
                    };
                    out.insert(key.as_str().unwrap().to_string(), self.normalize(v));
                }
                Value::Object(out)
            }
            other => other.clone(),
        }
    }
}

/// Replaces `$name` and `$graph:/pointer` strings with saved values.
fn substitute(value: &Value, saved: &HashMap<String, Value>, graph: &Value) -> Value {
    match value {
        Value::String(s) if s.starts_with("$graph:") => graph
            .pointer(&s["$graph:".len()..])
            .cloned()
            .unwrap_or_else(|| panic!("{s} not in the graph")),
        Value::String(s) if s.starts_with('$') => saved
            .get(&s[1..])
            .cloned()
            .unwrap_or_else(|| panic!("{s} was never saved")),
        Value::Array(items) => {
            Value::Array(items.iter().map(|v| substitute(v, saved, graph)).collect())
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), substitute(v, saved, graph)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The open actor's graph, plus the editor state that rides on it.
fn graph_snapshot(backend: &Backend, actor: Option<&Value>) -> Value {
    let state = backend.dispatch("get_state", json!({})).expect("get_state");
    let graph = actor
        .and_then(|id| {
            state["project"]["actors"]
                .as_array()?
                .iter()
                .find(|a| &a["id"] == id)
                .cloned()
        })
        .map(|a| {
            let keys = [
                "strands",
                "floating_values",
                "comments",
                "block_defs",
                "variables",
                "lists",
                "dicts",
            ];
            Value::Object(
                keys.iter()
                    .map(|k| (k.to_string(), a[*k].clone()))
                    .collect(),
            )
        })
        .unwrap_or(Value::Null);
    // Buffers come from a map, so their order is not stable.
    let mut buffers = state["invalid_field_buffers"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    buffers.sort_by_key(|b| b.to_string());
    json!({
        "graph": graph,
        "invalid_field_buffers": buffers,
        "can_undo": state["can_undo"],
        "can_redo": state["can_redo"],
    })
}

// The data dir is process-global, so the tests take turns.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn canvas_edits_match_the_recorded_baseline() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = scratch("data");
    let projects = scratch("projects");
    unsafe {
        std::env::set_var("BLOCKLOOM_DATA_DIR", &data);
    }
    let trace: Value =
        serde_json::from_str(&std::fs::read_to_string(fixtures().join("trace.json")).unwrap())
            .unwrap();
    let backend = Backend::start(AppHandle::new(|_| {}));
    let mut saved: HashMap<String, Value> = HashMap::new();
    saved.insert("PROJECTS".into(), json!(projects.to_string_lossy()));
    let mut ids = Ids::default();
    let mut steps = Vec::new();
    for (n, step) in trace["steps"].as_array().unwrap().iter().enumerate() {
        let command = step["command"].as_str().unwrap();
        let before = graph_snapshot(&backend, saved.get("actor"));
        let args = substitute(&step["args"], &saved, &before["graph"]);
        let reply = backend.dispatch(command, args.clone());
        if let (Some(name), Ok(result)) = (step["save"].as_str(), &reply) {
            saved.insert(name.to_string(), result.clone());
        }
        let after = graph_snapshot(&backend, saved.get("actor"));
        steps.push(json!({
            "step": n,
            "command": command,
            "args": ids.normalize(&step["args"]),
            "ok": reply.is_ok(),
            "result": reply.as_ref().map(|r| ids.normalize(r)).unwrap_or(Value::Null),
            "error": reply.as_ref().err(),
            "after": ids.normalize(&after),
        }));
    }
    // The documents as written to disk.
    let read = |rel: &str| -> Value {
        serde_json::from_str(
            &std::fs::read_to_string(projects.join("Baseline").join(rel)).expect(rel),
        )
        .unwrap()
    };
    let document = json!({
        "project": read("project.blockloom"),
        "scene": read("assets/scenes/Scene 1.blockscene"),
    });
    let actual = json!({ "steps": steps, "saved_document": ids.normalize(&document) });
    let text = serde_json::to_string_pretty(&actual).unwrap() + "\n";
    let expected_path = fixtures().join("expected.json");
    if std::env::var_os("BLOCKLOOM_BLESS").is_some() {
        std::fs::write(&expected_path, &text).unwrap();
    } else {
        let expected = std::fs::read_to_string(&expected_path)
            .expect("expected.json; run with BLOCKLOOM_BLESS=1");
        if expected != text {
            let out = std::env::temp_dir().join("editor-baseline-actual.json");
            std::fs::write(&out, &text).unwrap();
            let want: Value = serde_json::from_str(&expected).unwrap();
            let first = want["steps"]
                .as_array()
                .and_then(|w| w.iter().zip(&steps).position(|(a, b)| a != b));
            panic!(
                "editor baseline changed (first differing step: {first:?}); actual written to {}",
                out.display()
            );
        }
    }
    let _ = std::fs::remove_dir_all(&data);
    if std::env::var_os("BLOCKLOOM_KEEP").is_none() {
        let _ = std::fs::remove_dir_all(&projects);
    } else {
        eprintln!("kept {}", projects.display());
    }
}

/// Every key and value of `old` is still in `new`; a newer save may add keys.
fn assert_preserved(old: &Value, new: &Value, at: &str) {
    match (old, new) {
        (Value::Object(o), Value::Object(n)) => {
            for (k, v) in o {
                let field = n.get(k).unwrap_or_else(|| panic!("{at}/{k} was dropped"));
                assert_preserved(v, field, &format!("{at}/{k}"));
            }
        }
        (Value::Array(o), Value::Array(n)) => {
            assert_eq!(o.len(), n.len(), "{at} changed length");
            for (i, (a, b)) in o.iter().zip(n).enumerate() {
                assert_preserved(a, b, &format!("{at}/{i}"));
            }
        }
        _ => assert_eq!(old, new, "{at} changed"),
    }
}

/// A project saved by the baseline loads and saves back without losing anything.
#[test]
fn saved_project_round_trips() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = scratch("data");
    let parent = scratch("open");
    unsafe {
        std::env::set_var("BLOCKLOOM_DATA_DIR", &data);
    }
    let source = fixtures().join("project");
    let dir = parent.join("Baseline");
    for rel in ["project.blockloom", "assets/scenes/Scene 1.blockscene"] {
        std::fs::create_dir_all(dir.join(rel).parent().unwrap()).unwrap();
        std::fs::copy(source.join(rel), dir.join(rel)).unwrap();
    }
    let backend = Backend::start(AppHandle::new(|_| {}));
    backend
        .dispatch("open_project", json!({ "path": dir.to_string_lossy() }))
        .expect("open");
    backend.dispatch("save_project", json!({})).expect("save");
    for rel in ["project.blockloom", "assets/scenes/Scene 1.blockscene"] {
        let parse = |p: PathBuf| -> Value {
            serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
        };
        assert_preserved(&parse(source.join(rel)), &parse(dir.join(rel)), rel);
    }
    backend.dispatch("close_project", json!({})).expect("close");
    let _ = std::fs::remove_dir_all(&data);
    let _ = std::fs::remove_dir_all(&parent);
}
