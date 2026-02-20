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

## Auto-paste toggle is unreliable (BUG-001)

**Status:** Open
**Affects:** Settings panel — auto-paste toggle
**Symptom:** The auto-paste toggle button flickers or appears to toggle in and out inconsistently. The setting does not always reflect the true state of auto-paste.
**Likely cause:** State sync issue between the frontend settings and the Rust injector, or a race condition when the toggle fires before the settings are persisted.
**Entry point:** `ui/src/components/settings/SettingsPanel.tsx`, `ui/src/lib/settings.ts`, `ui/src-tauri/src/injector.rs`

---

## App goes to sleep / becomes unresponsive (BUG-003)

**Status:** Open
**Affects:** General — app responsiveness
**Symptom:** App appears to go to sleep intermittently. Hotkeys stop responding or the app becomes unresponsive until restarted.
**Likely cause:** Unknown — could be macOS suspending the process, rdev listener dying silently, or the audio device going idle. Needs logging to diagnose.
**Next step:** Reproduce and check `app.log` for any events around the time it goes unresponsive. Add a periodic heartbeat log or watchdog if the pattern becomes clearer.
**Entry point:** `ui/src-tauri/src/lib.rs`, `ui/src-tauri/src/keyboard.rs`

---

## Model dropdown uses outdated UI (BUG-002)

**Status:** Open
**Affects:** Settings panel — Whisper model selector
**Symptom:** The model dropdown does not match the current UI design standards (stone palette, FEAT-001 design language). It uses a native/old-style `<select>` element that looks inconsistent with the rest of the settings panel.
**Proposed fix:** Replace with a styled custom select component that matches the existing UI — consistent with other inputs in the settings panel.
**Entry point:** `ui/src/components/settings/SettingsPanel.tsx`

---
