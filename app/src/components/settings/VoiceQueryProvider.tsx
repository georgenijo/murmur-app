import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  CUSTOM_PRESET_ID,
  launchQueryProviderLogin,
  listQueryPresets,
  loadQueryEnvVars,
  MAX_QUERY_ENV_VARS,
  presetSelection,
  probeQueryProviderAuth,
  probeSummary,
  queryEnvVarError,
  saveQueryEnvVars,
  type QueryAuthProbeReport,
  type QueryCommandSnapshot,
  type QueryEnvVar,
  type QueryPresetInfo,
} from '../../lib/queryProviders';
import { Select } from '../ui/Select';

/** How long the post-login watch keeps re-checking before it gives up. */
const SIGN_IN_WATCH_MS = 120_000;
const SIGN_IN_POLL_MS = 4_000;

interface VoiceQueryProviderProps {
  presetId: string;
  executable: string;
  args: string[];
  timeoutSeconds: number;
  onChange: (patch: { queryPresetId?: string; queryExecutable?: string; queryArguments?: string[] }) => void;
}

/**
 * Provider picker, sign-in preflight, and declared environment variables for
 * Voice Query (#550).
 *
 * The preset only fills in configuration the user can still edit — Murmur
 * always spawns exactly what the executable and argument fields say. The
 * sign-in check runs the provider's own status command through the identical
 * spawn path the query uses, so a green check here means the real thing will
 * work; its output is shown here and goes nowhere else.
 */
export function VoiceQueryProvider({
  presetId,
  executable,
  args,
  timeoutSeconds,
  onChange,
}: VoiceQueryProviderProps) {
  const [presets, setPresets] = useState<QueryPresetInfo[] | null>(null);
  const [probe, setProbe] = useState<QueryAuthProbeReport | null>(null);
  const [probing, setProbing] = useState(false);
  const [probeError, setProbeError] = useState<string | null>(null);
  const [watching, setWatching] = useState(false);
  const [envVars, setEnvVars] = useState<QueryEnvVar[]>([]);
  const [envError, setEnvError] = useState<string | null>(null);
  const [envSaved, setEnvSaved] = useState(false);
  const watchdogRef = useRef<{ timer: number | null; until: number }>({ timer: null, until: 0 });

  const selected = presets?.find((preset) => preset.id === presetId) ?? null;
  // Arrays arrive with a fresh identity every render, so the memo key is the
  // joined argument list rather than the array itself.
  const argumentsKey = args.join('\n');
  const command: QueryCommandSnapshot = useMemo(() => ({
    executable,
    arguments: argumentsKey.length > 0 ? argumentsKey.split('\n') : [],
    timeoutSeconds,
    presetId: presetId === CUSTOM_PRESET_ID ? null : presetId,
  }), [executable, argumentsKey, timeoutSeconds, presetId]);

  useEffect(() => {
    let disposed = false;
    void listQueryPresets()
      .then((list) => { if (!disposed) setPresets(Array.isArray(list) ? list : []); })
      .catch(() => { if (!disposed) setPresets([]); });
    void loadQueryEnvVars()
      .then((variables) => { if (!disposed) setEnvVars(Array.isArray(variables) ? variables : []); })
      .catch(() => { if (!disposed) setEnvError('Murmur could not read the declared environment variables.'); });
    return () => { disposed = true; };
  }, []);

  const stopWatching = useCallback(() => {
    if (watchdogRef.current.timer !== null) {
      window.clearTimeout(watchdogRef.current.timer);
      watchdogRef.current.timer = null;
    }
    setWatching(false);
  }, []);

  useEffect(() => stopWatching, [stopWatching]);

  const runProbe = useCallback(async (): Promise<QueryAuthProbeReport | null> => {
    setProbing(true);
    setProbeError(null);
    try {
      const report = await probeQueryProviderAuth(command.presetId, command);
      setProbe(report);
      return report;
    } catch (error) {
      setProbe(null);
      setProbeError(typeof error === 'string' ? error : 'The provider check could not be completed.');
      return null;
    } finally {
      setProbing(false);
    }
  }, [command]);

  /**
   * After the vendor login opens, keep re-checking until the provider reports
   * signed in. The user finishes in Terminal; Settings turns green on its own
   * rather than asking them to come back and press Test again.
   */
  const watchUntilSignedIn = useCallback(() => {
    watchdogRef.current.until = Date.now() + SIGN_IN_WATCH_MS;
    setWatching(true);
    const tick = async () => {
      const report = await runProbe();
      if (report?.verdict === 'authenticated' || Date.now() >= watchdogRef.current.until) {
        stopWatching();
        return;
      }
      watchdogRef.current.timer = window.setTimeout(() => { void tick(); }, SIGN_IN_POLL_MS);
    };
    watchdogRef.current.timer = window.setTimeout(() => { void tick(); }, SIGN_IN_POLL_MS);
  }, [runProbe, stopWatching]);

  const signIn = async () => {
    if (presetId === CUSTOM_PRESET_ID) return;
    setProbeError(null);
    try {
      await launchQueryProviderLogin(presetId, command);
      watchUntilSignedIn();
    } catch (error) {
      setProbeError(typeof error === 'string' ? error : 'Murmur could not open the provider sign-in.');
    }
  };

  const choosePreset = (nextId: string) => {
    setProbe(null);
    setProbeError(null);
    stopWatching();
    if (nextId === CUSTOM_PRESET_ID) {
      onChange({ queryPresetId: CUSTOM_PRESET_ID });
      return;
    }
    const preset = presets?.find((entry) => entry.id === nextId);
    if (!preset) {
      onChange({ queryPresetId: nextId });
      return;
    }
    const selection = presetSelection(preset, executable);
    onChange({
      queryPresetId: nextId,
      queryExecutable: selection.executable,
      queryArguments: selection.arguments,
    });
  };

  const updateEnvVars = (next: QueryEnvVar[]) => {
    setEnvVars(next);
    setEnvSaved(false);
    setEnvError(queryEnvVarError(next));
  };

  const saveEnv = async () => {
    const validationError = queryEnvVarError(envVars);
    if (validationError) {
      setEnvError(validationError);
      return;
    }
    try {
      await saveQueryEnvVars(envVars);
      setEnvError(null);
      setEnvSaved(true);
    } catch (error) {
      setEnvSaved(false);
      setEnvError(typeof error === 'string' ? error : 'Murmur could not save the environment variables.');
    }
  };

  const presetItems = [
    ...(presets ?? []).map((preset) => ({ value: preset.id, label: preset.label })),
    { value: CUSTOM_PRESET_ID, label: 'Custom command' },
  ];

  return (
    <div className="space-y-4">
      <div>
        <label className="mb-1.5 block text-sm font-medium text-on-surface">Provider</label>
        <Select value={presetId} onChange={choosePreset} items={presetItems} />
        <p className="mt-1 text-xs text-on-surface-variant">
          {selected
            ? selected.discoveredPath
              ? `${selected.summary} Found at ${selected.discoveredPath}.`
              : `${selected.summary} Murmur could not find ${selected.binaryName} — choose the path below.`
            : 'Configure the executable and arguments yourself. No provider-specific checks are available.'}
        </p>
      </div>

      <div className="rounded-xl border border-outline-variant/30 p-3">
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={() => { stopWatching(); void runProbe(); }}
            disabled={probing || presetId === CUSTOM_PRESET_ID || !executable.trim()}
            className="rounded-lg border border-outline-variant/30 px-3 py-1.5 text-xs font-semibold text-on-surface hover:bg-surface-container disabled:opacity-40"
          >
            {probing ? 'Checking…' : 'Test sign-in'}
          </button>
          {probe && probe.verdict !== 'authenticated' && presetId !== CUSTOM_PRESET_ID && (
            <button
              type="button"
              onClick={() => void signIn()}
              className="rounded-lg bg-primary px-3 py-1.5 text-xs font-semibold text-on-primary hover:bg-primary-dim"
            >
              Sign in…
            </button>
          )}
          {watching && <span className="text-xs text-on-surface-variant">Waiting for the sign-in to finish…</span>}
        </div>

        {presetId === CUSTOM_PRESET_ID && (
          <p className="mt-2 text-xs text-on-surface-variant">
            A custom command has no known status check. Murmur still reports a sign-in failure from
            the command's own output when a query fails.
          </p>
        )}
        {probeError && <p role="alert" className="mt-2 text-xs text-error">{probeError}</p>}
        {probe && selected && (
          <div className="mt-2 space-y-1.5">
            <p
              className={`text-xs font-medium ${probe.verdict === 'authenticated' ? 'text-on-surface' : 'text-error'}`}
            >
              {probeSummary(probe, selected.label)}
            </p>
            {probe.loginHint && (
              <p className="text-xs text-on-surface-variant">
                Fix: run <code className="rounded bg-surface-container px-1 py-0.5 font-mono">{probe.loginHint}</code> in Terminal.
              </p>
            )}
            {probe.output && (
              <pre className="max-h-32 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-surface-container-lowest p-2 font-mono text-[11px] leading-snug text-on-surface-variant">
                {probe.output}
              </pre>
            )}
            <p className="text-[11px] text-on-surface-variant">
              This output stays on this screen. It is never logged, saved, or uploaded.
            </p>
          </div>
        )}
      </div>

      <div>
        <div className="mb-1.5 flex items-center justify-between">
          <label className="block text-sm font-medium text-on-surface">Environment variables</label>
          <button
            type="button"
            onClick={() => updateEnvVars([...envVars, { name: '', value: '' }])}
            disabled={envVars.length >= MAX_QUERY_ENV_VARS}
            className="rounded-lg border border-outline-variant/30 px-2 py-1 text-xs font-semibold text-on-surface hover:bg-surface-container disabled:opacity-40"
          >
            Add
          </button>
        </div>
        <p className="mb-2 text-xs text-on-surface-variant">
          The CLI otherwise starts with a cleared environment. Declare only the names it needs to
          find its own configuration
          {selected && selected.suggestedEnvKeys.length > 0 ? ` — for ${selected.label}: ${selected.suggestedEnvKeys.join(', ')}` : ''}
          . Values are stored in plain text in Murmur's app data, so do not put API keys or other
          secrets here.
        </p>
        <div className="space-y-2">
          {envVars.map((variable, index) => (
            <div key={index} className="flex gap-2">
              <input
                type="text"
                aria-label={`Environment variable ${index + 1} name`}
                value={variable.name}
                onChange={(event) => updateEnvVars(envVars.map((entry, entryIndex) =>
                  entryIndex === index ? { ...entry, name: event.target.value } : entry))}
                placeholder="CLAUDE_CONFIG_DIR"
                spellCheck={false}
                className="w-1/3 min-w-0 rounded-lg border border-outline-variant bg-surface-container-lowest px-3 py-2 font-mono text-xs text-on-surface outline-none focus:border-primary"
              />
              <input
                type="text"
                aria-label={`Environment variable ${index + 1} value`}
                value={variable.value}
                onChange={(event) => updateEnvVars(envVars.map((entry, entryIndex) =>
                  entryIndex === index ? { ...entry, value: event.target.value } : entry))}
                placeholder="/Users/you/.claude"
                spellCheck={false}
                className="min-w-0 flex-1 rounded-lg border border-outline-variant bg-surface-container-lowest px-3 py-2 font-mono text-xs text-on-surface outline-none focus:border-primary"
              />
              <button
                type="button"
                aria-label={`Remove environment variable ${index + 1}`}
                onClick={() => updateEnvVars(envVars.filter((_, entryIndex) => entryIndex !== index))}
                className="rounded-lg border border-outline-variant/30 px-2 py-2 text-xs text-on-surface-variant hover:bg-surface-container"
              >
                ×
              </button>
            </div>
          ))}
        </div>
        {envVars.length > 0 && (
          <div className="mt-2 flex items-center gap-2">
            <button
              type="button"
              onClick={() => void saveEnv()}
              className="rounded-lg border border-outline-variant/30 px-3 py-1.5 text-xs font-semibold text-on-surface hover:bg-surface-container"
            >
              Save variables
            </button>
            {envSaved && <span className="text-xs text-on-surface-variant">Saved.</span>}
          </div>
        )}
        {envError && <p role="alert" className="mt-2 text-xs text-error">{envError}</p>}
      </div>
    </div>
  );
}
