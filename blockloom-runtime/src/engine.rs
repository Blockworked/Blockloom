//! The runtime's own state: the loaded project, the VM running it, and which
//! entity each actor is.
//!
//! This is a `!Send` resource (the VM holds `Rc`s), which is exactly what's
//! wanted: every system that touches it is therefore scheduled on the main
//! thread, and so is the thread-local sensor snapshot the VM reads through.

use bevy::prelude::*;
use blockloom_core::project::Project;
use blockloom_core::scene::Mode;
use blockloom_core::vm::Vm;
use blockloom_protocol::EditorMessage;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Receiver;

/// Marks a spawned actor and ties it back to its id in the project.
#[derive(Component, Debug, Clone)]
pub struct ActorId(pub String);

/// A `glide` in progress: the host interpolates while the script sleeps.
#[derive(Component, Debug, Clone)]
pub struct Gliding {
    pub from: Vec3,
    pub to: Vec3,
    pub elapsed: f32,
    pub duration: f32,
}

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
    /// `Time::elapsed_secs` when the green flag was pressed.
    pub started_at: f64,
    /// Who is touching whom, from collision messages, by actor id.
    pub touching: HashMap<String, HashSet<String>>,
    /// The last few `say`s, for the on-screen overlay.
    pub says: Vec<String>,
    /// When the next status report is due, in elapsed seconds.
    pub next_report: f64,
    /// Set when the world needs rebuilding from `project` before the next tick.
    pub rebuild: bool,
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
            started_at: 0.0,
            touching: HashMap::new(),
            says: Vec::new(),
            next_report: 0.0,
            rebuild: true,
        }
    }

    /// Seconds since the green flag, which is what the `timer` reporter reads.
    pub fn run_time(&self, now: f64) -> f64 {
        (now - self.started_at).max(0.0)
    }

    pub fn actor_id_of(&self, entity: Entity) -> Option<&str> {
        self.entities
            .iter()
            .find(|(_, candidate)| **candidate == entity)
            .map(|(id, _)| id.as_str())
    }

    pub fn note_say(&mut self, actor: &str, text: &str) {
        let name = self
            .project
            .actor(actor)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| actor.to_string());
        self.says.push(format!("{name}: {text}"));
        let overflow = self.says.len().saturating_sub(6);
        self.says.drain(..overflow);
    }
}
