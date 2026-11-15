import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

import { flog } from './log';

// Raw text of the TypeScript union and of the Rust allowlist (Vite glob, no node:fs).
const logSource = Object.values(
  import.meta.glob('./log.ts', { query: '?raw', import: 'default', eager: true }),
)[0] as string;
const telemetrySource = Object.values(
  import.meta.glob('../../src-tauri/src/telemetry.rs', {
    query: '?raw',
    import: 'default',
    eager: true,
  }),
)[0] as string;

function unionCodes(): string[] {
  const union = logSource.match(/export type FrontendEventCode =([^;]+);/)?.[1] ?? '';
  return [...union.matchAll(/'([a-z_.]+)'/g)].map((match) => match[1]).sort();
}

/** Codes `canonical_event_code` keeps that the frontend sends: hydration and updater. */
function allowlistedFrontendCodes(): string[] {
  const body = telemetrySource.match(/fn canonical_event_code\b[\s\S]*?\n\}\n/)?.[0] ?? '';
  return [...body.matchAll(/^\s*"((?:system\.launch_frontend_hydrated|updater\.[a-z_]+))" =>/gm)]
    .map((match) => match[1])
    .sort();
}

describe('FrontendEventCode and the Rust allowlist', () => {
  it('list the same codes, so a union-only addition cannot silently be dropped', () => {
    const union = unionCodes();
    expect(union.length).toBeGreaterThan(0);
    expect(allowlistedFrontendCodes()).toEqual(union);
  });
});

describe('flog event codes', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('forwards the content-free frontend hydration milestone', () => {
    flog.info('system.launch_frontend_hydrated');
    expect(mocks.invoke).toHaveBeenCalledWith('log_frontend', {
      level: 'INFO',
      eventCode: 'system.launch_frontend_hydrated',
      updaterVersion: null,
      updaterSelection: null,
    });
  });

  it('sends the level and event code with no message', () => {
    flog.warn('updater.check_failed');

    expect(mocks.invoke).toHaveBeenCalledWith('log_frontend', {
      level: 'WARN',
      eventCode: 'updater.check_failed',
      updaterVersion: null,
      updaterSelection: null,
    });
  });

  it('forwards updater version and selection as separate structured fields', () => {
    flog.info('updater.download_starting', { version: '0.24.0', selection: 'fresh_check' });
    expect(mocks.invoke).toHaveBeenCalledWith('log_frontend', expect.objectContaining({
      updaterVersion: '0.24.0', updaterSelection: 'fresh_check',
    }));
  });
});
