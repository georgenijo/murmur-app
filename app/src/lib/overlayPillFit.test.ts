import { describe, expect, it } from 'vitest';
import geometry from '../components/overlay/overlay-geometry.fixture.json';
import {
  PILL_COMPACT_W, PILL_FIT_CHROME, PILL_FIT_REST_W, overlayPillFitWidth, pillWordsMaxText, waveWidth,
} from './overlayPillFit';

describe('overlayPillFitWidth', () => {
  const max = geometry.pillWords.island.pillW;

  it('matches the compact pill Rust sizes', () => {
    expect(PILL_COMPACT_W).toBe(geometry.pillRecording.island.pillW);
    expect(waveWidth(4)).toBe(12.5);
    expect(waveWidth(12)).toBe(40.5);
  });

  it('hugs the front line, in whole points, up to the words size', () => {
    expect(overlayPillFitWidth(true, 'words', max, true, 40)).toBe(Math.ceil(PILL_FIT_CHROME + 40 + 1));
    expect(overlayPillFitWidth(true, 'words', max, true, pillWordsMaxText(max))).toBe(max);
    expect(overlayPillFitWidth(true, 'words', max, true, 999)).toBe(max);
  });

  it('hugs the dot and full waveform before words, and rests at the compact width outside recording', () => {
    expect(PILL_FIT_REST_W).toBe(74);
    expect(overlayPillFitWidth(true, 'words', max, true, null)).toBe(PILL_FIT_REST_W);
    expect(overlayPillFitWidth(true, 'words', max, false, 80)).toBe(PILL_COMPACT_W);
  });

  it('leaves the notch, compact and warning sizes alone', () => {
    expect(overlayPillFitWidth(false, 'words', max, true, 40)).toBeUndefined();
    expect(overlayPillFitWidth(true, 'compact', 120, true, 40)).toBeUndefined();
    expect(overlayPillFitWidth(true, 'warning', 256, true, 40)).toBeUndefined();
  });
});
