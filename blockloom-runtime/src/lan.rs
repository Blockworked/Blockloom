//! LAN exposure attaches to the existing world and retains settled replicas.
use crate::engine::Engine;
use blockloom_protocol::{LanSessionStatus, RuntimeMessage};

#[cfg(all(
    feature = "multiplayer",
    not(target_arch = "wasm32"),
    not(target_os = "android")
))]
mod native;
#[cfg(all(
    feature = "multiplayer",
    not(target_arch = "wasm32"),
    not(target_os = "android")
))]
pub use native::*;

#[cfg(not(all(
    feature = "multiplayer",
    not(target_arch = "wasm32"),
    not(target_os = "android")
)))]
#[derive(Default)]
pub struct Session;

pub fn report(engine: &Engine, error: Option<String>) {
    let mut status = status(engine);
    status.error = error;
    crate::bridge::send(&RuntimeMessage::LanSession(status));
}
#[cfg(not(all(
    feature = "multiplayer",
    not(target_arch = "wasm32"),
    not(target_os = "android")
)))]
pub fn status(_: &Engine) -> LanSessionStatus {
    LanSessionStatus::default()
}
#[cfg(not(all(
    feature = "multiplayer",
    not(target_arch = "wasm32"),
    not(target_os = "android")
)))]
pub fn open(e: &mut Engine, _: &str, _: usize) {
    report(e, Some("This runtime has no native LAN support".into()));
}
#[cfg(not(all(
    feature = "multiplayer",
    not(target_arch = "wasm32"),
    not(target_os = "android")
)))]
pub fn close(_: &mut Engine) {}
#[cfg(not(all(
    feature = "multiplayer",
    not(target_arch = "wasm32"),
    not(target_os = "android")
)))]
pub fn kick(_: &mut Engine, _: u64) {}
#[cfg(not(all(
    feature = "multiplayer",
    not(target_arch = "wasm32"),
    not(target_os = "android")
)))]
pub fn register(_: &mut bevy::prelude::App) {}

#[cfg(all(
    feature = "multiplayer",
    not(target_arch = "wasm32"),
    not(target_os = "android")
))]
pub mod guest;

#[cfg(not(all(
    feature = "multiplayer",
    not(target_arch = "wasm32"),
    not(target_os = "android")
)))]
pub fn begin(e: &mut Engine) {
    report(e, None);
}
