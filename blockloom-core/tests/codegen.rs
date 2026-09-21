//! The rule the compiler has to keep: a compiled program asks the world for
//! exactly what the VM asks for, in the same order, with the same values, and
//! with the same complaints about the same bad slots.
//!
//! Each case is one project run twice - stepped by the VM, and compiled by
//! `codegen` into Rust that a `rustc` run turns into a program which prints
//! what it would have done. Both are driven tick by tick off the same clock,
//! and every line says which tick it landed on, so a loop that forgot to hand
//! the frame back fails here rather than merely running fast. The two
//! transcripts have to match line for line.
//!
//! Anything the two could disagree about that isn't the blocks' fault stays
//! out of these projects: `random` and `current time` read a world the two
//! halves don't share, and `broadcast` starts scripts the VM has a queue for
//! and this harness doesn't.
//!
//! No toolchain means no compiled half to compare against, so the whole file
//! skips rather than fails - the same bargain `blockloom-runtime`'s script
//! tests make.

use blockloom_core::blocks::{Instruction, InstructionKind as K, Strand, VariableDef};
use blockloom_core::project::{Actor, Project};
use blockloom_core::scene::{Axis, Mode, Visual};
use blockloom_core::sense::{ActorSense, Sensors};
use blockloom_core::value::{Evaluated, Op, Value};
use blockloom_core::vm::{Effect, Event, Vm};
use std::process::Command;

// ─── The world both halves see ──────────────────────────────────────────────
// Small and fixed. The VM reads it from a published snapshot; the compiled
// program is handed the same numbers by the harness below.

const ACTOR: &str = "a1";
const TIMER: f64 = 2.5;
const MY_POSITION: [f32; 3] = [3.0, 7.0, 0.0];
const OTHER_POSITION: [f32; 3] = [10.0, -2.0, 0.0];

fn publish_world() {
    let mut sensors = Sensors {
        time: TIMER,
        ..Default::default()
    };
    sensors.keys.insert("space".to_string());
    sensors.actors.insert(
        ACTOR.to_string(),
        ActorSense {
            name: "Player".to_string(),
            position: MY_POSITION,
            ..Default::default()
        },
    );
    sensors.actors.insert(
        "a2".to_string(),
        ActorSense {
            name: "Friend".to_string(),
            position: OTHER_POSITION,
            ..Default::default()
        },
    );
    blockloom_core::sense::publish(sensors);
}

/// The same answers, written as Rust for the compiled half to be handed.
const HARNESS: &str = r#"
use std::collections::HashMap;

struct Recorder {
    vars: HashMap<String, Val>,
    /// Which tick is being run, so every line says when it happened and not
    /// just what order things came in.
    tick: usize,
    out: Vec<String>,
}

impl Host for Recorder {
    fn act(&mut self, actor: &str, act: Act) {
        let line = line_of(&act);
        self.out.push(format!("{} {actor}|{line}", self.tick));
    }

    fn sense(&mut self, _actor: &str, kind: &str, args: &[Val]) -> R {
        match kind {
            "KeyDown" => Ok(Val::Bool(args[0].as_text().to_lowercase() == "space")),
            "Timer" => Ok(Val::Num(2.5)),
            "MyPosition" => Ok(Val::Num(axis_of(&args[0], [3.0, 7.0, 0.0]))),
            "ActorPosition" => {
                let name = args[0].as_text();
                if name != "Friend" {
                    return Err(format!("there's no actor named \"{name}\""));
                }
                Ok(Val::Num(axis_of(&args[1], [10.0, -2.0, 0.0])))
            }
            other => Err(format!("unknown operator '{other}'")),
        }
    }

    fn variable(&mut self, _actor: &str, name: &str) -> Val {
        self.vars.get(name).cloned().unwrap_or(Val::Num(0.0))
    }

    fn set_variable(&mut self, _actor: &str, name: &str, value: Val) {
        self.vars.insert(name.to_string(), value);
    }

    fn error(&mut self, actor: &str, message: &str) {
        self.out.push(format!("{} {actor}|Error {message}", self.tick));
    }
}

fn axis_of(value: &Val, position: [f64; 3]) -> f64 {
    match value.as_text().as_str() {
        "Y" | "y" => position[1],
        "Z" | "z" => position[2],
        _ => position[0],
    }
}

/// One line per thing done, in the same words the test builds from an effect.
fn line_of(act: &Act) -> String {
    match act {
        Act::Move { steps } => format!("Move {steps:?}"),
        Act::GoTo { position } => format!("GoTo {position:?}"),
        Act::ChangePosition { axis, by } => format!("ChangePosition {axis} {by:?}"),
        Act::Glide { seconds, target } => format!("Glide {seconds:?} {target:?}"),
        Act::Turn { axis, degrees } => format!("Turn {axis} {degrees:?}"),
        Act::SetScale { factor } => format!("SetScale {factor:?}"),
        Act::Say { text } => format!("Say {text}"),
        Act::SetColor { color } => format!("SetColor {color}"),
        Act::SetVisible { visible } => format!("SetVisible {visible}"),
        Act::SetComponentField { component, field, value } => {
            format!("SetComponentField {component} {field} {}", shown(value))
        }
        Act::AttachComponent { component } => format!("AttachComponent {component}"),
        Act::SetBody { body } => format!("SetBody {body}"),
        other => format!("{other:?}"),
    }
}

fn shown(value: &Val) -> String {
    match value {
        Val::Num(n) => format!("n{n:?}"),
        Val::Text(s) => format!("t{s}"),
        Val::Bool(b) => format!("b{b}"),
    }
}

/// The same schedule the VM runs on: every live strand gets one slice per
/// fixed tick, in the order they started, and the run ends when they are all
/// done or one of them says `stop all`.
fn main() {
    let mut recorder = Recorder { vars: HashMap::new(), tick: 0, out: Vec::new() };
    for (name, value) in seeded() {
        recorder.vars.insert(name.to_string(), value);
    }

    let mut running: Vec<(&Entry, State)> = ENTRIES
        .iter()
        .filter(|entry| entry.trigger == "Started")
        .map(|entry| (entry, entry.begin()))
        .collect();

    for tick in 0..TICKS {
        let now = tick as f64 * DT;
        recorder.tick = tick;
        let mut stopped = false;
        for (entry, state) in running.iter_mut() {
            state.now = now;
            (entry.run)(&mut recorder, state);
            if state.stopping {
                stopped = true;
                break;
            }
        }
        if stopped {
            // Everything after this one in the tick is gone too, as the VM
            // has it: `stop all` empties the list where it stands.
            recorder.out.push(format!("{tick} |Stopped"));
            break;
        }
        running.retain(|(_, state)| !state.done());
        if running.is_empty() {
            break;
        }
    }

    for line in &recorder.out {
        println!("{line}");
    }
}
"#;

/// The VM's side of the same transcript.
fn line_of(effect: &Effect) -> Option<String> {
    let line = match effect {
        Effect::Move { actor, steps } => format!("{actor}|Move {steps:?}"),
        Effect::GoTo { actor, position } => format!("{actor}|GoTo {position:?}"),
        Effect::ChangePosition { actor, axis, by } => {
            format!("{actor}|ChangePosition {} {by:?}", axis.index())
        }
        Effect::Glide {
            actor,
            seconds,
            target,
        } => format!("{actor}|Glide {seconds:?} {target:?}"),
        // Nobody's effect in particular: the run itself ending.
        Effect::Stopped => "|Stopped".to_string(),
        Effect::Turn {
            actor,
            axis,
            degrees,
        } => format!("{actor}|Turn {} {degrees:?}", axis.index()),
        Effect::SetScale { actor, factor } => format!("{actor}|SetScale {factor:?}"),
        Effect::Say { actor, text } => format!("{actor}|Say {text}"),
        Effect::SetColor { actor, color } => format!("{actor}|SetColor {color}"),
        Effect::SetVisible { actor, visible } => format!("{actor}|SetVisible {visible}"),
        Effect::SetComponentField {
            actor,
            component,
            field,
            value,
        } => format!(
            "{actor}|SetComponentField {component} {field} {}",
            shown(value)
        ),
        Effect::AttachComponent { actor, component } => {
            format!("{actor}|AttachComponent {component}")
        }
        Effect::SetBody { actor, body } => format!("{actor}|SetBody {body:?}"),
        Effect::Error { actor, message } => format!("{actor}|Error {message}"),
        // The world's own doing rather than the program's, and nothing the
        // compiled half is asked to produce.
        Effect::SetGravity { .. } => return None,
        other => panic!("this test has no line for {other:?}"),
    };
    Some(line)
}

fn shown(value: &Evaluated) -> String {
    match value {
        Evaluated::Number(n) => format!("n{n:?}"),
        Evaluated::Text(s) => format!("t{s}"),
        Evaluated::Bool(b) => format!("b{b}"),
    }
}

// ─── Building the two halves ────────────────────────────────────────────────

/// The clock both halves step on, and how long a case is given to finish.
/// Neither number matters in itself - what matters is that the two sides
/// compute the very same `now` from the very same tick.
const DT: f64 = 1.0 / 60.0;
const TICKS: usize = 200;

fn project(body: Vec<K>, globals: &[(&str, Evaluated)]) -> Project {
    project_with_strands(vec![body], globals)
}

/// One actor, one strand per body, each headed by a green flag. Several
/// strands on one actor start in the order they're written on both sides,
/// which is what makes them worth comparing; several actors wouldn't, since
/// the VM finds them through a hash map.
fn project_with_strands(bodies: Vec<Vec<K>>, globals: &[(&str, Evaluated)]) -> Project {
    let mut actor = Actor::new(
        "Player",
        Visual::Rect {
            color: "#FFFFFF".to_string(),
            size: [10.0, 10.0],
        },
    );
    actor.id = ACTOR.to_string();
    actor.graph.strands = bodies
        .into_iter()
        .enumerate()
        .map(|(index, body)| {
            let mut instructions = vec![Instruction::new(K::WhenStarted)];
            instructions.extend(body.into_iter().map(Instruction::new));
            Strand::with_instructions(0, index as i32 * 400, instructions)
        })
        .collect();

    Project {
        id: "p".to_string(),
        name: "differential".to_string(),
        world: blockloom_core::scene::World {
            mode: Mode::TwoD,
            ..Default::default()
        },
        actors: vec![actor],
        globals: globals
            .iter()
            .map(|(name, value)| VariableDef {
                name: name.to_string(),
                value: value.clone(),
            })
            .collect(),
    }
}

/// What the VM does with it, as transcript lines.
fn by_vm(project: &Project) -> Vec<String> {
    blockloom_core::init();
    publish_world();
    let mut vm = Vm::new();
    vm.load(project);
    vm.fire(Event::Started);
    let mut lines = Vec::new();
    for tick in 0..TICKS {
        let mut effects = Vec::new();
        vm.tick(tick as f64 * DT, &mut effects);
        // The tick each line landed on, not just the order they came in: a
        // loop that forgot to yield gets everything right but the when.
        lines.extend(
            effects
                .iter()
                .filter_map(line_of)
                .map(|line| format!("{tick} {line}")),
        );
        if !vm.is_running() {
            break;
        }
    }
    lines
}

/// What the compiled program does with it, as the same lines.
fn by_compiler(project: &Project, globals: &[(&str, Evaluated)], case: &str) -> Vec<String> {
    let source = blockloom_core::codegen::compile(project).expect("this project compiles");
    let seed: Vec<String> = globals
        .iter()
        .map(|(name, value)| format!("({name:?}, {})", as_val(value)))
        .collect();
    // The clock, and what the project's variables start at - which is what
    // the VM loads out of `globals`. The harness's `main` reads both.
    let source = format!(
        "{source}\n{HARNESS}\nfn seeded() -> Vec<(&'static str, Val)> {{ vec![{}] }}\n\
         const DT: f64 = {DT:?};\nconst TICKS: usize = {TICKS};\n",
        seed.join(", ")
    );

    let dir = std::env::temp_dir().join(format!("blockloom-codegen-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a build folder");
    let path = dir.join("program.rs");
    std::fs::write(&path, &source).expect("the generated source");

    let binary = dir.join(if cfg!(windows) {
        "program.exe"
    } else {
        "program"
    });
    let built = Command::new("rustc")
        .arg("--edition")
        .arg("2024")
        .arg("-C")
        .arg("opt-level=0")
        .arg("-o")
        .arg(&binary)
        .arg(&path)
        .output()
        .expect("rustc runs");
    assert!(
        built.status.success(),
        "the generated program didn't compile:\n{}\n\n{source}",
        String::from_utf8_lossy(&built.stderr)
    );

    // Through files rather than pipes, so waiting on it can't deadlock on a
    // full pipe buffer while the deadline below is what we want to hit.
    let printed = dir.join("out.txt");
    let complained = dir.join("err.txt");
    let mut child = Command::new(&binary)
        .stdout(std::fs::File::create(&printed).expect("somewhere to print"))
        .stderr(std::fs::File::create(&complained).expect("somewhere to complain"))
        .spawn()
        .expect("the program runs");

    // A strand that never hands the frame back would spin here instead of
    // failing, and a hung test says far less than a failed one. Every case is
    // a few hundred blocks across `TICKS` slices, so this is wildly generous.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let status = loop {
        match child.try_wait().expect("the program can be waited on") {
            Some(status) => break status,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("{case}: the generated program never finished - a loop that doesn't yield?");
            }
            None => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    };
    assert!(
        status.success(),
        "{case}: the generated program failed:\n{}",
        std::fs::read_to_string(&complained).unwrap_or_default()
    );

    let lines = std::fs::read_to_string(&printed)
        .expect("the transcript")
        .lines()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    lines
}

fn as_val(value: &Evaluated) -> String {
    match value {
        Evaluated::Number(n) => format!("Val::Num({n:?}f64)"),
        Evaluated::Text(s) => format!("Val::Text({s:?}.to_string())"),
        Evaluated::Bool(b) => format!("Val::Bool({b})"),
    }
}

fn toolchain() -> bool {
    blockloom_core::script::toolchain_version().is_ok()
}

fn assert_same(case: &str, body: Vec<K>, globals: &[(&str, Evaluated)]) {
    assert_same_strands(case, vec![body], globals);
}

fn assert_same_strands(case: &str, bodies: Vec<Vec<K>>, globals: &[(&str, Evaluated)]) {
    if !toolchain() {
        return;
    }
    let project = project_with_strands(bodies, globals);
    let interpreted = by_vm(&project);
    let compiled = by_compiler(&project, globals, case);
    assert_eq!(
        interpreted, compiled,
        "{case}: the VM and the compiled program disagreed"
    );
    assert!(!interpreted.is_empty(), "{case}: nothing was compared");
}

// ─── The cases ──────────────────────────────────────────────────────────────

fn number(n: f64) -> Value {
    Value::number(n)
}

fn op(name: &str, args: Vec<Value>) -> Value {
    Value::op(Op::from_name(name), args)
}

#[test]
fn arithmetic_lands_on_the_same_numbers() {
    assert_same(
        "arithmetic",
        vec![
            K::Move {
                steps: op(
                    "Add",
                    vec![number(2.0), op("Mul", vec![number(3.0), number(4.0)])],
                ),
            },
            K::ChangePosition {
                axis: Axis::Y,
                by: op("Div", vec![number(7.0), number(2.0)]),
            },
            K::Turn {
                axis: Axis::Z,
                degrees: op("Mod", vec![number(-90.0), number(360.0)]),
            },
            K::SetScale {
                factor: op("Round", vec![number(1.5)]),
            },
            K::Move {
                steps: op("Math", vec![Value::text("Sqrt"), number(2.0)]),
            },
        ],
        &[],
    );
}

#[test]
fn text_and_comparison_land_on_the_same_answers() {
    assert_same(
        "text",
        vec![
            K::Say {
                text: op("Join", vec![Value::text("a"), Value::text("b")]),
            },
            K::Say {
                text: op("Case", vec![Value::text("Mixed"), Value::text("Upper")]),
            },
            K::Say {
                text: op("LetterOf", vec![number(2.0), Value::text("abc")]),
            },
            K::Say {
                text: op("Length", vec![Value::text("hello")]),
            },
            K::Say {
                text: op("Eq", vec![Value::text("5"), number(5.0)]),
            },
            K::Say {
                text: op("Gt", vec![Value::text("10"), number(9.0)]),
            },
            K::Say {
                text: op("IndexOf", vec![Value::text("l"), Value::text("hello")]),
            },
        ],
        &[],
    );
}

#[test]
fn a_bad_slot_is_reported_once_and_stands_in_as_zero() {
    assert_same(
        "errors",
        vec![
            // Reported, and the move is zero.
            K::Move {
                steps: op("Div", vec![number(1.0), number(0.0)]),
            },
            // Evaluated fine, but text that isn't a number: zero, and
            // nothing said about it.
            K::Move {
                steps: Value::text("not a number"),
            },
            // One error for the whole tree, not one per bad branch.
            K::ChangePosition {
                axis: Axis::X,
                by: op(
                    "Add",
                    vec![
                        op("Div", vec![number(1.0), number(0.0)]),
                        op("Div", vec![number(2.0), number(0.0)]),
                    ],
                ),
            },
            K::Say {
                text: op("LetterOf", vec![number(9.0), Value::text("abc")]),
            },
        ],
        &[],
    );
}

#[test]
fn a_short_circuit_keeps_the_bad_slot_on_the_right_quiet() {
    assert_same(
        "shortcircuit",
        vec![
            K::Say {
                text: op(
                    "And",
                    vec![
                        op("False", vec![]),
                        op("Div", vec![number(1.0), number(0.0)]),
                    ],
                ),
            },
            K::Say {
                text: op(
                    "Or",
                    vec![
                        op("True", vec![]),
                        op("Div", vec![number(1.0), number(0.0)]),
                    ],
                ),
            },
        ],
        &[],
    );
}

#[test]
fn sensing_reads_the_same_world() {
    assert_same(
        "sensing",
        vec![
            K::Say {
                text: op("KeyDown", vec![Value::text("Space")]),
            },
            K::Move {
                steps: op("Timer", vec![]),
            },
            K::ChangePosition {
                axis: Axis::X,
                by: op("MyPosition", vec![Value::text("Y")]),
            },
            K::Say {
                text: op(
                    "ActorPosition",
                    vec![Value::text("Friend"), Value::text("X")],
                ),
            },
            // The same complaint, in the same words, when it isn't there.
            K::Say {
                text: op(
                    "ActorPosition",
                    vec![Value::text("Nobody"), Value::text("X")],
                ),
            },
        ],
        &[],
    );
}

#[test]
fn variables_read_and_write_the_same_way() {
    assert_same(
        "variables",
        vec![
            K::Say {
                text: Value::Var {
                    name: "score".to_string(),
                },
            },
            K::SetVariable {
                name: "score".to_string(),
                value: number(10.0),
            },
            K::ChangeVariable {
                name: "score".to_string(),
                value: number(2.5),
            },
            K::Say {
                text: Value::Var {
                    name: "score".to_string(),
                },
            },
            // A variable nothing ever set reads as zero on both sides.
            K::Say {
                text: Value::Var {
                    name: "never".to_string(),
                },
            },
        ],
        &[("score", Evaluated::Number(4.0))],
    );
}

#[test]
fn the_rest_of_the_leaf_blocks_land_the_same() {
    assert_same(
        "leaves",
        vec![
            K::GoTo {
                x: number(1.0),
                y: op("Add", vec![number(2.0), number(3.0)]),
                z: number(0.0),
            },
            K::SetVisible { visible: false },
            K::SetColor {
                color: Value::text("#ff0000"),
            },
            K::SetComponentField {
                component: "Stats".to_string(),
                field: "hp".to_string(),
                value: op("Sub", vec![number(10.0), number(4.0)]),
            },
            K::AttachComponent {
                component: "Body".to_string(),
            },
        ],
        &[],
    );
}

fn body(kinds: Vec<K>) -> Vec<Instruction> {
    kinds.into_iter().map(Instruction::new).collect()
}

#[test]
fn a_branch_takes_the_same_side() {
    assert_same(
        "branches",
        vec![
            K::If {
                condition: op("Gt", vec![number(2.0), number(1.0)]),
                body: body(vec![K::Say {
                    text: Value::text("taken"),
                }]),
            },
            K::If {
                condition: op("Gt", vec![number(1.0), number(2.0)]),
                body: body(vec![K::Say {
                    text: Value::text("skipped"),
                }]),
            },
            K::IfElse {
                condition: op("KeyDown", vec![Value::text("space")]),
                then_body: body(vec![K::Say {
                    text: Value::text("then"),
                }]),
                else_body: body(vec![K::Say {
                    text: Value::text("else"),
                }]),
            },
            // A condition that won't evaluate reports itself once and reads
            // as false, so the else side runs on both.
            K::IfElse {
                condition: op("Div", vec![number(1.0), number(0.0)]),
                then_body: body(vec![K::Say {
                    text: Value::text("bad then"),
                }]),
                else_body: body(vec![K::Say {
                    text: Value::text("bad else"),
                }]),
            },
        ],
        &[],
    );
}

/// A loop hands the frame back every time round, so the count of iterations
/// and the ticks they land on both have to agree.
#[test]
fn a_repeat_goes_round_the_same_number_of_times() {
    assert_same(
        "repeat",
        vec![
            K::Repeat {
                count: number(3.0),
                body: body(vec![
                    K::ChangeVariable {
                        name: "n".to_string(),
                        value: number(1.0),
                    },
                    K::Say {
                        text: Value::Var {
                            name: "n".to_string(),
                        },
                    },
                ]),
            },
            // Counted once on the way in: changing it mid-loop changes
            // nothing, and a count of zero skips the body outright.
            K::Repeat {
                count: number(0.0),
                body: body(vec![K::Say {
                    text: Value::text("never"),
                }]),
            },
            K::Repeat {
                count: number(-1.0),
                body: body(vec![K::Say {
                    text: Value::text("never either"),
                }]),
            },
            K::Say {
                text: Value::text("after"),
            },
        ],
        &[("n", Evaluated::Number(0.0))],
    );
}

#[test]
fn a_while_rechecks_its_condition_every_time_round() {
    assert_same(
        "while",
        vec![
            K::While {
                condition: op(
                    "Lt",
                    vec![
                        Value::Var {
                            name: "n".to_string(),
                        },
                        number(4.0),
                    ],
                ),
                body: body(vec![
                    K::ChangeVariable {
                        name: "n".to_string(),
                        value: number(1.0),
                    },
                    K::Move {
                        steps: Value::Var {
                            name: "n".to_string(),
                        },
                    },
                ]),
            },
            K::Say {
                text: Value::Var {
                    name: "n".to_string(),
                },
            },
        ],
        &[("n", Evaluated::Number(0.0))],
    );
}

/// `escape` leaves the loop it is in and nothing further out; `continue`
/// lands on the back edge, so it still costs the frame an iteration does.
#[test]
fn escaping_and_continuing_leave_from_the_same_places() {
    assert_same(
        "escape",
        vec![
            K::Repeat {
                count: number(5.0),
                body: body(vec![
                    K::ChangeVariable {
                        name: "n".to_string(),
                        value: number(1.0),
                    },
                    K::If {
                        condition: op(
                            "Eq",
                            vec![
                                Value::Var {
                                    name: "n".to_string(),
                                },
                                number(2.0),
                            ],
                        ),
                        body: body(vec![K::ContinueLoop]),
                    },
                    K::Say {
                        text: Value::Var {
                            name: "n".to_string(),
                        },
                    },
                    K::If {
                        condition: op(
                            "Gte",
                            vec![
                                Value::Var {
                                    name: "n".to_string(),
                                },
                                number(3.0),
                            ],
                        ),
                        body: body(vec![K::EscapeLoop]),
                    },
                ]),
            },
            K::Say {
                text: Value::text("out"),
            },
        ],
        &[("n", Evaluated::Number(0.0))],
    );
}

/// The inner loop's own `escape` must not take the outer one with it, and
/// the two counters must not tread on each other.
#[test]
fn nested_loops_keep_their_own_tallies() {
    assert_same(
        "nested",
        vec![
            K::Repeat {
                count: number(3.0),
                body: body(vec![
                    K::Say {
                        text: Value::text("outer"),
                    },
                    K::Repeat {
                        count: number(2.0),
                        body: body(vec![
                            K::Say {
                                text: Value::text("inner"),
                            },
                            K::EscapeLoop,
                            K::Say {
                                text: Value::text("unreachable"),
                            },
                        ]),
                    },
                ]),
            },
            K::Say {
                text: Value::text("done"),
            },
        ],
        &[],
    );
}

/// The timing rule, not just the ordering one: a `wait` sleeps for as long on
/// both sides, so what happens on which tick has to line up as well.
#[test]
fn waiting_wakes_on_the_same_tick() {
    assert_same(
        "wait",
        vec![
            K::Say {
                text: Value::text("before"),
            },
            K::Wait {
                duration: number(0.25),
            },
            K::Say {
                text: Value::text("after"),
            },
            // A zero-second wait isn't a yield at all.
            K::Wait {
                duration: number(0.0),
            },
            K::Say {
                text: Value::text("straight on"),
            },
            // Nor is a negative one, which clamps to nothing.
            K::Wait {
                duration: number(-3.0),
            },
            K::Say {
                text: Value::text("still going"),
            },
            K::Repeat {
                count: number(2.0),
                body: body(vec![
                    K::Wait {
                        duration: number(0.1),
                    },
                    K::Move { steps: number(1.0) },
                ]),
            },
        ],
        &[],
    );
}

#[test]
fn a_wait_until_holds_the_strand_where_it_is() {
    assert_same(
        "waituntil",
        vec![
            // The world these tests publish never changes, so a condition
            // that holds passes straight through and one that doesn't holds
            // the strand until the run gives up - on both sides alike.
            K::WaitUntil {
                condition: op("KeyDown", vec![Value::text("space")]),
            },
            K::Say {
                text: Value::text("through"),
            },
            K::WaitUntil {
                condition: op("KeyDown", vec![Value::text("escape")]),
            },
            K::Say {
                text: Value::text("never reached"),
            },
        ],
        &[],
    );
}

#[test]
fn a_glide_starts_the_same_slide_and_sleeps_as_long() {
    assert_same(
        "glide",
        vec![
            K::Glide {
                seconds: number(0.2),
                x: number(5.0),
                y: op("Add", vec![number(1.0), number(2.0)]),
                z: number(0.0),
            },
            K::Say {
                text: Value::text("landed"),
            },
            // No time to slide over: the effect still goes out, and nothing
            // is suspended.
            K::Glide {
                seconds: number(0.0),
                x: number(1.0),
                y: number(1.0),
                z: number(1.0),
            },
            K::Say {
                text: Value::text("instant"),
            },
        ],
        &[],
    );
}

/// `stop all` ends the run where it stands, and everything queued behind it
/// in that same tick goes with it.
#[test]
fn stopping_ends_the_run_at_the_same_point() {
    assert_same_strands(
        "stopall",
        vec![
            vec![
                K::Say {
                    text: Value::text("first"),
                },
                K::Wait {
                    duration: number(0.1),
                },
                K::StopAll,
                K::Say {
                    text: Value::text("after the stop"),
                },
            ],
            vec![K::Forever {
                body: body(vec![K::Move { steps: number(1.0) }]),
            }],
        ],
        &[],
    );
}

/// Two strands on one actor share the ticks between them, so which one gets
/// to act first on each is part of what has to match.
#[test]
fn strands_take_their_turns_in_the_same_order() {
    assert_same_strands(
        "turns",
        vec![
            vec![K::Repeat {
                count: number(3.0),
                body: body(vec![K::Say {
                    text: Value::text("a"),
                }]),
            }],
            vec![K::Repeat {
                count: number(3.0),
                body: body(vec![K::Say {
                    text: Value::text("b"),
                }]),
            }],
            vec![
                K::Wait {
                    duration: number(0.05),
                },
                K::Say {
                    text: Value::text("late"),
                },
            ],
        ],
        &[],
    );
}

#[test]
fn a_project_the_compiler_cant_do_yet_is_refused_by_name() {
    let project = project(
        vec![K::CallBlock {
            block_id: "b1".to_string(),
            args: vec![],
        }],
        &[],
    );
    let error =
        blockloom_core::codegen::compile(&project).expect_err("custom blocks aren't compiled yet");
    assert_eq!(error.what, "a custom block");
}
