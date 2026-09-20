//! End-to-end VM behavior: canvases in, effects out. These are the scheduling
//! rules `blockloom-runtime` relies on, checked without a window.

use blockloom_core::blocks::{
    BlockDef, BlockPiece, BlockShape, Instruction, InstructionKind, Strand,
};
use blockloom_core::project::{Actor, Project};
use blockloom_core::scene::{Axis, Mode, Visual};
use blockloom_core::sense::Sensors;
use blockloom_core::value::{Evaluated, Op, Value};
use blockloom_core::vm::{Effect, Event, Vm};

fn rect() -> Visual {
    Visual::Rect {
        color: "#FFFFFF".to_string(),
        size: [10.0, 10.0],
    }
}

/// A project with one actor whose canvas is `strands`.
fn project_with(strands: Vec<Strand>) -> Project {
    let mut actor = Actor::new("Player", rect());
    actor.graph.strands = strands;
    Project {
        id: "p".to_string(),
        name: "test".to_string(),
        world: blockloom_core::scene::World {
            mode: Mode::TwoD,
            ..Default::default()
        },
        actors: vec![actor],
        globals: Vec::new(),
    }
}

fn started(body: Vec<InstructionKind>) -> Strand {
    let mut instructions = vec![Instruction::new(InstructionKind::WhenStarted)];
    instructions.extend(body.into_iter().map(Instruction::new));
    Strand::with_instructions(0, 0, instructions)
}

fn ins(kind: InstructionKind) -> Instruction {
    Instruction::new(kind)
}

fn move_by(steps: f64) -> InstructionKind {
    InstructionKind::Move {
        steps: Value::number(steps),
    }
}

fn say(text: &str) -> InstructionKind {
    InstructionKind::Say {
        text: Value::text(text),
    }
}

/// A VM plus the clock a host would drive it with: every `run` advances a
/// tenth of a second per frame and publishes the matching sensor snapshot,
/// exactly as `blockloom-runtime` does.
struct Harness {
    vm: Vm,
    time: f64,
}

impl Harness {
    fn new(project: &Project) -> Self {
        blockloom_core::init();
        let mut vm = Vm::new();
        vm.load(project);
        Self { vm, time: 0.0 }
    }

    fn started(project: &Project) -> Self {
        let mut harness = Self::new(project);
        harness.vm.fire(Event::Started);
        harness
    }

    fn run(&mut self, frames: usize) -> Vec<Effect> {
        let mut out = Vec::new();
        for _ in 0..frames {
            self.time += 0.1;
            blockloom_core::sense::publish(Sensors {
                time: self.time,
                ..Default::default()
            });
            self.vm.tick(self.time, &mut out);
        }
        out
    }
}

fn says(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Say { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn moves(effects: &[Effect]) -> Vec<f32> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Move { steps, .. } => Some(*steps),
            _ => None,
        })
        .collect()
}

#[test]
fn a_started_script_runs_its_body_in_one_frame() {
    let project = project_with(vec![started(vec![move_by(10.0), say("hi")])]);
    let mut vm = Harness::started(&project);
    let effects = vm.run(1);
    assert_eq!(moves(&effects), vec![10.0]);
    assert_eq!(says(&effects), vec!["hi".to_string()]);
    assert!(!vm.vm.is_running());
}

#[test]
fn a_repeat_loop_runs_one_iteration_per_frame_and_counts_once() {
    let project = project_with(vec![started(vec![InstructionKind::Repeat {
        count: Value::number(3.0),
        body: vec![ins(move_by(1.0))],
    }])]);
    let mut vm = Harness::started(&project);
    let after_one = vm.run(1);
    assert_eq!(moves(&after_one).len(), 1);
    let rest = vm.run(5);
    assert_eq!(moves(&rest).len(), 2);
    assert!(!vm.vm.is_running());
}

#[test]
fn a_forever_loop_keeps_going_one_step_at_a_time() {
    let project = project_with(vec![started(vec![InstructionKind::Forever {
        body: vec![ins(InstructionKind::ChangePosition {
            axis: Axis::X,
            by: Value::number(2.0),
        })],
    }])]);
    let mut vm = Harness::started(&project);
    let effects = vm.run(4);
    assert_eq!(effects.len(), 4);
    assert!(vm.vm.is_running());
}

#[test]
fn escape_loop_leaves_the_loop_and_continue_skips_the_rest_of_the_iteration() {
    let project = project_with(vec![started(vec![
        InstructionKind::Repeat {
            count: Value::number(5.0),
            body: vec![ins(move_by(1.0)), ins(InstructionKind::EscapeLoop)],
        },
        say("done"),
    ])]);
    let mut vm = Harness::started(&project);
    let effects = vm.run(10);
    assert_eq!(moves(&effects), vec![1.0]);
    assert_eq!(says(&effects), vec!["done".to_string()]);
}

#[test]
fn a_wait_suspends_the_script_for_exactly_that_long() {
    let project = project_with(vec![started(vec![
        say("before"),
        InstructionKind::Wait {
            duration: Value::number(0.25),
        },
        say("after"),
    ])]);
    let mut vm = Harness::started(&project);
    // Frames are 0.1s apart, so "after" can't land before the fourth.
    let early = vm.run(3);
    assert_eq!(says(&early), vec!["before".to_string()]);
    let late = vm.run(3);
    assert_eq!(says(&late), vec!["after".to_string()]);
}

#[test]
fn wait_until_rechecks_its_condition_every_frame() {
    // `wait until (timer > 0.25)` - nothing else can move it along.
    let condition = Value::op(
        Op::Gt,
        vec![
            Value::op(Op::from_name("Timer"), vec![]),
            Value::number(0.25),
        ],
    );
    let project = project_with(vec![started(vec![
        InstructionKind::WaitUntil { condition },
        say("go"),
    ])]);
    let mut vm = Harness::started(&project);
    // Frames land on 0.1 and 0.2 seconds: still waiting.
    assert!(says(&vm.run(2)).is_empty());
    assert_eq!(says(&vm.run(2)), vec!["go".to_string()]);
}

#[test]
fn variables_are_read_and_changed_through_their_own_scope() {
    let mut project = project_with(vec![started(vec![
        InstructionKind::SetVariable {
            name: "score".to_string(),
            value: Value::number(1.0),
        },
        InstructionKind::ChangeVariable {
            name: "score".to_string(),
            value: Value::number(4.0),
        },
        InstructionKind::Say {
            text: Value::Var {
                name: "score".to_string(),
            },
        },
    ])]);
    project.create_global("score").unwrap();
    let mut vm = Harness::started(&project);
    assert_eq!(says(&vm.run(1)), vec!["5".to_string()]);
    assert_eq!(
        vm.vm.variables().globals.get("score"),
        Some(&Evaluated::Number(5.0))
    );
}

#[test]
fn a_broadcast_starts_every_listening_strand_on_the_next_frame() {
    let project = project_with(vec![
        started(vec![InstructionKind::Broadcast {
            name: "jump".to_string(),
        }]),
        Strand::with_instructions(
            0,
            0,
            vec![
                ins(InstructionKind::WhenMessage {
                    name: "jump".to_string(),
                }),
                ins(say("jumped")),
            ],
        ),
    ]);
    let mut vm = Harness::started(&project);
    assert!(says(&vm.run(1)).is_empty());
    assert_eq!(says(&vm.run(1)), vec!["jumped".to_string()]);
}

#[test]
fn a_reporter_block_returns_a_value_into_the_slot_that_called_it() {
    // `double (n)` returns n * 2; `say (double (21))` says 42.
    let block = BlockDef {
        id: "b1".to_string(),
        pieces: vec![
            BlockPiece::Label {
                id: "l".to_string(),
                text: "double".to_string(),
            },
            BlockPiece::Input {
                id: "i".to_string(),
                name: "n".to_string(),
                value_type: Default::default(),
            },
        ],
        shape: BlockShape::ReturnsValue,
        color: "#4C97FF".to_string(),
    };
    let call = Value::Call {
        block_id: "b1".to_string(),
        args: vec![Value::number(21.0)],
        saved: Box::new(Value::number(0.0)),
    };
    let mut project = project_with(vec![
        started(vec![InstructionKind::Say { text: call }]),
        Strand::with_instructions(
            0,
            0,
            vec![
                ins(InstructionKind::BlockHeader {
                    block_id: "b1".to_string(),
                }),
                ins(InstructionKind::Return {
                    value: Value::op(
                        Op::Mul,
                        vec![
                            Value::Param {
                                name: "n".to_string(),
                            },
                            Value::number(2.0),
                        ],
                    ),
                }),
            ],
        ),
    ]);
    project.actors[0].graph.block_defs.push(block);
    let mut vm = Harness::started(&project);
    assert_eq!(says(&vm.run(1)), vec!["42".to_string()]);
}

#[test]
fn a_command_block_runs_inline_and_comes_back() {
    let block = BlockDef {
        id: "b2".to_string(),
        pieces: vec![BlockPiece::Label {
            id: "l".to_string(),
            text: "hop".to_string(),
        }],
        shape: BlockShape::Normal,
        color: "#4C97FF".to_string(),
    };
    let mut project = project_with(vec![
        started(vec![
            InstructionKind::CallBlock {
                block_id: "b2".to_string(),
                args: vec![],
            },
            say("after"),
        ]),
        Strand::with_instructions(
            0,
            0,
            vec![
                ins(InstructionKind::BlockHeader {
                    block_id: "b2".to_string(),
                }),
                ins(say("inside")),
            ],
        ),
    ]);
    project.actors[0].graph.block_defs.push(block);
    let mut vm = Harness::started(&project);
    assert_eq!(
        says(&vm.run(1)),
        vec!["inside".to_string(), "after".to_string()]
    );
}

#[test]
fn stop_all_ends_every_script_including_the_one_that_asked() {
    let project = project_with(vec![
        started(vec![InstructionKind::Forever {
            body: vec![ins(move_by(1.0))],
        }]),
        started(vec![
            InstructionKind::Wait {
                duration: Value::number(0.15),
            },
            InstructionKind::StopAll,
        ]),
    ]);
    let mut vm = Harness::started(&project);
    vm.run(3);
    assert!(!vm.vm.is_running());
    let after = vm.run(3);
    assert!(moves(&after).is_empty());
}

#[test]
fn a_key_press_starts_its_own_strand_and_restarts_it_when_pressed_again() {
    let project = project_with(vec![Strand::with_instructions(
        0,
        0,
        vec![
            ins(InstructionKind::WhenKeyPressed {
                key: "Space".to_string(),
            }),
            ins(say("pressed")),
        ],
    )]);
    let mut vm = Harness::new(&project);
    assert!(says(&vm.run(1)).is_empty());
    vm.vm.fire(Event::Key("space".to_string()));
    assert_eq!(says(&vm.run(1)), vec!["pressed".to_string()]);
}

#[test]
fn a_collision_only_starts_the_strand_whose_target_matches() {
    let mut project = project_with(vec![Strand::with_instructions(
        0,
        0,
        vec![
            ins(InstructionKind::WhenCollision {
                with: "Ground".to_string(),
            }),
            ins(say("landed")),
        ],
    )]);
    let ground = project.add_actor(Actor::new("Ground", rect()));
    let wall = project.add_actor(Actor::new("Wall", rect()));
    let player = project.actors[0].id.clone();
    let mut vm = Harness::new(&project);
    vm.vm.fire(Event::Collision {
        actor: player.clone(),
        with: wall,
    });
    assert!(says(&vm.run(1)).is_empty());
    vm.vm.fire(Event::Collision {
        actor: player,
        with: ground,
    });
    assert_eq!(says(&vm.run(1)), vec!["landed".to_string()]);
}

#[test]
fn a_bad_value_reports_an_error_and_the_script_carries_on() {
    let project = project_with(vec![started(vec![
        InstructionKind::Move {
            steps: Value::op(Op::Div, vec![Value::number(1.0), Value::number(0.0)]),
        },
        say("still here"),
    ])]);
    let mut vm = Harness::started(&project);
    let effects = vm.run(1);
    assert!(effects.iter().any(|e| matches!(e, Effect::Error { .. })));
    assert_eq!(says(&effects), vec!["still here".to_string()]);
}
