import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

import { configure } from './dictation';

function pendingResponse() {
  let finish = (_value: { type: string }) => {};
  const promise = new Promise<{ type: string }>((resolve) => { finish = resolve; });
  return { promise, finish };
}

describe('configure_dictation ordering', () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue({ type: 'configured' });
  });

  it('applies rapid vocabulary edits in order so the saved alias wins', async () => {
    const older = pendingResponse();
    mocks.invoke.mockReturnValueOnce(older.promise);
    const first = configure({ vocabularyEntries: [{
      id: 'term', written: 'Tauri', aliases: ['Tori'], enabled: true, scope: { kind: 'global' },
    }] });
    const second = configure({ vocabularyEntries: [{
      id: 'term', written: 'Tauri', aliases: ['Tori', 'Tory'], enabled: true, scope: { kind: 'global' },
    }] });

    await Promise.resolve();
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    older.finish({ type: 'configured' });
    await Promise.all([first, second]);

    expect(mocks.invoke.mock.calls.map(([, args]) => args.options.vocabularyEntries[0].aliases))
      .toEqual([['Tori'], ['Tori', 'Tory']]);
  });

  it('allows the next saved configuration after a rejected edit', async () => {
    mocks.invoke.mockRejectedValueOnce(new Error('invalid alias'));
    const rejected = configure({ vocabularyEntries: [] });
    const latest = configure({ vocabularyEntries: [{
      id: 'term', written: 'Tauri', aliases: ['Tory'], enabled: true, scope: { kind: 'global' },
    }] });

    await expect(rejected).rejects.toThrow('invalid alias');
    await expect(latest).resolves.toEqual({ type: 'configured' });
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
  });

  it('reads startup settings after a queued rejected edit has rolled back', async () => {
    let currentAliases = ['invalid'];
    mocks.invoke.mockRejectedValueOnce(new Error('invalid alias'));
    const rejected = configure({ vocabularyEntries: [] });
    const startup = configure(() => ({ vocabularyEntries: [{
      id: 'term', written: 'Tauri', aliases: currentAliases, enabled: true,
      scope: { kind: 'global' },
    }] }));
    await rejected.catch(() => { currentAliases = ['Tory']; });
    await startup;

    expect(mocks.invoke.mock.calls[1][1].options.vocabularyEntries[0].aliases).toEqual(['Tory']);
  });
});
