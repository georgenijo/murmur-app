# Agent 5 — Config Findings

## User-Facing Features

### Windows

1. **Main window** (`label: "main"`) — Primary app window titled "Murmur". Default size 720x560, min size 520x400, centered on launch.
2. **Overlay window** (`label: "overlay"`) — Transparent, borderless, always-on-top floating widget (260x100). Not focusable, not resizable, hidden by default, visible on all workspaces, skips taskbar. Loads `overlay.html`. Used to show recording/transcription status near the notch/screen edge.
3. **Log Viewer window** (`label: "log-viewer"`) — Secondary window titled "Murmur — Log Viewer" (800x600, min 600x400). Hidden by default, centered. Loads `log-viewer.html`. For viewing structured event logs.

### Settings (user-configurable via `Settings` interface in `settings.ts`)

- **Model selection** (`model: ModelOption`) — Seven model options across two backends:
  - Moonshine backend: `moonshine-tiny` (~124 MB, default, labeled "Fastest"), `moonshine-base` (~286 MB)
  - Whisper backend: `tiny.en` (~75 MB), `base.en` (~150 MB), `small.en` (~500 MB), `medium.en` (~1.5 GB), `large-v3-turbo` (~3 GB)
- **Double-tap key** (`doubleTapKey: DoubleTapKey`) — Key used for double-tap recording trigger. Options: `shift_l` (Shift, default), `alt_l` (Option), `ctrl_r` (Control).
- **Language** (`language: string`) — Transcription language, default `"en"`.
- **Auto-paste** (`autoPaste: boolean`) — Whether to automatically paste transcribed text after copying to clipboard. Default `false`.
- **Auto-paste delay** (`autoPasteDelayMs: number`) — Delay in milliseconds before auto-paste fires. Default `50`.
- **Recording mode** (`recordingMode: RecordingMode`) — How recording is triggered. Options: `hold_down` (default), `double_tap`, `both`.
- **Microphone** (`microphone: string`) — Audio input device identifier. Default `"system_default"`.
- **Launch at login** (`launchAtLogin: boolean`) — Whether the app starts on macOS login. Default `false`.
- **VAD sensitivity** (`vadSensitivity: number`) — Voice Activity Detection sensitivity. Default `50`. Numeric scale (presumably 0-100).

### Settings Persistence

- Settings are stored in `localStorage` under the key `"dictation-settings"`.
- `loadSettings()` merges stored partial settings with `DEFAULT_SETTINGS`, performing migration for the removed legacy `hotkey` recording mode (maps invalid/missing `recordingMode` to `hold_down` and deletes the `hotkey` field).
- `saveSettings()` serializes the full `Settings` object to `localStorage`.

### Model Default: Frontend vs. Rust Precedence

The frontend default in `settings.ts` (`moonshine-tiny`) takes precedence over the Rust `DictationState::default()` (`base.en`) in practice. Here is the initialization flow:

1. On mount, `useSettings()` calls `loadSettings()` synchronously, returning `DEFAULT_SETTINGS` (with `model: 'moonshine-tiny'`) for a brand-new user with no `localStorage`.
2. `useInitialization(settings)` runs a `useEffect` on mount that calls `initDictation()` (which invokes the Rust `init_dictation` command), then immediately calls `configure({ model: settings.model, language: settings.language, autoPaste: settings.autoPaste })` — passing `'moonshine-tiny'` from the loaded settings.
3. The Rust `configure_dictation` command receives the model name, updates `dictation.model_name` from its default `"base.en"` to `"moonshine-tiny"`, and swaps the backend if necessary.
4. This all happens before the first transcription can occur (recording controls are gated on `initialized` being `true`, which only becomes `true` after `configure` resolves).

Therefore, the Rust-side `DictationState::default()` value of `"base.en"` is effectively dead code for the model name field. It is overwritten by the frontend within the first few hundred milliseconds of app startup, before any user interaction is possible. A brand-new user (no localStorage) will always start with `moonshine-tiny`.

### Auto-Update

- Updater plugin configured with endpoint: `https://github.com/georgenijo/murmur-app/releases/latest/download/latest.json`
- Public key for signature verification is embedded in config (base64-encoded minisign public key).
- `createUpdaterArtifacts: true` in bundle config means the build generates update manifest artifacts.

### Application Identity

- Product name: **Murmur**
- Bundle identifier: `com.localdictation`
- Version: `0.8.0` (in both `tauri.conf.json` and `Cargo.toml`)

### Tray Icon

- Tauri feature `tray-icon` is enabled in Cargo dependencies, indicating a system tray icon is present.

### Autostart

- Autostart plugin is included on both Rust (`tauri-plugin-autostart`) and frontend (`@tauri-apps/plugin-autostart`) sides. Permissions granted: `allow-enable`, `allow-disable`, `allow-is-enabled`.

### Notifications

- Notification plugin included on both sides. Default permissions granted to the main window.

## Internal Systems

### Build Pipeline

- **Frontend dev**: `vite` dev server on `http://localhost:1420`, with `npm run dev` as the before-dev command.
- **Frontend build**: `tsc && vite build`, output to `../dist` (relative to `src-tauri`).
- **Rust build**: Tauri 2 with `tauri-build` for build-time code generation.
- **Dev config override**: Script `tauri:dev` uses `--config src-tauri/tauri.dev.conf.json`, which overrides two fields from the production `tauri.conf.json`: `identifier` becomes `"com.localdictation.dev"` (vs. production `"com.localdictation"`) and `productName` becomes `"Local Dictation Dev"` (vs. production `"Murmur"`). This is a minimal override — only the app identity differs, ensuring the dev build has a separate bundle identifier and display name so it does not conflict with an installed production build. All other configuration (windows, plugins, capabilities, security, updater, etc.) is inherited from the production config unchanged.
- **Test**: `vitest run` for frontend tests; `cargo test -- --test-threads=1` for Rust tests (single-threaded, likely due to shared global state or hardware resources).
- **Timed build**: `build:timed` wraps `tauri build` with `time` for performance measurement.

### Transcription Backends

- **whisper-rs** v0.15 with `metal` and `log_backend` features — GPU-accelerated transcription via Apple Metal.
- **sherpa-rs** v0.6 with `download-binaries` and `static` features — likely the Moonshine model backend (sherpa-onnx bindings). Downloads pre-built binaries, statically linked.

### Audio Capture

- **cpal** v0.15 — Cross-platform audio input library for microphone capture.

### Clipboard / Text Injection

- **arboard** v3 — Cross-platform clipboard access for copying transcribed text.

### Keyboard Monitoring

- **rdev** from git (`https://github.com/Narsil/rdev`, `main` branch) — Global keyboard event listener for hold-down and double-tap detection. Pinned to git main, not a crates.io release.

### macOS-Specific Dependencies (conditional on `target_os = "macos"`)

- **objc2** v0.6 — Rust Objective-C bridge.
- **objc2-app-kit** v0.3 with features: `NSWindow`, `NSScreen`, `NSApplication`, `NSRunningApplication` — for native macOS window/screen inspection (notch detection, overlay positioning).
- **objc2-foundation** v0.3 — Foundation framework bindings.
- **block2** v0.6 — Objective-C block support.

### Networking

- **reqwest** v0.12 with `stream` and `rustls-tls` features (no default features) — HTTP client for model downloads. Uses rustls (pure Rust TLS), not native TLS.

### Model Download Support

- **tar** v0.4 and **bzip2** v0.5 — For extracting downloaded model archives (tar.bz2 format).

### Logging / Tracing

- **tracing** v0.1, **tracing-subscriber** v0.3 (with `json`, `env-filter`, `fmt` features), **tracing-appender** v0.2 — Structured logging with JSON output, environment-based log filtering, and file-based log appending with rotation.
- **chrono** v0.4 (with `clock` feature) — Timestamps for logging.

### System Information

- **sysinfo** v0.32 — System information gathering (likely for diagnostics or resource checks).

### Serialization

- **serde** v1 (with `derive`), **serde_json** v1 — JSON serialization/deserialization for settings, IPC, events.
- **base64** v0.22 — Base64 encoding/decoding (possibly for model key verification or data transport).

### Async Runtime

- **tokio** v1 with features: `process`, `io-util`, `sync`, `rt`, `fs` — Async runtime for process spawning, I/O, synchronization, file system operations.

### Frontend Dependencies

- **React** 18.3.1, **React DOM** 18.3.1 — UI framework.
- **Tailwind CSS** 4.1.18 with `@tailwindcss/vite` plugin — Utility-first CSS.
- **react-markdown** v10.1.0 — Markdown rendering in the UI (possibly for release notes, model descriptions, or log display).
- **rehype-sanitize** v6.0.0 — HTML sanitization for rendered markdown.
- **Vite** 6.0.1 — Frontend bundler and dev server.
- **TypeScript** ~5.6.2 — Type checking.
- **vitest** v4.0.18, **jsdom** v28.1.0 — Frontend testing framework with DOM environment.
- **@vitejs/plugin-react** v4.3.3 — React Fast Refresh for Vite.

### Release Profile (Rust)

- `panic = "abort"` — No unwinding on panic (smaller binary).
- `codegen-units = 1` — Single codegen unit for maximum optimization.
- `lto = true` — Link-Time Optimization enabled.
- `opt-level = "s"` — Optimized for binary size.
- `strip = true` — Debug symbols stripped from release binary.

### macOS Bundle Configuration

- Custom `entitlements.plist` referenced at `./entitlements.plist`.
- Custom `Info.plist` at `./macos/Info.plist`.
- `macOSPrivateApi: true` — Enables private macOS APIs in Tauri (required for transparent windows and other system-level features).
- Bundle targets: `"all"` (generates .app, .dmg, and updater artifacts).

### Security

- **CSP is null** (`"csp": null`) — Content Security Policy is explicitly disabled. This allows unrestricted resource loading in the webview.

### Rust Crate Configuration

- Package name: `ui`, library name: `ui_lib`.
- Crate types: `lib`, `cdylib`, `staticlib` — supports both dynamic and static linking for Tauri.

## Commands / Hooks / Events

### Tauri Capabilities (Permission Grants)

**Main window (`default` capability):**
- `core:default` — Default Tauri core IPC permissions.
- `opener:default` — Default opener permissions (open URLs/files in system apps).
- `autostart:allow-enable` — Enable launch-at-login.
- `autostart:allow-disable` — Disable launch-at-login.
- `autostart:allow-is-enabled` — Check if autostart is enabled.
- `updater:default` — Default updater permissions (check for and install updates).
- `notification:default` — Default notification permissions (send system notifications).
- `process:allow-restart` — Allow app restart (e.g., after update).
- `process:allow-exit` — Allow app exit.

**Overlay window (`overlay` capability):**
- `core:event:default` — Listen/emit Tauri events.
- `core:window:allow-start-dragging` — Allow user to drag the overlay window.
- `core:window:allow-set-position` — Allow programmatic positioning of the overlay.
- `core:default` — Default Tauri core IPC permissions.

**Log Viewer window (`log-viewer` capability):**
- `core:default` — Default Tauri core IPC permissions.
- `core:event:default` — Listen/emit Tauri events.

### Tauri Commands (from CLAUDE.md file map — not directly in config files but referenced)

These are Rust-side IPC commands exposed to the frontend (7 recording/status, 6 permission/audio, 4 keyboard, 3 logging, model download, tray, overlay commands). Their exact names are not in configuration files but are registered in `lib.rs`.

### React Hooks (from CLAUDE.md file map)

- `useHoldDownToggle` — Hold-down recording mode via rdev press/release events.
- `useDoubleTapToggle` — Double-tap recording mode via rdev events.
- `useRecordingState` — Recording status polling, transcription triggering, toggle logic.

### Events (inferred from capability permissions)

- Overlay and log-viewer windows both have `core:event:default` permissions, indicating they receive events from the main window (likely recording state changes, transcription results, log entries).

## Gaps / Unclear

1. **CSP is null** — Content Security Policy is completely disabled. This is a security concern for a shipping application, even though Murmur is local-only. Any loaded webview content has unrestricted script/resource access.

2. **Version mismatch** — `package.json` version is `0.1.0` while `tauri.conf.json` and `Cargo.toml` are both `0.8.0`. The npm package version has not been kept in sync.

3. **`entitlements.plist` and `Info.plist` not read** — These macOS bundle files at `./entitlements.plist` and `./macos/Info.plist` define critical macOS permissions (microphone access, accessibility, etc.) but were not part of this read scope.

4. **rdev pinned to git main** — The `rdev` dependency uses a git branch (`main` on `Narsil/rdev`) rather than a versioned crates.io release. This is fragile: upstream changes could break the build without warning.

5. **VAD sensitivity scale undocumented** — `vadSensitivity` defaults to `50` but the valid range, units, and mapping to underlying VAD parameters are not defined in the settings type (it is just `number`).

6. **Language setting is a free-form string** — `language` is typed as `string` with no validation or enumeration of supported language codes. Users could enter invalid values.

7. **Microphone is a free-form string** — `microphone` is typed as `string` defaulting to `"system_default"`. No validation or enumeration at the type level.

8. **Legacy migration only handles `hotkey`** — `loadSettings()` migrates away the removed `hotkey` recording mode but does not handle other potential future migrations (no version field in stored settings).

9. **No settings schema versioning** — There is no version number in the persisted settings object, making future migrations fragile. Migration logic is ad hoc.

10. **sherpa-rs purpose not explicit** — `sherpa-rs` is included with `download-binaries` and `static` features but the config files do not clarify its role. From context (Moonshine models), it is the ONNX-based inference backend, but this is only inferred.

11. **Overlay window lacks `notification` and `updater` permissions** — Only the main window can trigger updates and notifications. If the overlay needs to display update prompts or notifications, it cannot do so directly.

12. **Log viewer has minimal permissions** — Only `core:default` and `core:event:default`. It cannot invoke commands beyond default IPC. If it needs to call custom logging commands (e.g., to fetch log entries), those would need to be registered as core-accessible or the capability would need expanding.

13. **`overlay.html` and `log-viewer.html` existence not verified** — These HTML entry points are referenced in `tauri.conf.json` but their actual existence/contents were not examined in this scope.

14. **Rust `DictationState::default()` model name is misleading** — The Rust-side default model is `"base.en"` (a Whisper model) while the frontend default is `"moonshine-tiny"` (a Moonshine model). The frontend always overwrites the Rust default via `configure_dictation` during initialization, so the Rust default never actually takes effect. This creates a misleading impression that `base.en` is the default model. Aligning the Rust default with the frontend default would reduce confusion.

## Notes

1. **Privacy-first architecture confirmed by config** — No cloud service endpoints except the GitHub updater URL. Model downloads use `reqwest` with `rustls-tls` (no native TLS). All transcription is local via whisper-rs (Metal GPU) and sherpa-rs.

2. **Dual transcription backend** — The app supports two distinct transcription engines: Whisper (via whisper-rs with Metal acceleration) and Moonshine (via sherpa-rs/sherpa-onnx). The default model (`moonshine-tiny`) uses the Moonshine backend, suggesting it is the preferred/faster option.

3. **macOS-only in practice** — Despite cpal and arboard being cross-platform, the app has deep macOS dependencies: `objc2*`, `macOSPrivateApi: true`, entitlements.plist, osascript injection, Metal GPU acceleration. It would not run on Windows or Linux without significant work.

4. **Binary size optimization** — Release profile aggressively optimizes for size (`opt-level = "s"`, `strip = true`, `lto = true`, `codegen-units = 1`, `panic = "abort"`).

5. **Markdown rendering in frontend** — `react-markdown` and `rehype-sanitize` are included, suggesting some UI surfaces render markdown content (possibly release notes from the updater, or formatted transcription output).

6. **Three-window architecture** — The app uses three distinct webview windows with separate capabilities, following the principle of least privilege (overlay gets minimal permissions, log viewer gets event-only access, main window gets full permissions).

7. **Tauri 2 throughout** — All Tauri dependencies (core, plugins, CLI, API) are consistently on version 2.x.

8. **Frontend test infrastructure** — `vitest` v4 with `jsdom` is set up, indicating frontend unit/integration tests exist or are expected.
