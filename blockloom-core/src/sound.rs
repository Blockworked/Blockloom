//! The project's sound: mixing buses and the dials blocks drive.
//!
//! A game routes every voice through one of three buses - `Master`, `Music`
//! or `Sfx` - each with its own volume, so one slider tames the soundtrack
//! while another tames the pew-pew. [`SoundMixer`] is the saved mix, kept on
//! the [`crate::scene::World`]; the runtime seeds its live gains from it and
//! a `set bus volume` block moves them mid-run without touching the document,
//! the same split world gravity has.

use serde::{Deserialize, Serialize};

/// One mixing bus. `Master` scales everything; `Music` and `Sfx` scale their
/// own voices on top of it, so the effective gain of a voice is always
/// `master * bus * voice`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum SoundBus {
    Master,
    Music,
    /// Sound effects. Spelled `Sfx` on the wire, "sound effects" in the UI.
    #[default]
    Sfx,
}

impl SoundBus {
    /// Every bus, in the order the project settings dialog lists them.
    pub const ALL: &[SoundBus] = &[SoundBus::Master, SoundBus::Music, SoundBus::Sfx];

    /// What the editor's dropdown and the reporters agree on.
    pub fn name(self) -> &'static str {
        match self {
            SoundBus::Master => "Master",
            SoundBus::Music => "Music",
            SoundBus::Sfx => "Sfx",
        }
    }

    /// Parses what a reporter arg or a document names. Case-insensitive, so a
    /// `volume of "music"` reads the same bus a block routes to.
    pub fn parse(name: &str) -> Option<SoundBus> {
        match name.trim().to_lowercase().as_str() {
            "master" => Some(SoundBus::Master),
            "music" => Some(SoundBus::Music),
            "sfx" | "effects" | "sound effects" => Some(SoundBus::Sfx),
            _ => None,
        }
    }
}

/// The saved mix: one linear gain per bus. `1.0` is unity - the file as
/// mastered - and the runtime clamps live writes to twice that, so a block
/// can push a whisper without blowing the speakers out.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SoundMixer {
    #[serde(default = "unity")]
    pub master_volume: f32,
    #[serde(default = "unity")]
    pub music_volume: f32,
    #[serde(default = "unity")]
    pub sfx_volume: f32,
}

fn unity() -> f32 {
    1.0
}

impl Default for SoundMixer {
    fn default() -> Self {
        Self {
            master_volume: 1.0,
            music_volume: 1.0,
            sfx_volume: 1.0,
        }
    }
}

impl SoundMixer {
    pub fn gain(self, bus: SoundBus) -> f32 {
        match bus {
            SoundBus::Master => self.master_volume,
            SoundBus::Music => self.music_volume,
            SoundBus::Sfx => self.sfx_volume,
        }
    }

    pub fn set_gain(&mut self, bus: SoundBus, gain: f32) {
        let gain = clamp_gain(gain);
        match bus {
            SoundBus::Master => self.master_volume = gain,
            SoundBus::Music => self.music_volume = gain,
            SoundBus::Sfx => self.sfx_volume = gain,
        }
    }
}

/// Blocks speak 0-100; the engine speaks linear gain. Anything plays, and the
/// runtime clamps to silence..double.
pub fn user_to_gain(volume: f64) -> f32 {
    clamp_gain(volume as f32 / 100.0)
}

/// The engine back to blocks: a gain as 0-100.
pub fn gain_to_user(gain: f32) -> f64 {
    (gain.max(0.0) * 100.0) as f64
}

/// A gain into the live range: silence at the bottom, double at the top.
pub fn clamp_gain(gain: f32) -> f32 {
    gain.clamp(0.0, 2.0)
}

/// A pitch factor into the playable range. `1.0` is the file as recorded;
/// an octave down and two octaves up is as far as a game usefully goes.
pub fn clamp_pitch(pitch: f32) -> f32 {
    if !pitch.is_finite() {
        return 1.0;
    }
    pitch.clamp(0.125, 4.0)
}

/// A sound slot into the asset path the runtime loads: trimmed, still
/// project-relative (`assets/sounds/jump.wav`). Empty means the block named
/// nothing, which the VM reports rather than plays.
pub fn normalize_sound(path: &str) -> String {
    path.trim().to_string()
}

/// How many voices of one file may overlap before the oldest is stolen.
/// Without a cap a `forever` loop playing a blip stacks hundreds of copies.
pub const MAX_VOICES_PER_SOUND: usize = 8;

/// A backstop on total live voices, so pathological projects degrade to
/// stealing rather than to unbounded entity spawning.
pub const MAX_VOICES: usize = 128;

/// How far a positional voice carries: linear falloff to silence at this
/// distance, in world units - pixels in 2D, metres in 3D.
pub fn max_audible_distance(is_3d: bool) -> f32 {
    if is_3d { 45.0 } else { 900.0 }
}

/// A distance into a gain: full volume adjacent, silence past the max.
pub fn distance_gain(distance: f32, is_3d: bool) -> f32 {
    let max = max_audible_distance(is_3d);
    (1.0 - distance / max).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buses_parse_the_names_blocks_and_reporters_use() {
        assert_eq!(SoundBus::parse("Master"), Some(SoundBus::Master));
        assert_eq!(SoundBus::parse("music"), Some(SoundBus::Music));
        assert_eq!(SoundBus::parse("Sfx"), Some(SoundBus::Sfx));
        assert_eq!(SoundBus::parse("sound effects"), Some(SoundBus::Sfx));
        assert_eq!(SoundBus::parse("nope"), None);
    }

    #[test]
    fn the_mixer_reads_and_writes_each_bus() {
        let mut mixer = SoundMixer::default();
        assert_eq!(mixer.gain(SoundBus::Music), 1.0);
        mixer.set_gain(SoundBus::Music, 0.5);
        assert_eq!(mixer.gain(SoundBus::Music), 0.5);
        // Live writes clamp rather than explode.
        mixer.set_gain(SoundBus::Sfx, 99.0);
        assert_eq!(mixer.gain(SoundBus::Sfx), 2.0);
        mixer.set_gain(SoundBus::Master, -3.0);
        assert_eq!(mixer.gain(SoundBus::Master), 0.0);
    }

    #[test]
    fn volumes_round_trip_between_blocks_and_gains() {
        assert_eq!(user_to_gain(100.0), 1.0);
        assert_eq!(user_to_gain(50.0), 0.5);
        assert_eq!(user_to_gain(0.0), 0.0);
        assert_eq!(user_to_gain(500.0), 2.0);
        assert_eq!(gain_to_user(0.5), 50.0);
    }

    #[test]
    fn pitch_clamps_to_a_playable_range() {
        assert_eq!(clamp_pitch(1.0), 1.0);
        assert_eq!(clamp_pitch(0.0), 0.125);
        assert_eq!(clamp_pitch(-2.0), 0.125);
        assert_eq!(clamp_pitch(99.0), 4.0);
        assert_eq!(clamp_pitch(f32::NAN), 1.0);
    }

    #[test]
    fn distance_falloff_is_full_up_close_and_silent_past_the_max() {
        assert_eq!(distance_gain(0.0, false), 1.0);
        assert_eq!(distance_gain(450.0, false), 0.5);
        assert_eq!(distance_gain(900.0, false), 0.0);
        assert_eq!(distance_gain(5000.0, false), 0.0);
        assert_eq!(distance_gain(0.0, true), 1.0);
        assert_eq!(distance_gain(45.0, true), 0.0);
    }

    #[test]
    fn an_old_mixer_gets_unity_gains() {
        let mixer: SoundMixer = serde_json::from_str("{}").unwrap();
        assert_eq!(mixer, SoundMixer::default());
    }
}
