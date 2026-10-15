use crate::voice::contract::{AssetError, CancelToken};
use std::ffi::{c_char, c_int, CString};
use std::fs::File;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::os::fd::AsRawFd;
    const TRANSFER_POLL_INTERVAL: Duration = Duration::from_millis(25);

    const PENDING: u32 = 0;
    const OK: u32 = 1;
    const CANCELLED: u32 = 2;
    const TIMEOUT: u32 = 3;
    const NETWORK_ERROR: u32 = 4;
    const RESPONSE_REJECTED: u32 = 5;
    const SIZE_REJECTED: u32 = 6;
    const STORAGE_ERROR: u32 = 7;
    const REDIRECT_REJECTED: u32 = 8;
    const INVALID_ARGUMENT: u32 = 9;

    #[repr(C)]
    struct SaturdayAssetTransferHandle {
        _private: [u8; 0],
    }

    #[link(name = "saturday_voice_assets", kind = "static")]
    unsafe extern "C" {
        fn saturday_asset_transfer_start(
            url: *const c_char,
            allowed_origins: *const *const c_char,
            allowed_origin_count: usize,
            destination_fd: c_int,
            expected_bytes: u64,
            timeout_milliseconds: u32,
            out_handle: *mut *mut SaturdayAssetTransferHandle,
        ) -> u32;
        fn saturday_asset_transfer_poll(
            handle: *mut SaturdayAssetTransferHandle,
            out_bytes_received: *mut u64,
            out_result: *mut u32,
            out_complete: *mut u32,
        ) -> c_int;
        fn saturday_asset_transfer_cancel_and_wait(
            handle: *mut SaturdayAssetTransferHandle,
            out_bytes_received: *mut u64,
            out_result: *mut u32,
        ) -> c_int;
        fn saturday_asset_transfer_destroy(handle: *mut SaturdayAssetTransferHandle);
    }

    struct TransferHandle(NonNull<SaturdayAssetTransferHandle>);

    impl TransferHandle {
        fn raw(&self) -> *mut SaturdayAssetTransferHandle {
            self.0.as_ptr()
        }

        fn cancel_and_wait(&self) -> Result<(u64, u32), AssetError> {
            let mut bytes = 0;
            let mut result = PENDING;
            // SAFETY: the handle remains live until this guard is dropped; native cancellation
            // waits for NSURLSession completion and closes its duplicated writer descriptor.
            if unsafe {
                saturday_asset_transfer_cancel_and_wait(self.raw(), &mut bytes, &mut result)
            } != 0
            {
                return Err(AssetError::CleanupFailed);
            }
            Ok((bytes, result))
        }
    }

    impl Drop for TransferHandle {
        fn drop(&mut self) {
            let mut bytes = 0;
            let mut result = PENDING;
            // SAFETY: this guard uniquely owns the opaque handle and cancellation joins the writer.
            unsafe {
                let _ =
                    saturday_asset_transfer_cancel_and_wait(self.raw(), &mut bytes, &mut result);
                saturday_asset_transfer_destroy(self.raw());
            }
        }
    }

    fn result_to_error(result: u32) -> AssetError {
        match result {
            CANCELLED => AssetError::Cancelled,
            TIMEOUT => AssetError::Timeout,
            STORAGE_ERROR => AssetError::StorageFailure,
            INVALID_ARGUMENT => AssetError::InvalidBinding,
            NETWORK_ERROR | RESPONSE_REJECTED | SIZE_REJECTED | REDIRECT_REJECTED => {
                AssetError::Failed
            }
            _ => AssetError::Failed,
        }
    }

    pub fn transfer_to_fd(
        url: &str,
        allowed_redirect_origins: &[String],
        destination: &File,
        expected_bytes: u64,
        cancellation: &CancelToken,
        deadline: Instant,
    ) -> Result<u64, AssetError> {
        if cancellation.is_cancelled() {
            return Err(AssetError::Cancelled);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(AssetError::Timeout);
        }
        let timeout_milliseconds = u32::try_from(remaining.as_millis().clamp(1, 900_000))
            .map_err(|_| AssetError::Timeout)?;
        let url = CString::new(url).map_err(|_| AssetError::InvalidBinding)?;
        let origins = allowed_redirect_origins
            .iter()
            .map(|origin| CString::new(origin.as_str()).map_err(|_| AssetError::InvalidBinding))
            .collect::<Result<Vec<_>, _>>()?;
        let origin_pointers = origins
            .iter()
            .map(|origin| origin.as_ptr())
            .collect::<Vec<_>>();
        let mut raw = std::ptr::null_mut();
        // SAFETY: the URL/origin strings are NUL-terminated for the duration of the call; the
        // destination descriptor is live and the native bridge duplicates it before returning.
        let start_result = unsafe {
            saturday_asset_transfer_start(
                url.as_ptr(),
                origin_pointers.as_ptr(),
                origin_pointers.len(),
                destination.as_raw_fd(),
                expected_bytes,
                timeout_milliseconds,
                &mut raw,
            )
        };
        if start_result != PENDING {
            return Err(result_to_error(start_result));
        }
        let handle = TransferHandle(NonNull::new(raw).ok_or(AssetError::Failed)?);
        loop {
            if cancellation.is_cancelled() {
                handle.cancel_and_wait()?;
                return Err(AssetError::Cancelled);
            }
            if Instant::now() >= deadline {
                handle.cancel_and_wait()?;
                return Err(AssetError::Timeout);
            }
            let mut bytes = 0;
            let mut result = PENDING;
            let mut complete = 0;
            // SAFETY: the transfer handle is owned by `handle` and output pointers are valid.
            if unsafe {
                saturday_asset_transfer_poll(handle.raw(), &mut bytes, &mut result, &mut complete)
            } != 0
            {
                handle.cancel_and_wait()?;
                return Err(AssetError::Failed);
            }
            if complete != 0 {
                return if result == OK && bytes == expected_bytes {
                    Ok(bytes)
                } else if result == OK {
                    Err(AssetError::DigestMismatch)
                } else {
                    Err(result_to_error(result))
                };
            }
            std::thread::sleep(
                TRANSFER_POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
}

#[cfg(target_os = "macos")]
pub use macos::transfer_to_fd;

#[cfg(not(target_os = "macos"))]
pub fn transfer_to_fd(
    _url: &str,
    _allowed_redirect_origins: &[String],
    _destination: &File,
    _expected_bytes: u64,
    _cancellation: &CancelToken,
    _deadline: Instant,
) -> Result<u64, AssetError> {
    Err(AssetError::Unavailable)
}
