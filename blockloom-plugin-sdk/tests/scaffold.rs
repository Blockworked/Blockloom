//! The source `plugin-new` writes is real code: it compiles here and behaves
//! as the schema it ships beside says.

#[path = "../templates/lib.rs"]
mod starter;

use blockloom_plugin_sdk::{Value, harness, json};

#[test]
fn the_scaffolded_plugin_greets_and_counts() {
    let plugin = harness!(starter::Starter).unwrap();
    let hello = plugin.call("greet", json!({"name": "Ada"})).unwrap();
    assert_eq!(hello["text"], "Hello, Ada!");
    assert_eq!(hello["value"], 1);
    assert_eq!(plugin.call("count", Value::Null).unwrap()["value"], 1);
    assert!(plugin.call("greet", json!({"name": ""})).is_err());
    assert!(plugin.call("nope", Value::Null).is_err());
}
