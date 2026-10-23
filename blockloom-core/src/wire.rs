//! The one shape difference between Blockloom's documents and the frontend's.
//!
//! blockstitch stores an instruction as `{"id": _, "kind": {"type": _, ...}}`
//! but its canvas wants the flat `{"id": _, "type": _, ...}` a `BlockNode` is.
//! blockstitch's typed codec flattens exactly the [`Instruction`] positions a
//! value holds, so adding a block to [`crate::blocks::InstructionKind`] needs
//! no wire code, and plugin payloads and other host JSON pass through as
//! they are. A kind field may not be called `id`.
//!
//! [`Instruction`]: crate::blocks::Instruction

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Serializes `value` with every instruction in it flattened.
pub fn to_wire<T: Serialize>(value: &T) -> Result<Value, String> {
    blockstitch_core::graph::to_frontend(value).map_err(|e| e.to_string())
}

/// Deserializes `json`, reading every instruction in it as flat (the stored
/// `{id, kind}` shape is accepted too).
pub fn from_wire<T: DeserializeOwned>(json: Value) -> Result<T, String> {
    blockstitch_core::graph::from_frontend(json).map_err(|e| e.to_string())
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

    #[test]
    fn plugin_payloads_keep_their_id_kind_and_type_keys() {
        use blockloom_plugin_api::record::PluginRecord;
        let payload = serde_json::json!({
            "id": "cell",
            "kind": {"solid": true},
            "nested": [{"id": "a", "type": "Move", "steps": 3}],
        });
        let mut before = crate::project::Project::starter("p", crate::scene::Mode::ThreeD);
        before.plugin_resources.push(PluginRecord::new(
            "com.example.test",
            "grid",
            1,
            payload.clone(),
        ));
        let json = to_wire(&before).unwrap();
        let wired = json["plugin_resources"][0]["payload"].clone();
        assert_eq!(wired, payload);
        let after: crate::project::Project = from_wire(json).unwrap();
        assert_eq!(after, before);
    }

    #[test]
    fn the_stored_nested_shape_is_still_accepted() {
        let before = nested();
        let stored = serde_json::to_value(&before).unwrap();
        assert!(stored["kind"].is_object());
        let after: Instruction = from_wire(stored).unwrap();
        assert_eq!(after.id, before.id);
        assert_eq!(after.kind, before.kind);
    }
}
