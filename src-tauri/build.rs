fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("native/code_identity.c")
            .warnings(true)
            .compile("saturday_code_identity");
        cc::Build::new()
            .file("native/owner_authorization.c")
            .flag("-x")
            .flag("objective-c")
            .warnings(true)
            .compile("saturday_owner_authorization");
        cc::Build::new()
            .file("native/voice_authorization.m")
            .flag("-fobjc-arc")
            .warnings(true)
            .compile("saturday_voice_authorization");
        cc::Build::new()
            .file("native/voice_capture.m")
            .flag("-fobjc-arc")
            .warnings(true)
            .compile("saturday_voice_capture");
        cc::Build::new()
            .file("native/voice_speech.m")
            .flag("-fobjc-arc")
            .warnings(true)
            .compile("saturday_voice_speech");
        cc::Build::new()
            .file("native/voice_assets.m")
            .flag("-fobjc-arc")
            .warnings(true)
            .compile("saturday_voice_assets");
        cc::Build::new()
            .file("native/process_rss.c")
            .warnings(true)
            .compile("saturday_process_rss");
        cc::Build::new()
            .file("native/voice_lifecycle.m")
            .flag("-fobjc-arc")
            .warnings(true)
            .compile("saturday_voice_lifecycle");
        println!("cargo:rustc-link-lib=proc");
        println!("cargo:rustc-link-lib=framework=Security");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=LocalAuthentication");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=framework=AVFoundation");
        println!("cargo:rustc-link-lib=framework=CoreAudio");
        println!("cargo:rustc-link-lib=framework=AudioToolbox");
        println!("cargo:rerun-if-changed=native/code_identity.c");
        println!("cargo:rerun-if-changed=native/owner_authorization.c");
        println!("cargo:rerun-if-changed=native/voice_authorization.m");
        println!("cargo:rerun-if-changed=native/voice_capture.m");
        println!("cargo:rerun-if-changed=native/voice_speech.m");
        println!("cargo:rerun-if-changed=native/voice_capture.h");
        println!("cargo:rerun-if-changed=native/voice_speech.h");
        println!("cargo:rerun-if-changed=native/voice_assets.m");
        println!("cargo:rerun-if-changed=native/voice_assets.h");
        println!("cargo:rerun-if-changed=native/process_rss.c");
        println!("cargo:rerun-if-changed=native/process_rss.h");
        println!("cargo:rerun-if-changed=native/voice_lifecycle.m");
    }
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let generated_binding =
        std::path::Path::new("target/voice-owner-build/voice-asset-bindings.json");
    let checked_in_binding = std::path::Path::new("resources/voice-asset-bindings.json");
    let binding = if generated_binding.is_file() {
        println!("cargo:rerun-if-changed={}", generated_binding.display());
        generated_binding
    } else {
        println!("cargo:rerun-if-changed={}", checked_in_binding.display());
        checked_in_binding
    };
    let binding_bytes = std::fs::read(binding).expect("voice asset binding must be readable");
    std::fs::write(out_dir.join("voice-asset-bindings.json"), &binding_bytes)
        .expect("voice asset binding must be staged for compilation");
    println!("cargo:rerun-if-changed=native/voice_assets.m");
    println!("cargo:rerun-if-changed=native/voice_assets.h");
    println!("cargo:rerun-if-changed=native/voice_lifecycle.m");
    println!("cargo:rerun-if-changed=native/voice_lifecycle.h");
    tauri_build::build()
}
