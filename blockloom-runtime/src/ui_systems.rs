//! Bindings, collection recycling and navigation for the retained interface.
use crate::engine::{ActorId, CustomComponents, Engine, PendingEffects};
use crate::ui::UiManager;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use blockloom_core::ui::*;
use blockloom_core::value::Evaluated;
use blockloom_core::vm::{Effect, Event};

/// Runs before either scheduler so both read the same widget sample.
pub fn bindings(
    mut manager: ResMut<UiManager>,
    engine: NonSend<Engine>,
    customs: Query<&CustomComponents>,
    mut effects: ResMut<PendingEffects>,
) {
    let pending = std::mem::take(&mut manager.binding_writes);
    for (binding, value) in pending.iter().cloned() {
        match binding.source {
            UiSource::Variable { actor, name } => engine.variables.write(&actor, &name, value),
            UiSource::Component {
                actor,
                component,
                field,
            } => effects.0.push(Effect::SetComponentField {
                actor,
                component,
                field,
                value,
            }),
        }
    }
    let mut writes = Vec::new();
    for id in manager.ids() {
        let Some(n) = manager.get(id) else {
            continue;
        };
        for binding in &n.bindings {
            let value = pending
                .iter()
                .rev()
                .find(|(b, _)| b.source == binding.source)
                .map(|(_, value)| value.clone())
                .or_else(|| match &binding.source {
                    UiSource::Variable { actor, name } => Some(engine.variables.read(actor, name)),
                    UiSource::Component {
                        actor,
                        component,
                        field,
                    } => engine
                        .entities
                        .get(actor)
                        .and_then(|entity| customs.get(*entity).ok())
                        .and_then(|c| c.0.get(component))
                        .and_then(|fields| fields.get(field))
                        .cloned(),
                });
            if let Some(value) = value {
                let value = binding.converter.read(value);
                let same = match binding.property {
                    UiProp::Text => crate::ui::text_of(n) == value.as_text(),
                    UiProp::Visible => n.visible == value.as_bool(),
                    UiProp::Value | UiProp::SelectedIndex => n.value == value,
                    _ => true,
                };
                if !same {
                    writes.push((id.clone(), binding.property, value));
                }
            }
        }
    }
    for (id, prop, value) in writes {
        manager.set(&id, prop, &value);
    }
    let sample = manager.senses();
    blockloom_core::sense::publish_ui(sample, manager.focus().unwrap_or_default().into());
}

/// Reuses a bounded set of ordinary button nodes as the viewport moves.
pub fn collections(
    mut manager: ResMut<UiManager>,
    mut scrolls: Query<(&mut ScrollPosition, &ComputedNode)>,
) {
    for id in manager.ids().to_vec() {
        let node = manager.get(&id).unwrap();
        if let Some(index) = node.tab_index {
            let active = manager.get(&node.parent).is_some_and(|parent| {
                parent.kind == UiKind::Tabs && parent.value.as_number() == Ok(index as f64)
            });
            let node = manager.get_mut(&id).unwrap();
            if node.tab_active != active {
                node.tab_active = active;
                node.dirty = true;
            }
        }
        let target = manager.get(&id).unwrap().scroll_target.clone();
        if let Some(owner) = manager.get(&target) {
            let height = owner.layout.as_ref().map_or(32., |l| l.row_height);
            let viewport = scrolls
                .get(owner.entity)
                .map_or(owner.spec.size[1].max(240.), |(_, n)| {
                    n.size().y * n.inverse_scale_factor()
                });
            let range = [0., (owner.items.len() as f32 * height - viewport).max(0.)];
            let value = Evaluated::Number(owner.scroll.clamp(0., range[1]) as f64);
            let node = manager.get_mut(&id).unwrap();
            if node.range != range || node.value != value {
                node.range = range;
                node.value = value;
                node.dirty = true;
            }
        }
    }
    let ids: Vec<String> = manager
        .ids()
        .iter()
        .filter(|id| manager.get(id).is_some_and(|n| n.kind.selectable()))
        .cloned()
        .collect();
    for id in ids {
        let n = manager.get(&id).unwrap();
        let height = n.layout.as_ref().map_or(32., |l| l.row_height).max(1.);
        let mut scroll = n.scroll;
        let mut viewport = n.style.height.unwrap_or(n.spec.size[1]).max(240.);
        if let Ok((mut position, computed)) = scrolls.get_mut(n.entity) {
            viewport = computed.size().y * computed.inverse_scale_factor();
            if n.scroll_dirty {
                position.y = n
                    .scroll
                    .min((n.items.len() as f32 * height - viewport).max(0.));
            }
            scroll = position.y;
        }

        let range = if n.kind == UiKind::Select && !n.expanded {
            0..0
        } else if matches!(n.kind, UiKind::Tabs | UiKind::Select) {
            0..n.items.len()
        } else {
            visible_rows(n.items.len(), scroll, viewport, height)
        };
        let count = range.len();
        let span = (range.start, range.end);
        let kind = n.kind;
        let total = n.items.len();
        if !n.collection_dirty && n.collection_range == Some(span) {
            let node = manager.get_mut(&id).unwrap();
            node.scroll = scroll;
            node.scroll_dirty = false;
            continue;
        }
        let selected = n.value.as_number().unwrap_or(0.) as usize;
        let theme = n.theme;
        let mut row_style = n.styles.clone();
        row_style.normal.background = None;
        let items: Vec<_> = range.clone().map(|i| n.items[i].clone()).collect();
        let node = manager.get_mut(&id).unwrap();
        node.collection_range = Some(span);
        node.collection_dirty = false;
        node.scroll = scroll;
        node.scroll_dirty = false;
        for (pool, index) in range.enumerate() {
            let row_id = format!("{id}::row:{pool}");
            let spec = UiElement {
                id: row_id.clone(),
                kind: UiKind::Button,
                content: items[pool].clone(),
                parent: id.clone(),
                anchor: UiAnchor::TopLeft,
                offset: [
                    0.,
                    (index + usize::from(kind == UiKind::Select)) as f32 * height,
                ],
                size: [0., height],
                ..Default::default()
            };
            manager.show(spec);
            let row = manager.get_mut(&row_id).unwrap();
            let layout = UiLayout {
                absolute: kind != UiKind::Tabs,
                width: if kind == UiKind::Tabs {
                    UiLength::Auto
                } else {
                    UiLength::Percent(100.)
                },
                ..Default::default()
            };
            if row.layout.as_ref() != Some(&layout) {
                row.layout = Some(layout);
                row.dirty = true;
            }
            row.value = Evaluated::Number((index + 1) as f64);
            let mut style = row_style.clone();
            if selected == index + 1 {
                style.normal = style.focused.over(&style.normal);
                style.normal.background.get_or_insert("#355882".into());
            }
            if row.styles != style || row.theme != theme {
                row.styles = style;
                row.theme = theme;
                row.dirty = true;
            }
        }
        let stale: Vec<String> = manager
            .ids()
            .iter()
            .filter(|key| key.starts_with(&format!("{id}::row:")))
            .filter(|key| {
                key.rsplit(':')
                    .next()
                    .and_then(|x| x.parse::<usize>().ok())
                    .is_some_and(|pool| pool >= count)
            })
            .cloned()
            .collect();
        for key in stale {
            manager.delete(&key);
        }
        if kind == UiKind::ListView {
            manager.show(UiElement {
                id: format!("{id}::extent"),
                parent: id.clone(),
                kind: UiKind::Spacer,
                size: [1., total as f32 * height],
                ..Default::default()
            });
        }
    }
}

pub fn project_widgets(
    mut manager: ResMut<UiManager>,
    cameras: Query<(&Camera, &GlobalTransform), With<crate::world::WorldCamera>>,
    actors: Query<(&ActorId, &GlobalTransform)>,
) {
    let Some((camera, transform)) = cameras.iter().next() else {
        return;
    };
    let scale = manager.canvas_scale;
    let origin = manager.canvas_origin;
    let ids = manager.ids().to_vec();
    for id in ids {
        let Some(node) = manager.get_mut(&id) else {
            continue;
        };
        if node.world_actor.is_empty() {
            continue;
        }
        let point = actors
            .iter()
            .find(|(actor, _)| actor.0 == node.world_actor)
            .and_then(|(_, at)| camera.world_to_viewport(transform, at.translation()).ok());
        let visible = point.is_some();
        if node.projected != visible {
            node.projected = visible;
            node.dirty = true;
        }
        if let Some(point) = point {
            let point = (point - origin) / scale.max(0.001);
            let offset = [point.x, point.y];
            if node.spec.offset != offset {
                node.spec.offset = offset;
                node.spec.anchor = UiAnchor::TopLeft;
                node.dirty = true;
            }
        }
    }
}

pub fn activate(manager: &mut UiManager, engine: &mut Engine, id: &str) {
    let Some(node) = manager.get(id).cloned() else {
        return;
    };
    if !node.enabled || !manager.shown(&node) {
        return;
    }
    if id.contains("::row:") {
        if let Some(value) = manager.changed(&node.parent, node.value.clone()) {
            engine.fire(Event::UiChanged {
                id: node.parent.clone(),
                value,
            });
        }
        if let Some(parent) = manager.get_mut(&node.parent) {
            parent.expanded = false;
            parent.collection_dirty = true;
        }
    } else if node.kind == UiKind::Toggle {
        if let Some(value) = manager.changed(id, Evaluated::Bool(!node.value.as_bool())) {
            engine.fire(Event::UiChanged {
                id: id.into(),
                value,
            });
        }
    } else if node.kind == UiKind::Select
        && !node.items.is_empty()
        && let Some(n) = manager.get_mut(id)
    {
        n.expanded = !n.expanded;
        n.collection_dirty = true;
    }
    manager.focus_on((node.kind == UiKind::Input).then_some(id));
    for target in manager.bubble(id) {
        engine.fire(Event::UiClicked { id: target });
    }
}

pub fn navigation(
    mut manager: ResMut<UiManager>,
    mut engine: NonSendMut<Engine>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    boxes: Query<(&ComputedNode, &UiGlobalTransform)>,
) {
    if !engine.running || manager.focus().is_some() {
        return;
    }
    let direction = if keys.just_pressed(KeyCode::ArrowDown)
        || keys.just_pressed(KeyCode::Tab)
        || pads.iter().any(|p| p.just_pressed(GamepadButton::DPadDown))
    {
        Vec2::Y
    } else if keys.just_pressed(KeyCode::ArrowUp)
        || pads.iter().any(|p| p.just_pressed(GamepadButton::DPadUp))
    {
        Vec2::NEG_Y
    } else if keys.just_pressed(KeyCode::ArrowLeft)
        || pads.iter().any(|p| p.just_pressed(GamepadButton::DPadLeft))
    {
        Vec2::NEG_X
    } else if keys.just_pressed(KeyCode::ArrowRight)
        || pads
            .iter()
            .any(|p| p.just_pressed(GamepadButton::DPadRight))
    {
        Vec2::X
    } else {
        Vec2::ZERO
    };
    if direction != Vec2::ZERO {
        let origin = manager
            .navigation
            .as_ref()
            .and_then(|id| manager.get(id))
            .and_then(|n| crate::world::screen_rect(&boxes, n.entity))
            .map(|r| r.center());
        let modal = manager
            .ids()
            .iter()
            .rev()
            .find(|id| manager.get(id).is_some_and(|n| n.modal && manager.shown(n)))
            .cloned();
        let candidates = manager.ids().iter().filter_map(|id| {
            let n = manager.get(id)?;
            if !n.kind.focusable() || !n.enabled || !manager.shown(n) {
                return None;
            }
            if modal
                .as_ref()
                .is_some_and(|m| !manager.bubble(id).contains(m))
            {
                return None;
            }
            let rect = crate::world::screen_rect(&boxes, n.entity)?;
            let score = if let Some(origin) = origin {
                let delta = rect.center() - origin;
                let along = delta.dot(direction);
                if along <= 0.1 {
                    return None;
                }
                along + (delta - direction * along).length() * 3.
            } else {
                0.
            };
            Some((id.clone(), score))
        });
        if let Some((id, _)) = candidates.min_by(|a, b| a.1.total_cmp(&b.1)) {
            if let Some(old) = manager.navigation.clone() {
                manager.mark_dirty(&old);
            }
            manager.navigation = Some(id.clone());
            manager.mark_dirty(&id);
            emit_event(&manager, &mut engine, &id, "focus");
        }
    }
    if (keys.just_pressed(KeyCode::Enter)
        || keys.just_pressed(KeyCode::Space)
        || pads.iter().any(|p| p.just_pressed(GamepadButton::South)))
        && let Some(id) = manager.navigation.clone()
    {
        activate(&mut manager, &mut engine, &id);
    }
}

pub fn hover(
    mut engine: NonSendMut<Engine>,
    mut captured: Local<Option<(String, Vec2)>>,
    mut manager: ResMut<UiManager>,
    windows: Query<&Window, With<PrimaryWindow>>,
    pointer: Option<Res<crate::preview::PreviewPointer>>,
    buttons: Res<ButtonInput<MouseButton>>,
    boxes: Query<(&ComputedNode, &UiGlobalTransform)>,
) {
    let window = windows.iter().next();
    let point = pointer
        .as_ref()
        .filter(|p| crate::preview::pointer_live(p))
        .and_then(|p| p.pos)
        .or_else(|| window.and_then(Window::cursor_position));
    if !engine.running {
        return;
    }
    let hit = point
        .and_then(|point| {
            manager.hit(point * window.map_or(1., Window::scale_factor), |n| {
                crate::world::screen_rect(&boxes, n.entity)
            })
        })
        .map(|n| n.spec.id.clone());
    let ids = manager.ids().to_vec();
    for id in ids {
        let n = manager.get_mut(&id).unwrap();
        let hovered = hit.as_ref() == Some(&id);
        let pressed = hovered && buttons.pressed(MouseButton::Left);
        let entered = n.hovered != hovered;
        if entered || n.pressed != pressed {
            n.hovered = hovered;
            n.pressed = pressed;
            n.dirty = true;
        }
        if entered {
            emit_event(
                &manager,
                &mut engine,
                &id,
                if hovered { "hover" } else { "leave" },
            );
        }
    }
    if buttons.just_pressed(MouseButton::Left)
        && let (Some(id), Some(point)) = (hit.clone(), point)
    {
        emit_event(&manager, &mut engine, &id, "press");
        *captured = Some((id, point));
    }
    if buttons.just_released(MouseButton::Left)
        && let Some((id, _)) = captured.take()
    {
        emit_event(&manager, &mut engine, &id, "release");
    }
    if buttons.pressed(MouseButton::Left)
        && let (Some((id, previous)), Some(point)) = (captured.as_mut(), point)
        && *previous != point
    {
        emit_event(&manager, &mut engine, id, "drag");
        if let Some(n) = manager.get(id).cloned()
            && matches!(n.kind, UiKind::Slider | UiKind::Scrollbar)
            && let Some(rect) = crate::world::screen_rect(&boxes, n.entity)
        {
            let physical = point * window.map_or(1., Window::scale_factor);
            let fraction = (physical.x - rect.min.x) / rect.width().max(1.);
            let value = crate::ui::slider_at(n.range, n.step(), fraction);
            if let Some(value) = manager.changed(id, Evaluated::Number(value)) {
                engine.fire(Event::UiChanged {
                    id: id.clone(),
                    value,
                });
            }
        }
        *previous = point;
    }
    let tooltip = hit
        .as_ref()
        .and_then(|id| manager.get(id))
        .filter(|n| !n.tooltip.is_empty())
        .map(|n| n.tooltip.clone());
    if let (Some(text), Some(point)) = (tooltip, point) {
        let point = (point - manager.canvas_origin) / manager.canvas_scale.max(0.01);
        manager.show(UiElement {
            id: "__tooltip".into(),
            kind: UiKind::Tooltip,
            content: text,
            anchor: UiAnchor::TopLeft,
            offset: [point.x + 16., point.y + 20.],
            ..Default::default()
        });
        if let Some(n) = manager.get_mut("__tooltip") {
            n.enabled = false;
        }
    } else if manager.get("__tooltip").is_some_and(|n| n.visible) {
        manager.hide("__tooltip", false);
    }
}

pub fn emit_event(manager: &UiManager, engine: &mut Engine, id: &str, event: &str) {
    for target in manager.bubble(id) {
        engine.fire(Event::UiEvent {
            id: target,
            event: event.into(),
        });
    }
}

pub fn canvas(
    mut manager: ResMut<UiManager>,
    cameras: Query<&Camera, With<crate::world::WorldCamera>>,
    ui_scale: Res<bevy::ui::UiScale>,
    insets: Option<Res<crate::ui::DeviceInsets>>,
    mut roots: Query<(&mut Node, &mut UiTransform), With<crate::ui::UiRoot>>,
) {
    let Some(size) = cameras
        .iter()
        .next()
        .and_then(Camera::logical_viewport_size)
    else {
        return;
    };
    let size = size / ui_scale.0.max(0.01);
    let insets = insets.map(|insets| insets.0).unwrap_or([0.; 4]);
    let [left, top, right, bottom] =
        blockloom_core::ui::effective_safe_area(manager.document.safe_area, insets);
    let available = (size - Vec2::new(left + right, top + bottom)).max(Vec2::ONE);
    let reference = Vec2::from_array(manager.document.reference_size).max(Vec2::ONE);
    let scale = if manager.document.scale == blockloom_core::ui::UiScale::ScaleWithSize {
        (available / reference).min_element().max(0.01)
    } else {
        1.
    };
    manager.canvas_scale = scale * ui_scale.0.max(0.01);
    manager.canvas_origin = Vec2::new(left, top) * ui_scale.0.max(0.01);
    let logical = available / scale;
    for (mut node, mut transform) in &mut roots {
        let wanted = Node {
            position_type: PositionType::Absolute,
            width: Val::Px(logical.x),
            height: Val::Px(logical.y),
            left: Val::Px(left + (scale - 1.) * logical.x * 0.5),
            top: Val::Px(top + (scale - 1.) * logical.y * 0.5),
            ..default()
        };
        if *node != wanted {
            *node = wanted;
        }
        if transform.scale != Vec2::splat(scale) {
            transform.scale = Vec2::splat(scale);
        }
    }
}

#[derive(Component)]
pub struct UiTween {
    pub start: Color,
    pub end: Color,
    pub elapsed: f32,
    pub duration: f32,
    pub width: (Val, Val),
    pub height: (Val, Val),
    pub hide: bool,
    pub show: bool,
}
pub fn animate(
    mut commands: Commands,
    time: Res<Time>,
    mut nodes: Query<(
        Entity,
        &mut UiTween,
        &mut BackgroundColor,
        &mut Node,
        &mut UiTransform,
    )>,
) {
    for (entity, mut tween, mut color, mut node, mut transform) in &mut nodes {
        tween.elapsed += time.delta_secs();
        let t = (tween.elapsed / tween.duration.max(0.001)).clamp(0., 1.);
        let eased = t * t * (3. - 2. * t);
        color.0 = tween.start.mix(&tween.end, eased);
        let lerp = |(from, to): (Val, Val)| match (from, to) {
            (Val::Px(a), Val::Px(b)) => Val::Px(a + (b - a) * eased),
            _ => to,
        };
        node.width = lerp(tween.width);
        node.height = lerp(tween.height);
        if tween.hide {
            transform.scale = Vec2::splat((1. - eased).max(0.001));
        } else if tween.show {
            transform.scale = Vec2::splat(eased.max(0.001));
        } else {
            transform.scale = Vec2::ONE;
        }
        if t >= 1. {
            if tween.hide {
                node.display = Display::None;
            }
            commands.entity(entity).remove::<UiTween>();
        }
    }
}

pub fn touch(
    mut touches: MessageReader<bevy::input::touch::TouchInput>,
    mut manager: ResMut<UiManager>,
    mut engine: NonSendMut<Engine>,
    boxes: Query<(&ComputedNode, &UiGlobalTransform)>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut capture: Local<std::collections::HashMap<u64, (String, Vec2, bool)>>,
) {
    use bevy::input::touch::TouchPhase;
    for touch in touches.read() {
        if !engine.running {
            continue;
        }
        let point = touch.position * windows.iter().next().map_or(1., Window::scale_factor);
        match touch.phase {
            TouchPhase::Started => {
                let hit = manager
                    .hit(point, |n| crate::world::screen_rect(&boxes, n.entity))
                    .map(|n| n.spec.id.clone());
                if let Some(id) = hit {
                    emit_event(&manager, &mut engine, &id, "press");
                    touch_slider(&mut manager, &mut engine, &boxes, &id, point);
                    capture.insert(touch.id, (id, point, false));
                }
            }
            TouchPhase::Moved => {
                if let Some((id, previous, dragged)) = capture.get_mut(&touch.id) {
                    *dragged |= previous.distance(point) > 3.;
                    emit_event(&manager, &mut engine, id, "drag");
                    let slider = touch_slider(&mut manager, &mut engine, &boxes, id, point);
                    let owner = (!slider)
                        .then(|| manager.scroll_owner(id).map(|n| n.spec.id.clone()))
                        .flatten();
                    if let Some(owner) = owner {
                        let scroll = manager.get(&owner).unwrap().scroll + previous.y - point.y;
                        manager.set(&owner, UiProp::Scroll, &Evaluated::Number(scroll as f64));
                    }
                    *previous = point;
                }
            }
            TouchPhase::Ended => {
                if let Some((id, _, dragged)) = capture.remove(&touch.id) {
                    emit_event(&manager, &mut engine, &id, "release");
                    if !dragged
                        && manager
                            .get(&id)
                            .and_then(|n| crate::world::screen_rect(&boxes, n.entity))
                            .is_some_and(|r| r.contains(point))
                    {
                        activate(&mut manager, &mut engine, &id);
                    }
                }
            }
            TouchPhase::Canceled => {
                capture.remove(&touch.id);
            }
        }
    }
}

fn touch_slider(
    manager: &mut UiManager,
    engine: &mut Engine,
    boxes: &Query<(&ComputedNode, &UiGlobalTransform)>,
    id: &str,
    point: Vec2,
) -> bool {
    let Some(node) = manager.get(id) else {
        return false;
    };
    if !matches!(node.kind, UiKind::Slider | UiKind::Scrollbar) {
        return false;
    }
    if let Some(rect) = crate::world::screen_rect(boxes, node.entity) {
        let fraction = (point.x - rect.min.x) / rect.width().max(1.);
        let value = crate::ui::slider_at(node.range, node.step(), fraction);
        if let Some(value) = manager.changed(id, Evaluated::Number(value)) {
            engine.fire(Event::UiChanged {
                id: id.into(),
                value,
            });
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> App {
        let mut app = App::new();
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(rx, blockloom_core::scene::Mode::TwoD);
        engine.running = true;
        app.insert_non_send(engine)
            .init_resource::<UiManager>()
            .init_resource::<PendingEffects>();
        app
    }
    #[test]
    fn tab_pages_follow_selection_without_overwriting_authored_visibility() {
        let mut app = app();
        app.add_systems(Update, collections);
        let mut m = app.world_mut().resource_mut::<UiManager>();
        m.show_widget(UiWidget {
            element: UiElement {
                id: "tabs".into(),
                kind: UiKind::Tabs,
                ..default()
            },
            items: vec!["First".into(), "Second".into()],
            ..default()
        });
        m.show_widget(UiWidget {
            element: UiElement {
                id: "page".into(),
                parent: "tabs".into(),
                ..default()
            },
            tab_index: Some(2),
            ..default()
        });
        app.update();
        let m = app.world().resource::<UiManager>();
        assert!(!m.shown(m.get("page").unwrap()));
        app.world_mut()
            .resource_mut::<UiManager>()
            .changed("tabs", Evaluated::Number(2.));
        app.update();
        let m = app.world().resource::<UiManager>();
        assert!(m.shown(m.get("page").unwrap()));
        app.world_mut()
            .resource_mut::<UiManager>()
            .hide("page", false);
        app.update();
        let m = app.world().resource::<UiManager>();
        assert!(!m.shown(m.get("page").unwrap()));
    }

    #[test]
    fn a_scrollbar_updates_its_list_and_follows_programmatic_scrolling() {
        let mut app = app();
        app.add_systems(Update, collections);
        let mut m = app.world_mut().resource_mut::<UiManager>();
        m.show_widget(UiWidget {
            element: UiElement {
                id: "list".into(),
                kind: UiKind::ListView,
                size: [200., 240.],
                ..default()
            },
            items: vec!["Item".into(); 100],
            ..default()
        });
        m.show_widget(UiWidget {
            element: UiElement {
                id: "scroll".into(),
                kind: UiKind::Scrollbar,
                ..default()
            },
            scroll_target: "list".into(),
            ..default()
        });
        app.update();
        app.world_mut()
            .resource_mut::<UiManager>()
            .changed("scroll", Evaluated::Number(320.));
        assert_eq!(
            app.world()
                .resource::<UiManager>()
                .get("list")
                .unwrap()
                .scroll,
            320.
        );
        app.world_mut().resource_mut::<UiManager>().set(
            "list",
            UiProp::Scroll,
            &Evaluated::Number(640.),
        );
        app.update();
        assert_eq!(
            app.world()
                .resource::<UiManager>()
                .get("scroll")
                .unwrap()
                .value,
            Evaluated::Number(640.)
        );
    }

    #[test]
    fn list_view_recycles_a_bounded_pool_and_maps_to_absolute_indices() {
        let mut app = app();
        app.add_systems(Update, collections);
        let mut m = app.world_mut().resource_mut::<UiManager>();
        m.show(UiElement {
            id: "inventory".into(),
            kind: UiKind::ListView,
            size: [200., 240.],
            ..Default::default()
        });
        m.get_mut("inventory").unwrap().items = (0..100000).map(|i| format!("item {i}")).collect();
        app.update();
        let m = app.world().resource::<UiManager>();
        assert!(m.ids().len() < 15);
        assert_eq!(m.get("inventory::row:0").unwrap().spec.content, "item 0");
        app.world_mut().resource_mut::<UiManager>().set(
            "inventory",
            UiProp::Scroll,
            &Evaluated::Number(32000.),
        );
        app.update();
        let m = app.world().resource::<UiManager>();
        assert!(m.ids().len() < 15);
        assert_eq!(m.get("inventory::row:0").unwrap().spec.content, "item 999");
        assert_eq!(
            m.get("inventory::row:0").unwrap().value,
            Evaluated::Number(1000.)
        );
    }
    #[test]
    fn two_way_variable_binding_writes_before_the_next_sample() {
        let mut app = app();
        app.add_systems(Update, bindings);
        app.world().non_send::<Engine>().variables.write(
            "actor",
            "name",
            Evaluated::Text("Ada".into()),
        );
        let mut m = app.world_mut().resource_mut::<UiManager>();
        m.show_widget(UiWidget {
            element: UiElement {
                id: "name".into(),
                kind: UiKind::Input,
                ..Default::default()
            },
            bindings: vec![UiBinding {
                property: UiProp::Value,
                source: UiSource::Variable {
                    actor: "actor".into(),
                    name: "name".into(),
                },
                converter: UiConverter::default(),
                two_way: true,
            }],
            ..Default::default()
        });
        app.update();
        assert_eq!(
            app.world()
                .resource::<UiManager>()
                .get("name")
                .unwrap()
                .value,
            Evaluated::Text("Ada".into())
        );
        app.world_mut()
            .resource_mut::<UiManager>()
            .changed("name", Evaluated::Text("Grace".into()));
        app.update();
        assert_eq!(
            app.world()
                .non_send::<Engine>()
                .variables
                .read("actor", "name"),
            Evaluated::Text("Grace".into())
        );
        assert_eq!(
            blockloom_core::sense::read(|s| s.ui["name"].value.clone()),
            Evaluated::Text("Grace".into())
        );
    }
    #[test]
    fn drawing_all_widget_kinds_has_disjoint_queries_and_retains_entities() {
        let mut app = app();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Image>();
        app.add_systems(Update, crate::overlay::draw_ui);
        for i in 0..23 {
            app.world_mut().resource_mut::<UiManager>().show(UiElement {
                id: format!("widget{i}"),
                kind: UiKind::from_index(i),
                content: "Hello [b]world[/b]".into(),
                ..Default::default()
            });
        }
        app.update();
        app.update();
        app.update();
        let count = app.world().entities().len();
        app.update();
        assert_eq!(app.world().entities().len(), count);
        assert_eq!(
            app.world_mut().resource_mut::<UiManager>().take_dirty(),
            Vec::<String>::new()
        );
    }
}

#[derive(Default)]
pub struct UiAtlas {
    sources: Vec<(String, Handle<Image>)>,
    sheet: Option<Handle<Image>>,
    rects: std::collections::HashMap<String, Rect>,
}
/// Packs loaded UI images once per source set; Bevy batches nodes sharing the sheet.
pub fn atlas(
    manager: Res<UiManager>,
    engine: NonSend<Engine>,
    server: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    mut nodes: Query<&mut ImageNode>,
    mut cache: Local<UiAtlas>,
) {
    let mut sources: Vec<(String, Handle<Image>)> = Vec::new();
    for id in manager.ids() {
        let Some(node) = manager.get(id).filter(|n| n.kind == UiKind::Image) else {
            continue;
        };
        let path = node.spec.content.trim();
        if path.is_empty() || sources.iter().any(|(p, _)| p == path) {
            continue;
        }
        let handle = server.load(crate::world::asset_path(
            engine.project_dir.as_deref(),
            path,
        ));
        if images.get(&handle).is_some_and(|image| {
            image.width() <= 512 && image.height() <= 512 && image.data.is_some()
        }) {
            sources.push((path.into(), handle));
        }
    }
    sources.sort_by(|a, b| a.0.cmp(&b.0));
    sources.truncate(64);
    if cache.sources != sources {
        let mut builder = bevy::image::TextureAtlasBuilder::default();
        builder.padding(UVec2::splat(2));
        for (_, handle) in &sources {
            builder.add_texture(Some(handle.id()), images.get(handle).unwrap());
        }
        let built = if sources.len() > 1 {
            builder.build().ok()
        } else {
            None
        };
        cache.rects.clear();
        cache.sheet = None;
        if let Some((layout, lookup, image)) = built {
            for (path, handle) in &sources {
                if let Some(rect) = lookup.texture_rect(&layout, handle.id()) {
                    cache.rects.insert(
                        path.clone(),
                        Rect::from_corners(rect.min.as_vec2(), rect.max.as_vec2()),
                    );
                }
            }
            cache.sheet = Some(images.add(image));
        }
        cache.sources = sources;
    }
    for id in manager.ids() {
        let Some(node) = manager.get(id).filter(|n| n.kind == UiKind::Image) else {
            continue;
        };
        let Ok(mut image) = nodes.get_mut(node.entity) else {
            continue;
        };
        let path = node.spec.content.trim();
        let (handle, rect) = match (&cache.sheet, cache.rects.get(path)) {
            (Some(sheet), Some(rect)) => (sheet.clone(), Some(*rect)),
            _ => (
                server.load(crate::world::asset_path(
                    engine.project_dir.as_deref(),
                    path,
                )),
                None,
            ),
        };
        if image.image != handle || image.rect != rect {
            image.image = handle;
            image.rect = rect;
        }
    }
}
