use super::*;

/// A tree of `n` widgets: a chain `depth` deep under each of several roots.
fn tree(n: usize, depth: usize) -> UiDocument {
    let mut doc = UiDocument::default();
    for i in 0..n {
        let mut w = UiWidget::default();
        w.element.id = format!("w{i}");
        if i % depth != 0 {
            w.element.parent = format!("w{}", i - 1);
        }
        w.layout = Some(UiLayout {
            absolute: true,
            ..UiLayout::default()
        });
        doc.widgets.push(w);
    }
    doc
}

#[test]
fn a_deep_chain_validates_and_a_cycle_deep_in_it_is_found() {
    let mut doc = tree(2000, 2000);
    doc.validate().unwrap();
    doc.widgets[0].element.parent = "w1999".into();
    let err = doc.validate().unwrap_err();
    assert!(err.starts_with("Widget parent cycle"), "{err}");
    doc.widgets[0].element.parent = "gone".into();
    assert_eq!(doc.validate().unwrap_err(), "Missing widget parent: gone");
}

#[test]
fn world_actor_is_a_typed_property() {
    let mut doc = tree(2, 2);
    doc.apply_edit(&UiEdit::SetProperty {
        id: "w1".into(),
        property: UiPropertyEdit::WorldActor(" Hero ".into()),
    })
    .unwrap();
    assert_eq!(doc.widgets[1].world_actor, "Hero");
}

/// Prints edit and validate cost at the sizes the plan names:
/// `cargo test -p blockloom-core scale_tests::timings -- --ignored --nocapture`
#[test]
#[ignore]
fn timings() {
    for n in [1_000, 10_000] {
        let doc = tree(n, 8);
        let started = std::time::Instant::now();
        doc.validate().unwrap();
        let validate = started.elapsed();
        let edit = UiEdit::Move {
            id: format!("w{}", n - 1),
            offset: [3.0, 4.0],
        };
        let started = std::time::Instant::now();
        let next = doc.with_edit(&edit).unwrap();
        let apply = started.elapsed();
        assert_ne!(next, doc);
        println!("{n} widgets: validate {validate:?}, edit {apply:?}");
    }
}
