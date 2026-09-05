import { describe, expect, it } from 'vitest';
import {
  audioDeviceSelectOptions,
  followSystemDefaultOptionLabel,
  migrateLegacyMicrophoneId,
  parseAudioInputInventory,
  selectedDeviceExists,
  previewSmartAutoSelection,
} from './audioDevices';

const devices = [
  { id: 'BuiltInMicrophoneDevice', name: 'MacBook Microphone', kind: 'builtIn' as const, connected: true, hasInput: true },
  { id: 'USB-A', name: 'Studio Mic', kind: 'external' as const, connected: true, hasInput: true },
];

describe('audio device persistence', () => {
  it('preserves stable IDs and the system default sentinel', () => {
    expect(migrateLegacyMicrophoneId('USB-A', devices)).toBe('USB-A');
    expect(migrateLegacyMicrophoneId('system_default', devices)).toBe('system_default');
  });

  it('migrates an unambiguous legacy display name to its raw device ID', () => {
    expect(migrateLegacyMicrophoneId('Studio Mic', devices)).toBe('USB-A');
  });

  it('fails closed when a legacy display name is ambiguous', () => {
    const duplicates = [...devices, { id: 'USB-B', name: 'Studio Mic', kind: 'external' as const, connected: true, hasInput: true }];
    expect(migrateLegacyMicrophoneId('Studio Mic', duplicates)).toBe('Studio Mic');
    expect(selectedDeviceExists('Studio Mic', duplicates)).toBe(false);
    expect(selectedDeviceExists('Missing Mic', duplicates)).toBe(false);
  });

  it('adds stable IDs only to colliding picker labels', () => {
    const duplicates = [...devices, { id: 'USB-B', name: 'Studio Mic', kind: 'external' as const, connected: true, hasInput: true }];
    expect(audioDeviceSelectOptions(duplicates)).toEqual([
      { value: 'BuiltInMicrophoneDevice', label: 'MacBook Microphone' },
      { value: 'USB-A', label: 'Studio Mic (USB-A)' },
      { value: 'USB-B', label: 'Studio Mic (USB-B)' },
    ]);
  });

  it('makes automatic selection explicit and identifies the live macOS default', () => {
    expect(followSystemDefaultOptionLabel(devices, 'USB-A')).toBe(
      'Follow macOS Default — Studio Mic',
    );
    expect(followSystemDefaultOptionLabel(devices, null)).toBe('Follow macOS Default');
    expect(followSystemDefaultOptionLabel(devices, 'missing')).toBe('Follow macOS Default');
  });

  it('disambiguates duplicate names in the resolved default label', () => {
    const duplicates = [...devices, { id: 'USB-B', name: 'Studio Mic', kind: 'external' as const, connected: true, hasInput: true }];
    expect(followSystemDefaultOptionLabel(duplicates, 'USB-B')).toBe(
      'Follow macOS Default — Studio Mic (USB-B)',
    );
  });
});

describe('parseAudioInputInventory', () => {
  const available = {
    schemaVersion: 2,
    revision: 4,
    status: 'available',
    devices: [{ id: 'uid-1', name: 'Studio Mic', kind: 'external', connected: true, hasInput: true }],
    defaultInputId: 'uid-1',
    lidState: 'open',
    errorCode: null,
  };

  it('accepts the exact v2 contract', () => {
    expect(parseAudioInputInventory(available)).toEqual(available);
  });

  it('accepts exact UTF-8 byte boundaries for stable IDs and display names', () => {
    const id = 'x'.repeat(4096);
    const name = '🎙'.repeat(128);
    expect(parseAudioInputInventory({
      ...available,
      devices: [{ id, name, kind: 'external', connected: true, hasInput: true }],
      defaultInputId: id,
    })?.devices[0]).toEqual({ id, name, kind: 'external', connected: true, hasInput: true });
  });

  it.each([
    { ...available, schemaVersion: 1 },
    { ...available, revision: -1 },
    { ...available, extra: true },
    { ...available, devices: [{ id: 'uid-1', name: 'Mic', kind: 'external', connected: true, hasInput: true, extra: true }] },
    { ...available, devices: Array.from({ length: 257 }, (_, index) => ({ id: `uid-${index}`, name: 'Mic', kind: 'external', connected: true, hasInput: true })) },
    { ...available, devices: [{ id: 'x'.repeat(4097), name: 'Mic', kind: 'external', connected: true, hasInput: true }], defaultInputId: null },
    { ...available, devices: [{ id: 'uid-1', name: '🎙'.repeat(129), kind: 'external', connected: true, hasInput: true }] },
    { ...available, lidState: 'maybe' },
    { ...available, status: 'available', errorCode: 'enumerationFailed' },
    { ...available, status: 'stale', errorCode: null },
    { ...available, status: 'stale', errorCode: 'captureActive' },
    { ...available, status: 'unavailable', errorCode: 'notInitialized' },
  ])('rejects malformed or semantically impossible payload %#', (payload) => {
    expect(parseAudioInputInventory(payload)).toBeNull();
  });
});

describe('previewSmartAutoSelection', () => {
  const request = { approvedDeviceIds: ['built-in', 'anker'], preferredDeviceIds: [], allowContinuity: false };
  const autoDevices = [
    { id: 'built-in', name: 'MacBook Microphone', kind: 'builtIn' as const, connected: true, hasInput: true },
    { id: 'anker', name: 'Anker PowerConf C200', kind: 'external' as const, connected: true, hasInput: true },
  ];

  it('excludes a cached built-in microphone when its lid is closed', () => {
    expect(previewSmartAutoSelection(request, autoDevices, 'built-in', 'closed')?.device.id).toBe('anker');
  });

  it('keeps Continuity out until explicitly allowed', () => {
    const continuity = [{ id: 'iphone', name: 'iPhone', kind: 'continuity' as const, connected: true, hasInput: true }];
    expect(previewSmartAutoSelection({ ...request, approvedDeviceIds: ['iphone'] }, continuity, 'iphone', 'open')).toBeNull();
    expect(previewSmartAutoSelection({ ...request, approvedDeviceIds: ['iphone'], allowContinuity: true }, continuity, 'iphone', 'open')?.reason).toBe('approved_macos_default');
  });
});
