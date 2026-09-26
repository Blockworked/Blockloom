//! Tweens, sprite flipbooks and the animation player.
//!
//! Shared by both dimensions; Phase 6's 2D stack builds on it. Pieces:
//!
//! - [`TweenEasing`]: how a tween gets from here to there. `Glide` and the
//!   `tween ...` blocks all run through [`TweenEasing::apply`], so the VM,
//!   compiled logic and scripts agree on what "ease out" means.
//! - [`AnimationClip`]: a flipbook - image files or a sheet range, played at
//!   `fps` with optional per-frame durations, in a [`LoopMode`], with frame
//!   markers. [`AnimationClip::cursor`] counts steps so markers fire once
//!   per frame shown, however long the tick.
//! - [`AnimationState`]: the player's state machine. Transitions fire on the
//!   clip ending, a marker, a trigger or a variable, with a crossfade, and a
//!   state can hand its clip's motion to the actor (root motion).
//! - [`AnimationSpec::rig`] names a 2D skeleton (see [`crate::rig2d`]) the
//!   clips drive instead of swapping frames.

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

/// A run of cells on one sprite sheet: `count` cells from `first`, counted
/// row-major over a `columns` x `rows` grid. A strip is one row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SheetRange {
    #[serde(default)]
    pub image: String,
    #[serde(default = "one")]
    pub columns: u32,
    #[serde(default = "one")]
    pub rows: u32,
    #[serde(default)]
    pub first: u32,
    #[serde(default = "one")]
    pub count: u32,
}

fn one() -> u32 {
    1
}

impl SheetRange {
    pub fn normalize(&mut self) {
        self.image = self.image.trim().to_string();
        self.columns = self.columns.clamp(1, 256);
        self.rows = self.rows.clamp(1, 256);
        let cells = self.columns * self.rows;
        self.first = self.first.min(cells - 1);
        self.count = self.count.clamp(1, cells - self.first);
    }

    /// How many cells actually fit on the sheet from `first`.
    pub fn len(&self) -> usize {
        let cells = self.columns.max(1) * self.rows.max(1);
        (self.count.max(1)).min(cells.saturating_sub(self.first)) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.image.is_empty() || self.len() == 0
    }
}

/// One frame of a clip, as the renderer needs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClipFrame<'a> {
    /// A whole image file.
    File(&'a str),
    /// One cell of a sheet, by its row-major index.
    Cell {
        image: &'a str,
        index: u32,
        columns: u32,
        rows: u32,
    },
}

/// A named point on a clip's frames: `when animation reaches marker` fires
/// each time the frame it sits on starts showing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameMarker {
    #[serde(default)]
    pub frame: usize,
    #[serde(default)]
    pub name: String,
}

/// Where a playing clip stands: `step` counts frames shown since the clip
/// started, so markers fire once per step whatever the tick length was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipCursor {
    pub step: u64,
    pub frame: usize,
    pub done: bool,
}

/// One flipbook: frames played at `fps` in a [`LoopMode`]. Frames come from
/// `sheet` when it names an image, otherwise from `frames`, which are
/// project-relative paths (`assets/sprites/walk-1.png`). `durations` holds
/// each frame's seconds; a missing or non-positive entry means `1 / fps`.
///
/// `rig_animation` names the animation to play on the actor's rig instead;
/// empty means the clip's own name. Without a rig that has it, the frames
/// play, which is the flipbook fallback.
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
    #[serde(default)]
    pub sheet: Option<SheetRange>,
    #[serde(default)]
    pub durations: Vec<f32>,
    #[serde(default)]
    pub markers: Vec<FrameMarker>,
    /// How far one pass carries the actor when its state has root motion
    /// on, in world units.
    #[serde(default)]
    pub motion: [f32; 2],
    #[serde(default)]
    pub rig_animation: String,
}

fn default_fps() -> f32 {
    8.0
}

/// Most steps one advance walks for markers: a huge tick on a tiny clip
/// would otherwise loop for nothing.
const MAX_MARKER_STEPS: u64 = 256;

impl AnimationClip {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            frames: Vec::new(),
            fps: default_fps(),
            loop_mode: LoopMode::default(),
            sheet: None,
            durations: Vec::new(),
            markers: Vec::new(),
            motion: [0.0, 0.0],
            rig_animation: String::new(),
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
        if let Some(sheet) = &mut self.sheet {
            sheet.normalize();
        }
        if self
            .sheet
            .as_ref()
            .is_some_and(|sheet| sheet.image.is_empty())
        {
            self.sheet = None;
        }
        for seconds in &mut self.durations {
            if !seconds.is_finite() || *seconds < 0.0 {
                *seconds = 0.0;
            }
            *seconds = seconds.min(60.0);
        }
        self.markers.retain_mut(|marker| {
            marker.name = marker.name.trim().to_string();
            !marker.name.is_empty()
        });
        for value in &mut self.motion {
            if !value.is_finite() {
                *value = 0.0;
            }
        }
        self.rig_animation = self.rig_animation.trim().to_string();
    }

    /// How many frames the clip shows per pass.
    pub fn frame_count(&self) -> usize {
        match &self.sheet {
            Some(sheet) if !sheet.image.is_empty() => sheet.len(),
            _ => self.frames.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.frame_count() == 0
    }

    /// Frame `index`, from the sheet or the file list.
    pub fn frame(&self, index: usize) -> Option<ClipFrame<'_>> {
        if index >= self.frame_count() {
            return None;
        }
        match &self.sheet {
            Some(sheet) if !sheet.image.is_empty() => Some(ClipFrame::Cell {
                image: &sheet.image,
                index: sheet.first + index as u32,
                columns: sheet.columns.max(1),
                rows: sheet.rows.max(1),
            }),
            _ => self.frames.get(index).map(|path| ClipFrame::File(path)),
        }
    }

    /// Seconds frame `index` holds for.
    pub fn frame_duration(&self, index: usize) -> f32 {
        match self.durations.get(index) {
            Some(seconds) if *seconds > 0.0 => *seconds,
            _ => 1.0 / self.fps.max(0.1),
        }
    }

    /// The frames one cycle shows, in order: there and back for ping-pong.
    fn sequence(&self) -> Vec<usize> {
        let count = self.frame_count();
        let mut order: Vec<usize> = (0..count).collect();
        if self.loop_mode == LoopMode::PingPong && count > 2 {
            order.extend((1..count - 1).rev());
        }
        order
    }

    /// Seconds one pass takes. Zero for a clip with nothing to play.
    pub fn duration(&self) -> f32 {
        (0..self.frame_count())
            .map(|index| self.frame_duration(index))
            .sum()
    }

    /// Where the clip stands `elapsed` seconds in.
    pub fn cursor(&self, elapsed: f32) -> ClipCursor {
        let order = self.sequence();
        if order.is_empty() {
            return ClipCursor {
                step: 0,
                frame: 0,
                done: true,
            };
        }
        let cycle: f32 = order.iter().map(|&i| self.frame_duration(i)).sum();
        let elapsed = elapsed.max(0.0);
        let (cycles, mut rest) = match self.loop_mode {
            LoopMode::Once if elapsed >= cycle => {
                let last = order.len() - 1;
                return ClipCursor {
                    step: last as u64,
                    frame: order[last],
                    done: true,
                };
            }
            LoopMode::Once => (0, elapsed),
            _ if cycle <= 0.0 => (0, 0.0),
            _ => {
                let cycles = (elapsed / cycle).floor();
                (cycles as u64, elapsed - cycles * cycle)
            }
        };
        for (k, &frame) in order.iter().enumerate() {
            let hold = self.frame_duration(frame);
            if rest < hold {
                return ClipCursor {
                    step: cycles * order.len() as u64 + k as u64,
                    frame,
                    done: false,
                };
            }
            rest -= hold;
        }
        // Rounding left a sliver past the last frame: that is the last frame.
        let last = order.len() - 1;
        ClipCursor {
            step: cycles * order.len() as u64 + last as u64,
            frame: order[last],
            done: false,
        }
    }

    /// Which frame `elapsed` seconds in is showing, and whether a `Once`
    /// clip has run past its end. A looping clip never ends.
    pub fn frame_index(&self, elapsed: f32) -> (usize, bool) {
        let cursor = self.cursor(elapsed);
        (cursor.frame, cursor.done)
    }

    /// The frame showing at `elapsed` seconds.
    pub fn frame_at(&self, elapsed: f32) -> Option<ClipFrame<'_>> {
        if self.is_empty() {
            return None;
        }
        self.frame(self.cursor(elapsed).frame)
    }

    /// The markers on every frame that started after step `after` up to and
    /// including `now`, in order. `None` means the clip just started, so the
    /// first frame's markers count too.
    pub fn markers_reached(&self, after: Option<u64>, now: ClipCursor) -> Vec<&str> {
        if self.markers.is_empty() {
            return Vec::new();
        }
        let order = self.sequence();
        if order.is_empty() {
            return Vec::new();
        }
        let start = after.map_or(0, |step| step + 1);
        if start > now.step {
            return Vec::new();
        }
        let start = start.max(now.step.saturating_sub(MAX_MARKER_STEPS - 1));
        let mut out = Vec::new();
        for step in start..=now.step {
            let frame = order[(step % order.len() as u64) as usize];
            for marker in &self.markers {
                if marker.frame == frame {
                    out.push(marker.name.as_str());
                }
            }
        }
        out
    }
}

/// How a variable transition compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Compare {
    Less,
    LessOrEqual,
    #[default]
    Equal,
    NotEqual,
    Greater,
    GreaterOrEqual,
}

impl Compare {
    /// Numbers compare as numbers; anything else compares as text, ignoring
    /// case, where only equal and not-equal mean anything.
    pub fn holds(self, left: &str, right: &str) -> bool {
        let (l, r) = (left.trim(), right.trim());
        if let (Ok(a), Ok(b)) = (l.parse::<f64>(), r.parse::<f64>()) {
            return match self {
                Compare::Less => a < b,
                Compare::LessOrEqual => a <= b,
                Compare::Equal => a == b,
                Compare::NotEqual => a != b,
                Compare::Greater => a > b,
                Compare::GreaterOrEqual => a >= b,
            };
        }
        let same = l.eq_ignore_ascii_case(r);
        match self {
            Compare::Equal => same,
            Compare::NotEqual => !same,
            _ => false,
        }
    }
}

/// What makes a transition fire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum TransitionWhen {
    /// The state's clip ran out (a `Once` clip only).
    Ended,
    /// The clip reached a marker by this name.
    Marker { name: String },
    /// `fire animation trigger` named it this tick.
    Trigger { name: String },
    /// A variable, as the actor reads it, compares true.
    Variable {
        name: String,
        #[serde(default)]
        compare: Compare,
        #[serde(default)]
        value: String,
    },
}

/// A way out of a state. `blend` is the crossfade in seconds; a negative one
/// takes the spec's default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    #[serde(default)]
    pub to: String,
    pub when: TransitionWhen,
    #[serde(default = "default_blend")]
    pub blend: f32,
}

fn default_blend() -> f32 {
    -1.0
}

/// One named state of the animation player: which clip it plays, how fast,
/// and where it goes. `next` is the old shorthand for an `Ended` transition;
/// `transitions` are checked in order on every fixed tick, first match wins.
/// `root_motion` lets the clip move the actor.
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
    #[serde(default)]
    pub transitions: Vec<Transition>,
    #[serde(default)]
    pub root_motion: bool,
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
        self.transitions.retain_mut(|transition| {
            transition.to = transition.to.trim().to_string();
            if !transition.blend.is_finite() {
                transition.blend = default_blend();
            }
            transition.blend = transition.blend.min(10.0);
            match &mut transition.when {
                TransitionWhen::Ended => {}
                TransitionWhen::Marker { name } | TransitionWhen::Trigger { name } => {
                    *name = name.trim().to_string();
                }
                TransitionWhen::Variable { name, value, .. } => {
                    *name = name.trim().to_string();
                    *value = value.trim().to_string();
                }
            }
            !transition.to.is_empty()
        });
    }
}

/// What happened to a state this tick, for [`AnimationSpec::transition_from`].
#[derive(Default)]
pub struct TransitionInputs<'a> {
    pub ended: bool,
    pub markers: &'a [String],
    pub triggers: &'a [String],
}

/// Where a transition lands: the state, and the crossfade into it.
#[derive(Debug, Clone, PartialEq)]
pub struct Taken<'a> {
    pub state: &'a AnimationState,
    pub blend: f32,
}

/// What the `Animation` component authors: the clips, the states over them,
/// and optionally a 2D rig the clips drive. Empty by default, so an old
/// project loads with nothing to play.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AnimationSpec {
    #[serde(default)]
    pub clips: Vec<AnimationClip>,
    #[serde(default)]
    pub states: Vec<AnimationState>,
    /// The state the player starts in when the run does. Empty starts idle.
    #[serde(default)]
    pub initial: String,
    /// Crossfade seconds for a transition that doesn't set its own.
    #[serde(default)]
    pub crossfade: f32,
    /// A skeletal rig file (Spine or DragonBones JSON) under the project's
    /// assets. Empty, or a file that won't load, plays the flipbooks.
    #[serde(default)]
    pub rig: String,
    /// The rig's skin, by name. Empty takes the default skin.
    #[serde(default)]
    pub skin: String,
    /// Tints per rig slot, laid over the slot's own color: `(slot, color)`.
    #[serde(default)]
    pub slot_tints: Vec<SlotTint>,
}

/// A skin tint for one rig slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlotTint {
    #[serde(default)]
    pub slot: String,
    #[serde(default)]
    pub color: String,
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

    /// What `play animation` means by `name`: a state by that name, or else
    /// the first state over a clip by that name. `None` plays the clip bare.
    pub fn state_for(&self, name: &str) -> Option<&AnimationState> {
        self.find_state(name).or_else(|| {
            let wanted = name.trim();
            self.states
                .iter()
                .find(|state| state.clip.eq_ignore_ascii_case(wanted))
        })
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
        self.initial = self.initial.trim().to_string();
        if !self.crossfade.is_finite() {
            self.crossfade = 0.0;
        }
        self.crossfade = self.crossfade.clamp(0.0, 10.0);
        self.rig = self.rig.trim().to_string();
        self.skin = self.skin.trim().to_string();
        self.slot_tints.retain_mut(|tint| {
            tint.slot = tint.slot.trim().to_string();
            tint.color = tint.color.trim().to_string();
            !tint.slot.is_empty()
        });
    }

    /// The transition `state` takes this tick, if any. `variable` answers a
    /// variable's value as text, the way the actor reads it.
    pub fn transition_from<'a>(
        &'a self,
        state: &str,
        inputs: &TransitionInputs<'_>,
        variable: &dyn Fn(&str) -> Option<String>,
    ) -> Option<Taken<'a>> {
        let from = self.find_state(state)?;
        let blend = |own: f32| if own < 0.0 { self.crossfade } else { own };
        for transition in &from.transitions {
            let fires = match &transition.when {
                TransitionWhen::Ended => inputs.ended,
                TransitionWhen::Marker { name } => inputs
                    .markers
                    .iter()
                    .any(|marker| marker.eq_ignore_ascii_case(name)),
                TransitionWhen::Trigger { name } => inputs
                    .triggers
                    .iter()
                    .any(|trigger| trigger.eq_ignore_ascii_case(name)),
                TransitionWhen::Variable {
                    name,
                    compare,
                    value,
                } => variable(name).is_some_and(|current| compare.holds(&current, value)),
            };
            if !fires {
                continue;
            }
            // Re-entering the state you're in only restarts it on an event,
            // or a held variable would pin it to frame one.
            let to = self.find_state(&transition.to)?;
            if to.name.eq_ignore_ascii_case(&from.name)
                && matches!(transition.when, TransitionWhen::Variable { .. })
            {
                continue;
            }
            return Some(Taken {
                state: to,
                blend: blend(transition.blend),
            });
        }
        if inputs.ended && !from.next.is_empty() {
            let to = self.state_for(&from.next)?;
            return Some(Taken {
                state: to,
                blend: self.crossfade,
            });
        }
        None
    }

    /// The tint for rig slot `slot`, if the spec sets one.
    pub fn slot_tint(&self, slot: &str) -> Option<&str> {
        self.slot_tints
            .iter()
            .find(|tint| tint.slot.eq_ignore_ascii_case(slot.trim()))
            .map(|tint| tint.color.as_str())
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

    fn clip(frames: &[&str], fps: f32, loop_mode: LoopMode) -> AnimationClip {
        AnimationClip {
            name: "Walk".to_string(),
            frames: frames.iter().map(|f| f.to_string()).collect(),
            fps,
            loop_mode,
            ..AnimationClip::new("Walk")
        }
    }

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
        let once = clip(&["a", "b"], 2.0, LoopMode::Once);
        assert_eq!(once.duration(), 1.0);
        assert_eq!(once.frame_index(0.0), (0, false));
        assert_eq!(once.frame_index(0.6), (1, false));
        assert_eq!(once.frame_index(5.0), (1, true));
        let looping = clip(&["a", "b"], 2.0, LoopMode::Loop);
        assert_eq!(looping.frame_index(1.0), (0, false));
        assert_eq!(looping.frame_at(0.6), Some(ClipFrame::File("b")));
        let ping = clip(&["a", "b", "c"], 2.0, LoopMode::PingPong);
        assert_eq!(ping.frame_index(0.0).0, 0);
        assert_eq!(ping.frame_index(1.0).0, 2);
        assert_eq!(ping.frame_index(1.5).0, 1);
        assert_eq!(ping.frame_index(2.0).0, 0);
    }

    #[test]
    fn per_frame_durations_hold_their_frames() {
        let mut held = clip(&["a", "b", "c"], 10.0, LoopMode::Loop);
        // b holds for half a second; a and c take the fps.
        held.durations = vec![0.0, 0.5];
        assert!((held.duration() - 0.7).abs() < 1e-6);
        assert_eq!(held.frame_index(0.05).0, 0);
        assert_eq!(held.frame_index(0.15).0, 1);
        assert_eq!(held.frame_index(0.55).0, 1);
        assert_eq!(held.frame_index(0.65).0, 2);
        assert_eq!(held.frame_index(0.75).0, 0);
        assert_eq!(held.cursor(0.75).step, 3);
    }

    #[test]
    fn sheet_ranges_count_cells_row_major() {
        let mut strip = AnimationClip::new("Run");
        strip.sheet = Some(SheetRange {
            image: "assets/run.png".to_string(),
            columns: 4,
            rows: 2,
            first: 5,
            count: 10,
        });
        strip.normalize();
        // Only three cells are left on the sheet after the fifth.
        assert_eq!(strip.frame_count(), 3);
        assert_eq!(
            strip.frame(1),
            Some(ClipFrame::Cell {
                image: "assets/run.png",
                index: 6,
                columns: 4,
                rows: 2
            })
        );
        assert_eq!(strip.frame(3), None);
    }

    #[test]
    fn markers_fire_once_per_frame_shown_even_across_a_long_tick() {
        let mut steps = clip(&["a", "b", "c"], 10.0, LoopMode::Loop);
        steps.markers = vec![
            FrameMarker {
                frame: 0,
                name: "start".to_string(),
            },
            FrameMarker {
                frame: 2,
                name: "step".to_string(),
            },
        ];
        let start = steps.cursor(0.0);
        assert_eq!(steps.markers_reached(None, start), vec!["start"]);
        // Nothing new shown, nothing fires.
        let same = steps.cursor(0.05);
        assert!(steps.markers_reached(Some(start.step), same).is_empty());
        // One tick that skips from frame 0 of cycle one to frame 1 of cycle
        // two passes c, a and lands on b.
        let far = steps.cursor(0.45);
        assert_eq!(far.step, 4);
        assert_eq!(
            steps.markers_reached(Some(same.step), far),
            vec!["step", "start"]
        );
    }

    #[test]
    fn transitions_fire_on_events_and_variables_in_order() {
        let state = |name: &str, transitions: Vec<Transition>| AnimationState {
            name: name.to_string(),
            clip: name.to_string(),
            speed: 1.0,
            next: String::new(),
            transitions,
            root_motion: false,
        };
        let spec = AnimationSpec {
            states: vec![
                state(
                    "Idle",
                    vec![
                        Transition {
                            to: "Run".to_string(),
                            when: TransitionWhen::Variable {
                                name: "speed".to_string(),
                                compare: Compare::Greater,
                                value: "2".to_string(),
                            },
                            blend: 0.2,
                        },
                        Transition {
                            to: "Jump".to_string(),
                            when: TransitionWhen::Trigger {
                                name: "jump".to_string(),
                            },
                            blend: -1.0,
                        },
                    ],
                ),
                state("Run", Vec::new()),
                AnimationState {
                    next: "Idle".to_string(),
                    ..state("Jump", Vec::new())
                },
            ],
            crossfade: 0.1,
            ..AnimationSpec::default()
        };
        let slow = |_: &str| Some("1".to_string());
        let fast = |_: &str| Some("3".to_string());
        let quiet = TransitionInputs::default();
        assert!(spec.transition_from("Idle", &quiet, &slow).is_none());
        let run = spec.transition_from("Idle", &quiet, &fast).unwrap();
        assert_eq!((run.state.name.as_str(), run.blend), ("Run", 0.2));
        let triggers = vec!["JUMP".to_string()];
        let jumped = TransitionInputs {
            triggers: &triggers,
            ..TransitionInputs::default()
        };
        let jump = spec.transition_from("Idle", &jumped, &slow).unwrap();
        assert_eq!((jump.state.name.as_str(), jump.blend), ("Jump", 0.1));
        // `next` is still the Ended shorthand.
        let ended = TransitionInputs {
            ended: true,
            ..TransitionInputs::default()
        };
        assert_eq!(
            spec.transition_from("Jump", &ended, &slow)
                .unwrap()
                .state
                .name,
            "Idle"
        );
    }

    #[test]
    fn compare_reads_numbers_then_text() {
        assert!(Compare::Greater.holds("10", "9"));
        assert!(!Compare::Greater.holds("apple", "9"));
        assert!(Compare::Equal.holds("Walk", "walk"));
        assert!(Compare::NotEqual.holds("walk", "run"));
    }

    #[test]
    fn old_documents_still_load() {
        let spec: AnimationSpec = serde_json::from_str(
            r#"{"clips":[{"name":"Walk","frames":["a"],"fps":4,"loop_mode":"Loop"}],
                "states":[{"name":"Moving","clip":"Walk","speed":1,"next":""}]}"#,
        )
        .unwrap();
        assert!(spec.clips[0].sheet.is_none());
        assert!(spec.states[0].transitions.is_empty());
        assert!(spec.rig.is_empty());
    }

    #[test]
    fn clips_normalize_and_answer_by_name() {
        let mut spec = AnimationSpec {
            clips: vec![AnimationClip {
                name: "  Walk ".to_string(),
                frames: vec!["a".to_string(), String::new(), "b".to_string()],
                fps: f32::NAN,
                loop_mode: LoopMode::Loop,
                ..AnimationClip::new("")
            }],
            states: vec![AnimationState {
                name: "Moving".to_string(),
                clip: " walk ".to_string(),
                speed: f32::INFINITY,
                next: String::new(),
                transitions: Vec::new(),
                root_motion: false,
            }],
            ..AnimationSpec::default()
        };
        spec.normalize();
        assert_eq!(spec.clips[0].frames.len(), 2);
        assert!(spec.find_clip("walk").is_some());
        assert!(spec.find_state("moving").is_some());
        assert_eq!(spec.state_for("walk").unwrap().name, "Moving");
        assert_eq!(spec.unique_clip_name("Walk"), "Walk 2");
    }
}
