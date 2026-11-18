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

const GATE: &str = "feature = \"dictation-shadow\"";

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The attribute and doc lines right above `lines[n]` name the feature.
fn attributes_above_name_the_feature(lines: &[&str], n: usize) -> bool {
    lines[..n]
        .iter()
        .rev()
        .take_while(|line| {
            let line = line.trim_start();
            line.starts_with("#[") || line.starts_with("//") || line.is_empty()
        })
        .any(|line| line.contains(GATE))
}

/// Whether `lines[n]` sits inside an item or block that is compiled only with the
/// feature: some enclosing line (strictly less indented, walking outwards) has a
/// feature attribute right above it.
fn inside_a_gated_item(lines: &[&str], n: usize) -> bool {
    // An item header is gated by the attributes above it. The closing line of a
    // multi-line signature (`) -> T {`) belongs to the item its opening line starts.
    let header_is_gated = |i: usize| {
        let mut header = i;
        if lines[i].trim_start().starts_with(')') {
            if let Some(opening) = (0..i)
                .rev()
                .find(|&j| indent(lines[j]) == indent(lines[i]) && !lines[j].trim().is_empty())
            {
                header = opening;
            }
        }
        lines[i].contains(GATE) || attributes_above_name_the_feature(lines, header)
    };
    if header_is_gated(n) {
        return true;
    }
    let mut level = indent(lines[n]);
    for i in (0..n).rev() {
        let line = lines[i];
        if line.trim().is_empty() || indent(line) >= level {
            continue;
        }
        level = indent(line);
        if header_is_gated(i) {
            return true;
        }
        if level == 0 {
            break;
        }
    }
    false
}

/// A file whose own `mod` declaration sits behind the feature is compiled only with it.
fn file_is_gated(source_dir: &Path, relative: &str) -> bool {
    let path = Path::new(relative);
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return false;
    };
    let parent = path.parent().unwrap_or(Path::new(""));
    let declarations = [
        source_dir.join(parent).join("mod.rs"),
        source_dir.join(parent).with_extension("rs"),
        source_dir.join(parent).join(format!("{stem}.rs")),
    ];
    declarations.iter().any(|declaration| {
        let Ok(text) = std::fs::read_to_string(declaration) else {
            return false;
        };
        let lines: Vec<&str> = text.lines().collect();
        lines.iter().enumerate().any(|(n, line)| {
            let code = line.split("//").next().unwrap_or("");
            (code.contains(&format!("mod {stem};")))
                && (code.contains("dictation-shadow")
                    || attributes_above_name_the_feature(&lines, n))
        })
    })
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
        // A file whose module is declared behind the feature does not exist in this
        // build: its lines are gated as a whole (still counted).
        let file_gated = file_is_gated(&source_dir(), &relative);
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        for (n, line) in lines.iter().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if !(code.contains("dictation_shadow::") || code.contains("mod dictation_shadow;")) {
                continue;
            }
            checked += 1;
            let gated = file_gated
                || code.contains("dictation-shadow")
                || (n > 0 && lines[n - 1].contains(GATE))
                || inside_a_gated_item(&lines, n);
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
