# Known Bugs & Backlog

---

## Control key does not trigger recording

**Status:** Open
**Affects:** Key Combo recording mode
**Symptom:** Selecting a Control key combo as the recording hotkey does not trigger recording. Shift and Alt combos work fine.
**Likely cause:** The `global-shortcut` plugin or macOS intercepts Control key combos before the app receives them.
**Entry point:** `ui/src/lib/hooks/useHotkeyToggle.ts`, `ui/src/lib/hotkey.ts`

---

## Double-tap listener start/stop events are not logged

**Status:** Backlog
**Affects:** Observability / debugging
**Description:** When `start_double_tap_listener` and `stop_double_tap_listener` are called, nothing is written to `app.log`. This makes it hard to diagnose issues like the accessibility-permission restart bug without adding console logs manually.
**Proposed fix:** Add `log_info!` calls in `lib.rs` at the start and stop of the rdev listener, including the hotkey key name and whether accessibility permission was granted at the time.
**Entry point:** `ui/src-tauri/src/lib.rs` — `start_double_tap_listener`, `stop_double_tap_listener` commands

---
