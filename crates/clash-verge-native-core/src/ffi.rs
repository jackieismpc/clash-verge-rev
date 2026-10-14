use crate::{Client, Request, config::Options, lock};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
};
thread_local! {static LAST_ERROR:RefCell<String>=const{RefCell::new(String::new())};}
#[repr(C)]
pub struct CvnHandle {
    _private: [u8; 0],
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CvnBuffer {
    pub data: *const u8,
    pub length: u64,
    pub allocation_id: u64,
    pub status: i32,
}
impl CvnBuffer {
    fn status(status: i32) -> Self {
        Self {
            data: ptr::null(),
            length: 0,
            allocation_id: 0,
            status,
        }
    }
}
enum Handle {
    Open(Arc<Client>),
    Closing(JoinHandle<()>),
    Failed,
}
static HANDLES: Mutex<BTreeMap<usize, Handle>> = Mutex::new(BTreeMap::new());
static BUFFERS: Mutex<BTreeMap<u64, Box<[u8]>>> = Mutex::new(BTreeMap::new());
static NEXT_HANDLE: AtomicUsize = AtomicUsize::new(1);
static NEXT_BUFFER: AtomicU64 = AtomicU64::new(1);
const MAX_BUFFER_BYTES: usize = 128 * 1024 * 1024;
fn error(message: impl ToString) {
    let mut text = message.to_string();
    if text.len() > 4096 {
        let mut end = 4096;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    LAST_ERROR.with(|value| *value.borrow_mut() = text);
}
fn boundary<T>(fallback: T, operation: impl FnOnce() -> T) -> T {
    LAST_ERROR.with(|v| v.borrow_mut().clear());
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(v) => v,
        Err(_) => {
            error("panic contained at native ABI boundary");
            fallback
        }
    }
}
fn bytes(value: Vec<u8>) -> CvnBuffer {
    let mut buffers = lock(&BUFFERS);
    if value.len() > MAX_BUFFER_BYTES
        || buffers.len() >= 64
        || buffers.values().map(|v| v.len()).sum::<usize>() + value.len() > MAX_BUFFER_BYTES
    {
        error("native output buffer capacity exceeded");
        return CvnBuffer::status(2);
    }
    let Ok(id) = NEXT_BUFFER.try_update(Ordering::Relaxed, Ordering::Relaxed, |value| value.checked_add(1)) else {
        error("allocation IDs exhausted");
        return CvnBuffer::status(5);
    };
    let value = value.into_boxed_slice();
    let buffer = CvnBuffer {
        data: value.as_ptr(),
        length: value.len() as u64,
        allocation_id: id,
        status: 0,
    };
    buffers.insert(id, value);
    buffer
}
fn allocated(value: impl serde::Serialize) -> CvnBuffer {
    match serde_json::to_vec(&value) {
        Ok(value) => bytes(value),
        Err(reason) => {
            error(reason);
            CvnBuffer::status(5)
        }
    }
}
unsafe fn input<'a>(value: *const u8, length: u64, maximum: u64) -> Result<&'a str, &'static str> {
    if length > maximum {
        return Err("native input length exceeds limit");
    }
    if value.is_null() {
        return Err("null input span");
    }
    std::str::from_utf8(unsafe { std::slice::from_raw_parts(value, length as usize) })
        .map_err(|_| "native input is not UTF-8")
}
fn reap_closed(handles: &mut BTreeMap<usize, Handle>) {
    let ready: Vec<_> = handles
        .iter()
        .filter_map(|(id, handle)| match handle {
            Handle::Closing(task) if task.is_finished() => Some(*id),
            _ => None,
        })
        .collect();
    for id in ready {
        if let Some(Handle::Closing(task)) = handles.remove(&id)
            && task.join().is_err()
        {
            handles.insert(id, Handle::Failed);
        }
    }
}
fn client(handle: *mut CvnHandle) -> Option<Arc<Client>> {
    match lock(&HANDLES).get(&(handle as usize)) {
        Some(Handle::Open(client)) => Some(Arc::clone(client)),
        _ => {
            error("invalid or closing native handle");
            None
        }
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn cvn_abi_version() -> u32 {
    2
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cvn_create(value: *const u8, length: u64) -> *mut CvnHandle {
    boundary(ptr::null_mut(), || {
        let result = (|| {
            let text = unsafe { input(value, length, 65536) }.map_err(anyhow::Error::msg)?;
            let options: Options = serde_json::from_str(text)?;
            Client::open(options)
        })();
        match result {
            Ok(value) => {
                let client = Arc::new(value);
                let mut handles = lock(&HANDLES);
                reap_closed(&mut handles);
                if handles.len() >= 16 {
                    drop(handles);
                    drop(client);
                    error("native handle capacity exceeded");
                    return ptr::null_mut();
                }
                let Ok(id) = NEXT_HANDLE.try_update(Ordering::Relaxed, Ordering::Relaxed, |value| value.checked_add(1))
                else {
                    drop(handles);
                    drop(client);
                    error("handle IDs exhausted");
                    return ptr::null_mut();
                };
                handles.insert(id, Handle::Open(client));
                ptr::without_provenance_mut(id)
            }
            Err(reason) => {
                error(format!("{reason:#}"));
                ptr::null_mut()
            }
        }
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn cvn_last_error() -> CvnBuffer {
    let message = LAST_ERROR.with(|v| v.borrow().clone());
    catch_unwind(AssertUnwindSafe(|| bytes(message.into_bytes()))).unwrap_or(CvnBuffer::status(5))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cvn_submit(handle: *mut CvnHandle, value: *const u8, length: u64) -> i32 {
    boundary(5, || {
        let Some(client) = client(handle) else {
            return 1;
        };
        match unsafe { input(value, length, 64 * 1024 * 1024) }
            .map_err(str::to_string)
            .and_then(|text| serde_json::from_str::<Request>(text).map_err(|e| e.to_string()))
        {
            Ok(request) => {
                let status = client.submit(request);
                if status != 0 {
                    error(format!("request rejected with status {status}"));
                }
                status
            }
            Err(reason) => {
                error(reason);
                1
            }
        }
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cvn_cancel(handle: *mut CvnHandle, value: *const u8, length: u64) -> i32 {
    boundary(5, || {
        let Some(client) = client(handle) else {
            return 1;
        };
        match unsafe { input(value, length, 256) } {
            Ok(id) if client.cancel(id) => 0,
            Ok(_) => {
                error("request is no longer pending");
                1
            }
            Err(reason) => {
                error(reason);
                1
            }
        }
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn cvn_snapshot(handle: *mut CvnHandle) -> CvnBuffer {
    boundary(CvnBuffer::status(5), || {
        client(handle).map_or(CvnBuffer::status(1), |client| allocated(client.snapshot()))
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn cvn_next_event(handle: *mut CvnHandle) -> CvnBuffer {
    boundary(CvnBuffer::status(5), || {
        let Some(client) = client(handle) else {
            return CvnBuffer::status(1);
        };
        let mut events = lock(&client.shared.events);
        let Some(event) = events.front() else {
            return CvnBuffer::status(4);
        };
        let buffer = allocated(&event.value);
        if buffer.status == 0 {
            events.pop_front();
        }
        buffer
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn cvn_free_buffer(value: CvnBuffer) -> i32 {
    boundary(5, || {
        if value.allocation_id == 0 {
            return if value.data.is_null() && value.length == 0 {
                0
            } else {
                1
            };
        }
        let mut buffers = lock(&BUFFERS);
        if let Some(buffer) = buffers.get(&value.allocation_id) {
            if buffer.as_ptr() != value.data || buffer.len() as u64 != value.length {
                error("native buffer ownership mismatch");
                return 1;
            }
            buffers.remove(&value.allocation_id);
        }
        0
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn cvn_stop(handle: *mut CvnHandle) -> i32 {
    boundary(5, || {
        let Some(client) = client(handle) else {
            return 1;
        };
        client.stop();
        0
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn cvn_destroy(handle: *mut CvnHandle) -> i32 {
    boundary(5, || {
        let mut handles = lock(&HANDLES);
        let id = handle as usize;
        match handles.remove(&id) {
            None => 0,
            Some(Handle::Open(client)) => {
                client.stop();
                let backup = Arc::clone(&client);
                match std::thread::Builder::new()
                    .name("native-cleanup".into())
                    .spawn(move || {
                        while Arc::strong_count(&client) > 1 {
                            std::thread::sleep(std::time::Duration::from_millis(1));
                        }
                        drop(client);
                    }) {
                    Ok(task) => {
                        handles.insert(id, Handle::Closing(task));
                        2
                    }
                    Err(reason) => {
                        handles.insert(id, Handle::Open(backup));
                        error(reason);
                        5
                    }
                }
            }
            Some(Handle::Closing(task)) if task.is_finished() => {
                drop(handles);
                if task.join().is_err() {
                    lock(&HANDLES).insert(id, Handle::Failed);
                    error("native cleanup panicked; keep the library loaded");
                    5
                } else {
                    0
                }
            }
            Some(Handle::Failed) => {
                handles.insert(id, Handle::Failed);
                5
            }
            Some(Handle::Closing(task)) => {
                handles.insert(id, Handle::Closing(task));
                2
            }
        }
    })
}
