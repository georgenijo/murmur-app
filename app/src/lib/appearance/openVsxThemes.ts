import { sha256 } from '@noble/hashes/sha2.js';
import JSZip from 'jszip';
import { parse, type ParseError } from 'jsonc-parser';

import { parseThemeLibraryEntry } from './themeLibrary';
import type { ThemeLibraryEntryV1 } from './types';
import {
  isVsCodeThemeFile,
  pairVsCodeThemes,
  parseVsCodeThemeFile,
} from './vscodeThemeImport';

const OPEN_VSX_ORIGIN = 'https://open-vsx.org';
const OPEN_VSX_SEARCH_URL = `${OPEN_VSX_ORIGIN}/api/-/search`;
const MAX_VSIX_BYTES = 20 * 1024 * 1024;
const MAX_SEARCH_BYTES = 512 * 1024;
const MAX_DETAIL_BYTES = 256 * 1024;
const MAX_MANIFEST_BYTES = 256 * 1024;
const MAX_THEME_BYTES = 256 * 1024;
const SEARCH_REQUEST_TIMEOUT_MS = 10_000;
const MAX_ZIP_ENTRIES = 2_000;
const MAX_UNCOMPRESSED_BYTES = 50 * 1024 * 1024;
const MAX_COMPRESSION_RATIO = 200;
const MAX_THEMES_PER_EXTENSION = 40;
const MAX_INCLUDE_DEPTH = 8;
const MAX_PACKAGE_PATH_LENGTH = 1_024;
const MAX_COLOR_VALUE_LENGTH = 128;
const MAX_RESOLVED_THEME_FILES = MAX_THEMES_PER_EXTENSION * MAX_INCLUDE_DEPTH;

export const SUPPORTED_OPEN_VSX_LICENSES = new Set([
  '0BSD',
  'Apache-2.0',
  'BSD-2-Clause',
  'BSD-3-Clause',
  'CC0-1.0',
  'ISC',
  'MIT',
  'MPL-2.0',
  'Unlicense',
]);

const USED_WORKBENCH_COLORS = new Set([
  'activityBar.background',
  'activityBarBadge.background',
  'badge.background',
  'button.background',
  'button.foreground',
  'contrastBorder',
  'descriptionForeground',
  'disabledForeground',
  'dropdown.background',
  'editor.background',
  'editor.foreground',
  'editorError.foreground',
  'editorPane.background',
  'editorWarning.foreground',
  'editorWidget.background',
  'errorForeground',
  'focusBorder',
  'foreground',
  'list.activeSelectionBackground',
  'menu.background',
  'panel.background',
  'panel.border',
  'progressBar.background',
  'quickInput.background',
  'sideBar.background',
  'sideBar.border',
  'sideBar.foreground',
  'textLink.foreground',
]);

export type OpenVsxThemeSort = 'downloadCount' | 'rating' | 'timestamp' | 'relevance';

export interface OpenVsxThemeExtension {
  id: string;
  collectionId: string;
  name: string;
  publisher: string;
  description: string;
  downloadCount: number;
  sourceUrl: string | null;
  manifestUrl: string;
  sha256Url: string;
  vsixUrl: string;
  version: string;
  license: string;
}

export interface OpenVsxThemeSearchOptions {
  signal?: AbortSignal;
  sortBy?: OpenVsxThemeSort;
}

type ThemeContribution = { label?: unknown; uiTheme?: unknown; path?: unknown };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function shortHash(value: string): string {
  return [...sha256(new TextEncoder().encode(value))]
    .slice(0, 8)
    .map((byte) => byte.toString(16).padStart(2, '0'))
    .join('');
}

function openVsxThemeId(extensionId: string, source: string): string {
  return `ovx-${shortHash(`${extensionId}:${source}`)}`;
}

function openVsxCollectionId(extensionId: string): string {
  const normalized = `open-vsx:${extensionId.toLowerCase()}`;
  return /^[a-z0-9][a-z0-9.:-]{0,127}$/.test(normalized)
    ? normalized
    : `open-vsx:${shortHash(extensionId)}`;
}

function trustedOpenVsxUrl(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  try {
    const url = new URL(value);
    return url.protocol === 'https:' && url.origin === OPEN_VSX_ORIGIN && !url.username && !url.password
      ? url.toString()
      : null;
  } catch {
    return null;
  }
}

function publicSourceUrl(value: unknown): string | null {
  const rawValue = typeof value === 'string'
    ? value
    : isRecord(value) && typeof value.url === 'string'
      ? value.url
      : null;
  if (!rawValue) return null;
  try {
    const url = new URL(rawValue);
    return url.protocol === 'https:' && !url.username && !url.password ? url.toString() : null;
  } catch {
    return null;
  }
}

function themeContributions(manifest: Record<string, unknown>): ThemeContribution[] {
  const contributes = isRecord(manifest.contributes) ? manifest.contributes : null;
  return Array.isArray(contributes?.themes)
    ? (contributes.themes.filter(isRecord) as ThemeContribution[])
    : [];
}

function manifestLicenseMatches(manifest: Record<string, unknown>, license: string): boolean {
  return typeof manifest.license === 'string'
    && manifest.license.trim().toLowerCase() === license.toLowerCase();
}

function extensionFromDetail(value: unknown): OpenVsxThemeExtension | null {
  if (!isRecord(value) || !isRecord(value.files)) {
    throw new Error('Open VSX returned malformed theme details.');
  }
  const publisher = typeof value.namespace === 'string' ? value.namespace.trim() : '';
  const extensionName = typeof value.name === 'string' ? value.name.trim() : '';
  const name = (typeof value.displayName === 'string' ? value.displayName.trim() : '') || extensionName;
  const version = typeof value.version === 'string' ? value.version.trim() : '';
  const license = typeof value.license === 'string' ? value.license.trim() : '';
  const manifestUrl = trustedOpenVsxUrl(value.files.manifest);
  const sha256Url = trustedOpenVsxUrl(value.files.sha256);
  const vsixUrl = trustedOpenVsxUrl(value.files.download);
  if (!publisher || !extensionName || !version || !manifestUrl || !sha256Url || !vsixUrl) {
    throw new Error('Open VSX returned malformed theme details.');
  }
  if (!SUPPORTED_OPEN_VSX_LICENSES.has(license)) return null;
  const id = `${publisher}.${extensionName}`;
  return {
    id,
    collectionId: openVsxCollectionId(id),
    name: name.slice(0, 64),
    publisher,
    description: typeof value.description === 'string' ? value.description.slice(0, 280) : '',
    downloadCount: typeof value.downloadCount === 'number' && Number.isFinite(value.downloadCount)
      ? Math.max(0, value.downloadCount)
      : 0,
    sourceUrl: publicSourceUrl(value.repository)
      ?? publicSourceUrl(value.homepage)
      ?? publicSourceUrl(value.url),
    manifestUrl,
    sha256Url,
    vsixUrl,
    version,
    license,
  };
}

async function withTimeout<T>(
  operation: (signal: AbortSignal) => Promise<T>,
  parentSignal?: AbortSignal,
): Promise<T> {
  const controller = new AbortController();
  const abort = () => controller.abort();
  if (parentSignal?.aborted) abort();
  else parentSignal?.addEventListener('abort', abort, { once: true });
  const timeout = setTimeout(abort, SEARCH_REQUEST_TIMEOUT_MS);
  try {
    return await operation(controller.signal);
  } catch (cause) {
    if (controller.signal.aborted && !parentSignal?.aborted) {
      throw new Error(`Open VSX took too long to respond: ${String(cause)}`);
    }
    throw cause;
  } finally {
    clearTimeout(timeout);
    parentSignal?.removeEventListener('abort', abort);
  }
}

export async function searchOpenVsxThemes(
  query: string,
  { signal, sortBy = 'downloadCount' }: OpenVsxThemeSearchOptions = {},
): Promise<OpenVsxThemeExtension[]> {
  const searchText = query.trim();
  if (!searchText) return [];
  const url = new URL(OPEN_VSX_SEARCH_URL);
  url.searchParams.set('query', searchText);
  url.searchParams.set('category', 'Themes');
  url.searchParams.set('sortBy', sortBy);
  url.searchParams.set('sortOrder', 'desc');
  url.searchParams.set('size', '16');
  const value = await withTimeout(async (requestSignal) => {
    const response = await fetch(url, { signal: requestSignal, credentials: 'omit' });
    if (!response.ok) throw new Error('Open VSX search is unavailable right now.');
    const bytes = await readCappedResponse(
      response,
      MAX_SEARCH_BYTES,
      'Open VSX returned an unexpectedly large response.',
    );
    try {
      return JSON.parse(new TextDecoder().decode(bytes)) as unknown;
    } catch {
      throw new Error('Open VSX returned an unreadable response.');
    }
  }, signal);
  if (!isRecord(value) || !Array.isArray(value.extensions)) {
    throw new Error('Open VSX returned an unreadable search response.');
  }
  const identities = value.extensions.flatMap((candidate): Array<[string, string]> => {
    if (!isRecord(candidate)) return [];
    const namespace = typeof candidate.namespace === 'string' ? candidate.namespace : '';
    const name = typeof candidate.name === 'string' ? candidate.name : '';
    return namespace && name ? [[namespace, name]] : [];
  });
  const details = await Promise.allSettled(
    identities.slice(0, 16).map(([namespace, name]) => withTimeout(async (requestSignal) => {
      const detailUrl = `${OPEN_VSX_ORIGIN}/api/${encodeURIComponent(namespace)}/${encodeURIComponent(name)}`;
      const detailResponse = await fetch(detailUrl, { signal: requestSignal, credentials: 'omit' });
      if (!detailResponse.ok) throw new Error('Open VSX theme details are unavailable.');
      const detailBytes = await readCappedResponse(
        detailResponse,
        MAX_DETAIL_BYTES,
        'Open VSX returned unexpectedly large theme details.',
      );
      let extension: OpenVsxThemeExtension | null;
      try {
        extension = extensionFromDetail(JSON.parse(new TextDecoder().decode(detailBytes)));
      } catch {
        throw new Error('Open VSX returned unreadable theme details.');
      }
      if (!extension) return null;
      const [manifestResponse, packageResponse] = await Promise.all([
        fetch(extension.manifestUrl, { signal: requestSignal, credentials: 'omit' }),
        fetch(extension.vsixUrl, { method: 'HEAD', signal: requestSignal, credentials: 'omit' }),
      ]);
      if (!manifestResponse.ok || !packageResponse.ok) return null;
      const packageLength = Number(packageResponse.headers.get('content-length'));
      if (Number.isFinite(packageLength) && packageLength > MAX_VSIX_BYTES) return null;
      const manifestBytes = await readCappedResponse(
        manifestResponse,
        MAX_MANIFEST_BYTES,
        'Open VSX returned an unexpectedly large manifest.',
      );
      const manifest = parseJsoncObject(new TextDecoder().decode(manifestBytes), 'Extension manifest');
      return themeContributions(manifest).length > 0
        && manifestLicenseMatches(manifest, extension.license)
        ? extension
        : null;
    }, signal)),
  );
  if (signal?.aborted) throw new DOMException('The operation was aborted.', 'AbortError');
  const completed = details.filter(
    (result): result is PromiseFulfilledResult<OpenVsxThemeExtension | null> => result.status === 'fulfilled',
  );
  if (identities.length > 0 && completed.length === 0) {
    throw new Error('Open VSX theme details are unavailable right now.');
  }
  return completed.flatMap((result) => result.value ? [result.value] : []).slice(0, 8);
}

function parseJsoncObject(source: string, description: string): Record<string, unknown> {
  const errors: ParseError[] = [];
  const value: unknown = parse(source, errors, { allowTrailingComma: true });
  if (errors.length > 0 || !isRecord(value)) throw new Error(`${description} is not valid JSON.`);
  return value;
}

function sanitizeThemeObject(value: Record<string, unknown>): Record<string, unknown> {
  const colors: Record<string, string> = {};
  if (isRecord(value.colors)) {
    for (const [key, color] of Object.entries(value.colors)) {
      if (
        USED_WORKBENCH_COLORS.has(key)
        && typeof color === 'string'
        && color.length <= MAX_COLOR_VALUE_LENGTH
      ) {
        colors[key] = color;
      }
    }
  }
  return {
    ...(typeof value.include === 'string' ? { include: value.include } : {}),
    colors,
  };
}

function normalizePackagePath(path: string, relativeTo = 'extension/'): string {
  if (
    path.length > MAX_PACKAGE_PATH_LENGTH
    || path.includes('\0')
    || path.startsWith('/')
    || /^[a-zA-Z]:/.test(path)
  ) {
    throw new Error('Theme path is not a safe relative package path.');
  }
  const segments = relativeTo.replace(/\\/g, '/').split('/').slice(0, -1);
  for (const segment of path.replace(/\\/g, '/').split('/')) {
    if (!segment || segment === '.') continue;
    if (segment === '..') {
      if (segments.length <= 1) throw new Error('Theme path escapes the extension package.');
      segments.pop();
    } else {
      segments.push(segment);
    }
  }
  if (segments[0] !== 'extension') segments.unshift('extension');
  return segments.join('/');
}

function contributionType(value: unknown): ResolvedThemeType | null {
  if (value === 'vs') return 'light';
  if (value === 'vs-dark') return 'dark';
  if (value === 'hc-black' || value === 'hc-light') return value;
  return null;
}

type ResolvedThemeType = 'light' | 'dark' | 'hc-black' | 'hc-light';
type ZipEntrySizes = { uncompressedSize?: unknown };
type InspectableZipObject = JSZip.JSZipObject & {
  _data?: ZipEntrySizes;
  unsafeOriginalName?: string;
  internalStream?: (type: 'uint8array') => JSZip.JSZipStreamHelper<Uint8Array>;
};

function inspectZipDirectory(bytes: Uint8Array): Uint8Array {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const minimumOffset = Math.max(0, bytes.byteLength - 65_557);
  let endOffset = bytes.byteLength - 22;
  while (
    endOffset >= minimumOffset
    && (view.getUint32(endOffset, true) !== 0x06054b50
      || endOffset + 22 + view.getUint16(endOffset + 20, true) !== bytes.byteLength)
  ) {
    endOffset -= 1;
  }
  if (endOffset < minimumOffset) throw new Error('That extension package has no ZIP directory.');
  const directorySize = view.getUint32(endOffset + 12, true);
  const directoryOffset = view.getUint32(endOffset + 16, true);
  const directoryEnd = directoryOffset + directorySize;
  if (directoryEnd !== endOffset || directoryEnd > bytes.byteLength) {
    throw new Error('That extension package has an invalid ZIP directory.');
  }
  let entryCount = 0;
  let totalUncompressed = 0;
  let offset = directoryOffset;
  while (offset < directoryEnd) {
    if (offset + 46 > directoryEnd || view.getUint32(offset, true) !== 0x02014b50) {
      throw new Error('That extension package has an invalid ZIP directory.');
    }
    entryCount += 1;
    if (entryCount > MAX_ZIP_ENTRIES) throw new Error('That extension package has too many files.');
    const compressed = view.getUint32(offset + 20, true);
    const uncompressed = view.getUint32(offset + 24, true);
    if (compressed === 0xffffffff || uncompressed === 0xffffffff) {
      throw new Error('That extension package has unsupported ZIP64 metadata.');
    }
    totalUncompressed += uncompressed;
    if (totalUncompressed > MAX_UNCOMPRESSED_BYTES) {
      throw new Error('That extension package expands beyond the safe import limit.');
    }
    if (uncompressed > 0 && (compressed === 0 || uncompressed / compressed > MAX_COMPRESSION_RATIO)) {
      throw new Error('That extension package has an unsafe compression ratio.');
    }
    offset += 46
      + view.getUint16(offset + 28, true)
      + view.getUint16(offset + 30, true)
      + view.getUint16(offset + 32, true);
  }
  if (offset !== directoryEnd) throw new Error('That extension package has an invalid ZIP directory.');
  const commentLength = view.getUint16(endOffset + 20, true);
  if (commentLength === 0) return bytes;
  const withoutComment = bytes.slice(0, endOffset + 22);
  withoutComment[endOffset + 20] = 0;
  withoutComment[endOffset + 21] = 0;
  return withoutComment;
}

function inspectZip(zip: JSZip): void {
  const entries = Object.values(zip.files) as InspectableZipObject[];
  if (entries.length > MAX_ZIP_ENTRIES) throw new Error('That extension package has too many files.');
  for (const entry of entries) {
    if (entry.unsafeOriginalName) normalizePackagePath(entry.unsafeOriginalName);
  }
}

async function readZipText(
  zip: JSZip,
  path: string,
  description: string,
  signal?: AbortSignal,
): Promise<string> {
  signal?.throwIfAborted();
  const file = zip.file(path) as InspectableZipObject | null;
  if (!file) throw new Error(`${description} is missing from the extension package.`);
  if (typeof file._data?.uncompressedSize !== 'number' || !file.internalStream) {
    throw new Error(`${description} has unreadable size metadata.`);
  }
  if (file._data.uncompressedSize > MAX_THEME_BYTES) throw new Error(`${description} is too large.`);
  return new Promise((resolve, reject) => {
    const chunks: Uint8Array[] = [];
    let byteLength = 0;
    let settled = false;
    const stream = file.internalStream!('uint8array');
    const cleanup = () => signal?.removeEventListener('abort', handleAbort);
    const handleAbort = () => {
      if (settled) return;
      settled = true;
      stream.pause();
      cleanup();
      reject(signal?.reason);
    };
    signal?.addEventListener('abort', handleAbort, { once: true });
    stream
      .on('data', (chunk) => {
        if (settled) return;
        byteLength += chunk.byteLength;
        if (byteLength > MAX_THEME_BYTES) {
          settled = true;
          stream.pause();
          cleanup();
          reject(new Error(`${description} is too large.`));
          return;
        }
        chunks.push(chunk);
      })
      .on('error', (cause) => {
        if (settled) return;
        settled = true;
        cleanup();
        reject(cause);
      })
      .on('end', () => {
        if (settled) return;
        settled = true;
        cleanup();
        const bytes = new Uint8Array(byteLength);
        let offset = 0;
        for (const chunk of chunks) {
          bytes.set(chunk, offset);
          offset += chunk.byteLength;
        }
        resolve(new TextDecoder().decode(bytes));
      })
      .resume();
  });
}

async function loadThemeObject(
  zip: JSZip,
  path: string,
  cache: Map<string, Record<string, unknown>>,
  budget: { files: number },
  ancestors: ReadonlySet<string> = new Set(),
  signal?: AbortSignal,
): Promise<Record<string, unknown>> {
  signal?.throwIfAborted();
  if (ancestors.size >= MAX_INCLUDE_DEPTH) throw new Error('Theme includes are nested too deeply.');
  if (ancestors.has(path)) throw new Error('Theme includes contain a cycle.');
  const cached = cache.get(path);
  if (cached) return cached;
  budget.files += 1;
  if (budget.files > MAX_RESOLVED_THEME_FILES) {
    throw new Error('That extension references too many theme files.');
  }
  const value = sanitizeThemeObject(
    parseJsoncObject(await readZipText(zip, path, path, signal), path),
  );
  if (typeof value.include !== 'string') {
    cache.set(path, value);
    return value;
  }
  const includePath = normalizePackagePath(value.include, path);
  const nextAncestors = new Set(ancestors);
  nextAncestors.add(path);
  const base = await loadThemeObject(zip, includePath, cache, budget, nextAncestors, signal);
  const resolved = {
    ...base,
    ...value,
    colors: {
      ...(isRecord(base.colors) ? base.colors : {}),
      ...(isRecord(value.colors) ? value.colors : {}),
    },
  };
  cache.set(path, resolved);
  return resolved;
}

async function readCappedResponse(
  response: Response,
  limit: number,
  tooLargeMessage: string,
): Promise<Uint8Array> {
  const contentLength = response.headers.get('content-length');
  if (contentLength && Number(contentLength) > limit) throw new Error(tooLargeMessage);
  if (!response.body) {
    const bytes = new Uint8Array(await response.arrayBuffer());
    if (bytes.byteLength > limit) throw new Error(tooLargeMessage);
    return bytes;
  }
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let byteLength = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      byteLength += value.byteLength;
      if (byteLength > limit) {
        await reader.cancel();
        throw new Error(tooLargeMessage);
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  const result = new Uint8Array(byteLength);
  let offset = 0;
  for (const chunk of chunks) {
    result.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return result;
}

async function fetchPackage(url: string, signal?: AbortSignal): Promise<Uint8Array> {
  const response = await fetch(url, { ...(signal ? { signal } : {}), credentials: 'omit' });
  if (!response.ok) throw new Error('That Open VSX theme could not be downloaded.');
  return readCappedResponse(response, MAX_VSIX_BYTES, 'That theme extension is too large to import safely.');
}

export async function importOpenVsxThemeExtension(
  extension: OpenVsxThemeExtension,
  signal?: AbortSignal,
): Promise<ThemeLibraryEntryV1[]> {
  for (const url of [extension.manifestUrl, extension.sha256Url, extension.vsixUrl]) {
    if (!trustedOpenVsxUrl(url)) throw new Error('That Open VSX theme contains an untrusted URL.');
  }
  const manifestResponse = await fetch(extension.manifestUrl, {
    ...(signal ? { signal } : {}),
    credentials: 'omit',
  });
  if (!manifestResponse.ok) throw new Error('That Open VSX extension has no readable manifest.');
  const manifest = parseJsoncObject(
    new TextDecoder().decode(await readCappedResponse(
      manifestResponse,
      MAX_MANIFEST_BYTES,
      'That Open VSX extension manifest is too large.',
    )),
    'Extension manifest',
  );
  const advertised = themeContributions(manifest);
  if (advertised.length === 0) throw new Error('That extension does not contain color themes.');
  if (advertised.length > MAX_THEMES_PER_EXTENSION) {
    throw new Error('That extension contains too many color themes to import safely.');
  }
  const packageBytes = await fetchPackage(extension.vsixUrl, signal);
  signal?.throwIfAborted();
  const checksumResponse = await fetch(extension.sha256Url, {
    ...(signal ? { signal } : {}),
    credentials: 'omit',
  });
  if (!checksumResponse.ok) throw new Error('That Open VSX theme has no readable checksum.');
  const expectedChecksum = new TextDecoder()
    .decode(await readCappedResponse(checksumResponse, 256, 'That Open VSX checksum is invalid.'))
    .trim()
    .split(/\s+/)[0];
  if (!expectedChecksum || !/^[a-f\d]{64}$/i.test(expectedChecksum)) {
    throw new Error('That Open VSX theme has an invalid checksum.');
  }
  const actualChecksum = [...sha256(packageBytes)]
    .map((byte) => byte.toString(16).padStart(2, '0'))
    .join('');
  if (actualChecksum.toLowerCase() !== expectedChecksum.toLowerCase()) {
    throw new Error('That Open VSX theme failed its integrity check.');
  }
  signal?.throwIfAborted();
  let zip: JSZip;
  try {
    zip = await JSZip.loadAsync(inspectZipDirectory(packageBytes));
    signal?.throwIfAborted();
    inspectZip(zip);
  } catch (cause) {
    if (signal?.aborted) signal.throwIfAborted();
    if (cause instanceof Error && cause.message.startsWith('That extension package')) throw cause;
    throw new Error(`That Open VSX extension package could not be opened: ${String(cause)}`);
  }
  const packagedManifest = parseJsoncObject(
    await readZipText(zip, 'extension/package.json', 'Extension manifest', signal),
    'Extension manifest',
  );
  if (
    typeof packagedManifest.publisher !== 'string'
    || packagedManifest.publisher.toLowerCase() !== extension.publisher.toLowerCase()
    || typeof packagedManifest.name !== 'string'
    || `${packagedManifest.publisher}.${packagedManifest.name}`.toLowerCase() !== extension.id.toLowerCase()
    || packagedManifest.version !== extension.version
  ) {
    throw new Error('That extension package does not match the selected Open VSX theme.');
  }
  if (!manifestLicenseMatches(packagedManifest, extension.license)) {
    throw new Error('That extension package does not match its advertised license.');
  }
  const contributions = themeContributions(packagedManifest);
  if (contributions.length === 0) throw new Error('That extension does not contain color themes.');
  if (contributions.length > MAX_THEMES_PER_EXTENSION) {
    throw new Error('That extension contains too many color themes to import safely.');
  }
  const converted = [];
  const cache = new Map<string, Record<string, unknown>>();
  const budget = { files: 0 };
  for (const contribution of contributions) {
    signal?.throwIfAborted();
    if (typeof contribution.path !== 'string') {
      throw new Error('One or more color themes have no package path.');
    }
    const path = normalizePackagePath(contribution.path);
    const value = await loadThemeObject(zip, path, cache, budget, new Set(), signal);
    const type = contributionType(contribution.uiTheme);
    const label = typeof contribution.label === 'string' && contribution.label.trim()
      ? contribution.label.trim().slice(0, 64)
      : extension.name;
    const decorated = { ...value, displayName: label, ...(type ? { type } : {}) };
    if (!isVsCodeThemeFile(decorated)) {
      throw new Error('One or more declared color themes are incompatible.');
    }
    converted.push(parseVsCodeThemeFile(decorated, {
      sourceName: path.split('/').slice(-1)[0],
      sourcePath: path,
    }));
  }
  if (converted.length === 0) throw new Error('That extension has no compatible color themes.');
  const extensionId = extension.id.toLowerCase();
  const collection = { id: extension.collectionId, label: extension.name };
  const source = {
    kind: 'open-vsx' as const,
    extensionId: extension.id,
    version: extension.version,
    license: extension.license,
    ...(extension.sourceUrl ? { sourceUrl: extension.sourceUrl } : {}),
  };
  const entries = pairVsCodeThemes(converted).map((theme) => {
    const entry: ThemeLibraryEntryV1 = {
      version: 1,
      id: openVsxThemeId(extensionId, [...theme.sourceIdentities].sort().join(':')),
      label: theme.label,
      modes: theme.modes,
      theme: theme.theme,
      source,
      collection,
    };
    const parsed = parseThemeLibraryEntry(entry);
    if (!parsed) throw new Error('That extension produced an invalid theme entry.');
    return parsed;
  });
  if (new Set(entries.map((entry) => entry.id)).size !== entries.length) {
    throw new Error('That extension produced duplicate theme identities.');
  }
  return entries;
}
