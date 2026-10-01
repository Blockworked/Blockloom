//! The plugin SDK: one trait and one macro for both code tiers.
//!
//! A plugin implements [`Plugin`] and names it with [`export_plugin!`]. Built
//! for a desktop target as a `cdylib` it exports the C entry symbol
//! (`blockloom_plugin_entry_v1`); built for `wasm32-unknown-unknown` it
//! exports the portable module's four functions. The author never sees a
//! pointer, a [`Slice`](blockloom_plugin_api::abi::Slice) or a linear memory.
//!
//! ```ignore
//! use blockloom_plugin_sdk::{Error, Host, Plugin, Value, export_plugin, json};
//!
//! struct Counter(i64);
//!
//! impl Plugin for Counter {
//!     fn start(_host: &Host) -> Result<Self, Error> {
//!         Ok(Counter(0))
//!     }
//!
//!     fn call_json(&mut self, _host: &Host, op: &str, args: Value) -> Result<Value, Error> {
//!         match op {
//!             "bump" => {
//!                 self.0 += args["by"].as_i64().unwrap_or(1);
//!                 Ok(json!({ "count": self.0 }))
//!             }
//!             _ => Err(Error::unsupported(op)),
//!         }
//!     }
//! }
//!
//! export_plugin!(Counter);
//! ```
//!
//! The plugin instance lives as long as the module does, so state kept in
//! `self` survives between calls; it does not survive a reload, which the
//! host does after a fault, so anything that matters belongs in the project
//! (a plugin component or resource).

use blockloom_plugin_api::abi::Status;

pub use serde_json::{self, Value, json};

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(not(target_arch = "wasm32"))]
use native as imp;
#[cfg(target_arch = "wasm32")]
use wasm as imp;

/// What the macro expands against; not part of the SDK's own API.
#[doc(hidden)]
pub mod __private {
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::native::*;
    #[cfg(target_arch = "wasm32")]
    pub use crate::wasm::*;
    pub use blockloom_plugin_api::abi;
}

/// A failed call: the status the host sees and a line for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub status: Status,
    pub message: String,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Error {
        Error {
            status: Status::Error,
            message: message.into(),
        }
    }

    /// The arguments were not what the op takes.
    pub fn bad_argument(message: impl Into<String>) -> Error {
        Error {
            status: Status::BadArgument,
            message: message.into(),
        }
    }

    /// The plugin has no such op.
    pub fn unsupported(op: &str) -> Error {
        Error {
            status: Status::Unsupported,
            message: format!("no op named {op}"),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for Error {}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Error {
        Error::bad_argument(error.to_string())
    }
}

/// How loud a log line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
}

/// What the host offers a plugin. Valid inside a call from the host.
pub struct Host {
    imp: imp::Host,
}

impl Host {
    pub fn log(&self, level: Level, message: &str) {
        self.imp.log(level as u32, message);
    }

    pub fn error(&self, message: &str) {
        self.log(Level::Error, message);
    }

    pub fn warn(&self, message: &str) {
        self.log(Level::Warn, message);
    }

    pub fn info(&self, message: &str) {
        self.log(Level::Info, message);
    }

    pub fn debug(&self, message: &str) {
        self.log(Level::Debug, message);
    }

    /// Asks a named host service (`host.version`, `storage.read`, ...). A
    /// service the manifest's capabilities do not cover answers
    /// [`Status::Unsupported`].
    pub fn call(&self, service: &str, input: &[u8]) -> Result<Vec<u8>, Error> {
        self.imp.call(service, input).map_err(|status| Error {
            status,
            message: format!("host service {service}: {status:?}"),
        })
    }

    /// [`Host::call`] with a JSON input and answer.
    pub fn call_json(&self, service: &str, input: &Value) -> Result<Value, Error> {
        let answer = self.call(service, &serde_json::to_vec(input)?)?;
        if answer.is_empty() {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_slice(&answer)?)
    }
}

/// A plugin's code. One instance serves the whole module's life.
pub trait Plugin: Sized + 'static {
    /// Builds the instance, once, when the host loads the module.
    fn start(host: &Host) -> Result<Self, Error>;

    /// Runs `op` over raw bytes. The default treats both sides as JSON and
    /// calls [`Plugin::call_json`]; override it for an op with a bulk layout
    /// of its own (chunk buffers, instance lists).
    fn call(&mut self, host: &Host, op: &str, input: &[u8]) -> Result<Vec<u8>, Error> {
        let args = if input.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(input)?
        };
        let answer = self.call_json(host, op, args)?;
        if answer.is_null() {
            return Ok(Vec::new());
        }
        Ok(serde_json::to_vec(&answer)?)
    }

    /// Runs `op` over a JSON value and answers one. A command's
    /// `{"do": "module", "op": ...}` action lands here with its arguments.
    fn call_json(&mut self, _host: &Host, op: &str, _args: Value) -> Result<Value, Error> {
        Err(Error::unsupported(op))
    }
}

/// One live plugin and the host it was started with.
#[doc(hidden)]
pub struct Instance<P> {
    host: Host,
    plugin: P,
}

impl<P: Plugin> Instance<P> {
    fn start(host: Host) -> Result<Instance<P>, Status> {
        match P::start(&host) {
            Ok(plugin) => Ok(Instance { host, plugin }),
            Err(error) => Err(refuse(&host, error)),
        }
    }

    /// Runs one call; a failure is logged here and its status handed back.
    fn run(&mut self, op: &[u8], input: &[u8]) -> Result<Vec<u8>, Status> {
        let Ok(op) = std::str::from_utf8(op) else {
            self.host.error("an op name that is not UTF-8");
            return Err(Status::BadArgument);
        };
        self.plugin
            .call(&self.host, op, input)
            .map_err(|error| refuse(&self.host, error))
    }
}

fn refuse(host: &Host, error: Error) -> Status {
    host.error(&error.message);
    error.status
}
