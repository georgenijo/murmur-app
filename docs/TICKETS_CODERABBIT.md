# CodeRabbit Review Tickets

Issues from CodeRabbit review on PR #4.

## Critical (Major)

### CR-001: Remove unsupported `model` input from GitHub Actions
**Status:** Open
**Files:** `.github/workflows/claude-code-review.yml:28`, `.github/workflows/claude.yml:38`
**Description:** The `anthropics/claude-code-action@v1` does not define a `model` input. Remove `model: claude-opus-4-5-20251101` from both workflow files.

**Fix:**
```diff
        uses: anthropics/claude-code-action@v1
        with:
          claude_code_oauth_token: ${{ secrets.CLAUDE_CODE_OAUTH_TOKEN }}
-         model: claude-opus-4-5-20251101
```

---

### CR-002: Cleanup on audio init failure to avoid stale recording state
**Status:** Open
**File:** `ui/src-tauri/src/audio.rs:87`
**Description:** If `ready_rx.recv_timeout` returns error/timeout, the sender/handle stay set, so `is_recording()` may report true and the worker may continue running. Clear state and send stop on failure.

**Fix:**
```rust
let init_result = match ready_rx.recv_timeout(std::time::Duration::from_secs(5)) {
    Ok(Ok(())) => Ok(()),
    Ok(Err(e)) => Err(e),
    Err(_) => Err("Audio thread failed to initialize within timeout".to_string()),
};

if init_result.is_err() {
    if let Some(sender) = state_guard.command_sender.take() {
        let _ = sender.send(AudioCommand::Stop);
    }
    state_guard.thread_handle.take();
}

init_result
```

---

## Minor

### CR-003: `open_system_preferences` lacks platform-specific handling
**Status:** Open
**File:** `ui/src-tauri/src/lib.rs:177`
**Description:** Uses macOS-specific `open` command but lacks `#[cfg(target_os = "macos")]` guard. Add platform gate.

**Fix:**
```rust
#[tauri::command]
fn open_system_preferences() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        return Err("System preferences shortcut not supported on this platform".to_string());
    }
    Ok(())
}
```

---

### CR-004: Status not reset if `audio::start_recording()` fails
**Status:** Open
**File:** `ui/src-tauri/src/lib.rs:228`
**Description:** If `audio::start_recording()` fails, dictation status was already set to `Recording`, leaving state inconsistent.

**Fix:**
```rust
if let Err(e) = audio::start_recording() {
    let mut dictation = state.app_state.dictation.lock_or_recover();
    dictation.status = DictationStatus::Idle;
    return Err(e);
}
```

---

### CR-005: Fix Biome lint - forEach callback should not return value
**Status:** Open
**File:** `ui/src/components/PermissionsBanner.tsx:29`
**Description:** Expression-bodied callback returns a value; use block body.

**Fix:**
```diff
-          stream.getTracks().forEach(track => track.stop());
+          stream.getTracks().forEach(track => {
+            track.stop();
+          });
```

---

### CR-006: Propagate WAV sample decoding errors
**Status:** Open
**File:** `ui/src-tauri/src/transcriber.rs:118`
**Description:** `filter_map(|s| s.ok())` silently discards decoding errors. Propagate errors instead.

**Fix:**
```rust
let samples: Vec<f32> = reader
    .into_samples::<i16>()
    .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| format!("Failed to decode WAV samples: {}", e))?;
```

---

### CR-007: Propagate segment retrieval errors
**Status:** Open
**File:** `ui/src-tauri/src/transcriber.rs:149`
**Description:** `full_get_segment_text()` errors are silently skipped. Propagate errors instead.

**Fix:**
```rust
for i in 0..num_segments {
    let segment = state
        .full_get_segment_text(i)
        .map_err(|e| format!("Failed to get segment {}: {}", i, e))?;
    text.push_str(&segment);
}
```

---

## Documentation

### CR-008: Correct platform scope in ARCHITECTURE.md
**Status:** Open
**File:** `docs/ARCHITECTURE.md:5`
**Description:** App is cross-platform with macOS optimizations, not macOS-only. Update description.

---

### CR-009: Add language specifiers to code fences in ARCHITECTURE.md
**Status:** Open
**File:** `docs/ARCHITECTURE.md:20,37`
**Description:** Code fences lack language specifiers. Add `text` or `tree`.

---

### CR-010: Fix table column alignment in ARCHITECTURE.md
**Status:** Open
**File:** `docs/ARCHITECTURE.md:68`
**Description:** Table pipes don't align consistently with header separator row.

---

### CR-011: Remove non-standard log path from DEVELOPMENT.md
**Status:** Open
**File:** `docs/DEVELOPMENT.md:84`
**Description:** Path `/private/tmp/claude/-Users-*/tasks/*.output` is environment-specific. Remove or document standard log location.

---

### CR-012: Document all model search locations in DEVELOPMENT.md
**Status:** Open
**File:** `docs/DEVELOPMENT.md:97`
**Description:** "Model Not Found" section lists only 3 locations but README documents 6. Add all locations including `WHISPER_MODEL_DIR`.

---

### CR-013: Fix table separator spacing in README.md
**Status:** Open
**File:** `README.md:116`
**Description:** Separator row lacks spaces around pipes (MD060). Add spaces for consistency.

---

## Already Addressed

- ~~README.md:140 - Model search paths~~ (Commit 5b29903)
- ~~README.md:99 - Table pipe spacing~~ (Commit 5b29903)
