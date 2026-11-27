//! A build **without** the `fake-permissions` feature (every release build, and the
//! default dev and test builds) contains no fake permission path (#972).

use std::path::{Path, PathBuf};

const GATE: &str = "#[cfg(feature = \"fake-permissions\")]";

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn names_the_module(line: &str) -> bool {
    let code = line.split("//").next().unwrap_or("");
    code.contains("fake_permissions::") || code.contains("mod fake_permissions;")
}

/// Whether `lines[n]` lies inside the item right after a `GATE` attribute: the
/// attribute sits at or left of `n`'s indent, and the item it starts (ended by
/// its first line at the attribute's indent that closes a block or a statement)
/// has not ended before `n`.
fn gated(lines: &[&str], n: usize) -> bool {
    for i in (0..n).rev() {
        let line = lines[i];
        if line.trim().is_empty() {
            continue;
        }
        let g = indent(line);
        if g > indent(lines[n]) {
            continue;
        }
        if line.trim() == GATE {
            // The item's lines after the attribute, up to `n`, must not include
            // an earlier end of that item.
            let item_ended_early = lines[i + 1..n].iter().any(|l| {
                let t = l.trim();
                indent(l) == g && !t.is_empty() && (t.starts_with('}') || t.ends_with(';'))
            });
            return !item_ended_early;
        }
    }
    false
}

#[test]
fn feature_is_dev_only_and_off_by_default() {
    let manifest = std::fs::read_to_string(manifest_dir().join("Cargo.toml")).unwrap();
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
            .any(|l| l.trim() == "fake-permissions = []"),
        "the feature is a plain switch that pulls in no dependency"
    );
    for line in features
        .lines()
        .filter(|l| l.trim_start().starts_with("default"))
    {
        assert!(
            !line.contains("fake-permissions"),
            "not a default feature: {line}"
        );
    }

    // Only the dev script turns it on; no build or bundle script does.
    let package = std::fs::read_to_string(manifest_dir().join("../package.json")).unwrap();
    let enabling: Vec<&str> = package
        .lines()
        .filter(|l| l.contains("fake-permissions"))
        .collect();
    assert_eq!(
        enabling.len(),
        1,
        "exactly one npm script enables it: {enabling:?}"
    );
    let script = enabling[0];
    assert!(
        script.contains("\"tauri:dev:fake-permissions\""),
        "{script}"
    );
    assert!(
        script.contains("tauri dev ") && !script.contains("tauri build"),
        "{script}"
    );
    let build_script = manifest_dir().join("../../scripts/tauri_build_local.mjs");
    let text = std::fs::read_to_string(&build_script).unwrap();
    assert!(
        !text.contains("fake-permissions"),
        "{}",
        build_script.display()
    );

    // `lib.rs` refuses the feature in a build without debug assertions.
    let lib = std::fs::read_to_string(manifest_dir().join("src/lib.rs")).unwrap();
    assert!(lib.contains(
        "#[cfg(all(feature = \"fake-permissions\", not(debug_assertions)))]\ncompile_error!("
    ));
}

#[test]
fn every_use_of_the_fake_sits_behind_the_feature() {
    let source = manifest_dir().join("src");
    let mut files = Vec::new();
    rust_files(&source, &mut files);
    let mut checked = 0;
    let mut ungated = Vec::new();
    for path in files {
        let relative = path
            .strip_prefix(&source)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if relative == "fake_permissions.rs" || relative == "fake_permissions_default_tests.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        for n in 0..lines.len() {
            if names_the_module(lines[n]) {
                checked += 1;
                if !gated(&lines, n) {
                    ungated.push(format!("{relative}:{}", n + 1));
                }
            }
        }
    }
    assert!(
        ungated.is_empty(),
        "ungated fake permission uses: {ungated:?}"
    );
    // The hooks exist: microphone, accessibility, system audio, calendar, marker.
    assert!(
        checked >= 20,
        "found only {checked} uses; did the hooks move?"
    );
}

#[test]
fn the_gate_check_catches_an_ungated_use() {
    let lines = [
        "fn f() {",
        "    #[cfg(feature = \"fake-permissions\")]",
        "    if let Some(x) = crate::fake_permissions::state(p) {",
        "        return crate::fake_permissions::map(x);",
        "    }",
        "    crate::fake_permissions::leak();",
        "}",
    ];
    assert!(gated(&lines, 2));
    assert!(gated(&lines, 3));
    assert!(!gated(&lines, 5));
}

/// Symbol check: the compiled default binary carries none of the provider's
/// strings (`fake_permissions::tests` checks the same needles are found in a
/// build with the feature, so the scan is not vacuous). The needles are stored
/// reversed so this test's own binary does not contain them; split literals do
/// not work, because the compiler can place the halves next to each other.
#[test]
fn the_compiled_default_binary_has_no_fake_path() {
    let binary = std::fs::read(std::env::current_exe().unwrap()).unwrap();
    for reversed in [
        "EVITCA SNOISSIMREP EKAF VED",
        "rekram-snoissimrep-ekaf-rumrum",
        " :evitca ton snoissimrep ekaf",
        "EKAF_RUMRUM( evitca era snoissimrep ekaF",
    ] {
        let needle: String = reversed.chars().rev().collect();
        let needle = needle.as_bytes();
        let found = binary.windows(needle.len()).any(|window| window == needle);
        assert!(
            !found,
            "default build contains {:?}",
            String::from_utf8_lossy(needle)
        );
    }
}
