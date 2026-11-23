//! Tick-exact snapshots for the playable corpus (`scripts/corpus.py`).
//!
//! With `BLOCKLOOM_RECORD_TICKS=N` and `BLOCKLOOM_RECORD_OUT=<file>` set, the
//! world counts the fixed ticks of its first run and, after tick N, writes
//! every actor's settled pose and the global variables as JSON and quits.
//! The fixed tick makes N the same moment however fast frames arrive.

use crate::engine::{ActorId, Engine, PhysicsPose};
use bevy::prelude::*;
use serde_json::json;

#[derive(Resource)]
pub struct Recorder {
    ticks: u64,
    out: std::path::PathBuf,
    seen: u64,
    done: bool,
}

impl Recorder {
    /// The recorder the environment asks for, if any.
    pub fn from_env() -> Option<Self> {
        let ticks = std::env::var("BLOCKLOOM_RECORD_TICKS").ok()?.parse().ok()?;
        let out = std::env::var_os("BLOCKLOOM_RECORD_OUT")?.into();
        Some(Self {
            ticks,
            out,
            seen: 0,
            done: false,
        })
    }
}

/// Runs last in `FixedPostUpdate`, after poses are recorded.
pub fn record(
    engine: NonSend<Engine>,
    recorder: Option<ResMut<Recorder>>,
    actors: Query<(&ActorId, &PhysicsPose)>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(mut recorder) = recorder else { return };
    if recorder.done || !engine.running || engine.paused {
        return;
    }
    recorder.seen += 1;
    if recorder.seen < recorder.ticks {
        return;
    }
    recorder.done = true;
    let mut poses: Vec<_> = actors
        .iter()
        .map(|(id, pose)| {
            let (rx, ry, rz) = pose.0.rotation.to_euler(EulerRot::XYZ);
            json!({
                "id": id.0,
                "position": pose.0.translation.to_array(),
                "rotation": [rx.to_degrees(), ry.to_degrees(), rz.to_degrees()],
                "scale": pose.0.scale.to_array(),
            })
        })
        .collect();
    poses.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    let globals: serde_json::Map<_, _> = engine
        .variables
        .snapshot()
        .globals
        .iter()
        .map(|(name, value)| {
            (
                name.clone(),
                serde_json::to_value(value).unwrap_or_default(),
            )
        })
        .collect();
    let snapshot = json!({ "ticks": recorder.seen, "actors": poses, "globals": globals });
    let written = serde_json::to_vec_pretty(&snapshot)
        .map_err(|e| e.to_string())
        .and_then(|bytes| std::fs::write(&recorder.out, bytes).map_err(|e| e.to_string()));
    match written {
        Ok(()) => {
            exit.write(AppExit::Success);
        }
        Err(e) => {
            error!("could not write {}: {e}", recorder.out.display());
            exit.write(AppExit::error());
        }
    }
}
