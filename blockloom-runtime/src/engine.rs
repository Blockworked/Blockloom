//! The runtime's own state: the loaded project, the VM running it, and which
//! entity each actor is.
//!
//! This is a `!Send` resource (the VM holds `Rc`s), which is exactly what's
//! wanted: every system that touches it is therefore scheduled on the main
//! thread, and so is the thread-local sensor snapshot the VM reads through.

use bevy::prelude::*;
use blockloom_core::components::CameraAttach;
use blockloom_core::project::Project;
use blockloom_core::scene::Mode;
use blockloom_core::value::Evaluated;
use blockloom_core::vm::Vm;
use blockloom_protocol::EditorMessage;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

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

/// The physics pose settled at the end of a fixed step. The renderer lerps
/// between this and [`PrevPose`] to smooth the gaps between fixed steps.
#[derive(Component, Debug, Clone)]
pub struct PhysicsPose(pub Transform);

/// The physics pose one fixed step older than [`PhysicsPose`].
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
    pub project: Project,
    pub vm: Vm,
    /// Actor id -> its entity, for as long as the world stands.
    pub entities: HashMap<String, Entity>,
    pub running: bool,
    pub paused: bool,
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
    /// Whether this run has called every script's `start` yet.
    pub scripts_started: bool,
    /// Which components each actor is carrying right now. Seeded from the
    /// project on every rebuild and moved by `attach`/`detach`, so it - not
    /// the document - is what a mid-run question about a component answers.
    pub attached: HashMap<String, HashSet<String>>,
}

impl Engine {
    pub fn new(incoming: Receiver<EditorMessage>, mode: Mode) -> Self {
        Self {
            incoming,
            project: Project::starter("Untitled", mode),
            vm: Vm::new(),
            entities: HashMap::new(),
            running: false,
            paused: false,
            pause_began: None,
            started_at: 0.0,
            touching: HashMap::new(),
            speech: HashMap::new(),
            next_report: 0.0,
            rebuild: true,
            project_dir: None,
            scripts: HashMap::new(),
            scripts_started: false,
            attached: HashMap::new(),
        }
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
