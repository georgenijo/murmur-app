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

const FEATURE: &str = "dictation-shadow";

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Evaluate a `cfg` predicate for the **default test build**: the feature is off,
/// `test` is on, and any other atom (`debug_assertions`, `target_os`, ..) is assumed
/// on. An item whose predicate is false there does not exist in this build, which is
/// what "gated" means for the lint. Returns the value and the unparsed rest.
fn eval_cfg(text: &str) -> (bool, &str) {
    let text = text.trim_start();
    let ident_end = text
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(text.len());
    let (ident, rest) = text.split_at(ident_end);
    let rest = rest.trim_start();
    if matches!(ident, "all" | "any" | "not") && rest.starts_with('(') {
        let mut rest = &rest[1..];
        let mut values = Vec::new();
        loop {
            let (value, after) = eval_cfg(rest);
            values.push(value);
            let after = after.trim_start();
            if let Some(after) = after.strip_prefix(',') {
                if after.trim_start().starts_with(')') {
                    rest = after;
                    continue;
                }
                rest = after;
                continue;
            }
            rest = after.strip_prefix(')').unwrap_or(after);
            break;
        }
        let value = match ident {
            "all" => values.iter().all(|v| *v),
            "any" => values.iter().any(|v| *v),
            _ => !values.first().copied().unwrap_or(true),
        };
        return (value, rest);
    }
    if let Some(after) = rest.strip_prefix('=') {
        let after = after.trim_start();
        if let Some(quoted) = after.strip_prefix('"') {
            let end = quoted.find('"').unwrap_or(quoted.len());
            let value = &quoted[..end];
            let rest = quoted.get(end + 1..).unwrap_or("");
            let on = !(ident == "feature" && value == FEATURE);
            return (on, rest);
        }
    }
    (ident != "feature", rest)
}

/// Whether the attribute on `line` (a `#[cfg(..)]`) removes the item from the default
/// test build. Only real `cfg` attributes count: a comment or a doc line that names
/// the feature is not a gate.
fn attribute_gates(line: &str) -> bool {
    let line = line.trim_start();
    let Some(inner) = line
        .strip_prefix("#[cfg(")
        .or_else(|| line.strip_prefix("#![cfg("))
    else {
        return false;
    };
    !eval_cfg(inner).0
}

/// The attribute lines right above `lines[n]` (comments, doc lines and blanks may sit
/// between them) remove it from the default build.
fn attributes_above_gate(lines: &[&str], n: usize) -> bool {
    lines[..n]
        .iter()
        .rev()
        .take_while(|line| {
            let line = line.trim_start();
            line.starts_with("#[") || line.starts_with("//") || line.is_empty()
        })
        .any(|line| attribute_gates(line))
}

/// Whether `lines[n]` sits inside an item or block that is compiled only with the
/// feature: some enclosing line (strictly less indented, walking outwards) has a
/// gating `cfg` attribute right above it, or on its own line.
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
        attribute_gates(lines[i]) || attributes_above_gate(lines, header)
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
            code.contains(&format!("mod {stem};"))
                && (inside_a_gated_item(&lines, n)
                    || code
                        .split("mod ")
                        .next()
                        .is_some_and(|before| attribute_gates(before)))
        })
    })
}

/// The 1-based lines of `text` that name the shadow module without a gating `cfg`
/// attribute, for a file that is not itself gated.
fn ungated_uses(text: &str) -> Vec<usize> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        if !(code.contains("dictation_shadow::") || code.contains("mod dictation_shadow;")) {
            continue;
        }
        let same_line = code
            .find("dictation_shadow")
            .is_some_and(|at| attribute_gates(&code[..at]) || code[..at].contains("#[cfg("));
        let gated = same_line_gates(code) || inside_a_gated_item(&lines, n) || same_line;
        if !gated {
            found.push(n + 1);
        }
    }
    found
}

/// `#[cfg(feature = "dictation-shadow")] use ...;` on one line.
fn same_line_gates(code: &str) -> bool {
    code.match_indices("#[cfg(")
        .any(|(at, _)| attribute_gates(&code[at..]))
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
        let uses = text
            .lines()
            .filter(|line| {
                let code = line.split("//").next().unwrap_or("");
                code.contains("dictation_shadow::") || code.contains("mod dictation_shadow;")
            })
            .count();
        checked += uses;
        if file_gated {
            continue;
        }
        for line in ungated_uses(&text) {
            panic!("{relative}:{line} names the shadow without the feature gate");
        }
    }
    assert!(
        checked >= 3,
        "lib.rs declares, starts and flushes it: {checked}"
    );
}

/// Regression (D-S2c-7 g): a comment, a doc line or a `not(feature)` attribute that
/// merely names the feature is not a gate.
#[test]
fn the_lint_reads_cfg_attributes_not_comments() {
    let fixture = |lines: &[&str]| lines.join("\n");
    let gate = "#[cfg(feature = \"dictation-shadow\")]";

    // The scratch fixture Sol found passing: an ungated probe in a function preceded
    // by a comment naming the feature.
    let comment_only = fixture(&[
        "// cfg(feature = \"dictation-shadow\") is what this needs",
        "fn live_path() {",
        "    dictation_shadow::probe();",
        "}",
    ]);
    assert_eq!(ungated_uses(&comment_only), vec![3]);

    let doc_only = fixture(&[
        "/// Only with `#[cfg(feature = \"dictation-shadow\")]`.",
        "fn live_path() {",
        "    dictation_shadow::probe();",
        "}",
    ]);
    assert_eq!(ungated_uses(&doc_only), vec![3]);

    let negated = fixture(&[
        "#[cfg(not(feature = \"dictation-shadow\"))]",
        "fn live_path() {",
        "    dictation_shadow::probe();",
        "}",
    ]);
    assert_eq!(ungated_uses(&negated), vec![3]);

    let either = fixture(&[
        "#[cfg(any(test, feature = \"dictation-shadow\"))]",
        "fn live_path() {",
        "    dictation_shadow::probe();",
        "}",
    ]);
    assert_eq!(
        ungated_uses(&either),
        vec![3],
        "`any(test, ..)` exists in the default test build"
    );

    // What is gated stays accepted.
    let gated_fn = fixture(&[
        "// a comment",
        gate,
        "fn live_path() {",
        "    dictation_shadow::probe();",
        "}",
    ]);
    assert!(ungated_uses(&gated_fn).is_empty());

    let gated_all = fixture(&[
        "#[cfg(all(feature = \"dictation-shadow\", test))]",
        "fn live_path() {",
        "    dictation_shadow::probe();",
        "}",
    ]);
    assert!(ungated_uses(&gated_all).is_empty());

    let gated_statement = fixture(&[
        "fn live_path() {",
        "    #[cfg(feature = \"dictation-shadow\")]",
        "    dictation_shadow::probe();",
        "}",
    ]);
    assert!(ungated_uses(&gated_statement).is_empty());

    let gated_same_line = fixture(&[
        "fn live_path() {",
        "    #[cfg(feature = \"dictation-shadow\")] dictation_shadow::probe();",
        "}",
    ]);
    assert!(ungated_uses(&gated_same_line).is_empty());
}
