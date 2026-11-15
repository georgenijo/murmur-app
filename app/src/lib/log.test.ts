import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

import { flog } from './log';

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
