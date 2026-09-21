//! Compiling a project's blocks into Rust.
//!
//! The VM walks a `Value` tree and matches on a `Step` every time a block
//! runs. This emits the same program as Rust source instead: an expression
//! becomes an expression, a variable read becomes a call rather than a hash
//! lookup, and an actor's canvas becomes a function. Built with the same
//! no-Cargo `rustc` pipeline the scripts use (`crate::script`), a game's
//! logic ends up as native code beside them.
//!
//! The rule the whole thing has to keep is that a compiled program and the VM
//! ask the world for exactly the same things in exactly the same order -
//! including the mistakes, since a bad slot reports itself once and stands a
//! zero in its place. `blockloom-core/tests/codegen.rs` is that rule: it runs
//! a project both ways and compares, line for line, what each one asked for.
//!
//! Anything that reads the world or the clock is a question for the host:
//! sensing reporters, but also `random` and `current time`, so one run of a
//! game has one source of both rather than two that disagree.
//!
//! A strand can be suspended, so it can't be a straight run of Rust: it comes
//! out as a `match` over the program counter, over the very same flattened
//! program the VM steps. Straight-line blocks are fused into one arm, and
//! every point the VM can give the frame back - a `wait`, a `glide`, each
//! iteration of a loop - is an arm of its own that picks up where it left
//! off. Loop nesting is known here rather than at run time, so a `repeat`
//! keeps its tally in a flat slot and a `break` is a jump, where the VM
//! needs a frame stack.
//!
//! One function covers a whole actor rather than one strand, because every
//! strand and every custom block body live in one step list with one set of
//! numbers, and a custom block called as a statement is a jump into somebody
//! else's region. The VM's `immediate` flag - a reporter body run to
//! completion in place - becomes a second function over the same steps, so
//! each actor is emitted at most twice however many reporters it has.
//!
//! The one place the two deliberately part company is the VM's per-tick step
//! budget, which a compiled strand has no counter for. Every back edge is a
//! loop's, and every loop yields, so a compiled strand can't spin - the
//! budget only ever catches a strand of ten thousand straight-line blocks,
//! and paying for a counter on every block to match it there would cost the
//! whole point of compiling. A reporter body is the exception and does count,
//! since nothing in it yields and the budget is all that ends a runaway one.
//!
//! What it won't compile is a custom block that can reach itself through
//! statement calls: [`Unsupported`] refuses the project rather than emitting
//! half of it, so a build can fall back to the VM knowing exactly why.

// Half of this is only ever used by the programs it is pasted into.
#[allow(dead_code)]
mod runtime;

pub use runtime::{
    ABI_MISSING, ABI_OK, ABI_PANIC, ABI_TOO_LONG, ACT_APPLY_IMPULSE, ACT_ATTACH, ACT_BROADCAST,
    ACT_CHANGE_POSITION, ACT_DETACH, ACT_ERROR, ACT_GLIDE, ACT_GO_TO, ACT_MOVE, ACT_POINT_TOWARDS,
    ACT_SAY, ACT_SET_BODY, ACT_SET_CAMERA_VIEW, ACT_SET_COLOR, ACT_SET_DENSITY, ACT_SET_FIELD,
    ACT_SET_GRAVITY, ACT_SET_MASS, ACT_SET_ROTATION, ACT_SET_SCALE, ACT_SET_VELOCITY,
    ACT_SET_VISIBLE, ACT_TURN, AbiStr, AbiValue, Act, Entry, Host, LOGIC_ABI_VERSION, LogicHostApi,
    R, READ_SENSE, READ_VARIABLE, Runner, SYM_LOGIC_ABI, SYM_LOGIC_FIRE, SYM_LOGIC_FREE,
    SYM_LOGIC_NEW, SYM_LOGIC_RESET, SYM_LOGIC_TICK, State, Status, TICK_STOPPED, VALUE_BOOL,
    VALUE_ERROR, VALUE_NUMBER, VALUE_TEXT, Val,
};

use crate::project::Project;
use crate::value::{Op, Value};
use crate::vm::{Action, LoopKind, Program, Step, compile as compile_program};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The support code every generated program is built on, pasted in whole -
/// there is no Cargo behind the compile, so there is nothing to depend on.
const RUNTIME_SOURCE: &str = include_str!("runtime.rs");

const EXPORT_SOURCE: &str = r#"
#[unsafe(no_mangle)]
pub extern "C" fn blockloom_logic_abi() -> u32 {
    LOGIC_ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn blockloom_logic_new() -> *mut std::ffi::c_void {
    Box::into_raw(Box::new(Runner::new())).cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn blockloom_logic_free(state: *mut std::ffi::c_void) {
    if !state.is_null() {
        drop(unsafe { Box::from_raw(state.cast::<Runner>()) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn blockloom_logic_reset(state: *mut std::ffi::c_void) {
    if let Some(runner) = unsafe { state.cast::<Runner>().as_mut() } {
        runner.reset();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn blockloom_logic_fire(
    state: *mut std::ffi::c_void,
    kind: AbiStr,
    actor: AbiStr,
    detail: AbiStr,
    other_name: AbiStr,
) {
    let Some(runner) = (unsafe { state.cast::<Runner>().as_mut() }) else {
        return;
    };
    runner.fire(
        ENTRIES,
        unsafe { kind.as_str() },
        unsafe { actor.as_str() },
        unsafe { detail.as_str() },
        unsafe { other_name.as_str() },
    );
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn blockloom_logic_tick(
    state: *mut std::ffi::c_void,
    ctx: *mut std::ffi::c_void,
    api: *const LogicHostApi,
    now: f64,
) -> u32 {
    let Some(runner) = (unsafe { state.cast::<Runner>().as_mut() }) else {
        return ABI_PANIC;
    };
    let Some(api_ref) = (unsafe { api.as_ref() }) else {
        return ABI_PANIC;
    };
    if api_ref.abi != LOGIC_ABI_VERSION {
        return ABI_PANIC;
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut host = unsafe { AbiHost::new(ctx, api) };
        runner.tick(ENTRIES, &mut host, now)
    })) {
        Ok(true) => TICK_STOPPED,
        Ok(false) => ABI_OK,
        Err(_) => ABI_PANIC,
    }
}
"#;

/// A block this doesn't compile. Naming it is the point: a build that falls
/// back to the VM says which block sent it there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported {
    pub what: String,
}

impl Unsupported {
    fn new(what: impl Into<String>) -> Self {
        Self { what: what.into() }
    }
}

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} can't be compiled", self.what)
    }
}

type Emit<T> = Result<T, Unsupported>;

pub const LOGIC_STEM: &str = "blockloom_logic";

pub fn library_path(project_dir: &Path) -> PathBuf {
    library_path_for(project_dir, None)
}

pub fn library_path_for(project_dir: &Path, target: Option<&str>) -> PathBuf {
    crate::script::build_dir_for(project_dir, target)
        .join(crate::script::dylib_name(LOGIC_STEM, target))
}

/// Compiles the generated state machines as one native shared library.
pub fn compile_for(
    project: &Project,
    project_dir: &Path,
    target: Option<&str>,
) -> Result<PathBuf, String> {
    let source = compile(project).map_err(|error| error.to_string())?;
    let toolchain = crate::script::toolchain_version()?;
    if let Some(triple) = target {
        crate::script::target_installed(triple)?;
    }
    let build = crate::script::build_dir_for(project_dir, target);
    std::fs::create_dir_all(&build).map_err(|error| format!("{}: {error}", build.display()))?;
    let source_path = build.join(format!("{LOGIC_STEM}.rs"));
    let library = library_path_for(project_dir, target);
    let stamp = build.join(format!("{LOGIC_STEM}.stamp"));
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    let wanted = format!(
        "logic abi {LOGIC_ABI_VERSION}\n{toolchain}\ntarget {}\nprofile opt3-lto-fat-cu1\nsource {:016x}\n",
        target.unwrap_or("host"),
        hasher.finish()
    );
    if library.is_file() && std::fs::read_to_string(&stamp).is_ok_and(|previous| previous == wanted)
    {
        return Ok(library);
    }
    std::fs::write(&source_path, source)
        .map_err(|error| format!("{}: {error}", source_path.display()))?;
    let mut command = Command::new("rustc");
    if let Some(triple) = target {
        command.arg("--target").arg(triple);
    }
    let output = command
        .arg("--edition")
        .arg("2024")
        .arg("--crate-type")
        .arg("cdylib")
        .arg("--crate-name")
        .arg(LOGIC_STEM)
        .arg("-C")
        .arg("opt-level=3")
        .arg("-C")
        .arg("codegen-units=1")
        .arg("-C")
        .arg("lto=fat")
        .arg("-o")
        .arg(&library)
        .arg(&source_path)
        .output()
        .map_err(|error| format!("couldn't run rustc: {error}"))?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&stamp);
        return Err(format!(
            "Blockloom's generated logic didn't compile:\n{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    std::fs::write(&stamp, wanted).map_err(|error| format!("{}: {error}", stamp.display()))?;
    Ok(library)
}

/// Compiles every actor's canvas into one Rust source file, or names the
/// first thing that stopped it.
pub fn compile(project: &Project) -> Emit<String> {
    let mut entries = Vec::new();
    let mut bodies = String::new();

    for (index, actor) in project.actors.iter().enumerate() {
        let program = compile_program(&actor.graph);
        if program.steps.is_empty() {
            continue;
        }
        let canvas = Canvas::of(&actor.id, index, &program, &actor.graph)?;

        bodies.push_str(&Pass::new(&canvas, false).emit()?);
        // A reporter runs on a state of its own, so its mode is only worth
        // emitting for an actor that has a custom block to call.
        if !program.blocks.is_empty() {
            bodies.push_str(&Pass::new(&canvas, true).emit()?);
        }

        let counters = canvas.plan.counters.len();
        for entry in &program.entries {
            entries.push(format!(
                "    Entry {{ actor: {}, strand: {}, trigger: {}, detail: {}, \
                 start: {}, counters: {counters}, run: actor_{index} }},",
                literal(&actor.id),
                literal(&entry.strand_id),
                literal(trigger_name(&entry.trigger)),
                literal(&trigger_detail(&entry.trigger)),
                entry.pc,
            ));
        }
    }

    Ok(format!(
        "// Generated by Blockloom from {}. Rebuilt on every build; don't edit.\n\
         #![allow(unused, clippy::all)]\n\n\
         {RUNTIME_SOURCE}\n\
         pub static ENTRIES: &[Entry] = &[\n{}\n];\n\n{bodies}\n{EXPORT_SOURCE}",
        literal(&project.name),
        entries.join("\n")
    ))
}

/// What `trigger`'s [`Entry`] says, in the two halves the runtime matches on.
fn trigger_name(trigger: &crate::vm::Trigger) -> &'static str {
    use crate::vm::Trigger;
    match trigger {
        Trigger::Started => "Started",
        Trigger::KeyPressed(_) => "Key",
        Trigger::Clicked => "Clicked",
        Trigger::Collision { .. } => "Collision",
        Trigger::Message(_) => "Message",
    }
}

fn trigger_detail(trigger: &crate::vm::Trigger) -> String {
    use crate::vm::Trigger;
    match trigger {
        Trigger::KeyPressed(key) => key.clone(),
        Trigger::Collision { with } => with.clone(),
        Trigger::Message(name) => name.clone(),
        Trigger::Started | Trigger::Clicked => String::new(),
    }
}

/// How far into a generated function an arm's own statements sit.
const PAD: &str = "                ";

// ─── What the emitter knows that the step list doesn't say ──────────────────

/// One actor's program, plus everything the emitter has to work out about it
/// before writing a line.
struct Canvas<'a> {
    id: &'a str,
    index: usize,
    program: &'a Program,
    /// Custom block id -> its input names, in prototype order: the positional
    /// key a call site's arguments line up against.
    inputs: HashMap<&'a str, Vec<&'a str>>,
    /// Step index -> the custom block whose body it is in. A parameter is
    /// read back by position, and the position is a fact about that block.
    owner: HashMap<usize, &'a str>,
    plan: Plan,
}

impl<'a> Canvas<'a> {
    fn of(
        id: &'a str,
        index: usize,
        program: &'a Program,
        graph: &'a crate::blocks::ActorGraph,
    ) -> Emit<Self> {
        let inputs = graph
            .block_defs
            .iter()
            .map(|def| (def.id.as_str(), def.input_names().collect()))
            .collect();
        let owner = owners(program);
        refuse_recursion(program, &owner)?;
        Ok(Self {
            id,
            index,
            program,
            inputs,
            owner,
            plan: Plan::of(program),
        })
    }

    /// How many inputs a call to `block` actually binds. The VM zips names
    /// against arguments, so the shorter of the two is the whole of it.
    fn bound(&self, block: &str) -> usize {
        self.inputs.get(block).map_or(0, |names| names.len())
    }
}

/// Which custom block each step belongs to, for reading a parameter back.
fn owners(program: &Program) -> HashMap<usize, &str> {
    let mut owner = HashMap::new();
    for (id, &start) in &program.blocks {
        for (pc, step) in program.steps.iter().enumerate().skip(start) {
            owner.insert(pc, id.as_str());
            if matches!(step, Step::End) {
                break;
            }
        }
    }
    owner
}

/// Refuses a custom block that can reach itself through statement calls.
///
/// Its loops would share one set of counters between the outer call and the
/// inner one, where the VM gives every invocation a frame of its own.
/// Reporters may recurse all they like: each runs on a state of its own, the
/// same way the VM builds a fresh script for one.
fn refuse_recursion(program: &Program, owner: &HashMap<usize, &str>) -> Emit<()> {
    let mut calls: HashMap<&str, Vec<&str>> = HashMap::new();
    for (pc, step) in program.steps.iter().enumerate() {
        if let (Step::Call { block_id, .. }, Some(from)) = (step, owner.get(&pc)) {
            calls.entry(from).or_default().push(block_id.as_str());
        }
    }

    for &start in calls.keys() {
        let mut seen = HashSet::new();
        let mut stack = vec![start];
        while let Some(block) = stack.pop() {
            for &next in calls.get(block).into_iter().flatten() {
                if next == start {
                    return Err(Unsupported::new("a custom block that calls itself"));
                }
                if seen.insert(next) {
                    stack.push(next);
                }
            }
        }
    }
    Ok(())
}

/// Where each arm begins, which loop a `break` leaves, and which slot a
/// `repeat` counts down in - all of it settled before a line is written.
struct Plan {
    /// Every step an arm starts at, in order: the ways in, both sides of a
    /// branch, and every point a strand can be resumed at.
    leaders: Vec<usize>,
    /// Step index -> the `LoopEnd` of the loop around it. The VM finds this
    /// on its frame stack; here it's a fact about the source.
    enclosing: HashMap<usize, usize>,
    /// `LoopBegin` index -> its counter slot. `repeat` only; the other two
    /// kinds have nothing to remember.
    counters: HashMap<usize, usize>,
}

impl Plan {
    fn of(program: &Program) -> Self {
        let steps = &program.steps;
        // Every way into the program: a trigger's strand, and a custom
        // block's body, which a `Call` jumps straight at.
        let mut leaders: BTreeSet<usize> = program.entries.iter().map(|entry| entry.pc).collect();
        leaders.extend(program.blocks.values().copied());
        let mut enclosing = HashMap::new();
        let mut counters = HashMap::new();
        let mut open: Vec<usize> = Vec::new();

        for (pc, step) in steps.iter().enumerate() {
            match step {
                Step::Jump { to } | Step::JumpUnless { to, .. } => {
                    leaders.extend([*to, pc + 1]);
                }
                Step::LoopBegin { kind, end: tail } => {
                    // The head is a jump target of its own: `forever` and
                    // `while` bounce off it to decide on another go round.
                    leaders.extend([pc, pc + 1, tail + 1]);
                    if matches!(kind, LoopKind::Repeat(_)) {
                        counters.insert(pc, counters.len());
                    }
                    open.push(*tail);
                }
                Step::LoopEnd { begin } => {
                    // The back edge is where a loop yields, and `continue`
                    // jumps straight at it.
                    leaders.extend([pc, *begin, begin + 1, pc + 1]);
                    open.pop();
                }
                // Each of these hands the frame back, or can be jumped past.
                // A `Call` is here for its return address, which has to be an
                // arm the callee's `End` can land on.
                Step::Wait(_)
                | Step::Glide { .. }
                | Step::Break
                | Step::Continue
                | Step::Return(_)
                | Step::StopAll
                | Step::Call { .. } => {
                    leaders.insert(pc + 1);
                }
                // Re-checked from the top every frame until it holds, so it
                // is its own resume point.
                Step::WaitUntil(_) => {
                    leaders.extend([pc, pc + 1]);
                }
                // One region's end. Loops never span two, so nothing is left
                // open across the boundary.
                Step::End => {
                    open.clear();
                    leaders.insert(pc + 1);
                }
                Step::Action(_) => {}
            }
            if let Some(&tail) = open.last() {
                enclosing.insert(pc, tail);
            }
        }

        Self {
            leaders: leaders.into_iter().filter(|&pc| pc < steps.len()).collect(),
            enclosing,
            counters,
        }
    }
}

// ─── Writing it out ─────────────────────────────────────────────────────────

/// One emission of an actor's program, in one of the two modes `Vm::run` has.
struct Pass<'a> {
    canvas: &'a Canvas<'a>,
    /// The VM's `immediate`: a reporter's body runs to completion in place,
    /// so nothing suspends and a step budget is what stops it running away.
    immediate: bool,
    /// The custom block the step being written belongs to, if any.
    owner: Option<&'a str>,
    /// Numbers the `let`s the resolve phase leaves behind.
    bindings: usize,
}

impl<'a> Pass<'a> {
    fn new(canvas: &'a Canvas<'a>, immediate: bool) -> Self {
        Self {
            canvas,
            immediate,
            owner: None,
            bindings: 0,
        }
    }

    /// The state a `stop all` has to reach and a reporter has to be handed:
    /// the caller's, which in immediate mode isn't the one being stepped.
    fn outer(&self) -> &'static str {
        if self.immediate { "top" } else { "s" }
    }

    /// The VM's `depth` where this code runs. A strand is always at the top.
    fn depth(&self) -> &'static str {
        if self.immediate { "depth" } else { "0" }
    }

    fn emit(&mut self) -> Emit<String> {
        let steps = &self.canvas.program.steps;
        // A reporter body is written one step to an arm: the budget is
        // checked between steps, exactly where the VM checks it.
        let leaders: Vec<usize> = if self.immediate {
            (0..steps.len()).collect()
        } else {
            self.canvas.plan.leaders.clone()
        };

        let mut arms = String::new();
        for (index, &leader) in leaders.iter().enumerate() {
            let next = leaders.get(index + 1).copied().unwrap_or(steps.len());
            let mut body = String::new();
            let mut pc = leader;
            let mut falls_through = true;
            while pc < next {
                body.push_str(&self.emit_step(pc)?);
                // Only a plain action leaves the program counter to the arm.
                if !matches!(steps[pc], Step::Action(_)) {
                    falls_through = false;
                    break;
                }
                pc += 1;
            }
            if falls_through {
                body.push_str(&format!("{PAD}s.pc = {next};\n"));
            }
            arms.push_str(&format!(
                "            {leader} => {{\n{body}            }}\n"
            ));
        }

        let me = literal(self.canvas.id);
        let index = self.canvas.index;
        Ok(if self.immediate {
            format!(
                "/// A custom block of this actor's, run to completion in place as a\n\
                 /// reporter. Its own state, so one of these may call another.\n\
                 fn reporter_{index}(\n    \
                 h: &mut dyn Host,\n    top: &mut State,\n    start: usize,\n    \
                 depth: usize,\n    args: Vec<Val>,\n) -> Val {{\n    \
                 const ME: &str = {me};\n    \
                 let mut s = State::new(start, {});\n    \
                 s.enter_call(State::RETURN, args);\n    \
                 let mut budget = STEP_BUDGET;\n    \
                 loop {{\n        \
                 if budget == 0 {{\n            return Val::Num(0.0);\n        }}\n        \
                 budget -= 1;\n        match s.pc {{\n{arms}            \
                 _ => return Val::Num(0.0),\n        }}\n    }}\n}}\n\n",
                self.canvas.plan.counters.len()
            )
        } else {
            format!(
                "fn actor_{index}(h: &mut dyn Host, s: &mut State) {{\n    \
                 const ME: &str = {me};\n    \
                 if !s.resume() {{\n        return;\n    }}\n    \
                 loop {{\n        match s.pc {{\n{arms}            \
                 _ => {{\n                s.finish();\n                return;\n            }}\n        \
                 }}\n    }}\n}}\n\n"
            )
        })
    }

    /// One step, as the statements that carry it out and say where to go
    /// next. Everything but an action sets `s.pc` itself, and so ends its arm.
    fn emit_step(&mut self, pc: usize) -> Emit<String> {
        self.owner = self.canvas.owner.get(&pc).copied();
        let steps = &self.canvas.program.steps;
        let next = pc + 1;
        Ok(match &steps[pc] {
            Step::Action(action) => nested(&self.emit_action(action)?),
            Step::Jump { to } => format!("{PAD}s.pc = {to};\n"),
            Step::JumpUnless { condition, to } => format!(
                "{PAD}let holds = {};\n{PAD}s.pc = if holds {{ {next} }} else {{ {to} }};\n",
                self.boolean(condition)?
            ),
            Step::LoopBegin { kind, end } => {
                let end = *end;
                match kind {
                    // Counted once, on the way in - the back edge lands past
                    // the head.
                    LoopKind::Repeat(count) => {
                        let slot = self.canvas.plan.counters[&pc];
                        format!(
                            "{PAD}let n = {}.round() as i64;\n\
                             {PAD}if n <= 0 {{\n{PAD}    s.pc = {};\n{PAD}}} else {{\n\
                             {PAD}    s.enter_repeat({slot}, n);\n{PAD}    s.pc = {next};\n{PAD}}}\n",
                            self.number_f64(count)?,
                            end + 1
                        )
                    }
                    LoopKind::Forever => format!("{PAD}s.pc = {next};\n"),
                    // Re-checked here every time round, which is what the
                    // back edge pointing at the head is for.
                    LoopKind::While(condition) => format!(
                        "{PAD}let holds = {};\n{PAD}s.pc = if holds {{ {next} }} else {{ {} }};\n",
                        self.boolean(condition)?,
                        end + 1
                    ),
                }
            }
            // The per-iteration yield, and the only one a `forever` reaches.
            Step::LoopEnd { begin } => {
                let begin = *begin;
                let jump = match self.canvas.plan.counters.get(&begin) {
                    Some(slot) => format!(
                        "{PAD}let left = s.next_iteration({slot});\n\
                         {PAD}s.pc = if left <= 0 {{ {next} }} else {{ {} }};\n",
                        begin + 1
                    ),
                    None => format!("{PAD}s.pc = {begin};\n"),
                };
                format!("{jump}{}", self.yield_frame())
            }
            // Outside a loop both are no-ops, exactly as the VM leaves them.
            Step::Break => match self.canvas.plan.enclosing.get(&pc) {
                Some(tail) => format!("{PAD}s.pc = {};\n", tail + 1),
                None => format!("{PAD}s.pc = {next};\n"),
            },
            Step::Continue => match self.canvas.plan.enclosing.get(&pc) {
                Some(tail) => format!("{PAD}s.pc = {tail};\n"),
                None => format!("{PAD}s.pc = {next};\n"),
            },
            // A zero-second wait doesn't give the frame back, so a `wait 0`
            // stays the way to do nothing rather than a way to yield.
            Step::Wait(duration) => format!(
                "{PAD}let seconds = {}.max(0.0);\n{PAD}s.pc = {next};\n{}",
                self.number_f64(duration)?,
                self.sleep()
            ),
            Step::WaitUntil(condition) => {
                let holds = self.boolean(condition)?;
                if self.immediate {
                    // Nothing to wait on in a reporter: the condition is read
                    // for its complaints and then passed straight through.
                    format!("{PAD}let holds = {holds};\n{PAD}s.pc = {next};\n")
                } else {
                    format!(
                        "{PAD}let holds = {holds};\n{PAD}if !holds {{\n{PAD}    return;\n{PAD}}}\n\
                         {PAD}s.pc = {next};\n"
                    )
                }
            }
            // The slide is the host's to draw; the strand just sleeps as long.
            Step::Glide { seconds, target } => {
                let seconds = self.number_f64(seconds)?;
                let target = self.vec3(target)?;
                format!(
                    "{PAD}let seconds = {seconds}.max(0.0);\n{PAD}let target = {target};\n\
                     {PAD}h.act(ME, Act::Glide {{ seconds: seconds as f32, target }});\n\
                     {PAD}s.pc = {next};\n{}",
                    self.sleep()
                )
            }
            Step::Call { block_id, args } => {
                let target = self.canvas.program.blocks.get(block_id.as_str()).copied();
                match target {
                    // Nothing by that id: skipped where it stands, and its
                    // arguments go unevaluated with it.
                    None => format!("{PAD}s.pc = {next};\n"),
                    Some(start) => {
                        let bound = self.canvas.bound(block_id);
                        let mut args = self.arguments(args, bound)?;
                        args = format!("vec![{args}]");
                        format!(
                            "{PAD}let args = {args};\n{PAD}s.enter_call({next}, args);\n\
                             {PAD}s.pc = {start};\n"
                        )
                    }
                }
            }
            // The slot is evaluated whoever is listening, so a bad one still
            // reports itself even when the value goes nowhere.
            Step::Return(value) => {
                let result = self.evaluated(value)?;
                format!("{PAD}let result = {result};\n{}", self.leave("result"))
            }
            Step::StopAll => {
                let outer = self.outer();
                let stop = format!("{PAD}{outer}.stopping = true;\n");
                if self.immediate {
                    format!("{stop}{PAD}return Val::Num(0.0);\n")
                } else {
                    format!("{stop}{PAD}s.finish();\n{PAD}return;\n")
                }
            }
            Step::End => self.leave("Val::Num(0.0)"),
        })
    }

    /// Handing the frame back at a loop's back edge - unless this is a
    /// reporter, where there is nobody to hand it to.
    fn yield_frame(&self) -> String {
        if self.immediate {
            String::new()
        } else {
            format!("{PAD}return;\n")
        }
    }

    /// The same, for a `wait` or a `glide` with time on it.
    fn sleep(&self) -> String {
        if self.immediate {
            String::new()
        } else {
            format!(
                "{PAD}if seconds > 0.0 {{\n{PAD}    s.sleep(seconds);\n{PAD}    return;\n{PAD}}}\n"
            )
        }
    }

    /// Leaving a custom block: back to whoever called it, or out of here with
    /// `result` when what called it was a reporter - or nothing at all.
    fn leave(&self, result: &str) -> String {
        let end = if self.immediate {
            format!("{PAD}    None => return {result},\n")
        } else {
            format!(
                "{PAD}    None => {{\n{PAD}        s.finish();\n{PAD}        return;\n{PAD}    }}\n"
            )
        };
        format!("{PAD}match s.resume_at() {{\n{PAD}    Some(pc) => s.pc = pc,\n{end}{PAD}}}\n")
    }

    /// A call's arguments, evaluated left to right and only as far as the
    /// block declared inputs to bind them to.
    fn arguments(&mut self, args: &[Value], bound: usize) -> Emit<String> {
        let mut parts = Vec::with_capacity(bound.min(args.len()));
        for value in args.iter().take(bound) {
            parts.push(self.evaluated(value)?);
        }
        Ok(parts.join(", "))
    }

    fn emit_action(&mut self, action: &Action) -> Emit<String> {
        let line = match action {
            Action::Move(steps) => reading(self.number(steps)?, "Act::Move { steps: slot }"),
            Action::GoTo(target) => reading(self.vec3(target)?, "Act::GoTo { position: slot }"),
            Action::ChangePosition { axis, by } => reading(
                self.number(by)?,
                &format!("Act::ChangePosition {{ axis: {}, by: slot }}", axis.index()),
            ),
            Action::Turn { axis, degrees } => reading(
                self.number(degrees)?,
                &format!("Act::Turn {{ axis: {}, degrees: slot }}", axis.index()),
            ),
            Action::SetRotation { axis, degrees } => reading(
                self.number(degrees)?,
                &format!(
                    "Act::SetRotation {{ axis: {}, degrees: slot }}",
                    axis.index()
                ),
            ),
            Action::PointTowards(target) => act(format!(
                "Act::PointTowards {{ target: {} }}",
                literal(target)
            )),
            Action::SetScale(factor) => {
                reading(self.number(factor)?, "Act::SetScale { factor: slot }")
            }
            Action::SetBody(body) => act(format!("Act::SetBody {{ body: {} }}", name_of(body))),
            Action::ApplyImpulse(vector) => {
                reading(self.vec3(vector)?, "Act::ApplyImpulse { impulse: slot }")
            }
            Action::SetVelocity(vector) => {
                reading(self.vec3(vector)?, "Act::SetVelocity { velocity: slot }")
            }
            Action::SetGravity(vector) => {
                reading(self.vec3(vector)?, "Act::SetGravity { gravity: slot }")
            }
            Action::SetDensity(density) => {
                reading(self.number(density)?, "Act::SetDensity { density: slot }")
            }
            Action::SetMass(mass) => reading(self.number(mass)?, "Act::SetMass { mass: slot }"),
            Action::Say(value) => reading(self.text(value)?, "Act::Say { text: slot }"),
            Action::SetVisible(visible) => act(format!("Act::SetVisible {{ visible: {visible} }}")),
            Action::SetColor(color) => reading(self.text(color)?, "Act::SetColor { color: slot }"),
            Action::SetComponentField {
                component,
                field,
                value,
            } => reading(
                self.evaluated(value)?,
                &format!(
                    "Act::SetComponentField {{ component: {}, field: {}, value: slot }}",
                    literal(component.trim()),
                    literal(field.trim())
                ),
            ),
            Action::SetCameraView(view) => {
                act(format!("Act::SetCameraView {{ view: {} }}", name_of(view)))
            }
            Action::AttachComponent(component) => act(format!(
                "Act::AttachComponent {{ component: {} }}",
                literal(component)
            )),
            Action::DetachComponent(component) => act(format!(
                "Act::DetachComponent {{ component: {} }}",
                literal(component)
            )),
            Action::Broadcast(name) => act(format!(
                "Act::Broadcast {{ name: {} }}",
                literal(name.trim())
            )),
            // A variable write isn't an effect: it is the host's own state, so
            // it goes through the host rather than through `Act`.
            Action::SetVariable { name, value } => format!(
                "    let value = {};\n    h.set_variable(ME, {}, value);\n",
                self.evaluated(value)?,
                literal(name)
            ),
            // The delta and the current value are both numbers-or-zero, and
            // neither reports a non-numeric one, exactly as in Scratch.
            Action::ChangeVariable { name, value } => format!(
                "    let by = {};\n    \
                 let now = h.variable(ME, {name_literal}).as_number().unwrap_or(0.0);\n    \
                 h.set_variable(ME, {name_literal}, Val::Num(now + by));\n",
                self.number_f64(value)?,
                name_literal = literal(name)
            ),
        };
        Ok(line)
    }

    // ─── Value slots ────────────────────────────────────────────────────────

    /// One value slot: the resolve phase the VM runs first, and the read that
    /// follows it.
    ///
    /// `Vm::eval` resolves a whole tree - every variable, parameter and
    /// reporter call replaced by a literal - before one operator runs. That
    /// order is observable: a reporter on the unused side of an `and` is run
    /// all the same, even though the operator never looks at what it said. So
    /// each of those gets a `let` of its own here, in the order `resolve`
    /// walks them, and the expression below only reads them back.
    fn read_with(&mut self, reader: &str, value: &Value) -> Emit<String> {
        let mut prelude = String::new();
        let expr = self.resolve(value, &mut prelude)?;
        Ok(format!("{{ {prelude}let v = {expr}; {reader}(h, ME, v) }}"))
    }

    fn number(&mut self, value: &Value) -> Emit<String> {
        self.read_with("number", value)
    }

    fn number_f64(&mut self, value: &Value) -> Emit<String> {
        self.read_with("number_f64", value)
    }

    fn text(&mut self, value: &Value) -> Emit<String> {
        self.read_with("text", value)
    }

    fn evaluated(&mut self, value: &Value) -> Emit<String> {
        self.read_with("evaluated", value)
    }

    fn boolean(&mut self, value: &Value) -> Emit<String> {
        self.read_with("boolean", value)
    }

    /// Three slots, each read on its own: the VM evaluates and reports one
    /// before it so much as looks at the next.
    fn vec3(&mut self, values: &[Value; 3]) -> Emit<String> {
        let x = self.number(&values[0])?;
        let y = self.number(&values[1])?;
        let z = self.number(&values[2])?;
        Ok(format!("[{x}, {y}, {z}]"))
    }

    /// Puts `expression` in a `let` of its own and answers its name.
    fn bind(&mut self, prelude: &mut String, expression: String) -> String {
        let name = format!("v{}", self.bindings);
        self.bindings += 1;
        prelude.push_str(&format!("let {name} = {expression}; "));
        name
    }

    fn resolve(&mut self, value: &Value, prelude: &mut String) -> Emit<String> {
        Ok(match value {
            Value::Number { value } => format!("Ok(Val::Num({}))", float(*value)),
            Value::Text { value } => format!("Ok(Val::Text({}.to_string()))", literal(value)),
            Value::Bool => "Ok(Val::Bool(false))".to_string(),
            Value::Var { name } => {
                let read = format!("h.variable(ME, {})", literal(name));
                format!("Ok({})", self.bind(prelude, read))
            }
            Value::Param { name } => {
                // Bound by position, because that is how the VM binds it. A
                // name the block never declared had no value to read.
                let slot = self
                    .owner
                    .and_then(|block| self.canvas.inputs.get(block))
                    .and_then(|names| names.iter().rposition(|declared| declared == name));
                match slot {
                    Some(index) => {
                        let read = format!("s.param({index})");
                        format!("Ok({})", self.bind(prelude, read))
                    }
                    None => "Ok(Val::Num(0.0))".to_string(),
                }
            }
            Value::Call { block_id, args, .. } => {
                let target = self.canvas.program.blocks.get(block_id.as_str()).copied();
                let Some(start) = target else {
                    // Nothing by that id, so nothing is run and nothing is
                    // evaluated to run it with.
                    return Ok("Ok(Val::Num(0.0))".to_string());
                };
                let bound = self.canvas.bound(block_id);
                let args = self.arguments(args, bound)?;
                let (depth, outer, index) = (self.depth(), self.outer(), self.canvas.index);
                // The depth is checked before the arguments, as the VM checks
                // it: too deep and they are never worked out at all. They are
                // bound before the call for the usual reason - working one
                // out borrows the host, and so does running the block.
                let call = format!(
                    "if {depth} >= MAX_REPORTER_DEPTH {{ too_deep(h, ME) }} \
                     else {{ let a = vec![{args}]; \
                     reporter_{index}(h, {outer}, {start}, {depth} + 1, a) }}"
                );
                format!("Ok({})", self.bind(prelude, call))
            }
            Value::Op { op, args, .. } => self.op_expr(op, args, prelude)?,
        })
    }

    fn op_expr(&mut self, op: &Op, args: &[Value], prelude: &mut String) -> Emit<String> {
        // Every operand is resolved, in order, before the operator itself is
        // written - including the one a short circuit will never look at.
        let mut parts = Vec::with_capacity(args.len());
        for value in args {
            parts.push(self.resolve(value, prelude)?);
        }
        let arg = |index: usize| -> Emit<String> {
            parts
                .get(index)
                .cloned()
                .ok_or_else(|| Unsupported::new(format!("`{}` with a missing slot", op.name())))
        };
        let call = |name: &str, count: usize| -> Emit<String> {
            let mut written = Vec::with_capacity(count);
            for index in 0..count {
                written.push(arg(index)?);
            }
            Ok(format!("{name}({})", written.join(", ")))
        };

        match op {
            Op::Add => call("add", 2),
            Op::Sub => call("sub", 2),
            Op::Mul => call("mul", 2),
            Op::Div => call("div", 2),
            Op::Mod => call("modulo", 2),
            Op::Round => call("round", 1),
            Op::Math => call("math", 2),
            Op::Length => call("length", 1),
            Op::IndexOf => call("index_of", 2),
            Op::LastIndexOf => call("last_index_of", 2),
            Op::LetterOf => call("letter_of", 2),
            Op::Case => call("case", 2),
            Op::Eq => call("eq", 2),
            Op::Neq => call("neq", 2),
            Op::Gt => call("gt", 2),
            Op::Lt => call("lt", 2),
            Op::Gte => call("gte", 2),
            Op::Lte => call("lte", 2),
            Op::Not => call("not", 1),
            Op::True => Ok("Ok(Val::Bool(true))".to_string()),
            Op::False => Ok("Ok(Val::Bool(false))".to_string()),
            Op::NewLine => Ok("Ok(Val::Text(\"\\n\".to_string()))".to_string()),
            Op::Tab => Ok("Ok(Val::Text(\"\\t\".to_string()))".to_string()),
            // The second operand stays unevaluated: `false and <a bad slot>`
            // reports nothing, and that is observable.
            Op::And => Ok(format!("and({}, || {})", arg(0)?, arg(1)?)),
            Op::Or => Ok(format!("or({}, || {})", arg(0)?, arg(1)?)),
            // Join is the one built-in that takes however many it was given.
            Op::Join => Ok(format!("join(vec![{}])", parts.join(", "))),
            // The world and the clock are the host's to answer, so that one
            // run of a game has one of each rather than two that disagree.
            Op::Random => Ok(format!(
                "{{ let a = vec![{}, {}]; sense(h, ME, \"Random\", a) }}",
                arg(0)?,
                arg(1)?
            )),
            Op::CurrentTime => Ok(format!(
                "{{ let a = vec![{}]; sense(h, ME, \"CurrentTime\", a) }}",
                arg(0)?
            )),
            Op::Ext(name) => Ok(format!(
                "{{ let a = vec![{}]; sense(h, ME, {}, a) }}",
                parts.join(", "),
                literal(name)
            )),
        }
    }
}

// ─── Small pieces of Rust ───────────────────────────────────────────────────

/// One emitted line, handing the host something to do.
fn act(built: String) -> String {
    format!("    h.act(ME, {built});\n")
}

/// The same, for an act with a slot in it. The slot is read into a `let`
/// first: building the act borrows the host and so does reading a slot, and
/// one expression can't do both.
fn reading(reader: String, built: &str) -> String {
    format!("    let slot = {reader};\n    h.act(ME, {built});\n")
}

/// Lines written for a bare statement, moved in to sit inside an arm.
fn nested(lines: &str) -> String {
    lines
        .lines()
        .map(|line| format!("            {line}\n"))
        .collect()
}

/// An enum variant as a string literal holding its Rust name, which is what
/// the host maps back. Only fieldless enums travel this way, so `Debug` is
/// the name and nothing else.
fn name_of(value: &impl std::fmt::Debug) -> String {
    literal(&format!("{value:?}"))
}

/// A Rust string literal holding exactly `text`.
fn literal(text: &str) -> String {
    format!("{text:?}")
}

/// A Rust `f64` literal. `{:?}` keeps every digit that matters but spells the
/// three values that aren't numbers in a way rustc won't take.
fn float(value: f64) -> String {
    if value.is_nan() {
        return "f64::NAN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 {
            "f64::INFINITY".to_string()
        } else {
            "f64::NEG_INFINITY".to_string()
        };
    }
    format!("{value:?}f64")
}

#[cfg(test)]
mod tests;
