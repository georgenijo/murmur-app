import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { meetingAudioExportNotice, saveMeetingAudioExport, saveMeetingExport, supportsMeetingAudioExport } from './meetings';

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

describe('meeting review and audio ZIP export', () => {
  beforeEach(() => vi.resetAllMocks());

  it('asks for a .zip destination and sends only identifiers, format, and the chosen path', async () => {
    vi.mocked(save).mockResolvedValue('/tmp/meeting.zip');
    vi.mocked(invoke).mockResolvedValue({ bytes: 9, audioFiles: 2, missingAudio: 1, pendingAudio: 0 });

    expect(await saveMeetingAudioExport('meeting', Date.UTC(2026, 8, 10), 'markdown')).toEqual({
      path: '/tmp/meeting.zip', bytes: 9, audioFiles: 2, missingAudio: 1, pendingAudio: 0,
    });
    expect(save).toHaveBeenCalledWith({
      defaultPath: 'Murmur Meeting 2026-09-10 with audio.zip',
      filters: [{ name: 'Murmur meeting with audio', extensions: ['zip'] }],
    });
    expect(invoke).toHaveBeenCalledWith('save_meeting_review_audio_export', {
      id: 'meeting', format: 'markdown', path: '/tmp/meeting.zip',
    });
  });

  it('does nothing when the save dialog is cancelled and surfaces native refusals', async () => {
    vi.mocked(save).mockResolvedValue(null);
    expect(await saveMeetingAudioExport('meeting', 0, 'json')).toBeNull();
    expect(invoke).not.toHaveBeenCalled();
    vi.mocked(save).mockResolvedValue('/tmp/meeting.zip');
    vi.mocked(invoke).mockRejectedValue('This meeting did not keep its audio, so there is no audio to export.');
    await expect(saveMeetingAudioExport('meeting', 0, 'text')).rejects.toMatch(/did not keep its audio/);
  });

  it('only supports document formats and words gaps visibly', () => {
    expect(['markdown', 'text', 'json', 'srt', 'vtt'].map((format) => supportsMeetingAudioExport(format as never)))
      .toEqual([true, true, true, false, false]);
    expect(meetingAudioExportNotice({ path: 'p', bytes: 1, audioFiles: 1, missingAudio: 0, pendingAudio: 0 }))
      .toBe('Review and 1 audio file exported.');
    expect(meetingAudioExportNotice({ path: 'p', bytes: 1, audioFiles: 4, missingAudio: 1, pendingAudio: 0 }))
      .toContain('Not included: 1 missing or unreadable.');
  });
});
