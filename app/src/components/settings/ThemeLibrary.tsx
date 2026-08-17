import { save } from '@tauri-apps/plugin-dialog';
import { useMemo, useState } from 'react';
import {
  appearanceSelection,
  resolveTheme,
  type MurmurTokens,
  type ResolvedAppearance,
  type ThemeImportPreview,
  type ThemeLibraryEntryV1,
} from '../../lib/appearance';
import { useAppearance } from '../../lib/hooks/useAppearance';

interface Props {
  onBrowse: () => void;
  onPreview: (preview: ThemeImportPreview, sourceLabel: string) => void;
}

function PalettePreview({ tokens, label }: { tokens: MurmurTokens; label: string }) {
  return (
    <span
      role="img"
      aria-label={label}
      className="relative block h-12 w-12 overflow-hidden rounded-full border border-outline-variant/30 shadow-sm"
      style={{ background: tokens.background }}
    >
      <span
        className="absolute inset-x-1.5 bottom-1.5 top-5 rounded-md"
        style={{ background: tokens['surface-container-high'] }}
      />
      <span
        className="absolute bottom-2 left-2 h-2.5 w-5 rounded-full"
        style={{ background: tokens.primary }}
      />
      <span
        className="absolute right-2 top-2 h-1.5 w-5 rounded-full"
        style={{ background: tokens['on-surface'] }}
      />
    </span>
  );
}

function ModeButton({
  mode,
  active,
  tokens,
  label,
  onClick,
}: {
  mode: ResolvedAppearance;
  active: boolean;
  tokens: MurmurTokens;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={`Use ${label} for ${mode} appearance`}
      aria-pressed={active}
      onClick={onClick}
      className={`relative rounded-full p-0.5 outline-none transition-transform hover:scale-105 focus-visible:ring-2 focus-visible:ring-primary ${active ? 'ring-2 ring-primary' : ''}`}
    >
      <PalettePreview tokens={tokens} label={`${label} ${mode} palette`} />
      <span className="absolute -bottom-0.5 -right-0.5 flex h-4 min-w-4 items-center justify-center rounded-full border border-outline-variant/30 bg-surface-container-lowest px-1 text-[9px] font-medium text-on-surface">
        {mode === 'light' ? '☀' : '☾'}
      </span>
    </button>
  );
}

function ThemeVariant({
  entry,
  activeLight,
  activeDark,
  onPreview,
  onExport,
}: {
  entry: ThemeLibraryEntryV1;
  activeLight: boolean;
  activeDark: boolean;
  onPreview: (entry: ThemeLibraryEntryV1, mode?: ResolvedAppearance) => void;
  onExport: (entry: ThemeLibraryEntryV1) => void;
}) {
  const light = entry.modes.includes('light') ? resolveTheme(entry.theme, 'light').tokens : null;
  const dark = entry.modes.includes('dark') ? resolveTheme(entry.theme, 'dark').tokens : null;
  return (
    <div className="flex items-center gap-3 rounded-lg border border-outline-variant/20 bg-surface-container-lowest p-2.5">
      <div className="flex shrink-0 gap-2">
        {light && (
          <ModeButton
            mode="light"
            active={activeLight}
            tokens={light}
            label={entry.label}
            onClick={() => onPreview(entry, 'light')}
          />
        )}
        {dark && (
          <ModeButton
            mode="dark"
            active={activeDark}
            tokens={dark}
            label={entry.label}
            onClick={() => onPreview(entry, 'dark')}
          />
        )}
      </div>
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium text-on-surface">{entry.label}</p>
        <p className="mt-0.5 truncate text-[11px] text-on-surface-variant">
          {entry.source.kind === 'open-vsx'
            ? `${entry.source.extensionId} · ${entry.source.license}`
            : 'Saved on this Mac'}
        </p>
      </div>
      <div className="flex shrink-0 gap-1">
        {entry.modes.length === 2 && (
          <button
            type="button"
            onClick={() => onPreview(entry)}
            className="rounded-md px-2 py-1 text-[11px] font-medium text-on-surface hover:bg-surface-container"
          >
            Use both
          </button>
        )}
        <button
          type="button"
          aria-label={`Export ${entry.label}`}
          onClick={() => onExport(entry)}
          className="rounded-md px-2 py-1 text-[11px] text-on-surface-variant hover:bg-surface-container hover:text-on-surface"
        >
          Export
        </button>
      </div>
    </div>
  );
}

export function ThemeLibrary({ onBrowse, onPreview }: Props) {
  const appearance = useAppearance();
  const selection = appearanceSelection(appearance.document);
  const [saveName, setSaveName] = useState('Custom theme');
  const [showSave, setShowSave] = useState(false);
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

  const previewInstalled = (entry: ThemeLibraryEntryV1, mode?: ResolvedAppearance) => {
    try {
      setError(null);
      onPreview(appearance.library.previewSelection(entry.id, mode), entry.label);
    } catch (cause) {
      setError(String(cause));
    }
  };

  const previewSonic = (mode?: ResolvedAppearance) => {
    try {
      setError(null);
      onPreview(appearance.library.previewSelection('sonic', mode), 'Sonic');
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

  const saveCurrent = async () => {
    const label = saveName.trim();
    if (!label) {
      setError('Enter a name for the theme.');
      return;
    }
    try {
      setError(null);
      await appearance.library.saveCurrent(label);
      setShowSave(false);
      setSaveName('Custom theme');
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
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <p className="text-sm font-medium text-on-surface">Theme library</p>
          <p className="mt-0.5 text-xs text-on-surface-variant">
            Pick each appearance independently or use a paired theme for both.
          </p>
        </div>
        <div className="flex gap-2">
          <button
            type="button"
            onClick={() => setShowSave(true)}
            className="rounded-lg border border-outline-variant/30 px-3 py-1.5 text-xs font-medium text-on-surface hover:bg-surface-container"
          >
            Save current
          </button>
          <button
            type="button"
            onClick={onBrowse}
            className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-on-primary hover:bg-primary-dim"
          >
            Browse community
          </button>
        </div>
      </div>

      {showSave && (
        <div className="flex flex-wrap items-center gap-2 rounded-xl border border-primary/30 bg-primary/5 p-3">
          <label className="min-w-0 flex-1 text-xs font-medium text-on-surface">
            Theme name
            <input
              autoFocus
              value={saveName}
              maxLength={64}
              onChange={(event) => setSaveName(event.currentTarget.value)}
              onKeyDown={(event) => { if (event.key === 'Enter') void saveCurrent(); }}
              className="mt-1 w-full rounded-lg border border-on-surface-variant bg-surface-container-lowest px-3 py-1.5 text-sm text-on-surface outline-none focus:border-primary"
            />
          </label>
          <button type="button" onClick={() => void saveCurrent()} className="mt-4 rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-on-primary">
            Save
          </button>
          <button type="button" onClick={() => setShowSave(false)} className="mt-4 rounded-lg px-3 py-1.5 text-xs text-on-surface hover:bg-surface-container">
            Cancel
          </button>
        </div>
      )}

      <div className="rounded-xl border border-outline-variant/30 bg-surface-container-low p-3">
        <div className="flex items-center gap-3">
          <div className="flex gap-2">
            <ModeButton mode="light" active={selection.light === 'sonic'} tokens={sonicLight} label="Sonic" onClick={() => previewSonic('light')} />
            <ModeButton mode="dark" active={selection.dark === 'sonic'} tokens={sonicDark} label="Sonic" onClick={() => previewSonic('dark')} />
          </div>
          <div className="min-w-0 flex-1">
            <p className="text-sm font-semibold text-on-surface">Sonic</p>
            <p className="mt-0.5 text-xs text-on-surface-variant">Murmur’s built-in accessible palette.</p>
          </div>
          <button type="button" onClick={() => previewSonic()} className="rounded-lg px-2.5 py-1.5 text-xs font-medium text-on-surface hover:bg-surface-container">
            Use both
          </button>
        </div>
      </div>

      {groups.length === 0 ? (
        <div className="rounded-xl border border-dashed border-outline-variant/40 px-4 py-6 text-center">
          <p className="text-sm font-medium text-on-surface">No saved themes yet</p>
          <p className="mt-1 text-xs text-on-surface-variant">Save your current colors, import a file, or browse Open VSX.</p>
        </div>
      ) : (
        <div className="space-y-2">
          {groups.map(([key, group]) => (
            <section key={key} className="rounded-xl border border-outline-variant/30 bg-surface-container-low p-3">
              <div className="mb-2 flex items-center justify-between gap-3">
                <div className="min-w-0">
                  <h3 className="truncate text-sm font-semibold text-on-surface">{group.label}</h3>
                  {group.entries.length > 1 && (
                    <p className="text-[11px] text-on-surface-variant">{group.entries.length} variants</p>
                  )}
                </div>
                <button
                  type="button"
                  onClick={() => setRemoveTarget({ label: group.label, ids: group.entries.map((entry) => entry.id) })}
                  className="rounded-md px-2 py-1 text-[11px] text-on-surface-variant hover:bg-error/10 hover:text-error"
                >
                  Remove
                </button>
              </div>
              <div className="space-y-2">
                {group.entries.map((entry) => (
                  <ThemeVariant
                    key={entry.id}
                    entry={entry}
                    activeLight={selection.light === entry.id}
                    activeDark={selection.dark === entry.id}
                    onPreview={previewInstalled}
                    onExport={(theme) => void exportEntry(theme)}
                  />
                ))}
              </div>
            </section>
          ))}
        </div>
      )}

      {removeTarget && (
        <div className="rounded-xl border border-error/30 bg-error/10 p-3">
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
              className="rounded-lg border border-error/40 bg-surface-container-lowest px-3 py-1.5 text-xs font-medium text-error hover:bg-error/10"
            >
              Remove
            </button>
            <button type="button" onClick={() => setRemoveTarget(null)} className="rounded-lg px-3 py-1.5 text-xs text-on-surface hover:bg-surface-container">
              Cancel
            </button>
          </div>
        </div>
      )}

      {(error || appearance.library.error) && (
        <p role="alert" className="rounded-lg border border-error/30 bg-error/10 px-3 py-2 text-xs text-error">
          {error ?? appearance.library.error}
        </p>
      )}
    </div>
  );
}
