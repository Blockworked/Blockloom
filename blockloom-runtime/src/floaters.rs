//! Floating numbers (`set sprite FloatNumber`): a label that rises from where
//! the sprite stood and fades. It is a UI node projected through the world
//! camera each frame, like a speech bubble, and runs on the real clock.

use crate::engine::Dimension;
use crate::world::WorldCamera;
use bevy::prelude::*;
use blockloom_core::sprite2d::{FLOAT_SECONDS, float_label, float_state};

#[derive(Component)]
pub struct Floater {
    anchor: Vec3,
    age: f32,
    color: Color,
}

pub fn register(app: &mut App) {
    app.add_systems(
        Update,
        update_floaters.run_if(|d: Res<Dimension>| d.0 == blockloom_core::scene::Mode::TwoD),
    );
}

/// Starts a floating `value` at a world point.
pub fn spawn(commands: &mut Commands, value: f32, anchor: Vec3) {
    let label = float_label(value);
    if label.is_empty() {
        return;
    }
    let color = if value < 0.0 {
        Color::srgb(1.0, 0.3, 0.25)
    } else if value > 0.0 {
        Color::srgb(0.45, 1.0, 0.5)
    } else {
        Color::WHITE
    };
    commands.spawn((
        Name::new("floating number"),
        Floater {
            anchor,
            age: 0.0,
            color,
        },
        Node {
            position_type: PositionType::Absolute,
            display: Display::None,
            ..default()
        },
        UiTransform::from_translation(Val2::percent(-50.0, -100.0)),
        Text::new(label),
        TextFont {
            font_size: FontSize::Px(22.0),
            ..default()
        },
        TextColor(color),
        GlobalZIndex(30),
    ));
}

fn update_floaters(
    mut commands: Commands,
    real: Res<Time<Real>>,
    ui_scale: Res<UiScale>,
    cameras: Query<(&Camera, &Transform), With<WorldCamera>>,
    mut floaters: Query<(Entity, &mut Floater, &mut Node, &mut TextColor)>,
) {
    let camera = cameras.iter().next();
    let ui = ui_scale.0.max(0.01);
    for (entity, mut floater, mut node, mut color) in &mut floaters {
        floater.age += real.delta_secs();
        if floater.age >= FLOAT_SECONDS {
            commands.entity(entity).despawn();
            continue;
        }
        let Some((camera, transform)) = camera else {
            node.display = Display::None;
            continue;
        };
        let global = GlobalTransform::from(*transform);
        let Ok(viewport) = camera.world_to_viewport(&global, floater.anchor) else {
            node.display = Display::None;
            continue;
        };
        let (rise, alpha) = float_state(floater.age);
        node.display = Display::Flex;
        node.left = Val::Px(viewport.x.round() / ui);
        node.top = Val::Px((viewport.y - rise * ui).round() / ui);
        color.0 = floater.color.with_alpha(alpha);
    }
}
