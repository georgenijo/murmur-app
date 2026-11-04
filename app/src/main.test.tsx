import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  render: vi.fn(), settings: vi.fn(), history: vi.fn(), themes: vi.fn(), log: vi.fn(),
}));
vi.mock('react-dom/client', () => ({ default: { createRoot: () => ({ render: mocks.render }) } }));
vi.mock('./App', () => ({ default: () => null }));
vi.mock('./lib/windowLifecycle', () => ({ NativeWindowLifecycle: () => null }));
vi.mock('./lib/hooks/useAppearance', () => ({ AppearanceProvider: () => null }));
vi.mock('./lib/settings', () => ({ hydrateSettingsFromDisk: () => mocks.settings(), ownSettingsMigrations: vi.fn() }));
vi.mock('./lib/durableUserData', () => ({
  hydrateUserDataFromDisk: () => mocks.history(), hydrateThemeLibraryFromDisk: () => mocks.themes(),
}));
vi.mock('./lib/log', () => ({ flog: { info: (...args: unknown[]) => mocks.log(...args) } }));

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => { resolve = done; });
  return { promise, resolve };
}

beforeEach(() => { vi.resetModules(); vi.clearAllMocks(); });

describe('main hydration ordering', () => {
  it.each(['settings first', 'settings last'])('waits for authoritative settings and mutation-owning caches (%s)', async (order) => {
    const settings = deferred();
    const history = deferred();
    const themes = deferred();
    mocks.settings.mockReturnValue(settings.promise);
    mocks.history.mockReturnValue(history.promise);
    mocks.themes.mockReturnValue(themes.promise);
    await import('./main');
    expect(mocks.render).not.toHaveBeenCalled();
    if (order === 'settings first') settings.resolve();
    else { history.resolve(); themes.resolve(); }
    await Promise.resolve();
    expect(mocks.render).not.toHaveBeenCalled();
    settings.resolve(); history.resolve(); themes.resolve();
    await vi.waitFor(() => expect(mocks.render).toHaveBeenCalledTimes(1));
    expect(mocks.log).toHaveBeenCalledWith('main', 'Frontend hydrated', {
      event_code: 'system.launch_frontend_hydrated',
    });
  });
});
