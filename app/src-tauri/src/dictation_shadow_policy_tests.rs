//! Build restrictions of the `dictation-shadow` feature, pinned in every build
//! (decision D-S2c-1 e).
//!
//! The shadow catches its own panics so they can never reach the live path, and
//! `catch_unwind` does nothing under `panic = "abort"`. The release profile aborts, so
//! the feature refuses to compile there, and native soak builds use the `shadow-soak`
//! profile instead. These tests read `lib.rs` and `Cargo.toml` and fail if either half
//! of that is removed. (`cargo check --release --features dictation-shadow` is also
//! run by hand in the PR verification: it stops at the `compile_error!`.)

use std::path::Path;

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The lines of one `[table]` of Cargo.toml, up to the next table.
fn table(toml: &str, header: &str) -> Vec<String> {
    let mut lines = toml.lines().skip_while(|l| l.trim() != header).skip(1);
    let mut body = Vec::new();
    for line in lines.by_ref() {
        if line.trim_start().starts_with('[') {
            break;
        }
        let code = line.split('#').next().unwrap_or("").trim();
        if !code.is_empty() {
            body.push(code.to_string());
        }
    }
    body
}

#[test]
fn the_shadow_feature_is_refused_under_panic_abort() {
    let lib = std::fs::read_to_string(manifest_dir().join("src/lib.rs")).unwrap();
    let gate = "#[cfg(all(feature = \"dictation-shadow\", panic = \"abort\"))]";
    let at = lib.find(gate).expect("the panic = abort gate is in lib.rs");
    let after = &lib[at + gate.len()..];
    assert!(
        after.trim_start().starts_with("compile_error!("),
        "the gate must guard a compile_error!"
    );
    assert!(
        after.contains("shadow-soak"),
        "the error must say which profile to use"
    );
}

#[test]
fn the_release_profile_aborts_and_the_soak_profile_unwinds() {
    let toml = std::fs::read_to_string(manifest_dir().join("Cargo.toml")).unwrap();
    let release = table(&toml, "[profile.release]");
    assert!(
        release.iter().any(|l| l == "panic = \"abort\""),
        "the release profile aborts: that is why the feature needs its own profile: {release:?}"
    );
    let soak = table(&toml, "[profile.shadow-soak]");
    assert!(
        soak.iter().any(|l| l == "inherits = \"release\""),
        "shadow-soak inherits release: {soak:?}"
    );
    assert!(
        soak.iter().any(|l| l == "panic = \"unwind\""),
        "shadow-soak unwinds: {soak:?}"
    );
    // The soak profile changes nothing else about the build.
    assert_eq!(soak.len(), 2, "{soak:?}");
}

#[test]
fn the_default_features_and_dev_profiles_unwind() {
    // `cargo test` and `tauri dev` build with `panic = "unwind"`, which the feature
    // needs: the dev and test profiles must not set `panic = "abort"`.
    let toml = std::fs::read_to_string(manifest_dir().join("Cargo.toml")).unwrap();
    for header in ["[profile.dev]", "[profile.test]"] {
        assert!(
            !table(&toml, header).iter().any(|l| l.contains("abort")),
            "{header} must unwind"
        );
    }
}

/// Every production acquisition of the dictation lock D goes through
/// `MutexExt::lock_or_recover`, the one function that counts it for the shadow's
/// conservation check (D-S2c-9 a). A raw `.lock()` / `.try_lock()` on `dictation` in
/// production code would take D without being counted, which is the bypass the guard-level
/// count exists to close.
#[test]
fn every_production_dictation_lock_goes_through_the_counting_guard() {
    fn production_files(dir: &Path, found: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if path.is_dir() {
                if name != "tests" {
                    production_files(&path, found);
                }
            } else if name.ends_with(".rs")
                && !name.ends_with("_tests.rs")
                && name != "tests.rs"
                && !name.ends_with("_harness.rs")
            {
                found.push(path);
            }
        }
    }
    /// The text with inline `#[cfg(test)] mod x { .. }` blocks (to the `}` at column 0 that
    /// rustfmt puts at their end) and `//` comment lines removed.
    fn production_text(text: &str) -> String {
        let lines: Vec<&str> = text.lines().collect();
        let mut keep = vec![true; lines.len()];
        for at in 0..lines.len().saturating_sub(1) {
            if lines[at].trim() == "#[cfg(test)]"
                && lines[at + 1].contains("mod ")
                && lines[at + 1].trim_end().ends_with('{')
            {
                let end = lines[at + 2..]
                    .iter()
                    .position(|line| *line == "}")
                    .map_or(lines.len() - 1, |offset| at + 2 + offset);
                keep[at..=end].iter_mut().for_each(|k| *k = false);
            }
        }
        lines
            .iter()
            .zip(keep)
            .filter(|(line, keep)| *keep && !line.trim_start().starts_with("//"))
            .map(|(line, _)| *line)
            .collect::<Vec<_>>()
            .join("\n")
    }
    let mut files = Vec::new();
    production_files(&manifest_dir().join("src"), &mut files);
    assert!(
        files.len() > 50,
        "the walk found {} source files",
        files.len()
    );
    let mut raw = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        for (number, line) in production_text(&text).lines().enumerate() {
            if line.contains("dictation.lock()") || line.contains("dictation.try_lock()") {
                raw.push(format!("{}: {}", file.display(), line.trim()));
                let _ = number;
            }
        }
    }
    assert!(
        raw.is_empty(),
        "a production site takes the dictation lock without `lock_or_recover` (uncounted by \
         the shadow's conservation check): {raw:#?}"
    );
}
