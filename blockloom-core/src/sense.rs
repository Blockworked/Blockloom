//! The world state reporter blocks read.
//!
//! blockstitch evaluates a value tree through plain functions with no
//! context, so the sensing operators in [`crate::value`] read a snapshot the
//! host publishes once per frame instead. [`with_actor`] supplies the other
//! half: which actor's script is being evaluated right now, so "x position"
//! means the running actor's own.

use crate::value::Evaluated;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// One actor, as its own blocks, its script, and other actors can see it.
#[derive(Debug, Clone)]
pub struct ActorSense {
    pub name: String,
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: f32,
    pub visible: bool,
    /// Ids of the actors this one is currently touching.
    pub touching: HashSet<String>,
    /// Which components the actor is carrying right now, which an `attach`
    /// or `detach` earlier in the run may have changed.
    pub attached: HashSet<String>,
    /// The actor's custom components as they stand this frame, by component
    /// name then field name. Live values, not the authored ones: a block that
    /// wrote a field last frame reads its own number back.
    pub components: HashMap<String, HashMap<String, Evaluated>>,
}

/// Everything sensible about the world this frame. Keyed by actor id.
#[derive(Debug, Clone, Default)]
pub struct Sensors {
    /// Seconds since the run started.
    pub time: f64,
    /// Held keys, in [`normalize_key`]'s spelling.
    pub keys: HashSet<String>,
    /// Pointer position in world units.
    pub mouse: [f32; 2],
    pub mouse_down: bool,
    pub actors: HashMap<String, ActorSense>,
}

impl Default for ActorSense {
    fn default() -> Self {
        Self {
            name: String::new(),
            position: [0.0; 3],
            rotation: [0.0; 3],
            scale: 1.0,
            visible: true,
            touching: HashSet::new(),
            attached: HashSet::new(),
            components: HashMap::new(),
        }
    }
}

impl Sensors {
    /// Looks an actor up by id first, then by name (case-insensitively), so a
    /// block can name either.
    pub fn find(&self, id_or_name: &str) -> Option<&ActorSense> {
        if let Some(actor) = self.actors.get(id_or_name) {
            return Some(actor);
        }
        self.actors
            .values()
            .find(|actor| actor.name.eq_ignore_ascii_case(id_or_name))
    }
}

thread_local! {
    /// The snapshot this thread published. Thread-local rather than global
    /// because the VM is `!Send` and always runs on the same thread that
    /// publishes for it - the renderer's main thread - so a lock here would
    /// only ever let unrelated threads (or tests) interfere with each other.
    static SENSORS: RefCell<Sensors> = RefCell::new(Sensors::default());

    /// Actor id whose script is currently being stepped.
    static CURRENT_ACTOR: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Replaces this thread's snapshot - the host calls it once a frame, before
/// stepping any script.
pub fn publish(sensors: Sensors) {
    SENSORS.with(|slot| *slot.borrow_mut() = sensors);
}

/// Reads the published snapshot. `f` sees a default-empty one before the
/// first [`publish`], so a reporter previewed in the editor still answers.
pub fn read<R>(f: impl FnOnce(&Sensors) -> R) -> R {
    SENSORS.with(|slot| f(&slot.borrow()))
}

/// Runs `f` with `actor_id` as the actor "my x position" and friends refer
/// to. Nested calls restore the previous actor on the way out.
pub fn with_actor<R>(actor_id: &str, f: impl FnOnce() -> R) -> R {
    let previous = CURRENT_ACTOR.with(|cell| cell.replace(Some(actor_id.to_string())));
    let result = f();
    CURRENT_ACTOR.with(|cell| *cell.borrow_mut() = previous);
    result
}

/// The actor whose script is being stepped, if any.
pub fn current_actor() -> Option<String> {
    CURRENT_ACTOR.with(|cell| cell.borrow().clone())
}

/// The key names blocks and the runtime agree on. Anything not listed here
/// is still usable - a single printable character works as itself - this is
/// just what the editor's dropdown offers.
pub const KEY_NAMES: &[&str] = &[
    "space",
    "up arrow",
    "down arrow",
    "left arrow",
    "right arrow",
    "enter",
    "escape",
    "shift",
    "control",
    "alt",
    "tab",
    "backspace",
    "a",
    "b",
    "c",
    "d",
    "e",
    "f",
    "g",
    "h",
    "i",
    "j",
    "k",
    "l",
    "m",
    "n",
    "o",
    "p",
    "q",
    "r",
    "s",
    "t",
    "u",
    "v",
    "w",
    "x",
    "y",
    "z",
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
];

/// Canonical spelling of a key name - what both the `when key pressed`
/// header and the `key down?` reporter compare on.
pub fn normalize_key(key: &str) -> String {
    key.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actors_resolve_by_id_or_by_name() {
        let mut sensors = Sensors::default();
        sensors.actors.insert(
            "a1".to_string(),
            ActorSense {
                name: "Player".to_string(),
                ..Default::default()
            },
        );
        assert!(sensors.find("a1").is_some());
        assert!(sensors.find("player").is_some());
        assert!(sensors.find("Enemy").is_none());
    }

    #[test]
    fn the_current_actor_is_restored_after_a_nested_scope() {
        with_actor("outer", || {
            with_actor("inner", || {
                assert_eq!(current_actor().as_deref(), Some("inner"))
            });
            assert_eq!(current_actor().as_deref(), Some("outer"));
        });
        assert_eq!(current_actor(), None);
    }
}
