//! The WebAssembly side: the four exports and the two imports.

use crate::{Host as PublicHost, Instance, Plugin};
use blockloom_plugin_api::abi::{ABI_VERSION, LOG_ERROR, Status};
use std::alloc::Layout;
use std::cell::UnsafeCell;
use std::sync::Once;

// Names match `blockloom_plugin_api::wasm`: IMPORT_MODULE, IMPORT_LOG, IMPORT_CALL.
#[link(wasm_import_module = "blockloom")]
unsafe extern "C" {
    #[link_name = "log"]
    fn host_log(level: u32, ptr: *const u8, len: usize);
    #[link_name = "call"]
    fn host_call(
        service: *const u8,
        service_len: usize,
        input: *const u8,
        input_len: usize,
        out: *mut [u32; 2],
    ) -> i32;
}

/// The host's imports. They hold no state.
pub struct Host;

impl Host {
    pub(crate) fn log(&self, level: u32, message: &str) {
        unsafe { host_log(level, message.as_ptr(), message.len()) };
    }

    pub(crate) fn call(&self, service: &str, input: &[u8]) -> Result<Vec<u8>, Status> {
        let mut pair = [0u32; 2];
        let code = unsafe {
            host_call(
                service.as_ptr(),
                service.len(),
                input.as_ptr(),
                input.len(),
                &mut pair,
            )
        };
        match Status::from_code(code) {
            Status::Ok => {}
            status => return Err(status),
        }
        let (ptr, len) = (pair[0] as usize as *mut u8, pair[1] as usize);
        if len == 0 {
            return Ok(Vec::new());
        }
        // The host allocated the answer here; it is ours to read and free.
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec();
        unsafe { free(ptr, len) };
        Ok(bytes)
    }
}

/// Where the one instance lives. WebAssembly here is single threaded, and the
/// host never re-enters a module that is mid-call.
pub struct Slot<P>(UnsafeCell<Option<Instance<P>>>);

unsafe impl<P> Sync for Slot<P> {}

impl<P> Slot<P> {
    pub const fn new() -> Slot<P> {
        Slot(UnsafeCell::new(None))
    }
}

impl<P> Default for Slot<P> {
    fn default() -> Self {
        Self::new()
    }
}

/// A panic aborts a module, so say why first; the host then sees a trap.
fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            Host.log(LOG_ERROR, &format!("panic: {info}"));
        }));
    });
}

pub fn abi_version() -> i32 {
    ABI_VERSION as i32
}

pub fn alloc(len: usize) -> *mut u8 {
    match Layout::from_size_align(len.max(1), 1) {
        Ok(layout) => unsafe { std::alloc::alloc(layout) },
        Err(_) => std::ptr::null_mut(),
    }
}

/// # Safety
/// `ptr` and `len` must be what [`alloc`] handed out.
pub unsafe fn free(ptr: *mut u8, len: usize) {
    if !ptr.is_null()
        && let Ok(layout) = Layout::from_size_align(len.max(1), 1)
    {
        unsafe { std::alloc::dealloc(ptr, layout) };
    }
}

/// The body of `blockloom_call`.
///
/// # Safety
/// The pointers must be what the host passes `blockloom_call`.
pub unsafe fn call<P: Plugin>(
    slot: &Slot<P>,
    op: *const u8,
    op_len: usize,
    input: *const u8,
    input_len: usize,
    out: *mut [u32; 2],
) -> i32 {
    install_panic_hook();
    let instance = unsafe { &mut *slot.0.get() };
    if instance.is_none() {
        match Instance::<P>::start(PublicHost { imp: Host }) {
            Ok(started) => *instance = Some(started),
            Err(status) => return status as i32,
        }
    }
    let instance = instance.as_mut().expect("started above");
    let bytes = |ptr: *const u8, len: usize| -> &[u8] {
        if len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(ptr, len) }
        }
    };
    match instance.run(bytes(op, op_len), bytes(input, input_len)) {
        Ok(answer) => {
            let (ptr, len) = if answer.is_empty() {
                (std::ptr::null_mut(), 0)
            } else {
                // len == capacity, so the host's free(ptr, len) is exact.
                let boxed = answer.into_boxed_slice();
                let len = boxed.len();
                (Box::into_raw(boxed) as *mut u8, len)
            };
            unsafe { *out = [ptr as usize as u32, len as u32] };
            Status::Ok as i32
        }
        Err(status) => status as i32,
    }
}

/// Exports the four functions a portable module must provide.
#[macro_export]
macro_rules! export_plugin {
    ($plugin:ty) => {
        static BLOCKLOOM_PLUGIN: $crate::__private::Slot<$plugin> = $crate::__private::Slot::new();

        #[unsafe(no_mangle)]
        pub extern "C" fn blockloom_abi() -> i32 {
            $crate::__private::abi_version()
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn blockloom_alloc(len: usize) -> *mut u8 {
            $crate::__private::alloc(len)
        }

        /// # Safety
        /// Called by the host with a pointer and length from `blockloom_alloc`.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn blockloom_free(ptr: *mut u8, len: usize) {
            unsafe { $crate::__private::free(ptr, len) }
        }

        /// # Safety
        /// Called by the host with pointers into this module's memory.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn blockloom_call(
            op: *const u8,
            op_len: usize,
            input: *const u8,
            input_len: usize,
            out: *mut [u32; 2],
        ) -> i32 {
            unsafe { $crate::__private::call(&BLOCKLOOM_PLUGIN, op, op_len, input, input_len, out) }
        }
    };
}
