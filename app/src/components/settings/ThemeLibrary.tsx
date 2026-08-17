import { save } from '@tauri-apps/plugin-dialog';
import { useMemo, useState, type MouseEvent as ReactMouseEvent } from 'react';
import {
  appearanceSelection,
  previewThemeLibraryPairSelection,
  resolveTheme,
  type MurmurTokens,
  type ResolvedAppearance,
  type ThemeImportPreview,
  type ThemeLibraryEntryV1,
} from '../../lib/appearance';
import { useAppearance } from '../../lib/hooks/useAppearance';

interface Props {
  onBrowse: () => void;
  onImport: () => void;
  onCustomize: () => void;
}

function SunIcon({ className = 'h-3 w-3' }: { className?: string }) {
  return (
    <svg aria-hidden="true" className={className} fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2v2m0 16v2M4.9 4.9l1.4 1.4m11.4 11.4 1.4 1.4M2 12h2m16 0h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />
    </svg>
  );
}

function MoonIcon({ className = 'h-3 w-3' }: { className?: string }) {
  return (
    <svg aria-hidden="true" className={className} fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
      <path d="M21 12.8A9 9 0 1 1 11.2 3 7 7 0 0 0 21 12.8Z" />
    </svg>
  );
}

function DownloadIcon() {
  return (
    <svg aria-hidden="true" className="h-3.5 w-3.5" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
      <path d="M12 3v12m0 0 4-4m-4 4-4-4M4 17v3h16v-3" />
    </svg>
  );
}

function UploadIcon() {
  return (
    <svg aria-hidden="true" className="h-3.5 w-3.5" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
      <path d="M12 21V9m0 0 4 4m-4-4-4 4M4 7V4h16v3" />
    </svg>
  );
}

function TrashIcon() {
  return (
    <svg aria-hidden="true" className="h-3.5 w-3.5" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
      <path d="M4 7h16M9 7V4h6v3m3 0-1 14H7L6 7m4 4v6m4-6v6" />
    </svg>
  );
}

function EditIcon() {
  return (
    <svg aria-hidden="true" className="h-3.5 w-3.5" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
      <path d="m4 20 4.5-1 10-10a2.1 2.1 0 0 0-3-3l-10 10L4 20Zm10-12 3 3" />
    </svg>
  );
}

function CopyIcon() {
  return (
    <svg aria-hidden="true" className="h-3.5 w-3.5" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
      <rect x="8" y="8" width="11" height="11" rx="2" />
      <path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2" />
    </svg>
  );
}

function ThemeOrb({ tokens, mode, small = false }: { tokens: MurmurTokens; mode: ResolvedAppearance; small?: boolean }) {
  return (
    <span
      aria-hidden="true"
      className={`${small ? 'h-9 w-9 scale-[0.55]' : 'h-9 w-9'} block shrink-0 rounded-full border-2 border-surface-container-lowest`}
      style={{
        backgroundColor: tokens.background,
        backgroundImage: [
          `radial-gradient(circle at 35% 70%, ${tokens.primary} 0%, transparent 62%)`,
          `radial-gradient(circle at 75% 25%, ${tokens['surface-container-highest']} 0%, transparent 58%)`,
        ].join(', '),
        boxShadow: mode === 'dark'
          ? `inset 0 0 0 1px ${tokens['on-surface-variant']}, 0 1px 3px ${tokens.background}`
          : `inset 0 0 0 1px ${tokens['outline-variant']}, 0 1px 3px ${tokens['on-surface-variant']}`,
      }}
    />
  );
}

function shortVariantLabels(entries: readonly ThemeLibraryEntryV1[]) {
  if (entries.length === 0) return new Map<string, string>();
  const words = entries.map((entry) => entry.label.trim().split(/\s+/));
  const first = words[0]!;
  const mismatch = first.findIndex((word, index) =>
    words.some((label) => label[index]?.toLocaleLowerCase() !== word.toLocaleLowerCase()),
  );
  const prefix = mismatch === -1 ? Math.max(0, first.length - 1) : mismatch;
  return new Map(entries.map((entry, index) => [
    entry.id,
    words[index]?.slice(prefix).join(' ').trim() || entry.label,
  ]));
}

interface ModeOption {
  entry: ThemeLibraryEntryV1;
  tokens: MurmurTokens;
}

function ModePreview({
  mode,
  options,
  selected,
  active,
  interactive,
  labels,
  open,
  offset,
  showLabel,
  onOpen,
  onChoose,
}: {
  mode: ResolvedAppearance;
  options: readonly ModeOption[];
  selected: ModeOption;
  active: boolean;
  interactive: boolean;
  labels: ReadonlyMap<string, string>;
  open: boolean;
  offset: number;
  showLabel: boolean;
  onOpen: () => void;
  onChoose: (entry: ThemeLibraryEntryV1) => void;
}) {
  const modeLabel = mode === 'light' ? 'Light' : 'Dark';
  const selectedLabel = labels.get(selected.entry.id) ?? selected.entry.label;
  const preview = (
    <>
      <ThemeOrb tokens={selected.tokens} mode={mode} />
      <span className="pointer-events-none absolute -bottom-0.5 -right-0.5 flex h-4 w-4 items-center justify-center rounded-full border border-on-surface-variant bg-background text-on-surface shadow-sm">
        {mode === 'light' ? <SunIcon className="h-2.5 w-2.5" /> : <MoonIcon className="h-2.5 w-2.5" />}
      </span>
    </>
  );
  return (
    <>
      {interactive ? (
        <button
          type="button"
          aria-label={`Choose ${mode} variant, ${options.length} options, currently ${selectedLabel}`}
          aria-pressed={active}
          title={`${modeLabel}: ${selectedLabel} · hover for variants`}
          onClick={(event) => {
            event.stopPropagation();
            onChoose(selected.entry);
          }}
          onFocus={onOpen}
          onMouseEnter={onOpen}
          className="absolute left-1/2 top-1 z-20 flex h-11 w-11 items-center justify-center rounded-full outline-none transition-transform hover:scale-105 focus-visible:ring-2 focus-visible:ring-primary"
          style={{ transform: `translateX(calc(-50% + ${offset}px))` }}
        >
          {preview}
        </button>
      ) : (
        <span
          aria-label={`${modeLabel} preview: ${selectedLabel}`}
          role="img"
          title={`${modeLabel}: ${selectedLabel}`}
          className="pointer-events-none absolute left-1/2 top-1 z-20 flex h-11 w-11 items-center justify-center"
          style={{ transform: `translateX(calc(-50% + ${offset}px))` }}
        >
          {preview}
        </span>
      )}
      {showLabel && (
        <span
          className="pointer-events-none absolute bottom-0 left-1/2 inline-flex max-w-20 -translate-x-1/2 items-center gap-1 text-[10px] font-medium text-on-surface"
          style={{ marginLeft: offset }}
        >
          <span className="truncate">{selectedLabel}</span>
          {options.length > 1 && (
            <span className="shrink-0 rounded-full bg-surface-container-high px-1 text-[9px] text-on-surface-variant">
              +{options.length - 1}
            </span>
          )}
        </span>
      )}
      {options.length > 1 && options.map((option, index) => {
        const progress = index / (options.length - 1) - 0.5;
        const childOffsetX = offset + progress * 44;
        const childOffsetY = Math.abs(progress) * 7;
        const optionActive = option.entry.id === selected.entry.id;
        const optionLabel = labels.get(option.entry.id) ?? option.entry.label;
        return (
          <button
            type="button"
            key={`${mode}-${option.entry.id}`}
            aria-label={`Use ${optionLabel} for ${mode} mode${optionActive ? ', currently selected' : ''}`}
            aria-pressed={optionActive}
            title={`Use ${optionLabel} for ${mode} mode`}
            onClick={(event) => {
              event.stopPropagation();
              onChoose(option.entry);
            }}
            onFocus={onOpen}
            onMouseEnter={onOpen}
            className={`absolute left-1/2 top-0 z-30 flex h-6 w-6 items-center justify-center overflow-hidden rounded-full bg-background shadow-sm outline-none transition-[transform,opacity] duration-200 focus-visible:ring-2 focus-visible:ring-primary ${optionActive ? 'ring-2 ring-primary' : 'ring-1 ring-on-surface-variant'}`}
            style={{
              opacity: open ? 1 : 0,
              pointerEvents: open ? 'auto' : 'none',
              transform: `translate(calc(-50% + ${open ? childOffsetX : offset}px), ${open ? childOffsetY : 20}px) scale(${open ? 1 : 0.55})`,
              transitionDelay: open ? `${index * 35}ms` : '0ms',
            }}
          >
            <ThemeOrb tokens={option.tokens} mode={mode} small />
          </button>
        );
      })}
    </>
  );
}

function ActionButton({ label, onClick, children, danger = false }: {
  label: string;
  onClick: () => void;
  children: React.ReactNode;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={(event) => {
        event.stopPropagation();
        onClick();
      }}
      className={`grid h-6 w-6 place-items-center rounded-md outline-none hover:bg-surface-container focus-visible:ring-2 focus-visible:ring-primary ${danger ? 'text-on-surface-variant hover:text-error' : 'text-on-surface-variant hover:text-on-surface'}`}
    >
      {children}
    </button>
  );
}

function CollectionCard({
  label,
  entries,
  selection,
  onApplyPair,
  onApplyMode,
  onCustomize,
  customizeAsCopy = false,
  onExport,
  onRemove,
}: {
  label: string;
  entries: readonly ThemeLibraryEntryV1[];
  selection: { light: string; dark: string };
  onApplyPair: (light: ThemeLibraryEntryV1 | null, dark: ThemeLibraryEntryV1 | null) => void;
  onApplyMode: (entry: ThemeLibraryEntryV1, mode: ResolvedAppearance) => void;
  onCustomize?: () => void;
  customizeAsCopy?: boolean;
  onExport?: (entry: ThemeLibraryEntryV1) => void;
  onRemove?: () => void;
}) {
  const [radialOpen, setRadialOpen] = useState<ResolvedAppearance | null>(null);
  const labels = shortVariantLabels(entries);
  const groups = (['light', 'dark'] as const).flatMap((mode) => {
    const options = entries
      .filter((entry) => entry.modes.includes(mode))
      .map((entry) => ({ entry, tokens: resolveTheme(entry.theme, mode).tokens }));
    if (options.length === 0) return [];
    const selected = options.find((option) => option.entry.id === selection[mode]) ?? options[0]!;
    return [{ mode, options, selected }];
  });
  const light = groups.find((group) => group.mode === 'light')?.selected.entry ?? null;
  const dark = groups.find((group) => group.mode === 'dark')?.selected.entry ?? null;
  const ids = new Set(entries.map((entry) => entry.id));
  const fullyActive = ids.has(selection.light) && ids.has(selection.dark);
  const showVariantLabels = new Set(entries.map((entry) => entry.id)).size > 1;
  const exportEntry = groups.find((group) => group.mode === 'dark')?.selected.entry
    ?? groups[0]?.selected.entry;
  const applyPair = () => onApplyPair(light, dark);
  const handleCardClick = (event: ReactMouseEvent<HTMLElement>) => {
    if (event.defaultPrevented) return;
    applyPair();
  };

  return (
    <article
      data-theme-collection={label}
      onClick={handleCardClick}
      className="w-52 max-w-full cursor-pointer overflow-hidden rounded-xl border border-on-surface-variant/70 bg-surface-container-lowest transition-colors hover:bg-surface-container-low"
    >
      <div
        role="group"
        aria-label={`${label} light and dark styles`}
        className={showVariantLabels ? "relative h-16" : "relative h-12"}
        onMouseLeave={() => setRadialOpen(null)}
        onBlurCapture={(event) => {
          const next = event.relatedTarget;
          if (!(next instanceof Node) || !event.currentTarget.contains(next)) setRadialOpen(null);
        }}
        onKeyDown={(event) => {
          if (event.key === 'Escape') setRadialOpen(null);
        }}
      >
        {groups.map((group, index) => {
          const offset = groups.length === 1 ? 0 : index === 0 ? -34 : 34;
          return (
            <ModePreview
              key={group.mode}
              mode={group.mode}
              options={group.options}
              selected={group.selected}
              active={selection[group.mode] === group.selected.entry.id}
              interactive={group.options.length > 1}
              labels={labels}
              open={radialOpen === group.mode}
              offset={offset}
              showLabel={showVariantLabels}
              onOpen={() => setRadialOpen(group.mode)}
              onChoose={(entry) => {
                onApplyMode(entry, group.mode);
                setRadialOpen(null);
              }}
            />
          );
        })}
      </div>
      <div className="flex min-h-9 items-center gap-2 px-2.5 pb-2 pt-1">
        <button
          type="button"
          aria-label={`Use ${label} theme`}
          aria-pressed={fullyActive}
          onClick={(event) => {
            event.stopPropagation();
            applyPair();
          }}
          className="min-w-0 flex-1 truncate rounded-sm text-left text-sm font-medium text-on-surface outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          <span className="inline-flex items-center gap-1.5">
            <span className="truncate">{label}</span>
            {fullyActive && <span aria-label="Active theme" title="Active theme" className="text-xs font-bold text-primary">✓</span>}
          </span>
        </button>
        {(onCustomize || onExport || onRemove) && (
          <div className="flex shrink-0 items-center gap-0.5">
            {onCustomize && (
              <ActionButton label={customizeAsCopy ? `Create theme from ${label}` : `Edit ${label}`} onClick={onCustomize}>
                {customizeAsCopy ? <CopyIcon /> : <EditIcon />}
              </ActionButton>
            )}
            {onExport && exportEntry && <ActionButton label={`Export ${label}`} onClick={() => onExport(exportEntry)}><UploadIcon /></ActionButton>}
            {onRemove && <ActionButton label={`Remove ${label}`} onClick={onRemove} danger><TrashIcon /></ActionButton>}
          </div>
        )}
      </div>
    </article>
  );
}

export function ThemeLibrary({ onBrowse, onImport, onCustomize }: Props) {
  const appearance = useAppearance();
  const selection = appearanceSelection(appearance.document);
  const [removeTarget, setRemoveTarget] = useState<{ label: string; ids: string[] } | null>(null);
  const [importMenuOpen, setImportMenuOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const groups = useMemo(() => {
    const grouped = new Map<string, { label: string; entries: ThemeLibraryEntryV1[] }>();
    for (const entry of appearance.library.document.themes) {
      const key = entry.collection ? `collection:${entry.collection.id}` : `theme:${entry.id}`;
      const existing = grouped.get(key);
      if (existing) existing.entries.push(entry);
      else grouped.set(key, { label: entry.collection?.label ?? entry.label, entries: [entry] });
    }
    return [...grouped.entries()];
  }, [appearance.library.document.themes]);

  const commit = async (preview: ThemeImportPreview) => {
    try {
      setError(null);
      await appearance.commitImport(preview);
    } catch (cause) {
      setError(String(cause));
    }
  };

  const applyPair = (light: ThemeLibraryEntryV1 | null, dark: ThemeLibraryEntryV1 | null) => {
    try {
      void commit(previewThemeLibraryPairSelection(
        appearance.document,
        appearance.library.document,
        light?.id ?? 'sonic',
        dark?.id ?? 'sonic',
      ));
    } catch (cause) {
      setError(String(cause));
    }
  };

  const applyMode = (entry: ThemeLibraryEntryV1, mode: ResolvedAppearance) => {
    try {
      void commit(appearance.library.previewSelection(entry.id, mode));
    } catch (cause) {
      setError(String(cause));
    }
  };

  const exportEntry = async (entry: ThemeLibraryEntryV1) => {
    setError(null);
    try {
      const path = await save({
        defaultPath: `${entry.id}.murmur-theme.json`,
        filters: [{ name: 'Murmur Theme', extensions: ['json'] }],
      });
      if (typeof path === 'string') await appearance.library.exportEntryToPath(entry, path);
    } catch (cause) {
      setError(String(cause));
    }
  };

  const sonicEntries: ThemeLibraryEntryV1[] = (['light', 'dark'] as const).map((mode) => ({
    version: 1,
    id: 'sonic',
    label: 'Sonic',
    modes: [mode],
    theme: { version: 1, presetId: 'sonic' },
    source: { kind: 'local' },
  }));
  const customEntry: ThemeLibraryEntryV1 = {
    version: 1,
    id: 'custom',
    label: 'Custom',
    modes: ['light', 'dark'],
    theme: appearance.document.theme,
    source: { kind: 'local' },
  };
  const applyCustom = () => void commit({
    mode: appearance.document.mode,
    theme: appearance.document.theme,
    light: appearance.document.cache.light,
    dark: appearance.document.cache.dark,
    adjustments: appearance.adjustments,
    selection: { light: 'custom', dark: 'custom' },
  });

  return (
    <section aria-labelledby="themes-heading" className="space-y-2">
      <div className="flex min-h-8 flex-wrap items-center justify-between gap-3 pt-1">
        <h2 id="themes-heading" className="text-sm font-medium text-on-surface">Themes</h2>
        <div className="flex items-center gap-2">
          <button type="button" onClick={onCustomize} className="inline-flex h-7 items-center gap-1 rounded-lg border border-on-surface-variant/70 bg-surface-container-lowest px-2.5 text-xs font-medium text-on-surface hover:bg-surface-container">
            <span aria-hidden="true">＋</span> Create theme
          </button>
          <div
            className="relative"
            onBlurCapture={(event) => {
              const next = event.relatedTarget;
              if (!(next instanceof Node) || !event.currentTarget.contains(next)) setImportMenuOpen(false);
            }}
          >
            <button
              type="button"
              aria-haspopup="menu"
              aria-expanded={importMenuOpen}
              onClick={() => setImportMenuOpen((open) => !open)}
              onKeyDown={(event) => {
                if (event.key === 'Escape') setImportMenuOpen(false);
              }}
              className="inline-flex h-7 items-center gap-1 rounded-lg border border-on-surface-variant/70 bg-surface-container-lowest px-2.5 text-xs font-medium text-on-surface hover:bg-surface-container"
            >
              <DownloadIcon /> Import theme
            </button>
            {importMenuOpen && (
              <div role="menu" className="absolute right-0 top-9 z-40 w-44 overflow-hidden rounded-lg border border-on-surface-variant bg-surface-container-lowest p-1 shadow-lg">
                <button type="button" role="menuitem" onClick={() => { setImportMenuOpen(false); onImport(); }} className="flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-xs text-on-surface hover:bg-surface-container">
                  <DownloadIcon /> Import file
                </button>
                <button type="button" role="menuitem" onClick={() => { setImportMenuOpen(false); onBrowse(); }} className="flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-xs text-on-surface hover:bg-surface-container">
                  <span aria-hidden="true" className="text-sm">⌕</span> Browse Open VSX
                </button>
              </div>
            )}
          </div>
        </div>
      </div>

      <div className="flex flex-wrap items-start gap-2">
        <CollectionCard
          label="Sonic"
          entries={sonicEntries}
          selection={selection}
          onApplyPair={() => {
            try { void commit(appearance.library.previewSelection('sonic')); }
            catch (cause) { setError(String(cause)); }
          }}
          onApplyMode={(_entry, mode) => {
            try { void commit(appearance.library.previewSelection('sonic', mode)); }
            catch (cause) { setError(String(cause)); }
          }}
          onCustomize={onCustomize}
          customizeAsCopy
        />
        {(selection.light === 'custom' || selection.dark === 'custom') && (
          <CollectionCard
            label="Custom"
            entries={[customEntry]}
            selection={selection}
            onApplyPair={applyCustom}
            onApplyMode={applyCustom}
            onCustomize={onCustomize}
          />
        )}
        {groups.map(([key, group]) => (
          <CollectionCard
            key={key}
            label={group.label}
            entries={group.entries}
            selection={selection}
            onApplyPair={applyPair}
            onApplyMode={applyMode}
            onExport={(entry) => void exportEntry(entry)}
            onRemove={() => setRemoveTarget({ label: group.label, ids: group.entries.map((entry) => entry.id) })}
          />
        ))}
      </div>

      {removeTarget && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-background/80 p-5 backdrop-blur-[2px]" onMouseDown={(event) => { if (event.target === event.currentTarget) setRemoveTarget(null); }}>
          <div role="dialog" aria-modal="true" aria-labelledby="remove-theme-title" className="w-full max-w-sm rounded-2xl border border-on-surface-variant bg-surface p-5 shadow-2xl">
            <h2 id="remove-theme-title" className="text-base font-semibold text-on-surface">Remove {removeTarget.label}?</h2>
            <p className="mt-1 text-xs text-on-surface-variant">Every imported variant in this collection will be removed. You can import it again later.</p>
            <div className="mt-4 flex justify-end gap-2">
              <button type="button" onClick={() => setRemoveTarget(null)} className="rounded-lg border border-on-surface-variant px-3 py-1.5 text-xs font-medium text-on-surface hover:bg-surface-container">Cancel</button>
              <button type="button" onClick={() => { const target = removeTarget; setRemoveTarget(null); void appearance.library.remove(target.ids).catch((cause) => setError(String(cause))); }} className="rounded-lg border border-error bg-error/10 px-3 py-1.5 text-xs font-medium text-error">Remove</button>
            </div>
          </div>
        </div>
      )}

      {(error || appearance.library.error) && (
        <p role="alert" className="rounded-lg border border-error bg-error/10 px-3 py-2 text-xs text-error">{error ?? appearance.library.error}</p>
      )}
    </section>
  );
}
