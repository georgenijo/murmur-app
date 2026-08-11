import { describe, expect, it } from 'vitest';
import { LEVEL_COLORS, STREAMS, STREAM_COLORS } from './events';

describe('event semantic colors', () => {
  it('keeps every stream visually distinct without adding theme tokens', () => {
    const signatures = STREAMS.map((stream) => {
      const colors = STREAM_COLORS[stream];
      return `${colors.bg}|${colors.text}|${colors.dot}`;
    });

    expect(new Set(signatures).size).toBe(STREAMS.length);
    expect(STREAM_COLORS).toEqual({
      pipeline: {
        bg: 'bg-surface-container-high',
        text: 'text-on-surface',
        dot: 'bg-on-surface',
      },
      audio: {
        bg: 'bg-primary/10',
        text: 'text-on-surface',
        dot: 'bg-on-surface',
      },
      keyboard: {
        bg: 'bg-surface-container-lowest',
        text: 'text-primary',
        dot: 'bg-primary',
      },
      transform: {
        bg: 'bg-warning/10',
        text: 'text-warning',
        dot: 'bg-warning',
      },
      query: {
        bg: 'bg-primary/10',
        text: 'text-primary',
        dot: 'bg-primary',
      },
      system: {
        bg: 'bg-success/10',
        text: 'text-success',
        dot: 'bg-success',
      },
    });
  });

  it('uses clean class tokens and maps warnings to the warning foreground', () => {
    const classes = [
      ...Object.values(STREAM_COLORS).flatMap(({ bg, text, dot }) => [bg, text, dot]),
      ...Object.values(LEVEL_COLORS),
    ];

    expect(classes.every((className) => className === className.trim())).toBe(true);
    expect(LEVEL_COLORS.warn).toBe('text-warning');
  });
});
