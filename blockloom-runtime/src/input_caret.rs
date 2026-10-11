//! The caret, selection and composition underline of a focused text input,
//! drawn over its text from the text layout's own glyph geometry.

use crate::ui::{UiElementText, UiManager};
use bevy::prelude::*;
use bevy::text::{ComputedTextBlock, TextColor};
use bevy::ui::{ComputedNode, UiGlobalTransform};
use parley::{Affinity, Cursor, Selection};

/// One drawn mark; they are children of the text's parent element.
#[derive(Component)]
pub struct InputMark;

/// Seconds a blink phase lasts.
const BLINK: f64 = 0.53;

/// The byte offset into the text node's shown text under `point`, in physical
/// screen pixels. None when the node's transform can't be inverted.
pub fn byte_at(
    block: &ComputedTextBlock,
    node: &ComputedNode,
    transform: &UiGlobalTransform,
    point: Vec2,
) -> Option<usize> {
    let local = transform.try_inverse()?.transform_point2(point) + node.size() / 2.0;
    Some(Cursor::from_point(block.buffer(), local.x, local.y).index())
}

/// A mark in a node's own pixels: `[x0, y0, x1, y1]`.
type Mark = ([f32; 4], Color);

fn marks_of(
    block: &ComputedTextBlock,
    shown: &blockloom_core::ui::ShownText,
    width: f32,
    ink: Color,
    selection: Color,
    blink_on: bool,
) -> Vec<Mark> {
    let layout = block.buffer();
    let bounds = |b: parley::BoundingBox| [b.x0 as f32, b.y0 as f32, b.x1 as f32, b.y1 as f32];
    let at = |byte| Cursor::from_byte_index(layout, byte, Affinity::Downstream);
    let mut marks = Vec::new();
    if let Some((start, end)) = shown.selection {
        for (rect, _) in Selection::new(at(start), at(end)).geometry(layout) {
            marks.push((bounds(rect), selection));
        }
    }
    if let Some((start, end)) = shown.preedit {
        for (rect, _) in Selection::new(at(start), at(end)).geometry(layout) {
            let b = bounds(rect);
            marks.push(([b[0], b[3] - width, b[2], b[3]], ink));
        }
    }
    if shown.selection.is_none() && blink_on {
        marks.push((bounds(at(shown.caret).geometry(layout, width)), ink));
    }
    marks
}

/// Keeps the marks of the focused input in step with its layout. Runs on last
/// frame's layout, so a caret that just moved may trail it by one frame.
pub fn draw_input_caret(
    mut commands: Commands,
    manager: Res<UiManager>,
    time: Res<Time<Real>>,
    texts: Query<(
        &UiElementText,
        &ComputedTextBlock,
        &ComputedNode,
        &UiGlobalTransform,
        &TextColor,
        &ChildOf,
    )>,
    parents: Query<(&ComputedNode, &UiGlobalTransform)>,
    mut marks: Query<(Entity, &ChildOf, &mut Node, &mut BackgroundColor), With<InputMark>>,
    mut last: Local<(usize, f64)>,
) {
    let mut wanted: Vec<(Entity, [f32; 4], Color)> = Vec::new();
    let focused = manager
        .focus()
        .and_then(|id| manager.get(id).map(|node| (id, node)));
    if let Some((id, node)) = focused
        && let Some(shown) = crate::ui::shown_input(node, true)
        && let Some((_, block, text_node, text_at, ink, child_of)) =
            texts.iter().find(|(owner, ..)| owner.0 == id)
        && let Ok((parent_node, parent_at)) = parents.get(child_of.parent())
        && let Some(into_parent) = parent_at.try_inverse()
    {
        let now = time.elapsed_secs_f64();
        if last.0 != shown.caret {
            *last = (shown.caret, now);
        }
        let blink_on = (((now - last.1) / BLINK) as u64).is_multiple_of(2);
        let scale = 1.0 / text_node.inverse_scale_factor().max(f32::EPSILON);
        let accent = crate::ui::accent(manager.theme()).with_alpha(0.45);
        let (inset, shrink) = (parent_node.border(), parent_node.inverse_scale_factor());
        for (rect, color) in marks_of(
            block,
            &shown,
            (1.5 * scale).max(1.0),
            ink.0,
            accent,
            blink_on,
        ) {
            // Text-local pixels to the parent's padding box, in logical units.
            let to_parent = |x: f32, y: f32| {
                let screen = text_at.transform_point2(Vec2::new(x, y) - text_node.size() / 2.0);
                let local = into_parent.transform_point2(screen) + parent_node.size() / 2.0;
                (local - inset.min_inset) * shrink
            };
            let (min, max) = (to_parent(rect[0], rect[1]), to_parent(rect[2], rect[3]));
            wanted.push((
                child_of.parent(),
                [
                    min.x,
                    min.y,
                    (max.x - min.x).max(0.0),
                    (max.y - min.y).max(0.0),
                ],
                color,
            ));
        }
    }
    let have: Vec<_> = marks.iter().map(|(e, c, ..)| (e, c.parent())).collect();
    let same =
        have.len() == wanted.len() && have.iter().zip(&wanted).all(|((_, p), (q, ..))| p == q);
    if !same {
        for (entity, ..) in &have {
            commands.entity(*entity).despawn();
        }
        for (parent, [x, y, w, h], color) in wanted {
            commands.entity(parent).with_child((
                InputMark,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(x),
                    top: Val::Px(y),
                    width: Val::Px(w),
                    height: Val::Px(h),
                    ..default()
                },
                BackgroundColor(color),
            ));
        }
        return;
    }
    for ((entity, ..), (_, [x, y, w, h], color)) in have.iter().zip(wanted) {
        let Ok((_, _, mut node, mut fill)) = marks.get_mut(*entity) else {
            continue;
        };
        let (left, top, width, height) = (Val::Px(x), Val::Px(y), Val::Px(w), Val::Px(h));
        if node.left != left || node.top != top || node.width != width || node.height != height {
            (node.left, node.top, node.width, node.height) = (left, top, width, height);
        }
        if fill.0 != color {
            fill.0 = color;
        }
    }
}
