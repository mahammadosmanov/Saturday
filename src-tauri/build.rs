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
        println!("cargo:rustc-link-lib=framework=Security");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=LocalAuthentication");
        println!("cargo:rerun-if-changed=native/code_identity.c");
        println!("cargo:rerun-if-changed=native/owner_authorization.c");
    }
    tauri_build::build()
}
