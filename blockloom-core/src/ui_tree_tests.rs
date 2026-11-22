use super::*;
use crate::ui::{UiAnchor, UiKind, UiLayout, UiLength};

fn widget(id: &str, kind: UiKind, parent: &str) -> UiWidget {
    let mut w = UiWidget::default();
    w.element.id = id.into();
    w.element.kind = kind;
    w.element.parent = parent.into();
    w.element.value = UiElement::blank(kind, false);
    w
}

/// The same document with each widget's subtree after it, which is the
/// order a tree writes back.
fn preorder(doc: &UiDocument) -> UiDocument {
    let mut out = doc.clone();
    let order = |widgets: &[UiWidget]| -> Vec<UiWidget> {
        fn visit(parent: &str, widgets: &[UiWidget], out: &mut Vec<UiWidget>) {
            for w in widgets.iter().filter(|w| w.element.parent == parent) {
                out.push(w.clone());
                visit(&w.element.id, widgets, out);
            }
        }
        let mut list = Vec::new();
        visit("", widgets, &mut list);
        list
    };
    out.widgets = order(&doc.widgets);
    for list in out.prefabs.values_mut() {
        *list = order(list);
    }
    out.version = crate::ui::UI_SCHEMA_VERSION;
    out
}

fn round_trip(doc: &UiDocument) {
    let screen = UiScreen::from_document(doc);
    let back = screen.lower().expect("lowers");
    assert_eq!(back, preorder(doc));
    // And through its own file format.
    let text = serde_json::to_string(&screen).unwrap();
    let read: UiScreen = serde_json::from_str(&text).unwrap();
    assert_eq!(read, screen);
}

/// A menu of every kind, in a stack, a canvas, a grid and tabs.
fn everything() -> UiDocument {
    let mut doc = UiDocument::default();
    let mut menu = widget("menu", UiKind::Panel, "");
    menu.element.content = "Paused".into();
    menu.element.modal = true;
    menu.element.anchor = UiAnchor::Center;
    menu.element.size = [320., 0.];
    doc.widgets.push(menu);
    for (i, kind) in (0..23).map(UiKind::from_index).enumerate() {
        let mut w = widget(&format!("w{i}"), kind, "menu");
        w.element.content = format!("content {i}");
        w.element.value = UiElement::blank(kind, i % 2 == 0);
        if kind == UiKind::Slider {
            w.element.range = [1., 9.];
            w.element.value = Evaluated::Number(4.);
        }
        if kind.selectable() {
            w.items = vec!["One".into(), "Two".into()];
        }
        if kind == UiKind::Scrollbar {
            w.scroll_target = "w7".into();
        }
        doc.widgets.push(w);
    }
    let mut canvas = widget("canvas", UiKind::Canvas, "");
    canvas.element.anchor = UiAnchor::TopLeft;
    doc.widgets.push(canvas);
    let mut pinned = widget("pinned", UiKind::Label, "canvas");
    pinned.element.anchor = UiAnchor::BottomRight;
    pinned.element.offset = [-8., -8.];
    doc.widgets.push(pinned);
    let mut grid = widget("grid", UiKind::Grid, "menu");
    grid.layout = Some(UiLayout {
        columns: 4,
        gap: 2.,
        ..UiLayout::default()
    });
    doc.widgets.push(grid);
    for i in 0..3 {
        let mut cell = widget(&format!("cell{i}"), UiKind::Image, "grid");
        cell.layout = Some(UiLayout {
            width: UiLength::Percent(50.),
            grow: 1.,
            ..UiLayout::default()
        });
        doc.widgets.push(cell);
    }
    let mut tabs = widget("tabs", UiKind::Tabs, "");
    tabs.items = vec!["Audio".into(), "Video".into()];
    doc.widgets.push(tabs);
    for page in 0..2 {
        let mut body = widget(&format!("page{page}"), UiKind::VerticalBox, "tabs");
        body.tab_index = Some(page);
        body.element.anchor = UiAnchor::Top;
        body.element.offset = [0., 40.];
        body.layout = Some(UiLayout {
            absolute: true,
            ..UiLayout::default()
        });
        doc.widgets.push(body);
    }
    // Pinned at the top level too, and a flow child that named an offset.
    let mut hud = widget("hud", UiKind::Label, "");
    hud.element.anchor = UiAnchor::Right;
    hud.element.offset = [5., 6.];
    hud.layout = Some(UiLayout {
        absolute: true,
        ..UiLayout::default()
    });
    doc.widgets.push(hud);
    let mut stray = widget("stray", UiKind::Button, "menu");
    stray.element.offset = [3., 4.];
    stray.element.anchor = UiAnchor::Left;
    stray.element.range = [2., 3.];
    stray.world_actor = "Hero".into();
    stray.tooltip = "Press".into();
    doc.widgets.push(stray);
    doc
}

#[test]
fn an_old_document_comes_back_unchanged() {
    round_trip(&everything());
    round_trip(&UiDocument::default());
}

#[test]
fn prefabs_become_components_with_their_exports() {
    let mut doc = everything();
    doc.prefabs.insert(
        "card".into(),
        vec![
            widget("card", UiKind::Panel, ""),
            widget("title", UiKind::Label, "card"),
        ],
    );
    doc.exports.insert(
        "card".into(),
        vec![UiExport {
            name: "heading".into(),
            widget: "title".into(),
            path: "element.content".into(),
            slot: false,
        }],
    );
    let mut instance = widget("card-1", UiKind::Panel, "menu");
    instance.instance_of = "card".into();
    doc.widgets.push(instance);
    round_trip(&doc);
    let screen = UiScreen::from_document(&doc);
    assert_eq!(screen.components["card"].nodes[0].children[0].name, "title");
    assert_eq!(screen.components["card"].exports[0].name, "heading");
}

#[test]
fn the_tree_splits_placement_from_layout() {
    let screen = UiScreen::from_document(&everything());
    let find = |name: &str| screen.walk().find(|n| n.name == name).unwrap();
    // A menu child flows; a canvas child and a root hang off anchors.
    assert_eq!(find("w1").place.mode, Placement::Flow);
    assert_eq!(
        find("pinned").place.mode.point(),
        Some((UiAnchor::BottomRight, [-8., -8.]))
    );
    assert_eq!(find("menu").place.mode.point().unwrap().0, UiAnchor::Center);
    // A pinned page sits at its parent's top left whatever it named.
    assert_eq!(
        find("page1").place.mode.point(),
        Some((UiAnchor::TopLeft, [0., 40.]))
    );
    assert_eq!(find("page1").page, Some(1));
    // Each kind carries only its own fields.
    assert_eq!(
        find("menu").widget,
        Widget::Panel {
            title: "Paused".into(),
            modal: true
        }
    );
    let slider = (0..23).position(|i| UiKind::from_index(i) == UiKind::Slider);
    assert_eq!(
        find(&format!("w{}", slider.unwrap())).widget,
        Widget::Slider {
            range: [1., 9.],
            value: 4.
        }
    );
    assert_eq!(
        find("tabs").widget,
        Widget::Tabs {
            items: vec!["Audio".into(), "Video".into()],
            selected: 0.
        }
    );
    assert_eq!(find("grid").spacing.columns, Some(4));
    assert_eq!(find("grid").widget.container(), Some(Container::Grid));
    // What a button can't use waits in `legacy`.
    let stray = find("stray").legacy.clone().unwrap();
    assert_eq!(stray.offset, Some([3., 4.]));
    assert_eq!(stray.range, Some([2., 3.]));
    assert_eq!(find("stray").world_actor, "Hero");
    assert!(find("w1").children.is_empty());
}

#[test]
fn a_new_tree_lowers_with_the_kinds_own_spacing() {
    let screen = UiScreen {
        nodes: vec![UiNode {
            name: "menu".into(),
            widget: Widget::blank(UiKind::VerticalBox),
            place: LayoutItem {
                mode: Placement::at(UiAnchor::Center, [0., 0.]),
                ..LayoutItem::default()
            },
            spacing: Spacing {
                gap: Some(20.),
                ..Spacing::default()
            },
            children: vec![
                UiNode {
                    name: "play".into(),
                    widget: Widget::Button {
                        caption: "Play".into(),
                    },
                    ..UiNode::default()
                },
                UiNode {
                    name: "badge".into(),
                    widget: Widget::blank(UiKind::Label),
                    place: LayoutItem {
                        mode: Placement::at(UiAnchor::TopLeft, [4., 4.]),
                        ..LayoutItem::default()
                    },
                    ..UiNode::default()
                },
            ],
            ..UiNode::default()
        }],
        ..UiScreen::default()
    };
    let doc = screen.lower().unwrap();
    let layout = doc.widgets[0].layout.clone().unwrap();
    assert_eq!((layout.gap, layout.padding), (20., [12.; 4]));
    assert_eq!(doc.widgets[1].element.parent, "menu");
    assert_eq!(doc.widgets[1].element.content, "Play");
    assert!(doc.widgets[1].layout.is_none());
    assert!(doc.widgets[2].layout.as_ref().unwrap().absolute);
    assert_eq!(doc.widgets[2].element.offset, [4., 4.]);
    // Read back, its layout blocks say everything, and lower the same.
    assert_eq!(UiScreen::from_document(&doc).lower().unwrap(), doc);

    // A stretched anchor waits for the runtime.
    let mut stretched = screen.clone();
    stretched.nodes[0].place.mode = Placement::Anchored {
        anchor_min: [0., 0.],
        anchor_max: [1., 1.],
        pivot: [0.5, 0.5],
        position: [0., 0.],
    };
    assert!(stretched.lower().unwrap_err().contains("stretched"));
}

#[test]
fn a_missing_parent_or_a_cycle_still_gives_every_widget_a_place() {
    let mut doc = UiDocument::default();
    doc.widgets.push(widget("a", UiKind::Panel, "b"));
    doc.widgets.push(widget("b", UiKind::Panel, "a"));
    doc.widgets.push(widget("c", UiKind::Label, "gone"));
    let screen = UiScreen::from_document(&doc);
    let names: Vec<&str> = screen.walk().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["a", "b", "c"]);
}

#[test]
fn the_golden_projects_interfaces_round_trip() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/golden");
    let mut seen = 0;
    // `BLOCKLOOM_UI_FIXTURES` adds a project folder of your own.
    let extra = std::env::var_os("BLOCKLOOM_UI_FIXTURES").map(std::path::PathBuf::from);
    let files = walk(&root)
        .into_iter()
        .chain(extra.iter().flat_map(|dir| walk(dir)));
    for entry in files {
        let Ok(text) = std::fs::read_to_string(&entry) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let mut found = Vec::new();
        find_interfaces(&json, &mut found);
        for value in found {
            let Ok(doc) = serde_json::from_value::<UiDocument>(value.clone()) else {
                continue;
            };
            if doc.validate().is_err() {
                continue;
            }
            round_trip(&doc);
            seen += 1;
        }
    }
    assert!(seen > 0, "no interfaces under {}", root.display());
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

fn find_interfaces<'a>(json: &'a serde_json::Value, out: &mut Vec<&'a serde_json::Value>) {
    match json {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if key == "interface" && value.get("widgets").is_some() {
                    out.push(value);
                } else {
                    find_interfaces(value, out);
                }
            }
        }
        serde_json::Value::Array(list) => list.iter().for_each(|v| find_interfaces(v, out)),
        _ => {}
    }
}

/// An inventory: one `slot` component used by reference in a grid.
fn inventory() -> UiScreen {
    let slot = UiComponent {
        nodes: vec![UiNode {
            name: "slot".into(),
            widget: Widget::blank(UiKind::Panel),
            children: vec![
                UiNode {
                    name: "icon".into(),
                    widget: Widget::blank(UiKind::Image),
                    ..UiNode::default()
                },
                UiNode {
                    name: "count".into(),
                    widget: Widget::Label { text: "0".into() },
                    ..UiNode::default()
                },
                UiNode {
                    name: "extra".into(),
                    widget: Widget::blank(UiKind::VerticalBox),
                    ..UiNode::default()
                },
            ],
            ..UiNode::default()
        }],
        exports: vec![
            UiExport {
                name: "count".into(),
                widget: "count".into(),
                path: "element.content".into(),
                slot: false,
            },
            UiExport {
                name: "extra".into(),
                widget: "extra".into(),
                path: String::new(),
                slot: true,
            },
        ],
    };
    let cell = |i: usize| UiNode {
        name: format!("cell{i}"),
        instance: Some(UiInstance {
            component: "slot".into(),
            inputs: [("count".to_string(), serde_json::json!(format!("{i}")))].into(),
            slots: if i == 2 {
                [(
                    "extra".to_string(),
                    vec![UiNode {
                        name: "badge".into(),
                        widget: Widget::Label { text: "New".into() },
                        ..UiNode::default()
                    }],
                )]
                .into()
            } else {
                BTreeMap::new()
            },
        }),
        ..UiNode::default()
    };
    UiScreen {
        nodes: vec![UiNode {
            name: "bag".into(),
            widget: Widget::blank(UiKind::Grid),
            place: LayoutItem {
                mode: Placement::at(UiAnchor::Center, [0., 0.]),
                ..LayoutItem::default()
            },
            spacing: Spacing {
                columns: Some(3),
                ..Spacing::default()
            },
            children: (0..3).map(cell).collect(),
            ..UiNode::default()
        }],
        components: [("slot".to_string(), slot)].into(),
        inputs: vec![UiExport {
            name: "title".into(),
            widget: "cell0/count".into(),
            path: "element.content".into(),
            slot: false,
        }],
        ..UiScreen::default()
    }
}

#[test]
fn a_component_used_by_reference_lowers_to_named_instances() {
    let doc = inventory().lower().unwrap();
    let ids: Vec<&str> = doc.widgets.iter().map(|w| w.element.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "bag",
            "cell0/slot",
            "cell0/icon",
            "cell0/count",
            "cell0/extra",
            "cell1/slot",
            "cell1/icon",
            "cell1/count",
            "cell1/extra",
            "cell2/slot",
            "cell2/icon",
            "cell2/count",
            "cell2/extra",
            "badge",
        ]
    );
    let get = |id: &str| doc.widgets.iter().find(|w| w.element.id == id).unwrap();
    assert_eq!(get("cell1/count").element.content, "1");
    assert_eq!(get("cell2/slot").instance_of, "slot");
    assert_eq!(get("cell2/slot").element.parent, "bag");
    assert_eq!(get("badge").element.parent, "cell2/extra");
    // What the instance set is an override the designer can reset.
    let overrides = doc.instance_overrides("cell1/slot").unwrap();
    assert_eq!(overrides, ["cell1/count: element.content"]);
    doc.validate().unwrap();

    // A screen input writes through to the node it names.
    let doc = inventory()
        .lower_with(&[("title".to_string(), serde_json::json!("Bag"))].into())
        .unwrap();
    let get = |id: &str| doc.widgets.iter().find(|w| w.element.id == id).unwrap();
    assert_eq!(get("cell0/count").element.content, "Bag");
    assert!(
        inventory()
            .lower_with(&[("nope".to_string(), serde_json::json!(1))].into())
            .unwrap_err()
            .contains("no input called nope")
    );
    // And the screen keeps the reference form in its own file.
    let text = serde_json::to_string(&inventory()).unwrap();
    assert_eq!(
        serde_json::from_str::<UiScreen>(&text).unwrap(),
        inventory()
    );
}

#[test]
fn components_nest_and_refuse_to_contain_themselves() {
    let mut screen = inventory();
    // A row of two slots, used once.
    screen.components.insert(
        "pair".into(),
        UiComponent {
            nodes: vec![UiNode {
                name: "row".into(),
                widget: Widget::blank(UiKind::HorizontalBox),
                children: ["a", "b"]
                    .iter()
                    .map(|n| UiNode {
                        name: n.to_string(),
                        instance: Some(UiInstance {
                            component: "slot".into(),
                            ..UiInstance::default()
                        }),
                        ..UiNode::default()
                    })
                    .collect(),
                ..UiNode::default()
            }],
            exports: Vec::new(),
        },
    );
    screen.nodes.push(UiNode {
        name: "quick".into(),
        instance: Some(UiInstance {
            component: "pair".into(),
            ..UiInstance::default()
        }),
        ..UiNode::default()
    });
    let doc = screen.lower().unwrap();
    let get = |id: &str| doc.widgets.iter().find(|w| w.element.id == id).unwrap();
    assert_eq!(get("quick/b/count").element.parent, "quick/b/slot");
    assert_eq!(get("quick/b/slot").element.parent, "quick/row");

    // A component that uses itself has no bottom.
    let looped = screen.components.get_mut("pair").unwrap();
    looped.nodes[0].children[0].instance = Some(UiInstance {
        component: "pair".into(),
        ..UiInstance::default()
    });
    assert!(screen.lower().unwrap_err().contains("deeper"));
}

#[test]
fn a_designer_edit_folds_back_into_references() {
    let original = inventory();
    let mut doc = original.lower().unwrap();
    // Unchanged, every reference folds back and it lowers the same.
    let same = UiScreen::from_edit(&original, &doc);
    assert_eq!(same.nodes[0].children, original.nodes[0].children);
    assert_eq!(same.lower().unwrap(), doc);

    // An exported property changed: still one reference, new input.
    let at = doc
        .widgets
        .iter()
        .position(|w| w.element.id == "cell1/count")
        .unwrap();
    doc.widgets[at].element.content = "9".into();
    let edited = UiScreen::from_edit(&original, &doc);
    let cell1 = &edited.nodes[0].children[1];
    assert_eq!(cell1.name, "cell1");
    assert_eq!(
        cell1.instance.as_ref().unwrap().inputs["count"],
        serde_json::json!("9")
    );

    // A change no input covers keeps that one expanded, the rest folded.
    let at = doc
        .widgets
        .iter()
        .position(|w| w.element.id == "cell0/icon")
        .unwrap();
    doc.widgets[at].tooltip = "Sword".into();
    let edited = UiScreen::from_edit(&original, &doc);
    let cells = &edited.nodes[0].children;
    assert!(cells[0].instance.is_none());
    assert_eq!(cells[0].name, "cell0/slot");
    assert!(cells[1].instance.is_some() && cells[2].instance.is_some());
    assert_eq!(
        cells[2].instance.as_ref().unwrap().slots["extra"][0].name,
        "badge"
    );
    // Either way it lowers to the edited document.
    assert_eq!(edited.lower().unwrap().widgets, doc.widgets);
    assert_eq!(edited.inputs, original.inputs);
}
