//! Editor/headless project sync: locks, revisions, live reload, attach mode.
//!
//! Every test isolates `BLOCKLOOM_DATA_DIR` under a shared lock, since the
//! project registry and the attach socket both live under it and the
//! environment is process-global.

use blockloom_app::{AppHandle, Backend};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A scratch dir of its own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn fresh(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "blockloom-synctest-{tag}-{}",
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

/// Points the data dir at scratch space and hands back a projects parent.
/// Holds the env lock for the whole test through the returned guard.
fn isolated(test: &str) -> (std::sync::MutexGuard<'static, ()>, TempDir, TempDir) {
    let guard = ENV_LOCK.lock().unwrap();
    let data = TempDir::fresh(&format!("{test}-data"));
    unsafe {
        std::env::set_var("BLOCKLOOM_DATA_DIR", data.path());
    }
    let projects = TempDir::fresh(&format!("{test}-projects"));
    (guard, data, projects)
}

fn backend() -> Backend {
    Backend::start(AppHandle::new(|_| {}))
}

fn create_project(backend: &Backend, parent: &Path, name: &str) -> PathBuf {
    let result = backend
        .dispatch(
            "create_project",
            json!({"name": name, "location": parent.to_string_lossy()}),
        )
        .expect("create-project");
    assert_eq!(result, Value::Null);
    // Close it again: the maker only staged the folder, and an open-but-
    // dropped backend would keep looking like a live owner to the test.
    backend.dispatch("close_project", json!({})).expect("close");
    let dir = parent.join(name);
    assert!(dir.is_dir(), "project folder {}", dir.display());
    dir
}

fn state_text(backend: &Backend) -> String {
    let state = backend.dispatch("get_state", json!({})).expect("get-state");
    serde_json::to_string(&state).unwrap()
}

fn sync(backend: &Backend) -> Value {
    backend
        .dispatch("sync_status", json!({}))
        .expect("sync-status")
}

#[test]
fn second_opener_attaches_to_live_owner() {
    let (_lock, _data, projects) = isolated("attach");
    let dir = create_project(&backend(), projects.path(), "SyncGame");

    let first = backend();
    first
        .dispatch("open_project", json!({"path": dir.to_string_lossy()}))
        .expect("first open");
    let owner = sync(&first);
    assert_eq!(owner["owns_lock"], true);
    assert_eq!(owner["attached"], false);

    let second = backend();
    let report = second
        .dispatch("open_project", json!({"path": dir.to_string_lossy()}))
        .expect("second open");
    assert_eq!(report["attached"], true);
    assert_eq!(report["took_over"], false);
    assert!(
        report["warning"]
            .as_str()
            .unwrap()
            .contains("open in another")
    );
    assert_eq!(
        report["owner"]["session"], owner["session"],
        "the report names the live owner"
    );
    let follower = sync(&second);
    assert_eq!(follower["attached"], true);
    assert_eq!(follower["owns_lock"], false);
}

#[test]
fn force_open_takes_over_explicitly() {
    let (_lock, _data, projects) = isolated("takeover");
    let dir = create_project(&backend(), projects.path(), "SyncGame");

    let first = backend();
    first
        .dispatch("open_project", json!({"path": dir.to_string_lossy()}))
        .expect("first open");

    let second = backend();
    let report = second
        .dispatch(
            "open_project",
            json!({"path": dir.to_string_lossy(), "force": true}),
        )
        .expect("forced open");
    assert_eq!(report["attached"], false);
    assert_eq!(report["took_over"], true);
    assert_eq!(sync(&second)["owns_lock"], true);
}

#[test]
fn edits_converge_through_live_reload() {
    let (_lock, _data, projects) = isolated("converge");
    let dir = create_project(&backend(), projects.path(), "SyncGame");

    let editor = backend();
    editor
        .dispatch("open_project", json!({"path": dir.to_string_lossy()}))
        .expect("editor open");
    editor
        .dispatch(
            "add_actor",
            json!({"shape": "Circle", "name": "SyncProbeBrick"}),
        )
        .expect("brick");

    let agent = backend();
    agent
        .dispatch("open_project", json!({"path": dir.to_string_lossy()}))
        .expect("agent open");
    assert!(state_text(&agent).contains("SyncProbeBrick"));

    editor
        .dispatch(
            "add_actor",
            json!({"shape": "Rect", "name": "SyncProbeAlfa"}),
        )
        .expect("alfa");
    assert!(state_text(&editor).contains("SyncProbeAlfa"));

    // The agent's next command reloads the editor's save first.
    let _ = agent.dispatch("get_state", json!({})).expect("poll");
    assert!(
        state_text(&agent).contains("SyncProbeAlfa"),
        "the idle copy follows the folder"
    );
    assert_eq!(sync(&agent)["stale"], false);
}

#[test]
fn lock_released_on_close() {
    let (_lock, _data, projects) = isolated("release");
    let dir = create_project(&backend(), projects.path(), "SyncGame");

    let first = backend();
    first
        .dispatch("open_project", json!({"path": dir.to_string_lossy()}))
        .expect("open");
    assert!(dir.join(".blockloom").join("lock.json").is_file());
    first.dispatch("close_project", json!({})).expect("close");
    assert!(
        !dir.join(".blockloom").join("lock.json").exists(),
        "closing drops our owner lock"
    );

    // Nobody owns it now, so the next opener owns outright.
    let second = backend();
    let report = second
        .dispatch("open_project", json!({"path": dir.to_string_lossy()}))
        .expect("reopen");
    assert_eq!(report["attached"], false);
    assert_eq!(sync(&second)["owns_lock"], true);
}

#[test]
fn take_over_lock_command_changes_hands() {
    let (_lock, _data, projects) = isolated("handover");
    let dir = create_project(&backend(), projects.path(), "SyncGame");

    let first = backend();
    first
        .dispatch("open_project", json!({"path": dir.to_string_lossy()}))
        .expect("first open");
    let second = backend();
    second
        .dispatch("open_project", json!({"path": dir.to_string_lossy()}))
        .expect("second open");
    assert_eq!(sync(&second)["attached"], true);

    let taken = second
        .dispatch("take_over_lock", json!({}))
        .expect("take over");
    assert_eq!(taken["session"], sync(&second)["session"]);
    let status = sync(&second);
    assert_eq!(status["owns_lock"], true);
    assert_eq!(status["attached"], false);
}

#[test]
#[cfg(unix)]
fn dirty_copy_refuses_reload_until_it_chooses() {
    use std::os::unix::fs::PermissionsExt;
    let (_lock, _data, projects) = isolated("dirty");
    let dir = create_project(&backend(), projects.path(), "SyncGame");

    let agent = backend();
    agent
        .dispatch("open_project", json!({"path": dir.to_string_lossy()}))
        .expect("open");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    agent
        .dispatch("add_actor", json!({"shape": "Circle", "name": "LostWork"}))
        .expect("in-memory edit lands even when the save fails");
    if sync(&agent)["dirty"] != true {
        // Running as root: permissions don't stop the save, so no conflict
        // to stage. Restore and bow out.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let refused = agent.dispatch("reload_project", json!({})).unwrap_err();
    assert!(refused.contains("take_theirs"), "conflict names the choice");
    let taken = agent
        .dispatch("reload_project", json!({"take_theirs": true}))
        .expect("take theirs");
    assert_eq!(taken["reloaded"], true);
    assert!(!state_text(&agent).contains("LostWork"));
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
#[cfg(unix)]
fn attach_mode_drives_the_editors_backend() {
    let (_lock, _data, projects) = isolated("socket");
    let dir = create_project(&backend(), projects.path(), "SyncGame");

    let editor = backend();
    editor
        .dispatch("open_project", json!({"path": dir.to_string_lossy()}))
        .expect("editor open");
    blockloom_app::attach::serve_attach(editor.clone()).expect("serve attach");

    let mut client = blockloom_app::attach::AttachClient::connect().expect("connect");
    let state = client
        .roundtrip("get_state", json!({}))
        .expect("get-state over the socket");
    assert_eq!(state["ok"], true);
    let added = client
        .roundtrip(
            "add_actor",
            json!({"shape": "Circle", "name": "SocketPuppet"}),
        )
        .expect("add-actor over the socket");
    assert_eq!(added["ok"], true);
    let id = added["result"].as_str().unwrap().to_string();
    assert!(
        state_text(&editor).contains(&id),
        "the editor holds the only copy"
    );
}

#[test]
fn cloud_settings_normalize_undo_and_reload() {
    let (_lock, _data, projects) = isolated("clouds");
    let backend = backend();
    let dir = create_project(&backend, projects.path(), "Clouds");
    backend
        .dispatch("open_project", json!({"path": dir}))
        .unwrap();
    let clouds = |backend: &Backend| {
        let state = backend.dispatch("get_state", json!({})).unwrap();
        state["project"]["world"]["clouds"].clone()
    };
    let before = clouds(&backend);
    backend
        .dispatch(
            "set_clouds",
            json!({"clouds": {
                "enabled": true, "coverage": 2.0, "bottom": 4000.0, "top": 1000.0,
                "quality": "Ultra", "moon_shadows": false
            }}),
        )
        .unwrap();
    let edited = clouds(&backend);
    assert_eq!(edited["coverage"], 1.0);
    assert_eq!(edited["top"], 4010.0);
    backend.dispatch("undo", json!({})).unwrap();
    assert_eq!(clouds(&backend), before);
    backend.dispatch("redo", json!({})).unwrap();
    assert_eq!(clouds(&backend), edited);
    backend.dispatch("close_project", json!({})).unwrap();
    backend
        .dispatch("open_project", json!({"path": dir}))
        .unwrap();
    assert_eq!(clouds(&backend), edited);
    backend.dispatch("close_project", json!({})).unwrap();
}

#[test]
fn cloud_noise_bakes_to_volume_assets() {
    let (_lock, _data, projects) = isolated("cloud-noise");
    let backend = backend();
    let dir = create_project(&backend, projects.path(), "Noise");
    backend
        .dispatch("open_project", json!({"path": dir}))
        .unwrap();
    let written = backend.dispatch("bake_cloud_noise", json!({})).unwrap();
    assert_eq!(
        written,
        json!(["assets/clouds/shape.png", "assets/clouds/detail.png"])
    );
    let state = backend.dispatch("get_state", json!({})).unwrap();
    let clouds = &state["project"]["world"]["clouds"];
    assert_eq!(clouds["shape_volume"], "assets/clouds/shape.png");
    assert_eq!(clouds["detail_volume"], "assets/clouds/detail.png");
    let shape = blockloom_core::pipeline::load_volume(&dir, "assets/clouds/shape.png").unwrap();
    assert_eq!(shape.info.size, [128, 128, 128]);
    let detail = blockloom_core::pipeline::load_volume(&dir, "assets/clouds/detail.png").unwrap();
    assert_eq!(detail.info.size, [32, 32, 32]);
    backend.dispatch("undo", json!({})).unwrap();
    let state = backend.dispatch("get_state", json!({})).unwrap();
    assert_eq!(state["project"]["world"]["clouds"]["shape_volume"], "");
    backend.dispatch("close_project", json!({})).unwrap();
}

#[test]
fn cloud_layers_normalize_paint_and_undo() {
    let (_lock, _data, projects) = isolated("cloud-layers");
    let backend = backend();
    let dir = create_project(&backend, projects.path(), "Layers");
    backend
        .dispatch("open_project", json!({"path": dir}))
        .unwrap();
    let layers = json!([{}, {}, {}, {}, {}, {"coverage": 7.0}]);
    backend
        .dispatch("set_cloud_layers", json!({"layers": layers}))
        .unwrap();
    let state = backend.dispatch("get_state", json!({})).unwrap();
    let saved = &state["project"]["world"]["cloud_layers"];
    assert_eq!(saved.as_array().unwrap().len(), 4);
    assert_eq!(saved[0]["coverage_texture"], "");
    let painted = backend
        .dispatch(
            "paint_cloud_layer",
            json!({"layer": 1, "brush": {"tool": "Cloud", "radius": 0.1, "strength": 1.0},
                   "points": [[0.2, 0.2], [0.8, 0.2]]}),
        )
        .unwrap();
    assert_eq!(painted, json!("assets/clouds/layer-2.png"));
    let state = backend.dispatch("get_state", json!({})).unwrap();
    let layer = &state["project"]["world"]["cloud_layers"][1];
    assert_eq!(layer["coverage_texture"], "assets/clouds/layer-2.png");
    assert_eq!(layer["revision"], 1);
    let file = blockloom_core::cloud_layers::CloudLayer {
        coverage_texture: "assets/clouds/layer-2.png".into(),
        ..Default::default()
    };
    let coverage = blockloom_core::cloud_layers::load_coverage(Some(&dir), &file).unwrap();
    assert_eq!(coverage.len(), 512 * 512);
    assert_eq!(coverage[102 * 512 + 256], 255);
    assert!(
        backend
            .dispatch(
                "paint_cloud_layer",
                json!({"layer": 9, "brush": {}, "points": [[0.5, 0.5]]})
            )
            .is_err()
    );
    // A second stroke erases, and undo brings the first one's pixels back.
    backend
        .dispatch(
            "paint_cloud_layer",
            json!({"layer": 1, "brush": {"tool": "Eraser", "radius": 0.2, "strength": 1.0},
                   "points": [[0.5, 0.2]]}),
        )
        .unwrap();
    let pixel =
        || blockloom_core::cloud_layers::load_coverage(Some(&dir), &file).unwrap()[102 * 512 + 256];
    assert_eq!(pixel(), 0);
    backend.dispatch("undo", json!({})).unwrap();
    assert_eq!(pixel(), 255);
    backend.dispatch("redo", json!({})).unwrap();
    assert_eq!(pixel(), 0);
    backend.dispatch("undo", json!({})).unwrap();
    backend.dispatch("undo", json!({})).unwrap();
    let state = backend.dispatch("get_state", json!({})).unwrap();
    assert_eq!(
        state["project"]["world"]["cloud_layers"][1]["coverage_texture"],
        ""
    );
    backend.dispatch("close_project", json!({})).unwrap();
}

#[test]
fn interface_preview_never_saves_a_draft_and_cancel_uses_the_same_runtime() {
    use blockloom_app::EmbeddedRuntime;
    use blockloom_core::scene::Mode;
    use blockloom_protocol::{EditorMessage, RuntimeMessage};
    use std::sync::{
        Arc, Mutex,
        mpsc::{Receiver, Sender},
    };

    #[derive(Default)]
    struct PreviewHost(Mutex<Option<Receiver<EditorMessage>>>);
    impl EmbeddedRuntime for PreviewHost {
        fn start(
            &self,
            _: Mode,
            incoming: Receiver<EditorMessage>,
            outgoing: Sender<RuntimeMessage>,
        ) -> Result<Box<dyn Send>, String> {
            *self.0.lock().unwrap() = Some(incoming);
            Ok(Box::new(outgoing))
        }
    }

    let (_guard, _data, projects) = isolated("interface-preview");
    let host = Arc::new(PreviewHost::default());
    let backend = Backend::start_embedded(AppHandle::new(|_| {}), host.clone());
    let dir = create_project(&backend, projects.path(), "Design");
    backend
        .dispatch("open_project", json!({"path": dir}))
        .unwrap();
    let saved = std::fs::read(dir.join("project.blockloom")).unwrap();
    let revision = blockloom_core::sync::read_revision(&dir);
    let project = backend.dispatch("get_state", json!({})).unwrap()["project"].clone();
    let document: Value = serde_json::from_str(include_str!(
        "../../blockloom-runtime/tests/fixtures/interface/design-spike.json"
    ))
    .unwrap();
    let design = json!({"revision": 2, "generation": 1, "document": document});
    backend
        .dispatch("preview_interface", json!({"design": design}))
        .unwrap();
    assert!(
        backend
            .dispatch("preview_interface", json!({"design": design}))
            .is_err()
    );
    assert_eq!(
        backend.dispatch("interface_layout", json!({})).unwrap(),
        Value::Null
    );
    assert_eq!(
        backend.dispatch("get_state", json!({})).unwrap()["project"],
        project
    );
    assert_eq!(std::fs::read(dir.join("project.blockloom")).unwrap(), saved);
    assert_eq!(blockloom_core::sync::read_revision(&dir), revision);
    backend.dispatch("preview_interface", json!({})).unwrap();
    let receiver = host.0.lock().unwrap().take().unwrap();
    let messages: Vec<_> = receiver.try_iter().collect();
    assert_eq!(
        messages
            .iter()
            .filter(|m| matches!(m, EditorMessage::Load { .. }))
            .count(),
        1
    );
    assert_eq!(
        messages
            .iter()
            .filter(|m| matches!(m, EditorMessage::InterfaceDesign { .. }))
            .count(),
        2
    );
    assert!(messages.iter().all(|m| !matches!(m, EditorMessage::Start)));
    assert!(matches!(
        messages.last(),
        Some(EditorMessage::InterfaceDesign { design: None })
    ));
    backend.dispatch("close_project", json!({})).unwrap();
}
