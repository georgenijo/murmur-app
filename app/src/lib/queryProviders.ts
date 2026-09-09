import { invoke } from '@tauri-apps/api/core';
import type { QueryContextLevel, QueryProviderId } from './settings';

export interface QueryCommandConfig {
  provider: QueryProviderId;
  executable: string;
  arguments: string[];
  timeoutSeconds: number;
  contextLevel: QueryContextLevel;
  retainQueryHistory: boolean;
}

export interface QueryProviderPreset {
  id: QueryProviderId;
  label: string;
  discoveryPaths: string[];
  discoveredExecutable: string | null;
  recommendedArguments: string[];
  authProbeArguments: string[];
  authFailureSignatures: string[];
  signInArguments: string[];
  signInFix: string | null;
  permittedEnvironmentVariables: string[];
}

export interface QueryEnvironmentVariable {
  name: string;
  value: string;
}

export interface QueryProviderTestResult {
  ok: boolean;
  authenticated: boolean | null;
  errorCode: string | null;
  stdout: string;
  stderr: string;
  stdoutTruncated: boolean;
  stderrTruncated: boolean;
  signInFix: string | null;
}

export const CUSTOM_QUERY_PRESET: QueryProviderPreset = {
  id: 'custom',
  label: 'Custom',
  discoveryPaths: [],
  discoveredExecutable: null,
  recommendedArguments: [],
  authProbeArguments: [],
  authFailureSignatures: [],
  signInArguments: [],
  signInFix: null,
  permittedEnvironmentVariables: ['CLAUDE_CONFIG_DIR', 'CODEX_HOME'],
};

export async function listQueryProviderPresets(): Promise<QueryProviderPreset[]> {
  const presets = await invoke<QueryProviderPreset[]>('list_query_provider_presets');
  return Array.isArray(presets) && presets.length > 0 ? presets : [CUSTOM_QUERY_PRESET];
}

export async function loadQueryEnvironment(
  provider: QueryProviderId,
): Promise<string[]> {
  return invoke<string[]>('load_query_environment', { provider });
}

export async function saveQueryEnvironment(
  provider: QueryProviderId,
  variables: QueryEnvironmentVariable[],
): Promise<void> {
  return invoke('save_query_environment', { provider, variables });
}

export async function validateQueryCommand(command: QueryCommandConfig): Promise<void> {
  await invoke('validate_query_command', { command });
}

export async function testQueryProvider(
  command: QueryCommandConfig,
): Promise<QueryProviderTestResult> {
  return invoke<QueryProviderTestResult>('test_query_provider', { command });
}

export async function launchQueryProviderSignIn(command: QueryCommandConfig): Promise<void> {
  await invoke('launch_query_provider_sign_in', { command });
}

export interface PollQuerySignInOptions<T> {
  /** Kicks off sign-in (e.g. opens a Terminal window). Errors propagate to the caller. */
  launch: () => Promise<void>;
  /** Called once launch succeeds and this attempt is still current. */
  onLaunched?: () => void;
  /** Probed on each tick after launch succeeds. Errors propagate to the caller. */
  probe: () => Promise<T>;
  /** Whether a probe result counts as "signed in", ending the poll. */
  isSignedIn: (result: T) => boolean;
  /** Called with every probe result, whether or not it counts as signed in. */
  onProbeResult?: (result: T) => void;
  /** Called once a probe result is judged signed in. */
  onSignedIn?: (result: T) => void;
  /** Called if the deadline is reached without a signed-in result. */
  onPending?: () => void;
  /**
   * Returns false once this attempt is stale (superseded or unmounted) and
   * polling should stop silently without invoking any further callback.
   */
  ownsAttempt: () => boolean;
  /** Poll interval; defaults to 2000ms. */
  intervalMs?: number;
  /** Total time budget from launch to giving up; defaults to 60_000ms. */
  timeoutMs?: number;
  /** Injectable clock, defaults to `Date.now`. */
  now?: () => number;
  /** Injectable delay, defaults to a real `setTimeout`. */
  sleep?: (ms: number) => Promise<void>;
}

/**
 * Shared sign-in poll loop: launch a provider sign-in flow, then probe on a
 * fixed interval until either a probe result counts as signed in or the
 * overall timeout elapses. Every step re-checks `ownsAttempt` so a stale or
 * superseded attempt stops silently instead of applying late UI updates.
 */
export async function pollQuerySignIn<T>(options: PollQuerySignInOptions<T>): Promise<void> {
  const {
    launch,
    onLaunched,
    probe,
    isSignedIn,
    onProbeResult,
    onSignedIn,
    onPending,
    ownsAttempt,
    intervalMs = 2000,
    timeoutMs = 60_000,
    now = Date.now,
    sleep = (ms: number) => new Promise((resolve) => { window.setTimeout(resolve, ms); }),
  } = options;

  await launch();
  if (!ownsAttempt()) return;
  onLaunched?.();

  const deadline = now() + timeoutMs;
  while (ownsAttempt() && now() < deadline) {
    await sleep(intervalMs);
    if (!ownsAttempt()) return;
    const result = await probe();
    if (!ownsAttempt()) return;
    onProbeResult?.(result);
    if (isSignedIn(result)) {
      onSignedIn?.(result);
      return;
    }
  }
  if (ownsAttempt()) {
    onPending?.();
  }
}
