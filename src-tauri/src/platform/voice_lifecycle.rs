#[cfg(target_os = "macos")]
use std::sync::OnceLock;
#[cfg(target_os = "macos")]
use tauri::Manager;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleError {
    AlreadyStarted,
    RegistrationFailed,
    UnsupportedPlatform,
}

#[cfg(target_os = "macos")]
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn saturday_voice_lifecycle_start(callback: extern "C" fn()) -> i32;
}

#[cfg(target_os = "macos")]
pub fn start(app: tauri::AppHandle) -> Result<(), LifecycleError> {
    APP_HANDLE
        .set(app)
        .map_err(|_| LifecycleError::AlreadyStarted)?;
    let result = unsafe { saturday_voice_lifecycle_start(on_system_lock_or_sleep) };
    if result == 0 {
        Ok(())
    } else {
        Err(LifecycleError::RegistrationFailed)
    }
}

#[cfg(not(target_os = "macos"))]
pub fn start(_app: tauri::AppHandle) -> Result<(), LifecycleError> {
    Err(LifecycleError::UnsupportedPlatform)
}

#[cfg(target_os = "macos")]
extern "C" fn on_system_lock_or_sleep() {
    let Some(app) = APP_HANDLE.get() else {
        return;
    };
    let Some(supervisor) = app.try_state::<crate::voice::supervisor::VoiceSessionSupervisor>()
    else {
        return;
    };
    supervisor.stop_for_window_loss(crate::voice::supervisor::NativeStopReason::ApplicationLocked);
}
