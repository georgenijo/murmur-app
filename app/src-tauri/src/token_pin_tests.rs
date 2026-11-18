//! Test-only token normalization shared by the source pins (D-S2b-15 (b)).
//!
//! The functions below are the shell ratchet's (`dictation_executor/ratchet_tests.rs` in
//! #1152: `normalized_tokens`, `token_hash`) copied verbatim, so a pin here and a pin
//! there agree on what an edit is. #1152 can delete its copy and `use` this module once
//! both have merged; until then the two are identical by construction (the tests at the
//! bottom pin the behaviour both rely on).
//!
//! A token stream is normalized to text with **no whitespace or comment information**:
//! every other token, every comma and every punctuation's spacing (`Joint` is `~`) is
//! kept, so `(x,)` and `(x)`, `m!(&&x)` and `m!(& &x)`, and `c"x"` and `c "x"` differ,
//! and `x; /* review */ y` hashes like `x; y`.
//!
//! [`flat_tokens`] is the same stream flattened with each token's source line, for
//! matching a cited fragment of source as a token sequence ([`find_fragment`]).

use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};

/// A token stream as text with no whitespace or comment (doc comment) information:
/// every other token, every comma and every punctuation's spacing (`Joint` is `~`) is
/// kept, so `(x,)` and `(x)`, and `m!(&&x)` and `m!(& &x)`, differ.
pub(crate) fn normalized_tokens(tokens: TokenStream) -> String {
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
                out.push(format!(
                    "{open} {} {close}",
                    normalized_tokens(group.stream())
                ));
            }
            TokenTree::Punct(punct) => out.push(match punct.spacing() {
                Spacing::Joint => format!("{}~", punct.as_char()),
                Spacing::Alone => punct.as_char().to_string(),
            }),
            other => out.push(other.to_string()),
        }
        at += 1;
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

/// The tokens of `tokens` in source order, doc comments dropped, delimiters as tokens.
/// Lines are those of the source the stream was parsed from (`proc-macro2`'s
/// `span-locations`); a stream built from a string starts at line 1.
pub(crate) fn flat_tokens(tokens: TokenStream) -> Vec<Flat> {
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();
    let mut out = Vec::new();
    let mut at = 0;
    while at < tokens.len() {
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
                if !open.is_empty() {
                    out.push(Flat {
                        text: open.to_string(),
                        line: group.span_open().start().line,
                    });
                }
                out.extend(flat_tokens(group.stream()));
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
        at += 1;
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
