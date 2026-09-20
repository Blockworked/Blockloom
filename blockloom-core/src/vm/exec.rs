//! Running compiled programs.
//!
//! Every live script is a program counter plus a small frame stack, stepped
//! by [`Vm::tick`] once per rendered frame. A script gives the frame back at a
//! `wait`, at every loop iteration, and when it finishes - so a `forever` loop
//! costs one iteration per frame instead of hanging the host.

use super::effect::Effect;
use super::program::{Action, LoopKind, Program, Step, Trigger, compile};
use crate::project::Project;
use crate::sense;
use crate::value::{Evaluated, Value};
use std::collections::HashMap;
use std::rc::Rc;

/// Steps one script may take in a single frame before being made to yield.
/// Loops yield on their own; this only catches pathological straight-line code.
const STEP_BUDGET: usize = 10_000;

/// How deeply reporter blocks may call each other.
const MAX_REPORTER_DEPTH: usize = 32;

/// Something that can start scripts.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The green flag: every `when the project starts` strand.
    Started,
    /// A key went down, in [`sense::normalize_key`]'s spelling.
    Key(String),
    Click {
        actor: String,
    },
    /// `actor` started touching `with` (both actor ids).
    Collision {
        actor: String,
        with: String,
    },
    Message(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Status {
    Run,
    /// Asleep until this timestamp, in run seconds.
    Sleep(f64),
    Done,
}

#[derive(Debug, Clone)]
enum Frame {
    Loop {
        begin: usize,
        end: usize,
        /// Iterations left for a `repeat`; `None` for `forever`/`while`.
        remaining: Option<i64>,
    },
    Call {
        /// [`usize::MAX`] marks a reporter's own boundary - returning through
        /// it hands the value back to whoever asked instead of resuming.
        return_pc: usize,
        params: HashMap<String, Evaluated>,
    },
}

#[derive(Debug, Clone)]
struct Script {
    actor: String,
    /// `(actor, strand)` of the entry point, so re-triggering restarts the
    /// same script rather than stacking a second copy - as in Scratch.
    key: Option<(String, String)>,
    pc: usize,
    frames: Vec<Frame>,
    status: Status,
}

impl Script {
    /// A finished, empty script - the stand-in left in the list while the real
    /// one is being stepped.
    fn spent() -> Self {
        Self {
            actor: String::new(),
            key: None,
            pc: 0,
            frames: Vec::new(),
            status: Status::Done,
        }
    }
}

/// The block virtual machine: compiled programs, live scripts and variables
/// for one loaded project.
pub struct Vm {
    programs: HashMap<String, Rc<Program>>,
    /// Actor id -> name, for matching a collision against `when I touch`.
    names: HashMap<String, String>,
    /// Actor id -> custom block id -> input names, in declaration order.
    block_inputs: HashMap<String, HashMap<String, Vec<String>>>,
    globals: HashMap<String, Evaluated>,
    actor_vars: HashMap<String, HashMap<String, Evaluated>>,
    scripts: Vec<Script>,
    /// Events to start scripts for, drained at the top of the next tick.
    pending: Vec<Event>,
    now: f64,
    /// Set by `stop all`, which invalidates the script list mid-tick.
    stopping: bool,
    depth: usize,
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

impl Vm {
    pub fn new() -> Self {
        Self {
            programs: HashMap::new(),
            names: HashMap::new(),
            block_inputs: HashMap::new(),
            globals: HashMap::new(),
            actor_vars: HashMap::new(),
            scripts: Vec::new(),
            pending: Vec::new(),
            now: 0.0,
            stopping: false,
            depth: 0,
        }
    }

    /// Compiles every actor's canvas and takes the project's variables as the
    /// starting values. Drops any scripts from a previous run.
    pub fn load(&mut self, project: &Project) {
        self.programs.clear();
        self.names.clear();
        self.block_inputs.clear();
        self.actor_vars.clear();
        self.scripts.clear();
        self.pending.clear();
        self.stopping = false;
        self.globals = project
            .globals
            .iter()
            .map(|v| (v.name.clone(), v.value.clone()))
            .collect();
        for actor in &project.actors {
            self.programs
                .insert(actor.id.clone(), Rc::new(compile(&actor.graph)));
            self.names.insert(actor.id.clone(), actor.name.clone());
            self.actor_vars
                .insert(actor.id.clone(), actor.graph.variable_values());
            let inputs = actor
                .graph
                .block_defs
                .iter()
                .map(|def| {
                    (
                        def.id.clone(),
                        def.input_names().map(str::to_string).collect::<Vec<_>>(),
                    )
                })
                .collect();
            self.block_inputs.insert(actor.id.clone(), inputs);
        }
    }

    /// Queues an event. Its scripts start at the beginning of the next tick.
    pub fn fire(&mut self, event: Event) {
        self.pending.push(event);
    }

    /// True while any script is still live.
    pub fn is_running(&self) -> bool {
        self.scripts.iter().any(|s| s.status != Status::Done)
    }

    /// Ends every script, as `stop all` does.
    pub fn stop_all(&mut self) {
        self.scripts.clear();
        self.pending.clear();
    }

    /// Current variable values: the project's globals, then each actor's own
    /// by actor id - what the editor's watchers show.
    pub fn variables(
        &self,
    ) -> (
        &HashMap<String, Evaluated>,
        &HashMap<String, HashMap<String, Evaluated>>,
    ) {
        (&self.globals, &self.actor_vars)
    }

    /// Runs every live script for one frame. `now` is seconds since the run
    /// started; effects are appended to `out` in the order they happened.
    pub fn tick(&mut self, now: f64, out: &mut Vec<Effect>) {
        self.now = now;
        let events = std::mem::take(&mut self.pending);
        for event in events {
            self.start_for(event);
        }

        let mut index = 0;
        while index < self.scripts.len() {
            let mut script = std::mem::replace(&mut self.scripts[index], Script::spent());
            let Some(program) = self.programs.get(&script.actor).map(Rc::clone) else {
                index += 1;
                continue;
            };
            let actor = script.actor.clone();
            sense::with_actor(&actor, || {
                self.run(&mut script, &program, false, out);
            });
            if self.stopping {
                // `stop all` emptied the list; everything after is gone too.
                self.stopping = false;
                self.scripts.clear();
                out.push(Effect::Stopped);
                return;
            }
            self.scripts[index] = script;
            index += 1;
        }
        self.scripts.retain(|script| script.status != Status::Done);
    }

    // ─── Starting scripts ───────────────────────────────────────────────────

    fn start_for(&mut self, event: Event) {
        let matches: Vec<(String, String, usize)> = self
            .programs
            .iter()
            .flat_map(|(actor, program)| {
                program.entries.iter().filter_map(|entry| {
                    self.entry_matches(actor, &entry.trigger, &event)
                        .then(|| (actor.clone(), entry.strand_id.clone(), entry.pc))
                })
            })
            .collect();
        for (actor, strand_id, pc) in matches {
            self.start(actor, strand_id, pc);
        }
    }

    fn entry_matches(&self, actor: &str, trigger: &Trigger, event: &Event) -> bool {
        match (trigger, event) {
            (Trigger::Started, Event::Started) => true,
            (Trigger::KeyPressed(want), Event::Key(got)) => want == got,
            (Trigger::Clicked, Event::Click { actor: clicked }) => clicked == actor,
            (
                Trigger::Collision { with },
                Event::Collision {
                    actor: touched,
                    with: other,
                },
            ) => {
                touched == actor
                    && (with.is_empty()
                        || with == other
                        || self
                            .names
                            .get(other)
                            .is_some_and(|name| name.eq_ignore_ascii_case(with)))
            }
            (Trigger::Message(want), Event::Message(got)) => want == got,
            _ => false,
        }
    }

    /// Starts an entry point, restarting it if it's already running.
    fn start(&mut self, actor: String, strand_id: String, pc: usize) {
        let key = (actor.clone(), strand_id);
        let fresh = Script {
            actor,
            key: Some(key.clone()),
            pc,
            frames: Vec::new(),
            status: Status::Run,
        };
        match self
            .scripts
            .iter_mut()
            .find(|script| script.key.as_ref() == Some(&key))
        {
            Some(existing) => *existing = fresh,
            None => self.scripts.push(fresh),
        }
    }

    // ─── Stepping ───────────────────────────────────────────────────────────

    /// Steps `script` until it yields, finishes, or runs out of budget. In
    /// `immediate` mode (a reporter block's body) nothing suspends and the
    /// returned value is the block's result.
    fn run(
        &mut self,
        script: &mut Script,
        program: &Rc<Program>,
        immediate: bool,
        out: &mut Vec<Effect>,
    ) -> Option<Evaluated> {
        match script.status {
            Status::Done => return None,
            Status::Sleep(until) => {
                if self.now < until {
                    return None;
                }
                script.status = Status::Run;
            }
            Status::Run => {}
        }

        for _ in 0..STEP_BUDGET {
            let pc = script.pc;
            let Some(step) = program.steps.get(pc) else {
                script.status = Status::Done;
                return None;
            };
            match step {
                Step::Action(action) => {
                    script.pc = pc + 1;
                    let params = current_params(script);
                    self.perform(action, &script.actor, params.as_ref(), out);
                }
                Step::JumpUnless { condition, to } => {
                    let params = current_params(script);
                    let holds = self
                        .eval(condition, &script.actor, params.as_ref(), out)
                        .as_bool();
                    script.pc = if holds { pc + 1 } else { *to };
                }
                Step::Jump { to } => script.pc = *to,
                Step::LoopBegin { kind, end } => {
                    let end = *end;
                    let already_entered = matches!(
                        script.frames.last(),
                        Some(Frame::Loop { begin, .. }) if *begin == pc
                    );
                    match kind {
                        LoopKind::Repeat(count) => {
                            if already_entered {
                                script.pc = pc + 1;
                                continue;
                            }
                            let params = current_params(script);
                            let n = self
                                .eval(count, &script.actor, params.as_ref(), out)
                                .as_number()
                                .unwrap_or(0.0)
                                .round() as i64;
                            if n <= 0 {
                                script.pc = end + 1;
                                continue;
                            }
                            script.frames.push(Frame::Loop {
                                begin: pc,
                                end,
                                remaining: Some(n),
                            });
                            script.pc = pc + 1;
                        }
                        LoopKind::Forever => {
                            if !already_entered {
                                script.frames.push(Frame::Loop {
                                    begin: pc,
                                    end,
                                    remaining: None,
                                });
                            }
                            script.pc = pc + 1;
                        }
                        LoopKind::While(condition) => {
                            let params = current_params(script);
                            let holds = self
                                .eval(condition, &script.actor, params.as_ref(), out)
                                .as_bool();
                            if !holds {
                                if already_entered {
                                    script.frames.pop();
                                }
                                script.pc = end + 1;
                                continue;
                            }
                            if !already_entered {
                                script.frames.push(Frame::Loop {
                                    begin: pc,
                                    end,
                                    remaining: None,
                                });
                            }
                            script.pc = pc + 1;
                        }
                    }
                }
                Step::LoopEnd { begin } => {
                    let begin = *begin;
                    match script.frames.last_mut() {
                        Some(Frame::Loop {
                            begin: frame_begin,
                            end,
                            remaining,
                        }) if *frame_begin == begin => match remaining {
                            Some(left) => {
                                *left -= 1;
                                if *left <= 0 {
                                    let after = *end + 1;
                                    script.frames.pop();
                                    script.pc = after;
                                } else {
                                    // Straight back into the body: a `repeat`
                                    // counts its own head exactly once.
                                    script.pc = begin + 1;
                                }
                            }
                            // Back to the head, so a `while` re-checks.
                            None => script.pc = begin,
                        },
                        _ => script.pc = begin,
                    }
                    if !immediate {
                        return None;
                    }
                }
                Step::Break => match nearest_loop(script) {
                    Some((index, _, end)) => {
                        script.frames.truncate(index);
                        script.pc = end + 1;
                    }
                    None => script.pc = pc + 1,
                },
                Step::Continue => match nearest_loop(script) {
                    Some((index, _, end)) => {
                        script.frames.truncate(index + 1);
                        script.pc = end;
                    }
                    None => script.pc = pc + 1,
                },
                Step::Wait(duration) => {
                    let params = current_params(script);
                    let seconds = self
                        .eval(duration, &script.actor, params.as_ref(), out)
                        .as_number()
                        .unwrap_or(0.0)
                        .max(0.0);
                    script.pc = pc + 1;
                    if !immediate && seconds > 0.0 {
                        script.status = Status::Sleep(self.now + seconds);
                        return None;
                    }
                }
                Step::WaitUntil(condition) => {
                    let params = current_params(script);
                    let holds = self
                        .eval(condition, &script.actor, params.as_ref(), out)
                        .as_bool();
                    if holds || immediate {
                        script.pc = pc + 1;
                    } else {
                        // Leave the pc here and re-check next frame.
                        return None;
                    }
                }
                Step::Glide { seconds, target } => {
                    let params = current_params(script);
                    let seconds = self
                        .eval(seconds, &script.actor, params.as_ref(), out)
                        .as_number()
                        .unwrap_or(0.0)
                        .max(0.0);
                    let position = self.eval_vec3(target, &script.actor, params.as_ref(), out);
                    out.push(Effect::Glide {
                        actor: script.actor.clone(),
                        seconds: seconds as f32,
                        target: position,
                    });
                    script.pc = pc + 1;
                    if !immediate && seconds > 0.0 {
                        script.status = Status::Sleep(self.now + seconds);
                        return None;
                    }
                }
                Step::Call { block_id, args } => {
                    let Some(&start) = program.blocks.get(block_id) else {
                        script.pc = pc + 1;
                        continue;
                    };
                    let params = current_params(script);
                    let bound =
                        self.bind_params(&script.actor, block_id, args, params.as_ref(), out);
                    script.frames.push(Frame::Call {
                        return_pc: pc + 1,
                        params: bound,
                    });
                    script.pc = start;
                }
                Step::Return(value) => {
                    let params = current_params(script);
                    let result = self.eval(value, &script.actor, params.as_ref(), out);
                    match nearest_call(script) {
                        Some((index, return_pc)) if return_pc != usize::MAX => {
                            script.frames.truncate(index);
                            script.pc = return_pc;
                        }
                        // Either a reporter's own boundary or a top-level
                        // `return`: both end this script here.
                        _ => {
                            script.status = Status::Done;
                            return Some(result);
                        }
                    }
                }
                Step::StopAll => {
                    script.status = Status::Done;
                    self.stopping = true;
                    self.pending.clear();
                    return None;
                }
                Step::End => match nearest_call(script) {
                    Some((index, return_pc)) if return_pc != usize::MAX => {
                        script.frames.truncate(index);
                        script.pc = return_pc;
                    }
                    _ => {
                        script.status = Status::Done;
                        return None;
                    }
                },
            }
        }
        None
    }

    /// Applies one leaf instruction: either a world change, handed to the
    /// host as an [`Effect`], or a variable write, kept here.
    fn perform(
        &mut self,
        action: &Action,
        actor: &str,
        params: Option<&HashMap<String, Evaluated>>,
        out: &mut Vec<Effect>,
    ) {
        let owner = actor.to_string();
        match action {
            Action::Move(steps) => {
                let steps = self.eval_f32(steps, actor, params, out);
                out.push(Effect::Move {
                    actor: owner,
                    steps,
                });
            }
            Action::GoTo(target) => {
                let position = self.eval_vec3(target, actor, params, out);
                out.push(Effect::GoTo {
                    actor: owner,
                    position,
                });
            }
            Action::ChangePosition { axis, by } => {
                let by = self.eval_f32(by, actor, params, out);
                out.push(Effect::ChangePosition {
                    actor: owner,
                    axis: *axis,
                    by,
                });
            }
            Action::Turn { axis, degrees } => {
                let degrees = self.eval_f32(degrees, actor, params, out);
                out.push(Effect::Turn {
                    actor: owner,
                    axis: *axis,
                    degrees,
                });
            }
            Action::SetRotation { axis, degrees } => {
                let degrees = self.eval_f32(degrees, actor, params, out);
                out.push(Effect::SetRotation {
                    actor: owner,
                    axis: *axis,
                    degrees,
                });
            }
            Action::PointTowards(target) => out.push(Effect::PointTowards {
                actor: owner,
                target: target.clone(),
            }),
            Action::SetScale(factor) => {
                let factor = self.eval_f32(factor, actor, params, out);
                out.push(Effect::SetScale {
                    actor: owner,
                    factor,
                });
            }
            Action::SetBody(body) => out.push(Effect::SetBody {
                actor: owner,
                body: *body,
            }),
            Action::ApplyImpulse(vector) => {
                let impulse = self.eval_vec3(vector, actor, params, out);
                out.push(Effect::ApplyImpulse {
                    actor: owner,
                    impulse,
                });
            }
            Action::SetVelocity(vector) => {
                let velocity = self.eval_vec3(vector, actor, params, out);
                out.push(Effect::SetVelocity {
                    actor: owner,
                    velocity,
                });
            }
            Action::SetGravity(vector) => {
                let gravity = self.eval_vec3(vector, actor, params, out);
                out.push(Effect::SetGravity { gravity });
            }
            Action::Say(text) => {
                let text = self.eval(text, actor, params, out).as_text();
                out.push(Effect::Say { actor: owner, text });
            }
            Action::SetVisible(visible) => out.push(Effect::SetVisible {
                actor: owner,
                visible: *visible,
            }),
            Action::SetColor(color) => {
                let color = self.eval(color, actor, params, out).as_text();
                out.push(Effect::SetColor {
                    actor: owner,
                    color,
                });
            }
            Action::Broadcast(name) => self.pending.push(Event::Message(name.trim().to_string())),
            Action::SetVariable { name, value } => {
                let value = self.eval(value, actor, params, out);
                self.write_var(actor, name, value);
            }
            Action::ChangeVariable { name, value } => {
                let delta = self
                    .eval(value, actor, params, out)
                    .as_number()
                    .unwrap_or(0.0);
                // A non-numeric variable counts as zero, as in Scratch.
                let current = self.read_var(actor, name).as_number().unwrap_or(0.0);
                self.write_var(actor, name, Evaluated::Number(current + delta));
            }
        }
    }

    // ─── Variables ──────────────────────────────────────────────────────────

    fn read_var(&self, actor: &str, name: &str) -> Evaluated {
        self.actor_vars
            .get(actor)
            .and_then(|vars| vars.get(name))
            .or_else(|| self.globals.get(name))
            .cloned()
            .unwrap_or(Evaluated::Number(0.0))
    }

    /// Writes to the actor's own variable when it has one by that name, the
    /// project global when it doesn't, and otherwise declares it on the actor.
    fn write_var(&mut self, actor: &str, name: &str, value: Evaluated) {
        if let Some(slot) = self
            .actor_vars
            .get_mut(actor)
            .and_then(|vars| vars.get_mut(name))
        {
            *slot = value;
            return;
        }
        if let Some(slot) = self.globals.get_mut(name) {
            *slot = value;
            return;
        }
        self.actor_vars
            .entry(actor.to_string())
            .or_default()
            .insert(name.to_string(), value);
    }

    // ─── Evaluation ─────────────────────────────────────────────────────────

    /// Replaces every variable read, bound parameter and reporter-block call
    /// in `value` with a literal, then evaluates the result. An error is
    /// reported once and stands in as `0`, so one bad slot never kills a run.
    fn eval(
        &mut self,
        value: &Value,
        actor: &str,
        params: Option<&HashMap<String, Evaluated>>,
        out: &mut Vec<Effect>,
    ) -> Evaluated {
        let resolved = self.resolve(value, actor, params, out);
        match resolved.eval() {
            Ok(evaluated) => evaluated,
            Err(message) => {
                out.push(Effect::Error {
                    actor: actor.to_string(),
                    message,
                });
                Evaluated::Number(0.0)
            }
        }
    }

    fn eval_f32(
        &mut self,
        value: &Value,
        actor: &str,
        params: Option<&HashMap<String, Evaluated>>,
        out: &mut Vec<Effect>,
    ) -> f32 {
        self.eval(value, actor, params, out)
            .as_number()
            .unwrap_or(0.0) as f32
    }

    fn eval_vec3(
        &mut self,
        values: &[Value; 3],
        actor: &str,
        params: Option<&HashMap<String, Evaluated>>,
        out: &mut Vec<Effect>,
    ) -> [f32; 3] {
        [
            self.eval_f32(&values[0], actor, params, out),
            self.eval_f32(&values[1], actor, params, out),
            self.eval_f32(&values[2], actor, params, out),
        ]
    }

    fn resolve(
        &mut self,
        value: &Value,
        actor: &str,
        params: Option<&HashMap<String, Evaluated>>,
        out: &mut Vec<Effect>,
    ) -> Value {
        match value {
            Value::Number { .. } | Value::Text { .. } | Value::Bool => value.clone(),
            Value::Var { name } => self.read_var(actor, name).into_value(),
            Value::Param { name } => params
                .and_then(|bound| bound.get(name))
                .cloned()
                .unwrap_or(Evaluated::Number(0.0))
                .into_value(),
            Value::Op { op, args, saved } => Value::Op {
                op: op.clone(),
                args: args
                    .iter()
                    .map(|arg| self.resolve(arg, actor, params, out))
                    .collect(),
                saved: saved.clone(),
            },
            Value::Call { block_id, args, .. } => self
                .run_reporter(actor, block_id, args, params, out)
                .into_value(),
        }
    }

    /// Runs a reporter-shaped custom block's body to completion, right here,
    /// and takes its `return` as the value. A `wait` inside one has nothing to
    /// suspend, so it passes straight through.
    fn run_reporter(
        &mut self,
        actor: &str,
        block_id: &str,
        args: &[Value],
        params: Option<&HashMap<String, Evaluated>>,
        out: &mut Vec<Effect>,
    ) -> Evaluated {
        if self.depth >= MAX_REPORTER_DEPTH {
            out.push(Effect::Error {
                actor: actor.to_string(),
                message: "a reporter block calls itself too deeply".to_string(),
            });
            return Evaluated::Number(0.0);
        }
        let Some(program) = self.programs.get(actor).map(Rc::clone) else {
            return Evaluated::Number(0.0);
        };
        let Some(&start) = program.blocks.get(block_id) else {
            return Evaluated::Number(0.0);
        };
        let bound = self.bind_params(actor, block_id, args, params, out);
        let mut script = Script {
            actor: actor.to_string(),
            key: None,
            pc: start,
            frames: vec![Frame::Call {
                return_pc: usize::MAX,
                params: bound,
            }],
            status: Status::Run,
        };
        self.depth += 1;
        let result = self.run(&mut script, &program, true, out);
        self.depth -= 1;
        result.unwrap_or(Evaluated::Number(0.0))
    }

    /// Binds a call's arguments to the block's declared input names.
    fn bind_params(
        &mut self,
        actor: &str,
        block_id: &str,
        args: &[Value],
        params: Option<&HashMap<String, Evaluated>>,
        out: &mut Vec<Effect>,
    ) -> HashMap<String, Evaluated> {
        let names = self
            .block_inputs
            .get(actor)
            .and_then(|blocks| blocks.get(block_id))
            .cloned()
            .unwrap_or_default();
        names
            .iter()
            .zip(args)
            .map(|(name, arg)| (name.clone(), self.eval(arg, actor, params, out)))
            .collect()
    }
}

/// The parameters bound by the innermost custom-block call, if any.
fn current_params(script: &Script) -> Option<HashMap<String, Evaluated>> {
    script.frames.iter().rev().find_map(|frame| match frame {
        Frame::Call { params, .. } => Some(params.clone()),
        Frame::Loop { .. } => None,
    })
}

/// `(frame index, begin, end)` of the nearest enclosing loop - not looking
/// past a call boundary, since a loop can't be escaped from inside a block.
fn nearest_loop(script: &Script) -> Option<(usize, usize, usize)> {
    for (index, frame) in script.frames.iter().enumerate().rev() {
        match frame {
            Frame::Loop { begin, end, .. } => return Some((index, *begin, *end)),
            Frame::Call { .. } => return None,
        }
    }
    None
}

/// `(frame index, return pc)` of the innermost call frame.
fn nearest_call(script: &Script) -> Option<(usize, usize)> {
    script
        .frames
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, frame)| match frame {
            Frame::Call { return_pc, .. } => Some((index, *return_pc)),
            Frame::Loop { .. } => None,
        })
}
