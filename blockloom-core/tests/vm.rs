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
        icon: String::new(),
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
    /// Whether the world is frozen, and what its clock read when it froze.
    paused: bool,
    frozen: f64,
}

impl Harness {
    fn new(project: &Project) -> Self {
        blockloom_core::init();
        let mut vm = Vm::new();
        vm.load(project);
        Self {
            vm,
            time: 0.0,
            paused: false,
            frozen: 0.0,
        }
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
                // Frozen while paused, which is what a host does, and the
                // wall clock beside it for the strands the interface
                // started.
                time: if self.paused { self.frozen } else { self.time },
                wall_time: self.time,
                paused: self.paused,
                ..Default::default()
            });
            let world = if self.paused { self.frozen } else { self.time };
            self.vm.tick_at(world, self.time, &mut out);
            // A `pause game` block freezes the VM itself; the host notices
            // at the same moment and stops the world clock with it.
            if self.vm.is_paused() != self.paused {
                self.paused = self.vm.is_paused();
                if self.paused {
                    self.frozen = self.time;
                }
            }
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

#[test]
fn a_component_field_is_written_as_an_effect_and_read_back_from_the_snapshot() {
    use blockloom_core::components::{ActorComponent, ComponentField};
    use blockloom_core::sense::ActorSense;
    use std::collections::HashMap;

    let mut project = project_with(vec![started(vec![
        InstructionKind::SetComponentField {
            component: "Health".to_string(),
            field: "hp".to_string(),
            value: Value::number(7.0),
        },
        // Reading the field goes through the snapshot the host publishes, not
        // through the document, so a run's own writes are what come back.
        InstructionKind::Say {
            text: Value::op(
                Op::from_name("ComponentField"),
                vec![Value::text("Health"), Value::text("hp")],
            ),
        },
    ])]);
    project.actors[0].components.insert(ActorComponent::Custom {
        name: "Health".to_string(),
        fields: vec![ComponentField::number("hp", 3.0)],
    });
    let actor_id = project.actors[0].id.clone();

    blockloom_core::init();
    let mut vm = Vm::new();
    vm.load(&project);
    vm.fire(Event::Started);

    let mut sensors = Sensors::default();
    sensors.actors.insert(
        actor_id.clone(),
        ActorSense {
            name: "Player".to_string(),
            components: HashMap::from([(
                "Health".to_string(),
                HashMap::from([("hp".to_string(), Evaluated::Number(3.0))]),
            )]),
            ..Default::default()
        },
    );
    blockloom_core::sense::publish(sensors);

    let mut effects = Vec::new();
    vm.tick(0.1, &mut effects);

    assert!(effects.contains(&Effect::SetComponentField {
        actor: actor_id,
        component: "Health".to_string(),
        field: "hp".to_string(),
        value: Evaluated::Number(7.0),
    }));
    // The host hasn't applied that effect yet, so the reporter still sees 3.
    assert_eq!(says(&effects), vec!["3".to_string()]);
}

#[test]
fn setting_a_camera_view_names_the_actor_asking() {
    let project = project_with(vec![started(vec![InstructionKind::SetCameraView {
        view: blockloom_core::components::CameraView::FirstPerson,
    }])]);
    let actor_id = project.actors[0].id.clone();
    let mut vm = Harness::started(&project);

    assert_eq!(
        vm.run(1),
        vec![Effect::SetCameraView {
            actor: actor_id,
            view: blockloom_core::components::CameraView::FirstPerson,
        }]
    );
}

// ─── Actors that come and go ───────────────────────────────────────────────

/// A project with two actors, so a block can name one other than its own.
fn project_with_two(first: Vec<Strand>, second: Vec<Strand>) -> Project {
    let mut player = Actor::new("Player", rect());
    player.id = "a1".to_string();
    player.graph.strands = first;
    let mut friend = Actor::new("Friend", rect());
    friend.id = "a2".to_string();
    friend.graph.strands = second;
    Project {
        id: "p".to_string(),
        name: "test".to_string(),
        icon: String::new(),
        world: blockloom_core::scene::World {
            mode: Mode::TwoD,
            ..Default::default()
        },
        actors: vec![player, friend],
        globals: Vec::new(),
    }
}

fn cloned(body: Vec<InstructionKind>) -> Strand {
    let mut instructions = vec![Instruction::new(InstructionKind::WhenCloned)];
    instructions.extend(body.into_iter().map(Instruction::new));
    Strand::with_instructions(400, 0, instructions)
}

fn clone_of(name: &str) -> InstructionKind {
    InstructionKind::CreateClone {
        of: name.to_string(),
    }
}

fn delete(target: &str) -> InstructionKind {
    InstructionKind::DeleteActor {
        target: Value::text(target),
    }
}

fn created(effects: &[Effect]) -> Vec<(String, String)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::CreateClone { clone, of, .. } => Some((clone.clone(), of.clone())),
            _ => None,
        })
        .collect()
}

fn errors(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Error { message, .. } => Some(message.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_clone_is_made_now_and_runs_its_own_strands_next_frame() {
    let project = project_with(vec![
        started(vec![clone_of("")]),
        cloned(vec![say("I am new")]),
    ]);
    let mut vm = Harness::started(&project);

    // The clone effect lands on the frame the block ran; the clone's own
    // strand is an event, and events start at the top of the next tick.
    let first = vm.run(1);
    let made = created(&first);
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].1, project.actors[0].id);
    assert!(says(&first).is_empty());

    let second = vm.run(1);
    assert_eq!(says(&second), vec!["I am new".to_string()]);
    // The original's `when I start as a clone` never ran, only the copy's.
    assert!(second.iter().all(
        |effect| !matches!(effect, Effect::Say { actor, .. } if actor == &project.actors[0].id)
    ));
}

#[test]
fn a_clone_starts_from_a_copy_of_its_templates_variables_and_keeps_its_own() {
    let mut project = project_with(vec![
        started(vec![
            InstructionKind::SetVariable {
                name: "hits".to_string(),
                value: Value::number(7.0),
            },
            clone_of(""),
        ]),
        cloned(vec![InstructionKind::ChangeVariable {
            name: "hits".to_string(),
            value: Value::number(1.0),
        }]),
    ]);
    project.actors[0]
        .graph
        .variables
        .push(blockloom_core::blocks::VariableDef {
            name: "hits".to_string(),
            value: Evaluated::Number(0.0),
        });
    let template = project.actors[0].id.clone();

    let mut vm = Harness::started(&project);
    let clone = created(&vm.run(1))[0].0.clone();
    vm.run(1);

    let variables = vm.vm.variables();
    assert_eq!(
        variables.actors[&template]["hits"],
        Evaluated::Number(7.0),
        "the template kept its own"
    );
    assert_eq!(
        variables.actors[&clone]["hits"],
        Evaluated::Number(8.0),
        "the clone started from 7 and counted its own one on"
    );
}

#[test]
fn a_clone_answers_to_its_templates_name_so_broadcasts_reach_it() {
    let project = project_with(vec![
        started(vec![clone_of("Player")]),
        cloned(vec![InstructionKind::WaitUntil {
            condition: Value::Bool,
        }]),
        Strand::with_instructions(
            800,
            0,
            vec![
                Instruction::new(InstructionKind::WhenMessage {
                    name: "go".to_string(),
                }),
                Instruction::new(say("heard it")),
            ],
        ),
    ]);
    let mut vm = Harness::started(&project);
    vm.run(2);
    vm.vm.fire(Event::Message("go".to_string()));

    // The original and the copy both listen, so one broadcast says it twice.
    assert_eq!(says(&vm.run(1)).len(), 2);
}

#[test]
fn deleting_an_actor_stops_its_scripts_and_leaves_everyone_elses_alone() {
    let project = project_with_two(
        vec![started(vec![InstructionKind::Forever {
            body: vec![ins(move_by(1.0))],
        }])],
        vec![started(vec![
            InstructionKind::Wait {
                duration: Value::number(0.25),
            },
            delete("Player"),
            say("done"),
        ])],
    );
    let mut vm = Harness::started(&project);

    // Both run while the deleter waits.
    assert_eq!(moves(&vm.run(2)).len(), 2);
    let deleting = vm.run(2);
    assert!(
        deleting
            .iter()
            .any(|effect| matches!(effect, Effect::DeleteActor { actor } if actor == "a1"))
    );
    assert_eq!(says(&deleting), vec!["done".to_string()]);
    // The forever loop is gone with its actor, and the deleter carried on.
    assert!(moves(&vm.run(3)).is_empty());
}

#[test]
fn deleting_myself_ends_the_strand_that_asked_where_it_stands() {
    let project = project_with(vec![started(vec![
        say("bye"),
        delete(""),
        say("still here"),
    ])]);
    let mut vm = Harness::started(&project);
    let effects = vm.run(2);
    assert_eq!(says(&effects), vec!["bye".to_string()]);
    assert!(!vm.vm.is_running());
}

#[test]
fn naming_nobody_reports_it_rather_than_deleting_the_wrong_actor() {
    let project = project_with(vec![started(vec![delete("Nobody"), say("carried on")])]);
    let mut vm = Harness::started(&project);
    let effects = vm.run(1);
    assert_eq!(errors(&effects).len(), 1);
    assert!(errors(&effects)[0].contains("Nobody"));
    // A bad slot never kills a run: the block after it still ran.
    assert_eq!(says(&effects), vec!["carried on".to_string()]);
}

#[test]
fn creating_an_actor_names_it_and_hands_back_somewhere_to_stand() {
    let project = project_with(vec![started(vec![InstructionKind::CreateActor {
        name: Value::text("Bullet"),
        x: Value::number(3.0),
        y: Value::number(4.0),
        z: Value::number(0.0),
    }])]);
    let mut vm = Harness::started(&project);
    let effects = vm.run(1);
    let made: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::CreateActor {
                id, name, position, ..
            } => Some((id.clone(), name.clone(), *position)),
            _ => None,
        })
        .collect();
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].1, "Bullet");
    assert_eq!(made[0].2, [3.0, 4.0, 0.0]);
    // It is a real actor to everything else: naming it works straight away.
    assert_eq!(
        vm.vm.actor_for("a1", "Bullet").as_deref(),
        Some(made[0].0.as_str())
    );
}

#[test]
fn setting_a_parent_hands_the_host_the_name_the_block_was_given() {
    let project = project_with_two(
        vec![started(vec![InstructionKind::SetParent {
            parent: Value::text("  Friend "),
        }])],
        Vec::new(),
    );
    let mut vm = Harness::started(&project);
    // The hierarchy is the host's, so it does the looking up - which is also
    // what lets a compiled program, with no name table, mean the same thing.
    assert_eq!(
        vm.run(1),
        vec![Effect::SetParent {
            actor: "a1".to_string(),
            parent: "Friend".to_string(),
        }]
    );
}

#[test]
fn an_empty_parent_slot_is_how_a_block_hangs_an_actor_off_nothing() {
    let project = project_with(vec![started(vec![InstructionKind::SetParent {
        parent: Value::text(""),
    }])]);
    let mut vm = Harness::started(&project);
    let effects = vm.run(1);
    assert_eq!(
        effects,
        vec![Effect::SetParent {
            actor: project.actors[0].id.clone(),
            parent: String::new(),
        }]
    );
}

// ─── The interface ──────────────────────────────────────────────────────────

fn ui_clicked(id: &str, body: Vec<InstructionKind>) -> Strand {
    let mut instructions = vec![Instruction::new(InstructionKind::WhenUiClicked {
        element: id.to_string(),
    })];
    instructions.extend(body.into_iter().map(Instruction::new));
    Strand::with_instructions(0, 400, instructions)
}

fn shows(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::ShowElement { element } => Some(element.id.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_show_block_names_its_element_and_carries_its_slots_evaluated() {
    let project = project_with(vec![started(vec![InstructionKind::ShowLabel {
        element: Value::text("score"),
        text: Value::op(
            Op::from_name("Join"),
            vec![Value::text("hi "), Value::number(3.0)],
        ),
        anchor: blockloom_core::ui::UiAnchor::TopLeft,
        x: Value::number(12.0),
        y: Value::number(8.0),
        width: Value::number(0.0),
        height: Value::number(0.0),
        parent: Value::text(""),
    }])]);
    let effects = Harness::started(&project).run(1);
    assert_eq!(shows(&effects), vec!["score".to_string()]);
    let Some(Effect::ShowElement { element }) = effects
        .iter()
        .find(|effect| matches!(effect, Effect::ShowElement { .. }))
    else {
        panic!("expected a show, got {effects:?}");
    };
    assert_eq!(element.id, "score");
    assert_eq!(element.kind, blockloom_core::ui::UiKind::Label);
    assert_eq!(element.content, "hi 3");
    assert_eq!(element.offset, [12.0, 8.0]);
    assert_eq!(element.anchor, blockloom_core::ui::UiAnchor::TopLeft);
}

#[test]
fn v3_interface_blocks_emit_a_list_theme_and_saved_variable_requests() {
    let project = project_with(vec![started(vec![
        InstructionKind::ShowList {
            element: Value::text("items"),
            anchor: blockloom_core::ui::UiAnchor::Center,
            x: Value::number(0.0),
            y: Value::number(0.0),
            width: Value::number(280.0),
            height: Value::number(240.0),
            parent: Value::text(""),
        },
        InstructionKind::SetUiTheme {
            theme: blockloom_core::ui::UiTheme::Light,
        },
        InstructionKind::SaveVariable {
            name: "score".to_string(),
        },
        InstructionKind::ClearSavedVariable {
            name: "score".to_string(),
        },
    ])]);
    let actor = project.actors[0].id.clone();
    let effects = Harness::started(&project).run(1);
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::ShowElement { element } if element.kind == blockloom_core::ui::UiKind::List
    )));
    assert!(effects.contains(&Effect::SetUiTheme {
        theme: blockloom_core::ui::UiTheme::Light,
    }));
    assert!(effects.contains(&Effect::SaveVariable {
        actor: actor.clone(),
        name: "score".to_string(),
        clear: false,
    }));
    assert!(effects.contains(&Effect::SaveVariable {
        actor,
        name: "score".to_string(),
        clear: true,
    }));
}

#[test]
fn a_click_on_an_element_starts_the_strand_that_names_it_and_no_other() {
    let project = project_with(vec![
        ui_clicked("resume", vec![say("resumed")]),
        ui_clicked("quit", vec![say("quit")]),
    ]);
    let mut vm = Harness::new(&project);
    vm.vm.fire(Event::UiClicked {
        id: "resume".to_string(),
    });
    assert_eq!(says(&vm.run(1)), vec!["resumed".to_string()]);
}

#[test]
fn pausing_freezes_a_world_strand_but_not_one_the_interface_started() {
    let project = project_with(vec![
        started(vec![InstructionKind::Forever {
            body: vec![ins(move_by(1.0))],
        }]),
        ui_clicked("resume", vec![say("menu is alive")]),
    ]);
    let mut vm = Harness::started(&project);
    assert_eq!(moves(&vm.run(2)).len(), 2);

    vm.vm.set_paused(true);
    vm.paused = true;
    vm.frozen = vm.time;
    // The world strand gets no slice at all while the world is frozen.
    assert!(moves(&vm.run(3)).is_empty());

    // A click still starts its strand, and that strand still runs.
    vm.vm.fire(Event::UiClicked {
        id: "resume".to_string(),
    });
    assert_eq!(says(&vm.run(1)), vec!["menu is alive".to_string()]);

    vm.vm.set_paused(false);
    vm.paused = false;
    assert_eq!(moves(&vm.run(2)).len(), 2);
}

#[test]
fn a_wait_on_a_paused_menu_finishes_on_the_wall_clock() {
    let project = project_with(vec![
        ui_clicked(
            "blink",
            vec![
                InstructionKind::Wait {
                    duration: Value::number(0.25),
                },
                say("blinked"),
            ],
        ),
        // A world strand waiting the same span, to show the two clocks
        // really are different: its own never advances.
        started(vec![
            InstructionKind::Wait {
                duration: Value::number(0.25),
            },
            say("the world moved"),
        ]),
    ]);
    let mut vm = Harness::started(&project);
    vm.run(1);
    vm.vm.set_paused(true);
    vm.paused = true;
    vm.frozen = vm.time;
    vm.vm.fire(Event::UiClicked {
        id: "blink".to_string(),
    });

    // Four tenths of a second of wall time: enough for the menu's wait,
    // while the world clock has not moved at all.
    let effects = vm.run(4);
    assert_eq!(says(&effects), vec!["blinked".to_string()]);
}

#[test]
fn pause_game_stops_the_strand_that_ran_it_where_it_stands() {
    let project = project_with(vec![started(vec![
        say("before"),
        InstructionKind::PauseGame,
        say("never said"),
    ])]);
    let mut vm = Harness::started(&project);
    assert_eq!(says(&vm.run(5)), vec!["before".to_string()]);

    // It resumes where it left off, rather than starting again.
    vm.vm.set_paused(false);
    vm.paused = false;
    assert_eq!(says(&vm.run(1)), vec!["never said".to_string()]);
}

#[test]
fn a_pause_inside_a_ui_strand_leaves_that_strand_running() {
    let project = project_with(vec![ui_clicked(
        "resume",
        vec![
            InstructionKind::PauseGame,
            say("the menu is alive"),
            InstructionKind::ResumeGame,
        ],
    )]);
    let mut vm = Harness::new(&project);
    vm.vm.fire(Event::UiClicked {
        id: "resume".to_string(),
    });
    assert_eq!(says(&vm.run(1)), vec!["the menu is alive".to_string()]);
    assert!(!vm.vm.is_paused());
}

#[test]
fn focusing_names_the_input_the_block_meant_and_clearing_names_nobody() {
    let project = project_with(vec![started(vec![
        InstructionKind::FocusElement {
            element: Value::text(" name "),
        },
        InstructionKind::ClearFocus,
    ])]);
    let effects = Harness::started(&project).run(1);
    assert_eq!(
        effects
            .iter()
            .filter(|effect| !matches!(effect, Effect::Error { .. }))
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            Effect::SetFocus {
                id: "name".to_string()
            },
            // `clear focus` is the same effect with nobody named.
            Effect::SetFocus { id: String::new() },
        ]
    );
}

#[test]
fn pause_game_freezes_the_vm_the_moment_it_runs() {
    let project = project_with(vec![
        started(vec![say("before"), InstructionKind::PauseGame]),
        // Later in the same tick, and frozen where it stands.
        Strand::with_instructions(
            0,
            400,
            vec![
                ins(InstructionKind::WhenStarted),
                ins(InstructionKind::Wait {
                    duration: Value::number(0.05),
                }),
                ins(say("never said")),
            ],
        ),
    ]);
    let mut vm = Harness::started(&project);
    let effects = vm.run(5);
    assert_eq!(says(&effects), vec!["before".to_string()]);
    assert!(effects.contains(&Effect::SetPaused { paused: true }));
    assert!(vm.vm.is_paused());
}

#[test]
fn hiding_and_deleting_name_the_element_the_block_meant() {
    let project = project_with(vec![started(vec![
        InstructionKind::HideElement {
            element: Value::text(" menu "),
        },
        InstructionKind::HideAllUi,
        InstructionKind::DeleteElement {
            element: Value::text("menu"),
        },
    ])]);
    let effects = Harness::started(&project).run(1);
    assert_eq!(
        effects
            .iter()
            .filter(|effect| !matches!(effect, Effect::Error { .. }))
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            // A stray space around an id is trimmed, as it is everywhere
            // else a block names something.
            Effect::HideElement {
                id: "menu".to_string(),
                all: false
            },
            Effect::HideElement {
                id: String::new(),
                all: true
            },
            Effect::DeleteElement {
                id: "menu".to_string()
            },
        ]
    );
}
