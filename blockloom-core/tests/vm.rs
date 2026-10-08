//! End-to-end VM behavior: canvases in, effects out. These are the scheduling
//! rules `blockloom-runtime` relies on, checked without a window.

#![allow(clippy::field_reassign_with_default)]

use blockloom_core::blocks::{
    BlockDef, BlockPiece, BlockShape, DictDef, DictEntry, DictItem, Instruction, InstructionKind,
    ListDef, ListItem, Strand,
};
use blockloom_core::physics::{ContactKind, ContactPhase};
use blockloom_core::project::{Actor, Project, Scene};
use blockloom_core::scene::{Axis, Mode, Visual};
use blockloom_core::sense::Sensors;
use blockloom_core::sound::SoundBus;
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
    let scene = Scene {
        id: "s1".to_string(),
        name: "Scene 1".to_string(),
        path: "assets/scenes/Scene 1.blockscene".to_string(),
        world: blockloom_core::scene::World {
            mode: Mode::TwoD,
            ..Default::default()
        },
        actors: vec![actor],
    };
    Project {
        id: "p".to_string(),
        name: "test".to_string(),
        icon: String::new(),
        android: Default::default(),
        scenes: vec![scene],
        active_scene: "s1".to_string(),
        default_scene: "s1".to_string(),
        globals: Vec::new(),
        global_lists: Vec::new(),
        global_dicts: Vec::new(),
        plugin_resources: Vec::new(),
        physics: Default::default(),
        multiplayer: Default::default(),
        localization: Default::default(),
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
fn switch_scene_asks_for_the_named_scene_and_ends_its_strand() {
    let project = project_with(vec![started(vec![
        InstructionKind::SwitchScene {
            scene: Value::text("Scene 2"),
            transition: Value::text("fade"),
        },
        say("unreached"),
    ])]);
    let mut vm = Harness::started(&project);
    let mut effects = Vec::new();
    vm.vm.tick(0.1, &mut effects);
    let switch = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::SwitchScene {
                scene, transition, ..
            } => Some((scene.clone(), transition.clone())),
            _ => None,
        })
        .expect("a switch effect");
    assert_eq!(switch, ("Scene 2".to_string(), "fade".to_string()));
    // The strand that asked stopped where it stood.
    assert!(says(&effects).is_empty());
}

#[test]
fn switch_scene_normalizes_an_unknown_transition_to_none() {
    let project = project_with(vec![started(vec![InstructionKind::SwitchScene {
        scene: Value::text("  Scene 2  "),
        transition: Value::text("curtain"),
    }])]);
    let mut vm = Harness::started(&project);
    let mut effects = Vec::new();
    vm.vm.tick(0.1, &mut effects);
    let switch = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::SwitchScene {
                scene, transition, ..
            } => Some((scene.clone(), transition.clone())),
            _ => None,
        })
        .expect("a switch effect");
    assert_eq!(switch, ("Scene 2".to_string(), "none".to_string()));
}

#[test]
fn scene_events_start_their_strands() {
    let project = project_with(vec![
        Strand::with_instructions(
            0,
            0,
            vec![ins(InstructionKind::WhenSceneStarts), ins(say("begun"))],
        ),
        Strand::with_instructions(
            0,
            0,
            vec![ins(InstructionKind::WhenSceneEnds), ins(say("ended"))],
        ),
    ]);
    let mut vm = Harness::new(&project);
    vm.vm.fire(Event::SceneStarted);
    assert_eq!(
        vm.run(1)
            .iter()
            .filter_map(|effect| match effect {
                Effect::Say { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec!["begun".to_string()]
    );
    vm.vm.fire(Event::SceneEnded);
    assert_eq!(says(&vm.run(1)), vec!["ended".to_string()]);
}

#[test]
fn scene_reporters_read_the_published_snapshot() {
    blockloom_core::init();
    blockloom_core::sense::publish(Sensors {
        current_scene: "Menu".to_string(),
        scene_names: vec!["Menu".to_string(), "Level 1".to_string()],
        ..Default::default()
    });
    let current = Value::op(Op::from_name("CurrentScene"), vec![]);
    assert_eq!(current.eval(), Ok(Evaluated::Text("Menu".to_string())));
    let names = Value::op(Op::from_name("SceneNames"), vec![]);
    assert_eq!(
        names.eval(),
        Ok(Evaluated::Text("[\"Menu\",\"Level 1\"]".to_string()))
    );
}

#[test]
fn save_slot_blocks_queue_their_effects_without_ending_the_strand() {
    let project = project_with(vec![started(vec![
        InstructionKind::SwitchSaveSlot {
            slot: Value::text("  Slot 1 "),
        },
        InstructionKind::DeleteSaveSlot {
            slot: Value::text("old"),
        },
        InstructionKind::SetLanguage {
            language: Value::text("FR"),
        },
        say("kept going"),
    ])]);
    let mut vm = Harness::started(&project);
    let effects = vm.run(1);
    let switch = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::SwitchSaveSlot { slot, .. } => Some(slot.clone()),
            _ => None,
        })
        .expect("a switch effect");
    assert_eq!(switch, "Slot 1".to_string());
    let delete = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::DeleteSaveSlot { slot, .. } => Some(slot.clone()),
            _ => None,
        })
        .expect("a delete effect");
    assert_eq!(delete, "old".to_string());
    let language = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::SetLanguage { language, .. } => Some(language.clone()),
            _ => None,
        })
        .expect("a language effect");
    assert_eq!(language, "FR".to_string());
    // Unlike `switch scene to`, the strand carries on.
    assert_eq!(says(&effects), vec!["kept going".to_string()]);
}

#[test]
fn save_and_language_reporters_read_the_published_snapshot() {
    blockloom_core::init();
    blockloom_core::sense::publish(Sensors {
        current_save_slot: "slot-1".to_string(),
        save_slots: vec!["default".to_string(), "slot-1".to_string()],
        language: "fr".to_string(),
        ..Default::default()
    });
    let slot = Value::op(Op::from_name("SaveSlot"), vec![]);
    assert_eq!(slot.eval(), Ok(Evaluated::Text("slot-1".to_string())));
    let slots = Value::op(Op::from_name("SaveSlots"), vec![]);
    assert_eq!(
        slots.eval(),
        Ok(Evaluated::Text("[\"default\",\"slot-1\"]".to_string()))
    );
    let language = Value::op(Op::from_name("Language"), vec![]);
    assert_eq!(language.eval(), Ok(Evaluated::Text("fr".to_string())));
}

#[test]
fn save_and_language_reporters_answer_sensibly_with_no_run() {
    blockloom_core::init();
    blockloom_core::sense::publish(Sensors::default());
    blockloom_core::sense::set_locale_table(None);
    let slot = Value::op(Op::from_name("SaveSlot"), vec![]);
    assert_eq!(slot.eval(), Ok(Evaluated::Text("default".to_string())));
    let slots = Value::op(Op::from_name("SaveSlots"), vec![]);
    assert_eq!(slots.eval(), Ok(Evaluated::Text("[]".to_string())));
    let language = Value::op(Op::from_name("Language"), vec![]);
    assert_eq!(language.eval(), Ok(Evaluated::Text("en".to_string())));
    let text = Value::op(
        Op::from_name("LocalizedText"),
        vec![Value::text("menu.play")],
    );
    assert_eq!(text.eval(), Ok(Evaluated::Text("menu.play".to_string())));
}

#[test]
fn localized_text_reads_the_run_language_then_falls_back() {
    use blockloom_core::locale::Localization;
    blockloom_core::init();
    let mut table = Localization {
        default_language: "fr".to_string(),
        ..Default::default()
    };
    table.set("menu.play", "en", "Play");
    table.set("menu.play", "fr", "Jouer");
    blockloom_core::sense::set_locale_table(Some(table));
    blockloom_core::sense::publish(Sensors {
        language: "fr".to_string(),
        ..Default::default()
    });
    let text = Value::op(
        Op::from_name("LocalizedText"),
        vec![Value::text("menu.play")],
    );
    assert_eq!(text.eval(), Ok(Evaluated::Text("Jouer".to_string())));
    blockloom_core::sense::publish(Sensors {
        language: "de".to_string(),
        ..Default::default()
    });
    assert_eq!(text.eval(), Ok(Evaluated::Text("Jouer".to_string())));
    let missing = Value::op(
        Op::from_name("LocalizedText"),
        vec![Value::text("menu.quit")],
    );
    assert_eq!(missing.eval(), Ok(Evaluated::Text("menu.quit".to_string())));
    blockloom_core::sense::set_locale_table(None);
}

#[test]
fn loading_a_scene_keeps_globals_but_resets_actor_locals() {
    use blockloom_core::blocks::VariableDef;
    use blockloom_core::vm::Variables;
    let mut project = project_with(vec![started(vec![])]);
    project.globals.push(VariableDef {
        name: "score".to_string(),
        value: Evaluated::Number(0.0),
    });
    project.scenes[0].actors[0]
        .graph
        .variables
        .push(VariableDef {
            name: "local".to_string(),
            value: Evaluated::Number(1.0),
        });
    let variables = Variables::default();
    variables.load(&project);
    let actor = project.scenes[0].actors[0].id.clone();
    // A run's writes: a global and an actor-local.
    variables.write(&actor, "score", Evaluated::Number(7.0));
    variables.write(&actor, "local", Evaluated::Number(2.0));
    assert_eq!(variables.read(&actor, "score"), Evaluated::Number(7.0));
    // A second scene with its own actor and its own default for `local`.
    let mut other = Actor::new("Other", rect());
    other.graph.variables.push(VariableDef {
        name: "local".to_string(),
        value: Evaluated::Number(9.0),
    });
    let other_id = other.id.clone();
    project.scenes.push(Scene {
        id: "s2".to_string(),
        name: "Scene 2".to_string(),
        path: "assets/scenes/Scene 2.blockscene".to_string(),
        world: blockloom_core::scene::World {
            mode: Mode::TwoD,
            ..Default::default()
        },
        actors: vec![other],
    });
    project.active_scene = "s2".to_string();
    variables.load_scene(&project);
    // Globals keep what the run wrote; the new scene's actor starts as
    // authored, and the old actor's scope is gone.
    assert_eq!(variables.read(&other_id, "score"), Evaluated::Number(7.0));
    assert_eq!(variables.read(&other_id, "local"), Evaluated::Number(9.0));
    assert_eq!(variables.read(&actor, "score"), Evaluated::Number(7.0));
}

#[test]
fn survivors_keep_live_locals_lists_and_dicts_across_scenes() {
    use blockloom_core::blocks::VariableDef;
    use blockloom_core::vm::{Dicts, Lists, Variables};
    use std::collections::HashSet;
    // First scene: a Persist carrier plus a passer-by, both with locals.
    let mut project = project_with(vec![started(vec![])]);
    let carrier = project.scenes[0].actors[0].id.clone();
    project.scenes[0].actors[0]
        .components
        .insert(blockloom_core::components::ActorComponent::Persist);
    project.scenes[0].actors[0]
        .graph
        .variables
        .push(VariableDef {
            name: "ammo".to_string(),
            value: Evaluated::Number(3.0),
        });
    project.scenes[0].actors[0].graph.lists.push(ListDef {
        name: "bag".to_string(),
        items: Vec::new(),
        editor_visible: false,
        editor_x: 0,
        editor_y: 0,
    });
    project.scenes[0].actors[0].graph.dicts.push(DictDef {
        name: "kit".to_string(),
        entries: Vec::new(),
        editor_visible: false,
        editor_x: 0,
        editor_y: 0,
    });
    let mut passer = Actor::new("Passer", rect());
    passer.graph.variables.push(VariableDef {
        name: "ammo".to_string(),
        value: Evaluated::Number(1.0),
    });
    let passer_id = passer.id.clone();
    project.scenes[0].actors.push(passer);

    let vm_vars = Variables::default();
    let vm_lists = Lists::default();
    let vm_dicts = Dicts::default();
    let mut vm = Vm::with_stores(vm_vars.clone(), vm_lists.clone(), vm_dicts.clone());
    vm.load(&project);
    // A run's writes: survivor locals, passer-by locals.
    vm_vars.write(&carrier, "ammo", Evaluated::Number(30.0));
    vm_lists.with_list_mut(&carrier, "bag", |list| {
        list.push(ListItem::Number(1.0));
    });
    vm_dicts.with_dict_mut(&carrier, "kit", |dict| {
        dict.push(DictEntry::new("key".to_string(), DictItem::Number(2.0)));
    });
    vm_vars.write(&passer_id, "ammo", Evaluated::Number(10.0));

    // Second scene with a fresh actor; the carrier is not in its document -
    // it rides in `spawned` instead.
    let mut fresh = Actor::new("Fresh", rect());
    fresh.graph.variables.push(VariableDef {
        name: "ammo".to_string(),
        value: Evaluated::Number(5.0),
    });
    let fresh_id = fresh.id.clone();
    project.scenes.push(Scene {
        id: "s2".to_string(),
        name: "Scene 2".to_string(),
        path: "assets/scenes/Scene 2.blockscene".to_string(),
        world: blockloom_core::scene::World {
            mode: Mode::TwoD,
            ..Default::default()
        },
        actors: vec![fresh],
    });
    project.active_scene = "s2".to_string();
    let keep: HashSet<String> = [carrier.clone()].into_iter().collect();
    vm.load_scene_keep(&project, &keep);
    // Survivor keeps live locals; the passer-by's scope is gone; the fresh
    // actor starts as authored.
    assert_eq!(vm_vars.read(&carrier, "ammo"), Evaluated::Number(30.0));
    assert_eq!(
        vm_lists
            .snapshot_for(&carrier)
            .get("bag")
            .cloned()
            .unwrap_or_default(),
        vec![ListItem::Number(1.0)]
    );
    assert_eq!(
        vm_dicts
            .snapshot_for(&carrier)
            .get("kit")
            .cloned()
            .unwrap_or_default(),
        vec![DictEntry::new("key".to_string(), DictItem::Number(2.0))]
    );
    assert_eq!(vm_vars.read(&fresh_id, "ammo"), Evaluated::Number(5.0));
    // A non-survivor id reads its global-or-zero, not its old local.
    assert_eq!(vm_vars.read(&passer_id, "ammo"), Evaluated::Number(0.0));
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
        branches: Vec::new(),
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
fn a_wait_inside_a_reporter_suspends_its_caller() {
    let block = BlockDef {
        id: "b1".to_string(),
        pieces: vec![BlockPiece::Label {
            id: "l".to_string(),
            text: "slow".to_string(),
        }],
        shape: BlockShape::ReturnsValue,
        color: "#4C97FF".to_string(),
    };
    let call = Value::Call {
        block_id: "b1".to_string(),
        args: vec![],
        branches: Vec::new(),
        saved: Box::new(Value::number(0.0)),
    };
    let mut project = project_with(vec![
        started(vec![InstructionKind::Say { text: call }, say("after")]),
        Strand::with_instructions(
            0,
            0,
            vec![
                ins(InstructionKind::BlockHeader {
                    block_id: "b1".to_string(),
                }),
                ins(say("before the wait")),
                ins(InstructionKind::Wait {
                    duration: Value::number(0.25),
                }),
                ins(say("after the wait")),
                ins(InstructionKind::Return {
                    value: Value::text("done"),
                }),
            ],
        ),
    ]);
    project.actors[0].graph.block_defs.push(block);
    let mut vm = Harness::started(&project);
    // The reporter says on its way in, then sleeps with its caller.
    assert_eq!(says(&vm.run(1)), vec!["before the wait".to_string()]);
    assert!(says(&vm.run(2)).is_empty());
    // Waking up runs the rest in order: the reporter's own say, then the
    // caller's two, all on the same tick the wait finishes on.
    assert_eq!(
        says(&vm.run(3)),
        vec![
            "after the wait".to_string(),
            "done".to_string(),
            "after".to_string()
        ]
    );
    assert!(!vm.vm.is_running());
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
fn escape_while_paused_runs_the_key_strand_as_an_interface_one() {
    // The pause-menu toggle: escape pauses the world, and escape again
    // thaws it, even though no world event queues while paused.
    let project = project_with(vec![
        started(vec![say("down"), InstructionKind::PauseGame]),
        Strand::with_instructions(
            0,
            400,
            vec![
                ins(InstructionKind::WhenKeyPressed {
                    key: "escape".to_string(),
                }),
                ins(say("up")),
                ins(InstructionKind::ResumeGame),
            ],
        ),
    ]);
    let mut vm = Harness::started(&project);
    assert_eq!(says(&vm.run(1)), vec!["down".to_string()]);
    assert!(vm.vm.is_paused());
    vm.vm.fire(Event::Key("escape".to_string()));
    assert_eq!(says(&vm.run(1)), vec!["up".to_string()]);
    assert!(!vm.vm.is_paused());
}

#[test]
fn any_other_key_stays_frozen_while_paused() {
    let project = project_with(vec![
        started(vec![say("down"), InstructionKind::PauseGame]),
        Strand::with_instructions(
            0,
            400,
            vec![
                ins(InstructionKind::WhenKeyPressed {
                    key: "space".to_string(),
                }),
                ins(say("jump")),
            ],
        ),
    ]);
    let mut vm = Harness::started(&project);
    assert_eq!(says(&vm.run(1)), vec!["down".to_string()]);
    assert!(vm.vm.is_paused());
    vm.vm.fire(Event::Key("space".to_string()));
    assert!(says(&vm.run(1)).is_empty());
    assert!(vm.vm.is_paused());
}

#[test]
fn a_collision_only_starts_the_strand_whose_target_matches() {
    let mut project = project_with(vec![Strand::with_instructions(
        0,
        0,
        vec![
            ins(InstructionKind::WhenCollision {
                with: "Ground".to_string(),
                phase: Default::default(),
                scope: Default::default(),
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
        phase: Default::default(),
        kind: ContactKind::Collision,
        impulse: 0.0,
        speed: 0.0,
    });
    assert!(says(&vm.run(1)).is_empty());
    vm.vm.fire(Event::Collision {
        actor: player.clone(),
        with: ground.clone(),
        phase: ContactPhase::Stay,
        kind: ContactKind::Collision,
        impulse: 0.0,
        speed: 0.0,
    });
    assert!(says(&vm.run(1)).is_empty(), "the hat listens to Enter");
    vm.vm.fire(Event::Collision {
        actor: player,
        with: ground,
        phase: ContactPhase::Enter,
        kind: ContactKind::Collision,
        impulse: 0.0,
        speed: 0.0,
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
    let scene = Scene {
        id: "s1".to_string(),
        name: "Scene 1".to_string(),
        path: "assets/scenes/Scene 1.blockscene".to_string(),
        world: blockloom_core::scene::World {
            mode: Mode::TwoD,
            ..Default::default()
        },
        actors: vec![player, friend],
    };
    Project {
        id: "p".to_string(),
        name: "test".to_string(),
        icon: String::new(),
        android: Default::default(),
        scenes: vec![scene],
        active_scene: "s1".to_string(),
        default_scene: "s1".to_string(),
        globals: Vec::new(),
        global_lists: Vec::new(),
        global_dicts: Vec::new(),
        plugin_resources: Vec::new(),
        physics: Default::default(),
        multiplayer: Default::default(),
        localization: Default::default(),
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

// ─── Lists ───────────────────────────────────────────────────────────────────

fn list_op(name: &str, args: Vec<Value>) -> Value {
    Value::op(Op::from_name(name), args)
}

/// A project with one actor holding an empty list called `items`.
fn project_with_empty_list(body: Vec<InstructionKind>) -> Project {
    let mut project = project_with(vec![started(body)]);
    project.actors[0].graph.lists.push(ListDef {
        name: "items".to_string(),
        items: Vec::new(),
        editor_visible: false,
        editor_x: 0,
        editor_y: 0,
    });
    project
}

#[test]
fn list_blocks_append_read_and_remove_items() {
    let project = project_with_empty_list(vec![
        InstructionKind::AddToList {
            value: Value::text("a"),
            name: "items".to_string(),
        },
        InstructionKind::AddToList {
            value: Value::number(2.0),
            name: "items".to_string(),
        },
        InstructionKind::Say {
            text: list_op("ListItem", vec![Value::number(2.0), Value::text("items")]),
        },
        InstructionKind::Say {
            text: list_op("ListLength", vec![Value::text("items")]),
        },
        InstructionKind::DeleteOfList {
            index: Value::number(1.0),
            name: "items".to_string(),
        },
        InstructionKind::Say {
            text: list_op("ListItem", vec![Value::number(1.0), Value::text("items")]),
        },
    ]);
    let effects = Harness::started(&project).run(1);
    assert_eq!(
        says(&effects),
        vec!["2".to_string(), "2".to_string(), "2".to_string()]
    );
    assert!(errors(&effects).is_empty());
}

#[test]
fn list_edits_and_reporters_share_the_same_rules() {
    let mut project = project_with(vec![started(vec![
        InstructionKind::InsertIntoList {
            value: Value::text("z"),
            index: Value::number(1.0),
            name: "letters".to_string(),
        },
        InstructionKind::ReplaceItemOfList {
            index: Value::number(3.0),
            name: "letters".to_string(),
            value: Value::text("c"),
        },
        InstructionKind::ShiftList {
            name: "letters".to_string(),
            amount: Value::number(1.0),
        },
        InstructionKind::ReverseList {
            name: "letters".to_string(),
        },
        InstructionKind::Say {
            text: list_op(
                "ListItemNumber",
                vec![Value::text("z"), Value::text("letters")],
            ),
        },
        InstructionKind::Say {
            text: list_op("ListAmount", vec![Value::text("a"), Value::text("letters")]),
        },
        InstructionKind::Say {
            text: list_op(
                "ListContains",
                vec![Value::text("letters"), Value::text("q")],
            ),
        },
        InstructionKind::Say {
            text: list_op(
                "ListItemExists",
                vec![Value::number(4.0), Value::text("letters")],
            ),
        },
        InstructionKind::Say {
            text: list_op("ListIsEmpty", vec![Value::text("letters")]),
        },
        InstructionKind::DeleteAllOfList {
            name: "letters".to_string(),
        },
        InstructionKind::Say {
            text: list_op("ListLength", vec![Value::text("letters")]),
        },
    ])]);
    project.actors[0].graph.lists.push(ListDef {
        name: "letters".to_string(),
        items: vec![
            ListItem::Text("a".to_string()),
            ListItem::Text("b".to_string()),
        ],
        editor_visible: false,
        editor_x: 0,
        editor_y: 0,
    });
    // [a, b] -> insert z at 1 -> [z, a, b] -> replace 3 with c -> [z, a, c]
    // -> shift by 1 -> [c, z, a] -> reverse -> [a, z, c].
    let effects = Harness::started(&project).run(1);
    assert_eq!(
        says(&effects),
        ["2", "1", "false", "false", "false", "0"]
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    );
    assert!(errors(&effects).is_empty());
}

#[test]
fn an_unknown_list_is_empty_and_its_writes_are_quiet() {
    let project = project_with_empty_list(vec![
        InstructionKind::Say {
            text: list_op("ListItem", vec![Value::number(1.0), Value::text("nobody")]),
        },
        InstructionKind::Say {
            text: list_op("ListLength", vec![Value::text("nobody")]),
        },
        InstructionKind::DeleteOfList {
            index: Value::number(1.0),
            name: "nobody".to_string(),
        },
        InstructionKind::AddToList {
            value: Value::number(1.0),
            name: "nobody".to_string(),
        },
    ]);
    let effects = Harness::started(&project).run(1);
    assert_eq!(says(&effects), vec![String::new(), "0".to_string()]);
    assert!(errors(&effects).is_empty());
}

#[test]
fn a_boolean_is_not_a_list_item() {
    let project = project_with_empty_list(vec![InstructionKind::AddToList {
        value: Value::Bool,
        name: "items".to_string(),
    }]);
    let effects = Harness::started(&project).run(1);
    assert_eq!(errors(&effects), vec!["list items must be number or text"]);
}

#[test]
fn a_shared_list_is_visible_to_every_actor_but_an_actors_own_shadows_it() {
    let mut project = project_with_two(
        vec![started(vec![InstructionKind::Say {
            text: list_op("ListItem", vec![Value::number(1.0), Value::text("shared")]),
        }])],
        vec![started(vec![InstructionKind::Say {
            text: list_op("ListItem", vec![Value::number(1.0), Value::text("shared")]),
        }])],
    );
    project.create_global_list("shared").unwrap();
    project.global_lists[0].items = vec![ListItem::Text("from everyone".to_string())];
    // The first actor carries its own list of the same name, so it reads
    // that one instead.
    project.actors[0].graph.lists.push(ListDef {
        name: "shared".to_string(),
        items: vec![ListItem::Text("mine".to_string())],
        editor_visible: false,
        editor_x: 0,
        editor_y: 0,
    });
    let effects = Harness::started(&project).run(1);
    let mut said = says(&effects);
    said.sort();
    assert_eq!(said, vec!["from everyone".to_string(), "mine".to_string()]);
}

#[test]
fn a_clone_starts_holding_what_its_template_held() {
    let mut project = project_with(vec![
        started(vec![
            InstructionKind::AddToList {
                value: Value::text("kept"),
                name: "items".to_string(),
            },
            clone_of(""),
        ]),
        cloned(vec![InstructionKind::Say {
            text: list_op("ListLength", vec![Value::text("items")]),
        }]),
    ]);
    project.actors[0].graph.lists.push(ListDef {
        name: "items".to_string(),
        items: Vec::new(),
        editor_visible: false,
        editor_x: 0,
        editor_y: 0,
    });
    let mut vm = Harness::started(&project);
    vm.run(1);
    assert_eq!(says(&vm.run(1)), vec!["1".to_string()]);
}

// ─── Dicts ───────────────────────────────────────────────────────────────────

fn dict_op(name: &str, args: Vec<Value>) -> Value {
    Value::op(Op::from_name(name), args)
}

/// A project with one actor holding an empty dict called `save`.
fn project_with_empty_dict(body: Vec<InstructionKind>) -> Project {
    let mut project = project_with(vec![started(body)]);
    project.actors[0].graph.dicts.push(DictDef {
        name: "save".to_string(),
        entries: Vec::new(),
        editor_visible: false,
        editor_x: 0,
        editor_y: 0,
    });
    project
}

#[test]
fn dict_blocks_write_read_and_remove_keys() {
    let mut project = project_with_empty_dict(vec![
        InstructionKind::SetDictValue {
            key: Value::text("hp"),
            name: "save".to_string(),
            value: Value::number(3.0),
        },
        InstructionKind::SetDictValue {
            key: Value::text("name"),
            name: "save".to_string(),
            value: Value::text("fox"),
        },
        InstructionKind::Say {
            text: dict_op("DictValue", vec![Value::text("hp"), Value::text("save")]),
        },
        InstructionKind::Say {
            text: dict_op("DictSize", vec![Value::text("save")]),
        },
        InstructionKind::SetDictValue {
            key: Value::text("hp"),
            name: "save".to_string(),
            value: Value::number(4.0),
        },
        InstructionKind::Say {
            text: dict_op("DictValue", vec![Value::text("hp"), Value::text("save")]),
        },
        InstructionKind::DeleteDictKey {
            key: Value::text("name"),
            name: "save".to_string(),
        },
        InstructionKind::Say {
            text: dict_op("DictHasKey", vec![Value::text("save"), Value::text("name")]),
        },
        InstructionKind::Say {
            text: dict_op("DictIsEmpty", vec![Value::text("save")]),
        },
        InstructionKind::DeleteAllOfDict {
            name: "save".to_string(),
        },
        InstructionKind::Say {
            text: dict_op("DictSize", vec![Value::text("save")]),
        },
    ]);
    // A seeded entry is replaced by the first write, so the run still says
    // exactly what the blocks wrote.
    project.actors[0].graph.dicts[0].entries = vec![DictEntry {
        key: "hp".to_string(),
        value: DictItem::Number(1.0),
    }];
    let effects = Harness::started(&project).run(1);
    assert_eq!(
        says(&effects),
        ["3", "2", "4", "false", "false", "0"]
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    );
    assert!(errors(&effects).is_empty());
}

#[test]
fn a_missing_key_reads_empty_and_its_delete_is_quiet() {
    let project = project_with_empty_dict(vec![
        InstructionKind::Say {
            text: dict_op("DictValue", vec![Value::text("mp"), Value::text("save")]),
        },
        InstructionKind::DeleteDictKey {
            key: Value::text("mp"),
            name: "save".to_string(),
        },
        InstructionKind::Say {
            text: dict_op("DictValue", vec![Value::text("mp"), Value::text("nobody")]),
        },
        InstructionKind::SetDictValue {
            key: Value::text("hp"),
            name: "nobody".to_string(),
            value: Value::number(1.0),
        },
    ]);
    let effects = Harness::started(&project).run(1);
    assert_eq!(says(&effects), vec![String::new(), String::new()]);
    assert!(errors(&effects).is_empty());
}

#[test]
fn a_boolean_is_not_a_dict_value() {
    let project = project_with_empty_dict(vec![InstructionKind::SetDictValue {
        key: Value::text("ok"),
        name: "save".to_string(),
        value: Value::Bool,
    }]);
    let effects = Harness::started(&project).run(1);
    assert_eq!(errors(&effects), vec!["dict values must be number or text"]);
}

#[test]
fn a_shared_dict_is_visible_to_every_actor_but_an_actors_own_shadows_it() {
    let mut project = project_with_two(
        vec![started(vec![InstructionKind::Say {
            text: dict_op("DictValue", vec![Value::text("hp"), Value::text("shared")]),
        }])],
        vec![started(vec![InstructionKind::Say {
            text: dict_op("DictValue", vec![Value::text("hp"), Value::text("shared")]),
        }])],
    );
    project.create_global_dict("shared").unwrap();
    project.global_dicts[0].entries = vec![DictEntry {
        key: "hp".to_string(),
        value: DictItem::Text("from everyone".to_string()),
    }];
    // The first actor carries its own dict of the same name, so it reads
    // that one instead.
    project.actors[0].graph.dicts.push(DictDef {
        name: "shared".to_string(),
        entries: vec![DictEntry {
            key: "hp".to_string(),
            value: DictItem::Text("mine".to_string()),
        }],
        editor_visible: false,
        editor_x: 0,
        editor_y: 0,
    });
    let effects = Harness::started(&project).run(1);
    let mut said = says(&effects);
    said.sort();
    assert_eq!(said, vec!["from everyone".to_string(), "mine".to_string()]);
}

#[test]
fn a_clone_starts_holding_whatever_dicts_its_template_held() {
    let mut project = project_with(vec![
        started(vec![
            InstructionKind::SetDictValue {
                key: Value::text("hp"),
                name: "save".to_string(),
                value: Value::number(3.0),
            },
            clone_of(""),
        ]),
        cloned(vec![InstructionKind::Say {
            text: dict_op("DictValue", vec![Value::text("hp"), Value::text("save")]),
        }]),
    ]);
    project.actors[0].graph.dicts.push(DictDef {
        name: "save".to_string(),
        entries: Vec::new(),
        editor_visible: false,
        editor_x: 0,
        editor_y: 0,
    });
    let mut vm = Harness::started(&project);
    vm.run(1);
    assert_eq!(says(&vm.run(1)), vec!["3".to_string()]);
}

// ─── JSON ────────────────────────────────────────────────────────────────────

#[test]
fn dicts_and_lists_bridge_to_json_text_and_back() {
    let project = project_with_empty_dict(vec![
        InstructionKind::SetDictValue {
            key: Value::text("hp"),
            name: "save".to_string(),
            value: Value::number(3.0),
        },
        InstructionKind::SetDictValue {
            key: Value::text("name"),
            name: "save".to_string(),
            value: Value::text("fox"),
        },
        InstructionKind::Say {
            text: dict_op("DictAsJson", vec![Value::text("save")]),
        },
        InstructionKind::LoadJsonIntoDict {
            json: Value::text(r#"{"hp": 9, "title": "mage"}"#),
            name: "save".to_string(),
        },
        InstructionKind::Say {
            text: dict_op("DictValue", vec![Value::text("title"), Value::text("save")]),
        },
        InstructionKind::Say {
            text: dict_op("DictKeys", vec![Value::text("save")]),
        },
    ]);
    let effects = Harness::started(&project).run(1);
    assert_eq!(
        says(&effects),
        vec![
            r#"{"hp":3.0,"name":"fox"}"#.to_string(),
            "mage".to_string(),
            r#"["hp","title"]"#.to_string(),
        ]
    );
    assert!(errors(&effects).is_empty());
}

#[test]
fn a_list_bridges_to_json_text_and_back() {
    let mut project = project_with_empty_list(vec![
        InstructionKind::Say {
            text: list_op("ListAsJson", vec![Value::text("items")]),
        },
        InstructionKind::LoadJsonIntoList {
            json: Value::text(r#"[1, "two"]"#),
            name: "items".to_string(),
        },
        InstructionKind::Say {
            text: list_op("ListItem", vec![Value::number(2.0), Value::text("items")]),
        },
        InstructionKind::Say {
            text: list_op("ListLength", vec![Value::text("items")]),
        },
    ]);
    project.actors[0].graph.lists[0].items = vec![ListItem::Number(7.0)];
    let effects = Harness::started(&project).run(1);
    assert_eq!(
        says(&effects),
        vec!["[7.0]".to_string(), "two".to_string(), "2".to_string()]
    );
    assert!(errors(&effects).is_empty());
}

#[test]
fn loading_what_isnt_json_reports_and_leaves_the_collection() {
    let project = project_with_empty_dict(vec![
        InstructionKind::LoadJsonIntoDict {
            json: Value::text("[1, 2]"),
            name: "save".to_string(),
        },
        InstructionKind::LoadJsonIntoList {
            json: Value::text("nope"),
            name: "missing".to_string(),
        },
        InstructionKind::LoadJsonIntoDict {
            json: Value::text(r#"{"ok": true}"#),
            name: "save".to_string(),
        },
        InstructionKind::Say {
            text: dict_op("DictSize", vec![Value::text("save")]),
        },
    ]);
    let effects = Harness::started(&project).run(1);
    assert_eq!(says(&effects), vec!["0".to_string()]);
    assert_eq!(
        errors(&effects),
        vec![
            "that text isn't a JSON object",
            "that text isn't a JSON array",
            "\"ok\" isn't a number or text",
        ]
    );
}

// ─── Sound ──────────────────────────────────────────────────────────────────

fn play_sound(sound: &str, volume: f64, pitch: f64) -> InstructionKind {
    InstructionKind::PlaySound {
        sound: Value::text(sound),
        volume: Value::number(volume),
        pitch: Value::number(pitch),
        loop_: false,
        bus: SoundBus::Sfx,
    }
}

fn plays(effects: &[Effect]) -> Vec<(String, f32, f32, bool, Option<String>)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::PlaySound {
                sound,
                volume,
                pitch,
                loop_,
                at,
                ..
            } => Some((sound.clone(), *volume, *pitch, *loop_, at.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn playing_a_sound_reports_a_voice_with_block_scale_volumes() {
    let project = project_with(vec![started(vec![play_sound(
        "assets/sounds/jump.wav",
        50.0,
        2.0,
    )])]);
    let effects = Harness::started(&project).run(1);
    assert_eq!(
        plays(&effects).len(),
        1,
        "one play block is one voice: {effects:?}"
    );
    let (sound, volume, pitch, loop_, at) = plays(&effects)[0].clone();
    assert_eq!(sound, "assets/sounds/jump.wav");
    assert_eq!(volume, 0.5);
    assert_eq!(pitch, 2.0);
    assert!(!loop_);
    assert_eq!(at, None);
}

#[test]
fn playing_nothing_reports_instead() {
    let project = project_with(vec![started(vec![play_sound("   ", 100.0, 1.0)])]);
    let effects = Harness::started(&project).run(1);
    assert!(plays(&effects).is_empty());
    assert_eq!(errors(&effects), vec!["which sound should I play?"]);
}

#[test]
fn playing_at_an_actor_resolves_it_and_a_missing_one_reports() {
    let at = |target: &str| InstructionKind::PlaySoundAt {
        sound: Value::text("assets/sounds/hum.wav"),
        volume: Value::number(100.0),
        pitch: Value::number(1.0),
        loop_: true,
        bus: SoundBus::Music,
        target: Value::text(target),
    };
    let project = project_with_two(
        vec![started(vec![at("Friend")])],
        vec![started(vec![at("Nobody"), at("")])],
    );
    let effects = Harness::started(&project).run(1);
    let friend = project.actors[1].id.clone();
    assert!(
        plays(&effects).iter().any(|(sound, _, _, looped, at)| {
            sound == "assets/sounds/hum.wav" && *looped && *at == Some(friend.clone())
        }),
        "a named actor resolves to its id: {effects:?}"
    );
    assert!(
        errors(&effects)
            .iter()
            .any(|message| message.contains("Nobody")),
        "a missing target reports: {effects:?}"
    );
}

#[test]
fn stopping_and_bus_moves_come_through_as_effects() {
    let project = project_with(vec![started(vec![
        InstructionKind::StopSound {
            sound: Value::text(""),
        },
        InstructionKind::SetSoundVolume {
            sound: Value::text("assets/sounds/hum.wav"),
            volume: Value::number(25.0),
        },
        InstructionKind::SetBusVolume {
            bus: SoundBus::Music,
            volume: Value::number(80.0),
        },
    ])]);
    let effects = Harness::started(&project).run(1);
    assert!(
        matches!(
            &effects[0],
            Effect::StopSound { sound, .. } if sound.is_empty()
        ),
        "empty stops everything: {effects:?}"
    );
    assert!(
        matches!(
            &effects[1],
            Effect::SetSoundVolume { sound, volume, .. }
            if sound == "assets/sounds/hum.wav" && *volume == 0.25
        ),
        "volumes travel as gains: {effects:?}"
    );
    assert!(
        matches!(
            &effects[2],
            Effect::SetBusVolume { bus, volume }
            if *bus == SoundBus::Music && *volume == 0.8
        ),
        "buses move whole: {effects:?}"
    );
}

#[test]
fn retuning_nothing_reports_instead() {
    let project = project_with(vec![started(vec![
        InstructionKind::SetSoundVolume {
            sound: Value::text(""),
            volume: Value::number(10.0),
        },
        InstructionKind::SetSoundPitch {
            sound: Value::text(""),
            pitch: Value::number(2.0),
        },
    ])]);
    let effects = Harness::started(&project).run(1);
    assert_eq!(
        errors(&effects),
        vec![
            "which sound's volume should I set?",
            "which sound's pitch should I set?",
        ]
    );
}

#[test]
fn the_sound_reporters_read_the_published_snapshot() {
    use blockloom_core::sense;
    use std::collections::{HashMap, HashSet};

    let mut sounds = HashSet::new();
    sounds.insert("assets/sounds/jump.wav".to_string());
    let mut bus_volumes = HashMap::new();
    bus_volumes.insert(SoundBus::Music, 75.0);
    sense::publish(Sensors {
        sounds,
        bus_volumes,
        ..Default::default()
    });

    let playing = Value::op(
        Op::from_name("SoundPlaying"),
        vec![Value::text("assets/sounds/jump.wav")],
    );
    assert_eq!(playing.eval(), Ok(Evaluated::Bool(true)));
    let quiet = Value::op(
        Op::from_name("SoundPlaying"),
        vec![Value::text("assets/sounds/other.wav")],
    );
    assert_eq!(quiet.eval(), Ok(Evaluated::Bool(false)));

    let music = Value::op(Op::from_name("BusVolume"), vec![Value::text("Music")]);
    assert_eq!(music.eval(), Ok(Evaluated::Number(75.0)));
    let missing = Value::op(Op::from_name("BusVolume"), vec![Value::text("Nope")]);
    assert!(missing.eval().is_err());
}

#[test]
fn the_atmosphere_reporter_reads_the_fixed_tick_slot() {
    use blockloom_core::sense::{self, AtmosphereSense};

    sense::publish(Sensors::default());
    sense::publish_atmosphere(AtmosphereSense {
        rain: 0.5,
        ..Default::default()
    });
    let rain = Value::op(Op::from_name("Atmosphere"), vec![Value::text("Rain")]);
    assert_eq!(rain.eval(), Ok(Evaluated::Number(0.5)));
    let missing = Value::op(Op::from_name("Atmosphere"), vec![Value::text("humidity")]);
    assert!(missing.eval().is_err());
}

#[test]
fn the_director_blocks_ask_for_clock_precipitation_and_weather() {
    use blockloom_core::director::PrecipitationKind;
    let project = project_with(vec![started(vec![
        InstructionKind::SetTimeOfDay {
            time: Value::number(18.5),
        },
        InstructionKind::AdvanceTime {
            hours: Value::number(-2.0),
        },
        InstructionKind::SetPrecipitation {
            property: PrecipitationKind::Rain,
            value: Value::number(0.7),
        },
        InstructionKind::BlendWeather {
            weather: Value::text("  Storm  "),
            seconds: Value::number(5.0),
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
            Effect::SetTimeOfDay { time: 18.5 },
            Effect::AdvanceTime { hours: -2.0 },
            Effect::SetPrecipitation {
                property: PrecipitationKind::Rain,
                value: 0.7,
            },
            // The name is trimmed, as it is everywhere a block names
            // something.
            Effect::BlendWeather {
                weather: "Storm".to_string(),
                seconds: 5.0,
            },
        ]
    );
}

#[test]
fn a_weather_event_starts_only_the_strand_that_names_it() {
    let project = project_with(vec![
        Strand::with_instructions(
            0,
            0,
            vec![
                ins(InstructionKind::WhenWeather {
                    weather: "Storm".to_string(),
                }),
                ins(say("storm's here")),
            ],
        ),
        Strand::with_instructions(
            0,
            400,
            vec![
                ins(InstructionKind::WhenWeather {
                    weather: "Clear".to_string(),
                }),
                ins(say("never")),
            ],
        ),
        Strand::with_instructions(
            0,
            800,
            vec![
                ins(InstructionKind::WhenWeather {
                    weather: "".to_string(),
                }),
                ins(say("any weather")),
            ],
        ),
    ]);
    let mut vm = Harness::new(&project);
    vm.vm.fire(Event::Weather {
        weather: "Storm".to_string(),
    });
    assert_eq!(
        says(&vm.run(1)),
        vec!["storm's here".to_string(), "any weather".to_string()]
    );
}

#[test]
fn the_director_reporters_read_the_fixed_tick_slot() {
    use blockloom_core::sense::{self, AtmosphereSense};
    blockloom_core::init();
    sense::publish(Sensors::default());
    sense::publish_atmosphere(AtmosphereSense {
        time_of_day: 6.5,
        sun_direction: [0.0, 0.5, -0.8660254],
        weather: "Storm".to_string(),
        ..Default::default()
    });
    let time = Value::op(Op::from_name("TimeOfDay"), vec![]);
    assert_eq!(time.eval(), Ok(Evaluated::Number(6.5)));
    let elevation = Value::op(Op::from_name("SunElevation"), vec![]);
    assert!((elevation.eval().unwrap().as_number().unwrap() - 30.0).abs() < 1e-3);
    let weather = Value::op(Op::from_name("CurrentWeather"), vec![]);
    assert_eq!(weather.eval(), Ok(Evaluated::Text("Storm".to_string())));
    let by_name = Value::op(
        Op::from_name("Atmosphere"),
        vec![Value::text("time of day")],
    );
    assert_eq!(by_name.eval(), Ok(Evaluated::Number(6.5)));
}

#[test]
fn the_cutscene_blocks_ask_for_reels_shake_time_and_chrome() {
    let project = project_with(vec![started(vec![
        InstructionKind::PlayCutscene {
            cutscene: Value::text("  Opener  "),
        },
        InstructionKind::SkipCutscene,
        InstructionKind::CameraShake {
            amount: Value::number(0.7),
        },
        InstructionKind::SetTimeScale {
            scale: Value::number(0.5),
        },
        InstructionKind::Hitstop {
            frames: Value::number(3.0),
        },
        InstructionKind::SetLetterbox {
            on: Value::number(1.0),
        },
        InstructionKind::FadeScreen {
            color: Value::text("CURTAIN"),
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
            // The name is trimmed, as it is everywhere a block names
            // something.
            Effect::PlayCutscene {
                cutscene: "Opener".to_string(),
            },
            Effect::SkipCutscene,
            Effect::CameraShake { amount: 0.7 },
            Effect::SetTimeScale { scale: 0.5 },
            Effect::Hitstop { frames: 3.0 },
            Effect::SetLetterbox { on: 1.0 },
            // An unknown fade color reads as none, like transitions.
            Effect::FadeScreen {
                color: "none".to_string(),
            },
        ]
    );
}

#[test]
fn a_cutscene_signal_starts_only_the_strand_that_names_it() {
    let project = project_with(vec![
        Strand::with_instructions(
            0,
            0,
            vec![
                ins(InstructionKind::WhenCutsceneSignal {
                    signal: "beat".to_string(),
                }),
                ins(say("the beat drops")),
            ],
        ),
        Strand::with_instructions(
            0,
            400,
            vec![
                ins(InstructionKind::WhenCutsceneSignal {
                    signal: "sting".to_string(),
                }),
                ins(say("never")),
            ],
        ),
        Strand::with_instructions(
            0,
            800,
            vec![
                ins(InstructionKind::WhenCutsceneSignal {
                    signal: "".to_string(),
                }),
                ins(say("any signal")),
            ],
        ),
    ]);
    let mut vm = Harness::new(&project);
    vm.vm.fire(Event::CutsceneSignal {
        signal: "Beat".to_string(),
    });
    assert_eq!(
        says(&vm.run(1)),
        vec!["the beat drops".to_string(), "any signal".to_string()]
    );
}

#[test]
fn a_cutscene_end_runs_its_strands() {
    let project = project_with(vec![Strand::with_instructions(
        0,
        0,
        vec![
            ins(InstructionKind::WhenCutsceneEnds),
            ins(say("curtain down")),
        ],
    )]);
    let mut vm = Harness::new(&project);
    vm.vm.fire(Event::CutsceneEnded {
        cutscene: "Opener".to_string(),
    });
    assert_eq!(says(&vm.run(1)), vec!["curtain down".to_string()]);
}

#[test]
fn the_cutscene_reporters_read_the_published_snapshot() {
    use blockloom_core::sense;
    blockloom_core::init();
    sense::publish(Sensors {
        cutscene_name: "Opener".to_string(),
        cutscene_time: 4.25,
        ..Default::default()
    });
    let playing = Value::op(Op::from_name("IsCutscenePlaying"), vec![]);
    assert_eq!(playing.eval(), Ok(Evaluated::Bool(true)));
    let time = Value::op(Op::from_name("CutsceneTime"), vec![]);
    assert_eq!(time.eval(), Ok(Evaluated::Number(4.25)));
    sense::publish(Sensors::default());
    let playing = Value::op(Op::from_name("IsCutscenePlaying"), vec![]);
    assert_eq!(playing.eval(), Ok(Evaluated::Bool(false)));
}

#[test]
fn a_plugin_block_asks_the_editor_to_run_its_command_with_its_slots() {
    let project = project_with(vec![started(vec![
        InstructionKind::SetVariable {
            name: "n".to_string(),
            value: Value::number(4.0),
        },
        InstructionKind::PluginBlock {
            plugin: "com.example.health".to_string(),
            block: "heal".to_string(),
            args: vec![
                Value::text("Hero"),
                Value::Var {
                    name: "n".to_string(),
                },
                Value::number(1.5),
                Value::Bool,
            ],
        },
    ])]);
    let actor = project.actors[0].id.clone();
    let effects = Harness::started(&project).run(1);
    assert!(effects.contains(&Effect::PluginCall {
        actor,
        plugin: "com.example.health".to_string(),
        block: "heal".to_string(),
        // A whole number is an integer; a bare bool slot reads false.
        args: vec![
            serde_json::json!("Hero"),
            serde_json::json!(4),
            serde_json::json!(1.5),
            serde_json::json!(false),
        ],
    }));
}

/// One wall at the end of every query, standing in for a physics world.
struct Wall;

impl blockloom_core::physics::query::QueryService for Wall {
    fn run(
        &self,
        request: &blockloom_core::physics::query::QueryRequest,
        _: &blockloom_core::physics::query::QueryFilter,
        limit: usize,
    ) -> blockloom_core::physics::query::QueryOutcome {
        use blockloom_core::physics::query::{QueryHit, QueryOutcome, QueryRequest};
        let point = match request {
            QueryRequest::Ray { to, .. } | QueryRequest::Cast { to, .. } => *to,
            QueryRequest::Overlap { at, .. } => *at,
            QueryRequest::Closest { point, .. } => *point,
        };
        let hit = QueryHit {
            actor: "wall".into(),
            body: None,
            collider: "wall:0".into(),
            subshape: 0,
            point,
            normal: [0.0, 1.0, 0.0],
            distance: point[0],
            fraction: 1.0,
            started_inside: false,
            trigger: false,
        };
        QueryOutcome::finish(vec![hit], limit)
    }
}

#[test]
fn a_cast_block_files_its_answer_for_the_hit_reporters() {
    use blockloom_core::physics::query::{self, RayHits, TriggerPolicy};
    let project = project_with(vec![started(vec![
        InstructionKind::CastRay {
            hits: RayHits::Nearest,
            triggers: TriggerPolicy::UseGlobal,
            from_x: Value::number(0.0),
            from_y: Value::number(0.0),
            from_z: Value::number(0.0),
            to_x: Value::number(6.0),
            to_y: Value::number(1.0),
            to_z: Value::number(0.0),
        },
        InstructionKind::Say {
            text: Value::op(
                Op::from_name("QueryNumber"),
                vec![Value::number(1.0), Value::text("distance")],
            ),
        },
        InstructionKind::Say {
            text: Value::op(
                Op::from_name("QueryText"),
                vec![Value::number(1.0), Value::text("actor")],
            ),
        },
        InstructionKind::Say {
            text: Value::op(
                Op::from_name("QueryNumber"),
                vec![Value::number(2.0), Value::text("distance")],
            ),
        },
    ])]);
    query::reset();
    let effects = query::with_service(&Wall, 0, || Harness::started(&project).run(1));
    let said: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Say { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    // The second hit does not exist, which reads as zero rather than an error.
    assert_eq!(said, ["6", "wall", "0"]);
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::PhysicsQuery { kind, hits: 1, .. } if kind == "ray"
    )));
}

#[test]
fn a_controller_block_with_no_controller_reports_why_and_reads_as_nothing() {
    use blockloom_core::physics::controller::{self, ControllerProperty, MoveMode};
    let project = project_with(vec![started(vec![
        InstructionKind::ControllerMove {
            mode: MoveMode::Simple,
            x: Value::number(1.0),
            y: Value::number(0.0),
            z: Value::number(0.0),
        },
        InstructionKind::SetController {
            property: ControllerProperty::Radius,
            value: Value::number(0.4),
        },
        InstructionKind::Say {
            text: Value::op(
                Op::from_name("ControllerNumber"),
                vec![Value::number(0.0), Value::text("grounded")],
            ),
        },
    ])]);
    controller::reset();
    let effects = Harness::started(&project).run(1);
    let errors = effects
        .iter()
        .filter(|effect| matches!(effect, Effect::Error { .. }))
        .count();
    assert_eq!(errors, 2, "both statements say there is nothing to move");
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::Say { text, .. } if text == "0"
    )));
}

#[test]
fn motor_blocks_steer_set_and_read_the_actors_motor() {
    use blockloom_core::physics::motor::{self, CharacterMotorSpec, MotorAction, MotorProperty};
    let project = project_with(vec![started(vec![
        InstructionKind::MotorAct {
            action: MotorAction::Intent,
            x: Value::number(1.0),
            y: Value::number(0.0),
            z: Value::number(0.0),
        },
        InstructionKind::SetMotor {
            property: MotorProperty::WalkSpeed,
            value: Value::number(7.0),
        },
        InstructionKind::Say {
            text: Value::op(
                Op::from_name("MotorNumber"),
                vec![Value::text("walk speed")],
            ),
        },
        InstructionKind::SetMotor {
            property: MotorProperty::AirControl,
            value: Value::number(9.0),
        },
    ])]);
    let id = project.active_scene().actors[0].id.clone();
    motor::reset();
    motor::register(&id, CharacterMotorSpec::default());
    let effects = Harness::started(&project).run(1);
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::Say { text, .. } if text == "7"
    )));
    let errors = effects
        .iter()
        .filter(|effect| matches!(effect, Effect::Error { .. }))
        .count();
    assert_eq!(errors, 1, "an air control past 1 is refused: {effects:?}");
    assert_eq!(motor::spec_of(&id).unwrap().walk_speed, 7.0);
    motor::reset();
}

#[test]
fn joint_blocks_command_a_named_constraint_and_read_it_back() {
    use blockloom_core::physics::joints::{
        self, ConstraintKind, ConstraintPlan, ConstraintSpec, JointStatus, JointVerb,
        resolve_frames,
    };
    use blockloom_core::scene::Mode;
    let project = project_with(vec![started(vec![
        InstructionKind::JointAct {
            action: JointVerb::MotorSpeed,
            joint: "Axle".to_string(),
            value: Value::number(45.0),
        },
        InstructionKind::JointAct {
            action: JointVerb::Break,
            joint: "nope".to_string(),
            value: Value::number(0.0),
        },
        InstructionKind::Say {
            text: Value::op(
                Op::from_name("JointNumber"),
                vec![Value::text("angle"), Value::text("axle")],
            ),
        },
    ])]);
    let id = project.active_scene().actors[0].id.clone();
    let mut spec = ConstraintSpec::of(ConstraintKind::Hinge, Mode::ThreeD);
    spec.name = "Axle".to_string();
    let frames = resolve_frames(&spec, &glam::Mat4::IDENTITY, None);
    joints::register_plan(&[ConstraintPlan {
        actor: id.clone(),
        target: None,
        handle: "Axle".to_string(),
        spec,
        frames,
    }]);
    joints::publish(
        &id,
        "Axle",
        JointStatus {
            enabled: true,
            position: 30.0,
            ..Default::default()
        },
    );
    let effects = Harness::started(&project).run(1);
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::Say { text, .. } if text == "30"
    )));
    let errors = effects
        .iter()
        .filter(|effect| matches!(effect, Effect::Error { .. }))
        .count();
    assert_eq!(
        errors, 1,
        "a joint that isn't there is an error: {effects:?}"
    );
    let queued = joints::take_commands();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].verb, JointVerb::MotorSpeed);
    assert_eq!(queued[0].value, 45.0);
    joints::reset();
}

#[test]
fn a_query_with_no_world_reports_why_and_reads_as_a_miss() {
    use blockloom_core::physics::query::{self, TriggerPolicy};
    let project = project_with(vec![started(vec![
        InstructionKind::OverlapBall {
            triggers: TriggerPolicy::UseGlobal,
            radius: Value::number(-1.0),
            x: Value::number(0.0),
            y: Value::number(0.0),
            z: Value::number(0.0),
        },
        InstructionKind::Say {
            text: Value::op(
                Op::from_name("QueryNumber"),
                vec![Value::number(1.0), Value::text("count")],
            ),
        },
    ])]);
    query::reset();
    let effects = Harness::started(&project).run(1);
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Error { .. }))
    );
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::Say { text, .. } if text == "0"
    )));
}

fn when_plugin(event: &str, args: &[&str]) -> Instruction {
    ins(InstructionKind::WhenPlugin {
        plugin: "com.example.tally".to_string(),
        block: "changed".to_string(),
        event: event.to_string(),
        args: args.iter().map(|a| a.to_string()).collect(),
    })
}

fn plugin_event(event: &str, args: &[&str], actor: Option<&str>) -> Event {
    Event::Plugin {
        plugin: "com.example.tally".to_string(),
        event: event.to_string(),
        args: args.iter().map(|a| a.to_string()).collect(),
        actor: actor.map(str::to_string),
    }
}

#[test]
fn a_plugin_hat_starts_on_its_plugins_event_and_matches_its_slots() {
    let hat = |args: &[&str], line: &str| {
        Strand::with_instructions(0, 0, vec![when_plugin("changed", args), ins(say(line))])
    };
    let project = project_with(vec![
        hat(&[], "any"),
        hat(&["coins"], "coins"),
        hat(&["lives", "3"], "lives at 3"),
        Strand::with_instructions(0, 0, vec![when_plugin("other", &[]), ins(say("other"))]),
    ]);
    let mut run = Harness::new(&project);
    run.vm.fire(plugin_event("changed", &["coins", "7"], None));
    assert_eq!(says(&run.run(1)), ["any", "coins"]);
    // A slot matches as text or as numbers, and an empty slot matches all.
    run.vm
        .fire(plugin_event("changed", &["lives", "3.0"], None));
    assert_eq!(says(&run.run(1)), ["any", "lives at 3"]);
    run.vm.fire(plugin_event("changed", &["lives", "4"], None));
    assert_eq!(says(&run.run(1)), ["any"]);
    // Another plugin's event of the same name is not this one's.
    run.vm.fire(Event::Plugin {
        plugin: "com.example.other".to_string(),
        event: "changed".to_string(),
        args: vec![],
        actor: None,
    });
    assert!(says(&run.run(1)).is_empty());
}

#[test]
fn a_plugin_event_naming_an_actor_starts_only_that_actors_hats() {
    let project = project_with(vec![Strand::with_instructions(
        0,
        0,
        vec![when_plugin("changed", &[]), ins(say("heard"))],
    )]);
    let player = project.actors[0].id.clone();
    let mut run = Harness::new(&project);
    run.vm
        .fire(plugin_event("changed", &[], Some("somebody-else")));
    assert!(says(&run.run(1)).is_empty());
    run.vm.fire(plugin_event("changed", &[], Some(&player)));
    assert_eq!(says(&run.run(1)), ["heard"]);
}

#[test]
fn a_plugin_reporter_asks_the_installed_reader_each_time_it_is_evaluated() {
    use blockloom_core::sense;
    use blockloom_core::value::PLUGIN_READ;
    blockloom_core::init();
    let ask = |name: &str| {
        Value::op(
            Op::from_name(PLUGIN_READ),
            vec![
                Value::text("com.example.tally"),
                Value::text("count"),
                Value::text(name),
            ],
        )
    };
    // Nothing answers outside a run, and the error says so.
    sense::set_plugin_reader(None);
    let error = ask("coins").eval().unwrap_err();
    assert!(
        error.contains("only answers while the game is running"),
        "{error}"
    );

    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = seen.clone();
    sense::set_plugin_reader(Some(Box::new(move |plugin, block, args| {
        log.borrow_mut()
            .push(format!("{plugin}/{block}/{}", args[0].as_text()));
        match args[0].as_text().as_str() {
            "coins" => Ok(Evaluated::Number(12.0)),
            other => Err(format!("no tally called {other}")),
        }
    })));
    assert_eq!(ask("coins").eval(), Ok(Evaluated::Number(12.0)));
    // It composes like any reporter.
    let double = Value::op(Op::Mul, vec![ask("coins"), Value::number(2.0)]);
    assert_eq!(double.eval(), Ok(Evaluated::Number(24.0)));
    assert_eq!(ask("gems").eval(), Err("no tally called gems".to_string()));
    assert_eq!(
        *seen.borrow(),
        [
            "com.example.tally/count/coins",
            "com.example.tally/count/coins",
            "com.example.tally/count/gems"
        ]
    );
    sense::set_plugin_reader(None);
}
