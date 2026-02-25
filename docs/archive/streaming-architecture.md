# Streaming Transcription Architecture (Shelved)

> **Status:** Explored and shelved (Feb 2025, Issue #24). This doc preserves the implementation details so the architecture can be re-added without re-discovery.

## Why it was shelved

1. **whisper.cpp's `new_segment_callback` does not fire progressively.** It processes the entire audio in one forward pass, then iterates through completed segments. All segments arrive within the same millisecond — there is no gradual streaming during inference.
2. **`single_segment(false)` adds overhead** without reducing total latency. Whisper must detect segment boundaries, making inference slightly slower than `single_segment(true)`.
3. **The codebase already used greedy decoding** (`SamplingStrategy::Greedy { best_of: 1 }`). The ticket assumed beam_size=5 with room to reduce — that wasn't the case.
4. **The app is typically minimized** during dictation, so streaming text in the main window has near-zero user value.

## What would make streaming valuable

**Real-time streaming ASR during recording** (Apple dictation-style) — feeding audio chunks to the model while the user is still speaking, displaying words as they're recognized. This requires:

- An **online/streaming ASR model** (not offline/batch like whisper.cpp)
- **sherpa-onnx** (already a dependency via sherpa-rs) supports online streaming models
- Audio fed in ~200ms chunks during recording, not after
- A fundamentally different pipeline: record + transcribe simultaneously, not record → stop → transcribe

## Architecture: Tauri Channel for Rust→Frontend streaming

### The problem with `emit()` / `listen()`

Tauri 2's `AppHandle::emit()` + frontend `listen()` from `@tauri-apps/api/event` **did not work** for delivering events from within whisper.cpp's C callback trampoline. Events emitted via `emit()` were never received by the frontend listener, even though `emit()` returned `Ok(())`. Other events using the same pattern (e.g., `recording-status-changed`) work fine — the issue is specific to calling `emit()` from within a C FFI callback context.

A channel-based relay (mpsc channel + emitter thread) also failed because `emit()` from a spawned thread didn't reach the frontend either.

### Working solution: Tauri 2 Channel API

Tauri 2's `Channel<T>` provides a direct IPC pipe from a Rust command to a frontend callback. Unlike `emit()`/`listen()`, it works reliably from any context.

**Rust side:**

```rust
use tauri::ipc::Channel;

#[tauri::command]
async fn stop_native_recording(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    on_segment: Channel<String>,  // Frontend passes this in
) -> Result<serde_json::Value, String> {
    // ... recording teardown ...

    // Pass channel to the transcription pipeline
    let result = run_transcription_pipeline(
        &samples, &app_handle, &state.app_state, Some(&on_segment)
    );

    // ...
}

fn run_transcription_pipeline(
    samples: &[f32],
    app_handle: &tauri::AppHandle,
    app_state: &AppState,
    segment_channel: Option<&Channel<String>>,
) -> Result<String, String> {
    // Create a callback that sends segments through the channel
    let on_segment: Option<Box<dyn Fn(String) + Send>> = segment_channel.map(|ch| {
        let ch = ch.clone();
        Box::new(move |text: String| {
            let _ = ch.send(text);
        }) as Box<dyn Fn(String) + Send>
    });

    let mut backend = app_state.backend.lock_or_recover();
    backend.transcribe(samples, &language, on_segment)?
    // ...
}
```

**Frontend side:**

```typescript
import { Channel, invoke } from '@tauri-apps/api/core';

// In dictation.ts
export async function stopRecording(onSegment?: (text: string) => void): Promise<DictationResponse> {
  const channel = new Channel<string>();
  if (onSegment) {
    channel.onmessage = onSegment;
  }
  return await invoke('stop_native_recording', { onSegment: channel });
}

// In useRecordingState.ts
const res = await stopRecording((segment) => {
  partialTextRef.current += segment;
  setPartialText(partialTextRef.current);
});
```

### TranscriptionBackend trait with callback

```rust
pub trait TranscriptionBackend: Send + Sync {
    fn transcribe(
        &mut self,
        samples: &[f32],
        language: &str,
        on_segment: Option<Box<dyn Fn(String) + Send>>,
    ) -> Result<String, String>;
    // ... other methods unchanged
}
```

**WhisperBackend** wires up `set_segment_callback_safe`:

```rust
use whisper_rs::SegmentCallbackData;

fn transcribe(&mut self, samples: &[f32], language: &str, on_segment: Option<Box<dyn Fn(String) + Send>>) -> Result<String, String> {
    let streaming = on_segment.is_some();
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_single_segment(!streaming);  // multi-segment needed for callbacks
    // ... other params ...

    if let Some(callback) = on_segment {
        params.set_segment_callback_safe(move |data: SegmentCallbackData| {
            callback(data.text);
        });
    }

    state.full(params, samples)?;
    // ... collect segments into final text ...
}
```

**MoonshineBackend** ignores the callback (no streaming support in sherpa-rs offline mode):

```rust
fn transcribe(&mut self, samples: &[f32], _language: &str, _on_segment: Option<Box<dyn Fn(String) + Send>>) -> Result<String, String> {
    let result = self.recognizer.as_mut().unwrap().transcribe(16000, samples);
    Ok(result.text.trim().to_string())
}
```

## Key lessons

1. **`emit()` does not work from C FFI callback contexts.** Use Tauri `Channel<T>` instead.
2. **whisper.cpp segment callbacks are not progressive.** All segments fire at once after the full forward pass completes. For true streaming, you need an online/streaming model.
3. **`single_segment(false)` is required** for segment callbacks to fire (with `true`, whisper forces one segment and the callback fires once at the end).
4. **`set_segment_callback_safe`** requires `FnMut + 'static` and takes `SegmentCallbackData { segment, start_timestamp, end_timestamp, text }`. Import `SegmentCallbackData` from `whisper_rs`.
5. **whisper-rs 0.13** does not have `set_beam_size()`. Beam size is set via `SamplingStrategy::BeamSearch { beam_size, patience }` at `FullParams` construction time.
6. **Dev vs prod log files:** dev builds write to `app.dev.log`, prod to `app.log` in `~/Library/Application Support/local-dictation/logs/`.
7. **Verify the running binary** with `lsof -p $(pgrep -f "target/debug/ui") | grep target/debug/ui` — worktrees can cause the wrong binary to run.
