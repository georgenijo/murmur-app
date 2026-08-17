import { save } from '@tauri-apps/plugin-dialog';
import { useMemo, useState } from 'react';
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

function SunIcon() {
  return (
    <svg aria-hidden="true" className="h-3 w-3" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2v2m0 16v2M4.9 4.9l1.4 1.4m11.4 11.4 1.4 1.4M2 12h2m16 0h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />
    </svg>
  );
}

function MoonIcon() {
  return (
    <svg aria-hidden="true" className="h-3 w-3" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
      <path d="M21 12.8A9 9 0 1 1 11.2 3 7 7 0 0 0 21 12.8Z" />
    </svg>
  );
}

function ThemeOrb({ tokens, mode }: { tokens: MurmurTokens; mode: ResolvedAppearance }) {
  return (
    <span
      aria-hidden="true"
      className="block h-14 w-14 shrink-0 rounded-full border-2 border-surface-container-lowest"
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

function ModeChoice({
  mode,
  entry,
  label,
  optionCount,
  selected,
  onOpen,
}: {
  mode: ResolvedAppearance;
  entry: ThemeLibraryEntryV1 | null;
  label: string;
  optionCount: number;
  selected: boolean;
  onOpen: () => void;
}) {
  const tokens = entry ? resolveTheme(entry.theme, mode).tokens : resolveTheme({ version: 1, presetId: 'sonic' }, mode).tokens;
  const modeLabel = mode === 'light' ? 'Light' : 'Dark';
  return (
    <button
      type="button"
      aria-label={`Choose ${modeLabel.toLowerCase()} variant, currently ${label}`}
      aria-pressed={selected}
      disabled={entry === null}
      onClick={onOpen}
      className={`relative flex min-w-0 flex-1 flex-col items-center rounded-xl px-2 py-1.5 outline-none focus-visible:ring-2 focus-visible:ring-primary ${entry ? 'pointer-events-auto cursor-pointer hover:bg-surface-container' : 'pointer-events-none'}`}
    >
      <span className={`relative rounded-full p-1 ${selected ? 'ring-2 ring-primary' : ''}`}>
        <ThemeOrb tokens={tokens} mode={mode} />
        <span className="absolute bottom-0 right-0 flex h-5 w-5 items-center justify-center rounded-full border border-on-surface-variant bg-surface-container-lowest text-on-surface shadow-sm">
          {mode === 'light' ? <SunIcon /> : <MoonIcon />}
        </span>
      </span>
      <span className="mt-1 flex max-w-full items-center gap-1 text-[11px] font-semibold text-on-surface">
        <span className="truncate">{modeLabel}: {entry ? label : 'Sonic fallback'}</span>
        {optionCount > 1 && (
          <span className="shrink-0 rounded-full bg-surface-container-high px-1.5 py-0.5 text-[9px] text-on-surface-variant">
            +{optionCount - 1}
          </span>
        )}
      </span>
    </button>
  );
}

function VariantPicker({
  mode,
  entries,
  selectedId,
  labels,
  onChoose,
  onClose,
}: {
  mode: ResolvedAppearance;
  entries: readonly ThemeLibraryEntryV1[];
  selectedId: string;
  labels: ReadonlyMap<string, string>;
  onChoose: (entry: ThemeLibraryEntryV1) => void;
  onClose: () => void;
}) {
  return (
    <div className="pointer-events-auto relative z-20 border-t border-outline-variant bg-surface-container-low px-3 pb-3 pt-2">
      <div className="mb-2 flex items-center justify-between gap-2">
        <p className="text-xs font-semibold text-on-surface">Choose {mode} style</p>
        <button type="button" onClick={onClose} className="rounded-md px-2 py-1 text-xs font-medium text-on-surface-variant hover:bg-surface-container hover:text-on-surface">
          Done
        </button>
      </div>
      <div className="grid gap-1.5 sm:grid-cols-2">
        {entries.map((entry) => {
          const active = entry.id === selectedId;
          return (
            <button
              type="button"
              key={entry.id}
              aria-pressed={active}
              onClick={() => onChoose(entry)}
              className={`flex min-w-0 items-center gap-2 rounded-lg border px-2.5 py-2 text-left text-xs font-medium outline-none focus-visible:ring-2 focus-visible:ring-primary ${active ? 'border-primary bg-primary/10 text-on-surface' : 'border-on-surface-variant/70 bg-surface-container-lowest text-on-surface hover:border-primary'}`}
            >
              <span className="shrink-0">{active ? '✓' : '○'}</span>
              <span className="truncate">{labels.get(entry.id) ?? entry.label}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

function CollectionCard({
  label,
  source,
  entries,
  selection,
  currentAppearance,
  onApplyPair,
  onApplyMode,
  onExport,
  onRemove,
}: {
  label: string;
  source: string;
  entries: readonly ThemeLibraryEntryV1[];
  selection: { light: string; dark: string };
  currentAppearance: ResolvedAppearance;
  onApplyPair: (light: ThemeLibraryEntryV1 | null, dark: ThemeLibraryEntryV1 | null) => void;
  onApplyMode: (entry: ThemeLibraryEntryV1, mode: ResolvedAppearance) => void;
  onExport?: (entry: ThemeLibraryEntryV1) => void;
  onRemove?: () => void;
}) {
  const [picker, setPicker] = useState<ResolvedAppearance | null>(null);
  const ids = new Set(entries.map((entry) => entry.id));
  const forMode = (mode: ResolvedAppearance) => entries.filter((entry) => entry.modes.includes(mode));
  const lightEntries = forMode('light');
  const darkEntries = forMode('dark');
  const light = lightEntries.find((entry) => entry.id === selection.light) ?? lightEntries[0] ?? null;
  const dark = darkEntries.find((entry) => entry.id === selection.dark) ?? darkEntries[0] ?? null;
  const labels = shortVariantLabels(entries);
  const activeNow = ids.has(selection[currentAppearance]);
  const lightSelected = light !== null && selection.light === light.id;
  const darkSelected = dark !== null && selection.dark === dark.id;
  const exportEntry = currentAppearance === 'light' ? light ?? dark : dark ?? light;

  return (
    <article
      data-theme-collection={label}
      className={`relative overflow-hidden rounded-xl border-2 bg-surface-container-lowest shadow-sm transition-colors ${activeNow ? 'border-primary' : 'border-on-surface-variant/70 hover:border-primary'}`}
    >
      <button
        type="button"
        aria-label={`Use ${label} theme`}
        aria-pressed={activeNow}
        onClick={() => onApplyPair(light, dark)}
        className="absolute inset-0 z-0 cursor-pointer rounded-xl outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary"
      />
      <div className="pointer-events-none relative z-10 px-3 pb-3 pt-2.5">
        <div className="flex min-h-24 items-start justify-center gap-3">
          <ModeChoice
            mode="light"
            entry={light}
            label={light ? labels.get(light.id) ?? light.label : 'Sonic'}
            optionCount={lightEntries.length}
            selected={lightSelected}
            onOpen={() => setPicker((open) => open === 'light' ? null : 'light')}
          />
          <ModeChoice
            mode="dark"
            entry={dark}
            label={dark ? labels.get(dark.id) ?? dark.label : 'Sonic'}
            optionCount={darkEntries.length}
            selected={darkSelected}
            onOpen={() => setPicker((open) => open === 'dark' ? null : 'dark')}
          />
        </div>
        <div className="mt-1 flex min-w-0 items-start gap-2">
          <div className="min-w-0 flex-1">
            <h3 className="truncate text-sm font-bold text-on-surface">{label}</h3>
            <p className="mt-0.5 truncate text-[11px] text-on-surface-variant">{source}</p>
          </div>
          <span className={`shrink-0 rounded-full px-3 py-1.5 text-xs font-bold ${activeNow ? 'bg-primary text-on-primary' : 'border border-on-surface-variant bg-surface-container-low text-on-surface'}`}>
            {activeNow ? `✓ Active theme · ${currentAppearance}` : 'Use theme'}
          </span>
        </div>
      </div>
      {(onExport || onRemove) && (
        <div className="pointer-events-none relative z-10 flex min-h-10 items-center justify-end gap-1 border-t border-outline-variant px-2">
          {onExport && exportEntry && (
            <button
              type="button"
              aria-label={`Export ${exportEntry.label}`}
              onClick={() => onExport(exportEntry)}
              className="pointer-events-auto rounded-lg border border-on-surface-variant px-2.5 py-1.5 text-xs font-semibold text-on-surface hover:border-primary hover:bg-surface-container"
            >
              Export
            </button>
          )}
          {onRemove && (
            <button
              type="button"
              aria-label={`Remove ${label}`}
              onClick={onRemove}
              className="pointer-events-auto rounded-lg px-2.5 py-1.5 text-xs font-semibold text-on-surface-variant hover:bg-error/10 hover:text-error"
            >
              Remove
            </button>
          )}
        </div>
      )}
      {picker === 'light' && lightEntries.length > 0 && (
        <VariantPicker
          mode="light"
          entries={lightEntries}
          selectedId={selection.light}
          labels={labels}
          onChoose={(entry) => { onApplyMode(entry, 'light'); setPicker(null); }}
          onClose={() => setPicker(null)}
        />
      )}
      {picker === 'dark' && darkEntries.length > 0 && (
        <VariantPicker
          mode="dark"
          entries={darkEntries}
          selectedId={selection.dark}
          labels={labels}
          onChoose={(entry) => { onApplyMode(entry, 'dark'); setPicker(null); }}
          onClose={() => setPicker(null)}
        />
      )}
    </article>
  );
}

export function ThemeLibrary({ onBrowse, onImport, onCustomize }: Props) {
  const appearance = useAppearance();
  const selection = appearanceSelection(appearance.document);
  const [removeTarget, setRemoveTarget] = useState<{ label: string; ids: string[] } | null>(null);
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

  const sonicEntry = (mode: ResolvedAppearance): ThemeLibraryEntryV1 => ({
    version: 1,
    id: 'sonic',
    label: 'Sonic',
    modes: [mode],
    theme: { version: 1, presetId: 'sonic' },
    source: { kind: 'local' },
  });
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
    <section aria-labelledby="themes-heading" className="space-y-3">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h2 id="themes-heading" className="text-sm font-semibold text-on-surface">Themes</h2>
          <p className="mt-0.5 text-xs text-on-surface-variant">One card per theme. Click anywhere on a card to use its light and dark styles.</p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button type="button" onClick={onCustomize} className="rounded-lg border border-on-surface-variant bg-surface-container-lowest px-3 py-1.5 text-xs font-semibold text-on-surface hover:border-primary hover:bg-surface-container">
            + Create theme
          </button>
          <button type="button" onClick={onImport} className="rounded-lg border border-on-surface-variant bg-surface-container-lowest px-3 py-1.5 text-xs font-semibold text-on-surface hover:border-primary hover:bg-surface-container">
            ↓ Import theme
          </button>
          <button type="button" onClick={onBrowse} className="rounded-lg bg-primary px-3 py-1.5 text-xs font-semibold text-on-primary hover:bg-primary-dim">
            Browse community
          </button>
        </div>
      </div>

      <div className="grid gap-3 sm:grid-cols-2">
        <CollectionCard
          label="Sonic"
          source="Built into Murmur"
          entries={[sonicEntry('light'), sonicEntry('dark')]}
          selection={selection}
          currentAppearance={appearance.resolvedAppearance}
          onApplyPair={() => {
            try { void commit(appearance.library.previewSelection('sonic')); }
            catch (cause) { setError(String(cause)); }
          }}
          onApplyMode={(_entry, mode) => {
            try { void commit(appearance.library.previewSelection('sonic', mode)); }
            catch (cause) { setError(String(cause)); }
          }}
        />
        {(selection.light === 'custom' || selection.dark === 'custom') && (
          <CollectionCard
            label="Custom"
            source="Your current color edits"
            entries={[customEntry]}
            selection={selection}
            currentAppearance={appearance.resolvedAppearance}
            onApplyPair={applyCustom}
            onApplyMode={applyCustom}
          />
        )}
        {groups.map(([key, group]) => {
          const first = group.entries[0]!;
          const source = first.source.kind === 'open-vsx'
            ? `${first.source.extensionId} · ${first.source.license}`
            : 'Saved on this Mac';
          return (
            <CollectionCard
              key={key}
              label={group.label}
              source={source}
              entries={group.entries}
              selection={selection}
              currentAppearance={appearance.resolvedAppearance}
              onApplyPair={applyPair}
              onApplyMode={applyMode}
              onExport={(entry) => void exportEntry(entry)}
              onRemove={() => setRemoveTarget({ label: group.label, ids: group.entries.map((entry) => entry.id) })}
            />
          );
        })}
      </div>

      {groups.length === 0 && (
        <p className="rounded-xl border border-dashed border-on-surface-variant bg-surface-container-lowest px-4 py-5 text-center text-xs text-on-surface-variant">
          Import a file or browse Open VSX to add community themes.
        </p>
      )}

      {removeTarget && (
        <div className="rounded-xl border border-error bg-error/10 p-3">
          <p className="text-sm font-semibold text-on-surface">Remove {removeTarget.label}?</p>
          <p className="mt-1 text-xs text-on-surface">Every imported variant in this collection will be removed. Any affected appearance falls back to Sonic.</p>
          <div className="mt-3 flex gap-2">
            <button
              type="button"
              onClick={() => {
                const target = removeTarget;
                setRemoveTarget(null);
                void appearance.library.remove(target.ids).catch((cause) => setError(String(cause)));
              }}
              className="rounded-lg border border-error bg-surface-container-lowest px-3 py-1.5 text-xs font-semibold text-error hover:bg-error/10"
            >
              Remove collection
            </button>
            <button type="button" onClick={() => setRemoveTarget(null)} className="rounded-lg border border-on-surface-variant px-3 py-1.5 text-xs font-semibold text-on-surface hover:bg-surface-container">
              Cancel
            </button>
          </div>
        </div>
      )}

      {(error || appearance.library.error) && (
        <p role="alert" className="rounded-lg border border-error bg-error/10 px-3 py-2 text-xs text-error">{error ?? appearance.library.error}</p>
      )}
    </section>
  );
}
