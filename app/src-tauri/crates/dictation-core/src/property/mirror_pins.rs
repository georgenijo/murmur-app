//! Whole-function pins of the supervisor mirror (D-S2b-13 (a)).
//!
//! The mirror table (`supervisor.rs`) cites lines of `audio_lifecycle.rs`; a fragment or a
//! guard check only sees what its author thought to pin. Two rounds in a row a reviewer
//! turned around a guard nobody had pinned, so the model now pins **every function it
//! mirrors in full**: [`PINS`] holds the hash of the function's normalized token stream,
//! and any edit of a pinned function fails [`the_mirrored_functions_are_pinned`] until the
//! row is re-pinned. **A re-pin says in its commit message that the model step was
//! re-checked against the new code.**
//!
//! The normalization is the one the shell's ratchet uses (D-S2b-10 (e), #1152): whitespace
//! and comments (doc comments included) carry no information; every other token, every
//! comma and every punctuation's spacing (`Joint` is `~`) is kept, so `(x,)` and `(x)`,
//! and `m!(&&x)` and `m!(& &x)`, differ. The core has no dependencies (not even dev
//! ones), so the tokenizer here is the small lexer below instead of `proc_macro2`; it
//! produces the same stream up to the spelling of a token.
//!
//! The test also fails when a pinned function calls a function of the same file that has
//! no pin and no entry in [`NOT_MODELED`] (which says why the model does not mirror it).

use std::collections::{BTreeMap, BTreeSet};

const LIFECYCLE_SOURCE: &str = include_str!("../../../../src/audio_lifecycle.rs");
const AUDIO_SOURCE: &str = include_str!("../../../../src/audio.rs");

/// One pinned function of the production source.
pub struct FnPin {
    pub file: &'static str,
    pub function: &'static str,
    /// FNV-1a of the function's normalized token stream.
    pub hash: u64,
}

const fn pin(file: &'static str, function: &'static str, hash: u64) -> FnPin {
    FnPin {
        file,
        function,
        hash,
    }
}

/// Every function the mirror table cites, plus the functions they call that decide a
/// step. Re-pin only after re-checking the model step against the new code.
pub const PINS: &[FnPin] = &[
    pin("audio.rs", "spawn_capture_worker", 0x364c427848c65c89),
    pin(
        "audio_lifecycle.rs",
        "await_start_response",
        0x3f69bfd37b4c8132,
    ),
    pin("audio_lifecycle.rs", "begin_recovery", 0x03b05ac17991c8f8),
    pin("audio_lifecycle.rs", "cancel", 0x05ad59e26a075384),
    pin(
        "audio_lifecycle.rs",
        "cancel_starting_for_environment_change",
        0xad9b3fc38d82573e,
    ),
    pin("audio_lifecycle.rs", "clear_owner", 0x84027fdc52c83db9),
    pin("audio_lifecycle.rs", "dictation_id", 0xbc86783bd640d5c7),
    pin("audio_lifecycle.rs", "finish_attempt", 0xb718f179aabfd700),
    pin(
        "audio_lifecycle.rs",
        "handle_deadlines_at",
        0x4a9663d1bb8e70d3,
    ),
    pin("audio_lifecycle.rs", "handle_message", 0x86622aed59556537),
    pin("audio_lifecycle.rs", "handle_start", 0xdde70149a1ca08af),
    pin(
        "audio_lifecycle.rs",
        "handle_worker_event",
        0x18785082c3f23326,
    ),
    pin(
        "audio_lifecycle.rs",
        "interrupted_captures",
        0xa8c556f79ec3cbc9,
    ),
    pin(
        "audio_lifecycle.rs",
        "report_cleanup_stalled",
        0x1e7da2b580885986,
    ),
    pin(
        "audio_lifecycle.rs",
        "report_failure_once",
        0xf48b1da5a7d01380,
    ),
    pin("audio_lifecycle.rs", "retains_samples", 0x23b292790295a604),
    pin("audio_lifecycle.rs", "run_supervisor", 0xc5fc5af2ca7df54f),
    pin(
        "audio_lifecycle.rs",
        "send_start_request",
        0xd2842ed6352f2856,
    ),
    pin("audio_lifecycle.rs", "send_worker_stop", 0x9c49d7203dfaea88),
    pin("audio_lifecycle.rs", "set_owner", 0xc686d40a874eb6dd),
    pin("audio_lifecycle.rs", "set_phase", 0xb888721f05eb8cd2),
    pin(
        "audio_lifecycle.rs",
        "stop_with_trigger",
        0x2a34f43638964c6d,
    ),
    pin("audio_lifecycle.rs", "take_samples", 0xa217054e6dbce9ae),
];

/// Functions of `audio_lifecycle.rs` that a pinned function calls and the model does not
/// mirror, each with the reason.
pub const NOT_MODELED: &[(&str, &str)] = &[
    ("active_initialization_elapsed", "a duration: the hard and still-connecting deadlines are timer steps the harness fires nondeterministically"),
    ("adopt_speculative", "the speculative phase is an optional feature the dictation commands never enter"),
    ("as_str", "trait and enum spelling helpers used by logs"),
    ("automatic_capture_authorized", "a permission probe of the automatic preview, which dictation never starts"),
    ("convert_for_delivery", "sample-rate conversion; the model carries sample counts, not samples"),
    ("deadline_wait", "how long the loop sleeps; the model's timers are steps, not durations"),
    ("default", "constructors"),
    ("drop", "a guard's destructor"),
    ("hal_boundary", "the HAL mutex around device enumeration; no state change the model reads"),
    ("hand_preview_to", "the preview handoff: the model's start accepts a handoff only as a start (PD-20's shell fixture drives and cites it)"),
    ("invalidate_input_health", "a health-cache write with no lifecycle effect"),
    ("is_microphone_benchmark", "an owner-kind predicate on a non-dictation owner"),
    ("kind", "an owner-kind name for logs"),
    ("log_speculative_discard", "logging of the speculative phase"),
    ("log_speculative_failure", "logging of the speculative phase"),
    ("notify", "the lifecycle sink; the model's steps are the notifications"),
    ("preview_handoff_ready", "the handoff's readiness check (see hand_preview_to)"),
    ("publish_gesture_ready", "a telemetry mark of the Ready notification"),
    ("recording_duration_since_ready", "a duration for the failure report; the model keeps no clocks"),
    ("release_anchor", "the stop tail's length; the tail is a timer step in the model"),
    ("release_tail", "the stop tail's length; the tail is a timer step in the model"),
    ("snapshot_for_asr", "a peek: no state change"),
    ("snapshot_since", "a peek: no state change"),
    ("spawn", "the worker factory; the model's worker is a task"),
    ("start_speculative", "the speculative phase is an optional feature the dictation commands never enter"),
    ("supervisor", "the process-wide supervisor handle"),
    ("tail_deadline", "the stop tail's length; the tail is a timer step in the model"),
    ("telemetry_id", "an owner's id for logs"),
    ("warm_epoch", "a warm-standby topology probe; standby is not modeled"),
    ("warm_topology_is_current", "a warm-standby topology probe; standby is not modeled"),
];

// ------------------------------------------------------------------- tokenizer

fn is_punct(c: char) -> bool {
    "!#$%&*+,-./:;<=>?@^|~\\".contains(c)
}

/// The tokens of `source` with no whitespace and no comment: a literal is one token, a
/// delimiter is one token, and a punctuation character carries `~` when another
/// punctuation character follows it directly (`proc_macro2`'s `Joint`).
fn normalized_tokens(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c.is_whitespace() {
            i += 1;
        } else if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 0;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else if let Some(end) = raw_string_end(&chars, i) {
            out.push(chars[i..end].iter().collect());
            i = end;
        } else if c == '"' || (c == 'b' && next == Some('"')) {
            let start = i;
            i += if c == 'b' { 2 } else { 1 };
            while i < chars.len() && chars[i] != '"' {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            i += 1;
            out.push(chars[start..i.min(chars.len())].iter().collect());
        } else if c == '\'' || (c == 'b' && next == Some('\'')) {
            let start = i;
            let quote = if c == 'b' { i + 1 } else { i };
            let after = chars.get(quote + 1).copied();
            let is_char = after == Some('\\') || chars.get(quote + 2) == Some(&'\'');
            if is_char {
                i = quote + 1;
                while i < chars.len() && chars[i] != '\'' {
                    i += if chars[i] == '\\' { 2 } else { 1 };
                }
                i += 1;
            } else {
                // A lifetime or a label.
                i = quote + 1;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
            }
            out.push(chars[start..i.min(chars.len())].iter().collect());
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            if c == 'r' && next == Some('#') {
                i += 2;
            }
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(chars[start..i].iter().collect());
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric()
                    || chars[i] == '_'
                    || (chars[i] == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit)))
            {
                i += 1;
            }
            out.push(chars[start..i].iter().collect());
        } else if "(){}[]".contains(c) {
            out.push(c.to_string());
            i += 1;
        } else {
            let joint = next.is_some_and(is_punct);
            out.push(if joint {
                format!("{c}~")
            } else {
                c.to_string()
            });
            i += 1;
        }
    }
    out
}

/// The end (exclusive) of a raw string literal (`r"…"`, `r#"…"#`, `br"…"`) starting at `i`.
fn raw_string_end(chars: &[char], i: usize) -> Option<usize> {
    let c = chars[i];
    if !(c == 'r' || (c == 'b' && chars.get(i + 1) == Some(&'r'))) {
        return None;
    }
    if i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
        return None;
    }
    let mut j = i + if c == 'b' { 2 } else { 1 };
    let mut hashes = 0;
    while chars.get(j) == Some(&'#') {
        hashes += 1;
        j += 1;
    }
    if chars.get(j) != Some(&'"') {
        return None;
    }
    j += 1;
    while j < chars.len() {
        if chars[j] == '"' && (0..hashes).all(|k| chars.get(j + 1 + k) == Some(&'#')) {
            return Some(j + 1 + hashes);
        }
        j += 1;
    }
    Some(chars.len())
}

fn fnv(tokens: &[String]) -> u64 {
    tokens
        .join(" ")
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
}

/// The token ranges `(start, end)` (inclusive) of every function named `name`, from `fn`
/// to the closing brace of its body. A declaration without a body is skipped.
fn function_ranges(tokens: &[String], name: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    for at in 0..tokens.len().saturating_sub(1) {
        if tokens[at] != "fn" || tokens[at + 1] != name {
            continue;
        }
        let mut depth = 0_i32;
        let mut k = at + 2;
        let mut body = None;
        while k < tokens.len() {
            match tokens[k].as_str() {
                "(" | "[" => depth += 1,
                ")" | "]" => depth -= 1,
                ";" if depth == 0 => break,
                "{" if depth == 0 => {
                    body = Some(k);
                    break;
                }
                _ => {}
            }
            k += 1;
        }
        let Some(open) = body else { continue };
        let mut braces = 0_i32;
        for (index, token) in tokens.iter().enumerate().skip(open) {
            match token.as_str() {
                "{" => braces += 1,
                "}" => {
                    braces -= 1;
                    if braces == 0 {
                        found.push((at, index));
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    found
}

/// The production part of a source: everything before its `#[cfg(test)] mod tests`.
fn production(source: &str) -> &str {
    source
        .find("\n#[cfg(test)]\nmod tests")
        .map_or(source, |end| &source[..end])
}

fn source_of(file: &str) -> &'static str {
    match file {
        "audio_lifecycle.rs" => production(LIFECYCLE_SOURCE),
        "audio.rs" => production(AUDIO_SOURCE),
        other => panic!("unknown source file {other}"),
    }
}

/// Every name that is declared as `fn name` in the token stream.
fn defined_functions(tokens: &[String]) -> BTreeMap<String, usize> {
    let mut defined = BTreeMap::new();
    for at in 0..tokens.len().saturating_sub(1) {
        if tokens[at] == "fn" {
            *defined.entry(tokens[at + 1].clone()).or_insert(0) += 1;
        }
    }
    defined
}

/// The names called as `name(` inside `tokens`.
fn called_names(tokens: &[String]) -> BTreeSet<String> {
    let mut called = BTreeSet::new();
    for at in 0..tokens.len().saturating_sub(1) {
        let token = &tokens[at];
        let identifier = token
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
            && !token.starts_with("r#")
            && token != "fn";
        if identifier && tokens[at + 1] == "(" && (at == 0 || tokens[at - 1] != "fn") {
            called.insert(token.clone());
        }
    }
    called
}

#[test]
fn the_mirrored_functions_are_pinned() {
    let mut problems = Vec::new();
    let mut lifecycle_calls = BTreeSet::new();
    // Every pin names exactly one function of its file, with its current hash.
    for entry in PINS {
        let tokens = normalized_tokens(source_of(entry.file));
        let ranges = function_ranges(&tokens, entry.function);
        match ranges.as_slice() {
            [(start, end)] => {
                let body = &tokens[*start..=*end];
                let hash = fnv(body);
                if hash != entry.hash {
                    problems.push(format!(
                        "{}: `{}` was edited (hash {hash:#018x}, pinned {:#018x}): re-check the \
                         model steps that cite it, then re-pin and say so in the commit message",
                        entry.file, entry.function, entry.hash
                    ));
                }
                if entry.file == "audio_lifecycle.rs" {
                    lifecycle_calls.extend(called_names(body));
                }
            }
            other => problems.push(format!(
                "{}: `{}` must be declared exactly once with a body, found {}",
                entry.file,
                entry.function,
                other.len()
            )),
        }
    }
    // Every mirror row cites a pinned function.
    for row in super::supervisor::MIRROR {
        if !PINS
            .iter()
            .any(|entry| entry.file == row.file && entry.function == row.function)
        {
            problems.push(format!(
                "{}: `{}` in {} has no pin",
                row.step, row.function, row.file
            ));
        }
    }
    // A pin that no row cites and no pinned function calls is stale.
    // Every function a pinned function calls is pinned or says why not.
    let lifecycle_tokens = normalized_tokens(source_of("audio_lifecycle.rs"));
    let defined = defined_functions(&lifecycle_tokens);
    let pinned: BTreeSet<&str> = PINS
        .iter()
        .filter(|entry| entry.file == "audio_lifecycle.rs")
        .map(|entry| entry.function)
        .collect();
    let exempt: BTreeSet<&str> = NOT_MODELED.iter().map(|(name, _)| *name).collect();
    for name in &lifecycle_calls {
        if defined.contains_key(name.as_str())
            && !pinned.contains(name.as_str())
            && !exempt.contains(name.as_str())
        {
            problems.push(format!(
                "`{name}` is called by a pinned function and has no pin and no NOT_MODELED \
                 entry"
            ));
        }
    }
    for (name, why) in NOT_MODELED {
        if !lifecycle_calls.contains(*name) {
            problems.push(format!(
                "NOT_MODELED `{name}` is not called by a pinned function"
            ));
        }
        if pinned.contains(name) {
            problems.push(format!("NOT_MODELED `{name}` is pinned"));
        }
        if why.trim().is_empty() {
            problems.push(format!("NOT_MODELED `{name}` has no reason"));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// D-S2b-10 (e), as the shell's ratchet pins it: the hash keeps commas and punctuation
/// spacing and ignores whitespace and comments.
#[test]
fn the_token_hash_keeps_commas_and_punctuation_spacing() {
    let hash = |code: &str| fnv(&normalized_tokens(code));
    assert_ne!(
        hash("fn f() { let _ = (x,); }"),
        hash("fn f() { let _ = (x); }")
    );
    assert_ne!(hash("fn f() { m!(&&x); }"), hash("fn f() { m!(& &x); }"));
    assert_ne!(hash("fn f(a: u8,) {}"), hash("fn f(a: u8) {}"));
    assert_eq!(
        hash("fn f() {\n    // comment\n    /// doc\n    /* block /* nested */ */ let _ = 1;\n}"),
        hash("fn f(){let _=1;}")
    );
    // A guard turned around, a literal or a lifetime changed: all differ.
    assert_ne!(
        hash("fn f() { if a.is_none() {} }"),
        hash("fn f() { if a.is_some() {} }")
    );
    assert_ne!(
        hash("fn f() { g(\"a // b\"); }"),
        hash("fn f() { g(\"a // c\"); }")
    );
    assert_ne!(
        hash("fn f<'a>(x: &'a u8) {}"),
        hash("fn f<'b>(x: &'b u8) {}")
    );
    assert_ne!(
        hash("fn f() { g(r#\"x\"#); }"),
        hash("fn f() { g(r#\"y\"#); }")
    );
    assert_ne!(hash("fn f() { g('a'); }"), hash("fn f() { g('b'); }"));
}

/// The extraction finds a function by name wherever it sits, including a method.
#[test]
fn a_function_is_found_by_name_with_its_whole_body() {
    let tokens = normalized_tokens(
        "impl T { fn a(&self) -> u8 { if x { 1 } else { 2 } } fn b() { a() } }\nfn c();",
    );
    let (start, end) = function_ranges(&tokens, "a")[0];
    assert_eq!(tokens[end], "}");
    assert!(tokens[start..=end].join(" ").ends_with("} }"));
    assert_eq!(function_ranges(&tokens, "b").len(), 1);
    assert!(function_ranges(&tokens, "c").is_empty(), "no body");
}

/// Prints the `PINS` table for the functions it lists today, for a re-pin after the
/// model steps were re-checked: `cargo test -p murmur-dictation-core print_current_pins
/// -- --ignored --nocapture`.
#[test]
#[ignore = "prints the table for a re-pin"]
fn print_current_pins() {
    let mut names: BTreeSet<(&str, &str)> = PINS.iter().map(|e| (e.file, e.function)).collect();
    names.extend(
        super::supervisor::MIRROR
            .iter()
            .map(|row| (row.file, row.function)),
    );
    for (file, function) in names {
        let tokens = normalized_tokens(source_of(file));
        let ranges = function_ranges(&tokens, function);
        let hash = fnv(&tokens[ranges[0].0..=ranges[0].1]);
        println!("    pin(\"{file}\", \"{function}\", {hash:#018x}),");
    }
    let tokens = normalized_tokens(source_of("audio_lifecycle.rs"));
    let defined = defined_functions(&tokens);
    let mut called = BTreeSet::new();
    for entry in PINS.iter().filter(|e| e.file == "audio_lifecycle.rs") {
        let ranges = function_ranges(&tokens, entry.function);
        called.extend(called_names(&tokens[ranges[0].0..=ranges[0].1]));
    }
    for name in called {
        if defined.contains_key(&name) && !PINS.iter().any(|e| e.function == name) {
            println!("// unpinned callee: {name} (declared {}x)", defined[&name]);
        }
    }
}
