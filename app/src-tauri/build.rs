use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "macos" {
        panic!("Murmur app builds support macOS only (target OS: {target_os})");
    }

    // AVFoundation is needed for AVCaptureDevice microphone authorization status
    // (commands::permissions::check_microphone_permission). AppKit does not load it.
    println!("cargo:rustc-link-lib=framework=AVFoundation");
    build_foundation_models_bridge();
    tauri_build::build()
}

/// Compile the small Swift bridge used by the optional homophone context pass
/// (`homophone_context.rs`) into a static library. FoundationModels is weakly
/// linked so the app still launches on macOS 14/15; an SDK without the
/// framework compiles a stub that always reports "unavailable".
fn build_foundation_models_bridge() {
    let source = Path::new("macos/FoundationModelsBridge.swift");
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        Ok(other) => panic!("unsupported macOS target architecture: {other}"),
        Err(_) => panic!("CARGO_CFG_TARGET_ARCH is not set"),
    };
    let deployment_target =
        std::env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "14.0".to_string());
    let library = out_dir.join("libmurmur_foundation_models.a");

    let status = Command::new("xcrun")
        .args([
            "swiftc",
            "-parse-as-library",
            "-emit-library",
            "-static",
            "-O",
        ])
        .args(["-module-name", "MurmurFoundationModels"])
        .args(["-target", &format!("{arch}-apple-macos{deployment_target}")])
        .arg("-suppress-warnings")
        // The static archive must not strongly autolink the framework.
        .args(["-Xfrontend", "-disable-autolink-framework"])
        .args(["-Xfrontend", "FoundationModels"])
        .arg("-o")
        .arg(&library)
        .arg(source)
        .status()
        .expect("failed to run swiftc for the FoundationModels bridge");
    if !status.success() {
        panic!("swiftc failed to build the FoundationModels bridge");
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=murmur_foundation_models");
    if sdk_has_foundation_models() {
        println!("cargo:rustc-link-arg=-Wl,-weak_framework,FoundationModels");
    }
}

fn sdk_has_foundation_models() -> bool {
    Command::new("xcrun")
        .args(["--sdk", "macosx", "--show-sdk-path"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
                .join("System/Library/Frameworks/FoundationModels.framework")
                .exists()
        })
        .unwrap_or(false)
}
