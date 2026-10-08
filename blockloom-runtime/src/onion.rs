//! Onion skin: translucent ghosts of the frames either side of a sprite's
//! animation frame, an editor aid for the scene view (2D).

use std::collections::HashMap;

use bevy::prelude::*;
use blockloom_core::animation::LoopMode;

use crate::anim2d::{resolve, show_frame};
use crate::edit::SceneEditor;
use crate::engine::{ActorId, AnimationPlayer, Dimension, Engine};

/// One ghost sprite: the frame before (-1) or after (+1) its owner's.
#[derive(Component)]
pub struct OnionGhost {
    owner: Entity,
    side: i8,
}

pub fn register(app: &mut App) {
    app.add_systems(
        Update,
        sync_onion.run_if(|d: Res<Dimension>| d.0 == blockloom_core::scene::Mode::TwoD),
    );
}

/// The frame `step` away from `index` in a clip of `count`, or `None` where
/// the clip stops (a clip that plays once has no frame past either end).
pub fn neighbour(index: usize, count: usize, step: i8, mode: LoopMode) -> Option<usize> {
    if count < 2 {
        return None;
    }
    let to = index as i64 + i64::from(step);
    if (0..count as i64).contains(&to) {
        return Some(to as usize);
    }
    (mode != LoopMode::Once).then(|| to.rem_euclid(count as i64) as usize)
}

fn tint(side: i8) -> Color {
    if side < 0 {
        Color::srgba(0.4, 0.6, 1.0, 0.4)
    } else {
        Color::srgba(0.5, 1.0, 0.5, 0.4)
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_onion(
    mut commands: Commands,
    engine: NonSend<Engine>,
    editor: Res<SceneEditor>,
    assets: Res<AssetServer>,
    images: Res<Assets<Image>>,
    actors: Query<(Entity, &ActorId, &Sprite, Option<&AnimationPlayer>), Without<OnionGhost>>,
    mut ghosts: Query<(Entity, &OnionGhost, &mut Sprite), Without<ActorId>>,
) {
    let wanted = editor.view.tiles.onion_skin && (!engine.running || engine.paused);
    let mut held: HashMap<(Entity, i8), Entity> = HashMap::new();
    for (ghost, info, _) in &ghosts {
        if wanted && actors.contains(info.owner) {
            held.insert((info.owner, info.side), ghost);
        } else {
            commands.entity(ghost).despawn();
        }
    }
    if !wanted {
        return;
    }
    let dir = engine.project_dir.clone();
    for (entity, id, sprite, player) in &actors {
        let Some(spec) = engine
            .actor(&id.0)
            .and_then(|a| a.components.animation())
            .filter(|_| engine.has_component(&id.0, "Animation"))
        else {
            continue;
        };
        // A running actor shows its player's frame, otherwise its first.
        let (clip_name, index) = match player {
            Some(player) => (player.clip.clone(), player.frame.saturating_sub(1)),
            None => {
                let start = if spec.initial.is_empty() {
                    spec.clips
                        .first()
                        .map(|c| c.name.clone())
                        .unwrap_or_default()
                } else {
                    resolve(spec, None, &spec.initial)
                        .map(|(clip, ..)| clip)
                        .unwrap_or_default()
                };
                (start, 0)
            }
        };
        let Some(clip) = spec.find_clip(&clip_name) else {
            continue;
        };
        for side in [-1i8, 1] {
            let frame = neighbour(index, clip.frame_count(), side, clip.loop_mode)
                .and_then(|i| clip.frame(i));
            let key = (entity, side);
            let Some(frame) = frame else {
                if let Some(ghost) = held.remove(&key) {
                    commands.entity(ghost).despawn();
                }
                continue;
            };
            let mut drawn = sprite.clone();
            show_frame(&mut drawn, frame, dir.as_deref(), &assets, &images);
            drawn.color = tint(side);
            match held.get(&key).and_then(|g| ghosts.get_mut(*g).ok()) {
                Some((_, _, mut existing)) => *existing = drawn,
                None => {
                    let child = commands
                        .spawn((
                            Name::new("onion ghost"),
                            drawn,
                            OnionGhost {
                                owner: entity,
                                side,
                            },
                            Transform::from_xyz(0.0, 0.0, -0.0001),
                        ))
                        .id();
                    commands.entity(entity).add_child(child);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbours_wrap_unless_the_clip_plays_once() {
        assert_eq!(neighbour(0, 4, -1, LoopMode::Loop), Some(3));
        assert_eq!(neighbour(3, 4, 1, LoopMode::PingPong), Some(0));
        assert_eq!(neighbour(0, 4, -1, LoopMode::Once), None);
        assert_eq!(neighbour(1, 4, 1, LoopMode::Once), Some(2));
        assert_eq!(neighbour(0, 1, 1, LoopMode::Loop), None);
    }
}
