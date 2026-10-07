// Per-actor script storage, owned by the host. A script file is shared by
// every actor running it - including clones - so `static`s would leak state
// between them. This map lives on the host side instead, keyed by the running
// actor's id, which is also what gives each clone its own copy. The runtime
// clears it when a run starts; nothing here is saved.

use std::collections::HashMap;

/// One stored value: a number or some text, never both.
#[derive(Clone, Debug, PartialEq)]
pub enum ScriptDatum {
    Number(f64),
    Text(String),
}

/// `(actor id, key)` pairs to values. Keys are plain strings the script
/// invents; an empty key never stores anything.
#[derive(Clone, Debug, Default)]
pub struct ScriptData {
    entries: HashMap<(String, String), ScriptDatum>,
}

impl ScriptData {
    pub fn new() -> ScriptData {
        ScriptData::default()
    }

    /// Whether this actor has anything under `key`, of either kind.
    pub fn has(&self, actor: &str, key: &str) -> bool {
        !key.is_empty()
            && self
                .entries
                .contains_key(&(actor.to_string(), key.to_string()))
    }

    /// The number under `key`, or nothing when it is unset or holds text.
    pub fn get_number(&self, actor: &str, key: &str) -> Option<f64> {
        match self.entries.get(&(actor.to_string(), key.to_string())) {
            Some(ScriptDatum::Number(value)) => Some(*value),
            _ => None,
        }
    }

    /// The text under `key`, or nothing when it is unset or holds a number.
    pub fn get_text(&self, actor: &str, key: &str) -> Option<&str> {
        match self.entries.get(&(actor.to_string(), key.to_string())) {
            Some(ScriptDatum::Text(value)) => Some(value),
            _ => None,
        }
    }

    /// Stores a number, replacing whatever the key held. An empty key is a
    /// no-op, so a typo can't fill the map with junk.
    pub fn set_number(&mut self, actor: &str, key: &str, value: f64) {
        if key.is_empty() {
            return;
        }
        self.entries.insert(
            (actor.to_string(), key.to_string()),
            ScriptDatum::Number(value),
        );
    }

    /// Stores text, replacing whatever the key held.
    pub fn set_text(&mut self, actor: &str, key: &str, value: &str) {
        if key.is_empty() {
            return;
        }
        self.entries.insert(
            (actor.to_string(), key.to_string()),
            ScriptDatum::Text(value.to_string()),
        );
    }

    /// Forgets one key. Missing keys are fine.
    pub fn remove(&mut self, actor: &str, key: &str) {
        self.entries.remove(&(actor.to_string(), key.to_string()));
    }

    /// Forgets everything one actor stored, as a reset without touching the
    /// other actors running the same file.
    pub fn clear_actor(&mut self, actor: &str) {
        self.entries.retain(|(owner, _), _| owner != actor);
    }

    /// Forgets everything. Runs start from silence.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_text_keep_to_their_kind() {
        let mut data = ScriptData::new();
        assert!(!data.has("a1", "t"));
        assert_eq!(data.get_number("a1", "t"), None);
        assert_eq!(data.get_text("a1", "t"), None);

        data.set_number("a1", "t", 1.5);
        assert!(data.has("a1", "t"));
        assert_eq!(data.get_number("a1", "t"), Some(1.5));
        assert_eq!(data.get_text("a1", "t"), None);

        // Writing the other kind replaces, rather than coexisting with, the
        // first - a key means one value.
        data.set_text("a1", "t", "hot");
        assert!(data.has("a1", "t"));
        assert_eq!(data.get_number("a1", "t"), None);
        assert_eq!(data.get_text("a1", "t"), Some("hot"));
    }

    #[test]
    fn actors_do_not_see_each_others_keys() {
        let mut data = ScriptData::new();
        data.set_number("a1", "cooldown", 3.0);
        assert_eq!(data.get_number("a1", "cooldown"), Some(3.0));
        assert!(!data.has("~1", "cooldown"));
        assert_eq!(data.get_number("~1", "cooldown"), None);

        // A clone writes its own copy; the template keeps its own.
        data.set_number("~1", "cooldown", 0.0);
        assert_eq!(data.get_number("a1", "cooldown"), Some(3.0));
        assert_eq!(data.get_number("~1", "cooldown"), Some(0.0));
    }

    #[test]
    fn empty_keys_store_nothing() {
        let mut data = ScriptData::new();
        data.set_number("a1", "", 1.0);
        data.set_text("a1", "", "x");
        assert!(!data.has("a1", ""));
    }

    #[test]
    fn remove_and_clear_actor_leave_neighbours_alone() {
        let mut data = ScriptData::new();
        data.set_number("a1", "a", 1.0);
        data.set_number("a1", "b", 2.0);
        data.set_number("b2", "a", 9.0);

        data.remove("a1", "a");
        assert!(!data.has("a1", "a"));
        assert!(data.has("a1", "b"));

        data.clear_actor("a1");
        assert!(!data.has("a1", "b"));
        assert!(data.has("b2", "a"));

        data.clear();
        assert!(!data.has("b2", "a"));
    }
}
