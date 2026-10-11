//! The Hub's project and editor logic against fake installations and a fake
//! editor script, so no editor has to be built.

use blockloom_core::{hub as contract, sync};
use blockloom_hub_state::{Hub, installs::host_target, process};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::TempDir;

struct Fixture {
    _dir: TempDir,
    base: PathBuf,
    hub: Hub,
    project: PathBuf,
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn write_executable(path: &Path, text: &str) {
    write(path, text);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// A "blockloom" that claims `--project <dir>` the way an editor does, then
/// idles until killed.
const FAKE_EDITOR: &str = r#"#!/bin/sh
if [ "$1" = "--project" ]; then
  mkdir -p "$2/.blockloom"
  printf '{"pid": %s, "session": "fake", "heartbeat": %s, "app": "blockloom"}' "$$" "$(date +%s)" > "$2/.blockloom/lock.json"
fi
exec sleep 30
"#;

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let mut hub = Hub::new(Some(base.join("Hub")), Some(base.join("data/projects.json")));
    hub.open_timeout = Duration::from_millis(0);
    hub.focus_timeout = Duration::from_millis(0);
    hub.reuse_timeout = Duration::from_millis(300);
    let project = base.join("A project with spaces");
    write(
        &project.join("project.blockloom"),
        &json!({"name": "Demo", "scenes": [{"id": "main", "path": "main.blockscene", "mode": "ThreeD"}],
                "active_scene": "main"})
        .to_string(),
    );
    Fixture { _dir: dir, base, hub, project }
}

impl Fixture {
    fn install(&self, version: &str) -> String {
        let id = format!("release-{version}");
        let slot = self.hub.root.join("installations").join(&id);
        write_executable(&slot.join("blockloom"), FAKE_EDITOR);
        write_executable(&slot.join("blockloom-runtime"), "#!/bin/sh\n");
        let rust = slot.join("tools/rust");
        write_executable(&rust.join("bin/rustc"), "#!/bin/sh\n");
        write_executable(&rust.join("bin/cargo"), "#!/bin/sh\n");
        std::fs::create_dir_all(rust.join("lib/rustlib").join(host_target()).join("lib")).unwrap();
        write(
            &slot.join("blockloom-installation.json"),
            &json!({"id": id, "kind": "release", "version": version, "target": host_target(),
                    "status": "ready", "tools": {"rust": {"channel": "1.98.1"}}})
            .to_string(),
        );
        id
    }

    fn lock(&self, pid: u32) {
        write(
            &self.project.join(".blockloom/lock.json"),
            &json!({"pid": pid, "session": "s", "heartbeat": sync::now_secs(), "app": "blockloom"}).to_string(),
        );
    }
}

fn kill(pid: u64) {
    let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
}

/// An editor stand-in: a live process with a fresh presence file.
fn idle_editor(f: &Fixture, installation: &str) -> std::process::Child {
    let child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
    publish(f, child.id(), installation, None);
    child
}

fn publish(f: &Fixture, pid: u32, installation: &str, project: Option<&str>) {
    let presence = contract::Presence {
        schema: 1,
        pid,
        installation: Some(installation.to_string()),
        project: project.map(str::to_string),
        heartbeat: sync::now_secs(),
    };
    contract::write_file_atomic(
        &contract::presence_path(&f.hub.editors, pid),
        &serde_json::to_string(&presence).unwrap(),
    )
    .unwrap();
}

/// Answers the next request this editor stand-in receives.
fn respond(f: &Fixture, pid: u32, outcome: Result<(), String>) -> std::thread::JoinHandle<contract::Request> {
    let editors = f.hub.editors.clone();
    std::thread::spawn(move || {
        let path = contract::request_path(&editors, pid);
        for _ in 0..200 {
            if let Ok(text) = std::fs::read_to_string(&path) {
                let request: contract::Request = serde_json::from_str(&text).unwrap();
                std::fs::remove_file(&path).unwrap();
                let reply = contract::Reply {
                    schema: 1,
                    id: request.id.clone(),
                    ok: outcome.is_ok(),
                    error: outcome.clone().err(),
                };
                contract::write_file_atomic(
                    &contract::reply_path(&editors, pid),
                    &serde_json::to_string(&reply).unwrap(),
                )
                .unwrap();
                return request;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("no request arrived");
    })
}

#[test]
fn listing_keeps_missing_projects_with_a_cause_and_is_read_only() {
    let f = fixture();
    f.hub.remember(&f.project).unwrap();
    let gone = f.base.join("gone");
    write(
        &f.hub.registry,
        &json!({"projects": [
            {"path": f.project, "opened_at": 5},
            {"path": gone, "opened_at": 9},
        ]})
        .to_string(),
    );
    let before = std::fs::read_to_string(&f.hub.registry).unwrap();
    let rows = f.hub.projects().unwrap();
    assert_eq!(std::fs::read_to_string(&f.hub.registry).unwrap(), before);
    assert_eq!(rows[0]["name"], "Demo");
    assert_eq!(rows[0]["mode"], "ThreeD");
    assert_eq!(rows[0]["available"], true);
    assert_eq!(rows[1]["available"], false);
    assert!(rows[1]["error"].as_str().unwrap().contains("missing"));
}

#[test]
fn remember_forget_and_relocate_edit_the_one_shared_list() {
    let f = fixture();
    f.hub.remember(&f.project).unwrap();
    f.hub.remember(&f.project).unwrap();
    assert_eq!(f.hub.projects().unwrap().len(), 1);
    let moved = f.base.join("moved");
    std::fs::rename(&f.project, &moved).unwrap();
    assert_eq!(f.hub.projects().unwrap()[0]["available"], false);
    f.hub.relocate(&f.project, &moved).unwrap();
    let rows = f.hub.projects().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["available"], true);
    f.hub.forget(&moved).unwrap();
    assert!(f.hub.projects().unwrap().is_empty());
    assert!(moved.join("project.blockloom").exists(), "the folder is never touched");
}

#[test]
fn an_older_hub_only_list_folds_into_the_shared_one() {
    let f = fixture();
    write(
        &f.hub.root.join("projects.json"),
        &json!({"projects": [{"path": f.project, "opened_at": 3}]}).to_string(),
    );
    assert_eq!(f.hub.projects().unwrap().len(), 1);
    f.hub.remember(&f.project).unwrap();
    assert!(!f.hub.root.join("projects.json").exists());
    assert!(f.hub.registry.exists());
}

#[test]
fn the_owner_lock_decides_who_holds_a_project() {
    let f = fixture();
    f.hub.remember(&f.project).unwrap();
    assert!(f.hub.projects().unwrap()[0]["owner"].is_null());
    f.lock(std::process::id());
    let rows = f.hub.projects().unwrap();
    assert_eq!(rows[0]["active"], true);
    assert_eq!(rows[0]["owner"]["pid"], std::process::id());
    // A lock whose process is gone and whose heartbeat is old holds nothing.
    write(
        &f.project.join(".blockloom/lock.json"),
        &json!({"pid": 4_000_000, "session": "s", "heartbeat": 1, "app": "blockloom"}).to_string(),
    );
    assert_eq!(f.hub.projects().unwrap()[0]["active"], false);
}

#[test]
fn bind_writes_the_binding_and_refuses_a_busy_project() {
    let f = fixture();
    let id = f.install("0.1.0");
    f.hub.bind(&f.project, &id, false).unwrap();
    let written: Value = serde_json::from_str(&std::fs::read_to_string(f.project.join(".blockloom/hub.json")).unwrap()).unwrap();
    assert_eq!(written["installation"], id);
    f.lock(std::process::id());
    let error = f.hub.bind(&f.project, &id, false).unwrap_err();
    assert!(error.message.contains("Close this project"));
    assert!(f.hub.bind(&f.project, "../x", false).is_err());
}

#[test]
fn changing_editor_with_backup_snapshots_first() {
    let f = fixture();
    let old = f.install("0.1.0");
    let new = f.install("0.2.0");
    f.hub.bind(&f.project, &old, false).unwrap();
    let result = f.hub.bind(&f.project, &new, true).unwrap();
    assert!(Path::new(result["backup"]["path"].as_str().unwrap()).is_file());
    // The same editor again makes no second snapshot.
    let again = f.hub.bind(&f.project, &new, true).unwrap();
    assert!(again.get("backup").is_none());
    assert_eq!(f.hub.backups(&f.project).unwrap().len(), 1);
}

#[test]
fn backups_are_listed_and_restored_keeping_what_was_there() {
    let f = fixture();
    write(&f.project.join("assets/a.txt"), "one");
    let first = f.hub.backup_project(&f.project).unwrap();
    let name = Path::new(first["path"].as_str().unwrap()).file_name().unwrap().to_string_lossy().into_owned();
    write(&f.project.join("assets/a.txt"), "two");
    write(&f.project.join("assets/extra.txt"), "later");
    let restored = f.hub.restore_backup(&f.project, &name).unwrap();
    assert_eq!(std::fs::read_to_string(f.project.join("assets/a.txt")).unwrap(), "one");
    assert!(!f.project.join("assets/extra.txt").exists());
    assert!(Path::new(restored["previous"]["path"].as_str().unwrap()).is_file());
    assert_eq!(f.hub.backups(&f.project).unwrap().len(), 2);
}

#[test]
fn restore_refuses_a_busy_project_and_a_foreign_name() {
    let f = fixture();
    let made = f.hub.backup_project(&f.project).unwrap();
    let name = Path::new(made["path"].as_str().unwrap()).file_name().unwrap().to_string_lossy().into_owned();
    assert!(f.hub.restore_backup(&f.project, "../../etc/passwd").is_err());
    assert!(f.hub.restore_backup(&f.project, "missing.zip").is_err());
    f.lock(std::process::id());
    assert!(f.hub.restore_backup(&f.project, &name).is_err());
}

#[test]
fn a_backup_never_follows_links_or_includes_the_owner_lock() {
    let f = fixture();
    f.lock(std::process::id());
    let made = f.hub.backup_project(&f.project);
    // The project is held, so the snapshot is refused rather than torn.
    assert!(made.is_err());
    std::fs::remove_file(f.project.join(".blockloom/lock.json")).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc", f.project.join("link")).unwrap();
        let error = f.hub.backup_project(&f.project).unwrap_err();
        assert!(error.message.contains("links"));
    }
}

#[test]
fn the_operation_lock_serializes_and_a_dead_owner_does_not_block() {
    let f = fixture();
    let held = blockloom_hub_state::oplock::OperationLock::acquire(&f.hub.root).unwrap();
    let error = f.hub.remember(&f.project).unwrap_err();
    assert!(error.message.contains("Another Hub operation"));
    drop(held);
    write(
        &f.hub.root.join(".operation-lock/owner.json"),
        &json!({"pid": 4_000_000}).to_string(),
    );
    f.hub.remember(&f.project).unwrap();
}

#[test]
fn the_default_editor_is_the_newest_release() {
    let f = fixture();
    assert_eq!(f.hub.default_installation().unwrap(), None);
    f.install("0.2.0");
    f.install("0.10.0");
    f.install("0.9.0");
    assert_eq!(f.hub.default_installation().unwrap().as_deref(), Some("release-0.10.0"));
}

#[test]
fn usage_names_the_projects_bound_to_an_installation() {
    let f = fixture();
    let id = f.install("0.1.0");
    f.hub.remember(&f.project).unwrap();
    f.hub.bind(&f.project, &id, false).unwrap();
    let usage = f.hub.installation_usage(&id).unwrap();
    assert_eq!(usage["bound"], json!(["Demo"]));
    assert_eq!(usage["active"], json!([]));
    f.lock(std::process::id());
    assert_eq!(f.hub.installation_usage(&id).unwrap()["active"], json!(["Demo"]));
}

#[test]
fn focus_needs_an_owner_and_reports_whether_it_was_heard() {
    let f = fixture();
    f.hub.remember(&f.project).unwrap();
    assert!(f.hub.focus(&f.project).unwrap_err().message.contains("No editor"));
    f.lock(std::process::id());
    let result = f.hub.focus(&f.project).unwrap();
    assert_eq!(result["acknowledged"], false);
    let request: Value =
        serde_json::from_str(&std::fs::read_to_string(f.project.join(".blockloom/focus.json")).unwrap()).unwrap();
    assert_eq!(request["pid"], std::process::id());
    assert!(request["requested_at"].as_u64().unwrap() + 5 > sync::now_secs());
    // With a wait, an editor that takes the file is an acknowledgement, and
    // one that never does leaves nothing behind.
    let mut hub = f.hub.clone();
    hub.focus_timeout = Duration::from_millis(500);
    let focus = f.project.join(".blockloom/focus.json");
    let taker = {
        let focus = focus.clone();
        std::thread::spawn(move || {
            for _ in 0..100 {
                if focus.exists() {
                    std::fs::remove_file(&focus).unwrap();
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        })
    };
    let _ = std::fs::remove_file(&focus);
    assert_eq!(hub.focus(&f.project).unwrap()["acknowledged"], true);
    taker.join().unwrap();
    assert_eq!(hub.focus(&f.project).unwrap()["acknowledged"], false);
    assert!(!focus.exists());
}

#[cfg(unix)]
#[test]
fn opening_starts_an_editor_that_claims_the_project() {
    let mut f = fixture();
    let id = f.install("0.1.0");
    f.hub.bind(&f.project, &id, false).unwrap();
    f.hub.open_timeout = Duration::from_secs(5);
    let result = f.hub.launch(&f.project).unwrap();
    assert_eq!(result["opened"], true);
    assert_eq!(result["reused"], false);
    assert_eq!(result["installation"], id);
    let pid = result["pid"].as_u64().unwrap();
    assert_eq!(f.hub.installations().unwrap()[0]["running"], 1);
    assert!(f.hub.running_editors().iter().any(|r| r["pid"] == pid));
    kill(pid);
}

#[cfg(unix)]
#[test]
fn an_unbound_project_gets_the_default_editor() {
    let mut f = fixture();
    f.install("0.1.0");
    let newest = f.install("0.2.0");
    f.hub.open_timeout = Duration::from_secs(5);
    let result = f.hub.launch(&f.project).unwrap();
    assert_eq!(result["bound"], true);
    assert_eq!(result["installation"], newest);
    assert_eq!(f.hub.project(&f.project).unwrap()["installation"], newest);
    kill(result["pid"].as_u64().unwrap());
}

#[test]
fn opening_without_any_editor_installed_says_what_to_do() {
    let f = fixture();
    let error = f.hub.launch(&f.project).unwrap_err();
    assert!(error.message.contains("Install an editor first"));
}

#[cfg(unix)]
#[test]
fn an_idle_editor_of_the_right_installation_is_reused() {
    let f = fixture();
    let id = f.install("0.1.0");
    f.hub.bind(&f.project, &id, false).unwrap();
    let mut editor = idle_editor(&f, &id);
    let answered = respond(&f, editor.id(), Ok(()));
    let result = f.hub.launch(&f.project).unwrap();
    let request = answered.join().unwrap();
    assert_eq!(result["reused"], true);
    assert_eq!(result["opened"], true);
    assert_eq!(result["pid"], editor.id());
    assert_eq!(request.action, "open");
    assert_eq!(request.path.as_deref(), Some(f.project.to_str().unwrap()));
    assert!(f.hub.running_editors().is_empty(), "no process was started");
    let _ = editor.kill();
    let _ = editor.wait();
}

#[cfg(unix)]
#[test]
fn an_editor_that_is_busy_or_another_installation_is_not_reused() {
    let mut f = fixture();
    let id = f.install("0.1.0");
    let other = f.install("0.2.0");
    f.hub.bind(&f.project, &id, false).unwrap();
    f.hub.open_timeout = Duration::from_secs(5);
    let mut wrong = idle_editor(&f, &other);
    let mut busy = std::process::Command::new("sleep").arg("30").spawn().unwrap();
    publish(&f, busy.id(), &id, Some("/somewhere/else"));
    let result = f.hub.launch(&f.project).unwrap();
    assert_eq!(result["reused"], false);
    assert!(!contract::request_path(&f.hub.editors, wrong.id()).exists());
    assert!(!contract::request_path(&f.hub.editors, busy.id()).exists());
    kill(result["pid"].as_u64().unwrap());
    for child in [&mut wrong, &mut busy] {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(unix)]
#[test]
fn an_editor_that_never_answers_is_passed_over() {
    let mut f = fixture();
    let id = f.install("0.1.0");
    f.hub.bind(&f.project, &id, false).unwrap();
    f.hub.open_timeout = Duration::from_secs(5);
    let mut silent = idle_editor(&f, &id);
    let result = f.hub.launch(&f.project).unwrap();
    assert_eq!(result["reused"], false);
    assert_eq!(result["opened"], true);
    assert!(
        !contract::request_path(&f.hub.editors, silent.id()).exists(),
        "the unanswered request is withdrawn"
    );
    kill(result["pid"].as_u64().unwrap());
    let _ = silent.kill();
    let _ = silent.wait();
}

#[cfg(unix)]
#[test]
fn a_declined_request_is_an_error_not_a_second_editor() {
    let f = fixture();
    let id = f.install("0.1.0");
    f.hub.bind(&f.project, &id, false).unwrap();
    let mut editor = idle_editor(&f, &id);
    let answered = respond(&f, editor.id(), Err("This project is set to Blockloom 0.2.0.".into()));
    let error = f.hub.launch(&f.project).unwrap_err();
    answered.join().unwrap();
    assert!(error.message.contains("set to Blockloom 0.2.0"));
    assert!(f.hub.running_editors().is_empty());
    let _ = editor.kill();
    let _ = editor.wait();
}

#[cfg(unix)]
#[test]
fn a_new_project_is_created_by_an_idle_editor_when_there_is_one() {
    let f = fixture();
    let id = f.install("0.1.0");
    let mut editor = idle_editor(&f, &id);
    let answered = respond(&f, editor.id(), Ok(()));
    let location = f.base.join("Games");
    let result = f
        .hub
        .new_project("Pond", None, Some(location.to_str().unwrap()), "ThreeD", Some("physics-playground"))
        .unwrap();
    let request = answered.join().unwrap();
    assert_eq!(result["reused"], true);
    assert_eq!(request.action, "create");
    let spec = request.project.unwrap();
    assert_eq!(spec["name"], "Pond");
    assert_eq!(spec["mode"], "ThreeD");
    assert_eq!(spec["sample"], "physics-playground");
    assert_eq!(spec["location"], location.to_str().unwrap());
    let _ = editor.kill();
    let _ = editor.wait();
}

#[test]
fn a_new_project_needs_a_name_and_a_dimension() {
    let f = fixture();
    assert!(f.hub.new_project("  ", None, None, "TwoD", None).unwrap_err().message.contains("name"));
    assert!(f.hub.new_project("x", None, None, "FourD", None).unwrap_err().message.contains("Dimension"));
}

#[cfg(unix)]
#[test]
fn presence_of_a_dead_editor_is_pruned_and_never_reused() {
    let f = fixture();
    let id = f.install("0.1.0");
    publish(&f, 4_000_000, &id, None);
    assert!(!process::alive(4_000_000));
    f.hub.bind(&f.project, &id, false).unwrap();
    let started = f.hub.launch(&f.project).unwrap();
    assert_eq!(started["reused"], false);
    assert!(!contract::request_path(&f.hub.editors, 4_000_000).exists());
    kill(started["pid"].as_u64().unwrap());
}

#[test]
fn a_busy_project_is_not_snapshotted_by_a_bind_that_changes_editor() {
    let f = fixture();
    let old = f.install("0.1.0");
    let new = f.install("0.2.0");
    f.hub.bind(&f.project, &old, false).unwrap();
    f.lock(std::process::id());
    assert!(f.hub.bind(&f.project, &new, true).is_err());
    assert!(!f.hub.root.join("backups").exists());
    assert_eq!(f.hub.project(&f.project).unwrap()["installation"], old);
}

#[cfg(unix)]
#[test]
fn a_failed_snapshot_leaves_no_partial_archive_and_keeps_the_binding() {
    use std::os::unix::fs::PermissionsExt;
    let f = fixture();
    let old = f.install("0.1.0");
    let new = f.install("0.2.0");
    f.hub.bind(&f.project, &old, false).unwrap();
    let secret = f.project.join("assets/unreadable.bin");
    write(&secret, "x");
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::File::open(&secret).is_ok() {
        return; // running as root: permissions don't bind
    }
    assert!(f.hub.bind(&f.project, &new, true).is_err());
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(f.hub.project(&f.project).unwrap()["installation"], old);
    assert!(f.hub.backups(&f.project).unwrap().is_empty());
    assert!(!f.hub.root.join(".operation-lock").exists());
}
