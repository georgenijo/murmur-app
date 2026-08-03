import { beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import {
  APPEARANCE_STORAGE_KEY,
  MURMUR_TOKEN_NAMES,
  SONIC_DARK,
  SONIC_LIGHT,
  createAppearanceDocument,
  resolveTheme,
} from '.';

const boot = readFileSync('./public/appearance-boot.js', 'utf8');
const mainHtml = readFileSync('./index.html', 'utf8');

function runBoot(systemDark: boolean): void {
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    value: vi.fn(() => ({
      matches: systemDark,
      media: '(prefers-color-scheme: dark)',
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  });
  // Execute the committed classic-script artifact, not a test-only copy.
  new Function('window', 'document', boot)(window, document);
}

function appliedTokens(): Record<string, string> {
  return Object.fromEntries(MURMUR_TOKEN_NAMES.map((token) => [
    token,
    document.documentElement.style.getPropertyValue(`--murmur-${token}`),
  ]));
}

describe('appearance bootstrap', () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.removeAttribute('data-appearance');
    document.documentElement.removeAttribute('style');
  });

  it('is parser-blocking before the main themed module entry', () => {
    const bootIndex = mainHtml.indexOf('<script src="/appearance-boot.js"></script>');
    const moduleIndex = mainHtml.indexOf('<script type="module"');
    expect(bootIndex).toBeGreaterThan(-1);
    expect(moduleIndex).toBeGreaterThan(bootIndex);
  });

  it('uses exact matching Sonic fallback for empty and corrupt storage', () => {
    runBoot(true);
    expect(document.documentElement.dataset.appearance).toBe('dark');
    expect(appliedTokens()).toEqual(SONIC_DARK);

    document.documentElement.removeAttribute('style');
    localStorage.setItem(APPEARANCE_STORAGE_KEY, '{bad json');
    runBoot(false);
    expect(document.documentElement.dataset.appearance).toBe('light');
    expect(appliedTokens()).toEqual(SONIC_LIGHT);
  });

  it('honors forced mode independently of the OS', () => {
    localStorage.setItem(
      APPEARANCE_STORAGE_KEY,
      JSON.stringify(createAppearanceDocument('light')),
    );
    runBoot(true);
    expect(document.documentElement.dataset.appearance).toBe('light');
    expect(document.documentElement.style.colorScheme).toBe('light');

    document.documentElement.removeAttribute('style');
    localStorage.setItem(
      APPEARANCE_STORAGE_KEY,
      JSON.stringify(createAppearanceDocument('dark')),
    );
    runBoot(false);
    expect(document.documentElement.dataset.appearance).toBe('dark');
  });

  it('applies a valid custom cache with runtime parity', () => {
    const documentValue = createAppearanceDocument('dark', {
      version: 1,
      presetId: 'custom',
      accent: '#ff5500',
      background: '#152535',
      foreground: '#eeeeee',
      contrast: 75,
    });
    localStorage.setItem(APPEARANCE_STORAGE_KEY, JSON.stringify(documentValue));
    runBoot(false);
    expect(appliedTokens()).toEqual(resolveTheme(documentValue.theme, 'dark').tokens);
  });

  it.each([
    ['unknown theme version', (value: ReturnType<typeof createAppearanceDocument>) => {
      value.theme.version = 2 as 1;
    }],
    ['unknown preset', (value: ReturnType<typeof createAppearanceDocument>) => {
      value.theme.presetId = 'downloaded-theme' as 'custom';
    }],
  ])('rejects valid-looking cache with %s without a custom-color flash', (_case, mutate) => {
    const documentValue = createAppearanceDocument('dark', {
      version: 1,
      presetId: 'custom',
      accent: '#ff0000',
    });
    mutate(documentValue);
    localStorage.setItem(APPEARANCE_STORAGE_KEY, JSON.stringify(documentValue));
    runBoot(false);
    expect(document.documentElement.dataset.appearance).toBe('dark');
    expect(appliedTokens()).toEqual(SONIC_DARK);
  });

  it.each([
    ['missing key', (documentValue: ReturnType<typeof createAppearanceDocument>) => {
      const dark = { ...documentValue.cache.dark } as Partial<typeof documentValue.cache.dark>;
      delete dark.warning;
      documentValue.cache.dark = dark as typeof documentValue.cache.dark;
    }],
    ['invalid value', (documentValue: ReturnType<typeof createAppearanceDocument>) => {
      documentValue.cache.dark.primary = 'red' as `#${string}`;
    }],
    ['extra key', (documentValue: ReturnType<typeof createAppearanceDocument>) => {
      Object.assign(documentValue.cache.dark, { arbitrary: '#ffffff' });
    }],
    ['unknown cache version', (documentValue: ReturnType<typeof createAppearanceDocument>) => {
      documentValue.cache.version = 2 as 1;
    }],
  ])('rejects a cache with %s', (_case, mutate) => {
    const documentValue = createAppearanceDocument('dark', {
      version: 1,
      presetId: 'custom',
      accent: '#ff0000',
    });
    mutate(documentValue);
    localStorage.setItem(APPEARANCE_STORAGE_KEY, JSON.stringify(documentValue));
    runBoot(false);
    expect(appliedTokens()).toEqual(SONIC_DARK);
  });

  it('rejects oversized storage before parsing', () => {
    localStorage.setItem(APPEARANCE_STORAGE_KEY, ' '.repeat(64 * 1024 + 1));
    runBoot(false);
    expect(appliedTokens()).toEqual(SONIC_LIGHT);
  });
});
