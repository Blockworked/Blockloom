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
//! needs a frame stack. Each live call keeps its own slots, saved and
//! restored across the boundary, which is what lets a block call itself.
//!
//! One function covers a whole actor rather than one strand, because every
//! strand and every custom block body live in one step list with one set of
//! numbers, and a custom block called as a statement is a jump into somebody
//! else's region. The VM's `immediate` flag - a reporter body run to
//! completion in place - becomes a second function over the same steps, so
//! each actor is emitted at most twice however many reporters it has.
//!
//! The one place the two deliberately part company is the VM's per-tick step
//! budget, which a compiled strand only counts when it has to. Every back
//! edge is a loop's, and every loop yields, so a compiled strand can't spin -
//! the budget only ever catches a strand of ten thousand straight-line blocks,
//! and paying for a counter on every block to match it there would cost the
//! whole point of compiling. A reporter body is the exception and does count,
//! since nothing in it yields and the budget is all that ends a runaway one.
//! A recursive statement block is the other: a runaway there would spin inside
//! a single tick where the VM hands the frame back, so one of those counts too.
//!
//! An actor is a value rather than a constant here: the emitted function is
//! handed the id it is running under, so one function covers an authored
//! actor and every clone of it. The clones themselves, and the actors a run
//! makes and unmakes, live in the generated program's own [`Actors`] table -
//! the same table the VM keeps, for the same reason: `delete` names an actor
//! the way every block does, and has to be answerable before the host has
//! done anything about it.

// Half of this is only ever used by the programs it is pasted into.
#[allow(dead_code)]
mod runtime;

pub use runtime::{
    ABI_MISSING, ABI_OK, ABI_PANIC, ABI_TOO_LONG, ACT_APPLY_IMPULSE, ACT_ATTACH, ACT_BIND_ACTION,
    ACT_BROADCAST, ACT_BURST_PARTICLES, ACT_CAPTURE_PROBES, ACT_CHANGE_POSITION,
    ACT_CLEAR_ACTION_BINDINGS, ACT_CREATE_ACTOR, ACT_CREATE_CLONE, ACT_DELETE_ACTOR,
    ACT_DELETE_ELEMENT, ACT_DETACH, ACT_DICT_CLEAR, ACT_DICT_DELETE_KEY, ACT_DICT_SET,
    ACT_ENABLE_VOLUME, ACT_ERROR, ACT_GLIDE, ACT_GO_TO, ACT_HIDE_ELEMENT, ACT_JSON_TO_DICT,
    ACT_JSON_TO_LIST, ACT_LIST_ADD, ACT_LIST_CLEAR, ACT_LIST_DELETE, ACT_LIST_INSERT,
    ACT_LIST_REPLACE, ACT_LIST_REVERSE, ACT_LIST_SHIFT, ACT_MOVE, ACT_NAVIGATE_TO,
    ACT_PLAY_ANIMATION, ACT_PLAY_SOUND, ACT_POINT_TOWARDS, ACT_RUMBLE_GAMEPAD, ACT_SAVE_VARIABLE,
    ACT_SAY, ACT_SET_ANIMATION_SPEED, ACT_SET_AURORA, ACT_SET_BODY, ACT_SET_BUS_VOLUME,
    ACT_SET_CAMERA_FOV, ACT_SET_CAMERA_PITCH, ACT_SET_CAMERA_VIEW, ACT_SET_CLOUD_DRIFT,
    ACT_SET_CLOUD_LAYER, ACT_SET_CLOUDS, ACT_SET_COLLISION_LAYER, ACT_SET_COLLISION_MASK,
    ACT_SET_COLOR, ACT_SET_DENSITY, ACT_SET_EMISSIVE_STRENGTH, ACT_SET_EMITTER_DIAL,
    ACT_SET_EMITTER_PLAYING, ACT_SET_EXPOSURE, ACT_SET_FIELD, ACT_SET_FOCUS, ACT_SET_FOG_DENSITY,
    ACT_SET_GI_BOUNCES, ACT_SET_GI_SAMPLES, ACT_SET_GRAVITY, ACT_SET_HDR_OUTPUT,
    ACT_SET_LIGHT_INTENSITY, ACT_SET_LIGHT_SHADOWS, ACT_SET_LIGHTNING_RATE, ACT_SET_MASS,
    ACT_SET_MOUSE_LOCKED, ACT_SET_PARENT, ACT_SET_PAUSED, ACT_SET_PEAK_BRIGHTNESS,
    ACT_SET_RAY_TRACING, ACT_SET_ROTATION, ACT_SET_SCALE, ACT_SET_SHADOW_DISTANCE,
    ACT_SET_SOUND_PITCH, ACT_SET_SOUND_VOLUME, ACT_SET_TRAIL_ENABLED, ACT_SET_TRIGGER,
    ACT_SET_UI_PROP, ACT_SET_UI_THEME, ACT_SET_VELOCITY, ACT_SET_VISIBLE, ACT_SET_VOLUME_WEIGHT,
    ACT_SET_WATER, ACT_SET_WIND, ACT_SHOW_ELEMENT, ACT_STOP_ANIMATION, ACT_STOP_SOUND,
    ACT_STOP_TWEENS, ACT_STRIKE_LIGHTNING, ACT_TURN, ACT_TWEEN_COLOR, ACT_TWEEN_ROTATION,
    ACT_TWEEN_SCALE, AbiStr, AbiValue, Act, Actors, Entry, Host, LOGIC_ABI_VERSION, LogicHostApi,
    R, READ_SENSE, READ_VARIABLE, Runner, SYM_LOGIC_ABI, SYM_LOGIC_FIRE, SYM_LOGIC_FREE,
    SYM_LOGIC_NEW, SYM_LOGIC_PAUSE, SYM_LOGIC_RESET, SYM_LOGIC_TICK, State, Status, TICK_STOPPED,
    VALUE_BOOL, VALUE_ERROR, VALUE_NUMBER, VALUE_TEXT, Val,
};

use crate::project::Project;
use crate::ui::UiKind;
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
    Box::into_raw(Box::new(Runner::new(NAMES))).cast()
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
pub unsafe extern "C" fn blockloom_logic_pause(state: *mut std::ffi::c_void, paused: u32) {
    if let Some(runner) = unsafe { state.cast::<Runner>().as_mut() } {
        runner.set_paused(paused != 0);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn blockloom_logic_tick(
    state: *mut std::ffi::c_void,
    ctx: *mut std::ffi::c_void,
    api: *const LogicHostApi,
    now: f64,
    wall: f64,
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
        runner.tick_at(ENTRIES, &mut host, now, wall)
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

    let programs: Vec<Program> = project
        .actors
        .iter()
        .map(|actor| compile_program(&actor.graph))
        .collect();
    // The VM ends a strand the moment the actor running it has been deleted,
    // whoever deleted it, and it checks after every act. That costs a read
    // per act, so it is only emitted for a project that can delete at all -
    // where nothing deletes, the check could never have held.
    let deletes = programs.iter().any(|program| {
        program
            .steps
            .iter()
            .any(|step| matches!(step, Step::Action(Action::DeleteActor(_))))
    });

    for (index, actor) in project.actors.iter().enumerate() {
        let program = &programs[index];
        if program.steps.is_empty() {
            continue;
        }
        let canvas = Canvas::of(index, program, &actor.graph, deletes)?;

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

    // Every actor the document has, blocks or none: `delete` and `create a
    // clone of` name one the way a block does, and an actor with nothing on
    // its canvas still answers to its name.
    let names: Vec<String> = project
        .actors
        .iter()
        .map(|actor| format!("    ({}, {}),", literal(&actor.id), literal(&actor.name)))
        .collect();

    Ok(format!(
        "// Generated by Blockloom from {}. Rebuilt on every build; don't edit.\n\
         #![allow(unused, clippy::all)]\n\n\
         {RUNTIME_SOURCE}\n\
         pub static NAMES: &[(&str, &str)] = &[\n{}\n];\n\n\
         pub static ENTRIES: &[Entry] = &[\n{}\n];\n\n{bodies}\n{EXPORT_SOURCE}",
        literal(&project.name),
        names.join("\n"),
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
        Trigger::Cloned => "Cloned",
        Trigger::AnimationEnded { .. } => "AnimationEnded",
        Trigger::Particles(_) => "Particles",
        Trigger::ActionPressed(_) => "Action",
        Trigger::Touched => "Touched",
        Trigger::UiEvent { .. } => "UiEvent",
        Trigger::UiClicked(_) => "UiClicked",
        Trigger::UiChanged(_) => "UiChanged",
    }
}

fn trigger_detail(trigger: &crate::vm::Trigger) -> String {
    use crate::vm::Trigger;
    match trigger {
        Trigger::KeyPressed(key) => key.clone(),
        Trigger::Collision { with } => with.clone(),
        Trigger::Message(name) => name.clone(),
        Trigger::AnimationEnded { clip } => clip.clone(),
        Trigger::Particles(event) => event.name().to_string(),
        Trigger::UiEvent { id, event } => format!("{event}\n{id}"),
        Trigger::UiClicked(id) | Trigger::UiChanged(id) => id.clone(),
        Trigger::ActionPressed(action) => action.clone(),
        Trigger::Started | Trigger::Clicked | Trigger::Cloned | Trigger::Touched => String::new(),
    }
}

/// How far into a generated function an arm's own statements sit.
const PAD: &str = "                ";

// ─── What the emitter knows that the step list doesn't say ──────────────────

/// One actor's program, plus everything the emitter has to work out about it
/// before writing a line.
struct Canvas<'a> {
    index: usize,
    program: &'a Program,
    /// Custom block id -> its input names, in prototype order: the positional
    /// key a call site's arguments line up against.
    inputs: HashMap<&'a str, Vec<&'a str>>,
    /// Step index -> the custom block whose body it is in. A parameter is
    /// read back by position, and the position is a fact about that block.
    owner: HashMap<usize, &'a str>,
    plan: Plan,
    /// Whether anything in the project deletes an actor, which is what makes
    /// the after-every-act check worth emitting.
    deletes: bool,
    /// Whether a custom block can reach itself through statement calls. One
    /// of those needs a step budget, or a runaway would spin inside a single
    /// tick where the VM would hand the frame back.
    recursive: bool,
}

impl<'a> Canvas<'a> {
    fn of(
        index: usize,
        program: &'a Program,
        graph: &'a crate::blocks::ActorGraph,
        deletes: bool,
    ) -> Emit<Self> {
        let inputs = graph
            .block_defs
            .iter()
            .map(|def| (def.id.as_str(), def.input_names().collect()))
            .collect();
        let owner = owners(program);
        let recursive = has_recursion(program, &owner);
        Ok(Self {
            index,
            program,
            inputs,
            owner,
            plan: Plan::of(program),
            deletes,
            recursive,
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

/// True when a custom block can reach itself through statement calls. Each
/// invocation keeps its own loop counters now, so this is allowed - but one
/// of those needs a step budget, or a runaway would spin inside a single tick
/// where the VM would hand the frame back.
fn has_recursion(program: &Program, owner: &HashMap<usize, &str>) -> bool {
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
                    return true;
                }
                if seen.insert(next) {
                    stack.push(next);
                }
            }
        }
    }
    false
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
                // arm the callee's `End` can land on. An `Invoke` is the same:
                // its callee may wait, so what follows has to be resumable.
                Step::Wait(_)
                | Step::Glide { .. }
                | Step::TweenScale { .. }
                | Step::TweenRotation { .. }
                | Step::TweenColor { .. }
                | Step::Break
                | Step::Continue
                | Step::Return(_)
                | Step::StopAll
                | Step::Call { .. }
                | Step::Invoke { .. } => {
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
                // `pause game` hands the frame back like a `wait`, so
                // what follows it has to be an arm the strand resumes at.
                Step::Action(Action::SetPaused(true)) => {
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

        let index = self.canvas.index;
        Ok(if self.immediate {
            format!(
                "/// A custom block of this actor's, run to completion in place as a\n\
                 /// reporter. Its own state, so one of these may call another.\n\
                 fn reporter_{index}(\n    \
                 h: &mut dyn Host,\n    top: &mut State,\n    actors: &mut Actors,\n    \
                 start: usize,\n    depth: usize,\n    args: Vec<Val>,\n) -> Val {{\n    \
                 let me = Rc::clone(&top.me);\n    \
                 let mut s = State::new(&me, start, {});\n    \
                 s.enter_call(State::RETURN, args);\n    \
                 let mut budget = STEP_BUDGET;\n    \
                 loop {{\n        \
                 if budget == 0 {{\n            return Val::Num(0.0);\n        }}\n        \
                 budget -= 1;\n        match s.pc {{\n{arms}            \
                 _ => return Val::Num(0.0),\n        }}\n    }}\n}}\n\n",
                self.canvas.plan.counters.len()
            )
        } else {
            // The actor is read off the state rather than written in: one
            // emitted function runs for the authored actor and for every
            // clone of it. A recursive block gets the same step budget the VM
            // runs under, or a runaway would spin here where the VM yields.
            if self.canvas.recursive {
                format!(
                    "fn actor_{index}(h: &mut dyn Host, s: &mut State, actors: &mut Actors) {{\n    \
                     let me = Rc::clone(&s.me);\n    \
                     if !s.resume() {{\n        return;\n    }}\n    \
                     let mut budget = STEP_BUDGET;\n    \
                     loop {{\n        \
                     if budget == 0 {{\n            return;\n        }}\n        \
                     budget -= 1;\n        match s.pc {{\n{arms}            \
                     _ => {{\n                s.finish();\n                return;\n            }}\n        \
                     }}\n    }}\n}}\n\n"
                )
            } else {
                format!(
                    "fn actor_{index}(h: &mut dyn Host, s: &mut State, actors: &mut Actors) {{\n    \
                     let me = Rc::clone(&s.me);\n    \
                     if !s.resume() {{\n        return;\n    }}\n    \
                     loop {{\n        match s.pc {{\n{arms}            \
                     _ => {{\n                s.finish();\n                return;\n            }}\n        \
                     }}\n    }}\n}}\n\n"
                )
            }
        })
    }

    /// One step, as the statements that carry it out and say where to go
    /// next. Everything but an action sets `s.pc` itself, and so ends its arm.
    fn emit_step(&mut self, pc: usize) -> Emit<String> {
        self.owner = self.canvas.owner.get(&pc).copied();
        let steps = &self.canvas.program.steps;
        let next = pc + 1;
        Ok(match &steps[pc] {
            Step::Action(action) => nested(&self.emit_action(action, next)?),
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
            Step::Glide {
                seconds,
                target,
                easing,
            } => {
                let seconds = self.number_f64(seconds)?;
                let target = self.vec3(target)?;
                format!(
                    "{PAD}let seconds = {seconds}.max(0.0);\n{PAD}let target = {target};\n\
                     {PAD}h.act(&me, Act::Glide {{ seconds: seconds as f32, target, easing: \"{easing:?}\" }});\n\
                     {PAD}s.pc = {next};\n{}",
                    self.sleep()
                )
            }
            Step::TweenScale {
                factor,
                seconds,
                easing,
            } => {
                let factor = self.number(factor)?;
                let seconds = self.number_f64(seconds)?;
                format!(
                    "{PAD}let factor = {factor};\n{PAD}let seconds = {seconds}.max(0.0);\n\
                     {PAD}h.act(&me, Act::TweenScale {{ factor: factor as f32, seconds: seconds as f32, easing: \"{easing:?}\" }});\n\
                     {PAD}s.pc = {next};\n{}",
                    self.sleep()
                )
            }
            Step::TweenRotation {
                axis,
                degrees,
                seconds,
                easing,
            } => {
                let degrees = self.number(degrees)?;
                let seconds = self.number_f64(seconds)?;
                format!(
                    "{PAD}let degrees = {degrees};\n{PAD}let seconds = {seconds}.max(0.0);\n\
                     {PAD}h.act(&me, Act::TweenRotation {{ axis: {}, degrees: degrees as f32, seconds: seconds as f32, easing: \"{easing:?}\" }});\n\
                     {PAD}s.pc = {next};\n{}",
                    axis.index(),
                    self.sleep()
                )
            }
            Step::TweenColor {
                color,
                seconds,
                easing,
            } => {
                let color = self.text(color)?;
                let seconds = self.number_f64(seconds)?;
                format!(
                    "{PAD}let color = {color}.trim().to_string();\n{PAD}let seconds = {seconds}.max(0.0);\n\
                     {PAD}h.act(&me, Act::TweenColor {{ color, seconds: seconds as f32, easing: \"{easing:?}\" }});\n\
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
            Step::Invoke {
                block_id,
                args,
                temp,
            } => {
                let temp = *temp;
                let target = self.canvas.program.blocks.get(block_id.as_str()).copied();
                match target {
                    None => {
                        format!("{PAD}s.store_temp({temp}, Val::Num(0.0));\n{PAD}s.pc = {next};\n")
                    }
                    Some(start) => {
                        let bound = self.canvas.bound(block_id);
                        let mut built = self.arguments(args, bound)?;
                        built = format!("vec![{built}]");
                        if self.immediate {
                            // Never reached: a suspendable call never survives
                            // lifting inside a value, so an immediate body has
                            // no `Invoke` of its own to run.
                            format!(
                                "{PAD}s.store_temp({temp}, Val::Num(0.0));\n{PAD}s.pc = {next};\n"
                            )
                        } else {
                            format!(
                                "{PAD}let args = {built};\n\
                                 {PAD}s.enter_invoke({next}, args, {temp});\n\
                                 {PAD}s.pc = {start};\n"
                            )
                        }
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
    /// `result` when what called it was a reporter - or nothing at all. An
    /// `Invoke` caller gets the value in its temp slot on the way home.
    fn leave(&self, result: &str) -> String {
        let end = if self.immediate {
            format!("{PAD}    None => return {result},\n")
        } else {
            format!(
                "{PAD}    None => {{\n{PAD}        s.finish();\n{PAD}        return;\n{PAD}    }}\n"
            )
        };
        let call = if !self.immediate && result != "Val::Num(0.0)" {
            format!("s.return_with({result})")
        } else {
            "s.resume_at()".to_string()
        };
        format!("{PAD}match {call} {{\n{PAD}    Some(pc) => s.pc = pc,\n{end}{PAD}}}\n")
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

    fn emit_action(&mut self, action: &Action, next: usize) -> Emit<String> {
        let line = match action {
            Action::Move(steps) => reading(self.number(steps)?, "Act::Move { steps: slot }"),
            Action::GoTo(target) => reading(self.vec3(target)?, "Act::GoTo { position: slot }"),
            Action::NavigateTo { target, speed } => format!(
                "    let target = {};\n    let speed = {};\n    \
                 h.act(&me, Act::NavigateTo {{ target, speed: speed as f32 }});\n",
                self.vec3(target)?,
                self.number(speed)?
            ),
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
            Action::TweenScale {
                factor,
                seconds,
                easing,
            } => format!(
                "    let factor = {};\n    let seconds = {}.max(0.0);\n    \
                 h.act(&me, Act::TweenScale {{ factor: factor as f32, seconds: seconds as f32, easing: \"{easing:?}\" }});\n",
                self.number(factor)?,
                self.number_f64(seconds)?,
            ),
            Action::TweenRotation {
                axis,
                degrees,
                seconds,
                easing,
            } => format!(
                "    let degrees = {};\n    let seconds = {}.max(0.0);\n    \
                 h.act(&me, Act::TweenRotation {{ axis: {}, degrees: degrees as f32, seconds: seconds as f32, easing: \"{easing:?}\" }});\n",
                self.number(degrees)?,
                self.number_f64(seconds)?,
                axis.index(),
            ),
            Action::TweenColor {
                color,
                seconds,
                easing,
            } => format!(
                "    let color = {}.trim().to_string();\n    let seconds = {}.max(0.0);\n    \
                 h.act(&me, Act::TweenColor {{ color, seconds: seconds as f32, easing: \"{easing:?}\" }});\n",
                self.text(color)?,
                self.number_f64(seconds)?,
            ),
            Action::StopTweens => act("Act::StopTweens".to_string()),
            Action::PlayAnimation { clip, speed } => format!(
                "    let clip = {}.trim().to_string();\n    let speed = {};\n    \
                 h.act(&me, Act::PlayAnimation {{ clip, speed: speed as f32 }});\n",
                self.text(clip)?,
                self.number(speed)?,
            ),
            Action::StopAnimation => act("Act::StopAnimation".to_string()),
            Action::SetAnimationSpeed(speed) => reading(
                self.number(speed)?,
                "Act::SetAnimationSpeed { speed: slot }",
            ),
            Action::SetExposure(ev) => reading(self.number(ev)?, "Act::SetExposure { ev: slot }"),
            Action::SetLightIntensity(intensity) => reading(
                self.number(intensity)?,
                "Act::SetLightIntensity { intensity: slot }",
            ),
            Action::SetEmissiveStrength(strength) => reading(
                self.number(strength)?,
                "Act::SetEmissiveStrength { strength: slot }",
            ),
            Action::SetHdrOutput(enabled) => {
                act(format!("Act::SetHdrOutput {{ enabled: {enabled} }}"))
            }
            Action::SetPeakBrightness(nits) => {
                reading(self.number(nits)?, "Act::SetPeakBrightness { nits: slot }")
            }
            Action::EnableVolume { volume, enabled } => format!(
                "    let volume = {}.trim().to_string();\n    \
                 h.act(&me, Act::EnableVolume {{ volume, enabled: {enabled} }});\n",
                self.text(volume)?,
            ),
            Action::SetVolumeWeight { volume, weight } => format!(
                "    let volume = {}.trim().to_string();\n    let weight = {};\n    \
                 h.act(&me, Act::SetVolumeWeight {{ volume, weight }});\n",
                self.text(volume)?,
                self.number(weight)?,
            ),
            Action::CaptureProbes => act("Act::CaptureProbes".to_string()),
            Action::SetShadowDistance(distance) => reading(
                self.number(distance)?,
                "Act::SetShadowDistance { distance: slot }",
            ),
            Action::SetLightShadows(enabled) => {
                act(format!("Act::SetLightShadows {{ enabled: {enabled} }}"))
            }
            Action::SetRayTracing(enabled) => {
                act(format!("Act::SetRayTracing {{ enabled: {enabled} }}"))
            }
            Action::SetGiBounces(bounces) => {
                reading(self.number(bounces)?, "Act::SetGiBounces { bounces: slot }")
            }
            Action::SetGiSamples(samples) => {
                reading(self.number(samples)?, "Act::SetGiSamples { samples: slot }")
            }
            Action::SetFogDensity(density) => reading(
                self.number(density)?,
                "Act::SetFogDensity { density: slot }",
            ),
            Action::SetAurora(kp) => reading(self.number(kp)?, "Act::SetAurora { kp: slot }"),
            Action::StrikeLightning(vector) => {
                reading(self.vec3(vector)?, "Act::StrikeLightning { at: slot }")
            }
            Action::SetLightningRate(rate) => {
                reading(self.number(rate)?, "Act::SetLightningRate { rate: slot }")
            }
            Action::SetWind { property, value } => reading(
                self.number(value)?,
                &format!(
                    "Act::SetWind {{ property: \"{}\", value: slot }}",
                    property.name()
                ),
            ),
            Action::SetCloudLayer {
                layer,
                property,
                value,
            } => format!(
                "    let layer = {};\n    let value = {};\n    \
                 h.act(&me, Act::SetCloudLayer {{ layer, property: \"{}\", value }});\n",
                self.number(layer)?,
                self.number(value)?,
                property.name(),
            ),
            Action::SetClouds { property, value } => reading(
                self.number(value)?,
                &format!(
                    "Act::SetClouds {{ property: \"{}\", value: slot }}",
                    property.name()
                ),
            ),
            Action::SetWater { property, value } => reading(
                self.number(value)?,
                &format!(
                    "Act::SetWater {{ property: \"{}\", value: slot }}",
                    property.name()
                ),
            ),
            Action::SetCloudDrift(vector) => {
                reading(self.vec3(vector)?, "Act::SetCloudDrift { drift: slot }")
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
            Action::BurstParticles(count) => reading(
                self.number(count)?,
                "Act::BurstParticles { count: (slot as i64).clamp(0, 512) as u32 }",
            ),
            Action::SetEmitterDial { dial, value } => reading(
                self.number(value)?,
                &format!("Act::SetEmitterDial {{ dial: \"{dial:?}\", value: slot }}"),
            ),
            Action::SetTrailEnabled(enabled) => {
                act(format!("Act::SetTrailEnabled {{ enabled: {enabled} }}"))
            }
            Action::SetEmitterPlaying(playing) => {
                act(format!("Act::SetEmitterPlaying {{ playing: {playing} }}"))
            }
            Action::SetTrigger(trigger) => act(format!("Act::SetTrigger {{ trigger: {trigger} }}")),
            Action::SetCollisionLayer(layer) => reading(
                self.number(layer)?,
                "Act::SetCollisionLayer { layer: (slot as i32).clamp(1, 8) as u8 }",
            ),
            Action::SetCollisionMask(mask) => reading(
                self.number(mask)?,
                "Act::SetCollisionMask { mask: (slot as i32).clamp(0, 255) as u8 }",
            ),
            Action::Say(value) => reading(self.text(value)?, "Act::Say { text: slot }"),
            Action::SetVisible(visible) => act(format!("Act::SetVisible {{ visible: {visible} }}")),
            Action::SetColor(color) => reading(self.text(color)?, "Act::SetColor { color: slot }"),
            // Sound slots read in row order, like every other block: sound,
            // volume, pitch, then the target. An empty sound reports itself
            // instead of playing, exactly as the VM does.
            Action::PlaySound {
                sound,
                volume,
                pitch,
                loop_,
                bus,
            } => format!(
                "    let sound = {}.trim().to_string();\n    let volume = sound_gain({});\n    \
                 let pitch = sound_pitch({});\n    \
                 if sound.is_empty() {{\n        h.error(&me, \"which sound should I play?\");\n    \
                 }} else {{\n        \
                 h.act(&me, Act::PlaySound {{ sound, volume, pitch, loop_: {loop_}, bus: {}, at: None }});\n    \
                 }}\n",
                self.text(sound)?,
                self.number_f64(volume)?,
                self.number(pitch)?,
                name_of(bus),
            ),
            Action::PlaySoundAt {
                sound,
                volume,
                pitch,
                loop_,
                bus,
                target,
            } => format!(
                "    let sound = {}.trim().to_string();\n    let volume = sound_gain({});\n    \
                 let pitch = sound_pitch({});\n    let wanted = {};\n    \
                 if sound.is_empty() {{\n        h.error(&me, \"which sound should I play?\");\n    \
                 }} else {{\n        \
                 match actors.find(&me, &wanted) {{\n            \
                 Some(at) => h.act(&me, Act::PlaySound {{ sound, volume, pitch, loop_: {loop_}, \
                 bus: {}, at: Some(at.to_string()) }}),\n            \
                 None => h.error(&me, &format!(\"there's no actor named \\\"{{wanted}}\\\" \
                 to play at\")),\n        \
                 }}\n    }}\n",
                self.text(sound)?,
                self.number_f64(volume)?,
                self.number(pitch)?,
                self.text(target)?,
                name_of(bus),
            ),
            Action::StopSound { sound } => format!(
                "    let sound = {}.trim().to_string();\n    \
                 h.act(&me, Act::StopSound {{ sound }});\n",
                self.text(sound)?
            ),
            Action::SetSoundVolume { sound, volume } => format!(
                "    let sound = {}.trim().to_string();\n    let volume = sound_gain({});\n    \
                 if sound.is_empty() {{\n        \
                 h.error(&me, \"which sound's volume should I set?\");\n    \
                 }} else {{\n        h.act(&me, Act::SetSoundVolume {{ sound, volume }});\n    }}\n",
                self.text(sound)?,
                self.number_f64(volume)?,
            ),
            Action::SetSoundPitch { sound, pitch } => format!(
                "    let sound = {}.trim().to_string();\n    let pitch = sound_pitch({});\n    \
                 if sound.is_empty() {{\n        \
                 h.error(&me, \"which sound's pitch should I set?\");\n    \
                 }} else {{\n        h.act(&me, Act::SetSoundPitch {{ sound, pitch }});\n    }}\n",
                self.text(sound)?,
                self.number(pitch)?,
            ),
            Action::SetBusVolume { bus, volume } => format!(
                "    let volume = sound_gain({});\n    \
                 h.act(&me, Act::SetBusVolume {{ bus: {}, volume }});\n",
                self.number_f64(volume)?,
                name_of(bus),
            ),
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
            Action::SetCameraPitch(degrees) => reading(
                self.number(degrees)?,
                "Act::SetCameraPitch { degrees: slot }",
            ),
            Action::SetCameraFov(fov) => {
                reading(self.number(fov)?, "Act::SetCameraFov { fov: slot }")
            }
            Action::AttachComponent(component) => act(format!(
                "Act::AttachComponent {{ component: {} }}",
                literal(component)
            )),
            Action::DetachComponent(component) => act(format!(
                "Act::DetachComponent {{ component: {} }}",
                literal(component)
            )),
            Action::SetParent(target) => {
                reading(self.text(target)?, "Act::SetParent { target: slot }")
            }
            // The three that make and unmake actors. Each does its own
            // half here - the id, the name and the scheduling - and hands
            // the host the world's half, which is the same split the VM
            // makes between `register_clone` and `Effect::CreateClone`.
            Action::CreateClone(of) => format!(
                "    match actors.find(&me, {}) {{\n        \
                 Some(template) => {{\n            \
                 let clone = actors.clone_of(&template);\n            \
                 h.act(&me, Act::CreateClone {{ of: template.to_string(), \
                 clone: clone.to_string() }});\n        }}\n        \
                 None => h.error(&me, {}),\n    }}\n",
                literal(of),
                literal(&format!("there's no actor named \"{of}\" to clone"))
            ),
            // A name and a place, and no blocks at all: nothing schedules it,
            // so only naming it has to keep working.
            Action::CreateActor { name, position } => format!(
                "    let name = {};\n    let position = {};\n    \
                 let id = actors.create(&name);\n    \
                 h.act(&me, Act::CreateActor {{ id: id.to_string(), name, position }});\n",
                self.text(name)?,
                self.vec3(position)?
            ),
            Action::DeleteActor(target) => format!(
                "    let wanted = {};\n    match actors.find(&me, &wanted) {{\n        \
                 Some(gone) => {{\n            actors.remove(&gone);\n            \
                 h.act(&me, Act::DeleteActor {{ target: gone.to_string() }});\n        }}\n        \
                 None => h.error(&me, &format!(\"there's no actor named \\\"{{wanted}}\\\" \
                 to delete\")),\n    }}\n",
                self.text(target)?
            ),
            Action::Broadcast(name) => act(format!(
                "Act::Broadcast {{ name: {} }}",
                literal(name.trim())
            )),
            Action::SetMouseLocked(locked) => {
                act(format!("Act::SetMouseLocked {{ locked: {locked} }}"))
            }
            Action::RumbleGamepad { strength, duration } => format!(
                "    let strength = ({} as f32).clamp(0.0, 100.0);\n    \
                 let duration = ({} as f32).max(0.0);\n    \
                 h.act(&me, Act::RumbleGamepad {{ strength, duration }});\n",
                self.number(strength)?,
                self.number(duration)?,
            ),
            Action::BindAction { action, binding } => format!(
                "    let action = {}.trim().to_string();\n    \
                 let binding = {}.trim().to_string();\n    \
                 h.act(&me, Act::BindAction {{ action, binding }});\n",
                self.text(action)?,
                self.text(binding)?,
            ),
            Action::ClearActionBindings { action } => format!(
                "    let action = {}.trim().to_string();\n    \
                 h.act(&me, Act::ClearActionBindings {{ action }});\n",
                self.text(action)?,
            ),
            // The interface. Every slot is hoisted into a `let` first, in
            // the order the VM evaluates them, because reading a slot
            // borrows the host and so does handing it something to do.
            Action::ShowElement(spec) => {
                let crate::vm::ShowElement {
                    kind,
                    id,
                    content,
                    range,
                    value,
                    anchor,
                    offset,
                    size,
                    parent,
                    flag,
                } = &**spec;
                let id = self.text(id)?;
                let content = self.text(content)?;
                let range = match range {
                    Some([low, high]) => {
                        format!("[{}, {}]", self.number(low)?, self.number(high)?)
                    }
                    None => "[0.0f32, 1.0f32]".to_string(),
                };
                // A slider reads its starting number off the canvas; every
                // other kind carries a fixed flag instead and asks nothing.
                // Only a slider's row has a value slot; the rest start at
                // whatever their kind means by blank.
                let started_at = match value {
                    Some(value) => {
                        format!("ui_value({}, {})", kind.index(), self.evaluated(value)?)
                    }
                    None => format!("ui_blank({}, {flag})", kind.index()),
                };
                let x = self.number(&offset[0])?;
                let y = self.number(&offset[1])?;
                let width = self.number(&size[0])?;
                let height = self.number(&size[1])?;
                let parent = self.text(parent)?;
                format!(
                    "    let id = {id};\n    let content = {content};\n    \
                     let range = {range};\n    let value = {started_at};\n    \
                     let offset = [{x}, {y}];\n    let size = [{width}, {height}];\n    \
                     let parent = {parent};\n    \
                     h.act(&me, Act::ShowElement {{ id: id.trim().to_string(), kind: {}, \
                     content, anchor: {}, offset, size, parent: parent.trim().to_string(), \
                     flag: {}, range, value }});\n",
                    kind.index(),
                    anchor.index(),
                    *kind == UiKind::Panel && *flag,
                )
            }
            Action::SetUiProp { prop, id, value } => format!(
                "    let id = {};\n    let value = {};\n    \
                 h.act(&me, Act::SetUiProp {{ id: id.trim().to_string(), prop: {}, value }});\n",
                self.text(id)?,
                self.evaluated(value)?,
                literal(prop.name()),
            ),
            Action::HideElement { id, all } => format!(
                "    let id = {};\n    \
                 h.act(&me, Act::HideElement {{ id: id.trim().to_string(), all: {all} }});\n",
                self.text(id)?
            ),
            Action::DeleteElement(id) => format!(
                "    let id = {};\n    \
                 h.act(&me, Act::DeleteElement {{ id: id.trim().to_string() }});\n",
                self.text(id)?
            ),
            Action::SetFocus(id) => format!(
                "    let id = {};\n    \
                 h.act(&me, Act::SetFocus {{ id: id.trim().to_string() }});\n",
                self.text(id)?
            ),
            Action::SetUiTheme(theme) => {
                act(format!("Act::SetUiTheme {{ theme: {} }}", theme.index()))
            }
            // The table freezes now, not when the host answers: the rest of
            // this tick has to see a paused world, exactly as the VM does -
            // and this strand stops here unless the interface started it.
            Action::SetPaused(paused) => format!(
                "    actors.set_paused({paused});\n    \
                 h.act(&me, Act::SetPaused {{ paused: {paused} }});\n{}",
                if *paused {
                    self.freeze(next)
                } else {
                    String::new()
                }
            ),
            Action::SaveVariable { name, clear } => act(format!(
                "Act::SaveVariable {{ name: {}, clear: {clear} }}",
                literal(name)
            )),
            // A variable write isn't an effect: it is the host's own state, so
            // it goes through the host rather than through `Act`.
            Action::SetVariable { name, value } => format!(
                "    let value = {};\n    h.set_variable(&me, {}, value);\n",
                self.evaluated(value)?,
                literal(name)
            ),
            // The delta and the current value are both numbers-or-zero, and
            // neither reports a non-numeric one, exactly as in Scratch.
            Action::ChangeVariable { name, value } => format!(
                "    let by = {};\n    \
                 let now = h.variable(&me, {name_literal}).as_number().unwrap_or(0.0);\n    \
                 h.set_variable(&me, {name_literal}, Val::Num(now + by));\n",
                self.number_f64(value)?,
                name_literal = literal(name)
            ),
            // List writes go through `Act` rather than the host's own state:
            // the host owns the lists, so a new one is a new `Act` the way a
            // new world verb is, never a change to the `Host` trait.
            Action::AddToList { name, value } => reading(
                self.evaluated(value)?,
                &format!("Act::AddToList {{ name: {}, value: slot }}", literal(name)),
            ),
            Action::DeleteOfList { index, name } => reading(
                self.number_f64(index)?,
                &format!(
                    "Act::DeleteOfList {{ name: {}, index: slot }}",
                    literal(name)
                ),
            ),
            Action::DeleteAllOfList { name } => act(format!(
                "Act::DeleteAllOfList {{ name: {} }}",
                literal(name)
            )),
            Action::ShiftList { name, amount } => reading(
                self.number_f64(amount)?,
                &format!("Act::ShiftList {{ name: {}, amount: slot }}", literal(name)),
            ),
            Action::InsertIntoList { value, index, name } => format!(
                "    let value = {};\n    let index = {};\n    \
                 h.act(&me, Act::InsertIntoList {{ name: {}, index, value }});\n",
                self.evaluated(value)?,
                self.number_f64(index)?,
                literal(name)
            ),
            Action::ReplaceItemOfList { index, name, value } => format!(
                "    let index = {};\n    let value = {};\n    \
                 h.act(&me, Act::ReplaceItemOfList {{ name: {}, index, value }});\n",
                self.number_f64(index)?,
                self.evaluated(value)?,
                literal(name)
            ),
            Action::SetDictValue { key, name, value } => format!(
                "    let key = {};\n    let value = {};\n    \
                 h.act(&me, Act::SetDictValue {{ name: {}, key, value }});\n",
                self.text(key)?,
                self.evaluated(value)?,
                literal(name)
            ),
            Action::DeleteDictKey { key, name } => format!(
                "    let key = {};\n    \
                 h.act(&me, Act::DeleteDictKey {{ name: {}, key }});\n",
                self.text(key)?,
                literal(name)
            ),
            Action::DeleteAllOfDict { name } => act(format!(
                "Act::DeleteAllOfDict {{ name: {} }}",
                literal(name)
            )),
            Action::LoadJsonIntoDict { json, name } => format!(
                "    let json = {};\n    \
                 h.act(&me, Act::LoadJsonIntoDict {{ name: {}, json }});\n",
                self.evaluated(json)?,
                literal(name)
            ),
            Action::LoadJsonIntoList { json, name } => format!(
                "    let json = {};\n    \
                 h.act(&me, Act::LoadJsonIntoList {{ name: {}, json }});\n",
                self.evaluated(json)?,
                literal(name)
            ),
            Action::ReverseList { name } => {
                act(format!("Act::ReverseList {{ name: {} }}", literal(name)))
            }
        };
        Ok(format!("{line}{}", self.check_deleted(next)))
    }

    /// The VM ends a strand as soon as the actor running it has been deleted,
    /// and it looks after every act - so a `delete` inside a reporter this
    /// act read counts, and so does one another actor ran earlier this tick.
    /// Nothing is emitted for a project with no `delete` in it, where the
    /// answer could only ever be no.
    /// Handing the frame back at a `pause game`: the strand stops where it
    /// stands, unless the interface is what started it. Nothing to hand back
    /// in a reporter, where a `pause` is just the act.
    fn freeze(&self, next: usize) -> String {
        if self.immediate {
            return String::new();
        }
        format!("    if !s.ui {{\n        s.pc = {next};\n        return;\n    }}\n")
    }

    fn check_deleted(&self, next: usize) -> String {
        if !self.canvas.deletes {
            return String::new();
        }
        let leave = if self.immediate {
            "        return Val::Num(0.0);\n"
        } else {
            "        s.finish();\n        return;\n"
        };
        format!("    if actors.is_gone(&me) {{\n        s.pc = {next};\n{leave}    }}\n")
    }

    // ─── Value slots ────────────────────────────────────────────────────────

    /// One value slot: the resolve phase the VM runs first, and the read that
    /// follows it.
    ///
    /// `Vm::eval` reads every variable, parameter and reporter call in a
    /// tree before the operator over it runs. That order is observable: a reporter on the unused side of an `and` is run
    /// all the same, even though the operator never looks at what it said. So
    /// each of those gets a `let` of its own here, in the order `resolve`
    /// walks them, and the expression below only reads them back.
    fn read_with(&mut self, reader: &str, value: &Value) -> Emit<String> {
        let mut prelude = String::new();
        let expr = self.resolve(value, &mut prelude)?;
        Ok(format!(
            "{{ {prelude}let v = {expr}; {reader}(h, &me, v) }}"
        ))
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
                if let Some(index) = crate::vm::temp_index(name) {
                    let read = format!("s.temp({index})");
                    format!("Ok({})", self.bind(prelude, read))
                } else {
                    let read = format!("h.variable(&me, {})", literal(name));
                    format!("Ok({})", self.bind(prelude, read))
                }
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
                    "if {depth} >= MAX_REPORTER_DEPTH {{ too_deep(h, &me) }} \
                     else {{ let a = vec![{args}]; \
                     reporter_{index}(h, {outer}, actors, {start}, {depth} + 1, a) }}"
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
                "{{ let a = vec![{}, {}]; sense(h, &me, \"Random\", a) }}",
                arg(0)?,
                arg(1)?
            )),
            Op::CurrentTime => Ok(format!(
                "{{ let a = vec![{}]; sense(h, &me, \"CurrentTime\", a) }}",
                arg(0)?
            )),
            Op::Ext(name) => Ok(format!(
                "{{ let a = vec![{}]; sense(h, &me, {}, a) }}",
                parts.join(", "),
                literal(name)
            )),
        }
    }
}

// ─── Small pieces of Rust ───────────────────────────────────────────────────

/// One emitted line, handing the host something to do.
fn act(built: String) -> String {
    format!("    h.act(&me, {built});\n")
}

/// The same, for an act with a slot in it. The slot is read into a `let`
/// first: building the act borrows the host and so does reading a slot, and
/// one expression can't do both.
fn reading(reader: String, built: &str) -> String {
    format!("    let slot = {reader};\n    h.act(&me, {built});\n")
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
