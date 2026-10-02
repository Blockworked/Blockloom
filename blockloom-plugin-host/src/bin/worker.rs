//! Hosts one plugin module for a game that asked for isolation: see
//! `blockloom_plugin_host::isolated`. Started by the host, never by hand.

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn main() {
    std::process::exit(blockloom_plugin_host::isolated::serve());
}

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
fn main() {
    eprintln!("plugin workers are not available on this platform");
}
