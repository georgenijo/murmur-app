//! Whole-file pins and citations of the property harness's supervisor mirror
//! (D-S2b-15, Stage 2b-1).
//!
//! The core crate's property harness (`crates/dictation-core/src/property`) models the
//! audio supervisor of `audio_lifecycle.rs` and cites, row by row, the code it mirrors
//! (`MIRROR`, 45+ rows). A citation or a guard fragment only sees what its author
//! thought to pin, and three review rounds in a row turned around something nobody had
//! pinned, so this test pins **every production function, method, constant and static of
//! `audio_lifecycle.rs`**, each with its own hash ([`LIFECYCLE_PINS`]):
//!
//! * editing, adding or removing any of them fails [`every_production_item_of_audio_lifecycle_is_pinned`]
//!   and names the item, until its pin is updated;
//! * **a re-pin says in its commit message that the model was re-checked against the
//!   change** (which model step, which row), or that the item is outside the model and why
//!   ([`NOT_MODELED`]);
//! * the hash is the shell ratchet's: `proc_macro2` tokens with token kinds, punctuation
//!   spacing and commas kept, and only whitespace and comments stripped
//!   ([`crate::token_pin_tests`]). A comment-only edit passes; a one-token edit fails.
//!
//! The per-item pins miss what is not an item with a body: a `serde` attribute on an enum,
//! a `macro_rules!` body, an `impl` with no methods, a module's own `cfg`, a `use`. So the
//! **whole production file** is pinned too ([`LIFECYCLE_FILE_PIN`], D-S2b-17 (a)): the
//! normalized tokens of every item of `audio_lifecycle.rs` except what is test-only, which
//! is a module or an item whose own attribute is exactly `#[cfg(test)]` (and, inside an
//! `impl` or a trait, a member with that attribute). Any production edit fails
//! [`the_audio_lifecycle_production_source_is_pinned_as_a_whole`] until the file is
//! re-pinned; the per-item pins stay, to name the item that changed.
//!
//! The citations are checked here too, as tokens: every row's function exists, its cited
//! lines lie inside it, and its fragment and every guard occur as a token sequence within
//! the cited lines. The rows themselves live in the core crate beside the model
//! (`property/mirror_table.rs`, included here by path), so the table the harness runs and
//! the table checked here are the same data.

use crate::token_pin_tests::{find_fragment, flat_tokens, fragment_tokens, token_hash, Flat};
use proc_macro2::TokenStream;
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};

#[path = "../crates/dictation-core/src/property/mirror_table.rs"]
mod mirror_table;
use mirror_table::{Mirror, MIRROR};

const LIFECYCLE_SOURCE: &str = include_str!("audio_lifecycle.rs");
const AUDIO_SOURCE: &str = include_str!("audio.rs");

/// One pinned item of a source file.
#[derive(Clone)]
struct Item {
    /// Module path, `impl` type (or `<Type as Trait>`) and name: `Type::name`.
    key: String,
    /// The bare name: `name`.
    name: String,
    hash: u64,
    /// The first and last source line of the item (attributes included).
    lines: (usize, usize),
    tokens: Vec<Flat>,
    /// The item sits under a `#[cfg(test)]`.
    test_only: bool,
}

/// Whether an attribute list gates the item behind `cfg(test)` (a predicate that requires
/// `test`: `test`, or `all(.., test, ..)`; never one under `not(..)` or `any(..)`).
fn requires_test(attrs: &[syn::Attribute]) -> bool {
    fn needs(tokens: TokenStream) -> bool {
        use proc_macro2::TokenTree;
        let tokens: Vec<TokenTree> = tokens.into_iter().collect();
        let mut at = 0;
        while at < tokens.len() {
            match &tokens[at] {
                TokenTree::Ident(ident) if ident == "test" => return true,
                TokenTree::Ident(ident) if ident == "all" => {
                    if let Some(TokenTree::Group(group)) = tokens.get(at + 1) {
                        if needs(group.stream()) {
                            return true;
                        }
                    }
                    at += 1;
                }
                _ => {}
            }
            at += 1;
        }
        false
    }
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .meta
                .require_list()
                .is_ok_and(|list| needs(list.tokens.clone()))
    })
}

fn written<T: ToTokens>(node: &T) -> String {
    node.to_token_stream()
        .to_string()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

fn line_span(stream: &TokenStream) -> (usize, usize) {
    use proc_macro2::TokenTree;
    let all: Vec<TokenTree> = stream.clone().into_iter().collect();
    let first = all.first().map_or(1, |token| token.span().start().line);
    let last = all.last().map_or(first, |token| match token {
        TokenTree::Group(group) => group.span_close().start().line,
        other => other.span().end().line,
    });
    (first, last)
}

fn item_of(prefix: &[String], name: &str, node: &impl ToTokens, test_only: bool) -> Item {
    let stream = node.to_token_stream();
    let mut path = prefix.to_vec();
    path.push(name.to_string());
    Item {
        key: path.join("::"),
        name: name.to_string(),
        hash: token_hash(node),
        lines: line_span(&stream),
        tokens: flat_tokens(stream),
        test_only,
    }
}

fn walk(items: &[syn::Item], prefix: &mut Vec<String>, test_only: bool, out: &mut Vec<Item>) {
    for item in items {
        match item {
            syn::Item::Fn(function) => out.push(item_of(
                prefix,
                &function.sig.ident.to_string(),
                function,
                test_only || requires_test(&function.attrs),
            )),
            syn::Item::Const(constant) => out.push(item_of(
                prefix,
                &format!("const {}", constant.ident),
                constant,
                test_only || requires_test(&constant.attrs),
            )),
            syn::Item::Static(item_static) => out.push(item_of(
                prefix,
                &format!("static {}", item_static.ident),
                item_static,
                test_only || requires_test(&item_static.attrs),
            )),
            syn::Item::Mod(module) => {
                if let Some((_, content)) = &module.content {
                    prefix.push(module.ident.to_string());
                    walk(
                        content,
                        prefix,
                        test_only || requires_test(&module.attrs),
                        out,
                    );
                    prefix.pop();
                }
            }
            syn::Item::Impl(implementation) => {
                let owner = match &implementation.trait_ {
                    Some((_, path, _)) => {
                        format!(
                            "<{} as {}>",
                            written(&*implementation.self_ty),
                            written(path)
                        )
                    }
                    None => written(&*implementation.self_ty),
                };
                prefix.push(owner);
                let impl_test = test_only || requires_test(&implementation.attrs);
                for member in &implementation.items {
                    match member {
                        syn::ImplItem::Fn(method) => out.push(item_of(
                            prefix,
                            &method.sig.ident.to_string(),
                            method,
                            impl_test || requires_test(&method.attrs),
                        )),
                        syn::ImplItem::Const(constant) => out.push(item_of(
                            prefix,
                            &format!("const {}", constant.ident),
                            constant,
                            impl_test || requires_test(&constant.attrs),
                        )),
                        _ => {}
                    }
                }
                prefix.pop();
            }
            syn::Item::Trait(item_trait) => {
                prefix.push(item_trait.ident.to_string());
                let trait_test = test_only || requires_test(&item_trait.attrs);
                for member in &item_trait.items {
                    if let syn::TraitItem::Fn(method) = member {
                        if method.default.is_some() {
                            out.push(item_of(
                                prefix,
                                &method.sig.ident.to_string(),
                                method,
                                trait_test || requires_test(&method.attrs),
                            ));
                        }
                    }
                }
                prefix.pop();
            }
            _ => {}
        }
    }
}

/// Every function, method, constant and static of `source`, keyed by its qualified name
/// (a repeated key gets `#2`, `#3`): the production items and the `cfg(test)` ones.
fn items_of(source: &str) -> Vec<Item> {
    let file = syn::parse_file(source).expect("the source parses");
    let mut out = Vec::new();
    walk(&file.items, &mut Vec::new(), false, &mut out);
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for item in &mut out {
        let count = seen.entry(item.key.clone()).or_insert(0);
        *count += 1;
        if *count > 1 {
            item.key = format!("{}#{count}", item.key);
        }
    }
    out
}

fn production_items(source: &str) -> Vec<Item> {
    items_of(source)
        .into_iter()
        .filter(|item| !item.test_only)
        .collect()
}

/// The differences between the pinned items of a source and its items today, as one
/// message line each.
fn pin_problems(
    file: &str,
    items: &[Item],
    pins: &[(&str, u64)],
    only_pinned_names: bool,
) -> Vec<String> {
    let mut problems = Vec::new();
    let pinned: BTreeMap<&str, u64> = pins.iter().copied().collect();
    let today: BTreeMap<&str, &Item> = items.iter().map(|i| (i.key.as_str(), i)).collect();
    for (key, hash) in &pinned {
        match today.get(key) {
            None => problems.push(format!(
                "{file}: `{key}` is pinned and no longer exists (removed or renamed)"
            )),
            Some(item) if item.hash != *hash => problems.push(format!(
                "{file}: `{key}` was edited (hash {:#018x}, pinned {hash:#018x})",
                item.hash
            )),
            Some(_) => {}
        }
    }
    if !only_pinned_names {
        for (key, item) in &today {
            if !pinned.contains_key(key) {
                problems.push(format!(
                    "{file}: `{key}` (lines {}-{}) is new and has no pin (hash {:#018x})",
                    item.lines.0, item.lines.1, item.hash
                ));
            }
        }
    }
    problems
}

/// Whether an attribute list carries exactly `#[cfg(test)]` (not `cfg(all(test, ..))`, not
/// `cfg(not(test))`: only the plain test gate marks an item as test-only).
fn is_exactly_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .meta
                .require_list()
                .is_ok_and(|list| list.tokens.to_string().trim() == "test")
    })
}

/// `items` without the test-only ones, at every depth an item can sit (modules, `impl`
/// and trait members).
fn without_test_items(items: Vec<syn::Item>) -> Vec<syn::Item> {
    items
        .into_iter()
        .filter_map(|item| match item {
            syn::Item::Fn(i) if is_exactly_cfg_test(&i.attrs) => None,
            syn::Item::Const(i) if is_exactly_cfg_test(&i.attrs) => None,
            syn::Item::Static(i) if is_exactly_cfg_test(&i.attrs) => None,
            syn::Item::Struct(i) if is_exactly_cfg_test(&i.attrs) => None,
            syn::Item::Enum(i) if is_exactly_cfg_test(&i.attrs) => None,
            syn::Item::Use(i) if is_exactly_cfg_test(&i.attrs) => None,
            syn::Item::Type(i) if is_exactly_cfg_test(&i.attrs) => None,
            syn::Item::Macro(i) if is_exactly_cfg_test(&i.attrs) => None,
            syn::Item::Union(i) if is_exactly_cfg_test(&i.attrs) => None,
            syn::Item::ExternCrate(i) if is_exactly_cfg_test(&i.attrs) => None,
            syn::Item::ForeignMod(mut block) => {
                if is_exactly_cfg_test(&block.attrs) {
                    return None;
                }
                // A foreign declaration with its own exact test gate is test-only too.
                block.items.retain(|member| match member {
                    syn::ForeignItem::Fn(m) => !is_exactly_cfg_test(&m.attrs),
                    syn::ForeignItem::Static(m) => !is_exactly_cfg_test(&m.attrs),
                    syn::ForeignItem::Type(m) => !is_exactly_cfg_test(&m.attrs),
                    syn::ForeignItem::Macro(m) => !is_exactly_cfg_test(&m.attrs),
                    _ => true,
                });
                Some(syn::Item::ForeignMod(block))
            }
            syn::Item::TraitAlias(i) if is_exactly_cfg_test(&i.attrs) => None,
            syn::Item::Mod(mut module) => {
                if is_exactly_cfg_test(&module.attrs) {
                    return None;
                }
                if let Some((brace, content)) = module.content.take() {
                    module.content = Some((brace, without_test_items(content)));
                }
                Some(syn::Item::Mod(module))
            }
            syn::Item::Impl(mut implementation) => {
                if is_exactly_cfg_test(&implementation.attrs) {
                    return None;
                }
                implementation.items.retain(|member| match member {
                    syn::ImplItem::Fn(m) => !is_exactly_cfg_test(&m.attrs),
                    syn::ImplItem::Const(m) => !is_exactly_cfg_test(&m.attrs),
                    syn::ImplItem::Type(m) => !is_exactly_cfg_test(&m.attrs),
                    syn::ImplItem::Macro(m) => !is_exactly_cfg_test(&m.attrs),
                    _ => true,
                });
                Some(syn::Item::Impl(implementation))
            }
            syn::Item::Trait(mut item_trait) => {
                if is_exactly_cfg_test(&item_trait.attrs) {
                    return None;
                }
                item_trait.items.retain(|member| match member {
                    syn::TraitItem::Fn(m) => !is_exactly_cfg_test(&m.attrs),
                    syn::TraitItem::Const(m) => !is_exactly_cfg_test(&m.attrs),
                    syn::TraitItem::Type(m) => !is_exactly_cfg_test(&m.attrs),
                    syn::TraitItem::Macro(m) => !is_exactly_cfg_test(&m.attrs),
                    _ => true,
                });
                Some(syn::Item::Trait(item_trait))
            }
            other => Some(other),
        })
        .collect()
}

/// The hash of the whole production source: the normalized tokens of the file (inner
/// attributes included) with the test-only items removed.
fn production_file_hash(source: &str) -> u64 {
    let mut file = syn::parse_file(source).expect("the source parses");
    file.items = without_test_items(std::mem::take(&mut file.items));
    token_hash(&file)
}

/// D-S2b-17 (a): the whole production file is pinned, so what no per-item pin sees (a serde
/// attribute, a `macro_rules!` body, an impl with no methods, a module's cfg, a `use`)
/// fails too.
#[test]
fn the_audio_lifecycle_production_source_is_pinned_as_a_whole() {
    let today = production_file_hash(LIFECYCLE_SOURCE);
    assert_eq!(
        today, LIFECYCLE_FILE_PIN,
        "the production source of audio_lifecycle.rs changed (hash {today:#018x}, pinned \
         {LIFECYCLE_FILE_PIN:#018x}); the per-item test names the item when one changed.\n\n{RE_PIN}"
    );
}

/// What the whole-file pin sees and what it ignores, on the real source and on probes.
#[test]
fn the_whole_file_pin_sees_every_production_edit_and_no_test_or_comment_edit() {
    let base = production_file_hash(LIFECYCLE_SOURCE);
    let edit = |from: &str, to: &str| {
        assert!(
            LIFECYCLE_SOURCE.contains(from),
            "the probe's anchor `{from}` exists in audio_lifecycle.rs"
        );
        production_file_hash(&LIFECYCLE_SOURCE.replacen(from, to, 1))
    };
    // Sol's serde probe: the stop trigger's wire spelling is an attribute on the enum, which
    // no per-item pin hashes.
    assert_ne!(
        edit(
            "#[serde(rename_all = \"snake_case\")]",
            "#[serde(rename_all = \"camelCase\")]"
        ),
        base,
        "a serde rename is a production edit"
    );
    assert_ne!(
        edit(
            "#[cfg(target_os = \"macos\")]\npub(crate) fn register_sleep",
            "#[cfg(not(target_os = \"linux\"))]\npub(crate) fn register_sleep"
        ),
        base
    );
    assert_ne!(
        edit("use std::", "use core::"),
        base,
        "a `use` is production"
    );
    // Probes the real file has no instance of: appended, then edited.
    let appended = |extra: &str| production_file_hash(&format!("{LIFECYCLE_SOURCE}\n{extra}\n"));
    for (one, other, what) in [
        (
            "macro_rules! probe { () => { 1 } }",
            "macro_rules! probe { () => { 2 } }",
            "a top-level macro_rules! body",
        ),
        (
            "impl Marker for Probe {}",
            "impl Other for Probe {}",
            "an impl with no methods",
        ),
        (
            "unsafe impl Send for Probe {}",
            "impl Send for Probe {}",
            "an unsafe impl",
        ),
        (
            "#[cfg(target_os = \"macos\")] mod probe { pub fn f() {} }",
            "#[cfg(target_os = \"linux\")] mod probe { pub fn f() {} }",
            "a production module's cfg",
        ),
        (
            "#[derive(Debug)] struct Probe;",
            "#[derive(Clone)] struct Probe;",
            "a derive",
        ),
    ] {
        assert_ne!(appended(one), base, "{what}: an addition is an edit");
        assert_ne!(
            appended(one),
            appended(other),
            "{what}: an edit of it is an edit"
        );
    }
    // Test-only content is not production: a `#[cfg(test)]` module, an item whose own
    // attribute is exactly `#[cfg(test)]`, an impl member with it.
    assert_eq!(
        appended("#[cfg(test)] mod probe_tests { fn f() { 1 } }"),
        appended("#[cfg(test)] mod probe_tests { fn f() { 2 } }")
    );
    assert_eq!(
        appended("#[cfg(test)] fn probe() { 1 }"),
        appended("#[cfg(test)] fn probe() { 2 }")
    );
    assert_eq!(
        appended("struct P; impl P { #[cfg(test)] fn t() { 1 } }"),
        appended("struct P; impl P { #[cfg(test)] fn t() { 2 } }")
    );
    // A foreign declaration with the exact test gate is test-only; the test-looking gates
    // are production and an edit of any of them is an edit (Sol's round-7 probes).
    assert_eq!(
        appended("extern \"C\" { #[cfg(test)] fn probe(); }"),
        appended("extern \"C\" { #[cfg(test)] fn probe(x: u8); }")
    );
    assert_eq!(
        appended("extern \"C\" { #[cfg(test)] static PROBE: u8; }"),
        appended("extern \"C\" { #[cfg(test)] static PROBE: u16; }")
    );
    assert_ne!(
        appended("extern \"C\" { fn probe(); }"),
        appended("extern \"C\" { fn probe(x: u8); }"),
        "an ungated foreign declaration is production"
    );
    // ... and only the exact attribute: `cfg(all(test, unix))`, `cfg(any(test, unix))`,
    // `cfg(not(test))` and `cfg_attr(test, ..)` are kept (production in some build, so an
    // edit of any of them is an edit), on items and on foreign declarations alike.
    for gate in [
        "#[cfg(all(test, unix))]",
        "#[cfg(any(test, unix))]",
        "#[cfg(not(test))]",
        "#[cfg_attr(test, inline)]",
    ] {
        assert_ne!(
            appended(&format!("{gate} fn probe() {{ 1 }}")),
            appended(&format!("{gate} fn probe() {{ 2 }}")),
            "{gate} on an item"
        );
        assert_ne!(
            appended(&format!("extern \"C\" {{ {gate} fn probe(); }}")),
            appended(&format!("extern \"C\" {{ {gate} fn probe(x: u8); }}")),
            "{gate} on a foreign declaration"
        );
    }
    // Sol's round-7 probe: a macro input's `#[doc = ".."]` is an edit (a macro can expand it
    // into anything); a source doc comment outside a macro is not.
    assert_ne!(
        appended("macro_rules! pick { (#[doc = \"normal\"]) => { 1 }; (#[doc = $x:literal]) => { 2 }; } fn probe() -> u8 { pick!(#[doc = \"normal\"]) }"),
        appended("macro_rules! pick { (#[doc = \"normal\"]) => { 1 }; (#[doc = $x:literal]) => { 2 }; } fn probe() -> u8 { pick!(#[doc = \"changed\"]) }"),
        "a #[doc] argument of a macro call is an edit"
    );
    assert_eq!(
        appended("/// one\nfn probe() { let _ = 1; }"),
        appended("/// two\n/// lines\nfn probe() { let _ = 1; }")
    );
    // A comment is not an edit; a doc comment is not an edit.
    assert_eq!(
        edit(
            "let cancelled = should_abandon;",
            "let cancelled = should_abandon;/* x */ // note\n"
        ),
        base
    );
    assert_eq!(
        edit(
            "pub(crate) fn dictation_id(",
            "/// a new doc line\n    pub(crate) fn dictation_id("
        ),
        base
    );
}

const RE_PIN: &str = "re-check the model steps that cite the item (`property/mirror_table.rs`), \
    or the NOT_MODELED reason, then update the pin and say in the commit message that the model \
    was re-checked against the change";

/// Every non-test function, method, constant and static of `audio_lifecycle.rs` has its
/// own pin. The pin table is [`LIFECYCLE_PINS`]; `print_current_pins` prints it.
#[test]
fn every_production_item_of_audio_lifecycle_is_pinned() {
    let problems = pin_problems(
        "audio_lifecycle.rs",
        &production_items(LIFECYCLE_SOURCE),
        LIFECYCLE_PINS,
        false,
    );
    assert!(problems.is_empty(), "{}\n\n{RE_PIN}", problems.join("\n"));
}

/// The functions of `audio.rs` the mirror cites (the worker thread's own end).
#[test]
fn the_cited_functions_of_audio_rs_are_pinned() {
    let problems = pin_problems(
        "audio.rs",
        &production_items(AUDIO_SOURCE),
        AUDIO_PINS,
        true,
    );
    assert!(problems.is_empty(), "{}\n\n{RE_PIN}", problems.join("\n"));
}

fn items_for(file: &str) -> Vec<Item> {
    match file {
        "audio_lifecycle.rs" => production_items(LIFECYCLE_SOURCE),
        "audio.rs" => production_items(AUDIO_SOURCE),
        other => panic!("unknown source file {other}"),
    }
}

/// The item a row cites, by its qualified key (`Type::name`) or, when the bare name is
/// unique in the file, by its name.
fn cited_item(row: &Mirror) -> Result<Item, String> {
    let items = items_for(row.file);
    let exact: Vec<&Item> = items.iter().filter(|i| i.key == row.function).collect();
    let found: Vec<&Item> = if exact.is_empty() {
        items.iter().filter(|i| i.name == row.function).collect()
    } else {
        exact
    };
    match found.as_slice() {
        [item] => Ok((*item).clone()),
        other => Err(format!(
            "{}: `{}` in {} must name exactly one production item, found {}",
            row.step,
            row.function,
            row.file,
            other.len()
        )),
    }
}

/// The mirror's citations are true, as tokens: the cited function exists, its cited lines
/// lie inside it, and the fragment and every guard of the row occur as a sequence of
/// tokens within the cited lines. A row that cites a real-supervisor test names a test
/// that exists.
#[test]
fn the_supervisor_mirror_cites_the_real_source() {
    let mut problems = Vec::new();
    let tests: BTreeSet<String> = items_of(LIFECYCLE_SOURCE)
        .into_iter()
        .filter(|item| item.test_only)
        .map(|item| item.name)
        .collect();
    for row in MIRROR {
        let item = match cited_item(row) {
            Ok(item) => item,
            Err(problem) => {
                problems.push(problem);
                continue;
            }
        };
        let (first, last) = (row.lines.0 as usize, row.lines.1 as usize);
        if !(item.lines.0 <= first && first <= last && last <= item.lines.1) {
            problems.push(format!(
                "{}: lines {first}-{last} are not inside `{}` (lines {}-{})",
                row.step, row.function, item.lines.0, item.lines.1
            ));
            continue;
        }
        for fragment in std::iter::once(&row.needle).chain(row.guards.iter()) {
            match fragment_tokens(fragment) {
                Err(problem) => problems.push(format!("{}: {problem}", row.step)),
                Ok(wanted) => {
                    if find_fragment(&item.tokens, &wanted, (first, last)).is_none() {
                        problems.push(format!(
                            "{}: the tokens `{fragment}` are not in lines {first}-{last} of \
                             `{}`: the model mirrors code the source no longer has",
                            row.step, row.function
                        ));
                    }
                }
            }
        }
        if let Some(test) = row.real_test {
            if !tests.contains(test) {
                problems.push(format!(
                    "{}: the real-supervisor test `{test}` does not exist in audio_lifecycle.rs",
                    row.step
                ));
            }
        }
        if row.what.is_empty() {
            problems.push(format!("{}: the row says what the model does", row.step));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The names a token sequence calls: `name(` where `name` is not a declaration.
fn called_names(tokens: &[Flat]) -> BTreeSet<String> {
    let mut called = BTreeSet::new();
    for at in 0..tokens.len().saturating_sub(1) {
        let token = &tokens[at].text;
        let identifier = token
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
            && !token.starts_with("r#");
        if identifier
            && tokens[at + 1].text == "("
            && (at == 0 || tokens[at - 1].text != "fn")
            && token != "fn"
        {
            called.insert(token.clone());
        }
    }
    called
}

/// Every function of `audio_lifecycle.rs` that a cited function calls is either cited by a
/// row itself (so the model mirrors it and a reader sees its lines) or listed in
/// [`NOT_MODELED`] with the reason the model does not mirror it. A pin alone does not
/// say the model looked at it.
#[test]
fn every_callee_of_a_cited_function_is_mirrored_or_not_modeled() {
    let lifecycle = production_items(LIFECYCLE_SOURCE);
    let defined: BTreeSet<&str> = lifecycle.iter().map(|item| item.name.as_str()).collect();
    let cited_names: BTreeSet<&str> = MIRROR
        .iter()
        .filter(|row| row.file == "audio_lifecycle.rs")
        .map(|row| row.function.rsplit("::").next().unwrap_or(row.function))
        .collect();
    let exempt: BTreeSet<&str> = NOT_MODELED.iter().map(|(name, _)| *name).collect();
    let mut calls = BTreeSet::new();
    let mut problems = Vec::new();
    for row in MIRROR.iter().filter(|row| row.file == "audio_lifecycle.rs") {
        match cited_item(row) {
            Ok(item) => calls.extend(called_names(&item.tokens)),
            Err(problem) => problems.push(problem),
        }
    }
    for name in &calls {
        if defined.contains(name.as_str())
            && !cited_names.contains(name.as_str())
            && !exempt.contains(name.as_str())
        {
            problems.push(format!(
                "`{name}` is called by a cited function and is neither cited by a mirror row nor \
                 in NOT_MODELED"
            ));
        }
    }
    for (name, why) in NOT_MODELED {
        if !calls.contains(*name) {
            problems.push(format!(
                "NOT_MODELED `{name}` is not called by a cited function"
            ));
        }
        if cited_names.contains(name) {
            problems.push(format!("NOT_MODELED `{name}` is cited by a mirror row"));
        }
        if why.trim().is_empty() {
            problems.push(format!("NOT_MODELED `{name}` has no reason"));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Every pin of [`AUDIO_PINS`] is cited by a mirror row, and every row's file has pins for
/// its function: a pin nobody cites is stale.
#[test]
fn every_audio_rs_pin_is_cited_by_a_row() {
    for (key, _) in AUDIO_PINS {
        assert!(
            MIRROR.iter().any(|row| row.file == "audio.rs"
                && (row.function == *key || key.ends_with(row.function))),
            "the audio.rs pin `{key}` is cited by no mirror row"
        );
    }
    for row in MIRROR.iter().filter(|row| row.file == "audio.rs") {
        let item = cited_item(row).expect("the row cites one item");
        assert!(
            AUDIO_PINS.iter().any(|(key, _)| *key == item.key),
            "{}: `{}` has no pin in AUDIO_PINS",
            row.step,
            item.key
        );
    }
}

// ---------------------------------------------------------------- the checks, mutated

/// Source text with the item `function` of `LIFECYCLE_SOURCE` replaced: for the tests of
/// the checks themselves (a check that cannot fail proves nothing).
fn mutated_lifecycle(edit: impl FnOnce(&str) -> String) -> Vec<Item> {
    items_of(&edit(LIFECYCLE_SOURCE))
        .into_iter()
        .filter(|item| !item.test_only)
        .collect()
}

/// A comment-only edit (`/* x */` right after a `;`, a line comment, a doc comment) passes
/// the pins and the citations.
#[test]
fn a_comment_only_edit_passes_the_pins_and_the_citations() {
    let source = LIFECYCLE_SOURCE.replacen(
        "let cancelled = should_abandon;",
        "let cancelled = should_abandon;/* x */ // note\n        /// doc\n",
        1,
    );
    assert_ne!(source, LIFECYCLE_SOURCE, "the probe's anchor exists");
    let items = items_of(&source)
        .into_iter()
        .filter(|item| !item.test_only)
        .collect::<Vec<_>>();
    assert!(pin_problems("audio_lifecycle.rs", &items, LIFECYCLE_PINS, false).is_empty());
    // A comment between a receiver and its call does not hide the cited fragment.
    let source = LIFECYCLE_SOURCE.replacen("response.send(", "response/* x */.send(", 1);
    assert_ne!(source, LIFECYCLE_SOURCE);
    let items = items_of(&source);
    let handled = items
        .iter()
        .filter(|item| !item.test_only)
        .cloned()
        .collect::<Vec<_>>();
    assert!(pin_problems("audio_lifecycle.rs", &handled, LIFECYCLE_PINS, false).is_empty());
}

/// A one-token edit of any production function fails the pins and names it: every guard
/// turned around, a literal changed, a clause dropped.
#[test]
fn a_one_token_edit_of_a_pinned_function_fails() {
    let edits: [(&str, &str); 6] = [
        // Sol's round-5 mutant: the cleanup-pending phase `Stopping` becomes `Starting`.
        (
            "PublicPhase::Stopping | PublicPhase::Recovering\n        )",
            "PublicPhase::Starting | PublicPhase::Recovering\n        )",
        ),
        ("let cancelled = should_abandon;", "let cancelled = false;"),
        (
            ".remove(&AudioOwner::Dictation(recording_id))",
            ".remove(&AudioOwner::Dictation(recording_id + 1))",
        ),
        (
            "if attempt.failure_reported {",
            "if !attempt.failure_reported {",
        ),
        (
            "public.owns(AudioOwner::Dictation(recording_id))",
            "!public.owns(AudioOwner::Dictation(recording_id))",
        ),
        (
            ".is_recording_for(AudioOwner::Dictation(recording_id))",
            ".is_recording_for(AudioOwner::Preview(recording_id))",
        ),
    ];
    for (from, to) in edits {
        assert!(
            LIFECYCLE_SOURCE.contains(from),
            "the probe's anchor `{from}` exists in audio_lifecycle.rs"
        );
        let items = mutated_lifecycle(|source| source.replacen(from, to, 1));
        let problems = pin_problems("audio_lifecycle.rs", &items, LIFECYCLE_PINS, false);
        assert_eq!(
            problems.len(),
            1,
            "`{from}` -> `{to}` must fail exactly one pin, got {problems:?}"
        );
        assert!(problems[0].contains("was edited"), "{problems:?}");
    }
}

/// Adding or removing a function fails the pins too.
#[test]
fn an_added_or_removed_function_fails() {
    let items =
        mutated_lifecycle(|source| format!("{source}\nfn a_new_helper(x: u8) -> u8 {{ x }}\n"));
    let problems = pin_problems("audio_lifecycle.rs", &items, LIFECYCLE_PINS, false);
    assert!(problems
        .iter()
        .any(|p| p.contains("a_new_helper") && p.contains("no pin")));
    let items =
        mutated_lifecycle(|source| source.replacen("fn dictation_id(", "fn renamed_id(", 1));
    let problems = pin_problems("audio_lifecycle.rs", &items, LIFECYCLE_PINS, false);
    assert!(problems.iter().any(|p| p.contains("no longer exists")));
    assert!(problems.iter().any(|p| p.contains("renamed_id")));
}

// ------------------------------------------------------------------------- the data

/// Functions of `audio_lifecycle.rs` that a cited function calls and the model does not
/// mirror, each with the reason.
const NOT_MODELED: &[(&str, &str)] = &[
    ("active_initialization_elapsed", "a duration: the hard and still-connecting deadlines are timer steps the harness fires nondeterministically"),
    ("adopt_speculative", "the speculative phase is an optional feature the dictation commands never enter"),
    ("as_str", "trait and enum spelling helpers used by logs"),
    ("automatic_capture_authorized", "a permission probe of the automatic preview, which dictation never starts"),
    ("convert_for_delivery", "sample-rate conversion; the model carries sample counts, not samples"),
    ("deadline_wait", "how long the loop sleeps; the model's timers are steps, not durations"),
    ("default", "constructors"),
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

/// Every cited function of `audio.rs`, with its pin.
const AUDIO_PINS: &[(&str, u64)] = &[("spawn_capture_worker", 0x18d7e43cb5dc29ce)];

/// The whole production source of `audio_lifecycle.rs` (D-S2b-17 (a)).
const LIFECYCLE_FILE_PIN: u64 = 0x6f306b1fe4ba53ae;

/// Every production function, method, constant and static of `audio_lifecycle.rs`.
const LIFECYCLE_PINS: &[(&str, u64)] = &[
    ("const STILL_CONNECTING_AFTER", 0x35059d9cc16f5ec1),
    ("const HARD_INITIALIZATION_DEADLINE", 0x7ac8df8e56509f36),
    ("const RECOVERY_GUIDANCE_AFTER", 0x74531922ac4ac474),
    ("const SUPERVISOR_RESPONSE_TIMEOUT", 0xcc30e21a248e9a94),
    ("const STOP_RESPONSE_TIMEOUT", 0xcf38e89d1a37455b),
    ("AudioOwner::telemetry_id", 0x325d3738208cf8fc),
    ("AudioOwner::kind", 0x327929789a2fa247),
    ("AudioOwner::is_dictation_capture", 0xfce0d8f6ebcf66e4),
    ("AudioOwner::retains_samples", 0x77d1a32a94584bbd),
    ("AudioOwner::preview_id", 0xfd0069db2e43df76),
    ("AudioOwner::is_speculative", 0x5ddbe51d9d9e7ee8),
    ("AudioOwner::dictation_id", 0xa757fde066e93e24),
    ("AudioOwner::is_microphone_benchmark", 0xbcf9863c8a07fd89),
    ("DictationStopTrigger::resolve", 0x88fa8c532c8276c7),
    (
        "DictationStopTrigger::may_anchor_to_release",
        0x2466680075fe45a2,
    ),
    ("DictationStopTrigger::tail_anchor", 0x59bf3921d548ed8c),
    ("DictationStopTrigger::release_tail", 0xc430e3542a279ebe),
    ("DictationStopTrigger::as_str", 0xdada73506d7dbfe9),
    ("release_anchor", 0x45eedb8ab4df7eb1),
    ("tail_deadline", 0x515110914b073f98),
    ("AudioCancelReason::as_str", 0x4804f5d1f7a22810),
    ("<AudioStartError as fmt::Display>::fmt", 0xbe5aaa5842f0b4e9),
    ("PublicPhase::as_str", 0xa3c4b2acb05640a3),
    ("<PublicState as Default>::default", 0x7f124e253b91b8ea),
    ("PublicState::set_phase", 0xb888721f05eb8cd2),
    ("PublicState::is_active", 0x47207a2ab5e60814),
    ("PublicState::phase", 0x9682bd3e9a87f5c3),
    ("PublicState::set_owner", 0x254081cb9f8ffc5b),
    ("PublicState::clear_owner", 0xad533c6a9f74fae7),
    ("PublicState::is_recording_for", 0xd9ad67295f1a48b9),
    ("PublicState::owns", 0x719b9acb1b12babb),
    ("PublicState::preview_owner", 0xc312adc50e25c998),
    ("<SupervisorConfig as Default>::default", 0x42beb56c0d33e2cb),
    (
        "WorkerFactory::automatic_capture_authorized",
        0xb071f934a6c2cde0,
    ),
    (
        "<ProductionWorkerFactory as WorkerFactory>::spawn",
        0x60b59767c66ea6b9,
    ),
    (
        "<ProductionLifecycleSink as LifecycleSink>::invalidate_input_health",
        0x167378c2dd6224bd,
    ),
    (
        "<ProductionLifecycleSink as LifecycleSink>::notify",
        0x588f22d4922eae79,
    ),
    ("Attempt::send_worker_stop", 0xaf715ff93b076ad2),
    ("Attempt::warm_epoch", 0x968ed6d23767fd6c),
    ("Attempt::warm_topology_is_current", 0x02bbc749ce64eae9),
    ("Attempt::active_initialization_elapsed", 0xf3e32dd22086f012),
    ("static INPUT_TOPOLOGY_GENERATION", 0xe7e9f5e862c43141),
    ("static SUPERVISOR", 0xa093c35c4ebb077c),
    ("static HAL_BOUNDARY", 0xf47b78b1b1d452ec),
    ("hal_boundary", 0x240c020aba7a7079),
    ("supervisor", 0x786873c48abe172e),
    ("spawn_supervisor", 0x41f455dfa7e57327),
    ("run_supervisor", 0x5d4b34895a9c5d33),
    ("deadline_wait", 0xbd355e4aaeb8fe2b),
    ("handle_message", 0xfcb0d2e39a716eb5),
    ("handle_start", 0x32b7acb2718dbb75),
    ("publish_gesture_ready", 0x559343a1d7f57d62),
    ("log_speculative_discard", 0xc012c3899bfdf74c),
    ("log_speculative_failure", 0x925e85479d4c7815),
    ("start_speculative", 0x4b18e086775592ec),
    ("is_visible_preview", 0x771e898c900ddb87),
    ("preview_handoff_ready", 0x8d2cc27ce2d08ecc),
    ("hand_preview_to", 0x6092481168159643),
    ("adopt_speculative", 0x1eeb250abda3d4c4),
    ("handle_worker_event", 0xd1f59193f4ca8fec),
    ("finish_attempt", 0x5dd11666db90cc60),
    ("take_samples", 0xbdfdfcd9572618b6),
    ("convert_for_delivery", 0x02443553d4888da6),
    ("samples_for_asr", 0xf9252dfba64683ac),
    ("snapshot_for_asr", 0xaff6e94ff3b86d34),
    ("snapshot_since", 0x34e94ad1484e284d),
    ("recording_duration_since_ready", 0x23331d8f9a1b75bb),
    ("begin_recovery", 0xbe6eb594c9fab444),
    ("static INTERRUPTED_CAPTURES", 0x5fcce94cc042d990),
    ("interrupted_captures", 0x3e7dcd33996b8e43),
    ("take_interrupted_dictation", 0xc5a3f253df856183),
    ("report_failure_once", 0xaf629931edec741e),
    ("handle_deadlines_at", 0xf9bc30f69d09c855),
    ("report_cleanup_stalled", 0xdecc87dd4d535bb8),
    ("send_start", 0xa9dea90369a93eb0),
    ("send_start_with_permit", 0x3ebd2ceeb173e05c),
    ("send_start_with_timeout", 0xdcead7302ebb0a87),
    ("send_start_request", 0x5c2b0e1b9eddcd67),
    ("await_start_response", 0x3392209b566462cc),
    ("start_dictation_recording", 0x1e5e078df5370d84),
    ("const START_AFTER_PREVIEW_TIMEOUT", 0xb98f59fc61c438fe),
    (
        "start_dictation_recording_after_preview",
        0x4f7452016c84ebb3,
    ),
    ("start_preview_recording", 0x610c986b55651088),
    ("start_automatic_preview_recording", 0x2e4660d968bea7cc),
    ("start_microphone_benchmark_recording", 0x99e575f80f0b0105),
    ("start_corpus_recording", 0xb482f0ce360aa3b3),
    ("stop_dictation_recording", 0x20f82528714d126a),
    ("peek_dictation_samples", 0xb750406a42bb2bc0),
    ("peek_dictation_samples_since", 0xb0c6e8ca3adc39b3),
    ("stop_preview_recording", 0x2aeff9b33a526541),
    ("stop_microphone_benchmark_recording", 0x1e4a6be7757565f2),
    ("stop_corpus_recording", 0xb7efd8b44cf5feab),
    ("stop", 0x267407cffb424af8),
    ("stop_with_trigger", 0x6408b31746ec4389),
    ("cancel_dictation_capture", 0xe4f624348b581554),
    ("cancel_preview_capture", 0x9d56e62681953cb4),
    ("cancel_microphone_benchmark_capture", 0xd5f11226b500f681),
    ("cancel_corpus_capture", 0x7e0201e7ff429d56),
    ("cancel_starting_for_environment_change", 0x9adf633f91a3b865),
    ("cancel_preview_for_environment_change", 0x5ad79bdcf9688980),
    ("cancel", 0x8e8561bc7c601986),
    ("input_topology_generation", 0x5fba2513c22287e4),
    ("input_topology_changed", 0xcaeb94b2d63a1412),
    ("configure_standby", 0xea314faee4da687f),
    ("retire_standby", 0x7affc5701d2e7910),
    ("request_speculative_capture", 0x628d7ae68db10c73),
    ("await_speculative_admission", 0xe5e38691d079c77f),
    ("discard_speculative_capture", 0xd7c2cce84d9ad177),
    ("speculative_capture_is_flowing", 0x027bf4e499086058),
    ("is_audio_active", 0x1571a14257d60b4e),
    ("is_unconfirmed_key_down_capture", 0xe88f1b6da4b4b5e5),
    ("holds_only_unconfirmed_key_down", 0x3ab52d499f9b6098),
    ("is_dictation_cleanup_pending", 0xfd0b1606b796a288),
    ("diagnostic_snapshot", 0x576886fc392b594f),
    ("render_owner", 0x118be025d3b63a20),
    ("is_microphone_benchmark_owner", 0xbd773805af560e13),
    ("with_idle_hal_boundary", 0xd34f23f51a37b5d2),
    ("is_dictation_recording", 0x506744df769a7227),
    ("register_sleep_wake_observer", 0x98862280ee757269),
    ("register_sleep_wake_observer#2", 0xe640326e1ceefab8),
];

/// Prints the pin tables for a re-pin: `cargo test --lib print_current_pins -- --ignored
/// --nocapture`. Re-pin only after re-checking the model steps against the change.
#[test]
#[ignore = "prints the pin tables for a re-pin"]
fn print_current_pins() {
    println!(
        "const LIFECYCLE_FILE_PIN: u64 = {:#018x};",
        production_file_hash(LIFECYCLE_SOURCE)
    );
    println!("const LIFECYCLE_PINS: &[(&str, u64)] = &[");
    for item in production_items(LIFECYCLE_SOURCE) {
        println!(
            "    ({:?}, {:#018x}), // lines {}-{}",
            item.key, item.hash, item.lines.0, item.lines.1
        );
    }
    println!("];");
    println!("const AUDIO_PINS: &[(&str, u64)] = &[");
    let audio = production_items(AUDIO_SOURCE);
    let cited: BTreeSet<&str> = MIRROR
        .iter()
        .filter(|row| row.file == "audio.rs")
        .map(|row| row.function)
        .collect();
    for item in audio
        .iter()
        .filter(|i| cited.contains(i.key.as_str()) || cited.contains(i.name.as_str()))
    {
        println!("    ({:?}, {:#018x}),", item.key, item.hash);
    }
    println!("];");
}
