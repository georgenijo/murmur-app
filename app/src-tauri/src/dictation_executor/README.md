# SP5 ratchet: what it proves, and what it does not

`ratchet_tests.rs` is a syntactic ratchet over the app crate's production source
(appendix C7.2, C13.3 #117; decisions D-S2-17, D-S2b-8, D-S2b-10). It parses every
`src/**/*.rs` file with `syn`, skips `#[cfg(test)]` items, and fails closed: a shape it
cannot prove is a failure, never a pass.

## The status-event check

`recording-status-changed` is the app's one lifecycle status event. The check pins
every production call that can carry it, per enclosing function
(`STATUS_EVENT_ALLOWLIST`), and rejects everything else:

* **Emitters are called directly.** `emit`, `emit_to`, `emit_filter`, `emit_json`,
  `emit_event`, `emit_import_json` (and their `_str` forms) appear only as the callee of a
  call or method call. Binding one to a value (`let publish = Emitter::emit;`), passing
  it or returning it fails. Macro bodies are parsed as arguments, else as statements,
  and their calls get the same checks; a body that parses as neither and names an
  emitter fails.
* **Event names resolve by proof.** A string literal, `concat!` of literals, or a path to
  a `const` / `static` declared exactly once at that module path (or imported by `use`),
  with no pattern, parameter, closure argument or `use .. as` binding the same
  identifier. Only module-level `use` items are consulted: a `use` inside a function or
  block that binds an identifier makes that identifier unresolved wherever it is
  emitted (D-S2b-12 a). `concat!` counts only as `concat!` / `std::concat!` /
  `core::concat!`; any `use .. as concat`, any import whose final name is `concat`, and
  any `macro_rules! concat` anywhere in the crate fails the ratchet (D-S2b-12 b). There
  is no value-flow analysis: a `let`, a loop variable or a call result is unresolved.
* **An unresolved emit is allowed only inside a reviewed wrapper** on
  `DYNAMIC_EMIT_ALLOWLIST`, pinned by token hash. Each wrapper name has one entry and
  one destination, and every caller of it passes a name that resolves and that the entry
  declares. The owned idle emitter (`OwnedIdleEmit`) takes no event parameter and emits
  one fixed event; its two forwarding bodies are pinned (`PINNED_FUNCTIONS`).
* **Pins keep what matters.** The token hash strips only whitespace and comments (doc
  comments included). Every comma and every punctuation token's spacing is kept, so
  `(x,)` differs from `(x)` and `m!(&&x)` from `m!(& &x)`. Editing a pinned function
  means re-reviewing it and re-pinning the hash the failure message prints.

## The walk

A source file is skipped only when it starts with its own `#![cfg(test)]`. Any
`#[path = ".."]` or `cfg_attr(.., path = ..)` in production source fails, except the
seven reviewed declarations in `PATH_ATTR_ALLOWLIST` (none of their targets is
self-gated); a path alias is how a self-gated file could be compiled into the app
and skipped by the walk. A `#[path]` module's real module path differs from the one the
scan derives from the file name, which fails closed for constants declared in it. A
`#[path` or `path =` token sequence inside any `macro_rules!` body fails too, since a
macro could expand to a module declaration that loads a file outside the walk
(D-S2b-12 c).

## Threat model

The ratchet catches **accidental changes in ordinary Rust source**: a new emit in an
unrelated file, a refactor that moves or renames an emit, a wrapper that remaps an
event, a name spelled through a constant, local or macro. It is not a defence against
code written to evade it (D-S2b-12 e). A construction that only makes sense as a
deliberate evasion of the scanner is a nit, recorded here, and does not block. Known
examples, rejected where cheap and otherwise accepted:

* aliasing a macro to a built-in's name (`use m as concat`; rejected by blanket rule);
* generating module attributes from macros (`macro_rules!` bodies with `#[path`;
  rejected by blanket rule; other macro-generated attributes are not tracked);
* shadowing a constant through scoped imports that reuse the name (a function- or
  block-scoped `use` makes the identifier unresolved; nothing finer is tracked);
* glob imports (`use m::*`), which the resolver does not follow.

Out of scope entirely:

* proc-macro and build-script output;
* `include!`d files (the four `capture_*_tests.rs` files are test-only and scanned
  as ordinary files);
* FFI, `unsafe` function-pointer casts and transmutes;
* dynamic dispatch through values the scan cannot see (it has no type information).

The replays in `commands/recording_bug_replay_tests.rs` are the behavioural half: they
run the real code and fail on a late or wrong status event, which the ratchet cannot
prove for the shapes above.
