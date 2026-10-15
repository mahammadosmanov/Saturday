use std::collections::HashSet;
use std::ffi::{c_char, c_void};
use std::os::fd::{FromRawFd, RawFd};
use std::ptr::NonNull;

use crate::voice::contract::{
    CaptureError, InputDeviceCatalog, InputDeviceHandle, InputDeviceRecord,
};

const DEVICE_LIMIT: usize = 64;
const LABEL_BYTES: usize = 256;
const HANDLE_BYTES: usize = 16;

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeDeviceRecord {
    handle: [u8; HANDLE_BYTES],
    label_length: u16,
    is_default: u8,
    reserved: u8,
    label: [c_char; LABEL_BYTES],
}

impl Default for NativeDeviceRecord {
    fn default() -> Self {
        Self {
            handle: [0; HANDLE_BYTES],
            label_length: 0,
            is_default: 0,
            reserved: 0,
            label: [0; LABEL_BYTES],
        }
    }
}

#[repr(C)]
struct NativeDeviceCatalog {
    revision: u64,
    count: u32,
    reserved: u32,
    devices: [NativeDeviceRecord; DEVICE_LIMIT],
}

impl Default for NativeDeviceCatalog {
    fn default() -> Self {
        Self {
            revision: 0,
            count: 0,
            reserved: 0,
            devices: [NativeDeviceRecord::default(); DEVICE_LIMIT],
        }
    }
}

#[repr(C)]
struct NativeCaptureOpaque {
    _private: [u8; 0],
}

pub type NativeCaptureEventCallback = unsafe extern "C" fn(
    context: *mut c_void,
    operation_id: u64,
    event: u32,
    elapsed_ms: u32,
    level: f32,
) -> i32;

extern "C" {
    fn saturday_voice_capture_create(out_capture: *mut *mut NativeCaptureOpaque) -> i32;
    fn saturday_voice_capture_destroy(capture: *mut NativeCaptureOpaque);
    fn saturday_voice_capture_enumerate(
        capture: *mut NativeCaptureOpaque,
        out_catalog: *mut NativeDeviceCatalog,
    ) -> i32;
    fn saturday_voice_capture_start(
        capture: *mut NativeCaptureOpaque,
        handle: *const u8,
        operation_id: u64,
        callback: NativeCaptureEventCallback,
        callback_context: *mut c_void,
    ) -> i32;
    fn saturday_voice_capture_finish(
        capture: *mut NativeCaptureOpaque,
        operation_id: u64,
        out_read_fd: *mut i32,
        out_sample_count: *mut u32,
        out_duration_capped: *mut u8,
    ) -> i32;
    fn saturday_voice_capture_cancel(capture: *mut NativeCaptureOpaque, operation_id: u64) -> i32;
}

pub(crate) struct NativePcm {
    pub(crate) file: std::fs::File,
    pub(crate) sample_count: u32,
    pub(crate) duration_capped: bool,
}

pub(crate) struct NativeCapture {
    handle: NonNull<NativeCaptureOpaque>,
}

unsafe impl Send for NativeCapture {}

impl NativeCapture {
    pub(crate) fn new() -> Result<Self, CaptureError> {
        let mut raw = std::ptr::null_mut();
        let status = unsafe { saturday_voice_capture_create(&mut raw) };
        if status != 0 {
            return Err(map_capture_status(status));
        }
        let handle = NonNull::new(raw).ok_or(CaptureError::Unavailable)?;
        Ok(Self { handle })
    }

    pub(crate) fn enumerate_devices(&self) -> Result<InputDeviceCatalog, CaptureError> {
        let mut native = NativeDeviceCatalog::default();
        let status = unsafe { saturday_voice_capture_enumerate(self.handle.as_ptr(), &mut native) };
        if status != 0 {
            return Err(map_capture_status(status));
        }
        let count = usize::try_from(native.count).map_err(|_| CaptureError::Failed)?;
        if count > DEVICE_LIMIT || native.revision == 0 {
            return Err(CaptureError::Failed);
        }

        let mut seen = HashSet::with_capacity(count);
        let mut records = Vec::with_capacity(count);
        let mut default_count = 0usize;
        for entry in native.devices.into_iter().take(count) {
            let label_len = usize::from(entry.label_length);
            if label_len == 0
                || label_len > LABEL_BYTES
                || entry.reserved != 0
                || entry.is_default > 1
            {
                return Err(CaptureError::Failed);
            }
            let label_bytes: Vec<u8> = entry.label[..label_len]
                .iter()
                .map(|byte| *byte as u8)
                .collect();
            let label = String::from_utf8(label_bytes).map_err(|_| CaptureError::Failed)?;
            if label.chars().any(char::is_control) {
                return Err(CaptureError::Failed);
            }
            let handle = encode_handle(entry.handle);
            if !seen.insert(handle.clone()) {
                return Err(CaptureError::Failed);
            }
            let is_default = entry.is_default == 1;
            default_count += usize::from(is_default);
            records.push(InputDeviceRecord {
                handle: InputDeviceHandle(handle),
                label,
                is_default,
            });
        }
        if default_count > 1 {
            return Err(CaptureError::Failed);
        }
        Ok(InputDeviceCatalog {
            revision: native.revision,
            devices: records,
        })
    }

    pub(crate) fn start(
        &mut self,
        device: &InputDeviceHandle,
        operation_id: u64,
        callback: NativeCaptureEventCallback,
        callback_context: *mut c_void,
    ) -> Result<(), CaptureError> {
        let token = decode_handle(&device.0).ok_or(CaptureError::DeviceLost)?;
        let status = unsafe {
            saturday_voice_capture_start(
                self.handle.as_ptr(),
                token.as_ptr(),
                operation_id,
                callback,
                callback_context,
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(map_capture_status(status))
        }
    }

    pub(crate) fn finish(&mut self, operation_id: u64) -> Result<NativePcm, CaptureError> {
        let mut fd: RawFd = -1;
        let mut sample_count = 0;
        let mut duration_capped = 0;
        let status = unsafe {
            saturday_voice_capture_finish(
                self.handle.as_ptr(),
                operation_id,
                &mut fd,
                &mut sample_count,
                &mut duration_capped,
            )
        };
        if status != 0 {
            return Err(map_capture_status(status));
        }
        if fd < 0 || duration_capped > 1 {
            if fd >= 0 {
                unsafe { libc::close(fd) };
            }
            return Err(CaptureError::InvalidAudio);
        }
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        Ok(NativePcm {
            file,
            sample_count,
            duration_capped: duration_capped == 1,
        })
    }

    pub(crate) fn cancel(&mut self, operation_id: u64) -> Result<(), CaptureError> {
        let status = unsafe { saturday_voice_capture_cancel(self.handle.as_ptr(), operation_id) };
        if status == 0 {
            Ok(())
        } else {
            Err(map_capture_status(status))
        }
    }
}

impl Drop for NativeCapture {
    fn drop(&mut self) {
        unsafe { saturday_voice_capture_destroy(self.handle.as_ptr()) };
    }
}

fn encode_handle(handle: [u8; HANDLE_BYTES]) -> String {
    let mut text = String::with_capacity(HANDLE_BYTES * 2);
    for byte in handle {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn decode_handle(text: &str) -> Option<[u8; HANDLE_BYTES]> {
    if text.len() != HANDLE_BYTES * 2 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut handle = [0u8; HANDLE_BYTES];
    for (index, byte) in handle.iter_mut().enumerate() {
        let start = index * 2;
        *byte = u8::from_str_radix(&text[start..start + 2], 16).ok()?;
    }
    Some(handle)
}

fn map_capture_status(status: i32) -> CaptureError {
    match status {
        1 => CaptureError::Unavailable,
        2 => CaptureError::PermissionDenied,
        3 => CaptureError::DeviceLost,
        4 => CaptureError::UnsupportedFormat,
        5 => CaptureError::Overflow,
        6 => CaptureError::InvalidAudio,
        7 => CaptureError::Cancelled,
        8 => CaptureError::Timeout,
        9 => CaptureError::Failed,
        _ => CaptureError::Failed,
    }
}
