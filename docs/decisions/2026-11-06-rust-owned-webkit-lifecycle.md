# Rust-owned state and WebKit data lifecycle

Date: 2026-11-06. Status: adopted for GPUI stack step 1.5 (#989).

This decision supersedes DECISIONS.md 2026-08-09, "Rust-owned settings.json is
the source of truth; localStorage is a cache", and 2026-07-20 item 5,
"Cross-window settings stay localStorage + settings-changed events". The other
four geometry/lifecycle decisions remain active.

Rust owns durable settings, onboarding metadata, appearance source, theme library,
discovery/disclosure state, History, statistics and saved reports. Schemas and
versioned source migrations belong to the Tauri-free core. React owns presentation
snapshots, editor input validation and derived appearance token caches. Native
quick-control patches and full settings saves share serialized publication and
durable owned-field ordering. LocalStorage may be a synchronous presentation cache
while React lives; it is never the sole durable owner of v2 user state.

Extend 1.3's v1 import and markers rather than creating another History/report
importer. Commit v2 source state separately. Reserve v3 for the 1.6 updater and
stop those writers in that same release. Import, transcript-key removal and
whole-store retirement have independent durable receipts. Removal requires each
destination's durable import/clear receipt. Retirement additionally requires all
versions, no React/webview activity, an empty WKWebsiteDataStore, and durable
bundle-store directory removal. React cannot request whole-store retirement.

Retain the standalone objc2 importer in later native hosts, including after
Phase 4, to support direct upgrades from 0.62 without React. Use the original
origin through WebKit APIs, never undocumented private files. On ambiguity,
preserve/quarantine data; never log transcripts or clipboard contents. All
verification fixtures use synthetic stores and temporary roots.

See [the design](../draft/gpui-webkit-lifecycle.md) for atomic publication,
concurrency, future schemas, the failure matrix and outcome-to-test mapping.
