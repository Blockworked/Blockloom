//! Running a script written in a component-model language (Python,
//! TypeScript, Go, ...) on the desktop, sandboxed under wasmtime.
//!
//! The guest is a wasm component built against the frozen
//! `blockloom:script@0.2.0` world ([`blockloom_core::script::wit`]): it
//! imports the typed host interfaces and exports `entry`. Every import lands
//! on the same `number_for`/`text_for`/`act_for_asked` the native and wasmi
//! paths use, so a component sees the same snapshot, writes the same effects
//! and reads its own write back the way a native script does.
//!
//! The sandbox is the guest's linear memory ceiling, fuel per entry point,
//! and an empty WASI context: no files, no sockets, no environment, no
//! arguments. Guest runtimes (CPython, QuickJS) import WASI for their own
//! startup, which is all that context answers. A trap or an empty tank stops
//! the script for the rest of the run with an error effect.

use super::script::{Asked, ScriptEvent};
use blockloom_core::script::abi;
use blockloom_core::sense;
use blockloom_core::vm::Effect;
use blockloom_plugin_api::wasm::FUEL_PER_MS;
use std::cell::RefCell;
use wasmtime::component::types::ComponentItem;
use wasmtime::component::{
    Component, ComponentNamedList, ComponentType, Instance, Lift, Linker, Lower, ResourceTable,
    ResourceType, TypedFunc,
};
use wasmtime::{Config, Engine, Store, StoreContextMut, StoreLimits, StoreLimitsBuilder, Trap};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

/// The world's package version, which every interface name carries.
const PKG: &str = "blockloom:script";
const VERSION: &str = "0.2.0";

/// Budgets match the core-module path; a component's runtime is heavier, so
/// its memory ceiling is higher (CPython alone maps tens of MiB).
pub const TICK_BUDGET_MS: u32 = super::script_wasm::TICK_BUDGET_MS;
pub const CALL_BUDGET_MS: u32 = super::script_wasm::CALL_BUDGET_MS;
pub const MEMORY_LIMIT_MIB: u32 = 256;

/// Whether `bytes` are a wasm component (layer 1) rather than a core module.
pub fn is_component(bytes: &[u8]) -> bool {
    bytes.len() >= 8 && bytes.starts_with(b"\0asm") && bytes[6..8] == [0x01, 0x00]
}

struct HostState {
    actor: String,
    /// Where effects land. Set for the entry call in progress and null
    /// otherwise; host functions run only inside it, on this thread.
    asked: *mut Asked,
    stopped: bool,
    limits: StoreLimits,
    wasi: WasiCtx,
    table: ResourceTable,
}

// Safety: the store lives in a `ComponentScript` the engine keeps on its
// main thread; `asked` is only dereferenced inside a call on that thread.
unsafe impl Send for HostState {}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

/// `read-error` in the interfaces that only know `missing`.
#[derive(ComponentType, Lift, Lower, Clone, Copy)]
#[component(enum)]
#[repr(u8)]
enum Missing {
    #[component(name = "missing")]
    Missing,
}

/// `read-error` in the interfaces that can also run out of room.
#[derive(ComponentType, Lift, Lower, Clone, Copy)]
#[component(enum)]
#[repr(u8)]
enum Unread {
    #[component(name = "missing")]
    Missing,
    #[component(name = "too-long")]
    #[allow(dead_code)]
    TooLong,
}

#[derive(ComponentType, Lift, Lower, Clone, Copy)]
#[component(record)]
struct Vec3 {
    #[component(name = "x")]
    x: f32,
    #[component(name = "y")]
    y: f32,
    #[component(name = "z")]
    z: f32,
}

#[derive(ComponentType, Lift, Lower, Clone, Copy)]
#[component(record)]
struct PoseData {
    #[component(name = "position")]
    position: Vec3,
    #[component(name = "rotation")]
    rotation: Vec3,
    #[component(name = "scale")]
    scale: f32,
}

type Ctx<'a> = StoreContextMut<'a, HostState>;

fn interface(name: &str) -> String {
    format!("{PKG}/{name}@{VERSION}")
}

/// A number read: `what` with `a`/`b`/`arg` through the shared sensing.
fn number(c: &Ctx<'_>, what: u32, a: &str, b: &str, arg: f64) -> Option<f64> {
    super::script::number_for(&c.data().actor, what, a, b, arg)
}

fn text(c: &Ctx<'_>, what: u32, a: &str, b: &str) -> Option<String> {
    super::script::text_for(&c.data().actor, what, a, b)
}

/// Writes one act into the call in progress.
fn act(c: &mut Ctx<'_>, what: u32, a: &str, b: &str, cc: &str, numbers: &[f64]) {
    let data = c.data_mut();
    if data.asked.is_null() {
        return;
    }
    // Safety: `asked` is set for the entry call in progress and a host
    // function runs only inside it, on this thread.
    let asked = unsafe { &mut *data.asked };
    super::script::act_for_asked(&data.actor, asked, what, a, b, cc, numbers);
}

fn read_number(c: &Ctx<'_>, what: u32, a: &str, b: &str, arg: f64) -> (Result<f64, Missing>,) {
    (number(c, what, a, b, arg).ok_or(Missing::Missing),)
}

fn read_text(c: &Ctx<'_>, what: u32, a: &str, b: &str) -> (Result<String, Missing>,) {
    (text(c, what, a, b).ok_or(Missing::Missing),)
}

/// A resource type the denied interfaces name; nothing ever makes one.
struct Denied;

/// Defines every import under `wasi:http/` as a function that traps and a
/// resource nothing can make. JS runtimes link `fetch` in even when the
/// script never calls it; a script that does gets a trap, not a network.
fn deny_http(linker: &mut Linker<HostState>, component: &Component) -> wasmtime::Result<()> {
    let engine = linker.engine().clone();
    for (name, item) in component.component_type().imports(&engine) {
        if !name.starts_with("wasi:http/") {
            continue;
        }
        let ComponentItem::ComponentInstance(instance) = item.ty else {
            continue;
        };
        let mut defined = linker.instance(name)?;
        for (export, item) in instance.exports(&engine) {
            match item.ty {
                ComponentItem::ComponentFunc(_) => {
                    defined.func_new(export, |_, _, _, _| {
                        Err(wasmtime::format_err!(
                            "networking isn't available to scripts"
                        ))
                    })?;
                }
                ComponentItem::Resource(_) => {
                    // The stream and pollable types are wasi:io's, shared
                    // with the interfaces already linked above.
                    use wasmtime_wasi::p2::{
                        DynInputStream, DynOutputStream, DynPollable, IoError,
                    };
                    let ty = match export {
                        "input-stream" => ResourceType::host::<DynInputStream>(),
                        "output-stream" => ResourceType::host::<DynOutputStream>(),
                        "pollable" => ResourceType::host::<DynPollable>(),
                        "io-error" => ResourceType::host::<IoError>(),
                        _ => ResourceType::host::<Denied>(),
                    };
                    defined.resource(export, ty, |_, _| Ok(()))?;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn link(linker: &mut Linker<HostState>, component: &Component) -> wasmtime::Result<()> {
    wasmtime_wasi::p2::add_to_linker_sync(linker)?;
    deny_http(linker, component)?;

    let mut sensors = linker.instance(&interface("sensors"))?;
    sensors.func_wrap(
        "read",
        |c: Ctx<'_>, (what, a, b, arg): (u32, String, String, f64)| {
            Ok((number(&c, what, &a, &b, arg).ok_or(Missing::Missing),))
        },
    )?;

    let mut texts = linker.instance(&interface("texts"))?;
    texts.func_wrap(
        "read-text",
        |c: Ctx<'_>, (what, a, b): (u32, String, String)| {
            Ok((text(&c, what, &a, &b).ok_or(Unread::Missing),))
        },
    )?;

    let mut acts = linker.instance(&interface("acts"))?;
    acts.func_wrap(
        "act",
        |mut c: Ctx<'_>, (what, a, b, cc, numbers): (u32, String, String, String, Vec<f64>)| {
            act(&mut c, what, &a, &b, &cc, &numbers);
            Ok(())
        },
    )?;

    let mut lifecycle = linker.instance(&interface("lifecycle"))?;
    lifecycle.func_wrap("pose", |c: Ctx<'_>, (target,): (String,)| {
        let pose = super::script::pose_bytes_for(&c.data().actor, &target).map(|bytes| {
            let f = |i: usize| f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
            PoseData {
                position: Vec3 {
                    x: f(0),
                    y: f(1),
                    z: f(2),
                },
                rotation: Vec3 {
                    x: f(3),
                    y: f(4),
                    z: f(5),
                },
                scale: f(6),
            }
        });
        Ok((pose.ok_or(Unread::Missing),))
    })?;
    lifecycle.func_wrap("pose-bytes", |c: Ctx<'_>, (target,): (String,)| {
        let bytes = super::script::pose_bytes_for(&c.data().actor, &target).map(|b| b.to_vec());
        Ok((bytes.ok_or(Unread::Missing),))
    })?;

    let mut state = linker.instance(&interface("state"))?;
    state.func_wrap("get-number", |c: Ctx<'_>, (key,): (String,)| {
        Ok(read_number(&c, abi::READ_DATA, &key, "", 0.0))
    })?;
    state.func_wrap("get-text", |c: Ctx<'_>, (key,): (String,)| {
        Ok(read_text(&c, abi::TEXT_DATA, &key, ""))
    })?;
    state.func_wrap(
        "set-number",
        |mut c: Ctx<'_>, (key, value): (String, f64)| {
            act(&mut c, abi::ACT_SET_DATA, &key, "", "", &[value]);
            Ok(())
        },
    )?;
    state.func_wrap(
        "set-text",
        |mut c: Ctx<'_>, (key, value): (String, String)| {
            act(&mut c, abi::ACT_SET_DATA_TEXT, &key, "", &value, &[]);
            Ok(())
        },
    )?;
    state.func_wrap("clear", |mut c: Ctx<'_>, (key,): (String,)| {
        act(&mut c, abi::ACT_CLEAR_DATA, &key, "", "", &[]);
        Ok(())
    })?;

    let mut vars = linker.instance(&interface("vars"))?;
    vars.func_wrap("get-number", |c: Ctx<'_>, (name,): (String,)| {
        Ok((number(&c, abi::READ_VARIABLE, &name, "", 0.0).unwrap_or(0.0),))
    })?;
    vars.func_wrap("get-text", |c: Ctx<'_>, (name,): (String,)| {
        Ok((text(&c, abi::TEXT_VARIABLE, &name, "").unwrap_or_default(),))
    })?;
    vars.func_wrap(
        "set-number",
        |mut c: Ctx<'_>, (name, value): (String, f64)| {
            act(&mut c, abi::ACT_SET_VARIABLE, &name, "", "", &[value]);
            Ok(())
        },
    )?;
    vars.func_wrap(
        "set-text",
        |mut c: Ctx<'_>, (name, value): (String, String)| {
            act(&mut c, abi::ACT_SET_VARIABLE_TEXT, &name, "", &value, &[]);
            Ok(())
        },
    )?;

    let mut lists = linker.instance(&interface("lists"))?;
    lists.func_wrap("len", |c: Ctx<'_>, (name,): (String,)| {
        Ok((number(&c, abi::READ_LIST_LENGTH, &name, "", 0.0).unwrap_or(0.0) as u32,))
    })?;
    lists.func_wrap("get-number", |c: Ctx<'_>, (name, index): (String, u32)| {
        Ok(read_number(
            &c,
            abi::READ_LIST_ITEM,
            &name,
            "",
            f64::from(index),
        ))
    })?;
    lists.func_wrap("get-text", |c: Ctx<'_>, (name, index): (String, u32)| {
        Ok(read_text(
            &c,
            abi::TEXT_LIST_ITEM,
            &name,
            &index.to_string(),
        ))
    })?;
    lists.func_wrap(
        "add-number",
        |mut c: Ctx<'_>, (name, value): (String, f64)| {
            act(&mut c, abi::ACT_LIST_ADD, &name, "", "", &[value]);
            Ok(())
        },
    )?;
    lists.func_wrap(
        "add-text",
        |mut c: Ctx<'_>, (name, value): (String, String)| {
            act(&mut c, abi::ACT_LIST_ADD_TEXT, &name, "", &value, &[]);
            Ok(())
        },
    )?;
    lists.func_wrap(
        "insert-number",
        |mut c: Ctx<'_>, (name, index, value): (String, u32, f64)| {
            act(
                &mut c,
                abi::ACT_LIST_INSERT,
                &name,
                "",
                "",
                &[f64::from(index), value],
            );
            Ok(())
        },
    )?;
    lists.func_wrap(
        "insert-text",
        |mut c: Ctx<'_>, (name, index, value): (String, u32, String)| {
            act(
                &mut c,
                abi::ACT_LIST_INSERT_TEXT,
                &name,
                "",
                &value,
                &[f64::from(index)],
            );
            Ok(())
        },
    )?;
    lists.func_wrap(
        "replace-number",
        |mut c: Ctx<'_>, (name, index, value): (String, u32, f64)| {
            act(
                &mut c,
                abi::ACT_LIST_REPLACE,
                &name,
                "",
                "",
                &[f64::from(index), value],
            );
            Ok(())
        },
    )?;
    lists.func_wrap(
        "replace-text",
        |mut c: Ctx<'_>, (name, index, value): (String, u32, String)| {
            act(
                &mut c,
                abi::ACT_LIST_REPLACE_TEXT,
                &name,
                "",
                &value,
                &[f64::from(index)],
            );
            Ok(())
        },
    )?;
    lists.func_wrap("delete", |mut c: Ctx<'_>, (name, index): (String, u32)| {
        act(
            &mut c,
            abi::ACT_LIST_DELETE,
            &name,
            "",
            "",
            &[f64::from(index)],
        );
        Ok(())
    })?;
    lists.func_wrap("clear", |mut c: Ctx<'_>, (name,): (String,)| {
        act(&mut c, abi::ACT_LIST_CLEAR, &name, "", "", &[]);
        Ok(())
    })?;

    let mut physics = linker.instance(&interface("physics"))?;
    physics.func_wrap(
        "query",
        |mut c: Ctx<'_>, (kind, policy, mask, numbers): (String, String, u32, Vec<f64>)| {
            let mut all = vec![f64::from(mask)];
            all.extend(numbers);
            act(&mut c, abi::ACT_PHYSICS_QUERY, &kind, &policy, "", &all);
            Ok(())
        },
    )?;
    physics.func_wrap("query-number", |c: Ctx<'_>, (field, hit): (String, u32)| {
        Ok((number(&c, abi::READ_QUERY, &field, "", f64::from(hit)).unwrap_or(0.0),))
    })?;
    physics.func_wrap("query-text", |c: Ctx<'_>, (field, hit): (String, u32)| {
        Ok(read_text(&c, abi::TEXT_QUERY, &field, &hit.to_string()))
    })?;
    physics.func_wrap(
        "controller-move",
        |mut c: Ctx<'_>, (op, value): (String, Vec<f64>)| {
            act(&mut c, abi::ACT_CONTROLLER, &op, "", "", &value);
            Ok(())
        },
    )?;
    physics.func_wrap(
        "controller-number",
        |c: Ctx<'_>, (field, obstacle): (String, u32)| {
            Ok(
                (
                    number(&c, abi::READ_CONTROLLER, &field, "", f64::from(obstacle))
                        .unwrap_or(0.0),
                ),
            )
        },
    )?;
    physics.func_wrap(
        "controller-text",
        |c: Ctx<'_>, (field, obstacle): (String, u32)| {
            Ok(read_text(
                &c,
                abi::TEXT_CONTROLLER,
                &field,
                &obstacle.to_string(),
            ))
        },
    )?;
    physics.func_wrap(
        "add-force",
        |mut c: Ctx<'_>, (mode, torque, target, v): (String, bool, String, Vec3)| {
            let torque = if torque { "torque" } else { "" };
            let vector = [f64::from(v.x), f64::from(v.y), f64::from(v.z)];
            if target.trim().is_empty() {
                act(&mut c, abi::ACT_ADD_FORCE, &mode, torque, "", &vector);
            } else {
                act(
                    &mut c,
                    abi::ACT_ADD_FORCE_OTHER,
                    &target,
                    &mode,
                    torque,
                    &vector,
                );
            }
            Ok(())
        },
    )?;

    let mut plugins = linker.instance(&interface("plugins"))?;
    let asked = |block: &str, slots: &str| format!("{block}{}{slots}", abi::PLUGIN_SEP);
    plugins.func_wrap(
        "read-number",
        move |c: Ctx<'_>, (plugin, block, slots): (String, String, String)| {
            Ok(read_number(
                &c,
                abi::READ_PLUGIN,
                &plugin,
                &asked(&block, &slots),
                0.0,
            ))
        },
    )?;
    plugins.func_wrap(
        "read-text",
        move |c: Ctx<'_>, (plugin, block, slots): (String, String, String)| {
            Ok(read_text(
                &c,
                abi::TEXT_PLUGIN,
                &plugin,
                &asked(&block, &slots),
            ))
        },
    )?;
    plugins.func_wrap(
        "call",
        |mut c: Ctx<'_>, (plugin, block, slots): (String, String, String)| {
            act(&mut c, abi::ACT_PLUGIN_CALL, &plugin, &block, &slots, &[]);
            Ok(())
        },
    )?;
    Ok(())
}

/// One engine for every script: compiled code is shared between engines of
/// the same configuration only, and each actor loads its own copy.
fn shared_engine() -> Result<Engine, String> {
    static ENGINE: std::sync::OnceLock<Result<Engine, String>> = std::sync::OnceLock::new();
    ENGINE
        .get_or_init(|| {
            let mut config = Config::new();
            config.consume_fuel(true);
            Engine::new(&config).map_err(|e| e.to_string())
        })
        .clone()
}

/// FNV-1a over the bytes: a cache key, not a security boundary.
fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

/// The component compiled once per process, and once per file on disk:
/// compiling a guest runtime such as CPython takes seconds, so the second
/// actor and the next Play reuse the machine code. The disk copy is keyed by
/// the bytes and wasmtime's compatibility hash, and only ever holds what
/// this function serialized.
fn compiled(
    engine: &Engine,
    bytes: &[u8],
    relative: &str,
    project_dir: Option<&std::path::Path>,
) -> Result<Component, String> {
    type Cache = std::sync::Mutex<std::collections::HashMap<u64, Component>>;
    static LOADED: std::sync::OnceLock<Cache> = std::sync::OnceLock::new();
    let key = fingerprint(bytes);
    let loaded = LOADED.get_or_init(Default::default);
    if let Some(component) = loaded.lock().unwrap().get(&key) {
        return Ok(component.clone());
    }
    let disk = project_dir.map(|dir| {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&engine.precompile_compatibility_hash(), &mut hasher);
        dir.join(".blockloom/build/components").join(format!(
            "{key:016x}-{:016x}.cwasm",
            std::hash::Hasher::finish(&hasher)
        ))
    });
    let cached = disk
        .as_ref()
        .and_then(|path| std::fs::read(path).ok())
        // Safety: the file is only ever written below from `serialize`.
        .and_then(|image| unsafe { Component::deserialize(engine, image) }.ok());
    let component = match cached {
        Some(component) => component,
        None => {
            let component =
                Component::new(engine, bytes).map_err(|e| format!("{relative}: {e}"))?;
            if let (Some(path), Ok(image)) = (&disk, component.serialize()) {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(path, image);
            }
            component
        }
    };
    loaded.lock().unwrap().insert(key, component.clone());
    Ok(component)
}

fn export<P: ComponentNamedList + Lower, R: ComponentNamedList + Lift>(
    instance: &Instance,
    store: &mut Store<HostState>,
    entry: &wasmtime::component::ComponentExportIndex,
    name: &str,
) -> Option<TypedFunc<P, R>> {
    let index = instance.get_export_index(&mut *store, Some(entry), name)?;
    instance.get_typed_func(&mut *store, index).ok()
}

/// One actor's script as a sandboxed component. Held on the main thread
/// like the other backends.
pub struct ComponentScript {
    relative: String,
    store: RefCell<Store<HostState>>,
    start: TypedFunc<(), ()>,
    tick: TypedFunc<(f32,), ()>,
    frame: TypedFunc<(f32,), ()>,
    ui: TypedFunc<(f32,), ()>,
    stop: TypedFunc<(), ()>,
    destroy: TypedFunc<(), ()>,
    event: TypedFunc<(u32, f64, f64, f64, f64), ()>,
}

impl ComponentScript {
    /// Compiles and instantiates `bytes` as `relative`'s script.
    pub fn load_bytes(bytes: &[u8], relative: &str) -> Result<ComponentScript, String> {
        Self::load_with(bytes, relative, None)
    }

    /// As [`Self::load_bytes`], keeping the compiled code under the
    /// project's `.blockloom/build/components` when a folder is given.
    pub fn load_with(
        bytes: &[u8],
        relative: &str,
        project_dir: Option<&std::path::Path>,
    ) -> Result<ComponentScript, String> {
        if !is_component(bytes) {
            return Err(format!("{relative} isn't a WebAssembly component"));
        }
        let engine = shared_engine()?;
        let component = compiled(&engine, bytes, relative, project_dir)?;
        let mut linker = <Linker<HostState>>::new(&engine);
        link(&mut linker, &component).map_err(|e| e.to_string())?;
        let limits = StoreLimitsBuilder::new()
            .memory_size(MEMORY_LIMIT_MIB as usize * 1024 * 1024)
            .memories(4)
            .instances(16)
            .tables(16)
            .trap_on_grow_failure(false)
            .build();
        let mut store = Store::new(
            &engine,
            HostState {
                actor: String::new(),
                asked: std::ptr::null_mut(),
                stopped: false,
                limits,
                wasi: WasiCtx::builder().build(),
                table: ResourceTable::new(),
            },
        );
        store.limiter(|state| &mut state.limits);
        // Instantiation runs the guest runtime's own startup, so it gets the
        // generous budget.
        store
            .set_fuel(u64::from(CALL_BUDGET_MS) * FUEL_PER_MS * 20)
            .map_err(|e| e.to_string())?;
        let instance = linker
            .instantiate(&mut store, &component)
            .map_err(|e| format!("{relative} couldn't start: {e:#}"))?;
        let name = interface("entry");
        let entry = instance
            .get_export_index(&mut store, None, &name)
            .ok_or_else(|| format!("{relative} doesn't export {name}"))?;
        macro_rules! func {
            ($n:expr) => {
                export(&instance, &mut store, &entry, $n)
                    .ok_or_else(|| format!("{relative}: {name} has no usable `{}`", $n))?
            };
        }
        Ok(ComponentScript {
            relative: relative.to_string(),
            start: func!("start"),
            tick: func!("tick"),
            frame: func!("frame"),
            ui: func!("ui"),
            stop: func!("stop"),
            destroy: func!("destroy"),
            event: func!("on-event"),
            store: RefCell::new(store),
        })
    }

    pub fn load_file(
        project_dir: &std::path::Path,
        relative: &str,
    ) -> Result<ComponentScript, String> {
        let path = project_dir.join(relative);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::load_with(&bytes, relative, Some(project_dir))
    }

    pub fn is_stopped(&self) -> bool {
        self.store.borrow().data().stopped
    }

    fn budget_ms(hot: bool) -> u32 {
        if hot { TICK_BUDGET_MS } else { CALL_BUDGET_MS }
    }

    fn call(
        &self,
        actor: &str,
        asked: &mut Asked,
        hot: bool,
        event: Option<&ScriptEvent>,
        f: impl FnOnce(&mut Store<HostState>) -> wasmtime::Result<()>,
    ) {
        if self.is_stopped() {
            return;
        }
        let run = |store: &mut Store<HostState>| {
            store.set_fuel(u64::from(Self::budget_ms(hot)) * FUEL_PER_MS)?;
            store.data_mut().actor = actor.to_string();
            store.data_mut().asked = asked as *mut Asked;
            let result = f(store);
            store.data_mut().asked = std::ptr::null_mut();
            result
        };
        let mut result = Ok(());
        sense::with_actor(actor, || {
            if let Some(event) = event {
                super::script::with_event_words(event, || {
                    result = run(&mut self.store.borrow_mut());
                });
            } else {
                result = run(&mut self.store.borrow_mut());
            }
        });
        if let Err(error) = result {
            self.stop_with(actor, asked, hot, &error);
        }
    }

    fn stop_with(&self, actor: &str, asked: &mut Asked, hot: bool, error: &wasmtime::Error) {
        self.store.borrow_mut().data_mut().stopped = true;
        let reason = if error.downcast_ref::<Trap>() == Some(&Trap::OutOfFuel) {
            format!(
                "used more than its {} ms of work for one call and was stopped",
                Self::budget_ms(hot)
            )
        } else {
            format!("trapped: {error}")
        };
        asked.effects.push(Effect::Error {
            actor: actor.to_string(),
            message: format!("{}: {reason}. It won't run again this game.", self.relative),
        });
    }

    pub fn start(&self, actor: &str, asked: &mut Asked) {
        let f = self.start;
        self.call(actor, asked, false, None, |s| {
            f.call(s, ())?;
            Ok(())
        });
    }

    pub fn tick(&self, actor: &str, asked: &mut Asked, dt: f32) {
        let f = self.tick;
        self.call(actor, asked, true, None, |s| {
            f.call(s, (dt,))?;
            Ok(())
        });
    }

    pub fn frame(&self, actor: &str, asked: &mut Asked, dt: f32) {
        let f = self.frame;
        self.call(actor, asked, true, None, |s| {
            f.call(s, (dt,))?;
            Ok(())
        });
    }

    pub fn ui(&self, actor: &str, asked: &mut Asked, dt: f32) {
        let f = self.ui;
        self.call(actor, asked, true, None, |s| {
            f.call(s, (dt,))?;
            Ok(())
        });
    }

    pub fn stop(&self, actor: &str, asked: &mut Asked) {
        let f = self.stop;
        self.call(actor, asked, false, None, |s| {
            f.call(s, ())?;
            Ok(())
        });
    }

    pub fn destroy(&self, actor: &str, asked: &mut Asked) {
        let f = self.destroy;
        self.call(actor, asked, false, None, |s| {
            f.call(s, ())?;
            Ok(())
        });
    }

    pub fn event(&self, actor: &str, asked: &mut Asked, event: &ScriptEvent) {
        let [n0, n1, n2, n3] = event.numbers;
        let (f, kind) = (self.event, event.kind);
        self.call(actor, asked, false, Some(event), |s| {
            f.call(s, (kind, n0, n1, n2, n3))?;
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::scene::Axis;

    /// A component exporting `entry`: `start` says "hello from component"
    /// through the typed `acts.act` import, `tick` moves its actor by one,
    /// `on-event` says "event heard", and `spin` makes `frame` loop forever.
    fn fixture(spin: bool) -> String {
        let frame = if spin { "(loop $l (br $l))" } else { "" };
        format!(
            r#"(component
  (type $act (func (param "what" u32) (param "a" string) (param "b" string)
                   (param "c" string) (param "numbers" (list float64))))
  (import "blockloom:script/acts@0.2.0" (instance $acts (export "act" (func (type $act)))))
  (core module $libc (memory (export "memory") 1))
  (core instance $libc (instantiate $libc))
  (core func $act (canon lower (func $acts "act") (memory (core memory $libc "memory"))))
  (core module $guest
    (import "host" "act" (func $act (param i32 i32 i32 i32 i32 i32 i32 i32 i32)))
    (import "libc" "memory" (memory 1))
    (data (i32.const 16) "hello from component")
    (data (i32.const 48) "event heard")
    (data (i32.const 64) "\00\00\00\00\00\00\00\00\00\00\00\00\00\00\f0\3f")
    (func (export "start")
      (call $act (i32.const {say}) (i32.const 16) (i32.const 20)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)))
    (func (export "tick") (param f32)
      (call $act (i32.const {mv}) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 64) (i32.const 2)))
    (func (export "frame") (param f32) {frame})
    (func (export "ui") (param f32))
    (func (export "stop"))
    (func (export "destroy"))
    (func (export "event") (param i32 f64 f64 f64 f64)
      (call $act (i32.const {say}) (i32.const 48) (i32.const 11)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)))
  )
  (core instance $guest (instantiate $guest
    (with "host" (instance (export "act" (func $act))))
    (with "libc" (instance $libc))))
  (func $start (canon lift (core func $guest "start")))
  (func $tick (param "dt" float32) (canon lift (core func $guest "tick")))
  (func $frame (param "dt" float32) (canon lift (core func $guest "frame")))
  (func $ui (param "dt" float32) (canon lift (core func $guest "ui")))
  (func $stop (canon lift (core func $guest "stop")))
  (func $destroy (canon lift (core func $guest "destroy")))
  (func $event (param "kind" u32) (param "n0" float64) (param "n1" float64)
    (param "n2" float64) (param "n3" float64) (canon lift (core func $guest "event")))
  (instance $entry
    (export "start" (func $start)) (export "tick" (func $tick))
    (export "frame" (func $frame)) (export "ui" (func $ui))
    (export "stop" (func $stop)) (export "destroy" (func $destroy))
    (export "on-event" (func $event)))
  (export "blockloom:script/entry@0.2.0" (instance $entry))
)"#,
            say = abi::ACT_SAY,
            mv = abi::ACT_CHANGE_POSITION,
        )
    }

    fn load(spin: bool) -> ComponentScript {
        let bytes = wat::parse_str(fixture(spin)).expect("the fixture is valid");
        assert!(is_component(&bytes));
        ComponentScript::load_bytes(&bytes, "assets/scripts/test.wasm").expect("loads")
    }

    fn publish_one(id: &str) {
        crate::script_wasm::tests::publish_one(id);
    }

    #[test]
    fn a_component_drives_its_actor_through_the_typed_imports() {
        publish_one("a1");
        let script = load(false);
        let mut asked = Asked::default();
        script.start("a1", &mut asked);
        script.tick("a1", &mut asked, 1.0 / 60.0);
        script.event(
            "a1",
            &mut asked,
            &ScriptEvent::new(abi::EVENT_MESSAGE, "hi"),
        );
        assert!(asked.effects.iter().any(|e| matches!(
            e, Effect::Say { actor, text } if actor == "a1" && text == "hello from component")));
        assert!(asked.effects.iter().any(|e| matches!(
            e, Effect::ChangePosition { actor, axis: Axis::X, by }
            if actor == "a1" && (*by - 1.0).abs() < f32::EPSILON)));
        assert!(asked.effects.iter().any(|e| matches!(
            e, Effect::Say { text, .. } if text == "event heard")));
        assert!(!script.is_stopped());
    }

    #[test]
    fn a_runaway_component_is_stopped_not_the_run() {
        publish_one("a1");
        let script = load(true);
        let mut asked = Asked::default();
        script.frame("a1", &mut asked, 1.0 / 60.0);
        assert!(script.is_stopped());
        assert!(asked.effects.iter().any(|e| matches!(
            e, Effect::Error { message, .. } if message.contains("won't run again"))));
        // A stopped script is left alone.
        let mut asked = Asked::default();
        script.tick("a1", &mut asked, 1.0 / 60.0);
        assert!(asked.effects.is_empty());
    }

    #[test]
    fn a_core_module_is_not_a_component() {
        let module = wat::parse_str("(module)").unwrap();
        assert!(!is_component(&module));
        assert!(ComponentScript::load_bytes(&module, "x.wasm").is_err());
    }

    /// `BLOCKLOOM_COMPONENT=<file.wasm>` names a guest built against the
    /// frozen world with its language's toolchain (`templates/minimal.py`
    /// via componentize-py, `templates/minimal.mjs` via jco). Prints load and
    /// per-tick cost.
    #[test]
    #[ignore]
    fn a_component_guest_runs_and_costs_what_it_costs() {
        let Ok(path) = std::env::var("BLOCKLOOM_COMPONENT") else {
            return;
        };
        publish_one("a1");
        let started = std::time::Instant::now();
        let script = ComponentScript::load_file(
            &std::env::temp_dir().join("blockloom-component-test"),
            &path,
        )
        .expect("loads");
        eprintln!("component load: {:?}", started.elapsed());
        let mut asked = Asked::default();
        script.start("a1", &mut asked);
        assert!(
            !script.is_stopped(),
            "{:?}",
            asked
                .effects
                .iter()
                .map(|e| format!("{e:?}"))
                .collect::<Vec<_>>()
        );
        let started = std::time::Instant::now();
        for _ in 0..1000 {
            script.tick("a1", &mut asked, 1.0 / 60.0);
        }
        eprintln!("component tick: {:?} each", started.elapsed() / 1000);
        assert!(!script.is_stopped());
    }
}
