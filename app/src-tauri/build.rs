fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "macos" {
        panic!("Murmur app builds support macOS only (target OS: {target_os})");
    }

    // AVFoundation is needed for AVCaptureDevice microphone authorization status
    // (commands::permissions::check_microphone_permission). AppKit does not load it.
    println!("cargo:rustc-link-lib=framework=AVFoundation");
    tauri_build::build()
}
