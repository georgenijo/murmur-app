# GPUI startup integration (#996)

Layer 23 follows #1037 → #1038 → #1039 → #1040. Together the layers supply the
native shell, shared service ownership, OS services, production view adapters and
startup. Packaging remains in parallel layer 18. Shipping Tauri remains intact.

Main stays on a loading card until its existing completion owner is available,
legacy import finishes, and production configuration is applied. Failure keeps a
Retry action and preserves unacknowledged legacy keys. Missing installed models
route to setup. Legacy microphone names migrate only to an unambiguous native
UID, matching the retained frontend rule; ambiguous names remain unresolved.

The disposable `ui --native-legacy-import-worker` child loads WebKit dynamically
and uses the fixed `tauri://localhost/` origin with the current installed bundle's
default store. The GPUI process does not load WebKit. The child cannot choose a
profile, mutate destination files, access the clipboard or send arbitrary native
commands. It reads the fixed migration keys, accepts only receipt-authorized
transcript/updater key removals, verifies those removals, then exits. Both hosts
share the same core import/acknowledgement decisions. The parent owns its exact
child, bounded protocol, deadlines and shutdown writer permit. It never purges a
store or deletes unrelated keys. This transport survives Tauri removal.

Whole-window zoom uses one layout/paint scale, converts native input to layout
coordinates once, and converts IME bounds back to native points. AX bounds use
the combined display/zoom scale; accessibility actions already in layout space
are not converted again. NSWindow geometry stays in native points. Cmd-Plus,
Cmd-Minus and Cmd-0 act on the active window; stored UI scaling uses the same API.
Display changes and resize preserve the independent zoom factor.

## Native acceptance additions

Use the complete Phase 2 checklist in
[gpui-production-adapters.md](gpui-production-adapters.md#native-acceptance-checklist).
These cases extend it; no retained frontend assertion is retired.

| Matrix rows | Concrete native flow |
|---|---|
| U01, P03 | Launch a fresh isolated profile, finish setup, relaunch, then launch with the selected model missing. Confirm the loading gate and setup recovery. Hide Main while startup is pending; completion must not activate it. |
| V05 | Use the synthetic legacy worker with an isolated HOME. Verify one imported History row and migrated Spanish setting, acknowledged transcript removals, and identical counts on a second process launch. Interrupt the exact child before acknowledgement and retry; committed destinations must not duplicate or lose data. Production path equality remains pure computation only. |
| P03, U03 | At 50%, 100%, 200% and 300%, operate fixed-pixel buttons, scroll, select text, compose IME text, inspect AX frames and activate controls through AX. Move between Retina/non-Retina displays and resize. Window frame geometry remains unchanged by zoom. |
| P05, V01 | With setup complete, focus Main repeatedly during one six-hour window, then wake and manually check. The shared updater coalesces automatic checks; manual checks still work. Incomplete setup suppresses automatic checks. |
| V02, V03, V04 | Build `native-host-qa` and use only bundle `com.localdictation.gpui-host-qa110`, an isolated HOME and a loopback updater fixture. The existing `MURMUR_UPDATER_FIXTURE` JSON contains `feed` and `publicKey`. It is ignored by production builds/identities. Use #990's synthetic signed fixture/key, optional/required offers, bad signature, interrupted download and restart failure. Never use signing secrets or an installed production bundle. |

The explicit legacy QA switch requires `native-host-qa`, the exact test bundle,
`MURMUR_NATIVE_TEST_HOME=$HOME`, and a synthetic HOME basename beginning
`murmur-gpui-test-`. It creates only that HOME's test-identity WebKit marker and
uses a nonpersistent WebKit store seeded with synthetic values. Add
`--native-host-contract "$HOME/contract.json" --native-host-legacy-fixture` to
validate the passive projections, startup readiness and one-time import before
coordinated quit. Repeat with a new receipt filename to verify from a fresh
process. It requests no OS registrations or permissions.

Interactive native acceptance, signed installation/identity continuity and the
installed update/downgrade rehearsal remain assigned to the GUI QA worker and
the coordinated final stack with #994. These are acceptance work, not missing
production adapter implementations.
