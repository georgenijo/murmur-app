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
//! The scan sees **production code only**: `#[cfg(test)]` items are removed and
//! files that are test-only modules (`#[cfg(test)] mod x;`, with or without
//! `#[path]`) are skipped. The per-function call-site checks parse real Rust
//! syntax (`syn`) and name each caller by its qualified identity (module path,
//! `impl` type, fn names), so same-named functions in sibling modules or impls
//! differ; the textual checks blank comments and literals first.

use regex::Regex;
use std::collections::HashSet;
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
/// (module path, `impl` type, then fn names, joined by `::`). The sum is
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
        "CancelPorts::terminal",
        1,
    ),
    (
        "commands/recording/lifecycle_ports.rs",
        "LifecyclePorts::terminal",
        1,
    ),
    (
        "commands/recording/start_ports.rs",
        "StartPorts::terminal",
        1,
    ),
    ("commands/recording/stop_ports.rs", "StopPorts::terminal", 1),
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

/// Every `src/**/*.rs` file as `(path relative to src, production code)`, with
/// comments and literals blanked and `#[cfg(test)]` items removed.
fn production_sources() -> Vec<(String, String)> {
    raw_production_sources()
        .into_iter()
        .map(|(relative, text)| (relative, production_code(&text)))
        .collect()
}

/// Every non-test-module `src/**/*.rs` file as `(path relative to src, raw source)`,
/// for the syntax-aware scanner (it skips `#[cfg(test)]` items itself).
fn raw_production_sources() -> Vec<(String, String)> {
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
            (relative, text)
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

// ------------------------------------------------ syntax-aware call-site scanner

/// Where a call lives: the qualified identity of its enclosing function, built from
/// real Rust syntax: inline module path, the `impl` / `trait` type, then the `fn`
/// names (`a::b::Type::method`, nested functions append). A call outside any
/// function body (a `const` / `static` initializer) is `<const NAME>` / `<static
/// NAME>` under its module path. Two functions with the same name in different
/// modules or impls have different identities, and a closure belongs to the
/// function that contains it.
struct SiteFinder<'m> {
    scope: Vec<String>,
    include_cfg_test: bool,
    is_target: &'m dyn Fn(&[String]) -> bool,
    hits: Vec<String>,
}

impl SiteFinder<'_> {
    fn hit(&mut self) {
        self.hits.push(self.scope.join("::"));
    }

    /// Calls written inside a macro invocation's tokens (`name(..)` or `a::b(..)`):
    /// the macro's arguments are not parsed as expressions.
    fn scan_tokens(&mut self, tokens: proc_macro2::TokenStream) {
        use proc_macro2::TokenTree;
        let mut chain: Vec<String> = Vec::new();
        for token in tokens {
            match token {
                TokenTree::Ident(ident) => chain.push(ident.to_string()),
                TokenTree::Punct(punct) if punct.as_char() == ':' => {}
                TokenTree::Group(group) => {
                    if group.delimiter() == proc_macro2::Delimiter::Parenthesis
                        && !chain.is_empty()
                        && (self.is_target)(&chain)
                    {
                        self.hit();
                    }
                    chain.clear();
                    self.scan_tokens(group.stream());
                }
                _ => chain.clear(),
            }
        }
    }

    fn skip(&self, attrs: &[syn::Attribute]) -> bool {
        !self.include_cfg_test && attrs.iter().any(is_cfg_test)
    }

    fn within(&mut self, segment: String, walk: impl FnOnce(&mut Self)) {
        self.scope.push(segment);
        walk(self);
        self.scope.pop();
    }
}

/// `#[cfg(test)]` (also `cfg(all(test, ..))`; `cfg(not(test))` is production).
fn is_cfg_test(attr: &syn::Attribute) -> bool {
    if !attr.path().is_ident("cfg") {
        return false;
    }
    let syn::Meta::List(list) = &attr.meta else {
        return false;
    };
    let text = list.tokens.to_string();
    let words: Vec<&str> = text
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .collect();
    words.contains(&"test") && !words.contains(&"not")
}

fn type_name(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(path) => path
            .path
            .segments
            .last()
            .map_or_else(|| "_".to_string(), |segment| segment.ident.to_string()),
        syn::Type::Reference(reference) => type_name(&reference.elem),
        _ => "_".to_string(),
    }
}

impl<'ast> syn::visit::Visit<'ast> for SiteFinder<'_> {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if self.skip(&item.attrs) {
            return;
        }
        self.within(item.ident.to_string(), |this| {
            syn::visit::visit_item_mod(this, item)
        });
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if self.skip(&item.attrs) {
            return;
        }
        self.within(type_name(&item.self_ty), |this| {
            syn::visit::visit_item_impl(this, item)
        });
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        if self.skip(&item.attrs) {
            return;
        }
        self.within(item.ident.to_string(), |this| {
            syn::visit::visit_item_trait(this, item)
        });
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if self.skip(&item.attrs) {
            return;
        }
        self.within(item.sig.ident.to_string(), |this| {
            syn::visit::visit_item_fn(this, item)
        });
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if self.skip(&item.attrs) {
            return;
        }
        self.within(item.sig.ident.to_string(), |this| {
            syn::visit::visit_impl_item_fn(this, item)
        });
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        if self.skip(&item.attrs) {
            return;
        }
        self.within(item.sig.ident.to_string(), |this| {
            syn::visit::visit_trait_item_fn(this, item)
        });
    }

    fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
        if self.skip(&item.attrs) {
            return;
        }
        self.within(format!("<const {}>", item.ident), |this| {
            syn::visit::visit_item_const(this, item)
        });
    }

    fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
        if self.skip(&item.attrs) {
            return;
        }
        self.within(format!("<static {}>", item.ident), |this| {
            syn::visit::visit_item_static(this, item)
        });
    }

    fn visit_impl_item_const(&mut self, item: &'ast syn::ImplItemConst) {
        if self.skip(&item.attrs) {
            return;
        }
        self.within(format!("<const {}>", item.ident), |this| {
            syn::visit::visit_impl_item_const(this, item)
        });
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*call.func {
            let segments: Vec<String> = path
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect();
            if (self.is_target)(&segments) {
                self.hit();
            }
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if (self.is_target)(&[call.method.to_string()]) {
            self.hit();
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        self.scan_tokens(mac.tokens.clone());
        syn::visit::visit_macro(self, mac);
    }
}

/// The enclosing-function identity of every call `is_target` accepts in `source`
/// (parsed as a Rust file; one entry per call).
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
    };
    finder.visit_file(&file);
    finder.hits
}

/// `emit_dictation_terminal*(` (a call, a method call, or inside a macro's tokens).
fn is_terminal_emit(segments: &[String]) -> bool {
    segments
        .last()
        .is_some_and(|name| name.starts_with("emit_dictation_terminal"))
}

/// `PersistedCore::new(`.
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
