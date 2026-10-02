//! The native C boundary, version 1.
//!
//! Nothing Rust-specific crosses it: no references, trait objects, `Vec`s,
//! Qt objects or Bevy resources, only fixed-width integers, pointer plus
//! length pairs and opaque handles. Every struct starts with its own `size`
//! and the `abi_version` it was built to, so either side can refuse a
//! mismatch before touching a field, and a later version can append fields.
//!
//! # Contract
//!
//! - A native library exports [`ENTRY_SYMBOL`]. The host calls it once with a
//!   [`HostApi`] and an empty [`PluginApi`] to fill in.
//! - Payloads are UTF-8 JSON in [`Slice`]s and [`Buffer`]s; bulk data (chunk
//!   buffers, instance lists) is passed as raw little-endian bytes under an
//!   op that documents its layout. Batch it: never one call per voxel.
//! - A [`Buffer`] written into an `out` pointer is owned by whoever wrote it.
//!   The plugin's buffers are freed with [`PluginApi::free_buffer`], the
//!   host's with [`HostApi::free`]. Never `free` across the boundary.
//! - A [`Slice`] argument is borrowed for the duration of the call only.
//! - Calls into the plugin come from the thread the host created it on
//!   unless an op's documentation says it is thread-safe. Callbacks the
//!   plugin makes into [`HostApi`] must happen inside a call from the host.
//! - Every call returns a [`Status`]; a panic must not cross the boundary.
//! - [`Handle`]s are issued by the side that owns the thing and carry a
//!   generation, so a late reply naming a closed thing is refused.

use std::ffi::c_void;

/// The exported function a native library must provide.
pub const ENTRY_SYMBOL: &[u8] = b"blockloom_plugin_entry_v1";

pub const ABI_VERSION: u32 = crate::versions::PLUGIN_ABI;

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok = 0,
    Error = 1,
    BadArgument = 2,
    Unsupported = 3,
    Cancelled = 4,
    BufferTooSmall = 5,
    VersionMismatch = 6,
    /// The handle names something that has been closed.
    StaleHandle = 7,
    /// The plugin panicked and the panic was contained.
    Panicked = 8,
}

impl Status {
    pub fn from_code(code: i32) -> Status {
        match code {
            0 => Status::Ok,
            2 => Status::BadArgument,
            3 => Status::Unsupported,
            4 => Status::Cancelled,
            5 => Status::BufferTooSmall,
            6 => Status::VersionMismatch,
            7 => Status::StaleHandle,
            8 => Status::Panicked,
            _ => Status::Error,
        }
    }
}

/// Borrowed bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Slice {
    pub ptr: *const u8,
    pub len: usize,
}

impl Slice {
    pub const EMPTY: Slice = Slice {
        ptr: std::ptr::null(),
        len: 0,
    };

    pub fn of(bytes: &[u8]) -> Slice {
        Slice {
            ptr: bytes.as_ptr(),
            len: bytes.len(),
        }
    }

    /// # Safety
    /// `ptr` must point at `len` readable bytes that outlive the result.
    pub unsafe fn as_bytes<'a>(&self) -> &'a [u8] {
        if self.ptr.is_null() || self.len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
        }
    }
}

/// Owned bytes, freed by the side that made them.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Buffer {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
}

impl Buffer {
    pub const EMPTY: Buffer = Buffer {
        ptr: std::ptr::null_mut(),
        len: 0,
        cap: 0,
    };

    /// Hands a `Vec`'s allocation over as a buffer. Pair with
    /// [`Buffer::into_vec`] in the same library.
    pub fn from_vec(mut v: Vec<u8>) -> Buffer {
        let b = Buffer {
            ptr: v.as_mut_ptr(),
            len: v.len(),
            cap: v.capacity(),
        };
        std::mem::forget(v);
        b
    }

    /// # Safety
    /// The buffer must have come from [`Buffer::from_vec`] in this library
    /// and not have been freed.
    pub unsafe fn into_vec(self) -> Vec<u8> {
        if self.ptr.is_null() {
            Vec::new()
        } else {
            unsafe { Vec::from_raw_parts(self.ptr, self.len, self.cap) }
        }
    }

    /// # Safety
    /// `ptr` must point at `len` readable bytes.
    pub unsafe fn as_bytes(&self) -> &[u8] {
        if self.ptr.is_null() || self.len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
        }
    }
}

pub const LOG_ERROR: u32 = 1;
pub const LOG_WARN: u32 = 2;
pub const LOG_INFO: u32 = 3;
pub const LOG_DEBUG: u32 = 4;

pub type LogFn = unsafe extern "C" fn(ctx: *mut c_void, level: u32, message: Slice);
pub type AllocFn = unsafe extern "C" fn(ctx: *mut c_void, len: usize, out: *mut Buffer) -> i32;
pub type FreeFn = unsafe extern "C" fn(ctx: *mut c_void, buffer: Buffer);
pub type HostCallFn =
    unsafe extern "C" fn(ctx: *mut c_void, service: Slice, input: Slice, out: *mut Buffer) -> i32;

/// What the host offers a plugin. Valid for the plugin's lifetime.
#[repr(C)]
pub struct HostApi {
    pub size: u32,
    pub abi_version: u32,
    pub ctx: *mut c_void,
    pub log: Option<LogFn>,
    pub alloc: Option<AllocFn>,
    pub free: Option<FreeFn>,
    /// A named host service (`"storage.read"`, `"rng"`, ...), JSON in and out.
    pub call: Option<HostCallFn>,
}

pub type PluginCallFn =
    unsafe extern "C" fn(handle: u64, op: Slice, input: Slice, out: *mut Buffer) -> i32;
pub type PluginFreeFn = unsafe extern "C" fn(handle: u64, buffer: Buffer);
pub type PluginShutdownFn = unsafe extern "C" fn(handle: u64);

/// What the plugin offers the host. Filled in by the entry function.
#[repr(C)]
pub struct PluginApi {
    pub size: u32,
    pub abi_version: u32,
    /// The plugin's own instance handle, passed back on every call.
    pub handle: u64,
    pub call: Option<PluginCallFn>,
    pub free_buffer: Option<PluginFreeFn>,
    pub shutdown: Option<PluginShutdownFn>,
}

pub type EntryFn = unsafe extern "C" fn(host: *const HostApi, out: *mut PluginApi) -> i32;

impl PluginApi {
    pub const fn empty() -> PluginApi {
        PluginApi {
            size: std::mem::size_of::<PluginApi>() as u32,
            abi_version: ABI_VERSION,
            handle: 0,
            call: None,
            free_buffer: None,
            shutdown: None,
        }
    }

    /// What the host checks after the entry function returns.
    pub fn check(&self) -> Result<(), String> {
        if self.abi_version != ABI_VERSION {
            return Err(format!(
                "plugin speaks ABI {}, host speaks {ABI_VERSION}",
                self.abi_version
            ));
        }
        if (self.size as usize) < std::mem::size_of::<PluginApi>() {
            return Err(format!(
                "plugin table is {} bytes, expected at least {}",
                self.size,
                std::mem::size_of::<PluginApi>()
            ));
        }
        if self.call.is_none() || self.free_buffer.is_none() || self.shutdown.is_none() {
            return Err("plugin table is missing call, free_buffer or shutdown".to_string());
        }
        Ok(())
    }
}

impl HostApi {
    /// What a plugin checks before it reads a field.
    pub fn check(&self) -> Result<(), Status> {
        if self.abi_version != ABI_VERSION || (self.size as usize) < std::mem::size_of::<HostApi>()
        {
            return Err(Status::VersionMismatch);
        }
        Ok(())
    }
}

/// An opaque id for something the owner can close. The high half is a
/// generation, bumped every time a slot is reused, so a stale handle never
/// names a different live thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Handle(pub u64);

impl Handle {
    pub const NONE: Handle = Handle(0);

    fn pack(generation: u32, index: u32) -> Handle {
        Handle(((generation as u64) << 32) | (index as u64 + 1))
    }

    fn unpack(self) -> Option<(u32, u32)> {
        let index = (self.0 & 0xFFFF_FFFF) as u32;
        (index != 0).then(|| ((self.0 >> 32) as u32, index - 1))
    }
}

/// Owner-side storage for what handles name.
#[derive(Debug)]
pub struct HandleTable<T> {
    slots: Vec<(u32, Option<T>)>,
    free: Vec<u32>,
}

impl<T> Default for HandleTable<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }
}

impl<T> HandleTable<T> {
    pub fn insert(&mut self, value: T) -> Handle {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.1 = Some(value);
            Handle::pack(slot.0, index)
        } else {
            self.slots.push((0, Some(value)));
            Handle::pack(0, self.slots.len() as u32 - 1)
        }
    }

    pub fn get(&self, handle: Handle) -> Result<&T, Status> {
        let (generation, index) = handle.unpack().ok_or(Status::StaleHandle)?;
        match self.slots.get(index as usize) {
            Some((g, Some(value))) if *g == generation => Ok(value),
            _ => Err(Status::StaleHandle),
        }
    }

    pub fn get_mut(&mut self, handle: Handle) -> Result<&mut T, Status> {
        let (generation, index) = handle.unpack().ok_or(Status::StaleHandle)?;
        match self.slots.get_mut(index as usize) {
            Some((g, Some(value))) if *g == generation => Ok(value),
            _ => Err(Status::StaleHandle),
        }
    }

    pub fn remove(&mut self, handle: Handle) -> Result<T, Status> {
        let (generation, index) = handle.unpack().ok_or(Status::StaleHandle)?;
        let slot = self
            .slots
            .get_mut(index as usize)
            .ok_or(Status::StaleHandle)?;
        if slot.0 != generation || slot.1.is_none() {
            return Err(Status::StaleHandle);
        }
        slot.0 = slot.0.wrapping_add(1);
        self.free.push(index);
        Ok(slot.1.take().expect("checked above"))
    }

    pub fn len(&self) -> usize {
        self.slots.iter().filter(|(_, v)| v.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_handles_are_refused_after_slot_reuse() {
        let mut table = HandleTable::default();
        let a = table.insert("a");
        assert_eq!(table.remove(a), Ok("a"));
        let b = table.insert("b");
        assert_ne!(a, b, "the reused slot carries a new generation");
        assert_eq!(table.get(a), Err(Status::StaleHandle));
        assert_eq!(table.get(b), Ok(&"b"));
        assert_eq!(table.remove(a), Err(Status::StaleHandle));
        assert_eq!(table.get(Handle::NONE), Err(Status::StaleHandle));
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn buffers_round_trip_a_vec() {
        let buffer = Buffer::from_vec(b"hello".to_vec());
        assert_eq!(unsafe { buffer.as_bytes() }, b"hello");
        assert_eq!(unsafe { buffer.into_vec() }, b"hello");
        assert!(unsafe { Buffer::EMPTY.into_vec() }.is_empty());
    }

    #[test]
    fn version_and_size_mismatches_are_caught() {
        let mut api = PluginApi::empty();
        assert!(api.check().is_err(), "no callbacks yet");
        api.abi_version = ABI_VERSION + 1;
        assert!(api.check().unwrap_err().contains("ABI"));
        api.abi_version = ABI_VERSION;
        api.size = 4;
        assert!(api.check().unwrap_err().contains("bytes"));
    }

    #[test]
    fn status_codes_round_trip() {
        for s in [
            Status::Ok,
            Status::Cancelled,
            Status::StaleHandle,
            Status::Panicked,
        ] {
            assert_eq!(Status::from_code(s as i32), s);
        }
        assert_eq!(Status::from_code(99), Status::Error);
    }
}
