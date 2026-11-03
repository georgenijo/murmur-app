import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { BEZEL_PALETTE, inkContrastOver, worstInkContrast } from './bezelPalette';

const read = (path: string) => readFileSync(new URL(path, import.meta.url), 'utf8');

describe('Bezel palette', () => {
  it.each(['dark', 'light'] as const)('%s material floor keeps 11pt labels at 7:1 over white and black', tone => {
    const palette = BEZEL_PALETTE[tone];
    expect(palette.materialAlpha).toBeGreaterThanOrEqual(0.74);
    expect(palette.materialAlpha).toBeLessThanOrEqual(0.76);
    expect(worstInkContrast(palette)).toBeGreaterThanOrEqual(7);
  });

  it('keeps light-mode waveform bars at 3:1 over a white page', () => {
    expect(inkContrastOver(BEZEL_PALETTE.light, [255, 255, 255])).toBeGreaterThanOrEqual(3);
  });

  it.each(['dark', 'light'] as const)('%s tentative Live Words stay readable', tone => {
    const palette = BEZEL_PALETTE[tone];
    expect(worstInkContrast(palette, palette.inkAlpha * palette.tentativeAlpha)).toBeGreaterThanOrEqual(4.5);
  });

  it('matches the native tint and the CSS fallback tone', () => {
    const rust = read('../../src-tauri/src/commands/overlay_material.rs');
    const css = read('../styles/overlay.css');
    for (const tone of ['dark', 'light'] as const) {
      const { material: [r, g, b], materialAlpha, ink, inkAlpha } = BEZEL_PALETTE[tone];
      expect(rust).toContain(`fill: [${r}.0 / 255.0, ${g}.0 / 255.0, ${b}.0 / 255.0, ${materialAlpha}]`);
      expect(css).toContain(`--bezel-solid: rgb(${r}, ${g}, ${b});`);
      const fg = inkAlpha === 1 ? `rgb(${ink.join(', ')})` : `rgba(${ink.join(', ')}, ${inkAlpha})`;
      expect(css).toContain(`--bezel-ink: ${fg};`);
    }
  });
});
