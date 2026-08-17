import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  AppearanceController,
  OpenVsxThemeExtension,
  ThemeLibraryEntryV1,
} from '../../lib/appearance';
import { CommunityThemeDialog } from './CommunityThemeDialog';

const mocks = vi.hoisted(() => ({
  search: vi.fn(),
  importExtension: vi.fn(),
  openUrl: vi.fn(),
  controller: null as AppearanceController | null,
}));

vi.mock('../../lib/appearance/openVsxThemes', async (importOriginal) => ({
  ...await importOriginal<typeof import('../../lib/appearance/openVsxThemes')>(),
  searchOpenVsxThemes: mocks.search,
  importOpenVsxThemeExtension: mocks.importExtension,
}));
vi.mock('../../lib/hooks/useAppearance', () => ({
  useAppearance: () => {
    if (!mocks.controller) throw new Error('Missing appearance controller');
    return mocks.controller;
  },
}));
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: mocks.openUrl }));

const extension: OpenVsxThemeExtension = {
  id: 'sample.aurora',
  collectionId: 'open-vsx:sample.aurora',
  name: 'Aurora',
  publisher: 'sample',
  description: 'A community theme',
  downloadCount: 1200,
  sourceUrl: 'https://example.com/sample/aurora',
  manifestUrl: 'https://open-vsx.org/manifest',
  sha256Url: 'https://open-vsx.org/checksum',
  vsixUrl: 'https://open-vsx.org/theme.vsix',
  version: '1.0.0',
  license: 'MIT',
};

const tokens = {
  background: '#f7fafc', surface: '#f7fafc', 'surface-container-low': '#eff4f8',
  'surface-container': '#e9eff3', 'surface-container-high': '#e2e9ee',
  'surface-container-lowest': '#ffffff', 'surface-container-highest': '#dbe4e9',
  primary: '#036785', 'primary-dim': '#005a75', 'on-primary': '#f3faff',
  'on-surface': '#2b3438', 'on-surface-variant': '#586065',
  'outline-variant': '#abb3b9', error: '#a83836', success: '#247a52', warning: '#8b5d00',
} as const;

function controller(): AppearanceController {
  return {
    document: {
      version: 1,
      revision: 0,
      mode: 'system',
      theme: { version: 1, presetId: 'sonic' },
      cache: { version: 1, light: tokens, dark: tokens },
    },
    resolvedAppearance: 'light',
    adjustments: [],
    busy: false,
    error: null,
    setMode: vi.fn(),
    updateTheme: vi.fn(),
    reset: vi.fn(),
    previewImport: vi.fn(),
    importFromPath: vi.fn(),
    commitImport: vi.fn(),
    exportText: vi.fn(),
    exportToPath: vi.fn(),
    library: {
      document: { version: 1, revision: 0, themes: [] },
      error: null,
      saveCurrent: vi.fn(),
      savePreview: vi.fn(),
      install: vi.fn(async () => {}),
      replaceCollection: vi.fn(async () => {}),
      remove: vi.fn(),
      previewSelection: vi.fn(),
      exportEntryToPath: vi.fn(),
      clearError: vi.fn(),
    },
    clearError: vi.fn(),
  };
}

function button(container: HTMLElement, label: string): HTMLButtonElement {
  const match = Array.from(container.querySelectorAll('button'))
    .find((candidate) => candidate.textContent?.trim() === label);
  if (!match) throw new Error(`Missing button: ${label}`);
  return match;
}

describe('CommunityThemeDialog', () => {
  let container: HTMLDivElement;
  let root: Root;
  let onClose: () => void;

  beforeEach(async () => {
    mocks.controller = controller();
    mocks.search.mockReset();
    mocks.importExtension.mockReset();
    mocks.openUrl.mockReset();
    onClose = vi.fn();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(<CommunityThemeDialog open onClose={onClose} />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('discloses network behavior, searches on demand, and adds only after confirmation', async () => {
    expect(container.querySelector('[role="dialog"]')?.getAttribute('aria-modal')).toBe('true');
    expect(container.textContent).toContain('Search sends your query');
    expect(container.textContent).toContain('never runs extension code');
    mocks.search.mockResolvedValueOnce([extension]);
    await act(async () => {
      button(container, 'Dracula').click();
      await new Promise((resolve) => setTimeout(resolve, 100));
    });
    expect(mocks.search).toHaveBeenCalledWith('Dracula', expect.objectContaining({
      sortBy: 'downloadCount',
      signal: expect.any(AbortSignal),
    }));
    expect(container.textContent).toContain('Aurora');
    expect(mocks.importExtension).not.toHaveBeenCalled();

    const entry = {
      version: 1 as const,
      id: 'aurora',
      label: 'Aurora',
      modes: ['light', 'dark'] as const,
      theme: { version: 1 as const, presetId: 'custom' as const, accent: '#16789a' },
      source: { kind: 'local' as const },
    };
    mocks.importExtension.mockResolvedValueOnce([entry]);
    await act(async () => {
      button(container, 'Add').click();
      await new Promise((resolve) => setTimeout(resolve, 100));
    });
    expect(mocks.controller!.library.install).toHaveBeenCalledWith([entry]);
    expect(container.textContent).toContain('added to your theme library');

    await act(async () => button(container, 'Source ↗').click());
    expect(mocks.openUrl).toHaveBeenCalledWith(extension.sourceUrl);
  });

  it('requires explicit confirmation before replacing an installed collection', async () => {
    const installed: ThemeLibraryEntryV1 = {
      version: 1 as const,
      id: 'aurora-old',
      label: 'Aurora',
      modes: ['dark'],
      theme: { version: 1 as const, presetId: 'custom' as const, background: '#101010' },
      source: { kind: 'local' as const },
      collection: { id: extension.collectionId, label: extension.name },
    };
    mocks.controller!.library.document.themes = [installed];
    mocks.search.mockResolvedValueOnce([extension]);
    await act(async () => {
      button(container, 'Nord').click();
      await new Promise((resolve) => setTimeout(resolve, 100));
    });
    await act(async () => button(container, 'Update').click());
    expect(container.textContent).toContain('Update Aurora?');
    expect(mocks.importExtension).not.toHaveBeenCalled();

    const replacement = { ...installed, id: 'aurora-new' };
    mocks.importExtension.mockResolvedValueOnce([replacement]);
    await act(async () => {
      button(container, 'Update collection').click();
      await new Promise((resolve) => setTimeout(resolve, 100));
    });
    expect(mocks.controller!.library.replaceCollection).toHaveBeenCalledWith(
      extension.collectionId,
      [replacement],
      [installed],
    );
  });

  it('closes on Escape and restores focus when unmounted', async () => {
    const previous = document.createElement('button');
    document.body.appendChild(previous);
    previous.focus();
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(onClose).toHaveBeenCalledOnce();
    previous.remove();
  });
});
