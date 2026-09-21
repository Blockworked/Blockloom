//! Compiling a canvas into a flat program.
//!
//! A nested instruction tree can't be suspended mid-body without either
//! recursion or a path cursor, so every strand is flattened into one `Vec` of
//! [`Step`]s with jumps. A running script is then just a program counter and a
//! small frame stack - suspending it is free, which is what makes `wait` and
//! per-frame yielding work.

use crate::blocks::{ActorGraph, Instruction, InstructionKind};
use crate::components::CameraView;
use crate::scene::{Axis, BodyKind};
use crate::value::Value;
use std::collections::HashMap;

/// What starts a script.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Trigger {
    /// The green flag.
    Started,
    KeyPressed(String),
    Clicked,
    /// An empty `with` means "anything".
    Collision {
        with: String,
    },
    Message(String),
}

/// One entry point: a header strand's trigger and where its body starts.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub trigger: Trigger,
    pub strand_id: String,
    pub pc: usize,
}

/// How a loop decides whether to go round again.
#[derive(Debug, Clone, PartialEq)]
pub enum LoopKind {
    /// Counted once, on entry.
    Repeat(Value),
    Forever,
    /// Re-checked before every iteration.
    While(Value),
}

/// A leaf instruction: it changes the world and never affects control flow.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Move(Value),
    GoTo([Value; 3]),
    ChangePosition {
        axis: Axis,
        by: Value,
    },
    Turn {
        axis: Axis,
        degrees: Value,
    },
    SetRotation {
        axis: Axis,
        degrees: Value,
    },
    PointTowards(String),
    SetScale(Value),
    SetBody(BodyKind),
    ApplyImpulse([Value; 3]),
    SetVelocity([Value; 3]),
    SetGravity([Value; 3]),
    Say(Value),
    SetVisible(bool),
    SetColor(Value),
    SetComponentField {
        component: String,
        field: String,
        value: Value,
    },
    SetCameraView(CameraView),
    AttachComponent(String),
    DetachComponent(String),
    Broadcast(String),
    SetVariable {
        name: String,
        value: Value,
    },
    ChangeVariable {
        name: String,
        value: Value,
    },
}

/// One step of a compiled program.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Action(Action),
    /// Falls through when `condition` holds, jumps to `to` when it doesn't.
    JumpUnless {
        condition: Value,
        to: usize,
    },
    Jump {
        to: usize,
    },
    /// Pushes a loop frame (or re-checks an existing one, when jumped back
    /// to) and jumps past `end` when the loop is done.
    LoopBegin {
        kind: LoopKind,
        end: usize,
    },
    /// The back edge. Yields the script for this frame.
    LoopEnd {
        begin: usize,
    },
    /// Leaves the nearest enclosing loop, within this invocation.
    Break,
    /// Jumps to the nearest enclosing loop's back edge.
    Continue,
    /// Suspends for a number of seconds.
    Wait(Value),
    /// Suspends until the condition holds, re-checked once a frame.
    WaitUntil(Value),
    Glide {
        seconds: Value,
        target: [Value; 3],
    },
    Call {
        block_id: String,
        args: Vec<Value>,
    },
    Return(Value),
    /// `stop all`.
    StopAll,
    /// End of a body: return from a call, or finish the script.
    End,
}

/// An actor's whole canvas, compiled.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Program {
    pub steps: Vec<Step>,
    /// Header strands that a trigger can start.
    pub entries: Vec<Entry>,
    /// Custom block id -> where its body starts.
    pub blocks: HashMap<String, usize>,
}

/// Compiles every strand on `graph`. A strand with no header block is inert -
/// it's persisted, but nothing can ever start it, so it isn't compiled.
pub fn compile(graph: &ActorGraph) -> Program {
    let mut program = Program::default();
    for strand in &graph.strands {
        let Some(header) = strand.instructions.first() else {
            continue;
        };
        let trigger = match &header.kind {
            InstructionKind::WhenStarted => Some(Trigger::Started),
            InstructionKind::WhenKeyPressed { key } => {
                Some(Trigger::KeyPressed(crate::sense::normalize_key(key)))
            }
            InstructionKind::WhenClicked => Some(Trigger::Clicked),
            InstructionKind::WhenCollision { with } => Some(Trigger::Collision {
                with: with.trim().to_string(),
            }),
            InstructionKind::WhenMessage { name } => {
                Some(Trigger::Message(name.trim().to_string()))
            }
            InstructionKind::BlockHeader { .. } => None,
            // Not a header at all: a loose stack nothing can start.
            _ => continue,
        };
        let pc = program.steps.len();
        emit_body(&mut program.steps, &strand.instructions[1..]);
        program.steps.push(Step::End);
        match (&header.kind, trigger) {
            (InstructionKind::BlockHeader { block_id }, _) => {
                program.blocks.insert(block_id.clone(), pc);
            }
            (_, Some(trigger)) => program.entries.push(Entry {
                trigger,
                strand_id: strand.id.clone(),
                pc,
            }),
            (_, None) => {}
        }
    }
    program
}

fn emit_body(steps: &mut Vec<Step>, body: &[Instruction]) {
    for instruction in body {
        emit(steps, &instruction.kind);
    }
}

fn emit(steps: &mut Vec<Step>, kind: &InstructionKind) {
    use InstructionKind as K;
    match kind {
        // Headers only ever appear at index 0 of a strand, which the caller
        // already stripped; a stray one is skipped rather than run.
        K::WhenStarted
        | K::WhenKeyPressed { .. }
        | K::WhenClicked
        | K::WhenCollision { .. }
        | K::WhenMessage { .. }
        | K::BlockHeader { .. } => {}

        K::Move { steps: amount } => steps.push(Step::Action(Action::Move(amount.clone()))),
        K::GoTo { x, y, z } => steps.push(Step::Action(Action::GoTo([
            x.clone(),
            y.clone(),
            z.clone(),
        ]))),
        K::ChangePosition { axis, by } => steps.push(Step::Action(Action::ChangePosition {
            axis: *axis,
            by: by.clone(),
        })),
        K::Glide { seconds, x, y, z } => steps.push(Step::Glide {
            seconds: seconds.clone(),
            target: [x.clone(), y.clone(), z.clone()],
        }),
        K::Turn { axis, degrees } => steps.push(Step::Action(Action::Turn {
            axis: *axis,
            degrees: degrees.clone(),
        })),
        K::SetRotation { axis, degrees } => steps.push(Step::Action(Action::SetRotation {
            axis: *axis,
            degrees: degrees.clone(),
        })),
        K::PointTowards { target } => {
            steps.push(Step::Action(Action::PointTowards(target.clone())))
        }
        K::SetScale { factor } => steps.push(Step::Action(Action::SetScale(factor.clone()))),
        K::SetBody { body } => steps.push(Step::Action(Action::SetBody(*body))),
        K::ApplyImpulse { x, y, z } => steps.push(Step::Action(Action::ApplyImpulse([
            x.clone(),
            y.clone(),
            z.clone(),
        ]))),
        K::SetVelocity { x, y, z } => steps.push(Step::Action(Action::SetVelocity([
            x.clone(),
            y.clone(),
            z.clone(),
        ]))),
        K::SetGravity { x, y, z } => steps.push(Step::Action(Action::SetGravity([
            x.clone(),
            y.clone(),
            z.clone(),
        ]))),
        K::Say { text } => steps.push(Step::Action(Action::Say(text.clone()))),
        K::SetVisible { visible } => steps.push(Step::Action(Action::SetVisible(*visible))),
        K::SetColor { color } => steps.push(Step::Action(Action::SetColor(color.clone()))),
        K::SetComponentField {
            component,
            field,
            value,
        } => steps.push(Step::Action(Action::SetComponentField {
            component: component.clone(),
            field: field.clone(),
            value: value.clone(),
        })),
        K::SetCameraView { view } => steps.push(Step::Action(Action::SetCameraView(*view))),
        K::AttachComponent { component } => steps.push(Step::Action(Action::AttachComponent(
            component.trim().to_string(),
        ))),
        K::DetachComponent { component } => steps.push(Step::Action(Action::DetachComponent(
            component.trim().to_string(),
        ))),
        K::Broadcast { name } => steps.push(Step::Action(Action::Broadcast(name.clone()))),
        K::SetVariable { name, value } => steps.push(Step::Action(Action::SetVariable {
            name: name.clone(),
            value: value.clone(),
        })),
        K::ChangeVariable { name, value } => steps.push(Step::Action(Action::ChangeVariable {
            name: name.clone(),
            value: value.clone(),
        })),
        K::Wait { duration } => steps.push(Step::Wait(duration.clone())),
        K::WaitUntil { condition } => steps.push(Step::WaitUntil(condition.clone())),
        K::CallBlock { block_id, args } => steps.push(Step::Call {
            block_id: block_id.clone(),
            args: args.clone(),
        }),
        K::Return { value } => steps.push(Step::Return(value.clone())),
        K::StopAll => steps.push(Step::StopAll),
        K::EscapeLoop => steps.push(Step::Break),
        K::ContinueLoop => steps.push(Step::Continue),

        K::If { condition, body } => {
            let jump = steps.len();
            steps.push(Step::JumpUnless {
                condition: condition.clone(),
                to: 0,
            });
            emit_body(steps, body);
            let after = steps.len();
            steps[jump] = Step::JumpUnless {
                condition: condition.clone(),
                to: after,
            };
        }
        K::IfElse {
            condition,
            then_body,
            else_body,
        } => {
            let jump = steps.len();
            steps.push(Step::JumpUnless {
                condition: condition.clone(),
                to: 0,
            });
            emit_body(steps, then_body);
            let skip_else = steps.len();
            steps.push(Step::Jump { to: 0 });
            let else_start = steps.len();
            emit_body(steps, else_body);
            let after = steps.len();
            steps[jump] = Step::JumpUnless {
                condition: condition.clone(),
                to: else_start,
            };
            steps[skip_else] = Step::Jump { to: after };
        }
        K::Repeat { count, body } => emit_loop(steps, LoopKind::Repeat(count.clone()), body),
        K::Forever { body } => emit_loop(steps, LoopKind::Forever, body),
        K::While { condition, body } => emit_loop(steps, LoopKind::While(condition.clone()), body),
    }
}

fn emit_loop(steps: &mut Vec<Step>, kind: LoopKind, body: &[Instruction]) {
    let begin = steps.len();
    steps.push(Step::LoopBegin {
        kind: kind.clone(),
        end: 0,
    });
    emit_body(steps, body);
    let end = steps.len();
    steps.push(Step::LoopEnd { begin });
    steps[begin] = Step::LoopBegin { kind, end };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::Strand;

    fn graph_with(instructions: Vec<Instruction>) -> ActorGraph {
        let mut graph = ActorGraph::new();
        graph
            .strands
            .push(Strand::with_instructions(0, 0, instructions));
        graph
    }

    #[test]
    fn a_headerless_strand_compiles_to_nothing() {
        let program = compile(&graph_with(vec![Instruction::new(InstructionKind::Move {
            steps: Value::number(1.0),
        })]));
        assert!(program.entries.is_empty());
        assert!(program.steps.is_empty());
    }

    #[test]
    fn an_if_jumps_past_its_body() {
        let program = compile(&graph_with(vec![
            Instruction::new(InstructionKind::WhenStarted),
            Instruction::new(InstructionKind::If {
                condition: Value::Bool,
                body: vec![Instruction::new(InstructionKind::Move {
                    steps: Value::number(1.0),
                })],
            }),
            Instruction::new(InstructionKind::StopAll),
        ]));
        assert_eq!(program.entries[0].trigger, Trigger::Started);
        assert_eq!(program.entries[0].pc, 0);
        match &program.steps[0] {
            // Past the one body step, onto the StopAll.
            Step::JumpUnless { to, .. } => assert_eq!(*to, 2),
            other => panic!("expected a JumpUnless, got {other:?}"),
        }
        assert_eq!(program.steps[3], Step::End);
    }

    #[test]
    fn a_loops_back_edge_points_at_its_head_and_its_head_past_its_tail() {
        let program = compile(&graph_with(vec![
            Instruction::new(InstructionKind::WhenStarted),
            Instruction::new(InstructionKind::Forever {
                body: vec![Instruction::new(InstructionKind::Move {
                    steps: Value::number(1.0),
                })],
            }),
        ]));
        assert_eq!(
            program.steps[0],
            Step::LoopBegin {
                kind: LoopKind::Forever,
                end: 2
            }
        );
        assert_eq!(program.steps[2], Step::LoopEnd { begin: 0 });
    }

    #[test]
    fn a_custom_blocks_body_is_registered_by_id_not_as_an_entry() {
        let program = compile(&graph_with(vec![
            Instruction::new(InstructionKind::BlockHeader {
                block_id: "b1".to_string(),
            }),
            Instruction::new(InstructionKind::Return {
                value: Value::number(3.0),
            }),
        ]));
        assert!(program.entries.is_empty());
        assert_eq!(program.blocks.get("b1"), Some(&0));
    }
}
