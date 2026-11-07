import { beforeEach, expect, it, vi } from 'vitest';
const mock = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mock.invoke }));
beforeEach(() => { vi.resetModules(); mock.invoke.mockReset(); });

it('registers once and holds every concurrent producer until native owns the session', async () => {
  let registered!: () => void;
  mock.invoke.mockImplementation((command: string) => command === 'register_source_session'
    ? new Promise<void>(resolve => { registered = resolve; }) : Promise.resolve());
  const version = await import('./sourceWriteVersion');
  const a = version.invokeSource('save_settings_blob', version.nextSourceWriteVersion());
  const b = version.invokeSource('save_theme_library_blob', version.nextSourceWriteVersion());
  expect(mock.invoke.mock.calls).toEqual([['register_source_session', { sourceSession: version.sourceSession }]]);
  registered(); await Promise.all([a, b]);
  expect(mock.invoke.mock.calls.map(([command]) => command)).toEqual(['register_source_session', 'save_settings_blob', 'save_theme_library_blob']);
});
it('a reloaded renderer registers a fresh namespace before generation one', async () => {
  mock.invoke.mockResolvedValue(undefined);
  const old = await import('./sourceWriteVersion');
  await old.invokeSource('save_settings_blob', old.nextSourceWriteVersion());
  vi.resetModules();
  const fresh = await import('./sourceWriteVersion');
  const intent = fresh.nextSourceWriteVersion();
  expect(intent.intentGeneration).toBe(1);
  expect(intent.sourceSession).not.toBe(old.sourceSession);
  await fresh.invokeSource('save_settings_blob', intent);
  expect(mock.invoke.mock.calls.slice(-2)).toEqual([
    ['register_source_session', { sourceSession: fresh.sourceSession }], ['save_settings_blob', intent],
  ]);
});
it('a failed handshake never sends a write and a retry must register first', async () => {
  mock.invoke.mockRejectedValueOnce(new Error('registration unavailable')).mockResolvedValue(undefined);
  const version = await import('./sourceWriteVersion');
  await expect(version.invokeSource('save_settings_blob', version.nextSourceWriteVersion())).rejects.toThrow('registration unavailable');
  expect(mock.invoke).toHaveBeenCalledTimes(1);
  await version.invokeSource('save_settings_blob', version.nextSourceWriteVersion());
  expect(mock.invoke.mock.calls.map(([command]) => command)).toEqual(['register_source_session', 'register_source_session', 'save_settings_blob']);
});
