import { openUrl } from '@tauri-apps/plugin-opener';
import { useEffect, useRef, useState, type FormEvent } from 'react';
import type {
  OpenVsxThemeExtension,
  OpenVsxThemeSort,
} from '../../lib/appearance/openVsxThemes';
import { useAppearance } from '../../lib/hooks/useAppearance';

const SUGGESTIONS = ['Dracula', 'Catppuccin', 'Nord', 'Tokyo Night'];
const SORT_OPTIONS: Array<{ value: OpenVsxThemeSort; label: string }> = [
  { value: 'downloadCount', label: 'Most downloaded' },
  { value: 'rating', label: 'Best rated' },
  { value: 'timestamp', label: 'Newest' },
  { value: 'relevance', label: 'Most relevant' },
];
const DOWNLOAD_FORMAT = new Intl.NumberFormat(undefined, {
  notation: 'compact',
  maximumFractionDigits: 1,
});

interface Props {
  open: boolean;
  onClose: () => void;
}

export function CommunityThemeDialog({ open, onClose }: Props) {
  const appearance = useAppearance();
  const [query, setQuery] = useState('');
  const [sortBy, setSortBy] = useState<OpenVsxThemeSort>('downloadCount');
  const [results, setResults] = useState<OpenVsxThemeExtension[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [installingId, setInstallingId] = useState<string | null>(null);
  const [pendingUpdate, setPendingUpdate] = useState<OpenVsxThemeExtension | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const requestRef = useRef<AbortController | null>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    if (!open) {
      requestRef.current?.abort();
      requestRef.current = null;
      return;
    }
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setQuery('');
    setSortBy('downloadCount');
    setResults(null);
    setSearching(false);
    setInstallingId(null);
    setPendingUpdate(null);
    setError(null);
    setNotice(null);
    const focusTimer = window.setTimeout(() => searchRef.current?.focus(), 30);
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        onCloseRef.current();
        return;
      }
      if (event.key !== 'Tab') return;
      const focusable = Array.from(dialogRef.current?.querySelectorAll<HTMLElement>(
        'button:not([disabled]), input:not([disabled]), select:not([disabled]), a[href]',
      ) ?? []);
      if (focusable.length === 0) return;
      const first = focusable[0]!;
      const last = focusable[focusable.length - 1]!;
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener('keydown', onKeyDown);
    return () => {
      window.clearTimeout(focusTimer);
      document.removeEventListener('keydown', onKeyDown);
      requestRef.current?.abort();
      previous?.focus();
    };
  }, [open]);

  if (!open) return null;

  const runSearch = async (searchText: string, nextSort = sortBy) => {
    const trimmed = searchText.trim();
    if (!trimmed) return;
    requestRef.current?.abort();
    const controller = new AbortController();
    requestRef.current = controller;
    setQuery(trimmed);
    setSearching(true);
    setResults(null);
    setError(null);
    setNotice(null);
    try {
      const { searchOpenVsxThemes } = await import('../../lib/appearance/openVsxThemes');
      const themes = await searchOpenVsxThemes(trimmed, {
        signal: controller.signal,
        sortBy: nextSort,
      });
      if (!controller.signal.aborted) setResults(themes);
    } catch (cause) {
      if (!controller.signal.aborted) {
        setError(cause instanceof Error ? cause.message : 'Open VSX search failed.');
      }
    } finally {
      if (requestRef.current === controller) {
        requestRef.current = null;
        setSearching(false);
      }
    }
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    void runSearch(query);
  };

  const installedCollection = (extension: OpenVsxThemeExtension) =>
    appearance.library.document.themes.filter(
      (theme) => theme.collection?.id === extension.collectionId,
    );

  const install = async (extension: OpenVsxThemeExtension, allowUpdate: boolean) => {
    const existing = installedCollection(extension);
    if (existing.length > 0 && !allowUpdate) {
      setPendingUpdate(extension);
      return;
    }
    requestRef.current?.abort();
    const controller = new AbortController();
    requestRef.current = controller;
    setInstallingId(extension.id);
    setError(null);
    setNotice(null);
    try {
      const { importOpenVsxThemeExtension } = await import('../../lib/appearance/openVsxThemes');
      const entries = await importOpenVsxThemeExtension(extension, controller.signal);
      if (controller.signal.aborted) return;
      if (existing.length > 0) {
        await appearance.library.replaceCollection(extension.collectionId, entries, existing);
        setNotice(`${extension.name} updated. Review its variants in your library.`);
      } else {
        await appearance.library.install(entries);
        setNotice(`${extension.name} added to your theme library.`);
      }
    } catch (cause) {
      if (!controller.signal.aborted) {
        setError(cause instanceof Error ? cause.message : 'That theme could not be added.');
      }
    } finally {
      if (requestRef.current === controller) {
        requestRef.current = null;
        setInstallingId(null);
      }
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-background/80 p-5 backdrop-blur-[2px]"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && installingId === null) onClose();
      }}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="community-theme-title"
        className="flex max-h-[86vh] w-full max-w-[760px] flex-col overflow-hidden rounded-2xl border border-on-surface-variant bg-surface shadow-2xl"
      >
        <div className="flex items-start justify-between gap-4 border-b border-outline-variant px-5 py-4">
          <div>
            <h2 id="community-theme-title" className="text-base font-semibold text-on-surface">
              Community themes
            </h2>
            <p className="mt-1 text-xs text-on-surface-variant">
              Search sends your query and normal connection metadata to open-vsx.org. Murmur downloads only after you choose Add and never runs extension code.
            </p>
          </div>
          <button
            type="button"
            aria-label="Close community themes"
            disabled={installingId !== null}
            onClick={onClose}
            className="rounded-md px-2 py-1 text-on-surface-variant hover:bg-surface-container disabled:opacity-50"
          >
            ✕
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
          <form className="flex gap-2" onSubmit={submit}>
            <input
              ref={searchRef}
              type="search"
              aria-label="Search Open VSX themes"
              value={query}
              onChange={(event) => setQuery(event.currentTarget.value)}
              placeholder="Try Dracula, Nord, Catppuccin…"
              className="min-w-0 flex-1 rounded-lg border border-on-surface-variant bg-surface-container-lowest px-3 py-2 text-sm text-on-surface outline-none placeholder:text-on-surface-variant focus:border-primary focus:ring-1 focus:ring-primary"
            />
            <select
              aria-label="Sort community themes"
              value={sortBy}
              disabled={searching || installingId !== null}
              onChange={(event) => {
                const next = event.currentTarget.value as OpenVsxThemeSort;
                setSortBy(next);
                if (query.trim()) void runSearch(query, next);
              }}
              className="rounded-lg border border-on-surface-variant bg-surface-container-lowest px-2 text-xs text-on-surface"
            >
              {SORT_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>{option.label}</option>
              ))}
            </select>
            <button
              type="submit"
              disabled={!query.trim() || searching || installingId !== null}
              className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-on-primary hover:bg-primary-dim disabled:opacity-50"
            >
              {searching ? 'Searching…' : 'Search'}
            </button>
          </form>

          {!results && !searching && (
            <div className="mt-3 flex flex-wrap items-center gap-1.5">
              <span className="text-xs text-on-surface-variant">Popular:</span>
              {SUGGESTIONS.map((suggestion) => (
                <button
                  key={suggestion}
                  type="button"
                  onClick={() => void runSearch(suggestion)}
                  className="rounded-full px-2 py-1 text-xs text-on-surface hover:bg-surface-container"
                >
                  {suggestion}
                </button>
              ))}
            </div>
          )}

          {pendingUpdate && (
            <div className="mt-4 rounded-xl border border-warning/30 bg-warning/10 p-3">
              <p className="text-sm font-medium text-on-surface">Update {pendingUpdate.name}?</p>
              <p className="mt-1 text-xs text-on-surface">
                This replaces every installed variant from that extension, including local edits, and removes variants the new package no longer contains.
              </p>
              <div className="mt-3 flex gap-2">
                <button
                  type="button"
                  onClick={() => {
                    const extension = pendingUpdate;
                    setPendingUpdate(null);
                    void install(extension, true);
                  }}
                  className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-on-primary"
                >
                  Update collection
                </button>
                <button
                  type="button"
                  onClick={() => setPendingUpdate(null)}
                  className="rounded-lg px-3 py-1.5 text-xs font-medium text-on-surface hover:bg-surface-container"
                >
                  Cancel
                </button>
              </div>
            </div>
          )}

          <div className="sr-only" role="status">
            {searching ? 'Searching Open VSX themes.' : results ? `${results.length} supported themes found.` : notice ?? ''}
          </div>
          {notice && (
            <p className="mt-4 rounded-lg border border-success/30 bg-success/10 px-3 py-2 text-xs text-success">
              {notice}
            </p>
          )}
          {error && (
            <p role="alert" className="mt-4 rounded-lg border border-error/30 bg-error/10 px-3 py-2 text-xs text-error">
              {error}
            </p>
          )}
          {searching && (
            <div className="flex min-h-40 items-center justify-center text-sm text-on-surface-variant">
              Finding supported open-source themes…
            </div>
          )}
          {results && results.length === 0 && (
            <div className="mt-4 flex min-h-40 items-center justify-center rounded-xl border border-dashed border-on-surface-variant text-center">
              <div>
                <p className="text-sm font-medium text-on-surface">No supported themes found</p>
                <p className="mt-1 text-xs text-on-surface-variant">Try a broader search.</p>
              </div>
            </div>
          )}
          {results && results.length > 0 && (
            <div className="mt-4 grid gap-2 sm:grid-cols-2">
              {results.map((extension) => {
                const installed = installedCollection(extension).length > 0;
                const installing = installingId === extension.id;
                return (
                  <article
                    key={extension.id}
                    className="flex min-w-0 flex-col gap-2 rounded-xl border border-on-surface-variant/70 bg-surface-container-lowest p-3 shadow-sm"
                  >
                    <div className="flex items-start gap-3">
                      <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-on-surface" aria-hidden>
                        ◐
                      </div>
                      <div className="min-w-0 flex-1">
                        <h3 className="truncate text-sm font-medium text-on-surface">{extension.name}</h3>
                        <p className="truncate text-[11px] text-on-surface-variant">
                          {extension.publisher} · {DOWNLOAD_FORMAT.format(extension.downloadCount)} downloads
                        </p>
                      </div>
                    </div>
                    <p className="line-clamp-2 min-h-8 text-xs leading-4 text-on-surface-variant">
                      {extension.description || 'A community color theme for VS Code-compatible editors.'}
                    </p>
                    <div className="mt-auto flex items-center justify-between gap-2">
                      <div className="flex min-w-0 items-center gap-2 text-[11px] text-on-surface-variant">
                        <span>{extension.license}</span>
                        {extension.sourceUrl && (
                          <button
                            type="button"
                            onClick={() => void openUrl(extension.sourceUrl!)}
                            className="hover:text-primary"
                          >
                            Source ↗
                          </button>
                        )}
                      </div>
                      <button
                        type="button"
                        disabled={installingId !== null}
                        onClick={() => void install(extension, false)}
                        className="rounded-lg border border-on-surface-variant px-2.5 py-1.5 text-xs font-medium text-on-surface hover:border-primary hover:bg-surface-container disabled:opacity-50"
                      >
                        {installing ? (installed ? 'Updating…' : 'Adding…') : installed ? 'Update' : 'Add'}
                      </button>
                    </div>
                  </article>
                );
              })}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
