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
    /// The actor this one hangs off right now, by id, or empty for none.
    pub parent: String,
    /// True for an actor a `create clone` block made rather than the editor.
    pub is_clone: bool,
    /// The id of the last actor or clone this one made, so a block can move,
    /// parent or delete what it just created. Empty until it makes one.
    pub last_created: String,
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
    /// Seconds since the run started. Frozen while the game is paused.
    pub time: f64,
    /// The same, unfrozen: what a strand the interface started reads, so a
    /// clock on a pause menu keeps its own time.
    pub wall_time: f64,
    /// Whether `pause game` has the world frozen right now.
    pub paused: bool,
    /// Held keys, in [`normalize_key`]'s spelling.
    pub keys: HashSet<String>,
    /// Pointer position in world units.
    pub mouse: [f32; 2],
    /// Raw pointer motion in pixels since the last frame: x grows as the
    /// pointer moves right, y as it moves down. What a first-person camera
    /// wants, where `mouse` is a place an actor can stand.
    pub mouse_delta: [f32; 2],
    /// Whether the pointer is grabbed and hidden for first-person play.
    pub mouse_locked: bool,
    pub mouse_down: bool,
    pub actors: HashMap<String, ActorSense>,
    /// Each interface element's current value, by id: a slider's number, a
    /// toggle's on/off, an input's text. What `value of (id)` reads.
    pub ui: HashMap<String, Evaluated>,
}

impl Default for ActorSense {
    fn default() -> Self {
        Self {
            name: String::new(),
            position: [0.0; 3],
            rotation: [0.0; 3],
            scale: 1.0,
            visible: true,
            parent: String::new(),
            is_clone: false,
            last_created: String::new(),
            touching: HashSet::new(),
            attached: HashSet::new(),
            components: HashMap::new(),
        }
    }
}

impl Sensors {
    /// How many actors answer to `name` right now - clones included, since
    /// they share their template's. An empty name counts every actor.
    pub fn count_named(&self, name: &str) -> usize {
        let name = name.trim();
        if name.is_empty() {
            return self.actors.len();
        }
        self.actors
            .values()
            .filter(|actor| actor.name.eq_ignore_ascii_case(name))
            .count()
    }

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

    /// The clock a script should read: the world's, or the wall's when the
    /// interface is what started it.
    pub fn clock(&self, ui: bool) -> f64 {
        if ui { self.wall_time } else { self.time }
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

    /// Whether that script was started by the interface. Kept beside the
    /// actor for the same reason: the operators are plain `fn`s with no
    /// context, and `timer` has to know which clock it is being asked for.
    static UI_STRAND: RefCell<bool> = const { RefCell::new(false) };
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
    with_script(actor_id, in_ui_strand(), f)
}

/// The same, also saying whether the interface started this script - which
/// is what decides the clock `timer` answers with.
pub fn with_script<R>(actor_id: &str, ui: bool, f: impl FnOnce() -> R) -> R {
    let previous = CURRENT_ACTOR.with(|cell| cell.replace(Some(actor_id.to_string())));
    let was_ui = UI_STRAND.with(|cell| cell.replace(ui));
    let result = f();
    CURRENT_ACTOR.with(|cell| *cell.borrow_mut() = previous);
    UI_STRAND.with(|cell| *cell.borrow_mut() = was_ui);
    result
}

/// The actor whose script is being stepped, if any.
pub fn current_actor() -> Option<String> {
    CURRENT_ACTOR.with(|cell| cell.borrow().clone())
}

/// Whether the script being stepped was started by the interface.
pub fn in_ui_strand() -> bool {
    UI_STRAND.with(|cell| *cell.borrow())
}

/// Says which kind of strand the host is about to step, for a scheduler
/// that isn't the VM. The compiled runner calls this through the ABI before
/// each slice, so `timer` answers the same clock on both sides.
pub fn set_ui_strand(ui: bool) {
    UI_STRAND.with(|cell| *cell.borrow_mut() = ui);
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
    fn a_ui_strand_reads_the_wall_clock_and_everything_else_the_worlds() {
        let sensors = Sensors {
            time: 2.0,
            wall_time: 9.0,
            ..Default::default()
        };
        assert_eq!(sensors.clock(false), 2.0);
        assert_eq!(sensors.clock(true), 9.0);
    }

    #[test]
    fn the_ui_flag_is_restored_with_the_actor_after_a_nested_scope() {
        with_script("outer", true, || {
            assert!(in_ui_strand());
            with_script("inner", false, || assert!(!in_ui_strand()));
            assert!(in_ui_strand());
        });
        assert!(!in_ui_strand());
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
