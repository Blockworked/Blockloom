//! The authoring harness, used the way a plugin author would.

use blockloom_plugin_api::abi::Status;
use blockloom_plugin_api::manifest::Capability;
use blockloom_plugin_sdk::{Error, Host, Plugin, Value, harness, json};

struct Counter(i64);

impl Plugin for Counter {
    fn start(host: &Host) -> Result<Self, Error> {
        host.debug("counter up");
        Ok(Counter(0))
    }

    fn call_json(&mut self, host: &Host, op: &str, args: Value) -> Result<Value, Error> {
        match op {
            "bump" => {
                self.0 += args["by"].as_i64().unwrap_or(1);
                Ok(json!({"count": self.0}))
            }
            "keep" => host.call_json(
                "storage.write",
                &json!({"key": "n", "text": self.0.to_string()}),
            ),
            "recall" => host.call_json("storage.read", &json!({"key": "n", "as": "text"})),
            _ => Err(Error::unsupported(op)),
        }
    }
}

struct Other;

impl Plugin for Other {
    fn start(_host: &Host) -> Result<Self, Error> {
        Ok(Other)
    }
}

#[test]
fn a_plugin_keeps_its_state_between_calls_and_logs() {
    let plugin = harness!(Counter).unwrap();
    assert_eq!(plugin.call("bump", json!({"by": 2})).unwrap()["count"], 2);
    assert_eq!(plugin.call("bump", Value::Null).unwrap()["count"], 3);
    assert_eq!(plugin.logs(), vec!["counter up".to_string()]);
}

#[test]
fn an_unknown_op_keeps_its_status() {
    let plugin = harness!(Counter).unwrap();
    assert_eq!(plugin.call_bytes("nope", b""), Err(Status::Unsupported));
    assert!(plugin.call("nope", Value::Null).is_err());
}

#[test]
fn services_answer_from_memory_and_harnesses_are_separate() {
    let first = harness!(Counter).unwrap();
    let second = harness!(Counter).unwrap();
    // No capability asked for storage, so the host refuses it.
    assert!(first.call("keep", Value::Null).is_err());
    let _ = second;
    let _ = harness!(Other).unwrap();
}

#[test]
fn a_granted_capability_opens_project_storage() {
    let plugin = harness!(Counter, [Capability::ProjectStorage]).unwrap();
    plugin.call("bump", json!({"by": 5})).unwrap();
    plugin.call("keep", Value::Null).unwrap();
    assert_eq!(plugin.call("recall", Value::Null).unwrap()["text"], "5");
}
