//! Frame-time capture for `scripts/perf.py`.
//!
//! With `BLOCKLOOM_PROFILE_SECONDS=N` and `BLOCKLOOM_PROFILE_OUT=<file>` set,
//! a running world waits out `BLOCKLOOM_PROFILE_WARMUP` seconds (3 by
//! default), then samples every frame for N seconds: wall frame time, the
//! whole-frame GPU span, each render pass, the main world's CPU time with
//! its `Update` spans, and the fixed step's cost. It
//! writes the summary as JSON and quits. Vsync is off unless
//! `BLOCKLOOM_PROFILE_VSYNC=1`, so frame times are the game's, not the display's.

use crate::engine::Engine;
use bevy::diagnostic::DiagnosticsStore;
use bevy::prelude::*;
use bevy::render::renderer::RenderAdapter;
use bevy::window::PrimaryWindow;
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use web_time::Instant;

#[derive(Resource)]
pub struct Profiler {
    warmup: f64,
    seconds: f64,
    /// Off with `BLOCKLOOM_PROFILE_VSYNC=1`.
    uncapped: bool,
    out: std::path::PathBuf,
    /// When the run started, then when sampling did.
    began: Option<Instant>,
    sampling: Option<Instant>,
    last: Option<Instant>,
    frames: Vec<f64>,
    gpu: Vec<f64>,
    fixed: Vec<f64>,
    passes: BTreeMap<String, Vec<f64>>,
    /// Main-world CPU per frame, and its `Update` spans.
    main: Vec<f64>,
    spans: BTreeMap<String, Vec<f64>>,
    done: bool,
}

impl Profiler {
    pub fn from_env() -> Option<Self> {
        let seconds = std::env::var("BLOCKLOOM_PROFILE_SECONDS")
            .ok()?
            .parse()
            .ok()?;
        let out = std::env::var_os("BLOCKLOOM_PROFILE_OUT")?.into();
        let warmup = std::env::var("BLOCKLOOM_PROFILE_WARMUP")
            .ok()
            .and_then(|w| w.parse().ok())
            .unwrap_or(3.0);
        let uncapped = std::env::var("BLOCKLOOM_PROFILE_VSYNC").map_or(true, |v| v == "0");
        Some(Self {
            warmup,
            uncapped,
            seconds,
            out,
            began: None,
            sampling: None,
            last: None,
            frames: Vec::new(),
            gpu: Vec::new(),
            fixed: Vec::new(),
            passes: BTreeMap::new(),
            main: Vec::new(),
            spans: BTreeMap::new(),
            done: false,
        })
    }
}

/// Mean and percentiles of one series, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Stats {
    pub mean: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

pub fn stats(values: &[f64]) -> Option<Stats> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    // Nearest rank.
    let at =
        |p: f64| sorted[((p * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len()) - 1];
    Some(Stats {
        mean: sorted.iter().sum::<f64>() / sorted.len() as f64,
        p50: at(0.50),
        p95: at(0.95),
        p99: at(0.99),
        max: sorted[sorted.len() - 1],
    })
}

/// Runs last in `Update`, once a frame.
#[allow(clippy::too_many_arguments)]
pub fn sample(
    engine: NonSend<Engine>,
    profiler: Option<ResMut<Profiler>>,
    diagnostics: Option<Res<DiagnosticsStore>>,
    sim: Option<Res<crate::performance::SimSplit>>,
    (pace, split): (
        Option<Res<crate::performance::LoopPace>>,
        Option<Res<crate::performance::UpdateSplit>>,
    ),
    adapter: Option<Res<RenderAdapter>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    cameras: Query<&Msaa, With<crate::world::WorldCamera>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(mut p) = profiler else { return };
    if p.done || !engine.running || engine.paused {
        return;
    }
    let now = Instant::now();
    if p.began.is_none() && p.uncapped {
        // Vsync would measure the display, not the game.
        if let Ok(mut window) = windows.single_mut() {
            window.present_mode = bevy::window::PresentMode::AutoNoVsync;
        }
    }
    let began = *p.began.get_or_insert(now);
    let last = p.last.replace(now);
    if now.duration_since(began).as_secs_f64() < p.warmup {
        return;
    }
    let sampling = *p.sampling.get_or_insert(now);
    if let Some(last) = last.filter(|_| now > sampling) {
        p.frames
            .push(now.duration_since(last).as_secs_f64() * 1000.0);
        if let Some(sim) = &sim {
            p.fixed.push(sim.frame_fixed_ms);
        }
        if let Some(pace) = &pace {
            p.main.push(pace.frame_main_ms);
        }
        // Smoothed already, so their means are a fair split, not a series.
        if let Some(split) = &split {
            for (label, ms) in &split.segments {
                p.spans.entry(label.clone()).or_default().push(*ms);
            }
        }
        if let Some(store) = &diagnostics {
            for d in store.iter() {
                let name = d.path().as_str();
                let Some(value) = d.value() else { continue };
                if d.path() == &crate::gpu::FRAME_GPU {
                    p.gpu.push(value);
                } else if name.starts_with("render/")
                    && (name.ends_with("/elapsed_gpu") || name.ends_with("/elapsed_cpu"))
                {
                    p.passes.entry(name.to_string()).or_default().push(value);
                }
            }
        }
    }
    if now.duration_since(sampling).as_secs_f64() < p.seconds {
        return;
    }
    p.done = true;
    let seconds = now.duration_since(sampling).as_secs_f64();
    let passes: BTreeMap<_, _> = p
        .passes
        .iter()
        .filter_map(|(name, values)| Some((name.clone(), stats(values)?.mean)))
        .collect();
    let spans: BTreeMap<_, _> = p
        .spans
        .iter()
        .filter_map(|(name, values)| Some((name.clone(), stats(values)?.mean)))
        .collect();
    let info = adapter.map(|a| a.get_info());
    let resolution = windows
        .single()
        .map(|w| [w.physical_width(), w.physical_height()])
        .ok();
    let summary = json!({
        "seconds": seconds,
        "frames": p.frames.len(),
        "fps": p.frames.len() as f64 / seconds.max(1e-9),
        "frame_ms": stats(&p.frames),
        "gpu_ms": stats(&p.gpu),
        "fixed_ms": stats(&p.fixed),
        "main_ms": stats(&p.main),
        "update_spans_ms": spans,
        "passes_ms": passes,
        "adapter": info.map(|i| json!({
            "name": i.name,
            "backend": format!("{:?}", i.backend),
            "device_type": format!("{:?}", i.device_type),
            "driver": format!("{} {}", i.driver, i.driver_info),
        })),
        "resolution": resolution,
        "mode": format!("{:?}", engine.project.world.mode),
        "vsync": !p.uncapped,
        // What the world cameras ended up with, after presets and post effects.
        "msaa": cameras.iter().map(|m| m.samples()).max(),
    });
    let written = serde_json::to_vec_pretty(&summary)
        .map_err(|e| e.to_string())
        .and_then(|bytes| std::fs::write(&p.out, bytes).map_err(|e| e.to_string()));
    match written {
        Ok(()) => {
            exit.write(AppExit::Success);
        }
        Err(e) => {
            error!("could not write {}: {e}", p.out.display());
            exit.write(AppExit::error());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_use_the_nearest_rank() {
        let values: Vec<f64> = (1..=100).map(f64::from).collect();
        let s = stats(&values).unwrap();
        assert_eq!((s.p50, s.p95, s.p99, s.max), (50.0, 95.0, 99.0, 100.0));
        assert_eq!(s.mean, 50.5);
        assert_eq!(stats(&[7.0]).unwrap().p99, 7.0);
        assert!(stats(&[]).is_none());
    }
}
