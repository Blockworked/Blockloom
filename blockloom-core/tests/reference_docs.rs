//! Holds `docs/reference/` to the type registry (`BLESS=1` rewrites it).

use std::path::PathBuf;

#[test]
fn the_reference_docs_match_the_registry() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/reference");
    let bless = std::env::var_os("BLESS").is_some();
    if bless {
        std::fs::create_dir_all(&dir).unwrap();
    }
    for (name, text) in blockloom_core::reference::pages() {
        let path = dir.join(name);
        if bless {
            std::fs::write(&path, &text).unwrap();
            continue;
        }
        let held = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            held == text,
            "docs/reference/{name} is stale; rerun with BLESS=1 cargo test -p blockloom-core --test reference_docs"
        );
    }
}
