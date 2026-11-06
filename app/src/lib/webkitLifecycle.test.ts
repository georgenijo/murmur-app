import { beforeEach, expect, it, vi } from 'vitest';
const mock = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mock.invoke, isTauri: () => true }));
import { initializeWebkitState, V2_KEYS } from './webkitLifecycle';
beforeEach(() => { localStorage.clear(); mock.invoke.mockReset(); });
it('sends unmigrated raw strings and hydrates only after the Rust commit', async () => {
  localStorage.setItem('murmur-appearance', '{bad');
  mock.invoke.mockImplementation(async (command: string, args?: { source: Record<string, unknown> }) => {
    if(command === 'webkit_state_snapshot') return { v1: false, v2: false, values: {} };
    expect(args?.source['murmur-appearance']).toBe('{bad');
    V2_KEYS.forEach(key => expect(Object.prototype.hasOwnProperty.call(args!.source, key)).toBe(true));
    throw new Error('preserved');
  });
  await expect(initializeWebkitState(true)).rejects.toThrow('preserved');
  expect(localStorage.getItem('murmur-appearance')).toBe('{bad');
});
it('already imported state never sends a second source snapshot and disk wins', async () => {
  localStorage.setItem('usage-dashboard-collapsed', 'true');
  mock.invoke.mockResolvedValue({ v1: true, v2: true, values: { 'usage-dashboard-collapsed': false } });
  await initializeWebkitState(true);
  expect(mock.invoke.mock.calls.filter(([command]) => command === 'import_webkit_state')).toHaveLength(0);
  expect(localStorage.getItem('usage-dashboard-collapsed')).toBe('false');
});
it('removes durably imported transcript keys independently of a corrupt v2 import', async () => {
  const keys = ['dictation-history', 'dictation-stats', 'murmur-benchmark-report', 'murmur-benchmark-reports'];
  keys.forEach(key => localStorage.setItem(key, '[]'));
  mock.invoke.mockImplementation(async (command: string) => {
    if(command === 'webkit_state_snapshot') return { v1: true, v2: false, values: {} };
    if(command === 'import_webkit_state') throw new Error('preserved');
    if(command === 'completion_snapshot') return { revision: 999, v1Committed: true, history: [], reports: [], statistics: {}, removeKeys: keys };
  });
  await expect(initializeWebkitState(true)).rejects.toThrow('preserved');
  keys.forEach(key => expect(localStorage.getItem(key)).toBeNull());
  expect(mock.invoke).toHaveBeenCalledWith('acknowledge_completion_keys', { destinations: ['history','statistics','reports'] });
});
it('retry rebases retained settings on a newer native owned-field commit', async () => {
  const { persistSourceState, flushSourceState, rebasePendingOwnedSettings } = await import('./webkitLifecycle');
  mock.invoke.mockRejectedValueOnce(new Error('disk unavailable'));
  persistSourceState('dictation-settings', JSON.stringify({ autoPaste: false, disabled: false, language: 'es' }));
  await Promise.resolve();
  await expect(flushSourceState()).resolves.toBeUndefined();
  // Add a failed retained intent explicitly before the external commit.
  mock.invoke.mockRejectedValue(new Error('disk unavailable'));
  persistSourceState('dictation-settings', JSON.stringify({ autoPaste: false, disabled: false, language: 'es' }));
  await expect(flushSourceState()).rejects.toThrow();
  rebasePendingOwnedSettings({ autoPaste: true, disabled: true, ownedFieldsRevision: 1 });
  mock.invoke.mockResolvedValue(undefined);
  await flushSourceState();
  const args = mock.invoke.mock.calls.filter(([command]) => command === 'save_settings_blob').slice(-1)[0]?.[1];
  expect(JSON.parse(args.blob)).toMatchObject({ autoPaste: true, disabled: true, ownedFieldsRevision: 1, language: 'es' });
});
it('snapshot failure still removes already-receipted transcript data', async () => {
  localStorage.setItem('dictation-history', '[]');
  mock.invoke.mockImplementation(async (command: string) => {
    if(command === 'webkit_state_snapshot') throw new Error('future schema');
    if(command === 'completion_snapshot') return { revision: 1000, v1Committed: true, history: [], reports: [], statistics: {}, removeKeys: ['dictation-history'] };
  });
  await expect(initializeWebkitState(true)).rejects.toThrow('future schema');
  expect(localStorage.getItem('dictation-history')).toBeNull();
});

it('quota errors cannot suppress Rust writes for UI settings or themes', async () => {
  const { sourceStorage, flushSourceState } = await import('./webkitLifecycle');
  const { saveSettings, DEFAULT_SETTINGS } = await import('./settings');
  const { writeThemeLibrary } = await import('./appearance/themeLibrary');
  mock.invoke.mockResolvedValue(undefined);
  const quota = vi.spyOn(localStorage, 'setItem').mockImplementation(() => { throw new DOMException('synthetic quota', 'QuotaExceededError'); });
  sourceStorage.setItem('usage-dashboard-collapsed', 'true');
  saveSettings({ ...DEFAULT_SETTINGS, language: 'ja' });
  writeThemeLibrary({ version: 1, revision: 1, themes: [] });
  await flushSourceState();
  expect(mock.invoke).toHaveBeenCalledWith('save_webkit_source', { key: 'usage-dashboard-collapsed', value: true });
  expect(mock.invoke.mock.calls.some(([c,a]) => c === 'save_settings_blob' && JSON.parse(a.blob).language === 'ja')).toBe(true);
  expect(mock.invoke.mock.calls.some(([c]) => c === 'save_theme_library_blob')).toBe(true);
  quota.mockRestore();
});
it.each([false, true])('in-flight generation cannot discard the newer edit on failure=%s', async fail => {
  const { persistSourceState, flushSourceState } = await import('./webkitLifecycle');
  let release!: () => void;
  let entered!: () => void;
  const started = new Promise<void>(resolve => { entered = resolve; });
  mock.invoke.mockImplementationOnce(async () => {
    entered(); await new Promise<void>(resolve => { release = resolve; });
    if (fail) throw new Error('synthetic storage failure');
  }).mockResolvedValue(undefined);
  persistSourceState('usage-dashboard-collapsed', 'true');
  await started;
  persistSourceState('usage-dashboard-collapsed', 'false');
  release();
  await flushSourceState();
  expect(mock.invoke).toHaveBeenLastCalledWith('save_webkit_source', { key: 'usage-dashboard-collapsed', value: false });
});
it.each([false, true])('removes obsolete calibration key after v2, settings-present=%s', async present => {
  localStorage.setItem('murmur-overlay-vertical-offset', '48');
  mock.invoke.mockResolvedValue({ v1: true, v2: true, values: present ? { 'dictation-settings': { settingsVersion: 7, overlayVerticalOffset: 0 } } : {} });
  await initializeWebkitState();
  expect(localStorage.getItem('murmur-overlay-vertical-offset')).toBeNull();
});
it('retries interrupted individual transcript removals before acknowledgement', async () => {
  const { removeImportedKeys } = await import('./completionData');
  const keys = ['dictation-history','dictation-stats','murmur-benchmark-report','murmur-benchmark-reports'];
  for (const failing of keys) {
    keys.forEach(k => localStorage.setItem(k,'[]')); mock.invoke.mockClear();
    const original = localStorage.removeItem.bind(localStorage);
    const failure = vi.spyOn(localStorage,'removeItem').mockImplementation(k => { if(k===failing) throw new Error('synthetic interruption'); original(k); });
    const snapshot = { revision: 1001, v1Committed: true, history: [], reports: [], statistics: {} as never, removeKeys: keys, retryNotice: false };
    try { await removeImportedKeys(snapshot); } catch { /* preserved for retry */ }
    expect(localStorage.getItem(failing)).not.toBeNull();
    failure.mockRestore();
    await removeImportedKeys(snapshot);
    keys.forEach(k => expect(localStorage.getItem(k)).toBeNull());
    expect(mock.invoke).toHaveBeenCalledWith('acknowledge_completion_keys', { destinations: ['history','statistics','reports'] });
  }
});

it.each(['autoPaste', 'disabled'] as const)('older %s acknowledgement preserves the newer deliberate field intent', async field => {
  const { persistSourceState, flushSourceState, rebasePendingOwnedSettings } = await import('./webkitLifecycle');
  let release!: () => void;
  let entered!: () => void;
  const started = new Promise<void>(resolve => { entered = resolve; });
  mock.invoke.mockImplementationOnce(async (_command: string, args: { intentGeneration: number }) => {
    entered(); await new Promise<void>(resolve => { release = resolve; });
    const committed = rebasePendingOwnedSettings({ autoPaste: field === 'autoPaste', disabled: field === 'disabled', ownedFieldsRevision: 1, intentGeneration: args.intentGeneration });
    expect(committed[field]).toBe(false);
  }).mockResolvedValue(undefined);
  persistSourceState('dictation-settings', JSON.stringify({ autoPaste: false, disabled: false, [field]: true }), { [field]: true });
  await started;
  persistSourceState('dictation-settings', JSON.stringify({ autoPaste: false, disabled: false }), { [field]: false });
  release();
  await flushSourceState();
  const args = mock.invoke.mock.calls.filter(([c]) => c === 'save_settings_blob').slice(-1)[0]![1];
  expect(JSON.parse(args.blob)[field]).toBe(false);
  expect(args.ownedEdits[field]).toBe(false);
});
it('public onboarding complete and reset persist despite cache write and removal errors', async () => {
  const { markOnboardingComplete, resetOnboarding } = await import('./onboarding');
  const { flushSourceState } = await import('./webkitLifecycle');
  mock.invoke.mockResolvedValue(undefined);
  const write = vi.spyOn(localStorage, 'setItem').mockImplementation(() => { throw new Error('synthetic quota'); });
  const remove = vi.spyOn(localStorage, 'removeItem').mockImplementation(() => { throw new Error('synthetic quota'); });
  markOnboardingComplete(); await flushSourceState();
  expect(mock.invoke).toHaveBeenCalledWith('save_webkit_source', { key: 'murmur_onboarding_complete', value: true });
  resetOnboarding(); await flushSourceState();
  expect(mock.invoke).toHaveBeenCalledWith('save_webkit_source', { key: 'murmur_onboarding_complete', value: false });
  write.mockRestore(); remove.mockRestore();
});

it('submits a new native intent while an earlier publication is still blocked', async () => {
  const { persistSourceState, flushSourceState } = await import('./webkitLifecycle');
  let release!: () => void;
  mock.invoke.mockImplementationOnce(() => new Promise<void>(resolve => { release = resolve; })).mockResolvedValue(undefined);
  persistSourceState('usage-dashboard-collapsed', 'true');
  persistSourceState('usage-dashboard-collapsed', 'false');
  expect(mock.invoke).toHaveBeenCalledTimes(2);
  expect(mock.invoke).toHaveBeenLastCalledWith('save_webkit_source', { key: 'usage-dashboard-collapsed', value: false });
  release(); await flushSourceState();
});

it('a new source edit cannot carry an older owned-field patch past a native override', async () => {
  const { persistSourceState, flushSourceState } = await import('./webkitLifecycle');
  let release!: () => void;
  mock.invoke.mockImplementationOnce(() => new Promise<void>(resolve => { release = resolve; })).mockResolvedValue(undefined);
  persistSourceState('dictation-settings', JSON.stringify({ autoPaste: true, disabled: false, ownedFieldsRevision: 0 }), { autoPaste: true });
  // Native Auto Paste can now commit false while this ack is delayed. A newer
  // deliberate Disable must send only Disable, letting native revision merge
  // preserve that intervening Auto Paste commit.
  persistSourceState('dictation-settings', JSON.stringify({ autoPaste: true, disabled: true, ownedFieldsRevision: 0 }), { disabled: true });
  const latest = mock.invoke.mock.calls.filter(([command]) => command === 'save_settings_blob').slice(-1)[0]![1];
  expect(latest.ownedEdits).toEqual({ disabled: true });
  release(); await flushSourceState();
});

it.each(['autoPaste', 'disabled'] as const)('a successful unrelated save retains the failed %s patch', async field => {
  const { persistSourceState, flushSourceState } = await import('./webkitLifecycle');
  mock.invoke.mockRejectedValueOnce(new Error('synthetic failure'));
  persistSourceState('dictation-settings', JSON.stringify({ [field]: true, language: 'en' }), { [field]: true });
  await new Promise(resolve => setTimeout(resolve, 0));
  mock.invoke.mockResolvedValue(undefined);
  persistSourceState('dictation-settings', JSON.stringify({ [field]: true, language: 'es' }), {});
  await new Promise(resolve => setTimeout(resolve, 0));
  const beforeRetry = mock.invoke.mock.calls.length;
  await flushSourceState();
  expect(mock.invoke.mock.calls.length).toBeGreaterThan(beforeRetry);
  const args = mock.invoke.mock.calls.filter(([c]) => c === 'save_settings_blob').slice(-1)[0]?.[1];
  expect(args.ownedEdits).toEqual({ [field]: true });
  expect(JSON.parse(args.blob).language).toBe('es');
});

it('a different owned-field acknowledgement preserves the failed field until retry or native supersession', async () => {
  const { persistSourceState, flushSourceState, rebasePendingOwnedSettings } = await import('./webkitLifecycle');
  mock.invoke.mockRejectedValueOnce(new Error('synthetic failure'));
  persistSourceState('dictation-settings', JSON.stringify({ autoPaste: true, disabled: false }), { autoPaste: true });
  await new Promise(resolve => setTimeout(resolve, 0));
  mock.invoke.mockImplementation(async (_command: string, args: { intentGeneration: number }) => {
    const committed = rebasePendingOwnedSettings({ autoPaste: false, disabled: true, ownedFieldsRevision: 1,
      intentGeneration: args.intentGeneration, acknowledgedFields: ['disabled'] });
    expect(committed.autoPaste).toBe(true);
  });
  persistSourceState('dictation-settings', JSON.stringify({ autoPaste: true, disabled: true }), { disabled: true });
  await new Promise(resolve => setTimeout(resolve, 0));
  const native = rebasePendingOwnedSettings({ field: 'autoPaste', autoPaste: false, disabled: true, ownedFieldsRevision: 2 });
  expect(native.autoPaste).toBe(false);
  mock.invoke.mockResolvedValue(undefined);
  await flushSourceState();
  const args = mock.invoke.mock.calls.filter(([c]) => c === 'save_settings_blob').slice(-1)[0]?.[1];
  expect(args.ownedEdits).toEqual({});
  expect(JSON.parse(args.blob).autoPaste).toBe(false);
});
