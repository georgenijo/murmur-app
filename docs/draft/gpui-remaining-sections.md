# GPUI 3.5e: Remaining sections

Issue #1006 / epic #1011. Layer 17 starts from `e4d88955` on
`gpui/16-meetings` (#1034), following History, Settings, Onboarding/Home and
Meetings. This is a draft internal native-parity layer. The Phase 2 main host
injects existing service adapters; the shipping React routes and tests remain.

## Presentation and ownership

| Area | Native modules and behavior |
| --- | --- |
| Command palette | `palette/`: current-runtime registry labels/IDs, enabled next-recording Modes, empty-history exclusions, deterministic tiered token matching, wrapping arrows/Control-N/P, Enter/Escape/Tab, AX options and caller-focus return. `shortcut` preserves Control-letter editing inside fields and modified shortcuts. |
| Insights | `insights/`: durable `completion::Stats`, local calendar date, totals, typing time at 40 WPM minus dictation, Mode names/removed Modes, meetings/corrections/Paste Last, yesterday-inclusive streak, eight-week heatmap, seven-day words bars and WPM line. Shared charts expose exact values through keyboard, hover and AX. |
| Models | `models/`: complete existing catalog, support/capabilities, readiness, guidance, selected/active/busy removal locks, deliberate confirmation, explicit install/retry and exact request-ID cancellation. Native runtime generations reject older row updates. No automatic selection/fallback/download. |
| Knowledge | `knowledge/`: server search/type/state/scope filters, 50-record pages, recovery/unavailable retry, CRUD, record revisions, reviewed import/export, individual confirmation and typed DELETE for bulk/delete-all. Complete filtered-set collection validates revisions, totals, offsets, uniqueness and bounds before writes. Sequential writes retain original revisions, skip unchanged records, continue failures and stop subsequent writes on detach. |
| Saved spellings | `customize/`: saved canonical terms/aliases, preserved scopes, enable/remove/edit, production matcher/CLI preview, strict 256 KiB file parser, exact duplicates/conflicts, stale-preview rejection, reserved/ambiguous/cyclic alias refusal. The existing configure owner repeats cross-domain alias/Voice Command validation atomically before persistence. |
| Voice Commands | `customize/`: typed replacements/snippets, app/global scope, duplicate with blank trigger, enable/delete, snippet variables, per-command clipboard permission, separate default-off preview read consent and discard confirmation. Preview invokes the existing matcher and never writes/pastes. Global disabled state links to its Settings switch without enabling it. |
| App overrides and project vocabulary | `customize/`: explicit running-app picker/manual profile, tri-state overrides, writing style, default-off IDE context, four native-picked roots, refresh/clear/remove, correlated explicit code scan/cancel, bounded searchable/sortable term pages and explicit Copy. Index contents remain memory-only. |
| Modes | `modes/`: seven read-only built-ins, create/duplicate/edit/disable/delete, policy/model/language fields, app bindings, default-off exact browser-host rules, rule edit/toggle/confirmed removal, explicit probe, local before/after tester, strict file exchange with stale-preview rejection. Removal clears both app and site references in one intent. |
| Appearance | `themes/`: main-thread System/Light/Dark manager and chrome, Sonic/OpenCode, saved/imported Midnight, independent light/dark ownership, reset, create/edit/duplicate/remove/export, local JSON/JSONC preview and explicit community discovery/update. |

`section::Controller` shares the bounded workspace transport, subscribes Main
before reads, and fences replies/progress by attachment, request generation and
source revision. Close/hide revokes read/panel/network leases and subscriptions;
accepted durable preferences remain with the existing service owner. Drafts stay
open on failed saves. Modes and color edits refuse a changed source revision.
Native panels, clipboard writes, running-app enumeration and browser probes are
explicit host intents. Fixture hosts have no production adapters.

The shared `data_components::focus_reveal` / `scroll_parent` changes from the
History layer are carried onto this older meetings base. Buttons, forms and
charts reveal focus through scroll containers without animated movement.

## Appearance compilation and Open VSX

The authoritative sources remain `murmur-appearance` and `murmur-theme-library`,
with the existing `theme-library.json` durability/migration owner. No new saved
keys or schema versions are introduced. Caches are derived, never trusted input.
Rust ports sRGB/OKLab/OKLCH conversion, surface ladders, accent/foreground
controls, mutable-palette semantic contrast solving and Sonic's unchanged
fixtures. Neutral fixtures generated from the retained React resolver compare
all sixteen tokens byte-for-byte for fourteen light/dark cases. OpenCode,
Midnight, low-contrast and explicit color/contrast cases are included.

The application appearance manager follows system changes only in System mode;
forced modes retain their selected half. Native chrome uses GPUI's main-thread
application-appearance API. Overlay utility palettes retain their existing
separate policy. Library selection compiles both halves; removing an active
entry preserves its compiled appearance as custom. Collection updates compare
the exact installed collection before atomic replacement.

Community search is disclosed before any request. `open_vsx::Network` is an
injected blocking-worker transport with credential-free HTTPS, exact
`open-vsx.org` origin validation, no redirects, connect/request deadlines and
streamed size caps. Search is category/license filtered and validates manifest
identity/contributions before exposing results. Opening Appearance or the
community dialog makes no request; there is no polling or automatic update.

Downloads verify SHA-256 and packaged publisher/name/version/license. The
read-only ZIP parser bounds 20 MiB compressed, 50 MiB expanded, 2,000 entries,
200:1 compression, selected JSON at 256 KiB, 40 theme declarations, inheritance
depth eight and 320 resolved files. It rejects ZIP64, traversal/absolute/Windows
paths, duplicate members, symlinks/devices, local/central metadata mismatch,
overlapping payloads, malformed compression/CRC/JSON, cycles and outside-root
includes. JSONC processing preserves quoted strings; duplicate JSON keys fail.
Only declared color JSON and includes are interpreted as data. No archive is
extracted, and no script, grammar, font, icon, binary, command, activation event
or dependency is executed. Tests use synthetic transport/archive fixtures only.

## Native evidence and gates

Gallery-gated `remaining-qa` composes the real native views with synthetic hosts.
It has six primary sections and four Knowledge drill-ins. `MURMUR_SECTION` and
`MURMUR_REMAINING_STATE` select offscreen cases; all presentation data is synthetic.
`check_gpui_goldens.py --remaining-only` renders each case twice in Light, Dark,
Increase Contrast, Midnight and an imported Orchid theme. Every process enforces
`check_inventory`, zero-image launch-only baseline and the native linkage policy;
`webkit-policy.log` retains failure inventories too. The attribution policy is
unchanged. The native palette text hook compares framework image counts before
and after native focus and text delivery.

`check_gpui_remaining_native.py` traverses and presses only its exact child PID
using AX, records only that window, and uses internal native input hooks. It
never activates another app, posts HID, requests TCC, starts VoiceOver, or reads
the real pasteboard. A missing existing grant is a missing check, not a reason
to request one. Packaged launches may retain the inherited seven-image
`_addTextInputMenuItems` Writing Tools blocker; preserve/report it separately
from additional loading caused by input focus/typing.

The separate interactive/VoiceOver pass should cover real IME/Edit-menu/undo,
keyboard shortcuts with editable fields, palette active announcements and focus
return, chart tooltips/arrow navigation, compact scrolling, reduced motion and
contrast, native file-panel cancel, download failure/cancel/retry, recovery and
bulk partial failures, original-revision conflicts, stale import previews,
per-command clipboard consent, browser denial/return, IDE generation expiry,
and theme half-selection/chrome across OS appearance changes. Real service
admission, capture generation races and installed persistence remain Phase 2
adapter acceptance flows, as in the preceding native-parity layers.

No Vitest/React test is retired and no production route is switched. Independent
Sol high review belongs to the lead; this implementation child starts no
workers, watches no PR, and never merges. Exact checks, packaged launch status,
review app and immutable evidence link are recorded in the draft PR/artifact.
