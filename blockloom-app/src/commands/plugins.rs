//! Plugin commands: installing and removing packages, editing the records
//! they own, and checking a project before it runs or ships.
//!
//! Package changes touch only `plugins.json`, `plugins.lock` and the shared
//! cache (see `blockloom-plugin-host`); a plugin's data lives in the document
//! as opaque records the owning plugin's schema validates. Nothing here
//! needs the plugin's code: declarative contributions are applied by the
//! host itself.

use super::{auto_save, emit, lock, push_undo, push_undo_for, sync_runtime};
use crate::AppHandle;
use crate::state::{AppState, EditSession, LogLine, SharedState};
use blockloom_core::build::{self, ExtraFile, PluginPayload, Target};
use blockloom_core::components::ActorComponent;
use blockloom_core::library;
use blockloom_core::pack::{PLUGINS_DIR, PackedPlugin};
use blockloom_core::project::{PluginBlockShape, Project};
use blockloom_plugin_api::abi::LOG_WARN;
use blockloom_plugin_api::id::{self, validate_plugin_id};
use blockloom_plugin_api::manifest::TargetSupport;
use blockloom_plugin_api::record::PluginRecord;
use blockloom_plugin_api::schema::{
    BlockKind, CommandAction, ComponentSchema, FieldType, PanelItem, fill_template,
};
use blockloom_plugin_api::{Version, VersionReq};
use blockloom_plugin_host::active::{ActivePlugins, RecordIssue, RecordStatus, migrate_records};
use blockloom_plugin_host::cache::{self, Cache};
use blockloom_plugin_host::imports::{self, Imported};
use blockloom_plugin_host::install::{self, Change, Environment, PlanChange};
use blockloom_plugin_host::lock::ProjectPlugins;
use blockloom_plugin_host::module::CodeModule;
use blockloom_plugin_host::package;
use blockloom_plugin_host::registry::DirRegistry;
use blockloom_plugin_host::source::Source;
use blockloom_plugin_host::trust;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The engine version plugins are checked against.
fn engine_version() -> Version {
    env!("CARGO_PKG_VERSION")
        .parse()
        .unwrap_or_else(|_| Version::new(0, 0, 0))
}

/// The target triple this editor runs and plays on.
fn host_triple() -> String {
    build::host().map_or_else(|| "unknown".to_string(), |t| t.triple.to_string())
}

/// How installs reach the cache and registries on this machine.
/// `BLOCKLOOM_PLUGINS_OFFLINE=1` forces cache-only installs.
pub(crate) fn environment(offline: bool) -> Environment {
    let offline = offline
        || std::env::var("BLOCKLOOM_PLUGINS_OFFLINE").is_ok_and(|v| !v.is_empty() && v != "0");
    Environment {
        cache: Cache::new(cache::default_root()),
        engine: engine_version(),
        target: host_triple(),
        offline,
        registries: Default::default(),
    }
}

/// Loads what the project folder's lock names. Never fails: a package that
/// can't load is reported by [`ActivePlugins::problems`].
pub(crate) fn load_active(dir: &Path) -> ActivePlugins {
    let env = environment(true);
    ActivePlugins::load(
        &ProjectPlugins::new(dir),
        &env.cache,
        &env.target,
        &env.engine,
    )
}

/// The open project's folder, refusing a copy that only shares another
/// editor's files: package changes would race the owner's.
fn owned_dir(s: &AppState) -> Result<PathBuf, String> {
    let Some(open) = &s.open else {
        return Err("No project is open".to_string());
    };
    if open.attached || !open.owns_lock {
        return Err(
            "This copy shares another editor's project folder; change plugins there".to_string(),
        );
    }
    Ok(open.dir.clone())
}

fn open_dir(s: &AppState) -> Result<PathBuf, String> {
    s.project_dir()
        .map(Path::to_path_buf)
        .ok_or_else(|| "No project is open".to_string())
}

fn active(s: &AppState) -> Result<&ActivePlugins, String> {
    s.open
        .as_ref()
        .map(|open| &open.plugins)
        .ok_or_else(|| "No project is open".to_string())
}

/// The plugin code a run hosts in the game world: what the open project's
/// active plugins have for this machine.
pub(crate) fn loadout(s: &AppState) -> blockloom_plugin_api::loadout::Loadout {
    s.open
        .as_ref()
        .map(|open| open.plugins.loadout())
        .unwrap_or_default()
}

/// Reloads what the lock names. Another editor attached to this folder only
/// follows a revision bump, so `bump` is set after a package change this copy
/// made, which also keeps it from reloading its own change.
pub(crate) fn reload(s: &mut AppState, bump: bool) {
    if let Some(open) = s.open.as_mut() {
        open.plugins = load_active(&open.dir);
        open.modules.retain(&open.plugins);
        if bump {
            let revision = blockloom_core::sync::bump_revision(&open.dir);
            open.revision
                .store(revision, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

/// The modules the open project has loaded, by plugin id. A module is loaded
/// on its first call and unloaded when its package changes or goes.
#[derive(Default)]
pub(crate) struct Modules {
    loaded: std::collections::BTreeMap<String, (String, Arc<Mutex<CodeModule>>)>,
    /// What editor-side calls into modules cost and report.
    pub(crate) diagnostics: Arc<blockloom_plugin_host::diagnostics::Diagnostics>,
}

/// The services a module the editor opens gets: the project's data folder, the
/// project's plugin saves and the editor's diagnostics.
fn editor_services(
    dir: &Path,
    project_id: &str,
    diagnostics: &Arc<blockloom_plugin_host::diagnostics::Diagnostics>,
) -> blockloom_plugin_host::services::HostServices {
    use blockloom_plugin_host::storage::DiskStore;
    blockloom_plugin_host::services::HostServices::new(&engine_version().to_string())
        .with_diagnostics(diagnostics.clone())
        .with_project_store(Arc::new(DiskStore::new(
            dir.join(blockloom_plugin_api::data::DATA_DIR),
            false,
        )))
        .with_save_store(Arc::new(DiskStore::new(
            blockloom_core::save::path(project_id).with_extension("plugins"),
            false,
        )))
}

impl Modules {
    /// Drops every module whose package is gone or changed.
    pub(crate) fn retain(&mut self, active: &ActivePlugins) {
        self.loaded.retain(|id, (hash, _)| {
            active
                .code_runtime(id)
                .is_ok_and(|runtime| runtime.hash() == hash)
        });
    }

    fn get(
        &mut self,
        active: &ActivePlugins,
        id: &str,
        dir: &Path,
        project_id: &str,
    ) -> Result<Arc<Mutex<CodeModule>>, String> {
        let runtime = active.code_runtime(id)?;
        if let Some((hash, module)) = self.loaded.get(id)
            && hash == runtime.hash()
            && module.lock().is_ok_and(|m| !m.is_stopped())
        {
            return Ok(module.clone());
        }
        let hash = runtime.hash().to_string();
        let host = editor_services(dir, project_id, &self.diagnostics);
        let module =
            CodeModule::load_with(&runtime, id, &host).map_err(|e| format!("{id}: {e}"))?;
        let module = Arc::new(Mutex::new(module));
        self.loaded.insert(id.to_string(), (hash, module.clone()));
        Ok(module)
    }
}

// ─── Reading ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct InstalledDto {
    id: String,
    name: String,
    version: String,
    description: String,
    license: String,
    tier: String,
    source: String,
    dev: bool,
    /// How it runs on this machine.
    support: TargetSupport,
    capabilities: Vec<String>,
    dependencies: Vec<String>,
    components: Vec<String>,
    blocks: usize,
    commands: usize,
}

fn name_of<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn installed(active: &ActivePlugins) -> Vec<InstalledDto> {
    active
        .plugins
        .values()
        .map(|p| {
            let m = &p.package.manifest;
            let c = &p.package.contributions;
            InstalledDto {
                id: m.id.clone(),
                name: m.name.clone(),
                version: m.version.to_string(),
                description: m.description.clone(),
                license: m.license.clone(),
                tier: name_of(&m.tier),
                source: p.locked.source.to_string(),
                dev: p.locked.dev,
                support: m.support_for(active.target()),
                capabilities: m.capabilities.iter().map(name_of).collect(),
                dependencies: p.locked.dependencies.clone(),
                components: c
                    .components
                    .iter()
                    .chain(&c.resources)
                    .map(|s| id::qualified(&m.id, &s.type_id))
                    .collect(),
                blocks: c.blocks.len(),
                commands: c.commands.len(),
            }
        })
        .collect()
}

/// Every component and resource type the installed plugins declare, with the
/// fields and defaults the inspector builds its forms from.
fn types_json(active: &ActivePlugins) -> Vec<Value> {
    let mut out = Vec::new();
    for (plugin, p) in &active.plugins {
        let c = &p.package.contributions;
        for (kind, list) in [("component", &c.components), ("resource", &c.resources)] {
            for schema in list {
                out.push(json!({
                    "name": id::qualified(plugin, &schema.type_id),
                    "plugin": plugin,
                    "pluginName": p.package.manifest.name,
                    "type": schema.type_id,
                    "displayName": if schema.display_name.is_empty() {
                        &schema.type_id
                    } else {
                        &schema.display_name
                    },
                    "kind": kind,
                    "version": schema.version,
                    "editorOnly": schema.editor_only,
                    "fields": schema.fields,
                    "inspector": schema.inspector,
                    "defaults": schema.defaults(),
                }));
            }
        }
    }
    out
}

/// Every block the installed plugins add, with the plugin that owns it.
fn blocks_json(active: &ActivePlugins) -> Vec<Value> {
    active
        .blocks()
        .into_iter()
        .map(|(plugin, block)| json!({ "plugin": plugin, "block": block }))
        .collect()
}

/// The plugins that ship editor modules, each with whether the user has
/// trusted this exact package. Only a trusted plugin's modules get a file to
/// load, and the QML loads nothing else.
fn editor_modules_json(active: &ActivePlugins) -> Vec<Value> {
    let ledger = trust::TrustLedger::read(&trust::default_path());
    active
        .plugins
        .values()
        .filter(|p| {
            let editor = &p.package.manifest.editor;
            !editor.modules.is_empty() || !editor.inspectors.is_empty()
        })
        .map(|p| {
            let m = &p.package.manifest;
            let trusted = ledger.is_trusted(&m.id, &p.package.content_hash);
            let modules: Vec<Value> = m
                .editor
                .modules
                .iter()
                .map(|path| {
                    let stem = Path::new(path)
                        .file_stem()
                        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
                    json!({
                        "path": path,
                        "title": module_title(&stem),
                        "file": trusted.then(|| p.package.root.join(path).to_string_lossy().into_owned()),
                    })
                })
                .collect();
            // Sections the inspector draws itself for a component type.
            let inspectors: Vec<Value> = m
                .editor
                .inspectors
                .iter()
                .map(|section| {
                    json!({
                        "component": section.component,
                        "path": section.module,
                        "file": trusted.then(|| p.package.root.join(&section.module).to_string_lossy().into_owned()),
                    })
                })
                .collect();
            json!({
                "plugin": m.id,
                "pluginName": m.name,
                "hash": p.package.content_hash,
                "inspectors": inspectors,
                // Trusted once, but the package has changed since.
                "changed": !trusted && ledger.granted(&m.id).is_some(),
                "trusted": trusted,
                "modules": modules,
            })
        })
        .collect()
}

/// `PluginPanel` -> `Plugin panel`: a file's name as a title.
fn module_title(stem: &str) -> String {
    let mut out = String::new();
    for (i, c) in stem.chars().enumerate() {
        if c == '_' || c == '-' {
            out.push(' ');
        } else if c.is_uppercase() && i > 0 {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Trusts an installed plugin's editor modules at the package's current
/// content. Only the editor window may ask: the shell and MCP have no such
/// command, and the attach socket refuses it.
pub(crate) fn plugin_trust(
    state: &SharedState,
    app: &AppHandle,
    id: &str,
) -> Result<Value, String> {
    let s = lock(state)?;
    let hash = {
        let active = active(&s)?;
        let loaded = active
            .get(id)
            .ok_or_else(|| format!("{id} is not installed in this project"))?;
        if loaded.package.manifest.editor.modules.is_empty() {
            return Err(format!("{id} ships no editor modules"));
        }
        loaded.package.content_hash.clone()
    };
    let path = trust::default_path();
    let mut ledger = trust::TrustLedger::read(&path);
    ledger.grant(id, &hash);
    ledger.write(&path)?;
    emit(app, &s);
    Ok(json!({ "id": id, "hash": hash }))
}

/// Takes the trust back; the plugin's modules stop loading.
pub(crate) fn plugin_untrust(
    state: &SharedState,
    app: &AppHandle,
    id: &str,
) -> Result<Value, String> {
    let s = lock(state)?;
    let path = trust::default_path();
    let mut ledger = trust::TrustLedger::read(&path);
    let removed = ledger.revoke(id);
    if removed {
        ledger.write(&path)?;
    }
    emit(app, &s);
    Ok(json!({ "id": id, "removed": removed }))
}

/// Every scene-view tool the installed plugins add, with its owner's name.
fn tools_json(active: &ActivePlugins) -> Vec<Value> {
    active
        .tools()
        .into_iter()
        .map(|(plugin, tool)| {
            let name = active
                .get(plugin)
                .map_or_else(String::new, |p| p.package.manifest.name.clone());
            json!({ "plugin": plugin, "pluginName": name, "tool": tool })
        })
        .collect()
}

/// Every editor panel the installed plugins add, with its owner's name.
fn panels_json(active: &ActivePlugins) -> Vec<Value> {
    active
        .panels()
        .into_iter()
        .map(|(plugin, panel)| {
            let name = active
                .get(plugin)
                .map_or_else(String::new, |p| p.package.manifest.name.clone());
            // The commands its buttons run, so the panel needs no second lookup.
            let mut commands = serde_json::Map::new();
            if let Some(loaded) = active.get(plugin) {
                for item in &panel.items {
                    if let PanelItem::Command { command, .. } = item
                        && let Some(c) = loaded.package.contributions.command(command)
                    {
                        commands.insert(
                            command.clone(),
                            json!({ "summary": c.summary, "args": c.args }),
                        );
                    }
                }
            }
            json!({ "plugin": plugin, "pluginName": name, "panel": panel, "commands": commands })
        })
        .collect()
}

/// The part of the snapshot the frontend reads: what is installed, what
/// failed to load, and which records need attention.
pub(crate) fn summary(s: &AppState) -> Value {
    let (Some(open), Some(project)) = (&s.open, s.project()) else {
        return Value::Null;
    };
    let active = &open.plugins;
    if active.is_empty() && project.plugin_records().is_empty() {
        return Value::Null;
    }
    json!({
        "installed": installed(active),
        "blocks": blocks_json(active),
        "types": types_json(active),
        "panels": panels_json(active),
        "tools": tools_json(active),
        "editorModules": editor_modules_json(active),
        "problems": active.problems,
        "issues": active.audit(project.plugin_records()),
    })
}

/// `plugin-list`: everything about the project's plugins, with the
/// dependency tree.
pub(crate) fn plugin_list(state: &SharedState) -> Result<Value, String> {
    let s = lock(state)?;
    let dir = open_dir(&s)?;
    let project = ProjectPlugins::new(&dir);
    let plugins = project.read_plugins()?;
    let lock_file = project.read_lock()?;
    let active = active(&s)?;
    Ok(json!({
        "installed": installed(active),
        "blocks": blocks_json(active),
        "types": types_json(active),
        "panels": panels_json(active),
        "tools": tools_json(active),
        "editorModules": editor_modules_json(active),
        "problems": active.problems,
        "direct": plugins.plugins,
        "registries": plugins.registries,
        "tree": install::tree(&plugins, &lock_file, None),
        "history": project.history_len(),
        "offline": environment(false).offline,
    }))
}

/// `plugin-check`: every record that needs attention, and whether the
/// project may run.
pub(crate) fn plugin_check(state: &SharedState) -> Result<Value, String> {
    let s = lock(state)?;
    let project = s.project().ok_or("No project is open")?;
    let active = active(&s)?;
    let issues = active.audit(project.plugin_records());
    let blocking = issues.iter().filter(|i| i.blocks_run).count();
    Ok(json!({
        "ok": issues.is_empty() && active.problems.is_empty(),
        "canRun": preflight(active, project).is_ok(),
        "blocking": blocking,
        "issues": issues,
        "problems": active.problems,
    }))
}

/// `plugin-commands`: the registry of commands plugins contribute, shaped
/// like the shell's own command specs so a client can list them the same way.
pub(crate) fn plugin_commands(state: &SharedState) -> Result<Value, String> {
    let s = lock(state)?;
    let active = active(&s)?;
    let commands: Vec<Value> = active
        .commands()
        .into_iter()
        .map(|(name, plugin, c)| {
            json!({
                "name": name,
                "plugin": plugin,
                "summary": c.summary,
                "args": c.args,
            })
        })
        .collect();
    Ok(json!({ "commands": commands }))
}

// ─── Run and build preflight ────────────────────────────────────────────────

fn describe(issue: &RecordIssue) -> String {
    let why = match &issue.status {
        RecordStatus::Ok => "ok".to_string(),
        RecordStatus::NeedsMigration { from, to } => {
            format!("written at schema {from}, plugin is at {to}; run plugin-migrate")
        }
        RecordStatus::Missing { reason } => format!("{reason}; run plugin-install or plugin-sync"),
        RecordStatus::UnknownType => "the plugin has no such type".to_string(),
        RecordStatus::SchemaTooNew { found, supported } => format!(
            "written at schema {found}, newer than the installed plugin's {supported}; update the plugin"
        ),
        RecordStatus::Invalid { errors } => errors
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("; "),
    };
    format!("{} ({}): {why}", issue.record, issue.location)
}

fn shape_name(shape: PluginBlockShape) -> &'static str {
    match shape {
        PluginBlockShape::Statement => "block",
        PluginBlockShape::Reporter => "reporter",
        PluginBlockShape::Hat => "hat",
    }
}

/// Refuses to run or ship a project whose plugin data has nothing behind
/// it: missing runtime behavior is a report, never a silent no-op.
pub(crate) fn preflight(active: &ActivePlugins, project: &Project) -> Result<(), String> {
    let mut lines: Vec<String> = active
        .audit(project.plugin_records())
        .iter()
        .filter(|i| i.blocks_run)
        .map(describe)
        .collect();
    for used in project.plugin_blocks() {
        let found = active
            .blocks()
            .into_iter()
            .find(|(plugin, schema)| *plugin == used.plugin && schema.type_id == used.block);
        let want = match used.shape {
            PluginBlockShape::Statement => BlockKind::Statement,
            PluginBlockShape::Reporter => BlockKind::Reporter,
            PluginBlockShape::Hat => BlockKind::Hat,
        };
        match found {
            None => lines.push(format!(
                "{}: the block {}/{} isn't provided by an installed plugin",
                used.place, used.plugin, used.block
            )),
            Some((_, schema)) if schema.kind != want => lines.push(format!(
                "{}: the block {}/{} is no longer a {}",
                used.place,
                used.plugin,
                used.block,
                shape_name(used.shape)
            )),
            Some((_, schema)) if schema.slots.len() != used.slots => lines.push(format!(
                "{}: the block {}/{} has {} slots, the installed plugin's has {}",
                used.place,
                used.plugin,
                used.block,
                used.slots,
                schema.slots.len()
            )),
            // A reporter is answered inside the world, so only a module op can.
            Some(_)
                if used.shape == PluginBlockShape::Reporter
                    && !active.block_runs_in_world(&used.plugin, &used.block) =>
            {
                lines.push(format!(
                    "{}: the reporter {}/{} needs a plugin with code, and its command isn't a module op",
                    used.place, used.plugin, used.block
                ))
            }
            Some(_) => {}
        }
    }
    lines.extend(
        active
            .unavailable_code_plugins()
            .iter()
            .map(|p| format!("{} {}", p.id, p.message)),
    );
    if lines.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "The project's plugins aren't ready, so it won't run:\n- {}",
            lines.join("\n- ")
        ))
    }
}

/// Checks the open project before Play.
pub(crate) fn preflight_run(s: &AppState) -> Result<(), String> {
    match (s.open.as_ref(), s.project()) {
        (Some(open), Some(project)) => preflight(&open.plugins, project),
        _ => Ok(()),
    }
}

/// The plugins a build for `target` carries, freshly loaded from the lock so
/// the build sees what is on disk, not what the editor last cached. Fails
/// with the dependency report when a plugin the game needs can't be shipped.
pub(crate) fn payloads(
    dir: &Path,
    project: &Project,
    target: &Target,
) -> Result<Vec<PluginPayload>, String> {
    let active = load_active(dir);
    if active.is_empty()
        && project.plugin_records().is_empty()
        && project.plugin_blocks().is_empty()
    {
        return Ok(Vec::new());
    }
    preflight(&active, project).map_err(|e| e.replace("won't run", "won't build"))?;
    // A built game has no editor to run a block's command in, so a statement
    // has to be a module op the world runs itself.
    let editor_only: Vec<String> = project
        .plugin_blocks()
        .iter()
        .filter(|u| u.shape == PluginBlockShape::Statement)
        .filter(|u| !active.block_runs_in_world(&u.plugin, &u.block))
        .map(|u| format!("{}: {}/{}", u.place, u.plugin, u.block))
        .collect();
    if !editor_only.is_empty() {
        return Err(format!(
            "These plugin blocks run their command in the editor, so the project won't build:\n- {}",
            editor_only.join("\n- ")
        ));
    }
    let plan = active.ship_plan(target.triple)?;
    Ok(plan
        .into_iter()
        .map(|entry| {
            let root = active
                .get(&entry.id)
                .map(|p| p.package.root.clone())
                .unwrap_or_default();
            PluginPayload {
                entry: PackedPlugin {
                    dir: format!("{PLUGINS_DIR}/{}", entry.id),
                    id: entry.id,
                    version: entry.version.to_string(),
                    hash: entry.hash,
                    tier: name_of(&entry.tier),
                    files: entry.files,
                },
                root,
            }
        })
        .collect())
}

// ─── Package changes ────────────────────────────────────────────────────────

fn changes_json(changes: &[PlanChange]) -> Value {
    serde_json::to_value(changes).unwrap_or(Value::Null)
}

/// Plans a change, and unless `dry_run` applies it: fetch and verify into
/// staging, publish into the cache, then rewrite the project's files. The
/// state lock is not held while packages download.
pub(crate) fn plugin_change(
    state: &SharedState,
    app: &AppHandle,
    change: Change,
    dry_run: bool,
    offline: bool,
) -> Result<Value, String> {
    let dir = {
        let s = lock(state)?;
        owned_dir(&s)?
    };
    let env = environment(offline);
    let project = ProjectPlugins::new(&dir);
    let plan = install::plan(&env, &project, change)?;
    let changes = plan.changes.clone();
    // Anything that would leave records without their plugin is said up
    // front; the data itself is never deleted.
    let orphaned = {
        let s = lock(state)?;
        let removed: BTreeSet<&str> = changes
            .iter()
            .filter_map(|c| match c {
                PlanChange::Removed { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect();
        s.project()
            .map(|p| {
                p.plugin_records()
                    .into_iter()
                    .filter(|(_, r)| removed.contains(r.plugin.as_str()))
                    .map(|(place, r)| format!("{} ({place})", r.name()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    if !dry_run {
        install::apply(&env, &project, plan)?;
        let mut s = lock(state)?;
        reload(&mut s, true);
        sync_runtime(&mut s);
        emit(app, &s);
    }
    Ok(json!({
        "dryRun": dry_run,
        "changes": changes_json(&changes),
        "keptRecords": orphaned,
    }))
}

/// What `plugin-install` was asked for.
pub(crate) struct InstallRequest {
    pub id: String,
    pub version: Option<String>,
    pub source: Option<String>,
    pub features: Vec<String>,
    pub dry_run: bool,
    pub offline: bool,
}

pub(crate) fn plugin_install(
    state: &SharedState,
    app: &AppHandle,
    request: InstallRequest,
) -> Result<Value, String> {
    let InstallRequest {
        id,
        version,
        source,
        features,
        dry_run,
        offline,
    } = request;
    validate_plugin_id(&id)?;
    let req: VersionReq = version
        .as_deref()
        .filter(|v| !v.is_empty())
        .unwrap_or("*")
        .parse()
        .map_err(|e: blockloom_plugin_api::semver::Error| format!("invalid version: {e}"))?;
    let source: Option<Source> = source
        .filter(|s| !s.is_empty())
        .map(|s| s.parse())
        .transpose()?;
    plugin_change(
        state,
        app,
        Change::Add {
            id,
            req,
            source,
            features,
        },
        dry_run,
        offline,
    )
}

pub(crate) fn plugin_remove(
    state: &SharedState,
    app: &AppHandle,
    id: String,
    dry_run: bool,
) -> Result<Value, String> {
    plugin_change(state, app, Change::Remove(id), dry_run, true)
}

pub(crate) fn plugin_update(
    state: &SharedState,
    app: &AppHandle,
    ids: Vec<String>,
    dry_run: bool,
    offline: bool,
) -> Result<Value, String> {
    plugin_change(state, app, Change::Update(ids), dry_run, offline)
}

pub(crate) fn plugin_pin(
    state: &SharedState,
    app: &AppHandle,
    id: String,
    version: String,
    offline: bool,
) -> Result<Value, String> {
    let version: Version = version
        .parse()
        .map_err(|e: blockloom_plugin_api::semver::Error| format!("invalid version: {e}"))?;
    plugin_change(state, app, Change::Pin { id, version }, false, offline)
}

pub(crate) fn plugin_sync(
    state: &SharedState,
    app: &AppHandle,
    dry_run: bool,
    offline: bool,
) -> Result<Value, String> {
    plugin_change(state, app, Change::Sync, dry_run, offline)
}

/// Restores the plugin files from before the last change.
pub(crate) fn plugin_rollback(state: &SharedState, app: &AppHandle) -> Result<Value, String> {
    let dir = {
        let s = lock(state)?;
        owned_dir(&s)?
    };
    let env = environment(true);
    let changes = install::rollback(&env, &ProjectPlugins::new(&dir))?;
    let mut s = lock(state)?;
    reload(&mut s, true);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(json!({ "changes": changes_json(&changes) }))
}

/// Registers a registry in `plugins.json`: a folder under the project, or an
/// `https://` URL serving a published folder.
pub(crate) fn plugin_registry(
    state: &SharedState,
    app: &AppHandle,
    name: String,
    path: String,
) -> Result<(), String> {
    let dir = {
        let s = lock(state)?;
        owned_dir(&s)?
    };
    id::validate_type_id(&name.replace('-', "_"))?;
    let project = ProjectPlugins::new(&dir);
    let mut plugins = project.read_plugins()?;
    plugins.registries.insert(name, path);
    let lock_file = project.read_lock()?;
    project.commit(&plugins, &lock_file)?;
    let mut s = lock(state)?;
    reload(&mut s, true);
    emit(app, &s);
    Ok(())
}

/// Removes cached packages no project on this machine's Dashboard (or its
/// history) still needs.
pub(crate) fn plugin_gc(state: &SharedState) -> Result<Value, String> {
    let mut keep: BTreeSet<(String, String)> = BTreeSet::new();
    let mut dirs: Vec<PathBuf> = library::list().into_iter().map(|e| e.path).collect();
    {
        let s = lock(state)?;
        if let Some(dir) = s.project_dir() {
            dirs.push(dir.to_path_buf());
        }
    }
    for dir in dirs {
        keep.extend(install::keep_set(&ProjectPlugins::new(dir))?);
    }
    let removed = Cache::new(cache::default_root()).collect_garbage(&keep)?;
    Ok(json!({ "removed": removed }))
}

// ─── Author tooling ─────────────────────────────────────────────────────────

/// Recomputes a package folder's file hashes in its `plugin.json` and
/// verifies the result.
pub(crate) fn plugin_seal(path: String) -> Result<Value, String> {
    let root = PathBuf::from(&path);
    package::seal(&root)?;
    let package = package::Package::load(&root)?;
    Ok(json!({
        "id": package.manifest.id,
        "version": package.manifest.version.to_string(),
        "hash": package.content_hash,
        "files": package.manifest.files.len(),
    }))
}

/// What a package folder is, verified, before anything installs it.
pub(crate) fn plugin_inspect(path: String) -> Result<Value, String> {
    let package = package::Package::load(Path::new(&path))?;
    let m = &package.manifest;
    let c = &package.contributions;
    Ok(json!({
        "id": m.id,
        "name": m.name,
        "version": m.version.to_string(),
        "description": m.description,
        "license": m.license,
        "tier": name_of(&m.tier),
        "capabilities": m.capabilities.iter().map(name_of).collect::<Vec<_>>(),
        "components": c.components.len() + c.resources.len(),
        "blocks": c.blocks.len(),
        "commands": c.commands.len(),
        "source": format!("path:{path}"),
    }))
}

/// Publishes a sealed package folder to a folder registry.
pub(crate) fn plugin_publish(path: String, registry: String) -> Result<Value, String> {
    let entry = DirRegistry::open(registry).publish(Path::new(&path))?;
    Ok(json!({
        "id": entry.manifest.id,
        "version": entry.manifest.version.to_string(),
        "hash": entry.content_hash,
        "archive": entry.archive,
    }))
}

// ─── Records ────────────────────────────────────────────────────────────────

/// A validated record for `qualified` (`plugin/Type`), from `payload` over
/// the schema's defaults.
fn make_record(
    active: &ActivePlugins,
    qualified: &str,
    payload: Option<Value>,
    resource: bool,
) -> Result<PluginRecord, String> {
    let (plugin, type_id) = id::split_qualified(qualified)
        .ok_or_else(|| format!("\"{qualified}\" is not a plugin/Type name"))?;
    let schema = active
        .schema(plugin, type_id)
        .ok_or_else(|| match active.get(plugin) {
            Some(_) => format!("{plugin} has no component or resource named {type_id}"),
            None => format!("{plugin} is not installed in this project"),
        })?;
    if active.is_resource(plugin, type_id) != resource {
        return Err(if resource {
            format!("{qualified} is an actor component, not a project resource")
        } else {
            format!("{qualified} is a project resource, not an actor component")
        });
    }
    validated(schema, plugin, payload.unwrap_or_else(|| json!({})))
}

fn validated(
    schema: &ComponentSchema,
    plugin: &str,
    payload: Value,
) -> Result<PluginRecord, String> {
    let payload = schema.normalize(&payload);
    schema.validate(&payload).map_err(|errors| {
        format!(
            "{}/{} is invalid: {}",
            plugin,
            schema.type_id,
            errors
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    Ok(PluginRecord::new(
        plugin,
        &schema.type_id,
        schema.version,
        payload,
    ))
}

/// Adds a plugin's component to an actor, or replaces the one of the same
/// name. The payload is checked against the plugin's schema and starts from
/// its defaults.
pub(crate) fn add_plugin_component(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    component: String,
    payload: Option<Value>,
) -> Result<String, String> {
    let mut s = lock(state)?;
    let record = make_record(active(&s)?, &component, payload, false)?;
    if s.project().and_then(|p| p.actor(&actor_id)).is_none() {
        return Err("Actor not found".to_string());
    }
    push_undo(&mut s);
    if let Some(actor) = s.project_mut().and_then(|p| p.actor_mut(&actor_id)) {
        actor.components.insert(ActorComponent::Plugin { record });
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(component)
}

/// Replaces the payload of a plugin component an actor already has.
pub(crate) fn set_plugin_component(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    component: String,
    payload: Value,
) -> Result<(), String> {
    let mut s = lock(state)?;
    {
        let project = s.project().ok_or("No project is open")?;
        let actor = project.actor(&actor_id).ok_or("Actor not found")?;
        if actor.components.plugin_record(&component).is_none() {
            return Err(format!("This actor has no \"{component}\" component"));
        }
    }
    let record = make_record(active(&s)?, &component, Some(payload), false)?;
    push_undo_for(
        &mut s,
        Some(EditSession::Comment {
            comment_id: format!("plugin-component:{actor_id}:{component}"),
        }),
    );
    if let Some(actor) = s.project_mut().and_then(|p| p.actor_mut(&actor_id)) {
        actor.components.insert(ActorComponent::Plugin { record });
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

/// Sets a project resource a plugin owns.
pub(crate) fn set_plugin_resource(
    state: &SharedState,
    app: &AppHandle,
    resource: String,
    payload: Value,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let record = make_record(active(&s)?, &resource, Some(payload), true)?;
    push_undo(&mut s);
    if let Some(project) = s.project_mut() {
        project.set_plugin_resource(record);
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

pub(crate) fn remove_plugin_resource(
    state: &SharedState,
    app: &AppHandle,
    resource: String,
) -> Result<bool, String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let removed = s
        .project_mut()
        .is_some_and(|p| p.remove_plugin_resource(&resource));
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(removed)
}

/// A command's declared action and its arguments normalized and validated
/// against its schema.
fn checked_args(
    state: &SharedState,
    command: &str,
    args: &Value,
) -> Result<(CommandAction, Value), String> {
    let (action, fields) = {
        let s = lock(state)?;
        let (_, schema) = active(&s)?
            .command(command)
            .ok_or_else(|| format!("No plugin command named \"{command}\""))?;
        (schema.action.clone(), schema.args.clone())
    };
    let args_schema = ComponentSchema {
        type_id: "Arguments".to_string(),
        display_name: String::new(),
        version: 1,
        fields,
        editor_only: false,
        migrations: Vec::new(),
        inspector: None,
    };
    let args = args_schema.normalize(args);
    args_schema.validate(&args).map_err(|errors| {
        format!(
            "{command}: {}",
            errors
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    Ok((action, args))
}

/// Runs a command a plugin declared, by `plugin/name`. Arguments are checked
/// against its schema, then the declared action is applied to the project
/// through the same validated paths the editor uses.
pub(crate) fn plugin_call(
    state: &SharedState,
    app: &AppHandle,
    command: String,
    args: Value,
) -> Result<Value, String> {
    let (action, args) = checked_args(state, &command, &args)?;
    let plugin = id::split_qualified(&command)
        .map_or("", |(p, _)| p)
        .to_string();
    let actor_of = |args: &Value| args["actor"].as_str().unwrap_or_default().to_string();
    let rest = |args: &Value| {
        let mut object = args.as_object().cloned().unwrap_or_default();
        object.remove("actor");
        Value::Object(object)
    };
    match action {
        CommandAction::AddComponent { component } => {
            let name = id::qualified(&plugin, &component);
            add_plugin_component(state, app, actor_of(&args), name.clone(), Some(rest(&args)))?;
            Ok(json!({ "component": name }))
        }
        CommandAction::SetField { component, field } => {
            let name = id::qualified(&plugin, &component);
            let actor_id = actor_of(&args);
            let current = {
                let s = lock(state)?;
                let project = s.project().ok_or("No project is open")?;
                let actor = project.actor(&actor_id).ok_or("Actor not found")?;
                actor
                    .components
                    .plugin_record(&name)
                    .map(|r| r.payload.clone())
            };
            let mut payload = current.clone().unwrap_or_else(|| json!({}));
            payload[&field] = args["value"].clone();
            if current.is_some() {
                set_plugin_component(state, app, actor_id, name.clone(), payload)?;
            } else {
                add_plugin_component(state, app, actor_id, name.clone(), Some(payload))?;
            }
            Ok(json!({ "component": name, "field": field }))
        }
        CommandAction::SetResource { resource } => {
            let name = id::qualified(&plugin, &resource);
            set_plugin_resource(state, app, name.clone(), args)?;
            Ok(json!({ "resource": name }))
        }
        CommandAction::SetResourceField {
            resource,
            field,
            append,
            template,
        } => {
            let name = id::qualified(&plugin, &resource);
            let value = match &template {
                Some(template) => Value::String(fill_template(template, &args)?),
                None => args["value"].clone(),
            };
            let mut payload = {
                let s = lock(state)?;
                let project = s.project().ok_or("No project is open")?;
                project
                    .plugin_resources
                    .iter()
                    .find(|r| r.name() == name)
                    .map_or_else(|| json!({}), |r| r.payload.clone())
            };
            if append {
                let mut list = payload[&field].as_array().cloned().unwrap_or_default();
                list.push(value);
                payload[&field] = Value::Array(list);
            } else {
                payload[&field] = value;
            }
            set_plugin_resource(state, app, name.clone(), payload)?;
            Ok(json!({ "resource": name, "field": field }))
        }
        CommandAction::Module { op } => {
            with_module(state, app, &plugin, |module| module.call_json(&op, &args))
        }
    }
}

/// Runs `f` against `plugin`'s module, loading it on first use, then puts
/// what the module logged in the run log.
fn with_module<T>(
    state: &SharedState,
    app: &AppHandle,
    plugin: &str,
    f: impl FnOnce(&mut CodeModule) -> Result<T, String>,
) -> Result<T, String> {
    let module = {
        let mut s = lock(state)?;
        let open = s.open.as_mut().ok_or("No project is open")?;
        open.modules
            .get(&open.plugins, plugin, &open.dir, &open.project.id)?
    };
    let mut module = module.lock().map_err(|_| "the plugin module is poisoned")?;
    let answer = f(&mut module);
    let logs = module.take_logs();
    drop(module);
    if !logs.is_empty() {
        let mut s = lock(state)?;
        for (level, message) in logs {
            s.push_log(LogLine {
                kind: if level <= LOG_WARN { "error" } else { "say" }.to_string(),
                actor: plugin.to_string(),
                text: message,
            });
        }
        emit(app, &s);
    }
    answer
}

/// Runs the plugin block a running game reached: the block's command, its
/// slots named by the schema. A command that wants an `actor` the block has
/// no slot for gets the actor that ran the block, and a named actor is
/// resolved to its id.
pub(crate) fn run_block(
    state: &SharedState,
    app: &AppHandle,
    actor: &str,
    plugin: &str,
    block: &str,
    args: Vec<Value>,
) -> Result<Value, String> {
    let (command, mut object) = {
        let s = lock(state)?;
        let active = active(&s)?;
        let schema = active
            .blocks()
            .into_iter()
            .find(|(p, b)| *p == plugin && b.type_id == block)
            .map(|(_, b)| b)
            .ok_or_else(|| format!("{plugin}/{block}: no installed plugin provides this block"))?;
        if schema.slots.len() != args.len() {
            return Err(format!(
                "{plugin}/{block}: the block has {} slots, the project's has {}",
                schema.slots.len(),
                args.len()
            ));
        }
        let command = schema
            .command
            .clone()
            .ok_or_else(|| format!("{plugin}/{block}: the block runs no command"))?;
        let qualified = id::qualified(plugin, &command);
        let wants_actor = active
            .command(&qualified)
            .is_some_and(|(_, c)| c.args.iter().any(|a| a.name == "actor"));
        let mut object = serde_json::Map::new();
        for (slot, value) in schema.slots.iter().zip(args) {
            let value = match (&slot.ty, &value) {
                (FieldType::Actor, Value::String(name)) => Value::String(actor_id(&s, actor, name)),
                _ => value,
            };
            object.insert(slot.name.clone(), value);
        }
        if wants_actor && !object.contains_key("actor") {
            object.insert("actor".to_string(), Value::String(actor.to_string()));
        }
        (qualified, object)
    };
    plugin_call(
        state,
        app,
        command,
        Value::Object(std::mem::take(&mut object)),
    )
}

/// `plugin-run-tool`: a stroke of a plugin's scene tool (a click is a stroke
/// of one hit). Each hit's command arguments are read from the cast's answer
/// and the options. A stroke whose command writes a resource field lands as
/// one write, so it is one undo step and one reload; any other command runs
/// once per hit.
pub(crate) fn run_tool(
    state: &SharedState,
    app: &AppHandle,
    plugin: &str,
    tool: &str,
    hits: &[Value],
    options: &Value,
) -> Result<Value, String> {
    let (command, mut calls) = {
        let s = lock(state)?;
        let active = active(&s)?;
        let schema = active
            .tools()
            .into_iter()
            .find(|(p, t)| *p == plugin && t.name == tool)
            .map(|(_, t)| t)
            .ok_or_else(|| format!("{plugin}/{tool}: no installed plugin provides this tool"))?;
        let mut calls = Vec::with_capacity(hits.len());
        for hit in hits {
            let args = schema
                .resolve_args(hit, options)
                .map_err(|e| format!("{plugin}/{tool}: {e}"))?;
            // Two hits that ask for the same thing ask once.
            if !calls.contains(&args) {
                calls.push(args);
            }
        }
        (id::qualified(plugin, &schema.command), calls)
    };
    match calls.len() {
        0 => Ok(json!({ "applied": 0 })),
        1 => plugin_call(state, app, command, calls.remove(0)),
        count => {
            let checked = calls
                .iter()
                .map(|args| checked_args(state, &command, args))
                .collect::<Result<Vec<_>, _>>()?;
            if let Some(CommandAction::SetResourceField {
                resource,
                field,
                append,
                template,
            }) = checked.first().map(|(action, _)| action.clone())
            {
                let name = id::qualified(plugin, &resource);
                let mut payload = {
                    let s = lock(state)?;
                    let project = s.project().ok_or("No project is open")?;
                    project
                        .plugin_resources
                        .iter()
                        .find(|r| r.name() == name)
                        .map_or_else(|| json!({}), |r| r.payload.clone())
                };
                let mut list = payload[&field].as_array().cloned().unwrap_or_default();
                for (_, args) in &checked {
                    let value = match &template {
                        Some(template) => Value::String(fill_template(template, args)?),
                        None => args["value"].clone(),
                    };
                    if append {
                        list.push(value);
                    } else {
                        payload[&field] = value;
                    }
                }
                if append {
                    payload[&field] = Value::Array(list);
                }
                set_plugin_resource(state, app, name.clone(), payload)?;
                return Ok(json!({ "resource": name, "field": field, "applied": count }));
            }
            for args in calls {
                plugin_call(state, app, command.clone(), args)?;
            }
            Ok(json!({ "applied": count }))
        }
    }
}

/// An actor slot's text as an id: an id as it stands, else the first actor
/// of that name in the open scene, else `name` unchanged for the command to
/// report.
fn actor_id(s: &AppState, running: &str, name: &str) -> String {
    let Some(project) = s.project() else {
        return name.to_string();
    };
    if project.actor(name).is_some() {
        return name.to_string();
    }
    // The scene the running actor belongs to answers first.
    project
        .scenes
        .iter()
        .filter(|scene| scene.actors.iter().any(|a| a.id == running))
        .chain(project.scenes.iter())
        .flat_map(|scene| scene.actors.iter())
        .find(|a| a.name == name)
        .map_or_else(|| name.to_string(), |a| a.id.clone())
}

/// Upgrades every record of `plugin` to its schema's current version, on
/// copies: all of them or none. A snapshot of what was replaced is kept under
/// `.blockloom/plugin-migrations` beside the project's undo history.
pub(crate) fn plugin_migrate(
    state: &SharedState,
    app: &AppHandle,
    plugin: String,
    dry_run: bool,
) -> Result<Value, String> {
    let mut s = lock(state)?;
    let dir = open_dir(&s)?;
    let project = s.project().ok_or("No project is open")?;
    let old: Vec<PluginRecord> = project
        .plugin_records()
        .into_iter()
        .filter(|(_, r)| r.plugin == plugin)
        .map(|(_, r)| r.clone())
        .filter(|r| {
            matches!(
                active(&s).map(|a| a.check_record(r)),
                Ok(RecordStatus::NeedsMigration { .. })
            )
        })
        .collect();
    if old.is_empty() {
        return Ok(json!({ "dryRun": dry_run, "migrated": 0 }));
    }
    let migrated = migrate_records(active(&s)?, &old).map_err(|errors| {
        format!(
            "Nothing was changed; migration failed for:\n- {}",
            errors.join("\n- ")
        )
    })?;
    let count = old.len();
    if !dry_run {
        let folder = dir.join(".blockloom").join("plugin-migrations");
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let stamp = blockloom_core::sync::now_secs();
        let snapshot = folder.join(format!("{plugin}-{stamp}.json"));
        std::fs::write(
            &snapshot,
            serde_json::to_string_pretty(&old).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("{}: {e}", snapshot.display()))?;
        push_undo(&mut s);
        if let Some(project) = s.project_mut() {
            project.replace_plugin_records(&migrated);
        }
        auto_save(&s);
        sync_runtime(&mut s);
        emit(app, &s);
    }
    Ok(json!({
        "dryRun": dry_run,
        "migrated": count,
        "records": migrated,
    }))
}

// ─── Importers and build hooks ──────────────────────────────────────────────

/// `plugin-importers`: what plugins can import, and how each imported file
/// stands against its source.
pub(crate) fn plugin_importers(state: &SharedState) -> Result<Value, String> {
    let s = lock(state)?;
    let dir = open_dir(&s)?;
    let active = active(&s)?;
    let importers: Vec<Value> = active
        .importers()
        .into_iter()
        .map(|(plugin, i)| {
            json!({
                "plugin": plugin,
                "name": i.name,
                "summary": i.summary,
                "extensions": i.extensions,
                "limitMs": i.limit_ms,
            })
        })
        .collect();
    let hooks: Vec<Value> = active
        .build_hooks()
        .into_iter()
        .map(|(plugin, h)| {
            json!({
                "plugin": plugin,
                "name": h.name,
                "summary": h.summary,
                "limitMs": h.limit_ms,
            })
        })
        .collect();
    Ok(json!({
        "importers": importers,
        "buildHooks": hooks,
        "imports": imports::status(&dir),
    }))
}

/// Runs an importer over `path`. `importer` is `plugin-id/name` or just the
/// name; with none given, the first importer that takes the file's extension.
pub(crate) fn plugin_import(
    state: &SharedState,
    app: &AppHandle,
    path: String,
    importer: Option<String>,
) -> Result<Imported, String> {
    let (dir, name, plugin, schema) = {
        let s = lock(state)?;
        let dir = owned_dir(&s)?;
        let name = s.project().map(|p| p.name.clone()).unwrap_or_default();
        let extension = Path::new(&path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let candidates = active(&s)?.importers_for(&extension);
        let picked = match importer.as_deref().filter(|i| !i.is_empty()) {
            None => candidates.first().copied(),
            Some(wanted) => candidates
                .iter()
                .copied()
                .find(|(plugin, i)| wanted == i.name || wanted == id::qualified(plugin, &i.name)),
        };
        let (plugin, schema) = picked.ok_or_else(|| match importer {
            Some(wanted) if !wanted.is_empty() => {
                format!("No importer \"{wanted}\" takes .{extension} files")
            }
            _ => format!("No installed plugin imports .{extension} files"),
        })?;
        (dir, name, plugin.to_string(), schema.clone())
    };
    let imported = with_module(state, app, &plugin, |module| {
        imports::run_import(module, &plugin, &schema, &dir, &name, &path)
    })?;
    let mut s = lock(state)?;
    for warning in &imported.warnings {
        s.push_log(LogLine {
            kind: "error".to_string(),
            actor: plugin.clone(),
            text: format!("{path}: {warning}"),
        });
    }
    s.push_log(LogLine {
        kind: "say".to_string(),
        actor: plugin.clone(),
        text: format!("Imported {path}: {} file(s)", imported.outputs.len()),
    });
    emit(app, &s);
    Ok(imported)
}

/// `plugin-imports`: every remembered import and whether it is still current.
pub(crate) fn plugin_imports(state: &SharedState) -> Result<Value, String> {
    let s = lock(state)?;
    let dir = open_dir(&s)?;
    Ok(json!({ "imports": imports::status(&dir) }))
}

/// Imports each of `paths` an installed importer takes. A file nothing takes
/// is left alone, and a failed import is logged, not raised: the copy
/// already happened.
pub(crate) fn auto_import(state: &SharedState, app: &AppHandle, paths: &[String]) {
    for path in paths {
        if imports::is_imported_output(path) {
            continue;
        }
        let extension = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let wanted = lock(state)
            .ok()
            .and_then(|s| {
                s.open
                    .as_ref()
                    .map(|open| !open.plugins.importers_for(&extension).is_empty())
            })
            .unwrap_or(false);
        if !wanted {
            continue;
        }
        if let Err(error) = plugin_import(state, app, path.clone(), None)
            && let Ok(mut s) = lock(state)
        {
            s.push_log(LogLine {
                kind: "error".to_string(),
                actor: "Blockloom".to_string(),
                text: format!("Couldn't import {path}: {error}"),
            });
            emit(app, &s);
        }
    }
}

/// What `plugin-reimport` did.
#[derive(Debug, Default, Serialize)]
pub(crate) struct Reimported {
    /// Sources imported again.
    pub reimported: Vec<String>,
    /// Sources left alone, with why: an output edited by hand, a source that
    /// is gone, or one nothing imports now.
    pub skipped: Vec<Value>,
    /// Sources whose import failed, with the error.
    pub failed: Vec<Value>,
}

/// Imports again every file whose source, dependency or output changed since
/// its last import. With `path`, that one file is imported again whatever its
/// state, which is how a hand-edited output is replaced on purpose.
pub(crate) fn plugin_reimport(
    state: &SharedState,
    app: &AppHandle,
    path: Option<String>,
) -> Result<Reimported, String> {
    use imports::ImportState;
    let statuses = {
        let s = lock(state)?;
        imports::status(&open_dir(&s)?)
    };
    let mut out = Reimported::default();
    let mut found = path.is_none();
    for status in statuses {
        if path.as_deref().is_some_and(|p| p != status.source) {
            continue;
        }
        found = true;
        match &status.state {
            ImportState::Fresh => continue,
            ImportState::SourceMissing => {
                out.skipped
                    .push(json!({ "path": status.source, "reason": "its source is gone" }));
                continue;
            }
            ImportState::OutputEdited { path: edited } if path.is_none() => {
                out.skipped.push(json!({
                    "path": status.source,
                    "reason": format!("{edited} was edited by hand; import it by name to replace it"),
                }));
                continue;
            }
            _ => {}
        }
        let importer = id::qualified(&status.plugin, &status.importer);
        match plugin_import(state, app, status.source.clone(), Some(importer)) {
            Ok(_) => out.reimported.push(status.source),
            Err(error) => out
                .failed
                .push(json!({ "path": status.source, "error": error })),
        }
    }
    if !found && let Some(path) = path {
        return Err(format!("{path} was never imported"));
    }
    Ok(out)
}

/// Brings stale imports up to date before a run or a build, logging what
/// couldn't be. Never fails the caller: the old output is still there.
pub(crate) fn refresh_imports(state: &SharedState, app: &AppHandle) {
    // An attached copy can't write the owner's files.
    if !lock(state).is_ok_and(|s| owned_dir(&s).is_ok()) {
        return;
    }
    let Ok(done) = plugin_reimport(state, app, None) else {
        return;
    };
    if done.failed.is_empty() {
        return;
    }
    if let Ok(mut s) = lock(state) {
        for failure in &done.failed {
            s.push_log(LogLine {
                kind: "error".to_string(),
                actor: "Blockloom".to_string(),
                text: format!("Couldn't import {failure}"),
            });
        }
        emit(app, &s);
    }
}

/// Forgets what an import of `path` made, for a source that was deleted.
pub(crate) fn forget_import(dir: &Path, path: &str) {
    let _ = imports::forget(dir, path);
}

/// Where a build hook's files are staged before the build copies them.
const COOKED_DIR: &str = ".blockloom/cooked";

/// Runs every build hook of the project's plugins for `target` and stages
/// what they made. A hook that fails stops the build. The files ship under
/// `plugins/<id>/cooked/`.
pub(crate) fn run_build_hooks(
    state: &SharedState,
    app: &AppHandle,
    dir: &Path,
    project: &Project,
    target: &Target,
) -> Result<Vec<ExtraFile>, String> {
    let hooks: Vec<(String, blockloom_plugin_api::schema::BuildHookSchema)> = {
        let s = lock(state)?;
        match s.open.as_ref() {
            Some(open) => open
                .plugins
                .build_hooks()
                .into_iter()
                .map(|(plugin, hook)| (plugin.to_string(), hook.clone()))
                .collect(),
            None => Vec::new(),
        }
    };
    let staging = dir.join(COOKED_DIR);
    let _ = std::fs::remove_dir_all(&staging);
    let mut extras: Vec<ExtraFile> = Vec::new();
    for (plugin, hook) in hooks {
        blockloom_core::build_control::step(&format!("Running build hook {plugin}/{}", hook.name))?;
        let run = with_module(state, app, &plugin, |module| {
            imports::run_build_hook(module, &plugin, &hook, dir, &project.name, target.triple)
        })?;
        if !run.warnings.is_empty() {
            let mut s = lock(state)?;
            for warning in &run.warnings {
                s.push_log(LogLine {
                    kind: "error".to_string(),
                    actor: plugin.clone(),
                    text: format!("{}: {warning}", hook.name),
                });
            }
            emit(app, &s);
        }
        for file in run.files {
            let to = format!("{PLUGINS_DIR}/{plugin}/cooked/{}", file.path);
            if extras.iter().any(|e| e.to == to) {
                return Err(format!("two build hooks of {plugin} made {}", file.path));
            }
            let from = staging.join(&plugin).join(&file.path);
            if let Some(parent) = from.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            std::fs::write(&from, &file.data).map_err(|e| format!("{}: {e}", from.display()))?;
            extras.push(ExtraFile { to, from });
        }
    }
    Ok(extras)
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::blocks::{Instruction, InstructionKind, Strand};
    use blockloom_core::scene::Mode;

    fn with_block() -> Project {
        let mut project = Project::starter("Blocks", Mode::TwoD);
        project.actors[0]
            .graph
            .strands
            .push(Strand::with_instructions(
                0,
                0,
                vec![
                    Instruction::new(InstructionKind::WhenStarted),
                    Instruction::new(InstructionKind::PluginBlock {
                        plugin: "com.example.health".to_string(),
                        block: "set_hp".to_string(),
                        args: Vec::new(),
                    }),
                ],
            ));
        project
    }

    #[test]
    fn a_plugin_block_with_no_plugin_stops_play_and_build() {
        let project = with_block();
        let error = preflight(&ActivePlugins::default(), &project).unwrap_err();
        assert!(error.contains("com.example.health/set_hp"), "{error}");
        assert!(error.contains("isn't provided"), "{error}");
        assert!(
            preflight(
                &ActivePlugins::default(),
                &Project::starter("None", Mode::TwoD)
            )
            .is_ok()
        );
    }

    #[test]
    fn reporters_and_hats_are_found_wherever_they_sit() {
        use blockloom_core::value::{Op, PLUGIN_READ, Value as Slot};
        let read = |block: &str| {
            Slot::op(
                Op::from_name(PLUGIN_READ),
                vec![
                    Slot::text("com.example.tally"),
                    Slot::text(block),
                    Slot::text("coins"),
                ],
            )
        };
        let mut project = Project::starter("Blocks", Mode::TwoD);
        project.actors[0]
            .graph
            .strands
            .push(Strand::with_instructions(
                0,
                0,
                vec![
                    Instruction::new(InstructionKind::WhenPlugin {
                        plugin: "com.example.tally".to_string(),
                        block: "changed".to_string(),
                        event: "changed".to_string(),
                        args: vec!["coins".to_string()],
                    }),
                    // Nested in an operator, inside another block's slot.
                    Instruction::new(InstructionKind::Say {
                        text: Slot::op(Op::Add, vec![Slot::number(1.0), read("count")]),
                    }),
                ],
            ));
        let uses = project.plugin_blocks();
        let shapes: Vec<_> = uses
            .iter()
            .map(|u| (u.block.as_str(), u.shape, u.slots))
            .collect();
        assert_eq!(
            shapes,
            [
                ("changed", PluginBlockShape::Hat, 1),
                ("count", PluginBlockShape::Reporter, 1),
            ]
        );
        let error = preflight(&ActivePlugins::default(), &project).unwrap_err();
        assert!(error.contains("com.example.tally/changed"), "{error}");
        assert!(error.contains("com.example.tally/count"), "{error}");
    }
}
