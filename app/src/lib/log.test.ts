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

  it('forwards a string event code to the Rust logging command', () => {
    flog.info('updater', 'no update available', {
      event_code: 'updater.check_current',
    });

    expect(mocks.invoke).toHaveBeenCalledWith('log_frontend', {
      level: 'INFO',
      message:
        '[updater] no update available {"event_code":"updater.check_current"}',
      transformPassId: null,
      eventCode: 'updater.check_current',
    });
  });

  it.each([
    ['absent', undefined],
    ['non-string', { event_code: 42 }],
  ])('sends a null event code when the field is %s', (_label, data) => {
    flog.warn('updater', 'check failed', data);

    expect(mocks.invoke).toHaveBeenCalledWith('log_frontend', {
      level: 'WARN',
      message: data
        ? '[updater] check failed {"event_code":42}'
        : '[updater] check failed',
      transformPassId: null,
      eventCode: null,
    });
  });
});
