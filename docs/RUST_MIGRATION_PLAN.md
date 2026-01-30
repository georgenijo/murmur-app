# Rust Migration Plan: Python Sidecar to Pure Rust

## Executive Summary
Migrate the Local Dictation app from Python sidecar architecture to pure Rust within Tauri. Eliminates Python subprocess dependency while maintaining all functionality.

## Verification Results (Completed)
- whisper-rs v0.13 compiles on macOS arm64
- Metal GPU acceleration confirmed on Apple M4
- Sub-second transcription speed
- Test binary at /tmp/whisper-rs-test/

## Current Architecture (To Replace)

### Python Components
| File | Purpose | Lines |
|------|---------|-------|
| `dictation_bridge.py` | JSON IPC bridge, state machine | ~455 |
| `transcriber_cpp.py` | whisper.cpp via pywhispercpp | ~115 |
| `text_injector.py` | Clipboard + Cmd+V via pynput | ~143 |

### Current Data Flow
```
WebView (audioCapture.ts) → Base64 WAV → Tauri command → Python subprocess → Transcription → Text injection → Response
```

## New Architecture (Pure Rust)

### Recommended Crates
| Function | Crate | Version | Notes |
|----------|-------|---------|-------|
| Transcription | whisper-rs | 0.13 | Metal feature for GPU |
| Text injection | enigo | 0.6 | Cmd+V simulation |
| Clipboard | arboard | 3 | Maintained by 1Password |
| WAV parsing | hound | 3.5 | 16kHz mono support |
| Noise reduction | Skip | - | Whisper handles noise well |

### New Module Structure
```
src-tauri/src/
├── main.rs          # Entry point (unchanged)
├── lib.rs           # Tauri commands (refactored)
├── transcriber.rs   # whisper-rs integration
├── injector.rs      # enigo + arboard
└── state.rs         # DictationState management
```

### New Data Flow
```
WebView (audioCapture.ts) → Base64 WAV → Tauri command → hound parse → whisper-rs transcribe → enigo inject → Response
```

## Implementation Phases

### Phase 1: Foundation
- [ ] Update Cargo.toml with new dependencies
- [ ] Create module files (transcriber.rs, injector.rs, state.rs)
- [ ] Implement model path resolution

### Phase 2: Core Implementation
- [ ] Implement transcriber.rs (WAV parsing + whisper-rs)
- [ ] Implement injector.rs (arboard + enigo)
- [ ] Refactor lib.rs state management

### Phase 3: Integration
- [ ] Rewrite `init_dictation` - lazy whisper context
- [ ] Rewrite `process_audio` - full Rust pipeline
- [ ] Rewrite `get_status` - from Rust state
- [ ] Rewrite `configure_dictation` - update settings

### Phase 4: Cleanup
- [ ] Remove Python files
- [ ] Update build configuration
- [ ] Update documentation
- [ ] Test on macOS

## Code Snippets

### Updated Cargo.toml
```toml
[dependencies]
# Existing
tauri = { version = "2", features = ["tray-icon"] }
tauri-plugin-opener = "2"
tauri-plugin-global-shortcut = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["process", "io-util", "sync", "rt", "rt-multi-thread"] }
base64 = "0.22"

# New for Rust migration
whisper-rs = { version = "0.13", features = ["metal"] }
hound = "3.5"
enigo = "0.6"
arboard = "3"
dirs = "5"
```

### DictationState
```rust
pub enum DictationStatus {
    Idle,
    Recording,
    Processing,
}

pub struct DictationState {
    pub status: DictationStatus,
    pub model_name: String,
    pub language: String,
}

pub struct AppState {
    pub dictation: Mutex<DictationState>,
    pub whisper_context: Mutex<Option<WhisperContext>>,
}
```

### process_audio Command (Outline)
```rust
#[tauri::command]
async fn process_audio(
    state: State<'_, AppState>,
    audio_data: String,
) -> Result<DictationResponse, String> {
    // 1. Set state to Processing
    // 2. Decode base64 to WAV bytes
    // 3. Parse WAV with hound, extract i16 samples
    // 4. Convert to f32 for whisper
    // 5. Run transcription (spawn_blocking)
    // 6. Inject text via clipboard + Cmd+V
    // 7. Set state to Idle, return response
}
```

### Text Injection
```rust
fn inject_text(text: &str) -> Result<(), String> {
    // 1. Copy to clipboard with arboard
    let mut clipboard = Clipboard::new()?;
    clipboard.set_text(text)?;

    // 2. Small delay
    thread::sleep(Duration::from_millis(50));

    // 3. Simulate Cmd+V with enigo
    let mut enigo = Enigo::new(&Settings::default())?;
    enigo.key(Key::Meta, Press)?;
    enigo.key(Key::Unicode('v'), Click)?;
    enigo.key(Key::Meta, Release)?;

    thread::sleep(Duration::from_millis(50));
    Ok(())
}
```

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| whisper-rs build fails | Low (verified) | High | Already tested, works |
| Accessibility permission | Medium | Medium | Show permission dialog |
| Model distribution | Medium | Low | User downloads externally |
| enigo doesn't work | Low | High | Fallback to core-foundation |

## Model Management

Models are NOT bundled. Strategy:
1. Check `~/Library/Application Support/local-dictation/models/`
2. If missing, prompt user to download
3. Download URL: `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin`

## Files to Delete After Migration
- `dictation_bridge.py`
- `transcriber_cpp.py`
- `text_injector.py`
- `audio_recorder.py` (if exists)
- `requirements.txt`
- `venv/` directory

## Success Criteria
- [ ] App starts without Python
- [ ] Transcription works via whisper-rs
- [ ] Text injection works via enigo
- [ ] Same or better performance
- [ ] All existing features preserved
