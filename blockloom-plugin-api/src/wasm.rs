//! The portable (WebAssembly) boundary, version 1.
//!
//! The same contract as [`crate::abi`], carried over one linear memory
//! instead of pointers. A module is a core WebAssembly binary (no component
//! model, no WASI) that imports only the two functions below from
//! [`IMPORT_MODULE`] and exports the four below plus its memory.
//!
//! # Contract
//!
//! - `blockloom_abi() -> i32` returns [`crate::versions::PLUGIN_ABI`].
//! - `blockloom_alloc(len) -> ptr` and `blockloom_free(ptr, len)` are the
//!   module's allocator. The host allocates inside the module to hand it
//!   bytes, and frees what the module hands back. Memory the host did not
//!   allocate is never freed by the host except through `blockloom_free`.
//! - `blockloom_call(op_ptr, op_len, in_ptr, in_len, out) -> status` runs an
//!   op. `out` points at 8 bytes the module fills with a little-endian
//!   `(ptr, len)` pair naming its answer; a zero `len` is an empty answer.
//!   The status is an [`crate::abi::Status`] code.
//! - `blockloom.log(level, ptr, len)` writes a UTF-8 line to the host log.
//! - `blockloom.call(service_ptr, service_len, in_ptr, in_len, out) -> status`
//!   asks a host service. The host allocates the answer in the module through
//!   `blockloom_alloc` and writes `(ptr, len)` at `out`; the module frees it.
//! - Payloads are UTF-8 JSON, or raw little-endian bytes under an op that
//!   documents its layout. Batch them: one call is a boundary crossing.
//! - A call runs against a budget of work ([`FUEL_PER_MS`] per millisecond of
//!   the manifest's `call_limit_ms`) and a memory ceiling
//!   (`memory_limit_mib`). Exceeding either stops the call, and the module
//!   is reloaded fresh before its next one.

pub const IMPORT_MODULE: &str = "blockloom";
pub const IMPORT_LOG: &str = "log";
pub const IMPORT_CALL: &str = "call";

pub const EXPORT_MEMORY: &str = "memory";
pub const EXPORT_ABI: &str = "blockloom_abi";
pub const EXPORT_ALLOC: &str = "blockloom_alloc";
pub const EXPORT_FREE: &str = "blockloom_free";
pub const EXPORT_CALL: &str = "blockloom_call";

/// Interpreter fuel (about one unit per instruction) a call may spend per
/// millisecond of `call_limit_ms`. A budget of work rather than a clock: the
/// cheapest instruction mix runs about this fast on a desktop core, so the
/// limit is roughly a ceiling on time for it and a longer one for heavier
/// code, and a slow machine stops a runaway module at the same point.
pub const FUEL_PER_MS: u64 = 500_000;
