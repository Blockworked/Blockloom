//! The C side: the entry function and the table it fills in.

use crate::{Host as PublicHost, Instance, Plugin};
use blockloom_plugin_api::abi::{ABI_VERSION, Buffer, HostApi, PluginApi, Slice, Status};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// The host's table. It outlives the plugin and is only read.
pub struct Host {
    api: *const HostApi,
}

impl Host {
    pub(crate) fn log(&self, level: u32, message: &str) {
        let api = unsafe { &*self.api };
        if let Some(log) = api.log {
            unsafe { log(api.ctx, level, Slice::of(message.as_bytes())) };
        }
    }

    pub(crate) fn call(&self, service: &str, input: &[u8]) -> Result<Vec<u8>, Status> {
        let api = unsafe { &*self.api };
        let call = api.call.ok_or(Status::Unsupported)?;
        let mut out = Buffer::EMPTY;
        let code = unsafe {
            call(
                api.ctx,
                Slice::of(service.as_bytes()),
                Slice::of(input),
                &mut out,
            )
        };
        let bytes = unsafe { out.as_bytes() }.to_vec();
        // The host's buffer is freed by the host.
        if !out.ptr.is_null()
            && let Some(free) = api.free
        {
            unsafe { free(api.ctx, out) };
        }
        match Status::from_code(code) {
            Status::Ok => Ok(bytes),
            status => Err(status),
        }
    }
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "the plugin panicked".to_string())
}

/// The body of `blockloom_plugin_entry_v1`.
///
/// # Safety
/// `host` and `out` must be what the host passes the entry function.
pub unsafe fn entry<P: Plugin>(host: *const HostApi, out: *mut PluginApi) -> i32 {
    if host.is_null() || out.is_null() {
        return Status::BadArgument as i32;
    }
    if let Err(status) = unsafe { &*host }.check() {
        return status as i32;
    }
    let started = catch_unwind(AssertUnwindSafe(|| {
        Instance::<P>::start(PublicHost {
            imp: Host { api: host },
        })
    }));
    let instance = match started {
        Ok(Ok(instance)) => instance,
        Ok(Err(status)) => return status as i32,
        Err(_) => return Status::Panicked as i32,
    };
    let handle = Box::into_raw(Box::new(instance)) as usize as u64;
    unsafe {
        *out = PluginApi {
            size: std::mem::size_of::<PluginApi>() as u32,
            abi_version: ABI_VERSION,
            handle,
            call: Some(call::<P>),
            free_buffer: Some(free_buffer),
            shutdown: Some(shutdown::<P>),
        };
    }
    Status::Ok as i32
}

unsafe extern "C" fn call<P: Plugin>(
    handle: u64,
    op: Slice,
    input: Slice,
    out: *mut Buffer,
) -> i32 {
    if handle == 0 || out.is_null() {
        return Status::BadArgument as i32;
    }
    let instance = unsafe { &mut *(handle as usize as *mut Instance<P>) };
    let result = catch_unwind(AssertUnwindSafe(|| {
        instance.run(unsafe { op.as_bytes() }, unsafe { input.as_bytes() })
    }));
    match result {
        Ok(Ok(bytes)) => {
            unsafe { *out = Buffer::from_vec(bytes) };
            Status::Ok as i32
        }
        Ok(Err(status)) => status as i32,
        Err(payload) => {
            instance.host.error(&panic_text(&*payload));
            Status::Panicked as i32
        }
    }
}

unsafe extern "C" fn free_buffer(_handle: u64, buffer: Buffer) {
    drop(unsafe { buffer.into_vec() });
}

unsafe extern "C" fn shutdown<P: Plugin>(handle: u64) {
    if handle != 0 {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            drop(unsafe { Box::from_raw(handle as usize as *mut Instance<P>) });
        }));
    }
}

/// Exports `blockloom_plugin_entry_v1` for a [`Plugin`].
#[macro_export]
macro_rules! export_plugin {
    ($plugin:ty) => {
        /// # Safety
        /// Called by the host with the tables the C ABI describes.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn blockloom_plugin_entry_v1(
            host: *const $crate::__private::abi::HostApi,
            out: *mut $crate::__private::abi::PluginApi,
        ) -> i32 {
            unsafe { $crate::__private::entry::<$plugin>(host, out) }
        }
    };
}
