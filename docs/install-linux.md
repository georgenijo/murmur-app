# Linux Install

Murmur ships `.deb`, `.rpm`, and `.AppImage` artifacts for Linux x86_64. These are built on every release and attached to the corresponding [GitHub Release](https://github.com/georgenijo/murmur-app/releases/latest).

## Install

### Debian / Ubuntu / Pop!_OS

```bash
sudo apt install ./Murmur_<version>_amd64.deb
```

### Fedora / Nobara / RHEL

```bash
sudo dnf install ./Murmur-<version>-1.x86_64.rpm
```

### AppImage (any distro)

```bash
chmod +x Murmur_<version>_amd64.AppImage
./Murmur_<version>_amd64.AppImage
```

## Runtime dependencies

The packages declare their direct deps, but for reference Murmur needs:

- `webkit2gtk-4.1` — webview rendering
- `libayatana-appindicator3` — tray icon
- `libasound2` — ALSA audio capture
- `xdotool` — auto-paste injection (X11) or `wtype` (Wayland, install separately)
- CUDA 12.8+ runtime — GPU-accelerated whisper inference. Without it the app falls back to CPU and is significantly slower.

## Permissions

Murmur reads global keyboard events via [`rdev`](https://github.com/Narsil/rdev). On Linux this requires either:

- Running as a user in the `input` group: `sudo usermod -aG input $USER` (then re-login), or
- Granting read access to `/dev/uinput`: `sudo chmod a+rw /dev/uinput`

Microphone access uses ALSA directly — no portal involved. If your distro uses PipeWire, the ALSA-PipeWire bridge handles it transparently.

## Wayland

Murmur sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` and `WEBKIT_DISABLE_COMPOSITING_MODE=1` automatically on Linux to work around a webkit2gtk rendering bug on mesa/NVIDIA stacks (Fedora, Nobara, Ubuntu 23+) where the window would otherwise appear blank. To opt out — for example if you're on a stack where DMABUF works fine — set the variable to anything else before launching:

```bash
WEBKIT_DISABLE_DMABUF_RENDERER=0 murmur
```

## Auto-update

The Tauri updater checks `latest.json` against the running version. Linux artifacts are signed with the same minisign key used for macOS. Updates download as a fresh `.AppImage.tar.gz` and replace the running binary on next launch.

`.deb` and `.rpm` installs do **not** auto-update — use your package manager. Auto-update only applies to AppImage.

## Logs

Structured event log: `~/.local/share/local-dictation/logs/app.log`

JSONL event stream: `~/.local/share/local-dictation/logs/events.jsonl`

Open the in-app log viewer from the tray menu for a live view.

## Known issues

- **Notch overlay is disabled on Linux.** The Dynamic Island–style overlay is macOS-specific (depends on a real notch). Status feedback comes from the tray icon and main window only.
- **Auto-paste on Wayland is best-effort.** `xdotool` only works under XWayland; native Wayland clients may not receive the synthesized keystroke. The transcription is always written to the clipboard, so manual paste (`Ctrl+Shift+V`) is the reliable fallback.
- **First launch builds the whisper context** — expect a few seconds of latency on the first transcription while the model loads into VRAM.
