//! Tests that only make sense in a build **without** the `dictation-shadow` feature
//! (the default one): it contains no shadow code, and nothing arms it.

use std::path::{Path, PathBuf};

fn source_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn shadow_off_by_default() {
    // This file is compiled only without the feature (`lib.rs`), and the shadow
    // module only with it, so this build has none of that code.

    // No default feature set turns it on.
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    let features = manifest
        .split("[features]")
        .nth(1)
        .expect("a [features] table")
        .split("\n[")
        .next()
        .unwrap();
    assert!(
        features
            .lines()
            .any(|line| line.trim() == "dictation-shadow = []"),
        "the feature is a plain switch: it pulls in no dependency"
    );
    assert!(
        !manifest.contains("crossbeam-queue"),
        "the shadow's ring is in-crate; no queue dependency (D-S2c-2 a)"
    );
    for line in features
        .lines()
        .filter(|l| l.trim_start().starts_with("default"))
    {
        assert!(
            !line.contains("dictation-shadow"),
            "`dictation-shadow` must stay out of the default features: {line}"
        );
    }

    // Every use of the shadow module outside its own directory sits behind the
    // feature: the line, or the attribute right before it, names the feature.
    let mut files = Vec::new();
    rust_files(&source_dir(), &mut files);
    let mut checked = 0;
    for path in files {
        let relative = path
            .strip_prefix(source_dir())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if relative.starts_with("dictation_shadow/")
            || relative == "dictation_shadow_default_tests.rs"
        {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        for (n, line) in lines.iter().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if !(code.contains("dictation_shadow::") || code.contains("mod dictation_shadow;")) {
                continue;
            }
            checked += 1;
            let gated = code.contains("dictation-shadow")
                || (n > 0 && lines[n - 1].contains("feature = \"dictation-shadow\""));
            assert!(
                gated,
                "{relative}:{} names the shadow without the feature gate",
                n + 1
            );
        }
    }
    assert!(
        checked >= 3,
        "lib.rs declares, starts and flushes it: {checked}"
    );
}
