//! The Hub's side of the files it shares with the editor, held against the
//! fixtures `blockloom-core/tests/hub_contract.rs` reads too.

use blockloom_hub_state::Hub;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../blockloom-core/tests/fixtures/hub-contract")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn keys(value: &Value) -> BTreeSet<String> {
    value.as_object().unwrap().keys().cloned().collect()
}

fn scratch() -> (tempfile::TempDir, PathBuf, Hub) {
    let dir = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let project = base.join("Pond");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("project.blockloom"),
        json!({"name": "Pond", "scenes": []}).to_string(),
    )
    .unwrap();
    let hub = Hub::new(Some(base.join("Hub")), Some(base.join("data/projects.json")));
    (dir, project, hub)
}

#[test]
fn the_hub_reads_the_editors_lock_and_binding() {
    let (_dir, project, hub) = scratch();
    let mut lock = fixture("lock.json");
    assert!(keys(&lock).is_superset(&BTreeSet::from(["pid".into(), "heartbeat".into(), "app".into()])));
    lock["pid"] = json!(std::process::id());
    lock["heartbeat"] = json!(blockloom_core::sync::now_secs());
    std::fs::create_dir_all(project.join(".blockloom")).unwrap();
    std::fs::write(project.join(".blockloom/lock.json"), lock.to_string()).unwrap();
    let owner = hub.project_owner(&project).unwrap();
    assert_eq!(owner["pid"], std::process::id());
    assert_eq!(owner["app"], fixture("lock.json")["app"]);
    std::fs::remove_file(project.join(".blockloom/lock.json")).unwrap();
    std::fs::write(project.join(".blockloom/hub.json"), fixture("hub.json").to_string()).unwrap();
    assert_eq!(
        hub.project(&project).unwrap()["installation"],
        fixture("hub.json")["installation"]
    );
}

#[test]
fn what_the_hub_writes_has_the_editors_keys() {
    let (_dir, project, hub) = scratch();
    hub.remember(&project).unwrap();
    let library: Value = serde_json::from_str(&std::fs::read_to_string(&hub.registry).unwrap()).unwrap();
    let reference = fixture("projects.json");
    assert_eq!(keys(&library), keys(&reference));
    assert_eq!(keys(&library["projects"][0]), keys(&reference["projects"][0]));

    std::fs::create_dir_all(project.join(".blockloom")).unwrap();
    std::fs::write(
        project.join(".blockloom/lock.json"),
        json!({"pid": std::process::id(), "session": "s", "heartbeat": blockloom_core::sync::now_secs(), "app": "blockloom"})
            .to_string(),
    )
    .unwrap();
    let mut quick = hub.clone();
    quick.focus_timeout = std::time::Duration::ZERO;
    quick.focus(&project).unwrap();
    let focus: Value =
        serde_json::from_str(&std::fs::read_to_string(project.join(".blockloom/focus.json")).unwrap()).unwrap();
    assert_eq!(keys(&focus), keys(&fixture("focus.json")));
}

#[test]
fn the_hub_reads_the_presence_the_editor_writes() {
    let presence: blockloom_core::hub::Presence = serde_json::from_value(fixture("presence.json")).unwrap();
    assert_eq!(presence.installation.as_deref(), Some("release-0.1.0"));
    assert!(presence.project.is_none());
}
