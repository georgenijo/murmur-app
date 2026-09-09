import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import {
  QUERY_KEY_OPTIONS,
  type QueryContextLevel,
  type QueryKey,
  type QueryProviderId,
  type Settings,
} from '../settings';
import {
  CUSTOM_QUERY_PRESET,
  launchQueryProviderSignIn,
  listQueryProviderPresets,
  loadQueryEnvironment,
  pollQuerySignIn,
  saveQueryEnvironment,
  testQueryProvider,
  validateQueryCommand,
  type QueryCommandConfig,
  type QueryEnvironmentVariable,
  type QueryProviderPreset,
  type QueryProviderTestResult,
} from '../queryProviders';
import {
  QUERY_SWITCH_INTERRUPTED_NOTICE,
  queryCommand,
  queryCommandFingerprintFor,
  queryConfigurationMessage,
  queryProviderTestMessage,
} from '../voiceQuerySettings';

export interface UseVoiceQuerySettingsParams {
  settings: Settings;
  onUpdateSettings: (updates: Partial<Settings>) => void;
  /** True while the Voice Query settings page is the active page — gates the
   *  preset/environment fetch effects the same way `activeCat` used to. */
  active: boolean;
}

/**
 * State, effects, and handlers for the Voice Query settings page (F6),
 * extracted from `SettingsPanel.tsx`. Behavior-preserving: every generation
 * check, race guard, and effect dependency matches the original inline
 * implementation.
 */
export function useVoiceQuerySettings({ settings, onUpdateSettings, active }: UseVoiceQuerySettingsParams) {
  const [queryConfigError, setQueryConfigError] = useState<string | null>(null);
  const [queryConfigNotice, setQueryConfigNotice] = useState<string | null>(null);
  const [queryPresets, setQueryPresets] = useState<QueryProviderPreset[]>([CUSTOM_QUERY_PRESET]);
  const [queryEnvironment, setQueryEnvironment] = useState<QueryEnvironmentVariable[]>([]);
  const [configuredQueryEnvironment, setConfiguredQueryEnvironment] = useState<string[]>([]);
  const [queryEnvironmentStatus, setQueryEnvironmentStatus] = useState<string | null>(null);
  const [queryEnvironmentNeedsRepair, setQueryEnvironmentNeedsRepair] = useState(false);
  const [queryConfigBusy, setQueryConfigBusy] = useState(false);
  const [queryTestResult, setQueryTestResult] = useState<QueryProviderTestResult | null>(null);
  const [queryTestBusy, setQueryTestBusy] = useState(false);
  const [querySignInStatus, setQuerySignInStatus] = useState<string | null>(null);
  const signInPollRef = useRef(0);
  const queryConfigGenerationRef = useRef(0);
  const queryProviderSwitchRef = useRef<{ generation: number; hotkey: QueryKey } | null>(null);
  const queryCommandFingerprint = queryCommandFingerprintFor(
    queryCommand(settings),
    settings.transformHoldKey,
  );
  const queryCommandFingerprintRef = useRef(queryCommandFingerprint);

  const invalidateQueryRequests = useCallback(() => {
    const interruptedProviderSwitch = queryProviderSwitchRef.current !== null;
    queryConfigGenerationRef.current += 1;
    signInPollRef.current += 1;
    queryProviderSwitchRef.current = null;
    setQueryConfigBusy(false);
    setQueryTestBusy(false);
    setQueryTestResult(null);
    setQuerySignInStatus(null);
    if (interruptedProviderSwitch) {
      setQueryConfigNotice(QUERY_SWITCH_INTERRUPTED_NOTICE);
    }
    return queryConfigGenerationRef.current;
  }, []);

  const queryRequestIsCurrent = useCallback((generation: number) => (
    queryConfigGenerationRef.current === generation
  ), []);

  useEffect(() => () => {
    signInPollRef.current += 1;
    queryConfigGenerationRef.current += 1;
    queryProviderSwitchRef.current = null;
  }, []);

  useLayoutEffect(() => {
    if (queryCommandFingerprintRef.current === queryCommandFingerprint) return;
    const interruptedProviderSwitch = queryProviderSwitchRef.current !== null;
    queryCommandFingerprintRef.current = queryCommandFingerprint;
    queryConfigGenerationRef.current += 1;
    signInPollRef.current += 1;
    queryProviderSwitchRef.current = null;
    setQueryConfigBusy(false);
    setQueryTestBusy(false);
    setQueryTestResult(null);
    setQuerySignInStatus(null);
    if (interruptedProviderSwitch) {
      setQueryConfigError(null);
      setQueryConfigNotice(QUERY_SWITCH_INTERRUPTED_NOTICE);
    }
  }, [queryCommandFingerprint]);

  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    void listQueryProviderPresets()
      .then((presets) => {
        if (!cancelled) setQueryPresets(presets);
      })
      .catch(() => {
        if (!cancelled) setQueryPresets([CUSTOM_QUERY_PRESET]);
      });
    return () => { cancelled = true; };
  }, [active]);

  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    setQueryEnvironmentStatus(null);
    setQueryEnvironmentNeedsRepair(false);
    void loadQueryEnvironment(settings.queryProvider)
      .then((names) => {
        if (!cancelled) {
          setConfiguredQueryEnvironment(Array.isArray(names) ? names : []);
          setQueryEnvironment([]);
          setQueryEnvironmentNeedsRepair(false);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setQueryEnvironment([]);
          setConfiguredQueryEnvironment([]);
          setQueryEnvironmentNeedsRepair(true);
          setQueryEnvironmentStatus('Could not load the protected environment file. Clear saved values to repair it.');
        }
      });
    return () => { cancelled = true; };
  }, [active, settings.queryProvider]);

  const toggleVoiceQuery = async () => {
    setQueryConfigError(null);
    if (settings.queryHotkey !== null) {
      invalidateQueryRequests();
      setQueryConfigNotice(null);
      onUpdateSettings({ queryHotkey: null });
      return;
    }
    if (!settings.queryExecutable.trim()) {
      setQueryConfigError('Choose the absolute path to a CLI executable before enabling Voice Query.');
      return;
    }
    const key = QUERY_KEY_OPTIONS.find((option) => option.value !== settings.transformHoldKey)?.value;
    if (!key) {
      setQueryConfigError('No dedicated shortcut is available.');
      return;
    }
    const command = queryCommand(settings);
    const generation = invalidateQueryRequests();
    setQueryConfigBusy(true);
    try {
      await validateQueryCommand(command);
      if (queryRequestIsCurrent(generation)) {
        setQueryConfigNotice(null);
        onUpdateSettings({ queryHotkey: key });
      }
    } catch (error) {
      if (queryRequestIsCurrent(generation)) {
        setQueryConfigError(queryConfigurationMessage(error));
      }
    } finally {
      if (queryRequestIsCurrent(generation)) {
        setQueryConfigBusy(false);
      }
    }
  };

  const selectQueryProvider = async (provider: QueryProviderId) => {
    const selected = queryPresets.find((preset) => preset.id === provider)
      ?? (provider === 'custom' ? CUSTOM_QUERY_PRESET : null);
    if (!selected) return;
    // A rapid second selection sees the hotkey carried by the still-current
    // switch transaction even though Settings has already persisted the
    // fail-closed temporary `null`. No completed or failed transaction keeps
    // this ref alive.
    const hotkeyToPreserve = settings.queryHotkey
      ?? queryProviderSwitchRef.current?.hotkey
      ?? null;
    const command: QueryCommandConfig = {
      provider,
      executable: selected.discoveredExecutable ?? '',
      arguments: [...selected.recommendedArguments],
      timeoutSeconds: settings.queryTimeoutSeconds,
      contextLevel: settings.queryContextLevel,
      retainQueryHistory: settings.retainQueryHistory,
    };
    const generation = invalidateQueryRequests();
    if (hotkeyToPreserve !== null) {
      queryProviderSwitchRef.current = { generation, hotkey: hotkeyToPreserve };
    }
    // This is the exact command being committed below. Priming the ref keeps
    // the controlled-settings layout effect from invalidating its own switch;
    // any different edit still changes the fingerprint and wins the race.
    queryCommandFingerprintRef.current = queryCommandFingerprintFor(
      command,
      settings.transformHoldKey,
    );
    setQueryConfigError(null);
    setQueryConfigNotice(hotkeyToPreserve !== null
      ? `Checking ${selected.label} before keeping Voice Query enabled…`
      : null);
    setQueryTestResult(null);
    setQuerySignInStatus(null);
    setQueryEnvironmentStatus(null);
    setQueryEnvironmentNeedsRepair(false);
    setQueryEnvironment([]);
    setConfiguredQueryEnvironment([]);
    onUpdateSettings({
      queryProvider: provider,
      queryExecutable: command.executable,
      queryArguments: command.arguments,
      queryHotkey: null,
    });
    if (hotkeyToPreserve === null) return;

    setQueryConfigBusy(true);
    setQueryTestBusy(true);
    try {
      await validateQueryCommand(command);
      if (!queryRequestIsCurrent(generation)) return;
      const result = await testQueryProvider(command);
      if (!queryRequestIsCurrent(generation)) return;
      setQueryTestResult(result);
      if (!result.ok) {
        queryProviderSwitchRef.current = null;
        setQueryConfigNotice(null);
        setQueryConfigError(
          `Voice Query remains off. ${queryProviderTestMessage(provider, result)}`,
        );
        return;
      }
      const pending = queryProviderSwitchRef.current;
      if (!pending || pending.generation !== generation || pending.hotkey !== hotkeyToPreserve) {
        return;
      }
      queryProviderSwitchRef.current = null;
      setQueryConfigNotice(
        `Provider changed to ${selected.label}. Voice Query stayed enabled after validation and preflight.`,
      );
      onUpdateSettings({ queryHotkey: hotkeyToPreserve });
    } catch (error) {
      if (queryRequestIsCurrent(generation)) {
        queryProviderSwitchRef.current = null;
        setQueryConfigNotice(null);
        setQueryConfigError(`Voice Query remains off. ${queryConfigurationMessage(error)}`);
      }
    } finally {
      if (queryRequestIsCurrent(generation)) {
        setQueryConfigBusy(false);
        setQueryTestBusy(false);
      }
    }
  };

  const saveDeclaredEnvironment = async () => {
    setQueryEnvironmentStatus(null);
    const entered = queryEnvironment.filter((variable) => variable.value.length > 0);
    if (entered.length === 0) {
      setQueryEnvironmentStatus('Enter an absolute config-directory path to save.');
      return;
    }
    const provider = settings.queryProvider;
    const generation = invalidateQueryRequests();
    try {
      await saveQueryEnvironment(provider, entered);
      if (!queryRequestIsCurrent(generation)) return;
      setConfiguredQueryEnvironment((current) => [
        ...new Set([...current, ...entered.map((variable) => variable.name)]),
      ]);
      setQueryEnvironment([]);
      setQueryEnvironmentStatus('Saved in Murmur’s protected app-data directory.');
      setQueryEnvironmentNeedsRepair(false);
      setQueryTestResult(null);
      setQuerySignInStatus(null);
    } catch (error) {
      if (queryRequestIsCurrent(generation)) {
        setQueryEnvironmentStatus(queryConfigurationMessage(error));
      }
    }
  };

  const clearDeclaredEnvironment = async () => {
    setQueryEnvironmentStatus(null);
    const provider = settings.queryProvider;
    const generation = invalidateQueryRequests();
    try {
      await saveQueryEnvironment(provider, []);
      if (!queryRequestIsCurrent(generation)) return;
      setQueryEnvironment([]);
      setConfiguredQueryEnvironment([]);
      setQueryEnvironmentStatus('Saved config-directory values cleared.');
      setQueryEnvironmentNeedsRepair(false);
      setQueryTestResult(null);
      setQuerySignInStatus(null);
    } catch (error) {
      if (queryRequestIsCurrent(generation)) {
        setQueryEnvironmentStatus(queryConfigurationMessage(error));
      }
    }
  };

  const runQueryTest = async (): Promise<QueryProviderTestResult | null> => {
    setQueryConfigError(null);
    setQuerySignInStatus(null);
    const command = queryCommand(settings);
    const generation = invalidateQueryRequests();
    setQueryTestBusy(true);
    try {
      const result = await testQueryProvider(command);
      if (!queryRequestIsCurrent(generation)) return null;
      setQueryTestResult(result);
      return result;
    } catch (error) {
      if (queryRequestIsCurrent(generation)) {
        setQueryConfigError(queryConfigurationMessage(error));
        setQueryTestResult(null);
      }
      return null;
    } finally {
      if (queryRequestIsCurrent(generation)) {
        setQueryTestBusy(false);
      }
    }
  };

  const signInQueryProvider = async () => {
    const poll = signInPollRef.current + 1;
    signInPollRef.current = poll;
    const command = queryCommand(settings);
    const generation = queryConfigGenerationRef.current;
    const ownsRequest = () => (
      signInPollRef.current === poll && queryRequestIsCurrent(generation)
    );
    setQueryConfigError(null);
    setQuerySignInStatus('Opening Terminal…');
    try {
      await pollQuerySignIn({
        launch: () => launchQueryProviderSignIn(command),
        onLaunched: () => setQuerySignInStatus('Terminal opened. Waiting for sign-in…'),
        probe: () => testQueryProvider(command),
        isSignedIn: (result) => result.ok,
        onProbeResult: (result) => setQueryTestResult(result),
        onSignedIn: () => setQuerySignInStatus('Signed in and ready.'),
        onPending: () => setQuerySignInStatus('Sign-in is still pending. Finish in Terminal, then choose Test.'),
        ownsAttempt: ownsRequest,
      });
    } catch (error) {
      if (ownsRequest()) {
        setQuerySignInStatus(null);
        setQueryConfigError(String(error).includes('sign_in')
          ? 'Murmur could not open the provider sign-in in Terminal.'
          : queryConfigurationMessage(error));
      }
    }
  };

  const chooseQueryExecutable = async () => {
    const generation = queryConfigGenerationRef.current;
    try {
      const selected = await open({ directory: false, multiple: false });
      if (typeof selected === 'string' && queryRequestIsCurrent(generation)) {
        setQueryConfigError(null);
        setQueryConfigNotice(settings.queryHotkey !== null
          ? 'Command changed. Voice Query was turned off so the new command can be tested before use.'
          : null);
        setQueryTestResult(null);
        setQuerySignInStatus(null);
        invalidateQueryRequests();
        onUpdateSettings({ queryExecutable: selected, queryHotkey: null });
      }
    } catch {
      // Cancellation leaves the configured executable untouched.
    }
  };

  const updateQueryExecutableText = (value: string) => {
    setQueryConfigError(null);
    setQueryConfigNotice(settings.queryHotkey !== null
      ? 'Command changed. Voice Query was turned off so the new command can be tested before use.'
      : null);
    setQueryTestResult(null);
    setQuerySignInStatus(null);
    invalidateQueryRequests();
    onUpdateSettings({ queryExecutable: value, queryHotkey: null });
  };

  const updateQueryArgumentsText = (value: string) => {
    setQueryConfigError(null);
    setQueryConfigNotice(settings.queryHotkey !== null
      ? 'Command changed. Voice Query was turned off so the new command can be tested before use.'
      : null);
    setQueryTestResult(null);
    setQuerySignInStatus(null);
    invalidateQueryRequests();
    onUpdateSettings({
      queryArguments: value.split('\n').filter((argument) => argument.length > 0),
      queryHotkey: null,
    });
  };

  const updateQueryEnvironmentDraft = (name: string, value: string) => {
    invalidateQueryRequests();
    setQueryEnvironment((current) => [
      ...current.filter((variable) => variable.name !== name),
      ...(value ? [{ name, value }] : []),
    ]);
    setQueryEnvironmentStatus(null);
  };

  const updateQueryContextLevel = (queryContextLevel: QueryContextLevel) => {
    invalidateQueryRequests();
    onUpdateSettings({ queryContextLevel });
  };

  const updateQueryHotkey = (value: QueryKey) => {
    if (value === settings.transformHoldKey) {
      setQueryConfigError('That key is already assigned to Selected-text Transform.');
      return;
    }
    setQueryConfigError(null);
    onUpdateSettings({ queryHotkey: value });
  };

  const updateQueryTimeoutSeconds = (value: string) => {
    invalidateQueryRequests();
    onUpdateSettings({ queryTimeoutSeconds: Number(value) });
  };

  const selectedQueryPreset = queryPresets.find((preset) => preset.id === settings.queryProvider)
    ?? (settings.queryProvider === 'custom' ? CUSTOM_QUERY_PRESET : null);
  const queryProviderItems = queryPresets.map((preset) => ({
    value: preset.id,
    label: preset.discoveredExecutable || preset.id === 'custom'
      ? preset.label
      : `${preset.label} — not found`,
  }));

  return {
    queryConfigError,
    queryConfigNotice,
    queryPresets,
    queryEnvironment,
    configuredQueryEnvironment,
    queryEnvironmentStatus,
    queryEnvironmentNeedsRepair,
    queryConfigBusy,
    queryTestResult,
    queryTestBusy,
    querySignInStatus,
    selectedQueryPreset,
    queryProviderItems,
    toggleVoiceQuery,
    selectQueryProvider,
    saveDeclaredEnvironment,
    clearDeclaredEnvironment,
    runQueryTest,
    signInQueryProvider,
    chooseQueryExecutable,
    updateQueryExecutableText,
    updateQueryArgumentsText,
    updateQueryEnvironmentDraft,
    updateQueryContextLevel,
    updateQueryHotkey,
    updateQueryTimeoutSeconds,
  };
}
