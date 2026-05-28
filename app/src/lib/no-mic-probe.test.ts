import { describe, it, expect } from 'vitest';

// Issue #177 regression guard.
//
// Calling `navigator.mediaDevices.getUserMedia({ audio: true })` from the WebView
// opens the microphone through macOS voice-processing I/O (VPIO), which DUCKS all
// other system audio for as long as the session is live. The PermissionsBanner used
// to do this on every window focus to check mic permission, so surfacing Murmur
// quieted/stuttered any other audio playing (e.g. a video in another app).
//
// Recording must always go through the native cpal pipeline (Rust), and mic
// permission must be read via the `check_microphone_permission` Tauri command
// (AVCaptureDevice authorization status — never opens the device). The WebView must
// never call getUserMedia. This test fails the build if it is reintroduced anywhere.

// Load every frontend source file as raw text via Vite's glob (no node:fs — this is
// a browser/Vite project without node types).
const sources = import.meta.glob('../**/*.{ts,tsx}', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;

const CALL_RE = /getUserMedia\s*\(/;

describe('webview must never open the microphone (issue #177)', () => {
  it('actually scans the source tree (guard against a vacuous pass)', () => {
    // If the glob ever resolves to nothing, the getUserMedia check below would
    // pass for the wrong reason. Fail loudly instead.
    expect(Object.keys(sources).length).toBeGreaterThan(10);
  });

  it('no source file calls getUserMedia', () => {
    const offenders = Object.entries(sources)
      .filter(([path]) => !/\.test\.(ts|tsx)$/.test(path))
      .filter(([, src]) => CALL_RE.test(src))
      .map(([path]) => path);
    expect(
      offenders,
      `getUserMedia() must not be called from the WebView (issue #177 — it ducks ` +
        `other system audio). Use the check_microphone_permission command instead. ` +
        `Found in:\n  ${offenders.join('\n  ')}`
    ).toEqual([]);
  });
});
