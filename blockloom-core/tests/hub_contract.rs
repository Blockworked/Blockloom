//! The file formats the editor shares with the Hub, held against fixtures the
//! Hub's tests (`scripts/test-hub.py`) read too.

use blockloom_core::{hub, library, sync};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/hub-contract")
        .join(name)
}

fn keys(value: &Value) -> BTreeSet<String> {
    value.as_object().unwrap().keys().cloned().collect()
}

fn json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("blockloom-contract-{tag}-{}", sync::new_session()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_editor_reads_the_hub_binding() {
    let dir = scratch("bind");
    std::fs::create_dir_all(dir.join(".blockloom")).unwrap();
    std::fs::copy(fixture("hub.json"), dir.join(".blockloom/hub.json")).unwrap();
    assert_eq!(hub::bound_installation(&dir).as_deref(), Some("release-0.1.0"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_binding_the_editor_writes_has_the_hubs_keys() {
    let dir = scratch("rebind");
    unsafe { std::env::set_var(hub::INSTALLATION_ENV, "release-0.1.0") };
    assert!(hub::rebind(&dir).unwrap());
    assert_eq!(
        keys(&json(&dir.join(".blockloom/hub.json"))),
        keys(&json(&fixture("hub.json")))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_lock_the_editor_writes_carries_what_the_hub_reads() {
    let reference = json(&fixture("lock.json"));
    // The Hub reads pid, heartbeat and app; all must stay in the lock.
    let dir = scratch("lock");
    std::fs::create_dir_all(dir.join(".blockloom")).unwrap();
    std::fs::copy(fixture("lock.json"), dir.join(".blockloom/lock.json")).unwrap();
    let read = sync::read_lock(&dir).expect("the fixture lock parses");
    assert_eq!(read.pid, 4242);
    assert_eq!(read.app, "blockloom");
    sync::write_lock(&dir, &read).unwrap();
    assert_eq!(keys(&json(&sync::lock_path(&dir))), keys(&reference));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_library_file_the_editor_writes_has_the_hubs_keys() {
    let data = scratch("data");
    unsafe { std::env::set_var("BLOCKLOOM_DATA_DIR", &data) };
    let project = scratch("pond");
    library::remember(&project);
    let written = json(&data.join("blockloom/projects.json"));
    let reference = json(&fixture("projects.json"));
    assert_eq!(keys(&written), keys(&reference));
    assert_eq!(
        keys(&written["projects"][0]),
        keys(&reference["projects"][0])
    );
    let _ = std::fs::remove_dir_all(&data);
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
fn the_files_the_hub_and_editor_exchange_read_and_write_with_the_same_keys() {
    let dir = scratch("exchange");
    // Written by the editor, read by the Hub.
    unsafe { std::env::set_var(hub::INSTALLATION_ENV, "release-0.1.0") };
    hub::publish_presence(&dir, None);
    let written = json(&hub::presence_path(&dir, std::process::id()));
    assert_eq!(keys(&written), keys(&json(&fixture("presence.json"))));
    hub::publish_reply(&dir, "abc", Err("no".into()));
    let reply = json(&hub::reply_path(&dir, std::process::id()));
    assert_eq!(keys(&reply), keys(&json(&fixture("reply.json"))));
    // Written by the Hub, read by the editor.
    let request: hub::Request = serde_json::from_value(json(&fixture("request.json"))).unwrap();
    assert_eq!(request.action, "open");
    assert_eq!(
        keys(&serde_json::to_value(&request).unwrap()),
        keys(&json(&fixture("request.json")))
    );
    let _: hub::Presence = serde_json::from_value(json(&fixture("presence.json"))).unwrap();
    let _: hub::Reply = serde_json::from_value(json(&fixture("reply.json"))).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
