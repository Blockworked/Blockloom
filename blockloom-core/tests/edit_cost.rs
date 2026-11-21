//! What one editor edit costs in document copies, on a large project: the
//! undo snapshot (a whole-project clone), the runtime sync payload, the save
//! (every scene file plus the index) and the reload a live-sync does.
//! Numbers go in `docs/performance.md`.
//!
//! ```sh
//! cargo test --release -p blockloom-core --test edit_cost -- --ignored --nocapture
//! ```

use blockloom_core::project::{self, Project};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The kitchen-3d golden fixture, grown to `per_scene` actors in each of
/// three scenes, every actor carrying the editor-baseline canvas.
pub fn large_project(per_scene: usize) -> Project {
    blockloom_core::init();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/golden/kitchen-3d/input");
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

fn scratch() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "blockloom-edit-cost-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

/// The median of `runs` timings of `f`.
fn median(runs: usize, mut f: impl FnMut()) -> Duration {
    let mut times: Vec<Duration> = (0..runs)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .collect();
    times.sort();
    times[runs / 2]
}

fn ms(d: Duration) -> String {
    format!("{:.2} ms", d.as_secs_f64() * 1000.0)
}

fn kib(bytes: usize) -> String {
    format!("{:.0} KiB", bytes as f64 / 1024.0)
}

/// What `save_project` did before atomic writes: plain overwrites.
fn save_plainly(project: &Project, dir: &Path) {
    for scene in &project.scenes {
        let json = serde_json::to_string_pretty(&scene.to_file()).unwrap();
        std::fs::write(dir.join(scene.asset_path()), json).unwrap();
    }
    let json = serde_json::to_string_pretty(&project::project_to_file(project)).unwrap();
    std::fs::write(dir.join("project.blockloom"), json).unwrap();
}

#[test]
#[ignore = "a measurement; run with --release --nocapture"]
fn edit_cost_baseline() {
    println!(
        "| actors | scenes | doc on disk | sync payload | undo clone | sync serialize | save (atomic) | save (plain) | reload |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    for per_scene in [100, 400, 1600] {
        let project = large_project(per_scene);
        let dir = scratch();
        project::save_project(&project, &dir).unwrap();

        let on_disk: usize = project
            .scenes
            .iter()
            .map(|s| std::fs::metadata(dir.join(s.asset_path())).unwrap().len() as usize)
            .sum::<usize>()
            + std::fs::metadata(dir.join("project.blockloom"))
                .unwrap()
                .len() as usize;
        // A child runtime gets the project as one JSON line (`EditorMessage::Load`).
        let payload = serde_json::to_vec(&project).unwrap().len();

        let runs = if per_scene > 1000 { 5 } else { 15 };
        let clone = median(runs, || drop(std::hint::black_box(project.clone())));
        let serialize = median(runs, || {
            std::hint::black_box(serde_json::to_vec(&project).unwrap());
        });
        let save = median(runs, || {
            project::save_project(&project, &dir).unwrap();
        });
        let plain = median(runs, || save_plainly(&project, &dir));
        let reload = median(runs, || {
            std::hint::black_box(project::read_project_dir(&dir).unwrap());
        });
        println!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            per_scene * project.scenes.len(),
            project.scenes.len(),
            kib(on_disk),
            kib(payload),
            ms(clone),
            ms(serialize),
            ms(save),
            ms(plain),
            ms(reload),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
