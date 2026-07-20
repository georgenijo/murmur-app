import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import {
  type AppProfile,
  type WritingStyle,
  type WritingStyleChoice,
  WRITING_STYLE_OPTIONS,
} from '../../lib/settings';
import { Select } from '../ui/Select';

export interface RunningApplication {
  bundleId: string;
  name: string;
}

type OverrideChoice = 'inherit' | 'always' | 'never';

interface IdeContextStatus {
  state: 'disabled' | 'empty' | 'scanning' | 'ready' | 'stale' | 'cleared' | 'error';
  generation: number;
  roots: number;
  files: number;
  symbols: number;
  bytes: number;
  capped: boolean;
  ms: number;
}

const OVERRIDE_OPTIONS: { value: OverrideChoice; label: string }[] = [
  { value: 'inherit', label: 'Use global setting' },
  { value: 'always', label: 'Always' },
  { value: 'never', label: 'Never' },
];

const WRITING_STYLE_SUMMARIES: Record<WritingStyleChoice, string> = {
  inherit: 'Uses your global behavior and the explicit overrides below.',
  conversational: 'Removes filler and repeated words, tidies capitalization, and keeps your wording.',
  polished: 'Cleans speech and applies explicitly spoken lists, punctuation, symbols, and corrections.',
  code_technical: 'Preserves technical wording and enables deterministic command formatting.',
  verbatim: 'Leaves recognized text unchanged, including filler, spacing, and spoken command words.',
  notes: 'Removes filler, keeps note-like capitalization, and turns explicit list, paragraph, and line cues into structure.',
};

const WRITING_STYLE_CATEGORIES: Record<WritingStyleChoice, string> = {
  inherit: 'Cleanup, corrections, structured writing, and command formatting all inherit.',
  conversational: 'Cleanup on · Structured writing off · Automatic command formatting off',
  polished: 'Cleanup on · Preferred spellings on · Structured writing on · Automatic command formatting off',
  code_technical: 'Cleanup off · Preferred spellings on · Structured writing off · Command formatting on',
  verbatim: 'Cleanup, corrections, structured writing, and command formatting all off',
  notes: 'Cleanup on · Preferred spellings on · Structured writing on · Automatic command formatting off',
};

export function overrideChoice(value: boolean | null): OverrideChoice {
  return value === null ? 'inherit' : value ? 'always' : 'never';
}

export function overrideValue(choice: OverrideChoice): boolean | null {
  return choice === 'inherit' ? null : choice === 'always';
}

function ideStatusText(status: IdeContextStatus | undefined, roots: number): string {
  if (!status) return roots > 0 ? 'Checking the memory-only index…' : 'Add a project root to build an index.';
  switch (status.state) {
    case 'scanning': return `Indexing locally… generation ${status.generation}`;
    case 'ready': return `Ready · ${status.files} files · ${status.symbols} symbols${status.capped ? ' · capped' : ''}`;
    case 'stale': return 'Expired safely · refresh to use project symbols again.';
    case 'cleared': return 'Index cleared from memory. Configured roots are still available.';
    case 'error': return 'Index unavailable. Check the configured roots and refresh.';
    case 'empty': return 'Add a project root to build an index.';
    default: return 'Local project context is off.';
  }
}

function newProfile(bundleId: string, label: string): AppProfile {
  return {
    bundleId,
    label,
    autoPasteOverride: null,
    cleanupOverride: null,
    smartFormattingOverride: null,
    cliFormattingOverride: null,
    writingStyle: null,
    ideContextEnabled: false,
    ideProjectRoots: [],
  };
}

function OverrideSelect({
  label,
  appLabel,
  value,
  onChange,
}: {
  label: string;
  appLabel: string;
  value: boolean | null;
  onChange: (next: boolean | null) => void;
}) {
  return (
    <label className="block text-xs font-medium text-on-surface">
      {label}
      <select
        aria-label={`${label} for ${appLabel}`}
        value={overrideChoice(value)}
        onChange={(event) => onChange(overrideValue(event.target.value as OverrideChoice))}
        className="mt-1 w-full rounded-lg border border-outline-variant/30 bg-surface-container-lowest px-2.5 py-2 text-xs text-on-surface outline-none focus:border-primary focus:ring-1 focus:ring-primary"
      >
        {OVERRIDE_OPTIONS.map((option) => (
          <option key={option.value} value={option.value}>{option.label}</option>
        ))}
      </select>
    </label>
  );
}

export function AppOverridesEditor({ profiles, onChange }: {
  profiles: AppProfile[];
  onChange: (next: AppProfile[]) => void;
}) {
  const [runningApps, setRunningApps] = useState<RunningApplication[]>([]);
  const [selectedApp, setSelectedApp] = useState('');
  const [bundleId, setBundleId] = useState('');
  const [label, setLabel] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [ideStatuses, setIdeStatuses] = useState<Record<string, IdeContextStatus>>({});

  useEffect(() => {
    let disposed = false;
    invoke<RunningApplication[]>('list_running_applications')
      .then((apps) => { if (!disposed) setRunningApps(apps); })
      .catch(() => { if (!disposed) setRunningApps([]); });
    return () => { disposed = true; };
  }, []);

  const availableApps = useMemo(() => {
    const configured = new Set(profiles.map((profile) => profile.bundleId.toLocaleLowerCase()));
    return runningApps.filter((app) => !configured.has(app.bundleId.toLocaleLowerCase()));
  }, [profiles, runningApps]);

  useEffect(() => {
    if (selectedApp && !availableApps.some((app) => app.bundleId === selectedApp)) {
      setSelectedApp('');
    }
  }, [availableApps, selectedApp]);

  const pollIdeStatuses = useCallback(async () => {
    const enabled = profiles.filter((profile) => profile.ideContextEnabled);
    if (enabled.length === 0) return;
    const pairs = await Promise.all(enabled.map(async (profile) => {
      try {
        const status = await invoke<IdeContextStatus>('get_ide_context_status', { bundleId: profile.bundleId });
        return [profile.bundleId, status] as const;
      } catch {
        return null;
      }
    }));
    setIdeStatuses((current) => ({
      ...current,
      ...Object.fromEntries(pairs.filter((pair): pair is readonly [string, IdeContextStatus] => pair !== null)),
    }));
  }, [profiles]);

  useEffect(() => {
    void pollIdeStatuses();
    const timer = window.setInterval(() => void pollIdeStatuses(), 1000);
    return () => window.clearInterval(timer);
  }, [pollIdeStatuses]);

  const addProfile = (nextBundleId: string, nextLabel: string) => {
    const trimmedId = nextBundleId.trim();
    if (!trimmedId) return;
    if (profiles.some((profile) => profile.bundleId.toLocaleLowerCase() === trimmedId.toLocaleLowerCase())) {
      setError(`An override already exists for ${trimmedId}.`);
      return;
    }
    onChange([...profiles, newProfile(trimmedId, nextLabel.trim())]);
    setSelectedApp('');
    setBundleId('');
    setLabel('');
    setError(null);
  };

  const updateProfile = (id: string, update: Partial<AppProfile>) => {
    onChange(profiles.map((profile) => profile.bundleId === id ? { ...profile, ...update } : profile));
  };

  const addIdeRoot = async (id: string) => {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (typeof selected !== 'string') return;
      onChange(profiles.map((profile) => {
        if (profile.bundleId !== id || profile.ideProjectRoots.includes(selected) || profile.ideProjectRoots.length >= 4) return profile;
        return { ...profile, ideProjectRoots: [...profile.ideProjectRoots, selected] };
      }));
    } catch {
      // Dialog cancelled or unavailable — keep the configured roots.
    }
  };

  const refreshIdeIndex = async (id: string) => {
    try {
      const status = await invoke<IdeContextStatus>('refresh_ide_context', { bundleId: id });
      setIdeStatuses((current) => ({ ...current, [id]: status }));
    } catch {
      // Polling will surface the stable failure state.
    }
  };

  const clearIdeIndex = async (id: string) => {
    try {
      const status = await invoke<IdeContextStatus>('clear_ide_context', { bundleId: id });
      setIdeStatuses((current) => ({ ...current, [id]: status }));
    } catch {
      // Keep configured roots intact if the command is unavailable.
    }
  };

  return (
    <div className="space-y-4">
      <div className="rounded-xl border border-outline-variant/30 bg-surface-container-lowest p-3">
        <h2 className="text-sm font-medium text-on-surface">Add an app</h2>
        <p className="mt-1 text-xs text-on-surface-variant">
          Murmur reads only the names and bundle IDs of currently running apps for this picker. The list stays in memory, is never logged, and is not saved unless you choose an app.
        </p>
        {availableApps.length > 0 ? (
          <div className="mt-3 flex gap-2">
            <select
              aria-label="Running app"
              value={selectedApp}
              onChange={(event) => setSelectedApp(event.target.value)}
              className="min-w-0 flex-1 rounded-lg border border-outline-variant/30 bg-surface-container-lowest px-3 py-2 text-xs text-on-surface outline-none focus:border-primary focus:ring-1 focus:ring-primary"
            >
              <option value="">Choose a running app…</option>
              {availableApps.map((app) => (
                <option key={app.bundleId} value={app.bundleId}>{app.name} — {app.bundleId}</option>
              ))}
            </select>
            <button
              type="button"
              disabled={!selectedApp}
              onClick={() => {
                const app = availableApps.find((candidate) => candidate.bundleId === selectedApp);
                if (app) addProfile(app.bundleId, app.name);
              }}
              className="shrink-0 rounded-lg bg-primary px-3 py-2 text-xs font-medium text-on-primary disabled:cursor-not-allowed disabled:opacity-50"
            >
              Add app
            </button>
          </div>
        ) : (
          <p className="mt-3 text-xs text-on-surface-variant">No unconfigured running apps are available. Use the advanced entry below.</p>
        )}

        <details className="mt-3">
          <summary className="cursor-pointer text-xs font-medium text-on-surface-variant hover:text-primary">Advanced: enter a bundle ID</summary>
          <div className="mt-2 space-y-2">
            <input
              type="text"
              value={label}
              onChange={(event) => setLabel(event.target.value)}
              placeholder="App name (optional)"
              autoComplete="off"
              spellCheck={false}
              className="w-full rounded-lg border border-outline-variant/30 bg-surface-container-lowest px-3 py-2 text-xs text-on-surface placeholder:text-on-surface-variant focus:outline-none focus:ring-2 focus:ring-primary"
            />
            <div className="flex gap-2">
              <input
                type="text"
                value={bundleId}
                onChange={(event) => setBundleId(event.target.value)}
                onKeyDown={(event) => { if (event.key === 'Enter') addProfile(bundleId, label); }}
                placeholder="com.apple.Terminal"
                autoComplete="off"
                autoCorrect="off"
                autoCapitalize="off"
                spellCheck={false}
                className="min-w-0 flex-1 rounded-lg border border-outline-variant/30 bg-surface-container-lowest px-3 py-2 font-mono text-xs text-on-surface placeholder:text-on-surface-variant focus:outline-none focus:ring-2 focus:ring-primary"
              />
              <button
                type="button"
                onClick={() => addProfile(bundleId, label)}
                disabled={!bundleId.trim()}
                className="shrink-0 rounded-lg border border-outline-variant/30 bg-surface-container-lowest px-3 py-2 text-xs font-medium text-on-surface-variant hover:bg-surface-container hover:text-primary disabled:cursor-not-allowed disabled:opacity-50"
              >
                Add
              </button>
            </div>
          </div>
        </details>
        {error && <p role="alert" className="mt-2 text-xs text-error">{error}</p>}
      </div>

      {profiles.length === 0 ? (
        <p className="text-xs text-on-surface-variant">No app overrides configured. Global settings apply everywhere.</p>
      ) : (
        <ul className="space-y-3">
          {profiles.map((profile) => {
            const appLabel = profile.label || profile.bundleId;
            return (
              <li key={profile.bundleId} className="space-y-3 rounded-xl border border-outline-variant/25 bg-surface-container-lowest p-3 shadow-sm">
                <div className="flex items-start gap-2">
                  <div className="min-w-0 flex-1">
                    <h3 className="truncate text-sm font-medium text-on-surface">{appLabel}</h3>
                    <p className="truncate font-mono text-[11px] text-on-surface-variant">{profile.bundleId}</p>
                  </div>
                  <button
                    type="button"
                    onClick={() => onChange(profiles.filter((candidate) => candidate.bundleId !== profile.bundleId))}
                    aria-label={`Remove override for ${appLabel}`}
                    className="rounded-md px-2 py-1 text-xs text-on-surface-variant hover:bg-surface-container hover:text-error focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
                  >
                    Remove
                  </button>
                </div>

                <div>
                  <label className="mb-1 block text-xs font-medium text-on-surface">Writing style</label>
                  <Select
                    value={profile.writingStyle ?? 'inherit'}
                    onChange={(choice) => updateProfile(profile.bundleId, { writingStyle: choice === 'inherit' ? null : choice as WritingStyle })}
                    items={WRITING_STYLE_OPTIONS}
                    aria-label={`Writing style for ${appLabel}`}
                  />
                  <p className="mt-1 text-xs text-on-surface-variant">{WRITING_STYLE_SUMMARIES[profile.writingStyle ?? 'inherit']}</p>
                  <p className="mt-1 text-xs font-medium text-on-surface-variant">{WRITING_STYLE_CATEGORIES[profile.writingStyle ?? 'inherit']}</p>
                </div>

                <div className="grid grid-cols-2 gap-2">
                  <OverrideSelect label="Auto-paste" appLabel={appLabel} value={profile.autoPasteOverride} onChange={(value) => updateProfile(profile.bundleId, { autoPasteOverride: value })} />
                  <OverrideSelect label="Transcript cleanup" appLabel={appLabel} value={profile.cleanupOverride} onChange={(value) => updateProfile(profile.bundleId, { cleanupOverride: value })} />
                  <OverrideSelect label="Structured writing" appLabel={appLabel} value={profile.smartFormattingOverride} onChange={(value) => updateProfile(profile.bundleId, { smartFormattingOverride: value })} />
                  <OverrideSelect label="Command formatting" appLabel={appLabel} value={profile.cliFormattingOverride} onChange={(value) => updateProfile(profile.bundleId, { cliFormattingOverride: value })} />
                </div>

                <div className="rounded-lg border border-outline-variant/30 bg-surface-container-low p-3">
                  <div className="flex items-start justify-between gap-3">
                    <div>
                      <h4 className="text-xs font-medium text-on-surface">Local IDE project context</h4>
                      <p className="mt-1 text-xs text-on-surface-variant">Index only selected roots in memory for symbols and <span className="font-mono">@file</span> mentions. Murmur never reads editor text, selections, or the clipboard.</p>
                    </div>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={profile.ideContextEnabled}
                      aria-label={`Local IDE project context for ${appLabel}`}
                      onClick={() => updateProfile(profile.bundleId, { ideContextEnabled: !profile.ideContextEnabled })}
                      className={`relative inline-flex h-6 w-11 shrink-0 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-primary ${profile.ideContextEnabled ? 'bg-primary' : 'bg-surface-container-highest'}`}
                    >
                      <span className={`inline-block h-4 w-4 rounded-full bg-on-primary shadow transition-transform ${profile.ideContextEnabled ? 'translate-x-6' : 'translate-x-1'}`} />
                    </button>
                  </div>
                  {profile.ideContextEnabled && (
                    <div className="mt-3 space-y-2">
                      {profile.ideProjectRoots.length > 0 ? (
                        <ul className="space-y-1">
                          {profile.ideProjectRoots.map((root) => (
                            <li key={root} className="flex items-center gap-2 rounded-md bg-surface-container-lowest px-2 py-1.5">
                              <span className="min-w-0 flex-1 break-all font-mono text-xs text-on-surface">{root}</span>
                              <button type="button" onClick={() => updateProfile(profile.bundleId, { ideProjectRoots: profile.ideProjectRoots.filter((candidate) => candidate !== root) })} className="shrink-0 text-xs text-on-surface-variant underline hover:text-error">Remove root</button>
                            </li>
                          ))}
                        </ul>
                      ) : <p className="text-xs text-on-surface-variant">No project roots configured.</p>}
                      <div className="flex flex-wrap gap-3">
                        <button type="button" onClick={() => void addIdeRoot(profile.bundleId)} disabled={profile.ideProjectRoots.length >= 4} className="text-xs font-medium text-on-surface-variant underline hover:text-primary disabled:opacity-50">Add project root</button>
                        <button type="button" onClick={() => void refreshIdeIndex(profile.bundleId)} disabled={profile.ideProjectRoots.length === 0 || ideStatuses[profile.bundleId]?.state === 'scanning'} className="text-xs font-medium text-on-surface-variant underline hover:text-primary disabled:opacity-50">Refresh index</button>
                        <button type="button" onClick={() => void clearIdeIndex(profile.bundleId)} disabled={profile.ideProjectRoots.length === 0} className="text-xs font-medium text-on-surface-variant underline hover:text-error disabled:opacity-50">Clear index</button>
                      </div>
                      <p role="status" className="text-xs text-on-surface-variant">{ideStatusText(ideStatuses[profile.bundleId], profile.ideProjectRoots.length)}</p>
                      <p className="text-xs text-on-surface-variant">Clear index removes only memory contents; Remove root changes the persisted override. Paths are never written to logs.</p>
                    </div>
                  )}
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
