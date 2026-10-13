import { describe, expect, it } from 'vitest';
import {
  smartDestinationLabel,
  smartDestinationSelectionFromPayload,
} from './smartDestination';

describe('smart destination event parsing', () => {
  it('accepts a bounded app name and uses it in the destination label', () => {
    const selection = smartDestinationSelectionFromPayload({
      recordingId: 7,
      status: 'recent',
      appName: 'T3 Code',
    });

    expect(selection).toEqual({
      recordingId: 7,
      status: 'recent',
      appName: 'T3 Code',
    });
    expect(smartDestinationLabel('recent', selection?.appName)).toBe('To T3 Code');
  });

  it.each([
    ['', 'empty'],
    ['a'.repeat(81), 'too long'],
    ['T3\nCode', 'control character'],
    [42, 'not a string'],
  ])('drops a malformed app name (%s: %s) but keeps the safe status', (appName, _reason) => {
    expect(smartDestinationSelectionFromPayload({
      recordingId: 8,
      status: 'current',
      appName,
    })).toEqual({ recordingId: 8, status: 'current' });
  });

  it('ignores an app name for clipboard-only delivery', () => {
    const selection = smartDestinationSelectionFromPayload({
      recordingId: 9,
      status: 'unavailable',
      appName: 'T3 Code',
    });

    expect(selection).toEqual({ recordingId: 9, status: 'unavailable' });
    expect(smartDestinationLabel('unavailable', 'T3 Code')).toBe('Clipboard only');
  });
});
