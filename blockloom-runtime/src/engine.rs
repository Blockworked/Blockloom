//! The runtime's own state: the loaded project, its block scheduler, and which
//! entity each actor is.
//!
//! This is a `!Send` resource (the VM holds `Rc`s), which is exactly what's
//! wanted: every system that touches it is therefore scheduled on the main
//! thread, and so is the thread-local sensor snapshot the VM reads through.

use bevy::prelude::*;
use blockloom_core::components::CameraAttach;
use blockloom_core::project::{Actor, Project};
use blockloom_core::scene::Mode;
use blockloom_core::value::Evaluated;
use blockloom_core::vm::{Variables, Vm};
use blockloom_protocol::EditorMessage;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

/// Marks a spawned actor and ties it back to its id in the project.
#[derive(Component, Debug, Clone)]
pub struct ActorId(pub String);

/// The actor's custom components, live. Authored values seed it on every
/// rebuild; `set <field> of <component>` writes here, and the sensing
/// snapshot reads back out, so a run's changes last exactly as long as the
/// run does - like a position, and unlike a variable.
#[derive(Component, Debug, Clone, Default)]
pub struct CustomComponents(pub HashMap<String, HashMap<String, Evaluated>>);

/// A camera component on this actor. Mirrored onto the entity so
/// `set camera to first person` can change it without touching the document.
#[derive(Component, Debug, Clone, Copy)]
pub struct CameraRig(pub CameraAttach);

/// A `glide` in progress: the host interpolates while the script sleeps.
#[derive(Component, Debug, Clone)]
pub struct Gliding {
    pub from: Vec3,
    pub to: Vec3,
    pub elapsed: f32,
    pub duration: f32,
}

/// The pose an actor settled at the end of a fixed step - physically, or from
/// that step's own effects. The renderer lerps between this and [`PrevPose`]
/// to smooth the gaps between fixed steps.
#[derive(Component, Debug, Clone)]
pub struct PhysicsPose(pub Transform);

/// The pose one fixed step older than [`PhysicsPose`].
#[derive(Component, Debug, Clone)]
pub struct PrevPose(pub Transform);

/// Effects produced by this frame's VM tick, waiting to be applied.
#[derive(Resource, Default)]
pub struct PendingEffects(pub Vec<blockloom_core::vm::Effect>);

/// Which dimension this process was started for. The editor restarts the
/// runtime when a project switches mode, so it never changes here.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dimension(pub Mode);

pub struct Engine {
    pub incoming: Receiver<EditorMessage>,
    /// A built game's own end of that channel. There is no editor to send it
    /// Load and Start, so it sends them to itself and holds the sender for as
    /// long as it runs - dropping it would read as the editor hanging up.
    pub link: Option<Sender<EditorMessage>>,
    pub project: Project,
    pub vm: Vm,
    pub variables: Variables,
    /// A built game's native block program. Editor Play keeps using the VM so
    /// what is being edited always runs immediately.
    pub logic: Option<crate::logic::LoadedLogic>,
    /// How many actors the host itself has made this run. Only a compiled
    /// program's run needs them: it mints its own ids, so the host's carry a
    /// shape of their own and the two can never collide.
    made: usize,
    /// Actor id -> its entity, for as long as the world stands.
    pub entities: HashMap<String, Entity>,
    pub running: bool,
    pub paused: bool,
    /// Whether the game window actually holds OS focus, from `WindowFocused`
    /// events only. Bevy's `Window::focused` defaults to true and winit only
    /// reports changes, so a window opened behind the editor would read
    /// focused forever without this - and raw device input would drive a game
    /// whose cursor sits in another window.
    pub window_focused: bool,
    /// Whether a `lock mouse` block wants the pointer grabbed right now.
    /// Kept apart from the window's own options because the backend only
    /// attempts the grab on change: a request that lands before the window is
    /// focused degrades to confined-or-nothing, so the runtime re-asserts
    /// from this flag until the real lock sticks.
    pub wants_cursor_locked: bool,
    /// `Time::elapsed_secs` when the current pause began, if paused. Used to
    /// keep the `timer` reporter frozen while paused.
    pub pause_began: Option<f64>,
    /// `Time::elapsed_secs` when the green flag was pressed.
    pub started_at: f64,
    /// Who is touching whom, from collision messages, by actor id.
    pub touching: HashMap<String, HashSet<String>>,
    /// The current speech bubble for each actor. A later `say` replaces the
    /// earlier one, and an empty `say` clears it.
    pub speech: HashMap<String, String>,
    /// When the next status report is due, in elapsed seconds.
    pub next_report: f64,
    /// Set when the world needs rebuilding from `project` before the next tick.
    pub rebuild: bool,
    /// The open project's folder, which is where its assets and its built
    /// script libraries are. `None` until the editor says.
    pub project_dir: Option<PathBuf>,
    /// Each actor's loaded script, by actor id. Reopened on every rebuild, so
    /// a script edited and rebuilt between runs takes effect on the next Play.
    pub scripts: HashMap<String, crate::script::LoadedScript>,
    /// Which actors' scripts have had their `start` called. Per actor rather
    /// than one flag for the run, since a clone made half way through still
    /// needs its own.
    pub scripts_started: HashSet<String>,
    /// Which components each actor is carrying right now. Seeded from the
    /// project on every rebuild and moved by `attach`/`detach`, so it - not
    /// the document - is what a mid-run question about a component answers.
    pub attached: HashMap<String, HashSet<String>>,
    /// Actors the run made that the document never had: clones, and actors a
    /// `create actor` block conjured. Looked up before the project, so the
    /// rest of the runtime asks one question to find any actor at all.
    pub spawned: HashMap<String, Actor>,
    /// Clone id -> the authored actor it was copied from. Only clones are in
    /// here, which is what "am I a clone?" reads.
    pub clones: HashMap<String, String>,
    /// Child actor id -> the actor it hangs off. Seeded from every `Parent`
    /// component on a rebuild and moved by `set my parent to` after that.
    pub parents: HashMap<String, String>,
    /// Actor id -> the last actor or clone it made, for "the actor I made".
    pub last_created: HashMap<String, String>,
}

impl Engine {
    pub fn new(incoming: Receiver<EditorMessage>, mode: Mode) -> Self {
        let variables = Variables::default();
        Self {
            incoming,
            link: None,
            project: Project::starter("Untitled", mode),
            vm: Vm::with_variables(variables.clone()),
            variables,
            logic: None,
            made: 0,
            entities: HashMap::new(),
            running: false,
            paused: false,
            window_focused: false,
            wants_cursor_locked: false,
            pause_began: None,
            started_at: 0.0,
            touching: HashMap::new(),
            speech: HashMap::new(),
            next_report: 0.0,
            rebuild: true,
            project_dir: None,
            scripts: HashMap::new(),
            scripts_started: HashSet::new(),
            attached: HashMap::new(),
            spawned: HashMap::new(),
            clones: HashMap::new(),
            parents: HashMap::new(),
            last_created: HashMap::new(),
        }
    }

    /// Any actor in the running world: one the run made first, then one the
    /// document authored. Everything that needs an actor's authored shape,
    /// physics or components goes through here, so a clone answers the same
    /// questions the actor it was copied from does.
    pub fn actor(&self, id: &str) -> Option<&Actor> {
        self.spawned.get(id).or_else(|| self.project.actor(id))
    }

    /// Every actor in the world, authored and made, by id.
    pub fn actor_ids(&self) -> impl Iterator<Item = &String> {
        self.entities.keys()
    }

    /// Whether hanging `child` off `parent` would make a loop - following
    /// `parent` upwards eventually arrives back at `child`.
    pub fn would_loop(&self, child: &str, parent: &str) -> bool {
        blockloom_core::project::reaches(&self.parents, parent, child)
    }

    /// Whether `actor` is carrying `component` at this moment in the run.
    pub fn has_component(&self, actor: &str, component: &str) -> bool {
        self.attached
            .get(actor)
            .is_some_and(|held| held.contains(component))
    }

    /// Seconds since the green flag, which is what the `timer` reporter reads.
    /// Frozen while paused, so resuming doesn't jump the timer forward.
    pub fn run_time(&self, now: f64) -> f64 {
        let end = match self.pause_began {
            Some(began) => began.min(now),
            None => now,
        };
        (end - self.started_at).max(0.0)
    }

    pub fn actor_id_of(&self, entity: Entity) -> Option<&str> {
        self.entities
            .iter()
            .find(|(_, candidate)| **candidate == entity)
            .map(|(id, _)| id.as_str())
    }

    pub fn note_say(&mut self, actor: &str, text: &str) {
        if text.is_empty() {
            self.speech.remove(actor);
        } else {
            self.speech.insert(actor.to_string(), text.to_string());
        }
    }

    /// An id for an actor the host is making itself, distinct from anything
    /// the VM or a compiled program mints.
    pub fn new_actor_id(&mut self) -> String {
        self.made += 1;
        format!("~h{}", self.made)
    }

    pub fn fire(&mut self, event: blockloom_core::vm::Event) {
        if let Some(logic) = &mut self.logic {
            logic.fire(event, &self.project);
        } else {
            self.vm.fire(event);
        }
    }

    pub fn stop_program(&mut self) {
        self.vm.stop_all();
        self.made = 0;
        if let Some(logic) = &mut self.logic {
            logic.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn say_replaces_and_empty_say_clears_an_actors_bubble() {
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::TwoD);

        engine.note_say("player", "Hello");
        engine.note_say("player", "Still here");
        assert_eq!(
            engine.speech.get("player").map(String::as_str),
            Some("Still here")
        );

        engine.note_say("player", "");
        assert!(!engine.speech.contains_key("player"));
    }
}
