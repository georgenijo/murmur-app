//! The SP5 ratchet (appendix C7.2, C1.4.2): structural tests over the shell's own
//! source that only ever tighten.
//!
//! * `no_direct_m_field_writes_outside_the_adapter`: every production writer of
//!   M1 `status`, M2 `recording_id`, M3 `cancelled_id` and M4 `active_context`
//!   goes through `FieldAdapter`. The baseline is 0 and a count above it fails.
//! * `status_write_and_terminal_call_sites_only_go_down`: the plan's "15 status
//!   writes, 23 terminal emits" move into the executor family by family. Each
//!   family PR lowers its constant; a count **above** the constant fails (a new
//!   direct writer), and a count **below** it fails too, so a PR that removes
//!   call sites must lower the constant and the ratchet cannot be loosened by
//!   accident.
//! * `persisted_core_new_has_one_caller`: `PersistedCore::new` replaces the whole
//!   persisted value without an effect commit, so only the seeding adapter may
//!   call it (decision D-S1-5 a).
//!
//! The scan sees **production code only**: comments, string and char literals
//! are blanked, `#[cfg(test)]` items are removed, and files that are test-only
//! modules (`#[cfg(test)] mod x;`, with or without `#[path]`) are skipped.

use regex::Regex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Direct writes to the M fields outside the adapter, per field. Only goes down.
const DIRECT_WRITE_BASELINE: [(&str, usize); 4] = [("M1", 0), ("M2", 0), ("M3", 0), ("M4", 0)];

/// `transition_dictation_status(` call sites in production code (the 15 status
/// writes of plan §1; the definition is not counted). Lowered by each family PR
/// that moves its sites behind the executor's `ST` effect.
const STATUS_WRITE_SITES_BASELINE: usize = 5;

/// `emit_dictation_terminal*(` call sites in production code (the 23 terminal
/// emits, plus the one forward inside `emit_dictation_terminal` itself, which
/// the plan's 23 does not count: 24). Lowered by each family PR that moves its
/// sites behind `TERM`.
const TERMINAL_EMIT_SITES_BASELINE: usize = 7;

// ------------------------------------------------------------------ scanner

/// Blank comments and the contents of string and char literals (keeping line
/// structure and the delimiters), so later passes can match code and count
/// braces without being fooled by text.
fn blank_comments_and_literals(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let blank = |c: char| if c == '\n' { '\n' } else { ' ' };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                out.push(' ');
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 0;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    out.push(blank(chars[i]));
                    i += 1;
                }
            }
        } else if (c == 'r' || (c == 'b' && next == Some('r')))
            && (i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_'))
            && {
                let mut j = i + if c == 'b' { 2 } else { 1 };
                while chars.get(j) == Some(&'#') {
                    j += 1;
                }
                chars.get(j) == Some(&'"')
            }
        {
            // Raw string: r"..", r#".."#, br"..".
            let mut j = i + if c == 'b' { 2 } else { 1 };
            let mut hashes = 0;
            while chars.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            out.extend(chars[i..=j].iter());
            i = j + 1;
            while i < chars.len() {
                if chars[i] == '"' && (0..hashes).all(|k| chars.get(i + 1 + k) == Some(&'#')) {
                    out.push('"');
                    out.extend(std::iter::repeat_n('#', hashes));
                    i += 1 + hashes;
                    break;
                }
                out.push(blank(chars[i]));
                i += 1;
            }
        } else if c == '"' {
            out.push('"');
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' {
                    out.push(' ');
                    if let Some(escaped) = chars.get(i + 1) {
                        out.push(blank(*escaped));
                    }
                    i += 2;
                } else if chars[i] == '"' {
                    out.push('"');
                    i += 1;
                    break;
                } else {
                    out.push(blank(chars[i]));
                    i += 1;
                }
            }
        } else if c == '\'' {
            // A char literal ('x', '\n', '\'') or a lifetime ('a).
            let literal_end = if next == Some('\\') {
                (i + 2..chars.len().min(i + 12)).find(|&j| chars[j] == '\'')
            } else if chars.get(i + 2) == Some(&'\'') {
                Some(i + 2)
            } else {
                None
            };
            match literal_end {
                Some(end) => {
                    out.push('\'');
                    out.extend(std::iter::repeat_n(' ', end - i - 1));
                    out.push('\'');
                    i = end + 1;
                }
                None => {
                    out.push(c);
                    i += 1;
                }
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Index just past the bracket group that opens at `chars[open]` (`[`, `(`, `{`).
fn skip_group(chars: &[char], open: usize) -> usize {
    let (opener, closer) = match chars[open] {
        '[' => ('[', ']'),
        '(' => ('(', ')'),
        _ => ('{', '}'),
    };
    let mut depth = 0;
    let mut i = open;
    while i < chars.len() {
        if chars[i] == opener {
            depth += 1;
        } else if chars[i] == closer {
            depth -= 1;
            if depth == 0 {
                return i + 1;
            }
        }
        i += 1;
    }
    chars.len()
}

/// Remove every `#[cfg(test)]` / `#[cfg(all(test, ..))]` item from already
/// blanked source (line structure kept).
fn strip_cfg_test_items(clean: &str) -> String {
    let chars: Vec<char> = clean.chars().collect();
    let mut out: Vec<char> = chars.clone();
    let mut i = 0;
    while i < chars.len() {
        let starts_attr = chars[i] == '#' && chars.get(i + 1) == Some(&'[');
        let is_cfg_test = starts_attr && {
            let end = skip_group(&chars, i + 1);
            let attr: String = chars[i..end].iter().collect();
            let compact: String = attr.chars().filter(|c| !c.is_whitespace()).collect();
            compact == "#[cfg(test)]" || compact.starts_with("#[cfg(all(test,")
        };
        if !is_cfg_test {
            i += 1;
            continue;
        }
        // Skip this attribute and any that follow it, then the item itself.
        let mut j = skip_group(&chars, i + 1);
        loop {
            while chars.get(j).is_some_and(|c| c.is_whitespace()) {
                j += 1;
            }
            if chars.get(j) == Some(&'#') && chars.get(j + 1) == Some(&'[') {
                j = skip_group(&chars, j + 1);
            } else {
                break;
            }
        }
        let mut k = j;
        let end = loop {
            match chars.get(k) {
                None => break chars.len(),
                Some(';') => break k + 1,
                Some('(') | Some('[') => k = skip_group(&chars, k),
                Some('{') => {
                    let mut after = skip_group(&chars, k);
                    let mut peek = after;
                    while chars.get(peek).is_some_and(|c| c.is_whitespace()) {
                        peek += 1;
                    }
                    if chars.get(peek) == Some(&';') {
                        after = peek + 1;
                    }
                    break after;
                }
                Some(_) => k += 1,
            }
        };
        for slot in out.iter_mut().take(end).skip(i) {
            if *slot != '\n' {
                *slot = ' ';
            }
        }
        i = end;
    }
    out.into_iter().collect()
}

/// Production view of one source file.
fn production_code(source: &str) -> String {
    strip_cfg_test_items(&blank_comments_and_literals(source))
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Files that are `#[cfg(test)]` modules declared out of line (`mod x;`, with an
/// optional `#[path = ".."]`), resolved against the declaring file's directory.
fn test_only_files(files: &[(PathBuf, String)]) -> HashSet<PathBuf> {
    let path_attr = Regex::new(r#"#\[path\s*=\s*"([^"]+)"\]"#).expect("regex");
    let mod_decl = Regex::new(r"^\s*(?:pub(?:\([a-z]+\))?\s+)?mod\s+(\w+)\s*;").expect("regex");
    let mut skipped = HashSet::new();
    for (path, source) in files {
        let dir = path.parent().expect("parent");
        let mut cfg_test = false;
        let mut explicit: Option<String> = None;
        for line in source.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("#[cfg(test)]") || trimmed.starts_with("#[cfg(all(test") {
                cfg_test = true;
            } else if let Some(found) = path_attr.captures(trimmed) {
                explicit = Some(found[1].to_string());
            } else if let Some(found) = mod_decl.captures(line) {
                if cfg_test {
                    match explicit.take() {
                        Some(file) => {
                            skipped.insert(dir.join(file));
                        }
                        None => {
                            let name = &found[1];
                            skipped.insert(dir.join(format!("{name}.rs")));
                            skipped.insert(dir.join(name).join("mod.rs"));
                        }
                    }
                }
                cfg_test = false;
                explicit = None;
            } else if !trimmed.starts_with("#[") && !trimmed.is_empty() {
                cfg_test = false;
                explicit = None;
            }
        }
    }
    skipped
}

/// Every `src/**/*.rs` file as `(path relative to src, production code)`.
fn production_sources() -> Vec<(String, String)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut paths = Vec::new();
    rust_files(&src, &mut paths);
    paths.sort();
    let files: Vec<(PathBuf, String)> = paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("read source");
            (path, text)
        })
        .collect();
    let skipped = test_only_files(&files);
    files
        .into_iter()
        .filter(|(path, _)| !skipped.contains(path))
        .map(|(path, text)| {
            let relative = path
                .strip_prefix(&src)
                .expect("under src")
                .to_string_lossy()
                .replace('\\', "/");
            (relative, production_code(&text))
        })
        .collect()
}

fn line_of(code: &str, offset: usize) -> usize {
    code[..offset].matches('\n').count() + 1
}

/// `(file:line, text)` of every match of `pattern`, in files not in `exempt`.
fn hits(sources: &[(String, String)], pattern: &Regex, exempt: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for (file, code) in sources {
        if exempt.contains(&file.as_str()) {
            continue;
        }
        for m in pattern.find_iter(code) {
            found.push(format!(
                "{file}:{} `{}`",
                line_of(code, m.start()),
                m.as_str()
            ));
        }
    }
    found
}

/// The adapter and `state.rs` (where the live slots are defined: `active_context`
/// is private to it, so its primitives live there and only the adapter may call
/// them).
const ADAPTER_FILES: [&str; 2] = ["dictation_executor/adapter.rs", "state.rs"];

fn m_field_patterns() -> [(&'static str, Regex); 4] {
    [
        (
            "M1",
            Regex::new(
                r"\.status\s*=\s*(?:crate::state::)?DictationStatus::|dictation\w*(?:\.lock_or_recover\(\))?\.status\s*=[^=>]|\bguard\.status\s*=[^=>]",
            )
            .expect("regex"),
        ),
        (
            "M2",
            Regex::new(r"\.next_recording_id\(|\brecording_id\s*\.\s*(?:fetch_\w+|store|swap)\(")
                .expect("regex"),
        ),
        (
            "M3",
            Regex::new(r"\.cancel_recording\(|\bcancelled_id\s*\.\s*(?:fetch_\w+|store|swap)\(")
                .expect("regex"),
        ),
        (
            "M4",
            Regex::new(r"\.set_active_context\(|\.clear_active_context\(").expect("regex"),
        ),
    ]
}

fn direct_write_counts(sources: &[(String, String)]) -> Vec<(&'static str, Vec<String>)> {
    m_field_patterns()
        .into_iter()
        .map(|(field, pattern)| (field, hits(sources, &pattern, &ADAPTER_FILES)))
        .collect()
}

fn call_sites(sources: &[(String, String)], pattern: &str) -> Vec<String> {
    let call = Regex::new(pattern).expect("regex");
    let mut found = Vec::new();
    for (file, code) in sources {
        for m in call.find_iter(code) {
            let before = &code[..m.start()];
            if before.trim_end().ends_with("fn") {
                continue; // the definition
            }
            found.push(format!(
                "{file}:{} `{}`",
                line_of(code, m.start()),
                m.as_str()
            ));
        }
    }
    found
}

// -------------------------------------------------------------------- tests

/// SP5, field half: no production code writes M1-M4 except through the adapter.
#[test]
fn no_direct_m_field_writes_outside_the_adapter() {
    let sources = production_sources();
    for (field, found) in direct_write_counts(&sources) {
        let baseline = DIRECT_WRITE_BASELINE
            .iter()
            .find(|(name, _)| *name == field)
            .expect("baseline for every field")
            .1;
        assert!(
            found.len() <= baseline,
            "{field}: {} direct writes outside FieldAdapter (baseline {baseline}); route them \
             through the adapter:\n{}",
            found.len(),
            found.join("\n")
        );
        assert!(
            found.len() >= baseline,
            "{field}: the count fell to {}; lower DIRECT_WRITE_BASELINE (the ratchet only goes down)",
            found.len()
        );
    }
}

/// SP5, call-site half (`no_direct_status_write`): the status and terminal call
/// sites only go down, family by family.
#[test]
fn status_write_and_terminal_call_sites_only_go_down() {
    let sources = production_sources();
    let status = call_sites(&sources, r"\btransition_dictation_status\s*\(");
    let terminal = call_sites(&sources, r"\bemit_dictation_terminal\w*\s*\(");
    for (what, found, baseline) in [
        (
            "transition_dictation_status",
            &status,
            STATUS_WRITE_SITES_BASELINE,
        ),
        (
            "emit_dictation_terminal*",
            &terminal,
            TERMINAL_EMIT_SITES_BASELINE,
        ),
    ] {
        assert!(
            found.len() <= baseline,
            "{what}: {} call sites, baseline {baseline}; a new direct caller must go through the executor:\n{}",
            found.len(),
            found.join("\n")
        );
        assert!(
            found.len() >= baseline,
            "{what}: the count fell to {}; lower its baseline constant (the ratchet only goes down)",
            found.len()
        );
    }
}

/// The one function allowed to call `PersistedCore::new` (decision D-S1-5 a).
const SEEDING_FILE: &str = "dictation_executor/adapter.rs";
const SEEDING_FN: &str = "seed_core";

/// Every `PersistedCore::new` in `sources` (comments and literals already
/// blanked) as `(file, line, enclosing fn)`.
fn persisted_core_new_callers(sources: &[(String, String)]) -> Vec<(String, usize, String)> {
    let needle = Regex::new(r"\bPersistedCore\s*::\s*new\b").expect("regex");
    let function = Regex::new(r"\bfn\s+(\w+)").expect("regex");
    let mut callers = Vec::new();
    for (file, code) in sources {
        for m in needle.find_iter(code) {
            let enclosing = function
                .captures_iter(&code[..m.start()])
                .last()
                .map(|found| found[1].to_string())
                .unwrap_or_default();
            callers.push((file.clone(), line_of(code, m.start()), enclosing));
        }
    }
    callers
}

/// `Ok` iff there is exactly one caller and it is `SEEDING_FN` in `SEEDING_FILE`.
fn check_single_seeder(callers: &[(String, usize, String)]) -> Result<(), String> {
    match callers {
        [(file, _, function)] if file == SEEDING_FILE && function == SEEDING_FN => Ok(()),
        _ => Err(format!(
            "PersistedCore::new must have exactly one caller, `{SEEDING_FN}` in {SEEDING_FILE} \
             (it replaces the whole persisted value without an effect commit); found: {callers:?}"
        )),
    }
}

/// Decision D-S1-5 (a): `PersistedCore::new` replaces the whole persisted value
/// without a commit. Exactly one call exists in `src/` (the core crate itself is
/// outside it): `FieldAdapter::seed_core`. Any other adapter method that needs
/// the persisted core re-synced calls `seed_core`; it never builds one. Test
/// code is scanned too: a test that needs a core equal to live uses `seed_core`.
#[test]
fn persisted_core_new_has_one_caller() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut paths = Vec::new();
    rust_files(&src, &mut paths);
    paths.sort();
    let sources: Vec<(String, String)> = paths
        .iter()
        .map(|path| {
            let code = blank_comments_and_literals(&std::fs::read_to_string(path).expect("read"));
            let relative = path
                .strip_prefix(&src)
                .expect("under src")
                .to_string_lossy()
                .replace('\\', "/");
            (relative, code)
        })
        .collect();
    if let Err(message) = check_single_seeder(&persisted_core_new_callers(&sources)) {
        panic!("{message}");
    }
}

// ------------------------------------------------------------- scanner checks

#[test]
fn scanner_ignores_comments_strings_and_cfg_test_items() {
    let source = r##"
// state.dictation.status = DictationStatus::Idle;
/* app.cancel_recording(1); /* nested */ app.cancel_recording(2); */
fn real(app: &AppState) {
    let text = "app.cancel_recording(3); { unbalanced";
    let raw = r#"app.cancel_recording(4); " }"#;
    let ch = '{';
    let lifetime: &'static str = "x";
    app.cancel_recording(5);
}
#[cfg(test)]
mod tests {
    fn t(app: &AppState) { app.cancel_recording(6); let s = "}"; }
}
#[cfg(test)]
fn only_in_tests(app: &AppState) -> u8 { app.cancel_recording(7); 0 }
#[cfg(test)]
use std::fmt;
fn after(app: &AppState) { app.cancel_recording(8); }
"##;
    let code = production_code(source);
    let cancels = Regex::new(r"\.cancel_recording\((\d)\)").expect("regex");
    let seen: Vec<&str> = cancels
        .captures_iter(&code)
        .map(|c| c.get(1).expect("digit").as_str())
        .collect();
    assert_eq!(seen, ["5", "8"], "{code}");
    assert!(!code.contains("DictationStatus::Idle"));
}

#[test]
fn scanner_flags_a_planted_direct_write() {
    let planted = vec![(
        "commands/recording.rs".to_string(),
        production_code(
            "fn bad(state: &State) { state.app_state.dictation.lock_or_recover().status = \
             DictationStatus::Idle; let id = state.app_state.next_recording_id(); \
             state.app_state.clear_active_context(id); }",
        ),
    )];
    let counts = direct_write_counts(&planted);
    let by_field = |field: &str| {
        counts
            .iter()
            .find(|(name, _)| *name == field)
            .map(|(_, f)| f.len())
    };
    assert_eq!(by_field("M1"), Some(1));
    assert_eq!(by_field("M2"), Some(1));
    assert_eq!(by_field("M3"), Some(0));
    assert_eq!(by_field("M4"), Some(1));
    // The same text inside the adapter is exempt.
    let exempt = vec![(ADAPTER_FILES[0].to_string(), planted[0].1.clone())];
    assert!(direct_write_counts(&exempt)
        .iter()
        .all(|(_, f)| f.is_empty()));
}

#[test]
fn test_only_module_files_are_skipped() {
    let files = production_sources();
    let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
    for test_file in [
        "commands/recording_contract_tests.rs",
        "commands/recording_golden_harness.rs",
        "commands/recording_pd_witness_tests.rs",
        "dictation_executor/ratchet_tests.rs",
    ] {
        assert!(!names.contains(&test_file), "{test_file} must be skipped");
    }
    assert!(names.contains(&"commands/recording.rs"));
    assert!(names.contains(&"dictation_executor/adapter.rs"));
}

/// The D-S1-5 (a) gate fails on a second caller anywhere: in another file, in
/// another adapter method, or twice inside the seeder.
#[test]
fn scanner_flags_an_extra_persisted_core_new_inside_the_adapter() {
    let adapter = |code: &str| vec![(SEEDING_FILE.to_string(), production_code(code))];
    let check =
        |sources: &[(String, String)]| check_single_seeder(&persisted_core_new_callers(sources));
    let seeder = "fn seed_core(a: &A) { d.core = PersistedCore::new(s, i); }";
    assert_eq!(check(&adapter(seeder)), Ok(()));
    // The shape that used to pass: a second constructor call in another method.
    let in_other_method = format!(
        "{seeder} fn alloc_id(a: &A) {{ d.core = PersistedCore::new(s, RecordingId(id)); }}"
    );
    assert!(check(&adapter(&in_other_method)).is_err());
    // The only call in the wrong method.
    assert!(check(&adapter("fn alloc_id() { PersistedCore::new(s, i); }")).is_err());
    // Two calls inside the seeder itself.
    let twice = "fn seed_core() { PersistedCore::new(a, b); PersistedCore::new(c, d); }";
    assert!(check(&adapter(twice)).is_err());
    // Another file.
    let mut elsewhere = adapter(seeder);
    elsewhere.push(("commands/recording.rs".into(), production_code(seeder)));
    assert!(check(&elsewhere).is_err());
    // No caller at all.
    assert!(check(&adapter("fn seed_core() {}")).is_err());
    // A comment or a string mentioning it is not a caller.
    assert_eq!(
        check(&adapter(&format!(
            "{seeder} // PersistedCore::new(x)\nfn f() {{ let t = \"PersistedCore::new(\"; }}"
        ))),
        Ok(())
    );
}
