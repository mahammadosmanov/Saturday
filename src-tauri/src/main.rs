#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use saturday::owner_authorization::cli::{parse_native_mode, NativeMode};
use saturday::owner_authorization::issuer::{dispatch_owner_mode, NativeOwnerAuthorizationActions};

use tauri::Manager;
fn main() {
    let arguments: Vec<_> = std::env::args_os().collect();
    let mode = match parse_native_mode(arguments) {
        Ok(mode) => mode,
        Err(_) => {
            eprintln!("SATURDAY_STAGE1_OWNER_AUTHORIZATION_ERROR:arguments");
            std::process::exit(64);
        }
    };

    let owner_actions = NativeOwnerAuthorizationActions::new();
    match dispatch_owner_mode(&mode, &owner_actions) {
        Ok(Some(output)) => {
            println!("{output}");
            return;
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("SATURDAY_STAGE1_OWNER_AUTHORIZATION_ERROR:{}", error.code());
            std::process::exit(78);
        }
    }

    let (main_thread_probe, voice_owner_bootstrap) = match mode {
        NativeMode::VerifySignedHelper => {
            match saturday::signed_helper::supervisor::run_bundled_self_test() {
                Ok(()) => {
                    println!("SATURDAY_SIGNED_HELPER_OK");
                    return;
                }
                Err(error) => {
                    eprintln!("SATURDAY_SIGNED_HELPER_ERROR:{}", error.code());
                    std::process::exit(78);
                }
            }
        }
        NativeMode::MeasureMainThread(output) => (Some(output), false),
        NativeMode::VoiceOwnerBootstrap => (None, true),
        NativeMode::Ordinary => (None, false),
        NativeMode::Stage1OwnerKeyProvision
        | NativeMode::Stage1OwnerKeyProve
        | NativeMode::Stage1OwnerAuthorizationIssue
        | NativeMode::Stage1OwnerKeyDelete => unreachable!("owner mode must exit before Tauri"),
    };
    let bootstrap_pending =
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(voice_owner_bootstrap));
    let bootstrap_for_window = std::sync::Arc::clone(&bootstrap_pending);
    let close_authorized = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let close_for_window = std::sync::Arc::clone(&close_authorized);
    let app = tauri::Builder::default()
        .setup(|app| {
            let supervisor = saturday::voice::supervisor::VoiceSessionSupervisor::from_app(
                app.handle().clone(),
            )?;
            app.manage(supervisor);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            saturday::voice::ipc::voice_get_status,
            saturday::voice::ipc::voice_start,
            saturday::voice::ipc::voice_finish,
            saturday::voice::ipc::voice_submit_text,
            saturday::voice::ipc::voice_confirm_transcript,
            saturday::voice::ipc::voice_cancel,
            saturday::voice::ipc::voice_clear_session,
            saturday::voice::ipc::voice_select_preferences,
            saturday::voice::ipc::voice_ack_intent,
            saturday::voice::ipc::voice_assets_get_status,
            saturday::voice::ipc::voice_assets_request_install,
            saturday::voice::ipc::voice_assets_disable,
            saturday::voice::ipc::voice_assets_delete,
            saturday::voice::ipc::voice_assets_rollback,
            saturday::voice::ipc::voice_test_output,
        ])
        .on_window_event(move |window, event| {
            let supervisor = window
                .app_handle()
                .state::<saturday::voice::supervisor::VoiceSessionSupervisor>();
            match event {
                tauri::WindowEvent::Focused(true) => {
                    if should_start_voice_owner_bootstrap(&bootstrap_for_window, true) {
                        let bootstrap_handle = window.app_handle().clone();
                        let exit_handle = bootstrap_handle.clone();
                        let result = std::thread::Builder::new()
                            .name("saturday-voice-owner-bootstrap".to_owned())
                            .spawn(move || {
                                let result = bootstrap_handle
                                    .try_state::<
                                        saturday::voice::supervisor::VoiceSessionSupervisor,
                                    >()
                                    .ok_or(())
                                    .and_then(|supervisor| {
                                        supervisor.bootstrap_voice_owner_key().map_err(|_| ())
                                    });
                                match result {
                                    Ok((phase, sha256)) => {
                                        println!(
                                            "SATURDAY_VOICE_OWNER_BOOTSTRAP_OK:{phase}:{sha256}"
                                        );
                                        exit_handle.exit(0);
                                    }
                                    Err(()) => {
                                        eprintln!("SATURDAY_VOICE_OWNER_BOOTSTRAP_ERROR");
                                        exit_handle.exit(78);
                                    }
                                }
                            });
                        if result.is_err() {
                            eprintln!("SATURDAY_VOICE_OWNER_BOOTSTRAP_ERROR");
                            window.app_handle().exit(78);
                        }
                    }
                }
                tauri::WindowEvent::Focused(false) => {
                    supervisor.stop_for_window_loss(
                        saturday::voice::supervisor::NativeStopReason::WindowUnfocused,
                    );
                }
                tauri::WindowEvent::Resized(_) if window.is_minimized().unwrap_or(false) => {
                    supervisor.stop_for_window_loss(
                        saturday::voice::supervisor::NativeStopReason::WindowHidden,
                    );
                }
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    if close_for_window.swap(false, std::sync::atomic::Ordering::AcqRel) {
                        return;
                    }
                    api.prevent_close();
                    let cleanup = supervisor.stop_for_window_loss(
                        saturday::voice::supervisor::NativeStopReason::WindowClosed,
                    );
                    let window = window.clone();
                    let close_authorized = std::sync::Arc::clone(&close_for_window);
                    let _ = std::thread::Builder::new()
                        .name("saturday-voice-close-gate".to_owned())
                        .spawn(move || {
                            let receipt =
                                cleanup.wait_for_cleanup(std::time::Duration::from_millis(2250));
                            if receipt.stopped && receipt.verified {
                                close_authorized.store(true, std::sync::atomic::Ordering::Release);
                                if window.close().is_err() {
                                    close_authorized
                                        .store(false, std::sync::atomic::Ordering::Release);
                                }
                            }
                        });
                }
                _ => {}
            }
        })
        .build(tauri::generate_context!())
        .expect("Saturday could not build its local interface");
    if saturday::platform::voice_lifecycle::start(app.handle().clone()).is_err() {
        eprintln!("SATURDAY_VOICE_LIFECYCLE_ERROR:register");
        std::process::exit(78);
    }
    if let Some(output) = main_thread_probe {
        if saturday::platform::main_thread_probe::start_probe(app.handle().clone(), output).is_err()
        {
            eprintln!("SATURDAY_MAIN_THREAD_PROBE_ERROR:start");
            std::process::exit(78);
        }
    }

    let exit_authorized = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let exit_for_handler = std::sync::Arc::clone(&exit_authorized);
    app.run(move |handle, event| {
        if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
            if exit_for_handler.swap(false, std::sync::atomic::Ordering::AcqRel) {
                return;
            }
            api.prevent_exit();
            let Some(supervisor) =
                handle.try_state::<saturday::voice::supervisor::VoiceSessionSupervisor>()
            else {
                return;
            };
            let cleanup = supervisor.stop_for_app_exit();
            let handle = handle.clone();
            let exit_authorized = std::sync::Arc::clone(&exit_for_handler);
            let code = code.unwrap_or_default();
            let _ = std::thread::Builder::new()
                .name("saturday-voice-exit-gate".to_owned())
                .spawn(move || {
                    let receipt = cleanup.wait_for_cleanup(std::time::Duration::from_millis(2250));
                    if receipt.stopped && receipt.verified {
                        exit_authorized.store(true, std::sync::atomic::Ordering::Release);
                        handle.exit(code);
                    }
                });
        }
    });
}
fn should_start_voice_owner_bootstrap(
    pending: &std::sync::atomic::AtomicBool,
    focused: bool,
) -> bool {
    focused && pending.swap(false, std::sync::atomic::Ordering::AcqRel)
}

#[cfg(test)]
mod tests {
    use super::should_start_voice_owner_bootstrap;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn owner_bootstrap_starts_once_after_focus_only_when_requested() {
        let pending = AtomicBool::new(true);

        assert!(!should_start_voice_owner_bootstrap(&pending, false));
        assert!(pending.load(Ordering::Acquire));
        assert!(should_start_voice_owner_bootstrap(&pending, true));
        assert!(!should_start_voice_owner_bootstrap(&pending, true));
        assert!(!should_start_voice_owner_bootstrap(
            &AtomicBool::new(false),
            true
        ));
    }
}
