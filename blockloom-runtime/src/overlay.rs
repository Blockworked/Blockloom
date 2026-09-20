//! The small on-screen log in the corner: what actors have said, and whether
//! a run is going. It stands in for Scratch's speech bubbles, which need
//! per-actor world-space text the two dimensions would each do differently.

use crate::engine::Engine;
use bevy::prelude::*;

#[derive(Component)]
pub struct OverlayText;

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

pub fn update(engine: NonSend<Engine>, mut overlay: Query<&mut Text, With<OverlayText>>) {
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
    let lines = std::iter::once(format!("{} · {heading}", engine.project.name))
        .chain(engine.says.iter().cloned())
        .collect::<Vec<_>>()
        .join("\n");
    if text.0 != lines {
        text.0 = lines;
    }
}
