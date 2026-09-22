// What a generated program is built on: a value type, the operators over it,
// and the boundary it asks the world through. Like `script/abi.rs`, this file
// is compiled twice - once into `blockloom-core`, where the tests can hold it
// against the VM's own semantics, and once as text into every program
// `codegen` emits. Editing it changes both at once, which is the point.
//
// Nothing here may use anything but `std`: a generated program is compiled by
// one `rustc` run with no dependencies at all.

use std::rc::Rc;

/// A value as a generated program carries it. The same three cases
/// `Evaluated` has, with the same coercions - a difference here is a
/// difference in what a game does.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Num(f64),
    Text(String),
    Bool(bool),
}

/// What an expression evaluates to. One error anywhere gives up the whole
/// tree, exactly as `Value::eval` does, and the caller reports it once.
pub type R = Result<Val, String>;

impl Val {
    pub fn as_number(&self) -> Result<f64, String> {
        match self {
            Val::Num(n) => Ok(*n),
            Val::Text(s) => s
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("\"{s}\" is not a number")),
            Val::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
        }
    }

    pub fn as_bool(&self) -> bool {
        match self {
            Val::Bool(b) => *b,
            Val::Num(n) => *n != 0.0,
            Val::Text(s) => !s.is_empty() && s != "false",
        }
    }

    pub fn as_text(&self) -> String {
        match self {
            Val::Text(s) => s.clone(),
            Val::Num(n) => n.to_string(),
            Val::Bool(b) => b.to_string(),
        }
    }
}

/// What a generated program asks the world to do. One per leaf instruction
/// the blocks have, carrying values that are already evaluated - the host
/// never evaluates anything, just as it never does for the VM.
///
/// Names that are enums elsewhere (a body kind, a camera view) travel as the
/// text they serialize as, so this file stays free of everything but `std`.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    Move {
        steps: f32,
    },
    GoTo {
        position: [f32; 3],
    },
    ChangePosition {
        axis: usize,
        by: f32,
    },
    /// Starts a slide; the strand sleeps for exactly as long.
    Glide {
        seconds: f32,
        target: [f32; 3],
    },
    Turn {
        axis: usize,
        degrees: f32,
    },
    SetRotation {
        axis: usize,
        degrees: f32,
    },
    PointTowards {
        target: &'static str,
    },
    SetScale {
        factor: f32,
    },
    SetBody {
        body: &'static str,
    },
    ApplyImpulse {
        impulse: [f32; 3],
    },
    SetVelocity {
        velocity: [f32; 3],
    },
    SetGravity {
        gravity: [f32; 3],
    },
    SetDensity {
        density: f32,
    },
    SetMass {
        mass: f32,
    },
    Say {
        text: String,
    },
    SetVisible {
        visible: bool,
    },
    SetColor {
        color: String,
    },
    SetComponentField {
        component: &'static str,
        field: &'static str,
        value: Val,
    },
    SetCameraView {
        view: &'static str,
    },
    AttachComponent {
        component: &'static str,
    },
    DetachComponent {
        component: &'static str,
    },
    /// Hangs the actor off another one, by id or name; empty takes it off.
    /// A slot rather than a fixed name, so it travels as a `String`.
    SetParent {
        target: String,
    },
    /// A copy of `of`, which the program has already given an id and its own
    /// scripts. The host's half is the entity.
    CreateClone {
        of: String,
        clone: String,
    },
    /// An actor the document never had, under an id the program minted.
    CreateActor {
        id: String,
        name: String,
        position: [f32; 3],
    },
    /// Takes `target` - an id, already resolved - out of the run.
    DeleteActor {
        target: String,
    },
    Broadcast {
        name: &'static str,
    },
}

/// One compiled strand, and what starts it. The trigger travels as text for
/// the same reason [`Act`]'s names do.
pub struct Entry {
    pub actor: &'static str,
    pub strand: &'static str,
    /// `Started`, `Key`, `Clicked`, `Collision` or `Message`.
    pub trigger: &'static str,
    /// The key, the other actor or the message name; empty for the rest.
    pub detail: &'static str,
    /// Where this strand starts, in the same step numbering the VM uses.
    pub start: usize,
    /// How many `repeat` counters one run of it needs.
    pub counters: usize,
    pub run: fn(&mut dyn Host, &mut State, &mut Actors),
}

/// One live entry in a compiled run. `actor` is who is running it: the
/// entry's own actor for an authored strand, a clone's id for a copy.
struct Live {
    entry: usize,
    actor: Rc<str>,
    state: State,
}

/// Every actor a compiled run knows about: the ones the document had, and
/// the clones and creations the run has made.
///
/// The VM keeps the same table, for the same reason - `create a clone of`
/// and `delete` name an actor the way every block does, by id or by name,
/// and both have to be answerable before the host has done anything about
/// them.
pub struct Actors {
    /// Id and name, authored first and runtime ones appended.
    live: Vec<(Rc<str>, String)>,
    /// How many actors this run has made, which is where the next one's id
    /// comes from. Counted the way `Vm::new_actor_id` counts, so one project
    /// run either way makes the same ids.
    made: usize,
    /// Clone and template, waiting for `when I start as a clone` to run at
    /// the top of the next tick - the VM's `Event::Cloned` queue.
    fresh: Vec<(Rc<str>, Rc<str>)>,
    /// Ids taken out of the run. Their strands go at the end of the tick,
    /// and one that asked for its own deletion stops where it stands.
    gone: Vec<Rc<str>>,
    /// Clone id -> the authored actor it is a copy of, which is whose
    /// strands it runs.
    clones: Vec<(Rc<str>, Rc<str>)>,
}

impl Actors {
    fn new(names: &'static [(&'static str, &'static str)]) -> Self {
        Self {
            live: names
                .iter()
                .map(|(id, name)| (Rc::from(*id), (*name).to_string()))
                .collect(),
            made: 0,
            fresh: Vec::new(),
            gone: Vec::new(),
            clones: Vec::new(),
        }
    }

    /// Which actor `wanted` means: itself when the slot is empty or says so,
    /// then an id, then a name. The same order `Vm::find_actor` uses, so a
    /// block means the same actor whichever scheduler ran it.
    pub fn find(&self, running: &str, wanted: &str) -> Option<Rc<str>> {
        let wanted = wanted.trim();
        if wanted.is_empty()
            || wanted.eq_ignore_ascii_case("myself")
            || wanted.eq_ignore_ascii_case("me")
        {
            return Some(Rc::from(running));
        }
        if let Some((id, _)) = self.live.iter().find(|(id, _)| &**id == wanted) {
            return Some(Rc::clone(id));
        }
        self.live
            .iter()
            .find(|(_, name)| name.eq_ignore_ascii_case(wanted))
            .map(|(id, _)| Rc::clone(id))
    }

    fn mint(&mut self) -> Rc<str> {
        self.made += 1;
        Rc::from(format!("~{}", self.made).as_str())
    }

    /// Registers a copy of `template` and answers its id. It shares the
    /// template's name, so a broadcast and a `when I touch` reach it too.
    pub fn clone_of(&mut self, template: &str) -> Rc<str> {
        let id = self.mint();
        self.register_clone(Rc::clone(&id), template);
        id
    }

    /// The same, for a clone something outside the program made and named -
    /// a script's, which the host hands over through `fire`.
    fn adopt(&mut self, clone: &str, template: &str) {
        if clone.is_empty() || self.live.iter().any(|(id, _)| &**id == clone) {
            return;
        }
        self.register_clone(Rc::from(clone), template);
    }

    /// Registers an actor something outside the program made, so a block
    /// can still name it. It has no strands, so nothing is scheduled.
    fn adopt_created(&mut self, actor: &str, name: &str) {
        if actor.is_empty() || self.live.iter().any(|(id, _)| &**id == actor) {
            return;
        }
        self.live.push((Rc::from(actor), name.to_string()));
    }

    fn register_clone(&mut self, id: Rc<str>, template: &str) {
        let name = self
            .live
            .iter()
            .find(|(other, _)| &**other == template)
            .map(|(_, name)| name.clone())
            .unwrap_or_default();
        self.live.push((Rc::clone(&id), name));
        let root = self.template_of(template);
        self.clones.push((Rc::clone(&id), Rc::clone(&root)));
        self.fresh.push((id, root));
    }

    /// Registers an actor the document never had. It has no blocks, so
    /// nothing schedules it - only naming it has to keep working.
    pub fn create(&mut self, name: &str) -> Rc<str> {
        let id = self.mint();
        self.live.push((Rc::clone(&id), name.to_string()));
        id
    }

    /// Takes an actor out of the run.
    pub fn remove(&mut self, actor: &str) {
        self.live.retain(|(id, _)| &**id != actor);
        self.clones.retain(|(id, _)| &**id != actor);
        self.fresh.retain(|(id, _)| &**id != actor);
        if !self.is_gone(actor) {
            self.gone.push(Rc::from(actor));
        }
    }

    /// Whether `actor` has been deleted this tick, which is what ends the
    /// strand that asked.
    pub fn is_gone(&self, actor: &str) -> bool {
        self.gone.iter().any(|id| &**id == actor)
    }

    /// Whether `actor` is still in the run at all.
    fn is_live(&self, actor: &str) -> bool {
        self.live.iter().any(|(id, _)| &**id == actor)
    }

    /// The authored actor a runtime one runs the strands of. A clone of a
    /// clone is a clone of the same authored actor.
    fn template_of(&self, actor: &str) -> Rc<str> {
        self.clones
            .iter()
            .find(|(id, _)| &**id == actor)
            .map(|(_, root)| Rc::clone(root))
            .unwrap_or_else(|| Rc::from(actor))
    }

    /// Every runtime actor running an authored actor's strands: that actor,
    /// and each clone of it.
    fn copies_of(&self, authored: &str) -> Vec<Rc<str>> {
        let mut found = vec![Rc::from(authored)];
        found.extend(
            self.clones
                .iter()
                .filter(|(_, root)| &**root == authored)
                .map(|(id, _)| Rc::clone(id)),
        );
        found
    }
}

/// The scheduler held inside a compiled logic library.
pub struct Runner {
    live: Vec<Live>,
    actors: Actors,
    names: &'static [(&'static str, &'static str)],
}

impl Runner {
    pub fn new(names: &'static [(&'static str, &'static str)]) -> Self {
        Self {
            live: Vec::new(),
            actors: Actors::new(names),
            names,
        }
    }

    pub fn reset(&mut self) {
        self.live.clear();
        self.actors = Actors::new(self.names);
    }

    /// True while any strand is still live.
    pub fn is_running(&self) -> bool {
        self.live.iter().any(|live| !live.state.done())
    }

    /// Starts every matching entry, replacing an existing run of the same
    /// strand in place as the VM does.
    ///
    /// Three kinds are the host's word about an actor rather than a trigger:
    /// `Cloned` is a copy something outside the program made - a script's
    /// `create_clone` - `Created` an actor it conjured, and `Deleted` one it
    /// took out of the run. The program's own clones, creations and deletions
    /// go straight into [`Actors`] as they happen.
    pub fn fire(
        &mut self,
        entries: &[Entry],
        kind: &str,
        actor: &str,
        detail: &str,
        other_name: &str,
    ) {
        match kind {
            "Cloned" => {
                self.actors.adopt(actor, detail);
                return;
            }
            "Created" => {
                self.actors.adopt_created(actor, detail);
                return;
            }
            "Deleted" => {
                self.actors.remove(actor);
                self.drop_deleted();
                return;
            }
            _ => {}
        }
        // A clone answers to its template's entries, so what an event means
        // is worked out against the actor those entries were written for.
        let template = self.actors.template_of(actor);
        for (index, entry) in entries.iter().enumerate() {
            let matches = match (entry.trigger, kind) {
                ("Started", "Started") => true,
                ("Key", "Key") | ("Message", "Message") => entry.detail == detail,
                ("Clicked", "Clicked") => entry.actor == &*template,
                ("Collision", "Collision") => {
                    entry.actor == &*template
                        && (entry.detail.is_empty()
                            || entry.detail == detail
                            || entry.detail.eq_ignore_ascii_case(other_name))
                }
                _ => false,
            };
            if !matches {
                continue;
            }
            // An event aimed at one actor starts that actor's copy of the
            // strand; a broadcast starts every copy's.
            let running = match kind {
                "Clicked" | "Collision" => vec![Rc::from(actor)],
                _ => self.actors.copies_of(entry.actor),
            };
            for id in running {
                self.begin(entries, index, id);
            }
        }
    }

    /// Starts one entry under one actor, replacing that actor's own run of
    /// the same strand.
    fn begin(&mut self, entries: &[Entry], index: usize, actor: Rc<str>) {
        // A deleted actor has no strands to start: the VM drops its program,
        // so nothing of its matches an event any more.
        if !self.actors.is_live(&actor) {
            return;
        }
        let entry = &entries[index];
        let fresh = Live {
            entry: index,
            actor: Rc::clone(&actor),
            state: entry.begin(&actor),
        };
        match self.live.iter_mut().find(|live| {
            let old = &entries[live.entry];
            live.actor == actor && old.strand == entry.strand
        }) {
            Some(old) => *old = fresh,
            None => self.live.push(fresh),
        }
    }

    /// Gives every live strand one slice. True means `stop all` ended the run.
    pub fn tick(&mut self, entries: &[Entry], host: &mut dyn Host, now: f64) -> bool {
        // A clone made last tick starts its own strands now, by which time
        // the host has built the actor those blocks read through.
        for (clone, template) in std::mem::take(&mut self.actors.fresh) {
            for (index, entry) in entries.iter().enumerate() {
                if entry.trigger == "Cloned" && entry.actor == &*template {
                    self.begin(entries, index, Rc::clone(&clone));
                }
            }
        }
        let mut index = 0;
        while index < self.live.len() {
            let Self { live, actors, .. } = self;
            let slice = &mut live[index];
            slice.state.now = now;
            (entries[slice.entry].run)(host, &mut slice.state, actors);
            if slice.state.stopping {
                self.live.clear();
                return true;
            }
            index += 1;
        }
        self.live.retain(|live| !live.state.done());
        // A deleted actor's other strands go with it, wherever in the tick
        // they were - the one that ran the block already stopped itself.
        self.drop_deleted();
        false
    }

    fn drop_deleted(&mut self) {
        if self.actors.gone.is_empty() {
            return;
        }
        let gone = std::mem::take(&mut self.actors.gone);
        self.live.retain(|live| !gone.contains(&live.actor));
    }
}

impl Entry {
    /// A fresh run of this strand under `actor`, ready for the first
    /// [`Entry::run`]. The actor is the entry's own for an authored strand
    /// and a clone's id for a copy, which is the whole of what lets one
    /// emitted function run under many actors.
    pub fn begin(&self, actor: &Rc<str>) -> State {
        State::new(actor, self.start, self.counters)
    }
}

/// Where a suspended strand is. The same three the VM's scripts have, so a
/// `wait` means the same thing on both sides.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Status {
    Run,
    /// Asleep until this timestamp, in run seconds.
    Sleep(f64),
    Done,
}

/// How deeply reporter blocks may call each other, and how many steps one
/// reporter body may take before it is given up on. Both are the VM's own
/// limits; `codegen`'s tests hold these against them.
pub const MAX_REPORTER_DEPTH: usize = 32;
pub const STEP_BUDGET: usize = 10_000;

/// One custom block being run as a statement: where to carry on afterwards,
/// and what its inputs were bound to.
pub struct CallFrame {
    return_pc: usize,
    params: Vec<Val>,
}

/// One run of one strand: where it is, and what it was in the middle of.
///
/// A compiled strand is a `match` over the program counter rather than a
/// recursive walk, for the same reason the VM flattens its blocks - a nested
/// body can't be suspended, but a number can be put down and picked up.
pub struct State {
    /// Whose strand this is. An `Rc` because every reporter body this strand
    /// runs is handed the same one.
    pub me: Rc<str>,
    pub pc: usize,
    pub status: Status,
    /// Seconds since the run started. The scheduler sets it before each call.
    pub now: f64,
    /// `stop all` was reached, and the scheduler should end the whole run.
    pub stopping: bool,
    /// Iterations left, one slot per `repeat` in the actor. Loop nesting is
    /// known when the code is emitted, so this is a flat array rather than
    /// the VM's frame stack. A custom block that could call itself is refused
    /// at compile time, which is what makes one slot per loop enough.
    counters: Vec<i64>,
    /// Custom blocks entered as statements, innermost last.
    calls: Vec<CallFrame>,
}

impl State {
    /// A return address that means "hand the value back to whoever asked"
    /// rather than "carry on at this step": a reporter body's own boundary.
    pub const RETURN: usize = usize::MAX;

    pub fn new(me: &Rc<str>, start: usize, counters: usize) -> Self {
        Self {
            me: Rc::clone(me),
            pc: start,
            status: Status::Run,
            now: 0.0,
            stopping: false,
            counters: vec![0; counters],
            calls: Vec::new(),
        }
    }

    pub fn enter_call(&mut self, return_pc: usize, params: Vec<Val>) {
        self.calls.push(CallFrame { return_pc, params });
    }

    /// Leaves the innermost custom block, answering the step to carry on at.
    /// `None` ends the run instead: either nothing called this, or what did
    /// was a reporter waiting on the value.
    pub fn resume_at(&mut self) -> Option<usize> {
        match self.calls.pop() {
            Some(frame) if frame.return_pc != Self::RETURN => Some(frame.return_pc),
            _ => None,
        }
    }

    /// One of the innermost call's bound inputs. Anything a caller didn't
    /// pass reads as zero, as an unbound name does in the VM.
    pub fn param(&self, index: usize) -> Val {
        self.calls
            .last()
            .and_then(|frame| frame.params.get(index))
            .cloned()
            .unwrap_or(Val::Num(0.0))
    }

    /// Whether there is anything to do this frame. A sleeping strand wakes on
    /// the first frame at or past its deadline, as the VM's does.
    pub fn resume(&mut self) -> bool {
        match self.status {
            Status::Done => false,
            Status::Sleep(until) => {
                if self.now < until {
                    return false;
                }
                self.status = Status::Run;
                true
            }
            Status::Run => true,
        }
    }

    pub fn done(&self) -> bool {
        self.status == Status::Done
    }

    pub fn sleep(&mut self, seconds: f64) {
        self.status = Status::Sleep(self.now + seconds);
    }

    pub fn finish(&mut self) {
        self.status = Status::Done;
    }

    /// Enters a `repeat` with `n` iterations to go.
    pub fn enter_repeat(&mut self, loop_index: usize, n: i64) {
        self.counters[loop_index] = n;
    }

    /// Counts one iteration off, and answers how many are left.
    pub fn next_iteration(&mut self, loop_index: usize) -> i64 {
        self.counters[loop_index] -= 1;
        self.counters[loop_index]
    }
}

/// Everything a generated program can reach outside itself. Five methods
/// rather than one per verb: a new block is a new [`Act`] or a new `kind`
/// string, never a change to this trait, which is what keeps a program built
/// against an older Blockloom loadable by a newer one.
pub trait Host {
    fn act(&mut self, actor: &str, act: Act);
    /// A sensing reporter, by the name its operator is registered under.
    /// Arguments arrive evaluated, and an error comes back as one, the way
    /// the operator itself would answer.
    fn sense(&mut self, actor: &str, kind: &str, args: &[Val]) -> R;
    fn variable(&mut self, actor: &str, name: &str) -> Val;
    fn set_variable(&mut self, actor: &str, name: &str, value: Val);
    /// A value that wouldn't evaluate. The program carries on with a zero.
    fn error(&mut self, actor: &str, message: &str);
}

// --- Native logic boundary -------------------------------------------------

pub const LOGIC_ABI_VERSION: u32 = 3;
pub const ABI_OK: u32 = 0;
pub const ABI_TOO_LONG: u32 = 1;
pub const ABI_MISSING: u32 = 2;
pub const ABI_PANIC: u32 = 3;
pub const TICK_STOPPED: u32 = 1;

pub const VALUE_NUMBER: u32 = 0;
pub const VALUE_TEXT: u32 = 1;
pub const VALUE_BOOL: u32 = 2;
pub const VALUE_ERROR: u32 = 3;

pub const READ_SENSE: u32 = 1;
pub const READ_VARIABLE: u32 = 2;

pub const ACT_MOVE: u32 = 1;
pub const ACT_GO_TO: u32 = 2;
pub const ACT_CHANGE_POSITION: u32 = 3;
pub const ACT_GLIDE: u32 = 4;
pub const ACT_TURN: u32 = 5;
pub const ACT_SET_ROTATION: u32 = 6;
pub const ACT_POINT_TOWARDS: u32 = 7;
pub const ACT_SET_SCALE: u32 = 8;
pub const ACT_SET_BODY: u32 = 9;
pub const ACT_APPLY_IMPULSE: u32 = 10;
pub const ACT_SET_VELOCITY: u32 = 11;
pub const ACT_SET_GRAVITY: u32 = 12;
pub const ACT_SET_DENSITY: u32 = 13;
pub const ACT_SET_MASS: u32 = 14;
pub const ACT_SAY: u32 = 15;
pub const ACT_SET_VISIBLE: u32 = 16;
pub const ACT_SET_COLOR: u32 = 17;
pub const ACT_SET_FIELD: u32 = 18;
pub const ACT_SET_CAMERA_VIEW: u32 = 19;
pub const ACT_ATTACH: u32 = 20;
pub const ACT_DETACH: u32 = 21;
pub const ACT_BROADCAST: u32 = 22;
pub const ACT_ERROR: u32 = 23;
pub const ACT_SET_PARENT: u32 = 24;
pub const ACT_CREATE_CLONE: u32 = 25;
pub const ACT_CREATE_ACTOR: u32 = 26;
pub const ACT_DELETE_ACTOR: u32 = 27;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AbiStr {
    pub ptr: *const u8,
    pub len: usize,
}

impl AbiStr {
    pub const EMPTY: Self = Self {
        ptr: std::ptr::null(),
        len: 0,
    };

    pub fn borrow(value: &str) -> Self {
        Self {
            ptr: value.as_ptr(),
            len: value.len(),
        }
    }

    /// # Safety
    /// The pointer must name UTF-8 for `len` bytes for this call.
    pub unsafe fn as_str<'a>(self) -> &'a str {
        if self.ptr.is_null() || self.len == 0 {
            return "";
        }
        let bytes = unsafe { std::slice::from_raw_parts(self.ptr, self.len) };
        std::str::from_utf8(bytes).unwrap_or("")
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AbiValue {
    pub kind: u32,
    pub number: f64,
    pub text: AbiStr,
}

impl AbiValue {
    fn borrow(value: &Val) -> Self {
        match value {
            Val::Num(number) => Self {
                kind: VALUE_NUMBER,
                number: *number,
                text: AbiStr::EMPTY,
            },
            Val::Text(text) => Self {
                kind: VALUE_TEXT,
                number: 0.0,
                text: AbiStr::borrow(text),
            },
            Val::Bool(value) => Self {
                kind: VALUE_BOOL,
                number: if *value { 1.0 } else { 0.0 },
                text: AbiStr::EMPTY,
            },
        }
    }
}

#[repr(C)]
pub struct LogicHostApi {
    pub abi: u32,
    pub read: extern "C" fn(
        *mut std::ffi::c_void,
        AbiStr,
        u32,
        AbiStr,
        *const AbiValue,
        usize,
        *mut AbiValue,
        *mut u8,
        usize,
        *mut usize,
    ) -> u32,
    pub set_variable: extern "C" fn(*mut std::ffi::c_void, AbiStr, AbiStr, AbiValue),
    pub act:
        extern "C" fn(*mut std::ffi::c_void, AbiStr, u32, AbiStr, AbiStr, f64, f64, f64, AbiValue),
}

/// A generated program's safe view of the runtime callbacks.
pub struct AbiHost {
    ctx: *mut std::ffi::c_void,
    api: *const LogicHostApi,
}

impl AbiHost {
    /// # Safety
    /// Both pointers are supplied by the runtime for the duration of a call.
    pub unsafe fn new(ctx: *mut std::ffi::c_void, api: *const LogicHostApi) -> Self {
        Self { ctx, api }
    }

    fn api(&self) -> &LogicHostApi {
        unsafe { &*self.api }
    }

    fn read_value(&mut self, actor: &str, what: u32, name: &str, args: &[Val]) -> R {
        let wire: Vec<AbiValue> = args.iter().map(AbiValue::borrow).collect();
        let mut value = AbiValue {
            kind: VALUE_NUMBER,
            number: 0.0,
            text: AbiStr::EMPTY,
        };
        let mut needed = 0;
        let mut text = vec![0_u8; 256];
        loop {
            let status = (self.api().read)(
                self.ctx,
                AbiStr::borrow(actor),
                what,
                AbiStr::borrow(name),
                wire.as_ptr(),
                wire.len(),
                &mut value,
                text.as_mut_ptr(),
                text.len(),
                &mut needed,
            );
            if status == ABI_TOO_LONG {
                text.resize(needed, 0);
                continue;
            }
            if status == ABI_MISSING {
                return Ok(Val::Num(0.0));
            }
            if status != ABI_OK {
                return Err("compiled logic host failed to answer".to_string());
            }
            return match value.kind {
                VALUE_NUMBER => Ok(Val::Num(value.number)),
                VALUE_BOOL => Ok(Val::Bool(value.number != 0.0)),
                VALUE_TEXT => Ok(Val::Text(
                    String::from_utf8_lossy(&text[..needed]).into_owned(),
                )),
                VALUE_ERROR => Err(String::from_utf8_lossy(&text[..needed]).into_owned()),
                _ => Err("compiled logic host returned an invalid value".to_string()),
            };
        }
    }

    fn act_wire(
        &mut self,
        actor: &str,
        kind: u32,
        a: &str,
        b: &str,
        numbers: [f64; 3],
        value: &Val,
    ) {
        (self.api().act)(
            self.ctx,
            AbiStr::borrow(actor),
            kind,
            AbiStr::borrow(a),
            AbiStr::borrow(b),
            numbers[0],
            numbers[1],
            numbers[2],
            AbiValue::borrow(value),
        );
    }
}

impl Host for AbiHost {
    fn act(&mut self, actor: &str, act: Act) {
        let zero = Val::Num(0.0);
        match act {
            Act::Move { steps } => {
                self.act_wire(actor, ACT_MOVE, "", "", [steps as f64, 0.0, 0.0], &zero)
            }
            Act::GoTo { position } => {
                self.act_wire(actor, ACT_GO_TO, "", "", position.map(f64::from), &zero)
            }
            Act::ChangePosition { axis, by } => self.act_wire(
                actor,
                ACT_CHANGE_POSITION,
                "",
                "",
                [axis as f64, by as f64, 0.0],
                &zero,
            ),
            Act::Glide { seconds, target } => self.act_wire(
                actor,
                ACT_GLIDE,
                "",
                "",
                [seconds as f64, target[0] as f64, target[1] as f64],
                &Val::Num(target[2] as f64),
            ),
            Act::Turn { axis, degrees } => self.act_wire(
                actor,
                ACT_TURN,
                "",
                "",
                [axis as f64, degrees as f64, 0.0],
                &zero,
            ),
            Act::SetRotation { axis, degrees } => self.act_wire(
                actor,
                ACT_SET_ROTATION,
                "",
                "",
                [axis as f64, degrees as f64, 0.0],
                &zero,
            ),
            Act::PointTowards { target } => {
                self.act_wire(actor, ACT_POINT_TOWARDS, target, "", [0.0; 3], &zero)
            }
            Act::SetScale { factor } => self.act_wire(
                actor,
                ACT_SET_SCALE,
                "",
                "",
                [factor as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetBody { body } => self.act_wire(actor, ACT_SET_BODY, body, "", [0.0; 3], &zero),
            Act::ApplyImpulse { impulse } => self.act_wire(
                actor,
                ACT_APPLY_IMPULSE,
                "",
                "",
                impulse.map(f64::from),
                &zero,
            ),
            Act::SetVelocity { velocity } => self.act_wire(
                actor,
                ACT_SET_VELOCITY,
                "",
                "",
                velocity.map(f64::from),
                &zero,
            ),
            Act::SetGravity { gravity } => self.act_wire(
                actor,
                ACT_SET_GRAVITY,
                "",
                "",
                gravity.map(f64::from),
                &zero,
            ),
            Act::SetDensity { density } => self.act_wire(
                actor,
                ACT_SET_DENSITY,
                "",
                "",
                [density as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetMass { mass } => {
                self.act_wire(actor, ACT_SET_MASS, "", "", [mass as f64, 0.0, 0.0], &zero)
            }
            Act::Say { text } => self.act_wire(actor, ACT_SAY, &text, "", [0.0; 3], &zero),
            Act::SetVisible { visible } => self.act_wire(
                actor,
                ACT_SET_VISIBLE,
                "",
                "",
                [if visible { 1.0 } else { 0.0 }, 0.0, 0.0],
                &zero,
            ),
            Act::SetColor { color } => {
                self.act_wire(actor, ACT_SET_COLOR, &color, "", [0.0; 3], &zero)
            }
            Act::SetComponentField {
                component,
                field,
                value,
            } => self.act_wire(actor, ACT_SET_FIELD, component, field, [0.0; 3], &value),
            Act::SetCameraView { view } => {
                self.act_wire(actor, ACT_SET_CAMERA_VIEW, view, "", [0.0; 3], &zero)
            }
            Act::AttachComponent { component } => {
                self.act_wire(actor, ACT_ATTACH, component, "", [0.0; 3], &zero)
            }
            Act::DetachComponent { component } => {
                self.act_wire(actor, ACT_DETACH, component, "", [0.0; 3], &zero)
            }
            Act::SetParent { target } => {
                self.act_wire(actor, ACT_SET_PARENT, &target, "", [0.0; 3], &zero)
            }
            Act::CreateClone { of, clone } => {
                self.act_wire(actor, ACT_CREATE_CLONE, &of, &clone, [0.0; 3], &zero)
            }
            Act::CreateActor { id, name, position } => self.act_wire(
                actor,
                ACT_CREATE_ACTOR,
                &id,
                &name,
                position.map(f64::from),
                &zero,
            ),
            Act::DeleteActor { target } => {
                self.act_wire(actor, ACT_DELETE_ACTOR, &target, "", [0.0; 3], &zero)
            }
            Act::Broadcast { name } => {
                self.act_wire(actor, ACT_BROADCAST, name, "", [0.0; 3], &zero)
            }
        }
    }

    fn sense(&mut self, actor: &str, kind: &str, args: &[Val]) -> R {
        self.read_value(actor, READ_SENSE, kind, args)
    }

    fn variable(&mut self, actor: &str, name: &str) -> Val {
        self.read_value(actor, READ_VARIABLE, name, &[])
            .unwrap_or(Val::Num(0.0))
    }

    fn set_variable(&mut self, actor: &str, name: &str, value: Val) {
        (self.api().set_variable)(
            self.ctx,
            AbiStr::borrow(actor),
            AbiStr::borrow(name),
            AbiValue::borrow(&value),
        );
    }

    fn error(&mut self, actor: &str, message: &str) {
        self.act_wire(actor, ACT_ERROR, message, "", [0.0; 3], &Val::Num(0.0));
    }
}

pub const SYM_LOGIC_ABI: &[u8] = b"blockloom_logic_abi";
pub const SYM_LOGIC_NEW: &[u8] = b"blockloom_logic_new";
pub const SYM_LOGIC_FREE: &[u8] = b"blockloom_logic_free";
pub const SYM_LOGIC_RESET: &[u8] = b"blockloom_logic_reset";
pub const SYM_LOGIC_FIRE: &[u8] = b"blockloom_logic_fire";
pub const SYM_LOGIC_TICK: &[u8] = b"blockloom_logic_tick";

// ─── Reading a value at an instruction's slot ───────────────────────────────
// The VM reports a bad slot once and stands a zero in its place; a slot that
// evaluated fine but isn't a number is simply zero, with nothing reported.
// Both halves of that matter, so each has a function here.

pub fn number(host: &mut dyn Host, actor: &str, value: R) -> f32 {
    number_f64(host, actor, value) as f32
}

pub fn number_f64(host: &mut dyn Host, actor: &str, value: R) -> f64 {
    match value {
        Ok(value) => value.as_number().unwrap_or(0.0),
        Err(message) => {
            host.error(actor, &message);
            0.0
        }
    }
}

/// A reporter block nested too deeply to run. The complaint is the VM's, word
/// for word, and the call reads as zero.
pub fn too_deep(host: &mut dyn Host, actor: &str) -> Val {
    host.error(actor, "a reporter block calls itself too deeply");
    Val::Num(0.0)
}

pub fn text(host: &mut dyn Host, actor: &str, value: R) -> String {
    evaluated(host, actor, value).as_text()
}

/// A condition slot: an `if`, a `while`, a `wait until`. A bad one reports
/// itself and reads as false, since a zero isn't true.
pub fn boolean(host: &mut dyn Host, actor: &str, value: R) -> bool {
    evaluated(host, actor, value).as_bool()
}

pub fn evaluated(host: &mut dyn Host, actor: &str, value: R) -> Val {
    match value {
        Ok(value) => value,
        Err(message) => {
            host.error(actor, &message);
            Val::Num(0.0)
        }
    }
}

/// A sensing reporter, with its arguments evaluated left to right first - an
/// extension operator can't short-circuit, so a bad argument is the answer.
pub fn sense(host: &mut dyn Host, actor: &str, kind: &str, args: Vec<R>) -> R {
    let mut evaluated = Vec::with_capacity(args.len());
    for arg in args {
        evaluated.push(arg?);
    }
    host.sense(actor, kind, &evaluated)
}

// ─── Operators ──────────────────────────────────────────────────────────────

pub fn add(a: R, b: R) -> R {
    Ok(Val::Num(a?.as_number()? + b?.as_number()?))
}

pub fn sub(a: R, b: R) -> R {
    Ok(Val::Num(a?.as_number()? - b?.as_number()?))
}

pub fn mul(a: R, b: R) -> R {
    Ok(Val::Num(a?.as_number()? * b?.as_number()?))
}

pub fn div(a: R, b: R) -> R {
    let (l, r) = (a?.as_number()?, b?.as_number()?);
    if r == 0.0 {
        return Err("division by zero".to_string());
    }
    Ok(Val::Num(l / r))
}

pub fn modulo(a: R, b: R) -> R {
    let (l, r) = (a?.as_number()?, b?.as_number()?);
    if r == 0.0 {
        return Err("mod by zero".to_string());
    }
    Ok(Val::Num(l.rem_euclid(r)))
}

pub fn round(a: R) -> R {
    Ok(Val::Num(a?.as_number()?.round()))
}

pub fn math(function: R, a: R) -> R {
    let function = function?.as_text();
    let n = a?.as_number()?;
    let to_radians = std::f64::consts::PI / 180.0;
    let to_degrees = 180.0 / std::f64::consts::PI;
    let result = match function.as_str() {
        "Abs" => n.abs(),
        "Floor" => n.floor(),
        "Ceiling" => n.ceil(),
        "Sign" => {
            if n > 0.0 {
                1.0
            } else if n < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        "Sqrt" => n.sqrt(),
        "Sin" => (n * to_radians).sin(),
        "Cos" => (n * to_radians).cos(),
        "Tan" => (n * to_radians).tan(),
        "Asin" => n.asin() * to_degrees,
        "Acos" => n.acos() * to_degrees,
        "Atan" => n.atan() * to_degrees,
        "Ln" => n.ln(),
        "Log" => n.log10(),
        "Log2" => n.log2(),
        "EPower" => n.exp(),
        "TenPower" => 10.0_f64.powf(n),
        other => return Err(format!("unknown math function '{other}'")),
    };
    Ok(Val::Num(result))
}

pub fn join(parts: Vec<R>) -> R {
    let mut joined = String::new();
    for part in parts {
        joined.push_str(&part?.as_text());
    }
    Ok(Val::Text(joined))
}

pub fn length(a: R) -> R {
    Ok(Val::Num(a?.as_text().chars().count() as f64))
}

pub fn index_of(needle: R, haystack: R) -> R {
    let needle = needle?.as_text();
    let haystack = haystack?.as_text();
    Ok(Val::Num(char_index_of(&haystack, &needle) as f64))
}

pub fn last_index_of(needle: R, haystack: R) -> R {
    let needle = needle?.as_text();
    let haystack = haystack?.as_text();
    Ok(Val::Num(char_last_index_of(&haystack, &needle) as f64))
}

pub fn letter_of(index: R, a: R) -> R {
    let index = index?.as_number()? as i64;
    let text = a?.as_text();
    let chars: Vec<char> = text.chars().collect();
    if index < 1 || index as usize > chars.len() {
        return Err(format!(
            "letter {index} is out of range for a {}-character value",
            chars.len()
        ));
    }
    Ok(Val::Text(chars[index as usize - 1].to_string()))
}

pub fn case(a: R, which: R) -> R {
    let text = a?.as_text();
    let upper = which?.as_text() == "Upper";
    Ok(Val::Text(if upper {
        text.to_uppercase()
    } else {
        text.to_lowercase()
    }))
}

pub fn eq(a: R, b: R) -> R {
    Ok(Val::Bool(values_equal(&a?, &b?)))
}

pub fn neq(a: R, b: R) -> R {
    Ok(Val::Bool(!values_equal(&a?, &b?)))
}

pub fn gt(a: R, b: R) -> R {
    Ok(Val::Bool(a?.as_number()? > b?.as_number()?))
}

pub fn lt(a: R, b: R) -> R {
    Ok(Val::Bool(a?.as_number()? < b?.as_number()?))
}

pub fn gte(a: R, b: R) -> R {
    Ok(Val::Bool(a?.as_number()? >= b?.as_number()?))
}

pub fn lte(a: R, b: R) -> R {
    Ok(Val::Bool(a?.as_number()? <= b?.as_number()?))
}

/// `and` and `or` take the second operand unevaluated, because the VM's
/// short-circuit is observable: `false and <a bad slot>` reports nothing.
pub fn and(a: R, b: impl FnOnce() -> R) -> R {
    if !a?.as_bool() {
        return Ok(Val::Bool(false));
    }
    Ok(Val::Bool(b()?.as_bool()))
}

pub fn or(a: R, b: impl FnOnce() -> R) -> R {
    if a?.as_bool() {
        return Ok(Val::Bool(true));
    }
    Ok(Val::Bool(b()?.as_bool()))
}

pub fn not(a: R) -> R {
    Ok(Val::Bool(!a?.as_bool()))
}

/// `Op::Eq`'s rule: numeric when both sides parse as numbers, text otherwise.
fn values_equal(l: &Val, r: &Val) -> bool {
    match (l.as_number(), r.as_number()) {
        (Ok(l), Ok(r)) => l == r,
        _ => l.as_text() == r.as_text(),
    }
}

/// 1-based char index of the first `needle` in `haystack`, 0 for none.
fn char_index_of(haystack: &str, needle: &str) -> usize {
    let h: Vec<char> = haystack.chars().collect();
    let n: Vec<char> = needle.chars().collect();
    if n.is_empty() || n.len() > h.len() {
        return 0;
    }
    (0..=h.len() - n.len())
        .find(|&i| h[i..i + n.len()] == n[..])
        .map_or(0, |i| i + 1)
}

/// [`char_index_of`] from the other end.
fn char_last_index_of(haystack: &str, needle: &str) -> usize {
    let h: Vec<char> = haystack.chars().collect();
    let n: Vec<char> = needle.chars().collect();
    if n.is_empty() || n.len() > h.len() {
        return 0;
    }
    (0..=h.len() - n.len())
        .rev()
        .find(|&i| h[i..i + n.len()] == n[..])
        .map_or(0, |i| i + 1)
}
