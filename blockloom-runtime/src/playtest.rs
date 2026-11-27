//! Playmode tests: strands under `when test runs` played headless.
//!
//! Each test gets a fresh world built from the document the way Play builds
//! it, with the green flag pressed and then its own test started. It passes
//! at `pass test`, fails at `fail test`, a false `expect` or a block error,
//! and fails when neither happens within the tick limit.

use crate::engine::{Engine, PendingEffects};
use crate::simulation::SimStep;
use bevy::prelude::*;
use blockloom_core::blocks::InstructionKind;
use blockloom_core::project::Project;
use blockloom_core::vm::{Effect, Event};
use serde::Serialize;

/// Ten seconds at the default rate.
pub const DEFAULT_TICKS: u64 = 600;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TestOutcome {
    pub name: String,
    pub passed: bool,
    /// Why it failed; empty when it passed.
    pub reason: String,
    /// Fixed ticks it ran for.
    pub ticks: u64,
}

/// The first verdict a step reached, if any.
#[derive(Resource, Default)]
struct Verdict(Option<Result<(), String>>);

/// Every test name in the project, in the order the actors list them.
pub fn test_names(project: &Project) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for scene in &project.scenes {
        for actor in &scene.actors {
            for strand in &actor.graph.strands {
                if let Some(InstructionKind::WhenTestRuns { name }) =
                    strand.instructions.first().map(|i| &i.kind)
                {
                    let name = name.trim().to_string();
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
            }
        }
    }
    names
}

/// Reads this step's verdicts before the effects are cleared.
fn judge(effects: Res<PendingEffects>, engine: NonSend<Engine>, mut verdict: ResMut<Verdict>) {
    if verdict.0.is_some() {
        return;
    }
    for effect in &effects.0 {
        let found = match effect {
            Effect::Verb { verb, args, .. } => match verb.as_str() {
                "pass_test" => Some(Ok(())),
                "fail_test" => Some(Err(text(args.first(), "failed"))),
                "expect" if !truthy(args.first()) => {
                    Some(Err(text(args.get(1), "an expectation was false")))
                }
                _ => None,
            },
            Effect::Error { actor, message } => {
                let name = engine
                    .actor(actor)
                    .map_or(actor.as_str(), |a| a.name.as_str());
                Some(Err(format!("{name}: {message}")))
            }
            _ => None,
        };
        if let Some(found) = found {
            verdict.0 = Some(found);
            return;
        }
    }
}

fn text(value: Option<&serde_json::Value>, fallback: &str) -> String {
    let text = match value {
        Some(serde_json::Value::String(s)) => s.trim().to_string(),
        Some(serde_json::Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    };
    if text.is_empty() {
        fallback.to_string()
    } else {
        text
    }
}

fn truthy(value: Option<&serde_json::Value>) -> bool {
    match value {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        Some(serde_json::Value::String(s)) => {
            let s = s.trim();
            !(s.is_empty() || s == "0" || s.eq_ignore_ascii_case("false"))
        }
        _ => false,
    }
}

/// Plays the test called `name` for at most `max_ticks` fixed ticks.
pub fn run_test(project: &Project, name: &str, max_ticks: u64) -> TestOutcome {
    blockloom_core::value::register_blockloom_operators();
    let mode = project.world.mode;
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, mode);
    engine.project = project.clone();
    let loaded = engine.project.clone();
    engine.vm.load(&loaded);
    let mut app = crate::simulation::headless(engine, mode, true);
    app.init_resource::<Verdict>();
    app.add_systems(
        FixedUpdate,
        judge
            .after(SimStep::ComponentEffects)
            .before(SimStep::ClearEffects),
    );
    app.world_mut()
        .non_send_mut::<Engine>()
        .fire(Event::Test(name.trim().to_string()));
    let mut ticks = 0;
    let verdict = loop {
        if ticks >= max_ticks {
            break Err(format!("no pass test within {max_ticks} ticks"));
        }
        app.update();
        ticks += 1;
        if let Some(verdict) = app.world_mut().resource_mut::<Verdict>().0.take() {
            break verdict;
        }
    };
    TestOutcome {
        name: name.trim().to_string(),
        passed: verdict.is_ok(),
        reason: verdict.err().unwrap_or_default(),
        ticks,
    }
}

/// Runs `only` or every test, in order.
pub fn run_tests(project: &Project, only: Option<&str>, max_ticks: u64) -> Vec<TestOutcome> {
    test_names(project)
        .into_iter()
        .filter(|name| only.is_none_or(|only| only.trim() == name))
        .map(|name| run_test(project, &name, max_ticks))
        .collect()
}

/// `blockloom-runtime --test <folder or pack file> [--name N] [--ticks N]`: one
/// JSON outcome per line, then exits 0 when every test passed, 1 when one
/// failed and 2 when the folder can't be read or names no test.
pub fn main(args: &[String]) -> i32 {
    let after = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
    };
    let Some(dir) = after("--test") else {
        eprintln!("--test needs a project folder");
        return 2;
    };
    // A folder is a saved project; a file is a pack of one as it stands.
    let path = std::path::Path::new(dir);
    let read = if path.is_file() {
        std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|text| blockloom_core::pack::GamePack::from_json(&text, dir))
            .map(|pack| pack.project)
    } else {
        blockloom_core::project::read_project_dir(path)
    };
    let project = match read {
        Ok(project) => project,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let ticks = after("--ticks")
        .and_then(|t| t.parse().ok())
        .unwrap_or(DEFAULT_TICKS);
    let outcomes = run_tests(&project, after("--name").map(String::as_str), ticks);
    if outcomes.is_empty() {
        eprintln!("no test called that: add a strand under \"when test runs\"");
        return 2;
    }
    for outcome in &outcomes {
        println!("{}", serde_json::to_string(outcome).unwrap_or_default());
    }
    i32::from(outcomes.iter().any(|o| !o.passed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::blocks::{Instruction, InstructionKind as K, Strand};
    use blockloom_core::project::Actor;
    use blockloom_core::scene::{Mode, Visual};
    use blockloom_core::value::{Op, Value};

    fn verb(name: &str, args: Vec<Value>) -> K {
        K::Verb {
            verb: name.to_string(),
            args,
        }
    }

    fn test_strand(name: &str, body: Vec<K>) -> Strand {
        let mut kinds = vec![K::WhenTestRuns {
            name: name.to_string(),
        }];
        kinds.extend(body);
        Strand::with_instructions(0, 0, kinds.into_iter().map(Instruction::new).collect())
    }

    fn project() -> Project {
        let mut project = Project::starter("T", Mode::TwoD);
        let player = project
            .actors
            .iter_mut()
            .find(|a| a.name == "Player")
            .unwrap();
        // The game counts up; the tests watch it.
        player.graph.strands.push(Strand::with_instructions(
            0,
            0,
            [
                K::WhenStarted,
                K::Forever {
                    body: vec![Instruction::new(K::ChangeVariable {
                        name: "count".into(),
                        value: Value::number(1.0),
                    })],
                },
            ]
            .into_iter()
            .map(Instruction::new)
            .collect(),
        ));
        let count_over = |n: f64| {
            Value::op(
                Op::Gt,
                vec![
                    Value::Var {
                        name: "count".into(),
                    },
                    Value::number(n),
                ],
            )
        };
        player.graph.strands.extend([
            test_strand(
                "counts",
                vec![
                    K::Wait {
                        duration: Value::number(0.5),
                    },
                    verb("expect", vec![count_over(10.0), Value::text("too few")]),
                    verb("pass_test", vec![]),
                ],
            ),
            test_strand(
                "overshoots",
                vec![
                    K::Wait {
                        duration: Value::number(0.5),
                    },
                    verb(
                        "expect",
                        vec![count_over(1000.0), Value::text("not yet 1000")],
                    ),
                    verb("pass_test", vec![]),
                ],
            ),
            test_strand(
                "gives up",
                vec![verb("fail_test", vec![Value::text("on purpose")])],
            ),
            test_strand("hangs", vec![]),
        ]);
        let mut other = Actor::new(
            "Other",
            Visual::Rect {
                color: "#FFFFFF".into(),
                size: [10.0, 10.0],
            },
        );
        // A second actor's test of the same name is the same test.
        other.graph.strands.push(test_strand("counts", vec![]));
        project.actors.push(other);
        project
    }

    #[test]
    fn tests_pass_fail_and_time_out_headless() {
        let project = project();
        assert_eq!(
            test_names(&project),
            ["counts", "overshoots", "gives up", "hangs"]
        );
        let outcomes = run_tests(&project, None, 120);
        let by = |name: &str| outcomes.iter().find(|o| o.name == name).unwrap().clone();
        let counts = by("counts");
        assert!(counts.passed, "{counts:?}");
        assert!((30..=32).contains(&counts.ticks), "{counts:?}");
        let overshoots = by("overshoots");
        assert!(!overshoots.passed);
        assert_eq!(overshoots.reason, "not yet 1000");
        assert_eq!(by("gives up").reason, "on purpose");
        assert!(by("gives up").ticks <= 2);
        let hangs = by("hangs");
        assert!(!hangs.passed && hangs.ticks == 120, "{hangs:?}");
        // Only the named one.
        assert_eq!(run_tests(&project, Some("gives up"), 10).len(), 1);
    }

    #[test]
    fn a_block_error_fails_the_test() {
        let mut project = Project::starter("T", Mode::TwoD);
        project.actors[0].graph.strands.push(test_strand(
            "deletes nobody",
            vec![
                K::DeleteActor {
                    target: Value::text("Nobody"),
                },
                K::Wait {
                    duration: Value::number(1.0),
                },
                verb("pass_test", vec![]),
            ],
        ));
        let outcome = run_test(&project, "deletes nobody", 120);
        assert!(!outcome.passed);
        assert!(outcome.reason.contains("Nobody"), "{outcome:?}");
    }
}
