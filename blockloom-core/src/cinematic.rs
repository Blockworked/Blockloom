//! Cutscenes: camera direction on the wall clock. A cutscene is a reel of
//! shots - each a camera actor, a length and an optional dolly path with a
//! look-at target and roll - plus signal markers that fire block strands,
//! slow-motion keys and volume-weight keys. The player steps on the real
//! clock ahead of the sensor publish, so a cutscene plays through `pause
//! game` the way interface strands do, and reporters read what the frame
//! reads.
//!
//! Everything here is plain arithmetic over seconds, so the runtime owns the
//! only clock there is.

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

fn finite3(value: [f32; 3], fallback: [f32; 3]) -> [f32; 3] {
    [
        finite(value[0], fallback[0]),
        finite(value[1], fallback[1]),
        finite(value[2], fallback[2]),
    ]
}

/// Smootherstep: eases between path keys without the corner a plain lerp
/// would cut, so a dolly arrives instead of stopping.
fn ease(u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    u * u * u * (u * (u * 6.0 - 15.0) + 10.0)
}

/// One dolly key: where the camera stands at `at` seconds into its shot,
/// where it looks, how far it rolls and an optional FOV.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PathKey {
    pub at: f32,
    pub pos: [f32; 3],
    pub look: [f32; 3],
    pub roll: f32,
    pub fov: Option<f32>,
}

impl Default for PathKey {
    fn default() -> Self {
        Self {
            at: 0.0,
            pos: [0.0; 3],
            look: [0.0; 3],
            roll: 0.0,
            fov: None,
        }
    }
}

impl PathKey {
    fn normalize(&mut self) {
        self.at = finite(self.at, 0.0).max(0.0);
        self.pos = finite3(self.pos, [0.0; 3]);
        self.look = finite3(self.look, [0.0; 3]);
        self.roll = finite(self.roll, 0.0).clamp(-180.0, 180.0);
        if let Some(fov) = self.fov {
            self.fov = Some(finite(fov, 75.0).clamp(30.0, 110.0));
        }
    }
}

/// One shot: a camera actor's pose, an optional FOV, a length in seconds and
/// an optional dolly path through it. An empty path holds the actor's pose
/// for the shot - the plain camera cut.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shot {
    pub camera: String,
    pub fov: Option<f32>,
    pub dur: f32,
    pub path: Vec<PathKey>,
}

impl Default for Shot {
    fn default() -> Self {
        Self {
            camera: String::new(),
            fov: None,
            dur: 3.0,
            path: Vec::new(),
        }
    }
}

impl Shot {
    fn normalize(&mut self) {
        self.camera = self.camera.trim().to_string();
        self.dur = finite(self.dur, 3.0).clamp(0.1, 3600.0);
        if let Some(fov) = self.fov {
            self.fov = Some(finite(fov, 75.0).clamp(30.0, 110.0));
        }
        for key in &mut self.path {
            key.normalize();
        }
        self.path.sort_by(|a, b| a.at.total_cmp(&b.at));
        self.path.dedup_by(|a, b| (a.at - b.at).abs() < 1e-6);
        if self.path.len() > MAX_KEYS {
            self.path.truncate(MAX_KEYS);
        }
    }
}

/// A signal marker: fires `when cutscene signal` strands at `at` seconds.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SignalKey {
    pub at: f32,
    pub name: String,
}

impl SignalKey {
    fn normalize(&mut self) {
        self.at = finite(self.at, 0.0).max(0.0);
        self.name = self.name.trim().to_string();
        if self.name.len() > 64 {
            self.name.truncate(64);
        }
    }
}

/// A slow-motion key: eases the world's time scale to `scale` at `at`
/// seconds. Only while no `set time scale` block has claimed the run.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SlowKey {
    pub at: f32,
    pub scale: f32,
}

impl SlowKey {
    fn normalize(&mut self) {
        self.at = finite(self.at, 0.0).max(0.0);
        self.scale = finite(self.scale, 1.0).clamp(0.0, 2.0);
    }
}

/// A volume-weight key: sets the named volume's weight at `at` seconds.
/// Restored when the cutscene ends, so a timeline borrows the grade rather
/// than keeping it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VolumeKey {
    pub at: f32,
    pub volume: String,
    pub weight: f32,
}

impl VolumeKey {
    fn normalize(&mut self) {
        self.at = finite(self.at, 0.0).max(0.0);
        self.volume = self.volume.trim().to_string();
        if self.volume.len() > 64 {
            self.volume.truncate(64);
        }
        self.weight = finite(self.weight, 1.0).clamp(0.0, 1.0);
    }
}

/// One reel: shots back to back, then the end. Signals, slow-motion and
/// volume keys fire as the clock passes them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cutscene {
    pub name: String,
    pub duration: f32,
    pub shots: Vec<Shot>,
    pub signals: Vec<SignalKey>,
    pub slowmo: Vec<SlowKey>,
    pub volumes: Vec<VolumeKey>,
}

impl Default for Cutscene {
    fn default() -> Self {
        Self {
            name: "Cutscene".to_string(),
            duration: 10.0,
            shots: Vec::new(),
            signals: Vec::new(),
            slowmo: Vec::new(),
            volumes: Vec::new(),
        }
    }
}

/// At most this many shots, keys and markers per reel: a timeline is data,
/// not a place to hide a novel.
pub const MAX_SHOTS: usize = 32;
pub const MAX_KEYS: usize = 64;

impl Cutscene {
    pub fn normalize(&mut self) {
        self.name = self.name.trim().to_string();
        if self.name.is_empty() {
            self.name = "Cutscene".to_string();
        }
        if self.name.len() > 64 {
            self.name.truncate(64);
        }
        self.duration = finite(self.duration, 10.0).clamp(0.1, 3600.0);
        for shot in &mut self.shots {
            shot.normalize();
        }
        if self.shots.len() > MAX_SHOTS {
            self.shots.truncate(MAX_SHOTS);
        }
        for key in &mut self.signals {
            key.normalize();
        }
        for key in &mut self.slowmo {
            key.normalize();
        }
        for key in &mut self.volumes {
            key.normalize();
        }
        self.signals.sort_by(|a, b| a.at.total_cmp(&b.at));
        self.slowmo.sort_by(|a, b| a.at.total_cmp(&b.at));
        self.volumes.sort_by(|a, b| a.at.total_cmp(&b.at));
        if self.signals.len() > MAX_KEYS {
            self.signals.truncate(MAX_KEYS);
        }
        if self.slowmo.len() > MAX_KEYS {
            self.slowmo.truncate(MAX_KEYS);
        }
        if self.volumes.len() > MAX_KEYS {
            self.volumes.truncate(MAX_KEYS);
        }
    }

    /// Which shot owns `t` seconds, and how far into it the clock is.
    /// Past the last shot answers the last one, so the reel holds its end
    /// frame until the player calls it done.
    pub fn shot_at(&self, t: f32) -> (usize, f32) {
        let mut start = 0.0;
        for (i, shot) in self.shots.iter().enumerate() {
            if t < start + shot.dur || i + 1 == self.shots.len() {
                return (i, (t - start).max(0.0));
            }
            start += shot.dur;
        }
        (0, 0.0)
    }

    /// The reel's full length: its shots back to back, or the authored
    /// duration when there are no shots to measure.
    pub fn length(&self) -> f32 {
        if self.shots.is_empty() {
            return self.duration;
        }
        self.shots.iter().map(|s| s.dur).sum()
    }
}

/// Where a shot's path is at `t` seconds into the shot: a world position, a
/// world look-at, a roll in degrees and an FOV. An empty path holds the
/// camera actor's own pose. Falls back to the actor when a key looks at its
/// own feet, so a degenerate key holds rather than NaNs.
///
/// Poses cross as arrays: the runtime's glam and core's are different
/// versions of the crate, so the boundary speaks plain numbers.
pub fn sample_path(
    shot: &Shot,
    t: f32,
    actor_pos: [f32; 3],
    actor_rot: [f32; 4],
) -> ([f32; 3], [f32; 4], Option<f32>) {
    let actor_pos = Vec3::from_array(actor_pos);
    let actor_rot = Quat::from_array(actor_rot);
    if shot.path.is_empty() {
        return (actor_pos.to_array(), actor_rot.to_array(), shot.fov);
    }
    let keys = &shot.path;
    let (a, b, u) = if t <= keys[0].at {
        (&keys[0], &keys[0], 0.0)
    } else if t >= keys[keys.len() - 1].at {
        let last = &keys[keys.len() - 1];
        (last, last, 0.0)
    } else {
        let mut i = 0;
        while i + 1 < keys.len() && keys[i + 1].at < t {
            i += 1;
        }
        let (a, b) = (&keys[i], &keys[i + 1]);
        let span = (b.at - a.at).max(1e-6);
        (a, b, ((t - a.at) / span).clamp(0.0, 1.0))
    };
    let u = ease(u);
    let pos = Vec3::from_array(a.pos).lerp(Vec3::from_array(b.pos), u);
    let look = Vec3::from_array(a.look).lerp(Vec3::from_array(b.look), u);
    let roll = (a.roll + (b.roll - a.roll) * u).to_radians();
    let fov = match (a.fov, b.fov) {
        (Some(x), Some(y)) => Some(x + (y - x) * u),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => shot.fov,
    };
    (
        pos.to_array(),
        look_rotation(pos, look, roll, actor_rot).to_array(),
        fov,
    )
}

/// A look-at rotation with roll about the view axis, or the fallback when
/// the key looks at its own feet.
fn look_rotation(pos: Vec3, look: Vec3, roll: f32, fallback: Quat) -> Quat {
    let forward = look - pos;
    if forward.length_squared() < 1e-10 {
        return fallback;
    }
    let forward = forward.normalize();
    let world_up = if forward.y.abs() > 0.999 {
        Vec3::X
    } else {
        Vec3::Y
    };
    // Bevy looks down -Z: back is -forward, right is up cross back, and up
    // completes the right-handed basis.
    let back = -forward;
    let right = world_up.cross(back).normalize();
    let up = back.cross(right);
    let mat = glam::Mat3::from_cols(right, up, back);
    Quat::from_mat3(&mat) * Quat::from_axis_angle(Vec3::NEG_Z, roll)
}

/// A fade color by the name a block spells it. Unknown spellings read as
/// `none`, so a typo fades nothing rather than erroring mid-run.
pub fn normalize_fade(name: &str) -> String {
    match name.trim().to_ascii_lowercase().as_str() {
        "black" => "black".to_string(),
        "white" => "white".to_string(),
        _ => "none".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec4;

    fn shot(dur: f32) -> Shot {
        Shot {
            dur,
            ..Shot::default()
        }
    }

    #[test]
    fn shots_play_back_to_back_and_hold_the_end() {
        let reel = Cutscene {
            shots: vec![shot(2.0), shot(3.0)],
            ..Cutscene::default()
        };
        assert_eq!(reel.shot_at(0.0), (0, 0.0));
        assert_eq!(reel.shot_at(2.0), (1, 0.0));
        assert_eq!(reel.shot_at(4.9), (1, 2.9));
        assert_eq!(reel.shot_at(99.0), (1, 97.0));
        assert!((reel.length() - 5.0).abs() < 1e-6);
    }

    #[test]
    fn an_empty_reel_measures_its_authored_duration() {
        let reel = Cutscene::default();
        assert!((reel.length() - 10.0).abs() < 1e-6);
        assert_eq!(reel.shot_at(4.0), (0, 0.0));
    }

    #[test]
    fn a_path_eases_between_keys_and_holds_its_ends() {
        let reel = Shot {
            path: vec![
                PathKey {
                    at: 0.0,
                    pos: [0.0, 0.0, 0.0],
                    look: [0.0, 0.0, -10.0],
                    ..PathKey::default()
                },
                PathKey {
                    at: 4.0,
                    pos: [8.0, 0.0, 0.0],
                    look: [8.0, 0.0, -10.0],
                    ..PathKey::default()
                },
            ],
            ..Shot::default()
        };
        let id = Quat::IDENTITY.to_array();
        let (start, _, _) = sample_path(&reel, 0.0, [0.0; 3], id);
        assert!(Vec3::from_array(start).length() < 1e-6);
        let (end, _, _) = sample_path(&reel, 9.0, [0.0; 3], id);
        assert!((Vec3::from_array(end) - Vec3::new(8.0, 0.0, 0.0)).length() < 1e-6);
        let (mid, rot, _) = sample_path(&reel, 2.0, [0.0; 3], id);
        assert!((Vec3::from_array(mid) - Vec3::new(4.0, 0.0, 0.0)).length() < 1e-4);
        let rot = Quat::from_array(rot);
        assert!(rot.is_normalized());
        // Halfway along, the camera still faces down -Z.
        let forward = rot * Vec3::NEG_Z;
        assert!((forward - Vec3::NEG_Z).length() < 1e-4);
    }

    #[test]
    fn a_degenerate_key_holds_its_pose_with_the_actor_rotation() {
        let reel = Shot {
            path: vec![PathKey {
                pos: [3.0, 1.0, 2.0],
                look: [3.0, 1.0, 2.0],
                ..PathKey::default()
            }],
            ..Shot::default()
        };
        let actor = Quat::from_rotation_y(1.0);
        let (pos, rot, _) = sample_path(&reel, 1.0, Vec3::X.to_array(), actor.to_array());
        // The key holds its pose; only the uncomputable look direction
        // falls back to the actor, never NaNs.
        assert!((Vec3::from_array(pos) - Vec3::new(3.0, 1.0, 2.0)).length() < 1e-6);
        let rot = Quat::from_array(rot);
        assert!(
            (Vec4::from_array(rot.to_array()) - Vec4::from_array(actor.to_array()))
                .abs()
                .max_element()
                < 1e-6
        );
        assert!(rot.is_finite());
    }

    #[test]
    fn normalize_sorts_clamps_and_truncates() {
        let mut reel = Cutscene {
            name: "  ".to_string(),
            duration: f32::NAN,
            shots: vec![Shot {
                dur: -5.0,
                fov: Some(500.0),
                path: vec![
                    PathKey {
                        at: 9.0,
                        roll: 999.0,
                        ..PathKey::default()
                    },
                    PathKey {
                        at: 9.0,
                        ..PathKey::default()
                    },
                ],
                ..Shot::default()
            }],
            signals: vec![SignalKey {
                at: -2.0,
                name: "x".repeat(100),
            }],
            slowmo: vec![SlowKey {
                at: 1.0,
                scale: 9.0,
            }],
            volumes: vec![VolumeKey {
                at: 1.0,
                weight: -3.0,
                ..VolumeKey::default()
            }],
        };
        reel.normalize();
        assert_eq!(reel.name, "Cutscene");
        assert_eq!(reel.duration, 10.0);
        assert_eq!(reel.shots[0].dur, 0.1);
        assert_eq!(reel.shots[0].fov, Some(110.0));
        assert_eq!(reel.shots[0].path.len(), 1);
        assert_eq!(reel.shots[0].path[0].roll, 180.0);
        assert_eq!(reel.signals[0].at, 0.0);
        assert_eq!(reel.signals[0].name.len(), 64);
        assert_eq!(reel.slowmo[0].scale, 2.0);
        assert_eq!(reel.volumes[0].weight, 0.0);
        let old: Cutscene = serde_json::from_str("{}").unwrap();
        assert_eq!(old, Cutscene::default());
    }

    #[test]
    fn fades_spell_three_colors() {
        assert_eq!(normalize_fade("Black"), "black");
        assert_eq!(normalize_fade("WHITE"), "white");
        assert_eq!(normalize_fade("curtain"), "none");
    }
}
