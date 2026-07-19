//! Explicit, local IDE project context.
//!
//! Users opt a per-app profile into one or more local project roots. Murmur
//! builds a short-lived, memory-only index of source identifiers and
//! root-relative filenames. No source text, symbol, filename, or absolute path
//! from this index is serialized or logged.

use crate::correction::{derive_spoken_form, CorrectionMatcher};
use crate::state::AppProfile;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) const MAX_IDE_ROOTS: usize = 4;
pub(crate) const MAX_IDE_FILES: usize = 1_000;
pub(crate) const MAX_IDE_TOTAL_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const MAX_IDE_FILE_BYTES: u64 = 512 * 1024;
pub(crate) const MAX_IDE_SYMBOLS: usize = 500;
const MAX_IDE_CANDIDATE_SYMBOLS: usize = 10_000;
const MAX_RELATIVE_PATH_BYTES: usize = 512;
const MAX_FORMAT_INPUT_BYTES: usize = 16 * 1024;
const INDEX_TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileAlias {
    tokens: Vec<String>,
    relative: String,
}

/// Immutable resources captured by one recording-start context snapshot.
pub(crate) struct IdeContextIndex {
    symbol_matcher: Arc<CorrectionMatcher>,
    file_aliases: Arc<Vec<FileAlias>>,
    built_at: Instant,
    valid: AtomicBool,
}

impl IdeContextIndex {
    fn is_fresh(&self) -> bool {
        self.built_at.elapsed() < INDEX_TTL
    }

    fn is_usable(&self) -> bool {
        self.valid.load(Ordering::Acquire) && self.is_fresh()
    }

    fn invalidate(&self) {
        self.valid.store(false, Ordering::Release);
    }

    /// Canonicalize explicit file mentions, then apply the unique project-symbol
    /// matcher. Both operations are deterministic and bounded.
    pub(crate) fn apply(&self, input: &str) -> String {
        if !self.is_usable() || input.is_empty() || input.len() > MAX_FORMAT_INPUT_BYTES {
            return input.to_string();
        }
        let with_files = apply_file_mentions(input, &self.file_aliases);
        let transformed = if self.symbol_matcher.is_empty() {
            with_files
        } else {
            self.symbol_matcher.apply(&with_files)
        };
        if self.is_usable() {
            transformed
        } else {
            input.to_string()
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct IdeIndexStats {
    pub roots: usize,
    pub files: usize,
    pub symbols: usize,
    pub bytes: u64,
    pub capped: bool,
    pub ms: u64,
}

pub(crate) struct IdeIndexBuild {
    pub index: Arc<IdeContextIndex>,
    pub stats: IdeIndexStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryState {
    Disabled,
    Empty,
    Scanning,
    Ready,
    Cleared,
    Error,
}

impl EntryState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Empty => "empty",
            Self::Scanning => "scanning",
            Self::Ready => "ready",
            Self::Cleared => "cleared",
            Self::Error => "error",
        }
    }
}

struct ProfileIndexEntry {
    enabled: bool,
    roots: Vec<String>,
    generation: u64,
    state: EntryState,
    index: Option<Arc<IdeContextIndex>>,
    stats: IdeIndexStats,
}

impl ProfileIndexEntry {
    fn disabled(generation: u64) -> Self {
        Self {
            enabled: false,
            roots: Vec::new(),
            generation,
            state: EntryState::Disabled,
            index: None,
            stats: IdeIndexStats::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct IdeScanRequest {
    pub bundle_id: String,
    pub roots: Vec<String>,
    pub generation: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IdeContextStatus {
    pub state: String,
    pub generation: u64,
    pub roots: usize,
    pub files: usize,
    pub symbols: usize,
    pub bytes: u64,
    pub capped: bool,
    pub ms: u64,
}

/// Memory-only store keyed by the explicitly configured profile bundle id.
#[derive(Default)]
pub(crate) struct IdeContextStore {
    next_generation: u64,
    entries: HashMap<String, ProfileIndexEntry>,
}

impl IdeContextStore {
    fn next_generation(&mut self) -> u64 {
        self.next_generation = self.next_generation.saturating_add(1);
        self.next_generation
    }

    fn invalidate_current(&self, bundle_id: &str) {
        if let Some(index) = self
            .entries
            .get(bundle_id)
            .and_then(|entry| entry.index.as_ref())
        {
            index.invalidate();
        }
    }

    /// Reconcile the store with the first profile for each bundle id. A root or
    /// opt-in change invalidates the previous index immediately and returns a
    /// fresh scan request. Unchanged cleared/error states remain explicit until
    /// the user refreshes or changes configuration.
    pub(crate) fn reconcile_profiles(&mut self, profiles: &[AppProfile]) -> Vec<IdeScanRequest> {
        let mut configs: HashMap<String, (bool, Vec<String>)> = HashMap::new();
        for profile in profiles {
            configs.entry(profile.bundle_id.clone()).or_insert_with(|| {
                (
                    profile.ide_context_enabled,
                    normalize_config_roots(&profile.ide_project_roots),
                )
            });
        }

        let existing_ids = self.entries.keys().cloned().collect::<Vec<_>>();
        for bundle_id in existing_ids {
            if !configs.contains_key(&bundle_id) {
                self.invalidate_current(&bundle_id);
                let generation = self.next_generation();
                self.entries
                    .insert(bundle_id, ProfileIndexEntry::disabled(generation));
            }
        }

        let mut requests = Vec::new();
        for (bundle_id, (enabled, roots)) in configs {
            let changed = self
                .entries
                .get(&bundle_id)
                .is_none_or(|entry| entry.enabled != enabled || entry.roots != roots);
            if !changed {
                continue;
            }
            self.invalidate_current(&bundle_id);
            let generation = self.next_generation();
            let state = if !enabled {
                EntryState::Disabled
            } else if roots.is_empty() {
                EntryState::Empty
            } else {
                EntryState::Scanning
            };
            self.entries.insert(
                bundle_id.clone(),
                ProfileIndexEntry {
                    enabled,
                    roots: roots.clone(),
                    generation,
                    state,
                    index: None,
                    stats: IdeIndexStats {
                        roots: roots.len(),
                        ..IdeIndexStats::default()
                    },
                },
            );
            if state == EntryState::Scanning {
                requests.push(IdeScanRequest {
                    bundle_id,
                    roots,
                    generation,
                });
            }
        }
        requests
    }

    pub(crate) fn begin_refresh(
        &mut self,
        bundle_id: &str,
        roots: &[String],
    ) -> Option<IdeScanRequest> {
        let roots = normalize_config_roots(roots);
        self.invalidate_current(bundle_id);
        if roots.is_empty() {
            let generation = self.next_generation();
            self.entries.insert(
                bundle_id.to_string(),
                ProfileIndexEntry {
                    enabled: true,
                    roots,
                    generation,
                    state: EntryState::Empty,
                    index: None,
                    stats: IdeIndexStats::default(),
                },
            );
            return None;
        }
        let generation = self.next_generation();
        self.entries.insert(
            bundle_id.to_string(),
            ProfileIndexEntry {
                enabled: true,
                roots: roots.clone(),
                generation,
                state: EntryState::Scanning,
                index: None,
                stats: IdeIndexStats {
                    roots: roots.len(),
                    ..IdeIndexStats::default()
                },
            },
        );
        Some(IdeScanRequest {
            bundle_id: bundle_id.to_string(),
            roots,
            generation,
        })
    }

    /// Start a refresh only when a ready index has expired. The expired index is
    /// revoked before the request is returned, so no recording can use stale
    /// symbols, including one that already captured the previous generation.
    pub(crate) fn refresh_if_expired(
        &mut self,
        bundle_id: &str,
        roots: &[String],
    ) -> Option<IdeScanRequest> {
        let expired = self
            .entries
            .get(bundle_id)
            .and_then(|entry| entry.index.as_ref())
            .is_some_and(|index| !index.is_usable());
        expired
            .then(|| self.begin_refresh(bundle_id, roots))
            .flatten()
    }

    pub(crate) fn clear(&mut self, bundle_id: &str, roots: &[String]) -> u64 {
        self.invalidate_current(bundle_id);
        let generation = self.next_generation();
        let roots = normalize_config_roots(roots);
        self.entries.insert(
            bundle_id.to_string(),
            ProfileIndexEntry {
                enabled: true,
                roots: roots.clone(),
                generation,
                state: EntryState::Cleared,
                index: None,
                stats: IdeIndexStats {
                    roots: roots.len(),
                    ..IdeIndexStats::default()
                },
            },
        );
        generation
    }

    pub(crate) fn complete(
        &mut self,
        request: &IdeScanRequest,
        result: Result<IdeIndexBuild, &'static str>,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(&request.bundle_id) else {
            return false;
        };
        if entry.generation != request.generation
            || !entry.enabled
            || entry.roots != request.roots
            || entry.state != EntryState::Scanning
        {
            return false;
        }
        match result {
            Ok(build) => {
                entry.stats = build.stats;
                entry.index = Some(build.index);
                entry.state = EntryState::Ready;
            }
            Err(_) => {
                entry.index = None;
                entry.state = EntryState::Error;
            }
        }
        true
    }

    pub(crate) fn snapshot(
        &self,
        bundle_id: &str,
        roots: &[String],
    ) -> Option<Arc<IdeContextIndex>> {
        let entry = self.entries.get(bundle_id)?;
        if !entry.enabled
            || entry.roots != normalize_config_roots(roots)
            || entry.state != EntryState::Ready
        {
            return None;
        }
        entry
            .index
            .as_ref()
            .filter(|index| index.is_usable())
            .cloned()
    }

    pub(crate) fn status(&self, bundle_id: &str) -> IdeContextStatus {
        let Some(entry) = self.entries.get(bundle_id) else {
            return IdeContextStatus {
                state: EntryState::Disabled.as_str().to_string(),
                generation: 0,
                roots: 0,
                files: 0,
                symbols: 0,
                bytes: 0,
                capped: false,
                ms: 0,
            };
        };
        let expired = entry.index.as_ref().is_some_and(|index| !index.is_usable());
        IdeContextStatus {
            state: if expired {
                "stale".to_string()
            } else {
                entry.state.as_str().to_string()
            },
            generation: entry.generation,
            roots: entry.stats.roots,
            files: entry.stats.files,
            symbols: entry.stats.symbols,
            bytes: entry.stats.bytes,
            capped: entry.stats.capped,
            ms: entry.stats.ms,
        }
    }
}

pub(crate) fn normalize_config_roots(roots: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    roots
        .iter()
        .map(|root| root.trim())
        .filter(|root| !root.is_empty())
        .filter(|root| seen.insert((*root).to_string()))
        .take(MAX_IDE_ROOTS)
        .map(str::to_string)
        .collect()
}

/// Build one generation. Roots are canonicalized exactly once, then the walk
/// uses lexical descendants returned by `read_dir` and refuses all symlinks and
/// non-regular files before reading.
pub(crate) fn build_index(
    _generation: u64,
    configured_roots: &[String],
) -> Result<IdeIndexBuild, &'static str> {
    let started = Instant::now();
    let mut canonical_roots = Vec::new();
    let mut seen = HashSet::new();
    for configured in normalize_config_roots(configured_roots) {
        let path = Path::new(&configured);
        if !path.is_absolute() {
            continue;
        }
        let Ok(canonical) = std::fs::canonicalize(path) else {
            continue;
        };
        let Ok(metadata) = std::fs::symlink_metadata(&canonical) else {
            continue;
        };
        if !metadata.file_type().is_dir() || !seen.insert(canonical.clone()) {
            continue;
        }
        canonical_roots.push(canonical);
    }
    if canonical_roots.is_empty() {
        return Err("no_valid_roots");
    }
    canonical_roots.sort();

    let mut queue: VecDeque<(usize, PathBuf)> = canonical_roots
        .iter()
        .enumerate()
        .map(|(index, root)| (index, root.clone()))
        .collect();
    let mut vocab = crate::vocab::VocabAccumulator::new();
    let mut indexed_files = Vec::new();
    let mut files = 0usize;
    let mut bytes = 0u64;
    let mut capped = false;

    'walk: while let Some((root_index, dir)) = queue.pop_front() {
        if files >= MAX_IDE_FILES || bytes >= MAX_IDE_TOTAL_BYTES {
            capped = true;
            break;
        }
        let root = &canonical_roots[root_index];
        if !dir.starts_with(root) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries = entries.flatten().collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name();
            let name_lossy = name.to_string_lossy();
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if name_lossy.starts_with('.') || crate::vocab::is_skipped_dir(&name_lossy) {
                    continue;
                }
                if path.starts_with(root) {
                    queue.push_back((root_index, path));
                }
                continue;
            }
            if !file_type.is_file()
                || name_lossy.starts_with('.')
                || (!crate::vocab::is_source_file(&path)
                    && !crate::vocab::is_package_manifest(&path))
            {
                continue;
            }
            if files >= MAX_IDE_FILES || bytes >= MAX_IDE_TOTAL_BYTES {
                capped = true;
                break 'walk;
            }
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if !metadata.file_type().is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() > MAX_IDE_FILE_BYTES
                || bytes.saturating_add(metadata.len()) > MAX_IDE_TOTAL_BYTES
            {
                continue;
            }
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            let Some(relative) = relative_path_text(relative) else {
                continue;
            };
            let Ok(contents) = std::fs::read_to_string(&path) else {
                continue;
            };
            files += 1;
            bytes = bytes.saturating_add(contents.len() as u64);
            indexed_files.push(relative);
            let symbol_cap_reached = if crate::vocab::is_package_manifest(&path) {
                let mut reached = false;
                for term in crate::vocab::extract_package_json_terms(&contents) {
                    if vocab.add_written_term_bounded(&term, MAX_IDE_CANDIDATE_SYMBOLS) {
                        reached = true;
                        break;
                    }
                }
                reached
            } else {
                vocab.add_source_bounded(&contents, MAX_IDE_CANDIDATE_SYMBOLS)
            };
            if symbol_cap_reached {
                capped = true;
                break 'walk;
            }
        }
    }

    // Resolve spoken-form ambiguity across the complete bounded scan before
    // selecting the top terms. Otherwise a lower-ranked collision just beyond
    // the output cap could make a top-ranked symbol look falsely unique.
    let ranked = vocab.ranked(usize::MAX);
    let unique_symbols = unique_spoken_symbols(&ranked)
        .into_iter()
        .take(MAX_IDE_SYMBOLS)
        .collect::<Vec<_>>();
    let symbol_matcher = Arc::new(CorrectionMatcher::build(&unique_symbols, &[], false, false));
    let file_aliases = Arc::new(build_file_aliases(&indexed_files));
    let stats = IdeIndexStats {
        roots: canonical_roots.len(),
        files,
        symbols: unique_symbols.len(),
        bytes,
        capped,
        ms: started.elapsed().as_millis() as u64,
    };
    Ok(IdeIndexBuild {
        index: Arc::new(IdeContextIndex {
            symbol_matcher,
            file_aliases,
            built_at: Instant::now(),
            valid: AtomicBool::new(true),
        }),
        stats,
    })
}

fn relative_path_text(path: &Path) -> Option<String> {
    let text = path.to_str()?.replace(std::path::MAIN_SEPARATOR, "/");
    if text.is_empty()
        || text.len() > MAX_RELATIVE_PATH_BYTES
        || text.starts_with('/')
        || text
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        None
    } else {
        Some(text)
    }
}

fn unique_spoken_symbols(ranked: &[crate::vocab::RankedTerm]) -> Vec<String> {
    let mut by_spoken: HashMap<String, Vec<String>> = HashMap::new();
    for term in ranked {
        let spoken = derive_spoken_form(&term.term);
        let values = by_spoken.entry(spoken).or_default();
        if !values
            .iter()
            .any(|value| value.eq_ignore_ascii_case(&term.term))
        {
            values.push(term.term.clone());
        }
    }
    ranked
        .iter()
        .filter(|term| {
            let spoken = derive_spoken_form(&term.term);
            by_spoken
                .get(&spoken)
                .is_some_and(|values| values.len() == 1)
        })
        .map(|term| term.term.clone())
        .collect()
}

fn build_file_aliases(files: &[String]) -> Vec<FileAlias> {
    let mut candidates: HashMap<Vec<String>, Vec<(usize, String)>> = HashMap::new();
    for (file_id, relative) in files.iter().enumerate() {
        let basename = relative.rsplit('/').next().unwrap_or(relative);
        for tokens in [
            vec![relative.to_ascii_lowercase()],
            file_spoken_tokens(relative),
            vec![basename.to_ascii_lowercase()],
            file_spoken_tokens(basename),
        ] {
            if tokens.is_empty() {
                continue;
            }
            let entries = candidates.entry(tokens).or_default();
            if !entries.iter().any(|(id, _)| *id == file_id) {
                entries.push((file_id, relative.clone()));
            }
        }
    }
    let mut aliases = candidates
        .into_iter()
        .filter_map(|(tokens, candidates)| {
            (candidates.len() == 1).then(|| FileAlias {
                tokens,
                relative: candidates[0].1.clone(),
            })
        })
        .collect::<Vec<_>>();
    aliases.sort_by(|left, right| {
        right.tokens.len().cmp(&left.tokens.len()).then_with(|| {
            right
                .tokens
                .join(" ")
                .len()
                .cmp(&left.tokens.join(" ").len())
        })
    });
    aliases
}

fn file_spoken_tokens(relative: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for (component_index, component) in relative.split('/').enumerate() {
        if component_index > 0 {
            tokens.push("slash".to_string());
        }
        for (part_index, part) in component.split('.').enumerate() {
            if part_index > 0 {
                tokens.push("dot".to_string());
            }
            tokens.extend(
                derive_spoken_form(part)
                    .split_whitespace()
                    .map(str::to_string),
            );
        }
    }
    tokens
}

#[derive(Debug)]
struct InputToken {
    lower: String,
    start: usize,
    end: usize,
}

fn tokenize_reference_input(input: &str) -> Vec<InputToken> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let is_token = |byte: u8| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/' | b'@')
        };
        if !is_token(bytes[index]) {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        while index < bytes.len() && is_token(bytes[index]) {
            index += 1;
        }
        tokens.push(InputToken {
            lower: input[start..index].to_ascii_lowercase(),
            start,
            end: index,
        });
    }
    tokens
}

fn apply_file_mentions(input: &str, aliases: &[FileAlias]) -> String {
    let tokens = tokenize_reference_input(input);
    let mut replacements = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if tokens[index].lower != "mention" {
            index += 1;
            continue;
        }
        let candidate_start = index + 1;
        let matched = aliases.iter().find(|alias| {
            let end = candidate_start + alias.tokens.len();
            if end > tokens.len() {
                return false;
            }
            if input[tokens[index].end..tokens[end - 1].start]
                .bytes()
                .any(|byte| matches!(byte, b'\n' | b'\r'))
            {
                return false;
            }
            tokens[candidate_start..end]
                .iter()
                .map(|token| token.lower.as_str())
                .eq(alias.tokens.iter().map(String::as_str))
        });
        if let Some(alias) = matched {
            let end_index = candidate_start + alias.tokens.len() - 1;
            replacements.push((
                tokens[index].start,
                tokens[end_index].end,
                format!("@{}", alias.relative),
            ));
            index = end_index + 1;
        } else {
            index += 1;
        }
    }
    if replacements.is_empty() {
        return input.to_string();
    }
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    for (start, end, replacement) in replacements {
        output.push_str(&input[cursor..start]);
        output.push_str(&replacement);
        cursor = end;
    }
    output.push_str(&input[cursor..]);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "murmur-ide-context-{tag}-{}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn profile(bundle_id: &str, enabled: bool, roots: Vec<String>) -> AppProfile {
        AppProfile {
            bundle_id: bundle_id.to_string(),
            label: String::new(),
            auto_paste_override: None,
            cleanup_override: None,
            cli_formatting_override: None,
            smart_formatting_override: None,
            writing_style: None,
            ide_context_enabled: enabled,
            ide_project_roots: roots,
        }
    }

    #[test]
    fn builds_memory_index_and_formats_symbols_and_unique_files() {
        let root = scratch_dir("basic");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/recording.rs"),
            "fn useRecordingState() { useRecordingState(); }",
        )
        .unwrap();
        let build = build_index(7, &[root.to_string_lossy().to_string()]).unwrap();
        assert_eq!(
            build.index.apply("use recording state"),
            "useRecordingState"
        );
        assert_eq!(
            build.index.apply("mention recording dot rs"),
            "@src/recording.rs"
        );
        assert_eq!(
            build.index.apply("mention src slash recording dot rs"),
            "@src/recording.rs"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn duplicate_basenames_and_relative_paths_never_guess() {
        let first = scratch_dir("ambiguous-a");
        let second = scratch_dir("ambiguous-b");
        std::fs::create_dir_all(first.join("src")).unwrap();
        std::fs::create_dir_all(second.join("src")).unwrap();
        std::fs::write(first.join("src/main.rs"), "fn firstSymbol() {}").unwrap();
        std::fs::write(second.join("src/main.rs"), "fn secondSymbol() {}").unwrap();
        let build = build_index(
            1,
            &[
                first.to_string_lossy().to_string(),
                second.to_string_lossy().to_string(),
            ],
        )
        .unwrap();
        assert_eq!(
            build.index.apply("mention main dot rs"),
            "mention main dot rs"
        );
        assert_eq!(
            build.index.apply("mention src slash main dot rs"),
            "mention src slash main dot rs"
        );
        let _ = std::fs::remove_dir_all(first);
        let _ = std::fs::remove_dir_all(second);
    }

    #[cfg(unix)]
    #[test]
    fn walk_refuses_symlinked_files_and_directories_outside_root() {
        use std::os::unix::fs::symlink;
        let root = scratch_dir("symlink-root");
        let outside = scratch_dir("symlink-outside");
        std::fs::write(outside.join("secret.rs"), "fn leakedSecretSymbol() {}").unwrap();
        symlink(outside.join("secret.rs"), root.join("linked.rs")).unwrap();
        symlink(&outside, root.join("linked-dir")).unwrap();
        std::fs::write(root.join("local.rs"), "fn localProjectSymbol() {}").unwrap();
        let build = build_index(1, &[root.to_string_lossy().to_string()]).unwrap();
        assert_eq!(build.stats.files, 1);
        assert_eq!(
            build.index.apply("leaked secret symbol"),
            "leaked secret symbol"
        );
        assert_eq!(
            build.index.apply("mention linked dot rs"),
            "mention linked dot rs"
        );
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[test]
    fn walk_excludes_hidden_dependency_build_cache_and_disallowed_files() {
        let root = scratch_dir("excluded");
        for directory in [
            ".hidden",
            "node_modules",
            "vendor",
            "target",
            "build",
            "cache",
        ] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
            std::fs::write(
                root.join(directory).join("secret.rs"),
                "fn excludedProjectSecret() {}",
            )
            .unwrap();
        }
        std::fs::write(root.join("notes.txt"), "fn textFileSecret() {}").unwrap();
        std::fs::write(root.join("visible.rs"), "fn visibleProjectSymbol() {}").unwrap();
        #[cfg(unix)]
        let _socket = std::os::unix::net::UnixListener::bind(root.join("device.rs")).unwrap();

        let build = build_index(1, &[root.to_string_lossy().to_string()]).unwrap();
        assert_eq!(build.stats.files, 1);
        assert_eq!(
            build.index.apply("excluded project secret"),
            "excluded project secret"
        );
        assert_eq!(build.index.apply("text file secret"), "text file secret");
        assert_eq!(
            build.index.apply("visible project symbol"),
            "visibleProjectSymbol"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn file_formatting_is_explicit_false_positive_safe_and_idempotent() {
        let aliases = build_file_aliases(&["src/recording.rs".to_string()]);
        assert_eq!(
            apply_file_mentions("recording dot rs is useful", &aliases),
            "recording dot rs is useful"
        );
        assert_eq!(
            apply_file_mentions("please mention recording dot rs, thanks", &aliases),
            "please @src/recording.rs, thanks"
        );
        assert_eq!(
            apply_file_mentions("@src/recording.rs", &aliases),
            "@src/recording.rs"
        );
        assert_eq!(
            apply_file_mentions("mention recording\ndot rs", &aliases),
            "mention recording\ndot rs"
        );
    }

    #[test]
    fn symbol_ambiguity_is_resolved_before_the_output_cap() {
        let mut ranked = vec![crate::vocab::RankedTerm {
            term: "fooBar".to_string(),
            freq: 100,
        }];
        ranked.extend((0..MAX_IDE_SYMBOLS).map(|index| crate::vocab::RankedTerm {
            term: format!("uniqueSymbol{index}"),
            freq: 1,
        }));
        ranked.push(crate::vocab::RankedTerm {
            term: "foo_bar".to_string(),
            freq: 1,
        });

        let selected = unique_spoken_symbols(&ranked)
            .into_iter()
            .take(MAX_IDE_SYMBOLS)
            .collect::<Vec<_>>();
        assert!(!selected.iter().any(|term| term == "fooBar"));
        assert!(!selected.iter().any(|term| term == "foo_bar"));
    }

    #[test]
    fn root_change_clear_and_expiry_invalidate_generations() {
        let root = scratch_dir("generation");
        std::fs::write(root.join("main.rs"), "fn currentSymbol() {}").unwrap();
        let roots = vec![root.to_string_lossy().to_string()];
        let mut store = IdeContextStore::default();
        let request = store
            .reconcile_profiles(&[profile("com.example.Editor", true, roots.clone())])
            .remove(0);
        let build = build_index(request.generation, &request.roots).unwrap();
        assert!(store.complete(&request, Ok(build)));
        let captured = store.snapshot("com.example.Editor", &roots).unwrap();
        assert_eq!(captured.apply("current symbol"), "currentSymbol");

        store.clear("com.example.Editor", &roots);
        assert!(store.snapshot("com.example.Editor", &roots).is_none());
        assert_eq!(captured.apply("current symbol"), "current symbol");
        assert!(!store.complete(&request, build_index(request.generation, &request.roots)));

        let refreshed = store.begin_refresh("com.example.Editor", &roots).unwrap();
        let build = build_index(refreshed.generation, &refreshed.roots).unwrap();
        assert!(store.complete(&refreshed, Ok(build)));
        store
            .entries
            .get_mut("com.example.Editor")
            .unwrap()
            .index
            .as_mut()
            .unwrap();
        let entry = store.entries.get_mut("com.example.Editor").unwrap();
        let index = Arc::get_mut(entry.index.as_mut().unwrap()).unwrap();
        index.built_at = Instant::now() - INDEX_TTL - Duration::from_secs(1);
        assert!(store.snapshot("com.example.Editor", &roots).is_none());
        assert!(store
            .refresh_if_expired("com.example.Editor", &roots)
            .is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn resource_limits_are_explicit_and_bounded() {
        assert_eq!(MAX_IDE_ROOTS, 4);
        assert_eq!(MAX_IDE_FILES, 1_000);
        assert_eq!(MAX_IDE_TOTAL_BYTES, 32 * 1024 * 1024);
        assert_eq!(MAX_IDE_FILE_BYTES, 512 * 1024);
        assert_eq!(MAX_IDE_SYMBOLS, 500);
        assert_eq!(MAX_IDE_CANDIDATE_SYMBOLS, 10_000);
        assert_eq!(MAX_RELATIVE_PATH_BYTES, 512);
        assert_eq!(MAX_FORMAT_INPUT_BYTES, 16 * 1024);
        let mut bounded_vocab = crate::vocab::VocabAccumulator::new();
        assert!(bounded_vocab.add_source_bounded("firstSymbol secondSymbol thirdSymbol", 2));
        assert_eq!(bounded_vocab.len(), 2);
        assert_eq!(
            normalize_config_roots(&[
                " /a ".to_string(),
                "/a".to_string(),
                "/b".to_string(),
                "/c".to_string(),
                "/d".to_string(),
                "/e".to_string(),
            ]),
            vec!["/a", "/b", "/c", "/d"]
        );
    }
}
