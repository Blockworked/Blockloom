// Timing helpers for scripts, built on `dt` accumulation rather than threads.
// This file is compiled twice: once as `script::timing` in `blockloom-core`
// (so host tests cover it), and once as text into the `blockloom` crate a
// script links against (so `use blockloom::*` finds it). It touches nothing
// host-side, which is what keeps both halves identical.

/// A one-shot countdown built on `dt` accumulation, so `tick` stays the only
/// clock a script needs. Hold one in a `static` or in script data, feed it
/// the frame's `dt`, and it answers true once when the wait is over.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timer {
    remaining: f32,
}

impl Timer {
    /// Waits `seconds` from now. Zero or negative fires on the first tick.
    pub fn after(seconds: f32) -> Self {
        Self {
            remaining: seconds.max(0.0),
        }
    }

    /// Moves the countdown by `dt`. Answers true once, on the tick the wait
    /// ends; further ticks answer false until `reset`.
    pub fn tick(&mut self, dt: f32) -> bool {
        if self.remaining <= 0.0 {
            return false;
        }
        self.remaining -= dt.max(0.0);
        if self.remaining <= 0.0 {
            self.remaining = 0.0;
            return true;
        }
        false
    }

    /// Whether the wait is over.
    pub fn done(&self) -> bool {
        self.remaining <= 0.0
    }

    /// Starts another wait of `seconds`.
    pub fn reset(&mut self, seconds: f32) {
        self.remaining = seconds.max(0.0);
    }
}

/// A repeating interval, also on `dt`. `tick` answers how many whole
/// intervals elapsed this step (usually 0 or 1), so a slow frame still runs
/// each beat rather than dropping one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Every {
    interval: f32,
    acc: f32,
}

impl Every {
    /// Fires every `interval` seconds. Zero or negative means every tick.
    pub fn new(interval: f32) -> Self {
        Self {
            interval: interval.max(0.0),
            acc: 0.0,
        }
    }

    /// Moves the clock by `dt` and answers how many beats elapsed.
    pub fn tick(&mut self, dt: f32) -> u32 {
        if self.interval <= 0.0 {
            return 1;
        }
        self.acc += dt.max(0.0);
        let mut beats = 0;
        while self.acc >= self.interval {
            self.acc -= self.interval;
            beats += 1;
        }
        beats
    }
}

/// A cooldown gate: `trigger` starts the wait, `tick` counts it down, and
/// `ready` tells whether another trigger may fire. What a jump or a shot
/// wants, without touching the host.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cooldown {
    duration: f32,
    remaining: f32,
}

impl Cooldown {
    /// A gate that starts ready and, once triggered, waits `seconds`.
    pub fn new(seconds: f32) -> Self {
        Self {
            duration: seconds.max(0.0),
            remaining: 0.0,
        }
    }

    /// Whether a trigger may fire now.
    pub fn ready(&self) -> bool {
        self.remaining <= 0.0
    }

    /// Starts the wait. Does nothing while one is already running, so a held
    /// button doesn't restart the clock every tick.
    pub fn trigger(&mut self) {
        if self.ready() {
            self.remaining = self.duration;
        }
    }

    /// Moves a running wait by `dt`.
    pub fn tick(&mut self, dt: f32) {
        if self.remaining > 0.0 {
            self.remaining = (self.remaining - dt.max(0.0)).max(0.0);
        }
    }
}
