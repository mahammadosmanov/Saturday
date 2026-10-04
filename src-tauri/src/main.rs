#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use saturday::owner_authorization::cli::{parse_native_mode, NativeMode};
use saturday::owner_authorization::issuer::{dispatch_owner_mode, NativeOwnerAuthorizationActions};

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

    let main_thread_probe = match mode {
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
        NativeMode::MeasureMainThread(output) => Some(output),
        NativeMode::Ordinary => None,
        NativeMode::Stage1OwnerKeyProvision
        | NativeMode::Stage1OwnerKeyProve
        | NativeMode::Stage1OwnerAuthorizationIssue
        | NativeMode::Stage1OwnerKeyDelete => unreachable!("owner mode must exit before Tauri"),
    };

    // Native commands remain inaccessible to the webview.
    let app = tauri::Builder::default()
        .build(tauri::generate_context!())
        .expect("Saturday could not build its local interface");
    if let Some(output) = main_thread_probe {
        if saturday::platform::main_thread_probe::start_probe(app.handle().clone(), output).is_err()
        {
            eprintln!("SATURDAY_MAIN_THREAD_PROBE_ERROR:start");
            std::process::exit(78);
        }
    }
    app.run(|_, _| {});
}
