# GPUI accessible component layer

Issue #998, Phase 3.1 of #1011; stack 09, based on
`gpui/08-host-neutral-updater`. This is a development UI beside the shipping
Tauri app. No window, command, permission, persistence owner or event route in
that app is switched. No existing Vitest or Playwright coverage is retired.

## Layout and dependency boundary

`app/gpui` is a separate Cargo workspace and lockfile. Its `murmur-ui` library
exports reusable controls, and its feature-gated `control-gallery` binary uses
synthetic data. Later ports can add `windows/overlay`, `windows/preview`,
`windows/diagnostics` and `windows/main` without depending on the gallery.
The only application dependency points **UI → murmur-core** through a path
dependency. Core stays GPUI-free and Tauri-free; the all-target/feature dependency
guard checks both families. The shipping Tauri workspace and its toolchain do
not acquire GPUI dependencies.

GPUI is vendored from the same frozen revision; gpui_platform, gpui_macros
and sum_tree still resolve to exactly
`654efae441bc8eb117d0042bb28cd7cf3570bdc9`; Rust **1.98.1** is pinned in
`app/gpui/rust-toolchain.toml`. Preserve this pin until a separate decision
changes it. The matching Zed reqwest fork is pinned to
`33bc764aa15ff7b200bf7c93bd96e24878d53e14`. The
[Phase 0 decision](../decisions/2026-11-05-gpui-phase-0-exit.md) remains the
route and budget authority. Its frozen numbers are unchanged; this component
layer does not claim to meet a later screen's memory or hardware acceptance.

## Upstream port and patch record

The starting point is the successful Phase 0 Mac mini port, copied read-only
from `~/Builds/gpui-phase0/0b-settings-history`. Its scratch directory is not
modified. Four Apache-2.0 GPUI Kit v0.9.0 crates are included: `gpui-base`,
`gpui-component`, `gpui-component-macros`, `gpui-kit-assets`. The upstream tag
commit is `7591a35eced761e3613c01e71ff04c10129dae1a`. Only those GPUI Kit crates and
required assets/locales are vendored; no shell, story, webview, examples or
benchmark implementation is included. Zed's GPL `ui`, `editor`, `ui_input`
crates are neither dependencies nor copied source. Upstream Apache licenses,
AccessKit's MIT/Apache licenses, Lucide's ISC license and upstream documentation
attribution are retained. See [third-party notices](../../THIRD_PARTY_NOTICES.md).

| Record | Change and maintenance rule |
|---|---|
| `app/gpui/patches/component-port.patch` | Original proven dependency rewiring (21 errors) and the one macro resolver line recognizing the real `gpui` package. The ported dependency tables now live in the native workspace manifest. |
| `app/gpui/patches/phase0-accessibility.patch` | Original 89-line focus hook, scratch input bridge and AccessKit patch. Kept as provenance, not a script to apply wholesale to gpui-component. |
| `app/gpui/native/ax_focus_fix.m` | Production hook forwards GPUIApplication and both GPUIWindow/GPUIPanel's `accessibilityFocusedUIElement` to the window content view's AccessKit adapter, with the original application fallback. Window legacy AX attribute names/value lookup also exposes `AXFocusedUIElement`, preserving AppKit’s other attributes. Called idempotently on GPUI's main thread after platform initialization. No Tauri process installs it. |
| `app/gpui/vendor/accesskit_macos/src/node.rs` | Phase 0 selected-text setter bridge; exposes `setAccessibilitySelectedText:` only for an editable text-range node advertising ReplaceSelectedText. |
| `app/gpui/patches/accesskit-expanded.patch` | Additive to the Phase 0 adapter: exposes `isAccessibilityExpanded` only for nodes with an expanded property and emits AppKit row-expanded/row-collapsed notifications on state changes. Runner tests reverse to the recorded base hashes and reproduce both files byte-for-byte. |
| `app/gpui/patches/component-accessibility.patch` | Ports the scratch text-range bridge to the component's existing editor; adds TextRun, UTF-8 grapheme lengths, directed selection, SetTextSelection and ReplaceSelectedText. Disabled editor focus remains dispatchable but is excluded from Tab order. |
| `app/gpui/patches/component-review-fixes.patch` | Additive patch after `component-port.patch` and `component-accessibility.patch`: disabled-switch dispatch binding, current-layout single-line AX range geometry with source bidi and glyph-order guards, the existing `unicode-bidi` dependency wired into `gpui-base`, and explicit placeholder color. |
| `app/gpui/patches/gpui-manifest.patch` | Makes only the Apache GPUI crate self-contained, resolving inherited workspace dependency tables against the unchanged upstream revision. Examples, benches and upstream test-only assets are not vendored. |
| `app/gpui/patches/gpui-deferred-accessibility.patch` | Production fix: deferred draws capture their AX ancestry, append children to the finalized parent, and retain the focused-ancestor check. Ordinary builds use it through the workspace Cargo git-source patch. |
| `app/gpui/patches/accesskit-press-probe.patch` | Additive after the AccessKit section of `phase0-accessibility.patch`: extracts the existing native AXPress dispatch into a shared handler and adds a feature-gated windowless role/title/press probe. Production action behavior is unchanged. |
| `app/gpui/patches/test-accessibility.patch` | Test transport only: retain TestWindow AX callbacks/tree updates, expose activation, tree retrieval and action dispatch on TestAppContext. Production tree building, focus calculation and action handlers remain unchanged. |

The component editor already owns UTF-16 conversion, IME composition, clipboard,
undo, pointer selection and text editing. Murmur adds AX metadata to that owner,
never another editor or another focus handle. Selection requests must name its
actual TextRun and valid character indices; reversed selections stay reversed.
Edit actions recheck current editability, including requests queued before a
control became disabled. Read-only fields allow selection; masked fields never
publish their text, runs or selections. Graphemes exceeding AccessKit's u8 length
limit split at Unicode scalar boundaries instead of overflowing a byte length.

Single-line text ranges use the current frame's shaped character boundaries,
line height, alignment and horizontally scrolled paint origin, scaled into
AccessKit's physical window coordinates. The editor fills that geometry during
prepaint before its parent's synthetic AX children are emitted, including on
first layout and after text/scroll changes. Consumer tests exercise non-ASCII
substring and caret bounds and compare the caret to the painted editor layout.
RTL/mixed-direction fallback and LTR character-bound tests use the pinned
CoreText shaper directly in test windows, without a native platform or NSWindow;
the default no-op test shaper cannot reproduce glyph reordering.
Range geometry first checks the source text's Unicode bidi classes using the
already-resolved `unicode-bidi` 0.3.18 dependency. Any strong RTL character
(R or AL, including RLM U+200F and ALM U+061C) or explicit embedding, override,
pop or isolate control (U+202A–U+202E, U+2066–U+2069) disables ranges. Sorted
glyphs alone cannot establish direction: CoreText can shape Arabic `لا` into
one glyph whose indices and character positions remain nondecreasing despite
a zero-width character. The same issue can occur in `abc لا def`.
As an additional safety net, range geometry requires source-ordered shaped glyph
byte indices across font runs before calling `x_for_index`: that API can map
reversed RTL glyphs to nondecreasing but incorrect character positions. Equal
glyph indices within one LTR cluster remain valid, preserving combining
characters and emoji ZWJ sequences. CoreText/emitted-node regressions cover
pure and mixed lam-alef, Hebrew, all eleven controls above, and LTR ASCII,
accented text, CJK, combining marks and emoji with character bounds matched to
painted carets.
Multi-line, RTL/mixed-direction, explicitly bidi-controlled and nonmonotonic layouts require
multiple visual runs; until supported, they expose AXValue without TextRun,
TextSelection, SetTextSelection or ReplaceSelectedText. They do not advertise zero-geometry
range APIs. Masked fields continue to expose none of these values.

The review-fixes patch applies after the existing port and accessibility patches;
it records every additional GPUI Kit change. To refresh it, diff the changed
files against the recorded pre-review vendored tree. Preserve the earlier patch
records rather than silently absorbing new hunks into their provenance. Do not edit shared Cargo git checkouts. A pin update must
re-run the tree/action tests, the offscreen captures and the native focus and
VoiceOver pass before discarding any patch.

## Control inventory and design-system mapping

Use these controls inside a `gpui_base::Root` after `murmur_ui::init`. Root
provides Tab/Shift-Tab dispatch, selection/copy plumbing and popup infrastructure.
The gallery and the interaction harness use that same root. Screen owners
subscribe to `SelectionChanged`, write controlled toggle values and notify;
they retain subscriptions for the screen's lifetime.

| API | Design system and behavior |
|---|---|
| `Button` | `FormControls` / `.ui-form-button`: Primary, Secondary, Ghost, Danger; 32/28/24px, 8px corners, border-compensated 11/9/7px insets, 14/20px medium labels; enabled, hover, focus, disabled states. Space activates on release. Return is opt-in for a recoverable confirming button; Danger can never become a Return default. Held, modified and composing Return are rejected. |
| `Toggle` | `SettingToggle` / `--ui-switch-*`: 30×18px track, 14px knob, 2px inset, 12px travel; controlled on/off, disabled, visible focus, 150ms positional transition. |
| `TextField`, `TextFieldState`, `InputEvent` | `FormControls` / `--ui-input-*`: real component editor, 30px height, 11px inset, 8px corners, input border; empty placeholder, populated, read-only, disabled and focused states. |
| `Choices(ChoiceKind::Dropdown)` | `Select`: named combobox, selected display value, bounded scrollable popup, disabled options, explicit selection, Escape cancellation, outside dismissal and focus restoration. Placement uses the unchanged ported Base Popup positioner and deferred popup surface. |
| `Choices(ChoiceKind::List)` | `.history-list`: one Tab stop, 5px gap, named options, visible active and selected states, scrolling to the active row, Home/End and wrapping arrows skipping disabled options. This is a selection primitive, not the later History transcript model or virtualized screen. |
| `Choices(ChoiceKind::Navigation)` | Settings navigation: 32px rows, 8px radius, 10px inset, 14/20px medium type; selected surface without a shadow; manual selection with arrows plus Space/Return and a Tab exit. Layout-specific 240/56px rails belong to the main-window port. |
| `Motion`, `DisplayPreferences`, `Palette` | Shared fast/standard, switch, popup and search-pulse policies; Reduce Motion collapses scale, travel and duration to the final state. Increase Contrast strengthens borders and secondary text; Reduce Transparency makes popup surfaces opaque. |

`events.rs` contains one typed `gpui::Context::<T>::emit(cx, event)` adapter.
The repository's Tauri transport guard permits only that exact UFCS call there;
a method call, Tauri Emitter, extra payload argument or different receiver is
still rejected. Authored native code, manifests and source inclusions remain
scanned. Only the exact vendored GPUI/GPUI Kit/AccessKit roots and native Cargo target
output are added to the existing third-party/build-output inventory exclusions.
Rust inclusion from maintained code into an excluded tree remains forbidden.
The new guard tests exercise these rejection paths as well as the local event
adapter. No Tauri event destination or restart exception is added.

`tokens.rs` owns geometry and semantic role mapping; the default light/dark
Sonic colors match `app/src/lib/appearance/palettes.ts`. `Theme::update` is the
injection boundary for the future appearance port, not a new persistence owner.
Components never select literal colors. A changed theme can replace semantic
roles without rewriting screen code. No imported-theme parser, downloader or
appearance-document migration is introduced here.

TextField passes the preference-aware secondary text color directly to the
component editor's placeholder style. Tests inspect the color used to shape an
empty field across preference changes.

Focused controls show an accent border and ring; disabled opacity is 64%.
Normal switch motion uses Base transitions, whose engine also reads
`App::reduce_motion`. The native initializer reads NSWorkspace preferences and NSApplication’s
effective appearance on the main thread and checks for changes every 500ms,
invalidating windows only when values change. Light/dark changes replace the
semantic theme live; high-contrast Aqua variants select the corresponding base
appearance. The gallery follows this path, with explicit light/dark injection
reserved for offscreen goldens. Deterministic/headless hosts inject preferences through
`init_with_preferences` and do not start that OS watcher. Hosts must call the
initializer once, not per window or render.

## Accessibility contract

Every public control constructor requires a non-empty accessible name. Visible
switch state is a value, not an extra “on/off” phrase in the name.

| Control | AX role, value, actions and focus |
|---|---|
| Button | Button; mandatory name; Focus and Click when enabled; disabled flag and no activation when disabled. One focus owner; Tab skips disabled controls and can leave a newly disabled focused control. |
| Toggle | Switch; name and true/false toggled value; Focus and Click change the owner value exactly once; disabled excludes activation and traversal. |
| TextField | TextInput; name, text value, placeholder, TextRun and directed TextSelection; Focus, SetValue, SetTextSelection and ReplaceSelectedText as permitted by current state. Keyboard selection and AX selection use the same editor. Masked text is not exposed. |
| Dropdown | ComboBox with name, display value and expanded state; named ListBox/ListBoxOption children while open; Click opens and option Click selects; focused combobox reports the active option as the AX focused descendant while open. Escape, Tab/Shift-Tab, blur and outside dismissal cancel without committing; reopening resets active state to the selected or first enabled option. |
| List | ListBox and ListBoxOption; named options, selected value and disabled flags; Click selects enabled options; focused composite reports active descendant. |
| Navigation | TabList and Tab; names and selected state; Click activates an enabled destination; arrows/Home/End move active focus and Space/Return commit. One Tab stop and visible active focus. |

The production GPUI patch retains the AX ancestor IDs when scheduling deferred
content. During deferred prepaint it appends new AX children to that already
finalized parent and uses the captured ancestors when validating active focus.
The dropdown keeps Base Popup's existing drawing, hitbox stacking and Tab order.
Deferring the entire combobox changes traversal order; rendering its popup inline
loses overlay stacking. These UI-only alternatives cannot preserve all three
contracts at the pinned revision. Tests assert the descendant path and
TreeUpdate.focus before and after arrowing, plus normal forward/reverse Tab.
No GPUI revision is changed. Only its Apache crate is copied under
`app/gpui/vendor/gpui`; the manifest and source patches reproduce that tree from
the pinned source. The production workspace's `[patch]` applies it to library,
gallery and future hosts. No shared Cargo checkout is edited.

The tests assert the real emitted AccessKit nodes, supported actions and
focused node, then dispatch actions back through the engine. They do not assert
VoiceOver's spoken output. The application-level AppKit focus hook remains a
native acceptance check; a headless tree cannot establish AXFocusedUIElement
on a real key NSWindow.

## Automated checks

From the repository root, on macOS with Rust 1.98.1 and Python 3.11+:

```sh
export RUSTUP_TOOLCHAIN=1.98.1
(cd app/gpui && cargo check --locked -p murmur-ui --features gallery)
(cd app/gpui && cargo test --locked -p murmur-ui)
python3 scripts/test_gpui_native.py
python3 scripts/test_gpui.py -- --test-threads=1
python3 scripts/check_gpui_goldens.py
(cd app/gpui && cargo clippy --locked -p murmur-ui --all-targets --features gallery -- -D warnings)
(cd app/gpui && cargo fmt -p murmur-ui -- --check)
python3 scripts/check_core_dependencies.py
```

The native bridge tests verify process-local effective appearances without
windows, activation or System Settings changes. The focus shim test compiles
the production Objective-C source against inert
NSObject-based GPUIWindow/GPUIPanel/GPUIApplication doubles. It asserts both
window classes forward direct selectors and legacy AX attribute lookups to the
content adapter, preserve other attributes, and retain window/application
fallbacks. It creates no AppKit windows and does not drive any native UI.

The AX runner also runs the vendored adapter’s expansion-notification unit test.
It validates the production vendored source path and revision marker,
then copies that Apache-2.0 crate to the task target directory and adds only the
test transport patch. The production ancestry fix is already present in that
copy. Cargo's source patch is redirected to the private copy only for the test
invocation. The original lockfile is restored in `finally`, even on failure.
Ordinary library and gallery builds use the production vendored GPUI;
`a11y-tests` is a harness-only feature and should be run through that script.

Goldens render every button variant/size and disabled state, switches on/off,
empty/populated/read-only/disabled fields, dropdown, list and navigation, in
light, dark and increased-contrast modes. Each mode has resting, hover, pressed, button-focus,
switch-focus, field-focus, closed/open-dropdown, list-focus and navigation-focus
captures. Every capture is rendered twice and compared with strict byte equality.
Outputs and a hash/OS receipt live under `$CARGO_TARGET_DIR/goldens` (or the
workspace target directory). They are synthetic review artifacts, not a
cross-OS pixel baseline: system font/Metal rasterization can vary by OS/scale.
CI uploads all images and fails any same-host nondeterminism or blank frame.

The separate `gpui-macos` CI job uses Rust 1.98.1; existing Tauri jobs keep
1.96.0. It is included in `ci-pass`, and path filtering covers this workspace,
its test scripts and shared core. The existing advisory audit now separately
scans the native lockfile, preserving findings even if the Tauri audit fails.
The two newer Tauri lints are fixed directly (`as_chunks::<4>()`, byte string
iteration); no blanket lint allowance is added.

## Feature matrix and test retirement

This layer supplies primitives for O03/O06, H02–H05, S01–S04/S07 and later
settings controls, A01/A02, V01–V04 and G01–G05. It does not mark any complete
screen row passed in the [feature matrix](gpui-feature-matrix.md). Each later
screen PR must map its own concrete interactions and AX/golden tests to those
rows before retiring a React assertion. Keep every current Vitest/Playwright
suite through this PR. No fixture is deleted or relocated merely because a
component-level replacement exists.

## Verification receipt (Mac mini, 2026-11-06)

- GPUI: **25 passing tests** — 13 keyboard/pointer/focus and disabled-state
  interactions, 9 emitted-tree/AX action cases, 3 motion/preference cases.
  Pointer hitboxes account for the test display's actual scale factor.
- Offscreen Metal: **30 cases, 60 captures**, covering three appearance/contrast
  variants and ten resting/hover/press/focus/popup states. Each pair is byte-identical
  on macOS 27.0.1. The script also rejects blank images and indistinguishable
  resting, hover, press or keyboard-focus states.
- Existing frontend: TypeScript and **1,833 Vitest tests** pass on Linux.
- Existing core: **260 tests** pass on Linux; capture-helper protocol: **13** pass.
- Tauri on Rust 1.98.1: default **1,584 pass / 16 ignored**; shutdown QA config
  with `shutdown-test-support`: **1,595 pass / 16 ignored**. Both include the
  windowless main-thread queue and AppKit termination harnesses.
- GPUI and both Tauri configurations pass strict Clippy with `-D warnings`;
  the two newer lint allowances are unnecessary after the direct source fixes.
  `check_restart_clippy.py` passes all **29** foreign/resolved fixtures.
- Core dependency boundary, reference docs, Markdown links, rustfmt, and
  **41 workflow-policy tests**, **17 transport-guard tests**, and **22 restart-guard tests** pass. Existing dependency warnings are reported
  by their owning upstream crates; no root lint suppression was added.

The capture-helper publisher assumes its fixed workspace `target/debug` path,
so its preparation command used `env -u CARGO_TARGET_DIR`; subsequent Cargo
checks used the requested private `~/Builds/gpui-stack-37-target` directory.
Generated Tauri schemas are restored rather than committed. No live app window,
VoiceOver session or production profile is used by this verification.

## Review-fix verification receipt (Mac mini, checkout 38)

On 2026-11-06, Rust 1.98.1, an isolated checkout and private target directory:

- `cargo fmt --check`: pass.
- `cargo check --locked -p murmur-ui --features gallery`: pass.
- `cargo clippy --locked -p murmur-ui --features gallery --all-targets -- -D warnings`: pass.
- `python3 scripts/test_gpui.py -- --test-threads=1`: **31 passed**, zero
  failures/ignored; doc tests: zero. Includes emitted ancestry/focus while
  arrowing, nested deferred popups, disabled focused-switch traversal,
  consumer substring/caret geometry with horizontal scrolling, cancellation
  reopening, actual shaped placeholder color and unsupported multiline APIs.
- `python3 -m unittest scripts/test_test_gpui.py scripts/test_gpui_native.py`:
  **4 passed** (three runner-source checks, one compiled native shim test).
- `python3 scripts/check_gpui_goldens.py`: **30 cases / 60 captures** pass
  byte equality, nonblank and interaction-state distinction checks. The added
  nested-popup case subsequently changed test code only; all 31 tests and
  compile/lint/format gates passed on that revision.
- Python workflow, Markdown, runner-source and transport-guard suites:
  **87 passed / 260 subtests passed** on Linux. Maintained Markdown links
  (72 docs), reference docs, workflow policy, core dependency and transport /
  restart boundary checks pass.
- Reconstructed all **102 included GPUI files** byte-for-byte from the frozen
  upstream source plus the manifest and production AX patches. Reconstructed
  the **four additionally changed GPUI Kit files** from the pre-review vendor
  tree plus `component-review-fixes.patch`; forward/reverse patch checks pass.

The new standalone GPUI manifest reports upstream's `rust_analyzer` cfg warning;
the existing test transport methods also retain their missing-doc warnings.
Strict Clippy for `murmur-ui` passes; no root lint suppression is added.
No live GUI interaction was performed. Native window/panel AX focus,
VoiceOver, real text-range frames (including Retina), popup edge placement and
preference changes remain in the native QA handoff below. The task checkout
and target directory are removed after these headless gates.

## RTL ligature review-fix receipt (Mac mini, checkout 40)

On 2026-11-06, Rust 1.98.1, fresh `~/Builds/gpui-checkout-40` and its own
`~/Builds/gpui-checkout-40-target`, using Fleet through opti and headless only:

- The new CoreText lam-alef regression **failed on d194e6b8**: `لا` still
  advertised ranges. The same regression passes with the source bidi guard.
- `cargo tree --locked -i unicode-bidi` confirmed **0.3.18** already resolved
  through `usvg`; the fix adds only a direct dependency edge in `gpui-base`.
- `cargo fmt --check`, gallery `cargo check --locked`, and gallery/all-target
  `cargo clippy --locked -- -D warnings`: **all three pass**.
- `python3 scripts/test_gpui.py -- --test-threads=1`: **36 passed**, zero
  failures/ignored; zero doc tests. Includes pure/mixed lam-alef, Hebrew, all
  **11** bidi controls and unchanged LTR character bounds against painted carets.
- Runner-source and compiled native-shim unittest suite: **4 passed**.
- Offscreen Metal goldens: **30 cases / 60 captures**, each pair byte-identical;
  nonblank and interaction-state distinction checks pass.
- Reproduced **102 GPUI files** and all **6 component review-patch files**
  byte-for-byte from their pinned upstreams plus recorded patches. The six
  component files also match the recorded pre-review tree after reversing the
  review patch; forward/reverse application checks pass.
- Maintained Markdown links: **72 docs** checked; `git diff --check` passes.

No live GUI or production data was used. Checkout 40 and its private target
are removed after verification; native QA remains with the handoff below.

## Dependency audit receipt

`cargo-audit` 0.22.2 against the native lockfile on 2026-11-06 reports **zero
security vulnerabilities and nine unmaintained-package warnings**. Six advisory
IDs are new relative to the Tauri lockfile: bincode 1.3.3
(RUSTSEC-2025-0141), instant 0.1.13 (2024-0384), paste 1.0.15 (2024-0436),
rustls-pemfile 2.2.0 (2025-0134), rustybuzz 0.20.1 (2026-0206), and ttf-parser
0.25.1 (2026-0192). The other three maintenance IDs (zbus_names, zvariant,
zvariant_derive) already occur in the shipping dependency graph. These are
reported, not ignored or silenced. Altering pinned engine/component internals
solely to replace their maintenance dependencies is deferred to a dependency
upgrade with its own compatibility verification. The existing Tauri audit's
security findings are unchanged by this PR; CI still displays them separately.

## Native QA handoff (not claimed by automated checks)

The Mac mini GUI is reserved by another worker. This task uses only deterministic
test windows and offscreen Metal textures. Do not launch the gallery normally
until the parent owns that GUI session. Never use production Murmur data or
`/Applications/Murmur.app` for this pass.

When that session is available, build the exact PR head and run
`cargo run --locked -p murmur-ui --features gallery --bin control-gallery` from
`app/gpui`. Use a synthetic isolated build. Record the revision, OS, scale,
appearance and settings for each result:

1. Tab/Shift-Tab through every enabled control and back; inspect a visible ring,
   skip disabled controls, and leave a control after disabling it. Exercise
   pointer hover/press/release, Space and allowed/suppressed Return independently.
2. Toggle on/off with keyboard, pointer and VoiceOver. Check one state change
   per activation and no response when disabled.
3. Edit fields; select/reverse selection, copy/cut/paste, Select All and undo/redo.
   Inspect AXValue, AXSelectedText, AXSelectedTextRange and settable attributes.
   Confirm app/window AXFocusedUIElement is the actual editor after each move,
   including after returning from another app. Test disabled/read-only rejection.
4. Open dropdowns by keyboard, pointer and VoiceOver; move, scroll, Home/End,
   choose, Escape, click outside, Tab and Shift-Tab out. Check restored focus,
   unclipped placement near every window edge and no accidental commit.
5. Operate list and navigation with arrows/Home/End, Space/Return and VoiceOver;
   inspect names, roles, selected values, actions and active focused descendants.
6. Repeat in light/dark; enable Reduce Motion while open and observe immediate
   switch/feedback behavior; toggle Increase Contrast and Reduce Transparency
   while open and inspect borders/text/opaque popups. Check compact/narrow layouts.

George's MacBook pass additionally owns **VoiceOver speech and rotor behavior,
Japanese IME candidate/commit/cancel and marked-text Return, Retina (2×) focus
and typography**, and supported-macOS/hardware acceptance. The larger Phase 0
hardware/dictation/notch/second-display gates remain with their screen ports.
Do not equate AX tree assertions, 1× goldens or synthetic key tests with these
native checks. No live AX or VoiceOver automation was run in this task.
