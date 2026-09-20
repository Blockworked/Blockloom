//! The one shape difference between Blockloom's documents and the frontend's.
//!
//! blockstitch stores an instruction as `{"id": _, "kind": {"type": _, ...}}`
//! but its canvas wants the flat `{"id": _, "type": _, ...}` a `BlockNode` is.
//! Rather than hand-writing a DTO per block, these two functions flatten and
//! unflatten that one nesting generically, so adding a block to
//! [`crate::blocks::InstructionKind`] needs no wire code at all.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Serializes `value` and flattens every instruction in it.
pub fn to_wire<T: Serialize>(value: &T) -> Result<Value, String> {
    let mut json = serde_json::to_value(value).map_err(|e| e.to_string())?;
    flatten(&mut json);
    Ok(json)
}

/// Unflattens every instruction in `json` and deserializes it.
pub fn from_wire<T: DeserializeOwned>(mut json: Value) -> Result<T, String> {
    unflatten(&mut json);
    serde_json::from_value(json).map_err(|e| e.to_string())
}

/// `{"id": _, "kind": {...}}` -> `{"id": _, ...}`, innermost first so a
/// wrap block's body is already flat when its own turn comes.
fn flatten(json: &mut Value) {
    match json {
        Value::Array(items) => items.iter_mut().for_each(flatten),
        Value::Object(map) => {
            map.values_mut().for_each(flatten);
            let is_instruction = map.len() == 2
                && map.contains_key("id")
                && map.get("kind").is_some_and(Value::is_object);
            if is_instruction {
                let Some(Value::Object(kind)) = map.remove("kind") else {
                    return;
                };
                for (key, value) in kind {
                    map.insert(key, value);
                }
            }
        }
        _ => {}
    }
}

/// The inverse of [`flatten`]: an object carrying both an `id` and a `type` is
/// an instruction, so everything but the `id` moves back under `kind`.
fn unflatten(json: &mut Value) {
    match json {
        Value::Array(items) => items.iter_mut().for_each(unflatten),
        Value::Object(map) => {
            map.values_mut().for_each(unflatten);
            if map.contains_key("id") && map.contains_key("type") {
                let id = map.remove("id").unwrap_or(Value::Null);
                let kind = Value::Object(std::mem::take(map));
                map.insert("id".to_string(), id);
                map.insert("kind".to_string(), kind);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::{Instruction, InstructionKind};
    use crate::value::Value as BlockValue;

    fn nested() -> Instruction {
        Instruction::new(InstructionKind::If {
            condition: BlockValue::Bool,
            body: vec![Instruction::new(InstructionKind::Move {
                steps: BlockValue::number(10.0),
            })],
        })
    }

    #[test]
    fn an_instruction_flattens_to_the_frontends_block_node_shape() {
        let json = to_wire(&nested()).unwrap();
        assert_eq!(json["type"], "If");
        assert!(json.get("kind").is_none());
        let inner = &json["body"][0];
        assert_eq!(inner["type"], "Move");
        assert_eq!(inner["steps"]["kind"], "Number");
        assert!(inner["id"].is_string());
    }

    #[test]
    fn flattening_round_trips_through_unflattening() {
        let before = nested();
        let json = to_wire(&before).unwrap();
        let after: Instruction = from_wire(json).unwrap();
        assert_eq!(after.id, before.id);
        assert_eq!(after.kind, before.kind);
    }

    #[test]
    fn a_whole_project_round_trips() {
        let mut before = crate::project::Project::starter("p", crate::scene::Mode::ThreeD);
        before.actors[0]
            .graph
            .strands
            .push(crate::blocks::Strand::with_instructions(
                10,
                20,
                vec![Instruction::new(InstructionKind::WhenStarted), nested()],
            ));
        let json = to_wire(&before).unwrap();
        let after: crate::project::Project = from_wire(json).unwrap();
        assert_eq!(after, before);
    }
}
