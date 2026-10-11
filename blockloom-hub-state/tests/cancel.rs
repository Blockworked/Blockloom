//! A cancelled snapshot leaves the binding and no partial archive. This sets
//! a process-wide variable, so it is the only test in its own binary.

use blockloom_hub_state::Hub;
use serde_json::json;

#[test]
fn a_cancelled_backup_keeps_the_project_and_removes_the_partial() {
    let dir = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let project = base.join("Pond");
    std::fs::create_dir_all(project.join("assets")).unwrap();
    std::fs::write(project.join("project.blockloom"), json!({"name": "Pond", "scenes": []}).to_string()).unwrap();
    std::fs::write(project.join("assets/big.bin"), vec![7u8; 3 * 1024 * 1024]).unwrap();
    let hub = Hub::new(Some(base.join("Hub")), Some(base.join("data/projects.json")));
    let cancel = base.join("cancel");
    std::fs::write(&cancel, "").unwrap();
    unsafe { std::env::set_var(blockloom_hub_state::CANCEL_FILE_ENV, &cancel) };
    let error = hub.backup_project(&project).unwrap_err();
    assert!(error.cancelled, "{error}");
    let archives: Vec<_> = std::fs::read_dir(hub.root.join("backups"))
        .map(|d| d.flatten().flat_map(|e| std::fs::read_dir(e.path()).unwrap().flatten()).collect())
        .unwrap_or_default();
    assert!(archives.is_empty());
    assert!(!hub.root.join(".operation-lock").exists());
}
