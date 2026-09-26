//! Block VM throughput: how long one `Vm::tick` takes over a few typical
//! canvases. Run with `cargo bench -p blockloom-core --bench vm`.
//!
//! No bench framework: each case is timed over a fixed batch of ticks,
//! repeated, and the median batch is reported as nanoseconds per tick.

use blockloom_core::blocks::{
    BlockDef, BlockPiece, BlockShape, Instruction, InstructionKind, ListDef, ListItem, Strand,
};
use blockloom_core::project::{Actor, Project};
use blockloom_core::scene::{Axis, Mode, Visual, World};
use blockloom_core::sense::{self, Sensors};
use blockloom_core::value::{Op, Value};
use blockloom_core::vm::{Event, Vm};
use std::hint::black_box;
use std::time::Instant;

const ACTORS: usize = 50;
const TICKS: usize = 200;
const SAMPLES: usize = 25;

fn var(name: &str) -> Value {
    Value::Var {
        name: name.to_string(),
    }
}

fn param(name: &str) -> Value {
    Value::Param {
        name: name.to_string(),
    }
}

fn num(n: f64) -> Value {
    Value::number(n)
}

fn op(op: Op, args: Vec<Value>) -> Value {
    Value::op(op, args)
}

fn set(name: &str, value: Value) -> Instruction {
    Instruction::new(InstructionKind::SetVariable {
        name: name.to_string(),
        value,
    })
}

fn forever(body: Vec<Instruction>) -> Strand {
    Strand::with_instructions(
        0,
        0,
        vec![
            Instruction::new(InstructionKind::WhenStarted),
            Instruction::new(InstructionKind::Forever { body }),
        ],
    )
}

fn block(id: &str, shape: BlockShape, inputs: &[&str]) -> BlockDef {
    let mut pieces = vec![BlockPiece::Label {
        id: "l".to_string(),
        text: id.to_string(),
    }];
    for (i, name) in inputs.iter().enumerate() {
        pieces.push(BlockPiece::Input {
            id: format!("i{i}"),
            name: name.to_string(),
            value_type: Default::default(),
        });
    }
    BlockDef {
        id: id.to_string(),
        pieces,
        shape,
        color: "#4C97FF".to_string(),
    }
}

fn body_of(block_id: &str, body: Vec<Instruction>) -> Strand {
    let mut instructions = vec![Instruction::new(InstructionKind::BlockHeader {
        block_id: block_id.to_string(),
    })];
    instructions.extend(body);
    Strand::with_instructions(0, 0, instructions)
}

fn project(actor: impl Fn(usize) -> Actor) -> Project {
    Project {
        id: "bench".to_string(),
        name: "bench".to_string(),
        icon: String::new(),
        world: World {
            mode: Mode::TwoD,
            ..Default::default()
        },
        actors: (0..ACTORS).map(actor).collect(),
        globals: Vec::new(),
        global_lists: Vec::new(),
        global_dicts: Vec::new(),
    }
}

fn actor(i: usize, strands: Vec<Strand>) -> Actor {
    let mut actor = Actor::new(
        format!("A{i}"),
        Visual::Rect {
            color: "#FFFFFF".to_string(),
            size: [10.0, 10.0],
        },
    );
    actor.graph.strands = strands;
    actor
}

/// Variables and arithmetic: a counter, a derived value and a branch.
fn arithmetic() -> Project {
    let mut p = project(|i| {
        let mut a = actor(
            i,
            vec![forever(vec![
                set("x", op(Op::Add, vec![var("x"), num(1.0)])),
                set(
                    "y",
                    op(
                        Op::Sub,
                        vec![
                            op(Op::Mul, vec![var("x"), num(2.0)]),
                            op(Op::Div, vec![var("y"), num(3.0)]),
                        ],
                    ),
                ),
                Instruction::new(InstructionKind::If {
                    condition: op(
                        Op::And,
                        vec![
                            op(Op::Gt, vec![var("x"), num(10.0)]),
                            op(
                                Op::Eq,
                                vec![op(Op::Mod, vec![var("x"), num(7.0)]), num(0.0)],
                            ),
                        ],
                    ),
                    body: vec![Instruction::new(InstructionKind::ChangeVariable {
                        name: "hits".to_string(),
                        value: num(1.0),
                    })],
                }),
            ])],
        );
        a.graph.create_variable("x").ok();
        a.graph.create_variable("y").ok();
        a
    });
    p.create_global("hits").ok();
    p
}

/// Effects with constant and sensed arguments, the shape most game loops are.
fn effects() -> Project {
    project(|i| {
        actor(
            i,
            vec![forever(vec![
                Instruction::new(InstructionKind::ChangePosition {
                    axis: Axis::X,
                    by: op(Op::Mul, vec![num(2.0), num(0.5)]),
                }),
                Instruction::new(InstructionKind::Turn {
                    axis: Axis::Z,
                    degrees: op(
                        Op::Add,
                        vec![op(Op::Ext("MouseX".into()), vec![]), num(1.0)],
                    ),
                }),
                Instruction::new(InstructionKind::Move { steps: num(3.0) }),
            ])],
        )
    })
}

/// A command block with inputs and a reporter block, both called every tick.
fn custom_blocks() -> Project {
    project(|i| {
        let mut a = actor(
            i,
            vec![
                forever(vec![
                    Instruction::new(InstructionKind::CallBlock {
                        block_id: "step".to_string(),
                        args: vec![var("x"), num(2.0)],
                    }),
                    set(
                        "x",
                        Value::Call {
                            block_id: "double".to_string(),
                            args: vec![var("x")],
                            branches: Vec::new(),
                            saved: Box::new(num(0.0)),
                        },
                    ),
                ]),
                body_of(
                    "step",
                    vec![set(
                        "y",
                        op(
                            Op::Mul,
                            vec![op(Op::Add, vec![param("a"), param("b")]), num(0.5)],
                        ),
                    )],
                ),
                body_of(
                    "double",
                    vec![Instruction::new(InstructionKind::Return {
                        value: op(
                            Op::Mod,
                            vec![op(Op::Mul, vec![param("n"), num(2.0)]), num(1000.0)],
                        ),
                    })],
                ),
            ],
        );
        a.graph
            .block_defs
            .push(block("step", BlockShape::Normal, &["a", "b"]));
        a.graph
            .block_defs
            .push(block("double", BlockShape::ReturnsValue, &["n"]));
        a.graph.create_variable("x").ok();
        a.graph.create_variable("y").ok();
        a
    })
}

fn list(name: &str, len: usize) -> ListDef {
    ListDef {
        name: name.to_string(),
        items: (0..len).map(|i| ListItem::Number(i as f64)).collect(),
        editor_visible: false,
        editor_x: 0,
        editor_y: 0,
    }
}

/// Reading an actor's own list beside a big shared one.
fn lists() -> Project {
    let mut p = project(|i| {
        let mut a = actor(
            i,
            vec![forever(vec![
                set("x", op(Op::Add, vec![var("x"), num(1.0)])),
                set(
                    "item",
                    op(
                        Op::Ext("ListItem".into()),
                        vec![
                            op(
                                Op::Add,
                                vec![op(Op::Mod, vec![var("x"), num(20.0)]), num(1.0)],
                            ),
                            Value::text("inventory"),
                        ],
                    ),
                ),
                set(
                    "count",
                    op(Op::Ext("ListLength".into()), vec![Value::text("inventory")]),
                ),
            ])],
        );
        a.graph.lists.push(list("inventory", 20));
        a.graph.create_variable("x").ok();
        a
    });
    p.global_lists.push(list("scores", 1000));
    p
}

fn bench(name: &str, project: Project) {
    let mut vm = Vm::new();
    vm.load(&project);
    vm.fire(Event::Started);
    let mut out = Vec::new();
    let mut now = 0.0;
    let mut tick = |vm: &mut Vm, out: &mut Vec<_>| {
        now += 1.0 / 60.0;
        sense::publish(Sensors {
            time: now,
            wall_time: now,
            ..Default::default()
        });
        vm.tick(now, out);
        out.clear();
    };
    // Warm up: start the strands and settle allocations.
    for _ in 0..TICKS {
        tick(&mut vm, &mut out);
    }
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        for _ in 0..TICKS {
            tick(&mut vm, &mut out);
        }
        samples.push(start.elapsed().as_nanos() as f64 / TICKS as f64);
    }
    black_box(vm.variables());
    samples.sort_by(f64::total_cmp);
    let median = samples[SAMPLES / 2];
    println!(
        "{name:<14} {:>10.0} ns/tick  {:>7.0} ns/actor",
        median,
        median / ACTORS as f64
    );
}

fn main() {
    blockloom_core::init();
    // `cargo bench` passes `--bench`; a filter argument picks cases by name.
    let filter = std::env::args().skip(1).find(|arg| !arg.starts_with('-'));
    type Case = (&'static str, fn() -> Project);
    let cases: [Case; 4] = [
        ("arithmetic", arithmetic),
        ("effects", effects),
        ("custom_blocks", custom_blocks),
        ("lists", lists),
    ];
    for (name, make) in cases {
        if filter.as_deref().is_none_or(|f| name.contains(f)) {
            bench(name, make());
        }
    }
}
