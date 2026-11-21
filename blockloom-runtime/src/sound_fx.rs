//! Per-voice sound effects: a lowpass and a reverb over the decoded stream.
//!
//! Bevy's audio has no DSP hook, so a voice plays an [`FxSource`]: the loaded
//! file plus an `Arc<VoiceFx>` of atomics the main thread writes each frame
//! and the audio thread reads. The decoder smooths what it reads, so a
//! parameter move never clicks. With the lowpass open and the reverb dry the
//! stream passes through untouched.

use bevy::audio::{AudioSource, ChannelCount, Decodable, Sample, SampleRate, Source};
use bevy::prelude::*;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Cutoff at or above this is treated as open.
pub const OPEN_CUTOFF: f32 = 19_000.0;

/// What the main thread asks of one voice.
#[derive(Debug)]
pub struct VoiceFx {
    cutoff: AtomicU32,
    mix: AtomicU32,
    decay: AtomicU32,
    damping: AtomicU32,
    size: AtomicU32,
    excursion: AtomicU32,
    /// A recorded room to convolve with in place of the parametric tail,
    /// and a counter the decoder watches to notice it changed.
    ir: Mutex<Option<Arc<RawIr>>>,
    ir_serial: AtomicU32,
}

fn bits(value: f32) -> AtomicU32 {
    AtomicU32::new(value.to_bits())
}

fn load(cell: &AtomicU32) -> f32 {
    f32::from_bits(cell.load(Ordering::Relaxed))
}

fn store(cell: &AtomicU32, value: f32) {
    cell.store(value.to_bits(), Ordering::Relaxed);
}

impl Default for VoiceFx {
    fn default() -> Self {
        Self {
            cutoff: bits(OPEN_CUTOFF + 1000.0),
            mix: bits(0.0),
            decay: bits(1.5),
            damping: bits(0.5),
            size: bits(0.5),
            excursion: bits(0.2),
            ir: Mutex::new(None),
            ir_serial: AtomicU32::new(0),
        }
    }
}

impl VoiceFx {
    pub fn set_cutoff(&self, hz: f32) {
        store(&self.cutoff, hz.clamp(40.0, 22_000.0));
    }

    /// Points the voice at a recorded room, or back at the parametric one.
    /// Does nothing when it already has this one.
    pub fn set_impulse(&self, ir: Option<&Arc<RawIr>>) {
        let Ok(mut slot) = self.ir.lock() else {
            return;
        };
        let same = match (slot.as_ref(), ir) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            *slot = ir.cloned();
            self.ir_serial.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn set_reverb(&self, mix: f32, size: f32, decay: f32, damping: f32, excursion: f32) {
        store(&self.mix, mix.clamp(0.0, 1.0));
        store(&self.size, size.clamp(0.0, 1.0));
        store(&self.decay, decay.clamp(0.1, 20.0));
        store(&self.damping, damping.clamp(0.0, 1.0));
        store(&self.excursion, excursion.clamp(0.0, 1.0));
    }
}

/// A loaded sound file with its own effect parameters, made per voice.
#[derive(Asset, TypePath, Clone)]
pub struct FxSource {
    pub source: AudioSource,
    pub fx: Arc<VoiceFx>,
    /// Restart when the file ends. Looping lives here rather than in Bevy's
    /// `repeat_infinite`, which would replay the processed samples.
    pub looped: bool,
}

impl Decodable for FxSource {
    type Decoder = FxDecoder<<AudioSource as Decodable>::Decoder>;

    fn decoder(&self) -> Self::Decoder {
        let mut decoder = FxDecoder::new(self.source.decoder(), self.fx.clone());
        if self.looped {
            let source = self.source.clone();
            decoder.restart = Some(Box::new(move || source.decoder()));
        }
        decoder
    }
}

/// A decoded impulse response, as the file had it.
#[derive(Debug)]
pub struct RawIr {
    pub rate: u32,
    /// One vector per channel.
    pub channels: Vec<Vec<f32>>,
}

/// The longest tail convolved; the rest of a longer file is dropped.
pub const MAX_IR_SECONDS: f32 = 2.0;
/// Samples per convolution block.
const PART: usize = 1024;

impl RawIr {
    /// Decodes a WAV (16/24/32-bit int or float) and trims it to
    /// [`MAX_IR_SECONDS`].
    pub fn from_wav(bytes: &[u8]) -> Result<RawIr, String> {
        let mut reader = hound::WavReader::new(std::io::Cursor::new(bytes))
            .map_err(|e| format!("not a WAV file: {e}"))?;
        let spec = reader.spec();
        if !(1..=2).contains(&spec.channels) || !(8_000..=192_000).contains(&spec.sample_rate) {
            return Err("expected a mono or stereo WAV at 8-192 kHz".into());
        }
        let channels = spec.channels as usize;
        let limit = (MAX_IR_SECONDS * spec.sample_rate as f32) as usize * channels;
        let mut out = vec![Vec::new(); channels];
        let mut push = |index: usize, sample: f32| {
            out[index % channels].push(if sample.is_finite() {
                sample.clamp(-1.0, 1.0)
            } else {
                0.0
            });
        };
        match spec.sample_format {
            hound::SampleFormat::Float => {
                for (index, sample) in reader.samples::<f32>().take(limit).enumerate() {
                    push(index, sample.map_err(|e| e.to_string())?);
                }
            }
            hound::SampleFormat::Int => {
                let scale = 2.0f32.powi(1 - i32::from(spec.bits_per_sample));
                for (index, sample) in reader.samples::<i32>().take(limit).enumerate() {
                    push(index, sample.map_err(|e| e.to_string())? as f32 * scale);
                }
            }
        }
        if out.iter().any(Vec::is_empty) {
            return Err("the file holds no audio".into());
        }
        Ok(RawIr {
            rate: spec.sample_rate,
            channels: out,
        })
    }
}

type C = (f32, f32);

/// In-place radix-2 FFT. `inverse` also scales by 1/n.
fn fft(buf: &mut [C], inverse: bool) {
    let n = buf.len();
    debug_assert!(n.is_power_of_two());
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            buf.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = TAU / len as f32 * if inverse { 1.0 } else { -1.0 };
        let (wr, wi) = (angle.cos(), angle.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (ar, ai) = buf[start + k];
                let (br, bi) = buf[start + k + len / 2];
                let (tr, ti) = (br * cr - bi * ci, br * ci + bi * cr);
                buf[start + k] = (ar + tr, ai + ti);
                buf[start + k + len / 2] = (ar - tr, ai - ti);
                let next = (cr * wr - ci * wi, cr * wi + ci * wr);
                cr = next.0;
                ci = next.1;
            }
        }
        len <<= 1;
    }
    if inverse {
        let k = 1.0 / n as f32;
        for v in buf.iter_mut() {
            v.0 *= k;
            v.1 *= k;
        }
    }
}

/// One channel's uniform-partitioned overlap-save convolver. Its output
/// trails the input by one block.
struct Convolver {
    /// The impulse's partitions, already transformed.
    parts: Arc<Vec<Vec<C>>>,
    /// The last `parts.len()` input spectra, newest at `head`.
    history: Vec<Vec<C>>,
    head: usize,
    input: Vec<f32>,
    previous: Vec<f32>,
    output: Vec<f32>,
    spectrum_sum: Vec<C>,
    at: usize,
}

impl Convolver {
    fn new(parts: Arc<Vec<Vec<C>>>) -> Self {
        let n = parts.len();
        Self {
            history: vec![vec![(0.0, 0.0); 2 * PART]; n],
            head: 0,
            input: vec![0.0; PART],
            previous: vec![0.0; PART],
            output: vec![0.0; PART],
            spectrum_sum: vec![(0.0, 0.0); 2 * PART],
            parts,
            at: 0,
        }
    }

    fn tick(&mut self, x: f32) -> f32 {
        let y = self.output[self.at];
        self.input[self.at] = x;
        self.at += 1;
        if self.at == PART {
            self.at = 0;
            self.block();
        }
        y
    }

    fn block(&mut self) {
        let n = self.parts.len();
        self.head = (self.head + n - 1) % n;
        let spectrum = &mut self.history[self.head];
        for (bin, sample) in spectrum
            .iter_mut()
            .zip(self.previous.iter().chain(&self.input))
        {
            *bin = (*sample, 0.0);
        }
        fft(spectrum, false);
        self.spectrum_sum.fill((0.0, 0.0));
        for (k, part) in self.parts.iter().enumerate() {
            let past = &self.history[(self.head + k) % n];
            for ((a, x), h) in self.spectrum_sum.iter_mut().zip(past).zip(part) {
                a.0 += x.0 * h.0 - x.1 * h.1;
                a.1 += x.0 * h.1 + x.1 * h.0;
            }
        }
        fft(&mut self.spectrum_sum, true);
        for (o, v) in self.output.iter_mut().zip(&self.spectrum_sum[PART..]) {
            *o = v.0;
        }
        self.previous.copy_from_slice(&self.input);
    }
}

/// Splits one channel of an impulse response, resampled to `rate`, into
/// transformed partitions.
fn partition(raw: &RawIr, channel: usize, rate: f32) -> Arc<Vec<Vec<C>>> {
    let src = &raw.channels[channel % raw.channels.len()];
    let ratio = raw.rate as f32 / rate;
    let len = ((src.len() as f32 / ratio) as usize).max(1);
    let at = |i: usize| -> f32 {
        let pos = i as f32 * ratio;
        let a = pos as usize;
        let f = pos - a as f32;
        let x = src.get(a).copied().unwrap_or(0.0);
        let y = src.get(a + 1).copied().unwrap_or(0.0);
        x + (y - x) * f
    };
    let count = len.div_ceil(PART);
    let mut parts = Vec::with_capacity(count);
    for p in 0..count {
        let mut buf: Vec<C> = (0..2 * PART)
            .map(|i| {
                let idx = p * PART + i;
                (if i < PART && idx < len { at(idx) } else { 0.0 }, 0.0)
            })
            .collect();
        fft(&mut buf, false);
        parts.push(buf);
    }
    Arc::new(parts)
}

const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS: [usize; 4] = [556, 441, 341, 225];
const SPREAD: usize = 23;
const MAX_SIZE_SCALE: f32 = 2.0;
const EXCURSION_SAMPLES: f32 = 24.0;
const BLOCK: u64 = 32;

struct Delay {
    buf: Vec<f32>,
    write: usize,
}

impl Delay {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(4)],
            write: 0,
        }
    }

    /// The sample `delay` samples back, interpolated.
    fn read(&self, delay: f32) -> f32 {
        let len = self.buf.len();
        let delay = delay.clamp(1.0, (len - 2) as f32);
        let whole = delay.floor();
        let frac = delay - whole;
        let a = (self.write + len - whole as usize) % len;
        let b = (a + len - 1) % len;
        self.buf[a] * (1.0 - frac) + self.buf[b] * frac
    }

    fn push(&mut self, x: f32) {
        self.buf[self.write] = x;
        self.write = (self.write + 1) % self.buf.len();
    }

    fn clear(&mut self) {
        self.buf.iter_mut().for_each(|s| *s = 0.0);
    }
}

struct Comb {
    delay: Delay,
    base: f32,
    lp: f32,
    feedback: f32,
}

struct Allpass {
    delay: Delay,
}

struct Channel {
    low: f32,
    combs: Vec<Comb>,
    allpass: Vec<Allpass>,
}

impl Channel {
    fn new(index: usize, rate: f32) -> Self {
        let scale = rate / 44_100.0;
        let spread = (index * SPREAD) as f32;
        Self {
            low: 0.0,
            combs: COMBS
                .iter()
                .map(|len| {
                    let base = (*len as f32 + spread) * scale;
                    Comb {
                        delay: Delay::new(
                            (base * MAX_SIZE_SCALE + EXCURSION_SAMPLES * 2.0) as usize + 4,
                        ),
                        base,
                        lp: 0.0,
                        feedback: 0.0,
                    }
                })
                .collect(),
            allpass: ALLPASS
                .iter()
                .map(|len| Allpass {
                    delay: Delay::new(((*len as f32 + spread) * scale) as usize + 4),
                })
                .collect(),
        }
    }

    fn clear(&mut self) {
        self.low = 0.0;
        for comb in &mut self.combs {
            comb.delay.clear();
            comb.lp = 0.0;
        }
        for pass in &mut self.allpass {
            pass.delay.clear();
        }
    }
}

/// The values a block of samples runs at, smoothed towards the targets.
#[derive(Clone, Copy)]
struct Live {
    cutoff: f32,
    mix: f32,
    size: f32,
    decay: f32,
    damping: f32,
    excursion: f32,
}

pub struct FxDecoder<S> {
    inner: S,
    fx: Arc<VoiceFx>,
    rate: f32,
    channels: usize,
    chans: Vec<Channel>,
    live: Live,
    /// Frames processed, for the block clock and the excursion LFO.
    frame: u64,
    pos: usize,
    low_k: f32,
    dirty: bool,
    /// Makes a fresh stream when this one ends, for a looping voice.
    restart: Option<Box<dyn Fn() -> S + Send + Sync>>,
    /// One convolver per channel while a recorded room is in force.
    convolvers: Vec<Convolver>,
    ir_seen: u32,
}

impl<S: Source<Item = Sample>> FxDecoder<S> {
    pub fn new(inner: S, fx: Arc<VoiceFx>) -> Self {
        let rate = inner.sample_rate().get() as f32;
        let channels = inner.channels().get() as usize;
        let live = Live {
            cutoff: load(&fx.cutoff),
            mix: load(&fx.mix),
            size: load(&fx.size),
            decay: load(&fx.decay),
            damping: load(&fx.damping),
            excursion: load(&fx.excursion),
        };
        let mut decoder = Self {
            inner,
            fx,
            rate,
            channels,
            chans: Vec::new(),
            live,
            frame: 0,
            pos: 0,
            low_k: 1.0,
            dirty: false,
            restart: None,
            convolvers: Vec::new(),
            ir_seen: 0,
        };
        decoder.low_k = decoder.coefficient();
        decoder
    }

    fn coefficient(&self) -> f32 {
        1.0 - (-TAU * self.live.cutoff / self.rate).exp()
    }

    fn open(&self) -> bool {
        self.live.cutoff >= OPEN_CUTOFF && self.live.mix <= 0.001
    }

    /// Moves the live values a step towards what the main thread wants.
    fn refresh(&mut self) {
        let step = |now: f32, target: f32, k: f32| now + (target - now) * k;
        let target_cutoff = load(&self.fx.cutoff);
        // Cutoff moves in log space so a sweep sounds even.
        let cutoff = (self.live.cutoff.max(40.0).ln()
            + (target_cutoff.max(40.0).ln() - self.live.cutoff.max(40.0).ln()) * 0.2)
            .exp();
        self.live.cutoff = if (cutoff - target_cutoff).abs() < 1.0 {
            target_cutoff
        } else {
            cutoff
        };
        self.live.mix = step(self.live.mix, load(&self.fx.mix), 0.2);
        self.live.size = step(self.live.size, load(&self.fx.size), 0.05);
        self.live.decay = load(&self.fx.decay);
        self.live.damping = load(&self.fx.damping);
        self.live.excursion = load(&self.fx.excursion);
        self.low_k = self.coefficient();
        if self.open() {
            if !self.convolvers.is_empty() {
                self.convolvers.clear();
                self.ir_seen = 0;
            }
            return;
        }
        if self.chans.is_empty() {
            self.chans = (0..self.channels)
                .map(|i| Channel::new(i, self.rate))
                .collect();
        }
        let serial = self.fx.ir_serial.load(Ordering::Relaxed);
        if serial != self.ir_seen
            && let Ok(slot) = self.fx.ir.try_lock()
        {
            let raw = slot.clone();
            drop(slot);
            self.ir_seen = serial;
            self.convolvers = match raw {
                Some(raw) => (0..self.channels)
                    .map(|c| Convolver::new(partition(&raw, c, self.rate)))
                    .collect(),
                None => Vec::new(),
            };
        }
        if self.live.mix <= 0.001 || !self.convolvers.is_empty() {
            return;
        }
        let scale = 0.4 + 1.6 * self.live.size;
        let coefficient = -3.0 * scale / (self.rate * self.live.decay.max(0.1));
        for chan in &mut self.chans {
            for comb in &mut chan.combs {
                comb.feedback = 10f32.powf(coefficient * comb.base).min(0.985);
            }
        }
    }

    fn process(&mut self, ch: usize, x: f32) -> f32 {
        let mut y = x;
        if self.live.cutoff < OPEN_CUTOFF {
            let chan = &mut self.chans[ch];
            chan.low += self.low_k * (y - chan.low);
            y = chan.low;
        }
        if self.live.mix > 0.001 {
            let wet = match self.convolvers.get_mut(ch) {
                Some(conv) => conv.tick(y),
                None => self.reverb(ch, y),
            };
            y += wet * self.live.mix;
        }
        y
    }

    fn reverb(&mut self, ch: usize, x: f32) -> f32 {
        let scale = 0.4 + 1.6 * self.live.size;
        let lfo = (self.frame as f32 / self.rate * 0.7 * TAU + ch as f32).sin();
        let wobble = lfo * self.live.excursion * EXCURSION_SAMPLES;
        let damping = self.live.damping * 0.8;
        let chan = &mut self.chans[ch];
        let mut sum = 0.0;
        for comb in &mut chan.combs {
            let length = comb.base * scale + wobble;
            let out = comb.delay.read(length);
            comb.lp = out * (1.0 - damping) + comb.lp * damping;
            comb.delay.push(x * 0.15 + comb.lp * comb.feedback);
            sum += out;
        }
        let mut y = sum * 0.35;
        for pass in &mut chan.allpass {
            let buffered = pass.delay.read((pass.delay.buf.len() - 4) as f32);
            let out = buffered - y;
            pass.delay.push(y + buffered * 0.5);
            y = out;
        }
        y
    }
}

impl<S: Source<Item = Sample>> Iterator for FxDecoder<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        let sample = match self.inner.next() {
            Some(sample) => sample,
            None => {
                // A file that decodes to nothing must not spin.
                let fresh = (self.restart.as_ref()?)();
                self.inner = fresh;
                self.pos = 0;
                self.inner.next()?
            }
        };
        if self.pos == 0 {
            let channels = self.inner.channels().get() as usize;
            let rate = self.inner.sample_rate().get() as f32;
            let format_changed = channels != self.channels || rate != self.rate;
            if format_changed {
                self.channels = channels;
                self.rate = rate;
                self.chans.clear();
                self.convolvers.clear();
                self.ir_seen = 0;
            }
            if format_changed || self.frame.is_multiple_of(BLOCK) {
                self.refresh();
            }
        }
        let ch = self.pos;
        self.pos += 1;
        if self.pos >= self.channels {
            self.pos = 0;
            self.frame += 1;
        }
        if self.open() {
            // Leaving bypass starts from silence rather than a stale tail.
            if self.dirty {
                self.chans.iter_mut().for_each(Channel::clear);
                self.dirty = false;
            }
            return Some(sample);
        }
        self.dirty = true;
        Some(self.process(ch, sample))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.restart.is_some() {
            (self.inner.size_hint().0, None)
        } else {
            self.inner.size_hint()
        }
    }
}

impl<S: Source<Item = Sample>> Source for FxDecoder<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }

    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        if self.restart.is_some() {
            None
        } else {
            self.inner.total_duration()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZero;

    /// A mono source over fixed samples.
    struct Samples {
        data: Vec<f32>,
        at: usize,
    }

    impl Iterator for Samples {
        type Item = Sample;
        fn next(&mut self) -> Option<Sample> {
            let s = self.data.get(self.at).copied()?;
            self.at += 1;
            Some(s)
        }
    }

    impl Source for Samples {
        fn current_span_len(&self) -> Option<usize> {
            None
        }
        fn channels(&self) -> ChannelCount {
            NonZero::new(1).unwrap()
        }
        fn sample_rate(&self) -> SampleRate {
            NonZero::new(44_100).unwrap()
        }
        fn total_duration(&self) -> Option<Duration> {
            None
        }
    }

    fn run(data: Vec<f32>, fx: Arc<VoiceFx>) -> Vec<f32> {
        FxDecoder::new(Samples { data, at: 0 }, fx).collect()
    }

    #[test]
    fn open_and_dry_is_untouched() {
        let data: Vec<f32> = (0..2000).map(|i| (i as f32 * 0.05).sin()).collect();
        let out = run(data.clone(), Arc::new(VoiceFx::default()));
        assert_eq!(out, data);
    }

    #[test]
    fn a_low_cutoff_removes_highs() {
        let fx = Arc::new(VoiceFx::default());
        fx.set_cutoff(300.0);
        // Alternating samples are the highest frequency there is.
        let data: Vec<f32> = (0usize..4000)
            .map(|i| if i.is_multiple_of(2) { 1.0 } else { -1.0 })
            .collect();
        let out = run(data, fx);
        let peak = out[2000..].iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!(peak < 0.1, "peak {peak}");
    }

    #[test]
    fn fft_round_trips() {
        let mut buf: Vec<C> = (0..64).map(|i| ((i as f32 * 0.3).sin(), 0.0)).collect();
        let orig = buf.clone();
        fft(&mut buf, false);
        fft(&mut buf, true);
        for (a, b) in buf.iter().zip(&orig) {
            assert!((a.0 - b.0).abs() < 1e-4 && a.1.abs() < 1e-4);
        }
    }

    #[test]
    fn convolving_with_a_two_tap_room_echoes_the_input() {
        // A direct tap and a half-height echo 300 samples later.
        let mut ir = vec![0.0f32; 400];
        ir[0] = 1.0;
        ir[300] = 0.5;
        let raw = RawIr {
            rate: 44_100,
            channels: vec![ir],
        };
        let mut conv = Convolver::new(partition(&raw, 0, 44_100.0));
        let mut out = Vec::new();
        for i in 0..PART * 4 {
            out.push(conv.tick(if i == 10 { 1.0 } else { 0.0 }));
        }
        // One block of latency.
        assert!((out[PART + 10] - 1.0).abs() < 1e-3, "{}", out[PART + 10]);
        assert!((out[PART + 310] - 0.5).abs() < 1e-3, "{}", out[PART + 310]);
        assert!(out[PART + 100].abs() < 1e-3);
    }

    #[test]
    fn a_wav_decodes_to_an_impulse_response() {
        let mut bytes = Vec::new();
        {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: 8000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut w = hound::WavWriter::new(std::io::Cursor::new(&mut bytes), spec).unwrap();
            w.write_sample(16384i16).unwrap();
            w.write_sample(0i16).unwrap();
            w.finalize().unwrap();
        }
        let ir = RawIr::from_wav(&bytes).unwrap();
        assert_eq!(ir.rate, 8000);
        assert!((ir.channels[0][0] - 0.5).abs() < 1e-3);
        assert!(RawIr::from_wav(b"nope").is_err());
    }

    #[test]
    fn reverb_rings_after_an_impulse_and_dies() {
        let fx = Arc::new(VoiceFx::default());
        fx.set_reverb(1.0, 0.5, 0.5, 0.3, 0.0);
        let mut data = vec![0.0; 44_100];
        data[0] = 1.0;
        let out = run(data, fx);
        let tail = |a: usize, b: usize| out[a..b].iter().map(|s| s * s).sum::<f32>();
        assert!(tail(2000, 8000) > 1e-6, "no tail");
        assert!(tail(36_000, 44_100) < tail(2000, 8000) * 0.1, "no decay");
    }
}
