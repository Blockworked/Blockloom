//! One edit through `Backend::dispatch` on a large project, end to end: undo
//! snapshot, save, runtime sync (no runtime attached) and the state snapshot
//! the window gets. Numbers go in `docs/performance.md`.
//!
//! ```sh
//! cargo test --release -p blockloom-app --test edit_latency -- --ignored --nocapture
//! ```

use blockloom_app::{AppHandle, Backend};
use blockloom_core::project::{self, Project};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn scratch(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "blockloom-latency-{tag}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

/// The kitchen-3d golden fixture grown to `per_scene` actors in each of three
/// scenes, each with the editor-baseline canvas (as `edit_cost` in core).
fn large_project(per_scene: usize) -> Project {
    blockloom_core::init();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../blockloom-core/tests/fixtures/golden/kitchen-3d/input");
    let mut project = project::read_project_dir(&fixture).expect("the kitchen-3d fixture");
    while project.scenes.len() < 3 {
        let n = project.scenes.len() + 1;
        project.add_scene(&format!("Level {n}"), None);
    }
    let templates = project.scenes[0].actors.clone();
    let canvas = templates
        .iter()
        .find(|a| !a.graph.strands.is_empty())
        .map(|a| a.graph.clone())
        .unwrap_or_default();
    for scene in &mut project.scenes {
        scene.actors.clear();
        for i in 0..per_scene {
            let mut actor = templates[i % templates.len()].clone();
            actor.id = uuid::Uuid::new_v4().simple().to_string();
            actor.name = format!("{} {i}", actor.name);
            actor.graph = canvas.clone();
            scene.actors.push(actor);
        }
    }
    project
}

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort();
    times[times.len() / 2]
}

fn ms(d: Duration) -> String {
    format!("{:.2} ms", d.as_secs_f64() * 1000.0)
}

#[test]
#[ignore = "a measurement; run with --release --nocapture"]
fn edit_latency_baseline() {
    let data = scratch("data");
    unsafe {
        std::env::set_var("BLOCKLOOM_DATA_DIR", &data);
    }
    println!("| actors | state snapshot | rename (edit) | undo |");
    println!("| --- | --- | --- | --- |");
    for per_scene in [100, 400, 1600] {
        let dir = scratch("project");
        let source = large_project(per_scene);
        project::save_project(&source, &dir).unwrap();
        let backend = Backend::start(AppHandle::new(|_| {}));
        backend
            .dispatch("open_project", json!({ "path": dir, "force": true }))
            .expect("open the project");
        let state = backend.dispatch("get_state", json!({})).unwrap();
        let actor = state["project"]["actors"][1]["id"].clone();
        let snapshot = serde_json::to_vec(&state).unwrap().len();

        let runs = if per_scene > 1000 { 5 } else { 15 };
        let mut renames = Vec::new();
        for i in 0..runs {
            let start = Instant::now();
            backend
                .dispatch(
                    "rename_actor",
                    json!({ "actorId": actor, "name": format!("Renamed {i}") }),
                )
                .unwrap();
            renames.push(start.elapsed());
        }
        let mut undos = Vec::new();
        for _ in 0..runs {
            let start = Instant::now();
            backend.dispatch("undo", json!({})).unwrap();
            undos.push(start.elapsed());
        }
        println!(
            "| {} | {:.0} KiB | {} | {} |",
            per_scene * source.scenes.len(),
            snapshot as f64 / 1024.0,
            ms(median(renames)),
            ms(median(undos)),
        );
        drop(backend);
        let _ = std::fs::remove_dir_all(&dir);
    }
    let _ = std::fs::remove_dir_all(&data);
}
