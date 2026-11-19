//! Static checks over the shadow's own source: it never executes, never draws an op
//! id, never reaches `tracing`, and its capture path is the single push it claims.

use std::path::{Path, PathBuf};

use super::harness::*;

/// The shadow's production source text: comments removed and every `#[cfg(test)]`
/// item (a module, a function, an `if`, a field) removed, so the static checks see
/// exactly what a non-test build compiles.
pub(super) fn production_source(source: &str) -> String {
    let stripped = strip_cfg_test(source);
    stripped
        .lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Remove each `#[cfg(test)]` attribute and the item it gates. An item is a one-line
/// field or initializer (`name: value,`), or runs to the end of its `{ ... }` block,
/// or to its `;`.
fn strip_cfg_test(source: &str) -> String {
    const ATTRIBUTE: &str = "#[cfg(test)]";
    let mut out = String::new();
    let mut rest = source;
    while let Some(at) = rest.find(ATTRIBUTE) {
        out.push_str(&rest[..at]);
        let mut item = &rest[at + ATTRIBUTE.len()..];
        // Further attributes on the same item.
        loop {
            let trimmed = item.trim_start();
            if trimmed.starts_with("#[") {
                match trimmed.find(']') {
                    Some(end) => item = &trimmed[end + 1..],
                    None => break,
                }
            } else {
                item = trimmed;
                break;
            }
        }
        let first_line = item.lines().next().unwrap_or("");
        let is_field = {
            let line = first_line.trim_start();
            let line = line.strip_prefix("pub(crate) ").unwrap_or(line);
            let line = line.strip_prefix("pub ").unwrap_or(line);
            let name_end = line.find(':').unwrap_or(0);
            name_end > 0
                && line[..name_end]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !line[name_end..].starts_with("::")
        };
        if is_field {
            rest = &item[first_line.len()..];
            continue;
        }
        let brace = item.find('{');
        let semicolon = item.find(';');
        match (brace, semicolon) {
            (Some(open), semi) if semi.is_none_or(|s| open < s) => {
                let mut depth = 0usize;
                let mut end = item.len();
                for (offset, ch) in item[open..].char_indices() {
                    match ch {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                end = open + offset + 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                rest = &item[end..];
            }
            (_, Some(semi)) => rest = &item[semi + 1..],
            _ => rest = "",
        }
    }
    out.push_str(rest);
    out
}

/// The body of the first function or type whose signature starts with `signature`.
pub(super) fn function_body(source: &str, signature: &str) -> String {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("`{signature}` not found"));
    let open = start + source[start..].find('{').expect("a body");
    let mut depth = 0usize;
    for (offset, ch) in source[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return source[open..=open + offset].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("`{signature}` is unbalanced");
}

/// Every non-test source file of the shadow module, read from disk (so a new file is
/// scanned without anyone listing it): the whole files, not a hand-picked set.
fn shadow_sources() -> Vec<(String, String)> {
    let dir: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/dictation_shadow");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        // `tests.rs`, `tests/` and the counting allocator are test-only.
        if path.extension().is_some_and(|ext| ext == "rs")
            && name != "tests.rs"
            && name != "alloc_probe.rs"
        {
            files.push((name, std::fs::read_to_string(&path).unwrap()));
        }
    }
    files.sort();
    files
}

#[test]
fn the_cfg_test_stripper_removes_exactly_what_a_build_does_not_compile() {
    let source = "\
struct A {
    keep: u8,
    #[cfg(test)]
    gone: Mutex<Option<X>>,
}
fn f() {
    #[cfg(test)]
    if sync.load(Ordering::Relaxed) {
        deliver(msg);
        return;
    }
    push(msg);
}
#[cfg(test)]
impl A where T: Clone, {
    fn only_in_tests() { lock(); }
}
#[cfg(test)]
mod tests;
fn after() { kept(); }
";
    let code = production_source(source);
    assert!(code.contains("keep: u8") && code.contains("push(msg)") && code.contains("kept()"));
    for gone in ["Mutex", "deliver", "lock()", "only_in_tests", "mod tests"] {
        assert!(!code.contains(gone), "`{gone}` survived: {code}");
    }
}

#[test]
fn shadow_never_executes() {
    // The shadow side holds no port, adapter, executor, lock owner or window, and
    // nothing of it reaches `tracing` (so nothing passes through `TauriEmitterLayer`,
    // app `State`, capture health, the event buffer or a window): a scan over the
    // *whole* of every non-test source file.
    let forbidden = [
        "FieldAdapter",
        "dictation_executor",
        "core_history",
        "AppState",
        "AppHandle",
        "tauri::",
        "commands::",
        "emit_dictation_terminal",
        "transition_dictation_status",
        "keyboard::",
        "audio_lifecycle",
        "tracing",
        "TauriEmitterLayer",
        "State<",
        "capture_health",
        "event_buffer",
        "WebviewWindow",
        "get_webview_window",
        "emit_all",
        "app.emit(",
        "handle.emit(",
        ".emit_to(",
    ];
    let files = shadow_sources();
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    for expected in [
        "compare.rs",
        "mod.rs",
        "pd.rs",
        "summary.rs",
        "transport.rs",
        "types.rs",
    ] {
        assert!(names.contains(&expected), "{expected} is not scanned");
    }
    for (name, source) in &files {
        let code = production_source(source);
        for token in forbidden {
            // `live_counters.rs` is the one file that reads the live side's counters
            // (D-S2c-15 S3, S4); it is held to loads only, below.
            if token == "dictation_executor" && name == "live_counters.rs" {
                continue;
            }
            assert!(
                !code.contains(token),
                "dictation_shadow/{name} names `{token}`: the shadow never executes and never logs"
            );
        }
    }
    // The counter reader only loads: no lock, store, write or call into the executor
    // other than the two named reads.
    let (_, counters) = files
        .iter()
        .find(|(name, _)| name == "live_counters.rs")
        .expect("live_counters.rs is scanned");
    let counters = production_source(counters);
    for token in [
        ".lock(",
        ".store(",
        "fetch_add",
        "swap(",
        "write",
        "emit",
        "spawn",
    ] {
        assert!(
            !counters.contains(token),
            "live_counters.rs must only read: `{token}`"
        );
    }
    // Exactly these two calls into the live side, by full path (a substitute such as
    // `dictation_executor::shadow::no_decision()` is a different path and fails here).
    let mut paths: Vec<String> = counters
        .match_indices("dictation_executor::")
        .map(|(at, _)| {
            let rest = &counters[at..];
            let end = rest
                .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':'))
                .unwrap_or(rest.len());
            // The path and its empty argument list: a call, not a mention.
            let call = if rest[end..].starts_with("()") {
                "()"
            } else {
                "?"
            };
            format!("{}{call}", &rest[..end])
        })
        .collect();
    paths.sort();
    assert_eq!(
        paths,
        [
            "dictation_executor::adapter::core_divergences()",
            "dictation_executor::shadow::conservation_counts()",
        ],
        "live_counters.rs reads exactly the divergence count and the entry/header books"
    );
    // The only thing the shadow writes is its own two files, through `summary`.
    let mut writers = Vec::new();
    for (name, source) in &files {
        let code = production_source(source);
        if code.contains("File::create")
            || code.contains("OpenOptions")
            || code.contains("fs::write")
        {
            writers.push(name.clone());
        }
    }
    assert_eq!(
        writers,
        ["summary.rs"],
        "only summary.rs touches the file system"
    );

    // The comparison runs wherever the caller runs it and decides with the pure
    // function: a flight compares with no live call reachable.
    let mut h = harness();
    h.feed(&short_idle(1, 1));
    h.feed(&start(2, 2, Live::default()));
    assert_eq!(h.stats().matched, 2);
}

#[test]
fn shadow_replays_live_op_ids() {
    // The shadow never draws an op id: it does not touch `NEXT_OP`, the history or
    // its registration functions.
    for (name, source) in shadow_sources() {
        let code = production_source(&source);
        for token in [
            "NEXT_OP",
            "next_op",
            ".reg(",
            "reg_pipeline",
            "ret_if_matches",
        ] {
            assert!(
                !code.contains(token),
                "dictation_shadow/{name} names `{token}`: op ids are replayed, never drawn"
            );
        }
        // `fetch_add(1` is the transport's own seq and recorder counters, the summary's
        // temporary-file counter and the acquisition-site table's counts, nothing else.
        if name != "transport.rs" && name != "summary.rs" && name != "sites.rs" {
            assert!(
                !code.contains("fetch_add(1"),
                "dictation_shadow/{name} counts something that looks like an op id"
            );
        }
        // `sites.rs` may count only its own table: one slot's tally, and the overflow.
        if name == "sites.rs" {
            for line in code.lines().filter(|line| line.contains("fetch_add(1")) {
                assert!(
                    line.contains("slot.count.fetch_add(1")
                        || line.contains("OVERFLOW.fetch_add(1"),
                    "dictation_shadow/sites.rs counts something other than its table: {line}"
                );
            }
        }
    }
}

#[test]
fn shadow_off_by_default() {
    assert!(!crate::dictation_shadow::parse_flag(None));
    assert!(!crate::dictation_shadow::parse_flag(Some("")));
    assert!(!crate::dictation_shadow::parse_flag(Some("0")));
    assert!(!crate::dictation_shadow::parse_flag(Some("true")));
    assert!(crate::dictation_shadow::parse_flag(Some("1")));
    assert_eq!(crate::dictation_shadow::ENV_VAR, "MURMUR_SHADOW");
    // The feature alone does not arm it: with the variable unset, capture is off
    // and no recorder is created.
    if std::env::var(crate::dictation_shadow::ENV_VAR).is_err() {
        assert!(!crate::dictation_shadow::enabled());
        assert!(crate::dictation_shadow::recorder().is_none());
    }
}

#[test]
fn nothing_but_the_comparison_thread_writes_the_summary() {
    // Review fix (Sol 6.2 major 8): the exit hook used to lock the comparator and
    // write the summary itself. Now only `run` (the comparison thread) calls it.
    let mut callers = Vec::new();
    for (name, source) in shadow_sources() {
        let code = production_source(&source);
        if name == "summary.rs" {
            continue;
        }
        if code.contains("write_summary(") {
            callers.push(name);
        }
    }
    assert_eq!(callers, ["mod.rs"]);
    let module = production_source(include_str!("../mod.rs"));
    assert!(
        !function_body(&module, "pub(crate) fn finish_on_exit(").contains("write_summary"),
        "the exit hook signals the thread; it does not write"
    );
    assert!(function_body(&module, "fn write_stats(").contains("write_summary("));
}
