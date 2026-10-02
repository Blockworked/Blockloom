//! Temporary authored UI previews using the player's retained widget pipeline.
use crate::{bridge, engine::Engine, ui::UiManager};
use bevy::prelude::*;
use bevy::ui::{CalculatedClip, ComputedStackIndex};
use blockloom_protocol::{
    InterfaceClip, InterfaceDesign, InterfaceLayout, InterfaceWidgetBounds, RuntimeMessage,
};

#[derive(Resource, Default)]
pub(crate) struct DesignSession {
    request: Option<InterfaceDesign>,
    pub(crate) last: Option<InterfaceLayout>,
    viewport_before: Option<bevy::window::WindowResolution>,
}

impl DesignSession {
    pub fn active(&self) -> bool {
        self.request.is_some()
    }

    pub fn clear(&mut self) {
        self.request = None;
        self.last = None;
    }

    pub fn apply(
        &mut self,
        request: Option<InterfaceDesign>,
        engine: &Engine,
        manager: &mut UiManager,
    ) -> Result<(), String> {
        if engine.running || engine.starting {
            return Err("Stop the game before opening an interface design session".into());
        }
        let Some(request) = request else {
            self.clear();
            manager.clear();
            return Ok(());
        };
        request.validate()?;
        if self.request.as_ref().is_some_and(|old| {
            (request.generation, request.revision) <= (old.generation, old.revision)
        }) {
            return Err("Stale interface design revision or viewport generation".into());
        }
        let document = match engine.project_dir.as_deref() {
            Some(dir) => request.document.with_stylesheets(dir)?,
            None => request.document.clone(),
        };
        manager.load(&document);
        if let Some(screen) = &request.screen {
            for widget in &document.widgets {
                if widget.element.parent.is_empty() && widget.element.id != *screen {
                    manager.set(
                        &widget.element.id,
                        blockloom_core::ui::UiProp::Visible,
                        &blockloom_core::value::Evaluated::Bool(false),
                    );
                }
            }
        }
        self.request = Some(request);
        self.last = None;
        Ok(())
    }
}

/// Restore the process window when the design viewport releases it.
pub(crate) fn resize(
    mut session: ResMut<DesignSession>,
    mut windows: Query<&mut Window, With<bevy::window::PrimaryWindow>>,
) {
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    if let Some(size) = session.request.as_ref().and_then(|v| v.viewport) {
        if session.viewport_before.is_none() {
            session.viewport_before = Some(window.resolution.clone());
        }
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(size[0], size[1]);
    } else if let Some(resolution) = session.viewport_before.take() {
        window.resolution = resolution;
    }
}

pub(crate) fn inactive(session: Option<Res<DesignSession>>) -> bool {
    session.is_none_or(|s| !s.active())
}

/// Runs after Bevy's layout, clipping and paint-order passes.
pub(crate) fn report(
    mut session: ResMut<DesignSession>,
    manager: Res<UiManager>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    cameras: Query<(&Camera, Has<bevy::ui::IsDefaultUiCamera>)>,
    nodes: Query<(
        &ComputedNode,
        &UiGlobalTransform,
        &ComputedStackIndex,
        Option<&CalculatedClip>,
    )>,
) {
    let Some(request) = &session.request else {
        return;
    };
    let mut widgets = Vec::with_capacity(request.document.widgets.len());
    for widget in &request.document.widgets {
        let Some(node) = manager.get(&widget.element.id) else {
            return;
        };
        let Ok((computed, transform, order, clip)) = nodes.get(node.entity) else {
            return;
        };
        let clips = match clip {
            Some(CalculatedClip::Rects(rects)) => rects
                .iter()
                .map(|clip| InterfaceClip {
                    rect: [
                        clip.rect.min.x,
                        clip.rect.min.y,
                        clip.rect.max.x,
                        clip.rect.max.y,
                    ],
                    viewport_to_local: clip.world_to_clip_local.to_cols_array(),
                })
                .collect(),
            _ => Vec::new(),
        };
        widgets.push(InterfaceWidgetBounds {
            id: widget.element.id.clone(),
            size: computed.size().to_array(),
            transform: transform.to_cols_array(),
            visible: manager.shown(node)
                && computed.size().cmpgt(Vec2::ZERO).all()
                && !matches!(clip, Some(CalculatedClip::FullyClipped)),
            paint_order: order.0,
            clips,
        });
    }
    let viewport = cameras
        .iter()
        .filter(|(camera, default_ui)| camera.is_active && *default_ui)
        .find_map(|(camera, _)| camera.physical_viewport_size())
        .map(|v| v.to_array())
        .or_else(|| {
            windows
                .single()
                .ok()
                .map(|w| [w.physical_width(), w.physical_height()])
        });
    let Some(viewport) = viewport else {
        return;
    };
    let layout = InterfaceLayout {
        viewport,
        revision: request.revision,
        generation: request.generation,
        widgets,
    };
    if session.last.as_ref() != Some(&layout) {
        bridge::send(&RuntimeMessage::InterfaceLayout(layout.clone()));
        session.last = Some(layout);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::scene::Mode;

    fn fixture(revision: u64) -> InterfaceDesign {
        InterfaceDesign {
            screen: None,
            viewport: Some([960, 720]),
            revision,
            generation: 1,
            document: serde_json::from_str(include_str!(
                "../tests/fixtures/interface/design-spike.json"
            ))
            .unwrap(),
        }
    }

    #[test]
    fn drafts_are_temporary_atomic_and_revision_checked() {
        let (_sender, receiver) = std::sync::mpsc::channel();
        let mut engine = Engine::new(receiver, Mode::TwoD);
        let project = engine.project.clone();
        let mut manager = UiManager::default();
        let mut session = DesignSession::default();
        session
            .apply(Some(fixture(2)), &engine, &mut manager)
            .unwrap();
        assert_eq!(engine.project, project);
        assert!(!engine.running && !engine.starting);
        assert_eq!(manager.ids().len(), 6);
        assert!(
            session
                .apply(Some(fixture(1)), &engine, &mut manager)
                .is_err()
        );
        let mut too_small = fixture(3);
        too_small.viewport = Some([8, 720]);
        assert!(
            session
                .apply(Some(too_small), &engine, &mut manager)
                .is_err()
        );
        let mut invalid = fixture(3);
        invalid.document.widgets[1].element.parent = "missing".into();
        assert!(session.apply(Some(invalid), &engine, &mut manager).is_err());
        assert_eq!(session.request.as_ref().unwrap().revision, 2);
        assert_eq!(manager.document, fixture(2).document);
        engine.running = true;
        assert!(
            session
                .apply(Some(fixture(3)), &engine, &mut manager)
                .is_err()
        );
        engine.running = false;
        session.apply(None, &engine, &mut manager).unwrap();
        assert!(!session.active());
        assert!(manager.ids().is_empty());
        assert_eq!(engine.project, project);
    }

    #[test]
    fn isolation_preserves_document_and_rejects_invalid_screens_atomically() {
        let (_sender, receiver) = std::sync::mpsc::channel();
        let engine = Engine::new(receiver, Mode::TwoD);
        let mut manager = UiManager::default();
        let mut session = DesignSession::default();
        let mut design = fixture(1);
        let mut other = design.document.widgets[0].clone();
        other.element.id = "other".into();
        design.document.widgets.push(other);
        design.screen = Some("other".into());
        let document = design.document.clone();
        session
            .apply(Some(design.clone()), &engine, &mut manager)
            .unwrap();
        assert_eq!(manager.document, document);
        assert!(manager.shown(manager.get("other").unwrap()));
        assert!(!manager.shown(manager.get("screen").unwrap()));
        assert!(!manager.shown(manager.get("image").unwrap()));
        for id in ["missing", "nested", ""] {
            let mut invalid = design.clone();
            invalid.revision = 2;
            invalid.screen = Some(id.into());
            assert!(session.apply(Some(invalid), &engine, &mut manager).is_err());
            assert_eq!(session.request.as_ref().unwrap(), &design);
            assert!(!manager.shown(manager.get("image").unwrap()));
        }
        design.revision = 2;
        design.screen = None;
        session.apply(Some(design), &engine, &mut manager).unwrap();
        assert_eq!(manager.document, document);
        assert!(manager.shown(manager.get("image").unwrap()));
        assert!(manager.shown(manager.get("other").unwrap()));
    }

    #[test]
    fn design_resizes_and_restores_the_process_window() {
        let mut app = App::new();
        let mut session = DesignSession::default();
        session.request = Some(fixture(1));
        app.insert_resource(session);
        let original = Window::default();
        let resolution = original.resolution.clone();
        let window = app
            .world_mut()
            .spawn((original, bevy::window::PrimaryWindow))
            .id();
        app.add_systems(Update, resize);
        app.update();
        assert_eq!(
            app.world().get::<Window>(window).unwrap().physical_width(),
            960
        );
        assert_eq!(
            app.world()
                .get::<Window>(window)
                .unwrap()
                .resolution
                .scale_factor(),
            1.0
        );
        app.world_mut().resource_mut::<DesignSession>().clear();
        app.update();
        assert_eq!(
            app.world().get::<Window>(window).unwrap().resolution,
            resolution
        );
    }

    #[test]
    fn reported_geometry_comes_from_player_layout_without_starting_gameplay() {
        let (_sender, receiver) = std::sync::mpsc::channel();
        let engine = Engine::new(receiver, Mode::TwoD);
        let mut manager = UiManager::default();
        let mut session = DesignSession::default();
        let mut design = fixture(7);
        let mut other = design.document.widgets[0].clone();
        other.element.id = "other".into();
        design.document.widgets.push(other);
        session
            .apply(Some(design.clone()), &engine, &mut manager)
            .unwrap();
        let mut app = App::new();
        app.add_plugins((
            bevy::asset::AssetPlugin::default(),
            bevy::time::TimePlugin,
            bevy::text::TextPlugin,
            bevy::ui::UiPlugin,
        ));
        app.init_resource::<bevy::picking::hover::HoverMap>();
        app.init_resource::<bevy::picking::events::PointerState>();
        app.add_message::<bevy::picking::pointer::PointerInput>();
        app.add_message::<bevy::picking::backend::PointerHits>();
        app.init_resource::<ButtonInput<MouseButton>>();
        app.init_resource::<bevy::input::touch::Touches>();
        app.init_asset::<Image>();
        app.init_asset::<TextureAtlasLayout>();
        app.insert_non_send(engine);
        app.insert_resource(manager);
        app.insert_resource(session);
        app.world_mut()
            .spawn((Window::default(), bevy::window::PrimaryWindow));
        app.world_mut().spawn((
            Camera2d,
            Camera::default(),
            Transform::default(),
            GlobalTransform::default(),
            crate::world::WorldCamera,
        ));
        app.add_systems(
            Update,
            (
                crate::ui_systems::canvas,
                crate::ui_systems::collections,
                crate::overlay::draw_ui,
            )
                .chain(),
        );
        app.add_systems(
            PostUpdate,
            report
                .after(bevy::ui::UiSystems::PostLayout)
                .after(bevy::ui::UiSystems::Stack),
        );
        for _ in 0..6 {
            app.update();
        }
        let layout = app
            .world()
            .resource::<DesignSession>()
            .last
            .clone()
            .unwrap();
        assert_eq!((layout.revision, layout.generation), (7, 1));
        assert_eq!(layout.widgets.len(), 7);
        for widget in &layout.widgets {
            let entity = app
                .world()
                .resource::<UiManager>()
                .get(&widget.id)
                .unwrap()
                .entity;
            let node = app.world().get::<ComputedNode>(entity).unwrap();
            let transform = app.world().get::<UiGlobalTransform>(entity).unwrap();
            assert_eq!(widget.size, node.size().to_array());
            assert_eq!(widget.transform, transform.to_cols_array());
            assert!(widget.size[0] > 0. && widget.size[1] > 0., "{}", widget.id);
        }
        let find = |id: &str| layout.widgets.iter().find(|w| w.id == id).unwrap();
        assert!(find("slider").transform[5] > find("nested").transform[5]);
        assert!(find("list").paint_order > find("slider").paint_order);
        design.revision = 8;
        design.screen = Some("screen".into());
        app.world_mut()
            .resource_scope(|world, mut session: Mut<DesignSession>| {
                world.resource_scope(|world, mut manager: Mut<UiManager>| {
                    session
                        .apply(
                            Some(design.clone()),
                            world.non_send::<Engine>(),
                            &mut manager,
                        )
                        .unwrap();
                });
            });
        for _ in 0..6 {
            app.update();
        }
        let isolated = app
            .world()
            .resource::<DesignSession>()
            .last
            .as_ref()
            .unwrap();
        assert_eq!(isolated.revision, 8);
        for before in &layout.widgets {
            let after = isolated.widgets.iter().find(|w| w.id == before.id).unwrap();
            if before.id == "other" {
                assert!(!after.visible);
            } else {
                assert_eq!(after.size, before.size);
                assert_eq!(after.transform, before.transform);
                assert_eq!(after.visible, before.visible);
            }
        }
        // Preserve a measured box while moving it into a padded Canvas.
        let before = find("nested").clone();
        let parent = find("other");
        let manager = app.world().resource::<UiManager>();
        let scale = parent.transform[0];
        let origin = manager.canvas_origin;
        let logical_size = before.size;
        design
            .document
            .widgets
            .iter_mut()
            .find(|w| w.element.id == "other")
            .unwrap()
            .element
            .kind = blockloom_core::ui::UiKind::Canvas;
        design
            .document
            .apply_edit(&blockloom_core::ui::UiEdit::Reparent {
                id: "nested".into(),
                parent: "other".into(),
                placement: blockloom_core::ui::UiPlacement::Free {
                    offset: [
                        (before.transform[4] - parent.transform[4]) / scale
                            + (parent.size[0] - before.size[0]) / 2.,
                        (before.transform[5] - parent.transform[5]) / scale
                            + (parent.size[1] - before.size[1]) / 2.,
                    ],
                    size: logical_size,
                },
            })
            .unwrap();
        design.revision = 9;
        design.screen = None;
        app.world_mut()
            .resource_scope(|world, mut session: Mut<DesignSession>| {
                world.resource_scope(|world, mut manager: Mut<UiManager>| {
                    session
                        .apply(
                            Some(design.clone()),
                            world.non_send::<Engine>(),
                            &mut manager,
                        )
                        .unwrap();
                });
            });
        for _ in 0..6 {
            app.update();
        }
        let after = app
            .world()
            .resource::<DesignSession>()
            .last
            .as_ref()
            .unwrap()
            .widgets
            .iter()
            .find(|w| w.id == "nested")
            .unwrap();
        assert_eq!(after.size, before.size);
        for (a, b) in after.transform.iter().zip(before.transform.iter()) {
            assert!((a - b).abs() < 0.01, "{after:?} vs {before:?}");
        }
        design
            .document
            .apply_edit(&blockloom_core::ui::UiEdit::Reparent {
                id: "nested".into(),
                parent: String::new(),
                placement: blockloom_core::ui::UiPlacement::Free {
                    offset: [
                        (before.transform[4] - origin.x) / scale - before.size[0] / 2.,
                        (before.transform[5] - origin.y) / scale - before.size[1] / 2.,
                    ],
                    size: logical_size,
                },
            })
            .unwrap();
        design.revision = 10;
        app.world_mut()
            .resource_scope(|world, mut session: Mut<DesignSession>| {
                world.resource_scope(|world, mut manager: Mut<UiManager>| {
                    session
                        .apply(
                            Some(design.clone()),
                            world.non_send::<Engine>(),
                            &mut manager,
                        )
                        .unwrap();
                });
            });
        for _ in 0..6 {
            app.update();
        }
        let root = app
            .world()
            .resource::<DesignSession>()
            .last
            .as_ref()
            .unwrap()
            .widgets
            .iter()
            .find(|w| w.id == "nested")
            .unwrap();
        assert_eq!(root.size, before.size);
        for (a, b) in root.transform.iter().zip(before.transform.iter()) {
            assert!((a - b).abs() < 0.01, "{root:?} vs {before:?}");
        }
        assert!(!app.world().non_send::<Engine>().running);
    }
}
