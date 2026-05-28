import { describe, it, expect } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

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

const here = dirname(fileURLToPath(import.meta.url));
const SRC_DIR = resolve(here, '..'); // app/src
const CALL_RE = /getUserMedia\s*\(/;

function sourceFiles(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    if (statSync(p).isDirectory()) {
      out.push(...sourceFiles(p));
    } else if (/\.(ts|tsx)$/.test(p) && !/\.test\.(ts|tsx)$/.test(p)) {
      out.push(p);
    }
  }
  return out;
}

describe('webview must never open the microphone (issue #177)', () => {
  it('no source file calls getUserMedia', () => {
    const offenders = sourceFiles(SRC_DIR).filter((f) =>
      CALL_RE.test(readFileSync(f, 'utf8'))
    );
    expect(
      offenders,
      `getUserMedia() must not be called from the WebView (issue #177 — it ducks ` +
        `other system audio). Use the check_microphone_permission command instead. ` +
        `Found in:\n  ${offenders.join('\n  ')}`
    ).toEqual([]);
  });
});
