//! Blockloom's sensing reporters as blockstitch describes them, and the
//! copy the editor's Blocks.qml carries (`BLESS=1` rewrites it).

use blockloom_core::value::{
    host_operator_descriptors, register_blockloom_operators, render_descriptors_qml,
    replace_generated,
};

#[test]
fn every_reporter_has_a_slot_per_arg() {
    register_blockloom_operators();
    for d in host_operator_descriptors() {
        assert_eq!(d.args.len(), d.arity, "{}", d.kind);
    }
}

#[test]
fn blocks_qml_carries_the_current_table() {
    register_blockloom_operators();
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../blockloom-qt/qml/Blocks.qml"
    );
    let text = std::fs::read_to_string(path).unwrap();
    let block = format!(
        "    readonly property var hostDescriptors: {}",
        render_descriptors_qml(&host_operator_descriptors(), "    ")
    );
    let next = replace_generated(
        &text,
        "// BEGIN GENERATED operator descriptors",
        "// END GENERATED operator descriptors",
        &block,
    )
    .expect("markers in Blocks.qml");
    if std::env::var_os("BLESS").is_some() {
        std::fs::write(path, &next).unwrap();
    } else {
        assert!(
            next == text,
            "Blocks.qml's operator table is stale; rerun with BLESS=1"
        );
    }
}
