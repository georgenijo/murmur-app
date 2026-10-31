#!/usr/bin/env node
// Compare the real native simulation with the real frontend reference at
// mixed display rates, through every state/style and Reduced Motion transition.
import { createRequire } from 'node:module';
import { mkdtempSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { execFileSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

if (process.platform !== 'darwin') throw new Error('Native dot-scope parity requires macOS');
const root = resolve(import.meta.dirname, '..');
const require = createRequire(join(root, 'app/package.json'));
const { buildSync } = require('esbuild');
const work = mkdtempSync(join(tmpdir(), 'murmur-dot-parity-'));
try {
  const bundle = join(work, 'scope.mjs');
  buildSync({ entryPoints: [join(root, 'app/src/components/overlay/dotScope.ts')],
    bundle: true, platform: 'node', format: 'esm', outfile: bundle });
  const { DotScope } = await import(pathToFileURL(bundle).href);
  const cases = [];
  for (const style of ['sweep', 'flow', 'pulse']) for (const reduced of [false, true]) {
    const scope = new DotScope(); const frames = []; const expected = [];
    const sequence = ['idle', 'recording', 'processing', 'done', 'recording', 'idle'];
    for (let section = 0; section < sequence.length; section++) for (let f = 0; f < 90; f++) {
      const dt = [1 / 30, 1 / 60, 1 / 120][f % 3];
      const db = -44 + 14 * Math.sin((f + section * 90) * 0.17);
      // Toggle motion mid-session too: no resets or old-stage state may leak.
      const motion = f < 60 ? reduced : !reduced;
      frames.push({ dt, stage: section === 0 || section === 5 ? 0 : sequence[section] === 'recording' ? 1 : sequence[section] === 'processing' ? 2 : 3,
        db, reduced: motion });
      scope.advance(dt, sequence[section], db, motion); scope.sample(dt, style, motion);
      if (f % 15 === 0 || f === 89) expected.push({ frame: frames.length - 1,
        values: [scope.width.x, scope.scale.x, scope.opacity.x, scope.blur.x, ...scope.dots] });
    }
    cases.push({ style: ['sweep', 'flow', 'pulse'].indexOf(style), frames, expected });
  }
  writeFileSync(join(work, 'input.json'), JSON.stringify(cases));
  writeFileSync(join(work, 'main.swift'), `
import Foundation
let data = try Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1]))
let cases = try JSONSerialization.jsonObject(with: data) as! [[String: Any]]
var results = [[[Double]]]()
for item in cases {
    let scope = NativeDotScope(), style = item["style"] as! Int
    let frames = item["frames"] as! [[String: Any]]
    let checkpoints = Set((item["expected"] as! [[String: Any]]).map { $0["frame"] as! Int })
    var actual = [[Double]]()
    for (index, frame) in frames.enumerated() {
        let dt = frame["dt"] as! Double, reduced = frame["reduced"] as! Bool
        scope.advance(dt, frame["stage"] as! Int, frame["db"] as! Double, reduced)
        scope.sample(dt, style, reduced)
        if checkpoints.contains(index) {
            actual.append([scope.width.x, scope.scale.x, scope.opacity.x, scope.blur.x] + scope.dots.map(Double.init))
        }
    }
    results.append(actual)
}
FileHandle.standardOutput.write(try JSONSerialization.data(withJSONObject: results))
`);
  execFileSync('xcrun', ['swiftc', '-O', join(root, 'app/src-tauri/macos/OverlayDotRenderer.swift'),
    join(work, 'main.swift'), '-o', join(work, 'parity')], { stdio: 'inherit' });
  const actual = JSON.parse(execFileSync(join(work, 'parity'), [join(work, 'input.json')], { maxBuffer: 8 * 1024 * 1024 }));
  let compared = 0, maxError = 0;
  for (let c = 0; c < cases.length; c++) for (let f = 0; f < cases[c].expected.length; f++) {
    const expected = cases[c].expected[f].values;
    for (let n = 0; n < expected.length; n++) {
      const error = Math.abs(expected[n] - actual[c][f][n]); maxError = Math.max(maxError, error); compared++;
      if (!Number.isFinite(error) || error > 1e-6) throw new Error(`Native mismatch: case ${c}, checkpoint ${f}, value ${n}, ${error}`);
    }
  }
  console.log(JSON.stringify({ passed: true, scenarios: cases.length, frames: cases.reduce((sum, c) => sum + c.frames.length, 0), compared, maxError }));
} finally { rmSync(work, { recursive: true }); }
