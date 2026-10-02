//! A project's plugins once loaded, and what they say about its data.
//!
//! [`ActivePlugins::load`] never fails as a whole: a package that cannot be
//! loaded becomes a [`PluginProblem`] and the rest still work, so one broken
//! plugin cannot lock an author out of their project. Records owned by a
//! plugin that is not active are reported by [`ActivePlugins::audit`], never
//! dropped.

use crate::cache::Cache;
use crate::lock::{LockFile, LockedPackage, ProjectPlugins};
use crate::package::Package;
use crate::source::Source;
use blockloom_plugin_api::loadout::{Loadout, LoadoutBlock, LoadoutPlugin};
use blockloom_plugin_api::manifest::{
    DependencyScope, PluginManifest, Tier, loads_native_libraries,
};
use blockloom_plugin_api::record::PluginRecord;
use blockloom_plugin_api::schema::{
    BlockKind, BlockSchema, BuildHookSchema, CommandAction, CommandSchema, ComponentSchema,
    HookSchema, ImporterSchema, PanelSchema, SchemaError, ToolSchema,
};
use blockloom_plugin_api::surfaces::{MenuSchema, OverlaySchema, ShortcutSchema};
use blockloom_plugin_api::{Version, id};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PluginProblem {
    pub id: String,
    pub message: String,
}

pub use blockloom_plugin_api::loadout::{CodeRuntime, NativeLibrary, PortableLibrary};

#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    pub package: Package,
    pub locked: LockedPackage,
}

#[derive(Debug, Default, Clone)]
pub struct ActivePlugins {
    pub plugins: BTreeMap<String, LoadedPlugin>,
    pub problems: Vec<PluginProblem>,
    /// What the lock says, loaded or not: it still knows which of a missing
    /// plugin's types were editor-only.
    pub locked: BTreeMap<String, LockedPackage>,
    target: String,
}

/// Two active plugins that don't work together.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PluginConflict {
    pub plugins: Vec<String>,
    pub kind: ConflictKind,
    pub message: String,
    /// Whether Play and Build stop until it is fixed.
    pub blocks_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    /// One names the other in its `conflicts`.
    Declared,
    /// Both provide the same service.
    Service,
    /// Their hooks ask to run in an order that can't be met.
    HookOrder,
    /// Both bind the same keys.
    Shortcut,
}

/// What is wrong, if anything, with one plugin record.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RecordStatus {
    Ok,
    /// Written at an older schema version; an explicit migration upgrades it.
    NeedsMigration {
        from: u32,
        to: u32,
    },
    /// The owning plugin is not installed, or failed to load.
    Missing {
        reason: String,
    },
    /// The plugin is active but defines no such type.
    UnknownType,
    /// Written by a newer version of the plugin than is installed.
    SchemaTooNew {
        found: u32,
        supported: u32,
    },
    Invalid {
        errors: Vec<SchemaError>,
    },
}

/// One record that needs attention, and whether it stops a run or build.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecordIssue {
    /// Where the record lives, e.g. `actor Player` or `resource`.
    pub location: String,
    pub record: String,
    #[serde(flatten)]
    pub status: RecordStatus,
    pub blocks_run: bool,
}

/// One file or package a built game carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShipEntry {
    pub id: String,
    pub version: Version,
    pub hash: String,
    pub tier: Tier,
    /// Package paths to copy, relative to the package root.
    pub files: Vec<String>,
    /// How it runs on the build target.
    pub support: blockloom_plugin_api::manifest::TargetSupport,
}

impl ActivePlugins {
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty() && self.problems.is_empty()
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    /// Loads everything the project's lock names.
    pub fn load(
        project: &ProjectPlugins,
        cache: &Cache,
        target: &str,
        engine: &Version,
    ) -> ActivePlugins {
        let mut active = ActivePlugins {
            target: target.to_string(),
            ..Default::default()
        };
        let lock = match project.read_lock() {
            Ok(lock) => lock,
            Err(e) => {
                active.problems.push(PluginProblem {
                    id: String::new(),
                    message: e,
                });
                return active;
            }
        };
        active.load_lock(project, &lock, cache, engine);
        active
    }

    fn load_lock(
        &mut self,
        project: &ProjectPlugins,
        lock: &LockFile,
        cache: &Cache,
        engine: &Version,
    ) {
        self.locked = lock
            .packages
            .iter()
            .map(|p| (p.id.clone(), p.clone()))
            .collect();
        // Dependencies first, so a broken one disables its dependents.
        let mut loaded: BTreeSet<String> = BTreeSet::new();
        let mut pending: Vec<&LockedPackage> = lock.packages.iter().collect();
        let mut failed: BTreeMap<String, String> = BTreeMap::new();
        while !pending.is_empty() {
            let before = pending.len();
            let mut next = Vec::new();
            for locked in pending {
                let waiting: Vec<&String> = locked
                    .dependencies
                    .iter()
                    .filter(|d| !loaded.contains(*d) && !failed.contains_key(*d))
                    .collect();
                if !waiting.is_empty() {
                    next.push(locked);
                    continue;
                }
                if let Some(dep) = locked.dependencies.iter().find(|d| failed.contains_key(*d)) {
                    let message = format!("requires {dep}, which is unavailable");
                    failed.insert(locked.id.clone(), message.clone());
                    self.problems.push(PluginProblem {
                        id: locked.id.clone(),
                        message,
                    });
                    continue;
                }
                match self.load_one(project, locked, cache, engine) {
                    Ok(package) => {
                        loaded.insert(locked.id.clone());
                        self.plugins.insert(
                            locked.id.clone(),
                            LoadedPlugin {
                                package,
                                locked: locked.clone(),
                            },
                        );
                    }
                    Err(message) => {
                        failed.insert(locked.id.clone(), message.clone());
                        self.problems.push(PluginProblem {
                            id: locked.id.clone(),
                            message,
                        });
                    }
                }
            }
            pending = next;
            if pending.len() == before {
                for locked in pending.drain(..) {
                    self.problems.push(PluginProblem {
                        id: locked.id.clone(),
                        message: "is part of a dependency cycle in plugins.lock".to_string(),
                    });
                }
            }
        }
    }

    fn load_one(
        &self,
        project: &ProjectPlugins,
        locked: &LockedPackage,
        cache: &Cache,
        engine: &Version,
    ) -> Result<Package, String> {
        let package = if locked.dev {
            let Source::Path(path) = &locked.source else {
                return Err("a development override must be a path".to_string());
            };
            Package::load(&project.dir.join(path))?
        } else {
            let dir = cache
                .get(&locked.id, &locked.version, &locked.hash)
                .ok_or_else(|| {
                    format!(
                        "{} {} is not installed in the plugin cache; run plugin-install",
                        locked.id, locked.version
                    )
                })?;
            let package = Package::load(&dir)?;
            if package.content_hash != locked.hash {
                return Err("cached content does not match plugins.lock".to_string());
            }
            package
        };
        if package.manifest.id != locked.id {
            return Err(format!("holds {}, not {}", package.manifest.id, locked.id));
        }
        if !package.manifest.engine.matches(engine) {
            return Err(format!(
                "needs engine {}, this is {engine}",
                package.manifest.engine
            ));
        }
        let support = package.manifest.support_for(&self.target);
        if let blockloom_plugin_api::manifest::TargetSupport::Unsupported { reason } = support {
            return Err(format!("cannot run on {}: {reason}", self.target));
        }
        package.shader_modules()?;
        Ok(package)
    }

    /// The pairs of active plugins that can't work together: declared
    /// conflicts, a shared service, hook orders that cannot be met, and keys
    /// bound twice (only the last never blocks a run).
    pub fn conflicts(&self) -> Vec<PluginConflict> {
        let mut out = Vec::new();
        let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
        let mut pair = |a: &str, b: &str| {
            let key = if a < b {
                (a.to_string(), b.to_string())
            } else {
                (b.to_string(), a.to_string())
            };
            seen.insert(key)
        };
        for (id, p) in &self.plugins {
            for other in &p.package.manifest.conflicts {
                if self.plugins.contains_key(other) && pair(id, other) {
                    out.push(PluginConflict {
                        plugins: vec![id.clone(), other.clone()],
                        kind: ConflictKind::Declared,
                        message: format!("{id} says it cannot be active beside {other}"),
                        blocks_run: true,
                    });
                }
            }
        }
        let mut services: BTreeMap<&str, &str> = BTreeMap::new();
        for (id, p) in &self.plugins {
            for service in &p.package.manifest.provides {
                if let Some(first) = services.insert(service, id) {
                    out.push(PluginConflict {
                        plugins: vec![first.to_string(), id.clone()],
                        kind: ConflictKind::Service,
                        message: format!("{first} and {id} both provide {service}"),
                        blocks_run: true,
                    });
                }
            }
        }
        let hooks = self.plugins.iter().flat_map(|(id, p)| {
            p.package
                .contributions
                .hooks
                .iter()
                .map(move |h| (id.as_str(), h))
        });
        if let Err(message) = crate::hooks::order_hooks(hooks) {
            let plugins = self
                .plugins
                .iter()
                .filter(|(_, p)| !p.package.contributions.hooks.is_empty())
                .map(|(id, _)| id.clone())
                .collect();
            out.push(PluginConflict {
                plugins,
                kind: ConflictKind::HookOrder,
                message,
                blocks_run: true,
            });
        }
        let mut keys: BTreeMap<String, &str> = BTreeMap::new();
        let bound = self
            .shortcuts()
            .into_iter()
            .filter_map(|(plugin, s)| s.normalized().map(|k| (plugin, k)))
            .chain(
                self.menus()
                    .into_iter()
                    .filter_map(|(plugin, m)| m.keys().map(|k| (plugin, k))),
            );
        for (plugin, key) in bound {
            if let Some(first) = keys.insert(key.clone(), plugin)
                && first != plugin
            {
                out.push(PluginConflict {
                    plugins: vec![first.to_string(), plugin.to_string()],
                    kind: ConflictKind::Shortcut,
                    message: format!("{first} and {plugin} both bind {key}"),
                    blocks_run: false,
                });
            }
        }
        out
    }

    /// Plugins that run code and could not be loaded: a run without them
    /// would silently do less than the game says, so Play refuses.
    pub fn unavailable_code_plugins(&self) -> Vec<&PluginProblem> {
        self.problems
            .iter()
            .filter(|p| self.locked.get(&p.id).is_some_and(|l| l.has_code))
            .collect()
    }

    pub fn get(&self, plugin: &str) -> Option<&LoadedPlugin> {
        self.plugins.get(plugin)
    }

    /// The verified shared library a native plugin runs from on this
    /// machine, and the capabilities it asked for.
    pub fn native_library(&self, plugin: &str) -> Result<NativeLibrary, String> {
        let loaded = self
            .plugins
            .get(plugin)
            .ok_or_else(|| format!("{plugin} is not installed"))?;
        native_library_of(plugin, &loaded.package, &self.target)
    }

    /// What runs a plugin's `module` commands here: the native library for
    /// this target, or the portable module when there is none.
    pub fn code_runtime(&self, plugin: &str) -> Result<CodeRuntime, String> {
        let loaded = self
            .plugins
            .get(plugin)
            .ok_or_else(|| format!("{plugin} is not installed"))?;
        code_runtime_of(plugin, &loaded.package, &self.target)
    }

    /// What a running world needs to host the code of every plugin that has
    /// any for this target. A plugin that has none here (declarative, or no
    /// artifact for the target) is left out: its records and commands still
    /// work, and Play's preflight already refuses a code plugin that failed.
    pub fn loadout(&self) -> Loadout {
        let mut plugins = Vec::new();
        for (id, loaded) in &self.plugins {
            let Ok(runtime) = code_runtime_of(id, &loaded.package, &self.target) else {
                continue;
            };
            plugins.push(loadout_plugin(id, &loaded.package, runtime));
        }
        let shaders = self
            .plugins
            .values()
            .flat_map(|loaded| loaded.package.shader_modules().unwrap_or_default())
            .collect();
        Loadout { plugins, shaders }
    }

    pub fn manifest(&self, plugin: &str) -> Option<&PluginManifest> {
        self.plugins.get(plugin).map(|p| &p.package.manifest)
    }

    /// The schema of a component or resource type.
    pub fn schema(&self, plugin: &str, type_id: &str) -> Option<&ComponentSchema> {
        self.plugins
            .get(plugin)?
            .package
            .contributions
            .record(type_id)
    }

    pub fn is_resource(&self, plugin: &str, type_id: &str) -> bool {
        self.plugins
            .get(plugin)
            .is_some_and(|p| p.package.contributions.resource(type_id).is_some())
    }

    /// Every contributed command as `(plugin/name, plugin, schema)`.
    pub fn commands(&self) -> Vec<(String, &str, &CommandSchema)> {
        self.plugins
            .iter()
            .flat_map(|(plugin, p)| {
                p.package
                    .contributions
                    .commands
                    .iter()
                    .map(move |c| (id::qualified(plugin, &c.name), plugin.as_str(), c))
            })
            .collect()
    }

    pub fn command(&self, qualified: &str) -> Option<(&str, &CommandSchema)> {
        let (plugin, name) = id::split_qualified(qualified)?;
        let loaded = self.plugins.get(plugin)?;
        Some((
            loaded.package.manifest.id.as_str(),
            loaded.package.contributions.command(name)?,
        ))
    }

    /// Every contributed editor panel with the plugin that owns it.
    pub fn panels(&self) -> Vec<(&str, &PanelSchema)> {
        self.plugins
            .iter()
            .flat_map(|(plugin, p)| {
                p.package
                    .contributions
                    .panels
                    .iter()
                    .map(move |panel| (plugin.as_str(), panel))
            })
            .collect()
    }

    /// Every contributed scene-view tool with the plugin that owns it.
    pub fn tools(&self) -> Vec<(&str, &ToolSchema)> {
        self.plugins
            .iter()
            .flat_map(|(plugin, p)| {
                p.package
                    .contributions
                    .tools
                    .iter()
                    .map(move |tool| (plugin.as_str(), tool))
            })
            .collect()
    }

    /// Every contributed menu item with the plugin that owns it.
    pub fn menus(&self) -> Vec<(&str, &MenuSchema)> {
        self.plugins
            .iter()
            .flat_map(|(plugin, p)| {
                p.package
                    .contributions
                    .menus
                    .iter()
                    .map(move |m| (plugin.as_str(), m))
            })
            .collect()
    }

    /// Every contributed shortcut with the plugin that owns it.
    pub fn shortcuts(&self) -> Vec<(&str, &ShortcutSchema)> {
        self.plugins
            .iter()
            .flat_map(|(plugin, p)| {
                p.package
                    .contributions
                    .shortcuts
                    .iter()
                    .map(move |s| (plugin.as_str(), s))
            })
            .collect()
    }

    /// Every contributed scene-view overlay with the plugin that owns it.
    pub fn overlays(&self) -> Vec<(&str, &OverlaySchema)> {
        self.plugins
            .iter()
            .flat_map(|(plugin, p)| {
                p.package
                    .contributions
                    .overlays
                    .iter()
                    .map(move |o| (plugin.as_str(), o))
            })
            .collect()
    }

    pub fn blocks(&self) -> Vec<(&str, &BlockSchema)> {
        self.plugins
            .iter()
            .flat_map(|(plugin, p)| {
                p.package
                    .contributions
                    .blocks
                    .iter()
                    .map(move |b| (plugin.as_str(), b))
            })
            .collect()
    }

    /// Whether a running game, with no editor, can run `plugin`'s block
    /// `type_id`: its command is a module op. A hat has no command and
    /// answers to a module's events, so it always can.
    pub fn block_runs_in_world(&self, plugin: &str, type_id: &str) -> bool {
        let Some(loaded) = self.plugins.get(plugin) else {
            return false;
        };
        let contributions = &loaded.package.contributions;
        let Some(block) = contributions.blocks.iter().find(|b| b.type_id == type_id) else {
            return false;
        };
        if block.kind == BlockKind::Hat {
            return true;
        }
        block
            .command
            .as_deref()
            .and_then(|name| contributions.command(name))
            .is_some_and(|c| matches!(c.action, CommandAction::Module { .. }))
    }

    /// Every importer, as `(plugin, schema)`.
    pub fn importers(&self) -> Vec<(&str, &ImporterSchema)> {
        self.plugins
            .iter()
            .flat_map(|(plugin, p)| {
                p.package
                    .contributions
                    .importers
                    .iter()
                    .map(move |i| (plugin.as_str(), i))
            })
            .collect()
    }

    /// The importers that take a file with this extension.
    pub fn importers_for(&self, extension: &str) -> Vec<(&str, &ImporterSchema)> {
        self.importers()
            .into_iter()
            .filter(|(_, importer)| importer.handles(extension))
            .collect()
    }

    /// Every build hook, as `(plugin, schema)`.
    pub fn build_hooks(&self) -> Vec<(&str, &BuildHookSchema)> {
        self.plugins
            .iter()
            .flat_map(|(plugin, p)| {
                p.package
                    .contributions
                    .build_hooks
                    .iter()
                    .map(move |h| (plugin.as_str(), h))
            })
            .collect()
    }

    pub fn hooks(&self) -> Vec<(&str, &HookSchema)> {
        self.plugins
            .iter()
            .flat_map(|(plugin, p)| {
                p.package
                    .contributions
                    .hooks
                    .iter()
                    .map(move |h| (plugin.as_str(), h))
            })
            .collect()
    }

    /// What the plugin system knows about one record.
    pub fn check_record(&self, record: &PluginRecord) -> RecordStatus {
        let Some(plugin) = self.plugins.get(&record.plugin) else {
            let reason = match self.problems.iter().find(|p| p.id == record.plugin) {
                Some(p) => format!("{} {}", record.plugin, p.message),
                None if self.locked.contains_key(&record.plugin) => {
                    format!("{} is locked but could not be loaded", record.plugin)
                }
                None => format!("{} is not installed in this project", record.plugin),
            };
            return RecordStatus::Missing { reason };
        };
        let Some(schema) = plugin.package.contributions.record(&record.type_id) else {
            return RecordStatus::UnknownType;
        };
        if record.schema_version > schema.version {
            return RecordStatus::SchemaTooNew {
                found: record.schema_version,
                supported: schema.version,
            };
        }
        if record.schema_version < schema.version {
            return RecordStatus::NeedsMigration {
                from: record.schema_version,
                to: schema.version,
            };
        }
        match schema.validate(&record.payload) {
            Ok(()) => RecordStatus::Ok,
            Err(errors) => RecordStatus::Invalid { errors },
        }
    }

    /// Whether nothing at run time reads a record, so a problem with it
    /// need not stop Play or a build.
    fn editor_only(&self, record: &PluginRecord) -> bool {
        if let Some(schema) = self
            .plugins
            .get(&record.plugin)
            .and_then(|p| p.package.contributions.record(&record.type_id))
        {
            return schema.editor_only;
        }
        self.locked
            .get(&record.plugin)
            .is_some_and(|l| l.editor_only.contains(&record.type_id))
    }

    /// Checks every record, answering only the ones that are not fine.
    pub fn audit<'a>(
        &self,
        records: impl IntoIterator<Item = (String, &'a PluginRecord)>,
    ) -> Vec<RecordIssue> {
        records
            .into_iter()
            .filter_map(|(location, record)| {
                let status = self.check_record(record);
                if status == RecordStatus::Ok {
                    return None;
                }
                let blocks_run = !self.editor_only(record);
                Some(RecordIssue {
                    location,
                    record: record.name().to_string(),
                    status,
                    blocks_run,
                })
            })
            .collect()
    }

    /// Everything a built game for `target` has to carry: the plugins with
    /// something at run time, and their runtime dependencies. Fails when one
    /// of them cannot run on the target, instead of shipping a game that
    /// quietly does nothing.
    pub fn ship_plan(&self, target: &str) -> Result<Vec<ShipEntry>, String> {
        let mut needed: BTreeSet<String> = BTreeSet::new();
        let mut stack: Vec<String> = self
            .plugins
            .iter()
            .filter(|(_, p)| has_runtime_content(&p.package))
            .map(|(id, _)| id.clone())
            .collect();
        while let Some(id) = stack.pop() {
            if !needed.insert(id.clone()) {
                continue;
            }
            if let Some(p) = self.plugins.get(&id) {
                stack.extend(
                    p.package
                        .manifest
                        .dependencies
                        .iter()
                        .filter(|(_, d)| d.scope == DependencyScope::Runtime && !d.optional)
                        .map(|(dep, _)| dep.clone()),
                );
            }
        }
        let mut out = Vec::new();
        for id in needed {
            let Some(p) = self.plugins.get(&id) else {
                return Err(format!(
                    "{id} is needed by the game but is not available: {}",
                    self.problems
                        .iter()
                        .find(|p| p.id == id)
                        .map_or("not installed".to_string(), |p| p.message.clone())
                ));
            };
            let manifest = &p.package.manifest;
            let support = manifest.support_for(target);
            if let blockloom_plugin_api::manifest::TargetSupport::Unsupported { reason } = &support
            {
                return Err(format!("{id} cannot be built for {target}: {reason}"));
            }
            out.push(ShipEntry {
                id: id.clone(),
                version: manifest.version.clone(),
                hash: p.package.content_hash.clone(),
                tier: manifest.tier,
                files: shipped_files(manifest, target, &support),
                support,
            });
        }
        Ok(out)
    }
}

/// The verified shared library `package` runs from on `target`.
pub(crate) fn native_library_of(
    plugin: &str,
    package: &Package,
    target: &str,
) -> Result<NativeLibrary, String> {
    let manifest = &package.manifest;
    let entry = manifest.runtime.native.get(target).ok_or_else(|| {
        format!("{plugin} has no native library for {target} (only declarative parts work here)")
    })?;
    Ok(NativeLibrary {
        path: package.root.join(&entry.library),
        hash: package.content_hash.clone(),
        capabilities: manifest.capabilities.clone(),
    })
}

/// The native library for `target`, or the portable module when there is none.
pub(crate) fn code_runtime_of(
    plugin: &str,
    package: &Package,
    target: &str,
) -> Result<CodeRuntime, String> {
    let manifest = &package.manifest;
    let native = loads_native_libraries(target) && manifest.runtime.native.contains_key(target);
    if !native && let Some(entry) = &manifest.runtime.portable {
        return Ok(CodeRuntime::Portable(PortableLibrary {
            path: package.root.join(&entry.module),
            hash: package.content_hash.clone(),
            capabilities: manifest.capabilities.clone(),
            entry: entry.clone(),
        }));
    }
    native_library_of(plugin, package, target).map(CodeRuntime::Native)
}

/// One plugin as a world loads it: its code, hooks and module-op blocks.
pub(crate) fn loadout_plugin(id: &str, package: &Package, runtime: CodeRuntime) -> LoadoutPlugin {
    let contributions = &package.contributions;
    let blocks = contributions
        .blocks
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::Statement | BlockKind::Reporter))
        .filter_map(|b| {
            let command = contributions.command(b.command.as_deref()?)?;
            let CommandAction::Module { op } = &command.action else {
                return None;
            };
            Some(LoadoutBlock {
                type_id: b.type_id.clone(),
                op: op.clone(),
                slots: b.slots.clone(),
                wants_actor: command.args.iter().any(|a| a.name == "actor"),
                returns: b.returns.clone(),
            })
        })
        .collect();
    LoadoutPlugin {
        id: id.to_string(),
        runtime,
        hooks: contributions.hooks.clone(),
        blocks,
        preview: package.manifest.editor.preview,
        nodes: contributions.nodes.clone(),
    }
}

fn has_runtime_content(package: &Package) -> bool {
    let c = &package.contributions;
    // An importer and a build hook run in the editor, so a package that is
    // only those stays out of the game.
    let editor_tool = !(c.importers.is_empty() && c.build_hooks.is_empty());
    (package.manifest.tier != Tier::Declarative && !editor_tool)
        || !c.blocks.is_empty()
        || !c.commands.is_empty()
        || !c.hooks.is_empty()
        || !c.nodes.is_empty()
        || !c.shaders.is_empty()
        || c.components
            .iter()
            .chain(&c.resources)
            .any(|s| !s.editor_only)
}

/// The files a player needs: everything but editor modules, docs and
/// examples, and the other targets' native libraries.
fn shipped_files(
    manifest: &PluginManifest,
    target: &str,
    support: &blockloom_plugin_api::manifest::TargetSupport,
) -> Vec<String> {
    let other_native: BTreeSet<&str> = manifest
        .runtime
        .native
        .iter()
        .filter(|(triple, _)| triple.as_str() != target)
        .map(|(_, e)| e.library.as_str())
        .collect();
    let own_native = manifest
        .runtime
        .native
        .get(target)
        .map(|e| e.library.as_str());
    manifest
        .files
        .keys()
        .filter(|path| {
            let path = path.as_str();
            if manifest.editor.modules.iter().any(|m| m == path)
                || manifest.editor.inspectors.iter().any(|i| i.module == path)
                || path.starts_with("editor/")
                || path.starts_with("docs/")
                || path.starts_with("examples/")
            {
                return false;
            }
            if other_native.contains(path) && Some(path) != own_native {
                return false;
            }
            // A target served by the portable module has no use for the
            // native libraries it did not match either.
            if matches!(
                support,
                blockloom_plugin_api::manifest::TargetSupport::Portable
            ) && manifest.runtime.native.values().any(|e| e.library == path)
            {
                return false;
            }
            true
        })
        .cloned()
        .collect()
}

/// Upgrades `records` of one plugin to `schema`'s version on copies. Nothing
/// is applied here: the caller swaps the whole set in only if every record
/// came out valid.
pub fn migrate_records(
    active: &ActivePlugins,
    records: &[PluginRecord],
) -> Result<Vec<PluginRecord>, Vec<String>> {
    let mut out = Vec::with_capacity(records.len());
    let mut errors = Vec::new();
    for record in records {
        let Some(schema) = active.schema(&record.plugin, &record.type_id) else {
            errors.push(format!("{}: no schema to migrate to", record.name()));
            continue;
        };
        match schema.migrate(record.schema_version, &record.payload) {
            Ok(payload) => {
                let payload = schema.normalize(&payload);
                match schema.validate(&payload) {
                    Ok(()) => out.push(PluginRecord::new(
                        record.plugin.clone(),
                        record.type_id.clone(),
                        schema.version,
                        payload,
                    )),
                    Err(problems) => errors.push(format!(
                        "{}: {}",
                        record.name(),
                        problems
                            .iter()
                            .map(|e| e.to_string())
                            .collect::<Vec<_>>()
                            .join("; ")
                    )),
                }
            }
            Err(e) => errors.push(format!("{}: {e}", record.name())),
        }
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::{Change, Environment, install};
    use crate::package::fixtures::{declarative, portable};
    use crate::source::Source;
    use serde_json::json;
    use std::fs;

    struct Rig {
        dir: tempfile::TempDir,
        env: Environment,
        project: ProjectPlugins,
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let project = ProjectPlugins::new(dir.path().join("project"));
        fs::create_dir_all(&project.dir).unwrap();
        let env = Environment {
            cache: Cache::new(dir.path().join("cache")),
            engine: Version::new(0, 0, 1),
            target: "x86_64-unknown-linux-gnu".to_string(),
            offline: false,
            registries: BTreeMap::new(),
        };
        Rig { dir, env, project }
    }

    impl Rig {
        fn add(&self, id: &str, version: &str) -> std::path::PathBuf {
            let pkg = declarative(&self.dir.path().join("src"), id, version, json!({}));
            install(
                &self.env,
                &self.project,
                Change::Add {
                    id: id.into(),
                    req: "*".parse().unwrap(),
                    source: Some(Source::Path(pkg.clone())),
                    features: vec![],
                },
            )
            .unwrap();
            pkg
        }

        fn load(&self) -> ActivePlugins {
            ActivePlugins::load(
                &self.project,
                &self.env.cache,
                &self.env.target,
                &self.env.engine,
            )
        }
    }

    fn record(
        plugin: &str,
        type_id: &str,
        version: u32,
        payload: serde_json::Value,
    ) -> PluginRecord {
        PluginRecord::new(plugin, type_id, version, payload)
    }

    #[test]
    fn locked_plugins_load_and_expose_their_contributions() {
        let rig = rig();
        rig.add("com.x.a", "1.0.0");
        let active = rig.load();
        assert!(active.problems.is_empty(), "{:?}", active.problems);
        assert!(active.schema("com.x.a", "Health").is_some());
        assert_eq!(active.commands()[0].0, "com.x.a/set_hp");
        assert!(active.command("com.x.a/set_hp").is_some());
        assert!(active.command("com.x.a/none").is_none());
    }

    #[test]
    fn records_are_checked_against_their_schema_version() {
        let rig = rig();
        rig.add("com.x.a", "1.0.0");
        let active = rig.load();
        let ok = record("com.x.a", "Health", 1, json!({"hp": 5}));
        assert_eq!(active.check_record(&ok), RecordStatus::Ok);
        assert!(matches!(
            active.check_record(&record("com.x.a", "Health", 2, json!({"hp": 5}))),
            RecordStatus::SchemaTooNew {
                found: 2,
                supported: 1
            }
        ));
        assert!(matches!(
            active.check_record(&record("com.x.a", "Health", 1, json!({"hp": 500}))),
            RecordStatus::Invalid { .. }
        ));
        assert_eq!(
            active.check_record(&record("com.x.a", "Nope", 1, json!({}))),
            RecordStatus::UnknownType
        );
        assert!(matches!(
            active.check_record(&record("com.x.gone", "Health", 1, json!({}))),
            RecordStatus::Missing { .. }
        ));
    }

    #[test]
    fn a_missing_cache_entry_is_a_problem_not_a_crash() {
        let rig = rig();
        rig.add("com.x.a", "1.0.0");
        let cold = Cache::new(rig.dir.path().join("cold"));
        let active = ActivePlugins::load(&rig.project, &cold, &rig.env.target, &rig.env.engine);
        assert!(active.plugins.is_empty());
        assert!(active.problems[0].message.contains("not installed"));
        let issues = active.audit([(
            "actor Player".to_string(),
            &record("com.x.a", "Health", 1, json!({})),
        )]);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].blocks_run, "unknown whether anything reads it");
        assert!(
            matches!(&issues[0].status, RecordStatus::Missing { reason } if reason.contains("not installed"))
        );
    }

    #[test]
    fn editor_only_records_do_not_block_a_run_even_when_the_plugin_is_gone() {
        let rig = rig();
        let pkg = declarative(&rig.dir.path().join("src"), "com.x.e", "1.0.0", json!({}));
        let schema = pkg.join("schemas/main.json");
        let mut value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&schema).unwrap()).unwrap();
        value["components"][0]["editor_only"] = json!(true);
        fs::write(&schema, serde_json::to_string(&value).unwrap()).unwrap();
        crate::package::seal(&pkg).unwrap();
        install(
            &rig.env,
            &rig.project,
            Change::Add {
                id: "com.x.e".into(),
                req: "*".parse().unwrap(),
                source: Some(Source::Path(pkg)),
                features: vec![],
            },
        )
        .unwrap();
        let cold = Cache::new(rig.dir.path().join("cold"));
        let active = ActivePlugins::load(&rig.project, &cold, &rig.env.target, &rig.env.engine);
        let issues = active.audit([(
            "actor A".to_string(),
            &record("com.x.e", "Health", 1, json!({})),
        )]);
        assert_eq!(issues.len(), 1);
        assert!(!issues[0].blocks_run);
    }

    #[test]
    fn a_broken_dependency_disables_its_dependents_only() {
        let rig = rig();
        let base = declarative(
            &rig.dir.path().join("src"),
            "com.x.base",
            "1.0.0",
            json!({}),
        );
        let top = declarative(
            &rig.dir.path().join("src"),
            "com.x.top",
            "1.0.0",
            json!({"com.x.base": "*"}),
        );
        rig.add("com.x.other", "1.0.0");
        for (id, pkg) in [("com.x.base", &base), ("com.x.top", &top)] {
            install(
                &rig.env,
                &rig.project,
                Change::Add {
                    id: id.into(),
                    req: "*".parse().unwrap(),
                    source: Some(Source::Path(pkg.clone())),
                    features: vec![],
                },
            )
            .unwrap();
        }
        // Lose base from the cache.
        let hash = rig
            .project
            .read_lock()
            .unwrap()
            .get("com.x.base")
            .unwrap()
            .hash
            .clone();
        let dir = rig
            .env
            .cache
            .get("com.x.base", &Version::new(1, 0, 0), &hash)
            .unwrap();
        fs::remove_dir_all(dir).unwrap();
        let active = rig.load();
        assert!(active.get("com.x.other").is_some());
        assert!(active.get("com.x.base").is_none());
        assert!(active.get("com.x.top").is_none());
        assert!(
            active
                .problems
                .iter()
                .any(|p| p.id == "com.x.top" && p.message.contains("requires com.x.base"))
        );
    }

    #[test]
    fn migrations_run_on_copies_and_are_all_or_nothing() {
        let rig = rig();
        let pkg = declarative(&rig.dir.path().join("src"), "com.x.m", "2.0.0", json!({}));
        let schema = pkg.join("schemas/main.json");
        let mut value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&schema).unwrap()).unwrap();
        value["components"][0]["version"] = json!(2);
        value["components"][0]["migrations"] =
            json!([{"from": 1, "ops": [{"op": "rename_field", "from": "health", "to": "hp"}]}]);
        fs::write(&schema, serde_json::to_string(&value).unwrap()).unwrap();
        crate::package::seal(&pkg).unwrap();
        install(
            &rig.env,
            &rig.project,
            Change::Add {
                id: "com.x.m".into(),
                req: "*".parse().unwrap(),
                source: Some(Source::Path(pkg)),
                features: vec![],
            },
        )
        .unwrap();
        let active = rig.load();
        let old = record("com.x.m", "Health", 1, json!({"health": 7}));
        assert!(matches!(
            active.check_record(&old),
            RecordStatus::NeedsMigration { from: 1, to: 2 }
        ));
        let migrated = migrate_records(&active, std::slice::from_ref(&old)).unwrap();
        assert_eq!(migrated[0].schema_version, 2);
        assert_eq!(migrated[0].payload, json!({"hp": 7}));
        assert_eq!(active.check_record(&migrated[0]), RecordStatus::Ok);
        // One bad record fails the whole batch.
        let bad = record("com.x.m", "Health", 1, json!({"health": 700}));
        let errors = migrate_records(&active, &[old, bad]).unwrap_err();
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn the_ship_plan_leaves_out_editor_only_plugins_and_names_unsupported_targets() {
        let rig = rig();
        rig.add("com.x.a", "1.0.0");
        let active = rig.load();
        let plan = active.ship_plan("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(plan.len(), 1);
        assert!(plan[0].files.contains(&"schemas/main.json".to_string()));
        assert_eq!(plan[0].tier, Tier::Declarative);
    }

    #[test]
    fn a_loadout_names_each_plugins_code_hooks_and_module_blocks() {
        let rig = rig();
        rig.add("com.example.plain", "1.0.0");
        let pkg = portable(&rig.dir.path().join("code"), "com.example.code", "1.0.0");
        install(
            &rig.env,
            &rig.project,
            Change::Add {
                id: "com.example.code".into(),
                req: "*".parse().unwrap(),
                source: Some(Source::Path(pkg)),
                features: vec![],
            },
        )
        .unwrap();
        let loadout = rig.load().loadout();
        // The declarative plugin has no code to host.
        assert_eq!(loadout.plugins.len(), 1);
        let code = &loadout.plugins[0];
        assert_eq!(code.id, "com.example.code");
        assert!(matches!(code.runtime, CodeRuntime::Portable(_)));
        assert_eq!(code.hooks.len(), 1);
        // Only blocks whose command is a module op run in the world.
        let mut blocks: Vec<_> = code
            .blocks
            .iter()
            .map(|b| (&b.type_id[..], &b.op[..], b.wants_actor))
            .collect();
        blocks.sort();
        assert_eq!(
            blocks,
            [("echo_block", "echo", true), ("spin_block", "spin", false)]
        );
        let back: Loadout =
            serde_json::from_value(serde_json::to_value(&loadout).unwrap()).unwrap();
        assert_eq!(back, loadout);
    }

    /// Installs a declarative package after adding `extra` keys to its manifest.
    fn add_with(rig: &Rig, id: &str, extra: serde_json::Value) {
        let pkg = declarative(&rig.dir.path().join("src"), id, "1.0.0", json!({}));
        let path = pkg.join("plugin.json");
        let mut manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        for (key, value) in extra.as_object().unwrap() {
            manifest[key] = value.clone();
        }
        fs::write(&path, serde_json::to_string(&manifest).unwrap()).unwrap();
        crate::package::seal(&pkg).unwrap();
        install(
            &rig.env,
            &rig.project,
            Change::Add {
                id: id.into(),
                req: "*".parse().unwrap(),
                source: Some(Source::Path(pkg)),
                features: vec![],
            },
        )
        .unwrap();
    }

    #[test]
    fn a_declared_conflict_blocks_a_run() {
        let rig = rig();
        add_with(
            &rig,
            "com.example.a",
            json!({"conflicts": ["com.example.b"]}),
        );
        add_with(&rig, "com.example.b", json!({}));
        let conflicts = rig.load().conflicts();
        assert_eq!(conflicts.len(), 1, "{conflicts:?}");
        assert_eq!(conflicts[0].kind, ConflictKind::Declared);
        assert!(conflicts[0].blocks_run);
    }

    #[test]
    fn plugins_that_do_not_clash_have_no_conflicts() {
        let rig = rig();
        rig.add("com.example.a", "1.0.0");
        rig.add("com.example.b", "1.0.0");
        assert!(rig.load().conflicts().is_empty());
    }
}
