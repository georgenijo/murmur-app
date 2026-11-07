fn main() {
    println!("cargo:rerun-if-changed=native/ax_focus_fix.m");
    println!("cargo:rerun-if-changed=native/preferences.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .files(["native/ax_focus_fix.m", "native/preferences.m"])
            .flag("-fobjc-arc")
            .compile("murmur_accessibility");
        println!("cargo:rustc-link-lib=framework=AppKit");
    }
}
