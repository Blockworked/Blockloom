//! 2D rain and snow over the view, from the weather blend's precipitation and
//! snow and the global wind. UI nodes from a pool; the motes come from core.

use crate::engine::{Dimension, Engine};
use crate::wind::WindField;
use bevy::prelude::*;
use blockloom_core::weather2d::{MAX_MOTES, MoteKind, count, leaf_intensity, mote};

#[derive(Component)]
struct Speck {
    kind: MoteKind,
    index: usize,
}

pub fn register(app: &mut App) {
    app.add_systems(
        Update,
        draw_weather.run_if(|d: Res<Dimension>| d.0 == blockloom_core::scene::Mode::TwoD),
    );
}

fn spawn_pool(commands: &mut Commands, kind: MoteKind) {
    for index in 0..MAX_MOTES {
        commands.spawn((
            Name::new("weather mote"),
            Speck { kind, index },
            Node {
                position_type: PositionType::Absolute,
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::NONE),
            GlobalZIndex(5),
        ));
    }
}

fn draw_weather(
    mut commands: Commands,
    engine: NonSend<Engine>,
    real: Res<Time<Real>>,
    wind: Res<WindField>,
    mut have: Local<[bool; 3]>,
    mut specks: Query<(&Speck, &mut Node, &mut BackgroundColor, &mut UiTransform)>,
) {
    let now = engine.weather.sampled();
    let wind_x = wind.at(Vec3::ZERO).x;
    let time = real.elapsed_secs();
    // Leaves only blow in the rain-free air.
    let leaf = leaf_intensity(now.wind_speed) * (1.0 - now.precipitation.clamp(0.0, 1.0));
    for (slot, kind, intensity) in [
        (0, MoteKind::Rain, now.precipitation),
        (1, MoteKind::Snow, now.snow),
        (2, MoteKind::Leaf, leaf),
    ] {
        if !have[slot] && count(intensity) > 0 {
            have[slot] = true;
            spawn_pool(&mut commands, kind);
        }
    }
    for (speck, mut node, mut color, mut transform) in &mut specks {
        let intensity = match speck.kind {
            MoteKind::Rain => now.precipitation,
            MoteKind::Snow => now.snow,
            MoteKind::Leaf => leaf,
        };
        if speck.index >= count(intensity) {
            node.display = Display::None;
            continue;
        }
        let m = mote(speck.kind, speck.index, time, wind_x);
        node.display = Display::Flex;
        node.left = Val::Percent(m.x * 100.0);
        node.top = Val::Percent(m.y * 100.0);
        *transform = UiTransform::default();
        match (speck.kind, m.splash) {
            (_, Some(ring)) => {
                let w = 6.0 + 14.0 * ring;
                node.width = Val::Px(w);
                node.height = Val::Px(w * 0.3);
                node.border_radius = BorderRadius::MAX;
                color.0 = Color::srgba(0.8, 0.9, 1.0, m.alpha * 0.6);
            }
            (MoteKind::Rain, None) => {
                node.width = Val::Px(1.5);
                node.height = Val::Px(m.size);
                node.border_radius = BorderRadius::ZERO;
                transform.rotation = Rot2::radians(-(wind_x * 0.1).clamp(-0.6, 0.6));
                color.0 = Color::srgba(0.75, 0.85, 1.0, m.alpha);
            }
            (MoteKind::Leaf, None) => {
                node.width = Val::Px(m.size);
                node.height = Val::Px(m.size * 0.6);
                node.border_radius = BorderRadius::MAX;
                transform.rotation = Rot2::radians(time * 2.0 + speck.index as f32);
                color.0 = Color::srgba(0.85, 0.5, 0.15, m.alpha);
            }
            (MoteKind::Snow, None) => {
                node.width = Val::Px(m.size);
                node.height = Val::Px(m.size);
                node.border_radius = BorderRadius::MAX;
                color.0 = Color::srgba(1.0, 1.0, 1.0, m.alpha);
            }
        }
    }
}
