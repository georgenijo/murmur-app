import { save } from '@tauri-apps/plugin-dialog';
import { useMemo, useState } from 'react';
import {
  appearanceSelection,
  resolveTheme,
  type MurmurTokens,
  type ResolvedAppearance,
  type ThemeLibraryEntryV1,
} from '../../lib/appearance';
import { useAppearance } from '../../lib/hooks/useAppearance';

interface Props {
  onBrowse: () => void;
  onImport: () => void;
}

function PalettePreview({ tokens, label }: { tokens: MurmurTokens; label: string }) {
  return (
    <span
      role="img"
      aria-label={label}
      className="relative block h-16 min-w-0 flex-1 overflow-hidden rounded-xl border border-on-surface-variant bg-background shadow-sm"
      style={{ background: tokens.background }}
    >
      <span
        className="absolute bottom-2 left-2 top-2 w-5 rounded-md"
        style={{ background: tokens['surface-container-high'] }}
      />
      <span
        className="absolute left-9 right-2 top-3 h-2 rounded-full"
        style={{ background: tokens['on-surface'] }}
      />
      <span
        className="absolute left-9 right-5 top-7 h-1.5 rounded-full"
        style={{ background: tokens['on-surface-variant'] }}
      />
      <span
        className="absolute bottom-3 right-2 h-3 w-7 rounded-full"
        style={{ background: tokens.primary }}
      />
    </span>
  );
}

function ThemeCard({
  label,
  source,
  palettes,
  activeModes,
  onApply,
  onExport,
}: {
  label: string;
  source: string;
  palettes: readonly { mode: ResolvedAppearance; tokens: MurmurTokens }[];
  activeModes: readonly ResolvedAppearance[];
  onApply: () => void;
  onExport?: () => void;
}) {
  const active = palettes.every(({ mode }) => activeModes.includes(mode));
  const partiallyActive = !active && activeModes.length > 0;

  return (
    <article
      className={`overflow-hidden rounded-xl border bg-surface-container-lowest transition-colors ${
        active
          ? 'border-primary ring-1 ring-primary'
          : 'border-on-surface-variant hover:border-primary'
      }`}
    >
      <button
        type="button"
        aria-label={`Use ${label} theme`}
        aria-pressed={active}
        onClick={onApply}
        className="block w-full p-3 text-left outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary"
      >
        <span className="flex gap-2">
          {palettes.map(({ mode, tokens }) => (
            <PalettePreview key={mode} tokens={tokens} label={`${label} ${mode} palette`} />
          ))}
        </span>
        <span className="mt-2 flex min-w-0 items-start justify-between gap-2">
          <span className="min-w-0">
            <span className="block truncate text-sm font-semibold text-on-surface">{label}</span>
            <span className="mt-0.5 block truncate text-[11px] text-on-surface-variant">{source}</span>
          </span>
          <span className={`shrink-0 rounded-full px-2 py-0.5 text-[10px] font-semibold ${active ? 'bg-primary text-on-primary' : 'bg-surface-container-high text-on-surface'}`}>
            {active ? 'Active' : partiallyActive ? 'Partly active' : 'Apply'}
          </span>
        </span>
      </button>
      {onExport && (
        <div className="border-t border-outline-variant px-3 py-1.5 text-right">
          <button
            type="button"
            aria-label={`Export ${label}`}
            onClick={onExport}
            className="rounded-md px-2 py-1 text-[11px] font-medium text-on-surface-variant hover:bg-surface-container hover:text-on-surface"
          >
            Export
          </button>
        </div>
      )}
    </article>
  );
}

export function ThemeLibrary({ onBrowse, onImport }: Props) {
  const appearance = useAppearance();
  const selection = appearanceSelection(appearance.document);
  const [removeTarget, setRemoveTarget] = useState<{
    label: string;
    ids: string[];
  } | null>(null);
  const [error, setError] = useState<string | null>(null);

  const groups = useMemo(() => {
    const grouped = new Map<string, { label: string; entries: ThemeLibraryEntryV1[] }>();
    for (const entry of appearance.library.document.themes) {
      const key = entry.collection ? `collection:${entry.collection.id}` : `theme:${entry.id}`;
      const existing = grouped.get(key);
      if (existing) existing.entries.push(entry);
      else grouped.set(key, {
        label: entry.collection?.label ?? entry.label,
        entries: [entry],
      });
    }
    return [...grouped.entries()];
  }, [appearance.library.document.themes]);

  const applyTheme = async (id: string) => {
    try {
      setError(null);
      const preview = appearance.library.previewSelection(id);
      await appearance.commitImport(preview);
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

  const sonicLight = appearance.document.theme.presetId === 'sonic'
    ? appearance.document.cache.light
    : resolveTheme({ version: 1, presetId: 'sonic' }, 'light').tokens;
  const sonicDark = appearance.document.theme.presetId === 'sonic'
    ? appearance.document.cache.dark
    : resolveTheme({ version: 1, presetId: 'sonic' }, 'dark').tokens;

  return (
    <section aria-labelledby="themes-heading" className="space-y-3">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h2 id="themes-heading" className="text-sm font-semibold text-on-surface">Themes</h2>
          <p className="mt-0.5 text-xs text-on-surface-variant">
            Click any theme to apply it immediately.
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            onClick={onImport}
            className="rounded-lg border border-on-surface-variant bg-surface-container-lowest px-3 py-1.5 text-xs font-semibold text-on-surface hover:border-primary hover:bg-surface-container"
          >
            Import theme
          </button>
          <button
            type="button"
            onClick={onBrowse}
            className="rounded-lg bg-primary px-3 py-1.5 text-xs font-semibold text-on-primary hover:bg-primary-dim"
          >
            Browse community
          </button>
        </div>
      </div>

      <div className="grid gap-3 sm:grid-cols-2">
        <ThemeCard
          label="Sonic"
          source="Built in · Light + Dark"
          palettes={[
            { mode: 'light', tokens: sonicLight },
            { mode: 'dark', tokens: sonicDark },
          ]}
          activeModes={[
            ...(selection.light === 'sonic' ? ['light' as const] : []),
            ...(selection.dark === 'sonic' ? ['dark' as const] : []),
          ]}
          onApply={() => void applyTheme('sonic')}
        />
      </div>

      {groups.length > 0 && (
        <div className="space-y-4">
          {groups.map(([key, group]) => (
            <section key={key} aria-label={group.label}>
              <div className="mb-2 flex items-center justify-between gap-3">
                <div className="min-w-0">
                  <h3 className="truncate text-xs font-semibold text-on-surface">{group.label}</h3>
                  {group.entries.length > 1 && (
                    <p className="text-[11px] text-on-surface-variant">{group.entries.length} variants</p>
                  )}
                </div>
                <button
                  type="button"
                  onClick={() => setRemoveTarget({ label: group.label, ids: group.entries.map((entry) => entry.id) })}
                  className="rounded-md px-2 py-1 text-[11px] font-medium text-on-surface-variant hover:bg-error/10 hover:text-error"
                >
                  Remove
                </button>
              </div>
              <div className="grid gap-3 sm:grid-cols-2">
                {group.entries.map((entry) => {
                  const palettes = entry.modes.map((mode) => ({
                    mode,
                    tokens: resolveTheme(entry.theme, mode).tokens,
                  }));
                  const activeModes = entry.modes.filter((mode) => selection[mode] === entry.id);
                  return (
                    <ThemeCard
                      key={entry.id}
                      label={entry.label}
                      source={entry.source.kind === 'open-vsx'
                        ? `${entry.source.extensionId} · ${entry.source.license}`
                        : entry.modes.map((mode) => mode[0].toUpperCase() + mode.slice(1)).join(' + ')}
                      palettes={palettes}
                      activeModes={activeModes}
                      onApply={() => void applyTheme(entry.id)}
                      onExport={() => void exportEntry(entry)}
                    />
                  );
                })}
              </div>
            </section>
          ))}
        </div>
      )}

      {groups.length === 0 && (
        <div className="rounded-xl border border-dashed border-on-surface-variant bg-surface-container-lowest px-4 py-6 text-center">
          <p className="text-sm font-medium text-on-surface">No community themes installed</p>
          <p className="mt-1 text-xs text-on-surface-variant">Import a file or browse Open VSX to add one.</p>
        </div>
      )}

      {removeTarget && (
        <div className="rounded-xl border border-error bg-error/10 p-3">
          <p className="text-sm font-medium text-on-surface">Remove {removeTarget.label}?</p>
          <p className="mt-1 text-xs text-on-surface">Any active light or dark variant will fall back to Sonic.</p>
          <div className="mt-3 flex gap-2">
            <button
              type="button"
              onClick={() => {
                const target = removeTarget;
                setRemoveTarget(null);
                void appearance.library.remove(target.ids).catch((cause) => setError(String(cause)));
              }}
              className="rounded-lg border border-error bg-surface-container-lowest px-3 py-1.5 text-xs font-medium text-error hover:bg-error/10"
            >
              Remove
            </button>
            <button type="button" onClick={() => setRemoveTarget(null)} className="rounded-lg border border-on-surface-variant px-3 py-1.5 text-xs text-on-surface hover:bg-surface-container">
              Cancel
            </button>
          </div>
        </div>
      )}

      {(error || appearance.library.error) && (
        <p role="alert" className="rounded-lg border border-error bg-error/10 px-3 py-2 text-xs text-error">
          {error ?? appearance.library.error}
        </p>
      )}
    </section>
  );
}
