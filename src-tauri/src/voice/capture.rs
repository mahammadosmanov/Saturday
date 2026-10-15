use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use crate::platform::voice_capture::{NativeCapture, NativeCaptureEventCallback};
use crate::voice::contract::{
    CaptureError, CaptureLifecycleState, CaptureOperation, CaptureOperationState, CleanupCode,
    CleanupReceipt, InputDeviceCatalog, InputDeviceHandle, NativeCaptureProvider, OwnedPcm,
    TurnContext, VoiceEventKind, VoiceOwner,
};

pub const MAX_RAW_AUDIO_WORKSPACE_BYTES: usize = 8 * 1024 * 1024;
const MAX_CAPTURE_SAMPLES: u32 = crate::voice::contract::MAX_CAPTURE_SAMPLES;
const MAX_PCM_BYTES: usize = MAX_CAPTURE_SAMPLES as usize * std::mem::size_of::<f32>();
const CAPTURE_RING_BYTES: usize = 1024 * 1024;
const CALLBACK_SCRATCH_BYTES: usize = 8192 * 2 * std::mem::size_of::<f32>();
const CONVERTER_INPUT_BYTES: usize = 1024 * 2 * std::mem::size_of::<f32>();
const CONVERTER_OUTPUT_BYTES: usize = 4096 * std::mem::size_of::<f32>();
const PIPE_BUFFER_ALLOWANCE_BYTES: usize = 64 * 1024;
const METER_INTERVAL_MS: u32 = 50;

const _: () = assert!(
    CAPTURE_RING_BYTES
        + CALLBACK_SCRATCH_BYTES
        + CONVERTER_INPUT_BYTES
        + CONVERTER_OUTPUT_BYTES
        + MAX_PCM_BYTES
        + PIPE_BUFFER_ALLOWANCE_BYTES
        <= MAX_RAW_AUDIO_WORKSPACE_BYTES
);

struct CaptureCallbackContext {
    turn: TurnContext,
    last_meter_elapsed_ms: AtomicU32,
}

struct ActiveCapture {
    owner: VoiceOwner,
    operation_id: u64,
    _callback_context: Option<Box<CaptureCallbackContext>>,
    finished: bool,
}

pub struct NativeAudioCaptureProvider {
    native: NativeCapture,
    active: Option<ActiveCapture>,
}

impl NativeAudioCaptureProvider {
    pub fn new() -> Result<Self, CaptureError> {
        Ok(Self {
            native: NativeCapture::new()?,
            active: None,
        })
    }

    fn cancel_active(&mut self, owner: &VoiceOwner, operation_id: Option<u64>) -> CleanupReceipt {
        release_active_capture(
            &mut self.active,
            owner,
            operation_id,
            |active_operation_id| self.native.cancel(active_operation_id),
        )
    }

    pub(crate) fn cancel_operation(
        &mut self,
        owner: &VoiceOwner,
        operation_id: u64,
    ) -> CleanupReceipt {
        self.cancel_active(owner, Some(operation_id))
    }

    fn current_active(&self, owner: &VoiceOwner) -> Result<&ActiveCapture, CaptureError> {
        self.active
            .as_ref()
            .filter(|active| &active.owner == owner)
            .ok_or(CaptureError::DeviceLost)
    }
}
fn release_active_capture(
    active: &mut Option<ActiveCapture>,
    owner: &VoiceOwner,
    operation_id: Option<u64>,
    cancel_native: impl FnOnce(u64) -> Result<(), CaptureError>,
) -> CleanupReceipt {
    let Some(capture) = active.as_ref() else {
        return CleanupReceipt::verified(CleanupCode::NoResourcesOwned);
    };
    if &capture.owner != owner || operation_id.is_some_and(|id| id != capture.operation_id) {
        return CleanupReceipt::failed();
    }
    match cancel_native(capture.operation_id) {
        Ok(()) => {
            *active = None;
            CleanupReceipt::verified(CleanupCode::CaptureStopped)
        }
        Err(_) => CleanupReceipt::failed(),
    }
}

impl NativeCaptureProvider for NativeAudioCaptureProvider {
    fn enumerate_devices(&self) -> Result<InputDeviceCatalog, CaptureError> {
        self.native.enumerate_devices()
    }

    fn start(
        &mut self,
        context: &TurnContext,
        device: InputDeviceHandle,
    ) -> Result<CaptureOperation, CaptureError> {
        if self.active.is_some() {
            return Err(CaptureError::Failed);
        }
        if context.is_cancelled_or_expired() {
            return Err(if context.cancel.is_cancelled() {
                CaptureError::Cancelled
            } else {
                CaptureError::Timeout
            });
        }
        context
            .try_send_event(VoiceEventKind::CaptureLifecycle {
                state: CaptureLifecycleState::PermissionPending,
            })
            .map_err(|_| CaptureError::Failed)?;

        let mut callback_context = Box::new(CaptureCallbackContext {
            turn: context.clone(),
            last_meter_elapsed_ms: AtomicU32::new(0),
        });
        let callback_pointer = (&mut *callback_context as *mut CaptureCallbackContext).cast();
        let operation_id = context.operation_id.0;
        self.native.start(
            &device,
            operation_id,
            capture_event_callback as NativeCaptureEventCallback,
            callback_pointer,
        )?;
        self.active = Some(ActiveCapture {
            owner: context.owner.clone(),
            operation_id,
            _callback_context: Some(callback_context),
            finished: false,
        });
        Ok(CaptureOperation {
            owner: context.owner.clone(),
            operation_id: context.operation_id,
            started_at: Instant::now(),
            state: CaptureOperationState::PermissionPending,
        })
    }

    fn finish(&mut self, owner: &VoiceOwner) -> Result<OwnedPcm, CaptureError> {
        let operation_id = {
            let active = self.current_active(owner)?;
            if active.finished {
                return Err(CaptureError::Failed);
            }
            active.operation_id
        };
        let native_pcm = self.native.finish(operation_id)?;
        // Native finish proves the callback is quiescent, so it is now safe to
        // release its context and permit the next capture. On failure retain
        // both for cancellation/recovery.
        drop(self.active.take());
        OwnedPcm::new(
            native_pcm.file,
            native_pcm.sample_count,
            native_pcm.duration_capped,
        )
    }

    fn cancel(&mut self, owner: &VoiceOwner) -> CleanupReceipt {
        self.cancel_active(owner, None)
    }
}

unsafe extern "C" fn capture_event_callback(
    context: *mut std::ffi::c_void,
    operation_id: u64,
    event: u32,
    elapsed_ms: u32,
    level: f32,
) -> i32 {
    if context.is_null() {
        return 1;
    }
    let callback = &*(context.cast::<CaptureCallbackContext>());
    if callback.turn.operation_id.0 != operation_id || callback.turn.is_cancelled_or_expired() {
        return 1;
    }
    let event = match event {
        1 => VoiceEventKind::CaptureLifecycle {
            state: CaptureLifecycleState::Started,
        },
        2 => VoiceEventKind::CaptureLifecycle {
            state: CaptureLifecycleState::PermissionDenied,
        },
        3 => VoiceEventKind::CaptureLifecycle {
            state: CaptureLifecycleState::DeviceLost,
        },
        4 => VoiceEventKind::CaptureLifecycle {
            state: CaptureLifecycleState::Failed,
        },
        5 => {
            if !level.is_finite()
                || !meter_interval_elapsed(&callback.last_meter_elapsed_ms, elapsed_ms)
            {
                return 0;
            }
            VoiceEventKind::CaptureMeter {
                elapsed_ms,
                level: level.clamp(0.0, 1.0),
            }
        }
        _ => return 1,
    };
    let is_meter = matches!(&event, VoiceEventKind::CaptureMeter { .. });
    match callback.turn.try_send_event(event) {
        Ok(()) => 0,
        Err(_) if is_meter => 0,
        Err(_) => 1,
    }
}

fn meter_interval_elapsed(last_elapsed_ms: &AtomicU32, elapsed_ms: u32) -> bool {
    let mut previous = last_elapsed_ms.load(Ordering::Acquire);
    loop {
        if elapsed_ms.saturating_sub(previous) < METER_INTERVAL_MS {
            return false;
        }
        match last_elapsed_ms.compare_exchange_weak(
            previous,
            elapsed_ms,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return true,
            Err(actual) => previous = actual,
        }
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;

    fn owner(generation: u64) -> VoiceOwner {
        VoiceOwner {
            session_id: "capture-session".to_owned(),
            turn_id: format!("turn-{generation}"),
            generation,
        }
    }

    fn active_capture(owner: VoiceOwner, operation_id: u64) -> Option<ActiveCapture> {
        Some(ActiveCapture {
            owner,
            operation_id,
            _callback_context: None,
            finished: false,
        })
    }

    #[test]
    fn verified_device_loss_release_clears_provider_slot_for_next_capture() {
        let owner = owner(1);
        let operation_id = 7;
        let mut active = active_capture(owner.clone(), operation_id);

        let receipt =
            release_active_capture(&mut active, &owner, Some(operation_id), |cancelled| {
                assert_eq!(cancelled, operation_id);
                Ok(())
            });

        assert_eq!(
            receipt,
            CleanupReceipt::verified(CleanupCode::CaptureStopped)
        );
        assert!(active.is_none(), "a verified release allows the next start");
    }

    #[test]
    fn failed_device_loss_release_retains_provider_slot() {
        let owner = owner(1);
        let operation_id = 7;
        let mut active = active_capture(owner.clone(), operation_id);

        let receipt = release_active_capture(&mut active, &owner, Some(operation_id), |_| {
            Err(CaptureError::DeviceLost)
        });

        assert!(!receipt.verified);
        assert!(
            active.is_some(),
            "unconfirmed native release must keep blocking starts"
        );
    }

    #[test]
    fn stale_device_loss_release_cannot_cancel_a_different_operation() {
        let owner = owner(1);
        let mut active = active_capture(owner.clone(), 8);
        let mut native_cancel_called = false;

        let receipt = release_active_capture(&mut active, &owner, Some(7), |_| {
            native_cancel_called = true;
            Ok(())
        });

        assert!(!receipt.verified);
        assert!(!native_cancel_called);
        assert_eq!(active.as_ref().map(|capture| capture.operation_id), Some(8));
    }
}
