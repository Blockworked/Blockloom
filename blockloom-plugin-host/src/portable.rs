//! Running a portable module: WebAssembly under [`blockloom_plugin_api::wasm`].
//!
//! The module is isolated: its own linear memory with a ceiling, no imports
//! but the two host functions, and a budget of work per call. A call that
//! runs out of either stops, and the module is not trusted afterwards: it is
//! marked [`PortableModule::is_stopped`] and the owner reloads it.

use crate::native::{CapabilityGate, ServiceFn};
use blockloom_plugin_api::abi::{ABI_VERSION, LOG_ERROR, Status};
use blockloom_plugin_api::manifest::{Capability, PortableEntry};
use blockloom_plugin_api::wasm::{
    EXPORT_ABI, EXPORT_ALLOC, EXPORT_CALL, EXPORT_FREE, EXPORT_MEMORY, FUEL_PER_MS, IMPORT_CALL,
    IMPORT_LOG, IMPORT_MODULE,
};
use std::collections::BTreeSet;
use wasmi::{
    Caller, Config, Engine, Extern, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder,
    TrapCode, TypedFunc,
};

/// The most log lines and bytes per line a module may leave unread.
const MAX_LOGS: usize = 1000;
const MAX_LOG_BYTES: usize = 4096;

struct State {
    gate: CapabilityGate,
    services: Box<ServiceFn>,
    logs: Vec<(u32, String)>,
    limits: StoreLimits,
}

/// A loaded portable module. Not thread-safe by itself: the owner holds it
/// behind a `Mutex`, as with a native module.
pub struct PortableModule {
    store: Store<State>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    free: TypedFunc<(i32, i32), ()>,
    call: TypedFunc<(i32, i32, i32, i32, i32), i32>,
    fuel_per_call: u64,
    call_limit_ms: u32,
    stopped: bool,
}

fn guest_memory(caller: &Caller<'_, State>) -> Option<Memory> {
    match caller.get_export(EXPORT_MEMORY) {
        Some(Extern::Memory(memory)) => Some(memory),
        _ => None,
    }
}

fn read_guest(caller: &Caller<'_, State>, ptr: i32, len: i32) -> Option<Vec<u8>> {
    let (ptr, len) = (u32::try_from(ptr).ok()?, u32::try_from(len).ok()?);
    let memory = guest_memory(caller)?;
    let mut bytes = vec![0u8; len as usize];
    memory.read(caller, ptr as usize, &mut bytes).ok()?;
    Some(bytes)
}

fn host_log(mut caller: Caller<'_, State>, level: i32, ptr: i32, len: i32) {
    let Some(mut bytes) = read_guest(&caller, ptr, len.min(MAX_LOG_BYTES as i32)) else {
        return;
    };
    bytes.truncate(MAX_LOG_BYTES);
    let state = caller.data_mut();
    if state.logs.len() < MAX_LOGS {
        let level = u32::try_from(level).unwrap_or(LOG_ERROR);
        state
            .logs
            .push((level, String::from_utf8_lossy(&bytes).into_owned()));
    }
}

fn host_call(
    mut caller: Caller<'_, State>,
    service: i32,
    service_len: i32,
    input: i32,
    input_len: i32,
    out: i32,
) -> Result<i32, wasmi::Error> {
    let bad = Status::BadArgument as i32;
    let (Some(name), Some(input)) = (
        read_guest(&caller, service, service_len),
        read_guest(&caller, input, input_len),
    ) else {
        return Ok(bad);
    };
    let Ok(out) = usize::try_from(out) else {
        return Ok(bad);
    };
    let name = String::from_utf8_lossy(&name).into_owned();
    let answer = {
        let state = caller.data();
        if !state.gate.allows(&name) {
            return Ok(Status::Unsupported as i32);
        }
        match (state.services)(&name, &input) {
            Ok(bytes) => bytes,
            Err(status) => return Ok(status as i32),
        }
    };
    let Ok(len) = i32::try_from(answer.len()) else {
        return Ok(bad);
    };
    let Some(Extern::Func(alloc)) = caller.get_export(EXPORT_ALLOC) else {
        return Ok(Status::Error as i32);
    };
    let ptr = alloc.typed::<i32, i32>(&caller)?.call(&mut caller, len)?;
    let Some(memory) = guest_memory(&caller) else {
        return Ok(Status::Error as i32);
    };
    let mut slot = [0u8; 8];
    slot[..4].copy_from_slice(&ptr.to_le_bytes());
    slot[4..].copy_from_slice(&len.to_le_bytes());
    if memory
        .write(&mut caller, ptr as u32 as usize, &answer)
        .is_err()
        || memory.write(&mut caller, out, &slot).is_err()
    {
        return Ok(bad);
    }
    Ok(Status::Ok as i32)
}

impl PortableModule {
    /// Compiles and starts `wasm` under the manifest's limits.
    pub fn load(
        wasm: &[u8],
        entry: &PortableEntry,
        capabilities: BTreeSet<Capability>,
        services: Box<ServiceFn>,
    ) -> Result<PortableModule, String> {
        if !wasm.starts_with(b"\0asm") {
            return Err("not a WebAssembly binary".to_string());
        }
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, wasm).map_err(|e| e.to_string())?;
        let limits = StoreLimitsBuilder::new()
            .memory_size(entry.memory_limit_mib as usize * 1024 * 1024)
            .memories(1)
            .instances(1)
            .tables(8)
            .trap_on_grow_failure(false)
            .build();
        let mut store = Store::new(
            &engine,
            State {
                gate: CapabilityGate::new(capabilities),
                services,
                logs: Vec::new(),
                limits,
            },
        );
        store.limiter(|state| &mut state.limits);
        let fuel_per_call = u64::from(entry.call_limit_ms).saturating_mul(FUEL_PER_MS);
        store.set_fuel(fuel_per_call).map_err(|e| e.to_string())?;
        let mut linker = <Linker<State>>::new(&engine);
        linker
            .func_wrap(IMPORT_MODULE, IMPORT_LOG, host_log)
            .map_err(|e| e.to_string())?;
        linker
            .func_wrap(IMPORT_MODULE, IMPORT_CALL, host_call)
            .map_err(|e| e.to_string())?;
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| format!("could not start: {e}"))?;
        let export = |name: &str| format!("the module does not export {name} with the right type");
        let abi = instance
            .get_typed_func::<(), i32>(&store, EXPORT_ABI)
            .map_err(|_| export(EXPORT_ABI))?
            .call(&mut store, ())
            .map_err(|e| e.to_string())?;
        if abi != ABI_VERSION as i32 {
            return Err(format!(
                "the module speaks ABI {abi}, host speaks {ABI_VERSION}"
            ));
        }
        let memory = instance
            .get_memory(&store, EXPORT_MEMORY)
            .ok_or_else(|| format!("the module does not export {EXPORT_MEMORY}"))?;
        Ok(PortableModule {
            alloc: instance
                .get_typed_func(&store, EXPORT_ALLOC)
                .map_err(|_| export(EXPORT_ALLOC))?,
            free: instance
                .get_typed_func(&store, EXPORT_FREE)
                .map_err(|_| export(EXPORT_FREE))?,
            call: instance
                .get_typed_func(&store, EXPORT_CALL)
                .map_err(|_| export(EXPORT_CALL))?,
            store,
            memory,
            fuel_per_call,
            call_limit_ms: entry.call_limit_ms,
            stopped: false,
        })
    }

    /// Whether an earlier call ran out of budget or trapped. Its memory may
    /// be half-updated, so the owner drops it and loads a fresh one.
    pub fn is_stopped(&self) -> bool {
        self.stopped
    }

    fn explain(&self, op: &str, error: wasmi::Error) -> String {
        match error.as_trap_code() {
            Some(TrapCode::OutOfFuel) => format!(
                "{op}: used more than its {} ms of work for one call and was stopped",
                self.call_limit_ms
            ),
            _ => format!("{op}: the module trapped: {error}"),
        }
    }

    /// Copies `bytes` into the module's memory through its allocator.
    fn put(&mut self, bytes: &[u8]) -> Result<i32, wasmi::Error> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let len = i32::try_from(bytes.len()).map_err(|_| wasmi::Error::new("payload too large"))?;
        let ptr = self.alloc.call(&mut self.store, len)?;
        if ptr == 0 {
            return Err(wasmi::Error::new("the module is out of memory"));
        }
        self.memory
            .write(&mut self.store, ptr as u32 as usize, bytes)
            .map_err(|e| wasmi::Error::new(e.to_string()))?;
        Ok(ptr)
    }

    fn run(&mut self, op: &str, input: &[u8]) -> Result<Result<Vec<u8>, Status>, wasmi::Error> {
        let op_ptr = self.put(op.as_bytes())?;
        let in_ptr = self.put(input)?;
        let slot = self.put(&[0u8; 8])?;
        let status = self.call.call(
            &mut self.store,
            (op_ptr, op.len() as i32, in_ptr, input.len() as i32, slot),
        )?;
        let mut pair = [0u8; 8];
        self.memory
            .read(&self.store, slot as u32 as usize, &mut pair)
            .map_err(|e| wasmi::Error::new(e.to_string()))?;
        let ptr = i32::from_le_bytes(pair[..4].try_into().expect("4 bytes"));
        let len = i32::from_le_bytes(pair[4..].try_into().expect("4 bytes"));
        let mut answer = Vec::new();
        if len > 0 {
            answer = vec![0u8; len as usize];
            self.memory
                .read(&self.store, ptr as u32 as usize, &mut answer)
                .map_err(|_| wasmi::Error::new("the answer points outside the module's memory"))?;
            self.free.call(&mut self.store, (ptr, len))?;
        }
        // Newest first, so a stack-like allocator gets its memory back.
        for (ptr, len) in [(slot, 8), (in_ptr, input.len()), (op_ptr, op.len())] {
            if ptr != 0 {
                self.free.call(&mut self.store, (ptr, len as i32))?;
            }
        }
        let status = Status::from_code(status);
        Ok(if status == Status::Ok {
            Ok(answer)
        } else {
            Err(status)
        })
    }

    /// Calls `op` with `input` and returns the module's answer. Fails with a
    /// reason, and stops the module, when it traps or spends its budget.
    pub fn call(&mut self, op: &str, input: &[u8]) -> Result<Vec<u8>, String> {
        if self.stopped {
            return Err(format!("{op}: the module was stopped by an earlier fault"));
        }
        self.store
            .set_fuel(self.fuel_per_call)
            .map_err(|e| e.to_string())?;
        match self.run(op, input) {
            Ok(Ok(bytes)) => Ok(bytes),
            Ok(Err(status)) => Err(format!("{op}: {status:?}")),
            Err(error) => {
                self.stopped = true;
                Err(self.explain(op, error))
            }
        }
    }

    /// Calls `op` with a JSON value and parses the JSON answer.
    pub fn call_json(
        &mut self,
        op: &str,
        input: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let bytes = serde_json::to_vec(input).map_err(|e| e.to_string())?;
        let out = self.call(op, &bytes)?;
        if out.is_empty() {
            return Ok(serde_json::Value::Null);
        }
        serde_json::from_slice(&out).map_err(|e| format!("{op}: {e}"))
    }

    /// What the module has logged through the host so far.
    pub fn take_logs(&mut self) -> Vec<(u32, String)> {
        std::mem::take(&mut self.store.data_mut().logs)
    }
}

/// A tiny module in the text format, for tests here and in the app. It
/// answers `echo`, `sum` (little-endian u32s to a u64), `log`, `service`
/// (the input is a service name), `spin` (never returns) and `grow` (asks for
/// more memory than any sane limit).
#[cfg(any(test, feature = "test-fixtures"))]
pub mod fixture {
    pub const WAT: &str = r#"
(module
  (import "blockloom" "log" (func $log (param i32 i32 i32)))
  (import "blockloom" "call" (func $host (param i32 i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 2)
  (global $bump (mut i32) (i32.const 4096))
  (data (i32.const 0) "echo")
  (data (i32.const 8) "sum")
  (data (i32.const 16) "log")
  (data (i32.const 24) "service")
  (data (i32.const 32) "spin")
  (data (i32.const 40) "grow")
  (data (i32.const 48) "hello from wasm")

  (func (export "blockloom_abi") (result i32) (i32.const 1))
  (func (export "blockloom_alloc") (param $len i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $bump))
    (global.set $bump (i32.add (global.get $bump) (i32.and (i32.add (local.get $len) (i32.const 7)) (i32.const -8))))
    (local.get $p))
  ;; A bump allocator that gives back the last block, so frees in reverse order reclaim.
  (func (export "blockloom_free") (param $ptr i32) (param $len i32)
    (if (i32.eq (i32.add (local.get $ptr) (i32.and (i32.add (local.get $len) (i32.const 7)) (i32.const -8)))
                (global.get $bump))
      (then (global.set $bump (local.get $ptr)))))

  ;; Does the op at ptr/len spell the NUL-free word at `at` (len `n`)?
  (func $is (param $ptr i32) (param $len i32) (param $at i32) (param $n i32) (result i32)
    (local $i i32)
    (if (i32.ne (local.get $len) (local.get $n)) (then (return (i32.const 0))))
    (block $done
      (loop $next
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (if (i32.ne (i32.load8_u (i32.add (local.get $ptr) (local.get $i)))
                    (i32.load8_u (i32.add (local.get $at) (local.get $i))))
          (then (return (i32.const 0))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $next)))
    (i32.const 1))

  (func (export "blockloom_call")
    (param $op i32) (param $op_len i32) (param $in i32) (param $in_len i32) (param $out i32) (result i32)
    (local $i i32) (local $sum i64) (local $buf i32) (local $status i32)
    (if (call $is (local.get $op) (local.get $op_len) (i32.const 0) (i32.const 4))
      (then
        (i32.store (local.get $out) (local.get $in))
        (i32.store offset=4 (local.get $out) (local.get $in_len))
        (return (i32.const 0))))
    (if (call $is (local.get $op) (local.get $op_len) (i32.const 8) (i32.const 3))
      (then
        (block $done
          (loop $next
            (br_if $done (i32.ge_u (i32.add (local.get $i) (i32.const 4)) (i32.add (local.get $in_len) (i32.const 1))))
            (local.set $sum (i64.add (local.get $sum)
              (i64.extend_i32_u (i32.load (i32.add (local.get $in) (local.get $i))))))
            (local.set $i (i32.add (local.get $i) (i32.const 4)))
            (br $next)))
        (local.set $buf (call $alloc8))
        (i64.store (local.get $buf) (local.get $sum))
        (i32.store (local.get $out) (local.get $buf))
        (i32.store offset=4 (local.get $out) (i32.const 8))
        (return (i32.const 0))))
    (if (call $is (local.get $op) (local.get $op_len) (i32.const 16) (i32.const 3))
      (then (call $log (i32.const 3) (i32.const 48) (i32.const 15)) (return (i32.const 0))))
    (if (call $is (local.get $op) (local.get $op_len) (i32.const 24) (i32.const 7))
      (then
        (local.set $status (call $host (local.get $in) (local.get $in_len) (i32.const 0) (i32.const 0) (local.get $out)))
        (return (local.get $status))))
    (if (call $is (local.get $op) (local.get $op_len) (i32.const 32) (i32.const 4))
      (then (loop $forever (br $forever))))
    (if (call $is (local.get $op) (local.get $op_len) (i32.const 40) (i32.const 4))
      (then
        ;; A refused grow answers -1; report it as BufferTooSmall.
        (if (i32.eq (memory.grow (i32.const 60000)) (i32.const -1))
          (then (return (i32.const 5))))
        (return (i32.const 0))))
    (i32.const 3))

  (func $alloc8 (result i32)
    (local $p i32)
    (local.set $p (global.get $bump))
    (global.set $bump (i32.add (global.get $bump) (i32.const 8)))
    (local.get $p))
)
"#;

    pub fn wasm() -> Vec<u8> {
        wat::parse_str(WAT).expect("the fixture is valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_plugin_api::abi::LOG_INFO;

    fn entry(memory_limit_mib: u32, call_limit_ms: u32) -> PortableEntry {
        PortableEntry {
            module: "m.wasm".to_string(),
            memory_limit_mib,
            call_limit_ms,
        }
    }

    fn services() -> Box<ServiceFn> {
        Box::new(|name, _| match name {
            "host.version" => Ok(br#"{"abi":1}"#.to_vec()),
            "storage.read" => Ok(b"stored".to_vec()),
            _ => Err(Status::Unsupported),
        })
    }

    fn load(caps: &[Capability], memory: u32, ms: u32) -> PortableModule {
        PortableModule::load(
            &fixture::wasm(),
            &entry(memory, ms),
            caps.iter().cloned().collect(),
            services(),
        )
        .expect("loads")
    }

    #[test]
    fn a_module_echoes_and_sums_bulk_bytes() {
        let mut m = load(&[], 16, 100);
        assert_eq!(m.call("echo", b"hello").unwrap(), b"hello");
        let input: Vec<u8> = [1u32, 2, 3, 4000]
            .iter()
            .flat_map(|n| n.to_le_bytes())
            .collect();
        assert_eq!(m.call("sum", &input).unwrap(), 4006u64.to_le_bytes());
        assert_eq!(
            m.call_json("echo", &serde_json::json!({"a": 1})).unwrap()["a"],
            1
        );
        assert!(m.call("echo", b"").unwrap().is_empty());
    }

    #[test]
    fn an_unknown_op_is_a_status_not_a_trap() {
        let mut m = load(&[], 16, 100);
        let error = m.call("nope", b"").unwrap_err();
        assert!(error.contains("Unsupported"), "{error}");
        assert!(!m.is_stopped());
        assert_eq!(m.call("echo", b"still fine").unwrap(), b"still fine");
    }

    #[test]
    fn a_module_logs_and_asks_host_services_through_the_gate() {
        let mut m = load(&[], 16, 100);
        m.call("log", b"").unwrap();
        assert_eq!(
            m.take_logs(),
            vec![(LOG_INFO, "hello from wasm".to_string())]
        );
        assert!(m.take_logs().is_empty());
        let answer = m.call("service", b"host.version").unwrap();
        assert_eq!(answer, br#"{"abi":1}"#);
        // Storage needs a capability the module did not declare.
        assert!(
            m.call("service", b"storage.read")
                .unwrap_err()
                .contains("Unsupported")
        );
        let mut granted = load(&[Capability::ProjectStorage], 16, 100);
        assert_eq!(granted.call("service", b"storage.read").unwrap(), b"stored");
    }

    #[test]
    fn a_runaway_call_is_stopped_and_the_module_is_not_reused() {
        let mut m = load(&[], 16, 10);
        let error = m.call("spin", b"").unwrap_err();
        assert!(error.contains("10 ms of work"), "{error}");
        assert!(m.is_stopped());
        assert!(m.call("echo", b"x").unwrap_err().contains("stopped"));
    }

    #[test]
    fn memory_stops_at_the_ceiling() {
        // The fixture starts at 2 pages and asks for 60000 more.
        let mut m = load(&[], 16, 100);
        let error = m.call("grow", b"").unwrap_err();
        assert!(error.contains("BufferTooSmall"), "{error}");
        assert!(!m.is_stopped());
        // A ceiling below the module's own start refuses to load it.
        let small = PortableModule::load(
            &fixture::wasm(),
            &entry(0, 100),
            BTreeSet::new(),
            services(),
        );
        assert!(small.is_err());
    }

    #[test]
    fn only_a_valid_module_with_the_right_exports_loads() {
        let load = |bytes: &[u8]| {
            PortableModule::load(bytes, &entry(16, 100), BTreeSet::new(), services())
                .err()
                .unwrap_or_default()
        };
        assert!(load(b"not wasm").contains("not a WebAssembly binary"));
        let no_exports = wat::parse_str("(module (memory (export \"memory\") 1))").unwrap();
        assert!(load(&no_exports).contains(EXPORT_ABI));
        let wrong_abi = wat::parse_str(
            "(module (memory (export \"memory\") 1) (func (export \"blockloom_abi\") (result i32) i32.const 99))",
        )
        .unwrap();
        assert!(load(&wrong_abi).contains("ABI 99"));
        // Anything but the two host functions is refused, so no WASI.
        let wasi = wat::parse_str("(module (import \"wasi_snapshot_preview1\" \"fd_write\" (func (param i32 i32 i32 i32) (result i32))))").unwrap();
        assert!(load(&wasi).contains("could not start"));
    }

    /// `cargo test -p blockloom-plugin-host --release -- --ignored --nocapture wasm_call_cost`
    #[test]
    #[ignore = "a measurement, not a check"]
    fn wasm_call_cost() {
        use std::time::Instant;
        let mut m = load(&[], 64, 1000);
        let calls = 20_000u32;
        let start = Instant::now();
        for _ in 0..calls {
            m.call("echo", b"").unwrap();
        }
        let per_call = start.elapsed().as_nanos() as f64 / f64::from(calls);
        let items = 16_384usize;
        let input: Vec<u8> = (0..items as u32).flat_map(|n| n.to_le_bytes()).collect();
        let batches = 200u32;
        let start = Instant::now();
        for _ in 0..batches {
            m.call("sum", &input).unwrap();
        }
        let per_batch = start.elapsed().as_nanos() as f64 / f64::from(batches);
        let spin = {
            let mut m = load(&[], 64, 1000);
            let start = Instant::now();
            let _ = m.call("spin", b"");
            start.elapsed()
        };
        println!("wasm: {per_call:.0} ns per empty call");
        println!(
            "wasm: {per_batch:.0} ns per batch of {items} u32 ({:.2} ns per item)",
            per_batch / items as f64
        );
        println!(
            "wasm: a 1000 ms budget ({} fuel) ran out after {spin:?}",
            1000 * FUEL_PER_MS
        );
    }
}
