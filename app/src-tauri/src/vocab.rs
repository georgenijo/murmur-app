//! Code-aware vocabulary: scan a project folder for code identifiers and turn
//! the most frequent ones into a Whisper `initial_prompt` so dictating code
//! terms (e.g. "useEffect", "tauri", "stderr") transcribes accurately.
//!
//! The extraction and ranking logic is intentionally *pure* (string in, string
//! out) so it is unit-testable without touching the filesystem. The directory
//! walk lives in a thin wrapper (`build_vocab_prompt_from_dir`) that reads files
//! and then hands their contents to the pure functions.

use std::collections::HashMap;
use std::path::Path;

/// Skip individual files larger than this (bytes). A single huge minified bundle
/// or vendored blob shouldn't dominate the scan or hang it.
pub const MAX_FILE_BYTES: u64 = 512 * 1024;

/// Cap on the number of files we read in one scan, so a giant repo can't hang
/// the pipeline. Files are visited in directory-walk order until the cap is hit.
pub const MAX_FILES: usize = 200;

/// Total bytes we will read across all files in one scan. A second guard on top
/// of the per-file and file-count caps.
pub const MAX_TOTAL_BYTES: u64 = 8 * 1024 * 1024;

/// Source file extensions we scan. Kept to common code formats so we don't pull
/// identifiers out of, say, lockfiles or binary assets.
pub const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "java", "kt",
    "swift", "c", "h", "cc", "cpp", "hpp", "cs", "rb", "php", "scala", "sh",
    "lua", "dart", "vue", "svelte",
];

/// Return true if `path` has one of the source extensions we scan (case-insensitive).
pub fn is_source_file(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => {
            let lower = ext.to_ascii_lowercase();
            SOURCE_EXTENSIONS.contains(&lower.as_str())
        }
        None => false,
    }
}

/// Directory names we never descend into while walking a project. These hold
/// dependencies, build output, or VCS data — not the user's own identifiers.
const SKIP_DIRS: &[&str] = &[
    "node_modules", "target", ".git", "dist", "build", ".next", "vendor",
    "__pycache__", ".venv", "venv", ".svn", ".hg", "Pods", "DerivedData",
    ".cargo", ".idea", ".vscode", "coverage", "out",
];

/// Return true if a directory with this name should not be descended into.
pub fn is_skipped_dir(name: &str) -> bool {
    SKIP_DIRS.contains(&name)
}

/// Language keywords and ultra-common English/programming words to exclude from
/// the vocabulary. These add no value as a transcription bias (Whisper already
/// knows "function", "return", "true", etc.) and would crowd out real identifiers.
fn is_stop_word(word: &str) -> bool {
    // Compared case-insensitively against a lower-cased token.
    matches!(
        word,
        // Common control-flow / declaration keywords across languages
        "the" | "and" | "for" | "you" | "this" | "that" | "with" | "function"
            | "return" | "const" | "let" | "var" | "import" | "export" | "from"
            | "class" | "struct" | "enum" | "impl" | "trait" | "type" | "interface"
            | "public" | "private" | "protected" | "static" | "final" | "void"
            | "null" | "true" | "false" | "none" | "self" | "super" | "new"
            | "delete" | "async" | "await" | "yield" | "throw" | "throws" | "catch"
            | "try" | "finally" | "while" | "break" | "continue" | "else" | "elif"
            | "match" | "case" | "switch" | "default" | "default_" | "where" | "when"
            | "then" | "def" | "fun" | "func" | "val" | "use" | "mod" | "pub"
            | "string" | "number" | "boolean" | "bool" | "int" | "float" | "double"
            | "char" | "byte" | "long" | "short" | "object" | "array" | "list"
            | "map" | "set" | "not" | "are" | "was" | "were" | "has" | "have"
            | "had" | "will" | "can" | "all" | "any" | "out" | "get"
            | "value" | "values" | "data" | "result" | "error" | "name" | "names"
            | "key" | "keys" | "item" | "items" | "args" | "kwargs" | "params"
            | "param" | "index" | "length" | "size" | "count" | "into" | "über"
    )
}

/// Pull camelCase / snake_case / PascalCase identifiers out of arbitrary source
/// text. A token qualifies if it:
///   - starts with an ASCII letter,
///   - is composed of ASCII letters/digits/underscores,
///   - is at least 3 characters long,
///   - is not a language keyword / common word (case-insensitive),
///   - is not all-lowercase *and* devoid of structure (those are usually plain
///     English; we keep them only if they look code-ish — contain an underscore,
///     an internal capital, or a digit).
///
/// The returned vector preserves source order and may contain duplicates;
/// dedupe/ranking is the job of [`build_vocab_prompt`].
pub fn extract_identifiers(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];
        // Identifier must start with an ASCII letter (alpha-led).
        if b.is_ascii_alphabetic() {
            let start = i;
            i += 1;
            while i < len {
                let c = bytes[i];
                if c.is_ascii_alphanumeric() || c == b'_' {
                    i += 1;
                } else {
                    break;
                }
            }
            let token = &source[start..i];
            if is_candidate(token) {
                out.push(token.to_string());
            }
        } else {
            // Advance past one UTF-8 char (handles multi-byte without splitting).
            i += utf8_char_len(b);
        }
    }

    out
}

/// Length in bytes of a UTF-8 character given its leading byte.
fn utf8_char_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else if b >> 3 == 0b11110 {
        4
    } else {
        1
    }
}

/// Decide whether a raw token is worth keeping as a vocabulary candidate.
fn is_candidate(token: &str) -> bool {
    if token.len() < 3 {
        return false;
    }
    let first = token.as_bytes()[0];
    if !first.is_ascii_alphabetic() {
        return false;
    }

    let lower = token.to_ascii_lowercase();
    if is_stop_word(&lower) {
        return false;
    }

    let has_underscore = token.contains('_');
    let has_digit = token.bytes().any(|c| c.is_ascii_digit());
    // Internal uppercase after the first char => camelCase / PascalCase shape.
    let has_internal_upper = token
        .bytes()
        .skip(1)
        .any(|c| c.is_ascii_uppercase());
    // A leading uppercase letter => PascalCase / acronym (e.g. "TauriApp", "API").
    let leads_upper = first.is_ascii_uppercase();

    // Plain all-lowercase words with no structure are almost always ordinary
    // English ("hello", "value", "thing") — skip them. We keep a lowercase token
    // only when it carries code structure (underscore / digit) OR is an unusual
    // term Whisper is unlikely to know. We approximate the latter by allowing
    // lowercase tokens that contain an internal capital (impossible here) — so in
    // practice lowercase plain words are dropped unless they have a digit/underscore.
    if !has_underscore && !has_digit && !has_internal_upper && !leads_upper {
        return false;
    }

    true
}

/// Build a Whisper initial-prompt string from in-memory file contents.
///
/// Pure: takes `(label, contents)` pairs (the label is only used for ordering
/// stability and is otherwise ignored), extracts identifiers from every file,
/// dedupes case-insensitively, ranks by descending frequency (ties broken by
/// first-seen order for determinism), and returns the top `max_terms` joined by
/// spaces — directly usable as `params.set_initial_prompt(...)`.
pub fn build_vocab_prompt<S: AsRef<str>>(files: &[(S, S)], max_terms: usize) -> String {
    if max_terms == 0 {
        return String::new();
    }

    // frequency by lowercased key; remember the first surface form and first-seen
    // order so ranking is deterministic across runs.
    let mut freq: HashMap<String, u32> = HashMap::new();
    let mut surface: HashMap<String, String> = HashMap::new();
    let mut order: HashMap<String, usize> = HashMap::new();
    let mut next_order = 0usize;

    for (_label, contents) in files {
        for ident in extract_identifiers(contents.as_ref()) {
            let key = ident.to_ascii_lowercase();
            *freq.entry(key.clone()).or_insert(0) += 1;
            surface.entry(key.clone()).or_insert_with(|| ident.clone());
            order.entry(key.clone()).or_insert_with(|| {
                let o = next_order;
                next_order += 1;
                o
            });
        }
    }

    let mut ranked: Vec<(&String, u32)> = freq.iter().map(|(k, v)| (k, *v)).collect();
    // Sort by descending frequency, then ascending first-seen order for stable ties.
    ranked.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| order[a.0].cmp(&order[b.0]))
    });

    ranked
        .into_iter()
        .take(max_terms)
        .map(|(k, _)| surface[k].clone())
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// Thin (untested) filesystem wrapper. The pure functions above carry the logic;
// this just walks a directory, applies the size/count guards, and reads files.
// ---------------------------------------------------------------------------

/// Walk `folder`, read up to the guarded number/size of source files, and build
/// a vocabulary prompt of at most `max_terms` identifiers. Unreadable files are
/// logged and skipped. Returns an empty string if nothing usable is found.
///
/// This function is deliberately not unit-tested (it touches the filesystem);
/// all of its non-trivial logic delegates to the pure functions above.
#[allow(dead_code)]
pub fn build_vocab_prompt_from_dir(folder: &Path, max_terms: usize) -> String {
    let files = collect_source_files(folder);
    if files.is_empty() {
        return String::new();
    }
    build_vocab_prompt(&files, max_terms)
}

/// Iteratively walk `folder` (no recursion to bound stack), honoring the file
/// count, per-file size, total-byte, extension, and skip-dir guards. Returns
/// `(path_string, contents)` pairs ready for [`build_vocab_prompt`].
#[allow(dead_code)]
fn collect_source_files(folder: &Path) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = Vec::new();
    let mut total_bytes: u64 = 0;
    let mut stack: Vec<std::path::PathBuf> = vec![folder.to_path_buf()];

    while let Some(dir) = stack.pop() {
        if files.len() >= MAX_FILES || total_bytes >= MAX_TOTAL_BYTES {
            break;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                #[cfg(not(test))]
                tracing::warn!(target: "pipeline", "vocab: skipping unreadable dir {}: {}", dir.display(), e);
                let _ = e;
                continue;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };

            if file_type.is_dir() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                // Skip well-known dependency/build dirs and any hidden dir
                // (leading dot) — both are noise for a vocabulary scan.
                if name.starts_with('.') || is_skipped_dir(&name) {
                    continue;
                }
                stack.push(path);
                continue;
            }

            if !file_type.is_file() || !is_source_file(&path) {
                continue;
            }

            if files.len() >= MAX_FILES || total_bytes >= MAX_TOTAL_BYTES {
                break;
            }

            // Per-file size guard before reading.
            match std::fs::metadata(&path) {
                Ok(meta) if meta.len() > MAX_FILE_BYTES => continue,
                Ok(_) => {}
                Err(_) => continue,
            }

            match std::fs::read_to_string(&path) {
                Ok(contents) => {
                    total_bytes += contents.len() as u64;
                    files.push((path.to_string_lossy().to_string(), contents));
                }
                Err(e) => {
                    #[cfg(not(test))]
                    tracing::warn!(target: "pipeline", "vocab: skipping unreadable file {}: {}", path.display(), e);
                    let _ = e;
                }
            }
        }
    }

    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn has(v: &[String], s: &str) -> bool {
        v.iter().any(|x| x == s)
    }

    #[test]
    fn extracts_camel_case() {
        let ids = extract_identifiers("const x = useEffect(handleClick);");
        assert!(has(&ids, "useEffect"), "got {:?}", ids);
        assert!(has(&ids, "handleClick"), "got {:?}", ids);
    }

    #[test]
    fn extracts_pascal_case() {
        let ids = extract_identifiers("struct WhisperBackend; impl TranscriptionBackend {}");
        assert!(has(&ids, "WhisperBackend"));
        assert!(has(&ids, "TranscriptionBackend"));
    }

    #[test]
    fn extracts_snake_case() {
        let ids = extract_identifiers("let custom_vocabulary = parse_wav_to_samples(raw);");
        assert!(has(&ids, "custom_vocabulary"));
        assert!(has(&ids, "parse_wav_to_samples"));
    }

    #[test]
    fn keeps_identifiers_with_digits() {
        let ids = extract_identifiers("model = large_v3; let utf8 = 1;");
        assert!(has(&ids, "large_v3"));
        assert!(has(&ids, "utf8"));
    }

    #[test]
    fn skips_short_tokens() {
        let ids = extract_identifiers("a bc x y to id");
        assert!(ids.is_empty(), "got {:?}", ids);
    }

    #[test]
    fn skips_plain_lowercase_english() {
        // No structure (no underscore/digit/internal-cap) => treated as English.
        let ids = extract_identifiers("hello world this is some plain text here");
        assert!(ids.is_empty(), "got {:?}", ids);
    }

    #[test]
    fn skips_language_keywords() {
        let ids = extract_identifiers("function return const let var class import export");
        assert!(ids.is_empty(), "got {:?}", ids);
    }

    #[test]
    fn keyword_skip_is_case_insensitive() {
        // PascalCase keyword-ish words still drop if they map to a stop word.
        let ids = extract_identifiers("Function Return Class");
        // "Function"/"Return"/"Class" lowercase to stop words -> dropped.
        assert!(ids.is_empty(), "got {:?}", ids);
    }

    #[test]
    fn does_not_split_inside_token() {
        // Leading digit means the run isn't alpha-led at that position; the
        // alpha part still gets captured separately.
        let ids = extract_identifiers("var3Thing = 1");
        assert!(has(&ids, "var3Thing"), "got {:?}", ids);
    }

    #[test]
    fn handles_unicode_without_panicking() {
        // Multi-byte chars must not cause a byte-boundary panic.
        let ids = extract_identifiers("über naïve fooBar café");
        assert!(has(&ids, "fooBar"), "got {:?}", ids);
    }

    #[test]
    fn build_prompt_dedupes() {
        let files = vec![("a.rs", "fooBar fooBar fooBar")];
        let prompt = build_vocab_prompt(&files, 10);
        assert_eq!(prompt, "fooBar");
    }

    #[test]
    fn build_prompt_ranks_by_frequency() {
        // barBaz appears 3x, fooBar 1x => barBaz ranks first.
        let files = vec![("a.rs", "fooBar barBaz barBaz barBaz")];
        let prompt = build_vocab_prompt(&files, 10);
        let parts: Vec<&str> = prompt.split(' ').collect();
        assert_eq!(parts[0], "barBaz", "prompt was {:?}", prompt);
        assert!(parts.contains(&"fooBar"));
    }

    #[test]
    fn build_prompt_respects_max_terms() {
        let files = vec![("a.rs", "oneTwo threeFour fiveSix sevenEight")];
        let prompt = build_vocab_prompt(&files, 2);
        let parts: Vec<&str> = prompt.split(' ').filter(|s| !s.is_empty()).collect();
        assert_eq!(parts.len(), 2, "prompt was {:?}", prompt);
    }

    #[test]
    fn build_prompt_zero_max_terms_is_empty() {
        let files = vec![("a.rs", "fooBar barBaz")];
        assert_eq!(build_vocab_prompt(&files, 0), "");
    }

    #[test]
    fn build_prompt_empty_input_is_empty() {
        let files: Vec<(&str, &str)> = vec![];
        assert_eq!(build_vocab_prompt(&files, 10), "");
    }

    #[test]
    fn build_prompt_merges_across_files() {
        // fooBar in two files (2 total) outranks barBaz (1) -> fooBar first.
        let files = vec![("a.rs", "fooBar barBaz"), ("b.rs", "fooBar")];
        let prompt = build_vocab_prompt(&files, 10);
        let parts: Vec<&str> = prompt.split(' ').collect();
        assert_eq!(parts[0], "fooBar", "prompt was {:?}", prompt);
    }

    #[test]
    fn build_prompt_tie_break_is_stable_first_seen() {
        // Both appear once; first-seen order wins -> alphaOne before betaTwo.
        let files = vec![("a.rs", "alphaOne betaTwo")];
        let prompt = build_vocab_prompt(&files, 10);
        assert_eq!(prompt, "alphaOne betaTwo");
    }

    #[test]
    fn build_prompt_preserves_first_surface_form() {
        // First surface form (camelCase) is kept even if later occurrences differ
        // in case; they all map to the same lowercase key.
        let files = vec![("a.rs", "fooBar FOOBAR foobar")];
        let prompt = build_vocab_prompt(&files, 10);
        assert_eq!(prompt, "fooBar");
    }

    #[test]
    fn is_source_file_matches_extensions() {
        assert!(is_source_file(&PathBuf::from("a/b/c.rs")));
        assert!(is_source_file(&PathBuf::from("x.TS"))); // case-insensitive
        assert!(is_source_file(&PathBuf::from("comp.tsx")));
        assert!(!is_source_file(&PathBuf::from("README.md")));
        assert!(!is_source_file(&PathBuf::from("data.json")));
        assert!(!is_source_file(&PathBuf::from("noext")));
    }

    #[test]
    fn skip_dir_covers_dependency_dirs() {
        assert!(is_skipped_dir("node_modules"));
        assert!(is_skipped_dir("target"));
        assert!(is_skipped_dir(".git"));
        assert!(!is_skipped_dir("src"));
        assert!(!is_skipped_dir("components"));
    }

    #[test]
    fn guard_constants_are_sane() {
        // Sanity-check the guard values so an accidental edit to 0 is caught.
        assert!(MAX_FILE_BYTES > 0);
        assert!(MAX_FILES > 0);
        assert!(MAX_TOTAL_BYTES >= MAX_FILE_BYTES);
        assert!(!SOURCE_EXTENSIONS.is_empty());
    }

    #[test]
    fn realistic_snippet_prioritizes_repeated_identifiers() {
        let src = r#"
            import { useEffect, useState } from 'react';
            export function useRecordingState() {
              const [status, setStatus] = useState('idle');
              useEffect(() => { configureDictation(); }, []);
              useEffect(() => { configureDictation(); }, [status]);
              return { status, setStatus };
            }
        "#;
        let files = vec![("hook.ts", src)];
        let prompt = build_vocab_prompt(&files, 5);
        // useEffect appears twice; should be present and ranked highly.
        assert!(prompt.contains("useEffect"), "prompt was {:?}", prompt);
        assert!(prompt.contains("configureDictation"), "prompt was {:?}", prompt);
        // 'react'/'from'/'import'/'function'/'const'/'return' must be filtered.
        assert!(!prompt.contains("function"));
        assert!(!prompt.contains("import"));
        assert!(!prompt.contains("return"));
    }
}
