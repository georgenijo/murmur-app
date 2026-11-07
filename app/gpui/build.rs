fn main() {
    println!("cargo:rerun-if-changed=native/host.m");
    println!("cargo:rerun-if-changed=native/legacy.m");
    println!("cargo:rerun-if-changed=native/services.m");
    // Packaging and the UI consume Tauri's canonical release version directly;
    // the presentation crate's 0.1.0 is not another application version.
    println!("cargo:rerun-if-changed=../src-tauri/tauri.conf.json");
    let config: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string("../src-tauri/tauri.conf.json").expect("shared app config"),
    )
    .expect("shared app config JSON");
    println!(
        "cargo:rustc-env=MURMUR_APP_VERSION={}",
        config["version"].as_str().expect("app version")
    );
    println!("cargo:rerun-if-changed=native/ax_focus_fix.m");
    println!("cargo:rerun-if-changed=native/preferences.m");
    println!("cargo:rerun-if-changed=native/overlay.m");
    println!("cargo:rerun-if-changed=native/webkit_policy.m");
    println!("cargo:rerun-if-changed=native/meeting_playback.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .files([
                "native/ax_focus_fix.m",
                "native/host.m",
                "native/preferences.m",
                "native/overlay.m",
                "native/meeting_playback.m",
            ])
            .flag("-fobjc-arc")
            .compile("murmur_accessibility");
        if std::env::var_os("CARGO_FEATURE_PRODUCTION_HOST").is_some() {
            // ui_lib's Swift archive is a dependency here. Final-link arguments
            // from its build script do not propagate to this executable/test.
            let sdk = std::process::Command::new("xcrun")
                .args(["--sdk", "macosx", "--show-sdk-path"])
                .output()
                .expect("macOS SDK");
            if sdk.status.success()
                && std::path::Path::new(String::from_utf8_lossy(&sdk.stdout).trim())
                    .join("System/Library/Frameworks/FoundationModels.framework")
                    .exists()
            {
                println!("cargo:rustc-link-arg=-Wl,-weak_framework,FoundationModels");
            }

            cc::Build::new()
                .files(["native/services.m", "native/legacy.m"])
                .flag("-fobjc-arc")
                .compile("murmur_host_services");
            println!("cargo:rustc-link-lib=framework=ServiceManagement");
            println!("cargo:rustc-link-lib=framework=UserNotifications");
            println!("cargo:rustc-link-lib=framework=UniformTypeIdentifiers");
        }
        // Backtraces and policy storage exist only in explicit QA builds.
        if std::env::var_os("CARGO_FEATURE_GALLERY").is_some() {
            cc::Build::new()
                .file("native/webkit_policy.m")
                .flag("-fobjc-arc")
                .flag("-fno-omit-frame-pointer")
                .compile("murmur_webkit_policy");
        }
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=framework=QuartzCore");
        println!("cargo:rustc-link-lib=framework=AVFoundation");
        println!("cargo:rustc-link-arg=-mmacosx-version-min=14.0");
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
        let sdk = std::process::Command::new("xcrun")
            .args(["--sdk", "macosx", "--show-sdk-path"])
            .output()
            .expect("macOS SDK");
        assert!(sdk.status.success(), "macOS SDK lookup failed");
        if std::path::Path::new(String::from_utf8_lossy(&sdk.stdout).trim())
            .join("System/Library/Frameworks/FoundationModels.framework")
            .exists()
        {
            println!("cargo:rustc-link-arg=-Wl,-weak_framework,FoundationModels");
        }
    }
}
