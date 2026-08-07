# Murmur — Requested Fixes

**Captured:** 2026-07-28, from a dogfooding session on the MacBook dev build.
**Audience:** an agent picking this up with no prior context.
**Status of this file:** working spec, deliberately untracked. Delete it when the work lands.

Read [CLAUDE.md](CLAUDE.md) first — it is the authoritative map of the codebase, and its
invariants (fail-closed, clipboard-first, Rust owns geometry, no in-process llama) bind
everything below.

---

## The story so far

Read this before the items — it explains why the repo looks the way it does.

**What just happened (2026-07-27 → 28).** An agent session was asked to find and build
features that meaningfully improve the dictation workflow. It shipped three, on the branch
`feat/workflow-boosters`, as PR #375:

1. **History workspace** — search with match highlighting, Mic/File/Pinned filters, pinning,
   and Markdown/plain/JSON export of exactly what's on screen, to the clipboard or a file.
   Backed by a new narrow Rust command `save_text_export` (absolute paths only, `.md`/`.txt`/`.json`
   only, 8 MB cap, atomic temp-then-rename). `teachingContext` is never exported.
2. **Stop on Silence** — a deterministic trailing-silence detector that ends a hands-free
   double-tap recording. Consumes the existing `audio-level` RMS stream; must hear speech
   before it arms; threshold only ever rises above an absolute floor, so on a quiet mic it
   does nothing rather than cutting you off.
3. **Command palette** — ⌘K, plus ⌘F / ⌘, / ⌘L. Deterministic tiered ranking. Every row
   carries a `run` callback from `App.tsx`, so the palette owns no behaviour of its own.

That PR is **5 commits, 36 files, +3229/−63**, all eight CI checks green, CodeRabbit and
claude-review findings addressed and threads resolved. It is blocked only on human approval.

**Then George ran it on his MacBook and gave feedback.** That feedback is items A–G in this
file. The headline: Stop on Silence genuinely works and needs no retuning, but pinning should
never have shipped, the mode gate on Stop on Silence is too narrow, the focus ring looks like
an un-styled browser, and the home screen needs design work. Hence: fold the corrections into
#375 *before* merging, rather than shipping and immediately deleting.

**Along the way**, a bisect found a pre-existing test regression unrelated to any of this and
filed it as issue #376.

## Where things stand right now

| Thing | State |
|-------|-------|
| `main` | `d17be7a`, v0.21.3. Nothing from this work has landed yet. |
| PR [#375](https://github.com/georgenijo/murmur-app/pull/375) `feat/workflow-boosters` | **Open, CI green, unmerged**, blocked on review approval. Items A and B modify it *before* merge. |
| Branch `cursor/theme-engine-plan` | **Where the working tree is checked out right now.** One commit (`fd7b119`) adding `docs/draft/theme-engine-plan.md` — an Appearance theming engine plan, someone else's in-flight work. Leave it alone; **overlaps item D**, see §"Coordination" there. |
| Issue [#376](https://github.com/georgenijo/murmur-app/issues/376) | `llm_sidecar_integration::cooperative_cancel…` fails deterministically on `main`, bisected to `2010de6` (mock helper moved from `[[bin]]` to `[[example]]`). CI doesn't catch it because CI runs `cargo test --lib`, which skips integration targets. **Out of scope here.** |
| Stop on Silence | Confirmed working on the real device at 2.5s. Thresholds do **not** need retuning. |
| Other open issues | #340 transform latency, #312 transform feature, #305 CI release speedup. All unrelated. |

### Machines and how to see the app

- This agent runs on **mac-mini**. George's dev build runs on **macbook** (`100.70.184.103`,
  ssh user `georgenijo`) — a Tailscale mesh managed by the `fleet` CLI (`fleet ls`,
  `fleet exec macbook '<cmd>'`).
- **`fleet shot`'s inbox sync is broken** — nothing newer than 2026-07-20 arrives, so
  `fleet shot resolve` silently misses recent screenshots. When George pastes a
  `/Users/georgenijo/...` screenshot path, fetch it directly:
  ```bash
  scp "georgenijo@100.70.184.103:/Users/georgenijo/Library/Containers/com.sw33tlie.macshot.macshot/Data/tmp/macshot-share/<name>.png" /tmp/
  ```
- The **Vite dev server is not running** — the previous session's instance has exited.
  Start one with `cd app && npm run dev` (serves http://localhost:1420); `lsof -ti:1420 |
  xargs kill` if a stale one is holding the port.
- The previous session verified UI changes by driving that dev server headlessly with
  Playwright Chromium (already installed under `~/Library/Caches/ms-playwright`) plus a
  hand-written `window.__TAURI_INTERNALS__` stub, since the Playwright MCP server is
  configured for a Chrome channel that isn't installed. That approach works well — a
  scratch script and its screenshots are in the session scratchpad. The full desktop app
  (`tauri dev`) was deliberately **not** launched from this machine: it would seize the
  global dictation hotkey.

Verification bar for everything below (what CI runs):

```bash
cd app && npx tsc --noEmit
cd app && npm test
cd app/src-tauri && cargo test --lib -- --test-threads=1
```

`cargo test` on macOS needs the stubbed sidecar binary to exist at
`app/src-tauri/binaries/murmur-llm-sidecar-aarch64-apple-darwin` — see the macOS note in CLAUDE.md.

---

## A. Remove the pin feature — fold into PR #375

**Decision: cut it.** George does not see himself pinning transcripts, and does not want the
pinned-aware clear either. The strongest case for pinning was "history auto-trims at 50, so a
transcript you care about silently disappears" — but copy, export, and the knowledge store all
answer that better, and pinning costs a per-row button, a filter chip, a sort rule, a second
trim budget, a pin ceiling with its own error message, and a two-button split clear.

**Replacement for the underlying worry:** raise `MAX_ENTRIES` in `app/src/lib/history.ts`
from `50` to `200`. No UI, no new concepts.

### Remove

- `app/src/lib/history.ts` — `isPinned`, `MAX_PINNED_ENTRIES`, the pinned budget inside
  `trimHistory` (it becomes a plain `slice(-MAX_ENTRIES)` again, but **keep it index-based**
  if that reads cleaner), `togglePinned`, `removeUnpinned`, `remainingPinSlots`, the pin
  ordering in `sortForDisplay`, the `'pinned'` entry in `HISTORY_FILTER_OPTIONS`, the
  `pinned` field on `HistoryEntry`, and `pinned` from all three export formats.
- `app/src/lib/hooks/useHistoryManagement.ts` — `togglePin`, `clearUnpinnedEntries`.
- `app/src/components/history/HistoryPanel.tsx` — the pin button and `PinIcon`, the `Pinned`
  badge, the pin-ceiling notice, the `Pinned` filter chip, and the split clear (back to a
  single `Clear History` with the existing two-step confirm).
- `app/src/components/TranscriptionView.tsx` and `app/src/App.tsx` — the `onTogglePin` /
  `onClearUnpinned` props and wiring.

### Keep

Search, highlighting, the All/Mic/File filters, the result counter, all three export formats,
`save_text_export`, and the two-step confirm on clear. Those are the parts doing real work.

### Also update

- Tests: `app/src/lib/history.test.ts`, `app/src/components/history/HistoryPanel.test.tsx`,
  `app/src/components/SonicCanvasComponents.test.tsx`. Delete pin-specific cases; keep and
  extend the trim tests for the new 200 cap.
- Docs: `docs/features/history-workspace.md` (drop the Pinning section), `docs/FEATURES.md`,
  `docs/reference/settings.md` (the `dictation-history` localStorage row), `docs/reference/hooks.md`
  (`useHistoryManagement`), and the 2026-07-27 entry in `docs/decisions/DECISIONS.md`
  (rewrite the pinning clause rather than leaving a decision recorded for a feature that
  never shipped).
- PR #375's description mentions pinning in several places.

**Sanity check when done:** `git grep -i pinned app/src` should return only unrelated hits
(`transformSettings.ts` uses "pinned" for size/SHA pinning).

---

## B. Stop on Silence in every recording mode — fold into PR #375

**Problem:** the setting only appears when the trigger is Double-Tap. George uses Hold Down
sometimes and wants it there too.

**There is no technical blocker.** Traced end to end: in Hold Down, the key release fires
`hold-down-stop` → `handleStop`, which early-returns because status is no longer `recording`.
In Both, `holdActiveRef` is still true when auto-stop fires so the backend-sync effect skips,
and the later release clears it. Neither path corrupts state. The original gate was a
judgment call about meaning, not a limitation.

**Ship this rule instead of "on in every mode, period":**

> Stop on Silence applies to any recording that was **not started by holding the trigger key**.

- **Double-Tap** — unchanged.
- **Both** — double-tap-started recordings auto-stop; hold-started ones end on release, so
  both gestures keep their natural meaning.
- **Hold Down** — applies to recordings started from the main-window button, the overlay
  click, and locked mode. Those are toggle-started and today have no way to end except
  clicking again, so this is a real gain.

The rejected alternative is auto-stop firing while the key is physically held: pausing
mid-sentence would cut you off while you are still pressing the button that means
"I'm still going."

### Approach

`useSilenceAutoStop` needs to know how the current recording started. `hold-down-start` is
emitted in both hold-relevant modes and `useCombinedToggle` already tracks exactly this in
`holdActiveRef` — it just isn't surfaced. Suggested shape: a small `useRecordingOrigin` hook
(or a ref owned by `App.tsx`) that listens for `hold-down-start` → `'hold'`, and resets to
`'toggle'` on `double-tap-toggle`, on `hold-down-cancel`, and whenever a recording starts
through `handleStart` from a button/overlay path.

**Watch out:** in Both mode a short tap emits `hold-down-start` followed by
`hold-down-cancel`. The origin must not be left stale at `'hold'` after a cancelled
speculative recording, or the next toggle-started recording silently loses auto-stop.

### Also update

- `app/src/components/settings/SettingsPanel.tsx` — show the control in all three modes; the
  help text becomes something like *"Applies when you didn't start by holding the key."*
  Keep `disabled={isRecording}`.
- Tests: extend `app/src/lib/hooks/useSilenceAutoStop.test.tsx` with a hold-origin case;
  cover the tap-then-cancel sequence in Both.
- Docs: `docs/features/silence-auto-stop.md` §"Where it applies" is now wrong end to end —
  rewrite it. Also `docs/reference/settings.md` (`autoStopSilenceMs`) and `docs/FEATURES.md`.

---

## C. Auto-Enter after paste — new PR

**Want:** after auto-paste lands the transcript, optionally press Return so a prompt submits
itself. Dictate at a chat box, stop talking, and the message sends.

**Two shape decisions that are not negotiable without an explicit override:**

1. **This is a delivery feature, not a Stop-on-Silence feature.** George framed it as part of
   auto-stop, but "paste then submit" is equally wanted when you release the key or tap to
   stop. Tying it to silence means the same prompt behaves differently depending on how the
   recording ended. Implement it in the delivery path so it works regardless.
2. **Off by default, per-app allow-list only.** This is the one feature in this batch that
   takes an irreversible action inside someone else's app. If auto-paste lands in the wrong
   window — a real failure mode, it's why the paste-delay slider exists — Return sends a
   half-formed message, or runs a line in a terminal. Per-app profiles already carry
   `autoPasteOverride`; add `autoSubmitOverride` beside it. **Do not add a global
   always-submit toggle.**

### Constraints

- Fires only when auto-paste actually ran **and reported success**. A failed paste must never
  be followed by Return.
- Suppressed whenever auto-paste is suppressed (file-output mode).
- Respects the existing paste delay; the Return needs its own small delay after the paste so
  the target app has committed the text.
- Some apps submit on ⌘Return rather than Return. Ship Return only in v1, but shape the
  setting so a submit-key choice can be added without a migration.

### Approach

`app/src-tauri/src/injector.rs` already synthesizes ⌘V via `CGEvent` with an `osascript`
fallback — the Return keystroke is the same mechanism. Keep the *decision* ("should this
delivery submit?") as a pure function taking the resolved per-recording context so it can be
unit-tested; only the keystroke itself is untestable.

Per-recording context resolution is immutable and happens at recording start
(`dictation_context.rs`) — `autoSubmitOverride` must resolve there with the rest, not be read
live at delivery time.

### Also update

`app/src/lib/settings.ts` (`AppProfile`), `AppOverridesEditor.tsx`,
`docs/features/text-injection.md`, `docs/features/per-app-profiles.md`,
`docs/reference/settings.md`.

---

## D. Kill the browser-default focus ring — new PR

**Problem:** focusing the transcript search draws a bright cyan 2px ring with an offset gap
around the whole input. In dark mode it reads exactly like an un-styled browser default. It
shows on `Select`s and most buttons too, because the treatment is app-wide:
`focus-visible:ring-2 ring-primary` appears **48 times across 20 files**.

**Two things are actually going on** — fix both or the complaint won't be resolved:

1. The focus ring itself.
2. The active filter chip uses the same bright `bg-primary` fill, a few pixels away. Even
   with the ring fixed, the chip row still reads as "blue thing".

**Do not remove focus indication entirely.** George asked for "gone entirely"; the reason to
push back is that keyboard-only navigation becomes invisible — ⌘F then Tab and you have no
idea where you are. Replace it instead: a 1px border-colour shift plus a subtle inner shadow,
no offset, no primary hue. Same information, none of the browser look. It must stay legible
in **both** light and dark. If George still wants it gone on inputs specifically after seeing
it, that's a one-line follow-up.

### Coordination

`cursor/theme-engine-plan` is a live plan to make the `--murmur-*` tokens configurable
(Appearance page, accent picker, AA contrast checks). This item changes *which utilities
components apply*, not the token values, so the two are independent in principle — but they
touch the same components and the same visual language. Sequence deliberately: whoever goes
second rebases, and if the theme engine lands first, the new focus treatment should be
expressed in terms of its tokens rather than hardcoded colours.

---

## E. Collapse the search into an icon button — new PR

**Want:** the transcript search should be a small icon button by default and expand into a
full bar when you start using it.

- Expands on click **and** on ⌘F (⌘F must expand *and* focus in one action).
- Collapses on blur when the query is empty; stays expanded while there is a query.
- The result counter (`N of M`) lives in the expanded state.
- The filter chips row needs a resting layout that doesn't jump when the box expands.

Depends on nothing, but it is cosmetically entangled with D and F — land it after D if both
are in flight.

---

## F. Home screen cleanup — mockups first, then a PR

George: *"This whole UI on the homepage needs to be cleaned up."* That is a design
conversation, not a ticket.

**Deliverable is 2–3 interactive HTML mockups**, not a diff: (1) current layout with D and E
applied, (2) a tighter pass, (3) a more aggressive rethink. George picks, then it gets built.
Do not guess at what "tight" means and hand over a diff.

Reference material: `docs/draft/theme-engine-plan.md` §3 documents the token substrate and
the four independent webviews. The overlay and transform-review windows stay dark glass —
this is main-window only.

---

## G. ⌘K during a recording — no change, deliberately

Confirmed from a screenshot of the running app: with a recording active, the palette opens
with **Stop recording** as the first row, over the live Recording header and the red Stop
button. That is the designed behaviour and it works.

Keeping it because Murmur's window is not focused during real dictation — you're typing into
another app — so the only way to reach this state is to be deliberately inside Murmur. Listed
here so nobody "fixes" it.

If it comes back as a complaint, the likely real objection is that the palette is physically
large; shrink it as part of F rather than blocking ⌘K.

---

## Decisions still open

These have a recommended default so work is not blocked, but George has not explicitly
confirmed them. Flag them in the PR rather than silently assuming:

| # | Question | Default taken |
|---|----------|---------------|
| B | "Not started by holding the key" vs. plain "on in all modes" | The origin-based rule |
| C | Delivery feature, per-app allow-list vs. tied to Stop on Silence with a global toggle | Delivery + per-app |
| D | Quiet focus treatment vs. removing focus indication entirely | Quiet treatment |
| A | Raise the history cap to 200 alongside removing pinning | Yes, 200 |

---

## Non-goals

- Issue #376 (sidecar integration test). Separate, already filed.
- Any change to the transform / sidecar path.
- Loosening a privacy or fail-closed invariant to make any of the above easier. In
  particular `teachingContext` must stay out of exports, and `save_text_export` must keep its
  extension allow-list, size cap, and atomic write.
- Merging PR #375 before A and B are folded in — no point shipping pinning and deleting it
  the next day.

---

## Suggested order

1. **A + B** into `feat/workflow-boosters`, then merge #375.
2. **D**, coordinating with the theme-engine branch.
3. **E**.
4. **C** (independent of all of the above; can run in parallel by a second agent).
5. **F** mockups → George picks → build.
