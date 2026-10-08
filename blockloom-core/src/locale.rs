//! Localized strings for a game's interface: one table of text keys to
//! per-language text, plus which language a fresh run speaks.
//!
//! A game keeps its own keys (`menu.play`, `story.intro`) and reads them
//! back through the `text for key` reporter, which falls back to the
//! default language and then to the key itself so a missing translation
//! never blanks the screen. Slots and profiles stay in [`crate::save`]:
//! a slot name doubles as a profile name, so "Slot 1" and a player name
//! are the same file either way.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The language a project without one speaks.
pub const DEFAULT_LANGUAGE: &str = "en";

/// One project's string table. Written only when not default, so an
/// untouched project re-saves byte for byte.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Localization {
    #[serde(default)]
    pub default_language: String,
    /// Text key to language to text. Both halves trimmed on write; a blank
    /// text removes its entry rather than storing an empty string.
    #[serde(default)]
    pub strings: HashMap<String, HashMap<String, String>>,
}

impl Localization {
    pub fn is_default(&self) -> bool {
        (self.default_language.is_empty() || self.default_language == DEFAULT_LANGUAGE)
            && self.strings.is_empty()
    }

    /// The language a fresh run speaks: the project's own, or English.
    pub fn language_or_default(&self) -> &str {
        if self.default_language.trim().is_empty() {
            DEFAULT_LANGUAGE
        } else {
            self.default_language.trim()
        }
    }

    /// A language tag as the blocks spell it into the lookup key:
    /// trimmed and lowercased, so "EN" and "en" are the same language.
    /// Empty stays empty, and the caller falls back to the default.
    pub fn normalize_language(language: &str) -> String {
        let mut tag = language.trim().to_ascii_lowercase();
        if tag.len() > 16 {
            tag.truncate(16);
        }
        tag
    }

    /// A text key as stored: trimmed, at most 128 characters. Empty keys
    /// are refused by [`set`](Self::set).
    pub fn normalize_key(key: &str) -> String {
        let mut key = key.trim().to_string();
        if key.len() > 128 {
            key.truncate(128);
            key = key.trim().to_string();
        }
        key
    }

    /// Writes one entry. A blank text removes the language's entry, and a
    /// key left with no languages is forgotten. Answers whether the table
    /// changed.
    pub fn set(&mut self, key: &str, language: &str, text: &str) -> bool {
        let key = Self::normalize_key(key);
        let language = Self::normalize_language(language);
        if key.is_empty() || language.is_empty() {
            return false;
        }
        let text = text.trim();
        if text.is_empty() {
            let mut changed = false;
            if let Some(entry) = self.strings.get_mut(&key) {
                changed = entry.remove(&language).is_some();
                if entry.is_empty() {
                    self.strings.remove(&key);
                }
            }
            return changed;
        }
        let entry = self.strings.entry(key).or_default();
        if entry.get(&language).is_some_and(|old| old == text) {
            return false;
        }
        entry.insert(language, text.to_string());
        true
    }

    /// Forgets one key in every language. Answers whether one was there.
    pub fn remove_key(&mut self, key: &str) -> bool {
        let key = Self::normalize_key(key);
        self.strings.remove(&key).is_some()
    }

    /// Forgets one language everywhere, plus as the default when it was.
    /// Answers how many keys changed.
    pub fn remove_language(&mut self, language: &str) -> usize {
        let language = Self::normalize_language(language);
        if language.is_empty() {
            return 0;
        }
        let mut changed = 0;
        self.strings.retain(|_, entry| {
            if entry.remove(&language).is_some() {
                changed += 1;
            }
            !entry.is_empty()
        });
        if Self::normalize_language(&self.default_language) == language {
            self.default_language.clear();
        }
        changed
    }

    /// Every language with at least one string, plus the default when it
    /// names one, sorted. What `languages` reports.
    pub fn languages(&self) -> Vec<String> {
        let mut languages: Vec<String> = self
            .strings
            .values()
            .flat_map(|e| e.keys().cloned())
            .collect();
        languages.sort();
        languages.dedup();
        let default = Self::normalize_language(self.language_or_default());
        if !languages.contains(&default) {
            languages.insert(0, default);
        }
        languages
    }

    /// The text for `key` in `language`: the language itself, then the
    /// project's default, then English, then the key. Never blank for a
    /// non-blank key, so a label always has something to draw.
    pub fn text(&self, key: &str, language: &str) -> String {
        let key = Self::normalize_key(key);
        if key.is_empty() {
            return String::new();
        }
        let language = Self::normalize_language(language);
        if let Some(text) = self.strings.get(&key).and_then(|e| e.get(&language)) {
            return text.clone();
        }
        let default = Self::normalize_language(self.language_or_default());
        if default != language
            && let Some(text) = self.strings.get(&key).and_then(|e| e.get(&default))
        {
            return text.clone();
        }
        if let Some(text) = self.strings.get(&key).and_then(|e| e.get(DEFAULT_LANGUAGE)) {
            return text.clone();
        }
        key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_falls_back_to_default_then_key() {
        let mut table = Localization::default();
        table.default_language = "fr".to_string();
        table.set("menu.play", "en", "Play");
        table.set("menu.play", "fr", "Jouer");
        assert_eq!(table.text("menu.play", "fr"), "Jouer");
        assert_eq!(table.text("menu.play", "de"), "Jouer");
        assert_eq!(table.text("menu.quit", "de"), "menu.quit");
        assert_eq!(table.text("  ", "en"), "");
    }

    #[test]
    fn blank_text_forgets_its_entry() {
        let mut table = Localization::default();
        assert!(table.set("k", "en", "hi"));
        assert!(!table.set("k", "en", "hi"));
        assert!(table.set("k", "en", "  "));
        assert!(table.is_default());
    }

    #[test]
    fn languages_lists_the_default_first() {
        let mut table = Localization::default();
        table.set("k", "fr", "x");
        assert_eq!(table.languages(), vec!["en", "fr"]);
    }
}
