//! Installing, updating and removing plugins as one transaction.
//!
//! A change is planned first (resolve the whole graph, touch nothing), then
//! applied: fetch into staging, verify every package against its hash, publish
//! into the shared cache, and only then rewrite the project's two files. Any
//! failure before that last step leaves the project exactly as it was.

use crate::cache::{Cache, Staging};
use crate::lock::{DirectDependency, LockFile, LockedPackage, PluginsFile, ProjectPlugins};
use crate::package::Package;
use crate::registry::{DirRegistry, Registry};
use crate::resolver::{Candidate, CandidateProvider, Options, Request, Resolution, resolve};
use crate::source::{CACHE_REGISTRY, Source, fetch_local};
use blockloom_plugin_api::{Version, VersionReq};
use serde::Serialize;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// What an install needs to know about the machine and the engine.
pub struct Environment {
    pub cache: Cache,
    pub engine: Version,
    /// The target triple the project is edited and played on.
    pub target: String,
    /// Use only the cache and local sources. A package that is not already
    /// cached then fails with a clear message instead of reaching out.
    pub offline: bool,
    /// Registries every project can use, by name.
    pub registries: BTreeMap<String, Box<dyn Registry>>,
}

#[derive(Debug, Clone)]
pub enum Change {
    /// Make the project's files agree: install exactly what the lock says,
    /// resolving only if `plugins.json` asks for something the lock lacks.
    Sync,
    Add {
        id: String,
        req: VersionReq,
        source: Option<Source>,
        features: Vec<String>,
    },
    Remove(String),
    /// Move the named plugins (all of them when empty) to the newest
    /// versions their requirements allow.
    Update(Vec<String>),
    Pin {
        id: String,
        version: Version,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum PlanChange {
    Added {
        id: String,
        version: Version,
    },
    Removed {
        id: String,
        version: Version,
    },
    Changed {
        id: String,
        from: Version,
        to: Version,
    },
}

pub struct Plan {
    pub plugins: PluginsFile,
    pub lock: LockFile,
    pub changes: Vec<PlanChange>,
    /// Ids loaded in place from local-development overrides.
    dev: BTreeSet<String>,
}

fn diff(old: &LockFile, new: &LockFile) -> Vec<PlanChange> {
    let mut out = Vec::new();
    for package in &new.packages {
        match old.get(&package.id) {
            None => out.push(PlanChange::Added {
                id: package.id.clone(),
                version: package.version.clone(),
            }),
            Some(before) if before.version != package.version || before.hash != package.hash => out
                .push(PlanChange::Changed {
                    id: package.id.clone(),
                    from: before.version.clone(),
                    to: package.version.clone(),
                }),
            Some(_) => {}
        }
    }
    for package in &old.packages {
        if new.get(&package.id).is_none() {
            out.push(PlanChange::Removed {
                id: package.id.clone(),
                version: package.version.clone(),
            });
        }
    }
    out
}

/// Whether `lock` already answers everything `plugins` asks for.
pub fn lock_satisfies(plugins: &PluginsFile, lock: &LockFile) -> bool {
    let mut needed: Vec<&str> = Vec::new();
    for (id, dep) in &plugins.plugins {
        let Some(locked) = lock.get(id) else {
            return false;
        };
        if !dep.version.matches(&locked.version) {
            return false;
        }
        if let Some(source) = &dep.source
            && source != &locked.source
        {
            return false;
        }
        needed.push(id);
    }
    // Every dependency a locked package names must itself be locked, and
    // nothing may be left locked that nobody reaches.
    let mut reached: BTreeSet<&str> = BTreeSet::new();
    let mut stack = needed;
    while let Some(id) = stack.pop() {
        if !reached.insert(id) {
            continue;
        }
        let Some(locked) = lock.get(id) else {
            return false;
        };
        stack.extend(locked.dependencies.iter().map(String::as_str));
    }
    reached.len() == lock.packages.len()
}

struct Provider<'a> {
    env: &'a Environment,
    project: &'a ProjectPlugins,
    plugins: &'a PluginsFile,
    lock: &'a LockFile,
    overrides: BTreeMap<String, PathBuf>,
    staging: &'a Staging,
    counter: RefCell<usize>,
    dev: RefCell<BTreeSet<String>>,
}

impl Provider<'_> {
    fn registry_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.plugins.registries.keys().cloned().collect();
        for name in self.env.registries.keys() {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
        names
    }

    fn versions_in(&self, name: &str, id: &str) -> Result<Vec<Candidate>, String> {
        let entries = if let Some(path) = self.plugins.registries.get(name) {
            DirRegistry::open(self.project.dir.join(path)).versions(id)?
        } else if let Some(registry) = self.env.registries.get(name) {
            registry.versions(id)?
        } else {
            return Err(format!("no registry named \"{name}\" is configured"));
        };
        Ok(entries
            .into_iter()
            .map(|e| Candidate {
                manifest: e.manifest,
                source: Source::Registry(name.to_string()),
                hash: e.content_hash,
            })
            .collect())
    }

    /// Loads a non-registry source into staging just to read its manifest.
    fn load_source(&self, id: &str, source: &Source) -> Result<Candidate, String> {
        if self.env.offline && source.is_remote() {
            return Err(format!(
                "{id}: {source} needs the network and the install is offline"
            ));
        }
        let anchored = source.anchored(&self.project.dir);
        let n = {
            let mut c = self.counter.borrow_mut();
            *c += 1;
            *c
        };
        let dir = self.staging.subdir(&format!("probe-{n}"))?;
        let root = fetch_local(&anchored, &dir)?;
        let package = Package::load(&root)?;
        if package.manifest.id != id {
            return Err(format!("{source} holds {}, not {id}", package.manifest.id));
        }
        Ok(Candidate {
            manifest: package.manifest,
            source: source.clone(),
            hash: package.content_hash,
        })
    }
}

impl CandidateProvider for Provider<'_> {
    fn candidates(&self, id: &str, source: Option<&Source>) -> Result<Vec<Candidate>, String> {
        if let Some(path) = self.overrides.get(id) {
            let package = Package::load(&self.project.dir.join(path))?;
            self.dev.borrow_mut().insert(id.to_string());
            return Ok(vec![Candidate {
                manifest: package.manifest,
                source: Source::Path(path.clone()),
                hash: package.content_hash,
            }]);
        }
        match source {
            Some(Source::Registry(name)) if name != CACHE_REGISTRY => {
                if self.env.offline {
                    return Err(format!("{id}: registry:{name} is unavailable offline"));
                }
                return self.versions_in(name, id);
            }
            Some(Source::Registry(_)) | None => {}
            Some(other) => return Ok(vec![self.load_source(id, other)?]),
        }
        let mut out: Vec<Candidate> = Vec::new();
        let mut push = |c: Candidate| {
            if !out.iter().any(|o| o.manifest.version == c.manifest.version) {
                out.push(c);
            }
        };
        // What the lock names comes first, with the source it came from.
        if let Some(locked) = self.lock.get(id)
            && let Some(dir) = self.env.cache.get(id, &locked.version, &locked.hash)
            && let Ok(package) = Package::load(&dir)
        {
            push(Candidate {
                manifest: package.manifest,
                source: locked.source.clone(),
                hash: package.content_hash,
            });
        }
        if source.is_none() {
            for name in self.registry_names() {
                if self.env.offline {
                    break;
                }
                for candidate in self.versions_in(&name, id)? {
                    push(candidate);
                }
            }
        }
        for package in self.env.cache.versions(id)? {
            push(Candidate {
                manifest: package.manifest,
                source: Source::Registry(CACHE_REGISTRY.to_string()),
                hash: package.content_hash,
            });
        }
        Ok(out)
    }
}

fn requests(plugins: &PluginsFile) -> Vec<Request> {
    plugins
        .plugins
        .iter()
        .map(|(id, dep)| Request {
            id: id.clone(),
            req: dep.version.clone(),
            source: dep.source.clone(),
            features: dep.features.clone(),
        })
        .collect()
}

fn lock_from(resolution: &Resolution, dev: &BTreeSet<String>) -> LockFile {
    let mut packages = Vec::new();
    for id in &resolution.order {
        let candidate = &resolution.packages[id];
        packages.push(LockedPackage {
            id: id.clone(),
            version: candidate.manifest.version.clone(),
            source: candidate.source.clone(),
            hash: candidate.hash.clone(),
            dependencies: resolution.dependencies_of(id),
            targets: BTreeMap::new(),
            editor_only: Vec::new(),
            has_code: false,
            dev: dev.contains(id),
        });
    }
    LockFile {
        packages,
        ..LockFile::default()
    }
}

/// Works out what `change` would do, without writing or fetching anything
/// into the cache or the project.
pub fn plan(env: &Environment, project: &ProjectPlugins, change: Change) -> Result<Plan, String> {
    let mut plugins = project.read_plugins()?;
    let lock = project.read_lock()?;
    let overrides = project.read_local()?.overrides;
    let mut upgrade = BTreeSet::new();
    let mut force_resolve = false;
    match change {
        Change::Sync => {}
        Change::Add {
            id,
            req,
            source,
            features,
        } => {
            blockloom_plugin_api::id::validate_plugin_id(&id)?;
            plugins.plugins.insert(
                id.clone(),
                DirectDependency {
                    version: req,
                    source,
                    features,
                },
            );
            upgrade.insert(id);
            force_resolve = true;
        }
        Change::Remove(id) => {
            if plugins.plugins.remove(&id).is_none() {
                return Err(match lock.dependents(&id).first() {
                    Some(parent) => format!(
                        "{id} is not a direct dependency; it is required by {}",
                        parent.id
                    ),
                    None => format!("{id} is not installed in this project"),
                });
            }
            force_resolve = true;
        }
        Change::Update(ids) => {
            upgrade = if ids.is_empty() {
                lock.packages.iter().map(|p| p.id.clone()).collect()
            } else {
                for id in &ids {
                    if lock.get(id).is_none() {
                        return Err(format!("{id} is not installed in this project"));
                    }
                }
                ids.into_iter().collect()
            };
            force_resolve = true;
        }
        Change::Pin { id, version } => {
            let Some(dep) = plugins.plugins.get_mut(&id) else {
                return Err(format!("{id} is not a direct dependency of this project"));
            };
            dep.version = format!("={version}")
                .parse()
                .map_err(|e: semver::Error| e.to_string())?;
            upgrade.insert(id);
            force_resolve = true;
        }
    }

    if !force_resolve && lock_satisfies(&plugins, &lock) && overrides.is_empty() {
        let changes = Vec::new();
        return Ok(Plan {
            plugins,
            lock,
            changes,
            dev: BTreeSet::new(),
        });
    }

    let staging = env.cache.staging()?;
    let provider = Provider {
        env,
        project,
        plugins: &plugins,
        lock: &lock,
        overrides,
        staging: &staging,
        counter: RefCell::new(0),
        dev: RefCell::new(BTreeSet::new()),
    };
    let options = Options {
        engine: env.engine.clone(),
        locked: lock
            .packages
            .iter()
            .map(|p| (p.id.clone(), p.version.clone()))
            .collect(),
        upgrade,
    };
    let resolution =
        resolve(&requests(&plugins), &provider, &options).map_err(|e| e.to_string())?;
    let dev = provider.dev.borrow().clone();
    let mut new_lock = lock_from(&resolution, &dev);
    // Entries the resolution kept as they were keep what they recorded.
    for package in &mut new_lock.packages {
        if let Some(old) = lock.get(&package.id)
            && old.hash == package.hash
        {
            package.targets = old.targets.clone();
            package.editor_only = old.editor_only.clone();
            package.has_code = old.has_code;
        }
    }
    let changes = diff(&lock, &new_lock);
    Ok(Plan {
        plugins,
        lock: new_lock,
        changes,
        dev,
    })
}

/// Gets one locked package into the cache and verifies it, returning the
/// verified package.
fn ensure_cached(
    env: &Environment,
    project: &ProjectPlugins,
    plugins: &PluginsFile,
    locked: &LockedPackage,
    staging: &Staging,
    n: usize,
) -> Result<Package, String> {
    if let Some(dir) = env.cache.get(&locked.id, &locked.version, &locked.hash) {
        return Package::load(&dir);
    }
    if env.offline && locked.source.is_remote() {
        return Err(format!(
            "{} {} is not in the cache and the install is offline",
            locked.id, locked.version
        ));
    }
    let dir = staging.subdir(&format!("fetch-{n}"))?;
    match &locked.source {
        Source::Registry(name) if name == CACHE_REGISTRY => Err(format!(
            "{} {} is not in the cache (it was installed from a cache that no longer holds it)",
            locked.id, locked.version
        )),
        Source::Registry(name) => {
            if let Some(path) = plugins.registries.get(name) {
                DirRegistry::open(project.dir.join(path)).fetch(
                    &locked.id,
                    &locked.version,
                    &dir,
                )?;
            } else if let Some(registry) = env.registries.get(name) {
                registry.fetch(&locked.id, &locked.version, &dir)?;
            } else {
                return Err(format!("no registry named \"{name}\" is configured"));
            }
            verify_and_publish(env, locked, &dir)
        }
        other => {
            let root = fetch_local(&other.anchored(&project.dir), &dir)?;
            verify_and_publish(env, locked, &root)
        }
    }
}

fn verify_and_publish(
    env: &Environment,
    locked: &LockedPackage,
    root: &Path,
) -> Result<Package, String> {
    let package = Package::load(root)?;
    if package.manifest.id != locked.id || package.manifest.version != locked.version {
        return Err(format!(
            "expected {} {}, found {} {}",
            locked.id, locked.version, package.manifest.id, package.manifest.version
        ));
    }
    if package.content_hash != locked.hash {
        return Err(format!(
            "{} {}: content differs from what plugins.lock recorded; refusing to install it",
            locked.id, locked.version
        ));
    }
    env.cache.publish(root)
}

/// Fetches and verifies everything the plan needs, then replaces the
/// project's files. Returns the changes made.
pub fn apply(
    env: &Environment,
    project: &ProjectPlugins,
    mut plan: Plan,
) -> Result<Vec<PlanChange>, String> {
    let staging = env.cache.staging()?;
    let mut verified: Vec<Package> = Vec::new();
    for (n, locked) in plan.lock.packages.iter().enumerate() {
        if plan.dev.contains(&locked.id) || locked.dev {
            let path = match &locked.source {
                Source::Path(p) => project.dir.join(p),
                other => {
                    return Err(format!(
                        "{}: a development override must be a path, not {other}",
                        locked.id
                    ));
                }
            };
            verified.push(Package::load(&path)?);
            continue;
        }
        verified.push(ensure_cached(
            env,
            project,
            &plan.plugins,
            locked,
            &staging,
            n,
        )?);
    }
    // Everything verified: record what the packages themselves say.
    for (locked, package) in plan.lock.packages.iter_mut().zip(&verified) {
        locked.targets = package.target_hashes();
        locked.editor_only = package.editor_only_types();
        locked.has_code =
            package.manifest.tier != blockloom_plugin_api::manifest::Tier::Declarative;
        if locked.dev {
            locked.hash = package.content_hash.clone();
        }
        let support = package.manifest.support_for(&env.target);
        if !support.is_supported() {
            let blockloom_plugin_api::manifest::TargetSupport::Unsupported { reason } = support
            else {
                unreachable!()
            };
            return Err(format!(
                "{} cannot run on {}: {reason}",
                locked.id, env.target
            ));
        }
    }
    let current = project.read_lock()?;
    let changed = current != plan.lock || project.read_plugins()? != plan.plugins;
    if changed {
        project.commit(&plan.plugins, &plan.lock)?;
    }
    Ok(plan.changes)
}

/// Plans and applies in one go.
pub fn install(
    env: &Environment,
    project: &ProjectPlugins,
    change: Change,
) -> Result<Vec<PlanChange>, String> {
    let plan = plan(env, project, change)?;
    apply(env, project, plan)
}

/// Restores the project's plugin files from before the last change. The old
/// packages are still cached, so this works offline.
pub fn rollback(env: &Environment, project: &ProjectPlugins) -> Result<Vec<PlanChange>, String> {
    let before = project.read_lock()?;
    let Some((plugins, lock)) = project.rollback()? else {
        return Err("nothing to roll back".to_string());
    };
    // Make sure what we rolled back to is actually installable.
    let staging = env.cache.staging()?;
    for (n, locked) in lock.packages.iter().enumerate() {
        if locked.dev {
            continue;
        }
        if let Err(e) = ensure_cached(env, project, &plugins, locked, &staging, n) {
            return Err(format!("rolled back, but {e}"));
        }
    }
    Ok(diff(&before, &lock))
}

/// The cache entries every lock file of this project needs, for garbage
/// collection: current and history.
pub fn keep_set(project: &ProjectPlugins) -> Result<BTreeSet<(String, String)>, String> {
    let mut locks = project.history_locks();
    locks.push(project.read_lock()?);
    Ok(locks
        .into_iter()
        .flat_map(|l| l.packages.into_iter().map(|p| (p.id, p.hash)))
        .collect())
}

/// A readable dependency tree for `id` (or every direct dependency).
pub fn tree(plugins: &PluginsFile, lock: &LockFile, only: Option<&str>) -> String {
    fn walk(lock: &LockFile, id: &str, depth: usize, out: &mut String, seen: &mut Vec<String>) {
        let indent = "  ".repeat(depth);
        match lock.get(id) {
            Some(p) => {
                out.push_str(&format!("{indent}{id} {}\n", p.version));
                if seen.iter().any(|s| s == id) {
                    return;
                }
                seen.push(id.to_string());
                for dep in &p.dependencies {
                    walk(lock, dep, depth + 1, out, seen);
                }
                seen.pop();
            }
            None => out.push_str(&format!("{indent}{id} (not locked)\n")),
        }
    }
    let mut out = String::new();
    for id in plugins
        .plugins
        .keys()
        .filter(|id| only.is_none_or(|o| o == id.as_str()))
    {
        walk(lock, id, 0, &mut out, &mut Vec::new());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::fixtures::declarative;
    use serde_json::json;
    use std::fs;

    struct Rig {
        dir: tempfile::TempDir,
        env: Environment,
        project: ProjectPlugins,
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let project_dir = dir.path().join("project");
        fs::create_dir_all(&project_dir).unwrap();
        let env = Environment {
            cache: Cache::new(dir.path().join("cache")),
            engine: Version::new(0, 0, 1),
            target: "x86_64-unknown-linux-gnu".to_string(),
            offline: false,
            registries: BTreeMap::new(),
        };
        Rig {
            project: ProjectPlugins::new(project_dir),
            env,
            dir,
        }
    }

    impl Rig {
        fn registry(&self) -> DirRegistry {
            DirRegistry::open(self.dir.path().join("reg"))
        }

        /// Publishes into the project's registry and configures it.
        fn publish(&self, id: &str, version: &str, deps: serde_json::Value) {
            let pkg = declarative(&self.dir.path().join("src"), id, version, deps);
            self.registry().publish(&pkg).unwrap();
            let mut plugins = self.project.read_plugins().unwrap();
            plugins.registries.insert("main".into(), "../reg".into());
            fs::write(
                self.project.plugins_path(),
                serde_json::to_string(&plugins).unwrap(),
            )
            .unwrap();
        }

        fn add(&self, id: &str, req: &str) -> Result<Vec<PlanChange>, String> {
            install(
                &self.env,
                &self.project,
                Change::Add {
                    id: id.into(),
                    req: req.parse().unwrap(),
                    source: None,
                    features: vec![],
                },
            )
        }

        fn locked(&self, id: &str) -> Option<Version> {
            self.project
                .read_lock()
                .unwrap()
                .get(id)
                .map(|p| p.version.clone())
        }
    }

    #[test]
    fn adding_a_registry_plugin_installs_its_dependencies() {
        let rig = rig();
        rig.publish("com.x.base", "1.0.0", json!({}));
        rig.publish("com.x.top", "1.0.0", json!({"com.x.base": "^1"}));
        let changes = rig.add("com.x.top", "^1").unwrap();
        assert_eq!(changes.len(), 2);
        let lock = rig.project.read_lock().unwrap();
        assert_eq!(lock.packages.len(), 2);
        assert_eq!(lock.get("com.x.top").unwrap().dependencies, ["com.x.base"]);
        assert!(lock.get("com.x.top").unwrap().targets.is_empty());
        // Both live in the shared cache now.
        assert_eq!(rig.env.cache.ids().len(), 2);
        let plugins = rig.project.read_plugins().unwrap();
        assert!(plugins.plugins.contains_key("com.x.top"));
        assert!(
            !plugins.plugins.contains_key("com.x.base"),
            "only direct dependencies"
        );
        assert_eq!(
            tree(&plugins, &lock, None),
            "com.x.top 1.0.0\n  com.x.base 1.0.0\n"
        );
    }

    #[test]
    fn sync_never_upgrades_but_update_does() {
        let rig = rig();
        rig.publish("com.x.a", "1.0.0", json!({}));
        rig.add("com.x.a", "^1").unwrap();
        rig.publish("com.x.a", "1.1.0", json!({}));

        assert!(
            install(&rig.env, &rig.project, Change::Sync)
                .unwrap()
                .is_empty()
        );
        assert_eq!(rig.locked("com.x.a"), Some(Version::new(1, 0, 0)));

        // Even re-resolving for another reason keeps what is locked.
        rig.publish("com.x.b", "1.0.0", json!({}));
        rig.add("com.x.b", "*").unwrap();
        assert_eq!(rig.locked("com.x.a"), Some(Version::new(1, 0, 0)));

        let changes = install(
            &rig.env,
            &rig.project,
            Change::Update(vec!["com.x.a".into()]),
        )
        .unwrap();
        assert_eq!(
            changes,
            [PlanChange::Changed {
                id: "com.x.a".into(),
                from: Version::new(1, 0, 0),
                to: Version::new(1, 1, 0)
            }]
        );
    }

    #[test]
    fn a_failed_install_leaves_the_project_untouched() {
        let rig = rig();
        rig.publish("com.x.a", "1.0.0", json!({}));
        rig.add("com.x.a", "^1").unwrap();
        rig.publish("com.x.c", "1.0.0", json!({}));
        let plugins_before = fs::read(rig.project.plugins_path()).unwrap();
        let lock_before = fs::read(rig.project.lock_path()).unwrap();

        // Unsatisfiable: nothing matches ^9.
        let error = rig.add("com.x.a", "^9").unwrap_err();
        assert!(error.contains("com.x.a"), "{error}");
        // Missing package.
        assert!(rig.add("com.x.nope", "*").is_err());
        // Corrupted archive in the registry.
        let entry = rig.registry().versions("com.x.c").unwrap().remove(0);
        fs::write(rig.dir.path().join("reg").join(entry.archive), b"junk").unwrap();
        assert!(rig.add("com.x.c", "*").is_err());

        assert_eq!(
            fs::read(rig.project.plugins_path()).unwrap(),
            plugins_before
        );
        assert_eq!(fs::read(rig.project.lock_path()).unwrap(), lock_before);
        // And no staging litter.
        let staging = rig.dir.path().join("cache/staging");
        assert!(!staging.exists() || fs::read_dir(staging).unwrap().count() == 0);
    }

    #[test]
    fn removing_checks_dependents_and_prunes_unused_dependencies() {
        let rig = rig();
        rig.publish("com.x.base", "1.0.0", json!({}));
        rig.publish("com.x.top", "1.0.0", json!({"com.x.base": "^1"}));
        rig.add("com.x.top", "*").unwrap();
        let error =
            install(&rig.env, &rig.project, Change::Remove("com.x.base".into())).unwrap_err();
        assert!(error.contains("required by com.x.top"), "{error}");
        let changes = install(&rig.env, &rig.project, Change::Remove("com.x.top".into())).unwrap();
        assert_eq!(changes.len(), 2);
        assert!(rig.project.read_lock().unwrap().packages.is_empty());
        // Removing never deletes cached packages another project may use.
        assert_eq!(rig.env.cache.ids().len(), 2);
    }

    #[test]
    fn rollback_restores_the_previous_state_offline() {
        let rig = rig();
        rig.publish("com.x.a", "1.0.0", json!({}));
        rig.add("com.x.a", "^1").unwrap();
        rig.publish("com.x.a", "1.1.0", json!({}));
        install(&rig.env, &rig.project, Change::Update(vec![])).unwrap();
        assert_eq!(rig.locked("com.x.a"), Some(Version::new(1, 1, 0)));

        let mut env = rig.env;
        env.offline = true;
        let changes = rollback(&env, &rig.project).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(
            rig.project
                .read_lock()
                .unwrap()
                .get("com.x.a")
                .unwrap()
                .version,
            Version::new(1, 0, 0)
        );
    }

    #[test]
    fn pin_holds_a_version_exactly() {
        let rig = rig();
        rig.publish("com.x.a", "1.0.0", json!({}));
        rig.publish("com.x.a", "1.2.0", json!({}));
        rig.add("com.x.a", "^1").unwrap();
        assert_eq!(rig.locked("com.x.a"), Some(Version::new(1, 2, 0)));
        install(
            &rig.env,
            &rig.project,
            Change::Pin {
                id: "com.x.a".into(),
                version: Version::new(1, 0, 0),
            },
        )
        .unwrap();
        assert_eq!(rig.locked("com.x.a"), Some(Version::new(1, 0, 0)));
        assert_eq!(
            rig.project.read_plugins().unwrap().plugins["com.x.a"]
                .version
                .to_string(),
            "=1.0.0"
        );
    }

    #[test]
    fn offline_installs_use_only_the_cache() {
        let mut rig = rig();
        rig.publish("com.x.a", "1.0.0", json!({}));
        rig.add("com.x.a", "^1").unwrap();

        // A second project with the same plugins.json/lock and a warm cache.
        let other = ProjectPlugins::new(rig.dir.path().join("other"));
        fs::create_dir_all(&other.dir).unwrap();
        fs::copy(rig.project.plugins_path(), other.plugins_path()).unwrap();
        fs::copy(rig.project.lock_path(), other.lock_path()).unwrap();
        rig.env.offline = true;
        install(&rig.env, &other, Change::Sync).unwrap();

        // A cold cache cannot be installed from offline, and says so.
        rig.env.cache = Cache::new(rig.dir.path().join("cold"));
        let error = install(&rig.env, &other, Change::Sync).unwrap_err();
        assert!(error.contains("offline"), "{error}");
        // Offline resolution only sees the cache, so a new plugin is not found.
        let error = rig.add("com.x.new", "*").unwrap_err();
        assert!(
            error.contains("not found") || error.contains("offline"),
            "{error}"
        );
    }

    #[test]
    fn a_changed_source_is_refused_against_the_lock() {
        let rig = rig();
        let pkg = declarative(&rig.dir.path().join("src"), "com.x.a", "1.0.0", json!({}));
        install(
            &rig.env,
            &rig.project,
            Change::Add {
                id: "com.x.a".into(),
                req: "*".parse().unwrap(),
                source: Some(Source::Path(pkg.clone())),
                features: vec![],
            },
        )
        .unwrap();
        // Tamper with the source in place (a re-tagged release) and install
        // into a project with a cold cache.
        fs::write(pkg.join("schemas/extra.json"), "{}").unwrap();
        crate::package::seal(&pkg).unwrap();
        let mut env = rig.env;
        env.cache = Cache::new(rig.dir.path().join("cold"));
        let error = install(&env, &rig.project, Change::Sync).unwrap_err();
        assert!(error.contains("differs"), "{error}");
    }

    #[test]
    fn an_unsupported_target_is_refused_before_anything_is_written() {
        let rig = rig();
        let root = rig.dir.path().join("src/com.x.native");
        fs::create_dir_all(root.join("runtime")).unwrap();
        fs::write(root.join("runtime/lib.so"), b"x").unwrap();
        fs::write(
            root.join("plugin.json"),
            serde_json::to_string(&json!({
                "format": 1, "id": "com.x.native", "name": "n", "version": "1.0.0",
                "engine": "*", "tier": "native", "abi": 1, "sdk": "*",
                "capabilities": ["native-execution"],
                "runtime": {"native": {"aarch64-apple-darwin": {"library": "runtime/lib.so"}}}
            }))
            .unwrap(),
        )
        .unwrap();
        crate::package::seal(&root).unwrap();
        let error = install(
            &rig.env,
            &rig.project,
            Change::Add {
                id: "com.x.native".into(),
                req: "*".parse().unwrap(),
                source: Some(Source::Path(root)),
                features: vec![],
            },
        )
        .unwrap_err();
        assert!(
            error.contains("cannot run on x86_64-unknown-linux-gnu"),
            "{error}"
        );
        assert!(!rig.project.exists());
    }

    #[test]
    fn local_overrides_load_in_place_and_stay_out_of_the_shared_files() {
        let rig = rig();
        rig.publish("com.x.a", "1.0.0", json!({}));
        let dev = declarative(&rig.dir.path().join("dev"), "com.x.a", "9.9.9", json!({}));
        fs::write(
            rig.project.dir.join(crate::lock::LOCAL_FILE),
            serde_json::to_string(&json!({"overrides": {"com.x.a": dev}})).unwrap(),
        )
        .unwrap();
        rig.add("com.x.a", "*").unwrap();
        let lock = rig.project.read_lock().unwrap();
        let locked = lock.get("com.x.a").unwrap();
        assert!(locked.dev);
        assert_eq!(locked.version, Version::new(9, 9, 9));
        assert!(
            rig.env.cache.ids().is_empty(),
            "a dev package never enters the cache"
        );
    }

    #[test]
    fn keep_set_covers_history_so_rollback_stays_possible() {
        let rig = rig();
        rig.publish("com.x.a", "1.0.0", json!({}));
        rig.add("com.x.a", "^1").unwrap();
        rig.publish("com.x.a", "1.1.0", json!({}));
        install(&rig.env, &rig.project, Change::Update(vec![])).unwrap();
        let keep = keep_set(&rig.project).unwrap();
        assert_eq!(keep.len(), 2);
        assert!(rig.env.cache.collect_garbage(&keep).unwrap().is_empty());
    }
}
