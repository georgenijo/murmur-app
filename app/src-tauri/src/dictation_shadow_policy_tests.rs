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
/// conservation check (D-S2c-9 a, D-S2c-10 b R1). The scan matches the **receiver**, not
/// a method name: any use of the `AppState::dictation` field in production code that is
/// not `.dictation.lock_or_recover()` (a raw `lock`, `try_lock`, `try_lock_or_recover`,
/// any future method, a UFCS call, a borrow handed to a helper) fails, unless it is one
/// of the named non-decision diagnostic reads below. Test code is outside the scan.
#[test]
fn every_production_dictation_lock_goes_through_the_counting_guard() {
    /// Non-blocking diagnostic reads that never take a decision: (file, text near the
    /// use, why). Each stays uncounted on purpose.
    const NAMED: &[(&str, &str, &str)] = &[
        (
            "state.rs",
            "self.dictation\n            .try_lock_or_recover()",
            "hang-diagnostics status read, never contends with a decision",
        ),
        (
            "commands/recording.rs",
            "app_state.dictation.try_lock_or_recover() else",
            "supervisor-idle probe that reports contention instead of waiting",
        ),
        (
            "commands/models.rs",
            "state.app_state.dictation.try_lock_or_recover()",
            "model-switch refusal probe",
        ),
    ];
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
    /// Every `.dictation` receiver in `text`: (byte offset, the receiver identifier before
    /// the dot, what follows it with whitespace removed).
    fn receivers(text: &str) -> Vec<(usize, String, String)> {
        let bytes = text.as_bytes();
        let mut found = Vec::new();
        let mut from = 0;
        while let Some(at) = text[from..].find(".dictation") {
            let start = from + at;
            let end = start + ".dictation".len();
            from = end;
            if bytes
                .get(end)
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
            {
                continue; // `.dictation_diagnostics`, `.dictation_status` ...
            }
            let receiver: String = text[..start]
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let tail: String = text[end..]
                .chars()
                .filter(|c| !c.is_whitespace())
                .take(40)
                .collect();
            found.push((start, receiver, tail));
        }
        found
    }
    let mut files = Vec::new();
    production_files(&manifest_dir().join("src"), &mut files);
    assert!(
        files.len() > 50,
        "the walk found {} source files",
        files.len()
    );
    let mut raw = Vec::new();
    let mut named_seen = vec![false; NAMED.len()];
    for file in files {
        let text = production_text(&std::fs::read_to_string(&file).unwrap());
        let shown = file.to_string_lossy().replace('\\', "/");
        for (start, receiver, tail) in receivers(&text) {
            // The counted guard, and the executor context's own borrowed guard option:
            // a field of the executor's phase context, in the executor's own file only, so
            // an `AppState` bound to the name `ctx` elsewhere is still scanned.
            if tail.starts_with(".lock_or_recover()")
                || (receiver == "ctx" && shown.ends_with("dictation_executor/executor.rs"))
            {
                continue;
            }
            let context = &text[start.saturating_sub(40)..(start + 80).min(text.len())];
            let named = NAMED.iter().position(|(file_suffix, anchor, _)| {
                shown.ends_with(file_suffix) && context.contains(anchor)
            });
            if let Some(index) = named {
                named_seen[index] = true;
            } else {
                raw.push(format!("{shown}: `{receiver}.dictation{tail}`"));
            }
        }
    }
    assert!(
        raw.is_empty(),
        "a production site reaches the dictation mutex other than through `lock_or_recover` \
         (uncounted by the shadow's conservation check): {raw:#?}"
    );
    for (index, seen) in named_seen.iter().enumerate() {
        assert!(
            *seen,
            "the named uncounted read {:?} no longer exists: remove it",
            NAMED[index]
        );
    }
}

/// The detached auto-stop the audio lifecycle spawns is its own invocation: its flights
/// wait for the reply it returns, so with the shadow on it runs under
/// `capture::commanded`, which witnesses the real result (D-S2c-9 d). Its spawn closure
/// needs a live `AppHandle`, which no replay has, so this reads the source: every call of
/// `stop_native_recording_owned` is either the shadow twin (wrapped in `commanded`), the
/// feature-off original right below its own `#[cfg(not(feature = ...))]`, or the
/// wrapper command's own tail call.
#[test]
fn the_detached_auto_stop_is_witnessed_at_its_return() {
    let text = std::fs::read_to_string(manifest_dir().join("src/commands/recording.rs")).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let (mut twins, mut originals) = (0, 0);
    for (at, line) in lines.iter().enumerate() {
        let call = line.contains("stop_native_recording_owned(")
            && !line.contains("fn stop_native_recording_owned")
            && !line.trim_start().starts_with("//");
        if !call
            || line
                .trim_start()
                .starts_with("stop_native_recording_owned(app_handle, state")
        {
            continue;
        }
        if line.contains("capture::commanded(stop_native_recording_owned(") {
            twins += 1;
        } else {
            assert!(
                lines[at.saturating_sub(1)].contains("#[cfg(not(feature = \"dictation-shadow\"))]"),
                "line {}: a detached stop that the shadow does not witness: {line}",
                at + 1
            );
            originals += 1;
        }
    }
    assert_eq!(
        (twins, originals),
        (1, 1),
        "the detached auto-stop's two builds"
    );
}
