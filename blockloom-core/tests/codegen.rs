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

use blockloom_core::blocks::{
    BlockDef, BlockPiece, BlockShape, InputValueType, Instruction, InstructionKind as K, Strand,
    VariableDef,
};
use blockloom_core::project::{Actor, Project};
use blockloom_core::scene::{Axis, Mode, Visual};
use blockloom_core::sense::{ActorSense, Sensors};
use blockloom_core::ui::{UiAnchor, UiProp};
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
const MOUSE_DELTA: [f32; 2] = [24.0, -9.0];

fn publish_world() {
    let mut sensors = Sensors {
        time: TIMER,
        mouse_delta: MOUSE_DELTA,
        mouse_locked: true,
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
        // `delete` is recorded against the actor it takes out of the run
        // rather than the one that asked, because that is who the VM's own
        // effect is about.
        let actor = match &act {
            Act::DeleteActor { target } => target.clone(),
            // Window-global, or screen-space: against nobody in particular,
            // which is how the VM's own effects say it.
            Act::SetMouseLocked { .. }
            | Act::ShowElement { .. }
            | Act::SetUiProp { .. }
            | Act::HideElement { .. }
            | Act::DeleteElement { .. }
            | Act::SetPaused { .. } => String::new(),
            _ => actor.to_string(),
        };
        let line = line_of(&act);
        self.out.push(format!("{} {actor}|{line}", self.tick));
    }

    fn sense(&mut self, _actor: &str, kind: &str, args: &[Val]) -> R {
        match kind {
            "KeyDown" => Ok(Val::Bool(args[0].as_text().to_lowercase() == "space")),
            "Timer" => Ok(Val::Num(2.5)),
            "MouseDeltaX" => Ok(Val::Num(24.0)),
            "MouseDeltaY" => Ok(Val::Num(-9.0)),
            "MouseLocked" => Ok(Val::Bool(true)),
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
        Act::SetMouseLocked { locked } => format!("SetMouseLocked {locked}"),
        Act::SetCameraPitch { degrees } => format!("SetCameraPitch {degrees:?}"),
        Act::SetComponentField { component, field, value } => {
            format!("SetComponentField {component} {field} {}", shown(value))
        }
        Act::AttachComponent { component } => format!("AttachComponent {component}"),
        Act::SetParent { target } => format!("SetParent {target}"),
        Act::CreateClone { of, clone } => format!("CreateClone {clone} {of}"),
        Act::CreateActor { id, name, position } => {
            format!("CreateActor {id} {name} {position:?}")
        }
        Act::DeleteActor { .. } => "DeleteActor".to_string(),
        Act::SetBody { body } => format!("SetBody {body}"),
        Act::ShowElement {
            id,
            kind,
            content,
            anchor,
            offset,
            size,
            parent,
            flag,
            range,
            value,
        } => format!(
            "ShowElement {id} {kind} {content} {anchor} {offset:?} {size:?} \
             {parent} {flag} {range:?} {}",
            shown(value)
        ),
        Act::SetUiProp { id, prop, value } => {
            format!("SetUiProp {id} {prop} {}", shown(value))
        }
        Act::HideElement { id, all } => format!("HideElement {id} {all}"),
        Act::DeleteElement { id } => format!("DeleteElement {id}"),
        Act::SetPaused { paused } => format!("SetPaused {paused}"),
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

    // The program's own scheduler, which is what a built game runs on: one
    // slice per strand per fixed tick, clones started at the top of the tick
    // after they were made, and deleted actors dropped at the end of one.
    let mut runner = Runner::new(NAMES);
    runner.fire(ENTRIES, "Started", "", "", "");
    runner.fire(ENTRIES, "UiClicked", "", "resume", "");

    for tick in 0..TICKS {
        recorder.tick = tick;
        if runner.tick(ENTRIES, &mut recorder, tick as f64 * DT) {
            // Everything after this one in the tick is gone too, as the VM
            // has it: `stop all` empties the list where it stands.
            recorder.out.push(format!("{tick} |Stopped"));
            break;
        }
        if !runner.is_running() {
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
        Effect::SetParent { actor, parent } => format!("{actor}|SetParent {parent}"),
        Effect::CreateClone { actor, clone, of } => format!("{actor}|CreateClone {clone} {of}"),
        Effect::CreateActor {
            actor,
            id,
            name,
            position,
        } => format!("{actor}|CreateActor {id} {name} {position:?}"),
        // Against the actor it takes out of the run, which is the one thing
        // both halves say about it.
        Effect::DeleteActor { actor } => format!("{actor}|DeleteActor"),
        Effect::SetBody { actor, body } => format!("{actor}|SetBody {body:?}"),
        Effect::Error { actor, message } => format!("{actor}|Error {message}"),
        // The world's own doing rather than the program's, and nothing the
        // compiled half is asked to produce.
        Effect::SetGravity { .. } => return None,
        // Window-global, so against nobody: the harness blanks the actor
        // for this act the same way.
        Effect::SetMouseLocked { locked } => format!("|SetMouseLocked {locked}"),
        Effect::SetCameraPitch { actor, degrees } => {
            format!("{actor}|SetCameraPitch {degrees:?}")
        }
        // Screen-space, so against nobody: the harness blanks the actor for
        // these acts the same way.
        Effect::ShowElement { element } => format!(
            "|ShowElement {} {} {} {} {:?} {:?} {} {} {:?} {}",
            element.id,
            element.kind.index(),
            element.content,
            element.anchor.index(),
            element.offset,
            element.size,
            element.parent,
            element.modal,
            element.range,
            shown(&element.value)
        ),
        Effect::SetUiProp { id, prop, value } => {
            format!("|SetUiProp {id} {} {}", prop.name(), shown(value))
        }
        Effect::HideElement { id, all } => format!("|HideElement {id} {all}"),
        Effect::DeleteElement { id } => format!("|DeleteElement {id}"),
        Effect::SetPaused { paused } => format!("|SetPaused {paused}"),
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

/// A custom block: what a call names it, the inputs it declares, and the
/// body those inputs are read in.
struct Block {
    id: &'static str,
    inputs: &'static [&'static str],
    body: Vec<K>,
}

fn block(id: &'static str, inputs: &'static [&'static str], body: Vec<K>) -> Block {
    Block { id, inputs, body }
}

/// A reporter-shaped call, which resolves to whatever the body returns.
fn call(id: &str, args: Vec<Value>) -> Value {
    Value::Call {
        block_id: id.to_string(),
        args,
        // What the editor shows while the call isn't being run; never read.
        saved: Box::new(Value::number(0.0)),
    }
}

/// One actor: a green-flag strand per body, and a custom block per
/// definition. Several strands on one actor start in the order they're
/// written on both sides, which is what makes them worth comparing; several
/// actors wouldn't, since the VM finds those through a hash map.
fn project_with_blocks(
    bodies: Vec<Vec<K>>,
    blocks: Vec<Block>,
    globals: &[(&str, Evaluated)],
) -> Project {
    let headed = bodies
        .into_iter()
        .map(|body| (K::WhenStarted, body))
        .collect();
    project_with_headers(headed, blocks, globals)
}

/// The same, with each strand's own header: `when I start as a clone` is a
/// header like any other, and a clone's strands are the point of these cases.
fn project_with_headers(
    strands: Vec<(K, Vec<K>)>,
    blocks: Vec<Block>,
    globals: &[(&str, Evaluated)],
) -> Project {
    let mut actor = Actor::new(
        "Player",
        Visual::Rect {
            color: "#FFFFFF".to_string(),
            size: [10.0, 10.0],
        },
    );
    actor.id = ACTOR.to_string();
    actor.graph.strands = strands
        .into_iter()
        .enumerate()
        .map(|(index, (header, body))| {
            let mut instructions = vec![Instruction::new(header)];
            instructions.extend(body.into_iter().map(Instruction::new));
            Strand::with_instructions(0, index as i32 * 400, instructions)
        })
        .collect();

    // A custom block is a prototype plus a strand headed by it: that header
    // is how the VM finds the body, and where a call jumps to.
    for (index, block) in blocks.into_iter().enumerate() {
        let mut instructions = vec![Instruction::new(K::BlockHeader {
            block_id: block.id.to_string(),
        })];
        instructions.extend(block.body.into_iter().map(Instruction::new));
        actor.graph.strands.push(Strand::with_instructions(
            600,
            index as i32 * 400,
            instructions,
        ));
        let mut pieces = vec![BlockPiece::Label {
            id: format!("{}-label", block.id),
            text: block.id.to_string(),
        }];
        pieces.extend(block.inputs.iter().map(|name| BlockPiece::Input {
            id: format!("{}-{name}", block.id),
            name: name.to_string(),
            value_type: InputValueType::Any,
        }));
        actor.graph.block_defs.push(BlockDef {
            id: block.id.to_string(),
            pieces,
            shape: BlockShape::Normal,
            color: blockloom_core::blocks::default_block_color(),
        });
    }

    Project {
        id: "p".to_string(),
        name: "differential".to_string(),
        icon: String::new(),
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
    // The interface's own click, beside the green flag: a case with a
    // `when (resume) clicked` strand gets one that keeps running while the
    // world is frozen. No other case has that hat, so nothing else sees it.
    vm.fire(Event::UiClicked {
        id: "resume".to_string(),
    });
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
    assert_same_blocks(case, bodies, Vec::new(), globals);
}

fn assert_same_headed(case: &str, strands: Vec<(K, Vec<K>)>) {
    if !toolchain() {
        return;
    }
    assert_project(case, project_with_headers(strands, Vec::new(), &[]), &[]);
}

fn assert_same_blocks(
    case: &str,
    bodies: Vec<Vec<K>>,
    blocks: Vec<Block>,
    globals: &[(&str, Evaluated)],
) {
    if !toolchain() {
        return;
    }
    assert_project(case, project_with_blocks(bodies, blocks, globals), globals);
}

fn assert_project(case: &str, project: Project, globals: &[(&str, Evaluated)]) {
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
            K::Say {
                text: op("MouseDeltaX", vec![]),
            },
            K::Say {
                text: op("MouseDeltaY", vec![]),
            },
            K::Say {
                text: op("MouseLocked", vec![]),
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
            K::SetMouseLocked { locked: true },
            K::SetCameraPitch {
                degrees: number(12.5),
            },
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
            // The VM resolves the name to an id before the effect leaves it,
            // so the compiled half has to ask the host the same thing.
            K::SetParent {
                parent: Value::text("Friend"),
            },
        ],
        &[],
    );
}

#[test]
fn every_actor_is_in_the_name_table_whether_it_has_blocks_or_not() {
    // `delete` and `create a clone of` name an actor the way a block does, so
    // one with an empty canvas still has to be findable by name.
    let mut project = project_with_blocks(
        vec![vec![K::DeleteActor {
            target: Value::text("Scenery"),
        }]],
        Vec::new(),
        &[],
    );
    let mut quiet = Actor::new(
        "Scenery",
        Visual::Rect {
            color: "#000000".to_string(),
            size: [1.0, 1.0],
        },
    );
    quiet.id = "a2".to_string();
    project.actors.push(quiet);

    let source = blockloom_core::codegen::compile(&project).expect("this project compiles");
    assert!(source.contains("(\"a2\", \"Scenery\")"), "{source}");
    // And nothing was emitted for it: an actor with no steps has no strands.
    assert!(!source.contains("fn actor_1("), "{source}");
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

// ─── Custom blocks ──────────────────────────────────────────────────────────

fn calling(id: &str, args: Vec<Value>) -> K {
    K::CallBlock {
        block_id: id.to_string(),
        args,
    }
}

fn param(name: &str) -> Value {
    Value::Param {
        name: name.to_string(),
    }
}

#[test]
fn a_statement_call_runs_the_body_and_comes_back() {
    assert_same_blocks(
        "statement",
        vec![vec![
            K::Say {
                text: Value::text("before"),
            },
            calling("b1", vec![number(3.0)]),
            K::Say {
                text: Value::text("after"),
            },
            // The same block twice over: the second call has to find its own
            // way home rather than the first one's.
            calling("b1", vec![number(4.0)]),
            K::Say {
                text: Value::text("done"),
            },
        ]],
        vec![block(
            "b1",
            &["distance"],
            vec![
                K::Move {
                    steps: param("distance"),
                },
                // A name the block never declared reads as zero on both sides.
                K::Say {
                    text: param("nobody"),
                },
            ],
        )],
        &[],
    );
}

/// Arguments bind by position, and only as far as the two lists overlap: a
/// surplus one is never worked out at all, so its complaints never happen.
#[test]
fn arguments_bind_as_far_as_the_two_lists_overlap() {
    assert_same_blocks(
        "binding",
        vec![vec![
            calling("b1", vec![number(1.0), number(2.0)]),
            // One short: the second input has nothing to read.
            calling("b1", vec![number(9.0)]),
            // One over, and the surplus is a slot that would have complained.
            calling(
                "b1",
                vec![
                    number(5.0),
                    number(6.0),
                    op("Div", vec![number(1.0), number(0.0)]),
                ],
            ),
        ]],
        vec![block(
            "b1",
            &["a", "b"],
            vec![K::Move { steps: param("a") }, K::Say { text: param("b") }],
        )],
        &[],
    );
}

#[test]
fn a_reporter_runs_in_place_and_hands_its_value_back() {
    assert_same_blocks(
        "reporter",
        vec![vec![
            K::Say {
                text: call("b1", vec![number(4.0)]),
            },
            // In the middle of an expression, and twice over in one.
            K::Move {
                steps: op(
                    "Add",
                    vec![call("b1", vec![number(1.0)]), call("b1", vec![number(2.0)])],
                ),
            },
            // A body that falls off the end rather than returning is zero.
            K::Say {
                text: call("b2", vec![]),
            },
        ]],
        vec![
            block(
                "b1",
                &["n"],
                vec![
                    // A reporter may act on the world on its way to a value.
                    K::Say {
                        text: Value::text("asked"),
                    },
                    K::Return {
                        value: op("Mul", vec![param("n"), number(10.0)]),
                    },
                ],
            ),
            block(
                "b2",
                &[],
                vec![K::Say {
                    text: Value::text("no return"),
                }],
            ),
        ],
        &[],
    );
}

/// The order that makes hoisting necessary. A whole tree is resolved - every
/// variable and every reporter call - before one operator runs, so a reporter
/// on the side `and` never reads runs anyway, and says what it says.
#[test]
fn a_reporter_runs_even_on_the_side_a_short_circuit_never_reads() {
    assert_same_blocks(
        "resolve",
        vec![vec![
            K::Say {
                text: op("And", vec![op("False", vec![]), call("b1", vec![])]),
            },
            K::Say {
                text: op("Or", vec![op("True", vec![]), call("b1", vec![])]),
            },
        ]],
        vec![block(
            "b1",
            &[],
            vec![
                K::Say {
                    text: Value::text("ran anyway"),
                },
                K::Return {
                    value: op("True", vec![]),
                },
            ],
        )],
        &[],
    );
}

/// Nothing in a reporter suspends: it runs to completion where it stands, so
/// a `wait` in one passes straight through and a loop costs no ticks at all.
#[test]
fn nothing_inside_a_reporter_suspends() {
    assert_same_blocks(
        "immediate",
        vec![vec![
            K::Say {
                text: call("b1", vec![]),
            },
            K::Say {
                text: Value::text("same tick"),
            },
        ]],
        vec![block(
            "b1",
            &[],
            vec![
                K::Wait {
                    duration: number(5.0),
                },
                K::Repeat {
                    count: number(3.0),
                    body: vec![Instruction::new(K::Move { steps: number(1.0) })],
                },
                K::WaitUntil {
                    condition: op("KeyDown", vec![Value::text("escape")]),
                },
                K::Glide {
                    seconds: number(2.0),
                    x: number(1.0),
                    y: number(2.0),
                    z: number(3.0),
                },
                K::Return {
                    value: Value::text("straight through"),
                },
            ],
        )],
        &[],
    );
}

/// A statement call is part of the strand, so a loop inside one yields the
/// way any other loop does - the block boundary changes nothing about when.
#[test]
fn a_loop_inside_a_statement_call_still_costs_its_ticks() {
    assert_same_blocks(
        "callloop",
        vec![vec![
            calling("b1", vec![]),
            K::Say {
                text: Value::text("after"),
            },
        ]],
        vec![block(
            "b1",
            &[],
            vec![
                K::Repeat {
                    count: number(3.0),
                    body: vec![Instruction::new(K::Say {
                        text: Value::text("round"),
                    })],
                },
                K::Wait {
                    duration: number(0.1),
                },
                K::Say {
                    text: Value::text("slept"),
                },
            ],
        )],
        &[],
    );
}

/// Each call runs on a state of its own, so a reporter may call itself. This
/// one counts down, which only works if the inner call's input doesn't
/// trample the outer one's.
#[test]
fn a_recursive_reporter_keeps_each_calls_own_inputs() {
    assert_same_blocks(
        "recursion",
        vec![vec![K::Say {
            text: call("fact", vec![number(5.0)]),
        }]],
        vec![block(
            "fact",
            &["n"],
            vec![
                K::If {
                    condition: op("Lte", vec![param("n"), number(1.0)]),
                    body: vec![Instruction::new(K::Return { value: number(1.0) })],
                },
                K::Return {
                    value: op(
                        "Mul",
                        vec![
                            param("n"),
                            call("fact", vec![op("Sub", vec![param("n"), number(1.0)])]),
                        ],
                    ),
                },
            ],
        )],
        &[],
    );
}

/// And when it never stops, both sides give up at the same depth and say so
/// in the same words.
#[test]
fn a_reporter_that_never_stops_gives_up_at_the_same_depth() {
    assert_same_blocks(
        "toodeep",
        vec![vec![K::Say {
            text: call("b1", vec![]),
        }]],
        vec![block(
            "b1",
            &[],
            vec![K::Return {
                value: call("b1", vec![]),
            }],
        )],
        &[],
    );
}

/// A `return` from inside a loop leaves the whole block, not just the loop.
#[test]
fn a_return_from_inside_a_loop_leaves_the_block() {
    assert_same_blocks(
        "returnout",
        vec![vec![
            calling("b1", vec![]),
            K::Say {
                text: Value::text("back"),
            },
            K::Say {
                text: call("b2", vec![]),
            },
        ]],
        vec![
            block(
                "b1",
                &[],
                vec![
                    K::Forever {
                        body: vec![
                            Instruction::new(K::Say {
                                text: Value::text("once"),
                            }),
                            Instruction::new(K::Return { value: number(0.0) }),
                        ],
                    },
                    K::Say {
                        text: Value::text("never"),
                    },
                ],
            ),
            block(
                "b2",
                &[],
                vec![K::Repeat {
                    count: number(5.0),
                    body: vec![Instruction::new(K::Return { value: number(7.0) })],
                }],
            ),
        ],
        &[],
    );
}

/// A call to a block that isn't there is stepped over, arguments and all.
#[test]
fn a_call_to_nothing_is_skipped_the_same_way() {
    assert_same_blocks(
        "missing",
        vec![vec![
            calling("gone", vec![op("Div", vec![number(1.0), number(0.0)])]),
            K::Say {
                text: call("gone", vec![op("Div", vec![number(1.0), number(0.0)])]),
            },
            K::Say {
                text: Value::text("carried on"),
            },
        ]],
        Vec::new(),
        &[],
    );
}

/// Sensing on both sides of an `and`: the left is asked, the right is not,
/// and both halves have to agree about which.
#[test]
fn sensing_on_both_sides_of_a_short_circuit_agrees() {
    assert_same(
        "bothsides",
        vec![
            K::Say {
                text: op(
                    "And",
                    vec![
                        op("KeyDown", vec![Value::text("escape")]),
                        op(
                            "ActorPosition",
                            vec![Value::text("Nobody"), Value::text("X")],
                        ),
                    ],
                ),
            },
            K::Say {
                text: op(
                    "And",
                    vec![
                        op("KeyDown", vec![Value::text("space")]),
                        op(
                            "ActorPosition",
                            vec![Value::text("Friend"), Value::text("X")],
                        ),
                    ],
                ),
            },
        ],
        &[],
    );
}

#[test]
fn a_project_the_compiler_cant_do_is_refused_by_name() {
    // Two blocks that call each other as statements. Their loops would share
    // one set of counters where the VM gives every invocation a frame.
    let project = project_with_blocks(
        vec![vec![calling("b1", vec![])]],
        vec![
            block("b1", &[], vec![calling("b2", vec![])]),
            block("b2", &[], vec![calling("b1", vec![])]),
        ],
        &[],
    );
    let error = blockloom_core::codegen::compile(&project)
        .expect_err("a block that can reach itself is refused");
    assert_eq!(error.what, "a custom block that calls itself");
}

/// A statement call inside a reporter's body. It runs on the reporter's own
/// state, so the frame it pushes is the reporter's and not the strand's.
#[test]
fn a_statement_call_inside_a_reporter_stays_inside_it() {
    assert_same_blocks(
        "nestedcall",
        vec![vec![
            K::Say {
                text: call("outer", vec![number(3.0)]),
            },
            K::Say {
                text: Value::text("after"),
            },
        ]],
        vec![
            block(
                "outer",
                &["n"],
                vec![
                    calling("inner", vec![param("n")]),
                    // The inner call has been and gone, so this reads the
                    // outer block's own input again.
                    K::Return {
                        value: op("Add", vec![param("n"), number(100.0)]),
                    },
                ],
            ),
            block(
                "inner",
                &["m"],
                vec![K::Move {
                    steps: op("Mul", vec![param("m"), number(2.0)]),
                }],
            ),
        ],
        &[],
    );
}

/// One body, called both ways. Its inputs are read back the same however it
/// was entered, since both bind them by position.
#[test]
fn one_block_serves_as_a_statement_and_as_a_reporter() {
    assert_same_blocks(
        "bothways",
        vec![vec![
            calling("b1", vec![number(2.0)]),
            K::Say {
                text: call("b1", vec![number(5.0)]),
            },
        ]],
        vec![block(
            "b1",
            &["n"],
            vec![
                K::Move { steps: param("n") },
                K::Return {
                    value: op("Add", vec![param("n"), number(1.0)]),
                },
            ],
        )],
        &[],
    );
}

/// A `stop all` inside a reporter ends the run - but not before the strand
/// that asked for the value finishes the slice it was in.
#[test]
fn stopping_from_inside_a_reporter_ends_the_run_the_same_way() {
    assert_same_blocks(
        "stopinside",
        vec![
            vec![
                K::Say {
                    text: call("b1", vec![]),
                },
                K::Say {
                    text: Value::text("still this tick"),
                },
            ],
            vec![K::Forever {
                body: vec![Instruction::new(K::Move { steps: number(1.0) })],
            }],
        ],
        vec![block(
            "b1",
            &[],
            vec![
                K::Say {
                    text: Value::text("about to stop"),
                },
                K::StopAll,
            ],
        )],
        &[],
    );
}

/// A call frame has to survive a suspension: the block waits, and the strand
/// has to come back into it and then find its way home - twice, from two
/// different call sites.
#[test]
fn a_call_frame_survives_the_wait_inside_it() {
    assert_same_blocks(
        "suspendedcall",
        vec![vec![
            calling("b1", vec![number(1.0)]),
            K::Say {
                text: Value::text("first back"),
            },
            calling("b1", vec![number(2.0)]),
            K::Say {
                text: Value::text("second back"),
            },
        ]],
        vec![block(
            "b1",
            &["n"],
            vec![
                K::Move { steps: param("n") },
                K::Wait {
                    duration: number(0.05),
                },
                // Read after the wait: the frame, and its inputs, are still
                // there when the strand picks the block back up.
                K::Say { text: param("n") },
            ],
        )],
        &[],
    );
}

// ─── Actors that come and go ────────────────────────────────────────────────
// A clone is the one place a compiled program runs one emitted function under
// an id the document never had, so what matters here is that both halves make
// the same actors, in the same order, and give them the same slices.

fn say(text: &str) -> K {
    K::Say {
        text: Value::text(text),
    }
}

fn clone_of(name: &str) -> K {
    K::CreateClone {
        of: name.to_string(),
    }
}

fn delete(target: &str) -> K {
    K::DeleteActor {
        target: Value::text(target),
    }
}

#[test]
fn a_clone_runs_its_own_strand_under_its_own_id() {
    assert_same_headed(
        "clones",
        vec![
            (
                K::WhenStarted,
                vec![clone_of(""), clone_of(""), say("made them")],
            ),
            (
                K::WhenCloned,
                vec![say("I am new"), K::Move { steps: number(1.0) }],
            ),
        ],
    );
}

#[test]
fn a_clone_of_a_clone_is_a_clone_of_the_same_authored_actor() {
    assert_same_headed(
        "clones-of-clones",
        vec![
            (K::WhenStarted, vec![clone_of("Player")]),
            (
                K::WhenCloned,
                vec![
                    say("copy"),
                    K::Wait {
                        duration: number(0.1),
                    },
                    K::StopAll,
                ],
            ),
        ],
    );
}

#[test]
fn a_clone_keeps_the_variables_its_template_had_and_counts_its_own() {
    assert_same_headed(
        "clone-variables",
        vec![
            (
                K::WhenStarted,
                vec![
                    K::SetVariable {
                        name: "hits".to_string(),
                        value: number(7.0),
                    },
                    clone_of(""),
                    K::ChangeVariable {
                        name: "hits".to_string(),
                        value: number(100.0),
                    },
                    say("template done"),
                ],
            ),
            (
                K::WhenCloned,
                vec![
                    K::ChangeVariable {
                        name: "hits".to_string(),
                        value: number(1.0),
                    },
                    K::Say {
                        text: Value::Var {
                            name: "hits".to_string(),
                        },
                    },
                ],
            ),
        ],
    );
}

#[test]
fn an_actor_made_mid_run_is_named_and_placed_the_same_way_by_both() {
    assert_same(
        "create-actor",
        vec![
            K::CreateActor {
                name: Value::text("Bullet"),
                x: number(3.0),
                y: op("Add", vec![number(2.0), number(2.0)]),
                z: number(0.0),
            },
            say("made one"),
            delete("Bullet"),
            say("and unmade it"),
        ],
        &[],
    );
}

#[test]
fn deleting_myself_ends_the_strand_where_it_stands() {
    assert_same_strands(
        "delete-myself",
        vec![
            vec![say("bye"), delete(""), say("never said")],
            vec![
                K::Wait {
                    duration: number(0.1),
                },
                say("the other strand went with it"),
            ],
        ],
        &[],
    );
}

#[test]
fn naming_nobody_is_reported_by_both_and_kills_neither() {
    assert_same(
        "no-such-actor",
        vec![
            delete("Nobody"),
            clone_of("Nobody"),
            say("carried on regardless"),
        ],
        &[],
    );
}

// ─── The interface ──────────────────────────────────────────────────────────
// Every `show` block reads a run of slots left to right, so what matters here
// is not only that the element comes out the same but that the two halves
// asked the world for its pieces in the same order - which is why the cases
// below put reporters in the slots rather than plain numbers.

fn panel(id: &str, title: Value, modal: bool, parent: &str) -> K {
    K::ShowPanel {
        element: Value::text(id),
        title,
        modal,
        anchor: UiAnchor::Center,
        x: number(0.0),
        y: number(0.0),
        width: number(240.0),
        height: number(0.0),
        parent: Value::text(parent),
    }
}

fn label(id: &str, text: Value, parent: &str) -> K {
    K::ShowLabel {
        element: Value::text(id),
        text,
        anchor: UiAnchor::TopLeft,
        x: number(12.0),
        y: number(12.0),
        width: number(0.0),
        height: number(0.0),
        parent: Value::text(parent),
    }
}

#[test]
fn every_show_block_asks_for_its_slots_in_the_same_order() {
    assert_same(
        "interface-show",
        vec![
            panel("menu", Value::text("Paused"), true, ""),
            K::ShowButton {
                element: Value::text("resume"),
                label: op("Join", vec![Value::text("Res"), Value::text("ume")]),
                anchor: UiAnchor::Center,
                x: number(0.0),
                y: number(0.0),
                width: number(0.0),
                height: number(0.0),
                parent: Value::text("menu"),
            },
            K::ShowImage {
                element: Value::text("logo"),
                asset: Value::text("assets/logo.png"),
                anchor: UiAnchor::Top,
                x: number(0.0),
                y: number(8.0),
                width: number(64.0),
                height: number(64.0),
                parent: Value::text(""),
            },
            K::ShowInput {
                element: Value::text("name"),
                placeholder: Value::text("your name"),
                anchor: UiAnchor::Center,
                x: number(0.0),
                y: number(40.0),
                width: number(180.0),
                height: number(0.0),
                parent: Value::text("menu"),
            },
            // The one row with three slots of its own, and the one whose
            // starting value is asked for rather than fixed.
            K::ShowSlider {
                element: Value::text("volume"),
                min: number(0.0),
                max: op("Add", vec![number(5.0), number(5.0)]),
                value: op("MyPosition", vec![Value::text("X")]),
                anchor: UiAnchor::Center,
                x: number(0.0),
                y: number(80.0),
                width: number(180.0),
                height: number(0.0),
                parent: Value::text("menu"),
            },
            K::ShowToggle {
                element: Value::text("shadows"),
                label: Value::text("Shadows"),
                on: true,
                anchor: UiAnchor::Center,
                x: number(0.0),
                y: number(120.0),
                width: number(0.0),
                height: number(0.0),
                parent: Value::text("menu"),
            },
        ],
        &[],
    );
}

#[test]
fn writing_hiding_and_deleting_an_element_land_the_same_way() {
    assert_same(
        "interface-change",
        vec![
            label("score", Value::text("score: 0"), ""),
            K::SetUiProp {
                prop: UiProp::Text,
                element: Value::text("score"),
                value: op("Join", vec![Value::text("score: "), number(3.0)]),
            },
            K::SetUiProp {
                prop: UiProp::TextSize,
                element: Value::text("score"),
                value: number(22.0),
            },
            // A bad slot is reported once and stands a zero in its place,
            // on both sides and in the same place.
            K::SetUiProp {
                prop: UiProp::Width,
                element: Value::text("score"),
                value: op("Div", vec![number(1.0), number(0.0)]),
            },
            K::HideElement {
                element: Value::text("score"),
            },
            K::DeleteElement {
                element: Value::text("score"),
            },
            K::HideAllUi,
        ],
        &[],
    );
}

#[test]
fn a_paused_world_stops_every_strand_the_interface_did_not_start() {
    assert_same_strands(
        "interface-pause",
        vec![
            vec![
                say("before"),
                K::PauseGame,
                // The pause takes hold where it stands, the strand that ran
                // it included, so neither of these ever runs.
                say("never said"),
                K::Wait {
                    duration: number(0.1),
                },
                say("nor this"),
            ],
            // A second strand, later in the same tick: frozen too, wherever
            // in the tick the block landed.
            vec![
                K::Wait {
                    duration: number(0.05),
                },
                say("nor this either"),
            ],
        ],
        &[],
    );
}

#[test]
fn a_strand_the_interface_started_runs_through_a_pause_and_ends_it() {
    assert_same_headed(
        "interface-resume",
        vec![
            (
                K::WhenStarted,
                vec![say("before"), K::PauseGame, say("after")],
            ),
            // The interface's own strand: it pauses nothing by running, and
            // a `pause game` inside it doesn't stop it either.
            (
                K::WhenUiClicked {
                    element: "resume".to_string(),
                },
                vec![
                    K::PauseGame,
                    say("the menu is alive"),
                    K::ResumeGame,
                    say("and the world is back"),
                ],
            ),
            // Frozen at its `wait` until that resume lands, then it finishes.
            (
                K::WhenStarted,
                vec![
                    K::Wait {
                        duration: number(0.05),
                    },
                    say("the world moved again"),
                ],
            ),
        ],
    );
}
