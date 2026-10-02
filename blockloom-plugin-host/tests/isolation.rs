//! A native plugin hosted by a worker process: the same ops and services as
//! in-process, and a plugin that spins or dies costs only its own process.

use blockloom_plugin_api::loadout::{CodeRuntime, NativeLibrary};
use blockloom_plugin_api::manifest::Capability;
use blockloom_plugin_host::isolated::IsolatedModule;
use blockloom_plugin_host::module::is_unsupported;
use blockloom_plugin_host::native::fixture;
use blockloom_plugin_host::services::HostServices;
use serde_json::json;
use std::collections::BTreeSet;
use std::path::PathBuf;

const WORKER: &str = env!("CARGO_BIN_EXE_blockloom-plugin-worker");

fn runtime(dir: &std::path::Path, capabilities: &[Capability]) -> CodeRuntime {
    CodeRuntime::Native(NativeLibrary {
        path: fixture::build(dir, fixture::SOURCE),
        hash: "h".to_string(),
        capabilities: capabilities.iter().copied().collect::<BTreeSet<_>>(),
    })
}

fn open(runtime: &CodeRuntime) -> IsolatedModule {
    let host = HostServices::new("0.0.1");
    IsolatedModule::open_with(
        &PathBuf::from(WORKER),
        runtime,
        "com.example.fixture",
        host.for_plugin("com.example.fixture"),
    )
    .unwrap()
}

#[test]
fn calls_logs_and_host_services_cross_the_process_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let mut module = open(&runtime(dir.path(), &[]));
    assert_eq!(
        module.call_json("echo", &json!({"a": [1, 2]})).unwrap(),
        json!({"a": [1, 2]})
    );
    let numbers: Vec<u8> = (1u32..=1000).flat_map(|n| n.to_le_bytes()).collect();
    let sum = module.call("sum", &numbers, 0).unwrap();
    assert_eq!(u64::from_le_bytes(sum.try_into().unwrap()), 500_500);
    module.call("log", b"from the worker", 0).unwrap();
    assert_eq!(module.take_logs(), vec![(3, "from the worker".to_string())]);
    // The plugin asks the host for a service; the answer comes from this side.
    let version = module.call("service", b"host.version", 0).unwrap();
    let version: serde_json::Value = serde_json::from_slice(&version).unwrap();
    assert_eq!(version["engine"], "0.0.1");
    // A service it has no capability for is refused here, not in the worker.
    assert!(module.call("service", b"storage.read", 0).is_err());
    let missing = module.call("nope", b"", 0).unwrap_err();
    assert!(is_unsupported(&missing), "{missing}");
    assert!(!module.is_stopped());
}

#[test]
fn a_plugin_that_spins_is_stopped_and_the_game_goes_on() {
    let dir = tempfile::tempdir().unwrap();
    let mut module = open(&runtime(dir.path(), &[]));
    let started = std::time::Instant::now();
    let error = module.call("spin", b"", 300).unwrap_err();
    assert!(error.contains("ran past 300 ms"), "{error}");
    assert!(started.elapsed().as_secs() < 5);
    assert!(module.is_stopped());
    assert!(
        module
            .call("echo", b"x", 0)
            .unwrap_err()
            .contains("stopped")
    );
}

#[test]
fn a_plugin_that_aborts_takes_down_only_its_process() {
    let dir = tempfile::tempdir().unwrap();
    let mut module = open(&runtime(dir.path(), &[]));
    // A panic is contained by the library itself, so the module lives on.
    assert!(module.call("panic", b"", 0).is_err());
    assert_eq!(module.call("echo", b"y", 0).unwrap(), b"y");
}

#[test]
fn a_library_that_will_not_load_is_reported() {
    let runtime = CodeRuntime::Native(NativeLibrary {
        path: PathBuf::from("/nonexistent/lib.so"),
        hash: "h".to_string(),
        capabilities: BTreeSet::new(),
    });
    let host = HostServices::new("0.0.1");
    let error =
        IsolatedModule::open_with(&PathBuf::from(WORKER), &runtime, "p", host.for_plugin("p"))
            .err()
            .expect("fails");
    assert!(
        error.contains("lib.so") || error.contains("load"),
        "{error}"
    );
}
