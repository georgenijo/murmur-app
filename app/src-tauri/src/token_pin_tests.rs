//! Test-only token normalization shared by the source pins (D-S2b-15 (b)).
//!
//! This module is the one copy of the normalization: the shell ratchet
//! (`dictation_executor/ratchet_tests.rs`: its wrapper and `main.rs` pins) and the
//! supervisor mirror's pins (`audio_lifecycle_mirror_tests.rs`) both hash with
//! `normalized_tokens` and `token_hash` from here, so a pin in either agrees on what an
//! edit is (the tests at the bottom pin the behaviour both rely on).
//!
//! A token stream is normalized to text with **no whitespace or comment information**:
//! every other token, every comma and every punctuation's spacing (`Joint` is `~`) is
//! kept, so `(x,)` and `(x)`, `m!(&&x)` and `m!(& &x)`, and `c"x"` and `c "x"` differ,
//! and `x; /* review */ y` hashes like `x; y`.
//!
//! Only **source doc comments** (`///`, `//!`, `/** */`, `/*! */`) are dropped, and only
//! outside macro invocations. They are the one comment the lexer turns into tokens
//! (`#[doc = "..."]`), so [`without_doc_comments`] finds them by their span (a doc comment's
//! `#` covers the whole comment, an attribute's covers one character). Every token inside a
//! macro invocation (`name!(..)`, `name![..]`, `name!{..}`, `macro_rules! name {..}`) is
//! kept, `#[doc = ".."]` included: a macro can expand an argument into anything, and
//! `#[doc = "normal"]` against `#[doc = "changed"]` in a macro input is an edit. So is every
//! token inside a non-doc attribute (`#[derive(..)]`, `#[serde(..)]`, `#[cfg_attr(..)]`),
//! and an explicit `#[doc = ".."]` attribute anywhere.
//!
//! [`flat_tokens`] is the same stream flattened with each token's source line, for
//! matching a cited fragment of source as a token sequence ([`find_fragment`]).
#![cfg(test)]

use proc_macro2::{Delimiter, Group, Spacing, TokenStream, TokenTree};

/// Keywords after which `!` is the unary operator, not a macro call's bang (`if !(a)`).
const UNARY_BANG_AFTER: [&str; 12] = [
    "if", "while", "return", "match", "in", "else", "break", "let", "move", "for", "loop", "mut",
];

fn rebuilt(group: &Group, stream: TokenStream) -> TokenTree {
    let mut rebuilt = Group::new(group.delimiter(), stream);
    rebuilt.set_span(group.span());
    TokenTree::Group(rebuilt)
}

/// Whether the tokens at `at` are a source doc comment: `#` (and `!` for an inner one)
/// followed by `[doc ..]`, where the `#` covers more than one character (a comment's span
/// is the whole comment; an attribute's `#` is one character). Returns how many tokens it
/// spans.
fn doc_comment_len(tokens: &[TokenTree], at: usize) -> Option<usize> {
    let TokenTree::Punct(hash) = &tokens[at] else {
        return None;
    };
    if hash.as_char() != '#' {
        return None;
    }
    let mut next = at + 1;
    if matches!(tokens.get(next), Some(TokenTree::Punct(bang)) if bang.as_char() == '!') {
        next += 1;
    }
    let Some(TokenTree::Group(group)) = tokens.get(next) else {
        return None;
    };
    let is_doc = group.delimiter() == Delimiter::Bracket
        && matches!(
            group.stream().into_iter().next(),
            Some(TokenTree::Ident(ident)) if ident == "doc"
        );
    let (start, end) = (hash.span().start(), hash.span().end());
    let covers_a_comment = end.line != start.line || end.column.saturating_sub(start.column) > 1;
    (is_doc && covers_a_comment).then_some(next + 1 - at)
}

/// How many tokens from `at` are one attribute (`#[..]` or `#![..]`): `#`, an optional
/// `!`, the bracket group. `None` when the tokens are not an attribute.
fn attribute_len(tokens: &[TokenTree], at: usize) -> Option<usize> {
    let TokenTree::Punct(hash) = &tokens[at] else {
        return None;
    };
    if hash.as_char() != '#' {
        return None;
    }
    let mut next = at + 1;
    if matches!(tokens.get(next), Some(TokenTree::Punct(bang)) if bang.as_char() == '!') {
        next += 1;
    }
    matches!(
        tokens.get(next),
        Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Bracket
    )
    .then_some(next + 1 - at)
}

/// How many tokens from `at` are a macro invocation's bang and input: `!` after the
/// macro's name (a path's last identifier, not a keyword), then the delimited input, or for
/// `macro_rules! name {..}` the name and the body. A label (`'label`) is not a macro name.
fn macro_input_len(tokens: &[TokenTree], at: usize) -> Option<usize> {
    let TokenTree::Punct(bang) = &tokens[at] else {
        return None;
    };
    if bang.as_char() != '!' || bang.spacing() != Spacing::Alone || at == 0 {
        return None;
    }
    match &tokens[at - 1] {
        TokenTree::Ident(name) if !UNARY_BANG_AFTER.contains(&name.to_string().as_str()) => {}
        _ => return None,
    }
    // A lifetime is a `'` and an identifier: `break 'label !(..)` negates, it calls nothing.
    if at >= 2 && matches!(&tokens[at - 2], TokenTree::Punct(tick) if tick.as_char() == '\'') {
        return None;
    }
    match (tokens.get(at + 1), tokens.get(at + 2)) {
        (Some(TokenTree::Group(_)), _) => Some(2),
        (Some(TokenTree::Ident(_)), Some(TokenTree::Group(_))) => Some(3),
        _ => None,
    }
}

/// `tokens` without the source doc comments that sit outside a macro invocation or an
/// attribute's input. Everything else is kept as it is, spans included.
pub(crate) fn without_doc_comments(tokens: TokenStream) -> TokenStream {
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();
    let mut out: Vec<TokenTree> = Vec::new();
    let mut at = 0;
    while at < tokens.len() {
        if let Some(len) = doc_comment_len(&tokens, at) {
            at += len;
            continue;
        }
        // A non-doc attribute and a macro input are kept whole.
        if let Some(len) = attribute_len(&tokens, at).or_else(|| macro_input_len(&tokens, at)) {
            out.extend(tokens[at..at + len].iter().cloned());
            at += len;
            continue;
        }
        match &tokens[at] {
            TokenTree::Group(group) => {
                out.push(rebuilt(group, without_doc_comments(group.stream())));
            }
            other => out.push(other.clone()),
        }
        at += 1;
    }
    out.into_iter().collect()
}

/// A token stream as text with no whitespace or comment information: every other token,
/// every comma and every punctuation's spacing (`Joint` is `~`) is kept, so `(x,)` and
/// `(x)`, and `m!(&&x)` and `m!(& &x)`, differ. Only source doc comments outside macro
/// invocations are dropped (see the module documentation).
pub(crate) fn normalized_tokens(tokens: TokenStream) -> String {
    render(without_doc_comments(tokens))
}

fn render(tokens: TokenStream) -> String {
    let mut out = Vec::new();
    for tree in tokens {
        match &tree {
            TokenTree::Group(group) => {
                let (open, close) = match group.delimiter() {
                    Delimiter::Parenthesis => ("(", ")"),
                    Delimiter::Brace => ("{", "}"),
                    Delimiter::Bracket => ("[", "]"),
                    Delimiter::None => ("", ""),
                };
                out.push(format!("{open} {} {close}", render(group.stream())));
            }
            TokenTree::Punct(punct) => out.push(match punct.spacing() {
                Spacing::Joint => format!("{}~", punct.as_char()),
                Spacing::Alone => punct.as_char().to_string(),
            }),
            other => out.push(other.to_string()),
        }
    }
    out.join(" ")
}

/// FNV-1a over the normalized token stream of `item`: stable across runs and Rust
/// versions (std's hashers are not promised to be).
pub(crate) fn token_hash<T: quote::ToTokens>(item: &T) -> u64 {
    hash_text(&normalized_tokens(item.to_token_stream()))
}

pub(crate) fn hash_text(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// One token of a flattened stream: its normalized spelling (a group's delimiters are
/// tokens of their own) and the source line it starts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Flat {
    pub(crate) text: String,
    pub(crate) line: usize,
}

/// The tokens of `tokens` in source order, source doc comments outside macro invocations
/// dropped (as [`normalized_tokens`]), delimiters as tokens. Lines are those of the source
/// the stream was parsed from (`proc-macro2`'s `span-locations`); a stream built from a
/// string starts at line 1.
pub(crate) fn flat_tokens(tokens: TokenStream) -> Vec<Flat> {
    flatten(without_doc_comments(tokens))
}

fn flatten(tokens: TokenStream) -> Vec<Flat> {
    let mut out = Vec::new();
    for tree in tokens {
        match &tree {
            TokenTree::Group(group) => {
                let (open, close) = match group.delimiter() {
                    Delimiter::Parenthesis => ("(", ")"),
                    Delimiter::Brace => ("{", "}"),
                    Delimiter::Bracket => ("[", "]"),
                    Delimiter::None => ("", ""),
                };
                if !open.is_empty() {
                    out.push(Flat {
                        text: open.to_string(),
                        line: group.span_open().start().line,
                    });
                }
                out.extend(flatten(group.stream()));
                if !close.is_empty() {
                    out.push(Flat {
                        text: close.to_string(),
                        line: group.span_close().start().line,
                    });
                }
            }
            TokenTree::Punct(punct) => out.push(Flat {
                text: match punct.spacing() {
                    Spacing::Joint => format!("{}~", punct.as_char()),
                    Spacing::Alone => punct.as_char().to_string(),
                },
                line: punct.span().start().line,
            }),
            other => out.push(Flat {
                text: other.to_string(),
                line: other.span().start().line,
            }),
        }
    }
    out
}

/// The fragment `needle` as tokens, or the lexer's complaint: a fragment that is not
/// well-formed Rust tokens (an unbalanced delimiter, an unterminated string) cannot be
/// matched as a token sequence and must be written whole.
pub(crate) fn fragment_tokens(needle: &str) -> Result<Vec<Flat>, String> {
    needle
        .parse::<TokenStream>()
        .map(flat_tokens)
        .map_err(|error| format!("the fragment `{needle}` is not whole Rust tokens: {error}"))
}

/// Whether two flattened tokens are the same token. A citation matches the token, not its
/// punctuation spacing: the source is the `syn` item re-printed (a closure's `||` is two
/// separate pipes there, `&&` one joint pair), the fragment is lexed as written. The
/// pins ([`token_hash`]) keep the spacing; the citations only need to find the code.
fn same_token(source: &Flat, fragment: &Flat) -> bool {
    source.text.trim_end_matches('~') == fragment.text.trim_end_matches('~')
}

/// The index in `haystack` where `fragment` occurs as a sequence of tokens, restricted
/// to tokens whose line lies in `lines` (inclusive); `None` when it does not.
pub(crate) fn find_fragment(
    haystack: &[Flat],
    fragment: &[Flat],
    lines: (usize, usize),
) -> Option<usize> {
    if fragment.is_empty() {
        return None;
    }
    let inside: Vec<usize> = (0..haystack.len())
        .filter(|index| haystack[*index].line >= lines.0 && haystack[*index].line <= lines.1)
        .collect();
    inside.windows(fragment.len()).find_map(|window| {
        let contiguous = window.windows(2).all(|pair| pair[1] == pair[0] + 1);
        let same = window
            .iter()
            .enumerate()
            .all(|(offset, index)| same_token(&haystack[*index], &fragment[offset]));
        (contiguous && same).then_some(window[0])
    })
}

fn hash_of(code: &str) -> u64 {
    let stream: TokenStream = code.parse().expect("the test code lexes");
    hash_text(&normalized_tokens(stream))
}

/// D-S2b-10 (e), as the shell's ratchet pins it: the hash keeps commas and punctuation
/// spacing and token kinds, and ignores whitespace and comments.
#[test]
fn the_token_hash_keeps_commas_spacing_and_token_kinds() {
    assert_ne!(
        hash_of("fn f() { let _ = (x,); }"),
        hash_of("fn f() { let _ = (x); }")
    );
    assert_ne!(
        hash_of("fn f() { m!(&&x); }"),
        hash_of("fn f() { m!(& &x); }")
    );
    assert_ne!(hash_of("fn f(a: u8,) {}"), hash_of("fn f(a: u8) {}"));
    // A prefixed literal is one token and differs from an identifier and a literal.
    assert_ne!(
        hash_of("fn f() { m!(c\"x\"); }"),
        hash_of("fn f() { m!(c \"x\"); }")
    );
    assert_ne!(
        hash_of("fn f() { m!(b\"x\"); }"),
        hash_of("fn f() { m!(b \"x\"); }")
    );
    assert_ne!(
        hash_of("fn f() { if a.is_none() {} }"),
        hash_of("fn f() { if a.is_some() {} }")
    );
    assert_ne!(
        hash_of("fn f() { g(\"a // b\"); }"),
        hash_of("fn f() { g(\"a // c\"); }")
    );
    assert_ne!(
        hash_of("fn f<'a>(x: &'a u8) {}"),
        hash_of("fn f<'b>(x: &'b u8) {}")
    );
    assert_ne!(
        hash_of("fn f() { g(r#\"x\"#); }"),
        hash_of("fn f() { g(r#\"y\"#); }")
    );
    assert_ne!(hash_of("fn f() { g('a'); }"), hash_of("fn f() { g('b'); }"));
}

/// A comment is not an edit, wherever it sits: right after a `;`, between a receiver and
/// its method call, nested, a doc comment, a line comment.
#[test]
fn a_comment_changes_no_hash() {
    let plain = hash_of("fn f() { let cancelled = should_abandon; response.send(x); }");
    for commented in [
        "fn f() { let cancelled = should_abandon;/* x */ response.send(x); }",
        "fn f() { let cancelled = should_abandon; /* x */ response.send(x); }",
        "fn f() { let cancelled = should_abandon; response/* x */.send(x); }",
        "fn f() { let cancelled = should_abandon; response /* a /* nested */ b */ .send(x); }",
        "fn f() { let cancelled = should_abandon; // line\n response.send(x); }",
        "/// doc\nfn f() { /// inner\n let cancelled = should_abandon; response.send(x); }",
        "fn f() {\n    let cancelled = should_abandon;\n\n    response.send(x);\n}",
    ] {
        assert_eq!(hash_of(commented), plain, "{commented}");
    }
    assert_ne!(
        hash_of("fn f() { let cancelled = false; response.send(x); }"),
        plain,
        "a one-token edit is an edit"
    );
}

/// D-S2b-18 (2): the normalization keeps every token inside a macro invocation, `#[doc ..]`
/// attributes included, and drops only source doc comments outside one. Sol's compiled probe:
/// a macro argument `#[doc = "normal"]` -> `#[doc = "changed"]` changed what the macro
/// expands to (1 -> 2) and the hash did not move.
#[test]
fn a_doc_attribute_inside_a_macro_is_an_edit_and_a_doc_comment_outside_is_not() {
    let probe = |argument: &str| {
        format!(
            "macro_rules! pick {{ (#[doc = \"normal\"]) => {{ 1 }}; (#[doc = $x:literal]) => {{ 2 }}; }}\n\
             fn f() -> u8 {{ pick!({argument}) }}"
        )
    };
    assert_ne!(
        hash_of(&probe("#[doc = \"normal\"]")),
        hash_of(&probe("#[doc = \"changed\"]")),
        "the macro input's #[doc] is an edit"
    );
    // The body and the input of every macro form, with each delimiter.
    for (one, other) in [
        (
            "fn f() { m!(#[doc = \"a\"] x); }",
            "fn f() { m!(#[doc = \"b\"] x); }",
        ),
        (
            "fn f() { m![#[doc = \"a\"] x]; }",
            "fn f() { m![#[doc = \"b\"] x]; }",
        ),
        (
            "fn f() { m!{#[doc = \"a\"] x} }",
            "fn f() { m!{#[doc = \"b\"] x} }",
        ),
        (
            "fn f() { a::b::m!(#[doc = \"a\"]); }",
            "fn f() { a::b::m!(#[doc = \"b\"]); }",
        ),
        (
            "m!(#[doc = \"a\"] struct S;);",
            "m!(#[doc = \"b\"] struct S;);",
        ),
        (
            "macro_rules! m { () => { #[doc = \"a\"] fn f() {} } }",
            "macro_rules! m { () => { #[doc = \"b\"] fn f() {} } }",
        ),
        // Nested deeper in the input, and inside another macro's argument.
        (
            "fn f() { m!(n!({ #[doc = \"a\"] x })); }",
            "fn f() { m!(n!({ #[doc = \"b\"] x })); }",
        ),
        // A doc comment inside a macro input is the lexer's `#[doc = ..]` there: kept.
        ("fn f() { m!(/// a\n x); }", "fn f() { m!(/// b\n x); }"),
        // A non-doc attribute's input, and an explicit doc attribute, are kept.
        (
            "#[attr(#[doc = \"a\"] x)] fn f() {}",
            "#[attr(#[doc = \"b\"] x)] fn f() {}",
        ),
        ("#[doc = \"a\"] fn f() {}", "#[doc = \"b\"] fn f() {}"),
        (
            "#[cfg_attr(unix, doc = \"a\")] fn f() {}",
            "#[cfg_attr(unix, doc = \"b\")] fn f() {}",
        ),
    ] {
        assert_ne!(hash_of(one), hash_of(other), "{one}");
    }
    // Source doc comments outside a macro are not an edit: `///`, `//!`, `/** */`, `/*! */`,
    // on an item, a field, a statement-level item, and inside a block.
    let plain = hash_of("fn f() { let a = 1; m!(x); } struct S { x: u8 }");
    for commented in [
        "/// doc\nfn f() { let a = 1; m!(x); } struct S { x: u8 }",
        "//! inner\nfn f() { let a = 1; m!(x); } struct S { x: u8 }",
        "/** block doc */ fn f() { let a = 1; m!(x); } struct S { x: u8 }",
        "/*! inner block */ fn f() { let a = 1; m!(x); } struct S { x: u8 }",
        "fn f() { /// inner\n let a = 1; m!(x); } struct S { x: u8 }",
        "fn f() { let a = 1; m!(x); } struct S {\n    /// the field\n    x: u8 }",
        "fn f() { let a = 1; m!(x);/* c */ } // c\n struct S { x: u8 }",
    ] {
        assert_eq!(hash_of(commented), plain, "{commented}");
    }
    // `if !(a)` is the unary operator, not a macro: a doc comment after it is still dropped.
    assert_eq!(
        hash_of("fn f() { if !(a) { /// x\n b(); } }"),
        hash_of("fn f() { if !(a) { b(); } }")
    );
}

/// D-S2b-19 (b): `break 'label !(..)` is valid Rust in which `!` negates. The identifier
/// after a lifetime's apostrophe is a label, never a macro name, so a doc comment inside the
/// block is still a doc comment. `labeled_break_negation` is the compiled example; the test
/// hashes its source text.
#[allow(unused_doc_comments, dead_code)]
fn labeled_break_negation() -> bool {
    'label: {
        if true {
            break 'label !({
                /// a doc comment
                let ready = true;
                ready
            });
        }
        false
    }
}

#[test]
fn a_label_before_a_bang_is_not_a_macro_name() {
    let with = |doc: &str| {
        format!(
            "fn f() -> bool {{ 'label: {{ if true {{ break 'label !({{ {doc}\n let ready = true; ready }}); }} false }} }}"
        )
    };
    assert_eq!(
        hash_of(&with("/// one")),
        hash_of(&with("/// another, longer")),
        "a doc comment after `break 'label !(` is not an edit"
    );
    assert_eq!(hash_of(&with("/// one")), hash_of(&with("")));
    // The same text with a real macro name keeps its doc comment as an edit.
    let call = |doc: &str| format!("fn f() {{ label!({{ {doc}\n let ready = true; ready }}); }}");
    assert_ne!(hash_of(&call("/// one")), hash_of(&call("/// another")));
    // It compiles, and `!` negated: the break value is `!true`.
    assert!(!labeled_break_negation());
}

/// A cited fragment is matched as tokens: a comment or different whitespace inside the
/// source does not hide it, a different token does, and the cited lines bound it.
#[test]
fn a_fragment_is_matched_as_tokens_within_its_lines() {
    let source: TokenStream =
        "fn f() {\n    let a = 1;\n    response /* c */ .send(Ok(()));\n    other(\"x\");\n}\n"
            .parse()
            .expect("lexes");
    let haystack = flat_tokens(source);
    let find = |needle: &str, lines: (usize, usize)| {
        find_fragment(&haystack, &fragment_tokens(needle).expect("whole"), lines)
    };
    assert!(find("response.send(Ok(()))", (1, 5)).is_some());
    assert!(find("response.send(Ok(()))", (3, 3)).is_some());
    assert!(find("response.send(Err(()))", (1, 5)).is_none());
    assert!(
        find("response.send(Ok(()))", (4, 5)).is_none(),
        "outside the lines"
    );
    assert!(find("response . send", (3, 3)).is_some());
    assert!(find("other(\"x\")", (4, 4)).is_some());
    assert!(find("other(\"y\")", (4, 4)).is_none());
    // The fragment must be whole tokens.
    assert!(fragment_tokens("response.send(Err(\"Audio owner").is_err());
    assert!(fragment_tokens("caller_alive.store(false").is_err());
}
