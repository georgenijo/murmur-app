import { describe, expect, it } from 'vitest';

import {
  pairVsCodeThemes,
  parseVsCodeThemeFile,
  resolveTheme,
  resolveVsCodeThemeLabelCollisions,
} from '.';

describe('VS Code theme conversion', () => {
  it('maps allowlisted workbench colors and repairs unreadable text', () => {
    const converted = parseVsCodeThemeFile({
      name: 'sample-dark',
      type: 'dark',
      colors: {
        'editor.background': '#101214',
        'editor.foreground': '#121416',
        'sideBar.background': '#171a1d',
        'panel.background': '#1d2125',
        'button.background': '#3f9fc8',
        'button.foreground': '#ffffff',
        'editorError.foreground': '#ff817b',
        'terminal.ansiBlack': '#ffffff',
      },
    });
    const tokens = resolveTheme(converted.theme, 'dark').tokens;
    expect(converted).toMatchObject({ label: 'Sample Dark', appearance: 'dark' });
    expect(tokens.background).toBe('#101214');
    expect(tokens['surface-container-low']).toBe('#171a1d');
    expect(tokens['surface-container']).toBe('#1d2125');
    expect(tokens['on-surface']).not.toBe('#121416');
    expect(converted.theme.dark).not.toHaveProperty('terminal.ansiBlack');
  });

  it('flattens alpha and accepts display-p3 colors before gamut repair', () => {
    const converted = parseVsCodeThemeFile({
      displayName: 'P3 Theme',
      colors: {
        'editor.background': '#000000',
        'editor.foreground': '#ffffffcc',
        'focusBorder': 'color(display-p3 0.2 0.7 1 / 80%)',
      },
    });
    const tokens = resolveTheme(converted.theme, 'dark').tokens;
    expect(tokens.primary).toMatch(/^#[0-9a-f]{6}$/);
    expect(tokens['on-surface']).toBe('#cccccc');
  });

  it('requires a usable editor background', () => {
    expect(() => parseVsCodeThemeFile({ colors: { foreground: '#ffffff' } }))
      .toThrow(/editor\.background/);
  });

  it('pairs exactly one light and dark variant with the same family name', () => {
    const light = parseVsCodeThemeFile({
      displayName: 'Aurora Light',
      type: 'light',
      colors: { 'editor.background': '#fafafa', 'editor.foreground': '#222222' },
    }, { sourcePath: 'extension/themes/light.json' });
    const dark = parseVsCodeThemeFile({
      displayName: 'Aurora Dark',
      type: 'dark',
      colors: { 'editor.background': '#111111', 'editor.foreground': '#eeeeee' },
    }, { sourcePath: 'extension/themes/dark.json' });
    const paired = pairVsCodeThemes([light, dark]);
    expect(paired).toHaveLength(1);
    expect(paired[0]).toMatchObject({ label: 'Aurora', modes: ['light', 'dark'] });
    expect(paired[0]!.theme.light?.background).toBe('#fafafa');
    expect(paired[0]!.theme.dark?.background).toBe('#111111');
  });

  it('uses file names to disambiguate colliding labels', () => {
    const base = parseVsCodeThemeFile({
      displayName: 'Same',
      colors: { 'editor.background': '#ffffff' },
    });
    const resolved = resolveVsCodeThemeLabelCollisions([
      { ...base, sourceName: 'first-theme.json' },
      { ...base, sourceName: 'second-theme.json' },
    ]);
    expect(resolved.map((theme) => theme.label)).toEqual(['First Theme', 'Second Theme']);
  });
});
