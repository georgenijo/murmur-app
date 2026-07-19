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

/// Built-in dictionary of common programming / tooling terms fed to Whisper as an
/// initial-prompt bias whenever code-aware vocabulary is enabled — even with no
/// project folder selected, so the feature "just works" out of the box. Chosen for
/// terms Whisper tends to mangle (camelCase APIs, CLI names, abbreviations) rather
/// than words it already knows. A project scan, when configured, layers on top of
/// (and ranks ahead of) this list.
pub const BUILTIN_DEV_TERMS: &[&str] = &[
    // JS/TS framework + hooks
    "useEffect", "useState", "useRef", "useCallback", "useMemo", "useContext",
    "TypeScript", "JavaScript", "JSX", "TSX", "npm", "npx", "pnpm", "yarn",
    "Node.js", "Deno", "Vite", "Webpack", "ESLint", "Prettier", "Tailwind",
    "React", "Vue", "Svelte", "Next.js", "async", "await", "Promise", "nullable",
    // Rust
    "Rust", "cargo", "rustc", "clippy", "tokio", "serde", "async", "trait",
    "enum", "struct", "impl", "Mutex", "Arc", "borrow", "lifetime", "macro",
    "stdout", "stderr", "stdin", "dylib", "rustup", "Tauri", "whisper-rs",
    // Python
    "Python", "pip", "venv", "pytest", "numpy", "pandas", "asyncio", "dataclass",
    "Django", "Flask", "FastAPI", "PyTorch", "TensorFlow",
    // Go / other langs
    "Golang", "goroutine", "Kotlin", "Swift", "SwiftUI", "Xcode",
    // Web / protocols / data
    "API", "REST", "GraphQL", "JSON", "YAML", "TOML", "HTTP", "HTTPS", "WebSocket",
    "OAuth", "JWT", "CORS", "UUID", "regex", "stdin", "CRUD", "SQL",
    // Databases / infra / devops
    "Postgres", "PostgreSQL", "SQLite", "Redis", "MongoDB", "Docker", "Kubernetes",
    "kubectl", "nginx", "Terraform", "Ansible", "GitHub", "GitLab", "CI/CD",
    "DevOps", "Kafka", "RabbitMQ", "gRPC",
    // General CS / build
    "localhost", "config", "env", "boolean", "int", "struct", "endpoint",
    "middleware", "namespace", "runtime", "stack trace", "codebase", "repo",
    "commit", "rebase", "changelog", "metadata", "macOS", "Linux",
];

/// Skip individual files larger than this (bytes). A single huge minified bundle
/// or vendored blob shouldn't dominate the scan or hang it.
pub const MAX_FILE_BYTES: u64 = 512 * 1024;

/// Cap on the number of files we read in one scan, so a giant repo can't hang
/// the pipeline. Files are visited in directory-walk order until the cap is hit.
pub const MAX_FILES: usize = 1000;

/// Total bytes we will read across all files in one scan. A second guard on top
/// of the per-file and file-count caps.
pub const MAX_TOTAL_BYTES: u64 = 32 * 1024 * 1024;

/// Source file extensions we scan. Kept to common code formats so we don't pull
/// identifiers out of, say, lockfiles or binary assets.
pub const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "java", "kt",
    "swift", "c", "h", "cc", "cpp", "hpp", "cs", "rb", "php", "scala", "sh",
    "lua", "dart", "vue", "svelte",
];

/// Render [`BUILTIN_DEV_TERMS`] as a space-joined initial-prompt string, deduped
/// case-insensitively (the list carries intentional cross-language repeats like
/// "async"/"struct") while preserving the first surface form and order.
pub fn builtin_terms_prompt() -> String {
    let mut seen = std::collections::HashSet::new();
    BUILTIN_DEV_TERMS
        .iter()
        .filter(|t| seen.insert(t.to_ascii_lowercase()))
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
}

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

/// Project manifests are scanned only for their local package/script names.
/// Their dependency bodies are not treated as source text.
pub fn is_package_manifest(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("package.json"))
}

/// Extract command-relevant names from a package manifest. This is intentionally
/// narrow: package name, script keys, and dependency keys only. Script bodies
/// can contain arbitrary user text and are never retained as vocabulary.
pub fn extract_package_json_terms(source: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(source) else {
        return Vec::new();
    };
    let mut terms = Vec::new();
    if let Some(name) = value.get("name").and_then(|name| name.as_str()) {
        push_manifest_term(&mut terms, name);
    }
    for section in [
        "scripts",
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ] {
        if let Some(entries) = value.get(section).and_then(|entries| entries.as_object()) {
            for name in entries.keys() {
                push_manifest_term(&mut terms, name);
            }
        }
    }
    terms
}

fn push_manifest_term(terms: &mut Vec<String>, term: &str) {
    let term = term.trim();
    if !term.is_empty()
        && !term.chars().any(char::is_whitespace)
        && term
            .chars()
            .all(|ch| ch.is_alphanumeric() || "_-./:@".contains(ch))
    {
        terms.push(term.to_string());
    }
}

/// Directory names we never descend into while walking a project. These hold
/// dependencies, build output, or VCS data — not the user's own identifiers.
const SKIP_DIRS: &[&str] = &[
    "node_modules", "target", ".git", "dist", "build", ".next", "vendor",
    "__pycache__", ".venv", "venv", ".svn", ".hg", "Pods", "DerivedData",
    ".cargo", ".idea", ".vscode", "coverage", "out", "cache", "caches",
];

/// Return true if a directory with this name should not be descended into.
pub fn is_skipped_dir(name: &str) -> bool {
    SKIP_DIRS
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(name))
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

/// A ranked vocabulary term and the number of times it was seen across the scan.
/// `term` is the first surface form encountered (e.g. `useEffect`), `freq` its
/// total occurrence count. Returned by [`VocabAccumulator::ranked`] /
/// [`ranked_vocab_terms`] ordered by descending frequency, ties broken by
/// first-seen order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedTerm {
    pub term: String,
    pub freq: u32,
}

/// Streaming frequency accumulator for identifiers. Fold one file's contents in
/// at a time via [`VocabAccumulator::add_source`]; the file's `String` can be
/// dropped immediately after, so peak memory is bounded by the number of unique
/// terms (not the total bytes read). [`VocabAccumulator::ranked`] then produces a
/// deterministic descending-frequency ranking.
#[derive(Default)]
pub struct VocabAccumulator {
    /// lowercased key -> occurrence count.
    freq: HashMap<String, u32>,
    /// lowercased key -> first surface form seen (preserves original casing).
    surface: HashMap<String, String>,
    /// lowercased key -> first-seen ordinal (for stable tie-breaks).
    order: HashMap<String, usize>,
    next_order: usize,
}

impl VocabAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Extract identifiers from `source` and fold their counts into the running
    /// tallies. The caller may drop `source` immediately afterward — nothing from
    /// it is retained beyond the per-term bookkeeping above.
    pub fn add_source(&mut self, source: &str) {
        for ident in extract_identifiers(source) {
            self.add_identifier(&ident);
        }
    }

    /// Fold identifiers until `max_unique` distinct terms have been retained.
    /// Returns true when a new term was refused at the cap. Existing terms may
    /// still increment their frequency without growing memory.
    pub fn add_source_bounded(&mut self, source: &str, max_unique: usize) -> bool {
        for ident in extract_identifiers(source) {
            if self.add_identifier_bounded(&ident, max_unique) {
                return true;
            }
        }
        false
    }

    pub fn add_written_term(&mut self, term: &str) {
        self.add_identifier(term);
    }

    pub fn add_written_term_bounded(&mut self, term: &str, max_unique: usize) -> bool {
        self.add_identifier_bounded(term, max_unique)
    }

    fn add_identifier_bounded(&mut self, ident: &str, max_unique: usize) -> bool {
        let key = ident.to_ascii_lowercase();
        if !self.freq.contains_key(&key) && self.freq.len() >= max_unique {
            return true;
        }
        self.add_identifier(ident);
        false
    }

    /// Fold a single already-extracted identifier into the tallies.
    fn add_identifier(&mut self, ident: &str) {
        let key = ident.to_ascii_lowercase();
        *self.freq.entry(key.clone()).or_insert(0) += 1;
        self.surface.entry(key.clone()).or_insert_with(|| ident.to_string());
        let next = &mut self.next_order;
        self.order.entry(key).or_insert_with(|| {
            let o = *next;
            *next += 1;
            o
        });
    }

    /// Number of distinct (case-insensitive) terms accumulated so far. The walk
    /// hands this to the `on_file` callback so the live UI counter never has to
    /// re-extract identifiers from already-folded file contents.
    pub fn len(&self) -> usize {
        self.freq.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.freq.is_empty()
    }

    /// Produce the deterministic ranking: descending frequency, ties broken by
    /// ascending first-seen order, truncated to `max_terms`. `max_terms == 0`
    /// yields an empty vector.
    pub fn ranked(&self, max_terms: usize) -> Vec<RankedTerm> {
        if max_terms == 0 {
            return Vec::new();
        }
        let mut ranked: Vec<(&String, u32)> = self.freq.iter().map(|(k, v)| (k, *v)).collect();
        // Sort by descending frequency, then ascending first-seen order for stable ties.
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| self.order[a.0].cmp(&self.order[b.0])));
        ranked
            .into_iter()
            .take(max_terms)
            .map(|(k, freq)| RankedTerm {
                term: self.surface[k].clone(),
                freq,
            })
            .collect()
    }
}

/// Rank identifiers across in-memory file contents into `(term, freq)` pairs.
///
/// Pure: takes `(label, contents)` pairs (the label is only used for ordering
/// stability and is otherwise ignored), extracts identifiers from every file,
/// dedupes case-insensitively, ranks by descending frequency (ties broken by
/// first-seen order for determinism), and returns the top `max_terms`. The
/// joined-string [`build_vocab_prompt`] is a thin wrapper over this.
///
/// Retained as a pure, unit-tested reference for the ranking contract; the live
/// directory scan folds per-file via [`VocabAccumulator`] (which this delegates
/// to) so it never needs every file's contents resident at once.
#[allow(dead_code)]
pub fn ranked_vocab_terms<S: AsRef<str>>(files: &[(S, S)], max_terms: usize) -> Vec<RankedTerm> {
    let mut acc = VocabAccumulator::new();
    for (_label, contents) in files {
        acc.add_source(contents.as_ref());
    }
    acc.ranked(max_terms)
}

/// Build a Whisper initial-prompt string from in-memory file contents.
///
/// Pure: ranks identifiers via [`ranked_vocab_terms`] and joins the top
/// `max_terms` surface forms by spaces — directly usable as
/// `params.set_initial_prompt(...)`. Retained as the in-memory convenience entry
/// (and ranking-contract reference) used by tests; the live scan path joins via
/// [`ranked_terms_to_prompt`] on the streamed [`VocabAccumulator`] ranking.
#[allow(dead_code)]
pub fn build_vocab_prompt<S: AsRef<str>>(files: &[(S, S)], max_terms: usize) -> String {
    ranked_terms_to_prompt(&ranked_vocab_terms(files, max_terms))
}

/// Join a ranked term list into a space-separated Whisper initial prompt,
/// preserving rank order. Shared by [`build_vocab_prompt`] and the directory
/// scan so the prompt string and the ranked list never drift out of sync.
pub fn ranked_terms_to_prompt(ranked: &[RankedTerm]) -> String {
    ranked
        .iter()
        .map(|r| r.term.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// Thin (untested) filesystem wrapper. The pure functions above carry the logic;
// this just walks a directory, applies the size/count guards, and reads files.
// ---------------------------------------------------------------------------

/// Result of walking a project folder: the folded frequency [`VocabAccumulator`]
/// (identifiers ranked on demand — file contents are *not* retained, so memory is
/// bounded by unique-term count, not bytes read), how many files were read, how
/// many directories we declined to descend into, the total bytes read, and
/// whether the walk stopped *early* on a guard (MAX_FILES / MAX_TOTAL_BYTES)
/// before the tree was exhausted. `capped` is what the UI surfaces to explain a
/// partial scan; it is `false` when the walk simply ran dry.
pub struct ScanOutcome {
    pub vocab: VocabAccumulator,
    pub files_read: usize,
    pub dirs_skipped: usize,
    pub total_bytes: u64,
    pub capped: bool,
}

/// Walk `folder`, read up to the guarded number/size of source files, and build
/// a vocabulary prompt of at most `max_terms` identifiers. Unreadable files are
/// logged and skipped. Returns an empty string if nothing usable is found.
///
/// This function is deliberately not unit-tested (it touches the filesystem);
/// all of its non-trivial logic delegates to the pure functions above.
pub fn build_vocab_prompt_from_dir(folder: &Path, max_terms: usize) -> String {
    let outcome = collect_source_files(folder, |_, _| {}, |_| {});
    ranked_terms_to_prompt(&outcome.vocab.ranked(max_terms))
}

/// Iteratively walk `folder` **breadth-first** (FIFO queue, no recursion to
/// bound stack), honoring the file count, per-file size, total-byte, extension,
/// and skip-dir guards. Breadth-first so a parent folder (e.g. `~/code`) samples
/// fairly across its child projects instead of exhausting the file budget by
/// depth-diving the first subdirectory it finds.
///
/// `on_file` is invoked once per source file successfully read, with the file's
/// path and the running distinct-term count after that file was folded in (so a
/// caller can surface a live term tally *without* re-extracting identifiers — the
/// walk already folds each file into the accumulator). `on_skip` is invoked once
/// per dependency/build/hidden dir declined, with that dir's path. Callers use
/// both to surface a live scan stream (file rows and struck-through skip rows)
/// with running counts. Returns a [`ScanOutcome`] recording the files,
/// skipped-dir count, bytes read, and whether a guard cut the walk short
/// (`capped`).
pub fn collect_source_files<F: FnMut(&Path, usize), G: FnMut(&Path)>(
    folder: &Path,
    mut on_file: F,
    mut on_skip: G,
) -> ScanOutcome {
    // Fold each file's identifiers in as we read it, then drop the contents — so
    // peak memory is bounded by the unique-term count, not the bytes scanned.
    let mut vocab = VocabAccumulator::new();
    let mut files_read: usize = 0;
    let mut total_bytes: u64 = 0;
    let mut dirs_skipped: usize = 0;
    let mut capped = false;
    // FIFO queue (breadth-first): dirs are processed in the order discovered, so
    // the file budget spreads across sibling subtrees rather than the first one.
    let mut queue: std::collections::VecDeque<std::path::PathBuf> =
        std::collections::VecDeque::new();
    queue.push_back(folder.to_path_buf());

    'walk: while let Some(dir) = queue.pop_front() {
        // We just popped a directory but a guard blocks processing it, so this
        // subtree (and anything else still queued) goes un-indexed — a real
        // truncation. Mark `capped`. NOTE: when the tree is genuinely exhausted the
        // `while let` ends on an empty queue *without* reaching this break, so a scan
        // that read everything (even exactly MAX_FILES) still reports capped=false.
        if files_read >= MAX_FILES || total_bytes >= MAX_TOTAL_BYTES {
            capped = true;
            break;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                #[cfg(not(test))]
                tracing::warn!(
                    target: "pipeline",
                    error_kind = ?e.kind(),
                    "vocab: skipping unreadable directory"
                );
                let _ = e;
                continue;
            }
        };

        // Sort entries by name so repeat scans of the same tree are stable
        // (read_dir order is filesystem-dependent).
        let mut sorted: Vec<std::fs::DirEntry> = entries.flatten().collect();
        sorted.sort_by_key(|e| e.file_name());

        for entry in sorted {
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
                    dirs_skipped += 1;
                    on_skip(&path);
                    continue;
                }
                queue.push_back(path);
                continue;
            }

            if !file_type.is_file() || (!is_source_file(&path) && !is_package_manifest(&path)) {
                continue;
            }

            if files_read >= MAX_FILES || total_bytes >= MAX_TOTAL_BYTES {
                capped = true;
                break 'walk;
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
                    files_read += 1;
                    // Fold the file's identifiers into the accumulator and drop the
                    // contents — nothing keeps the `String` alive past this
                    // iteration. Then hand the callback the running distinct-term
                    // count so it never has to re-extract (which would double the
                    // tokenization CPU on a large walk).
                    if is_package_manifest(&path) {
                        for term in extract_package_json_terms(&contents) {
                            vocab.add_written_term(&term);
                        }
                    } else {
                        vocab.add_source(&contents);
                    }
                    on_file(&path, vocab.len());
                }
                Err(e) => {
                    #[cfg(not(test))]
                    tracing::warn!(
                        target: "pipeline",
                        error_kind = ?e.kind(),
                        "vocab: skipping unreadable file"
                    );
                    let _ = e;
                }
            }
        }
    }

    ScanOutcome { vocab, files_read, dirs_skipped, total_bytes, capped }
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
    fn package_manifest_extracts_only_command_relevant_names() {
        let terms = extract_package_json_terms(
            r#"{
                "name": "@acme/my-app",
                "scripts": {"tauri:dev": "cd /Users/private/CustomerProject && secret-setting", "test": "vitest"},
                "dependencies": {"ccusage": "1.0.0"},
                "devDependencies": {"create-vite": "latest"},
                "description": "private prose must not become vocabulary"
            }"#,
        );
        assert_eq!(
            terms,
            vec!["@acme/my-app", "tauri:dev", "test", "ccusage", "create-vite"]
        );
        assert!(!terms.iter().any(|term| term.contains("private")));
        assert!(!terms.iter().any(|term| term.contains("CustomerProject")));
        assert!(!terms.iter().any(|term| term.contains("secret-setting")));
    }

    #[test]
    fn malformed_package_manifest_is_ignored() {
        assert!(extract_package_json_terms("{not json").is_empty());
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

    // ---- Ranked (term, freq) pairs + budget split ----

    #[test]
    fn ranked_terms_returns_freq_and_rank_order() {
        // barBaz 3x, fooBar 1x => barBaz first with freq 3, fooBar second freq 1.
        let files = vec![("a.rs", "fooBar barBaz barBaz barBaz")];
        let ranked = ranked_vocab_terms(&files, 10);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0], RankedTerm { term: "barBaz".into(), freq: 3 });
        assert_eq!(ranked[1], RankedTerm { term: "fooBar".into(), freq: 1 });
    }

    #[test]
    fn ranked_terms_respects_max_and_zero() {
        let files = vec![("a.rs", "oneTwo threeFour fiveSix")];
        assert_eq!(ranked_vocab_terms(&files, 2).len(), 2);
        assert!(ranked_vocab_terms(&files, 0).is_empty());
    }

    #[test]
    fn ranked_terms_drives_prompt_string() {
        // build_vocab_prompt must equal the ranked list joined by spaces — the two
        // share ranked_terms_to_prompt so the prompt and the ranked list can't drift.
        let files = vec![("a.rs", "alphaOne betaTwo alphaOne")];
        let ranked = ranked_vocab_terms(&files, 10);
        assert_eq!(ranked_terms_to_prompt(&ranked), build_vocab_prompt(&files, 10));
        assert_eq!(ranked[0].term, "alphaOne", "ranked={:?}", ranked);
    }

    #[test]
    fn budget_split_top96_is_prefix_of_top500() {
        // The Whisper budget (96) is the rank-prefix of the correction budget (500):
        // both come from the same descending-frequency ranking, so the top 96 are
        // exactly the first 96 of the top 500. Synthesize >96 distinct terms with
        // strictly descending frequency so order is unambiguous.
        let mut src = String::new();
        for i in 0..150 {
            // term i repeats (150 - i) times => strictly descending frequency.
            for _ in 0..(150 - i) {
                src.push_str(&format!("term{:03}X ", i));
            }
        }
        let files = vec![("a.rs", src.as_str())];
        let top500 = ranked_vocab_terms(&files, 500);
        let top96 = ranked_vocab_terms(&files, 96);
        assert_eq!(top500.len(), 150, "all 150 distinct terms fit under 500");
        assert_eq!(top96.len(), 96, "Whisper budget truncates to 96");
        // top96 is a strict prefix of top500 (same ranking, just truncated).
        assert_eq!(&top500[..96], &top96[..]);
        // whisperCount semantics: min(96, kept) == 96 here.
        assert_eq!(top96.len().min(96), 96);
    }

    #[test]
    fn budget_split_whisper_count_clamps_to_kept() {
        // Fewer than 96 distinct terms => Whisper budget == kept count, not 96.
        let files = vec![("a.rs", "alphaOne betaTwo gammaThree")];
        let kept = ranked_vocab_terms(&files, 500);
        assert_eq!(kept.len(), 3);
        assert_eq!(kept.len().min(96), 3, "whisperCount clamps below 96");
    }

    // ---- Per-file streaming accumulator (bounded memory) ----

    #[test]
    fn accumulator_folds_per_file_without_retaining_contents() {
        // Fold three files one String at a time, dropping each before the next —
        // proving the ranking does NOT need all file contents resident at once.
        let mut acc = VocabAccumulator::new();
        {
            let f1 = String::from("fooBar barBaz");
            acc.add_source(&f1);
            // f1 dropped here; nothing in `acc` borrows it.
        }
        {
            let f2 = String::from("fooBar fooBar");
            acc.add_source(&f2);
        }
        {
            let f3 = String::from("quxQuux");
            acc.add_source(&f3);
        }
        // fooBar seen 3x total across files => ranks first.
        let ranked = acc.ranked(10);
        assert_eq!(ranked[0], RankedTerm { term: "fooBar".into(), freq: 3 });
        assert_eq!(acc.len(), 3, "fooBar, barBaz, quxQuux");
        // Folding files one at a time matches ranking them all together.
        let batch = ranked_vocab_terms(
            &[("a", "fooBar barBaz"), ("b", "fooBar fooBar"), ("c", "quxQuux")],
            10,
        );
        assert_eq!(ranked, batch, "streaming fold == batch ranking");
    }

    #[test]
    fn accumulator_empty_is_empty() {
        let acc = VocabAccumulator::new();
        assert!(acc.is_empty());
        assert_eq!(acc.len(), 0);
        assert!(acc.ranked(10).is_empty());
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
    fn builtin_prompt_is_nonempty_and_deduped() {
        let p = builtin_terms_prompt();
        assert!(p.contains("useEffect"));
        assert!(p.contains("kubectl"));
        // "async"/"struct" appear multiple times in the source list but must be
        // deduped (case-insensitively) in the rendered prompt.
        let count_async = p.split(' ').filter(|t| t.eq_ignore_ascii_case("async")).count();
        assert_eq!(count_async, 1, "async should appear once, prompt={:?}", p);
        let count_struct = p.split(' ').filter(|t| t.eq_ignore_ascii_case("struct")).count();
        assert_eq!(count_struct, 1, "struct should appear once");
    }

    #[test]
    fn guard_constants_are_sane() {
        // Sanity-check the guard values so an accidental edit to 0 is caught.
        assert!(MAX_FILE_BYTES > 0);
        assert!(MAX_FILES > 0);
        assert!(MAX_TOTAL_BYTES >= MAX_FILE_BYTES);
        assert!(!SOURCE_EXTENSIONS.is_empty());
        // Caps were raised to accommodate the decoupled correction budget (top
        // 500 terms) without scanning unbounded repos: 1000 files / 32MB total /
        // 512KB per file. Pin the new values so an accidental downgrade is caught.
        assert_eq!(MAX_FILES, 1000);
        assert_eq!(MAX_TOTAL_BYTES, 32 * 1024 * 1024);
        assert_eq!(MAX_FILE_BYTES, 512 * 1024);
    }

    // ---- Filesystem walk (breadth-first, guards, progress) ----

    /// Create a unique scratch dir under the OS temp dir for a walk test.
    fn scratch_dir(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!("murmur_vocab_test_{}_{}", tag, nanos));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn walk_reads_source_files_and_reports_progress() {
        let dir = scratch_dir("read");
        std::fs::write(dir.join("a.rs"), "let fooBar = barBaz;").unwrap();
        std::fs::write(dir.join("b.ts"), "const useWidget = 1;").unwrap();
        std::fs::write(dir.join("notes.md"), "ignored non-source").unwrap();

        let mut seen = 0usize;
        let mut last_term_count = 0usize;
        let outcome = collect_source_files(
            &dir,
            |_, terms_so_far| {
                seen += 1;
                last_term_count = terms_so_far;
            },
            |_| {},
        );
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(outcome.files_read, 2, "only the two source files");
        assert_eq!(seen, 2, "progress callback fired once per file read");
        // on_file receives the running distinct-term count (monotonically grows as
        // files fold in); by the last file it equals the accumulator's total.
        assert!(last_term_count > 0, "on_file receives a running term count");
        assert_eq!(last_term_count, outcome.vocab.len(), "final tally matches accumulator");
        assert!(!outcome.capped, "small tree should not be capped");
        assert!(outcome.total_bytes > 0);
        // The accumulator folded both files' identifiers (fooBar, barBaz, useWidget).
        assert!(outcome.vocab.len() >= 3, "got {} terms", outcome.vocab.len());
    }

    #[test]
    fn walk_adds_package_scripts_and_dependencies_to_project_vocabulary() {
        let dir = scratch_dir("package_json");
        std::fs::write(
            dir.join("package.json"),
            r#"{"scripts":{"tauri:dev":"tauri dev"},"dependencies":{"ccusage":"1"}}"#,
        )
        .unwrap();

        let outcome = collect_source_files(&dir, |_, _| {}, |_| {});
        let ranked = outcome.vocab.ranked(10);
        assert!(ranked.iter().any(|term| term.term == "tauri:dev"));
        assert!(ranked.iter().any(|term| term.term == "ccusage"));
        assert_eq!(outcome.files_read, 1);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn walk_skips_dependency_dirs_and_counts_them() {
        let dir = scratch_dir("skip");
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        std::fs::write(dir.join("node_modules").join("dep.js"), "var depThing = 1;").unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git").join("c.rs"), "let gitThing = 1;").unwrap();
        std::fs::write(dir.join("main.rs"), "let realThing = 1;").unwrap();

        let mut skipped: Vec<String> = Vec::new();
        let outcome = collect_source_files(&dir, |_, _| {}, |p| {
            skipped.push(p.file_name().unwrap().to_string_lossy().to_string());
        });
        std::fs::remove_dir_all(&dir).ok();

        // Only the top-level real source file; the two skip-dirs are not descended.
        assert_eq!(outcome.files_read, 1, "got {} files", outcome.files_read);
        assert_eq!(outcome.dirs_skipped, 2, "node_modules + .git both skipped");
        // on_skip fires once per declined dir, carrying its path.
        assert_eq!(skipped.len(), 2, "on_skip fired per skipped dir, got {:?}", skipped);
        assert!(skipped.contains(&"node_modules".to_string()), "got {:?}", skipped);
        assert!(skipped.contains(&".git".to_string()), "got {:?}", skipped);
    }

    #[test]
    fn walk_is_breadth_first_across_sibling_projects() {
        // Two sibling project dirs, each with a source file two levels deep. A
        // depth-first walk that exhausted a 1-file budget would only see one
        // project; breadth-first must touch the shallow files of both first.
        let dir = scratch_dir("bfs");
        for proj in ["projA", "projB"] {
            let src = dir.join(proj).join("src");
            std::fs::create_dir_all(&src).unwrap();
            std::fs::write(dir.join(proj).join(format!("{}Top.rs", proj)), "let topThing = 1;").unwrap();
            std::fs::write(src.join("deepThing.rs"), "let deepThing = 1;").unwrap();
        }

        let mut order: Vec<String> = Vec::new();
        let outcome = collect_source_files(&dir, |p, _| {
            order.push(p.file_name().unwrap().to_string_lossy().to_string());
        }, |_| {});
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(outcome.files_read, 4);
        // Both shallow (top-level-of-project) files must come before either deep
        // file — proof the walk is breadth-first, not depth-first.
        let pos_a_top = order.iter().position(|n| n == "projATop.rs").unwrap();
        let pos_b_top = order.iter().position(|n| n == "projBTop.rs").unwrap();
        let pos_deep = order.iter().position(|n| n == "deepThing.rs").unwrap();
        assert!(pos_a_top < pos_deep, "shallow projA file before deep, order={:?}", order);
        assert!(pos_b_top < pos_deep, "shallow projB file before deep, order={:?}", order);
    }

    #[test]
    fn walk_not_capped_when_tree_runs_dry() {
        let dir = scratch_dir("dry");
        std::fs::write(dir.join("only.rs"), "let oneThing = 1;").unwrap();
        let outcome = collect_source_files(&dir, |_, _| {}, |_| {});
        std::fs::remove_dir_all(&dir).ok();
        assert!(!outcome.capped, "exhausted tree under caps is not 'capped'");
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
