//! Input that the interface takes first: back, typing, clicks, scrolling and selection.

use super::*;

/// Fires the Android back button as the `back` key. It has no physical code
/// (winit reports it `Unidentified`) and arrives as logical `BrowserBack`,
/// which nothing else reads - so this small system owns it rather than
/// growing `publish_sensors` past Bevy's parameter limit.
///
/// Runs before `type_into_focused_input`: a focused input releases on the
/// same press (closing the soft keyboard), and the world never sees that
/// one. A visible modal swallows it, like clicks. Anything else fires even
/// while paused, the way escape does, so a game with no escape key can still
/// toggle its menu.
pub fn back_button(
    mut engine: NonSendMut<Engine>,
    manager: Res<crate::ui::UiManager>,
    mut presses: MessageReader<KeyboardInput>,
) {
    for press in presses.read() {
        if press.state != ButtonState::Pressed || press.logical_key != Key::BrowserBack {
            continue;
        }
        if !engine.running {
            continue;
        }
        if manager.focus().is_some() || manager.swallows_world_clicks() {
            continue;
        }
        engine.fire(Event::Key("back".to_string()));
    }
}

/// Types into whichever text input holds the keyboard. Escape lets go of
/// it, Backspace and Delete rub characters out around the caret, the arrows,
/// Home and End move it (Shift selects, Ctrl or Cmd with A selects all), and
/// every other key that means a character adds one at the caret. `changed`
/// fires per keystroke, as the spec has it.
///
/// While an input holds the keyboard, `publish_sensors` hands the game no
/// keys at all, so nothing else sees this typing.
pub fn type_into_focused_input(
    mut engine: NonSendMut<Engine>,
    mut manager: ResMut<crate::ui::UiManager>,
    mut typed: MessageReader<KeyboardInput>,
    // Optional so a world without a window (or a test) needs neither.
    ime_messages: Option<Res<Messages<Ime>>>,
    mut ime_cursor: Local<MessageCursor<Ime>>,
    held: Option<Res<ButtonInput<KeyCode>>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    let mut imes: Vec<Ime> = ime_messages
        .map(|m| ime_cursor.read(&m).cloned().collect())
        .unwrap_or_default();
    let focused = manager.focus().map(str::to_string);
    // The window asks the platform for composition only while an input holds
    // the keyboard, so a game's own keys never open an input method.
    for mut window in &mut windows {
        let wanted = focused.is_some();
        if window.ime_enabled != wanted {
            window.ime_enabled = wanted;
        }
    }
    let Some(focused) = focused else {
        // Nothing has the keyboard, but the queue still has to be drained:
        // otherwise a burst arrives the moment an input is clicked.
        typed.clear();
        return;
    };
    if !engine.running {
        return;
    }
    let Some(node) = manager.get(&focused) else {
        typed.clear();
        return;
    };
    let (allow, ceiling) = (node.allow(), node.max_length());
    let mut text = node.value.as_text();
    let before = text.clone();
    let mut caret = node.caret;
    caret.clamp(&text);
    let (caret_before, mut preedit) = (caret, node.preedit.clone());
    let mut release = false;
    let down = |keys: [KeyCode; 2]| held.as_ref().is_some_and(|h| h.any_pressed(keys));
    let shift = down([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let command = down([KeyCode::ControlLeft, KeyCode::ControlRight])
        || down([KeyCode::SuperLeft, KeyCode::SuperRight]);
    // Every character goes through the input's own rules, one at a time:
    // a full field takes no more, and a numeric one takes no letters. What
    // is refused is simply not there - a rubbed-out keystroke rather than
    // an error, since a person holding a key down means no harm by it.
    for key in typed.read() {
        if key.state != ButtonState::Pressed {
            continue;
        }
        match &key.logical_key {
            Key::Escape => release = true,
            // The phone's back button behaves like escape in a field: it
            // hands the keyboard back rather than typing anything.
            Key::BrowserBack => release = true,
            Key::Backspace => {
                caret.backspace(&mut text);
            }
            Key::Delete => {
                caret.delete(&mut text);
            }
            Key::ArrowLeft => caret.step(&text, -1, shift),
            Key::ArrowRight => caret.step(&text, 1, shift),
            Key::Home => caret.home(shift),
            Key::End => caret.end(&text, shift),
            Key::Space => {
                caret.insert(&mut text, ' ', allow, ceiling);
            }
            Key::Enter => release = true,
            Key::Character(written) if command => {
                if written.eq_ignore_ascii_case("a") {
                    caret.select_all(&text);
                }
            }
            Key::Character(written) => {
                for ch in written.chars() {
                    caret.insert(&mut text, ch, allow, ceiling);
                }
            }
            _ => {}
        }
    }
    // An input method commits whole runs instead of keys: same rules, one
    // character at a time. Until it does, its composition is only shown.
    for ime in imes.drain(..) {
        match ime {
            Ime::Preedit { value, cursor, .. } => {
                preedit = if cursor.is_some() {
                    value
                } else {
                    String::new()
                };
            }
            Ime::Commit { value, .. } => {
                preedit.clear();
                for ch in value.chars() {
                    caret.insert(&mut text, ch, allow, ceiling);
                }
            }
            Ime::Disabled { .. } => preedit.clear(),
            Ime::Enabled { .. } => {}
        }
    }
    let changed = text != before;
    let moved = caret != caret_before;
    let shown = manager.get(&focused).map(|n| n.preedit.clone());
    if (moved || changed || shown.as_ref() != Some(&preedit))
        && let Some(node) = manager.get_mut(&focused)
    {
        node.caret = caret;
        node.preedit = preedit;
        node.dirty = true;
    }
    if changed && let Some(value) = manager.changed(&focused, Evaluated::Text(text)) {
        if let Some(node) = manager.get_mut(&focused) {
            let settled = node.value.as_text();
            node.caret.clamp(&settled);
        }
        engine.fire(Event::UiChanged {
            id: focused.clone(),
            value,
        });
    }
    if release {
        manager.focus_on(None);
    }
}

/// Routes a click: the interface is asked first, and only what it doesn't
/// want reaches the world.
///
/// The interface is asked whether the game is running or paused - a pause
/// menu's own buttons are the whole point - while world picks need a running,
/// unpaused game with no modal element up. That is why this can't return
/// early on `paused` the way it used to.
pub fn detect_clicks(
    mut engine: NonSendMut<Engine>,
    mut manager: ResMut<crate::ui::UiManager>,
    buttons: Res<ButtonInput<MouseButton>>,
    dimension: Res<Dimension>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    actors: Query<(&ActorId, &Transform)>,
    laid_out: Query<(&ComputedNode, &UiGlobalTransform)>,
    words: Query<(
        &crate::ui::UiElementText,
        &bevy::text::ComputedTextBlock,
        &ComputedNode,
        &UiGlobalTransform,
    )>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    preview_pointer: Option<Res<crate::preview::PreviewPointer>>,
) {
    if !engine.running || !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    // Embedded in the editor there is no window, only the preview pointer.
    let window = windows.iter().next();
    // Clicks land in the focused window, so a click in the editor beside a
    // running game must never start its click strands. Event truth, same as
    // the motion gate above: the component defaults to focused. A live
    // preview pointer counts as attention for the same reason.
    let preview_live = preview_pointer
        .as_ref()
        .is_some_and(|pointer| crate::preview::pointer_live(pointer));
    if !engine.window_focused && !preview_live {
        return;
    }
    let Some(cursor) = preview_pointer
        .as_ref()
        .and_then(|pointer| pointer.pos.filter(|_| preview_live))
        .or_else(|| window.and_then(Window::cursor_position))
    else {
        return;
    };
    // Boxes are laid out in physical pixels while the cursor reads logical:
    // scale it up the way Bevy's own picking does, or every click falls
    // through on a scaled display.
    let cursor = cursor * window.map_or(1.0, Window::scale_factor);

    // The interface first, topmost-first, over the rectangles Bevy laid out
    // this frame - so a panel and a label are as clickable as a button.
    let hit = manager
        .hit(cursor, |node| screen_rect(&laid_out, node.entity))
        .map(|node| {
            (
                node.spec.id.clone(),
                node.kind,
                node.range,
                node.step(),
                node.entity,
            )
        });
    if let Some((id, kind, range, step, entity)) = hit {
        // Clicking a text input hands it the keyboard; clicking anything
        // else takes it back, which is how clicking away releases the keys.
        manager.focus_on(if kind == UiKind::Input {
            Some(id.as_str())
        } else {
            None
        });
        // A click on a focused input puts the caret under the pointer.
        if kind == UiKind::Input
            && let Some((_, block, node, at)) = words.iter().find(|(owner, ..)| owner.0 == id)
            && let Some(byte) = crate::input_caret::byte_at(block, node, at, cursor)
        {
            let shift = keys
                .as_ref()
                .is_some_and(|k| k.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]));
            manager.place_caret(&id, byte, shift);
        }
        // A drag on a slider and a press on a toggle are changes, not
        // clicks - though both also count as a click on the element.
        let changed = match kind {
            UiKind::Slider | UiKind::Scrollbar => screen_rect(&laid_out, entity).and_then(|rect| {
                let width = rect.width().max(1.0);
                let fraction = (cursor.x - rect.min.x) / width;
                let at = crate::ui::slider_at(range, step, fraction);
                manager.changed(&id, Evaluated::Number(at))
            }),
            UiKind::Toggle => {
                let was = manager.get(&id).is_some_and(|node| node.value.as_bool());
                manager.changed(&id, Evaluated::Bool(!was))
            }
            _ => None,
        };
        if kind != UiKind::Toggle {
            crate::ui_systems::activate(&mut manager, &mut engine, &id);
        } else {
            for target in manager.bubble(&id) {
                engine.fire(Event::UiClicked { id: target });
            }
        }
        if let Some(value) = changed {
            engine.fire(Event::UiChanged { id, value });
        }
        return;
    }
    manager.focus_on(None);

    // Nothing in the interface wanted it, so the world picks are next - if
    // the world is taking clicks at all.
    if !world_takes_clicks(&engine, &manager) {
        return;
    }
    let Some((camera, camera_transform)) = cameras.iter().next() else {
        return;
    };

    let mut hits: Vec<String> = Vec::new();
    match dimension.0 {
        Mode::TwoD => {
            let Ok(point) = camera.viewport_to_world_2d(camera_transform, cursor) else {
                return;
            };
            for (id, transform) in &actors {
                let Some(visual) = engine.actor(&id.0).and_then(|a| a.visual()).cloned() else {
                    continue;
                };
                let delta = point - transform.translation.truncate();
                // A circle is picked by distance, not by box: the corners of
                // its bounding square aren't part of the actor.
                if let Visual::Circle { radius, .. } = &visual {
                    let scale = transform.scale.truncate().abs().max_element();
                    if delta.length() <= radius * scale {
                        hits.push(id.0.clone());
                    }
                    continue;
                }
                let half = half_extents(&visual) * transform.scale.truncate();
                if delta.x.abs() <= half.x && delta.y.abs() <= half.y {
                    hits.push(id.0.clone());
                }
            }
        }
        Mode::ThreeD => {
            let Ok(ray) = camera.viewport_to_world(camera_transform, cursor) else {
                return;
            };
            for (id, transform) in &actors {
                let Some(visual) = engine.actor(&id.0).and_then(|a| a.visual()).cloned() else {
                    continue;
                };
                // A bounding sphere is close enough to pick with, and needs no
                // collider - an actor with no body is still clickable.
                let radius = (half_extents3(&visual) * transform.scale).length();
                let to_center = transform.translation - ray.origin;
                let along = to_center.dot(*ray.direction);
                if along < 0.0 {
                    continue;
                }
                let closest = ray.origin + *ray.direction * along;
                if closest.distance(transform.translation) <= radius {
                    hits.push(id.0.clone());
                }
            }
        }
    }
    for actor in hits {
        engine.fire(Event::Click { actor });
    }
}

/// Scrolls the nearest list under the pointer. Children count as part of the
/// list, and clipped children cannot receive clicks or wheel input outside its
/// viewport because `UiManager::hit` checks list ancestors.
pub fn scroll_ui_lists(
    mut engine: NonSendMut<Engine>,
    mut wheels: MessageReader<MouseWheel>,
    windows: Query<&Window, With<PrimaryWindow>>,
    manager: Res<crate::ui::UiManager>,
    laid_out: Query<(&ComputedNode, &UiGlobalTransform)>,
    mut scrolls: Query<(&mut ScrollPosition, &Node, &ComputedNode)>,
    preview_pointer: Option<Res<crate::preview::PreviewPointer>>,
) {
    // Drained whatever happens, so a wheel over nothing never lands later.
    let wheels: Vec<MouseWheel> = wheels.read().copied().collect();
    if wheels.is_empty() {
        return;
    }
    // Embedded there is no window, only the preview pointer, as for clicks.
    let window = windows.iter().next();
    let Some(point) = preview_pointer
        .as_ref()
        .filter(|pointer| crate::preview::pointer_live(pointer))
        .and_then(|pointer| pointer.pos)
        .or_else(|| window.and_then(Window::cursor_position))
    else {
        return;
    };
    // Physical boxes, logical cursor: scaled up like a click is.
    let point = point * window.map_or(1.0, Window::scale_factor);
    let Some(hit) = manager.hit(point, |node| screen_rect(&laid_out, node.entity)) else {
        return;
    };
    let Some(list) = manager.scroll_owner(&hit.spec.id) else {
        return;
    };
    crate::ui_systems::emit_event(&manager, &mut engine, &list.spec.id, "scroll");
    let Ok((mut position, node, computed)) = scrolls.get_mut(list.entity) else {
        return;
    };
    for wheel in wheels {
        let scale = if wheel.unit == MouseScrollUnit::Line {
            24.0
        } else {
            1.0
        };
        let max = (computed.content_size() - computed.size()) * computed.inverse_scale_factor();
        if node.overflow.y == OverflowAxis::Scroll {
            position.y = (position.y - wheel.y * scale).clamp(0.0, max.y.max(0.0));
        }
    }
}

/// Whether a click nothing in the interface wanted should reach the world.
/// A frozen world has no world clicks at all, and any visible modal element
/// swallows what lands beside it - so clicking next to a pause menu never
/// fires the gun behind it.
pub(crate) fn world_takes_clicks(engine: &Engine, manager: &crate::ui::UiManager) -> bool {
    !engine.paused && !manager.swallows_world_clicks()
}

/// Where one interface element's box ended up on screen, from the layout
/// Bevy computed this frame. `None` for an element nothing has drawn yet.
/// Holding the button down on a focused input drags its selection.
pub fn drag_input_selection(
    engine: NonSend<Engine>,
    mut manager: ResMut<crate::ui::UiManager>,
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    words: Query<(
        &crate::ui::UiElementText,
        &bevy::text::ComputedTextBlock,
        &ComputedNode,
        &UiGlobalTransform,
    )>,
    preview_pointer: Option<Res<crate::preview::PreviewPointer>>,
) {
    if !engine.running
        || !buttons.pressed(MouseButton::Left)
        || buttons.just_pressed(MouseButton::Left)
    {
        return;
    }
    let Some(id) = manager.focus().map(str::to_string) else {
        return;
    };
    let window = windows.iter().next();
    let Some(cursor) = preview_pointer
        .as_ref()
        .and_then(|pointer| {
            pointer
                .pos
                .filter(|_| crate::preview::pointer_live(pointer))
        })
        .or_else(|| window.and_then(Window::cursor_position))
    else {
        return;
    };
    let cursor = cursor * window.map_or(1.0, Window::scale_factor);
    if let Some((_, block, node, at)) = words.iter().find(|(owner, ..)| owner.0 == id)
        && let Some(byte) = crate::input_caret::byte_at(block, node, at, cursor)
    {
        manager.place_caret(&id, byte, true);
    }
}
