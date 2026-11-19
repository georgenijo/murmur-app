use std::path::{Path, PathBuf};
use std::process::Command;

// The dictation lock's site rules, shared with `dictation_shadow/sites.rs`.
include!("src/dictation_shadow/site_rules.in");

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "macos" {
        panic!("Murmur app builds support macOS only (target OS: {target_os})");
    }

    // AVFoundation is needed for AVCaptureDevice microphone authorization status
    // (commands::permissions::check_microphone_permission). AppKit does not load it.
    println!("cargo:rustc-link-lib=framework=AVFoundation");
    build_foundation_models_bridge();
    // Only a `dictation-shadow` build records its identity and site map: a default or
    // release build consults neither git nor the sources, and its rebuild triggers do
    // not change.
    if std::env::var_os("CARGO_FEATURE_DICTATION_SHADOW").is_some() {
        export_build_identity();
        generate_lock_site_map();
    }
    tauri_build::build()
}

/// Export the build's identity to the dictation shadow's session record
/// (`MURMUR_GIT_SHA`, `MURMUR_BUILD_PROFILE`; read with `option_env!`). The shadow's
/// summary names the revision and profile that produced it, so a soak pass can tell a
/// stale file from this build's. A checkout with no `git`, or no repository, gives
/// `"unknown"`; `MURMUR_GIT_SHA` set in the environment wins (a packaged build).
fn export_build_identity() {
    println!("cargo:rerun-if-env-changed=MURMUR_GIT_SHA");
    let sha = std::env::var("MURMUR_GIT_SHA")
        .ok()
        .filter(|sha| is_full_sha(sha))
        .or_else(|| git_output(&["rev-parse", "HEAD"]).filter(|sha| is_full_sha(sha)))
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=MURMUR_GIT_SHA={sha}");

    // Re-run when HEAD moves: the file that names the branch, and the branch's ref. Only a
    // path that exists is named: Cargo treats a missing `rerun-if-changed` path as stale
    // on every build. A branch whose ref is packed has no loose ref file, so
    // `packed-refs` stands for it.
    let head_ref = git_output(&["symbolic-ref", "-q", "HEAD"])
        .and_then(|head| git_output(&["rev-parse", "--git-path", &head]))
        .filter(|path| Path::new(path).exists())
        .or_else(|| git_output(&["rev-parse", "--git-path", "packed-refs"]));
    // A commit or pull on a branch whose ref is packed creates the loose ref and changes
    // neither `HEAD` nor `packed-refs`: the reflog (`logs/HEAD`, per worktree) grows on
    // every move of HEAD, so it is watched too.
    for path in [
        git_output(&["rev-parse", "--git-path", "HEAD"]),
        git_output(&["rev-parse", "--git-path", "logs/HEAD"]),
        head_ref,
    ]
    .into_iter()
    .flatten()
    .filter(|path| Path::new(path).exists())
    {
        println!("cargo:rerun-if-changed={path}");
    }

    // `PROFILE` is only `debug` or `release`; a custom profile (`shadow-soak`) is the
    // directory name under `target/`: OUT_DIR is `target/<profile>/build/<pkg>/out`.
    let profile = std::env::var("OUT_DIR")
        .ok()
        .and_then(|out| {
            Path::new(&out)
                .ancestors()
                .nth(3)
                .and_then(|dir| dir.file_name())
                .map(|name| name.to_string_lossy().into_owned())
        })
        .or_else(|| std::env::var("PROFILE").ok())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=MURMUR_BUILD_PROFILE={profile}");
}

/// Write `OUT_DIR/dictation_lock_sites.rs`: every `.lock_or_recover()` call site in
/// `src/`, as `(file, line, class)` per the shared rules, for the shadow's summary to
/// classify the dictation lock's acquisitions without reading a file at run time.
fn generate_lock_site_map() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=src/dictation_shadow/site_rules.in");
    let mut files = Vec::new();
    collect_sources(Path::new("src"), &mut files);
    files.sort();
    let mut entries = String::new();
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let name = path.to_string_lossy().replace('\\', "/");
        for (index, line) in text.lines().enumerate() {
            if !line.contains(".lock_or_recover()") {
                continue;
            }
            let class = classify_text(&name, &text, index + 1).code();
            entries.push_str(&format!("    ({name:?}, {}, {class}),\n", index + 1));
        }
    }
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(
        out_dir.join("dictation_lock_sites.rs"),
        format!("pub(super) const SITES: &[(&str, u32, u8)] = &[\n{entries}];\n"),
    )
    .expect("write the dictation lock site map");
}

fn collect_sources(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_sources(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

fn is_full_sha(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn git_output(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|text| !text.is_empty())
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
