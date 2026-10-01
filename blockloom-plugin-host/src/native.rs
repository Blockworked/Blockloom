//! Loading a native module behind the C ABI.
//!
//! The host never assumes a Rust layout across the boundary: it resolves one
//! exported symbol, hands over a [`HostApi`] table, and from then on speaks
//! only bytes and [`Status`] codes. A native module is trusted code running in
//! the host process (the manifest must declare `native-execution`); the ABI
//! prevents layout coupling, not crashes. Fault isolation is the process
//! runtime mode.

use blockloom_plugin_api::abi::{
    ABI_VERSION, Buffer, ENTRY_SYMBOL, EntryFn, HostApi, LOG_ERROR, PluginApi, Slice, Status,
};
use blockloom_plugin_api::manifest::Capability;
use libloading::Library;
use std::collections::BTreeSet;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::Mutex;

/// What a service call is allowed to do for a plugin, from its declared
/// capabilities. Services are named `area.verb`.
#[derive(Debug, Clone, Default)]
pub struct CapabilityGate {
    granted: BTreeSet<Capability>,
}

impl CapabilityGate {
    pub fn new(granted: BTreeSet<Capability>) -> Self {
        Self { granted }
    }

    /// The capability a service needs, or `None` when it is always allowed.
    pub fn required(service: &str) -> Option<Capability> {
        match service.split('.').next()? {
            "storage" => Some(Capability::ProjectStorage),
            "net" => Some(Capability::Network),
            "fs" => Some(Capability::ExternalFiles),
            "process" => Some(Capability::Subprocess),
            _ => None,
        }
    }

    pub fn allows(&self, service: &str) -> bool {
        Self::required(service).is_none_or(|c| self.granted.contains(&c))
    }
}

/// The host side of [`HostApi::call`]: a service name and its JSON input.
pub type ServiceFn = dyn Fn(&str, &[u8]) -> Result<Vec<u8>, Status> + Send + Sync;

struct HostState {
    gate: CapabilityGate,
    services: Box<ServiceFn>,
    logs: Mutex<Vec<(u32, String)>>,
}

unsafe extern "C" fn host_log(ctx: *mut c_void, level: u32, message: Slice) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let state = unsafe { &*(ctx as *const HostState) };
        let text = String::from_utf8_lossy(unsafe { message.as_bytes() }).into_owned();
        if let Ok(mut logs) = state.logs.lock() {
            logs.push((level, text));
        }
    }));
}

unsafe extern "C" fn host_alloc(_ctx: *mut c_void, len: usize, out: *mut Buffer) -> i32 {
    if out.is_null() {
        return Status::BadArgument as i32;
    }
    unsafe { *out = Buffer::from_vec(vec![0u8; len]) };
    Status::Ok as i32
}

unsafe extern "C" fn host_free(_ctx: *mut c_void, buffer: Buffer) {
    drop(unsafe { buffer.into_vec() });
}

unsafe extern "C" fn host_call(
    ctx: *mut c_void,
    service: Slice,
    input: Slice,
    out: *mut Buffer,
) -> i32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if out.is_null() {
            return Status::BadArgument;
        }
        let state = unsafe { &*(ctx as *const HostState) };
        let name = String::from_utf8_lossy(unsafe { service.as_bytes() }).into_owned();
        if !state.gate.allows(&name) {
            return Status::Unsupported;
        }
        match (state.services)(&name, unsafe { input.as_bytes() }) {
            Ok(bytes) => {
                unsafe { *out = Buffer::from_vec(bytes) };
                Status::Ok
            }
            Err(status) => status,
        }
    }));
    result.unwrap_or(Status::Panicked) as i32
}

/// A loaded native module. Dropping it shuts the plugin down, then unloads
/// the library, in that order.
pub struct NativeModule {
    api: PluginApi,
    state: Box<HostState>,
    // Held so the table's function pointers stay valid; dropped last.
    _host: Box<HostApi>,
    library: Option<Library>,
}

impl NativeModule {
    /// Loads `path`, negotiates the ABI and returns the live module.
    ///
    /// # Trust
    /// The library runs arbitrary native code in this process.
    pub fn load(
        path: &Path,
        capabilities: BTreeSet<Capability>,
        services: Box<ServiceFn>,
    ) -> Result<NativeModule, String> {
        let library =
            unsafe { Library::new(path) }.map_err(|e| format!("{}: {e}", path.display()))?;
        let entry: EntryFn = unsafe {
            *library.get::<EntryFn>(ENTRY_SYMBOL).map_err(|_| {
                format!(
                    "{} does not export {}",
                    path.display(),
                    String::from_utf8_lossy(ENTRY_SYMBOL)
                )
            })?
        };
        let state = Box::new(HostState {
            gate: CapabilityGate::new(capabilities),
            services,
            logs: Mutex::new(Vec::new()),
        });
        let host = Box::new(HostApi {
            size: std::mem::size_of::<HostApi>() as u32,
            abi_version: ABI_VERSION,
            ctx: &*state as *const HostState as *mut c_void,
            log: Some(host_log),
            alloc: Some(host_alloc),
            free: Some(host_free),
            call: Some(host_call),
        });
        let mut api = PluginApi::empty();
        let code = catch_unwind(AssertUnwindSafe(|| unsafe { entry(&*host, &mut api) }))
            .map_err(|_| "the plugin panicked during startup".to_string())?;
        let status = Status::from_code(code);
        if status != Status::Ok {
            return Err(format!("the plugin refused to start: {status:?}"));
        }
        api.check()?;
        Ok(NativeModule {
            api,
            state,
            _host: host,
            library: Some(library),
        })
    }

    /// Calls `op` with `input` and returns the plugin's answer. The
    /// plugin's buffer is freed by the plugin, never by the host.
    pub fn call(&self, op: &str, input: &[u8]) -> Result<Vec<u8>, Status> {
        let call = self.api.call.expect("checked at load");
        let free = self.api.free_buffer.expect("checked at load");
        let mut out = Buffer::EMPTY;
        let code = unsafe {
            call(
                self.api.handle,
                Slice::of(op.as_bytes()),
                Slice::of(input),
                &mut out,
            )
        };
        let status = Status::from_code(code);
        let bytes = unsafe { out.as_bytes() }.to_vec();
        if !out.ptr.is_null() {
            unsafe { free(self.api.handle, out) };
        }
        if status == Status::Ok {
            Ok(bytes)
        } else {
            Err(status)
        }
    }

    /// Calls `op` with a JSON value and parses the JSON answer.
    pub fn call_json(
        &self,
        op: &str,
        input: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let bytes = serde_json::to_vec(input).map_err(|e| e.to_string())?;
        let out = self.call(op, &bytes).map_err(|s| format!("{op}: {s:?}"))?;
        if out.is_empty() {
            return Ok(serde_json::Value::Null);
        }
        serde_json::from_slice(&out).map_err(|e| format!("{op}: {e}"))
    }

    /// What the plugin has logged through the host so far.
    pub fn take_logs(&self) -> Vec<(u32, String)> {
        self.state
            .logs
            .lock()
            .map(|mut l| std::mem::take(&mut *l))
            .unwrap_or_default()
    }
}

impl Drop for NativeModule {
    fn drop(&mut self) {
        if let Some(shutdown) = self.api.shutdown {
            let _ = catch_unwind(AssertUnwindSafe(|| unsafe { shutdown(self.api.handle) }));
        }
        // Only now, with nothing able to call in or out, unload.
        drop(self.library.take());
    }
}

/// The default host services: the engine and ABI versions, answered so a
/// plugin can adapt to what it is running under.
pub fn default_services(engine: String) -> Box<ServiceFn> {
    Box::new(move |name, _input| match name {
        "host.version" => Ok(serde_json::to_vec(&serde_json::json!({
            "engine": engine,
            "abi": ABI_VERSION,
        }))
        .expect("serializes")),
        _ => Err(Status::Unsupported),
    })
}

/// Logs a host-side error the plugin caused.
pub fn log_error(module: &NativeModule, message: &str) {
    if let Ok(mut logs) = module.state.logs.lock() {
        logs.push((LOG_ERROR, message.to_string()));
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// A plugin written against the C layout alone, the way a non-Rust
    /// author would: no blockloom crate, just the documented structs.
    pub const SOURCE: &str = r#"
use std::ffi::c_void;

#[repr(C)] #[derive(Clone, Copy)] pub struct Slice { ptr: *const u8, len: usize }
#[repr(C)] #[derive(Clone, Copy)] pub struct Buffer { ptr: *mut u8, len: usize, cap: usize }
#[repr(C)] pub struct HostApi {
    size: u32, abi_version: u32, ctx: *mut c_void,
    log: Option<unsafe extern "C" fn(*mut c_void, u32, Slice)>,
    alloc: Option<unsafe extern "C" fn(*mut c_void, usize, *mut Buffer) -> i32>,
    free: Option<unsafe extern "C" fn(*mut c_void, Buffer)>,
    call: Option<unsafe extern "C" fn(*mut c_void, Slice, Slice, *mut Buffer) -> i32>,
}
#[repr(C)] pub struct PluginApi {
    size: u32, abi_version: u32, handle: u64,
    call: Option<unsafe extern "C" fn(u64, Slice, Slice, *mut Buffer) -> i32>,
    free_buffer: Option<unsafe extern "C" fn(u64, Buffer)>,
    shutdown: Option<unsafe extern "C" fn(u64)>,
}

static mut HOST: *const HostApi = std::ptr::null();
static mut SHUTDOWNS: u32 = 0;

fn bytes<'a>(s: Slice) -> &'a [u8] {
    if s.ptr.is_null() { &[] } else { unsafe { std::slice::from_raw_parts(s.ptr, s.len) } }
}
fn give(out: *mut Buffer, v: Vec<u8>) {
    let mut v = std::mem::ManuallyDrop::new(v);
    unsafe { *out = Buffer { ptr: v.as_mut_ptr(), len: v.len(), cap: v.capacity() } };
}

unsafe extern "C" fn call(_h: u64, op: Slice, input: Slice, out: *mut Buffer) -> i32 {
    let result = std::panic::catch_unwind(|| {
        let op = std::str::from_utf8(bytes(op)).unwrap_or("");
        let input = bytes(input);
        match op {
            "echo" => { give(out, input.to_vec()); 0 }
            "sum" => {
                // Bulk op: little-endian u32s in, their sum as u64 out.
                let total: u64 = input.chunks_exact(4)
                    .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]) as u64).sum();
                give(out, total.to_le_bytes().to_vec()); 0
            }
            "log" => unsafe {
                let host = &*HOST;
                (host.log.unwrap())(host.ctx, 3, Slice { ptr: input.as_ptr(), len: input.len() }); 0
            },
            "service" => unsafe {
                let host = &*HOST;
                let name = Slice { ptr: input.as_ptr(), len: input.len() };
                let mut reply = Buffer { ptr: std::ptr::null_mut(), len: 0, cap: 0 };
                let code = (host.call.unwrap())(host.ctx, name, Slice { ptr: std::ptr::null(), len: 0 }, &mut reply);
                if code == 0 {
                    give(out, std::slice::from_raw_parts(reply.ptr, reply.len).to_vec());
                    (host.free.unwrap())(host.ctx, reply);
                }
                code
            },
            "panic" => panic!("boom"),
            _ => 3,
        }
    });
    result.unwrap_or(8)
}

unsafe extern "C" fn free_buffer(_h: u64, b: Buffer) {
    drop(unsafe { Vec::from_raw_parts(b.ptr, b.len, b.cap) });
}

unsafe extern "C" fn shutdown(_h: u64) { unsafe { SHUTDOWNS += 1; } }

#[unsafe(no_mangle)]
pub unsafe extern "C" fn blockloom_plugin_entry_v1(host: *const HostApi, out: *mut PluginApi) -> i32 {
    unsafe {
        let host_ref = &*host;
        if host_ref.abi_version != 1 { return 6; }
        HOST = host;
        *out = PluginApi {
            size: std::mem::size_of::<PluginApi>() as u32,
            abi_version: if cfg!(feature_wrong_abi) { 9 } else { 1 },
            handle: 42,
            call: Some(call), free_buffer: Some(free_buffer), shutdown: Some(shutdown),
        };
    }
    0
}
"#;

    /// Compiles [`SOURCE`] to a cdylib in `dir` and returns its path.
    pub fn build(dir: &Path, source: &str) -> PathBuf {
        let src = dir.join("fixture.rs");
        std::fs::write(&src, source).unwrap();
        let name = if cfg!(target_os = "windows") {
            "fixture.dll"
        } else if cfg!(target_os = "macos") {
            "libfixture.dylib"
        } else {
            "libfixture.so"
        };
        let out = dir.join(name);
        let output = Command::new("rustc")
            .args(["--edition", "2024", "--crate-type", "cdylib", "-O", "-o"])
            .arg(&out)
            .arg(&src)
            .output()
            .expect("rustc is available to the tests");
        assert!(
            output.status.success(),
            "fixture failed to compile:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{SOURCE, build};
    use super::*;
    use serde_json::json;

    fn load(dir: &Path, caps: &[Capability]) -> NativeModule {
        let lib = build(dir, SOURCE);
        NativeModule::load(
            &lib,
            caps.iter().copied().collect(),
            default_services("0.0.1".to_string()),
        )
        .unwrap()
    }

    #[test]
    fn round_trips_json_and_bulk_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let module = load(dir.path(), &[]);
        assert_eq!(
            module.call_json("echo", &json!({"a": [1, 2]})).unwrap(),
            json!({"a": [1, 2]})
        );
        let numbers: Vec<u8> = (1u32..=1000).flat_map(|n| n.to_le_bytes()).collect();
        let sum = module.call("sum", &numbers).unwrap();
        assert_eq!(u64::from_le_bytes(sum.try_into().unwrap()), 500_500);
        assert_eq!(module.call("nope", b""), Err(Status::Unsupported));
    }

    #[test]
    fn a_panic_in_the_plugin_is_contained() {
        let dir = tempfile::tempdir().unwrap();
        let module = load(dir.path(), &[]);
        assert_eq!(module.call("panic", b""), Err(Status::Panicked));
        // And the module still answers afterwards.
        assert_eq!(module.call("echo", b"x").unwrap(), b"x");
    }

    #[test]
    fn the_plugin_can_log_and_ask_the_host_for_ungated_services() {
        let dir = tempfile::tempdir().unwrap();
        let module = load(dir.path(), &[]);
        module.call("log", b"hello from native").unwrap();
        assert_eq!(
            module.take_logs(),
            vec![(3, "hello from native".to_string())]
        );
        let reply = module.call("service", b"host.version").unwrap();
        let reply: serde_json::Value = serde_json::from_slice(&reply).unwrap();
        assert_eq!(reply, json!({"engine": "0.0.1", "abi": 1}));
    }

    #[test]
    fn services_are_gated_by_declared_capabilities() {
        let dir = tempfile::tempdir().unwrap();
        let lib = build(dir.path(), SOURCE);
        let services: Box<ServiceFn> = Box::new(|name, _| match name {
            "storage.read" => Ok(b"saved".to_vec()),
            _ => Err(Status::Unsupported),
        });
        let denied = NativeModule::load(&lib, BTreeSet::new(), services).unwrap();
        assert_eq!(
            denied.call("service", b"storage.read"),
            Err(Status::Unsupported)
        );
        let services: Box<ServiceFn> = Box::new(|_, _| Ok(b"saved".to_vec()));
        let allowed =
            NativeModule::load(&lib, BTreeSet::from([Capability::ProjectStorage]), services)
                .unwrap();
        assert_eq!(allowed.call("service", b"storage.read").unwrap(), b"saved");
        // Granting one capability does not grant another.
        assert_eq!(
            allowed.call("service", b"net.fetch"),
            Err(Status::Unsupported)
        );
    }

    #[test]
    fn gate_maps_services_to_capabilities() {
        assert_eq!(
            CapabilityGate::required("storage.read"),
            Some(Capability::ProjectStorage)
        );
        assert_eq!(
            CapabilityGate::required("net.fetch"),
            Some(Capability::Network)
        );
        assert_eq!(
            CapabilityGate::required("fs.open"),
            Some(Capability::ExternalFiles)
        );
        assert_eq!(
            CapabilityGate::required("process.spawn"),
            Some(Capability::Subprocess)
        );
        assert_eq!(CapabilityGate::required("host.version"), None);
        let gate = CapabilityGate::default();
        assert!(gate.allows("host.version") && !gate.allows("net.fetch"));
    }

    #[test]
    fn a_library_without_the_entry_symbol_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let lib = build(
            dir.path(),
            "#[unsafe(no_mangle)] pub extern \"C\" fn unrelated() {}",
        );
        let error = NativeModule::load(&lib, BTreeSet::new(), default_services(String::new()))
            .err()
            .unwrap();
        assert!(error.contains("does not export"), "{error}");
    }

    #[test]
    fn an_abi_version_mismatch_is_refused_at_load() {
        let dir = tempfile::tempdir().unwrap();
        let source = SOURCE.replace("if cfg!(feature_wrong_abi) { 9 } else { 1 }", "9");
        let lib = build(dir.path(), &source);
        let error = NativeModule::load(&lib, BTreeSet::new(), default_services(String::new()))
            .err()
            .unwrap();
        assert!(error.contains("ABI 9"), "{error}");
    }

    /// Measures what one call costs, one at a time and batched, for the ADR.
    /// `cargo test -p blockloom-plugin-host --release -- --ignored --nocapture abi_call_cost`
    #[test]
    #[ignore]
    fn abi_call_cost() {
        let dir = tempfile::tempdir().unwrap();
        let module = load(dir.path(), &[]);
        let one = 7u32.to_le_bytes();
        let n = 1_000_000;
        let start = std::time::Instant::now();
        for _ in 0..n {
            std::hint::black_box(module.call("sum", &one).unwrap());
        }
        let per_call = start.elapsed().as_nanos() as f64 / n as f64;
        let batch: Vec<u8> = (0..1_000_000u32).flat_map(|v| v.to_le_bytes()).collect();
        let start = std::time::Instant::now();
        for _ in 0..20 {
            std::hint::black_box(module.call("sum", &batch).unwrap());
        }
        let per_item = start.elapsed().as_nanos() as f64 / (20.0 * 1_000_000.0);
        println!(
            "one call per item: {per_call:.0} ns/item; batched 1M items: {per_item:.2} ns/item"
        );
    }
}
