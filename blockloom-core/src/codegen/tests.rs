//! What can be checked without a compiler: that the value type this emits
//! against coerces exactly as the VM's does, that the emitted source says
//! what it should, and that a block it can't do yet is refused by name.
//!
//! Running the emitted program against the VM is `tests/codegen.rs`, which
//! has a `rustc` to hand.

use super::*;
use crate::blocks::{Instruction, InstructionKind as K, Strand};
use crate::project::{Actor, Project};
use crate::scene::{Axis, Mode};
use crate::value::Evaluated;

fn project_with(instructions: Vec<Instruction>) -> Project {
    let mut project = Project::starter("Codegen", Mode::TwoD);
    project.actors.clear();
    let mut actor = Actor::new(
        "Player",
        crate::scene::Visual::Circle {
            color: "#fff".to_string(),
            radius: 10.0,
        },
    );
    actor.id = "a1".to_string();
    actor
        .graph
        .strands
        .push(Strand::with_instructions(0, 0, instructions));
    project.actors.push(actor);
    project
}

fn started(body: Vec<Instruction>) -> Project {
    let mut instructions = vec![Instruction::new(K::WhenStarted)];
    instructions.extend(body);
    project_with(instructions)
}

/// Every case where a `Val` and an `Evaluated` could disagree. They are two
/// copies of one rule, and the day they differ a compiled game and a played
/// one stop agreeing about what a block means.
#[test]
fn a_generated_value_coerces_exactly_as_the_vms_does() {
    let pairs = [
        (Val::Num(3.5), Evaluated::Number(3.5)),
        (Val::Num(0.0), Evaluated::Number(0.0)),
        (Val::Num(-0.0), Evaluated::Number(-0.0)),
        (Val::Text("7".to_string()), Evaluated::Text("7".to_string())),
        (
            Val::Text("  7.5 ".to_string()),
            Evaluated::Text("  7.5 ".to_string()),
        ),
        (
            Val::Text("nope".to_string()),
            Evaluated::Text("nope".to_string()),
        ),
        (Val::Text(String::new()), Evaluated::Text(String::new())),
        (
            Val::Text("false".to_string()),
            Evaluated::Text("false".to_string()),
        ),
        (Val::Bool(true), Evaluated::Bool(true)),
        (Val::Bool(false), Evaluated::Bool(false)),
    ];
    for (val, evaluated) in pairs {
        assert_eq!(
            val.as_number().ok(),
            evaluated.as_number().ok(),
            "as_number disagreed on {val:?}"
        );
        assert_eq!(
            val.as_bool(),
            evaluated.as_bool(),
            "as_bool disagreed on {val:?}"
        );
        assert_eq!(
            val.as_text(),
            evaluated.as_text(),
            "as_text disagreed on {val:?}"
        );
    }
}

#[test]
fn an_actions_slots_become_expressions_in_its_own_line() {
    let project = started(vec![Instruction::new(K::ChangePosition {
        axis: Axis::Y,
        by: Value::op(
            Op::Add,
            vec![
                Value::number(2.0),
                Value::Var {
                    name: "speed".to_string(),
                },
            ],
        ),
    })]);
    let source = compile(&project).expect("actions compile");

    assert!(source.contains("const ME: &str = \"a1\";"), "{source}");
    // The slot is worked out into a local and then handed over: reading it
    // wants the host and so does the act, and one expression can't have both.
    assert!(
        source.contains(
            "let slot = { let v = add(Ok(Val::Num(2.0f64)), \
             Ok(h.variable(ME, \"speed\"))); number(h, ME, v) };"
        ),
        "{source}"
    );
    assert!(
        source.contains("h.act(ME, Act::ChangePosition { axis: 1, by: slot });"),
        "{source}"
    );
}

#[test]
fn a_strand_becomes_a_function_the_entry_table_points_at() {
    let project = started(vec![Instruction::new(K::Say {
        text: Value::text("hello"),
    })]);
    let source = compile(&project).expect("a say compiles");

    assert!(source.contains("trigger: \"Started\""), "{source}");
    // The entry carries where to start and how much state a run needs, since
    // the function itself is resumable and says neither.
    assert!(
        source.contains("start: 0, counters: 0, run: strand_0"),
        "{source}"
    );
    assert!(
        source.contains("fn strand_0(h: &mut dyn Host, s: &mut State)"),
        "{source}"
    );
    assert!(source.contains("if !s.resume()"), "{source}");
}

#[test]
fn a_key_trigger_carries_the_key_it_normalizes_to() {
    let mut project = started(vec![]);
    project.actors[0].graph.strands[0].instructions[0] = Instruction::new(K::WhenKeyPressed {
        key: "Space".to_string(),
    });
    let source = compile(&project).expect("a key strand compiles");

    assert!(
        source.contains("trigger: \"Key\", detail: \"space\""),
        "{source}"
    );
}

#[test]
fn and_keeps_its_second_operand_unevaluated() {
    let project = started(vec![Instruction::new(K::Say {
        text: Value::op(Op::And, vec![Value::Bool, Value::op(Op::True, vec![])]),
    })]);
    let source = compile(&project).expect("a condition compiles");

    // The closure is the short circuit: without it, a bad slot on the right
    // would report itself even when the left already decided the answer.
    assert!(source.contains("and(Ok(Val::Bool(false)), || "), "{source}");
}

#[test]
fn the_world_and_the_clock_are_asked_of_the_host() {
    let project = started(vec![
        Instruction::new(K::Say {
            text: Value::op(Op::from_name("KeyDown"), vec![Value::text("space")]),
        }),
        Instruction::new(K::Move {
            steps: Value::op(Op::Random, vec![Value::number(1.0), Value::number(6.0)]),
        }),
    ]);
    let source = compile(&project).expect("sensing compiles");

    assert!(
        source.contains(
            "{ let a = vec![Ok(Val::Text(\"space\".to_string()))]; \
             sense(h, ME, \"KeyDown\", a) }"
        ),
        "{source}"
    );
    // Random isn't a sensing block, but it reads something outside the
    // program all the same, so it goes the same way.
    assert!(source.contains("sense(h, ME, \"Random\", a) }"), "{source}");
}

#[test]
fn a_variable_write_goes_through_the_host_rather_than_an_act() {
    let project = started(vec![
        Instruction::new(K::SetVariable {
            name: "score".to_string(),
            value: Value::number(1.0),
        }),
        Instruction::new(K::ChangeVariable {
            name: "score".to_string(),
            value: Value::number(2.0),
        }),
    ]);
    let source = compile(&project).expect("variables compile");

    assert!(
        source.contains("h.set_variable(ME, \"score\", value);"),
        "{source}"
    );
    assert!(
        source.contains("h.variable(ME, \"score\").as_number().unwrap_or(0.0)"),
        "{source}"
    );
}

/// A `repeat` is the one loop with something to remember, and it remembers it
/// in a slot the entry table sized rather than on a frame stack.
#[test]
fn a_repeat_counts_down_in_a_slot_of_its_own() {
    let project = started(vec![Instruction::new(K::Repeat {
        count: Value::number(3.0),
        body: vec![Instruction::new(K::Move {
            steps: Value::number(1.0),
        })],
    })]);
    let source = compile(&project).expect("a repeat compiles");

    assert!(source.contains("counters: 1, run: strand_0"), "{source}");
    assert!(source.contains("s.enter_repeat(0, n);"), "{source}");
    assert!(
        source.contains("let left = s.next_iteration(0);"),
        "{source}"
    );
}

/// The one scheduling rule the whole engine leans on: a loop hands the frame
/// back every time round, so a `forever` costs one iteration per tick.
#[test]
fn a_loop_gives_the_frame_back_at_its_back_edge() {
    let project = started(vec![Instruction::new(K::Forever {
        body: vec![Instruction::new(K::Move {
            steps: Value::number(1.0),
        })],
    })]);
    let source = compile(&project).expect("a forever compiles");

    // Step 0 is the head, 1 the move, 2 the back edge that points at the head.
    assert!(
        source.contains("            2 => {\n                s.pc = 0;\n                return;"),
        "{source}"
    );
    // And nothing else in it does, or a `forever` would hang the game.
    assert_eq!(source.matches("return;").count(), 4, "{source}");
}

#[test]
fn an_escape_jumps_past_the_loop_it_is_in_and_a_continue_at_its_back_edge() {
    let project = started(vec![Instruction::new(K::Forever {
        body: vec![
            Instruction::new(K::EscapeLoop),
            Instruction::new(K::ContinueLoop),
        ],
    })]);
    let source = compile(&project).expect("escape and continue compile");

    // 0 head, 1 escape, 2 continue, 3 back edge. Past the loop is 4; the
    // back edge is 3.
    assert!(
        source.contains("            1 => {\n                s.pc = 4;"),
        "{source}"
    );
    assert!(
        source.contains("            2 => {\n                s.pc = 3;"),
        "{source}"
    );
}

/// A `wait` puts the strand to sleep and leaves the counter on what comes
/// after, so waking up doesn't run the wait again.
#[test]
fn a_wait_sleeps_and_comes_back_at_the_next_block() {
    let project = started(vec![
        Instruction::new(K::Wait {
            duration: Value::number(0.5),
        }),
        Instruction::new(K::Say {
            text: Value::text("hi"),
        }),
    ]);
    let source = compile(&project).expect("a wait compiles");

    assert!(source.contains("s.pc = 1;\n"), "{source}");
    assert!(source.contains("s.sleep(seconds);"), "{source}");
    // A zero-second wait isn't a yield, exactly as the VM has it.
    assert!(source.contains("if seconds > 0.0 {"), "{source}");
}

/// Unlike a `wait`, this one leaves the counter where it is: the condition is
/// asked again from the top of the same arm next frame.
#[test]
fn a_wait_until_stays_put_until_it_holds() {
    let project = started(vec![Instruction::new(K::WaitUntil {
        condition: Value::Bool,
    })]);
    let source = compile(&project).expect("a wait until compiles");

    assert!(
        source.contains("if !holds {\n                    return;\n                }"),
        "{source}"
    );
}

#[test]
fn a_custom_block_is_still_named_rather_than_half_emitted() {
    let calling = K::CallBlock {
        block_id: "b1".to_string(),
        args: vec![],
    };
    let error = compile(&started(vec![Instruction::new(calling.clone())]))
        .expect_err("custom blocks aren't compiled yet");
    assert_eq!(error.what, "a custom block");
    assert!(!supports(&calling));

    // A project that only defines one is refused too: its body is a strand
    // nothing here can enter, so half of it would be missing.
    let mut project = started(vec![]);
    project.actors[0]
        .graph
        .strands
        .push(Strand::with_instructions(
            0,
            0,
            vec![
                Instruction::new(K::BlockHeader {
                    block_id: "b1".to_string(),
                }),
                Instruction::new(K::Return {
                    value: Value::number(1.0),
                }),
            ],
        ));
    let error = compile(&project).expect_err("a defined custom block is refused too");
    assert_eq!(error.what, "a custom block");
}

#[test]
fn every_block_the_emitter_claims_it_can_do_it_can() {
    // `supports` is what a caller asks before offering a fast build, so it
    // must not promise more than `compile` delivers.
    let kinds = [
        K::Move {
            steps: Value::number(1.0),
        },
        K::Say {
            text: Value::text("hi"),
        },
        K::SetVisible { visible: false },
        K::Broadcast {
            name: "go".to_string(),
        },
        K::SetVariable {
            name: "v".to_string(),
            value: Value::number(1.0),
        },
        K::AttachComponent {
            component: "Body".to_string(),
        },
        K::If {
            condition: Value::Bool,
            body: vec![Instruction::new(K::StopAll)],
        },
        K::IfElse {
            condition: Value::Bool,
            then_body: vec![Instruction::new(K::EscapeLoop)],
            else_body: vec![Instruction::new(K::ContinueLoop)],
        },
        K::Repeat {
            count: Value::number(2.0),
            body: vec![Instruction::new(K::Wait {
                duration: Value::number(1.0),
            })],
        },
        K::Forever {
            body: vec![Instruction::new(K::WaitUntil {
                condition: Value::Bool,
            })],
        },
        K::While {
            condition: Value::Bool,
            body: vec![Instruction::new(K::Return {
                value: Value::number(0.0),
            })],
        },
        K::Glide {
            seconds: Value::number(1.0),
            x: Value::number(0.0),
            y: Value::number(0.0),
            z: Value::number(0.0),
        },
        K::StopAll,
    ];
    for kind in kinds {
        assert!(supports(&kind), "{kind:?}");
        compile(&started(vec![Instruction::new(kind.clone())]))
            .unwrap_or_else(|e| panic!("{kind:?} is claimed supported but {e}"));
    }
}

#[test]
fn a_float_slot_survives_the_round_trip_through_source() {
    assert_eq!(float(0.1), "0.1f64");
    assert_eq!(float(-2.0), "-2.0f64");
    assert_eq!(float(f64::INFINITY), "f64::INFINITY");
    assert!(float(f64::NAN).contains("NAN"));
}
