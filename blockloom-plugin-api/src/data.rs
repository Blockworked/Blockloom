//! Where a plugin's project data lives and how a build lists it.
//!
//! A plugin keeps blobs in `<project>/.blockloom/plugin-data/<plugin id>/`.
//! A build copies them beside the game and writes an index, because a player
//! may read files from an APK or a page's mounted files and cannot list a
//! folder.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The folder, relative to a project or a built game.
pub const DATA_DIR: &str = ".blockloom/plugin-data";
/// The index a build writes inside [`DATA_DIR`].
pub const INDEX_FILE: &str = "index.json";

/// Every shipped data file by its store key (`<plugin id>/<key>`) with its size.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataIndex {
    pub files: BTreeMap<String, u64>,
}
