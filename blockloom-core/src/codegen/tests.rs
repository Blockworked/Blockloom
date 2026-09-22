//! What can be checked without a compiler: that the value type this emits
//! against coerces exactly as the VM's does, that the emitted source says
//! what it should, and that what it won't compile is refused by name.
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

/// A green-flag strand, and one custom block `b1` taking a `distance` for it
/// to call. Its body starts one past the strand's `End`.
fn with_block(body: Vec<K>, block: Vec<Instruction>) -> Project {
    use blockstitch_core::graph::{BlockDef, BlockPiece, BlockShape};

    let mut project = started(body.into_iter().map(Instruction::new).collect());
    let mut header = vec![Instruction::new(K::BlockHeader {
        block_id: "b1".to_string(),
    })];
    header.extend(block);
    project.actors[0]
        .graph
        .strands
        .push(Strand::with_instructions(0, 400, header));
    project.actors[0].graph.block_defs.push(BlockDef {
        id: "b1".to_string(),
        pieces: vec![
            BlockPiece::Label {
                id: "p0".to_string(),
                text: "go".to_string(),
            },
            BlockPiece::Input {
                id: "p1".to_string(),
                name: "distance".to_string(),
                value_type: Default::default(),
            },
        ],
        shape: BlockShape::Normal,
        color: blockstitch_core::graph::default_block_color(),
    });
    project
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

    assert!(source.contains("let me = Rc::clone(&s.me);"), "{source}");
    assert!(source.contains("(\"a1\", \"Player\")"), "{source}");
    // The variable is read first, the way `resolve` reads it, and the slot is
    // worked out into a local before it is handed over: reading it wants the
    // host and so does the act, and one expression can't have both.
    assert!(
        source.contains(
            "let slot = { let v0 = h.variable(&me, \"speed\"); \
             let v = add(Ok(Val::Num(2.0f64)), Ok(v0)); number(h, &me, v) };"
        ),
        "{source}"
    );
    assert!(
        source.contains("h.act(&me, Act::ChangePosition { axis: 1, by: slot });"),
        "{source}"
    );
}

#[test]
fn an_actors_canvas_becomes_a_function_the_entry_table_points_at() {
    let project = started(vec![Instruction::new(K::Say {
        text: Value::text("hello"),
    })]);
    let source = compile(&project).expect("a say compiles");

    assert!(source.contains("trigger: \"Started\""), "{source}");
    // The entry carries where to start and how much state a run needs, since
    // the function itself is resumable and says neither.
    assert!(
        source.contains("start: 0, counters: 0, run: actor_0"),
        "{source}"
    );
    assert!(
        source.contains("fn actor_0(h: &mut dyn Host, s: &mut State, actors: &mut Actors)"),
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
             sense(h, &me, \"KeyDown\", a) }"
        ),
        "{source}"
    );
    // Random isn't a sensing block, but it reads something outside the
    // program all the same, so it goes the same way.
    assert!(
        source.contains("sense(h, &me, \"Random\", a) }"),
        "{source}"
    );
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
        source.contains("h.set_variable(&me, \"score\", value);"),
        "{source}"
    );
    assert!(
        source.contains("h.variable(&me, \"score\").as_number().unwrap_or(0.0)"),
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

    assert!(source.contains("counters: 1, run: actor_0"), "{source}");
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
    // And nothing else in it does, or a `forever` would hang the game. The
    // other three are the resume guard, the fall off the end, and the arm
    // that runs off the step list.
    let actor = source
        .split("fn actor_0")
        .nth(1)
        .and_then(|rest| {
            rest.split(
                "
}
",
            )
            .next()
        })
        .expect("the emitted function");
    assert_eq!(actor.matches("return;").count(), 4, "{source}");
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

/// Where the two copies of the VM's scheduling limits have to agree. A
/// generated program carries its own, since it links against nothing.
#[test]
fn a_generated_program_runs_to_the_vms_own_limits() {
    assert_eq!(runtime::MAX_REPORTER_DEPTH, crate::vm::MAX_REPORTER_DEPTH);
    assert_eq!(runtime::STEP_BUDGET, crate::vm::STEP_BUDGET);
}

/// A statement call is a jump into the callee's region with somewhere to come
/// back to, which is why one function covers a whole actor.
#[test]
fn a_custom_block_called_as_a_statement_is_a_jump_and_a_frame() {
    let source = compile(&with_block(
        vec![K::CallBlock {
            block_id: "b1".to_string(),
            args: vec![Value::number(7.0)],
        }],
        vec![Instruction::new(K::Move {
            steps: Value::Param {
                name: "distance".to_string(),
            },
        })],
    ))
    .expect("a statement call compiles");

    assert!(source.contains("s.enter_call(1, args);"), "{source}");
    // The body sits at 2: the call, the strand's `End`, then the block.
    assert!(source.contains("s.pc = 2;"), "{source}");
    assert!(source.contains("match s.resume_at()"), "{source}");
    // Its input is read back by position, not by the name it was given.
    assert!(source.contains("let v0 = s.param(0);"), "{source}");
}

/// A reporter is a function with a state of its own, so one can call another
/// - and itself - without the caller's program counter going anywhere.
#[test]
fn a_reporter_call_is_a_function_with_a_depth_guard() {
    let source = compile(&with_block(
        vec![K::Say {
            text: Value::Call {
                block_id: "b1".to_string(),
                args: vec![Value::number(2.0)],
                saved: Box::new(Value::number(0.0)),
            },
        }],
        vec![Instruction::new(K::Return {
            value: Value::Param {
                name: "distance".to_string(),
            },
        })],
    ))
    .expect("a reporter call compiles");

    assert!(
        source.contains("fn reporter_0(") && source.contains("s.enter_call(State::RETURN, args);"),
        "{source}"
    );
    // A strand is always at the top, so the guard reads against nothing yet.
    assert!(
        source.contains("if 0 >= MAX_REPORTER_DEPTH { too_deep(h, &me) }"),
        "{source}"
    );
    assert!(
        source.contains("reporter_0(h, s, actors, 2, 0 + 1,"),
        "{source}"
    );
    // Nothing suspends in there, and the budget is what ends a runaway one.
    assert!(source.contains("let mut budget = STEP_BUDGET;"), "{source}");
}

/// The one thing it won't do. The counters a `repeat` uses are picked when
/// the code is written, so a second live invocation would share the first's.
#[test]
fn a_custom_block_that_can_reach_itself_is_refused_by_name() {
    let error = compile(&with_block(
        vec![K::CallBlock {
            block_id: "b1".to_string(),
            args: vec![],
        }],
        vec![Instruction::new(K::CallBlock {
            block_id: "b1".to_string(),
            args: vec![],
        })],
    ))
    .expect_err("a block that calls itself is refused");
    assert_eq!(error.what, "a custom block that calls itself");
}

/// A reporter, though, may: each call builds a state of its own, exactly as
/// the VM builds a fresh script for one.
#[test]
fn a_reporter_may_call_itself() {
    compile(&with_block(
        vec![K::Say {
            text: Value::Call {
                block_id: "b1".to_string(),
                args: vec![],
                saved: Box::new(Value::number(0.0)),
            },
        }],
        vec![Instruction::new(K::Return {
            value: Value::Call {
                block_id: "b1".to_string(),
                args: vec![],
                saved: Box::new(Value::number(0.0)),
            },
        })],
    ))
    .expect("a recursive reporter compiles");
}

/// `resolve` replaces every variable and reporter call in a tree before one
/// operator runs, so the short circuit below can't be allowed to skip them.
#[test]
fn everything_resolve_touches_happens_before_the_operators_do() {
    let project = started(vec![Instruction::new(K::Say {
        text: Value::op(
            Op::And,
            vec![
                Value::op(Op::False, vec![]),
                Value::Var {
                    name: "score".to_string(),
                },
            ],
        ),
    })]);
    let source = compile(&project).expect("a short circuit compiles");

    // The read is hoisted out of the closure; only the operator is deferred.
    assert!(
        source.contains(
            "let v0 = h.variable(&me, \"score\"); let v = and(Ok(Val::Bool(false)), || Ok(v0));"
        ),
        "{source}"
    );
}

#[test]
fn a_float_slot_survives_the_round_trip_through_source() {
    assert_eq!(float(0.1), "0.1f64");
    assert_eq!(float(-2.0), "-2.0f64");
    assert_eq!(float(f64::INFINITY), "f64::INFINITY");
    assert!(float(f64::NAN).contains("NAN"));
}

#[test]
fn a_clone_trigger_is_an_entry_like_any_other() {
    let mut project = started(vec![Instruction::new(K::CreateClone { of: String::new() })]);
    project.actors[0]
        .graph
        .strands
        .push(Strand::with_instructions(
            0,
            400,
            vec![
                Instruction::new(K::WhenCloned),
                Instruction::new(K::Move {
                    steps: Value::number(1.0),
                }),
            ],
        ));
    let source = compile(&project).expect("clones compile");

    assert!(source.contains("trigger: \"Cloned\""), "{source}");
    // The copy is the program's own to name and to schedule; the host is
    // handed the entity and nothing else.
    assert!(
        source.contains("let clone = actors.clone_of(&template);"),
        "{source}"
    );
    assert!(
        source.contains("h.act(&me, Act::CreateClone { of: template.to_string()"),
        "{source}"
    );
}

#[test]
fn a_delete_ends_the_strand_that_asked_where_it_stands() {
    let project = started(vec![
        Instruction::new(K::DeleteActor {
            target: Value::text("myself"),
        }),
        Instruction::new(K::Say {
            text: Value::text("never"),
        }),
    ]);
    let source = compile(&project).expect("a delete compiles");

    assert!(source.contains("actors.remove(&gone);"), "{source}");
    // Checked after the act, as the VM checks it, and the `say` after it is
    // in the same arm - so the guard has to end the arm itself.
    assert!(source.contains("if actors.is_gone(&me) {"), "{source}");

    // A project that never deletes pays nothing for the check.
    let quiet = started(vec![Instruction::new(K::Say {
        text: Value::text("hello"),
    })]);
    let source = compile(&quiet).expect("a say compiles");
    assert!(!source.contains("if actors.is_gone(&me) {"), "{source}");
}
