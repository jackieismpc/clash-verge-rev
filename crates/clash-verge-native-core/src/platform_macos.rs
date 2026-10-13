use super::{NetworkBackend, NetworkMode, NetworkValue};
use anyhow::{Result, bail};
use serde_json::{Map, Value};
use std::{
    ffi::{CString, c_char, c_void},
    ptr,
};
type Ref = *const c_void;
const CF_UTF8: u32 = 0x08000100;
const CF_SINT64: isize = 4;
const CF_DOUBLE: isize = 13;
fn number_encoding(value: &serde_json::Number) -> Result<(isize, Option<i64>)> {
    if let Some(integer) = value.as_i64() {
        return Ok((CF_SINT64, Some(integer)));
    }
    if value.is_u64() {
        bail!("unsigned network integer exceeds signed CoreFoundation range");
    }
    Ok((CF_DOUBLE, None))
}
fn valid_string_snapshot(value: &str, utf16_length: isize) -> bool {
    !value.contains('\0') && value.encode_utf16().count() as isize == utf16_length
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: Ref);
    fn CFGetTypeID(value: Ref) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberIsFloatType(number: Ref) -> u8;
    fn CFBooleanGetTypeID() -> usize;
    fn CFArrayGetTypeID() -> usize;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFStringCreateWithCString(allocator: Ref, text: *const c_char, encoding: u32) -> Ref;
    fn CFStringGetLength(string: Ref) -> isize;
    fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
    fn CFStringGetCString(string: Ref, buffer: *mut c_char, size: isize, encoding: u32) -> u8;
    fn CFNumberGetValue(number: Ref, kind: isize, output: *mut c_void) -> u8;
    fn CFNumberCreate(allocator: Ref, kind: isize, value: *const c_void) -> Ref;
    fn CFBooleanGetValue(value: Ref) -> u8;
    static kCFBooleanTrue: Ref;
    static kCFBooleanFalse: Ref;
    fn CFArrayGetCount(array: Ref) -> isize;
    fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
    fn CFArrayCreate(allocator: Ref, values: *const Ref, count: isize, callbacks: Ref) -> Ref;
    fn CFDictionaryGetCount(dictionary: Ref) -> isize;
    fn CFDictionaryGetKeysAndValues(dictionary: Ref, keys: *mut Ref, values: *mut Ref);
    fn CFDictionaryCreate(
        allocator: Ref,
        keys: *const Ref,
        values: *const Ref,
        count: isize,
        key_callbacks: Ref,
        value_callbacks: Ref,
    ) -> Ref;
    static kCFTypeArrayCallBacks: [usize; 5];
    static kCFTypeDictionaryKeyCallBacks: [usize; 6];
    static kCFTypeDictionaryValueCallBacks: [usize; 5];
    fn CFRetain(value: Ref) -> Ref;
}
#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCPreferencesCreate(allocator: Ref, name: Ref, prefs_id: Ref) -> Ref;
    fn SCPreferencesCreateWithAuthorization(
        allocator: Ref,
        name: Ref,
        prefs_id: Ref,
        authorization: *mut c_void,
    ) -> Ref;
    fn SCPreferencesLock(preferences: Ref, wait: u8) -> u8;
    fn SCPreferencesUnlock(preferences: Ref) -> u8;
    fn SCPreferencesCommitChanges(preferences: Ref) -> u8;
    fn SCPreferencesApplyChanges(preferences: Ref) -> u8;
    fn SCNetworkServiceCopyAll(preferences: Ref) -> Ref;
    fn SCNetworkServiceGetServiceID(service: Ref) -> Ref;
    fn SCNetworkServiceGetEnabled(service: Ref) -> u8;
    fn SCNetworkServiceCopyProtocol(service: Ref, protocol: Ref) -> Ref;
    fn SCNetworkProtocolGetConfiguration(protocol: Ref) -> Ref;
    fn SCNetworkProtocolSetConfiguration(protocol: Ref, configuration: Ref) -> u8;
    fn SCError() -> i32;
}
#[repr(C)]
struct AuthorizationItem {
    name: *const c_char,
    value_length: usize,
    value: *mut c_void,
    flags: u32,
}
#[repr(C)]
struct AuthorizationItems {
    count: u32,
    items: *mut AuthorizationItem,
}
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn AuthorizationCreate(
        rights: *const AuthorizationItems,
        environment: *const AuthorizationItems,
        flags: u32,
        output: *mut *mut c_void,
    ) -> i32;
    fn AuthorizationCopyRights(
        authorization: *mut c_void,
        rights: *const AuthorizationItems,
        environment: *const AuthorizationItems,
        flags: u32,
        output: *mut *mut AuthorizationItems,
    ) -> i32;
    fn AuthorizationFree(authorization: *mut c_void, flags: u32) -> i32;
}
struct Authorization(*mut c_void);
// Authorization Services references are process-local handles, serialized by the business runtime.
unsafe impl Send for Authorization {}
impl Drop for Authorization {
    fn drop(&mut self) {
        unsafe {
            AuthorizationFree(self.0, 0);
        }
    }
}
struct Owned(Ref);
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CFRelease(self.0);
            }
        }
    }
}
fn owned(value: Ref) -> Result<Owned> {
    if value.is_null() {
        bail!("macOS framework returned null (SCError {})", unsafe { SCError() });
    }
    Ok(Owned(value))
}
fn string(text: &str) -> Result<Owned> {
    let text = CString::new(text)?;
    owned(unsafe { CFStringCreateWithCString(ptr::null(), text.as_ptr(), CF_UTF8) })
}
fn text(value: Ref) -> Result<String> {
    if value.is_null() || unsafe { CFGetTypeID(value) != CFStringGetTypeID() } {
        bail!("expected CFString");
    }
    let size = unsafe { CFStringGetMaximumSizeForEncoding(CFStringGetLength(value), CF_UTF8) } + 1;
    let mut bytes = vec![0_u8; size as usize];
    if unsafe { CFStringGetCString(value, bytes.as_mut_ptr().cast(), size, CF_UTF8) } == 0 {
        bail!("invalid CFString");
    }
    let length = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    let decoded = String::from_utf8(bytes[..length].to_vec())?;
    if !valid_string_snapshot(&decoded, unsafe { CFStringGetLength(value) }) {
        bail!("network CFString snapshot would lose embedded null or characters");
    }
    Ok(decoded)
}
fn decode(value: Ref, depth: usize) -> Result<Value> {
    if depth > 32 {
        bail!("network dictionary exceeds nesting limit");
    }
    if value.is_null() {
        return Ok(Value::Null);
    }
    let kind = unsafe { CFGetTypeID(value) };
    unsafe {
        if kind == CFStringGetTypeID() {
            return Ok(Value::String(text(value)?));
        }
        if kind == CFBooleanGetTypeID() {
            return Ok(Value::Bool(CFBooleanGetValue(value) != 0));
        }
        if kind == CFNumberGetTypeID() && CFNumberIsFloatType(value) == 0 {
            let mut number: i64 = 0;
            if CFNumberGetValue(value, CF_SINT64, (&mut number as *mut i64).cast()) == 0 {
                bail!("invalid integer");
            }
            return Ok(serde_json::json!(number));
        }
        if kind == CFNumberGetTypeID() {
            let mut number: f64 = 0.0;
            if CFNumberGetValue(value, CF_DOUBLE, (&mut number as *mut f64).cast()) == 0 {
                bail!("invalid CFNumber");
            }
            return serde_json::Number::from_f64(number)
                .map(Value::Number)
                .ok_or_else(|| anyhow::anyhow!("non-finite network CFNumber"));
        }
        if kind == CFArrayGetTypeID() {
            let mut values = vec![];
            for i in 0..CFArrayGetCount(value) {
                values.push(decode(CFArrayGetValueAtIndex(value, i), depth + 1)?);
            }
            return Ok(Value::Array(values));
        }
        if kind == CFDictionaryGetTypeID() {
            let count = CFDictionaryGetCount(value) as usize;
            let mut keys = vec![ptr::null(); count];
            let mut values = vec![ptr::null(); count];
            CFDictionaryGetKeysAndValues(value, keys.as_mut_ptr(), values.as_mut_ptr());
            let mut object = Map::new();
            for (key, value) in keys.into_iter().zip(values) {
                object.insert(text(key)?, decode(value, depth + 1)?);
            }
            return Ok(Value::Object(object));
        }
    }
    // Refuse lossy ownership snapshots rather than dropping unsupported data/date values.
    bail!("unsupported network property list type {kind}")
}
fn encode(value: &Value) -> Result<Owned> {
    match value {
        Value::String(value) => string(value),
        Value::Bool(value) => owned(unsafe { CFRetain(if *value { kCFBooleanTrue } else { kCFBooleanFalse }) }),
        Value::Number(value) => {
            let (kind, integer) = number_encoding(value)?;
            if let Some(integer) = integer {
                owned(unsafe { CFNumberCreate(ptr::null(), kind, (&integer as *const i64).cast()) })
            } else {
                let number = value
                    .as_f64()
                    .filter(|number| number.is_finite())
                    .ok_or_else(|| anyhow::anyhow!("invalid floating-point network number"))?;
                owned(unsafe { CFNumberCreate(ptr::null(), kind, (&number as *const f64).cast()) })
            }
        }
        Value::Array(values) => {
            let values: Vec<_> = values.iter().map(encode).collect::<Result<_>>()?;
            let pointers: Vec<_> = values.iter().map(|value| value.0).collect();
            owned(unsafe {
                CFArrayCreate(
                    ptr::null(),
                    pointers.as_ptr(),
                    pointers.len() as isize,
                    ptr::addr_of!(kCFTypeArrayCallBacks).cast(),
                )
            })
        }
        Value::Object(values) => {
            let owned_keys: Vec<_> = values.keys().map(|key| string(key)).collect::<Result<_>>()?;
            let values: Vec<_> = values.values().map(encode).collect::<Result<_>>()?;
            let keys: Vec<_> = owned_keys.iter().map(|key| key.0).collect();
            let pointers: Vec<_> = values.iter().map(|value| value.0).collect();
            owned(unsafe {
                CFDictionaryCreate(
                    ptr::null(),
                    keys.as_ptr(),
                    pointers.as_ptr(),
                    keys.len() as isize,
                    ptr::addr_of!(kCFTypeDictionaryKeyCallBacks).cast(),
                    ptr::addr_of!(kCFTypeDictionaryValueCallBacks).cast(),
                )
            })
        }
        Value::Null => bail!("null is not a network dictionary value"),
    }
}

/// Independent preferences session; no shell subprocesses, helper installation or implicit writes.
pub struct MacNetwork {
    mode: NetworkMode,
    authorization: Option<Authorization>,
}
impl MacNetwork {
    pub fn new(mode: NetworkMode) -> Self {
        Self {
            mode,
            authorization: None,
        }
    }
    /// Only call in response to an explicit native UI networking operation.
    /// Cancelling the administrator prompt leaves the network unchanged.
    pub fn authorize_interactive(&mut self) -> Result<()> {
        if self.mode == NetworkMode::IntegrationTest {
            bail!("integrationTest forbids administrator authorization");
        }
        let mut raw = ptr::null_mut();
        let status = unsafe { AuthorizationCreate(ptr::null(), ptr::null(), 0, &mut raw) };
        if status != 0 || raw.is_null() {
            bail!("cannot create administrator authorization ({status})");
        }
        let authorization = Authorization(raw);
        let right = CString::new("system.preferences.network")?;
        let mut item = AuthorizationItem {
            name: right.as_ptr(),
            value_length: 0,
            value: ptr::null_mut(),
            flags: 0,
        };
        let rights = AuthorizationItems {
            count: 1,
            items: &mut item,
        };
        let status = unsafe { AuthorizationCopyRights(authorization.0, &rights, ptr::null(), 3, ptr::null_mut()) };
        if status != 0 {
            bail!("administrator authorization denied or cancelled ({status})");
        }
        self.authorization = Some(authorization);
        Ok(())
    }
    fn preferences(&self) -> Result<Owned> {
        let name = string("independent.native.mac.network")?;
        owned(unsafe {
            if let Some(authorization) = &self.authorization {
                SCPreferencesCreateWithAuthorization(ptr::null(), name.0, ptr::null(), authorization.0)
            } else {
                SCPreferencesCreate(ptr::null(), name.0, ptr::null())
            }
        })
    }
    pub fn services(&mut self) -> Result<Vec<String>> {
        let preferences = self.preferences()?;
        let services = owned(unsafe { SCNetworkServiceCopyAll(preferences.0) })?;
        let mut identifiers = vec![];
        for i in 0..unsafe { CFArrayGetCount(services.0) } {
            let service = unsafe { CFArrayGetValueAtIndex(services.0, i) };
            if unsafe { SCNetworkServiceGetEnabled(service) } != 0 {
                identifiers.push(text(unsafe { SCNetworkServiceGetServiceID(service) })?);
            }
        }
        Ok(identifiers)
    }
    fn protocol(&self, preferences: Ref, service_id: &str, protocol: &str) -> Result<Owned> {
        if protocol != "Proxies" && protocol != "DNS" {
            bail!("unsupported protocol");
        }
        let services = owned(unsafe { SCNetworkServiceCopyAll(preferences) })?;
        let protocol_name = string(protocol)?;
        for i in 0..unsafe { CFArrayGetCount(services.0) } {
            let service = unsafe { CFArrayGetValueAtIndex(services.0, i) };
            if text(unsafe { SCNetworkServiceGetServiceID(service) })? == service_id {
                return owned(unsafe { SCNetworkServiceCopyProtocol(service, protocol_name.0) });
            }
        }
        bail!("network service no longer exists")
    }
}
impl NetworkBackend for MacNetwork {
    fn read(&mut self, service: &str, protocol: &str) -> Result<NetworkValue> {
        let preferences = self.preferences()?;
        let handle = self.protocol(preferences.0, service, protocol)?;
        let raw = unsafe { SCNetworkProtocolGetConfiguration(handle.0) };
        Ok(NetworkValue {
            service: service.into(),
            protocol: protocol.into(),
            configuration: if raw.is_null() { None } else { Some(decode(raw, 0)?) },
        })
    }
    fn write(&mut self, value: &NetworkValue) -> Result<()> {
        self.locked_write(None, value).map(|_| ())
    }
    fn compare_and_write(&mut self, expected: &NetworkValue, desired: &NetworkValue) -> Result<bool> {
        self.locked_write(Some(expected), desired)
    }
}
impl MacNetwork {
    fn locked_write(&mut self, expected: Option<&NetworkValue>, value: &NetworkValue) -> Result<bool> {
        if self.mode == NetworkMode::IntegrationTest {
            bail!("integrationTest forbids SystemConfiguration writes");
        }
        if value
            .configuration
            .as_ref()
            .is_some_and(|configuration| !configuration.is_object())
        {
            bail!("network configuration must be a dictionary");
        }
        let preferences = self.preferences()?;
        if unsafe { SCPreferencesLock(preferences.0, 0) } == 0 {
            bail!("cannot lock macOS network preferences (SCError {})", unsafe {
                SCError()
            });
        }
        let result = (|| {
            let handle = self.protocol(preferences.0, &value.service, &value.protocol)?;
            if let Some(expected) = expected {
                let raw = unsafe { SCNetworkProtocolGetConfiguration(handle.0) };
                let current = if raw.is_null() { None } else { Some(decode(raw, 0)?) };
                if current != expected.configuration {
                    return Ok(false);
                }
            }
            let configuration = value.configuration.as_ref().map(encode).transpose()?;
            if unsafe {
                SCNetworkProtocolSetConfiguration(handle.0, configuration.as_ref().map_or(ptr::null(), |value| value.0))
            } == 0
            {
                bail!("macOS rejected network configuration");
            }
            if unsafe { SCPreferencesCommitChanges(preferences.0) } == 0 {
                bail!("macOS network commit needs authorization (SCError {})", unsafe {
                    SCError()
                });
            }
            if unsafe { SCPreferencesApplyChanges(preferences.0) } == 0 {
                bail!("macOS network apply failed (SCError {})", unsafe { SCError() });
            }
            Ok(true)
        })();
        unsafe {
            SCPreferencesUnlock(preferences.0);
        }
        result
    }
}

#[cfg(test)]
mod encoding_tests {
    use super::*;
    #[test]
    fn numbers_preserve_signed_integer_precision_without_os_calls() {
        for integer in [i64::MIN, -1, 0, 7890, i64::MAX] {
            assert_eq!(number_encoding(&integer.into()).unwrap(), (CF_SINT64, Some(integer)));
        }
        assert!(number_encoding(&u64::MAX.into()).is_err());
        assert_eq!(
            number_encoding(&serde_json::Number::from_f64(1.25).unwrap()).unwrap(),
            (CF_DOUBLE, None)
        );
    }
    #[test]
    fn utf8_snapshot_preserves_utf16_length_and_rejects_truncation() {
        assert!(valid_string_snapshot("节点🌐", 4));
        assert!(!valid_string_snapshot("before", 10));
        assert!(!valid_string_snapshot("before\0after", 12));
    }
}
