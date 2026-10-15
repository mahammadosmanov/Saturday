fn main() {
    println!("cargo:rerun-if-changed=native/parent_auth.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("native/parent_auth.m")
            .flag("-fobjc-arc")
            .warnings(true)
            .compile("saturday_voice_parent_auth");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=Security");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
    }
}
