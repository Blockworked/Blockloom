//! The SDK's native side, driven through the host's real loader.
//!
//! `export_plugin!` defines one process-wide symbol, so these tests call the
//! entry function it wraps directly and keep plugin types in this file.

use blockloom_plugin_api::abi::{EntryFn, HostApi, PluginApi, Status};
use blockloom_plugin_api::manifest::Capability;
use blockloom_plugin_host::native::{NativeModule, ServiceFn, default_services};
use blockloom_plugin_sdk::__private::entry;
use blockloom_plugin_sdk::{Error, Host, Plugin, Value, json};

#[derive(Default)]
struct Probe {
    calls: u32,
}

impl Plugin for Probe {
    fn start(host: &Host) -> Result<Self, Error> {
        host.debug("probe up");
        Ok(Probe::default())
    }

    fn call(&mut self, host: &Host, op: &str, input: &[u8]) -> Result<Vec<u8>, Error> {
        self.calls += 1;
        match op {
            // A bulk op keeps its own byte layout: u32s in, their sum out.
            "sum" => {
                let total: u64 = input
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| u32::from_le_bytes(*c) as u64)
                    .sum();
                Ok(total.to_le_bytes().to_vec())
            }
            "boom" => panic!("probe exploded"),
            "stale" => Err(Error {
                status: Status::StaleHandle,
                message: "that thing is gone".to_string(),
            }),
            "warn" => {
                host.warn(std::str::from_utf8(input).unwrap_or("?"));
                Ok(Vec::new())
            }
            "service" => host
                .call(std::str::from_utf8(input).unwrap_or(""), b"{}")
                .map_err(|e| Error::new(e.message)),
            _ => {
                // Everything else is JSON, through the same default.
                let calls = self.calls;
                let args = if input.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_slice(input)?
                };
                Ok(serde_json::to_vec(
                    &json!({"op": op, "args": args, "calls": calls}),
                )?)
            }
        }
    }
}

struct Refuses;

impl Plugin for Refuses {
    fn start(_host: &Host) -> Result<Self, Error> {
        Err(Error::new("not today"))
    }
}

struct Panics;

impl Plugin for Panics {
    fn start(_host: &Host) -> Result<Self, Error> {
        panic!("dead on arrival")
    }
}

unsafe extern "C" fn probe_entry(host: *const HostApi, out: *mut PluginApi) -> i32 {
    unsafe { entry::<Probe>(host, out) }
}

unsafe extern "C" fn refuses_entry(host: *const HostApi, out: *mut PluginApi) -> i32 {
    unsafe { entry::<Refuses>(host, out) }
}

unsafe extern "C" fn panics_entry(host: *const HostApi, out: *mut PluginApi) -> i32 {
    unsafe { entry::<Panics>(host, out) }
}

fn start(
    entry: EntryFn,
    caps: &[Capability],
    services: Box<ServiceFn>,
) -> Result<NativeModule, String> {
    unsafe { NativeModule::from_entry(entry, caps.iter().copied().collect(), services) }
}

fn probe() -> NativeModule {
    start(probe_entry, &[], default_services("test".to_string())).unwrap()
}

#[test]
fn json_ops_use_the_default_and_the_instance_keeps_its_state() {
    let module = probe();
    let first = module.call_json("anything", &json!({"a": 1})).unwrap();
    assert_eq!(
        first,
        json!({"op": "anything", "args": {"a": 1}, "calls": 1})
    );
    let second = module.call_json("anything", &Value::Null).unwrap();
    assert_eq!(second["calls"], 2);
    assert_eq!(module.take_logs(), vec![(4, "probe up".to_string())]);
}

#[test]
fn bulk_bytes_pass_through_untouched() {
    let module = probe();
    let numbers: Vec<u8> = (1u32..=1000).flat_map(|n| n.to_le_bytes()).collect();
    let sum = module.call("sum", &numbers).unwrap();
    assert_eq!(u64::from_le_bytes(sum.try_into().unwrap()), 500_500);
}

#[test]
fn an_error_keeps_its_status_and_is_logged() {
    let module = probe();
    assert_eq!(module.call("stale", b""), Err(Status::StaleHandle));
    assert_eq!(module.call("warn", b"careful").unwrap(), b"");
    let logs = module.take_logs();
    assert!(
        logs.contains(&(1, "that thing is gone".to_string())),
        "{logs:?}"
    );
    assert!(logs.contains(&(2, "careful".to_string())), "{logs:?}");
    // Bad JSON through the default is the caller's mistake.
    assert_eq!(module.call("echo", b"{not json"), Err(Status::BadArgument));
}

#[test]
fn a_panic_is_contained_logged_and_the_module_carries_on() {
    let module = probe();
    assert_eq!(module.call("boom", b""), Err(Status::Panicked));
    let logs = module.take_logs();
    assert!(
        logs.iter()
            .any(|(l, t)| *l == 1 && t.contains("probe exploded")),
        "{logs:?}"
    );
    assert_eq!(
        module.call_json("again", &Value::Null).unwrap()["op"],
        "again"
    );
}

#[test]
fn host_services_reach_the_plugin_and_obey_the_capability_gate() {
    let services: Box<ServiceFn> = Box::new(|name, _| match name {
        "host.version" | "storage.read" => Ok(name.as_bytes().to_vec()),
        _ => Err(Status::Unsupported),
    });
    let denied = start(probe_entry, &[], services).unwrap();
    assert_eq!(
        denied.call("service", b"host.version").unwrap(),
        b"host.version"
    );
    assert_eq!(denied.call("service", b"storage.read"), Err(Status::Error));
    assert!(
        denied
            .take_logs()
            .iter()
            .any(|(l, t)| *l == 1 && t.contains("Unsupported"))
    );

    let services: Box<ServiceFn> = Box::new(|name, _| Ok(name.as_bytes().to_vec()));
    let allowed = start(probe_entry, &[Capability::ProjectStorage], services).unwrap();
    assert_eq!(
        allowed.call("service", b"storage.read").unwrap(),
        b"storage.read"
    );
}

#[test]
fn a_plugin_that_will_not_start_fails_the_load_cleanly() {
    let error = start(refuses_entry, &[], default_services(String::new()))
        .err()
        .unwrap();
    assert!(error.contains("refused to start"), "{error}");
    let error = start(panics_entry, &[], default_services(String::new()))
        .err()
        .unwrap();
    assert!(error.contains("refused to start"), "{error}");
}

#[test]
fn modules_are_independent_and_dropping_one_is_clean() {
    let a = probe();
    let b = probe();
    a.call_json("x", &Value::Null).unwrap();
    a.call_json("x", &Value::Null).unwrap();
    assert_eq!(b.call_json("x", &Value::Null).unwrap()["calls"], 1);
    drop(a);
    assert_eq!(b.call_json("x", &Value::Null).unwrap()["calls"], 2);
}

#[test]
fn a_host_speaking_another_abi_is_refused() {
    let host = HostApi {
        size: std::mem::size_of::<HostApi>() as u32,
        abi_version: 99,
        ctx: std::ptr::null_mut(),
        log: None,
        alloc: None,
        free: None,
        call: None,
    };
    let mut api = PluginApi::empty();
    let code = unsafe { probe_entry(&host, &mut api) };
    assert_eq!(Status::from_code(code), Status::VersionMismatch);
    assert_eq!(
        Status::from_code(unsafe { probe_entry(std::ptr::null(), &mut api) }),
        Status::BadArgument
    );
}
