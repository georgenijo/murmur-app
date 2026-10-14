import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { formatBundleSize, previewMeetingBundle, saveMeetingBundle, saveMeetingExport } from './meetings';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: vi.fn() }));

describe('meeting caption file export', () => {
  beforeEach(() => vi.resetAllMocks());

  it.each(['srt', 'vtt'] as const)('saves %s through the native meeting export boundary', async (format) => {
    vi.mocked(save).mockResolvedValue(`/tmp/captions.${format}`);
    vi.mocked(invoke).mockResolvedValue(80);

    expect(await saveMeetingExport('meeting', Date.UTC(2026, 8, 10), format)).toBe(`/tmp/captions.${format}`);
    expect(save).toHaveBeenCalledWith({
      defaultPath: `Murmur Meeting 2026-09-10.${format}`,
      filters: [{ name: `Murmur meeting ${format}`, extensions: [format] }],
    });
    expect(invoke).toHaveBeenCalledWith('save_meeting_review_export', {
      id: 'meeting', format, path: `/tmp/captions.${format}`,
    });
  });

  it('does not render or write captions when the save dialog is cancelled', async () => {
    vi.mocked(save).mockResolvedValue(null);
    expect(await saveMeetingExport('meeting', 0, 'vtt')).toBeNull();
    expect(invoke).not.toHaveBeenCalled();
  });

  it('keeps a native export failure visible to the caller', async () => {
    vi.mocked(save).mockResolvedValue('/tmp/captions.srt');
    vi.mocked(invoke).mockRejectedValue(new Error('Captions were not exported.'));
    await expect(saveMeetingExport('meeting', 0, 'srt')).rejects.toThrow('Captions were not exported.');
  });
});

describe('meeting bundle export', () => {
  beforeEach(() => vi.resetAllMocks());

  it('asks Rust for the bundle size before any save dialog', async () => {
    const preview = { audioSegments: 2, audioBytes: 2048, unavailableSegments: 0, maxAudioBytes: 4096 };
    vi.mocked(invoke).mockResolvedValue(preview);
    expect(await previewMeetingBundle('meeting')).toEqual(preview);
    expect(invoke).toHaveBeenCalledWith('get_meeting_bundle_preview', { id: 'meeting' });
    expect(save).not.toHaveBeenCalled();
  });

  it('saves a .zip through the native bundle command with the selected document format', async () => {
    const receipt = { bytes: 900, includedSegments: 2, omittedSegments: 1 };
    vi.mocked(save).mockResolvedValue('/tmp/meeting.zip');
    vi.mocked(invoke).mockResolvedValue(receipt);

    expect(await saveMeetingBundle('meeting', Date.UTC(2026, 8, 10), 'json')).toEqual({ path: '/tmp/meeting.zip', receipt });
    expect(save).toHaveBeenCalledWith({
      defaultPath: 'Murmur Meeting 2026-09-10.zip',
      filters: [{ name: 'Murmur meeting with audio', extensions: ['zip'] }],
    });
    expect(invoke).toHaveBeenCalledWith('save_meeting_bundle_export', { id: 'meeting', format: 'json', path: '/tmp/meeting.zip' });
  });

  it('does not touch audio when the save dialog is cancelled', async () => {
    vi.mocked(save).mockResolvedValue(null);
    expect(await saveMeetingBundle('meeting', 0, 'markdown')).toBeNull();
    expect(invoke).not.toHaveBeenCalled();
  });

  it('keeps a native refusal visible to the caller', async () => {
    vi.mocked(save).mockResolvedValue('/tmp/meeting.zip');
    vi.mocked(invoke).mockRejectedValue(new Error('Audio was not retained for this meeting.'));
    await expect(saveMeetingBundle('meeting', 0, 'text')).rejects.toThrow('not retained');
  });

  it('formats sizes for the confirmation', () => {
    expect(formatBundleSize(1)).toBe('1 KB');
    expect(formatBundleSize(5 * 1024 * 1024)).toBe('5.0 MB');
    expect(formatBundleSize(3 * 1024 * 1024 * 1024)).toBe('3.00 GB');
  });
});
