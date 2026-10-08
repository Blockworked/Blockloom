//! Running a script as WebAssembly on the desktop, beside the native path.
//!
//! The module is a core wasm binary (no component model, no WASI) importing
//! only the same three host calls a web script imports
//! (`blockloom.read_number/read_text/act` over [`abi::WasmCall`]) and
//! exporting its memory plus the `blockloom_script_*` entry points. Any
//! language that can emit that shape - Rust through
//! `compile_for(..., WEB_TARGET)`, or a TinyGo/componentize toolchain over
//! the frozen [`blockloom_core::script::wit`] world - runs here sandboxed:
//! its own linear memory with a ceiling, no imports but the three calls, and
//! a fuel budget per entry point. A call that runs out of fuel or traps stops
//! the script for the rest of the run, exactly like a trapped web script,
//! while native scripts keep their speed.
//!
//! Play builds the wasm beside the native library when this machine has the
//! wasm target installed, and the engine loads native first with wasm as the
//! fallback (`ScriptBackend`), or wasm first under
//! `BLOCKLOOM_SCRIPT_BACKEND=wasm`.

use super::script::{Asked, ScriptEvent};
use blockloom_core::script::abi;
use blockloom_core::sense;
use blockloom_core::vm::Effect;
use blockloom_plugin_api::wasm::FUEL_PER_MS;
use std::cell::RefCell;
use wasmi::{
    Caller, Config, Engine, Extern, Linker, Memory, Module, Store, StoreLimitsBuilder, TrapCode,
    TypedFunc,
};

/// Fuel budget for a hot entry point (`tick`/`frame`/`ui`): ten milliseconds
/// of work, after which the call stops rather than the frame.
pub const TICK_BUDGET_MS: u32 = 10;
/// Budget for a rare entry point (`start`/`event`/`stop`/`destroy`), matching
/// the plugin host's per-call default.
pub const CALL_BUDGET_MS: u32 = 50;
/// Linear-memory ceiling, matching the plugin host's default.
pub const MEMORY_LIMIT_MIB: u32 = 64;

/// Whether `bytes` look like a wasm module rather than a native library.
pub fn is_wasm(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\0asm")
}

struct HostState {
    /// Who is running while a guest call is in progress.
    actor: String,
    /// Where effects land. Set for the entry call in progress and null
    /// otherwise; host functions run only during guest execution on this
    /// thread, so the pointer is live whenever they can see it.
    asked: *mut Asked,
    stopped: bool,
    stop_reason: String,
    /// The last line the guest logged, which is where its panic hook puts the
    /// panic's message before the trap.
    last_log: String,
    limits: wasmi::StoreLimits,
}

fn guest_memory(caller: &Caller<'_, HostState>) -> Option<Memory> {
    match caller.get_export("memory") {
        Some(Extern::Memory(memory)) => Some(memory),
        _ => None,
    }
}

fn read_guest(caller: &Caller<'_, HostState>, ptr: u32, len: u32) -> Option<Vec<u8>> {
    if len == 0 {
        return Some(Vec::new());
    }
    let memory = guest_memory(caller)?;
    let mut bytes = vec![0u8; len as usize];
    memory.read(caller, ptr as usize, &mut bytes).ok()?;
    Some(bytes)
}

fn read_text(caller: &Caller<'_, HostState>, ptr: u32, len: u32) -> String {
    String::from_utf8(read_guest(caller, ptr, len).unwrap_or_default()).unwrap_or_default()
}

/// The `abi::WasmCall` at `at` in the guest's memory.
fn read_call(caller: &Caller<'_, HostState>, at: u32) -> Option<abi::WasmCall> {
    let bytes = read_guest(caller, at, std::mem::size_of::<abi::WasmCall>() as u32)?;
    if bytes.len() < std::mem::size_of::<abi::WasmCall>() {
        return None;
    }
    // Safety: every field is a plain number, so any bytes are one.
    Some(unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<abi::WasmCall>()) })
}

fn write_guest(caller: &mut Caller<'_, HostState>, at: u32, bytes: &[u8]) {
    if bytes.is_empty() || at == 0 {
        return;
    }
    let Some(memory) = guest_memory(caller) else {
        return;
    };
    let _ = memory.write(caller, at as usize, bytes);
}

fn host_read_number(mut caller: Caller<'_, HostState>, ctx: i32, what: i32, at: i32) -> i32 {
    let _ = ctx;
    let Some(call) = read_call(&caller, at as u32) else {
        return abi::MISSING as i32;
    };
    let a = read_text(&caller, call.a_ptr, call.a_len);
    let b = read_text(&caller, call.b_ptr, call.b_len);
    let actor = caller.data().actor.clone();
    match super::script::number_for(&actor, what as u32, &a, &b, call.arg) {
        Some(value) => {
            write_guest(&mut caller, call.out, &value.to_le_bytes());
            abi::OK as i32
        }
        None => abi::MISSING as i32,
    }
}

fn host_read_text(mut caller: Caller<'_, HostState>, ctx: i32, what: i32, at: i32) -> i32 {
    let _ = ctx;
    let Some(call) = read_call(&caller, at as u32) else {
        return abi::MISSING as i32;
    };
    let a = read_text(&caller, call.a_ptr, call.a_len);
    let b = read_text(&caller, call.b_ptr, call.b_len);
    let actor = caller.data().actor.clone();
    let answer: Option<Vec<u8>> = if what as u32 == abi::BYTES_POSE {
        super::script::pose_bytes_for(&actor, &a).map(|bytes| bytes.to_vec())
    } else {
        super::script::text_for(&actor, what as u32, &a, &b).map(|text| text.into_bytes())
    };
    let Some(answer) = answer else {
        return abi::MISSING as i32;
    };
    write_guest(
        &mut caller,
        call.out_len,
        &(answer.len() as u32).to_le_bytes(),
    );
    if answer.len() > call.out_cap as usize {
        return abi::TOO_LONG as i32;
    }
    write_guest(&mut caller, call.out, &answer);
    abi::OK as i32
}

fn host_act(mut caller: Caller<'_, HostState>, ctx: i32, what: i32, at: i32) -> i32 {
    let _ = ctx;
    let Some(call) = read_call(&caller, at as u32) else {
        return abi::MISSING as i32;
    };
    let a = read_text(&caller, call.a_ptr, call.a_len);
    let b = read_text(&caller, call.b_ptr, call.b_len);
    let c = read_text(&caller, call.c_ptr, call.c_len);
    let raw = read_guest(&caller, call.numbers, call.count.saturating_mul(8)).unwrap_or_default();
    let numbers: Vec<f64> = raw
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| f64::from_le_bytes(*chunk))
        .collect();
    if what as u32 == abi::ACT_LOG {
        caller.data_mut().last_log = a.clone();
    }
    let actor = caller.data().actor.clone();
    // Safety: `asked` is set for the entry call in progress and this host
    // function runs only inside it, on this thread.
    let asked = caller.data_mut().asked;
    let asked = unsafe { &mut *asked };
    super::script::act_for_asked(&actor, asked, what as u32, &a, &b, &c, &numbers);
    abi::OK as i32
}

/// One actor's script as a sandboxed wasm module. Not thread-safe by itself:
/// the engine holds it on the main thread, as with the VM.
pub struct WasmScript {
    relative: String,
    store: RefCell<Store<HostState>>,
    memory: Memory,
    start: TypedFunc<(i32, i32), ()>,
    tick: TypedFunc<(i32, i32, f32), ()>,
    frame: Option<TypedFunc<(i32, i32, f32), ()>>,
    ui: Option<TypedFunc<(i32, i32, f32), ()>>,
    stop: Option<TypedFunc<(i32, i32), ()>>,
    destroy: Option<TypedFunc<(i32, i32), ()>>,
    event: TypedFunc<(i32, i32, i32, f64, f64, f64, f64), ()>,
}

impl WasmScript {
    /// Compiles and starts `bytes` as `relative`'s script. Refuses anything
    /// but a core module importing only the three host calls, so no WASI or
    /// component-model imports reach the tick path.
    pub fn load_bytes(bytes: &[u8], relative: &str) -> Result<WasmScript, String> {
        if !is_wasm(bytes) {
            return Err(format!("{relative} isn't a WebAssembly module"));
        }
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, bytes).map_err(|e| format!("{relative}: {e}"))?;
        for import in module.imports() {
            let (module_name, name) = (import.module(), import.name());
            let allowed = module_name == abi::WASM_MODULE
                && [abi::WASM_READ_NUMBER, abi::WASM_READ_TEXT, abi::WASM_ACT].contains(&name);
            if !allowed {
                return Err(format!(
                    "{relative} imports {module_name}.{name}, but scripts may only import the three blockloom host calls"
                ));
            }
        }
        let limits = StoreLimitsBuilder::new()
            .memory_size(MEMORY_LIMIT_MIB as usize * 1024 * 1024)
            .memories(1)
            .instances(1)
            .tables(8)
            .trap_on_grow_failure(false)
            .build();
        let mut store = Store::new(
            &engine,
            HostState {
                actor: String::new(),
                asked: std::ptr::null_mut(),
                stopped: false,
                stop_reason: String::new(),
                last_log: String::new(),
                limits,
            },
        );
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(u64::from(CALL_BUDGET_MS) * FUEL_PER_MS)
            .map_err(|e| e.to_string())?;
        let mut linker = <Linker<HostState>>::new(&engine);
        linker
            .func_wrap(abi::WASM_MODULE, abi::WASM_READ_NUMBER, host_read_number)
            .map_err(|e| e.to_string())?;
        linker
            .func_wrap(abi::WASM_MODULE, abi::WASM_READ_TEXT, host_read_text)
            .map_err(|e| e.to_string())?;
        linker
            .func_wrap(abi::WASM_MODULE, abi::WASM_ACT, host_act)
            .map_err(|e| e.to_string())?;
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| format!("{relative} couldn't start: {e}"))?;
        let export = |name: &str| format!("{relative} doesn't export {name}");
        let version = instance
            .get_typed_func::<(), i32>(&store, "blockloom_script_abi")
            .map_err(|_| format!("{relative} isn't a Blockloom script"))?
            .call(&mut store, ())
            .map_err(|e| e.to_string())?;
        if version as u32 != abi::ABI_VERSION {
            return Err(format!(
                "{relative} was built against script ABI {version}, this runtime speaks {}. Build the game again.",
                abi::ABI_VERSION
            ));
        }
        let start = instance
            .get_typed_func(&store, "blockloom_script_start")
            .map_err(|_| export("blockloom_script_start"))?;
        let tick = instance
            .get_typed_func(&store, "blockloom_script_tick")
            .map_err(|_| export("blockloom_script_tick"))?;
        let event = instance
            .get_typed_func(&store, "blockloom_script_event")
            .map_err(|_| export("blockloom_script_event"))?;
        let optional_frame = |name: &str| instance.get_typed_func(&store, name).ok();
        let optional_lifecycle = |name: &str| instance.get_typed_func(&store, name).ok();
        let frame: Option<TypedFunc<(i32, i32, f32), ()>> =
            optional_frame("blockloom_script_frame");
        let ui: Option<TypedFunc<(i32, i32, f32), ()>> = optional_frame("blockloom_script_ui");
        let stop: Option<TypedFunc<(i32, i32), ()>> = optional_lifecycle("blockloom_script_stop");
        let destroy: Option<TypedFunc<(i32, i32), ()>> =
            optional_lifecycle("blockloom_script_destroy");
        let memory = instance
            .get_memory(&store, "memory")
            .ok_or_else(|| format!("{relative} exports no memory"))?;
        Ok(WasmScript {
            relative: relative.to_string(),
            store: RefCell::new(store),
            memory,
            start,
            tick,
            frame,
            ui,
            stop,
            destroy,
            event,
        })
    }

    /// Loads the wasm build of `relative` the editor produced with
    /// `compile_for(..., WEB_TARGET)`. Play builds it alongside the native
    /// library; this is the seam that wiring reads through.
    pub fn load_file(project_dir: &std::path::Path, relative: &str) -> Result<WasmScript, String> {
        let path = blockloom_core::script::library_path_for(
            project_dir,
            relative,
            Some(blockloom_core::script::WEB_TARGET),
        );
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::load_bytes(&bytes, relative)
    }

    /// Whether this script already trapped or ran out of fuel. Its memory may
    /// be half-updated, so the engine drops it rather than calling back in.
    pub fn is_stopped(&self) -> bool {
        self.store.borrow().data().stopped
    }

    fn budget_ms(hot: bool) -> u32 {
        if hot { TICK_BUDGET_MS } else { CALL_BUDGET_MS }
    }

    /// Runs one entry point with `actor`'s sensing scope and event words, the
    /// way a native script runs. A trap or an empty fuel tank stops the
    /// script with an error effect instead of taking the run down.
    fn call(
        &self,
        actor: &str,
        asked: &mut Asked,
        hot: bool,
        event: Option<&ScriptEvent>,
        f: impl FnOnce(&mut Store<HostState>) -> Result<(), wasmi::Error>,
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
        let mut result: Result<(), wasmi::Error> = Ok(());
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
            self.stop_with(actor, asked, hot, error);
        }
        let _ = &self.memory;
    }

    fn stop_with(&self, actor: &str, asked: &mut Asked, hot: bool, error: wasmi::Error) {
        let mut store = self.store.borrow_mut();
        store.data_mut().stopped = true;
        let reason = match error.as_trap_code() {
            Some(TrapCode::OutOfFuel) => format!(
                "used more than its {} ms of work for one call and was stopped",
                Self::budget_ms(hot)
            ),
            _ => {
                let logged = store.data().last_log.clone();
                if logged.starts_with("the script panicked") {
                    logged
                } else {
                    format!("trapped: {error}")
                }
            }
        };
        store.data_mut().stop_reason = reason.clone();
        asked.effects.push(Effect::Error {
            actor: actor.to_string(),
            message: format!("{}: {reason}. It won't run again this game.", self.relative),
        });
    }

    pub fn start(&self, actor: &str, asked: &mut Asked) {
        let start = self.start;
        self.call(actor, asked, false, None, |store| start.call(store, (0, 0)));
    }

    pub fn tick(&self, actor: &str, asked: &mut Asked, dt: f32) {
        let tick = self.tick;
        self.call(actor, asked, true, None, |store| {
            tick.call(store, (0, 0, dt))
        });
    }

    /// Once per rendered frame while unpaused. A module without a `frame`
    /// export does nothing here.
    pub fn frame(&self, actor: &str, asked: &mut Asked, dt: f32) {
        let Some(frame) = self.frame else {
            return;
        };
        self.call(actor, asked, true, None, |store| {
            frame.call(store, (0, 0, dt))
        });
    }

    /// Once per rendered frame even while paused. A module without a `ui`
    /// export does nothing here.
    pub fn ui(&self, actor: &str, asked: &mut Asked, dt: f32) {
        let Some(ui) = self.ui else {
            return;
        };
        self.call(actor, asked, true, None, |store| ui.call(store, (0, 0, dt)));
    }

    /// Once when the run ends. A module without a `stop` export does nothing.
    pub fn stop(&self, actor: &str, asked: &mut Asked) {
        let Some(stop) = self.stop else {
            return;
        };
        self.call(actor, asked, false, None, |store| stop.call(store, (0, 0)));
    }

    /// Once when this actor is deleted mid-run. A module without a `destroy`
    /// export does nothing.
    pub fn destroy(&self, actor: &str, asked: &mut Asked) {
        let Some(destroy) = self.destroy else {
            return;
        };
        self.call(actor, asked, false, None, |store| {
            destroy.call(store, (0, 0))
        });
    }

    pub fn event(&self, actor: &str, asked: &mut Asked, event: &ScriptEvent) {
        let [n0, n1, n2, n3] = event.numbers;
        let kind = event.kind;
        let event_func = self.event;
        self.call(actor, asked, false, Some(event), |store| {
            event_func.call(store, (0, 0, kind as i32, n0, n1, n2, n3))
        });
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use blockloom_core::scene::Axis;
    use blockloom_core::sense::{ActorSense, Sensors};

    /// Publishes one actor the module can read, as the runtime would.
    pub(crate) fn publish_one(actor: &str) {
        let mut sensors = Sensors::default();
        sensors.actors.insert(
            actor.to_string(),
            ActorSense {
                name: "Player".to_string(),
                position: [3.0, 7.0, 0.0],
                ..Default::default()
            },
        );
        sense::publish(sensors);
    }

    /// A non-Rust script's shape: hand-written WAT standing in for whatever a
    /// TinyGo/componentize toolchain emits - the imports, memory and entry
    /// points are the contract, not the source language. `start` says hello,
    /// `tick` reads its x and steps +1 along it, `event` answers.
    fn fixture_wat() -> String {
        format!(
            r#"
(module
  (import "blockloom" "read_number" (func $rn (param i32 i32 i32) (result i32)))
  (import "blockloom" "read_text" (func $rt (param i32 i32 i32) (result i32)))
  (import "blockloom" "act" (func $act (param i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 2048) "hello from wasm")
  (data (i32.const 2064) "event heard")

  (func (export "blockloom_script_abi") (result i32) (i32.const {abi}))

  ;; Zeroes the 56-byte call struct at 1024.
  (func $clear
    (i32.store (i32.const 1024) (i32.const 0))
    (i32.store (i32.const 1028) (i32.const 0))
    (i32.store (i32.const 1032) (i32.const 0))
    (i32.store (i32.const 1036) (i32.const 0))
    (i32.store (i32.const 1040) (i32.const 0))
    (i32.store (i32.const 1044) (i32.const 0))
    (i32.store (i32.const 1048) (i32.const 0))
    (i32.store (i32.const 1052) (i32.const 0))
    (i32.store (i32.const 1056) (i32.const 0))
    (i32.store (i32.const 1060) (i32.const 0))
    (i32.store (i32.const 1064) (i32.const 0))
    (i32.store (i32.const 1068) (i32.const 0))
    (i32.store (i32.const 1072) (i32.const 0))
    (i32.store (i32.const 1076) (i32.const 0)))

  (func $say (param $ptr i32) (param $len i32)
    (call $clear)
    (i32.store (i32.const 1024) (local.get $ptr))
    (i32.store (i32.const 1028) (local.get $len))
    (drop (call $act (i32.const 0) (i32.const {say}) (i32.const 1024))))

  (func (export "blockloom_script_start") (param i32 i32)
    (call $say (i32.const 2048) (i32.const 15)))

  (func (export "blockloom_script_tick") (param i32 i32 f32)
    (call $clear)
    ;; Read this actor's x (axis 0) into the f64 at 4096.
    (f64.store (i32.const 1056) (f64.const 0))
    (i32.store (i32.const 1064) (i32.const 4096))
    (i32.store (i32.const 1068) (i32.const 8))
    (drop (call $rn (i32.const 0) (i32.const {pos}) (i32.const 1024)))
    ;; Step +1 along x: numbers [axis 0, amount 1] at 4112.
    (call $clear)
    (f64.store (i32.const 4112) (f64.const 0))
    (f64.store (i32.const 4120) (f64.const 1))
    (i32.store (i32.const 1048) (i32.const 4112))
    (i32.store (i32.const 1052) (i32.const 2))
    (drop (call $act (i32.const 0) (i32.const {mv}) (i32.const 1024))))

  (func (export "blockloom_script_event") (param i32 i32 i32 f64 f64 f64 f64)
    (call $say (i32.const 2064) (i32.const 11)))

  (func (export "blockloom_script_frame") (param i32 i32 f32))
  (func (export "blockloom_script_ui") (param i32 i32 f32))
  (func (export "blockloom_script_stop") (param i32 i32)
    (call $say (i32.const 2064) (i32.const 11)))
  (func (export "blockloom_script_destroy") (param i32 i32)
    (call $say (i32.const 2048) (i32.const 15)))
)
"#,
            abi = abi::ABI_VERSION,
            say = abi::ACT_SAY,
            pos = abi::READ_POSITION,
            mv = abi::ACT_CHANGE_POSITION,
        )
    }

    fn load_fixture() -> WasmScript {
        let bytes = wat::parse_str(fixture_wat()).expect("the fixture is valid");
        WasmScript::load_bytes(&bytes, "assets/scripts/test.wasm").expect("loads")
    }

    #[test]
    fn wasm_start_tick_and_event_drive_the_actor() {
        publish_one("a1");
        let script = load_fixture();
        assert!(!script.is_stopped());
        let mut asked = Asked::default();
        script.start("a1", &mut asked);
        assert!(
            asked.effects.iter().any(|effect| matches!(
                effect,
                Effect::Say { actor, text }
                if actor == "a1" && text == "hello from wasm"
            )),
            "{:?}",
            asked.effects
        );
        let mut asked = Asked::default();
        script.tick("a1", &mut asked, 1.0 / 60.0);
        assert!(
            asked.effects.iter().any(|effect| matches!(
                effect,
                Effect::ChangePosition { actor, axis: Axis::X, by }
                if actor == "a1" && (*by - 1.0).abs() < f32::EPSILON
            )),
            "{:?}",
            asked.effects
        );
        let mut asked = Asked::default();
        script.event(
            "a1",
            &mut asked,
            &ScriptEvent::new(abi::EVENT_MESSAGE, "hi"),
        );
        assert!(
            asked.effects.iter().any(|effect| matches!(
                effect,
                Effect::Say { text, .. } if text == "event heard"
            )),
            "{:?}",
            asked.effects
        );
        assert!(!script.is_stopped());
    }

    #[test]
    fn wasm_optional_entries_run_where_present() {
        publish_one("a1");
        let script = load_fixture();
        let mut asked = Asked::default();
        // Present but quiet: no effects, no stop.
        script.frame("a1", &mut asked, 1.0 / 60.0);
        script.ui("a1", &mut asked, 1.0 / 60.0);
        assert!(asked.effects.is_empty());
        script.stop("a1", &mut asked);
        script.destroy("a1", &mut asked);
        let said: Vec<_> = asked
            .effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Say { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(said, vec!["event heard", "hello from wasm"]);
        assert!(!script.is_stopped());
    }

    #[test]
    fn wasm_loads_the_web_build_from_the_project_folder() {
        let dir = std::env::temp_dir().join(format!("blockloom-wasm-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let relative = "assets/scripts/test.rs";
        let path = blockloom_core::script::library_path_for(
            &dir,
            relative,
            Some(blockloom_core::script::WEB_TARGET),
        );
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, wat::parse_str(fixture_wat()).unwrap()).unwrap();
        let script = WasmScript::load_file(&dir, relative).expect("loads the web build");
        publish_one("a1");
        let mut asked = Asked::default();
        script.start("a1", &mut asked);
        assert!(asked.effects.iter().any(|effect| matches!(
            effect,
            Effect::Say { text, .. } if text == "hello from wasm"
        )));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wasm_refuses_foreign_imports_and_missing_exports() {
        // Anything but the three host calls is refused, so no WASI.
        let wasi = wat::parse_str(
            r#"(module (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32))))"#,
        )
        .unwrap();
        let error = WasmScript::load_bytes(&wasi, "wasi.wasm")
            .err()
            .expect("refused");
        assert!(error.contains("only import the three"), "{error}");
        assert!(
            WasmScript::load_bytes(b"not wasm", "x.wasm")
                .err()
                .expect("refused")
                .contains("isn't a WebAssembly")
        );
        // No entry points, no script.
        let bare = wat::parse_str(format!(
            r#"(module (memory (export "memory") 1) (func (export "blockloom_script_abi") (result i32) i32.const {}))"#,
            abi::ABI_VERSION
        ))
        .unwrap();
        assert!(
            WasmScript::load_bytes(&bare, "bare.wasm")
                .err()
                .expect("refused")
                .contains("doesn't export")
        );
        // A wrong ABI is refused like a native library's.
        let wrong = wat::parse_str(
            r#"(module (memory (export "memory") 1) (func (export "blockloom_script_abi") (result i32) i32.const 1))"#,
        )
        .unwrap();
        assert!(
            WasmScript::load_bytes(&wrong, "old.wasm")
                .err()
                .expect("refused")
                .contains("script ABI 1")
        );
    }

    #[test]
    fn wasm_runaway_is_stopped_with_a_budget_error() {
        let spinning = wat::parse_str(format!(
            r#"(module
  (import "blockloom" "read_number" (func $rn (param i32 i32 i32) (result i32)))
  (import "blockloom" "read_text" (func $rt (param i32 i32 i32) (result i32)))
  (import "blockloom" "act" (func $act (param i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "blockloom_script_abi") (result i32) i32.const {})
  (func (export "blockloom_script_start") (param i32 i32))
  (func (export "blockloom_script_tick") (param i32 i32 f32) (loop $s (br $s)))
  (func (export "blockloom_script_event") (param i32 i32 i32 f64 f64 f64 f64))
)"#,
            abi::ABI_VERSION
        ))
        .unwrap();
        publish_one("a1");
        let script = WasmScript::load_bytes(&spinning, "spin.wasm").expect("loads");
        let mut asked = Asked::default();
        script.tick("a1", &mut asked, 1.0 / 60.0);
        assert!(script.is_stopped());
        let errors: Vec<_> = asked
            .effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Error { message, .. } => Some(message.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("10 ms of work"), "{}", errors[0]);
        // Stopped means stopped: no second error, no effects.
        let mut asked = Asked::default();
        script.tick("a1", &mut asked, 1.0 / 60.0);
        assert!(asked.effects.is_empty());
    }

    /// What every `minimal` guest (Rust, C) must produce: hello and the
    /// slot/language switches on `start`, a +1 step on `tick`, an answer to
    /// an `event`, and a script that is still running afterwards.
    fn assert_minimal_guest(script: &WasmScript) {
        publish_one("a1");
        let variables = blockloom_core::vm::Variables::default();
        let lists = blockloom_core::vm::Lists::default();
        crate::script::with_script_stores(&variables, &lists, || {
            let mut asked = Asked::default();
            script.start("a1", &mut asked);
            assert!(
                asked.effects.iter().any(|effect| matches!(
                    effect,
                    Effect::Say { actor, text }
                    if actor == "a1" && text == "hello from wasm"
                )),
                "{:?}",
                asked.effects
            );
            assert!(
                asked.effects.iter().any(|effect| matches!(
                    effect,
                    Effect::SwitchSaveSlot { actor, slot }
                    if actor == "a1" && slot == "Slot 2"
                )),
                "{:?}",
                asked.effects
            );
            assert!(
                asked.effects.iter().any(|effect| matches!(
                    effect,
                    Effect::SetLanguage { actor, language }
                    if actor == "a1" && language == "fr"
                )),
                "{:?}",
                asked.effects
            );
            let mut asked = Asked::default();
            script.tick("a1", &mut asked, 1.0 / 60.0);
            assert!(
                asked.effects.iter().any(|effect| matches!(
                    effect,
                    Effect::ChangePosition { actor, axis: Axis::X, by }
                    if actor == "a1" && (*by - 1.0).abs() < f32::EPSILON
                )),
                "{:?}",
                asked.effects
            );
            let mut asked = Asked::default();
            script.event(
                "a1",
                &mut asked,
                &ScriptEvent::new(abi::EVENT_MESSAGE, "hi"),
            );
            assert!(
                asked.effects.iter().any(|effect| matches!(
                    effect,
                    Effect::Say { text, .. } if text == "event heard"
                )),
                "{:?}",
                asked.effects
            );
            assert!(!script.is_stopped());
        });
    }

    /// The C guest as a sandboxed module: one `clang --target=wasm32` run over
    /// the header binding, then the same assertions as the Rust guest. Skipped
    /// without clang and wasm-ld.
    #[test]
    fn c_guest_script_runs_in_sandbox() {
        let Some(bytes) = build_c_guest() else {
            return;
        };
        let script = WasmScript::load_bytes(&bytes, "minimal.wasm").expect("loads");
        assert_minimal_guest(&script);
    }

    /// Builds `templates/minimal.c`; `None` when this machine has no wasm clang.
    fn build_c_guest() -> Option<Vec<u8>> {
        let guest =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../blockloom-script-guest");
        let dir = std::env::temp_dir().join(format!("blockloom-c-guest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let module = dir.join("minimal.wasm");
        let output = std::process::Command::new("clang")
            .args(["--target=wasm32", "-O2", "-nostdlib"])
            .args(["-Wl,--no-entry", "-Wl,--export-memory"])
            .arg("-I")
            .arg(guest.join("c"))
            .arg("-o")
            .arg(&module)
            .arg(guest.join("templates/minimal.c"))
            .output()
            .ok()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // No wasm backend or linker here: nothing to test, not a failure.
            if stderr.contains("unable to execute") || stderr.contains("wasm-ld") {
                return None;
            }
            panic!("the C guest builds: {stderr}");
        }
        let bytes = std::fs::read(&module).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        Some(bytes)
    }

    /// The guest crate's template as a sandboxed module: two `rustc` runs (the
    /// guest `rlib` for the wasm target, then the template as a `cdylib` over
    /// it - the documented `cargo build --target wasm32-unknown-unknown`
    /// without the cargo), then the same start/tick/event assertions the
    /// hand-written fixture holds. Skipped without a toolchain and target.
    #[test]
    fn guest_crate_script_runs_in_sandbox() {
        use blockloom_core::script::{
            WEB_TARGET, rustc_command, target_installed, toolchain_version,
        };
        if toolchain_version().is_err() || target_installed(WEB_TARGET).is_err() {
            return;
        }
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let guest = manifest.join("../blockloom-script-guest");
        let dir = std::env::temp_dir().join(format!("blockloom-guest-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let rlib = dir.join("libblockloom_script_guest.rlib");
        let status = rustc_command()
            .arg("--edition")
            .arg("2024")
            .arg("--crate-type")
            .arg("rlib")
            .arg("--crate-name")
            .arg("blockloom_script_guest")
            .arg("--target")
            .arg(WEB_TARGET)
            .arg("-C")
            .arg("opt-level=2")
            .arg("-o")
            .arg(&rlib)
            .arg(guest.join("src/lib.rs"))
            .status()
            .expect("rustc runs");
        assert!(status.success(), "the guest crate builds for {WEB_TARGET}");
        let module = dir.join("minimal.wasm");
        let output = rustc_command()
            .arg("--edition")
            .arg("2024")
            .arg("--crate-type")
            .arg("cdylib")
            .arg("--crate-name")
            .arg("minimal")
            .arg("--target")
            .arg(WEB_TARGET)
            .arg("--extern")
            .arg(format!("blockloom_script_guest={}", rlib.display()))
            .arg("-C")
            .arg("opt-level=2")
            .arg("-o")
            .arg(&module)
            .arg(guest.join("templates/minimal.rs"))
            .output()
            .expect("rustc runs");
        assert!(
            output.status.success(),
            "the guest template builds: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let bytes = std::fs::read(&module).unwrap();
        let script = WasmScript::load_bytes(&bytes, "minimal.wasm").expect("loads");
        assert_minimal_guest(&script);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `cargo test -p blockloom-runtime --lib script_wasm -- --ignored --nocapture wasm_tick_cost`
    /// prints per-tick costs for a wasm script next to a native one (when this
    /// machine has a toolchain), which is Phase 5's gate number.
    #[test]
    #[ignore = "a measurement, not a check"]
    fn wasm_tick_cost() {
        use std::time::Instant;
        publish_one("a1");
        let script = load_fixture();
        // Warm the interpreter before timing it.
        let mut asked = Asked::default();
        for _ in 0..100 {
            script.tick("a1", &mut asked, 1.0 / 60.0);
        }
        let calls = 5_000u32;
        let start = Instant::now();
        for _ in 0..calls {
            script.tick("a1", &mut asked, 1.0 / 60.0);
        }
        let per_call = start.elapsed().as_nanos() as f64 / f64::from(calls);
        println!("script wasm: {per_call:.0} ns per tick (read + act)");
        if let Some(bytes) = build_c_guest() {
            let c_script = WasmScript::load_bytes(&bytes, "minimal.wasm").expect("loads");
            let mut asked = Asked::default();
            for _ in 0..100 {
                c_script.tick("a1", &mut asked, 1.0 / 60.0);
                asked.effects.clear();
            }
            let start = Instant::now();
            for _ in 0..calls {
                c_script.tick("a1", &mut asked, 1.0 / 60.0);
                asked.effects.clear();
            }
            let per_call = start.elapsed().as_nanos() as f64 / f64::from(calls);
            println!("script wasm (C guest): {per_call:.0} ns per tick (read + 2 sets + act)");
        }
        if blockloom_core::script::toolchain_version().is_err() {
            println!("script native: no toolchain, skipping");
            return;
        }
        let dir = std::env::temp_dir().join(format!("blockloom-wasm-cost-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let relative = "assets/scripts/test.rs";
        std::fs::create_dir_all(dir.join("assets/scripts")).unwrap();
        std::fs::write(
            dir.join(relative),
            "use blockloom::*;\nfn tick(me: &Actor, _dt: f32) {\n    let _ = me.position(Axis::X);\n    me.change_position(Axis::X, 1.0);\n}\nblockloom::export!(tick = tick);\n",
        )
        .unwrap();
        let built = blockloom_core::script::compile(&dir, relative).expect("compiles");
        assert!(built.is_file());
        let native = crate::script::LoadedScript::load(&dir, relative).expect("loads");
        let mut asked = Asked::default();
        for _ in 0..100 {
            native.tick("a1", &mut asked, 1.0 / 60.0);
        }
        let start = Instant::now();
        for _ in 0..calls {
            native.tick("a1", &mut asked, 1.0 / 60.0);
        }
        let native_per_call = start.elapsed().as_nanos() as f64 / f64::from(calls);
        println!("script native: {native_per_call:.0} ns per tick (read + act)");
        println!(
            "script overhead: wasm is {:.1}x native",
            per_call / native_per_call.max(1.0)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
