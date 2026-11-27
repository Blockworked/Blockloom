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

/// One action as a LAN guest holds it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RemoteAction {
    pub held: bool,
    pub value: f32,
    pub vector: [f32; 2],
}

/// The action a later player's copy stands for: `Jump P2` is player 1's `Jump`.
pub fn base_action(name: &str, player: u8) -> &str {
    let suffix = format!(" P{}", u16::from(player) + 1);
    if name.len() > suffix.len() && name[name.len() - suffix.len()..].eq_ignore_ascii_case(&suffix)
    {
        &name[..name.len() - suffix.len()]
    } else {
        name
    }
}

#[cfg(not(all(
    feature = "multiplayer",
    not(target_arch = "wasm32"),
    not(target_os = "android")
)))]
pub fn guest_players(_: &Engine) -> Vec<u8> {
    Vec::new()
}
#[cfg(not(all(
    feature = "multiplayer",
    not(target_arch = "wasm32"),
    not(target_os = "android")
)))]
pub fn remote_action(_: &Engine, _: u8, _: &str) -> Option<RemoteAction> {
    None
}

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
