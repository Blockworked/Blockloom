//! The animation player and the 2D rigs it drives.
//!
//! Everything that decides what shows runs on the fixed tick
//! ([`apply_animation_effects`], then [`step_animations`]), so a replay
//! lands on the same frames, markers and transitions whichever scheduler
//! ran the blocks. The per-frame half ([`ensure_rigs`], [`draw_rigs`]) only
//! spawns a rig's slot sprites and copies the solved pose onto them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::prelude::*;
use blockloom_core::animation::{
    AnimationClip, AnimationSpec, ClipFrame, LoopMode, TransitionInputs,
};
use blockloom_core::rig2d::{Affine2, Rig, RigAnimation, WorldPose};
use blockloom_core::vm::{Effect, Event};
use blockloom_protocol::RuntimeMessage;

use crate::bridge;
use crate::engine::{ActorId, AnimationFade, AnimationPlayer, Engine, PendingEffects};
use crate::sprites::SpriteDials;
use crate::world::{asset_path, parse_color};

/// Rig files by where they are, parsed once and reread when they change.
#[derive(Resource, Default)]
pub struct RigCache(HashMap<PathBuf, (Option<std::time::SystemTime>, Result<Arc<Rig>, String>)>);

impl RigCache {
    /// The rig at `path`, and whether this call was the one that read it (so
    /// a broken file complains once, not every frame).
    fn load(&mut self, dir: Option<&Path>, path: &str) -> (Result<Arc<Rig>, String>, bool) {
        let file = asset_path(dir, path);
        let modified = std::fs::metadata(&file)
            .and_then(|meta| meta.modified())
            .ok();
        if let Some((stamp, rig)) = self.0.get(&file)
            && *stamp == modified
        {
            return (rig.clone(), false);
        }
        let folder = Path::new(path)
            .parent()
            .map(|parent| parent.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let rig = std::fs::read_to_string(&file)
            .map_err(|error| format!("can't read rig \"{path}\": {error}"))
            .and_then(|text| {
                Rig::parse(&text, &folder).map_err(|error| format!("rig \"{path}\": {error}"))
            })
            .map(Arc::new);
        self.0.insert(file, (modified, rig.clone()));
        (rig, true)
    }
}

/// A loaded rig on an actor: one child sprite per slot, and what blocks
/// changed this run.
#[derive(Component)]
pub struct RigInstance {
    pub rig: Arc<Rig>,
    pub path: String,
    pub skin: String,
    pub parts: Vec<Entity>,
    /// Attachments a block picked, by slot, until the animation keys that
    /// slot again.
    pub attachments: HashMap<usize, String>,
    /// Tints a block laid on, by slot.
    pub tints: HashMap<usize, [f32; 4]>,
    /// IK targets a block set, by constraint, in the actor's own frame.
    pub targets: Vec<(usize, [f32; 2])>,
    /// What the animation showed in each slot last tick.
    sampled: Vec<String>,
    pub pose: WorldPose,
    /// False while a clip the rig has no animation for plays as a flipbook.
    pub active: bool,
}

/// One slot's sprite, a child of the rigged actor.
#[derive(Component)]
pub struct RigPart;

/// The crossfade ghost: the old clip's frame fading out over the new one.
#[derive(Component)]
pub struct FadeGhost(pub Entity);

/// Marks an actor whose initial state has already started this run.
#[derive(Component)]
pub struct AnimationBooted;

/// Depth between a rig's slots: two hundred still sit inside one order step.
const SLOT_DEPTH: f32 = 0.00005;

fn report(actor: &str, message: String) {
    bridge::send(&RuntimeMessage::Error {
        actor: actor.to_string(),
        message,
    });
}

/// The rig animation `clip` stands for: the clip's own `rig_animation`, its
/// name, or the bare name when no clip has it.
fn rig_animation<'a>(rig: &'a Rig, spec: &AnimationSpec, name: &str) -> Option<&'a RigAnimation> {
    match spec.find_clip(name) {
        Some(clip) if !clip.rig_animation.is_empty() => rig.animation(&clip.rig_animation),
        _ => rig.animation(name),
    }
}

fn playable(spec: &AnimationSpec, rig: Option<&Rig>, name: &str) -> Result<(), String> {
    if rig.is_some_and(|rig| rig_animation(rig, spec, name).is_some()) {
        return Ok(());
    }
    match spec.find_clip(name) {
        Some(clip) if clip.is_empty() => Err(format!("animation clip \"{name}\" has no frames")),
        Some(_) => Ok(()),
        None => Err(format!("there's no animation clip called \"{name}\"")),
    }
}

/// What `play animation` means by `name`: a state first, then a clip. The
/// clip, the state, a speed multiple and the root-motion switch.
fn resolve(
    spec: &AnimationSpec,
    rig: Option<&Rig>,
    name: &str,
) -> Result<(String, String, f32, bool), String> {
    let wanted = name.trim();
    if let Some(state) = spec.state_for(wanted) {
        playable(spec, rig, &state.clip)?;
        return Ok((
            state.clip.clone(),
            state.name.clone(),
            state.speed,
            state.root_motion,
        ));
    }
    playable(spec, rig, wanted)?;
    let clip = spec
        .find_clip(wanted)
        .map(|clip| clip.name.clone())
        .unwrap_or_else(|| wanted.to_string());
    Ok((clip, String::new(), 1.0, false))
}

/// Starts `clip` from its first frame, crossfading out of whatever played
/// for `blend` seconds.
fn start(
    player: &mut AnimationPlayer,
    clip: String,
    state: String,
    speed: f32,
    root_motion: bool,
    blend: f32,
) {
    player.fade = (blend > 0.0 && !player.clip.is_empty()).then(|| AnimationFade {
        clip: player.clip.clone(),
        elapsed: player.elapsed,
        speed: player.speed,
        left: blend,
        total: blend,
    });
    player.clip = clip;
    player.state = state;
    player.elapsed = 0.0;
    player.speed = speed.clamp(0.0, 8.0);
    player.playing = true;
    player.ended_fired = false;
    player.last_step = None;
    player.root_motion = root_motion;
}

/// The animation, rig and sprite-dial effects of this tick's blocks.
pub fn apply_animation_effects(
    mut commands: Commands,
    effects: Res<PendingEffects>,
    engine: NonSend<Engine>,
    mut players: Query<&mut AnimationPlayer>,
    mut rigs: Query<&mut RigInstance>,
    mut dials: Query<&mut SpriteDials>,
) {
    if !engine.running || engine.paused {
        return;
    }
    // Players this tick made, so a second effect on the same actor sees the
    // first before the commands land.
    let mut fresh: HashMap<Entity, AnimationPlayer> = HashMap::new();
    let mut fresh_dials: HashMap<Entity, SpriteDials> = HashMap::new();
    for effect in &effects.0 {
        let actor = match effect {
            Effect::PlayAnimation { actor, .. }
            | Effect::StopAnimation { actor }
            | Effect::SetAnimationSpeed { actor, .. }
            | Effect::FireAnimationTrigger { actor, .. }
            | Effect::SetRigSlot { actor, .. }
            | Effect::SetSlotTint { actor, .. }
            | Effect::SetIkTarget { actor, .. }
            | Effect::SetSpriteDial { actor, .. } => actor,
            _ => continue,
        };
        let Some(&entity) = engine.entities.get(actor) else {
            continue;
        };
        match effect {
            Effect::PlayAnimation { clip, speed, .. } => {
                let Some(spec) = engine
                    .actor(actor)
                    .and_then(|a| a.components.animation())
                    .filter(|_| engine.has_component(actor, "Animation"))
                else {
                    report(
                        actor,
                        "this actor has no Animation component to play".to_string(),
                    );
                    continue;
                };
                let rig = rigs.get(entity).ok().map(|instance| instance.rig.clone());
                let (clip, state, multiple, root_motion) = match resolve(spec, rig.as_deref(), clip)
                {
                    Ok(found) => found,
                    Err(message) => {
                        report(actor, message);
                        continue;
                    }
                };
                let blend = spec.crossfade;
                let speed = *speed * multiple;
                if let Some(player) = fresh.get_mut(&entity) {
                    start(player, clip, state, speed, root_motion, blend);
                } else if let Ok(mut player) = players.get_mut(entity) {
                    start(&mut player, clip, state, speed, root_motion, blend);
                } else {
                    let mut player = AnimationPlayer::default();
                    start(&mut player, clip, state, speed, root_motion, 0.0);
                    fresh.insert(entity, player);
                }
            }
            Effect::StopAnimation { .. } => {
                if let Some(player) = fresh.get_mut(&entity) {
                    player.playing = false;
                } else if let Ok(mut player) = players.get_mut(entity) {
                    player.playing = false;
                }
            }
            Effect::SetAnimationSpeed { speed, .. } => {
                let retune = |player: &mut AnimationPlayer| {
                    player.speed = speed.clamp(0.0, 8.0);
                    if player.speed > 0.0 {
                        player.playing = true;
                        player.ended_fired = false;
                    }
                };
                if let Some(player) = fresh.get_mut(&entity) {
                    retune(player);
                } else if let Ok(mut player) = players.get_mut(entity) {
                    retune(&mut player);
                }
            }
            Effect::FireAnimationTrigger { name, .. } => {
                if let Some(player) = fresh.get_mut(&entity) {
                    player.triggers.push(name.clone());
                } else if let Ok(mut player) = players.get_mut(entity) {
                    player.triggers.push(name.clone());
                }
            }
            Effect::SetRigSlot {
                slot, attachment, ..
            } => {
                let Ok(mut instance) = rigs.get_mut(entity) else {
                    report(actor, "this actor has no rig to swap a slot on".to_string());
                    continue;
                };
                let Some(index) = instance.rig.slot_index(slot) else {
                    report(actor, format!("the rig has no slot called \"{slot}\""));
                    continue;
                };
                instance.attachments.insert(index, attachment.clone());
            }
            Effect::SetSlotTint { slot, color, .. } => {
                let Ok(mut instance) = rigs.get_mut(entity) else {
                    report(actor, "this actor has no rig to tint".to_string());
                    continue;
                };
                let Some(index) = instance.rig.slot_index(slot) else {
                    report(actor, format!("the rig has no slot called \"{slot}\""));
                    continue;
                };
                instance
                    .tints
                    .insert(index, parse_color(color.trim()).to_srgba().to_f32_array());
            }
            Effect::SetIkTarget {
                constraint, x, y, ..
            } => {
                let Ok(mut instance) = rigs.get_mut(entity) else {
                    report(actor, "this actor has no rig to point".to_string());
                    continue;
                };
                let Some(index) = instance.rig.ik_index(constraint) else {
                    report(
                        actor,
                        format!("the rig has no IK constraint called \"{constraint}\""),
                    );
                    continue;
                };
                instance.targets.retain(|(which, _)| *which != index);
                instance.targets.push((index, [*x, *y]));
            }
            Effect::SetSpriteDial { dial, value, .. } => {
                if let Some(dials) = fresh_dials.get_mut(&entity) {
                    dials.set(*dial, *value);
                } else if let Ok(mut dials) = dials.get_mut(entity) {
                    dials.set(*dial, *value);
                } else {
                    let mut dials = SpriteDials::default();
                    dials.set(*dial, *value);
                    fresh_dials.insert(entity, dials);
                }
            }
            _ => {}
        }
    }
    for (entity, player) in fresh {
        commands.entity(entity).insert(player);
    }
    for (entity, dials) in fresh_dials {
        commands.entity(entity).insert(dials);
    }
}

/// Where a rig animation stands `elapsed` seconds in, and whether a `Once`
/// pass has run out.
fn rig_time(animation: &RigAnimation, elapsed: f32, mode: LoopMode) -> (f32, bool) {
    let length = animation.duration;
    if length <= 0.0 {
        return (0.0, mode == LoopMode::Once);
    }
    match mode {
        LoopMode::Once => (elapsed.min(length), elapsed >= length),
        LoopMode::Loop => (elapsed.rem_euclid(length), false),
        LoopMode::PingPong => {
            let phase = elapsed.rem_euclid(length * 2.0);
            (
                if phase > length {
                    length * 2.0 - phase
                } else {
                    phase
                },
                false,
            )
        }
    }
}

fn loop_mode_of(spec: &AnimationSpec, clip: &str) -> LoopMode {
    spec.find_clip(clip)
        .map(|clip| clip.loop_mode)
        .unwrap_or(LoopMode::Loop)
}

/// The rectangle cell `index` covers on a `columns` x `rows` sheet, once the
/// sheet has loaded.
pub fn cell_rect(
    images: &Assets<Image>,
    handle: &Handle<Image>,
    index: u32,
    columns: u32,
    rows: u32,
) -> Option<Rect> {
    let size = images.get(handle)?.size_f32();
    let cell = size / Vec2::new(columns.max(1) as f32, rows.max(1) as f32);
    let (x, y) = (
        (index % columns.max(1)) as f32,
        (index / columns.max(1)) as f32,
    );
    Some(Rect::new(
        x * cell.x,
        y * cell.y,
        (x + 1.0) * cell.x,
        (y + 1.0) * cell.y,
    ))
}

/// Puts `frame` on `sprite`. A sheet cell waits for its sheet to load rather
/// than flashing the whole sheet.
fn show_frame(
    sprite: &mut Sprite,
    frame: ClipFrame<'_>,
    dir: Option<&Path>,
    assets: &AssetServer,
    images: &Assets<Image>,
) {
    match frame {
        ClipFrame::File(path) => {
            let handle: Handle<Image> = assets.load(asset_path(dir, path));
            if sprite.image != handle {
                sprite.image = handle;
            }
            if sprite.rect.is_some() {
                sprite.rect = None;
            }
        }
        ClipFrame::Cell {
            image,
            index,
            columns,
            rows,
        } => {
            let handle: Handle<Image> = assets.load(asset_path(dir, image));
            if let Some(rect) = cell_rect(images, &handle, index, columns, rows) {
                if sprite.image != handle {
                    sprite.image = handle;
                }
                if sprite.rect != Some(rect) {
                    sprite.rect = Some(rect);
                }
            }
        }
    }
}

/// How far `clip` moves the actor between two points in its run.
fn clip_travel(clip: &AnimationClip, from: f32, to: f32) -> [f32; 2] {
    let length = clip.duration();
    if length <= 0.0 || to <= from {
        return [0.0, 0.0];
    }
    let (from, to) = if clip.loop_mode == LoopMode::Once {
        (from.min(length), to.min(length))
    } else {
        (from, to)
    };
    let share = (to - from) / length;
    [clip.motion[0] * share, clip.motion[1] * share]
}

/// Advances every animation player on the fixed tick: swaps flipbook
/// frames, fires markers and `when animation ends`, takes state
/// transitions, crossfades, moves root-motion actors, and solves rigs.
/// Frozen while paused or stopped, like the VM.
#[allow(clippy::type_complexity)]
pub fn step_animations(
    mut commands: Commands,
    time: Res<Time>,
    mut engine: NonSendMut<Engine>,
    assets: Res<AssetServer>,
    images: Res<Assets<Image>>,
    mut actors: Query<(
        Entity,
        &ActorId,
        Option<&mut AnimationPlayer>,
        Option<&mut Sprite>,
        Option<&mut RigInstance>,
        Option<&SpriteDials>,
        Option<&FadeGhost>,
        Has<AnimationBooted>,
        &mut Transform,
    )>,
    mut ghosts: Query<&mut Sprite, (With<FadeGhostSprite>, Without<ActorId>)>,
) {
    if !engine.running || engine.paused {
        return;
    }
    let dt = time.delta_secs();
    let dir = engine.project_dir.clone();
    let mut events: Vec<Event> = Vec::new();
    for (entity, id, player, sprite, rig, dials, ghost, booted, mut transform) in &mut actors {
        let actor = id.0.as_str();
        let spec = engine
            .actor(actor)
            .and_then(|a| a.components.animation())
            .filter(|_| engine.has_component(actor, "Animation"))
            .cloned();
        let flip_x = dials.is_some_and(|dials| dials.0.flip_x);
        let Some(mut player) = player else {
            // Start the initial state once per run.
            if let Some(spec) = &spec
                && !booted
                && !spec.initial.is_empty()
            {
                commands.entity(entity).insert(AnimationBooted);
                match resolve(spec, rig.as_ref().map(|r| &*r.rig), &spec.initial) {
                    Ok((clip, state, speed, root_motion)) => {
                        let mut fresh = AnimationPlayer::default();
                        start(&mut fresh, clip, state, speed, root_motion, 0.0);
                        commands.entity(entity).insert(fresh);
                    }
                    Err(message) => report(actor, message),
                }
            }
            // A rig still stands in its setup pose, bent by any IK target.
            if let Some(mut rig) = rig {
                rig.active = true;
                let pose = rig.rig.setup_pose();
                let targets = flipped_targets(&rig.targets, flip_x);
                rig.pose = rig.rig.solve(&pose, &rig.skin, &targets);
            }
            continue;
        };
        let Some(spec) = spec else {
            commands.entity(entity).remove::<AnimationPlayer>();
            continue;
        };
        let before = player.elapsed;
        if player.playing && player.speed > 0.0 {
            player.elapsed += dt * player.speed;
        }
        if let Some(fade) = &mut player.fade {
            fade.elapsed += dt * fade.speed;
            fade.left -= dt;
        }
        if player.fade.as_ref().is_some_and(|fade| fade.left <= 0.0) {
            player.fade = None;
        }
        let clip = spec.find_clip(&player.clip).cloned();
        let rig_file = rig.as_ref().map(|instance| instance.rig.clone());
        let rig_anim = rig_file
            .as_deref()
            .and_then(|file| rig_animation(file, &spec, &player.clip));
        let mut rig = rig;
        let mode = loop_mode_of(&spec, &player.clip);
        let mut markers: Vec<String> = Vec::new();
        let done;
        let mut travel = [0.0, 0.0];
        if let (Some(instance), Some(animation)) = (rig.as_mut(), rig_anim) {
            instance.active = true;
            if let Some(ghost) = ghost {
                commands.entity(ghost.0).despawn();
                commands.entity(entity).remove::<FadeGhost>();
            }
            let (t, ended) = rig_time(animation, player.elapsed, mode);
            let looping = mode != LoopMode::Once;
            let from = if player.last_step.is_none() {
                -1.0
            } else {
                before
            };
            markers.extend(
                animation
                    .events_between(from, player.elapsed, looping)
                    .into_iter()
                    .map(str::to_string),
            );
            player.last_step = Some(0);
            if player.root_motion {
                travel = animation.root_travel(before, player.elapsed, looping);
            }
            let fps = clip.as_ref().map_or(30.0, |clip| clip.fps);
            player.frame = (t * fps).floor() as usize + 1;
            done = ended;
            let mut pose = instance.rig.sample(Some(animation), t, player.root_motion);
            // The animation keying a slot again takes it back from a block.
            let sampled = pose.attachments.clone();
            if instance.sampled.len() == sampled.len() {
                let keyed: Vec<usize> = (0..sampled.len())
                    .filter(|&slot| instance.sampled[slot] != sampled[slot])
                    .collect();
                for slot in keyed {
                    instance.attachments.remove(&slot);
                }
            }
            instance.sampled = sampled;
            if let Some(fade) = &player.fade
                && let Some(old) = rig_animation(&instance.rig, &spec, &fade.clip)
            {
                let (old_t, _) = rig_time(old, fade.elapsed, loop_mode_of(&spec, &fade.clip));
                let from = instance.rig.sample(Some(old), old_t, player.root_motion);
                pose = from.blend(&pose, 1.0 - fade.left / fade.total.max(1e-4));
            }
            for (slot, name) in &instance.attachments {
                if let Some(shown) = pose.attachments.get_mut(*slot) {
                    *shown = name.clone();
                }
            }
            let targets = flipped_targets(&instance.targets, flip_x);
            instance.pose = instance.rig.solve(&pose, &instance.skin, &targets);
        } else if let Some(clip) = clip.as_ref().filter(|clip| !clip.is_empty()) {
            // The flipbook fallback: the rig steps aside for the frames.
            if let Some(instance) = rig.as_mut() {
                instance.active = false;
            }
            let cursor = clip.cursor(player.elapsed);
            markers.extend(
                clip.markers_reached(player.last_step, cursor)
                    .into_iter()
                    .map(str::to_string),
            );
            player.last_step = Some(cursor.step);
            player.frame = cursor.frame + 1;
            done = cursor.done;
            if player.root_motion {
                travel = clip_travel(clip, before, player.elapsed);
            }
            if let Some(mut sprite) = sprite {
                if let Some(frame) = clip.frame(cursor.frame) {
                    show_frame(&mut sprite, frame, dir.as_deref(), &assets, &images);
                }
                fade_ghost(
                    &mut commands,
                    entity,
                    &sprite,
                    ghost,
                    &mut ghosts,
                    &player,
                    &spec,
                    dir.as_deref(),
                    &assets,
                    &images,
                );
            }
        } else {
            // The rig that played this clip went away: nothing left to show.
            commands.entity(entity).remove::<AnimationPlayer>();
            continue;
        }
        // Root motion moves the actor through its own facing and size.
        if travel != [0.0, 0.0] {
            let local = Vec3::new(if flip_x { -travel[0] } else { travel[0] }, travel[1], 0.0);
            let moved = transform.rotation * (local * transform.scale);
            transform.translation += Vec3::new(moved.x, moved.y, 0.0);
        }
        let ended = done && !player.ended_fired;
        for marker in &markers {
            events.push(Event::AnimationMarker {
                actor: actor.to_string(),
                marker: marker.clone(),
            });
        }
        if ended {
            player.ended_fired = true;
            player.playing = false;
            events.push(Event::AnimationEnded {
                actor: actor.to_string(),
                clip: player.clip.clone(),
            });
        }
        // The state machine: the first transition out that fires wins.
        let triggers = std::mem::take(&mut player.triggers);
        if !player.state.is_empty() {
            let inputs = TransitionInputs {
                ended,
                markers: &markers,
                triggers: &triggers,
            };
            let variables = &engine.variables;
            let read = |name: &str| Some(variables.read(actor, name).as_text());
            if let Some(taken) = spec.transition_from(&player.state, &inputs, &read) {
                let state = taken.state.clone();
                if let Err(message) = playable(&spec, rig_file.as_deref(), &state.clip) {
                    report(actor, message);
                } else {
                    start(
                        &mut player,
                        state.clip.clone(),
                        state.name.clone(),
                        state.speed,
                        state.root_motion,
                        taken.blend,
                    );
                }
            }
        }
    }
    for event in events {
        engine.fire(event);
    }
}

/// Marks the ghost sprite a crossfade draws with.
#[derive(Component)]
pub struct FadeGhostSprite;

/// Draws the clip being faded out of over the new one, fading as it goes,
/// and takes the ghost away when the fade is over.
#[allow(clippy::too_many_arguments)]
fn fade_ghost(
    commands: &mut Commands,
    entity: Entity,
    sprite: &Sprite,
    ghost: Option<&FadeGhost>,
    ghosts: &mut Query<&mut Sprite, (With<FadeGhostSprite>, Without<ActorId>)>,
    player: &AnimationPlayer,
    spec: &AnimationSpec,
    dir: Option<&Path>,
    assets: &AssetServer,
    images: &Assets<Image>,
) {
    let fading = player.fade.as_ref().and_then(|fade| {
        let clip = spec.find_clip(&fade.clip)?;
        let frame = clip.frame(clip.cursor(fade.elapsed).frame)?;
        Some((fade, frame))
    });
    let Some((fade, frame)) = fading else {
        if let Some(ghost) = ghost {
            commands.entity(ghost.0).despawn();
            commands.entity(entity).remove::<FadeGhost>();
        }
        return;
    };
    let mut drawn = sprite.clone();
    show_frame(&mut drawn, frame, dir, assets, images);
    let weight = (fade.left / fade.total.max(1e-4)).clamp(0.0, 1.0);
    drawn.color = drawn.color.with_alpha(sprite.color.alpha() * weight);
    match ghost.and_then(|ghost| ghosts.get_mut(ghost.0).ok()) {
        Some(mut existing) => *existing = drawn,
        None => {
            let child = commands
                .spawn((
                    drawn,
                    FadeGhostSprite,
                    Transform::from_xyz(0.0, 0.0, SLOT_DEPTH),
                ))
                .id();
            commands
                .entity(entity)
                .add_child(child)
                .insert(FadeGhost(child));
        }
    }
}

/// IK targets as the rig sees them: a flipped actor mirrors its rig.
fn flipped_targets(targets: &[(usize, [f32; 2])], flip_x: bool) -> Vec<(usize, [f32; 2])> {
    targets
        .iter()
        .map(|(index, [x, y])| (*index, [if flip_x { -x } else { *x }, *y]))
        .collect()
}

/// Loads the rig an actor's Animation component names and hangs a sprite
/// per slot off it; takes them away when the component goes. Runs whether
/// or not the world is playing, so the scene view shows the setup pose.
pub fn ensure_rigs(
    mut commands: Commands,
    engine: NonSend<Engine>,
    mut cache: ResMut<RigCache>,
    actors: Query<(Entity, &ActorId, Option<&RigInstance>)>,
) {
    let dir = engine.project_dir.clone();
    for (entity, id, instance) in &actors {
        let wanted = engine
            .actor(&id.0)
            .and_then(|actor| actor.components.animation())
            .filter(|spec| !spec.rig.is_empty() && engine.has_component(&id.0, "Animation"));
        let stale = match (wanted, instance) {
            (Some(spec), Some(instance)) => instance.path != spec.rig || instance.skin != spec.skin,
            (None, Some(_)) => true,
            _ => false,
        };
        if stale && let Some(instance) = instance {
            for part in &instance.parts {
                commands.entity(*part).despawn();
            }
            commands.entity(entity).remove::<RigInstance>();
            continue;
        }
        let (Some(spec), None) = (wanted, instance) else {
            continue;
        };
        let (loaded, first) = cache.load(dir.as_deref(), &spec.rig);
        let rig = match loaded {
            Ok(rig) => rig,
            Err(message) => {
                if first {
                    report(&id.0, message);
                }
                continue;
            }
        };
        let parts: Vec<Entity> = (0..rig.slots.len())
            .map(|index| {
                let part = commands
                    .spawn((
                        Sprite::default(),
                        RigPart,
                        Transform::from_xyz(0.0, 0.0, index as f32 * SLOT_DEPTH),
                        Visibility::Hidden,
                    ))
                    .id();
                commands.entity(entity).add_child(part);
                part
            })
            .collect();
        let pose = rig.solve(&rig.setup_pose(), &spec.skin, &[]);
        commands.entity(entity).insert(RigInstance {
            sampled: Vec::new(),
            path: spec.rig.clone(),
            skin: spec.skin.clone(),
            parts,
            attachments: HashMap::new(),
            tints: HashMap::new(),
            targets: Vec::new(),
            pose,
            rig,
            active: true,
        });
    }
}

/// Copies each rig's solved pose onto its slot sprites.
pub fn draw_rigs(
    engine: NonSend<Engine>,
    assets: Res<AssetServer>,
    rigs: Query<(&ActorId, &RigInstance, Option<&SpriteDials>)>,
    mut parts: Query<(&mut Sprite, &mut Transform, &mut Visibility), With<RigPart>>,
) {
    let dir = engine.project_dir.clone();
    for (id, instance, dials) in &rigs {
        if !instance.active {
            for part in &instance.parts {
                if let Ok((_, _, mut visibility)) = parts.get_mut(*part)
                    && *visibility != Visibility::Hidden
                {
                    *visibility = Visibility::Hidden;
                }
            }
            continue;
        }
        let flip_x = dials.is_some_and(|dials| dials.0.flip_x);
        let flip_y = dials.is_some_and(|dials| dials.0.flip_y);
        let spec = engine
            .actor(&id.0)
            .and_then(|actor| actor.components.animation());
        for (slot, part) in instance.parts.iter().enumerate() {
            let Ok((mut sprite, mut transform, mut visibility)) = parts.get_mut(*part) else {
                continue;
            };
            let Some(draw) = instance.pose.draws.iter().find(|draw| draw.slot == slot) else {
                if *visibility != Visibility::Hidden {
                    *visibility = Visibility::Hidden;
                }
                continue;
            };
            let mut m = draw.transform;
            if flip_x {
                m = Affine2 {
                    a: -m.a,
                    c: -m.c,
                    tx: -m.tx,
                    ..m
                };
            }
            if flip_y {
                m = Affine2 {
                    b: -m.b,
                    d: -m.d,
                    ty: -m.ty,
                    ..m
                };
            }
            let [sx, sy] = m.scale();
            let next = Transform {
                translation: Vec3::new(m.tx, m.ty, slot as f32 * SLOT_DEPTH),
                rotation: Quat::from_rotation_z(m.rotation().to_radians()),
                scale: Vec3::new(sx, sy, 1.0),
            };
            if *transform != next {
                *transform = next;
            }
            let handle: Handle<Image> = assets.load(asset_path(dir.as_deref(), &draw.image));
            if sprite.image != handle {
                sprite.image = handle;
            }
            let size = (draw.size[0] > 0.0 && draw.size[1] > 0.0)
                .then(|| Vec2::new(draw.size[0], draw.size[1]));
            if sprite.custom_size != size {
                sprite.custom_size = size;
            }
            let mut color = draw.color;
            let name = &instance.rig.slots[slot].name;
            if let Some(tint) = spec.and_then(|spec| spec.slot_tint(name)) {
                let tint = parse_color(tint).to_srgba().to_f32_array();
                color = std::array::from_fn(|i| color[i] * tint[i]);
            }
            if let Some(tint) = instance.tints.get(&slot) {
                color = std::array::from_fn(|i| color[i] * tint[i]);
            }
            let color = Color::srgba(color[0], color[1], color[2], color[3]);
            if sprite.color != color {
                sprite.color = color;
            }
            if *visibility != Visibility::Inherited {
                *visibility = Visibility::Inherited;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::{TimePlugin, TimeUpdateStrategy};
    use blockloom_core::components::ActorComponent;
    use blockloom_core::project::Actor;
    use blockloom_core::scene::{Mode, Visual};

    /// One `update` is one 60 Hz fixed step of the player alone.
    fn player_app(spec: AnimationSpec) -> (App, Entity) {
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::TwoD);
        engine.running = true;
        let mut actor = Actor::new(
            "Hero",
            Visual::Rect {
                color: "#FFFFFF".to_string(),
                size: [10.0, 10.0],
            },
        );
        actor
            .components
            .insert(ActorComponent::Animation { animation: spec });
        let id = actor.id.clone();
        engine
            .attached
            .insert(id.clone(), ["Animation".to_string()].into_iter().collect());
        engine.project.actors.push(actor);
        let mut app = App::new();
        app.add_plugins((
            TaskPoolPlugin::default(),
            TimePlugin,
            bevy::asset::AssetPlugin::default(),
        ));
        app.init_asset::<Image>();
        app.insert_resource(Time::<Fixed>::from_hz(60.0));
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        ));
        app.init_resource::<PendingEffects>();
        let entity = app
            .world_mut()
            .spawn((ActorId(id.clone()), Transform::default(), Sprite::default()))
            .id();
        engine.entities.insert(id, entity);
        app.insert_non_send(engine);
        app.add_systems(
            FixedUpdate,
            (apply_animation_effects, step_animations).chain(),
        );
        (app, entity)
    }

    fn player(app: &App, entity: Entity) -> AnimationPlayer {
        app.world()
            .get::<AnimationPlayer>(entity)
            .cloned()
            .expect("a player")
    }

    #[test]
    fn states_boot_move_the_actor_and_take_trigger_and_end_transitions() {
        let spec: AnimationSpec = serde_json::from_str(
            r#"{
                "initial": "Walking",
                "clips": [
                    {"name":"Walk","frames":["a","b","c","d"],"fps":10,"loop_mode":"Loop","motion":[40,0]},
                    {"name":"Jump","frames":["j1","j2"],"fps":10,"loop_mode":"Once"}
                ],
                "states": [
                    {"name":"Walking","clip":"Walk","root_motion":true,
                     "transitions":[{"to":"Jumping","when":{"kind":"Trigger","name":"jump"}}]},
                    {"name":"Jumping","clip":"Jump","next":"Walking"}
                ]
            }"#,
        )
        .unwrap();
        let (mut app, entity) = player_app(spec);
        // The first tick boots the initial state; the next twelve walk.
        for _ in 0..13 {
            app.update();
        }
        let walking = player(&app, entity);
        assert_eq!(
            (walking.state.as_str(), walking.clip.as_str()),
            ("Walking", "Walk")
        );
        assert!(walking.elapsed > 0.15, "only {}", walking.elapsed);
        assert_eq!(
            walking.frame,
            (walking.elapsed * 10.0).floor() as usize % 4 + 1
        );
        let x = app.world().get::<Transform>(entity).unwrap().translation.x;
        // Forty units a 0.4 s pass.
        assert!((x - walking.elapsed * 100.0).abs() < 1e-3, "walked to {x}");

        let actor = app.world().get::<ActorId>(entity).unwrap().0.clone();
        app.world_mut().resource_mut::<PendingEffects>().0 = vec![Effect::FireAnimationTrigger {
            actor,
            name: "Jump".to_string(),
        }];
        app.update();
        app.world_mut().resource_mut::<PendingEffects>().0.clear();
        assert_eq!(player(&app, entity).state, "Jumping");
        // Jump runs 0.2 s, then `next` takes it back to walking.
        for _ in 0..14 {
            app.update();
        }
        let back = player(&app, entity);
        assert_eq!(
            (back.state.as_str(), back.clip.as_str()),
            ("Walking", "Walk")
        );
        assert!(back.playing);
    }

    #[test]
    fn rig_time_loops_ping_pongs_and_ends() {
        let animation = RigAnimation {
            duration: 2.0,
            ..RigAnimation::default()
        };
        assert_eq!(rig_time(&animation, 2.5, LoopMode::Loop), (0.5, false));
        assert_eq!(rig_time(&animation, 2.5, LoopMode::PingPong), (1.5, false));
        assert_eq!(rig_time(&animation, 2.5, LoopMode::Once), (2.0, true));
    }

    #[test]
    fn clip_motion_is_spread_over_a_pass() {
        let mut clip = AnimationClip::new("Walk");
        clip.frames = vec!["a".into(), "b".into()];
        clip.fps = 2.0;
        clip.motion = [10.0, 0.0];
        clip.loop_mode = LoopMode::Once;
        assert_eq!(clip_travel(&clip, 0.0, 0.5), [5.0, 0.0]);
        // A Once clip stops carrying the actor at its end.
        assert_eq!(clip_travel(&clip, 0.5, 5.0), [5.0, 0.0]);
    }

    #[test]
    fn a_state_name_resolves_before_a_clip_name() {
        let spec: AnimationSpec = serde_json::from_str(
            r#"{"clips":[{"name":"walk","frames":["a"]},{"name":"empty"}],
                "states":[{"name":"Moving","clip":"walk","speed":2,"root_motion":true}]}"#,
        )
        .unwrap();
        assert_eq!(
            resolve(&spec, None, "moving").unwrap(),
            ("walk".to_string(), "Moving".to_string(), 2.0, true)
        );
        assert_eq!(resolve(&spec, None, "walk").unwrap().1, "Moving");
        assert!(
            resolve(&spec, None, "empty")
                .unwrap_err()
                .contains("no frames")
        );
        assert!(
            resolve(&spec, None, "fly")
                .unwrap_err()
                .contains("no animation clip")
        );
    }
}
