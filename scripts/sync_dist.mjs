#!/usr/bin/env node
// Build the frontend into a staging directory, then sync it into `app/dist`
// touching only files whose bytes changed.
//
// Why: `tauri::generate_context!` embeds every file under `dist` with
// `include_bytes!`, so cargo marks the `ui` crate stale when any of those files
// has a newer mtime than the last compile, even if its content is identical.
// `vite build` empties and rewrites `dist` on every run, which made every
// `tauri build` recompile `ui` (about 30 s) with no frontend change. Leaving
// unchanged files alone keeps their mtimes, so cargo skips the recompile.
//
// Usage (from `app/`): node ../scripts/sync_dist.mjs
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

function listFiles(dir, base = dir) {
  const out = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) out.push(...listFiles(full, base));
    else out.push(path.relative(base, full));
  }
  return out;
}

function removeEmptyDirs(dir, keepRoot = true) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory()) removeEmptyDirs(path.join(dir, entry.name), false);
  }
  if (!keepRoot && fs.readdirSync(dir).length === 0) fs.rmdirSync(dir);
}

function sameBytes(a, b) {
  try {
    const sa = fs.statSync(a);
    const sb = fs.statSync(b);
    if (!sb.isFile() || sa.size !== sb.size) return false;
    return fs.readFileSync(a).equals(fs.readFileSync(b));
  } catch {
    return false;
  }
}

/**
 * Make `dest` byte-identical to `src`, writing only files that differ and
 * deleting files that are no longer in `src`. Unchanged files keep their mtime.
 * Returns the relative paths that were written and removed.
 */
export function syncDir(src, dest) {
  fs.mkdirSync(dest, { recursive: true });
  const wanted = new Set(listFiles(src));
  const written = [];
  const removed = [];
  for (const rel of [...wanted].sort()) {
    const from = path.join(src, rel);
    const to = path.join(dest, rel);
    if (sameBytes(from, to)) continue;
    fs.mkdirSync(path.dirname(to), { recursive: true });
    fs.rmSync(to, { recursive: true, force: true });
    fs.copyFileSync(from, to);
    written.push(rel);
  }
  for (const rel of listFiles(dest).sort()) {
    if (wanted.has(rel)) continue;
    fs.rmSync(path.join(dest, rel), { force: true });
    removed.push(rel);
  }
  removeEmptyDirs(dest);
  return { written, removed };
}

function main() {
  const appDir = process.cwd();
  const viteBin = path.join(appDir, 'node_modules', 'vite', 'bin', 'vite.js');
  const staging = path.join(appDir, 'node_modules', '.cache', 'murmur-dist-staging');
  const dist = path.join(appDir, 'dist');
  fs.rmSync(staging, { recursive: true, force: true });
  const build = spawnSync(
    process.execPath,
    [viteBin, 'build', '--outDir', staging, '--emptyOutDir'],
    { cwd: appDir, stdio: 'inherit' },
  );
  if (build.status !== 0) {
    fs.rmSync(staging, { recursive: true, force: true });
    process.exit(build.status ?? 1);
  }
  const { written, removed } = syncDir(staging, dist);
  fs.rmSync(staging, { recursive: true, force: true });
  console.log(`dist synced: ${written.length} written, ${removed.length} removed, unchanged files kept`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}
