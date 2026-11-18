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
//! * `terminal_emit_callers_are_exactly_the_allowlisted_functions`: the terminal
//!   emits that remain are pinned **per enclosing function**, like the
//!   `PersistedCore::new` check, so swapping a `TERM` call for an inline emit in
//!   another function fails even though the total count is unchanged.
//! * `persisted_core_new_has_one_caller`: `PersistedCore::new` replaces the whole
//!   persisted value without an effect commit, so only the seeding adapter may
//!   call it (decision D-S1-5 a).
//!
//! The scan sees **production code only**, and fails closed (D-S2-17). It walks every
//! `.rs` file under `src/` and skips a whole file only when it is listed in
//! `TEST_ONLY_FILES` (or the commented naming exceptions); the ratchet no longer
//! resolves the module tree. Guard tests keep the list honest: every entry exists and
//! follows the test naming convention. Inside a scanned file `#[cfg(test)]` items
//! are skipped by their `cfg` predicate (only a predicate that requires `test`), also
//! through the attributes of an enclosing `extern`, `impl`, `trait` or inline `mod`.
//! The per-function call-site checks parse real Rust syntax (`syn`) and name each
//! caller by its qualified identity (module path, `impl` type or `<Type as Trait>`,
//! fn names), so same-named functions in sibling modules, impls or trait impls
//! differ; the textual checks blank comments and literals first.

use regex::Regex;
use std::path::{Path, PathBuf};

/// Direct writes to the M fields outside the adapter, per field. Only goes down.
const DIRECT_WRITE_BASELINE: [(&str, usize); 4] = [("M1", 0), ("M2", 0), ("M3", 0), ("M4", 0)];

/// `transition_dictation_status(` call sites in production code (the 15 status
/// writes of plan §1; the definition is not counted). Lowered by each family PR
/// that moves its sites behind the executor's `ST` effect.
const STATUS_WRITE_SITES_BASELINE: usize = 0;

/// `emit_dictation_terminal*(` call sites in production code (the 23 terminal
/// emits, plus the one forward inside `emit_dictation_terminal` itself, which
/// the plan's 23 does not count: 24). Lowered by each family PR that moves its
/// sites behind `TERM`.
const TERMINAL_EMIT_SITES_BASELINE: usize = 6;

/// The functions that may still call `emit_dictation_terminal*(`, with the number
/// of calls each makes: `(file relative to src, qualified enclosing fn, calls)`
/// (module path, the `impl` as `Type` or `<Type as Trait>`, then fn names, joined by
/// `::`). The sum is
/// [`TERMINAL_EMIT_SITES_BASELINE`]. Moving a site behind `TERM` lowers its entry;
/// a call in any other function, or more calls in one, fails.
const TERMINAL_EMIT_ALLOWLIST: [(&str, &str, usize); 6] = [
    // The forward inside the helper itself (the plan's 23 does not count it).
    ("commands/recording.rs", "emit_dictation_terminal", 1),
    // The stop's own inline terminal: not yet behind `TERM`.
    ("commands/recording.rs", "stop_native_recording_admitted", 1),
    // The four families' `TERM` ports: the executor's terminal effect.
    (
        "commands/recording/cancel_ports.rs",
        "<CancelPorts<'_> as Ports>::terminal",
        1,
    ),
    (
        "commands/recording/lifecycle_ports.rs",
        "<LifecyclePorts<'_,R> as Ports>::terminal",
        1,
    ),
    (
        "commands/recording/start_ports.rs",
        "<StartPorts<'_> as Ports>::terminal",
        1,
    ),
    (
        "commands/recording/stop_ports.rs",
        "<StopPorts<'_> as Ports>::terminal",
        1,
    ),
];

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

/// Files that are test-only (compiled only under `#[cfg(test)]`, or `include!`d
/// into a test module), skipped whole by the scan. A file that is not listed is
/// scanned as production: a test file missing here is a loud false positive, never a
/// silent miss (D-S2-17). Paths are relative to `src/`; every entry follows the
/// naming convention ([`follows_test_naming`]) and exists (guard tests).
const TEST_ONLY_FILES: &[&str] = &[
    "capture_preview_handoff_tests.rs",
    "capture_speculative_tests.rs",
    "capture_standby_tests.rs",
    "capture_stop_diagnostics_tests.rs",
    "commands/recording_bug_replay_tests.rs",
    "commands/recording_cancel_tests.rs",
    "commands/recording_contract_tests.rs",
    "commands/recording_deferral_tests.rs",
    "commands/recording_golden_tests.rs",
    "commands/recording_host_tests.rs",
    "commands/recording_lifecycle_barrier_tests.rs",
    "commands/recording_lifecycle_tests.rs",
    "commands/recording_pd_witness_tests.rs",
    "commands/recording_preview_tests.rs",
    "commands/recording_process_audio_tests.rs",
    "commands/recording_start_tests.rs",
    "commands/recording_stop_tests.rs",
    "commands/recording_stream_reuse_tests.rs",
    "dictation_context_equivalence_tests.rs",
    "dictation_executor/ratchet_tests.rs",
    "frontmost/soak_tests.rs",
    "meeting_store/search_tests.rs",
    "release_tail_tests.rs",
    "stream_reuse_tests.rs",
    "vad_stream_tests.rs",
];

/// Test-only files that do not follow the naming convention. They are not renamed in
/// the ratchet's PR; each is skipped like a listed file and named in the PR body.
/// Every entry must NOT follow the convention (a guard test), so this list cannot
/// absorb a conforming file.
const TEST_ONLY_NAMING_EXCEPTIONS: &[&str] = &[
    // `#[cfg(test)] mod barrier_hook;` (lib.rs): the test barrier registry.
    "barrier_hook.rs",
    // `#[cfg(test)] #[path] mod golden_harness;` (recording.rs): the golden harness.
    "commands/recording_golden_harness.rs",
    // `#[cfg(test)] mod dictation_diagnostics_contract;` (lib.rs).
    "dictation_diagnostics_contract.rs",
    // The stream-reuse probe module, compiled only for tests.
    "stream_reuse_probe.rs",
];

/// The test naming convention: `*_tests.rs`, `tests.rs`, or a path under a `tests/`
/// or `test_support/` directory.
fn follows_test_naming(relative: &str) -> bool {
    let mut parts: Vec<&str> = relative.split('/').collect();
    let file = parts.pop().unwrap_or("");
    file == "tests.rs"
        || file.ends_with("_tests.rs")
        || parts
            .iter()
            .any(|dir| *dir == "tests" || *dir == "test_support")
}

/// Whether the scan skips the whole file at `relative` (a path relative to `src/`).
fn is_test_only_file(relative: &str) -> bool {
    TEST_ONLY_FILES.contains(&relative) || TEST_ONLY_NAMING_EXCEPTIONS.contains(&relative)
}

/// The attribute lines directly above line `at` (comment lines between them are
/// ignored): the contiguous run of `#[..]` lines.
fn attribute_lines_above<'l>(lines: &[&'l str], at: usize) -> Vec<&'l str> {
    let mut attrs = Vec::new();
    let mut index = at;
    while index > 0 {
        index -= 1;
        let line = lines[index].trim();
        if line.starts_with("#[") {
            attrs.push(line);
        } else if !(line.starts_with("//") || line.is_empty()) {
            break;
        }
    }
    attrs
}

/// Whether any of the attribute `lines` is a `cfg` whose predicate requires `test`.
fn lines_require_test(attrs: &[&str]) -> bool {
    attrs.iter().any(|line| {
        syn::parse::Parser::parse_str(syn::Attribute::parse_outer, line)
            .is_ok_and(|parsed| parsed.iter().any(is_cfg_test))
    })
}

fn indentation(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// A declaration grep (not module resolution): every `mod <name>;` or
/// `include!("..")` line in `sources` that declares the file at `relative`, and
/// whether `#[cfg(test)]` (a predicate that requires `test`) gates it, through the
/// attribute lines directly above it or above an enclosing `mod x {` block. A
/// `mod` line declares a file by its name (the file stem, or the directory for
/// `mod.rs`) or by a `#[path = ".."]` attribute naming it; an `include!` by its
/// path. Returns `(ungated declarations, all declarations)` as `file:line`.
fn declarations_of(sources: &[(String, String)], relative: &str) -> (Vec<String>, Vec<String>) {
    let file_name = relative.rsplit('/').next().unwrap_or(relative);
    let stem = file_name.trim_end_matches(".rs");
    let module_name = if stem == "mod" {
        relative.rsplit('/').nth(1).unwrap_or(stem)
    } else {
        stem
    };
    let mod_line = Regex::new(r"^(?:pub(?:\([a-z:]+\))?\s+)?mod\s+(\w+)\s*;").expect("regex");
    let include_line = Regex::new(r#"include!\(\s*"([^"]+)"\s*\)"#).expect("regex");
    let path_attr = Regex::new(r#"^#\[path\s*=\s*"([^"]+)"\s*\]"#).expect("regex");
    let names_file = |path: &str| path.rsplit('/').next() == Some(file_name);
    let (mut ungated, mut all) = (Vec::new(), Vec::new());
    for (source_file, source) in sources {
        let lines: Vec<&str> = source.lines().collect();
        for (at, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            let attrs = attribute_lines_above(&lines, at);
            let declares = if let Some(found) = mod_line.captures(trimmed) {
                let explicit = attrs
                    .iter()
                    .find_map(|attr| path_attr.captures(attr).map(|c| c[1].to_string()));
                match explicit {
                    Some(path) => names_file(&path),
                    None => &found[1] == module_name,
                }
            } else if let Some(found) = include_line.captures(trimmed) {
                names_file(&found[1])
            } else {
                false
            };
            if !declares {
                continue;
            }
            let here = format!("{source_file}:{}", at + 1);
            all.push(here.clone());
            // Gated by its own attributes, or by an enclosing `mod x {` block.
            let mut gated = lines_require_test(&attrs);
            let mut outer = indentation(line);
            for above in (0..at).rev() {
                if gated || outer == 0 {
                    break;
                }
                let candidate = lines[above];
                let header = candidate.trim_start();
                if !candidate.trim().is_empty()
                    && indentation(candidate) < outer
                    && (header.starts_with("mod ") || header.starts_with("pub mod "))
                    && header.trim_end().ends_with('{')
                {
                    outer = indentation(candidate);
                    gated = lines_require_test(&attribute_lines_above(&lines, above));
                }
            }
            if !gated {
                ungated.push(here);
            }
        }
    }
    (ungated, all)
}

/// D-S2-18 (a): the last fail-open path. A file in `TEST_ONLY_FILES` or
/// `TEST_ONLY_NAMING_EXCEPTIONS` is skipped whole, so something must declare it, and
/// every declaration (`mod <name>;`, `#[path]`, `include!`) must be gated by
/// `#[cfg(test)]`; otherwise a production module could be listed and silently dropped.
#[test]
fn every_test_only_file_is_declared_under_cfg_test() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    // Every file is searched, the listed ones too (a declaration can live in a test
    // module that a gated parent declares), except this file, whose fixtures are
    // text about declarations.
    let sources = scan_dir(&src, &|relative| {
        relative == "dictation_executor/ratchet_tests.rs"
    });
    for file in TEST_ONLY_FILES.iter().chain(TEST_ONLY_NAMING_EXCEPTIONS) {
        let (ungated, all) = declarations_of(&sources, file);
        assert!(
            !all.is_empty(),
            "{file} is listed as test-only but no `mod` or `include!` line declares it"
        );
        assert!(
            ungated.is_empty(),
            "{file} is listed as test-only but is declared without `#[cfg(test)]` at {ungated:?}"
        );
    }
}

/// The declaration grep on fixtures: gated, ungated, `#[path]`, `include!` in a gated
/// block, a predicate that does not require `test`, and no declaration at all.
#[test]
fn the_declaration_grep_sees_a_missing_cfg_test() {
    let one = |code: &str| vec![("lib.rs".to_string(), code.to_string())];
    let gated = |code: &str, file: &str| {
        let (ungated, all) = declarations_of(&one(code), file);
        (all.len(), ungated.len())
    };
    assert_eq!(
        gated("#[cfg(test)]\nmod helper_tests;\n", "helper_tests.rs"),
        (1, 0)
    );
    assert_eq!(gated("mod helper_tests;\n", "helper_tests.rs"), (1, 1));
    assert_eq!(
        gated(
            "#[cfg(any(test, unix))]\nmod helper_tests;\n",
            "helper_tests.rs"
        ),
        (1, 1)
    );
    assert_eq!(
        gated(
            "#[cfg(test)]\n#[path = \"dir/harness.rs\"]\nmod golden;\n",
            "dir/harness.rs"
        ),
        (1, 0)
    );
    assert_eq!(
        gated("#[path = \"harness.rs\"]\nmod golden;\n", "harness.rs"),
        (1, 1)
    );
    // A `mod.rs` is declared by its directory name.
    assert_eq!(
        gated("#[cfg(test)]\nmod nested;\n", "nested/mod.rs"),
        (1, 0)
    );
    // `include!` inside a gated block, and inside an ungated one.
    let block = |attr: &str| {
        format!("{attr}\nmod tests {{\n    mod inner {{\n        include!(\"a_tests.rs\");\n    }}\n}}\n")
    };
    assert_eq!(gated(&block("#[cfg(test)]"), "a_tests.rs"), (1, 0));
    assert_eq!(gated(&block(""), "a_tests.rs"), (1, 1));
    // Two declarations: one ungated is enough to fail.
    assert_eq!(
        gated(
            "#[cfg(test)]\nmod x_tests;\n#[path = \"x_tests.rs\"]\nmod again;\n",
            "x_tests.rs"
        ),
        (2, 1)
    );
    // No declaration at all.
    assert_eq!(gated("mod other;\n", "x_tests.rs"), (0, 0));
}

/// `(path relative to dir, raw source)` of every `.rs` file under `dir` that
/// `skip` does not name, sorted. No module resolution: whatever is not skipped by
/// name is scanned.
fn scan_dir(dir: &Path, skip: &dyn Fn(&str) -> bool) -> Vec<(String, String)> {
    let mut paths = Vec::new();
    rust_files(dir, &mut paths);
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| {
            let relative = path
                .strip_prefix(dir)
                .expect("under dir")
                .to_string_lossy()
                .replace('\\', "/");
            if skip(&relative) {
                None
            } else {
                Some((
                    relative,
                    std::fs::read_to_string(&path).expect("read source"),
                ))
            }
        })
        .collect()
}

/// Every `src/**/*.rs` file as `(path relative to src, production code)`, with
/// comments and literals blanked and `#[cfg(test)]` items removed.
fn production_sources() -> Vec<(String, String)> {
    raw_production_sources()
        .into_iter()
        .map(|(relative, text)| (relative, production_code(&text)))
        .collect()
}

/// Every `src/**/*.rs` file that is not test-only (see [`TEST_ONLY_FILES`]) as
/// `(path relative to src, raw source)`, for the syntax-aware scanner (it skips
/// `#[cfg(test)]` items itself).
fn raw_production_sources() -> Vec<(String, String)> {
    scan_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &is_test_only_file,
    )
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

// ------------------------------------------------ syntax-aware call-site scanner

/// Where a use lives: the qualified identity of its enclosing function, built from
/// real Rust syntax: inline module path, the `impl` / `trait` the function is in,
/// then the `fn` names (nested functions append). An inherent impl contributes its
/// full implementing type (`StopPorts<'a>`), a trait impl contributes
/// `<Type as Trait>` with both written out (two trait impls of one type, each with a
/// `terminal`, are two identities). A use outside any function body (a `const` /
/// `static` initializer) is `<const NAME>` / `<static NAME>` under its module path.
/// A closure belongs to the function that contains it.
///
/// A "use" is a call, a method call, a path used as a value (`let make =
/// PersistedCore::new;`), or any of these written inside a macro's tokens. Limits:
/// a renamed `use .. as ..` import is not followed.
struct SiteFinder<'m> {
    scope: Vec<String>,
    /// Scan `#[cfg(test)]` items too (the single-seeder check covers test code).
    include_cfg_test: bool,
    is_target: &'m dyn Fn(&[String]) -> bool,
    hits: Vec<String>,
    /// Record every call, method call and macro with its enclosing function and that
    /// function's token hash (the event-name scanner, SP5 status events).
    record_calls: bool,
    calls: Vec<CallRecord>,
    /// The token hash of each enclosing function, innermost last.
    fn_hashes: Vec<u64>,
}

impl SiteFinder<'_> {
    fn hit(&mut self) {
        self.hits.push(self.scope.join("::"));
    }

    /// Uses written inside a macro invocation's tokens (`name(..)`, `a::b(..)`, or
    /// the bare path): the macro's arguments are not parsed as expressions.
    fn scan_tokens(&mut self, tokens: proc_macro2::TokenStream) {
        use proc_macro2::TokenTree;
        let mut chain: Vec<String> = Vec::new();
        let flush = |this: &mut Self, chain: &mut Vec<String>| {
            if !chain.is_empty() && (this.is_target)(chain) {
                this.hit();
            }
            chain.clear();
        };
        for token in tokens {
            match token {
                TokenTree::Ident(ident) => {
                    chain.push(syn::ext::IdentExt::unraw(&ident).to_string())
                }
                TokenTree::Punct(punct) if punct.as_char() == ':' => {}
                TokenTree::Group(group) => {
                    flush(self, &mut chain);
                    self.scan_tokens(group.stream());
                }
                _ => flush(self, &mut chain),
            }
        }
        flush(self, &mut chain);
    }

    fn skip(&self, attrs: &[syn::Attribute]) -> bool {
        !self.include_cfg_test && attrs.iter().any(is_cfg_test)
    }

    fn record(&mut self, callee: String, args: Option<Vec<syn::Expr>>, path_call: bool) {
        if !self.record_calls {
            return;
        }
        self.calls.push(CallRecord {
            function: self.scope.join("::"),
            callee,
            args,
            path_call,
            fn_hash: self.fn_hashes.last().copied().unwrap_or(0),
        });
    }

    /// A macro invocation, with its arguments when they parse as expressions (the
    /// calls inside them are visited too), else the calls written in its tokens.
    fn record_macro(&mut self, mac: &syn::Macro) {
        use syn::visit::Visit;
        let Some(last) = mac.path.segments.last() else {
            return;
        };
        let name = plain(&last.ident);
        match parse_expr_list(mac.tokens.clone()) {
            Some(args) => {
                if !PURE_MACROS.contains(&name.as_str()) {
                    self.record(name, Some(args.clone()), false);
                }
                for arg in &args {
                    self.visit_expr(arg);
                }
            }
            None => self.record_token_calls(mac.tokens.clone()),
        }
    }

    /// `name(..)` written inside macro tokens: recorded with its arguments when they
    /// parse, and (unparsed) when `name` is an emit API.
    fn record_token_calls(&mut self, tokens: proc_macro2::TokenStream) {
        use proc_macro2::{Delimiter, TokenTree};
        let mut previous: Option<String> = None;
        for token in tokens {
            match token {
                TokenTree::Ident(ident) => previous = Some(plain(&ident)),
                TokenTree::Group(group) => {
                    if group.delimiter() == Delimiter::Parenthesis {
                        if let Some(name) = previous.take() {
                            let args = parse_expr_list(group.stream());
                            if args.is_some() || is_emit_api_name(&name) {
                                self.record(name, args, false);
                            }
                        }
                    }
                    previous = None;
                    self.record_token_calls(group.stream());
                }
                _ => previous = None,
            }
        }
    }

    /// Enter a function: its token hash is computed up front.
    fn within_fn(
        &mut self,
        segment: String,
        hash: impl FnOnce() -> u64,
        walk: impl FnOnce(&mut Self),
    ) {
        let pushed = self.record_calls;
        if pushed {
            self.fn_hashes.push(hash());
        }
        self.within(segment, walk);
        if pushed {
            self.fn_hashes.pop();
        }
    }

    fn within(&mut self, segment: String, walk: impl FnOnce(&mut Self)) {
        self.scope.push(segment);
        walk(self);
        self.scope.pop();
    }
}

/// Whether `meta` (a `cfg` predicate) can only be true when `test` is: `test`,
/// `all(test, ..)` (any member requires it), `any(..)` when every member requires
/// it, and `not(not(..))`. `any(test, target_os = "macos")` is also true outside
/// `test`, so it is production code; so is `not(test)`.
fn requires_test(meta: &syn::Meta) -> bool {
    let members = |list: &syn::MetaList| -> Vec<syn::Meta> {
        list.parse_args_with(
            syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
        )
        .map(|members| members.into_iter().collect())
        .unwrap_or_default()
    };
    match meta {
        syn::Meta::Path(path) => path.is_ident("test"),
        syn::Meta::List(list) if list.path.is_ident("all") => {
            members(list).iter().any(requires_test)
        }
        syn::Meta::List(list) if list.path.is_ident("any") => {
            let members = members(list);
            !members.is_empty() && members.iter().all(requires_test)
        }
        syn::Meta::List(list) if list.path.is_ident("not") => match members(list).as_slice() {
            [syn::Meta::List(inner)] if inner.path.is_ident("not") => {
                matches!(members(inner).as_slice(), [only] if requires_test(only))
            }
            _ => false,
        },
        _ => false,
    }
}

/// `#[cfg(<predicate>)]` where the predicate requires `test`.
fn is_cfg_test(attr: &syn::Attribute) -> bool {
    if !attr.path().is_ident("cfg") {
        return false;
    }
    attr.parse_args::<syn::Meta>()
        .is_ok_and(|predicate| requires_test(&predicate))
}

/// An identifier without a raw-identifier prefix: `r#PersistedCore` is `PersistedCore`.
fn plain(ident: &syn::Ident) -> String {
    syn::ext::IdentExt::unraw(ident).to_string()
}

/// A type or path as written, without spaces: `StopPorts<'_>`, `a::B<T>`.
fn written<T: quote::ToTokens>(node: &T) -> String {
    node.to_token_stream()
        .to_string()
        .replace(' ', "")
        .replace("r#", "")
}

/// The scope segment of an `impl` block.
fn impl_segment(item: &syn::ItemImpl) -> String {
    match &item.trait_ {
        Some((negation, trait_path, _)) => format!(
            "<{} as {}{}>",
            written(&*item.self_ty),
            if negation.is_some() { "!" } else { "" },
            written(trait_path)
        ),
        None => written(&*item.self_ty),
    }
}

impl<'ast> syn::visit::Visit<'ast> for SiteFinder<'_> {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if self.skip(&item.attrs) {
            return;
        }
        self.within(plain(&item.ident), |this| {
            syn::visit::visit_item_mod(this, item)
        });
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if self.skip(&item.attrs) {
            return;
        }
        self.within(impl_segment(item), |this| {
            syn::visit::visit_item_impl(this, item)
        });
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        if self.skip(&item.attrs) {
            return;
        }
        self.within(plain(&item.ident), |this| {
            syn::visit::visit_item_trait(this, item)
        });
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if self.skip(&item.attrs) {
            return;
        }
        self.within_fn(
            plain(&item.sig.ident),
            || token_hash(item),
            |this| syn::visit::visit_item_fn(this, item),
        );
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if self.skip(&item.attrs) {
            return;
        }
        self.within_fn(
            plain(&item.sig.ident),
            || token_hash(item),
            |this| syn::visit::visit_impl_item_fn(this, item),
        );
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        if self.skip(&item.attrs) {
            return;
        }
        self.within_fn(
            plain(&item.sig.ident),
            || token_hash(item),
            |this| syn::visit::visit_trait_item_fn(this, item),
        );
    }

    fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
        if self.skip(&item.attrs) {
            return;
        }
        self.within(format!("<const {}>", plain(&item.ident)), |this| {
            syn::visit::visit_item_const(this, item)
        });
    }

    fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
        if self.skip(&item.attrs) {
            return;
        }
        self.within(format!("<static {}>", plain(&item.ident)), |this| {
            syn::visit::visit_item_static(this, item)
        });
    }

    fn visit_impl_item_const(&mut self, item: &'ast syn::ImplItemConst) {
        if self.skip(&item.attrs) {
            return;
        }
        self.within(format!("<const {}>", plain(&item.ident)), |this| {
            syn::visit::visit_impl_item_const(this, item)
        });
    }

    /// An item-position macro: a `macro_rules!` definition (its body is scanned as
    /// tokens) or an invocation. A `#[cfg(test)]` one is not production.
    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        if self.skip(&item.attrs) {
            return;
        }
        syn::visit::visit_item_macro(self, item);
    }

    /// An `extern` block: `#[cfg(test)] extern \"C\" { m!(..) }` is not production
    /// whatever its members carry.
    fn visit_item_foreign_mod(&mut self, item: &'ast syn::ItemForeignMod) {
        if self.skip(&item.attrs) {
            return;
        }
        syn::visit::visit_item_foreign_mod(self, item);
    }

    /// An associated macro in an `impl`, a `trait` or an `extern` block.
    fn visit_impl_item_macro(&mut self, item: &'ast syn::ImplItemMacro) {
        if self.skip(&item.attrs) {
            return;
        }
        syn::visit::visit_impl_item_macro(self, item);
    }

    fn visit_trait_item_macro(&mut self, item: &'ast syn::TraitItemMacro) {
        if self.skip(&item.attrs) {
            return;
        }
        syn::visit::visit_trait_item_macro(self, item);
    }

    fn visit_foreign_item_macro(&mut self, item: &'ast syn::ForeignItemMacro) {
        if self.skip(&item.attrs) {
            return;
        }
        syn::visit::visit_foreign_item_macro(self, item);
    }

    fn visit_stmt_macro(&mut self, item: &'ast syn::StmtMacro) {
        if self.skip(&item.attrs) {
            return;
        }
        syn::visit::visit_stmt_macro(self, item);
    }

    fn visit_expr_macro(&mut self, item: &'ast syn::ExprMacro) {
        if self.skip(&item.attrs) {
            return;
        }
        syn::visit::visit_expr_macro(self, item);
    }

    fn visit_local(&mut self, item: &'ast syn::Local) {
        if self.skip(&item.attrs) {
            return;
        }
        syn::visit::visit_local(self, item);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if self.skip(&call.attrs) {
            return;
        }
        if let syn::Expr::Path(function) = &*call.func {
            if let Some(last) = function.path.segments.last() {
                self.record(
                    plain(&last.ident),
                    Some(call.args.iter().cloned().collect()),
                    true,
                );
            }
        }
        syn::visit::visit_expr_call(self, call);
    }

    /// A path used as a value or called (`PersistedCore::new(..)`, or `let make =
    /// PersistedCore::new;`): the call's function is such a path, so a call is counted
    /// once, here.
    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        if self.skip(&path.attrs) {
            return;
        }
        let segments: Vec<String> = path
            .path
            .segments
            .iter()
            .map(|segment| plain(&segment.ident))
            .collect();
        if (self.is_target)(&segments) {
            self.hit();
        }
        syn::visit::visit_expr_path(self, path);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if self.skip(&call.attrs) {
            return;
        }
        if (self.is_target)(&[plain(&call.method)]) {
            self.hit();
        }
        self.record(
            plain(&call.method),
            Some(call.args.iter().cloned().collect()),
            false,
        );
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if self.record_calls {
            self.record_macro(mac);
        }
        self.scan_tokens(mac.tokens.clone());
        syn::visit::visit_macro(self, mac);
    }
}

/// The enclosing-function identity of every use `is_target` accepts in `source`
/// (parsed as a Rust file; one entry per use).
fn call_scopes(
    source: &str,
    include_cfg_test: bool,
    is_target: &dyn Fn(&[String]) -> bool,
) -> Vec<String> {
    use syn::visit::Visit;
    let file = syn::parse_file(source).unwrap_or_else(|error| panic!("parse a source: {error}"));
    let mut finder = SiteFinder {
        scope: Vec::new(),
        include_cfg_test,
        is_target,
        hits: Vec::new(),
        record_calls: false,
        calls: Vec::new(),
        fn_hashes: Vec::new(),
    };
    finder.visit_file(&file);
    finder.hits
}

/// `emit_dictation_terminal*` (a call, a path used as a value, a method call, or
/// inside a macro's tokens).
fn is_terminal_emit(segments: &[String]) -> bool {
    segments
        .last()
        .is_some_and(|name| name.starts_with("emit_dictation_terminal"))
}

/// `PersistedCore::new`, called or used as a value.
fn is_persisted_core_new(segments: &[String]) -> bool {
    segments.ends_with(&["PersistedCore".to_string(), "new".to_string()])
}

/// `(file, qualified enclosing fn, calls)` of every `emit_dictation_terminal*(` call
/// in the production code of `sources` (`(file, raw Rust source)`; the definitions
/// are not calls, `#[cfg(test)]` items are skipped), sorted.
fn terminal_emit_callers(sources: &[(String, String)]) -> Vec<(String, String, usize)> {
    let mut counts: std::collections::BTreeMap<(String, String), usize> =
        std::collections::BTreeMap::new();
    for (file, source) in sources {
        for function in call_scopes(source, false, &is_terminal_emit) {
            *counts.entry((file.clone(), function)).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .map(|((file, function), calls)| (file, function, calls))
        .collect()
}

/// `Ok` iff the callers are exactly `allowed`.
fn check_terminal_emit_callers(
    callers: &[(String, String, usize)],
    allowed: &[(&str, &str, usize)],
) -> Result<(), String> {
    let mut want: Vec<(String, String, usize)> = allowed
        .iter()
        .map(|(file, function, calls)| (file.to_string(), function.to_string(), *calls))
        .collect();
    want.sort();
    if callers == want.as_slice() {
        Ok(())
    } else {
        Err(format!(
            "the terminal emit call sites changed.\n  found:   {callers:?}\n  allowed: {want:?}\n\
             A new caller must go through the executor's `TERM` effect; moving a site behind \
             `TERM` lowers its entry in TERMINAL_EMIT_ALLOWLIST (and the baseline sum)."
        ))
    }
}

/// The terminal emits that remain are pinned per enclosing function: the count
/// alone would let a `TERM` call be swapped for an inline emit elsewhere.
#[test]
fn terminal_emit_callers_are_exactly_the_allowlisted_functions() {
    let sources = raw_production_sources();
    let callers = terminal_emit_callers(&sources);
    if let Err(message) = check_terminal_emit_callers(&callers, &TERMINAL_EMIT_ALLOWLIST) {
        panic!("{message}");
    }
    assert_eq!(
        callers.iter().map(|(_, _, calls)| calls).sum::<usize>(),
        TERMINAL_EMIT_SITES_BASELINE,
        "the allowlist and the baseline constant agree"
    );
}

// ------------------------------------------------ SP5: status events (D-S2b-3, D-S2b-8)
//
// The ratchet does no value-flow analysis (D-S2b-8), so it is fail-closed by
// construction:
//
// (a) An emit call's event name is resolved only when the argument is a string
//     literal, a `concat!` of string literals, or a path to a `const` / `static`
//     whose initializer is one of those. Everything else is unresolved: a `let`
//     (mutable or not), a parameter, a captured value, a method result.
// (b) An unresolved emit is allowed only inside a function on
//     `DYNAMIC_EMIT_ALLOWLIST`; each entry pins the function's normalized token
//     stream by hash (whitespace, comments and doc comments stripped), so any edit to
//     a wrapper fails until it is re-reviewed and re-pinned.
// (c) Every call of an allowlisted wrapper (`emit_json`, `emit_event`, the latter
//     two also through `idle_emitter`'s returned `OwnedJsonEmit`) outside the
//     wrappers must pass a name that resolves under (a) and is declared by an entry
//     for that wrapper name.
//
// A call "touches" the status event when any argument resolves to it. The touch is
// an emit unless the callee is a listener (`listen`, `once`, ...).

/// The event name SP5 pins.
const STATUS_EVENT: &str = "recording-status-changed";

/// One call, method call or macro with the arguments written at it.
struct CallRecord {
    function: String,
    callee: String,
    /// `None`: macro tokens that did not parse as arguments.
    args: Option<Vec<syn::Expr>>,
    /// Written as a path call (`Emitter::emit(app, ..)`: the receiver is argument 0).
    path_call: bool,
    /// The token hash of the enclosing function (0 outside any function).
    fn_hash: u64,
}

/// Macros whose arguments are data, not calls with an event name.
const PURE_MACROS: [&str; 30] = [
    "concat",
    "stringify",
    "format",
    "vec",
    "json",
    "matches",
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "println",
    "eprintln",
    "print",
    "eprint",
    "write",
    "writeln",
    "panic",
    "unreachable",
    "todo",
    "unimplemented",
    "dbg",
    "cfg",
    "env",
    "info",
    "debug",
    "warn",
    "error",
    "trace",
];

fn parse_expr_list(tokens: proc_macro2::TokenStream) -> Option<Vec<syn::Expr>> {
    use syn::parse::Parser;
    syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated
        .parse2(tokens)
        .ok()
        .map(|list| list.into_iter().collect())
}

/// The listener APIs: their event argument is a subscription, not an emit.
const LISTENERS: [&str; 4] = ["listen", "once", "listen_any", "once_any"];

/// `(name, arguments of the method form, index of the event argument)`. The Tauri
/// `Emitter` API, and this code's own host wrappers `emit_json` / `emit_event`.
const EMIT_APIS: [(&str, usize, usize); 9] = [
    ("emit", 2, 0),
    ("emit_all", 2, 0),
    ("emit_str", 2, 0),
    ("emit_json", 2, 0),
    ("emit_event", 2, 0),
    ("emit_to", 3, 1),
    ("emit_str_to", 3, 1),
    ("emit_filter", 3, 0),
    ("emit_str_filter", 3, 0),
];

/// The host wrappers whose callers are checked against the declared event names.
const WRAPPER_NAMES: [&str; 2] = ["emit_json", "emit_event"];

fn is_emit_api_name(name: &str) -> bool {
    EMIT_APIS.iter().any(|(api, _, _)| *api == name)
}

/// The index of the event argument when `name(args)` has an emit API's shape (a path
/// call carries the receiver as argument 0, so one more argument and index).
fn emit_event_index(name: &str, args: usize, path_call: bool) -> Option<usize> {
    let shift = usize::from(path_call);
    EMIT_APIS
        .iter()
        .find(|(api, arity, _)| *api == name && arity + shift == args)
        .map(|(_, _, index)| index + shift)
}

/// A token stream as text with no spacing, comment or doc-comment information.
fn normalized_tokens(tokens: proc_macro2::TokenStream) -> String {
    use proc_macro2::{Delimiter, TokenTree};
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();
    let mut out = Vec::new();
    let mut at = 0;
    while at < tokens.len() {
        // `#[doc = "..."]` is a doc comment: not part of the function's code.
        if let (TokenTree::Punct(hash), Some(TokenTree::Group(group))) =
            (&tokens[at], tokens.get(at + 1))
        {
            let doc = matches!(
                group.stream().into_iter().next(),
                Some(TokenTree::Ident(ident)) if ident == "doc"
            );
            if hash.as_char() == '#' && group.delimiter() == Delimiter::Bracket && doc {
                at += 2;
                continue;
            }
        }
        match &tokens[at] {
            TokenTree::Group(group) => {
                let (open, close) = match group.delimiter() {
                    Delimiter::Parenthesis => ("(", ")"),
                    Delimiter::Brace => ("{", "}"),
                    Delimiter::Bracket => ("[", "]"),
                    Delimiter::None => ("", ""),
                };
                // A trailing comma is formatting (rustfmt adds and removes it).
                let inner = normalized_tokens(group.stream());
                let inner = inner.strip_suffix(" ,").unwrap_or(&inner);
                out.push(format!("{open} {inner} {close}"));
            }
            TokenTree::Punct(punct) => out.push(punct.as_char().to_string()),
            other => out.push(other.to_string()),
        }
        at += 1;
    }
    out.join(" ")
}

/// FNV-1a over the normalized token stream of `item`: stable across runs and Rust
/// versions (std's hashers are not promised to be).
fn token_hash<T: quote::ToTokens>(item: &T) -> u64 {
    let text = normalized_tokens(item.to_token_stream());
    text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Every `const` / `static` (also associated consts) by name, across files.
type Consts = std::collections::HashMap<String, Vec<syn::Expr>>;

struct ConstCollector<'c> {
    consts: &'c mut Consts,
}

impl<'ast> syn::visit::Visit<'ast> for ConstCollector<'_> {
    fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
        self.consts
            .entry(plain(&item.ident))
            .or_default()
            .push((*item.expr).clone());
        syn::visit::visit_item_const(self, item);
    }

    fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
        self.consts
            .entry(plain(&item.ident))
            .or_default()
            .push((*item.expr).clone());
        syn::visit::visit_item_static(self, item);
    }

    fn visit_impl_item_const(&mut self, item: &'ast syn::ImplItemConst) {
        self.consts
            .entry(plain(&item.ident))
            .or_default()
            .push(item.expr.clone());
        syn::visit::visit_impl_item_const(self, item);
    }
}

type Names = std::collections::BTreeSet<String>;

fn short(text: &str) -> String {
    let text: String = text.chars().take(60).collect();
    format!("`{text}`")
}

/// A string literal, or a `concat!` of string literals (nothing else).
fn literal_value(expr: &syn::Expr) -> Result<String, String> {
    match expr {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(text),
            ..
        }) => Ok(text.value()),
        syn::Expr::Macro(call)
            if call
                .mac
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "concat") =>
        {
            let parts =
                parse_expr_list(call.mac.tokens.clone()).ok_or("an unparsable `concat!`")?;
            let mut joined = String::new();
            for part in &parts {
                match part {
                    syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(text),
                        ..
                    }) => joined.push_str(&text.value()),
                    other => return Err(format!("`concat!` of {}", short(&written(other)))),
                }
            }
            Ok(joined)
        }
        other => Err(short(&written(other))),
    }
}

/// D-S2b-8 (a): the event names an argument can be: a literal, a `concat!` of
/// literals, or a path to a `const` / `static` whose initializer is one of those.
/// Anything else is unresolved, with the reason.
fn resolve_event(expr: &syn::Expr, consts: &Consts) -> Result<Names, String> {
    match expr {
        syn::Expr::Path(path) => {
            let Some(last) = path.path.segments.last() else {
                return Err("an empty path".to_string());
            };
            let name = plain(&last.ident);
            let Some(initializers) = consts.get(&name) else {
                return Err(format!("`{}` is not a const or static", written(path)));
            };
            initializers
                .iter()
                .map(|init| literal_value(init).map_err(|why| format!("const `{name}`: {why}")))
                .collect()
        }
        other => literal_value(other).map(|name| Names::from([name])),
    }
}

/// A call of a host wrapper (`emit_json`, `emit_event`) and the name it passes.
struct WrapperCall {
    file: String,
    function: String,
    callee: String,
    name: Result<Names, String>,
}

/// What the scan found.
#[derive(Default)]
struct EventSites {
    /// `(file, qualified fn, "emit" | "listen")` → calls with an argument that
    /// resolves to the status event.
    status: std::collections::BTreeMap<(String, String, &'static str), usize>,
    /// `(file, qualified fn)` → unresolved emits: how many, why, the function's hash.
    dynamic: std::collections::BTreeMap<(String, String), (usize, Names, u64)>,
    wrapper_calls: Vec<WrapperCall>,
}

/// Classify one call: a status touch (emit or listener), an unresolved emit, and a
/// wrapper call.
fn classify_call(file: &str, record: &CallRecord, consts: &Consts, found: &mut EventSites) {
    let function = record.function.clone();
    let name = record.callee.as_str();
    let mut dynamic = |why: String| {
        let entry = found
            .dynamic
            .entry((file.to_string(), function.clone()))
            .or_default();
        entry.0 += 1;
        entry.1.insert(why);
        entry.2 = record.fn_hash;
    };
    let Some(args) = &record.args else {
        dynamic(format!("the tokens of an `{name}(..)` in a macro"));
        return;
    };
    let touches = args
        .iter()
        .any(|arg| resolve_event(arg, consts).is_ok_and(|names| names.contains(STATUS_EVENT)));
    let emit_index = emit_event_index(name, args.len(), record.path_call);
    if let Some(index) = emit_index {
        if let Err(why) = resolve_event(&args[index], consts) {
            dynamic(why);
        }
        if WRAPPER_NAMES.contains(&name) {
            found.wrapper_calls.push(WrapperCall {
                file: file.to_string(),
                function: record.function.clone(),
                callee: name.to_string(),
                name: resolve_event(&args[index], consts),
            });
        }
    }
    if touches {
        let kind = if LISTENERS.contains(&name) {
            "listen"
        } else {
            "emit"
        };
        *found
            .status
            .entry((file.to_string(), record.function.clone(), kind))
            .or_default() += 1;
    }
}

/// The scan of `sources` (`(file relative to src, raw Rust source)`, production code
/// only: `#[cfg(test)]` items are skipped).
fn event_sites(sources: &[(String, String)]) -> EventSites {
    use syn::visit::Visit;
    let files: Vec<(&String, syn::File)> = sources
        .iter()
        .map(|(file, source)| {
            (
                file,
                syn::parse_file(source).unwrap_or_else(|error| panic!("parse {file}: {error}")),
            )
        })
        .collect();
    let mut consts = Consts::new();
    for (_, file) in &files {
        ConstCollector {
            consts: &mut consts,
        }
        .visit_file(file);
    }
    let never = |_: &[String]| false;
    let mut found = EventSites::default();
    for (path, file) in &files {
        let mut finder = SiteFinder {
            scope: Vec::new(),
            include_cfg_test: false,
            is_target: &never,
            hits: Vec::new(),
            record_calls: true,
            calls: Vec::new(),
            fn_hashes: Vec::new(),
        };
        finder.visit_file(file);
        for record in &finder.calls {
            classify_call(path, record, &consts, &mut found);
        }
    }
    found
}

/// SP5 (appendix C7.2, C13.3 #117): every production call that can carry the
/// `recording-status-changed` event name, pinned per enclosing function:
/// `(file relative to src, qualified enclosing fn, "emit" | "listen", calls)`. The
/// walk is the D-S2-17 fail-closed one (`raw_production_sources`: every `src/**/*.rs`
/// file except the declared test-only ones), so a status emit planted in any other
/// file (the #117 shape: `commands/models.rs`) fails. The set is:
///
/// * the executor's own `EM` effect ports (`*_ports.rs`), the sanctioned emitters;
/// * four idle emits the shell still passes to the legacy parse and pipeline-idle
///   finishers (`recording.rs`), the `204a` / `204b` / `302` tails that run under the
///   executor's decisions: pinned so they can only go down.
///
/// The two functions that subscribe to the event (`initialize` of
/// `smart_auto_probe.rs` and `meeting_suggestions.rs`) loop over an array of event
/// names, so their argument is unresolved: they are not listed, and an emit added in
/// either is an unresolved emit outside the dynamic allowlist and fails.
const STATUS_EVENT_ALLOWLIST: [(&str, &str, &str, usize); 8] = [
    // The legacy idle emits (two parse-failure closures and the 204b tail in
    // `process_audio_inner`, the stop's 204b tail): decided by site 204/302.
    ("commands/recording.rs", "process_audio_inner", "emit", 3),
    (
        "commands/recording.rs",
        "stop_native_recording_admitted",
        "emit",
        1,
    ),
    // The executor's `EM` effect ports.
    (
        "commands/recording/cancel_ports.rs",
        "<CancelPorts<'_> as Ports>::emit",
        "emit",
        1,
    ),
    (
        "commands/recording/lifecycle_ports.rs",
        "<LifecyclePorts<'_,R> as Ports>::emit",
        "emit",
        1,
    ),
    (
        "commands/recording/process_audio_ports.rs",
        "<ProcessAudioPorts<'_> as Ports>::emit",
        "emit",
        1,
    ),
    (
        "commands/recording/ready_ports.rs",
        "<ReadyPorts<'_,R> as Ports>::emit",
        "emit",
        1,
    ),
    (
        "commands/recording/start_ports.rs",
        "<StartPorts<'_> as Ports>::emit",
        "emit",
        1,
    ),
    (
        "commands/recording/stop_ports.rs",
        "<StopPorts<'_> as Ports>::emit",
        "emit",
        1,
    ),
];

/// A reviewed function that emits an event whose name is not a literal.
struct DynamicEmitter {
    file: &'static str,
    function: &'static str,
    /// Unresolved emits inside it.
    emits: usize,
    /// The wrapper names (`emit_json`, `emit_event`) whose callers feed it.
    wrappers: &'static [&'static str],
    /// The event names it may forward: every caller of its wrappers passes one.
    names: &'static [&'static str],
    /// The function's normalized token-stream hash (`token_hash`).
    hash: u64,
}

/// The lifecycle and delivery events the dictation host forwards (every
/// `emit_event` caller; the status event is among them).
const HOST_EVENTS: &[&str] = &[
    "dictation-delivery-outcome",
    "dictation-generation-started",
    "recording-cancelled",
    "recording-status-changed",
    "transcription-complete",
];

/// The events the imported-audio host forwards (every `emit_json` caller): the
/// status event (the idle emit and `Effect::EmitStatus`, including its `processing`
/// payload) and the generation-started event.
const IMPORT_EVENTS: &[&str] = &["dictation-generation-started", "recording-status-changed"];

/// Reviewed wrappers: each forwards its `event` parameter to `AppHandle::emit`
/// unchanged. Editing one fails the hash until it is re-reviewed.
const DYNAMIC_EMIT_ALLOWLIST: [DynamicEmitter; 4] = [
    DynamicEmitter {
        file: "commands/recording/process_audio_ports.rs",
        function: "<TauriImportHost as ImportedAudioHost>::emit_json",
        emits: 1,
        wrappers: &["emit_json"],
        names: IMPORT_EVENTS,
        hash: 0xb281a3c614b27342,
    },
    DynamicEmitter {
        file: "commands/recording/process_audio_ports.rs",
        function: "<TauriImportHost as ImportedAudioHost>::idle_emitter",
        emits: 1,
        wrappers: &["emit_json"],
        names: IMPORT_EVENTS,
        hash: 0x04390ac0450c9b93,
    },
    DynamicEmitter {
        file: "commands/recording_host.rs",
        function: "<TauriHost as DictationHost>::emit_json",
        emits: 1,
        wrappers: &["emit_event"],
        names: HOST_EVENTS,
        hash: 0xb281a3c614b27342,
    },
    DynamicEmitter {
        file: "commands/recording_host.rs",
        function: "dynDictationHost+'_::emit_event",
        emits: 1,
        wrappers: &["emit_event"],
        names: HOST_EVENTS,
        hash: 0xf1c0421dc47f2833,
    },
];

fn check_event_sites(
    found: &EventSites,
    status_allowed: &[(&str, &str, &str, usize)],
    dynamic_allowed: &[DynamicEmitter],
) -> Result<(), String> {
    let mut problems = Vec::new();
    // Status sites, per function.
    let status: Vec<(String, String, String, usize)> = found
        .status
        .iter()
        .map(|((file, function, kind), n)| (file.clone(), function.clone(), kind.to_string(), *n))
        .collect();
    let mut want: Vec<(String, String, String, usize)> = status_allowed
        .iter()
        .map(|(file, function, kind, n)| {
            (file.to_string(), function.to_string(), kind.to_string(), *n)
        })
        .collect();
    want.sort();
    if status != want {
        problems.push(format!(
            "the `recording-status-changed` sites changed (SP5).\n  found:   {status:?}\n  allowed: {want:?}\n\
             A status emit outside the executor's `EM` effect is not allowed; moving a legacy \
             emit behind the executor lowers its entry in STATUS_EVENT_ALLOWLIST."
        ));
    }
    // Unresolved emits: only inside the reviewed functions, unedited.
    let dynamic: Vec<(String, String, usize)> = found
        .dynamic
        .iter()
        .map(|((file, function), (n, _, _))| (file.clone(), function.clone(), *n))
        .collect();
    let mut want_dynamic: Vec<(String, String, usize)> = dynamic_allowed
        .iter()
        .map(|e| (e.file.to_string(), e.function.to_string(), e.emits))
        .collect();
    want_dynamic.sort();
    if dynamic != want_dynamic {
        let reasons: Vec<String> = found
            .dynamic
            .iter()
            .map(|((file, function), (_, why, _))| format!("{file} {function}: {why:?}"))
            .collect();
        problems.push(format!(
            "emits with an unresolvable event name changed (D-S2b-8 b).\n  found:   {dynamic:?}\n  allowed: {want_dynamic:?}\n  why: {reasons:?}\n\
             An event name must be a literal, a `concat!` of literals, or a const of one. A \
             reviewed wrapper goes on DYNAMIC_EMIT_ALLOWLIST with its declared names and hash."
        ));
    }
    for entry in dynamic_allowed {
        let key = (entry.file.to_string(), entry.function.to_string());
        if let Some((_, _, hash)) = found.dynamic.get(&key) {
            if *hash != entry.hash {
                problems.push(format!(
                    "{} {} was edited: its token hash is {hash:#018x}, pinned {:#018x}. \
                     Re-review the wrapper (it must forward `event` unchanged) and re-pin.",
                    entry.file, entry.function, entry.hash
                ));
            }
        }
        if entry.names.is_empty() || entry.wrappers.is_empty() {
            problems.push(format!(
                "{} {}: an entry declares its wrappers and event names",
                entry.file, entry.function
            ));
        }
    }
    // D-S2b-8 (c): every caller of a wrapper passes a resolvable, declared name.
    let reviewed: std::collections::BTreeSet<(&str, &str)> = dynamic_allowed
        .iter()
        .map(|e| (e.file, e.function))
        .collect();
    for call in &found.wrapper_calls {
        if reviewed.contains(&(call.file.as_str(), call.function.as_str())) {
            continue; // inside a pinned wrapper
        }
        let declared: Names = dynamic_allowed
            .iter()
            .filter(|e| e.wrappers.contains(&call.callee.as_str()))
            .flat_map(|e| e.names.iter().map(|name| name.to_string()))
            .collect();
        match &call.name {
            Err(why) => problems.push(format!(
                "{} {}: `{}(..)` passes an unresolvable event name ({why})",
                call.file, call.function, call.callee
            )),
            Ok(names) => {
                let undeclared: Vec<&String> = names.difference(&declared).collect();
                if !undeclared.is_empty() {
                    problems.push(format!(
                        "{} {}: `{}(..)` passes {undeclared:?}, which no `{}` entry declares",
                        call.file, call.function, call.callee, call.callee
                    ));
                }
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

#[test]
fn status_events_are_emitted_only_by_the_allowlisted_functions() {
    let found = event_sites(&raw_production_sources());
    if let Err(message) =
        check_event_sites(&found, &STATUS_EVENT_ALLOWLIST, &DYNAMIC_EMIT_ALLOWLIST)
    {
        panic!("{message}");
    }
}

/// A source file of the tree with `old` replaced by `new` (exactly one occurrence).
fn sources_with(file: &str, old: &str, new: &str) -> Vec<(String, String)> {
    let mut sources = raw_production_sources();
    let (_, text) = sources
        .iter_mut()
        .find(|(name, _)| name == file)
        .unwrap_or_else(|| panic!("{file} is scanned"));
    assert_eq!(
        text.matches(old).count(),
        1,
        "`{old}` occurs once in {file}"
    );
    *text = text.replace(old, new);
    sources
}

fn the_status_ratchet_rejects(sources: &[(String, String)]) -> String {
    let found = event_sites(sources);
    check_event_sites(&found, &STATUS_EVENT_ALLOWLIST, &DYNAMIC_EMIT_ALLOWLIST)
        .expect_err("the planted change must fail the ratchet")
}

#[test]
fn a_planted_status_emit_in_models_rs_fails_the_status_ratchet() {
    // The #117 shape: a status emit on the VAD download path.
    let mut sources = raw_production_sources();
    sources.push((
        "commands/models.rs".to_string(),
        "async fn ensure_vad_model(app: &AppHandle) { let _ = app.emit(\"recording-status-changed\", \"processing\"); }"
            .to_string(),
    ));
    let error = the_status_ratchet_rejects(&sources);
    assert!(error.contains("commands/models.rs") && error.contains("ensure_vad_model"));
}

/// Sol's round-1 probe 1: an extra emit that reuses the listener's event variable
/// inside `meeting_suggestions::initialize`. The variable is a loop variable: its
/// emit is unresolved, outside the dynamic allowlist.
#[test]
fn an_emit_reusing_the_listener_variable_fails_the_status_ratchet() {
    let sources = sources_with(
        "meeting_suggestions.rs",
        "        app.listen_any(event, move |_| busy_changed(&app_for_event));\n",
        "        app.listen_any(event, move |_| busy_changed(&app_for_event));\n        let _ = app.emit(event, \"processing\");\n",
    );
    let error = the_status_ratchet_rejects(&sources);
    assert!(
        error.contains("meeting_suggestions.rs") && error.contains("initialize"),
        "{error}"
    );
}

/// Sol's round-1 probe 2: the event name built with `concat!` (it resolves, so it is
/// a status site in a function that is not allowed one).
#[test]
fn a_concat_status_emit_fails_the_status_ratchet() {
    let sources = sources_with(
        "commands/models.rs",
        "    let _ = tokio::fs::remove_file(model_download::resume_path(&temp_path)).await;\n    tracing::info!(target: \"system\", \"VAD model downloaded: {} ({} bytes)\", vad::VAD_MODEL_FILENAME, received);\n",
        "    let _ = tokio::fs::remove_file(model_download::resume_path(&temp_path)).await;\n    let _ = app_handle.emit(concat!(\"recording-status-\", \"changed\"), \"processing\");\n    tracing::info!(target: \"system\", \"VAD model downloaded: {} ({} bytes)\", vad::VAD_MODEL_FILENAME, received);\n",
    );
    let error = the_status_ratchet_rejects(&sources);
    assert!(
        error.contains("commands/models.rs") && error.contains("ensure_vad_model_with_progress"),
        "{error}"
    );
}

/// Sol's round-2 probe 1: a `let mut event` initialized to another name and later set
/// to the status event. No value-flow analysis: the emit's argument is a `let`, so it
/// is unresolved.
#[test]
fn a_reassigned_event_name_fails_the_status_ratchet() {
    let sources = sources_with(
        "meeting_suggestions.rs",
        "    let coordinator = app.state::<State>().meeting_suggestions.clone();\n    for event in [",
        "    let coordinator = app.state::<State>().meeting_suggestions.clone();\n    let mut reassigned = \"model-runtime-status-changed\";\n    reassigned = \"recording-status-changed\";\n    let _ = app.emit(reassigned, \"processing\");\n    for event in [",
    );
    let error = the_status_ratchet_rejects(&sources);
    assert!(
        error.contains("meeting_suggestions.rs") && error.contains("initialize"),
        "{error}"
    );
}

/// Sol's round-2 probe 2: `TauriHost::emit_json` remaps `recording-cancelled` to the
/// status event. The wrapper's pinned token hash changes.
#[test]
fn a_wrapper_that_remaps_an_event_fails_the_status_ratchet() {
    let sources = sources_with(
        "commands/recording_host.rs",
        "    fn emit_json(&self, event: &str, payload: serde_json::Value) -> Result<(), HostEmitError> {\n        self.app\n            .emit(event, payload)",
        "    fn emit_json(&self, event: &str, payload: serde_json::Value) -> Result<(), HostEmitError> {\n        let event = if event == \"recording-cancelled\" { \"recording-status-changed\" } else { event };\n        self.app\n            .emit(event, payload)",
    );
    let error = the_status_ratchet_rejects(&sources);
    assert!(
        error.contains("<TauriHost as DictationHost>::emit_json") && error.contains("was edited"),
        "{error}"
    );
}

/// Editing a pinned wrapper fails; reformatting it or its comments does not.
#[test]
fn editing_a_pinned_wrapper_fails_but_reformatting_does_not() {
    const OLD: &str = "    fn emit_json(&self, event: &str, payload: serde_json::Value) -> Result<(), HostEmitError> {\n        self.app\n            .emit(event, payload)";
    let reformatted = sources_with(
        "commands/recording_host.rs",
        OLD,
        "    /// A doc comment.\n    fn emit_json(\n        &self,\n        event: &str,\n        payload: serde_json::Value,\n    ) -> Result<(), HostEmitError> {\n        // a comment\n        self.app.emit(event, payload)",
    );
    check_event_sites(
        &event_sites(&reformatted),
        &STATUS_EVENT_ALLOWLIST,
        &DYNAMIC_EMIT_ALLOWLIST,
    )
    .expect("a comment and a reformat keep the pinned hash");
    let edited = sources_with(
        "commands/recording_host.rs",
        OLD,
        "    fn emit_json(&self, event: &str, payload: serde_json::Value) -> Result<(), HostEmitError> {\n        let _ = &payload;\n        self.app\n            .emit(event, payload)",
    );
    assert!(the_status_ratchet_rejects(&edited).contains("was edited"));
}

/// (c): a caller of a wrapper passes a name that resolves and is declared, also
/// through the returned `OwnedJsonEmit` (its `.emit_json(..)`).
#[test]
fn a_wrapper_caller_must_pass_a_declared_resolvable_name() {
    let call = |body: &str| {
        let sources = sources_with(
            "commands/recording.rs",
            "fn process_audio_inner",
            &format!("fn planted(host: &H, name: &str) {{ {body} }}\nfn process_audio_inner"),
        );
        let found = event_sites(&sources);
        check_event_sites(&found, &STATUS_EVENT_ALLOWLIST, &DYNAMIC_EMIT_ALLOWLIST)
    };
    // A declared literal passes the wrapper check (only the status ones are status
    // sites, so use a non-status declared name).
    assert!(call("let _ = host.emit_event(\"transcription-complete\", 1);").is_ok());
    // An undeclared name, an unresolvable name, and the same through the closure.
    for body in [
        "let _ = host.emit_event(\"not-declared\", 1);",
        "let _ = host.emit_event(name, 1);",
        "let _ = host.emit_json(\"recording-cancelled\", 1);",
        "let emit = host.idle_emitter(); let _ = emit.emit_json(name, 1);",
        "let emit = host.idle_emitter(); let _ = emit.emit_json(\"recording-cancelled\", 1);",
    ] {
        assert!(call(body).is_err(), "{body}");
    }
}

/// Counts for one synthetic file: `(status emits, status listeners, unresolved emits)`.
fn event_counts(source: &str) -> (usize, usize, usize) {
    let found = event_sites(&[("x.rs".to_string(), source.to_string())]);
    let status = |wanted: &str| {
        found
            .status
            .iter()
            .filter(|((_, _, kind), _)| *kind == wanted)
            .map(|(_, n)| n)
            .sum::<usize>()
    };
    (
        status("emit"),
        status("listen"),
        found.dynamic.values().map(|(n, _, _)| n).sum(),
    )
}

#[test]
fn the_event_scanner_resolves_only_literals_concat_and_consts() {
    // (source, status emits, status listeners, unresolved emits)
    let cases: [(&str, (usize, usize, usize)); 24] = [
        ("fn f(a: &A) { a.emit(\"recording-status-changed\", 1); }", (1, 0, 0)),
        ("fn f(a: &A) { a.emit(\"other\", 1); }", (0, 0, 0)),
        ("fn f(a: &A) { a.emit(concat!(\"recording-status-\", \"changed\"), 1); }", (1, 0, 0)),
        (
            "fn f(a: &A) { a.emit(concat!(\"recording\", \"-status\", \"-changed\"), 1); }",
            (1, 0, 0),
        ),
        ("const E: &str = \"recording-status-changed\"; fn f(a: &A) { a.emit(E, 1); }", (1, 0, 0)),
        ("static E: &str = \"recording-status-changed\"; fn f(a: &A) { a.emit(E, 1); }", (1, 0, 0)),
        (
            "const E: &str = concat!(\"recording-status-\", \"changed\"); fn f(a: &A) { a.emit(crate::E, 1); }",
            (1, 0, 0),
        ),
        ("fn f(a: &A) { a.emit_to(\"main\", \"recording-status-changed\", 1); }", (1, 0, 0)),
        ("fn f(a: &A) { tauri::Emitter::emit(a, concat!(\"recording-status-\", \"changed\"), 1); }", (1, 0, 0)),
        ("fn f(a: &A) { a.listen_any(\"recording-status-changed\", |_| ()); }", (0, 1, 0)),
        ("fn f(a: &A) { w!(a, concat!(\"recording-status-\", \"changed\")); }", (1, 0, 0)),
        // A name that is not the status event, a test-only emit, a pure macro.
        ("fn f(a: &A) { a.emit(\"recording-status-changedx\", 1); }", (0, 0, 0)),
        ("#[cfg(test)] fn f(a: &A) { a.emit(\"recording-status-changed\", 1); }", (0, 0, 0)),
        ("fn f() { let n = format!(\"{}\", \"recording-status-changed\"); }", (0, 0, 0)),
        // Unresolved, however innocent: a parameter, a call, a field, a `let`, a
        // reassigned `let mut`, a loop variable, a method result, a const of a const,
        // a const that is not a literal.
        ("fn f(a: &A, name: &str) { a.emit(name, 1); }", (0, 0, 1)),
        ("fn f(a: &A) { a.emit(event_name(), 1); }", (0, 0, 1)),
        ("fn f(a: &A, s: &S) { a.emit(s.event, 1); }", (0, 0, 1)),
        ("fn f(a: &A) { let e = \"recording-status-changed\"; a.emit(e, 1); }", (0, 0, 1)),
        (
            "fn f(a: &A) { let mut e = \"x\"; e = \"recording-status-changed\"; a.emit(e, 1); }",
            (0, 0, 1),
        ),
        (
            "fn f(a: &A) { for e in [\"recording-status-changed\"] { a.emit(e, 1); } }",
            (0, 0, 1),
        ),
        ("fn f(a: &A) { a.emit(\"recording-status-changed\".to_string(), 1); }", (0, 0, 1)),
        (
            "const A: &str = \"recording-status-changed\"; const B: &str = A; fn f(a: &A) { a.emit(B, 1); }",
            (0, 0, 1),
        ),
        (
            "const A: &str = if c { \"recording-status-changed\" } else { \"x\" }; fn f(a: &A) { a.emit(A, 1); }",
            (0, 0, 1),
        ),
        ("fn f(a: &A) { a.emit(String::from(\"recording-status-changed\"), 1); }", (1, 0, 1)),
    ];
    for (source, expected) in cases {
        assert_eq!(event_counts(source), expected, "{source}");
    }
    // A call that is not an emit API's shape is not an emit (`ports.emit(effect)`).
    assert_eq!(event_counts("fn f(p: &P, e: &E) { p.emit(e); }"), (0, 0, 0));
    // Tokens of a macro definition: `$event` cannot be resolved.
    assert_eq!(
        event_counts("macro_rules! m { ($a:expr, $e:expr) => { $a.emit($e, 1) }; }"),
        (0, 0, 1)
    );
}

#[test]
fn a_reviewed_dynamic_emitter_needs_its_pin_and_declarations() {
    let source = "fn wrap(a: &A, name: &str) { a.emit(name, 1); }";
    let found = event_sites(&[("x.rs".to_string(), source.to_string())]);
    let hash = found.dynamic[&("x.rs".to_string(), "wrap".to_string())].2;
    let entry = |hash, names: &'static [&'static str]| {
        [DynamicEmitter {
            file: "x.rs",
            function: "wrap",
            emits: 1,
            wrappers: &["emit_json"],
            names,
            hash,
        }]
    };
    assert!(check_event_sites(&found, &[], &entry(hash, &["a"])).is_ok());
    assert!(check_event_sites(&found, &[], &entry(hash ^ 1, &["a"])).is_err());
    assert!(check_event_sites(&found, &[], &entry(hash, &[])).is_err());
    assert!(check_event_sites(&found, &[], &[]).is_err());
}

/// The one function allowed to call `PersistedCore::new` (decision D-S1-5 a).
const SEEDING_FILE: &str = "dictation_executor/adapter.rs";
const SEEDING_FN: &str = "FieldAdapter::seed_core";

/// Every `PersistedCore::new` in `sources` (`(file, raw Rust source)`, test code
/// included) as `(file, qualified enclosing fn)`.
fn persisted_core_new_callers(sources: &[(String, String)]) -> Vec<(String, String)> {
    let mut callers = Vec::new();
    for (file, source) in sources {
        for function in call_scopes(source, true, &is_persisted_core_new) {
            callers.push((file.clone(), function));
        }
    }
    callers
}

/// `Ok` iff there is exactly one caller and it is `SEEDING_FN` in `SEEDING_FILE`.
fn check_single_seeder(callers: &[(String, String)]) -> Result<(), String> {
    match callers {
        [(file, function)] if file == SEEDING_FILE && function == SEEDING_FN => Ok(()),
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
            let code = std::fs::read_to_string(path).expect("read");
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
fn test_only_files_are_skipped_and_production_files_are_scanned() {
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

/// D-S2-17: every skipped file follows the test naming convention, and the
/// exceptions do not (so the exception list cannot hide a conforming file or a
/// production file under a wrong name).
#[test]
fn every_test_only_file_follows_the_naming_convention() {
    for file in TEST_ONLY_FILES {
        assert!(
            follows_test_naming(file),
            "{file} is in TEST_ONLY_FILES but does not follow the test naming convention \
             (`*_tests.rs`, `tests.rs`, or under `tests/` or `test_support/`); list it in \
             TEST_ONLY_NAMING_EXCEPTIONS with a comment instead"
        );
    }
    for file in TEST_ONLY_NAMING_EXCEPTIONS {
        assert!(
            !follows_test_naming(file),
            "{file} follows the convention: move it to TEST_ONLY_FILES"
        );
        assert!(!TEST_ONLY_FILES.contains(file), "{file} is listed twice");
    }
}

/// D-S2-17: every listed file exists, so a rename or a deletion cannot leave a stale
/// entry that would later skip an unrelated file of the same name.
#[test]
fn every_test_only_file_exists() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for file in TEST_ONLY_FILES.iter().chain(TEST_ONLY_NAMING_EXCEPTIONS) {
        assert!(
            src.join(file).is_file(),
            "{file} is listed as test-only but does not exist under src/"
        );
    }
}

/// D-S2-17 (fail-closed): a file nobody listed is production. A production file that
/// is missing from the list and holds a terminal call fails the ratchet, and so does a
/// test-looking file that was not listed (a loud false positive, never a silent miss).
#[test]
fn an_unlisted_file_with_a_terminal_call_fails_the_ratchet() {
    let allowed = [("commands/a.rs", "run", 1)];
    let dir = tempfile::tempdir().expect("temp dir");
    let write = |name: &str, code: &str| {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, code).expect("write");
    };
    write("commands/a.rs", "fn run() { emit_dictation_terminal(a); }");
    write(
        "commands/new_production.rs",
        "fn f() { emit_dictation_terminal(b); }",
    );
    write(
        "commands/new_feature_tests.rs",
        "fn g() { emit_dictation_terminal(c); }",
    );
    let check = |skip: &dyn Fn(&str) -> bool| {
        check_terminal_emit_callers(
            &terminal_emit_callers(&scan_dir(dir.path(), skip)),
            &allowed,
        )
    };
    // Nothing listed: the unlisted production file and the unlisted test-named file
    // are both scanned.
    let found = check(&|_| false).expect_err("two unlisted files with a terminal call");
    assert!(found.contains("commands/new_production.rs"), "{found}");
    assert!(found.contains("commands/new_feature_tests.rs"), "{found}");
    // Listing the test file (and only it) leaves the production file failing.
    let listed = |name: &str| name == "commands/new_feature_tests.rs";
    let found = check(&listed).expect_err("the production file is still scanned");
    assert!(found.contains("commands/new_production.rs"), "{found}");
    assert!(!found.contains("new_feature_tests"), "{found}");
    // Listing both is the only way to pass.
    assert_eq!(
        check(&|name: &str| name != "commands/a.rs" && name.starts_with("commands/new_")),
        Ok(())
    );
}

/// The D-S1-5 (a) gate fails on a second caller anywhere: in another file, in
/// another adapter method, or twice inside the seeder.
#[test]
fn scanner_flags_an_extra_persisted_core_new_inside_the_adapter() {
    let adapter = |code: &str| vec![(SEEDING_FILE.to_string(), code.to_string())];
    let check =
        |sources: &[(String, String)]| check_single_seeder(&persisted_core_new_callers(sources));
    let seeder = "impl FieldAdapter { fn seed_core(a: &A) { d.core = PersistedCore::new(s, i); } }";
    assert_eq!(check(&adapter(seeder)), Ok(()));
    // The shape that used to pass: a second constructor call in another method.
    let in_other_method = format!(
        "{seeder} impl FieldAdapter {{ fn alloc_id(a: &A) {{ d.core = PersistedCore::new(s, RecordingId(id)); }} }}"
    );
    assert!(check(&adapter(&in_other_method)).is_err());
    // The only call in the wrong method.
    assert!(check(&adapter(
        "impl FieldAdapter { fn alloc_id() { PersistedCore::new(s, i); } }"
    ))
    .is_err());
    // A same-named `seed_core` on another type or at file scope is not the seeder.
    assert!(check(&adapter(
        "impl Other { fn seed_core() { PersistedCore::new(s, i); } }"
    ))
    .is_err());
    assert!(check(&adapter("fn seed_core() { PersistedCore::new(s, i); }")).is_err());
    // Two calls inside the seeder itself.
    let twice = "impl FieldAdapter { fn seed_core() { PersistedCore::new(a, b); PersistedCore::new(c, d); } }";
    assert!(check(&adapter(twice)).is_err());
    // Another file.
    let mut elsewhere = adapter(seeder);
    elsewhere.push(("commands/recording.rs".into(), seeder.to_string()));
    assert!(check(&elsewhere).is_err());
    // No caller at all.
    assert!(check(&adapter("impl FieldAdapter { fn seed_core() {} }")).is_err());
    // A comment or a string mentioning it is not a caller.
    assert_eq!(
        check(&adapter(&format!(
            "{seeder} // PersistedCore::new(x)\nfn f() {{ let t = \"PersistedCore::new(\"; }}"
        ))),
        Ok(())
    );
}

/// The terminal-emit gate fails when a `TERM` call is swapped for an inline emit in
/// another function (the count stays the same), when a site is added, and when one
/// is removed without lowering its entry.
#[test]
fn scanner_flags_a_swapped_terminal_emit() {
    let allowed = [
        ("commands/a.rs", "terminal", 1),
        ("commands/b.rs", "run", 1),
    ];
    let sources = |a: &str, b: &str| {
        vec![
            ("commands/a.rs".to_string(), a.to_string()),
            ("commands/b.rs".to_string(), b.to_string()),
        ]
    };
    let check = |sources: &[(String, String)]| {
        check_terminal_emit_callers(&terminal_emit_callers(sources), &allowed)
    };
    let ok = sources(
        "fn terminal() { emit_dictation_terminal(a); }",
        "fn run() { emit_dictation_terminal_x(b); }",
    );
    assert_eq!(check(&ok), Ok(()));
    // Same total: the port's call became an inline emit in another function.
    let swapped = sources(
        "fn terminal() {} fn other() { emit_dictation_terminal(a); }",
        "fn run() { emit_dictation_terminal_x(b); }",
    );
    assert!(check(&swapped).is_err());
    // One more call in an allowed function, one fewer, and a definition (ignored).
    assert!(check(&sources(
        "fn terminal() { emit_dictation_terminal(a); emit_dictation_terminal(b); }",
        "fn run() { emit_dictation_terminal_x(b); }"
    ))
    .is_err());
    assert!(check(&sources(
        "fn terminal() {}",
        "fn run() { emit_dictation_terminal_x(b); }"
    ))
    .is_err());
    let with_definition = sources(
        "fn terminal() { emit_dictation_terminal(a); } fn emit_dictation_terminal(x: u8) {}",
        "fn run() { emit_dictation_terminal_x(b); }",
    );
    assert_eq!(check(&with_definition), Ok(()));
    // Comments and strings are not calls.
    let quiet = sources(
        "fn terminal() { emit_dictation_terminal(a); // emit_dictation_terminal(z)\n let s = \"emit_dictation_terminal(\"; }",
        "fn run() { emit_dictation_terminal_x(b); }",
    );
    assert_eq!(check(&quiet), Ok(()));
}

/// The enclosing function is resolved from real syntax, not text position: a call is
/// credited to the function whose body contains it, and a function's identity is
/// qualified by its module path and `impl` type.
#[test]
fn scanner_attributes_a_call_to_its_qualified_function() {
    let callers =
        |code: &str| terminal_emit_callers(&[("commands/a.rs".to_string(), code.to_string())]);
    let only = |function: &str, calls: usize| {
        vec![("commands/a.rs".to_string(), function.to_string(), calls)]
    };
    assert_eq!(
        callers("fn terminal() { emit_dictation_terminal(a); }"),
        only("terminal", 1)
    );
    // A closure in a file-scope item right after the allowed function is that item's.
    assert_eq!(
        callers(
            "fn terminal() { work(); }\n\
             static FORWARD: fn(u8) = |a| { emit_dictation_terminal(a); };"
        ),
        only("<static FORWARD>", 1)
    );
    assert_eq!(
        callers(
            "fn terminal() { work(); }\n\
             const FORWARD: &dyn Fn(u8) = &|a| { emit_dictation_terminal(a); };"
        ),
        only("<const FORWARD>", 1)
    );
    // Nested items: the innermost function owns its call; the outer keeps its own.
    assert_eq!(
        callers(
            "fn outer() { fn inner() { emit_dictation_terminal(a); } emit_dictation_terminal(b); }"
        ),
        vec![
            ("commands/a.rs".to_string(), "outer".to_string(), 1),
            ("commands/a.rs".to_string(), "outer::inner".to_string(), 1),
        ]
    );
    // A closure inside a method stays in that method; signatures with generics, arrays
    // and `where` clauses, and bodiless trait declarations, do not confuse it.
    assert_eq!(
        callers(
            "trait T { fn decl(&self, a: [u8; 3]); }\n\
             impl X { fn run<F>(f: F, a: [u8; 3]) -> u8 where F: Fn() { let c = || { emit_dictation_terminal(a); }; 0 } }"
        ),
        only("X::run", 1)
    );
    // An `impl` associated const is at no function.
    assert_eq!(
        callers("fn terminal() {}\nimpl X { const C: u8 = { emit_dictation_terminal(a); 1 }; }"),
        only("X::<const C>", 1)
    );
    // Test code is not production; a macro's tokens and method calls are scanned.
    assert_eq!(
        callers(
            "#[cfg(test)] mod tests { fn terminal() { emit_dictation_terminal(a); } }\n\
             fn run() { log!(\"x\", emit_dictation_terminal(b)); s.emit_dictation_terminal_x(c); }"
        ),
        only("run", 2)
    );
}

/// Sol minor (#1151): a same-named function in a sibling module is a different
/// function. The allowed `terminal` in module `ports` must not excuse a `terminal`
/// in `other`; a last-textual-name or last-`fn` scan credits both to `terminal`.
#[test]
fn scanner_tells_same_named_functions_in_sibling_modules_apart() {
    let allowed = [("commands/a.rs", "ports::terminal", 1)];
    let check = |code: &str| {
        check_terminal_emit_callers(
            &terminal_emit_callers(&[("commands/a.rs".to_string(), code.to_string())]),
            &allowed,
        )
    };
    let good = "mod ports { pub fn terminal() { emit_dictation_terminal(a); } }\n\
                mod other { pub fn terminal() {} }";
    assert_eq!(check(good), Ok(()));
    // The same call moved into the sibling module's same-named function.
    let moved = "mod ports { pub fn terminal() {} }\n\
                 mod other { pub fn terminal() { emit_dictation_terminal(a); } }";
    assert!(check(moved).is_err());
    // Same-named method on two impl types.
    let methods = "impl A { fn terminal(&self) { emit_dictation_terminal(a); } }\n\
                   impl B { fn terminal(&self) {} }";
    assert!(
        check(methods).is_err(),
        "`A::terminal` is not `ports::terminal`"
    );
    assert_eq!(
        terminal_emit_callers(&[("commands/a.rs".to_string(), methods.to_string())]),
        vec![("commands/a.rs".to_string(), "A::terminal".to_string(), 1)]
    );
}

/// Sol minor (#1151): a `const` expression in a return type holds braces before the
/// body. The call inside the real body is still credited to `terminal`, and a call
/// in the const expression of a signature belongs to its function too.
#[test]
fn scanner_handles_a_const_expression_in_a_return_type() {
    let callers =
        |code: &str| terminal_emit_callers(&[("commands/a.rs".to_string(), code.to_string())]);
    let only = |function: &str, calls: usize| {
        vec![("commands/a.rs".to_string(), function.to_string(), calls)]
    };
    assert_eq!(
        callers("fn terminal() -> [u8; { 2 + 2 }] { emit_dictation_terminal(a); [0; 4] }"),
        only("terminal", 1)
    );
    // A following function is not swallowed by the braces in the previous signature.
    assert_eq!(
        callers(
            "fn terminal() -> [u8; { 4 }] { [0; 4] }\n\
             fn other() { emit_dictation_terminal(a); }"
        ),
        only("other", 1)
    );
    // A const block expression in a signature is inside that function's identity.
    assert_eq!(
        callers("fn terminal() -> [u8; { emit_dictation_terminal(a); 4 }] { [0; 4] }"),
        only("terminal", 1)
    );
}

/// Sol minor (#1151): only a `cfg` predicate that requires `test` hides an item.
/// `any(test, target_os = "macos")` is also true in production, so a caller under it
/// counts; `all(test, ..)`, `test` alone and a double negation do not.
#[test]
fn scanner_skips_an_item_only_when_its_cfg_requires_test() {
    let callers =
        |code: &str| terminal_emit_callers(&[("commands/a.rs".to_string(), code.to_string())]);
    let only = |function: &str| vec![("commands/a.rs".to_string(), function.to_string(), 1)];
    for production in [
        r#"#[cfg(any(test, target_os = "macos"))] fn run() { emit_dictation_terminal(a); }"#,
        r#"#[cfg(any(test, feature = "x"))] fn run() { emit_dictation_terminal(a); }"#,
        "#[cfg(not(test))] fn run() { emit_dictation_terminal(a); }",
        r#"#[cfg(target_os = "macos")] fn run() { emit_dictation_terminal(a); }"#,
        "#[cfg_attr(test, inline)] fn run() { emit_dictation_terminal(a); }",
    ] {
        assert_eq!(callers(production), only("run"), "{production}");
    }
    for test_only in [
        "#[cfg(test)] fn run() { emit_dictation_terminal(a); }",
        r#"#[cfg(all(test, target_os = "macos"))] fn run() { emit_dictation_terminal(a); }"#,
        "#[cfg(any(test, all(test, unix)))] fn run() { emit_dictation_terminal(a); }",
        "#[cfg(not(not(test)))] fn run() { emit_dictation_terminal(a); }",
        "#[cfg(test)] mod m { fn run() { emit_dictation_terminal(a); } }",
        "#[cfg(test)] impl X { fn run() { emit_dictation_terminal(a); } }",
        "fn run() { #[cfg(test)] emit_dictation_terminal(a); }",
        "fn run() { #[cfg(test)] let f = emit_dictation_terminal; }",
    ] {
        assert_eq!(callers(test_only), vec![], "{test_only}");
    }
}

/// Sol minor (#1151): a `#[cfg(test)]` macro definition (or invocation) is not
/// production, while an ordinary one is scanned.
#[test]
fn scanner_skips_a_cfg_test_macro_definition() {
    let callers =
        |code: &str| terminal_emit_callers(&[("commands/a.rs".to_string(), code.to_string())]);
    assert_eq!(
        callers("#[cfg(test)] macro_rules! m { () => { emit_dictation_terminal(a) }; }"),
        vec![]
    );
    assert_eq!(
        callers("#[cfg(test)] m!(emit_dictation_terminal(a));"),
        vec![]
    );
    assert_eq!(
        callers("fn run() { #[cfg(test)] log!(emit_dictation_terminal(a)); }"),
        vec![]
    );
    assert_eq!(
        callers(
            r#"#[cfg(any(test, unix))] macro_rules! m { () => { emit_dictation_terminal(a) }; }"#
        ),
        vec![("commands/a.rs".to_string(), String::new(), 1)]
    );
    assert_eq!(
        callers("macro_rules! m { () => { emit_dictation_terminal(a) }; }"),
        vec![("commands/a.rs".to_string(), String::new(), 1)]
    );
}

/// Sol minor (#1151): two trait impls for one type, each with a `terminal`, are two
/// identities; the allowed one does not excuse the other, and an inherent method of
/// the same name is a third.
#[test]
fn scanner_tells_trait_impls_of_one_type_apart() {
    let allowed = [("commands/a.rs", "<StopPorts<'_> as Ports>::terminal", 1)];
    let check = |code: &str| {
        check_terminal_emit_callers(
            &terminal_emit_callers(&[("commands/a.rs".to_string(), code.to_string())]),
            &allowed,
        )
    };
    let good =
        "impl Ports for StopPorts<'_> { fn terminal(&self) { emit_dictation_terminal(a); } }\n\
                impl Other for StopPorts<'_> { fn terminal(&self) {} }";
    assert_eq!(check(good), Ok(()));
    let other_trait = "impl Ports for StopPorts<'_> { fn terminal(&self) {} }\n\
                       impl Other for StopPorts<'_> { fn terminal(&self) { emit_dictation_terminal(a); } }";
    assert!(check(other_trait).is_err());
    let inherent = "impl Ports for StopPorts<'_> { fn terminal(&self) {} }\n\
                    impl StopPorts<'_> { fn terminal(&self) { emit_dictation_terminal(a); } }";
    assert!(check(inherent).is_err());
    // The full implementing type, not its last path segment: another type with the
    // same last segment is another identity.
    let other_type = "impl Ports for other::StopPorts<'_> { fn terminal(&self) { emit_dictation_terminal(a); } }";
    assert!(check(other_type).is_err());
    assert_eq!(
        terminal_emit_callers(&[("commands/a.rs".to_string(), other_trait.to_string())]),
        vec![(
            "commands/a.rs".to_string(),
            "<StopPorts<'_> as Other>::terminal".to_string(),
            1
        )]
    );
}

/// Sol minor (#1151): a constructor taken as a value (`let make = PersistedCore::new;`)
/// is a use of the constructor, so it cannot bypass the single-seeder gate; the same
/// holds for the terminal emit. Calls through a macro's tokens and a fully qualified
/// path are scanned too.
#[test]
fn scanner_counts_constructors_used_as_values() {
    let seeder = |code: &str| {
        check_single_seeder(&persisted_core_new_callers(&[(
            SEEDING_FILE.to_string(),
            code.to_string(),
        )]))
    };
    let seed = "impl FieldAdapter { fn seed_core(a: &A) { PersistedCore::new(s, i); } }";
    assert_eq!(seeder(seed), Ok(()));
    assert!(seeder(&format!(
        "{seed} impl FieldAdapter {{ fn other() {{ let make = PersistedCore::new; make(s, i); }} }}"
    ))
    .is_err());
    assert!(seeder(&format!(
        "{seed} fn other() {{ let make: fn(_, _) -> _ = state::PersistedCore::new; }}"
    ))
    .is_err());
    assert!(seeder(&format!(
        "{seed} fn other() {{ let all = [1].map(PersistedCore::new); }}"
    ))
    .is_err());
    assert!(seeder(&format!(
        "{seed} fn other() {{ dbg!(PersistedCore::new); }}"
    ))
    .is_err());
    // The seeder holding the constructor as a value is its one use.
    assert_eq!(
        seeder("impl FieldAdapter { fn seed_core(a: &A) { let make = PersistedCore::new; make(s, i); } }"),
        Ok(())
    );
    let emit_as_value = terminal_emit_callers(&[(
        "commands/a.rs".to_string(),
        "fn run() { let f = emit_dictation_terminal; f(a); }".to_string(),
    )]);
    assert_eq!(
        emit_as_value,
        vec![("commands/a.rs".to_string(), "run".to_string(), 1)]
    );
}

/// Sol round-5 (#1151): the layouts the old module resolver got wrong are all plain
/// production under the fail-closed scan, because nothing resolves modules: a `#[path]`
/// inside an inline module, a child of a module loaded with `#[path]`, and a
/// `#[cfg_attr(unix, path = ..)]` declaration. Each is written to a temp directory and
/// scanned with an empty skip list; every file's terminal call is counted.
#[test]
fn files_reached_through_path_attributes_are_scanned_as_production() {
    let dir = tempfile::tempdir().expect("temp dir");
    let write = |name: &str, code: &str| {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, code).expect("write");
    };
    let call = |name: &str| format!("pub fn {name}() {{ emit_dictation_terminal(a); }}");
    write(
        "lib.rs",
        "#[cfg(test)] mod t { #[path = \"inline_path.rs\"] mod a; }\n\
         mod outer { #[path = \"inner_path.rs\"] mod b; }\n\
         #[path = \"loaded.rs\"] mod loaded;\n\
         #[cfg_attr(unix, path = \"attr_path.rs\")] mod c;\n\
         #[cfg(test)] #[path = \"shared.rs\"] mod as_test;\n\
         #[path = \"shared.rs\"] mod as_prod;",
    );
    write("outer/inner_path.rs", &call("inner"));
    write("inline_path.rs", &call("inline"));
    write("loaded.rs", "mod child;");
    write("loaded/child.rs", &call("child"));
    write("attr_path.rs", &call("attr"));
    write("shared.rs", &call("shared"));
    let sources = scan_dir(dir.path(), &|_| false);
    let found: Vec<(String, String, usize)> = terminal_emit_callers(&sources);
    let names: Vec<(&str, &str)> = found
        .iter()
        .map(|(file, function, _)| (file.as_str(), function.as_str()))
        .collect();
    for expected in [
        ("outer/inner_path.rs", "inner"),
        ("inline_path.rs", "inline"),
        ("loaded/child.rs", "child"),
        ("attr_path.rs", "attr"),
        ("shared.rs", "shared"),
    ] {
        assert!(names.contains(&expected), "{expected:?} not in {names:?}");
    }
    // Only a listed file is skipped, and only by name.
    let skip_shared = |name: &str| name == "shared.rs";
    let after = terminal_emit_callers(&scan_dir(dir.path(), &skip_shared));
    assert_eq!(after.len(), found.len() - 1);
}

/// Sol minor (#1151): `r#PersistedCore::new(..)` is the same function as
/// `PersistedCore::new(..)`; a raw identifier cannot dodge the gates (paths, method
/// calls, macro tokens, and function names).
#[test]
fn scanner_sees_through_raw_identifiers() {
    let seeder = |code: &str| {
        check_single_seeder(&persisted_core_new_callers(&[(
            SEEDING_FILE.to_string(),
            code.to_string(),
        )]))
    };
    let seed = "impl FieldAdapter { fn seed_core(a: &A) { PersistedCore::new(s, i); } }";
    assert_eq!(seeder(seed), Ok(()));
    for extra in [
        "fn other() { r#PersistedCore::new(s, i); }",
        "fn other() { PersistedCore::r#new(s, i); }",
        "fn other() { let make = r#PersistedCore::r#new; }",
        "fn other() { dbg!(r#PersistedCore::new(s, i)); }",
    ] {
        assert!(seeder(&format!("{seed} {extra}")).is_err(), "{extra}");
    }
    // The seeder written with raw identifiers is the same function.
    assert_eq!(
        seeder("impl r#FieldAdapter { fn r#seed_core(a: &A) { r#PersistedCore::new(s, i); } }"),
        Ok(())
    );
    let emit =
        |code: &str| terminal_emit_callers(&[("commands/a.rs".to_string(), code.to_string())]);
    assert_eq!(
        emit("fn r#terminal() { r#emit_dictation_terminal(a); s.r#emit_dictation_terminal_x(b); }"),
        vec![("commands/a.rs".to_string(), "terminal".to_string(), 2)]
    );
}

/// Sol minor (#1151): the cfg filter covers associated macros: an invocation in an
/// `impl`, a `trait` or an `extern` block under `#[cfg(test)]` is not production.
#[test]
fn scanner_skips_cfg_test_associated_macros() {
    let callers =
        |code: &str| terminal_emit_callers(&[("commands/a.rs".to_string(), code.to_string())]);
    for test_only in [
        "impl X { #[cfg(test)] m! { emit_dictation_terminal(a) } }",
        "trait T { #[cfg(test)] m! { emit_dictation_terminal(a) } }",
        r#"extern "C" { #[cfg(test)] m! { emit_dictation_terminal(a) } }"#,
        "impl X { #[cfg(all(test, unix))] m!(emit_dictation_terminal(a)); }",
    ] {
        assert_eq!(callers(test_only), vec![], "{test_only}");
    }
    assert_eq!(
        callers("impl X { m! { emit_dictation_terminal(a) } }"),
        vec![("commands/a.rs".to_string(), "X".to_string(), 1)]
    );
    assert_eq!(
        callers("trait T { #[cfg(any(test, unix))] m! { emit_dictation_terminal(a) } }"),
        vec![("commands/a.rs".to_string(), "T".to_string(), 1)]
    );
    assert_eq!(
        callers(r#"extern "C" { m! { emit_dictation_terminal(a) } }"#),
        vec![("commands/a.rs".to_string(), String::new(), 1)]
    );
}

/// D-S2-17: the attributes of an enclosing `extern`, `impl`, `trait` or inline `mod`
/// are checked before descending: a macro invocation inside a `#[cfg(test)]` block is
/// not production, whatever the macro itself carries, and an `any(test, ..)` block is.
#[test]
fn scanner_skips_macros_in_cfg_test_enclosing_blocks() {
    let callers =
        |code: &str| terminal_emit_callers(&[("commands/a.rs".to_string(), code.to_string())]);
    for test_only in [
        r#"#[cfg(test)] extern "C" { m!(emit_dictation_terminal(a)); }"#,
        r#"#[cfg(test)] extern "C" { m! { emit_dictation_terminal(a) } }"#,
        "#[cfg(test)] impl X { m! { emit_dictation_terminal(a) } }",
        "#[cfg(test)] trait T { m! { emit_dictation_terminal(a) } }",
        r#"#[cfg(test)] mod m { extern "C" { m! { emit_dictation_terminal(a) } } }"#,
        "#[cfg(all(test, unix))] impl X { fn f() { emit_dictation_terminal(a); } }",
    ] {
        assert_eq!(callers(test_only), vec![], "{test_only}");
    }
    for production in [
        r#"#[cfg(any(test, unix))] extern "C" { m!(emit_dictation_terminal(a)); }"#,
        r#"extern "C" { m!(emit_dictation_terminal(a)); }"#,
        r#"#[cfg(not(test))] extern "C" { m!(emit_dictation_terminal(a)); }"#,
    ] {
        assert_eq!(
            callers(production),
            vec![("commands/a.rs".to_string(), String::new(), 1)],
            "{production}"
        );
    }
}
