//! Native QUIC transport for multiplayer (plan: `docs/multiplayer-and-embedded-server-plan.md`).
//!
//! Phase 0 slice: Quiche over UDP with a self-signed identity pinned by
//! fingerprint, address validation by Retry, reliable streams with a bounded
//! queue, datagrams, and a seeded impairment layer for loss/delay tests. It
//! carries opaque bytes; the game protocol sits above it.
//!
//! Everything is driven by `poll` from one thread, so nothing here ever blocks
//! the simulation.

mod client;
mod error;
mod identity;
mod link;
mod server;
mod session;

pub use client::{Client, ClientOptions};
pub use error::NetError;
pub use identity::{Fingerprint, Identity};
pub use link::{Impairment, LinkStats};
pub use server::{PeerId, Server, ServerOptions, ServerStats};
pub use session::{Event, TransportStats};

/// Application protocol, separate from the editor protocol.
pub const ALPN: &[u8] = b"blockloom-game/1";

/// Largest UDP payload either side sends or accepts.
pub const MAX_UDP_PAYLOAD: usize = 1350;
