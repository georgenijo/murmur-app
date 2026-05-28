fn main() {
    // AVFoundation is needed for AVCaptureDevice microphone authorization status
    // (commands::permissions::check_microphone_permission). AppKit does not load it.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-lib=framework=AVFoundation");
    }
    tauri_build::build()
}
