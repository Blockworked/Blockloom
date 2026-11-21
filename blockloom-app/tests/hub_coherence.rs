//! The Hub coherence rules the editor keeps: version routing, ownership
//! checks on delete, failed-save navigation and a library that never
//! forgets a project because its drive went away.

use blockloom_app::{AppHandle, Backend};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct TempDir(PathBuf);

impl TempDir {
    fn fresh(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "blockloom-hubtest-{tag}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn isolated(test: &str) -> (std::sync::MutexGuard<'static, ()>, TempDir, TempDir) {
    let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = TempDir::fresh(&format!("{test}-data"));
    unsafe {
        std::env::set_var("BLOCKLOOM_DATA_DIR", data.path());
        std::env::remove_var("BLOCKLOOM_INSTALLATION");
        std::env::set_var("XDG_DATA_HOME", data.path().join("xdg"));
    }
    let projects = TempDir::fresh(&format!("{test}-projects"));
    (guard, data, projects)
}

fn editor(installation: &str) -> Backend {
    unsafe { std::env::set_var("BLOCKLOOM_INSTALLATION", installation) };
    Backend::start(AppHandle::new(|_| {}))
}

fn make(backend: &Backend, parent: &Path, name: &str) -> PathBuf {
    backend
        .dispatch(
            "create_project",
            json!({"name": name, "location": parent.to_string_lossy()}),
        )
        .expect("create-project");
    backend.dispatch("close_project", json!({})).expect("close");
    parent.join(name)
}

fn bind(dir: &Path, installation: &str) {
    let folder = dir.join(".blockloom");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("hub.json"),
        json!({"schema": 1, "installation": installation}).to_string(),
    )
    .unwrap();
}

fn library(backend: &Backend) -> Vec<Value> {
    backend.dispatch("refresh_library", json!({})).unwrap();
    let state = backend.dispatch("get_state", json!({})).unwrap();
    state["library"].as_array().cloned().unwrap_or_default()
}

#[test]
fn a_project_bound_to_another_editor_is_not_opened_silently() {
    let (_lock, _data, projects) = isolated("routing");
    let v1 = editor("release-0.1.0");
    let a = make(&v1, projects.path(), "ProjectA");
    let b = make(&v1, projects.path(), "ProjectB");
    bind(&a, "release-0.1.0");
    bind(&b, "release-0.2.0");

    v1.dispatch("open_project", json!({"path": a.to_string_lossy()}))
        .expect("the matching editor opens its project");
    let refused = v1
        .dispatch("open_project", json!({"path": b.to_string_lossy()}))
        .expect_err("a bound project is not opened by another editor");
    assert!(refused.starts_with("wrong-editor:"), "{refused}");
    assert!(refused.contains("Blockloom 0.2.0"), "{refused}");
    assert!(
        !refused.contains("release-0.2.0"),
        "ids stay out of messages: {refused}"
    );
    // The refusal left project A open and untouched.
    let state = v1.dispatch("get_state", json!({})).unwrap();
    assert_eq!(
        state["project_path"].as_str().map(PathBuf::from),
        Some(std::fs::canonicalize(&a).unwrap_or(a.clone()))
            .filter(|p| p == &a)
            .or(Some(a.clone()))
    );

    // An explicit decision opens it, and the binding stays where it was.
    v1.dispatch(
        "open_project",
        json!({"path": b.to_string_lossy(), "other_editor": true}),
    )
    .expect("explicit open");
    assert_eq!(
        blockloom_core::hub::bound_installation(&b).as_deref(),
        Some("release-0.2.0")
    );
}

#[test]
fn opening_anyway_keeps_the_old_file_and_can_switch_the_project() {
    let (_lock, _data, projects) = isolated("anyway");
    let v1 = editor("release-0.1.0");
    let b = make(&v1, projects.path(), "ProjectB");
    bind(&b, "release-0.2.0");
    let before = std::fs::read(b.join("project.blockloom")).unwrap();

    v1.dispatch(
        "open_project",
        json!({"path": b.to_string_lossy(), "other_editor": true}),
    )
    .expect("open once");
    let copies: Vec<_> = std::fs::read_dir(b.join(".blockloom/backups"))
        .expect("a backup folder")
        .collect();
    assert_eq!(copies.len(), 1);
    assert_eq!(
        std::fs::read(copies[0].as_ref().unwrap().path()).unwrap(),
        before
    );
    assert_eq!(
        blockloom_core::hub::bound_installation(&b).as_deref(),
        Some("release-0.2.0"),
        "open once leaves the binding alone"
    );

    v1.dispatch("close_project", json!({})).unwrap();
    v1.dispatch(
        "open_project",
        json!({"path": b.to_string_lossy(), "other_editor": true, "rebind": true}),
    )
    .expect("open and switch");
    assert_eq!(
        blockloom_core::hub::bound_installation(&b).as_deref(),
        Some("release-0.1.0")
    );
}

#[test]
fn library_cards_name_the_editor_and_flag_a_mismatch() {
    let (_lock, _data, projects) = isolated("cards");
    let v1 = editor("release-0.1.0");
    let ok = make(&v1, projects.path(), "Fine");
    let other = make(&v1, projects.path(), "Elsewhere");
    bind(&ok, "release-0.1.0");
    bind(&other, "dev-0123456789abcdef");
    let cards = library(&v1);
    let card = |name: &str| cards.iter().find(|c| c["name"] == name).unwrap().clone();
    assert_eq!(card("Fine")["installation_label"], "Blockloom 0.1.0");
    assert_eq!(card("Fine")["wrong_editor"], false);
    assert_eq!(card("Elsewhere")["installation_label"], "Development build");
    assert_eq!(card("Elsewhere")["wrong_editor"], true);
}

#[test]
fn an_unmanaged_editor_must_decide_too_and_unbound_projects_open_freely() {
    let (_lock, _data, projects) = isolated("unmanaged");
    let managed = editor("release-0.1.0");
    let free = make(&managed, projects.path(), "Free");
    let bound = make(&managed, projects.path(), "Bound");
    std::fs::remove_file(free.join(".blockloom/hub.json")).ok();
    bind(&bound, "release-0.1.0");
    unsafe { std::env::remove_var("BLOCKLOOM_INSTALLATION") };
    let bare = Backend::start(AppHandle::new(|_| {}));
    bare.dispatch("open_project", json!({"path": free.to_string_lossy()}))
        .expect("unbound opens");
    let err = bare
        .dispatch("open_project", json!({"path": bound.to_string_lossy()}))
        .expect_err("bound refuses an unmanaged editor");
    assert!(err.starts_with("wrong-editor:"), "{err}");
}

#[test]
fn deleting_a_project_another_editor_owns_is_refused() {
    let (_lock, _data, projects) = isolated("delete");
    let first = editor("release-0.1.0");
    let dir = make(&first, projects.path(), "Owned");
    first
        .dispatch("open_project", json!({"path": dir.to_string_lossy()}))
        .unwrap();

    let second = editor("release-0.1.0");
    let err = second
        .dispatch("delete_project", json!({"path": dir.to_string_lossy()}))
        .expect_err("owned folder");
    assert!(err.contains("open in another"), "{err}");
    assert!(dir.join("project.blockloom").is_file());

    // Forgetting never touches the folder, owned or not.
    second
        .dispatch("forget_project", json!({"path": dir.to_string_lossy()}))
        .expect("forget");
    assert!(dir.join("project.blockloom").is_file());

    // Once released, deletion works.
    first.dispatch("close_project", json!({})).unwrap();
    second
        .dispatch("delete_project", json!({"path": dir.to_string_lossy()}))
        .expect("unowned delete");
    assert!(!dir.exists());
    #[cfg(target_os = "linux")]
    {
        // It went to the trash, not into thin air.
        let trashed = _data.path().join("xdg/Trash/files/Owned/project.blockloom");
        assert!(trashed.is_file(), "{}", trashed.display());
        assert!(
            _data
                .path()
                .join("xdg/Trash/info/Owned.trashinfo")
                .is_file()
        );
    }
}

#[test]
fn permanent_delete_skips_the_trash() {
    let (_lock, _data, projects) = isolated("permanent");
    let backend = editor("release-0.1.0");
    let dir = make(&backend, projects.path(), "Gone");
    backend
        .dispatch(
            "delete_project",
            json!({"path": dir.to_string_lossy(), "permanent": true}),
        )
        .unwrap();
    assert!(!dir.exists());
    assert!(!_data.path().join("xdg/Trash/files/Gone").exists());
}

#[test]
fn a_hub_focus_request_is_taken_once_by_the_owner() {
    let (_lock, _data, projects) = isolated("focus");
    let backend = editor("release-0.1.0");
    let dir = make(&backend, projects.path(), "Front");
    assert_eq!(
        backend.dispatch("take_focus_request", json!({})).unwrap(),
        json!(false)
    );
    backend
        .dispatch("open_project", json!({"path": dir.to_string_lossy()}))
        .unwrap();
    let now = blockloom_core::sync::now_secs();
    std::fs::write(
        dir.join(".blockloom/focus.json"),
        json!({"requested_at": now}).to_string(),
    )
    .unwrap();
    assert_eq!(
        backend.dispatch("take_focus_request", json!({})).unwrap(),
        json!(true)
    );
    assert_eq!(
        backend.dispatch("take_focus_request", json!({})).unwrap(),
        json!(false)
    );
}

#[cfg(unix)]
#[test]
fn a_failed_save_blocks_navigation_until_discard() {
    use std::os::unix::fs::PermissionsExt;
    let (_lock, _data, projects) = isolated("save");
    let backend = editor("release-0.1.0");
    let dir = make(&backend, projects.path(), "Fragile");
    backend
        .dispatch("open_project", json!({"path": dir.to_string_lossy()}))
        .unwrap();

    // A read-only project file is what an unwritable folder looks like to a save.
    let file = dir.join("project.blockloom");
    let set =
        |mode| std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
    set(0o444);
    // Root ignores permissions; nothing to prove there.
    if std::fs::OpenOptions::new().append(true).open(&file).is_ok() {
        set(0o644);
        return;
    }
    // Something unsaved: the edit's own save fails quietly and leaves it dirty.
    backend
        .dispatch("add_actor", json!({"shape": "Circle", "name": "Unsaved"}))
        .expect("edit");
    let err = backend
        .dispatch("close_project", json!({}))
        .expect_err("a failed save stops the close");
    assert!(err.starts_with("save-failed:"), "{err}");
    assert!(err.contains("Fragile"), "{err}");
    assert!(err.contains(&dir.display().to_string()), "{err}");
    let state = backend.dispatch("get_state", json!({})).unwrap();
    assert!(state["project"].is_object(), "the project stays open");

    let other = make_other(&backend, projects.path());
    let err = backend
        .dispatch("open_project", json!({"path": other.to_string_lossy()}))
        .expect_err("opening another project is also blocked");
    assert!(err.starts_with("save-failed:"), "{err}");

    backend
        .dispatch("close_project", json!({"discard": true}))
        .expect("explicit discard");
    let state = backend.dispatch("get_state", json!({})).unwrap();
    assert!(state["project"].is_null());
    set(0o644);
}

#[cfg(unix)]
fn make_other(_backend: &Backend, parent: &Path) -> PathBuf {
    let dir = parent.join("Other");
    let project = blockloom_core::project::Project::starter(
        "Other".to_string(),
        blockloom_core::scene::Mode::TwoD,
    );
    blockloom_core::project::create_project(&project, parent).unwrap();
    dir
}

#[test]
fn a_missing_folder_stays_listed_with_recovery_and_never_prunes() {
    let (_lock, data, projects) = isolated("library");
    let backend = editor("release-0.1.0");
    let dir = make(&backend, projects.path(), "Away");
    let moved_to = projects.path().join("Away-elsewhere");
    std::fs::rename(&dir, &moved_to).unwrap();

    let entries = library(&backend);
    let away = entries
        .iter()
        .find(|e| e["name"] == "Away")
        .expect("still listed");
    assert_eq!(away["available"], false);
    assert!(away["error"].as_str().unwrap().contains("missing"));
    let registry = std::fs::read_to_string(data.path().join("blockloom/projects.json")).unwrap();
    assert!(
        registry.contains("Away"),
        "the registry keeps it: {registry}"
    );

    // Listing again (a second backend) still keeps it, and the folder coming
    // back makes it available with no action.
    std::fs::rename(&moved_to, &dir).unwrap();
    let entries = library(&editor("release-0.1.0"));
    let away = entries.iter().find(|e| e["name"] == "Away").unwrap();
    assert_eq!(away["available"], true);

    // Remove from list drops the entry and leaves the folder.
    backend
        .dispatch("forget_project", json!({"path": dir.to_string_lossy()}))
        .unwrap();
    assert!(dir.join("project.blockloom").is_file());
    assert!(library(&backend).iter().all(|e| e["name"] != "Away"));
}

#[test]
fn locate_points_a_missing_entry_at_its_new_folder() {
    let (_lock, _data, projects) = isolated("locate");
    let backend = editor("release-0.1.0");
    let dir = make(&backend, projects.path(), "Roamer");
    let new_home = projects.path().join("Elsewhere");
    std::fs::rename(&dir, &new_home).unwrap();
    assert!(
        backend
            .dispatch(
                "locate_project",
                json!({"path": dir.to_string_lossy(), "folder": projects.path().to_string_lossy()}),
            )
            .is_err(),
        "a folder that holds no project is refused"
    );
    backend
        .dispatch(
            "locate_project",
            json!({"path": dir.to_string_lossy(), "folder": new_home.to_string_lossy()}),
        )
        .expect("locate");
    let entries = library(&backend);
    let roamer: Vec<_> = entries.iter().filter(|e| e["name"] == "Roamer").collect();
    assert_eq!(roamer.len(), 1);
    assert_eq!(roamer[0]["available"], true);
}

fn presence() -> Value {
    let dir = blockloom_core::hub::editors_dir();
    let path = blockloom_core::hub::presence_path(&dir, std::process::id());
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn ask(action: &str, id: &str, path: Option<&Path>) {
    let request = blockloom_core::hub::Request {
        schema: 1,
        id: id.to_string(),
        action: action.to_string(),
        path: path.map(|p| p.to_string_lossy().into_owned()),
        project: None,
        requested_at: blockloom_core::sync::now_secs(),
    };
    blockloom_core::hub::write_file_atomic(
        &blockloom_core::hub::request_path(&blockloom_core::hub::editors_dir(), std::process::id()),
        &serde_json::to_string(&request).unwrap(),
    )
    .unwrap();
}

#[test]
fn an_editor_at_its_dashboard_hands_over_the_hubs_open_request() {
    let (_lock, _data, projects) = isolated("reuse");
    let backend = editor("release-0.1.0");
    let dir = make(&backend, projects.path(), "Wanted");
    let poll = backend.dispatch("hub_poll", json!({})).unwrap();
    assert_eq!(poll["open"], Value::Null);
    let said = presence();
    assert_eq!(said["installation"], "release-0.1.0");
    assert_eq!(said["project"], Value::Null);

    ask("open", "r1", Some(&dir));
    let poll = backend.dispatch("hub_poll", json!({})).unwrap();
    assert_eq!(poll["open"]["id"], "r1");
    assert_eq!(poll["open"]["path"], dir.to_string_lossy().as_ref());
    // The window carries it out and answers.
    backend
        .dispatch("open_project", json!({"path": dir.to_string_lossy()}))
        .unwrap();
    backend.dispatch("hub_reply", json!({"id": "r1"})).unwrap();
    let reply =
        blockloom_core::hub::reply_path(&blockloom_core::hub::editors_dir(), std::process::id());
    let reply: Value = serde_json::from_str(&std::fs::read_to_string(reply).unwrap()).unwrap();
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["id"], "r1");
    // Now busy: presence names the project, and a second request is refused.
    backend.dispatch("hub_poll", json!({})).unwrap();
    assert_eq!(presence()["project"], dir.to_string_lossy().as_ref());
    ask("open", "r2", Some(&dir));
    let poll = backend.dispatch("hub_poll", json!({})).unwrap();
    assert_eq!(poll["open"], Value::Null);
    let refused: Value = serde_json::from_str(
        &std::fs::read_to_string(blockloom_core::hub::reply_path(
            &blockloom_core::hub::editors_dir(),
            std::process::id(),
        ))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["id"], "r2");
}
