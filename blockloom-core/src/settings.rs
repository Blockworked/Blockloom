//! Settings that hold for the whole game rather than one scene: quality,
//! input actions, the sound buses and the display output. They used to sit
//! on every scene's `World`, where a per-scene difference was only ever a
//! bug; [`GameSettings::adopt`] carries an old project's values over.

use crate::input::InputConfig;
use crate::quality;
use crate::scene::DisplayOutput;
use crate::sound::SoundMixer;
use blockloom_schema_derive::Schema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, Schema)]
#[serde(default)]
pub struct GameSettings {
    /// Render quality, resolution scaling and the upscaler.
    #[schema(label = "Quality")]
    pub quality: quality::Settings,
    /// Named input actions and their bindings.
    #[schema(label = "Input")]
    pub input: InputConfig,
    /// One gain per sound bus.
    #[schema(label = "Sound")]
    pub sound: SoundMixer,
    /// The output signal, peak brightness and paper white.
    #[schema(label = "Display")]
    pub display: DisplayOutput,
    /// A `.blockstyle` every interface starts from; a screen's own
    /// stylesheets still win.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[schema(label = "Interface theme", asset)]
    pub style: String,
    /// `.blockui` screens drawn over every scene, above the scene's own.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[schema(skip)]
    pub screens: Vec<crate::ui_layers::UiLayer>,
}

impl GameSettings {
    /// What a new project starts with: the starter input actions.
    pub fn starter() -> Self {
        Self {
            input: InputConfig::starter(),
            ..Self::default()
        }
    }

    pub fn normalize(&mut self) {
        self.quality.normalize();
        self.input.normalize();
    }
}

/// The four settings as an old scene carried them, read but never written.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LegacySettings {
    pub quality: Option<quality::Settings>,
    pub input: Option<InputConfig>,
    pub sound: Option<SoundMixer>,
    pub display: Option<DisplayOutput>,
}

impl LegacySettings {
    pub fn is_empty(&self) -> bool {
        self.quality.is_none()
            && self.input.is_none()
            && self.sound.is_none()
            && self.display.is_none()
    }

    /// Lays every value this scene carried over `settings`.
    pub fn adopt_into(&self, settings: &mut GameSettings) {
        if let Some(quality) = &self.quality {
            settings.quality = quality.clone();
        }
        if let Some(input) = &self.input {
            settings.input = input.clone();
        }
        if let Some(sound) = self.sound {
            settings.sound = sound;
        }
        if let Some(display) = self.display {
            settings.display = display;
        }
    }
}
