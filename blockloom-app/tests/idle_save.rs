//! Save on idle: the window's edits reach the disk once they go quiet, or
//! at once on Play, Build and Close.

use blockloom_app::{AppHandle, Backend};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn on_disk(dir: &std::path::Path) -> String {
    let mut text = String::new();
    for entry in walk(dir) {
        if entry
            .extension()
            .is_some_and(|e| e == "blockloom" || e == "blockscene")
        {
            text.push_str(&std::fs::read_to_string(&entry).unwrap_or_default());
        }
    }
    text
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

#[test]
fn edits_wait_for_a_quiet_spell_and_close_writes_them() {
    let data = std::env::temp_dir().join(format!("blockloom-idle-{}", uuid::Uuid::new_v4()));
    let projects = data.join("projects");
    std::fs::create_dir_all(&projects).unwrap();
    unsafe {
        std::env::set_var("BLOCKLOOM_DATA_DIR", &data);
    }
    let backend = Backend::start(AppHandle::new(|_| {}));
    backend.save_when_idle(Duration::from_millis(300));
    let run = |command: &str, args: Value| {
        backend
            .dispatch(command, args.clone())
            .unwrap_or_else(|e| panic!("{command} {args}: {e}"))
    };
    run(
        "create_project",
        json!({"name": "Idle", "location": projects, "mode": "TwoD"}),
    );
    let dir = projects.join("Idle");
    run("add_actor", json!({"shape": "Circle", "name": "Quietly"}));
    assert!(!on_disk(&dir).contains("Quietly"), "saved straight away");
    let start = Instant::now();
    while !on_disk(&dir).contains("Quietly") {
        assert!(start.elapsed() < Duration::from_secs(5), "never saved");
        std::thread::sleep(Duration::from_millis(50));
    }
    run("add_actor", json!({"shape": "Circle", "name": "Closing"}));
    run("close_project", json!({}));
    assert!(on_disk(&dir).contains("Closing"), "close didn't save");
}
