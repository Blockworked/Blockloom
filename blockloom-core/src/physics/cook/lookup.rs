//! Where cooked collision data comes from: the editor cooks on demand and caches
//! under the project; a shipped game only reads what the build put there.

use super::{
    COOKER_VERSION, CookControl, CookSettings, Cooked, MeshKind, TARGET_FORMAT, content_key,
    load::load_mesh,
};
use crate::project::Project;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// A folder inside a project that holds cooked collision.
pub const COOKED_DIR: &str = ".blockloom/cooked/collision";
/// The manifest a build ships so the player finds data without the sources.
pub const MANIFEST: &str = "index.json";

/// What the physics plan asks when a collider names a mesh.
pub trait CollisionLookup {
    fn cooked(
        &self,
        mesh: &str,
        kind: MeshKind,
        settings: &CookSettings,
    ) -> Result<Arc<Cooked>, String>;
}

/// A lookup with nothing behind it: a plan made without a project folder.
pub struct NoCollisionData;

impl CollisionLookup for NoCollisionData {
    fn cooked(&self, mesh: &str, _: MeshKind, _: &CookSettings) -> Result<Arc<Cooked>, String> {
        Err(format!(
            "Collision data for \"{mesh}\" is not available without a project folder to cook it in"
        ))
    }
}

/// Names one cooking request independent of the source file's bytes: what a
/// shipped build looks up by.
pub fn request_id(mesh: &str, kind: MeshKind, settings: &CookSettings) -> String {
    let decompose = settings
        .decomposition_of(mesh)
        .map(|d| serde_json::to_string(d).unwrap_or_default())
        .unwrap_or_default();
    content_key(&[
        mesh.as_bytes(),
        kind.name().as_bytes(),
        &settings.weld.to_le_bytes(),
        &settings.max_hull_vertices.to_le_bytes(),
        decompose.as_bytes(),
        &COOKER_VERSION.to_le_bytes(),
        TARGET_FORMAT.as_bytes(),
    ])
}

/// How a [`FolderCollision`] treats a miss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Cook from the model file and cache the result.
    Cook,
    /// Read the build's manifest only; a missing entry is an error.
    Shipped,
}

/// Cooked data in a project folder.
pub struct FolderCollision {
    dir: PathBuf,
    source: Source,
    control: CookControl,
    memory: Mutex<HashMap<String, Arc<Cooked>>>,
    /// Requests served, for the build manifest: request id -> (key, mesh, kind).
    served: Mutex<BTreeMap<String, (String, String, MeshKind)>>,
}

impl FolderCollision {
    pub fn new(dir: &Path, source: Source) -> Self {
        Self {
            dir: dir.to_path_buf(),
            source,
            control: CookControl::new(),
            memory: Mutex::default(),
            served: Mutex::default(),
        }
    }

    /// Cooks in the editor, cancellable and reporting progress.
    pub fn with_control(mut self, control: CookControl) -> Self {
        self.control = control;
        self
    }

    fn cooked_dir(&self) -> PathBuf {
        self.dir.join(COOKED_DIR)
    }

    fn shipped(&self, id: &str, mesh: &str) -> Result<Arc<Cooked>, String> {
        let manifest_path = self.cooked_dir().join(MANIFEST);
        let text = crate::vfs::read_to_string(&manifest_path).map_err(|_| {
            format!("The build carries no collision data, so \"{mesh}\" cannot collide")
        })?;
        let manifest: BTreeMap<String, String> = serde_json::from_str(&text)
            .map_err(|e| format!("The collision manifest is damaged: {e}"))?;
        let key = manifest
            .get(id)
            .ok_or_else(|| format!("The build carries no collision data for \"{mesh}\""))?;
        let bytes = crate::vfs::read(&self.cooked_dir().join(format!("{key}.cook")))
            .map_err(|e| format!("Collision data for \"{mesh}\" could not be read: {e}"))?;
        Cooked::from_bytes(&bytes)
            .map(Arc::new)
            .map_err(|why| format!("Collision data for \"{mesh}\" is unusable: {why}"))
    }

    fn cook(
        &self,
        mesh: &str,
        kind: MeshKind,
        settings: &CookSettings,
    ) -> Result<Arc<Cooked>, String> {
        let path = self.dir.join(mesh);
        let bytes = crate::vfs::read(&path).map_err(|e| format!("{mesh}: {e}"))?;
        let id = request_id(mesh, kind, settings);
        let key = content_key(&[&bytes, id.as_bytes()]);
        if let Some(hit) = self.memory.lock().unwrap().get(&key) {
            self.note(&id, &key, mesh, kind);
            return Ok(hit.clone());
        }
        let file = self.cooked_dir().join(format!("{key}.cook"));
        let cooked = match std::fs::read(&file)
            .ok()
            .and_then(|b| Cooked::from_bytes(&b).ok())
        {
            Some(cooked) => cooked,
            None => {
                let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
                let resolve = |name: &str| crate::vfs::read(&parent.join(name)).ok();
                let raw = load_mesh(mesh, &bytes, &resolve).map_err(|e| e.to_string())?;
                let cooked = raw
                    .cook(
                        kind,
                        settings,
                        settings.decomposition_of(mesh),
                        &self.control,
                    )
                    .map_err(|e| format!("{mesh}: {e}"))?;
                // A cache that can't be written costs a recook, not the cook.
                if std::fs::create_dir_all(self.cooked_dir()).is_ok() {
                    let tmp = file.with_extension("tmp");
                    if std::fs::write(&tmp, cooked.to_bytes()).is_ok() {
                        let _ = std::fs::rename(&tmp, &file);
                    }
                }
                cooked
            }
        };
        let cooked = Arc::new(cooked);
        self.memory
            .lock()
            .unwrap()
            .insert(key.clone(), cooked.clone());
        self.note(&id, &key, mesh, kind);
        Ok(cooked)
    }

    fn note(&self, id: &str, key: &str, mesh: &str, kind: MeshKind) {
        self.served
            .lock()
            .unwrap()
            .insert(id.to_string(), (key.to_string(), mesh.to_string(), kind));
    }

    /// Writes the manifest for everything served so far, and returns the files
    /// a build ships: `(path inside the folder, bytes)` relative to the project.
    pub fn manifest_files(&self) -> Result<Vec<(String, Vec<u8>)>, String> {
        let served = self.served.lock().unwrap();
        let mut manifest: BTreeMap<&str, &str> = BTreeMap::new();
        let mut files = Vec::new();
        for (id, (key, _, _)) in served.iter() {
            manifest.insert(id, key);
            let path = format!("{COOKED_DIR}/{key}.cook");
            let bytes = std::fs::read(self.dir.join(&path))
                .map_err(|e| format!("Cooked collision {key} is missing: {e}"))?;
            files.push((path, bytes));
        }
        if files.is_empty() {
            return Ok(files);
        }
        files.push((
            format!("{COOKED_DIR}/{MANIFEST}"),
            serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
        ));
        Ok(files)
    }

    /// What was cooked or read, for a report.
    pub fn served(&self) -> Vec<(String, MeshKind, Arc<Cooked>)> {
        let served = self.served.lock().unwrap();
        let memory = self.memory.lock().unwrap();
        served
            .values()
            .filter_map(|(key, mesh, kind)| Some((mesh.clone(), *kind, memory.get(key)?.clone())))
            .collect()
    }
}

impl CollisionLookup for FolderCollision {
    fn cooked(
        &self,
        mesh: &str,
        kind: MeshKind,
        settings: &CookSettings,
    ) -> Result<Arc<Cooked>, String> {
        let id = request_id(mesh, kind, settings);
        match self.source {
            Source::Shipped => {
                if let Some(hit) = self.memory.lock().unwrap().get(&id) {
                    return Ok(hit.clone());
                }
                let cooked = self.shipped(&id, mesh)?;
                self.memory.lock().unwrap().insert(id, cooked.clone());
                Ok(cooked)
            }
            Source::Cook => self.cook(mesh, kind, settings),
        }
    }
}

/// What cooking a whole project made.
#[derive(Debug, Clone, Default)]
pub struct CookReport {
    pub entries: Vec<CookedEntry>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CookedEntry {
    pub mesh: String,
    pub kind: MeshKind,
    pub stats: super::CookStats,
}

/// Cooks every collision mesh any scene of `project` asks for, through the same
/// plan Play uses, and returns the lookup (for its manifest) with a report.
pub fn cook_project(
    project: &Project,
    dir: &Path,
    control: CookControl,
) -> (FolderCollision, CookReport) {
    let lookup = FolderCollision::new(dir, Source::Cook).with_control(control);
    let mut report = CookReport::default();
    for scene in &project.scenes {
        let plan = scene.physics_plan_with(&project.physics, &lookup);
        for issue in plan.errors() {
            if !report.errors.contains(&issue.message) {
                report.errors.push(issue.message.clone());
            }
        }
    }
    for (mesh, kind, cooked) in lookup.served() {
        report.entries.push(CookedEntry {
            mesh,
            kind,
            stats: cooked.stats().clone(),
        });
    }
    (lookup, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::cook::tests::cube;

    fn obj_cube(dir: &Path, name: &str) {
        let mesh = cube(2.0);
        let mut text = String::new();
        for p in &mesh.positions {
            text.push_str(&format!("v {} {} {}\n", p[0], p[1], p[2]));
        }
        for t in mesh.indices.chunks(3) {
            text.push_str(&format!("f {} {} {}\n", t[0] + 1, t[1] + 1, t[2] + 1));
        }
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join(name), text).unwrap();
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("blockloom-cook-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_cook_is_cached_on_disk_and_reused_until_the_source_changes() {
        let dir = temp("cache");
        obj_cube(&dir, "assets/crate.obj");
        let settings = CookSettings::default();
        let a = FolderCollision::new(&dir, Source::Cook);
        let first = a
            .cooked("assets/crate.obj", MeshKind::Hull, &settings)
            .unwrap();
        assert_eq!(first.stats().hulls, 1);
        let files: Vec<_> = std::fs::read_dir(dir.join(COOKED_DIR)).unwrap().collect();
        assert_eq!(files.len(), 1);
        // A new lookup finds the file rather than cooking again: break the
        // source's meaning but keep its bytes by reading through the cache.
        let b = FolderCollision::new(&dir, Source::Cook);
        let second = b
            .cooked("assets/crate.obj", MeshKind::Hull, &settings)
            .unwrap();
        assert_eq!(*first, *second);
        // Another kind, and another source, are other cache entries.
        b.cooked("assets/crate.obj", MeshKind::Triangles, &settings)
            .unwrap();
        assert_eq!(std::fs::read_dir(dir.join(COOKED_DIR)).unwrap().count(), 2);
        std::fs::write(
            dir.join("assets/crate.obj"),
            "v 0 0 0\nv 3 0 0\nv 0 3 0\nv 0 0 3\nf 1 3 2\nf 1 2 4\nf 1 4 3\nf 2 3 4\n",
        )
        .unwrap();
        let c = FolderCollision::new(&dir, Source::Cook);
        let changed = c
            .cooked("assets/crate.obj", MeshKind::Hull, &settings)
            .unwrap();
        assert_ne!(*first, *changed);
    }

    #[test]
    fn a_missing_or_unreadable_model_is_an_error_with_its_name() {
        let dir = temp("missing");
        let lookup = FolderCollision::new(&dir, Source::Cook);
        let err = lookup
            .cooked(
                "assets/nothing.glb",
                MeshKind::Hull,
                &CookSettings::default(),
            )
            .unwrap_err();
        assert!(err.contains("assets/nothing.glb"), "{err}");
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("assets/bad.obj"), "hello").unwrap();
        let err = lookup
            .cooked("assets/bad.obj", MeshKind::Hull, &CookSettings::default())
            .unwrap_err();
        assert!(err.contains("bad.obj"), "{err}");
    }

    #[test]
    fn a_shipped_folder_reads_its_manifest_and_never_cooks() {
        let dir = temp("shipped-src");
        obj_cube(&dir, "assets/crate.obj");
        let settings = CookSettings::default();
        let cooker = FolderCollision::new(&dir, Source::Cook);
        let cooked = cooker
            .cooked("assets/crate.obj", MeshKind::Hull, &settings)
            .unwrap();
        let files = cooker.manifest_files().unwrap();
        assert_eq!(files.len(), 2, "one cooked file and the manifest");

        // The game folder has the data and no model to cook from.
        let game = temp("shipped-game");
        for (path, bytes) in &files {
            let at = game.join(path);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::write(at, bytes).unwrap();
        }
        let player = FolderCollision::new(&game, Source::Shipped);
        let read = player
            .cooked("assets/crate.obj", MeshKind::Hull, &settings)
            .unwrap();
        assert_eq!(*read, *cooked);
        let err = player
            .cooked("assets/crate.obj", MeshKind::Triangles, &settings)
            .unwrap_err();
        assert!(err.contains("no collision data"), "{err}");
        let empty = FolderCollision::new(&temp("shipped-none"), Source::Shipped);
        assert!(
            empty
                .cooked("assets/crate.obj", MeshKind::Hull, &settings)
                .is_err()
        );
    }

    #[test]
    fn settings_that_change_the_output_change_the_request() {
        let mut settings = CookSettings::default();
        let a = request_id("m.glb", MeshKind::Hull, &settings);
        settings.max_hull_vertices = 64;
        assert_ne!(a, request_id("m.glb", MeshKind::Hull, &settings));
        assert_ne!(
            a,
            request_id("m.glb", MeshKind::Triangles, &CookSettings::default())
        );
        assert_ne!(
            a,
            request_id("n.glb", MeshKind::Hull, &CookSettings::default())
        );
    }
}
