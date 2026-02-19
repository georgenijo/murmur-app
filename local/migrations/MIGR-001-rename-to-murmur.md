# MIGR-001: Rename App to Murmur

**Created:** 2026-02-19
**Status:** TODO
**Effort:** ~2 hours

---

## Summary

Rename the app from "Local Dictation" to "Murmur" across all display names, internal paths, app identifier, and documentation. Includes a startup migration that moves existing user data from the old path to the new one.

---

## What Changes

### Display / branding
- `ui/src-tauri/tauri.conf.json` — `productName: "Murmur"`, window `title: "Murmur"`
- `ui/index.html` — `<title>Murmur</title>`
- `ui/src/components/StatusHeader.tsx` — "Local Dictation" → "Murmur"
- `ui/src/components/AboutModal.tsx` — "Local Dictation" → "Murmur"

### App identifier
- `ui/src-tauri/tauri.conf.json` — `identifier: "com.murmur"` (was `com.localdictation`)
- **Side effect:** changes the Tauri WebView data directory, wiping localStorage (saved settings) for existing users. Settings fall back to defaults on first launch — acceptable given current user base size.

### Internal storage paths
- `ui/src-tauri/src/logging.rs` — log path: `local-dictation/logs/` → `murmur/logs/`
- `ui/src-tauri/src/transcriber.rs` — model search path: `local-dictation/models/` → `murmur/models/`
- `ui/src-tauri/src/lib.rs` — add `migrate_app_data()` call at startup (see below)

### Docs
- `README.md` — full rewrite (also outdated: still describes Python sidecar architecture)
- `CHANGELOG.md` — update app name throughout
- `CLAUDE.md` — update project name, identifier, paths
- `AGENT_START.md` — update project name
- `docs/onboarding.md` — update model install path example
- `docs/TICKETS_FEATURES.md` — update app name references

---

## Startup Migration (Rust)

Add to `lib.rs` before any other initialization:

```rust
fn migrate_app_data() {
    let Some(data_dir) = dirs::data_dir() else { return };
    let old = data_dir.join("local-dictation");
    let new = data_dir.join("murmur");
    if old.exists() && !new.exists() {
        if let Err(e) = std::fs::rename(&old, &new) {
            eprintln!("Migration failed: {e}");
        }
    }
}
```

- Idempotent: skips if old path doesn't exist or new path already exists
- Moves the entire `local-dictation/` directory in one operation (logs + models)
- Runs once on first launch after upgrade, no marker file needed
- Add `dirs` crate to `Cargo.toml` if not already present

---

## Testing Checklist

- [ ] Existing model in `~/Library/Application Support/local-dictation/models/` is found after launch
- [ ] Logs appear in `~/Library/Application Support/murmur/logs/app.log`
- [ ] App launches with default settings (localStorage cleared — expected)
- [ ] No reference to "Local Dictation" in visible UI
- [ ] `productName` shows "Murmur" in the .dmg and .app bundle
- [ ] Old `local-dictation/` directory is gone after first launch

---

## Notes

- The `WHISPER_MODEL_DIR` env var override still works regardless of rename — no change needed
- `signingIdentity` in `tauri.conf.json` stays the same (tied to Apple Developer account, not app name)
- Do this while user base is small — path migration gets harder with real users
