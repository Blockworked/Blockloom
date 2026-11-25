//! The declared verbs reach every generated surface: the script prelude's
//! typed methods and the palette table in Blocks.qml (`BLESS=1` rewrites
//! both).

use blockloom_core::value::replace_generated;
use blockloom_core::verbs;

fn check(path: &str, current: &str, next: &str, what: &str) {
    if std::env::var_os("BLESS").is_some() {
        std::fs::write(path, next).unwrap();
    } else {
        assert!(next == current, "{what} is stale; rerun with BLESS=1");
    }
}

#[test]
fn the_script_prelude_carries_a_method_per_verb() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/script/verbs_gen.rs");
    let current = std::fs::read_to_string(path).unwrap_or_default();
    check(
        path,
        &current,
        &verbs::render_prelude(),
        "script/verbs_gen.rs",
    );
}

#[test]
fn blocks_qml_carries_the_verb_table() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../blockloom-qt/qml/Blocks.qml"
    );
    let current = std::fs::read_to_string(path).unwrap();
    let block = format!(
        "    readonly property var builtinVerbs: {}",
        verbs::render_qml("    ")
    );
    let next = replace_generated(
        &current,
        "// BEGIN GENERATED verbs",
        "// END GENERATED verbs",
        &block,
    )
    .expect("verb markers in Blocks.qml");
    check(path, &current, &next, "Blocks.qml's verb table");
}
