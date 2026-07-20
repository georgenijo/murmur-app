import type { DictationStatus } from '../../lib/types';

/**
 * Truncate a live-transcript draft to its latest bounded suffix, breaking on a
 * word boundary and prefixing with an ellipsis so the below-notch preview row
 * never grows unbounded while a long recording is in progress.
 */
export function latestPreviewText(text: string, maxCharacters = 36): string {
  const normalized = text.trim();
  if (normalized.length <= maxCharacters) return normalized;
  const suffix = normalized.slice(-maxCharacters);
  const firstBoundary = suffix.indexOf(' ');
  return `…${(firstBoundary >= 0 ? suffix.slice(firstBoundary + 1) : suffix).trim()}`;
}

/** Parakeet models never emit incremental partial transcripts. */
export function supportsLiveTranscriptPreview(model: string): boolean {
  return !model.startsWith('parakeet-');
}

export interface OverlayPreviewPresentation {
  previewText: string;
  unavailable: boolean;
  visible: boolean;
}

/**
 * Pure derivation of the below-notch preview row's content from status +
 * settings + the live partial-transcript draft. Unavailable (final-only) is
 * shown only while actively recording on a backend that cannot produce partials.
 */
export function getOverlayPreviewPresentation(
  status: DictationStatus,
  enabled: boolean,
  model: string,
  text: string,
): OverlayPreviewPresentation {
  const supported = supportsLiveTranscriptPreview(model);
  const active = status === 'recording' || status === 'processing';
  const previewText = enabled && supported && active ? latestPreviewText(text) : '';
  const unavailable = !supported && status === 'recording';
  return {
    previewText,
    unavailable,
    visible: Boolean(previewText) || unavailable,
  };
}
