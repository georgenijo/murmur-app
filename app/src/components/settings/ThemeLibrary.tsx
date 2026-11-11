import { save } from '@tauri-apps/plugin-dialog';
import { useMemo, useState, useRef, type MouseEvent as ReactMouseEvent, type Ref, type CSSProperties } from 'react';
import {
  availableThemeId,
  BUILTIN_THEME_LIBRARY_ENTRIES,
  effectiveAppearanceSelection,
  makeLocalThemeEntry,
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
  onCustomize: (opener: HTMLButtonElement) => void;
  createThemeRef?: Ref<HTMLButtonElement>;
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

/** Preview the resolved palette, including imported per-mode colors. */
function ThemePreview({ tokens }: { tokens: MurmurTokens }) {
  const style = Object.fromEntries(['surface', 'surface-container-low', 'on-surface', 'on-surface-variant', 'primary', 'outline-variant']
    .map(key => [`--preview-${key}`, tokens[key as keyof MurmurTokens]])) as CSSProperties;
  return <span aria-hidden="true" className="settings-theme-preview" style={style}>
    <span className="settings-theme-preview-sidebar" />
    <span className="settings-theme-preview-copy"><i /><i /><i /><b /></span>
  </span>;
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

function ModePreview({ mode, options, selected, active, labels, open, onOpen, onChoose }: {
  mode: ResolvedAppearance;
  options: readonly ModeOption[];
  selected: ModeOption;
  active: boolean;
  labels: ReadonlyMap<string, string>;
  open: boolean;
  onOpen: () => void;
  onChoose: (entry: ThemeLibraryEntryV1) => void;
}) {
  const openerRef = useRef<HTMLButtonElement>(null);
  const modeLabel = mode === 'light' ? 'Light' : 'Dark';
  const selectedLabel = labels.get(selected.entry.id) ?? selected.entry.label;
  return <div className="settings-theme-mode">
    <button ref={openerRef} type="button" className="settings-theme-variant"
      aria-label={options.length > 1
        ? `Choose ${mode} variant, ${options.length} options, currently ${selectedLabel}`
        : `Use ${selectedLabel} for ${mode} mode`}
      aria-pressed={active}
      aria-expanded={options.length > 1 ? open : undefined}
      onClick={event => {
        event.stopPropagation();
        if (options.length > 1) onOpen();
        else onChoose(selected.entry);
      }}>
      <ThemePreview tokens={selected.tokens} />
      {active && <span aria-hidden="true" className="settings-theme-variant-check">✓</span>}
    </button>
    {options.length > 1 && open && <div className="settings-theme-options" role="group" aria-label={`${modeLabel} variants`}
      onKeyDown={event => { if (event.key === 'Escape') { event.stopPropagation(); onOpen(); openerRef.current?.focus(); } }}>
      {options.map(option => <button type="button" key={option.entry.id}
        aria-label={`Use ${labels.get(option.entry.id) ?? option.entry.label} for ${mode} mode${option.entry.id === selected.entry.id ? ', currently selected' : ''}`}
        aria-pressed={option.entry.id === selected.entry.id}
        onClick={event => { event.stopPropagation(); onChoose(option.entry); openerRef.current?.focus(); }}>
        <span>{labels.get(option.entry.id) ?? option.entry.label}</span>
        {option.entry.id === selected.entry.id && <span aria-hidden="true" className="settings-theme-option-check">✓</span>}
      </button>)}
    </div>}
  </div>;
}

function ActionButton({ label, onClick, children, danger = false, disabled = false }: {
  label: string;
  onClick: (event: ReactMouseEvent<HTMLButtonElement>) => void;
  children: React.ReactNode;
  danger?: boolean;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      aria-label={label}
      title={label}
      onClick={(event) => {
        event.stopPropagation();
        onClick(event);
      }}
      className={`settings-theme-action grid place-items-center rounded-md outline-none hover:bg-surface-container focus-visible:ring-2 focus-visible:ring-primary ${danger ? 'text-on-surface-variant hover:text-error' : 'text-on-surface-variant hover:text-on-surface'}`}
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
  onDuplicate,
  duplicateDisabled = false,
  onRemove,
  resolvedAppearance,
}: {
  label: string;
  entries: readonly ThemeLibraryEntryV1[];
  selection: { light: string; dark: string };
  onApplyPair: (light: ThemeLibraryEntryV1 | null, dark: ThemeLibraryEntryV1 | null) => void;
  onApplyMode: (entry: ThemeLibraryEntryV1, mode: ResolvedAppearance) => void;
  onCustomize?: (opener: HTMLButtonElement) => void;
  customizeAsCopy?: boolean;
  onExport?: (entry: ThemeLibraryEntryV1) => void;
  onDuplicate?: (entry: ThemeLibraryEntryV1) => void;
  duplicateDisabled?: boolean;
  onRemove?: () => void;
  resolvedAppearance: ResolvedAppearance;
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
  const activeNow = ids.has(selection[resolvedAppearance]);
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
      data-active={activeNow ? "true" : undefined}
      onClick={handleCardClick}
      className={`settings-theme-collection ${activeNow ? 'settings-theme-active' : ''}`}
    >
      <div
        role="group"
        aria-label={`${label} light and dark styles`}
        className="settings-theme-previews"
        onBlurCapture={(event) => {
          const next = event.relatedTarget;
          if (!(next instanceof Node) || !event.currentTarget.contains(next)) setRadialOpen(null);
        }}
        onKeyDown={(event) => {
          if (event.key === 'Escape') setRadialOpen(null);
        }}
      >
        {groups.map((group) => {
          return (
            <ModePreview
              key={group.mode}
              mode={group.mode}
              options={group.options}
              selected={group.selected}
              active={selection[group.mode] === group.selected.entry.id}
              labels={labels}
              open={radialOpen === group.mode}
              onOpen={() => setRadialOpen(current => current === group.mode ? null : group.mode)}
              onChoose={(entry) => {
                onApplyMode(entry, group.mode);
                setRadialOpen(null);
              }}
            />
          );
        })}
      </div>
      <div className="settings-theme-footer">
        <button
          type="button"
          aria-label={`Use ${label} theme`}
          aria-pressed={activeNow}
          onClick={(event) => {
            event.stopPropagation();
            applyPair();
          }}
          className="settings-theme-name block min-w-0 flex-1 truncate rounded-sm text-left text-[length:var(--ui-font-label)] font-medium text-on-surface outline-none focus-visible:ring-2 focus-visible:ring-primary"
          title={label}
        >
          {activeNow && <span aria-hidden="true" className="settings-theme-check">✓</span>}{label}
        </button>
        {(onCustomize || onDuplicate || onExport || onRemove) && (
          <div className="flex shrink-0 items-center gap-0.5">
            {onCustomize && (
              <ActionButton label={customizeAsCopy ? `Create theme from ${label}` : `Edit ${label}`} onClick={event => onCustomize(event.currentTarget)}>
                {customizeAsCopy ? <CopyIcon /> : <EditIcon />}
              </ActionButton>
            )}
            {onExport && exportEntry && <ActionButton label={`Export ${label}`} onClick={() => onExport(exportEntry)}><UploadIcon /></ActionButton>}
            {onDuplicate && exportEntry && <ActionButton label={`Duplicate ${label}`} disabled={duplicateDisabled} onClick={() => onDuplicate(exportEntry)}><CopyIcon /></ActionButton>}
            {onRemove && <ActionButton label={`Remove ${label}`} onClick={onRemove} danger><TrashIcon /></ActionButton>}
          </div>
        )}
      </div>
    </article>
  );
}

export function ThemeLibrary({ onBrowse, onImport, onCustomize, createThemeRef }: Props) {
  const appearance = useAppearance();
  const selection = effectiveAppearanceSelection(
    appearance.document,
    appearance.library.document,
  );
  const [removeTarget, setRemoveTarget] = useState<{ label: string; ids: string[] } | null>(null);
  const [importMenuOpen, setImportMenuOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const duplicateEntry = async (source: ThemeLibraryEntryV1) => {
    try {
      setError(null);
      const label = `Copy of ${source.label}`.slice(0, 64).trimEnd();
      const occupiedIds = new Set(appearance.library.document.themes.map((entry) => entry.id));
      const entry = makeLocalThemeEntry(availableThemeId(label, occupiedIds), label, source.theme, source.modes);
      await appearance.library.install([entry]);
    } catch (cause) {
      setError(String(cause));
    }
  };

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
      const onlyEntry = light ?? dark;
      if (!light || !dark) {
        if (onlyEntry) void commit(appearance.library.previewSelection(onlyEntry.id));
        return;
      }
      void commit(previewThemeLibraryPairSelection(
        appearance.document,
        appearance.library.document,
        light.id,
        dark.id,
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
  const applyCustomPair = () => void commit({
    mode: appearance.document.mode,
    theme: appearance.document.theme,
    light: appearance.document.cache.light,
    dark: appearance.document.cache.dark,
    adjustments: appearance.adjustments,
    selection: { light: 'custom', dark: 'custom' },
  });
  const applyCustomMode = (_entry: ThemeLibraryEntryV1, mode: ResolvedAppearance) => void commit({
    mode,
    theme: appearance.document.theme,
    light: appearance.document.cache.light,
    dark: appearance.document.cache.dark,
    adjustments: appearance.adjustments,
    selection: { ...selection, [mode]: 'custom' },
  });

  return (
    <section aria-labelledby="themes-heading" className="settings-theme-library">
      <div className="settings-theme-header">
        <h2 id="themes-heading" className="settings-group-heading">Themes</h2>
        <div className="flex items-center gap-2">
          <button ref={createThemeRef} type="button" onClick={event => onCustomize(event.currentTarget)} className="settings-quiet-btn inline-flex h-7 items-center gap-1 px-2.5 text-[length:var(--ui-font-caption)] text-on-surface">
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
              className="settings-quiet-btn inline-flex h-7 items-center gap-1 px-2.5 text-[length:var(--ui-font-caption)] text-on-surface"
            >
              <DownloadIcon /> Import theme
            </button>
            {importMenuOpen && (
              <div role="menu" className="dialog-popover absolute right-0 top-9 z-40 w-44 overflow-hidden p-1">
                <button type="button" role="menuitem" onClick={() => { setImportMenuOpen(false); onImport(); }} className="flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-[length:var(--ui-font-caption)] text-on-surface hover:bg-surface-container">
                  <DownloadIcon /> Import file
                </button>
                <button type="button" role="menuitem" onClick={() => { setImportMenuOpen(false); onBrowse(); }} className="flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-[length:var(--ui-font-caption)] text-on-surface hover:bg-surface-container">
                  <span aria-hidden="true" className="text-[length:var(--ui-font-label)]">⌕</span> Browse Open VSX
                </button>
              </div>
            )}
          </div>
        </div>
      </div>

      <div className="settings-card settings-appearance-themes"><div className="settings-theme-grid">
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
          resolvedAppearance={appearance.resolvedAppearance}
        />
        {BUILTIN_THEME_LIBRARY_ENTRIES.map((entry) => (
          <CollectionCard
            key={`builtin:${entry.id}`}
            label={entry.label}
            entries={[entry]}
            selection={selection}
            onApplyPair={applyPair}
            onApplyMode={applyMode}
            onExport={(target) => void exportEntry(target)}
            onDuplicate={(target) => void duplicateEntry(target)}
            duplicateDisabled={appearance.busy}
            resolvedAppearance={appearance.resolvedAppearance}
          />
        ))}
        {(selection.light === 'custom' || selection.dark === 'custom') && (
          <CollectionCard
            label="Custom"
            entries={[customEntry]}
            selection={selection}
            onApplyPair={applyCustomPair}
            onApplyMode={applyCustomMode}
            onCustomize={onCustomize}
            resolvedAppearance={appearance.resolvedAppearance}
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
            onDuplicate={(entry) => void duplicateEntry(entry)}
            duplicateDisabled={appearance.busy}
            onRemove={() => setRemoveTarget({ label: group.label, ids: group.entries.map((entry) => entry.id) })}
            resolvedAppearance={appearance.resolvedAppearance}
          />
        ))}
      </div>

      </div>

      {removeTarget && (
        <div className="dialog-backdrop fixed inset-0 z-50 flex items-center justify-center p-5 settings-theme-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) setRemoveTarget(null); }}>
          <div role="dialog" aria-modal="true" aria-labelledby="remove-theme-title" className="dialog-popover w-full max-w-sm p-5">
            <h2 id="remove-theme-title" className="text-[length:var(--ui-font-title)] font-semibold text-on-surface">Remove {removeTarget.label}?</h2>
            <p className="mt-1 text-[length:var(--ui-font-caption)] text-on-surface-variant">Every imported variant in this collection will be removed. You can import it again later.</p>
            <div className="mt-4 flex justify-end gap-2">
              <button type="button" onClick={() => setRemoveTarget(null)} className="settings-quiet-btn px-3 py-1.5 text-[length:var(--ui-font-caption)] text-on-surface">Cancel</button>
              <button type="button" onClick={() => { const target = removeTarget; setRemoveTarget(null); void appearance.library.remove(target.ids).catch((cause) => setError(String(cause))); }} className="rounded-lg border border-error bg-error/10 px-3 py-1.5 text-[length:var(--ui-font-caption)] font-medium text-error">Remove</button>
            </div>
          </div>
        </div>
      )}

      {(error || appearance.library.error) && (
        <p role="alert" className="rounded-lg border border-error bg-error/10 px-3 py-2 text-[length:var(--ui-font-caption)] text-error">{error ?? appearance.library.error}</p>
      )}
    </section>
  );
}
