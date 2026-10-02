//! GPU compute a plugin may ask for, without ever holding the GPU.
//!
//! A plugin never gets a device, a queue or a pipeline. It ships kernels
//! (WGSL files the host checks), and its module answers effects that name
//! host-owned buffers and kernels: make a buffer, write words into it,
//! dispatch a kernel over named buffers, read some words back. The world
//! queues those in order, runs them under a per-frame budget on its own
//! device, and hands a read back later as the `gpu.result` op.
//!
//! Every limit is a number here, so the host, the checker and the author's
//! tests share one set (see the `LIMITS` constants).

use crate::id::{validate_package_path, validate_type_id};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The words in one buffer (4 bytes each): 64 MiB.
pub const MAX_BUFFER_WORDS: u32 = 16 * 1024 * 1024;
/// The words one plugin's buffers may hold together: 128 MiB.
pub const MAX_PLUGIN_WORDS: u64 = 32 * 1024 * 1024;
/// Buffers one plugin may have at a time.
pub const MAX_BUFFERS: usize = 64;
/// Buffers one dispatch may bind.
pub const MAX_BINDINGS: usize = 8;
/// Invocations in one workgroup.
pub const MAX_WORKGROUP_INVOCATIONS: u32 = 256;
/// Workgroup memory a kernel may declare, in bytes.
pub const MAX_WORKGROUP_BYTES: u32 = 16 * 1024;
/// Workgroups along one axis of a dispatch.
pub const MAX_GROUPS_PER_AXIS: u32 = 65_535;
/// Invocations in one dispatch.
pub const MAX_DISPATCH_INVOCATIONS: u64 = 1 << 26;
/// Invocations all plugins together may run in one frame. Work over it waits
/// for the next frame, in order.
pub const FRAME_INVOCATIONS: u64 = 1 << 27;
/// Commands one plugin may have waiting.
pub const MAX_QUEUED: usize = 4096;
/// Reads one plugin may have in flight.
pub const MAX_PENDING_READS: usize = 16;
/// Words one read may return.
pub const MAX_READ_WORDS: u32 = 4 * 1024 * 1024;
/// The loop iterations a kernel may be proven to run in one invocation.
/// A loop must be a `for` with constant bounds, so this is checked, not hoped.
pub const MAX_STATIC_COST: u64 = 1 << 20;

/// How a kernel uses one of its bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingKind {
    /// `var<storage, read>`.
    Read,
    /// `var<storage, read_write>`.
    ReadWrite,
    /// `var<uniform>`.
    Uniform,
}

/// One buffer a kernel reads or writes, at `@group(0) @binding(binding)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KernelBinding {
    /// The name a dispatch binds a buffer by.
    pub name: String,
    pub binding: u32,
    pub kind: BindingKind,
}

fn default_entry() -> String {
    "main".to_string()
}

/// A compute kernel a package ships: a `.wgsl` file with one compute entry
/// point and only the buffers listed here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KernelSchema {
    pub name: String,
    /// The `.wgsl` file, relative to the package root.
    pub file: String,
    /// The compute entry point.
    #[serde(default = "default_entry")]
    pub entry: String,
    /// Must equal the entry point's `@workgroup_size`.
    pub workgroup_size: [u32; 3],
    pub bindings: Vec<KernelBinding>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

impl KernelSchema {
    pub fn check_definition(&self) -> Result<(), String> {
        let name = &self.name;
        validate_type_id(name)?;
        validate_package_path(&self.file).map_err(|e| format!("kernel {name}: {e}"))?;
        if !self.file.ends_with(".wgsl") {
            return Err(format!("kernel {name}: {} is not a .wgsl file", self.file));
        }
        validate_type_id(&self.entry).map_err(|e| format!("kernel {name}: entry: {e}"))?;
        let [x, y, z] = self.workgroup_size;
        if x == 0 || y == 0 || z == 0 {
            return Err(format!("kernel {name}: a workgroup size is zero"));
        }
        let size = u64::from(x) * u64::from(y) * u64::from(z);
        if size > u64::from(MAX_WORKGROUP_INVOCATIONS) {
            return Err(format!(
                "kernel {name}: {size} invocations a workgroup, at most {MAX_WORKGROUP_INVOCATIONS}"
            ));
        }
        if self.bindings.is_empty() || self.bindings.len() > MAX_BINDINGS {
            return Err(format!(
                "kernel {name}: 1 to {MAX_BINDINGS} bindings, not {}",
                self.bindings.len()
            ));
        }
        let mut names = BTreeSet::new();
        let mut slots = BTreeSet::new();
        for binding in &self.bindings {
            validate_type_id(&binding.name).map_err(|e| format!("kernel {name}: {e}"))?;
            if !names.insert(binding.name.as_str()) {
                return Err(format!(
                    "kernel {name}: two bindings named {}",
                    binding.name
                ));
            }
            if !slots.insert(binding.binding) {
                return Err(format!(
                    "kernel {name}: binding {} is used twice",
                    binding.binding
                ));
            }
        }
        Ok(())
    }

    pub fn binding(&self, name: &str) -> Option<&KernelBinding> {
        self.bindings.iter().find(|b| b.name == name)
    }
}

/// A kernel as a world loads it: the schema and the checked source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadoutKernel {
    pub plugin: String,
    pub schema: KernelSchema,
    pub source: String,
}

/// Words written into a buffer, as one of three number types. Exactly one
/// field is set; the bits are what the kernel sees.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Words {
    #[serde(default)]
    pub f32: Option<Vec<f32>>,
    #[serde(default)]
    pub u32: Option<Vec<u32>>,
    #[serde(default)]
    pub i32: Option<Vec<i32>>,
}

impl Words {
    pub fn len(&self) -> usize {
        self.f32
            .as_ref()
            .map(Vec::len)
            .or(self.u32.as_ref().map(Vec::len))
            .or(self.i32.as_ref().map(Vec::len))
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The raw 32-bit words, or why they are not one list of one type.
    pub fn bits(&self) -> Result<Vec<u32>, String> {
        let given = [self.f32.is_some(), self.u32.is_some(), self.i32.is_some()]
            .iter()
            .filter(|set| **set)
            .count();
        if given != 1 {
            return Err("give exactly one of f32, u32 or i32".to_string());
        }
        if let Some(values) = &self.f32 {
            if values.iter().any(|v| !v.is_finite()) {
                return Err("a value is not finite".to_string());
            }
            return Ok(values.iter().map(|v| v.to_bits()).collect());
        }
        if let Some(values) = &self.u32 {
            return Ok(values.clone());
        }
        Ok(self
            .i32
            .iter()
            .flatten()
            .map(|v| u32::from_ne_bytes(v.to_ne_bytes()))
            .collect())
    }
}

/// How a read's words are reported back.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadAs {
    #[default]
    U32,
    I32,
    F32,
}

impl ReadAs {
    /// The words as the JSON numbers the plugin's `gpu.result` op gets. A
    /// float that is not finite is `null`.
    pub fn json(self, words: &[u32]) -> serde_json::Value {
        use serde_json::Value;
        let list: Vec<Value> = words
            .iter()
            .map(|w| match self {
                ReadAs::U32 => Value::from(*w),
                ReadAs::I32 => Value::from(i32::from_ne_bytes(w.to_ne_bytes())),
                ReadAs::F32 => serde_json::Number::from_f64(f64::from(f32::from_bits(*w)))
                    .map(Value::Number)
                    .unwrap_or(Value::Null),
            })
            .collect();
        Value::Array(list)
    }
}

/// What a module's effect asks of the GPU. The world adds the plugin.
#[derive(Debug, Clone, PartialEq)]
pub enum GpuCommand {
    /// Makes (or replaces, zeroed) a buffer of `words` 32-bit words.
    Buffer { name: String, words: u32 },
    /// Writes into a buffer at a word offset.
    Write {
        buffer: String,
        offset: u32,
        data: Vec<u32>,
    },
    /// Runs a kernel over `groups` workgroups with its bindings bound to
    /// the plugin's buffers: `(binding name, buffer name)`.
    Dispatch {
        kernel: String,
        bindings: Vec<(String, String)>,
        groups: [u32; 3],
    },
    /// Reads words back, answered later as `gpu.result` with `tag`.
    Read {
        buffer: String,
        offset: u32,
        words: u32,
        tag: String,
        as_type: ReadAs,
    },
    /// Frees a buffer.
    Free { buffer: String },
}

impl GpuCommand {
    /// What the command asks of the invocation budget.
    pub fn invocations(&self, kernel: Option<&KernelSchema>) -> u64 {
        match (self, kernel) {
            (GpuCommand::Dispatch { groups, .. }, Some(kernel)) => {
                let per = u64::from(kernel.workgroup_size.iter().product::<u32>());
                groups.iter().map(|g| u64::from(*g)).product::<u64>() * per
            }
            _ => 0,
        }
    }

    /// The checks that need no device and no state: names, shapes and the
    /// limits that belong to one command.
    pub fn check(&self) -> Result<(), String> {
        match self {
            GpuCommand::Buffer { name, words } => {
                validate_type_id(name)?;
                if *words == 0 || *words > MAX_BUFFER_WORDS {
                    return Err(format!(
                        "buffer {name}: {words} words, 1 to {MAX_BUFFER_WORDS}"
                    ));
                }
            }
            GpuCommand::Write { buffer, data, .. } => {
                validate_type_id(buffer)?;
                if data.is_empty() {
                    return Err(format!("write to {buffer}: no words"));
                }
                if data.len() > MAX_BUFFER_WORDS as usize {
                    return Err(format!("write to {buffer}: more than a buffer holds"));
                }
            }
            GpuCommand::Dispatch {
                kernel,
                bindings,
                groups,
            } => {
                validate_type_id(kernel)?;
                if bindings.is_empty() || bindings.len() > MAX_BINDINGS {
                    return Err(format!("dispatch {kernel}: 1 to {MAX_BINDINGS} bindings"));
                }
                if groups.iter().any(|g| *g == 0 || *g > MAX_GROUPS_PER_AXIS) {
                    return Err(format!(
                        "dispatch {kernel}: each axis is 1 to {MAX_GROUPS_PER_AXIS} workgroups"
                    ));
                }
            }
            GpuCommand::Read {
                buffer, words, tag, ..
            } => {
                validate_type_id(buffer)?;
                if tag.is_empty() || tag.len() > 64 {
                    return Err(format!("read of {buffer}: a tag is 1 to 64 characters"));
                }
                if *words == 0 || *words > MAX_READ_WORDS {
                    return Err(format!(
                        "read of {buffer}: {words} words, 1 to {MAX_READ_WORDS}"
                    ));
                }
            }
            GpuCommand::Free { buffer } => validate_type_id(buffer)?,
        }
        Ok(())
    }
}

/// The op a module answers with a finished read: `{"tag", "buffer",
/// "offset", "values"}`. A module with no such op ignores it.
pub const RESULT_OP: &str = "gpu.result";

#[cfg(test)]
mod tests {
    use super::*;

    fn kernel() -> KernelSchema {
        KernelSchema {
            name: "scale".to_string(),
            file: "kernels/scale.wgsl".to_string(),
            entry: "main".to_string(),
            workgroup_size: [64, 1, 1],
            bindings: vec![
                KernelBinding {
                    name: "input".to_string(),
                    binding: 0,
                    kind: BindingKind::Read,
                },
                KernelBinding {
                    name: "output".to_string(),
                    binding: 1,
                    kind: BindingKind::ReadWrite,
                },
            ],
            description: String::new(),
        }
    }

    #[test]
    fn a_kernel_definition_is_checked() {
        assert!(kernel().check_definition().is_ok());
        let mut k = kernel();
        k.workgroup_size = [64, 64, 1];
        assert!(k.check_definition().unwrap_err().contains("at most 256"));
        let mut k = kernel();
        k.file = "kernels/scale.wesl".to_string();
        assert!(k.check_definition().unwrap_err().contains(".wgsl"));
        let mut k = kernel();
        k.bindings[1].binding = 0;
        assert!(k.check_definition().unwrap_err().contains("used twice"));
        let mut k = kernel();
        k.bindings.clear();
        assert!(k.check_definition().is_err());
    }

    #[test]
    fn words_are_one_list_of_one_type() {
        let w: Words = serde_json::from_str(r#"{"f32":[1.5,2.0]}"#).unwrap();
        assert_eq!(w.bits().unwrap(), vec![1.5f32.to_bits(), 2.0f32.to_bits()]);
        let w: Words = serde_json::from_str(r#"{"i32":[-1]}"#).unwrap();
        assert_eq!(w.bits().unwrap(), vec![u32::MAX]);
        assert!(Words::default().bits().is_err());
        let w: Words = serde_json::from_str(r#"{"f32":[1.0],"u32":[1]}"#).unwrap();
        assert!(w.bits().is_err());
    }

    #[test]
    fn a_read_reports_its_words_as_asked() {
        let words = [1.5f32.to_bits(), f32::NAN.to_bits(), u32::MAX];
        let f = ReadAs::F32.json(&words);
        assert_eq!(f[0], 1.5);
        assert!(f[1].is_null());
        assert_eq!(ReadAs::I32.json(&words)[2], -1);
        assert_eq!(ReadAs::U32.json(&words)[2], u32::MAX);
    }

    #[test]
    fn a_command_is_held_to_the_limits() {
        let buffer = |words| GpuCommand::Buffer {
            name: "a".to_string(),
            words,
        };
        assert!(buffer(16).check().is_ok());
        assert!(buffer(0).check().is_err());
        assert!(buffer(MAX_BUFFER_WORDS + 1).check().is_err());
        let dispatch = |groups| GpuCommand::Dispatch {
            kernel: "scale".to_string(),
            bindings: vec![("input".to_string(), "a".to_string())],
            groups,
        };
        assert!(dispatch([4, 1, 1]).check().is_ok());
        assert!(dispatch([0, 1, 1]).check().is_err());
        assert!(dispatch([MAX_GROUPS_PER_AXIS + 1, 1, 1]).check().is_err());
        assert_eq!(dispatch([4, 2, 1]).invocations(Some(&kernel())), 4 * 2 * 64);
    }
}
