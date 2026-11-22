//! Authored screens as files. A `.blockui` holds one [`UiScreen`]; a scene
//! (or the whole game, through `GameSettings.screens`) lists the files it
//! draws as [`UiLayer`]s, and [`scene_interface`] composes them over the
//! scene's own interface into the flat document the runtime draws. One file
//! listed by three scenes is one menu, edited once.

use crate::scene::World;
use crate::settings::GameSettings;
use crate::ui::UiDocument;
use crate::ui_tree::{UI_TREE_VERSION, UiScreen};
use blockloom_schema_derive::Schema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

pub const EXTENSION: &str = "blockui";

/// One screen a scene draws.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Schema)]
pub struct UiLayer {
    /// The `.blockui` file, relative to the project folder.
    #[schema(asset)]
    pub screen: String,
    /// Shown when the scene starts; a closed one waits to be opened.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub open: bool,
}

fn yes() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

impl UiLayer {
    pub fn new(screen: impl Into<String>) -> Self {
        Self {
            screen: screen.into(),
            open: true,
        }
    }
}

pub fn is_screen(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case(EXTENSION))
}

pub fn read(dir: &Path, relative: &str) -> Result<UiScreen, String> {
    let full = crate::assets::resolve(dir, relative)
        .ok_or_else(|| format!("\"{relative}\" isn't a path in this project"))?;
    let text = crate::vfs::read_to_string(&full).map_err(|e| format!("{relative}: {e}"))?;
    let screen: UiScreen = serde_json::from_str(&text).map_err(|e| format!("{relative}: {e}"))?;
    if screen.version > UI_TREE_VERSION {
        return Err(format!(
            "{relative} was saved by a newer Blockloom (screen format {})",
            screen.version
        ));
    }
    Ok(screen)
}

pub fn write(dir: &Path, relative: &str, screen: &UiScreen) -> Result<(), String> {
    let full = crate::assets::resolve(dir, relative)
        .ok_or_else(|| format!("\"{relative}\" isn't a path in this project"))?;
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut screen = screen.clone();
    screen.version = UI_TREE_VERSION;
    let json = serde_json::to_string_pretty(&screen).map_err(|e| e.to_string())?;
    crate::files::write_atomic(&full, json)
}

/// A scene's interface with its screens laid over it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Composed {
    pub document: UiDocument,
    /// Root widgets of closed layers, hidden at the start.
    pub closed: Vec<String>,
    /// Layers that couldn't be drawn, each with why. The rest still are.
    pub errors: Vec<String>,
}

/// The layers a scene draws, its own first and then the game's, each file
/// once.
pub fn layers<'a>(world: &'a World, settings: &'a GameSettings) -> Vec<&'a UiLayer> {
    let mut seen = HashSet::new();
    world
        .screens
        .iter()
        .chain(&settings.screens)
        .filter(|layer| seen.insert(layer.screen.as_str()))
        .collect()
}

/// The document the runtime draws for a scene. Without a folder only the
/// scene's own interface is drawn.
pub fn scene_interface(dir: Option<&Path>, world: &World, settings: &GameSettings) -> Composed {
    let Some(dir) = dir else {
        return Composed {
            document: world.interface.clone(),
            ..Composed::default()
        };
    };
    let mut out = Composed::default();
    out.document = match world.interface.with_stylesheets(dir) {
        Ok(document) => document,
        Err(message) => {
            out.errors.push(message);
            world.interface.clone()
        }
    };
    let screens: Vec<(&UiLayer, Result<UiDocument, String>)> = layers(world, settings)
        .into_iter()
        .map(|layer| {
            let document = read(dir, &layer.screen)
                .and_then(|screen| screen.lower())
                .and_then(|doc| doc.with_stylesheets(dir))
                .map_err(|e| format!("{}: {e}", layer.screen));
            (layer, document)
        })
        .collect();
    let mut first = world.interface.widgets.is_empty();
    for (layer, document) in screens {
        let merged = document.and_then(|doc| {
            merge(&mut out.document, doc, first).map_err(|e| format!("{}: {e}", layer.screen))
        });
        match merged {
            Ok(roots) => {
                first = false;
                if !layer.open {
                    out.closed.extend(roots);
                }
            }
            Err(message) => out.errors.push(message),
        }
    }
    out
}

/// Appends `layer` to `base`, all or nothing. Answers the layer's root ids.
/// With `adopt`, the layer's theme and scale stand in for an empty base's.
fn merge(base: &mut UiDocument, layer: UiDocument, adopt: bool) -> Result<Vec<String>, String> {
    let ids: HashSet<&str> = base.widgets.iter().map(|w| w.element.id.as_str()).collect();
    if let Some(clash) = layer
        .widgets
        .iter()
        .find(|w| ids.contains(w.element.id.as_str()))
    {
        return Err(format!(
            "the widget id \"{}\" is already taken",
            clash.element.id
        ));
    }
    fn agree<V: PartialEq>(
        what: &str,
        base: &BTreeMap<String, V>,
        layer: &BTreeMap<String, V>,
    ) -> Result<(), String> {
        match layer
            .iter()
            .find(|(k, v)| base.get(*k).is_some_and(|b| b != *v))
        {
            Some((name, _)) => Err(format!(
                "its {what} \"{name}\" differs from one already drawn"
            )),
            None => Ok(()),
        }
    }
    agree("style", &base.styles, &layer.styles)?;
    agree("component", &base.prefabs, &layer.prefabs)?;
    agree("animation", &base.animations, &layer.animations)?;
    let roots = layer
        .widgets
        .iter()
        .filter(|w| w.element.parent.is_empty())
        .map(|w| w.element.id.clone())
        .collect();
    if adopt {
        base.theme = layer.theme;
        base.scale = layer.scale;
        base.reference_size = layer.reference_size;
        base.safe_area = layer.safe_area;
    }
    base.widgets.extend(layer.widgets);
    base.styles.extend(layer.styles);
    base.prefabs.extend(layer.prefabs);
    base.exports.extend(layer.exports);
    base.animations.extend(layer.animations);
    Ok(roots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{UiAnchor, UiKind};
    use crate::ui_tree::{LayoutItem, Placement, UiNode, Widget};

    fn menu(name: &str) -> UiScreen {
        UiScreen {
            nodes: vec![UiNode {
                name: name.into(),
                widget: Widget::blank(UiKind::VerticalBox),
                place: LayoutItem {
                    mode: Placement::at(UiAnchor::Center, [0., 0.]),
                    ..LayoutItem::default()
                },
                children: vec![UiNode {
                    name: format!("{name}-resume"),
                    widget: Widget::Button {
                        caption: "Resume".into(),
                    },
                    ..UiNode::default()
                }],
                ..UiNode::default()
            }],
            ..UiScreen::default()
        }
    }

    struct Scratch(std::path::PathBuf);
    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("blockloom-ui-layers-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn ids(doc: &UiDocument) -> Vec<&str> {
        doc.widgets.iter().map(|w| w.element.id.as_str()).collect()
    }

    #[test]
    fn three_scenes_share_one_menu_file() {
        let dir = Scratch::new("three");
        write(dir.path(), "assets/ui/pause.blockui", &menu("pause")).unwrap();
        let mut scenes = vec![World::default(), World::default(), World::default()];
        for world in &mut scenes {
            world.screens.push(UiLayer::new("assets/ui/pause.blockui"));
        }
        let settings = GameSettings::default();
        let draw = |scenes: &[World]| -> Vec<Composed> {
            scenes
                .iter()
                .map(|w| scene_interface(Some(dir.path()), w, &settings))
                .collect()
        };
        for composed in draw(&scenes) {
            assert!(composed.errors.is_empty(), "{:?}", composed.errors);
            assert_eq!(ids(&composed.document), ["pause", "pause-resume"]);
        }
        // One edit to the file reaches every scene.
        let mut edited = menu("pause");
        edited.nodes[0].children[0].widget = Widget::Button {
            caption: "Back".into(),
        };
        write(dir.path(), "assets/ui/pause.blockui", &edited).unwrap();
        for composed in draw(&scenes) {
            assert_eq!(composed.document.widgets[1].element.content, "Back");
        }
    }

    #[test]
    fn a_layer_sits_over_the_scene_and_a_bad_one_is_skipped() {
        let dir = Scratch::new("over");
        write(dir.path(), "assets/ui/hud.blockui", &menu("hud")).unwrap();
        write(dir.path(), "assets/ui/pause.blockui", &menu("pause")).unwrap();
        let mut world = World::default();
        world.interface = menu("own").lower().unwrap();
        world.screens = vec![
            UiLayer::new("assets/ui/hud.blockui"),
            UiLayer::new("assets/ui/missing.blockui"),
            // Clashes with the scene's own ids.
            UiLayer::new("assets/ui/own.blockui"),
        ];
        write(dir.path(), "assets/ui/own.blockui", &menu("own")).unwrap();
        let mut settings = GameSettings::default();
        settings.screens = vec![
            UiLayer {
                open: false,
                ..UiLayer::new("assets/ui/pause.blockui")
            },
            // Listed by the scene too: drawn once.
            UiLayer::new("assets/ui/hud.blockui"),
        ];
        let composed = scene_interface(Some(dir.path()), &world, &settings);
        assert_eq!(
            ids(&composed.document),
            [
                "own",
                "own-resume",
                "hud",
                "hud-resume",
                "pause",
                "pause-resume"
            ]
        );
        assert_eq!(composed.closed, ["pause"]);
        assert_eq!(composed.errors.len(), 2, "{:?}", composed.errors);
        assert!(composed.errors[0].contains("missing.blockui"));
        assert!(composed.errors[1].contains("already taken"));
    }

    #[test]
    fn a_layer_round_trips_and_stays_off_an_empty_scene_file() {
        let layer = UiLayer::new("assets/ui/a.blockui");
        assert_eq!(
            serde_json::to_string(&layer).unwrap(),
            r#"{"screen":"assets/ui/a.blockui"}"#
        );
        let world = World::default();
        let list = crate::scene_components::SceneComponents::from_world(&world);
        assert!(!list.0.iter().any(|c| c.name() == "Screens"));
        let mut world = world;
        world.screens.push(layer);
        let list = crate::scene_components::SceneComponents::from_world(&world);
        assert_eq!(list.to_world().screens, world.screens);
    }
}
