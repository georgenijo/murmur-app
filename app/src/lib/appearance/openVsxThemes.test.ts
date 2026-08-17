import { sha256 } from '@noble/hashes/sha2.js';
import JSZip from 'jszip';
import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  importOpenVsxThemeExtension,
  searchOpenVsxThemes,
  type OpenVsxThemeExtension,
} from '.';

const extension: OpenVsxThemeExtension = {
  id: 'sample.aurora',
  collectionId: 'open-vsx:sample.aurora',
  name: 'Aurora',
  publisher: 'sample',
  description: 'A paired theme',
  downloadCount: 42,
  sourceUrl: 'https://example.com/sample/aurora',
  manifestUrl: 'https://open-vsx.org/api/sample/aurora/latest/file/package.json',
  sha256Url: 'https://open-vsx.org/api/sample/aurora/latest/file/sample.aurora.sha256',
  vsixUrl: 'https://open-vsx.org/api/sample/aurora/latest/file/sample.aurora.vsix',
  version: '1.2.3',
  license: 'MIT',
};

const contributions = [
  { label: 'Aurora Light', uiTheme: 'vs', path: './themes/light.json' },
  { label: 'Aurora Dark', uiTheme: 'vs-dark', path: './themes/dark.json' },
];

function checksum(bytes: Uint8Array): string {
  return [...sha256(bytes)].map((byte) => byte.toString(16).padStart(2, '0')).join('');
}

function packagedManifest(overrides: Record<string, unknown> = {}) {
  return {
    publisher: extension.publisher,
    name: 'aurora',
    version: extension.version,
    license: extension.license,
    contributes: { themes: contributions },
    ...overrides,
  };
}

async function packageBytes(
  manifest = packagedManifest(),
  files: Record<string, string> = {},
): Promise<Uint8Array> {
  const zip = new JSZip();
  zip.file('extension/package.json', JSON.stringify(manifest));
  zip.file('extension/themes/base.json', `{
    // JSONC and include inheritance are supported.
    "colors": {
      "editor.foreground": "#e9eef2",
      "sideBar.background": "#191d21",
    },
  }`);
  zip.file('extension/themes/light.json', JSON.stringify({
    colors: {
      'editor.background': '#f7f9fb',
      'editor.foreground': '#20262a',
      'button.background': '#086685',
    },
  }));
  zip.file('extension/themes/dark.json', JSON.stringify({
    include: './base.json',
    colors: {
      'editor.background': '#101316',
      'button.background': '#88d4f5',
    },
  }));
  for (const [path, contents] of Object.entries(files)) zip.file(path, contents);
  return zip.generateAsync({ type: 'uint8array', compression: 'DEFLATE' });
}

function response(body: BodyInit | null, init: ResponseInit = {}): Response {
  return new Response(body, { status: 200, ...init });
}

function mockImportFetch(bytes: Uint8Array, expectedChecksum = checksum(bytes)) {
  return vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url === extension.manifestUrl) {
      return response(JSON.stringify({
        license: extension.license,
        contributes: { themes: contributions },
      }));
    }
    if (url === extension.vsixUrl) return response(bytes);
    if (url === extension.sha256Url) return response(expectedChecksum);
    throw new Error(`Unexpected fetch: ${url}`);
  });
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('Open VSX discovery', () => {
  it('searches only theme extensions and verifies supported metadata before listing', async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input));
      if (url.pathname === '/api/-/search') {
        expect(url.searchParams.get('category')).toBe('Themes');
        expect(url.searchParams.get('sortBy')).toBe('rating');
        expect(init?.credentials).toBe('omit');
        return response(JSON.stringify({ extensions: [{ namespace: 'sample', name: 'aurora' }] }));
      }
      if (url.pathname === '/api/sample/aurora') {
        return response(JSON.stringify({
          namespace: extension.publisher,
          name: 'aurora',
          displayName: extension.name,
          version: extension.version,
          license: extension.license,
          description: extension.description,
          downloadCount: extension.downloadCount,
          repository: extension.sourceUrl,
          files: {
            manifest: extension.manifestUrl,
            sha256: extension.sha256Url,
            download: extension.vsixUrl,
          },
        }));
      }
      if (url.toString() === extension.manifestUrl) {
        return response(JSON.stringify({ license: 'MIT', contributes: { themes: contributions } }));
      }
      if (url.toString() === extension.vsixUrl && init?.method === 'HEAD') {
        return response(null, { headers: { 'content-length': '4096' } });
      }
      throw new Error(`Unexpected fetch: ${url}`);
    });
    vi.stubGlobal('fetch', fetchMock);

    await expect(searchOpenVsxThemes('aurora', { sortBy: 'rating' })).resolves.toEqual([extension]);
  });

  it('does not list extensions with unsupported licenses', async () => {
    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
      const url = new URL(String(input));
      if (url.pathname === '/api/-/search') {
        return response(JSON.stringify({ extensions: [{ namespace: 'sample', name: 'aurora' }] }));
      }
      return response(JSON.stringify({
        namespace: 'sample',
        name: 'aurora',
        displayName: 'Aurora',
        version: '1.2.3',
        license: 'Proprietary',
        files: {
          manifest: extension.manifestUrl,
          sha256: extension.sha256Url,
          download: extension.vsixUrl,
        },
      }));
    }));
    await expect(searchOpenVsxThemes('aurora')).resolves.toEqual([]);
  });
});

describe('Open VSX package import', () => {
  it('verifies the package and converts JSONC includes into a paired Murmur theme', async () => {
    const bytes = await packageBytes();
    vi.stubGlobal('fetch', mockImportFetch(bytes));
    const entries = await importOpenVsxThemeExtension(extension);
    expect(entries).toHaveLength(1);
    expect(entries[0]).toMatchObject({
      label: 'Aurora',
      modes: ['light', 'dark'],
      source: {
        kind: 'open-vsx',
        extensionId: extension.id,
        version: extension.version,
        license: 'MIT',
      },
      collection: { id: extension.collectionId, label: extension.name },
    });
    expect(entries[0]!.theme.light?.background).toBe('#f7f9fb');
    expect(entries[0]!.theme.dark?.background).toBe('#101316');
    expect(entries[0]!.theme.dark?.['surface-container-low']).toBe('#191d21');
  });

  it('rejects untrusted download hosts before making a request', async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);
    await expect(importOpenVsxThemeExtension({
      ...extension,
      vsixUrl: 'https://evil.example/theme.vsix',
    })).rejects.toThrow(/untrusted URL/);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('rejects checksum mismatches without opening the archive', async () => {
    const bytes = await packageBytes();
    vi.stubGlobal('fetch', mockImportFetch(bytes, '0'.repeat(64)));
    await expect(importOpenVsxThemeExtension(extension)).rejects.toThrow(/integrity check/);
  });

  it('rejects package identity and license mismatches', async () => {
    for (const manifest of [
      packagedManifest({ version: '9.9.9' }),
      packagedManifest({ license: 'Apache-2.0' }),
    ]) {
      const bytes = await packageBytes(manifest);
      vi.stubGlobal('fetch', mockImportFetch(bytes));
      await expect(importOpenVsxThemeExtension(extension)).rejects.toThrow(/does not match/);
    }
  });

  it('rejects theme paths that escape the extension package', async () => {
    const escaping = [{ label: 'Escape', uiTheme: 'vs-dark', path: '../../escape.json' }];
    const bytes = await packageBytes(packagedManifest({ contributes: { themes: escaping } }));
    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url === extension.manifestUrl) {
        return response(JSON.stringify({ license: 'MIT', contributes: { themes: escaping } }));
      }
      if (url === extension.vsixUrl) return response(bytes);
      if (url === extension.sha256Url) return response(checksum(bytes));
      throw new Error(`Unexpected fetch: ${url}`);
    }));
    await expect(importOpenVsxThemeExtension(extension)).rejects.toThrow(/escapes the extension package/);
  });

  it('rejects include cycles', async () => {
    const cycleContributions = [
      { label: 'Cycle', uiTheme: 'vs-dark', path: './themes/cycle-a.json' },
    ];
    const bytes = await packageBytes(
      packagedManifest({ contributes: { themes: cycleContributions } }),
      {
        'extension/themes/cycle-a.json': JSON.stringify({
          include: './cycle-b.json',
          colors: { 'editor.background': '#101010' },
        }),
        'extension/themes/cycle-b.json': JSON.stringify({
          include: './cycle-a.json',
          colors: { 'editor.foreground': '#ffffff' },
        }),
      },
    );
    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url === extension.manifestUrl) {
        return response(JSON.stringify({ license: 'MIT', contributes: { themes: cycleContributions } }));
      }
      if (url === extension.vsixUrl) return response(bytes);
      if (url === extension.sha256Url) return response(checksum(bytes));
      throw new Error(`Unexpected fetch: ${url}`);
    }));
    await expect(importOpenVsxThemeExtension(extension)).rejects.toThrow(/include.*cycle/i);
  });

  it('rejects declared package sizes beyond the bounded download limit', async () => {
    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url === extension.manifestUrl) {
        return response(JSON.stringify({ license: 'MIT', contributes: { themes: contributions } }));
      }
      if (url === extension.vsixUrl) {
        return response(new Uint8Array(), { headers: { 'content-length': String(21 * 1024 * 1024) } });
      }
      throw new Error(`Unexpected fetch: ${url}`);
    }));
    await expect(importOpenVsxThemeExtension(extension)).rejects.toThrow(/too large to import safely/);
  });
});
