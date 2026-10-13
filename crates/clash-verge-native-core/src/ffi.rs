use crate::{Client, Request, config::Options};
use std::{
    cell::RefCell,
    ffi::{CStr, CString, c_char},
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
};

thread_local! { static LAST_ERROR: RefCell<String> = const { RefCell::new(String::new()) }; }

fn error(message: impl ToString) {
    LAST_ERROR.with(|value| *value.borrow_mut() = message.to_string());
}
fn allocated(value: impl serde::Serialize) -> *mut c_char {
    serde_json::to_string(&value)
        .ok()
        .and_then(|text| CString::new(text).ok())
        .map_or(ptr::null_mut(), CString::into_raw)
}
fn boundary<T>(fallback: T, operation: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(value) => value,
        Err(_) => {
            error("panic contained at native ABI boundary");
            fallback
        }
    }
}
unsafe fn input<'a>(value: *const c_char) -> Result<&'a str, &'static str> {
    if value.is_null() {
        return Err("null string");
    }
    let text = unsafe { CStr::from_ptr(value) }
        .to_str()
        .map_err(|_| "input is not UTF-8")?;
    if text.len() > 64 * 1024 * 1024 {
        return Err("input exceeds 64 MiB limit");
    }
    Ok(text)
}

#[unsafe(no_mangle)]
pub extern "C" fn cvn_abi_version() -> u32 {
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cvn_create(config_json: *const c_char) -> *mut Client {
    boundary(ptr::null_mut(), || {
        let result = (|| {
            let text = unsafe { input(config_json) }.map_err(anyhow::Error::msg)?;
            let options: Options = serde_json::from_str(text)?;
            Client::open(options)
        })();
        match result {
            Ok(client) => Box::into_raw(Box::new(client)),
            Err(reason) => {
                error(format!("{reason:#}"));
                ptr::null_mut()
            }
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn cvn_last_error() -> *mut c_char {
    boundary(ptr::null_mut(), || {
        LAST_ERROR.with(|value| allocated(value.borrow().clone()))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cvn_submit(handle: *mut Client, request_json: *const c_char) -> i32 {
    boundary(1, || {
        let Some(client) = (unsafe { handle.as_ref() }) else {
            error("null handle");
            return 1;
        };
        let request = unsafe { input(request_json) }
            .map_err(str::to_string)
            .and_then(|text| serde_json::from_str::<Request>(text).map_err(|e| e.to_string()));
        match request {
            Ok(request) => client.submit(request),
            Err(reason) => {
                error(reason);
                1
            }
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cvn_cancel(handle: *mut Client, request_id: *const c_char) -> i32 {
    boundary(1, || {
        let Some(client) = (unsafe { handle.as_ref() }) else {
            return 1;
        };
        match unsafe { input(request_id) } {
            Ok(id) if client.cancel(id) => 0,
            _ => 1,
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cvn_snapshot(handle: *mut Client) -> *mut c_char {
    boundary(ptr::null_mut(), || {
        unsafe { handle.as_ref() }.map_or(ptr::null_mut(), |client| allocated(client.snapshot()))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cvn_next_event(handle: *mut Client) -> *mut c_char {
    boundary(ptr::null_mut(), || {
        unsafe { handle.as_ref() }
            .and_then(Client::next_event)
            .map_or(ptr::null_mut(), allocated)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cvn_free_string(value: *mut c_char) {
    if !value.is_null() {
        boundary((), || drop(unsafe { CString::from_raw(value) }));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cvn_stop(handle: *mut Client) {
    boundary((), || {
        if let Some(client) = unsafe { handle.as_ref() } {
            client.stop();
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cvn_destroy(handle: *mut Client) {
    boundary((), || {
        if !handle.is_null() {
            drop(unsafe { Box::from_raw(handle) });
        }
    });
}
