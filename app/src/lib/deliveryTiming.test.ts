import { describe, expect, it } from 'vitest';
import fixture from './fixtures/delivery-timing-v1.json';
import { isDeliveryTimingV1 } from './deliveryTiming';
import { isPerformanceRunV1 } from './performance';
import { makeRun } from '../components/log-viewer/testFixtures';

describe('delivery timing companion', () => {
  it('accepts the native schema and preserves historical absence', () => {
    expect(isDeliveryTimingV1(fixture)).toBe(true);
    const run = makeRun();
    expect(isPerformanceRunV1(run)).toBe(true);
    expect(isPerformanceRunV1({ ...run, deliveryTiming: fixture })).toBe(true);
  });
  it('rejects unknown versions, fields, enums and invalid measurements', () => {
    for (const change of [
      { schemaVersion: 2 }, { windowTitle: 'forbidden' },
      { outcome: { status: 'measured', value: 'arbitrary' } },
      { injectWorkUs: { status: 'measured', value: -1 } },
      { nativePasteRetryCount: 2 },
      { nativePost: { ...fixture.nativePost, transcript: 'forbidden' } },
      { targetVerification: [...fixture.targetVerification].reverse() },
    ]) expect(isDeliveryTimingV1({ ...fixture, ...change })).toBe(false);
    expect(isDeliveryTimingV1({ ...fixture, injectWorkUs: { status: 'measured', value: 0 } })).toBe(true);
  });
});
