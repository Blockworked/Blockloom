//! Screen-space UI layered over the running world: the status corner, each
//! actor's speech bubble projected through the world camera, and the
//! interface a game builds out of blocks.
//!
//! The id map and the rules that go with it live in `crate::ui`; this is the
//! Bevy half - what each element is drawn as, and where it sits.

use crate::bridge;
use crate::engine::{ActorId, Dimension, Engine, PendingEffects};
use crate::ui::{UiElementText, UiManager, UiRoot, UiSliderFill, UiToggleLamp};
use crate::world::{self, WorldCamera};
use bevy::prelude::*;
use blockloom_core::ui::UiKind;
use blockloom_core::vm::Effect;
use blockloom_protocol::RuntimeMessage;
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

// ─── The interface the blocks build ────────────────────────────────────────

/// Applies this step's interface effects to the id map, and the pause ones
/// to the engine and its scheduler. Runs inside the fixed step, before the
/// effect list is cleared; the drawing half happens per frame below.
pub fn apply_ui_effects(
    effects: Res<PendingEffects>,
    mut manager: ResMut<UiManager>,
    mut engine: NonSendMut<Engine>,
    time: Res<Time>,
) {
    if !engine.running {
        return;
    }
    for effect in &effects.0 {
        match effect {
            Effect::ShowElement { element } => {
                if element.id.is_empty() {
                    bridge::send(&RuntimeMessage::Error {
                        actor: String::new(),
                        message: "a show block needs an id to name its element by".to_string(),
                    });
                    continue;
                }
                // A menu behind a locked pointer is a dead menu: the clicks
                // would land at the centre of the screen. Say so rather than
                // letting it fail silently - the game unlocks, not us.
                if element.modal && engine.wants_cursor_locked {
                    bridge::send(&RuntimeMessage::Error {
                        actor: String::new(),
                        message: "modal shown while pointer locked - unlock mouse to click it"
                            .to_string(),
                    });
                }
                manager.show(element.clone());
            }
            Effect::HideElement { id, all } => manager.hide(id, *all),
            Effect::DeleteElement { id } => manager.delete(id),
            Effect::SetUiProp { id, prop, value } => {
                if !manager.set(id, *prop, value) {
                    bridge::send(&RuntimeMessage::Error {
                        actor: String::new(),
                        message: format!("there's no interface element called \"{id}\""),
                    });
                }
            }
            Effect::SetPaused { paused } => {
                let now = time.elapsed_secs() as f64;
                world::set_paused(&mut engine, *paused, now);
            }
            _ => {}
        }
    }
}

/// An element's own box, and what a `set` can do to it.
type Styled<'a> = (&'a mut Node, &'a mut BackgroundColor);

/// Which entities that means: the element itself, not the two pieces inside
/// one that follow its value rather than its style.
type OwnNode = (Without<UiSliderFill>, Without<UiToggleLamp>);

/// The text inside an element, and everything a `set` can do to it. Named
/// because a Bevy query of four components is a mouthful in a signature.
type Words<'a> = (
    &'a UiElementText,
    &'a mut Text,
    &'a mut TextFont,
    &'a mut TextColor,
);

/// Despawns the entities the manager has finished with. A panel's despawn
/// takes its children with it, so one of those may already be gone by the
/// time its own turn comes - hence the tolerant despawn.
pub fn despawn_dropped(commands: &mut Commands, manager: &mut UiManager) {
    for entity in manager.take_dropped() {
        if entity != Entity::PLACEHOLDER {
            commands.entity(entity).try_despawn();
        }
    }
}

/// Builds the entity for every element that has just been made, drops the
/// ones that have gone, and writes each changed element's own properties.
pub fn draw_ui(
    mut commands: Commands,
    mut manager: ResMut<UiManager>,
    engine: NonSend<Engine>,
    assets: Res<AssetServer>,
    roots: Query<Entity, With<UiRoot>>,
    // An element's own node, told apart from the two pieces inside one that
    // follow its value rather than its style - otherwise the three would be
    // asking for the same `Node` and `BackgroundColor` at once.
    mut nodes: Query<Styled, OwnNode>,
    mut labels: Query<Words>,
    mut fills: Query<(&UiSliderFill, &mut Node)>,
    mut lamps: Query<(&UiToggleLamp, &mut BackgroundColor)>,
) {
    despawn_dropped(&mut commands, &mut manager);

    let dir = engine.project_dir.clone();
    let mut screen = roots.iter().next();
    for id in manager.take_pending() {
        let Some(node) = manager.get(&id) else {
            continue;
        };
        let parent = node.parent.clone();
        // A parented element hangs off its parent's node; a top-level one
        // hangs off the screen root, which is made once and remembered -
        // a command is deferred, so the query can't see it this frame.
        let under = if parent.is_empty() {
            *screen.get_or_insert_with(|| {
                commands
                    .spawn((
                        Name::new("interface"),
                        Node {
                            position_type: PositionType::Absolute,
                            width: Val::Percent(100.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        // Over the world and the speech bubbles both.
                        GlobalZIndex(40),
                        UiRoot,
                    ))
                    .id()
            })
        } else {
            match manager.get(&parent).map(|above| above.entity) {
                Some(entity) if entity != Entity::PLACEHOLDER => entity,
                // Its parent hasn't been drawn yet, so there is nothing to
                // hang it off. Back in the queue for the next frame rather
                // than dropped on the floor.
                _ => {
                    manager.defer(&id);
                    continue;
                }
            }
        };
        let entity = spawn_element(&mut commands, &assets, dir.as_deref(), node);
        commands.entity(under).add_child(entity);
        manager.attach(&id, entity);
    }

    // An element spawned a moment ago isn't in the world yet - a command is
    // deferred - so its first pass here finds nothing and is kept for the
    // next frame rather than lost.
    let mut unseen = Vec::new();
    for id in manager.take_dirty() {
        let Some(element) = manager.get(&id) else {
            continue;
        };
        if element.entity == Entity::PLACEHOLDER {
            unseen.push(id);
            continue;
        }
        let Ok((mut node, mut background)) = nodes.get_mut(element.entity) else {
            unseen.push(id);
            continue;
        };
        node.display = if element.visible {
            Display::Flex
        } else {
            Display::None
        };
        if let Some(width) = element.style.width {
            node.width = Val::Px(width);
        }
        if let Some(height) = element.style.height {
            node.height = Val::Px(height);
        }
        if let Some(padding) = element.style.padding {
            node.padding = UiRect::all(Val::Px(padding));
        }
        // A top-level element is placed against its anchor; a parented one
        // flows after its siblings and has no coordinates at all.
        if element.parent.is_empty() {
            let at = crate::ui::anchoring(element.spec.anchor, element.spec.offset);
            node.left = at.left;
            node.top = at.top;
            node.margin = at.margin;
        }
        *background = BackgroundColor(match &element.style.background {
            Some(hex) => world::parse_color(hex),
            None => crate::ui::background_for(element.kind),
        });
        node.border_radius = BorderRadius::all(Val::Px(
            element
                .style
                .corner_radius
                .unwrap_or_else(crate::ui::corner_radius),
        ));

        let wanted = crate::ui::text_of(element);
        for (owner, mut text, mut font, mut color) in &mut labels {
            if owner.0 != id {
                continue;
            }
            if text.0 != wanted {
                text.0.clone_from(&wanted);
            }
            font.font_size =
                FontSize::Px(element.style.text_size.unwrap_or_else(crate::ui::text_size));
            *color = TextColor(match &element.style.text_color {
                Some(hex) => world::parse_color(hex),
                None => crate::ui::text_color(),
            });
        }
        // A slider's fill and a toggle's lamp are the only two whose look is
        // their value rather than their style.
        let fraction = slider_fraction(element);
        for (owner, mut bar) in &mut fills {
            if owner.0 == id {
                bar.width = Val::Percent(fraction * 100.0);
            }
        }
        let on = element.value.as_bool();
        for (owner, mut lamp) in &mut lamps {
            if owner.0 == id {
                *lamp = BackgroundColor(if on {
                    Color::srgb(0.30, 0.78, 0.44)
                } else {
                    Color::srgba(0.35, 0.38, 0.45, 0.9)
                });
            }
        }
    }
    for id in unseen {
        manager.mark_dirty(&id);
    }
}

/// Where along its own ends a slider currently stands, as a fraction.
fn slider_fraction(node: &crate::ui::UiNode) -> f32 {
    let [low, high] = node.range;
    let span = high - low;
    if span.abs() < f32::EPSILON {
        return 0.0;
    }
    ((node.value.as_number().unwrap_or(0.0) as f32 - low) / span).clamp(0.0, 1.0)
}

fn spawn_element(
    commands: &mut Commands,
    assets: &AssetServer,
    dir: Option<&std::path::Path>,
    node: &crate::ui::UiNode,
) -> Entity {
    let id = node.spec.id.clone();
    let parented = !node.parent.is_empty();
    let base = crate::ui::node_for(&node.spec, parented);
    let mut entity = commands.spawn((
        Name::new(format!("ui: {id}")),
        base,
        BackgroundColor(crate::ui::background_for(node.kind)),
    ));
    // A top-level element takes its anchor's share of its own size back off,
    // so a right-anchored one ends up inside the window rather than past it.
    if !parented {
        let at = crate::ui::anchoring(node.spec.anchor, node.spec.offset);
        entity.insert(UiTransform::from_translation(at.self_shift));
    }

    match node.kind {
        // An image is the one element whose content is a file rather than
        // words, so it carries the texture itself and no text node.
        UiKind::Image => {
            let path = node.spec.content.trim();
            if !path.is_empty() {
                entity.insert(ImageNode::new(assets.load(world::asset_path(dir, path))));
            }
        }
        UiKind::Slider => {
            let value = id.clone();
            entity.with_children(|parent| {
                parent
                    .spawn((
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Px(crate::ui::track_height()),
                            border_radius: BorderRadius::all(Val::Px(
                                crate::ui::track_height() / 2.0,
                            )),
                            ..default()
                        },
                        BackgroundColor(crate::ui::track_background()),
                    ))
                    .with_children(|track| {
                        track.spawn((
                            UiSliderFill(value),
                            Node {
                                width: Val::Percent(0.0),
                                height: Val::Percent(100.0),
                                border_radius: BorderRadius::all(Val::Px(
                                    crate::ui::track_height() / 2.0,
                                )),
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.36, 0.60, 0.94)),
                        ));
                    });
            });
        }
        UiKind::Toggle => {
            let lamp = id.clone();
            let caption = id.clone();
            entity.with_children(|parent| {
                parent.spawn((
                    UiToggleLamp(lamp),
                    Node {
                        width: Val::Px(crate::ui::knob_size()),
                        height: Val::Px(crate::ui::knob_size()),
                        border_radius: BorderRadius::all(Val::Px(4.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.35, 0.38, 0.45, 0.9)),
                ));
                parent.spawn((
                    UiElementText(caption),
                    Text::new(crate::ui::text_of(node)),
                    TextFont::from_font_size(FontSize::Px(crate::ui::text_size())),
                    TextColor(crate::ui::text_color()),
                ));
            });
        }
        // Everything else says its piece in words. A panel's is its title
        // row, which an empty title leaves off entirely.
        UiKind::Panel if node.spec.content.trim().is_empty() => {}
        _ => {
            let caption = id.clone();
            let words = crate::ui::text_of(node);
            entity.with_children(|parent| {
                parent.spawn((
                    UiElementText(caption),
                    Text::new(words),
                    TextFont::from_font_size(FontSize::Px(crate::ui::text_size())),
                    TextColor(crate::ui::text_color()),
                ));
            });
        }
    }
    entity.id()
}
