//! A plugin-owned record as a project document stores it.
//!
//! The document never interprets the payload: the owning plugin's schema
//! does. That is what makes a missing plugin harmless to the data - an
//! unresolved record is just a record nobody currently validates, and
//! opening and saving write it back byte for byte.

use crate::id::{qualified, split_qualified};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A component, resource or other persisted value owned by a plugin.
///
/// Identity is `plugin` + `type_id`, never a display name.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PluginRecord {
    pub plugin: String,
    pub type_id: String,
    /// The schema version the payload was written at.
    pub schema_version: u32,
    pub payload: Value,
    /// `plugin/type_id`, the name an actor's component list is keyed by.
    #[serde(skip)]
    key: String,
}

#[derive(Deserialize)]
struct RecordRepr {
    plugin: String,
    type_id: String,
    #[serde(default = "default_version")]
    schema_version: u32,
    #[serde(default)]
    payload: Value,
}

fn default_version() -> u32 {
    1
}

impl<'de> Deserialize<'de> for PluginRecord {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let repr = RecordRepr::deserialize(deserializer)?;
        Ok(PluginRecord::new(
            repr.plugin,
            repr.type_id,
            repr.schema_version,
            repr.payload,
        ))
    }
}

impl PluginRecord {
    pub fn new(
        plugin: impl Into<String>,
        type_id: impl Into<String>,
        schema_version: u32,
        payload: Value,
    ) -> Self {
        let plugin = plugin.into();
        let type_id = type_id.into();
        let key = qualified(&plugin, &type_id);
        Self {
            plugin,
            type_id,
            schema_version,
            payload,
            key,
        }
    }

    /// `plugin/type_id`.
    pub fn name(&self) -> &str {
        &self.key
    }

    /// Builds a record from its qualified name.
    pub fn from_name(name: &str, schema_version: u32, payload: Value) -> Option<Self> {
        let (plugin, type_id) = split_qualified(name)?;
        Some(Self::new(plugin, type_id, schema_version, payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn records_round_trip_losslessly() {
        let text = r#"{"plugin":"com.example.a","type_id":"Health","schema_version":3,"payload":{"hp":1,"extra":[1,{"x":null}]}}"#;
        let record: PluginRecord = serde_json::from_str(text).unwrap();
        assert_eq!(record.name(), "com.example.a/Health");
        let back: Value = serde_json::to_value(&record).unwrap();
        assert_eq!(back, serde_json::from_str::<Value>(text).unwrap());
    }

    #[test]
    fn old_records_without_a_version_read_as_one() {
        let record: PluginRecord =
            serde_json::from_value(json!({"plugin": "a.b", "type_id": "T"})).unwrap();
        assert_eq!(record.schema_version, 1);
        assert_eq!(record.payload, Value::Null);
    }
}
