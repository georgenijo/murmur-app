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
* **Event names resolve by three forms only (D-S2b-14).** A string literal; `concat!`
  (unqualified, `std::` or `core::`) of string literals; or a path written in full as
  `crate::events::NAME`. There is **no import resolution**: a bare identifier,
  `self::`, `super::`, an imported name, a glob or an alias is unresolved, whatever it
  names, and an unresolved emit outside a pinned wrapper fails. The events module
  (`src/events.rs`) is the one module that declares the event-name constants emitted
  outside wrappers. It holds only `pub const NAME: &str = "<literal>";` items: no
  `use`, no `cfg` or `cfg_attr` on any item (docs only), no macro, no nested module,
  each name once; any violation fails. `lib.rs` must declare exactly one module named
  `events` at the crate root, out of line (`mod events;` or `pub mod events;`) with no
  `cfg`, `cfg_attr` or `path` attribute, so no inline or cfg-selected replacement can
  give `crate::events::NAME` a value the scan never read. `crate::events` is
  resolved against `src/lib.rs` only, so every other production crate root is
  validated too: the ratchet knows `src/lib.rs` and `src/main.rs`. `Cargo.toml` is
  parsed as TOML (the `toml` dev-dependency, the version the lockfile already
  resolved), not line-scanned, so spacing, comments and inline tables cannot hide a
  target: the production targets must be exactly one `[lib]` named `ui_lib` at its
  default path and one `[[bin]]` at `src/main.rs`; any other `[[bin]]`, any
  non-default `path` and any file under `src/bin/` fails. `src/main.rs` is pinned
  whole by its token hash (`MAIN_RS_HASH`, the same normalization as the wrapper
  pins): any edit fails until it is re-reviewed (it may only call `ui_lib::..`) and
  re-pinned, which also closes `include!`, `mod` and `#[path]` in the binary. The
  earlier token checks (no `mod`, `macro_rules!`, `include`, emitter identifier or
  `crate::events` path) stay as diagnostics that name the reason. Build
  scripts, examples and integration tests are not production roots and are not
  scanned. Attribute names are matched after normalizing raw identifiers (`r#path`).
  Any `use .. as concat`, any import whose final
  name is `concat`, and any `macro_rules! concat` anywhere in the crate fails too
  (D-S2b-12 b). A new emitted constant goes in the events module and is emitted as
  `crate::events::NAME`.
* **An unresolved emit is allowed only inside a reviewed wrapper** on
  `DYNAMIC_EMIT_ALLOWLIST`, pinned by token hash. Each wrapper name has one entry and
  one destination, and every caller of it passes a name that resolves and that the entry
  declares. The owned idle emitter (`OwnedIdleEmit`) takes no event parameter and emits
  one fixed event; its two forwarding bodies are pinned (`PINNED_FUNCTIONS`).
* **Pins keep what matters.** The token hash strips only whitespace, comments and
  source doc comments (`///`, `//!`, `/** */`, `/*! */`) outside macro invocations and
  attributes. Every token inside a macro invocation or a non-doc attribute is kept, and
  so is an explicit `#[doc = ".."]` attribute: a macro can expand a `#[doc = "a"]`
  argument into anything, so `"a"` to `"b"` there is an edit. Every comma and every punctuation token's spacing is kept, so
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
* shadowing or re-binding a constant through imports, globs or `cfg`-gated
  declarations (the ratchet follows no import, so only the three forms above resolve).

Out of scope entirely:

* proc-macro and build-script output;
* `include!`d files (the four `capture_*_tests.rs` files are test-only and scanned
  as ordinary files);
* FFI, `unsafe` function-pointer casts and transmutes;
* dynamic dispatch through values the scan cannot see (it has no type information).

The replays in `commands/recording_bug_replay_tests.rs` are the behavioural half: they
run the real code and fail on a late or wrong status event, which the ratchet cannot
prove for the shapes above.
