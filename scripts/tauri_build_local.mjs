#!/usr/bin/env node
// `npm run tauri:build:local`: a local release `Murmur.app` that rebuilds fast
// after an edit. Same optimized release profile as `npm run tauri build`; only
// the order and overlap of the checks around the compile change.
//
// `tauri build` runs `beforeBuildCommand` (`npm run build` = `tsc && vite build`)
// to completion before it starts cargo, so a one-line edit waited for a full
// ~14 s type check, then ~5 s of vite, then the compile. Here:
//   - the type check is incremental (a warm `.tsbuildinfo` re-checks only what
//     the edit touches, ~4 s instead of ~14 s of CPU), and
//   - it runs while vite and cargo work, and the build waits for its verdict
//     at the end. A type error still fails the command (and stops cargo early).
//
// The shipped path is untouched: `npm run build`, `npm run tauri build` and
// release CI keep the sequential `tsc && sync_dist` of `beforeBuildCommand`.
// This script overrides that hook for this one invocation (`--config`), so it
// also gets a different embedded config string than a release build; it is a
// local test build, never a release artifact.
//
// Usage (from `app/`): node ../scripts/tauri_build_local.mjs [tauri build args]
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';

const appDir = process.cwd();
const tscBin = path.join(appDir, 'node_modules', 'typescript', 'bin', 'tsc');
const tauriBin = path.join(appDir, 'node_modules', '@tauri-apps', 'cli', 'tauri.js');
const cacheDir = path.join(appDir, 'node_modules', '.cache');

fs.mkdirSync(cacheDir, { recursive: true });

// Only vite + the dist sync run before cargo; tsc runs beside the build.
const config = JSON.stringify({ build: { beforeBuildCommand: 'node ../scripts/sync_dist.mjs' } });

const typeCheck = spawn(
  process.execPath,
  [
    tscBin,
    '--noEmit',
    '--incremental',
    '--tsBuildInfoFile',
    path.join(cacheDir, 'murmur-local-build.tsbuildinfo'),
  ],
  { cwd: appDir, stdio: ['ignore', 'pipe', 'pipe'] },
);
let typeCheckOutput = '';
typeCheck.stdout.on('data', (chunk) => (typeCheckOutput += chunk));
typeCheck.stderr.on('data', (chunk) => (typeCheckOutput += chunk));

// Its own process group, so a failed type check can stop tauri and cargo together.
const build = spawn(
  process.execPath,
  [tauriBin, 'build', '--bundles', 'app', '--config', config, ...process.argv.slice(2)],
  {
    cwd: appDir,
    stdio: 'inherit',
    detached: true,
    env: { ...process.env, CARGO_PROFILE_RELEASE_INCREMENTAL: 'true' },
  },
);

let typeCheckCode = null;
let buildCode = null;
let finished = false;

function stopBuild() {
  try {
    process.kill(-build.pid, 'SIGTERM');
  } catch {
    // already gone
  }
}

function finish(code) {
  if (finished) return;
  finished = true;
  process.exit(code);
}

function settle() {
  if (typeCheckCode === null || buildCode === null) return;
  finish(typeCheckCode !== 0 ? typeCheckCode : buildCode);
}

typeCheck.on('error', (error) => {
  console.error(`failed to start tsc: ${error.message}`);
  typeCheckCode = 1;
  stopBuild();
});
typeCheck.on('close', (code) => {
  typeCheckCode = code ?? 1;
  if (typeCheckCode !== 0) {
    process.stderr.write(typeCheckOutput);
    console.error(`\ntype check failed (tsc exit ${typeCheckCode}); stopping the build`);
    stopBuild();
    // The build's own close handler reports the exit; do not wait on a hung child.
    setTimeout(() => finish(typeCheckCode), 5000).unref();
  }
  settle();
});
build.on('error', (error) => {
  console.error(`failed to start tauri: ${error.message}`);
  buildCode = 1;
  typeCheck.kill('SIGTERM');
  finish(1);
});
build.on('close', (code) => {
  buildCode = code ?? 1;
  if (buildCode !== 0) typeCheck.kill('SIGTERM');
  if (buildCode !== 0 && typeCheckCode === null) finish(buildCode);
  settle();
});

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => {
    stopBuild();
    typeCheck.kill('SIGTERM');
    finish(130);
  });
}
