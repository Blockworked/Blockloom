//! Cutscenes: camera direction on the wall clock. A `play cutscene` block
//! starts a reel from the project's `cutscenes`; the player steps it on the
//! real clock ahead of the sensor publish, so it plays through `pause game`
//! the way interface strands do. Shots drive the world camera after the rig
//! and the room confine have had their say, signals fire hat blocks, and
//! slow-motion, volume and shake keys land where their block twins land.
//! Slow motion scales the virtual clock the fixed clock follows, so blocks
//! and physics keep their tick while the wall runs on.

use crate::engine::{ActorId, Dimension, Engine, PendingEffects};
use bevy::prelude::*;
use blockloom_core::cinematic::{normalize_fade, sample_path};
use blockloom_core::vm::{Effect, Event};
use std::collections::HashMap;

pub fn register(app: &mut App) {
    app.init_resource::<CinePlayer>()
        .add_systems(
            FixedUpdate,
            apply_cinematic_effects
                .in_set(crate::world::SimulationSet)
                .after(crate::overlay::apply_ui_effects)
                .before(crate::world::clear_effects),
        )
        .add_systems(Update, step_cutscene.before(crate::world::publish_sensors))
        .add_systems(
            Update,
            drive_cinematic
                .after(crate::tiles::confine_camera)
                .before(crate::edit::apply_view),
        );
}

/// Letterbox bars live here, over the interface root (40) but under the
/// scene veil (50). The fade wash sits just above them.
const BARS_Z: i32 = 45;
const WASH_Z: i32 = 47;
/// How much of the frame each bar covers when fully open.
const BAR_HEIGHT: f32 = 10.0;

#[derive(Component)]
pub(crate) struct CineBars;

#[derive(Component)]
pub(crate) struct CineWash;

/// The reel playing right now, if any.
struct Active {
    name: String,
    clock: f32,
    shot_idx: usize,
    fired_signals: usize,
    fired_slow: usize,
    fired_vol: usize,
    /// What `engine.volume_weight` held before the reel's keys, per actor
    /// id. Restored when the reel ends, so a timeline borrows the grade.
    vol_snapshot: HashMap<String, Option<f32>>,
    /// Which shot a missing-camera error was already sent for.
    missing_for: Option<usize>,
}

/// The wall-clock side of cinematics: the reel, the trauma, the bars and
/// the fade. Cleared on every rebuild, so nothing leaks run to run.
#[derive(Resource, Default)]
pub struct CinePlayer {
    active: Option<Active>,
    pub trauma: f32,
    pub letterbox: bool,
    bar: f32,
    pub fade: String,
    fade_alpha: f32,
    pub hitstop: u32,
    last_speed: f32,
}

impl Default for Active {
    fn default() -> Self {
        Self {
            name: String::new(),
            clock: 0.0,
            shot_idx: 0,
            fired_signals: 0,
            fired_slow: 0,
            fired_vol: 0,
            vol_snapshot: HashMap::new(),
            missing_for: None,
        }
    }
}

impl CinePlayer {
    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Which actors a volume name means: every actor carrying a `Volume` under
/// that name, in id order so a reel replays the same.
fn volume_targets(engine: &Engine, wanted: &str) -> Vec<String> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return Vec::new();
    }
    let mut ids: Vec<String> = engine
        .actor_ids()
        .filter(|id| {
            engine.actor(id).is_some_and(|actor| {
                actor.name.eq_ignore_ascii_case(wanted) && actor.components.volume().is_some()
            })
        })
        .cloned()
        .collect();
    ids.sort();
    ids
}

/// Which actor id a camera name means: the first in id order, like volume
/// targets, so a reel replays the same.
fn camera_target(engine: &Engine, wanted: &str) -> Option<String> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return None;
    }
    let mut ids: Vec<String> = engine
        .actor_ids()
        .filter(|id| {
            engine
                .actor(id)
                .is_some_and(|actor| actor.name.eq_ignore_ascii_case(wanted))
        })
        .cloned()
        .collect();
    ids.sort();
    ids.into_iter().next()
}

fn error(message: String) {
    crate::bridge::send(&blockloom_protocol::RuntimeMessage::Error {
        actor: "Blockloom".into(),
        message,
    });
}

/// `play cutscene` through `fade screen` for the rest of the run. Runs on
/// the fixed tick like every other effect consumer, paused or not, so a
/// pause menu can still cut the camera.
fn apply_cinematic_effects(
    effects: Res<PendingEffects>,
    mut engine: NonSendMut<Engine>,
    mut player: ResMut<CinePlayer>,
) {
    if !engine.running {
        return;
    }
    for effect in &effects.0 {
        match effect {
            Effect::PlayCutscene { cutscene } => {
                let Some(reel) = engine
                    .project
                    .world
                    .cutscenes
                    .iter()
                    .find(|c| c.name.eq_ignore_ascii_case(cutscene))
                    .cloned()
                else {
                    error(format!("there's no cutscene called \"{cutscene}\""));
                    continue;
                };
                // Cutting to a new reel ends the old one silently: volumes
                // and the time scale restore, but no end strands run for a
                // reel that never reached its end marker.
                if player.active.is_some() {
                    end_reel(&mut engine, &mut player, false);
                }
                engine.cine_name = reel.name.clone();
                engine.cine_time = 0.0;
                engine.cine_scale = None;
                player.active = Some(Active {
                    name: reel.name,
                    ..Active::default()
                });
            }
            Effect::SkipCutscene => {
                if player.active.is_some() {
                    skip_reel(&mut engine, &mut player);
                }
            }
            Effect::CameraShake { amount } if amount.is_finite() => {
                player.trauma = (player.trauma + amount.clamp(0.0, 1.0)).min(1.0);
            }
            Effect::SetTimeScale { scale } if scale.is_finite() => {
                engine.time_scale = Some(scale.clamp(0.0, 2.0));
            }
            Effect::Hitstop { frames } if frames.is_finite() => {
                player.hitstop = frames.clamp(0.0, 600.0).floor() as u32;
            }
            Effect::SetLetterbox { on } if on.is_finite() => {
                player.letterbox = *on != 0.0;
            }
            Effect::FadeScreen { color } => {
                player.fade = normalize_fade(color);
            }
            _ => {}
        }
    }
}

/// Fires every signal the clock hasn't reached, then ends the reel: the
/// skip lands on the end marker rather than cutting away from it.
fn skip_reel(engine: &mut Engine, player: &mut CinePlayer) {
    let Some(active) = player.active.as_mut() else {
        return;
    };
    let Some(reel) = engine
        .project
        .world
        .cutscenes
        .iter()
        .find(|c| c.name == active.name)
        .cloned()
    else {
        end_reel(engine, player, false);
        return;
    };
    active.clock = reel.length();
    while active.fired_signals < reel.signals.len() {
        let signal = reel.signals[active.fired_signals].name.clone();
        active.fired_signals += 1;
        engine.fire(Event::CutsceneSignal { signal });
    }
    end_reel(engine, player, true);
}

/// Ends the reel: end strands run when it got there, volumes and the
/// slow-motion keys restore either way.
fn end_reel(engine: &mut Engine, player: &mut CinePlayer, ended: bool) {
    let Some(active) = player.active.take() else {
        return;
    };
    for (id, was) in active.vol_snapshot {
        match was {
            Some(weight) => {
                engine.volume_weight.insert(id, weight);
            }
            None => {
                engine.volume_weight.remove(&id);
            }
        }
    }
    engine.cine_scale = None;
    engine.cine_name.clear();
    engine.cine_time = 0.0;
    if ended {
        engine.fire(Event::CutsceneEnded {
            cutscene: active.name,
        });
    }
}

/// Steps the reel, the trauma, the bars, the fade and the time scale on the
/// real clock, ahead of the sensor publish. Runs paused and slowed: the
/// cutscene is wall-clock direction, not simulation.
#[allow(clippy::too_many_arguments)]
pub fn step_cutscene(
    mut commands: Commands,
    mut engine: NonSendMut<Engine>,
    mut player: ResMut<CinePlayer>,
    real: Res<Time<Real>>,
    mut virtual_time: ResMut<Time<Virtual>>,
    mut bars: Query<(Entity, &mut Node, &mut BackgroundColor), With<CineBars>>,
    mut washes: Query<
        (Entity, &mut Node, &mut BackgroundColor),
        (With<CineWash>, Without<CineBars>),
    >,
) {
    if !engine.running || engine.rebuild {
        reset_presentation(
            &mut commands,
            &mut engine,
            &mut player,
            &mut virtual_time,
            &mut bars,
            &mut washes,
        );
        return;
    }
    let dt = real.delta_secs().max(0.0);
    // Hitstop freezes world strands for whole render frames while the
    // cutscene clock ticks on.
    let speed = if player.hitstop > 0 {
        player.hitstop -= 1;
        0.0
    } else {
        engine.time_scale.or(engine.cine_scale).unwrap_or(1.0)
    };
    if speed != player.last_speed {
        virtual_time.set_relative_speed(speed);
        player.last_speed = speed;
    }
    // Trauma settles a little over a second after the last kick.
    player.trauma = (player.trauma - dt * 0.9).max(0.0);
    // Bars and fade ease towards their targets at a few per second.
    player.bar =
        (player.bar + dt * 5.0 * if player.letterbox { 1.0 } else { -1.0 }).clamp(0.0, 1.0);
    let fade_target = if player.fade == "none" { 0.0 } else { 1.0 };
    player.fade_alpha =
        (player.fade_alpha + dt * 4.0 * if fade_target > 0.0 { 1.0 } else { -1.0 }).clamp(0.0, 1.0);
    draw_chrome(&mut commands, &player, &mut bars, &mut washes);
    let Some(active) = player.active.as_mut() else {
        return;
    };
    let Some(reel) = engine
        .project
        .world
        .cutscenes
        .iter()
        .find(|c| c.name == active.name)
        .cloned()
    else {
        end_reel(&mut engine, &mut player, false);
        return;
    };
    active.clock += dt;
    let clock = active.clock;
    while active.fired_signals < reel.signals.len()
        && reel.signals[active.fired_signals].at <= clock
    {
        let signal = reel.signals[active.fired_signals].name.clone();
        active.fired_signals += 1;
        engine.fire(Event::CutsceneSignal { signal });
    }
    while active.fired_slow < reel.slowmo.len() && reel.slowmo[active.fired_slow].at <= clock {
        let scale = reel.slowmo[active.fired_slow].scale;
        active.fired_slow += 1;
        // Explicit blocks win over the reel: a `set time scale` anywhere
        // stands, and the reel stops moving the dial.
        if engine.time_scale.is_none() {
            engine.cine_scale = Some(scale);
        }
    }
    while active.fired_vol < reel.volumes.len() && reel.volumes[active.fired_vol].at <= clock {
        let key = reel.volumes[active.fired_vol].clone();
        active.fired_vol += 1;
        for id in volume_targets(&engine, &key.volume) {
            active
                .vol_snapshot
                .entry(id.clone())
                .or_insert_with(|| engine.volume_weight.get(&id).copied());
            engine.volume_weight.insert(id, key.weight);
        }
    }
    let (shot_idx, _) = reel.shot_at(clock);
    active.shot_idx = shot_idx;
    engine.cine_name = active.name.clone();
    engine.cine_time = clock;
    if clock >= reel.length() {
        end_reel(&mut engine, &mut player, true);
    }
}

fn reset_presentation(
    commands: &mut Commands,
    engine: &mut Engine,
    player: &mut CinePlayer,
    virtual_time: &mut ResMut<Time<Virtual>>,
    bars: &mut Query<(Entity, &mut Node, &mut BackgroundColor), With<CineBars>>,
    washes: &mut Query<
        (Entity, &mut Node, &mut BackgroundColor),
        (With<CineWash>, Without<CineBars>),
    >,
) {
    player.reset();
    engine.cine_scale = None;
    engine.cine_name.clear();
    engine.cine_time = 0.0;
    if player.last_speed != 1.0 {
        virtual_time.set_relative_speed(1.0);
        player.last_speed = 1.0;
    }
    for (entity, _, _) in bars.iter() {
        commands.entity(entity).despawn();
    }
    for (entity, _, _) in washes.iter() {
        commands.entity(entity).despawn();
    }
}

/// Draws the letterbox bars and the fade wash over the interface but under
/// the scene veil.
fn draw_chrome(
    commands: &mut Commands,
    player: &CinePlayer,
    bars: &mut Query<(Entity, &mut Node, &mut BackgroundColor), With<CineBars>>,
    washes: &mut Query<
        (Entity, &mut Node, &mut BackgroundColor),
        (With<CineWash>, Without<CineBars>),
    >,
) {
    if player.bar > 0.001 || player.letterbox {
        let height = player.bar * BAR_HEIGHT;
        let mut found = 0;
        for (_, mut node, _) in bars.iter_mut() {
            node.height = Val::Percent(height);
            found += 1;
            if found >= 2 {
                break;
            }
        }
        for _ in found..2 {
            let mut top = bars_needed(found == 0);
            top.0.height = Val::Percent(height);
            commands.spawn((
                Name::new("cine-bars"),
                CineBars,
                top.0,
                top.1,
                GlobalZIndex(BARS_Z),
            ));
        }
        for (entity, _, _) in bars.iter().skip(2) {
            commands.entity(entity).despawn();
        }
    } else {
        for (entity, _, _) in bars.iter() {
            commands.entity(entity).despawn();
        }
    }
    if player.fade_alpha > 0.001 || player.fade != "none" {
        let color = if player.fade == "white" {
            Color::WHITE.with_alpha(player.fade_alpha)
        } else {
            Color::BLACK.with_alpha(player.fade_alpha)
        };
        if let Some((_, mut node, mut wash)) = washes.iter_mut().next() {
            node.width = Val::Percent(100.0);
            node.height = Val::Percent(100.0);
            wash.0 = color;
            for (entity, _, _) in washes.iter().skip(1) {
                commands.entity(entity).despawn();
            }
        } else {
            let node = Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(0.0),
                right: Val::Percent(0.0),
                top: Val::Percent(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..Default::default()
            };
            commands.spawn((
                Name::new("cine-fade"),
                CineWash,
                node,
                BackgroundColor(color),
                GlobalZIndex(WASH_Z),
            ));
        }
    } else {
        for (entity, _, _) in washes.iter() {
            commands.entity(entity).despawn();
        }
    }
}

/// One bar node: the first call makes the top bar, the second the bottom.
fn bars_needed(top: bool) -> (Node, BackgroundColor) {
    let node = Node {
        position_type: PositionType::Absolute,
        left: Val::Percent(0.0),
        right: Val::Percent(0.0),
        width: Val::Percent(100.0),
        height: Val::Percent(0.0),
        top: if top { Val::Percent(0.0) } else { Val::Auto },
        bottom: if top { Val::Auto } else { Val::Percent(0.0) },
        ..Default::default()
    };
    (node, BackgroundColor(Color::BLACK))
}

/// Drives the world camera from the playing reel, then shakes it by the
/// trauma. Runs after the rig and the room confine, so direction wins, and
/// on the real clock, so it holds while paused.
pub fn drive_cinematic(
    engine: NonSend<Engine>,
    dimension: Res<Dimension>,
    editor: Option<Res<crate::edit::SceneEditor>>,
    mut player: ResMut<CinePlayer>,
    real: Res<Time<Real>>,
    mut cameras: Query<(&mut Transform, Option<&mut Projection>), With<crate::world::WorldCamera>>,
    actors: Query<(&ActorId, &Transform), Without<crate::world::WorldCamera>>,
) {
    if !engine.running || editor.is_some_and(|e| crate::edit::editing(&engine, &e)) {
        return;
    }
    let Ok((mut camera, projection)) = cameras.single_mut() else {
        return;
    };
    if let Some(active) = player.active.as_mut()
        && let Some(reel) = engine
            .project
            .world
            .cutscenes
            .iter()
            .find(|c| c.name == active.name)
    {
        let (shot_idx, local) = reel.shot_at(active.clock);
        if let Some(shot) = reel.shots.get(shot_idx) {
            if !drive_shot(
                &engine,
                &dimension,
                &mut camera,
                projection,
                shot,
                local,
                &actors,
            ) {
                // A shot naming nobody holds the last frame and says so
                // once, rather than spamming the run log every frame.
                if active.missing_for != Some(shot_idx) {
                    active.missing_for = Some(shot_idx);
                    error(format!(
                        "shot {} of \"{}\" names no camera actor called \"{}\"",
                        shot_idx + 1,
                        active.name,
                        shot.camera
                    ));
                }
            } else {
                active.missing_for = None;
            }
        }
    }
    // Trauma shakes whatever the camera ended up as, reel or rig.
    let trauma = player.trauma;
    if trauma > 0.001 {
        shake_camera(&mut camera, &dimension, trauma, real.elapsed_secs());
    }
}

fn drive_shot(
    engine: &Engine,
    dimension: &Res<Dimension>,
    camera: &mut Transform,
    projection: Option<Mut<Projection>>,
    shot: &blockloom_core::cinematic::Shot,
    local: f32,
    actors: &Query<(&ActorId, &Transform), Without<crate::world::WorldCamera>>,
) -> bool {
    let actor_pose = camera_target(engine, &shot.camera).and_then(|id| {
        engine
            .entities
            .get(&id)
            .and_then(|entity| actors.get(*entity).ok())
            .map(|(_, transform)| (transform.translation, transform.rotation))
    });
    let Some((actor_pos, actor_rot)) = actor_pose.or(if shot.path.is_empty() {
        None
    } else {
        // A pathed shot needs no actor: the path is absolute.
        Some((camera.translation, camera.rotation))
    }) else {
        return false;
    };
    let (pos, rot, fov) = sample_path(shot, local, actor_pos.to_array(), actor_rot.to_array());
    let pos = Vec3::from_array(pos);
    let rot = Quat::from_array(rot);
    if dimension.0 == blockloom_core::scene::Mode::TwoD {
        camera.translation.x = pos.x;
        camera.translation.y = pos.y;
        return true;
    }
    camera.translation = pos;
    camera.rotation = rot;
    if let (Some(fov), Some(mut projection)) = (fov, projection)
        && let Projection::Perspective(perspective) = projection.as_mut()
    {
        perspective.fov = fov.clamp(30.0, 110.0).to_radians();
    }
    true
}

/// Seeded trauma shake: three noise lanes each for position and rotation,
/// scaled by the square of the trauma so a kick lands hard and settles
/// soft.
fn shake_camera(camera: &mut Transform, dimension: &Res<Dimension>, trauma: f32, wall: f32) {
    use blockloom_core::wind::gradient_noise;
    let kick = trauma * trauma;
    let lane = |i: u32| gradient_noise(0xC1E4, i, wall * 9.0);
    let offset = Vec3::new(lane(0), lane(1), lane(2)) * 0.45 * kick;
    if dimension.0 == blockloom_core::scene::Mode::TwoD {
        camera.translation.x += offset.x;
        camera.translation.y += offset.y;
        return;
    }
    camera.translation += camera.rotation * offset;
    let spin = Vec3::new(lane(3), lane(4), lane(5)) * 0.08 * kick;
    camera.rotation *= Quat::from_rotation_x(spin.x)
        * Quat::from_rotation_y(spin.y)
        * Quat::from_rotation_z(spin.z);
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::cinematic::Cutscene;
    use blockloom_core::scene::Mode;

    fn app() -> App {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut app = App::new();
        let engine = Engine::new(rx, Mode::ThreeD);
        app.insert_non_send(engine)
            .init_resource::<PendingEffects>()
            .init_resource::<CinePlayer>()
            .init_resource::<Time<Real>>()
            .init_resource::<Time<Virtual>>()
            .add_systems(Update, (step_cutscene, apply_cinematic_effects).chain());
        app
    }

    fn reel() -> Cutscene {
        Cutscene {
            name: "Opener".to_string(),
            shots: vec![blockloom_core::cinematic::Shot {
                dur: 1.0,
                ..Default::default()
            }],
            signals: vec![blockloom_core::cinematic::SignalKey {
                at: 0.05,
                name: "beat".to_string(),
            }],
            slowmo: vec![blockloom_core::cinematic::SlowKey {
                at: 0.0,
                scale: 0.5,
            }],
            ..Cutscene::default()
        }
    }

    #[test]
    fn an_unknown_reel_reports_itself_and_plays_nothing() {
        let mut app = app();
        app.world_mut().non_send_mut::<Engine>().running = true;
        // A fresh engine wants a rebuild; the real `rebuild_world` clears
        // it before the player steps.
        app.world_mut().non_send_mut::<Engine>().rebuild = false;
        app.world_mut().resource_mut::<PendingEffects>().0 = vec![Effect::PlayCutscene {
            cutscene: "Missing".to_string(),
        }];
        app.update();
        assert!(app.world().resource::<CinePlayer>().active.is_none());
    }

    #[test]
    fn a_reel_fires_its_keys_then_ends_and_restores() {
        let mut app = app();
        app.world_mut().non_send_mut::<Engine>().running = true;
        // A fresh engine wants a rebuild; the real `rebuild_world` clears
        // it before the player steps.
        app.world_mut().non_send_mut::<Engine>().rebuild = false;
        app.world_mut()
            .non_send_mut::<Engine>()
            .project
            .world
            .cutscenes = vec![reel()];
        app.world_mut().resource_mut::<PendingEffects>().0 = vec![Effect::PlayCutscene {
            cutscene: "opener".to_string(),
        }];
        // Effects land after the step that made them: one update applies,
        // the next samples.
        app.update();
        app.world_mut().resource_mut::<PendingEffects>().0.clear();
        app.update();
        assert!(app.world().resource::<CinePlayer>().active.is_some());
        assert_eq!(app.world().non_send::<Engine>().cine_name, "Opener");
        // The slow key at 0 fired on the first step.
        assert_eq!(app.world().non_send::<Engine>().cine_scale, Some(0.5));
        // Past the signal marker: it fired exactly once.
        app.world_mut()
            .resource_mut::<CinePlayer>()
            .active
            .as_mut()
            .unwrap()
            .clock = 0.06;
        app.update();
        assert_eq!(
            app.world()
                .resource::<CinePlayer>()
                .active
                .as_ref()
                .unwrap()
                .fired_signals,
            1
        );
        // Past the reel: it ended, restored the scale and reads dry.
        app.world_mut()
            .resource_mut::<CinePlayer>()
            .active
            .as_mut()
            .unwrap()
            .clock = 5.0;
        app.update();
        assert!(app.world().resource::<CinePlayer>().active.is_none());
        assert_eq!(app.world().non_send::<Engine>().cine_name, "");
        assert_eq!(app.world().non_send::<Engine>().cine_scale, None);
    }

    #[test]
    fn slow_motion_scales_the_virtual_clock_that_fixed_follows() {
        let mut app = app();
        app.world_mut().non_send_mut::<Engine>().running = true;
        // A fresh engine wants a rebuild; the real `rebuild_world` clears
        // it before the player steps.
        app.world_mut().non_send_mut::<Engine>().rebuild = false;
        app.world_mut().resource_mut::<PendingEffects>().0 =
            vec![Effect::SetTimeScale { scale: 0.5 }];
        app.update();
        app.world_mut().resource_mut::<PendingEffects>().0.clear();
        app.update();
        assert_eq!(
            app.world().resource::<Time<Virtual>>().relative_speed(),
            0.5
        );
        assert_eq!(app.world().non_send::<Engine>().time_scale, Some(0.5));
    }

    #[test]
    fn hitstop_freezes_whole_frames_then_releases() {
        let mut app = app();
        app.world_mut().non_send_mut::<Engine>().running = true;
        // A fresh engine wants a rebuild; the real `rebuild_world` clears
        // it before the player steps.
        app.world_mut().non_send_mut::<Engine>().rebuild = false;
        app.world_mut().resource_mut::<PendingEffects>().0 = vec![Effect::Hitstop { frames: 2.0 }];
        app.update();
        app.world_mut().resource_mut::<PendingEffects>().0.clear();
        app.update();
        assert_eq!(
            app.world().resource::<Time<Virtual>>().relative_speed(),
            0.0
        );
        app.update();
        app.update();
        assert_eq!(
            app.world().resource::<Time<Virtual>>().relative_speed(),
            1.0
        );
    }
}
