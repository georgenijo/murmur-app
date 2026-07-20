import { describe, expect, it } from 'vitest';
import {
  getOverlayPreviewPresentation,
  latestPreviewText,
  supportsLiveTranscriptPreview,
} from './previewPresentation';

describe('overlay preview presentation', () => {
  it('renders Whisper partial text only while the session is active', () => {
    expect(getOverlayPreviewPresentation('recording', true, 'small.en', 'visible draft')).toEqual({
      previewText: 'visible draft',
      unavailable: false,
      visible: true,
    });
    expect(getOverlayPreviewPresentation('idle', true, 'small.en', 'stale draft').visible).toBe(false);
    expect(getOverlayPreviewPresentation('recording', false, 'small.en', 'disabled draft').visible).toBe(false);
  });

  it('shows an explicit final-only state for Parakeet and Core ML', () => {
    expect(supportsLiveTranscriptPreview('parakeet-tdt-0.6b-v2-fp16')).toBe(false);
    expect(supportsLiveTranscriptPreview('parakeet-tdt-0.6b-v3-coreml')).toBe(false);
    expect(getOverlayPreviewPresentation(
      'recording',
      false,
      'parakeet-tdt-0.6b-v3-coreml',
      '',
    )).toEqual({ previewText: '', unavailable: true, visible: true });
  });

  it('keeps the latest bounded suffix without exposing an oversized row', () => {
    const words = `${'old '.repeat(50)}the latest provisional words`;
    const preview = latestPreviewText(words, 32);
    const cumulative = latestPreviewText(
      'First native provisional update followed by a visibly newer second update',
    );
    expect(preview.startsWith('…')).toBe(true);
    expect(preview.endsWith('latest provisional words')).toBe(true);
    expect(preview).not.toContain('old old old old old old old old');
    expect(cumulative).toBe('…by a visibly newer second update');
  });
});
