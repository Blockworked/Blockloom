//! Tweens, sprite flipbooks and the animation player.
//!
//! Phase 2's animation half, shared by both dimensions (the 2D-specific
//! extras - skeletal rigs, 9-slice, parallax - live in Phase 6, on top of
//! this). Three pieces:
//!
//! - [`TweenEasing`]: how a tween gets from here to there. `Glide` and the
//!   `tween ...` blocks all run through [`TweenEasing::apply`], so the VM,
//!   compiled logic and scripts agree on what "ease out" means.
//! - [`AnimationClip`]: a flipbook - frames of image assets played at `fps`
//!   in a [`LoopMode`]. An actor carries its clips on its `Animation`
//!   component; the runtime's player swaps the displayed frame.
//! - States are clip names: `play clip` changes state, `when animation ends`
//!   fires the transition, and the `current clip` / `current frame`
//!   reporters read it back. No second implementation for 2D.

use serde::{Deserialize, Serialize};

/// How a tween eases from 0 to 1. Travels as its name, which is what the
/// easing dropdown on the tween blocks writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TweenEasing {
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    Bounce,
    Elastic,
}

impl TweenEasing {
    /// Every easing, in palette order.
    pub const ALL: &[TweenEasing] = &[
        TweenEasing::Linear,
        TweenEasing::EaseIn,
        TweenEasing::EaseOut,
        TweenEasing::EaseInOut,
        TweenEasing::Bounce,
        TweenEasing::Elastic,
    ];

    pub fn name(self) -> &'static str {
        match self {
            TweenEasing::Linear => "Linear",
            TweenEasing::EaseIn => "EaseIn",
            TweenEasing::EaseOut => "EaseOut",
            TweenEasing::EaseInOut => "EaseInOut",
            TweenEasing::Bounce => "Bounce",
            TweenEasing::Elastic => "Elastic",
        }
    }

    pub fn parse(name: &str) -> Option<TweenEasing> {
        match name.trim().to_lowercase().as_str() {
            "linear" => Some(TweenEasing::Linear),
            "easein" | "ease-in" | "ease_in" => Some(TweenEasing::EaseIn),
            "easeout" | "ease-out" | "ease_out" => Some(TweenEasing::EaseOut),
            "easeinout" | "ease-in-out" | "ease_in_out" => Some(TweenEasing::EaseInOut),
            "bounce" => Some(TweenEasing::Bounce),
            "elastic" => Some(TweenEasing::Elastic),
            _ => None,
        }
    }

    /// Maps a linear 0-1 progress onto the eased one. Clamped, so a stray
    /// overstep never leaves the 0-1 range a lerp expects.
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            TweenEasing::Linear => t,
            TweenEasing::EaseIn => t * t,
            TweenEasing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            TweenEasing::EaseInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }
            TweenEasing::Bounce => bounce_out(t),
            TweenEasing::Elastic => elastic_out(t),
        }
    }
}

fn bounce_out(t: f32) -> f32 {
    const N1: f32 = 7.5625;
    const D1: f32 = 2.75;
    if t < 1.0 / D1 {
        N1 * t * t
    } else if t < 2.0 / D1 {
        let t = t - 1.5 / D1;
        N1 * t * t + 0.75
    } else if t < 2.5 / D1 {
        let t = t - 2.25 / D1;
        N1 * t * t + 0.9375
    } else {
        let t = t - 2.625 / D1;
        N1 * t * t + 0.984375
    }
}

fn elastic_out(t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let c = (2.0 * std::f32::consts::PI) / 3.0;
    2.0_f32.powf(-10.0 * t) * ((t * 10.0 - 0.75) * c).sin() + 1.0
}

/// What a flipbook does at its ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum LoopMode {
    /// Stops on the last frame and fires `when animation ends`.
    #[default]
    Once,
    /// Starts over, forever.
    Loop,
    /// Plays forward then back, forever.
    PingPong,
}

impl LoopMode {
    pub const ALL: &[LoopMode] = &[LoopMode::Once, LoopMode::Loop, LoopMode::PingPong];

    pub fn name(self) -> &'static str {
        match self {
            LoopMode::Once => "Once",
            LoopMode::Loop => "Loop",
            LoopMode::PingPong => "PingPong",
        }
    }

    pub fn parse(name: &str) -> Option<LoopMode> {
        match name.trim().to_lowercase().as_str() {
            "once" => Some(LoopMode::Once),
            "loop" => Some(LoopMode::Loop),
            "pingpong" | "ping-pong" | "ping_pong" => Some(LoopMode::PingPong),
            _ => None,
        }
    }
}

/// One flipbook: frames of image assets played at `fps`. `frames` are
/// project-relative paths, the same spelling a Look's image uses
/// (`assets/sprites/walk-1.png`). Empty plays nothing; a clip with no
/// frames is an error where a block names it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationClip {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub frames: Vec<String>,
    #[serde(default = "default_fps")]
    pub fps: f32,
    #[serde(default)]
    pub loop_mode: LoopMode,
}

fn default_fps() -> f32 {
    8.0
}

impl AnimationClip {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            frames: Vec::new(),
            fps: default_fps(),
            loop_mode: LoopMode::default(),
        }
    }

    pub fn normalize(&mut self) {
        self.name = self.name.trim().to_string();
        self.frames = self
            .frames
            .iter()
            .map(|frame| frame.trim().to_string())
            .filter(|frame| !frame.is_empty())
            .collect();
        if !self.fps.is_finite() {
            self.fps = default_fps();
        }
        self.fps = self.fps.clamp(0.1, 60.0);
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Seconds one pass takes. Zero for a clip with nothing to play.
    pub fn duration(&self) -> f32 {
        if self.frames.is_empty() || self.fps <= 0.0 {
            return 0.0;
        }
        self.frames.len() as f32 / self.fps
    }

    /// Which frame `elapsed` seconds in is showing, and whether a `Once`
    /// clip has run past its end. A looping clip never ends.
    pub fn frame_index(&self, elapsed: f32) -> (usize, bool) {
        if self.frames.is_empty() {
            return (0, true);
        }
        let fps = self.fps.max(0.1);
        match self.loop_mode {
            LoopMode::Once => {
                let index = (elapsed * fps).floor() as usize;
                if index >= self.frames.len() {
                    (self.frames.len() - 1, true)
                } else {
                    (index, false)
                }
            }
            LoopMode::Loop => {
                let count = self.frames.len();
                let index = (elapsed * fps).floor() as usize % count;
                (index, false)
            }
            LoopMode::PingPong => {
                let count = self.frames.len();
                if count == 1 {
                    return (0, false);
                }
                let period = (count * 2 - 2) as f32 / fps;
                if period <= 0.0 {
                    return (0, false);
                }
                let mut phase = elapsed % period;
                let mut index = (phase * fps).floor() as usize;
                if index >= count {
                    phase = period - phase;
                    index = (phase * fps).floor() as usize;
                }
                (index.min(count - 1), false)
            }
        }
    }

    /// The frame showing at `elapsed` seconds, by asset path.
    pub fn frame_at(&self, elapsed: f32) -> Option<&str> {
        if self.frames.is_empty() {
            return None;
        }
        let (index, _) = self.frame_index(elapsed);
        self.frames.get(index).map(String::as_str)
    }
}

/// One named state of the animation player: which clip it plays, how fast,
/// and which state comes next when a `Once` clip ends. Empty `next` stops
/// instead. Transitions on anything else - a variable, a collision - are
/// blocks: `if score > 10, play clip "run"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationState {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub clip: String,
    #[serde(default = "default_speed")]
    pub speed: f32,
    #[serde(default)]
    pub next: String,
}

fn default_speed() -> f32 {
    1.0
}

impl AnimationState {
    pub fn normalize(&mut self) {
        self.name = self.name.trim().to_string();
        self.clip = self.clip.trim().to_string();
        self.next = self.next.trim().to_string();
        if !self.speed.is_finite() {
            self.speed = default_speed();
        }
        self.speed = self.speed.clamp(0.0, 8.0);
    }
}

/// What the `Animation` component authors: the clips and the states over
/// them. Empty by default, so an old project loads with nothing to play.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AnimationSpec {
    #[serde(default)]
    pub clips: Vec<AnimationClip>,
    #[serde(default)]
    pub states: Vec<AnimationState>,
}

impl AnimationSpec {
    pub fn find_clip(&self, name: &str) -> Option<&AnimationClip> {
        let wanted = name.trim();
        if wanted.is_empty() {
            return None;
        }
        self.clips
            .iter()
            .find(|clip| clip.name.eq_ignore_ascii_case(wanted))
    }

    pub fn find_state(&self, name: &str) -> Option<&AnimationState> {
        let wanted = name.trim();
        if wanted.is_empty() {
            return None;
        }
        self.states
            .iter()
            .find(|state| state.name.eq_ignore_ascii_case(wanted))
    }

    pub fn normalize(&mut self) {
        for clip in &mut self.clips {
            clip.normalize();
        }
        for state in &mut self.states {
            state.normalize();
        }
        self.clips.retain(|clip| !clip.name.is_empty());
        self.states.retain(|state| !state.name.is_empty());
    }

    /// An unused clip name based on `base`, never colliding with another.
    pub fn unique_clip_name(&self, base: &str) -> String {
        let base = base.trim();
        let base = if base.is_empty() { "Clip" } else { base };
        if self.find_clip(base).is_none() {
            return base.to_string();
        }
        (2..)
            .map(|n| format!("{base} {n}"))
            .find(|candidate| self.find_clip(candidate).is_none())
            .expect("an unused name always exists")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easings_start_stop_and_stay_inside() {
        for easing in TweenEasing::ALL {
            assert_eq!(easing.apply(0.0), 0.0, "{easing:?}");
            assert!((easing.apply(1.0) - 1.0).abs() < 1e-5, "{easing:?}");
            for step in 0..=20 {
                let value = easing.apply(step as f32 / 20.0);
                // Bounce and elastic overshoot by design; the rest stay close.
                let bound = match easing {
                    TweenEasing::Bounce => 0.15,
                    TweenEasing::Elastic => 0.4,
                    _ => 0.001,
                };
                assert!(
                    (-bound..=1.0 + bound).contains(&value),
                    "{easing:?} at {step}/20 is {value}"
                );
            }
        }
        // Quad easings are exact halves at the midpoint.
        assert_eq!(TweenEasing::EaseIn.apply(0.5), 0.25);
        assert_eq!(TweenEasing::EaseOut.apply(0.5), 0.75);
        assert_eq!(TweenEasing::EaseInOut.apply(0.5), 0.5);
    }

    #[test]
    fn easing_names_round_trip() {
        for easing in TweenEasing::ALL {
            assert_eq!(TweenEasing::parse(easing.name()), Some(*easing));
        }
        assert_eq!(TweenEasing::parse("ease-in"), Some(TweenEasing::EaseIn));
        assert_eq!(TweenEasing::parse("nope"), None);
        // Serde keeps old documents reading: a missing easing is linear.
        let easing: TweenEasing = serde_json::from_str("null").unwrap_or_default();
        assert_eq!(easing, TweenEasing::Linear);
    }

    #[test]
    fn a_once_clip_ends_and_a_loop_never_does() {
        let clip = AnimationClip {
            name: "Walk".to_string(),
            frames: vec!["a".to_string(), "b".to_string()],
            fps: 2.0,
            loop_mode: LoopMode::Once,
        };
        assert_eq!(clip.duration(), 1.0);
        assert_eq!(clip.frame_index(0.0), (0, false));
        assert_eq!(clip.frame_index(0.6), (1, false));
        assert_eq!(clip.frame_index(5.0), (1, true));
        let looping = AnimationClip {
            loop_mode: LoopMode::Loop,
            ..clip.clone()
        };
        assert_eq!(looping.frame_index(1.0), (0, false));
        assert_eq!(looping.frame_at(0.6), Some("b"));
        let ping = AnimationClip {
            loop_mode: LoopMode::PingPong,
            frames: vec!["a".to_string(), "b".to_string(), "c".to_string()],
            ..clip.clone()
        };
        assert_eq!(ping.frame_index(0.0).0, 0);
        assert_eq!(ping.frame_index(1.0).0, 2);
        assert_eq!(ping.frame_index(1.5).0, 1);
    }

    #[test]
    fn clips_normalize_and_answer_by_name() {
        let mut spec = AnimationSpec {
            clips: vec![AnimationClip {
                name: "  Walk ".to_string(),
                frames: vec!["a".to_string(), String::new(), "b".to_string()],
                fps: f32::NAN,
                loop_mode: LoopMode::Loop,
            }],
            states: vec![AnimationState {
                name: "Moving".to_string(),
                clip: " walk ".to_string(),
                speed: f32::INFINITY,
                next: String::new(),
            }],
        };
        spec.normalize();
        assert_eq!(spec.clips[0].frames.len(), 2);
        assert!(spec.find_clip("walk").is_some());
        assert!(spec.find_state("moving").is_some());
        assert_eq!(spec.unique_clip_name("Walk"), "Walk 2");
    }
}
