//! Screen-space UI layered over the running world. Status stays in the corner;
//! each actor's current `say` is projected through the world camera so it
//! follows that actor in both 2D and 3D.

use crate::engine::{ActorId, Dimension, Engine};
use crate::world::{self, WorldCamera};
use bevy::prelude::*;
use std::collections::HashSet;

#[derive(Component)]
pub struct OverlayText;

#[derive(Component)]
pub(crate) struct SpeechBubble {
    actor: String,
}

#[derive(Component)]
pub(crate) struct SpeechBubbleText {
    actor: String,
}

pub fn spawn(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(10.0),
            left: Val::Px(12.0),
            max_width: Val::Percent(60.0),
            ..default()
        },
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(14.0),
            ..default()
        },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
        OverlayText,
    ));
}

pub fn update_status(engine: NonSend<Engine>, mut overlay: Query<&mut Text, With<OverlayText>>) {
    let Ok(mut text) = overlay.single_mut() else {
        return;
    };
    let heading = if !engine.running {
        "stopped - press Play in the editor"
    } else if engine.paused {
        "paused"
    } else {
        "running"
    };
    let next = format!("{} - {heading}", engine.project.name);
    if text.0 != next {
        text.0 = next;
    }
}

/// Reconciles the actor-to-bubble map, updates text immediately when another
/// `say` runs, and projects every bubble's actor anchor each frame.
///
/// The camera's own transform is up to date in `Update`; its `GlobalTransform`
/// only re-propagates in `PostUpdate`, so it would be the previous frame's.
/// The world camera is always spawned parentless, so the local transform is
/// the global one and can be read fresh.
pub fn update_speech_bubbles(
    mut commands: Commands,
    engine: NonSend<Engine>,
    dimension: Res<Dimension>,
    assets: Res<AssetServer>,
    cameras: Query<(&Camera, &Transform), With<WorldCamera>>,
    actors: Query<(&ActorId, &Transform, &Visibility)>,
    mut bubbles: Query<(Entity, &SpeechBubble, &mut Node)>,
    mut labels: Query<(&SpeechBubbleText, &mut Text)>,
) {
    let camera = cameras.iter().next();
    let mut existing = HashSet::new();

    for (entity, bubble, mut node) in &mut bubbles {
        let Some(text) = engine.speech.get(&bubble.actor) else {
            commands.entity(entity).despawn();
            continue;
        };
        existing.insert(bubble.actor.clone());

        for (label, mut rendered) in &mut labels {
            if label.actor == bubble.actor && rendered.0 != *text {
                rendered.0.clone_from(text);
            }
        }

        let Some(actor_entity) = engine.entities.get(&bubble.actor) else {
            node.display = Display::None;
            continue;
        };
        let Ok((_, transform, visibility)) = actors.get(*actor_entity) else {
            node.display = Display::None;
            continue;
        };
        let Some(actor) = engine.actor(&bubble.actor) else {
            node.display = Display::None;
            continue;
        };
        let Some((camera, camera_transform)) = camera else {
            node.display = Display::None;
            continue;
        };
        let camera_global = GlobalTransform::from(*camera_transform);
        let Ok(viewport) = camera.world_to_viewport(
            &camera_global,
            world::actor_top(actor, transform, dimension.0),
        ) else {
            node.display = Display::None;
            continue;
        };
        if *visibility == Visibility::Hidden {
            node.display = Display::None;
            continue;
        }

        let offset = engine.project.world.speech_bubble.offset;
        // Whole pixels keep the glyphs put instead of re-blitting them at a
        // new sub-pixel offset every frame while the bubble follows the actor.
        node.display = Display::Flex;
        node.left = Val::Px((viewport.x + offset[0]).round());
        node.top = Val::Px((viewport.y + offset[1]).round());
    }

    for (actor, text) in &engine.speech {
        if !existing.contains(actor) {
            spawn_speech_bubble(
                &mut commands,
                &assets,
                engine.project_dir.as_deref(),
                actor,
                text,
                &engine.project.world.speech_bubble,
            );
        }
    }
}

fn spawn_speech_bubble(
    commands: &mut Commands,
    assets: &AssetServer,
    dir: Option<&std::path::Path>,
    actor: &str,
    text: &str,
    style: &blockloom_core::scene::SpeechBubbleStyle,
) {
    let horizontal = Val::Px(style.padding[0].max(0.0));
    let vertical = Val::Px(style.padding[1].max(0.0));
    let background = world::parse_color(&style.background);
    let border = world::parse_color(&style.border);
    let mut font = TextFont::from_font_size(FontSize::Px(style.font_size.max(1.0)));
    if let Some(path) = style.font_asset.as_ref().filter(|path| !path.is_empty()) {
        font = font.with_font(assets.load(world::asset_path(dir, path)));
    }

    let actor_id = actor.to_string();
    let label_actor = actor_id.clone();
    commands
        .spawn((
            Name::new(format!("speech bubble: {actor}")),
            SpeechBubble { actor: actor_id },
            Node {
                position_type: PositionType::Absolute,
                max_width: Val::Px(style.max_width.max(40.0)),
                padding: UiRect::new(horizontal, horizontal, vertical, vertical),
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::all(Val::Px(12.0)),
                ..default()
            },
            UiTransform::from_translation(Val2::percent(-50.0, -100.0)),
            BackgroundColor(background),
            BorderColor::all(border),
            GlobalZIndex(20),
        ))
        .with_children(|parent| {
            // A rotated square gives the bubble its tail while inheriting the
            // same colors as the body. A future skin asset can replace this
            // presentation without touching actor or VM state.
            parent.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(12.0),
                    height: Val::Px(12.0),
                    left: Val::Percent(50.0),
                    bottom: Val::Px(-7.0),
                    border: UiRect::new(Val::ZERO, Val::Px(2.0), Val::ZERO, Val::Px(2.0)),
                    ..default()
                },
                UiTransform {
                    translation: Val2::percent(-50.0, 0.0),
                    rotation: Rot2::radians(std::f32::consts::FRAC_PI_4),
                    ..default()
                },
                BackgroundColor(background),
                BorderColor::all(border),
            ));
            parent.spawn((
                SpeechBubbleText { actor: label_actor },
                Text::new(text),
                font,
                TextColor(world::parse_color(&style.text)),
                TextLayout {
                    justify: Justify::Center,
                    ..default()
                },
            ));
        });
}
