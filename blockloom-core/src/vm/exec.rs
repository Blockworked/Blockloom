//! Running compiled programs.
//!
//! Every live script is a program counter plus a small frame stack, stepped
//! by [`Vm::tick`] once per rendered frame. A script gives the frame back at a
//! `wait`, at every loop iteration, and when it finishes - so a `forever` loop
//! costs one iteration per frame instead of hanging the host.

use super::effect::Effect;
use super::lower::{Expr, Loaded, lower};
use super::ops;
use super::program::{Action, LoopKind, Step, Trigger, compile};
use super::stores;
use super::variables::{ScopeId, VarId, VariableSnapshot, Variables};
use crate::project::Project;
use crate::sense;
use crate::sound::{clamp_pitch, normalize_sound, user_to_gain};
use crate::ui::{UiElement, UiKind};
use crate::value::{Evaluated, Op, Value};
use blockstitch_core::graph::{
    DictEntry, DictItem, ListItem, dict_remove, dict_set, list_index, parse_json_array,
    parse_json_object, resolve_dict_reporter, resolve_list_reporter,
};
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// A scene transition by the name a block spells it. Unknown spellings read
/// as `none`, so a typo fades nothing rather than erroring mid-run.
pub fn normalize_transition(name: &str) -> String {
    let key: String = name
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect();
    match key.as_str() {
        "fade" => "fade".to_string(),
        "wipe" => "wipe".to_string(),
        "circle" => "circle".to_string(),
        _ => "none".to_string(),
    }
}

/// One scope's lists, by name.
pub type ListValues = HashMap<String, Vec<ListItem>>;
/// Every actor's own lists, by actor id.
pub type ActorLists = HashMap<String, ListValues>;

#[derive(Debug, Default)]
struct ListState {
    globals: ListValues,
    actors: ActorLists,
}

/// The live lists of one run, parallel to [`Variables`]. An actor reads its
/// own list first, then the project's shared one - the same shadowing rule
/// variables use - and writes land wherever the name was declared.
#[derive(Debug, Clone, Default)]
pub struct Lists(Rc<RefCell<ListState>>);

impl Lists {
    pub fn load(&self, project: &Project) {
        let mut state = self.0.borrow_mut();
        state.globals = project
            .global_lists
            .iter()
            .map(|list| (list.name.clone(), list.items.clone()))
            .collect();
        state.actors = project
            .actors
            .iter()
            .map(|actor| (actor.id.clone(), actor.graph.list_values()))
            .collect();
    }

    /// Reloads one scene's actors without touching the shared lists: globals
    /// keep what the run has written, actor locals start as authored. What a
    /// `switch scene to` calls, so a score carries over and a clone's bag
    /// does not.
    pub fn load_scene(&self, project: &Project) {
        self.load_scene_keep(project, &std::collections::HashSet::new());
    }

    /// The same, keeping live lists for `keep` (survivor ids).
    pub fn load_scene_keep(&self, project: &Project, keep: &std::collections::HashSet<String>) {
        let mut state = self.0.borrow_mut();
        state.actors.retain(|id, _| keep.contains(id));
        for actor in &project.actors {
            if keep.contains(&actor.id) {
                continue;
            }
            state
                .actors
                .insert(actor.id.clone(), actor.graph.list_values());
        }
    }

    /// What `actor` reads: its own lists over the shared ones.
    pub fn snapshot_for(&self, actor: &str) -> ListValues {
        let state = self.0.borrow();
        let mut merged = state.globals.clone();
        if let Some(own) = state.actors.get(actor) {
            merged.extend(own.clone());
        }
        merged
    }

    /// The one list `name` means to `actor`, as the only entry of a scope -
    /// what a list reporter reads when its name is a plain literal, without
    /// copying every other list along with it.
    fn scope_of(&self, actor: &str, name: &str) -> ListValues {
        let state = self.0.borrow();
        state
            .actors
            .get(actor)
            .and_then(|lists| lists.get(name))
            .or_else(|| state.globals.get(name))
            .map(|list| HashMap::from([(name.to_string(), list.clone())]))
            .unwrap_or_default()
    }

    /// Mutates whichever scope declared `name` - the actor's own first, then
    /// the shared one. A name nobody declared is a no-op, the way reading an
    /// unknown list answers empty.
    pub fn with_list_mut(&self, actor: &str, name: &str, f: impl FnOnce(&mut Vec<ListItem>)) {
        let mut state = self.0.borrow_mut();
        if let Some(list) = state
            .actors
            .get_mut(actor)
            .and_then(|lists| lists.get_mut(name))
        {
            f(list);
            return;
        }
        if let Some(list) = state.globals.get_mut(name) {
            f(list);
        }
    }

    /// Gives `to` its own copy of `from`'s lists, as they stand. A clone
    /// starts life holding whatever its template held, and changes either
    /// way after that.
    pub fn copy_actor(&self, from: &str, to: &str) {
        let mut state = self.0.borrow_mut();
        let copied = state.actors.get(from).cloned().unwrap_or_default();
        state.actors.insert(to.to_string(), copied);
    }

    /// Forgets an actor's own lists. A deleted actor is gone for the rest of
    /// the run, and so is what it was holding.
    pub fn forget_actor(&self, actor: &str) {
        self.0.borrow_mut().actors.remove(actor);
    }
}

/// One scope's dicts, by name.
pub type DictValues = HashMap<String, Vec<DictEntry>>;
/// Every actor's own dicts, by actor id.
pub type ActorDicts = HashMap<String, DictValues>;

#[derive(Debug, Default)]
struct DictState {
    globals: DictValues,
    actors: ActorDicts,
}

/// The live dicts of one run, parallel to [`Lists`]. An actor reads its own
/// dict first, then the project's shared one - the same shadowing rule
/// lists use - and writes land wherever the name was declared.
#[derive(Debug, Clone, Default)]
pub struct Dicts(Rc<RefCell<DictState>>);

impl Dicts {
    pub fn load(&self, project: &Project) {
        let mut state = self.0.borrow_mut();
        state.globals = project
            .global_dicts
            .iter()
            .map(|dict| (dict.name.clone(), dict.entries.clone()))
            .collect();
        state.actors = project
            .actors
            .iter()
            .map(|actor| (actor.id.clone(), actor.graph.dict_values()))
            .collect();
    }

    /// Reloads one scene's actors without touching the shared dicts, the way
    /// [`Lists::load_scene`] does for lists.
    pub fn load_scene(&self, project: &Project) {
        self.load_scene_keep(project, &std::collections::HashSet::new());
    }

    /// The same, keeping live dicts for `keep` (survivor ids).
    pub fn load_scene_keep(&self, project: &Project, keep: &std::collections::HashSet<String>) {
        let mut state = self.0.borrow_mut();
        state.actors.retain(|id, _| keep.contains(id));
        for actor in &project.actors {
            if keep.contains(&actor.id) {
                continue;
            }
            state
                .actors
                .insert(actor.id.clone(), actor.graph.dict_values());
        }
    }

    /// What `actor` reads: its own dicts over the shared ones.
    pub fn snapshot_for(&self, actor: &str) -> DictValues {
        let state = self.0.borrow();
        let mut merged = state.globals.clone();
        if let Some(own) = state.actors.get(actor) {
            merged.extend(own.clone());
        }
        merged
    }

    /// The one dict `name` means to `actor`, as the only entry of a scope.
    fn scope_of(&self, actor: &str, name: &str) -> DictValues {
        let state = self.0.borrow();
        state
            .actors
            .get(actor)
            .and_then(|dicts| dicts.get(name))
            .or_else(|| state.globals.get(name))
            .map(|dict| HashMap::from([(name.to_string(), dict.clone())]))
            .unwrap_or_default()
    }

    /// Mutates whichever scope declared `name` - the actor's own first, then
    /// the shared one. A name nobody declared is a no-op, the way reading an
    /// unknown dict answers empty.
    pub fn with_dict_mut(&self, actor: &str, name: &str, f: impl FnOnce(&mut Vec<DictEntry>)) {
        let mut state = self.0.borrow_mut();
        if let Some(dict) = state
            .actors
            .get_mut(actor)
            .and_then(|dicts| dicts.get_mut(name))
        {
            f(dict);
            return;
        }
        if let Some(dict) = state.globals.get_mut(name) {
            f(dict);
        }
    }

    /// Gives `to` its own copy of `from`'s dicts, as they stand. A clone
    /// starts life holding whatever its template held, and changes either
    /// way after that.
    pub fn copy_actor(&self, from: &str, to: &str) {
        let mut state = self.0.borrow_mut();
        let copied = state.actors.get(from).cloned().unwrap_or_default();
        state.actors.insert(to.to_string(), copied);
    }

    /// Forgets an actor's own dicts. A deleted actor is gone for the rest of
    /// the run, and so is what it was holding.
    pub fn forget_actor(&self, actor: &str) {
        self.0.borrow_mut().actors.remove(actor);
    }
}

/// Steps one script may take in a single frame before being made to yield.
/// Loops yield on their own; this only catches pathological straight-line code.
pub const STEP_BUDGET: usize = 10_000;

/// How deeply reporter blocks may call each other.
pub const MAX_REPORTER_DEPTH: usize = 32;

/// Something that can start scripts.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The green flag: every `when the project starts` strand.
    Started,
    /// The newly loaded scene finished warming up: every `when scene
    /// starts` strand in it.
    SceneStarted,
    /// The outgoing scene is about to unload: every `when scene ends`
    /// strand in it.
    SceneEnded,
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
    /// A fresh clone is ready to run its own `when I start as a clone`.
    Cloned {
        actor: String,
    },
    /// A `Once` clip finished, in the actor playing it. An empty filter
    /// matches any clip; the event always names the one that ended.
    AnimationEnded {
        actor: String,
        clip: String,
    },
    /// The actor's particles spawned, died or hit something this frame.
    Particles {
        actor: String,
        event: crate::vfx::ParticleEvent,
    },
    /// The actor's clip reached a frame marker (or a rig event).
    AnimationMarker {
        actor: String,
        marker: String,
    },
    /// `actor` walked into the room actor named `room`.
    EnteredRoom {
        actor: String,
        room: String,
    },
    /// The named input action went down, in lowercase action spelling.
    Action(String),
    /// A finger touched the screen.
    Touched,
    /// An interface element was clicked.
    UiEvent {
        id: String,
        event: String,
    },
    UiClicked {
        id: String,
    },
    /// An input element changed. The value travels with it for a host that
    /// wants it; matching is on the id alone, since `value of (id)` is how a
    /// strand reads what it became.
    UiChanged {
        id: String,
        value: Evaluated,
    },
}

impl Event {
    /// True for an event the interface raised. A strand one of these starts
    /// keeps running while the game is paused.
    pub fn is_ui(&self) -> bool {
        matches!(
            self,
            Event::UiEvent { .. } | Event::UiClicked { .. } | Event::UiChanged { .. }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Status {
    Run,
    /// Asleep until this timestamp, in run seconds.
    Sleep(f64),
    Done,
}

/// A custom-block call's bound inputs, in declaration order. A block has a
/// handful, so a scan beats hashing the name.
#[derive(Debug, Clone)]
struct Params {
    names: Rc<[String]>,
    values: Vec<Evaluated>,
}

impl Params {
    /// The value bound to `name`. A repeated input name reads the last one.
    fn get(&self, name: &str) -> Option<&Evaluated> {
        self.names
            .iter()
            .zip(&self.values)
            .rev()
            .find(|(bound, _)| *bound == name)
            .map(|(_, value)| value)
    }
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
        params: Params,
        /// Where an `Invoke` stores its value in the caller, if anywhere.
        temp: Option<usize>,
        /// The caller's temps, restored when this call leaves.
        saved_temps: Vec<Evaluated>,
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
    /// Started by a UI event: it keeps ticking while the game is paused, and
    /// it sleeps against the wall clock rather than the frozen world one.
    ui: bool,
    /// Reporter results waiting for the step that asked for them.
    temps: Vec<Evaluated>,
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
            ui: false,
            temps: Vec::new(),
        }
    }
}

/// The block virtual machine: compiled programs, live scripts and variables
/// for one loaded project.
pub struct Vm {
    programs: FxHashMap<String, Rc<Loaded>>,
    /// Actor id -> name, for matching a collision against `when I touch`.
    names: FxHashMap<String, String>,
    /// Clone id -> the authored actor it is a copy of. Only runtime clones
    /// are in here, which is what makes "am I a clone?" answerable.
    clones: FxHashMap<String, String>,
    /// Actors deleted during this tick. Their scripts are dropped at the end
    /// of it, and the one that ran the block stops where it stands.
    deleted: Vec<String>,
    variables: Variables,
    /// The program being stepped, whose slots `eval` lowers and caches.
    current: Option<Rc<Loaded>>,
    /// The running actor's variable scope, as last resolved.
    scope: Option<ScopeId>,
    lists: Lists,
    dicts: Dicts,
    scripts: Vec<Script>,
    /// Events to start scripts for, drained at the top of the next tick.
    pending: Vec<Event>,
    now: f64,
    /// The wall clock: never frozen, and what a UI strand's `wait` counts
    /// against so a pause menu can still animate.
    wall: f64,
    /// Whether the world is frozen. Only UI strands advance while it is.
    paused: bool,
    /// Set by `stop all`, which invalidates the script list mid-tick.
    stopping: bool,
    depth: usize,
    /// How many actors this run has made, which is where the next one's id
    /// comes from.
    made: usize,
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

impl Vm {
    pub fn new() -> Self {
        Self::with_variables(Variables::default())
    }

    pub fn with_variables(variables: Variables) -> Self {
        Self::with_stores(variables, Lists::default(), Dicts::default())
    }

    pub fn with_stores(variables: Variables, lists: Lists, dicts: Dicts) -> Self {
        Self {
            programs: FxHashMap::default(),
            names: FxHashMap::default(),
            clones: FxHashMap::default(),
            deleted: Vec::new(),
            variables,
            current: None,
            scope: None,
            lists,
            dicts,
            scripts: Vec::new(),
            pending: Vec::new(),
            now: 0.0,
            wall: 0.0,
            paused: false,
            stopping: false,
            depth: 0,
            made: 0,
        }
    }

    /// Compiles every actor's canvas and takes the project's variables as the
    /// starting values. Drops any scripts from a previous run.
    pub fn load(&mut self, project: &Project) {
        self.programs.clear();
        self.names.clear();
        self.clones.clear();
        self.deleted.clear();
        self.scripts.clear();
        self.pending.clear();
        self.stopping = false;
        self.paused = false;
        self.made = 0;
        self.variables.load(project);
        self.lists.load(project);
        self.dicts.load(project);
        for actor in &project.actors {
            self.names.insert(actor.id.clone(), actor.name.clone());
            let inputs = actor
                .graph
                .block_defs
                .iter()
                .map(|def| {
                    (
                        def.id.clone(),
                        def.input_names()
                            .map(str::to_string)
                            .collect::<Rc<[String]>>(),
                    )
                })
                .collect();
            let program = Loaded::new(compile(&actor.graph), inputs);
            self.programs.insert(actor.id.clone(), Rc::new(program));
        }
    }

    /// Loads another scene mid-run: the new scene's actors compile fresh,
    /// actor locals start as authored, and globals plus shared lists and
    /// dicts keep what the run has written. Pending events from the old
    /// scene are dropped; the caller fires `SceneStarted` after.
    pub fn load_scene(&mut self, project: &Project) {
        self.load_scene_keep(project, &std::collections::HashSet::new());
    }

    /// The same, keeping `keep` (survivor ids) alive: their programs, names,
    /// variables, lists, dicts and running strands survive the switch, while
    /// every other actor starts fresh. Clones still die with the old scene.
    pub fn load_scene_keep(&mut self, project: &Project, keep: &std::collections::HashSet<String>) {
        self.programs.retain(|id, _| keep.contains(id));
        self.names.retain(|id, _| keep.contains(id));
        self.clones.clear();
        self.deleted.retain(|id| keep.contains(id));
        // Survivors keep running; everyone else's strands go with the old
        // scene. Their `when scene starts` strands fire via SceneStarted.
        self.scripts.retain(|script| keep.contains(&script.actor));
        self.pending.clear();
        self.stopping = false;
        self.made = 0;
        self.variables.load_scene_keep(project, keep);
        self.lists.load_scene_keep(project, keep);
        self.dicts.load_scene_keep(project, keep);
        for actor in &project.actors {
            if keep.contains(&actor.id) {
                continue;
            }
            self.names.insert(actor.id.clone(), actor.name.clone());
            let inputs = actor
                .graph
                .block_defs
                .iter()
                .map(|def| {
                    (
                        def.id.clone(),
                        def.input_names()
                            .map(str::to_string)
                            .collect::<Rc<[String]>>(),
                    )
                })
                .collect();
            let program = Loaded::new(compile(&actor.graph), inputs);
            self.programs.insert(actor.id.clone(), Rc::new(program));
        }
    }

    /// Queues an event. Its scripts start at the beginning of the next tick.
    pub fn fire(&mut self, event: Event) {
        self.pending.push(event);
    }

    /// Makes a clone on a script's behalf, answering `(clone id, template
    /// id)` or `None` when nothing answers to `wanted`. A block reaches the
    /// same code through `create a clone of`; this is the door for the
    /// script ABI, which has no `Action` to run.
    pub fn clone_actor(&mut self, running: &str, wanted: &str) -> Option<(String, String)> {
        let template = self.find_actor(running, wanted)?;
        let clone = self.register_clone(&template);
        self.pending.push(Event::Cloned {
            actor: clone.clone(),
        });
        Some((clone, template))
    }

    /// Registers a brand-new actor with no blocks and answers its id.
    pub fn create_actor(&mut self, name: &str) -> String {
        let id = self.new_actor_id();
        self.programs.insert(id.clone(), Rc::new(Loaded::default()));
        self.names.insert(id.clone(), name.to_string());
        id
    }

    /// Takes an actor out of the run, answering its id. Its scripts go at the
    /// end of this tick.
    pub fn delete_actor(&mut self, running: &str, wanted: &str) -> Option<String> {
        let gone = self.find_actor(running, wanted)?;
        self.forget_actor(&gone);
        Some(gone)
    }

    /// Which actor a name or an id means, for a caller outside the VM.
    pub fn actor_for(&self, running: &str, wanted: &str) -> Option<String> {
        self.find_actor(running, wanted)
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

    /// Current variable values - what the editor's watchers show.
    pub fn variables(&self) -> VariableSnapshot {
        self.variables.snapshot()
    }

    pub fn variable_store(&self) -> Variables {
        self.variables.clone()
    }

    pub fn list_store(&self) -> Lists {
        self.lists.clone()
    }

    pub fn dict_store(&self) -> Dicts {
        self.dicts.clone()
    }

    /// Whether the world is frozen. Set by the host from the `pause game`
    /// block's effect; while it holds, only strands a UI event started tick.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Runs every live script for one frame. `now` is seconds since the run
    /// started; effects are appended to `out` in the order they happened.
    pub fn tick(&mut self, now: f64, out: &mut Vec<Effect>) {
        self.tick_at(now, now, out);
    }

    /// The same, with the two clocks told apart: `now` is the world's, which
    /// the host freezes while paused, and `wall` is the one that never stops.
    /// A UI strand runs on `wall`, so a `wait 1` on a pause menu still
    /// finishes; everything else runs on `now`.
    pub fn tick_at(&mut self, now: f64, wall: f64, out: &mut Vec<Effect>) {
        self.now = now;
        self.wall = wall;
        let events = std::mem::take(&mut self.pending);
        for event in events {
            self.start_for(event);
        }

        let mut index = 0;
        while index < self.scripts.len() {
            // A paused world advances only what the interface started.
            if self.paused && !self.scripts[index].ui {
                index += 1;
                continue;
            }
            let mut script = std::mem::replace(&mut self.scripts[index], Script::spent());
            let Some(program) = self.programs.get(&script.actor).map(Rc::clone) else {
                index += 1;
                continue;
            };
            let actor = script.actor.clone();
            let ui = script.ui;
            sense::with_script(&actor, ui, || {
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
        // A deleted actor's other strands go with it, wherever in the tick
        // they were - the one that ran the block already stopped itself.
        if !self.deleted.is_empty() {
            let gone = std::mem::take(&mut self.deleted);
            self.scripts.retain(|script| !gone.contains(&script.actor));
        }
    }

    // ─── Starting scripts ───────────────────────────────────────────────────

    fn start_for(&mut self, event: Event) {
        // Escape is the pause key: while paused it starts key strands as
        // interface strands, so a pause menu can toggle itself shut.
        let ui =
            event.is_ui() || (self.paused && matches!(&event, Event::Key(key) if key == "escape"));
        let matches: Vec<(String, String, usize)> = self
            .programs
            .iter()
            .flat_map(|(actor, program)| {
                program
                    .entries
                    .iter()
                    .filter(|entry| self.entry_matches(actor, &entry.trigger, &event))
                    .map(|entry| (actor.clone(), entry.strand_id.clone(), entry.pc))
            })
            .collect();
        for (actor, strand_id, pc) in matches {
            self.start(actor, strand_id, pc, ui);
        }
    }

    fn entry_matches(&self, actor: &str, trigger: &Trigger, event: &Event) -> bool {
        match (trigger, event) {
            (Trigger::Started, Event::Started) => true,
            (Trigger::SceneStarted, Event::SceneStarted) => true,
            (Trigger::SceneEnded, Event::SceneEnded) => true,
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
            (Trigger::Cloned, Event::Cloned { actor: fresh }) => fresh == actor,
            (
                Trigger::AnimationEnded { clip: want },
                Event::AnimationEnded { actor: ended, clip },
            ) => ended == actor && (want.is_empty() || want.eq_ignore_ascii_case(clip)),
            (Trigger::Particles(want), Event::Particles { actor: from, event }) => {
                from == actor && want == event
            }
            (
                Trigger::AnimationMarker { marker: want },
                Event::AnimationMarker {
                    actor: reached,
                    marker,
                },
            ) => reached == actor && (want.is_empty() || want.eq_ignore_ascii_case(marker)),
            (
                Trigger::EnteredRoom { room: want },
                Event::EnteredRoom {
                    actor: entered,
                    room,
                },
            ) => entered == actor && (want.is_empty() || want.eq_ignore_ascii_case(room)),
            (Trigger::ActionPressed(want), Event::Action(got)) => want == got,
            (Trigger::Touched, Event::Touched) => true,
            (
                Trigger::UiEvent {
                    id: want,
                    event: kind,
                },
                Event::UiEvent { id, event },
            ) => want == id && kind == event,
            (Trigger::UiClicked(want), Event::UiClicked { id }) => want == id,
            (Trigger::UiChanged(want), Event::UiChanged { id, .. }) => want == id,
            _ => false,
        }
    }

    /// Starts an entry point, restarting it if it's already running.
    fn start(&mut self, actor: String, strand_id: String, pc: usize, ui: bool) {
        let key = (actor.clone(), strand_id);
        let fresh = Script {
            actor,
            key: Some(key.clone()),
            pc,
            frames: Vec::new(),
            status: Status::Run,
            ui,
            temps: Vec::new(),
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
        program: &Rc<Loaded>,
        immediate: bool,
        out: &mut Vec<Effect>,
    ) -> Option<Evaluated> {
        // A reporter body runs for the actor that asked, so it keeps that
        // actor's scope as it stands, declarations included.
        if !immediate {
            self.scope = self.variables.scope(&script.actor);
        }
        let caller = self.current.replace(Rc::clone(program));
        let result = self.step(script, program, immediate, out);
        self.current = caller;
        result
    }

    fn step(
        &mut self,
        script: &mut Script,
        program: &Loaded,
        immediate: bool,
        out: &mut Vec<Effect>,
    ) -> Option<Evaluated> {
        // A UI strand keeps its own clock: the world's is frozen while a
        // menu is up, and a `wait` on that menu still has to finish.
        let clock = if script.ui { self.wall } else { self.now };
        match script.status {
            Status::Done => return None,
            Status::Sleep(until) => {
                if clock < until {
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
                    let params = current_params(&script.frames);
                    let temps = &script.temps;
                    self.perform(action, &script.actor, params, temps, out);
                    // `delete myself` ends the strand that ran it where it
                    // stands, as `stop all` ends everything.
                    if self.deleted.iter().any(|gone| gone == &script.actor) {
                        script.status = Status::Done;
                        return None;
                    }
                    // `switch scene to` ends the strand that ran it where it
                    // stands: the world it stood in is going away.
                    if matches!(action, Action::SwitchScene { .. }) {
                        script.status = Status::Done;
                        return None;
                    }
                    // `pause game` freezes the world where it stands, the
                    // strand that ran it included - a strand the interface
                    // started is what has to start it again. The pc is
                    // already past the block, so resuming carries on.
                    if !immediate && !script.ui && matches!(action, Action::SetPaused(true)) {
                        return None;
                    }
                }
                Step::JumpUnless { condition, to } => {
                    let params = current_params(&script.frames);
                    let temps = &script.temps;
                    let holds = self
                        .eval(condition, &script.actor, params, temps, out)
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
                            let params = current_params(&script.frames);
                            let temps = &script.temps;
                            let n = self
                                .eval(count, &script.actor, params, temps, out)
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
                            let params = current_params(&script.frames);
                            let temps = &script.temps;
                            let holds = self
                                .eval(condition, &script.actor, params, temps, out)
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
                            remaining: Some(left),
                        }) if *frame_begin == begin => {
                            *left -= 1;
                            if *left <= 0 {
                                let after = *end + 1;
                                script.frames.pop();
                                script.pc = after;
                            } else {
                                // Straight back into the body: a `repeat` counts
                                // its own head exactly once.
                                script.pc = begin + 1;
                            }
                        }
                        // `forever` and `while` bounce off the head instead, so
                        // a `while`'s condition is re-checked every time.
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
                    let params = current_params(&script.frames);
                    let temps = &script.temps;
                    let seconds = self
                        .eval(duration, &script.actor, params, temps, out)
                        .as_number()
                        .unwrap_or(0.0)
                        .max(0.0);
                    script.pc = pc + 1;
                    if !immediate && seconds > 0.0 {
                        script.status = Status::Sleep(clock + seconds);
                        return None;
                    }
                }
                Step::WaitUntil(condition) => {
                    let params = current_params(&script.frames);
                    let temps = &script.temps;
                    let holds = self
                        .eval(condition, &script.actor, params, temps, out)
                        .as_bool();
                    if holds || immediate {
                        script.pc = pc + 1;
                    } else {
                        // Leave the pc here and re-check next frame.
                        return None;
                    }
                }
                Step::Glide {
                    seconds,
                    target,
                    easing,
                } => {
                    let params = current_params(&script.frames);
                    let temps = &script.temps;
                    let seconds = self
                        .eval(seconds, &script.actor, params, temps, out)
                        .as_number()
                        .unwrap_or(0.0)
                        .max(0.0);
                    let position = self.eval_vec3(target, &script.actor, params, temps, out);
                    out.push(Effect::Glide {
                        actor: script.actor.clone(),
                        seconds: seconds as f32,
                        target: position,
                        easing: *easing,
                    });
                    script.pc = pc + 1;
                    if !immediate && seconds > 0.0 {
                        script.status = Status::Sleep(clock + seconds);
                        return None;
                    }
                }
                Step::TweenScale {
                    factor,
                    seconds,
                    easing,
                } => {
                    let params = current_params(&script.frames);
                    let temps = &script.temps;
                    let factor = self.eval_f32(factor, &script.actor, params, temps, out);
                    let seconds = self
                        .eval(seconds, &script.actor, params, temps, out)
                        .as_number()
                        .unwrap_or(0.0)
                        .max(0.0);
                    out.push(Effect::TweenScale {
                        actor: script.actor.clone(),
                        factor,
                        seconds: seconds as f32,
                        easing: *easing,
                    });
                    script.pc = pc + 1;
                    if !immediate && seconds > 0.0 {
                        script.status = Status::Sleep(clock + seconds);
                        return None;
                    }
                }
                Step::TweenRotation {
                    axis,
                    degrees,
                    seconds,
                    easing,
                } => {
                    let params = current_params(&script.frames);
                    let temps = &script.temps;
                    let degrees = self.eval_f32(degrees, &script.actor, params, temps, out);
                    let seconds = self
                        .eval(seconds, &script.actor, params, temps, out)
                        .as_number()
                        .unwrap_or(0.0)
                        .max(0.0);
                    out.push(Effect::TweenRotation {
                        actor: script.actor.clone(),
                        axis: *axis,
                        degrees,
                        seconds: seconds as f32,
                        easing: *easing,
                    });
                    script.pc = pc + 1;
                    if !immediate && seconds > 0.0 {
                        script.status = Status::Sleep(clock + seconds);
                        return None;
                    }
                }
                Step::TweenColor {
                    color,
                    seconds,
                    easing,
                } => {
                    let params = current_params(&script.frames);
                    let temps = &script.temps;
                    let color = self
                        .eval(color, &script.actor, params, temps, out)
                        .as_text();
                    let seconds = self
                        .eval(seconds, &script.actor, params, temps, out)
                        .as_number()
                        .unwrap_or(0.0)
                        .max(0.0);
                    out.push(Effect::TweenColor {
                        actor: script.actor.clone(),
                        color,
                        seconds: seconds as f32,
                        easing: *easing,
                    });
                    script.pc = pc + 1;
                    if !immediate && seconds > 0.0 {
                        script.status = Status::Sleep(clock + seconds);
                        return None;
                    }
                }
                Step::Call { block_id, args } => {
                    let Some((start, names)) = program.block(block_id) else {
                        script.pc = pc + 1;
                        continue;
                    };
                    let params = current_params(&script.frames);
                    let temps = &script.temps;
                    let bound = self.bind_params(&script.actor, names, args, params, temps, out);
                    let saved = std::mem::take(&mut script.temps);
                    script.frames.push(Frame::Call {
                        return_pc: pc + 1,
                        params: bound,
                        temp: None,
                        saved_temps: saved,
                    });
                    script.pc = start;
                }
                Step::Invoke {
                    block_id,
                    args,
                    temp,
                } => {
                    let temp = *temp;
                    let Some((start, names)) = program.block(block_id) else {
                        store_temp(&mut script.temps, temp, Evaluated::Number(0.0));
                        script.pc = pc + 1;
                        continue;
                    };
                    let params = current_params(&script.frames);
                    let temps = &script.temps;
                    let bound = self.bind_params(&script.actor, names, args, params, temps, out);
                    let saved = std::mem::take(&mut script.temps);
                    script.frames.push(Frame::Call {
                        return_pc: pc + 1,
                        params: bound,
                        temp: Some(temp),
                        saved_temps: saved,
                    });
                    script.pc = start;
                }
                Step::Return(value) => {
                    let params = current_params(&script.frames);
                    let temps = &script.temps;
                    let result = self.eval(value, &script.actor, params, temps, out);
                    match nearest_call(script) {
                        Some((index, return_pc, temp)) if return_pc != usize::MAX => {
                            let saved = match &mut script.frames[index] {
                                Frame::Call { saved_temps, .. } => std::mem::take(saved_temps),
                                _ => Vec::new(),
                            };
                            script.frames.truncate(index);
                            script.temps = saved;
                            if let Some(temp) = temp {
                                store_temp(&mut script.temps, temp, result);
                            }
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
                    Some((index, return_pc, temp)) if return_pc != usize::MAX => {
                        let saved = match &mut script.frames[index] {
                            Frame::Call { saved_temps, .. } => std::mem::take(saved_temps),
                            _ => Vec::new(),
                        };
                        script.frames.truncate(index);
                        script.temps = saved;
                        if let Some(temp) = temp {
                            store_temp(&mut script.temps, temp, Evaluated::Number(0.0));
                        }
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
        params: Option<&Params>,
        temps: &[Evaluated],
        out: &mut Vec<Effect>,
    ) {
        match action {
            Action::Move(steps) => {
                let steps = self.eval_f32(steps, actor, params, temps, out);
                out.push(Effect::Move {
                    actor: actor.to_string(),
                    steps,
                });
            }
            Action::GoTo(target) => {
                let position = self.eval_vec3(target, actor, params, temps, out);
                out.push(Effect::GoTo {
                    actor: actor.to_string(),
                    position,
                });
            }
            Action::NavigateTo { target, speed } => {
                let position = self.eval_vec3(target, actor, params, temps, out);
                let speed = self.eval_f32(speed, actor, params, temps, out);
                out.push(Effect::NavigateTo {
                    actor: actor.to_string(),
                    target: position,
                    speed,
                });
            }
            Action::BurstParticles(count) => {
                let count = self.eval_f32(count, actor, params, temps, out);
                out.push(Effect::BurstParticles {
                    actor: actor.to_string(),
                    count: (count as i64).clamp(0, 512) as u32,
                });
            }
            Action::SetEmitterDial { dial, value } => {
                let value = self.eval_f32(value, actor, params, temps, out);
                out.push(Effect::SetEmitterDial {
                    actor: actor.to_string(),
                    dial: *dial,
                    value,
                });
            }
            Action::SetTrailEnabled(enabled) => out.push(Effect::SetTrailEnabled {
                actor: actor.to_string(),
                enabled: *enabled,
            }),
            Action::SetEmitterPlaying(playing) => out.push(Effect::SetEmitterPlaying {
                actor: actor.to_string(),
                playing: *playing,
            }),
            Action::ChangePosition { axis, by } => {
                let by = self.eval_f32(by, actor, params, temps, out);
                out.push(Effect::ChangePosition {
                    actor: actor.to_string(),
                    axis: *axis,
                    by,
                });
            }
            Action::Turn { axis, degrees } => {
                let degrees = self.eval_f32(degrees, actor, params, temps, out);
                out.push(Effect::Turn {
                    actor: actor.to_string(),
                    axis: *axis,
                    degrees,
                });
            }
            Action::SetRotation { axis, degrees } => {
                let degrees = self.eval_f32(degrees, actor, params, temps, out);
                out.push(Effect::SetRotation {
                    actor: actor.to_string(),
                    axis: *axis,
                    degrees,
                });
            }
            Action::PointTowards(target) => out.push(Effect::PointTowards {
                actor: actor.to_string(),
                target: target.clone(),
            }),
            Action::SetScale(factor) => {
                let factor = self.eval_f32(factor, actor, params, temps, out);
                out.push(Effect::SetScale {
                    actor: actor.to_string(),
                    factor,
                });
            }
            Action::TweenScale {
                factor,
                seconds,
                easing,
            } => {
                let factor = self.eval_f32(factor, actor, params, temps, out);
                let seconds = self
                    .eval(seconds, actor, params, temps, out)
                    .as_number()
                    .unwrap_or(0.0)
                    .max(0.0);
                out.push(Effect::TweenScale {
                    actor: actor.to_string(),
                    factor,
                    seconds: seconds as f32,
                    easing: *easing,
                });
            }
            Action::TweenRotation {
                axis,
                degrees,
                seconds,
                easing,
            } => {
                let degrees = self.eval_f32(degrees, actor, params, temps, out);
                let seconds = self
                    .eval(seconds, actor, params, temps, out)
                    .as_number()
                    .unwrap_or(0.0)
                    .max(0.0);
                out.push(Effect::TweenRotation {
                    actor: actor.to_string(),
                    axis: *axis,
                    degrees,
                    seconds: seconds as f32,
                    easing: *easing,
                });
            }
            Action::TweenColor {
                color,
                seconds,
                easing,
            } => {
                let color = self.eval(color, actor, params, temps, out).as_text();
                let seconds = self
                    .eval(seconds, actor, params, temps, out)
                    .as_number()
                    .unwrap_or(0.0)
                    .max(0.0);
                out.push(Effect::TweenColor {
                    actor: actor.to_string(),
                    color,
                    seconds: seconds as f32,
                    easing: *easing,
                });
            }
            Action::StopTweens => out.push(Effect::StopTweens {
                actor: actor.to_string(),
            }),
            Action::PlayAnimation { clip, speed } => {
                let clip = self.eval(clip, actor, params, temps, out).as_text();
                let speed = self.eval_f32(speed, actor, params, temps, out);
                out.push(Effect::PlayAnimation {
                    actor: actor.to_string(),
                    clip: clip.trim().to_string(),
                    speed,
                });
            }
            Action::StopAnimation => out.push(Effect::StopAnimation {
                actor: actor.to_string(),
            }),
            Action::SetAnimationSpeed(speed) => {
                let speed = self.eval_f32(speed, actor, params, temps, out);
                out.push(Effect::SetAnimationSpeed {
                    actor: actor.to_string(),
                    speed,
                });
            }
            Action::FireAnimationTrigger(name) => {
                let name = self.eval(name, actor, params, temps, out).as_text();
                out.push(Effect::FireAnimationTrigger {
                    actor: actor.to_string(),
                    name: name.trim().to_string(),
                });
            }
            Action::SetRigSlot { slot, attachment } => {
                let slot = self.eval(slot, actor, params, temps, out).as_text();
                let attachment = self.eval(attachment, actor, params, temps, out).as_text();
                out.push(Effect::SetRigSlot {
                    actor: actor.to_string(),
                    slot: slot.trim().to_string(),
                    attachment: attachment.trim().to_string(),
                });
            }
            Action::SetSlotTint { slot, color } => {
                let slot = self.eval(slot, actor, params, temps, out).as_text();
                let color = self.eval(color, actor, params, temps, out).as_text();
                out.push(Effect::SetSlotTint {
                    actor: actor.to_string(),
                    slot: slot.trim().to_string(),
                    color,
                });
            }
            Action::SetIkTarget { constraint, x, y } => {
                let constraint = self.eval(constraint, actor, params, temps, out).as_text();
                let x = self.eval_f32(x, actor, params, temps, out);
                let y = self.eval_f32(y, actor, params, temps, out);
                out.push(Effect::SetIkTarget {
                    actor: actor.to_string(),
                    constraint: constraint.trim().to_string(),
                    x,
                    y,
                });
            }
            Action::SetSpriteDial { dial, value } => {
                let value = self.eval_f32(value, actor, params, temps, out);
                out.push(Effect::SetSpriteDial {
                    actor: actor.to_string(),
                    dial: *dial,
                    value,
                });
            }
            Action::SetExposure(ev) => {
                let ev = self.eval_f32(ev, actor, params, temps, out);
                out.push(Effect::SetExposure { ev });
            }
            Action::SetLightIntensity(intensity) => {
                let intensity = self.eval_f32(intensity, actor, params, temps, out);
                out.push(Effect::SetLightIntensity {
                    actor: actor.to_string(),
                    intensity,
                });
            }
            Action::SetEmissiveStrength(strength) => {
                let strength = self.eval_f32(strength, actor, params, temps, out);
                out.push(Effect::SetEmissiveStrength {
                    actor: actor.to_string(),
                    strength,
                });
            }
            Action::SetHdrOutput(enabled) => out.push(Effect::SetHdrOutput { enabled: *enabled }),
            Action::SetPeakBrightness(nits) => {
                let nits = self.eval_f32(nits, actor, params, temps, out);
                out.push(Effect::SetPeakBrightness { nits });
            }
            Action::EnableVolume { volume, enabled } => {
                let volume = self.eval(volume, actor, params, temps, out).as_text();
                out.push(Effect::SetVolumeEnabled {
                    actor: actor.to_string(),
                    volume: volume.trim().to_string(),
                    enabled: *enabled,
                });
            }
            Action::SetVolumeWeight { volume, weight } => {
                let volume = self.eval(volume, actor, params, temps, out).as_text();
                let weight = self.eval_f32(weight, actor, params, temps, out);
                out.push(Effect::SetVolumeWeight {
                    actor: actor.to_string(),
                    volume: volume.trim().to_string(),
                    weight,
                });
            }
            Action::CaptureProbes => out.push(Effect::CaptureProbes),
            Action::SetShadowDistance(distance) => {
                let distance = self.eval_f32(distance, actor, params, temps, out);
                out.push(Effect::SetShadowDistance { distance });
            }
            Action::SetLightShadows(enabled) => out.push(Effect::SetLightShadows {
                actor: actor.to_string(),
                enabled: *enabled,
            }),
            Action::SetRayTracing(enabled) => out.push(Effect::SetRayTracing { enabled: *enabled }),
            Action::SetGiBounces(bounces) => {
                let bounces = self.eval_f32(bounces, actor, params, temps, out);
                out.push(Effect::SetGiBounces { bounces });
            }
            Action::SetGiSamples(samples) => {
                let samples = self.eval_f32(samples, actor, params, temps, out);
                out.push(Effect::SetGiSamples { samples });
            }
            Action::SetBody(body) => out.push(Effect::SetBody {
                actor: actor.to_string(),
                body: *body,
            }),
            Action::ApplyImpulse(vector) => {
                let impulse = self.eval_vec3(vector, actor, params, temps, out);
                out.push(Effect::ApplyImpulse {
                    actor: actor.to_string(),
                    impulse,
                });
            }
            Action::SetVelocity(vector) => {
                let velocity = self.eval_vec3(vector, actor, params, temps, out);
                out.push(Effect::SetVelocity {
                    actor: actor.to_string(),
                    velocity,
                });
            }
            Action::SetFogDensity(density) => {
                let density = self.eval_f32(density, actor, params, temps, out);
                out.push(Effect::SetFogDensity { density });
            }
            Action::SetAurora(kp) => {
                let kp = self.eval_f32(kp, actor, params, temps, out);
                out.push(Effect::SetAurora { kp });
            }
            Action::Fracture(target) => {
                let wanted = self.eval(target, actor, params, temps, out).as_text();
                match self.find_actor(actor, &wanted) {
                    Some(actor) => out.push(Effect::Fracture { actor }),
                    None => out.push(Effect::Error {
                        actor: actor.to_string(),
                        message: format!("there's no actor named \"{wanted}\" to fracture"),
                    }),
                }
            }
            Action::Splash(values) | Action::PuffSmoke(values) => {
                let [x, y, z, radius, strength] =
                    std::array::from_fn(|i| self.eval_f32(&values[i], actor, params, temps, out));
                out.push(if matches!(action, Action::Splash(_)) {
                    Effect::Splash {
                        at: [x, y, z],
                        radius,
                        strength,
                    }
                } else {
                    Effect::PuffSmoke {
                        at: [x, y, z],
                        radius,
                        strength,
                    }
                });
            }
            Action::SpawnDecal { preset, values } => {
                let [x, y, z, nx, ny, nz, size, lifetime, fade] =
                    std::array::from_fn(|i| self.eval_f32(&values[i], actor, params, temps, out));
                out.push(Effect::SpawnDecal(crate::decals::Spawn {
                    preset: *preset,
                    at: [x, y, z],
                    normal: [nx, ny, nz],
                    size,
                    lifetime,
                    fade,
                }));
            }
            Action::FadeDecals(values) => {
                let [x, y, z, radius, seconds] =
                    std::array::from_fn(|i| self.eval_f32(&values[i], actor, params, temps, out));
                out.push(Effect::FadeDecals {
                    at: [x, y, z],
                    radius,
                    seconds,
                });
            }
            Action::StrikeLightning(vector) => {
                let at = self.eval_vec3(vector, actor, params, temps, out);
                out.push(Effect::StrikeLightning { at });
            }
            Action::SetLightningRate(rate) => {
                let rate = self.eval_f32(rate, actor, params, temps, out);
                out.push(Effect::SetLightningRate { rate });
            }
            Action::SetWind { property, value } => {
                let value = self.eval_f32(value, actor, params, temps, out);
                out.push(Effect::SetWind {
                    property: *property,
                    value,
                });
            }
            Action::SetCloudLayer {
                layer,
                property,
                value,
            } => {
                let layer = self.eval_f32(layer, actor, params, temps, out);
                let value = self.eval_f32(value, actor, params, temps, out);
                out.push(Effect::SetCloudLayer {
                    layer,
                    property: *property,
                    value,
                });
            }
            Action::SetClouds { property, value } => {
                let value = self.eval_f32(value, actor, params, temps, out);
                out.push(Effect::SetClouds {
                    property: *property,
                    value,
                });
            }
            Action::SetWater { property, value } => {
                let value = self.eval_f32(value, actor, params, temps, out);
                out.push(Effect::SetWater {
                    actor: actor.to_string(),
                    property: *property,
                    value,
                });
            }
            Action::PaintTile { map, tile, x, y, z } => {
                let map = self.eval(map, actor, params, temps, out).as_text();
                let tile = self.eval_f32(tile, actor, params, temps, out);
                let x = self.eval_f32(x, actor, params, temps, out);
                let y = self.eval_f32(y, actor, params, temps, out);
                let z = self.eval_f32(z, actor, params, temps, out);
                out.push(Effect::PaintTile {
                    actor: actor.to_string(),
                    map: map.trim().to_string(),
                    tile: (tile.floor() as i32).max(-1),
                    x,
                    y,
                    z,
                });
            }
            Action::SetParallax { layer, axis, value } => {
                let layer = self.eval(layer, actor, params, temps, out).as_text();
                let value = self.eval_f32(value, actor, params, temps, out);
                out.push(Effect::SetParallax {
                    actor: actor.to_string(),
                    layer: layer.trim().to_string(),
                    axis: *axis,
                    value,
                });
            }
            Action::SetCloudDrift(vector) => {
                let drift = self.eval_vec3(vector, actor, params, temps, out);
                out.push(Effect::SetCloudDrift { drift });
            }
            Action::SetGravity(vector) => {
                let gravity = self.eval_vec3(vector, actor, params, temps, out);
                out.push(Effect::SetGravity { gravity });
            }
            Action::SetDensity(density) => {
                let density = self.eval_f32(density, actor, params, temps, out);
                out.push(Effect::SetDensity {
                    actor: actor.to_string(),
                    density,
                });
            }
            Action::SetMass(mass) => {
                let mass = self.eval_f32(mass, actor, params, temps, out);
                out.push(Effect::SetMass {
                    actor: actor.to_string(),
                    mass,
                });
            }
            Action::SetTrigger(trigger) => out.push(Effect::SetTrigger {
                actor: actor.to_string(),
                trigger: *trigger,
            }),
            Action::SetCollisionLayer(layer) => {
                let layer = self.eval_f32(layer, actor, params, temps, out).round() as u8;
                out.push(Effect::SetCollisionLayer {
                    actor: actor.to_string(),
                    layer: layer.clamp(1, 8),
                });
            }
            Action::SetCollisionMask(mask) => {
                let mask = self.eval_f32(mask, actor, params, temps, out).round() as i32;
                out.push(Effect::SetCollisionMask {
                    actor: actor.to_string(),
                    mask: mask.clamp(0, 255) as u8,
                });
            }
            Action::Say(text) => {
                let text = self.eval(text, actor, params, temps, out).as_text();
                out.push(Effect::Say {
                    actor: actor.to_string(),
                    text,
                });
            }
            Action::SetVisible(visible) => out.push(Effect::SetVisible {
                actor: actor.to_string(),
                visible: *visible,
            }),
            Action::SetColor(color) => {
                let color = self.eval(color, actor, params, temps, out).as_text();
                out.push(Effect::SetColor {
                    actor: actor.to_string(),
                    color,
                });
            }
            // Sound slots read left to right as the row is written - a
            // compiled program reads them in the same order, and a bad one
            // complains in the same place.
            Action::PlaySound {
                sound,
                volume,
                pitch,
                loop_,
                bus,
            } => {
                let sound = normalize_sound(&self.eval(sound, actor, params, temps, out).as_text());
                let volume = user_to_gain(
                    self.eval(volume, actor, params, temps, out)
                        .as_number()
                        .unwrap_or(0.0),
                );
                let pitch = clamp_pitch(self.eval_f32(pitch, actor, params, temps, out));
                if sound.is_empty() {
                    out.push(Effect::Error {
                        actor: actor.to_string(),
                        message: "which sound should I play?".to_string(),
                    });
                } else {
                    out.push(Effect::PlaySound {
                        actor: actor.to_string(),
                        sound,
                        volume,
                        pitch,
                        loop_: *loop_,
                        bus: *bus,
                        at: None,
                    });
                }
            }
            Action::PlaySoundAt {
                sound,
                volume,
                pitch,
                loop_,
                bus,
                target,
            } => {
                let sound = normalize_sound(&self.eval(sound, actor, params, temps, out).as_text());
                let volume = user_to_gain(
                    self.eval(volume, actor, params, temps, out)
                        .as_number()
                        .unwrap_or(0.0),
                );
                let pitch = clamp_pitch(self.eval_f32(pitch, actor, params, temps, out));
                let wanted = self.eval(target, actor, params, temps, out).as_text();
                if sound.is_empty() {
                    out.push(Effect::Error {
                        actor: actor.to_string(),
                        message: "which sound should I play?".to_string(),
                    });
                } else {
                    match self.find_actor(actor, &wanted) {
                        Some(at) => out.push(Effect::PlaySound {
                            actor: actor.to_string(),
                            sound,
                            volume,
                            pitch,
                            loop_: *loop_,
                            bus: *bus,
                            at: Some(at),
                        }),
                        None => out.push(Effect::Error {
                            actor: actor.to_string(),
                            message: format!("there's no actor named \"{wanted}\" to play at"),
                        }),
                    }
                }
            }
            Action::StopSound { sound } => {
                let sound = normalize_sound(&self.eval(sound, actor, params, temps, out).as_text());
                out.push(Effect::StopSound {
                    actor: actor.to_string(),
                    sound,
                });
            }
            Action::SetSoundVolume { sound, volume } => {
                let sound = normalize_sound(&self.eval(sound, actor, params, temps, out).as_text());
                let volume = user_to_gain(
                    self.eval(volume, actor, params, temps, out)
                        .as_number()
                        .unwrap_or(0.0),
                );
                if sound.is_empty() {
                    out.push(Effect::Error {
                        actor: actor.to_string(),
                        message: "which sound's volume should I set?".to_string(),
                    });
                } else {
                    out.push(Effect::SetSoundVolume {
                        actor: actor.to_string(),
                        sound,
                        volume,
                    });
                }
            }
            Action::SetSoundPitch { sound, pitch } => {
                let sound = normalize_sound(&self.eval(sound, actor, params, temps, out).as_text());
                let pitch = clamp_pitch(self.eval_f32(pitch, actor, params, temps, out));
                if sound.is_empty() {
                    out.push(Effect::Error {
                        actor: actor.to_string(),
                        message: "which sound's pitch should I set?".to_string(),
                    });
                } else {
                    out.push(Effect::SetSoundPitch {
                        actor: actor.to_string(),
                        sound,
                        pitch,
                    });
                }
            }
            Action::SetBusVolume { bus, volume } => {
                let volume = user_to_gain(
                    self.eval(volume, actor, params, temps, out)
                        .as_number()
                        .unwrap_or(0.0),
                );
                out.push(Effect::SetBusVolume { bus: *bus, volume });
            }
            Action::SetComponentField {
                component,
                field,
                value,
            } => {
                let value = self.eval(value, actor, params, temps, out);
                out.push(Effect::SetComponentField {
                    actor: actor.to_string(),
                    component: component.trim().to_string(),
                    field: field.trim().to_string(),
                    value,
                });
            }
            Action::SetCameraView(view) => out.push(Effect::SetCameraView {
                actor: actor.to_string(),
                view: *view,
            }),
            Action::SetCameraPitch(degrees) => {
                let degrees = self.eval_f32(degrees, actor, params, temps, out);
                out.push(Effect::SetCameraPitch {
                    actor: actor.to_string(),
                    degrees,
                });
            }
            Action::SetCameraFov(fov) => {
                let fov = self.eval_f32(fov, actor, params, temps, out);
                out.push(Effect::SetCameraFov {
                    actor: actor.to_string(),
                    fov,
                });
            }
            Action::AttachComponent(component) => out.push(Effect::AttachComponent {
                actor: actor.to_string(),
                component: component.clone(),
            }),
            Action::DetachComponent(component) => out.push(Effect::DetachComponent {
                actor: actor.to_string(),
                component: component.clone(),
            }),
            // The name goes to the host as it was written, rather than being
            // looked up here: the hierarchy is the host's, and a compiled
            // program has no name table to look one up in. An empty slot
            // takes the actor off whatever it hangs from, where an empty slot
            // elsewhere means "myself".
            Action::SetParent(target) => {
                let parent = self.eval(target, actor, params, temps, out).as_text();
                out.push(Effect::SetParent {
                    actor: actor.to_string(),
                    parent: parent.trim().to_string(),
                });
            }
            Action::CreateClone(of) => match self.clone_actor(actor, of) {
                Some((clone, template)) => out.push(Effect::CreateClone {
                    actor: actor.to_string(),
                    clone,
                    of: template,
                }),
                None => out.push(Effect::Error {
                    actor: actor.to_string(),
                    message: format!("there's no actor named \"{of}\" to clone"),
                }),
            },
            Action::CreateActor { name, position } => {
                let name = self.eval(name, actor, params, temps, out).as_text();
                let position = self.eval_vec3(position, actor, params, temps, out);
                // No blocks of its own, but a name other actors can find it
                // by and a program slot so deleting it is the same code path.
                let id = self.create_actor(&name);
                out.push(Effect::CreateActor {
                    actor: actor.to_string(),
                    id,
                    name,
                    position,
                });
            }
            Action::DeleteActor(target) => {
                let wanted = self.eval(target, actor, params, temps, out).as_text();
                match self.delete_actor(actor, &wanted) {
                    Some(gone) => out.push(Effect::DeleteActor { actor: gone }),
                    None => out.push(Effect::Error {
                        actor: actor.to_string(),
                        message: format!("there's no actor named \"{wanted}\" to delete"),
                    }),
                }
            }
            Action::Broadcast(name) => self.pending.push(Event::Message(name.trim().to_string())),
            Action::SwitchScene { scene, transition } => {
                let scene = self.eval(scene, actor, params, temps, out).as_text();
                let transition = self.eval(transition, actor, params, temps, out).as_text();
                out.push(Effect::SwitchScene {
                    actor: actor.to_string(),
                    scene: scene.trim().to_string(),
                    transition: normalize_transition(&transition),
                });
            }
            // The interface. Every slot is read here, left to right, exactly
            // as the row is written - a compiled program reads them in the
            // same order, and a bad one complains in the same place.
            Action::ShowElement(spec) => {
                let super::program::ShowElement {
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
                let id = self.eval(id, actor, params, temps, out).as_text();
                let content = self.eval(content, actor, params, temps, out).as_text();
                let range = match range {
                    Some([low, high]) => [
                        self.eval_f32(low, actor, params, temps, out),
                        self.eval_f32(high, actor, params, temps, out),
                    ],
                    None => [0.0, 1.0],
                };
                // Only a slider's row has a value slot; the rest start at
                // whatever their kind means by blank.
                let started_at = match value {
                    Some(value) => {
                        let value = self.eval(value, actor, params, temps, out);
                        UiElement::initial_value(*kind, &value)
                    }
                    None => UiElement::blank(*kind, *flag),
                };
                let offset = [
                    self.eval_f32(&offset[0], actor, params, temps, out),
                    self.eval_f32(&offset[1], actor, params, temps, out),
                ];
                let size = [
                    self.eval_f32(&size[0], actor, params, temps, out),
                    self.eval_f32(&size[1], actor, params, temps, out),
                ];
                let parent = self.eval(parent, actor, params, temps, out).as_text();
                out.push(Effect::ShowElement {
                    element: UiElement {
                        id: id.trim().to_string(),
                        kind: *kind,
                        content,
                        anchor: *anchor,
                        offset,
                        size,
                        parent: parent.trim().to_string(),
                        modal: *kind == UiKind::Panel && *flag,
                        range,
                        value: started_at,
                    },
                });
            }
            Action::SetUiProp { prop, id, value } => {
                let id = self.eval(id, actor, params, temps, out).as_text();
                let value = self.eval(value, actor, params, temps, out);
                out.push(Effect::SetUiProp {
                    id: id.trim().to_string(),
                    prop: *prop,
                    value,
                });
            }
            Action::HideElement { id, all } => {
                let id = self.eval(id, actor, params, temps, out).as_text();
                out.push(Effect::HideElement {
                    id: id.trim().to_string(),
                    all: *all,
                });
            }
            Action::DeleteElement(id) => {
                let id = self.eval(id, actor, params, temps, out).as_text();
                out.push(Effect::DeleteElement {
                    id: id.trim().to_string(),
                });
            }
            Action::SetFocus(id) => {
                let id = self.eval(id, actor, params, temps, out).as_text();
                out.push(Effect::SetFocus {
                    id: id.trim().to_string(),
                });
            }
            Action::SetUiTheme(theme) => out.push(Effect::SetUiTheme { theme: *theme }),
            // The VM freezes itself the moment the block runs, so the rest
            // of this tick already sees a paused world - the host catches up
            // when it applies the effect.
            Action::SetPaused(paused) => {
                self.paused = *paused;
                out.push(Effect::SetPaused { paused: *paused });
            }
            Action::SaveVariable { name, clear } => out.push(Effect::SaveVariable {
                actor: actor.to_string(),
                name: name.clone(),
                clear: *clear,
            }),
            Action::SetMouseLocked(locked) => out.push(Effect::SetMouseLocked { locked: *locked }),
            Action::RumbleGamepad { strength, duration } => {
                let strength = self
                    .eval_f32(strength, actor, params, temps, out)
                    .clamp(0.0, 100.0);
                let duration = self.eval_f32(duration, actor, params, temps, out).max(0.0);
                out.push(Effect::RumbleGamepad { strength, duration });
            }
            Action::BindAction { action, binding } => {
                let name = self.eval(action, actor, params, temps, out).as_text();
                let binding = self.eval(binding, actor, params, temps, out).as_text();
                if crate::input::parse_binding(&binding).is_none() {
                    out.push(Effect::Error {
                        actor: actor.to_string(),
                        message: format!("\"{binding}\" isn't a binding"),
                    });
                } else {
                    out.push(Effect::BindAction {
                        actor: actor.to_string(),
                        action: name.trim().to_string(),
                        binding: binding.trim().to_string(),
                    });
                }
            }
            Action::ClearActionBindings { action } => {
                let name = self.eval(action, actor, params, temps, out).as_text();
                out.push(Effect::ClearActionBindings {
                    actor: actor.to_string(),
                    action: name.trim().to_string(),
                });
            }
            Action::SetVariable { name, value } => {
                let value = self.eval(value, actor, params, temps, out);
                self.write_var(actor, name, value);
            }
            Action::ChangeVariable { name, value } => {
                let delta = self
                    .eval(value, actor, params, temps, out)
                    .as_number()
                    .unwrap_or(0.0);
                // A non-numeric variable counts as zero, as in Scratch.
                let current = self.read_var(name).as_number().unwrap_or(0.0);
                self.write_var(actor, name, Evaluated::Number(current + delta));
            }
            Action::AddToList { value, name } => {
                let value = self.eval(value, actor, params, temps, out);
                match ListItem::from_evaluated(value) {
                    Some(item) => self
                        .lists
                        .with_list_mut(actor, name, |list| list.push(item)),
                    None => out.push(Effect::Error {
                        actor: actor.to_string(),
                        message: "list items must be number or text".to_string(),
                    }),
                }
            }
            Action::DeleteOfList { index, name } => {
                let index = self
                    .eval(index, actor, params, temps, out)
                    .as_number()
                    .unwrap_or(0.0);
                self.lists.with_list_mut(actor, name, |list| {
                    if let Some(at) = list_index(index, list.len(), false) {
                        list.remove(at);
                    }
                });
            }
            Action::DeleteAllOfList { name } => {
                self.lists.with_list_mut(actor, name, |list| list.clear());
            }
            Action::ShiftList { name, amount } => {
                let amount = self
                    .eval(amount, actor, params, temps, out)
                    .as_number()
                    .unwrap_or(0.0);
                self.lists.with_list_mut(actor, name, |list| {
                    if !list.is_empty() {
                        let len = list.len();
                        let distance = amount.round() as isize;
                        if distance >= 0 {
                            list.rotate_right(distance as usize % len);
                        } else {
                            list.rotate_left(distance.unsigned_abs() % len);
                        }
                    }
                });
            }
            Action::InsertIntoList { value, index, name } => {
                let value = self.eval(value, actor, params, temps, out);
                let index = self
                    .eval(index, actor, params, temps, out)
                    .as_number()
                    .unwrap_or(0.0);
                match ListItem::from_evaluated(value) {
                    Some(item) => self.lists.with_list_mut(actor, name, |list| {
                        if let Some(at) = list_index(index, list.len(), true) {
                            list.insert(at, item);
                        }
                    }),
                    None => out.push(Effect::Error {
                        actor: actor.to_string(),
                        message: "list items must be number or text".to_string(),
                    }),
                }
            }
            Action::ReplaceItemOfList { index, name, value } => {
                let index = self
                    .eval(index, actor, params, temps, out)
                    .as_number()
                    .unwrap_or(0.0);
                let value = self.eval(value, actor, params, temps, out);
                match ListItem::from_evaluated(value) {
                    Some(item) => self.lists.with_list_mut(actor, name, |list| {
                        if let Some(at) = list_index(index, list.len(), false) {
                            list[at] = item;
                        }
                    }),
                    None => out.push(Effect::Error {
                        actor: actor.to_string(),
                        message: "list items must be number or text".to_string(),
                    }),
                }
            }
            Action::ReverseList { name } => {
                self.lists.with_list_mut(actor, name, |list| list.reverse());
            }
            Action::SetDictValue { key, name, value } => {
                let key = self.eval(key, actor, params, temps, out).as_text();
                let value = self.eval(value, actor, params, temps, out);
                match DictItem::from_evaluated(value) {
                    Some(item) => self
                        .dicts
                        .with_dict_mut(actor, name, |dict| dict_set(dict, key, item)),
                    None => out.push(Effect::Error {
                        actor: actor.to_string(),
                        message: "dict values must be number or text".to_string(),
                    }),
                }
            }
            Action::DeleteDictKey { key, name } => {
                let key = self.eval(key, actor, params, temps, out).as_text();
                self.dicts.with_dict_mut(actor, name, |dict| {
                    dict_remove(dict, &key);
                });
            }
            Action::DeleteAllOfDict { name } => {
                self.dicts.with_dict_mut(actor, name, |dict| dict.clear());
            }
            Action::LoadJsonIntoDict { json, name } => {
                let json = self.eval(json, actor, params, temps, out).as_text();
                match parse_json_object(&json) {
                    Ok(entries) => self
                        .dicts
                        .with_dict_mut(actor, name, |dict| *dict = entries),
                    Err(message) => out.push(Effect::Error {
                        actor: actor.to_string(),
                        message,
                    }),
                }
            }
            Action::LoadJsonIntoList { json, name } => {
                let json = self.eval(json, actor, params, temps, out).as_text();
                match parse_json_array(&json) {
                    Ok(items) => self.lists.with_list_mut(actor, name, |list| *list = items),
                    Err(message) => out.push(Effect::Error {
                        actor: actor.to_string(),
                        message,
                    }),
                }
            }
        }
    }

    // ─── Actors that come and go ────────────────────────────────────────────

    /// Which actor a block means by `wanted`: itself when the slot is empty
    /// or says so, then an id, then a name - the same order
    /// [`sense::Sensors::find`] uses, so a block and a reporter agree.
    ///
    /// Clones share their template's name, so a name picks whichever one the
    /// map hands over first. Anything that has to mean one clone in
    /// particular works from the id `the actor I made` reports.
    fn find_actor(&self, running: &str, wanted: &str) -> Option<String> {
        let wanted = wanted.trim();
        if wanted.is_empty()
            || wanted.eq_ignore_ascii_case("myself")
            || wanted.eq_ignore_ascii_case("me")
        {
            return Some(running.to_string());
        }
        if self.programs.contains_key(wanted) {
            return Some(wanted.to_string());
        }
        self.names
            .iter()
            .find(|(_, name)| name.eq_ignore_ascii_case(wanted))
            .map(|(id, _)| id.clone())
    }

    /// Gives a fresh clone of `template` everything the scheduler needs: the
    /// same compiled program, the same name, the same custom-block inputs,
    /// and a copy of the template's variables and lists as they stand.
    fn register_clone(&mut self, template: &str) -> String {
        let id = self.new_actor_id();
        if let Some(program) = self.programs.get(template).map(Rc::clone) {
            self.programs.insert(id.clone(), program);
        }
        if let Some(name) = self.names.get(template).cloned() {
            self.names.insert(id.clone(), name);
        }
        self.variables.copy_actor(template, &id);
        self.lists.copy_actor(template, &id);
        self.dicts.copy_actor(template, &id);
        // A clone of a clone is a clone of the same authored actor.
        let root = self
            .clones
            .get(template)
            .cloned()
            .unwrap_or_else(|| template.to_string());
        self.clones.insert(id.clone(), root);
        id
    }

    /// Takes an actor out of the run. Its scripts go at the end of the tick;
    /// everything else about it goes now.
    fn forget_actor(&mut self, actor: &str) {
        self.programs.remove(actor);
        self.names.remove(actor);
        self.clones.remove(actor);
        self.variables.forget_actor(actor);
        self.lists.forget_actor(actor);
        self.dicts.forget_actor(actor);
        self.pending
            .retain(|event| !matches!(event, Event::Cloned { actor: fresh } if fresh == actor));
        if !self.deleted.iter().any(|gone| gone == actor) {
            self.deleted.push(actor.to_string());
        }
    }

    /// A runtime actor's id. Clones and created actors get one the moment
    /// they are made, which is what everything else keys them by.
    ///
    /// Counted rather than random, so one run of a project makes the same
    /// ids however it is scheduled - which is what lets `tests/codegen.rs`
    /// hold a compiled program's clones against the VM's line for line. The
    /// shape is one the editor never writes, so nothing authored collides.
    fn new_actor_id(&mut self) -> String {
        self.made += 1;
        format!("~{}", self.made)
    }

    // ─── Variables ──────────────────────────────────────────────────────────

    fn read_var(&self, name: &String) -> Evaluated {
        let var = self.var_id(name);
        self.variables.read_slot(self.scope, var)
    }

    /// Writes to the actor's own variable when it has one by that name, the
    /// project global when it doesn't, and otherwise declares it on the actor.
    fn write_var(&mut self, actor: &str, name: &String, value: Evaluated) {
        let var = self.var_id(name);
        self.scope = self.variables.write_slot(actor, self.scope, var, value);
    }

    /// A variable name from the running program, interned.
    fn var_id(&self, name: &String) -> VarId {
        match &self.current {
            Some(program) => program.var(name, &self.variables),
            None => self.variables.intern(name),
        }
    }

    // ─── Evaluation ─────────────────────────────────────────────────────────

    /// Evaluates a slot. An error is reported once and stands in as `0`, so
    /// one bad slot never kills a run.
    fn eval(
        &mut self,
        value: &Value,
        actor: &str,
        params: Option<&Params>,
        temps: &[Evaluated],
        out: &mut Vec<Effect>,
    ) -> Evaluated {
        match self.evaluate(value, actor, params, temps, out) {
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

    /// What `Value::eval` would make of `value` with its variables, parameters
    /// and reporter calls filled in. The slot is lowered once and evaluated
    /// from there. Every argument runs, left to right, before its operator
    /// does, so a reporter the operator never reads (the far side of an
    /// `and`) still runs and still does whatever it does.
    fn evaluate(
        &mut self,
        value: &Value,
        actor: &str,
        params: Option<&Params>,
        temps: &[Evaluated],
        out: &mut Vec<Effect>,
    ) -> Result<Evaluated, String> {
        let expr = match &self.current {
            Some(program) => program.expr(value, &self.variables),
            None => Rc::new(lower(value, &self.variables)),
        };
        self.eval_expr(&expr, actor, params, temps, out)
    }

    /// Evaluates a lowered slot, reporting an error the way [`Vm::eval`] does.
    fn eval_reported(
        &mut self,
        expr: &Expr,
        actor: &str,
        params: Option<&Params>,
        temps: &[Evaluated],
        out: &mut Vec<Effect>,
    ) -> Evaluated {
        match self.eval_expr(expr, actor, params, temps, out) {
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

    fn eval_expr(
        &mut self,
        expr: &Expr,
        actor: &str,
        params: Option<&Params>,
        temps: &[Evaluated],
        out: &mut Vec<Effect>,
    ) -> Result<Evaluated, String> {
        match expr {
            Expr::Const(value) => value.clone(),
            Expr::Var(var) => Ok(self.variables.read_slot(self.scope, *var)),
            Expr::Temp(index, var) => Ok(temps
                .get(*index)
                .cloned()
                .unwrap_or_else(|| self.variables.read_slot(self.scope, *var))),
            Expr::Param(name) => Ok(params
                .and_then(|bound| bound.get(name))
                .cloned()
                .unwrap_or(Evaluated::Number(0.0))),
            Expr::Call { block_id, args } => {
                Ok(self.run_reporter(actor, block_id, args, params, temps, out))
            }
            Expr::Store { op, list, args } => {
                let args = args
                    .iter()
                    .map(|arg| self.eval_expr(arg, actor, params, temps, out))
                    .collect();
                // A store reporter's own error is reported here and reads as
                // zero, rather than failing the slot around it.
                match self.read_store(actor, op, *list, args) {
                    Ok(value) => Ok(value),
                    Err(message) => {
                        out.push(Effect::Error {
                            actor: actor.to_string(),
                            message,
                        });
                        Ok(Evaluated::Number(0.0))
                    }
                }
            }
            Expr::Op {
                op: op @ (Op::Join | Op::Ext(_)),
                args,
            } => {
                let args = args
                    .iter()
                    .map(|arg| self.eval_expr(arg, actor, params, temps, out))
                    .collect::<Vec<_>>();
                ops::apply_many(op, args)
            }
            Expr::Op { op, args } => {
                let mut args = args.iter();
                let a = args
                    .next()
                    .map(|arg| self.eval_expr(arg, actor, params, temps, out));
                let b = args
                    .next()
                    .map(|arg| self.eval_expr(arg, actor, params, temps, out));
                // No operator past these reads a third, but it still runs.
                for extra in args {
                    let _ = self.eval_expr(extra, actor, params, temps, out);
                }
                ops::apply(op, a, b)
            }
        }
    }

    fn eval_f32(
        &mut self,
        value: &Value,
        actor: &str,
        params: Option<&Params>,
        temps: &[Evaluated],
        out: &mut Vec<Effect>,
    ) -> f32 {
        self.eval(value, actor, params, temps, out)
            .as_number()
            .unwrap_or(0.0) as f32
    }

    fn eval_vec3(
        &mut self,
        values: &[Value; 3],
        actor: &str,
        params: Option<&Params>,
        temps: &[Evaluated],
        out: &mut Vec<Effect>,
    ) -> [f32; 3] {
        [
            self.eval_f32(&values[0], actor, params, temps, out),
            self.eval_f32(&values[1], actor, params, temps, out),
            self.eval_f32(&values[2], actor, params, temps, out),
        ]
    }

    /// A list or dict reporter over evaluated arguments, reading only the one
    /// store its name picks.
    fn read_store(
        &self,
        actor: &str,
        op: &str,
        list: bool,
        args: Vec<Result<Evaluated, String>>,
    ) -> Result<Evaluated, String> {
        let (name, args) = stores::literal_args(op, args)?;
        let name = name.unwrap_or_default();
        let value = if list {
            resolve_list_reporter(op, args, &self.lists.scope_of(actor, &name))?
        } else {
            resolve_dict_reporter(op, args, &self.dicts.scope_of(actor, &name))?
        };
        value.eval()
    }

    /// Runs a reporter-shaped custom block's body to completion, right here,
    /// and takes its `return` as the value. Only blocks without a `wait` come
    /// this way now: anything suspendable became `Invoke` steps when the
    /// program was compiled, so nothing in here suspends.
    fn run_reporter(
        &mut self,
        actor: &str,
        block_id: &str,
        args: &[Expr],
        params: Option<&Params>,
        temps: &[Evaluated],
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
        let Some((start, names)) = program.block(block_id) else {
            return Evaluated::Number(0.0);
        };
        let values = args
            .iter()
            .take(names.len())
            .map(|arg| self.eval_reported(arg, actor, params, temps, out))
            .collect();
        let bound = Params { names, values };
        let mut script = Script {
            actor: actor.to_string(),
            key: None,
            pc: start,
            frames: vec![Frame::Call {
                return_pc: usize::MAX,
                params: bound,
                temp: None,
                saved_temps: Vec::new(),
            }],
            status: Status::Run,
            // A reporter body runs to completion in place, so it keeps the
            // clock of whoever asked - and nothing in one sleeps anyway.
            ui: sense::in_ui_strand(),
            temps: Vec::new(),
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
        names: Rc<[String]>,
        args: &[Value],
        params: Option<&Params>,
        temps: &[Evaluated],
        out: &mut Vec<Effect>,
    ) -> Params {
        let values = args
            .iter()
            .take(names.len())
            .map(|arg| self.eval(arg, actor, params, temps, out))
            .collect();
        Params { names, values }
    }
}

/// The parameters bound by the innermost custom-block call, if any.
fn current_params(frames: &[Frame]) -> Option<&Params> {
    frames.iter().rev().find_map(|frame| match frame {
        Frame::Call { params, .. } => Some(params),
        Frame::Loop { .. } => None,
    })
}

fn store_temp(temps: &mut Vec<Evaluated>, temp: usize, value: Evaluated) {
    if temps.len() <= temp {
        temps.resize(temp + 1, Evaluated::Number(0.0));
    }
    temps[temp] = value;
}

/// `(frame index, begin, end)` of the nearest enclosing loop. Only the
/// innermost frame can be it: a loop pushes its frame on entry, so anything
/// nested deeper is above it - and a call frame on top means the `break` is
/// inside a custom block, which can't escape its caller's loop.
fn nearest_loop(script: &Script) -> Option<(usize, usize, usize)> {
    match script.frames.last()? {
        Frame::Loop { begin, end, .. } => Some((script.frames.len() - 1, *begin, *end)),
        Frame::Call { .. } => None,
    }
}

/// `(frame index, return pc, temp slot)` of the innermost call frame.
fn nearest_call(script: &Script) -> Option<(usize, usize, Option<usize>)> {
    script
        .frames
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, frame)| match frame {
            Frame::Call {
                return_pc, temp, ..
            } => Some((index, *return_pc, *temp)),
            Frame::Loop { .. } => None,
        })
}
