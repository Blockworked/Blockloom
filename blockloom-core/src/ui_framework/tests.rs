#![allow(clippy::field_reassign_with_default)]
use super::*;

#[test]
fn sibling_reorder_preserves_other_slots_and_references() {
    let mut doc: UiDocument = serde_json::from_value(serde_json::json!({"widgets": [
        {"element": {"id": "root"}},
        {"element": {"id": "a", "parent": "root"}},
        {"element": {"id": "grandchild", "parent": "a"}, "scroll_target": "b"},
        {"element": {"id": "other"}},
        {"element": {"id": "b", "parent": "root"}},
        {"element": {"id": "c", "parent": "root"}}
    ]}))
    .unwrap();
    let original = doc.clone();
    doc.apply_edit(&UiEdit::Reorder {
        id: "c".into(),
        index: 0,
    })
    .unwrap();
    assert_eq!(
        doc.widgets
            .iter()
            .map(|w| w.element.id.as_str())
            .collect::<Vec<_>>(),
        ["root", "c", "grandchild", "other", "a", "b"]
    );
    assert_eq!(doc.widgets[2], original.widgets[2]);
    assert_eq!(doc.widgets[3], original.widgets[3]);
    for (id, index) in [("missing", 0), ("a", 3), ("root", 2)] {
        let before = doc.clone();
        assert!(
            doc.apply_edit(&UiEdit::Reorder {
                id: id.into(),
                index
            })
            .is_err()
        );
        assert_eq!(doc, before);
    }
    let before = doc.clone();
    doc.apply_edit(&UiEdit::Reorder {
        id: "c".into(),
        index: 0,
    })
    .unwrap();
    assert_eq!(doc, before);
    doc.apply_edit(&UiEdit::Reorder {
        id: "c".into(),
        index: 2,
    })
    .unwrap();
    assert_eq!(doc, original);
    doc.apply_edit(&UiEdit::Reorder {
        id: "other".into(),
        index: 0,
    })
    .unwrap();
    assert_eq!(doc.widgets[0].element.id, "other");
    assert_eq!(doc.widgets[3].element.id, "root");
    assert_eq!(doc.widgets[2], original.widgets[2]);
}

#[test]
fn typed_properties_and_reparenting_are_atomic() {
    let mut doc: UiDocument = serde_json::from_value(serde_json::json!({"widgets": [
        {"element": {"id": "canvas", "kind": "Canvas"}},
        {"element": {"id": "flow", "kind": "VerticalBox"}},
        {"element": {"id": "child", "parent": "canvas", "content": "before"}},
        {"element": {"id": "grandchild", "parent": "child"}, "scroll_target": "child"}
    ]}))
    .unwrap();
    let parse = |value| serde_json::from_value::<UiEdit>(value).unwrap();
    doc.apply_edit(&parse(serde_json::json!({"kind": "SetProperty", "id": "child", "property": {"path": "element.content", "value": "after"}}))).unwrap();
    assert_eq!(doc.widgets[2].element.content, "after");
    for value in [
        serde_json::json!({"kind": "SetProperty", "id": "child", "property": {"path": "layout", "value": {"gap": -1}}}),
        serde_json::json!({"kind": "SetProperty", "id": "child", "property": {"path": "layout", "value": {"width": {"Percent": -1}}}}),
        serde_json::json!({"kind": "SetProperty", "id": "child", "property": {"path": "layout", "value": {"min_size": [20,0], "max_size": [10,0]}}}),
        serde_json::json!({"kind": "Reparent", "id": "child", "parent": "grandchild", "placement": {"mode": "Flow"}}),
        serde_json::json!({"kind": "Reparent", "id": "child", "parent": "flow", "placement": {"mode": "Free", "offset": [1,2], "size": [30,40]}}),
        serde_json::json!({"kind": "Reparent", "id": "child", "parent": "missing", "placement": {"mode": "Flow"}}),
    ] {
        let before = doc.clone();
        assert!(doc.apply_edit(&parse(value)).is_err());
        assert_eq!(doc, before);
    }
    for property in [
        serde_json::json!({"path": "element.id", "value": "renamed"}),
        serde_json::json!({"path": "element.modal", "value": "yes"}),
        serde_json::json!({"path": "layout", "value": {"typo": 1}}),
    ] {
        assert!(
            serde_json::from_value::<UiEdit>(
                serde_json::json!({"kind": "SetProperty", "id": "child", "property": property})
            )
            .is_err()
        );
    }
    doc.apply_edit(&parse(serde_json::json!({"kind": "Reparent", "id": "child", "parent": "", "placement": {"mode": "Free", "offset": [40,60], "size": [30,40]}}))).unwrap();
    assert!(doc.widgets[2].layout.as_ref().unwrap().absolute);
    assert_eq!(doc.widgets[2].element.offset, [40., 60.]);
    doc.apply_edit(&parse(serde_json::json!({"kind": "Reparent", "id": "child", "parent": "flow", "placement": {"mode": "Flow"}}))).unwrap();
    assert!(!doc.widgets[2].layout.as_ref().unwrap().absolute);
    assert_eq!(doc.widgets[2].element.size, [30., 40.]);
    assert_eq!(doc.widgets[3].element.parent, "child");
    assert_eq!(doc.widgets[3].scroll_target, "child");
    assert_eq!(
        doc.widgets
            .iter()
            .map(|w| w.element.id.as_str())
            .collect::<Vec<_>>(),
        ["canvas", "flow", "child", "grandchild"]
    );
}

#[test]
fn fx_is_a_typed_edit_and_an_animation_track_may_drive_it() {
    let mut doc: UiDocument = serde_json::from_value(serde_json::json!({
        "widgets": [{"element": {"id": "a"}}]
    }))
    .unwrap();
    let edit = |value| serde_json::from_value::<UiEdit>(value).unwrap();
    doc.apply_edit(&edit(serde_json::json!({"kind": "SetProperty", "id": "a",
        "property": {"path": "fx", "value": {"opacity": 0.0, "rotation": 15}}})))
        .unwrap();
    assert_eq!(doc.widgets[0].fx.opacity, 0.);
    assert_eq!(doc.widgets[0].fx.rotation, 15.);
    let bad = edit(serde_json::json!({"kind": "SetProperty", "id": "a",
        "property": {"path": "fx", "value": {"opacity": 2.0}}}));
    assert!(doc.apply_edit(&bad).is_err());
    // A default fx is not written out.
    doc.widgets[0].fx = UiFx::default();
    assert!(
        serde_json::to_value(&doc.widgets[0])
            .unwrap()
            .get("fx")
            .is_none()
    );
    let track = |property| UiTrack {
        widget: "a".into(),
        property,
        keys: vec![UiKey {
            time: 0.,
            value: UiKeyValue::Number(1.),
            ease: UiEase::Linear,
        }],
    };
    for property in [
        UiProp::Opacity,
        UiProp::OffsetX,
        UiProp::OffsetY,
        UiProp::Rotation,
    ] {
        assert!(track(property).validate(1.).is_ok(), "{property:?}");
    }
}

#[test]
fn property_metadata_covers_every_editable_path_and_batches_are_atomic() {
    let samples = [
        UiPropertyEdit::Kind(UiKind::Label),
        UiPropertyEdit::Content(String::new()),
        UiPropertyEdit::Anchor(UiAnchor::Top),
        UiPropertyEdit::Modal(true),
        UiPropertyEdit::Layout(None),
        UiPropertyEdit::Range([0., 1.]),
        UiPropertyEdit::Style(Box::default()),
        UiPropertyEdit::Class(String::new()),
        UiPropertyEdit::Bindings(vec![]),
        UiPropertyEdit::Actions(vec![]),
        UiPropertyEdit::Variants(vec![]),
        UiPropertyEdit::Items(vec![]),
        UiPropertyEdit::Tooltip(String::new()),
        UiPropertyEdit::ScrollTarget(String::new()),
        UiPropertyEdit::TabIndex(None),
        UiPropertyEdit::Transition(0.),
        UiPropertyEdit::Fx(UiFx::default()),
        UiPropertyEdit::WorldActor(String::new()),
    ];
    let metadata = property_metadata();
    assert_eq!(metadata.len(), samples.len());
    for sample in &samples {
        assert!(metadata.iter().any(|m| m.path == sample.path()));
    }
    assert!(metadata[0].choices.contains(&"Canvas".to_string()));
    assert_eq!(metadata[2].choices.len(), 9);
    let mut doc: UiDocument = serde_json::from_value(serde_json::json!({"widgets": [
        {"element": {"id": "a", "offset": [1, 1]}, "layout": {"absolute": true}},
        {"element": {"id": "b", "offset": [2, 2]}, "layout": {"absolute": true}}
    ]}))
    .unwrap();
    let parse = |value| serde_json::from_value::<UiEdit>(value).unwrap();
    doc.apply_edit(&parse(serde_json::json!({"kind": "Batch", "edits": [
        {"kind": "Move", "id": "a", "offset": [10, 10]},
        {"kind": "Move", "id": "b", "offset": [20, 20]}
    ]})))
    .unwrap();
    assert_eq!(doc.widgets[0].element.offset, [10., 10.]);
    assert_eq!(doc.widgets[1].element.offset, [20., 20.]);
    let before = doc.clone();
    for value in [
        serde_json::json!({"kind": "Batch", "edits": [
            {"kind": "Move", "id": "a", "offset": [99, 99]},
            {"kind": "Move", "id": "missing", "offset": [1, 1]}]}),
        serde_json::json!({"kind": "Batch", "edits": [{"kind": "Batch", "edits": []}]}),
    ] {
        assert!(doc.apply_edit(&parse(value)).is_err());
        assert_eq!(doc, before);
    }
}

#[test]
fn content_style_and_document_edits_validate_and_apply() {
    let mut doc: UiDocument = serde_json::from_value(serde_json::json!({"widgets": [
        {"element": {"id": "bar", "kind": "Scrollbar"}},
        {"element": {"id": "list", "kind": "List"}}
    ]}))
    .unwrap();
    let parse = |value| serde_json::from_value::<UiEdit>(value).unwrap();
    let prop = |id: &str, path: &str, value: serde_json::Value| {
        parse(
            serde_json::json!({"kind": "SetProperty", "id": id, "property": {"path": path, "value": value}}),
        )
    };
    doc.apply_edit(&prop("bar", "scroll_target", "list".into()))
        .unwrap();
    doc.apply_edit(&prop("list", "items", serde_json::json!(["a", "b"])))
        .unwrap();
    doc.apply_edit(&prop(
        "list",
        "style",
        serde_json::json!({"normal": {"background": "#102030", "radius": 4.0}}),
    ))
    .unwrap();
    doc.apply_edit(&prop(
        "list",
        "bindings",
        serde_json::json!([{"property": "Visible", "source": {"Variable": {"name": "shown"}}}]),
    ))
    .unwrap();
    doc.apply_edit(&prop("list", "tab_index", serde_json::json!(2)))
        .unwrap();
    doc.apply_edit(&prop("list", "element.range", serde_json::json!([0, 10])))
        .unwrap();
    assert_eq!(doc.widgets[0].scroll_target, "list");
    assert_eq!(doc.widgets[1].items, ["a", "b"]);
    assert_eq!(doc.widgets[1].style.normal.radius, Some(4.));
    assert_eq!(doc.widgets[1].bindings.len(), 1);
    assert_eq!(doc.widgets[1].tab_index, Some(2));
    let before = doc.clone();
    for edit in [
        prop("bar", "scroll_target", "missing".into()),
        prop("list", "element.range", serde_json::json!([5, 5])),
        prop("list", "transition", serde_json::json!(-1)),
        prop(
            "list",
            "style",
            serde_json::json!({"normal": {"radius": -2.0}}),
        ),
        prop(
            "list",
            "bindings",
            serde_json::json!([{"property": "Value", "source": {"Variable": {"name": "x"}}}, {"property": "Tooltip", "source": {"Variable": {"name": "x"}}}]),
        ),
    ] {
        assert!(doc.apply_edit(&edit).is_err());
        assert_eq!(doc, before);
    }
    doc.apply_edit(&parse(
        serde_json::json!({"kind": "SetDocument", "theme": "Light", "scale": "ScaleWithSize"}),
    ))
    .unwrap();
    assert_eq!(doc.theme, UiTheme::Light);
    assert_eq!(doc.scale, UiScale::ScaleWithSize);
    doc.apply_edit(&parse(serde_json::json!({"kind": "SetClass", "name": "title", "styles": {"normal": {"text_size": 24.0}}}))).unwrap();
    assert_eq!(doc.styles["title"].normal.text_size, Some(24.));
    doc.apply_edit(&parse(
        serde_json::json!({"kind": "SetClass", "name": "title", "styles": null}),
    ))
    .unwrap();
    assert!(doc.styles.is_empty());
    assert!(
        doc.apply_edit(&parse(
            serde_json::json!({"kind": "SetClass", "name": " ", "styles": null})
        ))
        .is_err()
    );
}

#[test]
fn schema_versions_migrate_forward_and_refuse_newer_documents() {
    let mut legacy: UiDocument =
        serde_json::from_value(serde_json::json!({"widgets": [{"element": {"id": "a"}}]})).unwrap();
    assert_eq!(legacy.version, 0);
    let before = legacy.widgets.clone();
    legacy.migrate();
    assert_eq!(legacy.version, UI_SCHEMA_VERSION);
    assert_eq!(legacy.widgets, before);
    let again = legacy.clone();
    legacy.migrate();
    assert_eq!(legacy, again);
    assert_eq!(UiDocument::default().version, UI_SCHEMA_VERSION);
    let newer: UiDocument = serde_json::from_value(serde_json::json!({"version": 99})).unwrap();
    assert!(newer.validate().unwrap_err().contains("schema version 99"));
    let mut kept = newer.clone();
    kept.migrate();
    assert_eq!(kept.version, 99);
}

#[test]
fn create_delete_and_duplicate_keep_ids_and_references_consistent() {
    let mut doc: UiDocument = serde_json::from_value(serde_json::json!({"widgets": [
        {"element": {"id": "canvas", "kind": "Canvas"}},
        {"element": {"id": "flow", "kind": "VerticalBox"}},
        {"element": {"id": "menu", "parent": "canvas", "offset": [5, 6]}, "layout": {"absolute": true}},
        {"element": {"id": "list", "parent": "menu"}, "scroll_target": "bar"},
        {"element": {"id": "bar", "parent": "menu"}, "scroll_target": "outside"},
        {"element": {"id": "outside"}}
    ]}))
    .unwrap();
    let ids = |d: &UiDocument| {
        d.widgets
            .iter()
            .map(|w| w.element.id.clone())
            .collect::<Vec<_>>()
    };
    let parse = |value| serde_json::from_value::<UiEdit>(value).unwrap();
    doc.apply_edit(&parse(serde_json::json!({"kind": "Create", "widget": {"element": {"id": "a", "parent": "canvas"}}}))).unwrap();
    assert!(doc.widgets[6].layout.as_ref().unwrap().absolute);
    doc.apply_edit(&parse(serde_json::json!({"kind": "Create", "widget": {"element": {"id": "b", "parent": "flow"}, "layout": {"absolute": true}}}))).unwrap();
    assert!(!doc.widgets[7].layout.as_ref().unwrap().absolute);
    for value in [
        serde_json::json!({"kind": "Create", "widget": {"element": {"id": "a"}}}),
        serde_json::json!({"kind": "Create", "widget": {"element": {"id": ""}}}),
        serde_json::json!({"kind": "Create", "widget": {"element": {"id": "c", "parent": "nope"}}}),
        serde_json::json!({"kind": "Create", "widget": {"element": {"id": "c"}, "world_actor": "x"}}),
        serde_json::json!({"kind": "Delete", "id": "nope"}),
        serde_json::json!({"kind": "Duplicate", "id": "menu", "new_id": "a"}),
        serde_json::json!({"kind": "Duplicate", "id": "b", "new_id": "b2", "offset": [1, 1]}),
    ] {
        let before = doc.clone();
        assert!(doc.apply_edit(&parse(value)).is_err());
        assert_eq!(doc, before);
    }
    doc.apply_edit(&parse(serde_json::json!({"kind": "Duplicate", "id": "menu", "new_id": "menu2", "offset": [21, 22]}))).unwrap();
    assert_eq!(
        ids(&doc),
        [
            "canvas",
            "flow",
            "menu",
            "list",
            "bar",
            "menu2",
            "menu2.list",
            "menu2.bar",
            "outside",
            "a",
            "b"
        ]
    );
    assert_eq!(doc.widgets[5].element.offset, [21., 22.]);
    assert_eq!(doc.widgets[2].element.offset, [5., 6.]);
    assert_eq!(doc.widgets[6].element.parent, "menu2");
    assert_eq!(doc.widgets[6].scroll_target, "menu2.bar");
    assert_eq!(doc.widgets[7].scroll_target, "outside");
    assert_eq!(doc.widgets[3].scroll_target, "bar");
    doc.apply_edit(&parse(serde_json::json!({"kind": "Delete", "id": "bar"})))
        .unwrap();
    assert_eq!(doc.widgets[3].scroll_target, "");
    doc.apply_edit(&parse(serde_json::json!({"kind": "Delete", "id": "menu"})))
        .unwrap();
    assert_eq!(
        ids(&doc),
        [
            "canvas",
            "flow",
            "menu2",
            "menu2.list",
            "menu2.bar",
            "outside",
            "a",
            "b"
        ]
    );
}

#[test]
fn typed_edits_preserve_layout_and_reject_invalid_dimensions() {
    let mut document = UiDocument::default();
    document.widgets = vec![
        UiWidget {
            element: UiElement {
                id: "parent".into(),
                kind: UiKind::Panel,
                ..Default::default()
            },
            ..Default::default()
        },
        UiWidget {
            element: UiElement {
                id: "child".into(),
                parent: "parent".into(),
                ..Default::default()
            },
            ..Default::default()
        },
    ];
    let move_child = UiEdit::Move {
        id: "child".into(),
        offset: [20., 30.],
    };
    let original = document.clone();
    assert!(document.apply_edit(&move_child).is_err());
    assert_eq!(document, original);
    document.widgets[0].element.kind = UiKind::Canvas;
    document.apply_edit(&move_child).unwrap();
    assert_eq!(document.widgets[1].element.offset, [20., 30.]);
    document.widgets[1].layout = Some(UiLayout {
        width: UiLength::Percent(50.),
        padding: [4.; 4],
        ..Default::default()
    });
    document
        .apply_edit(&UiEdit::Resize {
            id: "child".into(),
            offset: [21., 31.],
            size: [80., 60.],
        })
        .unwrap();
    let layout = document.widgets[1].layout.as_ref().unwrap();
    assert_eq!(layout.width, UiLength::Px(80.));
    assert_eq!(layout.height, UiLength::Px(60.));
    assert_eq!(layout.padding, [4.; 4]);
    let original = document.clone();
    for size in [[0., 1.], [-1., 1.], [f32::NAN, 1.], [1., f32::INFINITY]] {
        assert!(
            document
                .apply_edit(&UiEdit::Resize {
                    id: "child".into(),
                    offset: [0.; 2],
                    size
                })
                .is_err()
        );
        assert_eq!(document, original);
    }
    assert!(
        document
            .apply_edit(&UiEdit::Move {
                id: "child".into(),
                offset: [f32::NAN, 0.]
            })
            .is_err()
    );
    document.widgets[1].world_actor = "actor".into();
    assert!(document.apply_edit(&move_child).is_err());
}

#[test]
fn device_insets_add_over_the_authored_safe_area() {
    assert_eq!(
        effective_safe_area([10., 0., 10., 0.], [0., 40., 0., 20.]),
        [10., 40., 10., 20.]
    );
    assert_eq!(effective_safe_area([0.; 4], [-5., 0., 0., 0.]), [0.; 4]);
}
#[test]
fn rejects_cycles_and_missing_parents() {
    let mut d = UiDocument::default();
    let mut w = UiWidget::default();
    w.element.id = "a".into();
    w.element.parent = "a".into();
    d.widgets.push(w);
    assert!(d.validate().is_err());
    d.widgets[0].element.parent = "missing".into();
    assert!(d.validate().is_err());
    d.widgets[0].element.parent.clear();
    assert!(d.validate().is_ok());
}
fn row(id: &str, parent: &str, content: &str) -> UiWidget {
    let mut w = UiWidget::default();
    w.element.id = id.into();
    w.element.parent = parent.into();
    w.element.content = content.into();
    w
}

#[test]
fn image_fit_round_trips_inherits_and_rejects_bad_borders() {
    let mut doc = UiDocument::default();
    doc.widgets = vec![row("pic", "", "a.png")];
    let mut styles = UiStyles::default();
    styles.normal.image_fit = Some(UiImageFit::Sliced {
        border: [4., 4., 6., 6.],
    });
    doc.apply_edit(&UiEdit::SetProperty {
        id: "pic".into(),
        property: UiPropertyEdit::Style(Box::new(styles.clone())),
    })
    .unwrap();
    let json = serde_json::to_string(&doc).unwrap();
    let back: UiDocument = serde_json::from_str(&json).unwrap();
    assert_eq!(back.widgets[0].style, styles);
    let over = UiPaint::default().over(&styles.normal);
    assert_eq!(over.image_fit, styles.normal.image_fit);
    styles.normal.image_fit = Some(UiImageFit::Sliced {
        border: [-1., 0., 0., 0.],
    });
    assert!(
        doc.apply_edit(&UiEdit::SetProperty {
            id: "pic".into(),
            property: UiPropertyEdit::Style(Box::new(styles)),
        })
        .is_err()
    );
}

#[test]
fn prefab_instances_follow_the_source_and_keep_overrides() {
    let mut doc = UiDocument::default();
    doc.widgets = vec![
        row("row", "", "Row"),
        row("label", "row", "Volume"),
        row("hint", "row", "Hint"),
    ];
    doc.apply_edit(&UiEdit::SavePrefab {
        name: "setting".into(),
        root: "row".into(),
        update_instances: false,
    })
    .unwrap();
    for prefix in ["a.", "b."] {
        doc.apply_edit(&UiEdit::InstantiatePrefab {
            name: "setting".into(),
            prefix: prefix.into(),
            parent: String::new(),
        })
        .unwrap();
    }
    let a = doc.find("a.row").unwrap();
    assert_eq!(doc.widgets[a].instance_of, "setting");
    // Override one widget in instance a.
    let label = doc.find("a.label").unwrap();
    doc.widgets[label].element.content = "Music".into();
    // Change the source and push the update.
    let src = doc.find("label").unwrap();
    doc.widgets[src].element.content = "Sound".into();
    let hint = doc.find("hint").unwrap();
    doc.widgets.remove(hint);
    doc.apply_edit(&UiEdit::SavePrefab {
        name: "setting".into(),
        root: "row".into(),
        update_instances: true,
    })
    .unwrap();
    let content = |d: &UiDocument, id: &str| d.widgets[d.find(id).unwrap()].element.content.clone();
    assert_eq!(content(&doc, "a.label"), "Music");
    assert_eq!(content(&doc, "b.label"), "Sound");
    assert!(doc.find("a.hint").is_none() && doc.find("b.hint").is_none());
    // Deleting the prefab detaches instances; a dangling tag is refused.
    doc.apply_edit(&UiEdit::DeletePrefab {
        name: "setting".into(),
    })
    .unwrap();
    assert!(doc.widgets.iter().all(|w| w.instance_of.is_empty()));
    doc.widgets[0].instance_of = "gone".into();
    assert!(doc.validate().is_err());
}

#[test]
fn instances_set_exposed_properties_and_fill_slots() {
    let mut doc = UiDocument::default();
    let mut body = row("body", "card", "");
    body.element.kind = UiKind::VerticalBox;
    doc.widgets = vec![row("card", "", "Card"), row("title", "card", "Title"), body];
    doc.apply_edit(&UiEdit::SavePrefab {
        name: "card".into(),
        root: "card".into(),
        update_instances: false,
    })
    .unwrap();
    let expose = |doc: &mut UiDocument, edit: serde_json::Value| {
        doc.apply_edit(&serde_json::from_value(edit).unwrap())
    };
    expose(&mut doc, serde_json::json!({"kind": "ExposeProperty", "prefab": "card", "name": "title", "widget": "title", "path": "element.content"})).unwrap();
    expose(&mut doc, serde_json::json!({"kind": "ExposeSlot", "prefab": "card", "name": "body", "widget": "body"})).unwrap();
    // Bad exposures are refused.
    assert!(expose(&mut doc, serde_json::json!({"kind": "ExposeProperty", "prefab": "card", "name": "x", "widget": "nope", "path": "tooltip"})).is_err());
    assert!(expose(&mut doc, serde_json::json!({"kind": "ExposeProperty", "prefab": "card", "name": "x", "widget": "title", "path": "bogus"})).is_err());
    assert!(expose(&mut doc, serde_json::json!({"kind": "ExposeProperty", "prefab": "card", "name": "title", "widget": "title", "path": "tooltip"})).is_err());
    doc.apply_edit(&UiEdit::InstantiatePrefab {
        name: "card".into(),
        prefix: "a.".into(),
        parent: String::new(),
    })
    .unwrap();
    expose(&mut doc, serde_json::json!({"kind": "SetExposed", "instance": "a.card", "name": "title", "value": "Hello"})).unwrap();
    assert_eq!(
        doc.widgets[doc.find("a.title").unwrap()].element.content,
        "Hello"
    );
    assert_eq!(
        doc.exposed_values("a.card").unwrap(),
        vec![("title".to_string(), serde_json::json!("Hello"))]
    );
    // The wrong type, an unknown name and a slot used as a property fail.
    assert!(expose(&mut doc, serde_json::json!({"kind": "SetExposed", "instance": "a.card", "name": "title", "value": 5})).is_err());
    assert!(expose(&mut doc, serde_json::json!({"kind": "SetExposed", "instance": "a.card", "name": "body", "value": "x"})).is_err());
    // A loose widget moves into the slot.
    doc.widgets.push(row("extra", "", "Extra"));
    expose(&mut doc, serde_json::json!({"kind": "FillSlot", "instance": "a.card", "slot": "body", "widget": "extra"})).unwrap();
    assert_eq!(
        doc.widgets[doc.find("extra").unwrap()].element.parent,
        "a.body"
    );
    assert!(doc.validate().is_ok());
    // Unexposing and deleting the prefab clean up.
    expose(
        &mut doc,
        serde_json::json!({"kind": "Unexpose", "prefab": "card", "name": "title"}),
    )
    .unwrap();
    assert!(
        expose(
            &mut doc,
            serde_json::json!({"kind": "Unexpose", "prefab": "card", "name": "title"})
        )
        .is_err()
    );
    expose(
        &mut doc,
        serde_json::json!({"kind": "DeletePrefab", "name": "card"}),
    )
    .unwrap();
    assert!(doc.exports.is_empty());
    assert!(doc.validate().is_ok());
}

#[test]
fn animations_sample_ease_validate_and_edit() {
    let mut doc = UiDocument::default();
    doc.widgets = vec![row("bar", "", "")];
    let animation: UiAnimation = serde_json::from_value(serde_json::json!({
        "duration": 2.0, "looping": true,
        "tracks": [
            {"widget": "bar", "property": "Width", "keys": [
                {"time": 0.0, "value": 100.0}, {"time": 1.0, "value": 200.0, "ease": "EaseIn"}, {"time": 2.0, "value": 100.0}]},
            {"widget": "bar", "property": "Background", "keys": [
                {"time": 0.0, "value": "#000000"}, {"time": 2.0, "value": "#ffffff80"}]}
        ]
    }))
    .unwrap();
    let sample = |t: f32, i: usize| animation.tracks[i].sample(t).unwrap();
    assert_eq!(sample(0.5, 0), UiKeyValue::Number(150.));
    assert_eq!(sample(-1., 0), UiKeyValue::Number(100.));
    assert_eq!(sample(9., 0), UiKeyValue::Number(100.));
    assert_eq!(sample(1.0, 0), UiKeyValue::Number(200.));
    assert_eq!(sample(1.0, 1), UiKeyValue::Color("#808080c0".into()));
    let named = |a: Option<UiAnimation>| UiEdit::SetAnimation {
        name: "pulse".into(),
        animation: a,
    };
    doc.apply_edit(&named(Some(animation.clone()))).unwrap();
    let back: UiDocument = serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
    assert_eq!(back.animations["pulse"], animation);
    let mut bad = animation.clone();
    bad.tracks[0].keys[1].time = 0.0;
    assert!(doc.apply_edit(&named(Some(bad))).is_err());
    let mut bad = animation.clone();
    bad.tracks[0].property = UiProp::Text;
    assert!(doc.apply_edit(&named(Some(bad))).is_err());
    let mut bad = animation.clone();
    bad.tracks[1].keys[0].value = UiKeyValue::Number(1.);
    assert!(doc.apply_edit(&named(Some(bad))).is_err());
    let mut bad = animation.clone();
    bad.tracks[0].widget = "ghost".into();
    assert!(doc.apply_edit(&named(Some(bad))).is_err());
    assert_eq!(doc.animations.len(), 1);
    doc.apply_edit(&named(None)).unwrap();
    assert!(doc.apply_edit(&named(None)).is_err());
    // Actions can start and stop one by name.
    let play = [UiAction {
        on: UiEventKind::Click,
        run: UiDo::PlayAnimation {
            name: "pulse".into(),
        },
    }];
    assert!(check_actions(&play).is_ok());
    let empty = [UiAction {
        on: UiEventKind::Click,
        run: UiDo::StopAnimation { name: " ".into() },
    }];
    assert!(check_actions(&empty).is_err());
}

#[test]
fn variants_pick_the_first_matching_breakpoint_and_validate() {
    let mut doc = UiDocument::default();
    doc.widgets = vec![row("menu", "", "Menu")];
    let value = serde_json::json!([
        {"when": {"max_width": 600.0}, "layout": {"width": {"Percent": 100.0}}, "visible": true},
        {"when": {"min_width": 601.0, "max_width": 1200.0}, "visible": false}
    ]);
    let edit = |value: serde_json::Value| UiEdit::SetProperty {
        id: "menu".into(),
        property: serde_json::from_value(serde_json::json!({"path": "variants", "value": value}))
            .unwrap(),
    };
    doc.apply_edit(&edit(value)).unwrap();
    let w = &doc.widgets[0];
    assert_eq!(w.variant_for(400., 800.), Some(0));
    assert_eq!(w.variant_for(900., 800.), Some(1));
    assert_eq!(w.variant_for(1920., 1080.), None);
    let back: UiDocument = serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
    assert_eq!(back.widgets[0].variants, w.variants);
    let before = doc.clone();
    assert!(
        doc.apply_edit(&edit(
            serde_json::json!([{"when": {"min_width": 900.0, "max_width": 300.0}}])
        ))
        .is_err()
    );
    assert!(
        doc.apply_edit(&edit(serde_json::json!([{"when": {"min_height": -1.0}}])))
            .is_err()
    );
    assert_eq!(doc, before);
}

#[test]
fn widget_actions_edit_validate_and_round_trip() {
    let mut doc = UiDocument::default();
    doc.widgets = vec![row("play", "", "Play")];
    let good = serde_json::json!([
        {"on": "Click", "run": {"Broadcast": {"message": "start"}}},
        {"on": "Change", "run": {"SetVariable": {"name": "volume", "value": "$value"}}}
    ]);
    let edit = |value: serde_json::Value| UiEdit::SetProperty {
        id: "play".into(),
        property: serde_json::from_value(serde_json::json!({"path": "actions", "value": value}))
            .unwrap(),
    };
    doc.apply_edit(&edit(good)).unwrap();
    assert_eq!(doc.widgets[0].actions.len(), 2);
    let back: UiDocument = serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
    assert_eq!(back.widgets[0].actions, doc.widgets[0].actions);
    let before = doc.clone();
    assert!(
        doc.apply_edit(&edit(
            serde_json::json!([{"on": "Click", "run": {"Broadcast": {"message": " "}}}])
        ))
        .is_err()
    );
    assert_eq!(doc, before);
    assert!(UiDocument::default().widgets.is_empty());
    doc.widgets[0].actions[0].run = UiDo::SetVariable {
        actor: String::new(),
        name: String::new(),
        value: "1".into(),
    };
    assert!(doc.validate().is_err());
}

#[test]
fn overrides_merge_per_field_and_can_be_listed_and_reset() {
    let mut doc = UiDocument::default();
    doc.widgets = vec![row("row", "", "Row"), row("label", "row", "Volume")];
    doc.widgets[1].tooltip = "Old tip".into();
    doc.apply_edit(&UiEdit::SavePrefab {
        name: "setting".into(),
        root: "row".into(),
        update_instances: false,
    })
    .unwrap();
    doc.apply_edit(&UiEdit::InstantiatePrefab {
        name: "setting".into(),
        prefix: "a.".into(),
        parent: String::new(),
    })
    .unwrap();
    assert!(doc.instance_overrides("a.row").unwrap().is_empty());
    // The instance changes the caption only.
    let label = doc.find("a.label").unwrap();
    doc.widgets[label].element.content = "Music".into();
    assert_eq!(
        doc.instance_overrides("a.row").unwrap(),
        vec!["a.label: element.content".to_string()]
    );
    // The prefab changes the tooltip and the caption; the tooltip flows in,
    // the instance's caption stays.
    let src = doc.find("label").unwrap();
    doc.widgets[src].tooltip = "New tip".into();
    doc.widgets[src].element.content = "Sound".into();
    doc.apply_edit(&UiEdit::SavePrefab {
        name: "setting".into(),
        root: "row".into(),
        update_instances: true,
    })
    .unwrap();
    let label = &doc.widgets[doc.find("a.label").unwrap()];
    assert_eq!(label.element.content, "Music");
    assert_eq!(label.tooltip, "New tip");
    // Resetting brings the prefab's caption back.
    doc.apply_edit(&UiEdit::ResetInstance { id: "a.row".into() })
        .unwrap();
    assert_eq!(
        doc.widgets[doc.find("a.label").unwrap()].element.content,
        "Sound"
    );
    assert!(doc.instance_overrides("a.row").unwrap().is_empty());
    assert!(
        doc.apply_edit(&UiEdit::ResetInstance {
            id: "a.label".into()
        })
        .is_err()
    );
}

#[test]
fn one_field_of_an_instance_resets_without_touching_the_rest() {
    let mut doc = UiDocument::default();
    doc.widgets = vec![row("row", "", "Row"), row("label", "row", "Volume")];
    doc.widgets[1].tooltip = "Tip".into();
    doc.apply_edit(&UiEdit::SavePrefab {
        name: "setting".into(),
        root: "row".into(),
        update_instances: false,
    })
    .unwrap();
    doc.apply_edit(&UiEdit::InstantiatePrefab {
        name: "setting".into(),
        prefix: "a.".into(),
        parent: String::new(),
    })
    .unwrap();
    let label = doc.find("a.label").unwrap();
    doc.widgets[label].element.content = "Music".into();
    doc.widgets[label].tooltip = "Changed".into();
    assert_eq!(doc.instance_overrides("a.row").unwrap().len(), 2);
    doc.apply_edit(&UiEdit::ResetField {
        instance: "a.row".into(),
        widget: "a.label".into(),
        path: "tooltip".into(),
    })
    .unwrap();
    let label = &doc.widgets[doc.find("a.label").unwrap()];
    assert_eq!(label.tooltip, "Tip");
    assert_eq!(label.element.content, "Music");
    assert_eq!(
        doc.instance_overrides("a.row").unwrap(),
        vec!["a.label: element.content".to_string()]
    );
    // A nested field resets too, and a bad path or widget is refused.
    let label = doc.find("a.label").unwrap();
    doc.widgets[label].element.offset = [9., 9.];
    doc.apply_edit(&UiEdit::ResetField {
        instance: "a.row".into(),
        widget: "a.label".into(),
        path: "element.offset".into(),
    })
    .unwrap();
    assert_eq!(
        doc.widgets[doc.find("a.label").unwrap()].element.offset,
        [0., 0.]
    );
    for (widget, path) in [
        ("a.label", "added"),
        ("elsewhere", "tooltip"),
        ("a.label", ""),
    ] {
        assert!(
            doc.apply_edit(&UiEdit::ResetField {
                instance: "a.row".into(),
                widget: widget.into(),
                path: path.into(),
            })
            .is_err()
        );
    }
}

#[test]
fn rename_follows_parents_scroll_targets_and_tracks() {
    let mut doc: UiDocument = serde_json::from_value(serde_json::json!({
        "widgets": [
            {"element": {"id": "list"}},
            {"element": {"id": "bar"}, "scroll_target": "list"},
            {"element": {"id": "item", "parent": "list"}},
            {"element": {"id": "other"}}
        ],
        "animations": {"fade": {"duration": 1.0, "tracks": [
            {"widget": "list", "property": "Opacity", "keys": [{"time": 0.0, "value": 1.0}]}]}}
    }))
    .unwrap();
    doc.apply_edit(&UiEdit::Rename {
        id: "list".into(),
        new_id: "inventory".into(),
    })
    .unwrap();
    assert_eq!(doc.widgets[0].element.id, "inventory");
    assert_eq!(doc.widgets[1].scroll_target, "inventory");
    assert_eq!(doc.widgets[2].element.parent, "inventory");
    assert_eq!(doc.animations["fade"].tracks[0].widget, "inventory");
    let before = doc.clone();
    for (id, new_id) in [("inventory", "other"), ("inventory", " "), ("missing", "x")] {
        assert!(
            doc.apply_edit(&UiEdit::Rename {
                id: id.into(),
                new_id: new_id.into(),
            })
            .is_err()
        );
    }
    assert_eq!(doc, before);
}

#[test]
fn a_patch_carries_only_what_changed_and_rebuilds_the_document() {
    let mut base = UiDocument::default();
    base.widgets = (0..50).map(|i| row(&format!("w{i}"), "", "x")).collect();
    let next = base
        .with_edit(&UiEdit::Batch {
            edits: vec![
                UiEdit::SetProperty {
                    id: "w3".into(),
                    property: UiPropertyEdit::Tooltip("hi".into()),
                },
                UiEdit::Delete { id: "w10".into() },
                UiEdit::Reorder {
                    id: "w40".into(),
                    index: 0,
                },
            ],
        })
        .unwrap();
    let patch = next.patch_from(&base);
    assert!(patch.document.widgets.is_empty());
    assert_eq!(patch.order.len(), 49);
    assert_eq!(patch.order[0], "w40");
    // The tooltip edit is the only widget that changed; the rest are ids.
    assert_eq!(patch.widgets.len(), 1);
    assert_eq!(patch.widgets[0].element.id, "w3");
    assert_eq!(patch.apply(&base).unwrap(), next);
    let json = serde_json::to_string(&patch).unwrap();
    let full = serde_json::to_string(&next).unwrap();
    assert!(
        json.len() < full.len() / 2,
        "{} vs {}",
        json.len(),
        full.len()
    );
    let mut broken = patch.clone();
    broken.order.push("nope".into());
    assert!(broken.apply(&base).is_err());
}

#[test]
fn prefabs_repoint_internal_scroll_targets_and_roundtrip() {
    let mut document = UiDocument::default();
    document.prefabs.insert(
        "inventory".into(),
        vec![
            UiWidget {
                element: UiElement {
                    id: "list".into(),
                    kind: UiKind::ListView,
                    ..Default::default()
                },
                ..Default::default()
            },
            UiWidget {
                element: UiElement {
                    id: "bar".into(),
                    kind: UiKind::Scrollbar,
                    ..Default::default()
                },
                scroll_target: "list".into(),
                ..Default::default()
            },
        ],
    );
    document.widgets = document.instantiate("inventory", "copy_", "").unwrap();
    assert_eq!(document.widgets[1].scroll_target, "copy_list");
    document.validate().unwrap();
    let json = serde_json::to_string(&document).unwrap();
    assert_eq!(serde_json::from_str::<UiDocument>(&json).unwrap(), document);
}

#[test]
fn rich_text_preserves_nested_styles_and_unknown_tags() {
    let runs = rich_text("[b]A[b]B[/b]C[/b][i]D[/i][color=#ff0000]E[/color][unknown]");
    assert_eq!(
        runs.iter().map(|r| r.text.as_str()).collect::<String>(),
        "ABCDE[unknown]"
    );
    assert!(runs[..3].iter().all(|r| r.bold));
    assert!(runs[3].italic);
    assert_eq!(runs[4].color.as_deref(), Some("#ff0000"));
    assert!(!runs[5].bold);
}

#[test]
fn virtual_rows_stay_bounded_for_large_inventories() {
    assert_eq!(visible_rows(100000, 32000., 320., 32.), 999..1011);
    assert_eq!(visible_rows(0, 0., 320., 32.), 0..0);
}
#[test]
fn converters_roundtrip_and_refuse_zero_division() {
    let mut c = UiConverter {
        scale: Some(100.),
        prefix: "$".into(),
        ..Default::default()
    };
    assert_eq!(
        c.read(Evaluated::Number(0.5)),
        Evaluated::Text("$50".into())
    );
    assert_eq!(
        c.write(&Evaluated::Text("$50".into())),
        Some(Evaluated::Number(0.5))
    );
    c.scale = Some(0.);
    assert!(c.write(&Evaluated::Number(1.)).is_none());
}
