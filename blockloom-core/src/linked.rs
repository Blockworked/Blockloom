//! A value that follows a resource file except where it overrides it: the
//! shared half of material instances and scene environments. Both sides are
//! JSON objects compared by property path (`fog.height`, `roughness`), at
//! most two keys deep, and anything deeper is one value.

use crate::resources::ResourceRef;
use blockloom_schema_derive::Schema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// What links a value to the file it follows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Schema)]
pub struct AssetLink {
    /// The resource file followed.
    #[serde(default)]
    pub source: ResourceRef,
    /// Property paths set here rather than in the file, sorted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schema(skip)]
    pub overrides: Vec<String>,
}

const DEPTH: usize = 2;

/// `value` as property paths. Only objects on both levels are opened up.
pub fn flatten(value: &Value) -> BTreeMap<String, Value> {
    fn walk(value: &Value, at: &str, depth: usize, out: &mut BTreeMap<String, Value>) {
        match value {
            Value::Object(map) if depth < DEPTH && !map.is_empty() => {
                for (key, item) in map {
                    let path = if at.is_empty() {
                        key.clone()
                    } else {
                        format!("{at}.{key}")
                    };
                    walk(item, &path, depth + 1, out);
                }
            }
            _ => {
                out.insert(at.to_string(), value.clone());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(value, "", 0, &mut out);
    out
}

fn covers(overrides: &[String], path: &str) -> bool {
    overrides.iter().any(|o| {
        o == path
            || path
                .strip_prefix(o.as_str())
                .is_some_and(|rest| rest.starts_with('.'))
            || o.strip_prefix(path)
                .is_some_and(|rest| rest.starts_with('.'))
    })
}

/// Updates `overrides` after an edit took the value from `before` to `now`:
/// a changed path that differs from the file is overridden, one put back to
/// the file's value is not.
pub fn mark(before: &Value, now: &Value, source: &Value, overrides: &mut Vec<String>) {
    let (before, now, source) = (flatten(before), flatten(now), flatten(source));
    let keys: BTreeSet<&String> = before.keys().chain(now.keys()).collect();
    for key in keys {
        if key.is_empty() || before.get(key) == now.get(key) {
            continue;
        }
        let at = overrides.binary_search(key);
        match (now.get(key) == source.get(key), at) {
            (true, Ok(i)) => {
                overrides.remove(i);
            }
            (false, Err(i)) => overrides.insert(i, key.clone()),
            _ => {}
        }
    }
}

fn set(root: &mut Map<String, Value>, path: &str, value: Value) {
    match path.split_once('.') {
        None => {
            root.insert(path.to_string(), value);
        }
        Some((head, rest)) => {
            let child = root
                .entry(head.to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            if !child.is_object() {
                *child = Value::Object(Map::new());
            }
            if let Value::Object(map) = child {
                set(map, rest, value);
            }
        }
    }
}

/// The file's value with every overridden path taken from `own`.
pub fn merge(own: &Value, source: &Value, overrides: &[String]) -> Value {
    let (mine, theirs) = (flatten(own), flatten(source));
    let mut out = Map::new();
    for (path, value) in &theirs {
        if covers(overrides, path) {
            if let Some(value) = mine.get(path) {
                set(&mut out, path, value.clone());
            }
        } else {
            set(&mut out, path, value.clone());
        }
    }
    for (path, value) in &mine {
        if !theirs.contains_key(path) && covers(overrides, path) {
            set(&mut out, path, value.clone());
        }
    }
    Value::Object(out)
}

/// `merge` through a typed value: falls back to `own` when the merged JSON
/// no longer reads as `T` (an enum whose tag and fields came from different
/// sides, say).
pub fn merged<T: Serialize + serde::de::DeserializeOwned + Clone>(
    own: &T,
    source: &T,
    overrides: &[String],
) -> T {
    let (Ok(mine), Ok(theirs)) = (serde_json::to_value(own), serde_json::to_value(source)) else {
        return own.clone();
    };
    serde_json::from_value(merge(&mine, &theirs, overrides)).unwrap_or_else(|_| own.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_override_keeps_its_value_and_the_rest_follows() {
        let file = json!({"fog": {"density": 0.1, "color": "#fff"}, "sky": {"kind": "Flat"}});
        let mut own = file.clone();
        own["fog"]["density"] = json!(0.5);
        let mut overrides = Vec::new();
        mark(&file, &own, &file, &mut overrides);
        assert_eq!(overrides, ["fog.density"]);
        let next = json!({"fog": {"density": 0.2, "color": "#000"}, "sky": {"kind": "Physical"}});
        let out = merge(&own, &next, &overrides);
        assert_eq!(out["fog"]["density"], 0.5);
        assert_eq!(out["fog"]["color"], "#000");
        assert_eq!(out["sky"]["kind"], "Physical");
        // Put back to the file's value: no longer an override.
        let mut back = own.clone();
        back["fog"]["density"] = json!(0.2);
        mark(&own, &back, &next, &mut overrides);
        assert!(overrides.is_empty());
    }

    #[test]
    fn deeper_values_move_whole() {
        let file = json!({"fog": {"height": {"density": 1, "falloff": 2}}});
        let paths: Vec<String> = flatten(&file).into_keys().collect();
        assert_eq!(paths, ["fog.height"]);
    }
}
