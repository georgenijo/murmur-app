import type { ReviewErrorCode, ReviewState } from '../../lib/transformReview';
import { REVIEW_ERROR_COPY } from '../../lib/transformReview';
import { REVIEW_STILL_WORKING_HINT_MS } from '../../lib/overlayMotion';

/**
 * Raw state-machine data driving one render of the review popover: the
 * current state, an optional error code (only meaningful for `failed`), the
 * text content fetched via `get_transform_review_content`, and how long the
 * popover has been in `thinking` (used to gate the "Still working…" hint).
 */
export interface ReviewStateInput {
  state: ReviewState;
  errorCode?: ReviewErrorCode | null;
  instruction: string;
  original: string;
  proposed: string;
  thinkingElapsedMs: number;
}

/**
 * Pure view model: everything a presentational component needs to render one
 * state, with no knowledge of timers, events, or Tauri commands. Fully
 * determined by `ReviewStateInput` — no React, no I/O.
 */
export interface ReviewViewModel {
  state: ReviewState;
  /** Instruction chip text ("Listening…" placeholder while listening). */
  chipText: string;
  showOnDeviceBadge: boolean;
  showWaveform: boolean;
  subText: string | null;
  statusText: string | null;
  showStillWorkingHint: boolean;
  showDiff: boolean;
  cancelEnabled: boolean;
  retryEnabled: boolean;
  approveEnabled: boolean;
  /** Enter/Esc/Cmd+R keyboard shortcuts are only wired in ready/failed. */
  keyboardActionsActive: boolean;
  errorMessage: string | null;
  showUndo: boolean;
}

function baseViewModel(state: ReviewState, chipText: string): ReviewViewModel {
  return {
    state,
    chipText,
    showOnDeviceBadge: true,
    showWaveform: false,
    subText: null,
    statusText: null,
    showStillWorkingHint: false,
    showDiff: false,
    cancelEnabled: false,
    retryEnabled: false,
    approveEnabled: false,
    keyboardActionsActive: false,
    errorMessage: null,
    showUndo: false,
  };
}

function errorMessageFor(errorCode: ReviewErrorCode | null | undefined): string {
  if (errorCode) return REVIEW_ERROR_COPY[errorCode];
  return 'Something went wrong';
}

export function deriveReviewState(input: ReviewStateInput): ReviewViewModel {
  const { state, errorCode, instruction, thinkingElapsedMs } = input;

  switch (state) {
    case 'listening':
      return {
        ...baseViewModel(state, 'Listening…'),
        showWaveform: true,
        subText: 'Release key when done',
      };

    case 'thinking':
      return {
        ...baseViewModel(state, instruction || 'Listening…'),
        statusText: 'Transforming…',
        cancelEnabled: true,
        showStillWorkingHint: thinkingElapsedMs >= REVIEW_STILL_WORKING_HINT_MS,
      };

    case 'ready':
      return {
        ...baseViewModel(state, instruction),
        showDiff: true,
        cancelEnabled: true,
        retryEnabled: true,
        approveEnabled: true,
        keyboardActionsActive: true,
      };

    case 'failed':
      return {
        ...baseViewModel(state, instruction),
        cancelEnabled: true,
        retryEnabled: true,
        keyboardActionsActive: true,
        errorMessage: errorMessageFor(errorCode),
      };

    case 'applied':
      return {
        ...baseViewModel(state, instruction),
        showUndo: true,
      };

    default: {
      const exhaustive: never = state;
      return baseViewModel(exhaustive, '');
    }
  }
}
