//! Effects that ask the world to run GPU compute (needs the `gpu-compute`
//! capability and a kernel in the package). Each function returns the JSON
//! one entry of an op's `{"effects": [...]}` list is.
//!
//! ```
//! use blockloom_plugin_sdk::{gpu, json};
//! let answer = json!({"effects": [
//!     gpu::buffer("input", 1024),
//!     gpu::write_f32("input", 0, &[1.0, 2.0]),
//!     gpu::dispatch("scale", &[("input", "input"), ("output", "out")], [16, 1, 1]),
//!     gpu::read("out", 0, 1024, "scaled", gpu::As::F32),
//! ]});
//! # let _ = answer;
//! ```
//!
//! A read answers later: the module's `gpu.result` op gets `{tag, buffer,
//! offset, values}`; see [`result`].

use serde_json::{Value, json};

/// How a read's words come back.
#[derive(Debug, Clone, Copy)]
pub enum As {
    U32,
    I32,
    F32,
}

impl As {
    fn name(self) -> &'static str {
        match self {
            As::U32 => "u32",
            As::I32 => "i32",
            As::F32 => "f32",
        }
    }
}

/// The op a finished read is answered through.
pub const RESULT_OP: &str = blockloom_plugin_api::compute::RESULT_OP;

/// Makes (or replaces, zeroed) a buffer of `words` 32-bit words.
pub fn buffer(name: &str, words: u32) -> Value {
    json!({"effect": "gpu_buffer", "name": name, "words": words})
}

pub fn write_f32(buffer: &str, offset: u32, values: &[f32]) -> Value {
    json!({"effect": "gpu_write", "buffer": buffer, "offset": offset, "f32": values})
}

pub fn write_u32(buffer: &str, offset: u32, values: &[u32]) -> Value {
    json!({"effect": "gpu_write", "buffer": buffer, "offset": offset, "u32": values})
}

pub fn write_i32(buffer: &str, offset: u32, values: &[i32]) -> Value {
    json!({"effect": "gpu_write", "buffer": buffer, "offset": offset, "i32": values})
}

/// Runs `kernel` over `groups` workgroups; `bindings` pairs each of the
/// kernel's binding names with one of the plugin's buffers.
pub fn dispatch(kernel: &str, bindings: &[(&str, &str)], groups: [u32; 3]) -> Value {
    let bindings: serde_json::Map<String, Value> = bindings
        .iter()
        .map(|(binding, buffer)| (binding.to_string(), json!(buffer)))
        .collect();
    json!({"effect": "gpu_dispatch", "kernel": kernel, "bindings": bindings, "groups": groups})
}

/// Reads `words` words back; they arrive later under `tag`.
pub fn read(buffer: &str, offset: u32, words: u32, tag: &str, as_type: As) -> Value {
    json!({
        "effect": "gpu_read", "buffer": buffer, "offset": offset,
        "words": words, "tag": tag, "as": as_type.name(),
    })
}

pub fn free(buffer: &str) -> Value {
    json!({"effect": "gpu_free", "buffer": buffer})
}

/// A finished read, as the `gpu.result` op receives it.
#[derive(Debug, Clone, PartialEq)]
pub struct Read {
    pub tag: String,
    pub buffer: String,
    pub offset: u32,
    pub values: Vec<Value>,
}

/// Parses a `gpu.result` op's arguments.
pub fn result(args: &Value) -> Option<Read> {
    Some(Read {
        tag: args.get("tag")?.as_str()?.to_string(),
        buffer: args.get("buffer")?.as_str()?.to_string(),
        offset: u32::try_from(args.get("offset")?.as_u64()?).ok()?,
        values: args.get("values")?.as_array()?.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_have_the_shape_the_world_reads() {
        assert_eq!(
            dispatch("k", &[("a", "x")], [2, 1, 1]),
            json!({"effect": "gpu_dispatch", "kernel": "k", "bindings": {"a": "x"}, "groups": [2, 1, 1]})
        );
        assert_eq!(read("b", 0, 4, "t", As::F32)["as"], "f32");
        assert_eq!(write_u32("b", 1, &[7])["u32"], json!([7]));
    }

    #[test]
    fn a_result_is_parsed() {
        let read = result(&json!({"tag": "t", "buffer": "b", "offset": 2, "values": [1, 2]}));
        assert_eq!(read.unwrap().values.len(), 2);
        assert!(result(&json!({"tag": "t"})).is_none());
    }
}
