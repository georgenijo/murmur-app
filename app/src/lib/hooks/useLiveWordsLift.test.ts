import { describe, expect, it } from 'vitest';
import geometryFixture from '../../components/overlay/overlay-geometry.fixture.json';
import type { OverlayGeometry } from '../overlayGeometry';
import { liveWordsLift } from './useLiveWordsLift';

describe('liveWordsLift', () => {
  const pill = geometryFixture.pillExpanded as OverlayGeometry;

  it('moves Live Words by the dropdown height only while the island is expanded', () => {
    expect(liveWordsLift(pill, true)).toBe(pill.dropdownH);
    expect(liveWordsLift(pill, false)).toBe(0);
    expect(liveWordsLift(null, true)).toBe(0);
  });
});
