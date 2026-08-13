/**
 * Voice Query provider presets, auth preflight, and declared environment
 * variables (#550).
 *
 * Every command here is host-gated: presets and environment variables are main
 * window only, and the probe output it returns is shown in Settings and never
 * persisted, logged, or shipped.
 */

import { invoke } from '@tauri-apps/api/core';

export const CUSTOM_PRESET_ID = 'custom';

export interface QueryPresetInfo {
  id: string;
  label: string;
  summary: string;
  binaryName: string;
  recommendedArguments: string[];
  suggestedEnvKeys: string[];
  loginHint: string;
  /** Absolute path Murmur found for this provider, or null when not installed. */
  discoveredPath: string | null;
}

export type QueryAuthVerdict = 'authenticated' | 'not_authenticated' | 'unknown';

export interface QueryAuthProbeReport {
  verdict: QueryAuthVerdict;
  exitCode: number | null;
  output: string;
  truncated: boolean;
  durationMs: number;
  loginHint: string | null;
}

export interface QueryCommandSnapshot {
  executable: string;
  arguments: string[];
  timeoutSeconds: number;
  presetId: string | null;
}

export interface QueryEnvVar {
  name: string;
  value: string;
}

export const MAX_QUERY_ENV_VARS = 16;

export function listQueryPresets(): Promise<QueryPresetInfo[]> {
  return invoke<QueryPresetInfo[]>('list_query_presets');
}

/** Resolves when the command is usable; rejects with a stable error code. */
export function validateQueryCommand(command: QueryCommandSnapshot): Promise<void> {
  return invoke<void>('validate_query_command', { command });
}

export function probeQueryProviderAuth(
  presetId: string | null,
  command: QueryCommandSnapshot,
): Promise<QueryAuthProbeReport> {
  return invoke<QueryAuthProbeReport>('probe_query_provider_auth', { presetId, command });
}

export function launchQueryProviderLogin(
  presetId: string,
  command: QueryCommandSnapshot,
): Promise<void> {
  return invoke<void>('launch_query_provider_login', { presetId, command });
}

export function loadQueryEnvVars(): Promise<QueryEnvVar[]> {
  return invoke<QueryEnvVar[]>('load_query_env_vars');
}

export function saveQueryEnvVars(variables: QueryEnvVar[]): Promise<void> {
  return invoke<void>('save_query_env_vars', { variables });
}

/**
 * The executable and arguments choosing this preset should apply.
 *
 * A preset only ever *offers* configuration: an executable Murmur could not
 * find leaves the current path alone so a manually chosen one is not wiped by
 * re-selecting the same provider.
 */
export function presetSelection(
  preset: QueryPresetInfo,
  currentExecutable: string,
): { executable: string; arguments: string[] } {
  return {
    executable: preset.discoveredPath ?? currentExecutable,
    arguments: [...preset.recommendedArguments],
  };
}

/** One line summarising a finished probe, for the Settings status row. */
export function probeSummary(report: QueryAuthProbeReport, providerLabel: string): string {
  switch (report.verdict) {
    case 'authenticated':
      return `${providerLabel} is signed in and ready.`;
    case 'not_authenticated':
      return `${providerLabel} is not signed in.`;
    default:
      return report.exitCode === null
        ? `${providerLabel} did not report a result. Check the output below.`
        : `${providerLabel} answered with exit code ${report.exitCode}. Check the output below.`;
  }
}

/**
 * Client-side mirror of the Rust validator, so a bad row is reported as it is
 * typed instead of only on save. Rust remains authoritative.
 */
const RESERVED_ENV_NAMES = ['HOME', 'PATH', 'TMPDIR', 'LANG', 'LC_ALL', 'LC_CTYPE', 'USER', 'LOGNAME'];

/**
 * Names that load code into the child or redirect its TLS. Mirrors
 * `DENIED_NAMES` in `query_env.rs` — keep the two in step.
 */
const DENIED_ENV_NAMES = [
  'NODE_OPTIONS', 'NODE_PATH', 'NODE_REPL_EXTERNAL_MODULE', 'NODE_EXTRA_CA_CERTS',
  'PYTHONPATH', 'PYTHONHOME', 'PYTHONSTARTUP', 'RUBYOPT', 'RUBYLIB', 'PERL5OPT', 'PERL5LIB',
  'BASH_ENV', 'ENV', 'SHELLOPTS', 'JAVA_TOOL_OPTIONS', '_JAVA_OPTIONS', 'CLASSPATH',
  'HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'SSLKEYLOGFILE', 'SSL_CERT_FILE', 'SSL_CERT_DIR',
  'REQUESTS_CA_BUNDLE',
];

export function queryEnvVarError(variables: QueryEnvVar[]): string | null {
  if (variables.length > MAX_QUERY_ENV_VARS) {
    return `Voice Query accepts at most ${MAX_QUERY_ENV_VARS} declared environment variables.`;
  }
  const seen = new Set<string>();
  for (const variable of variables) {
    const name = variable.name.trim();
    if (!name) return 'Every declared environment variable needs a name.';
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) {
      return `“${name}” is not a valid environment variable name. Use letters, digits, and underscores.`;
    }
    // Case-insensitive, like the Rust validator: `https_proxy` is as real a
    // variable as `HTTPS_PROXY`.
    const upper = name.toUpperCase();
    if (RESERVED_ENV_NAMES.includes(upper)) {
      return `${name} is forwarded by Murmur itself and cannot be redeclared.`;
    }
    if (upper.startsWith('DYLD_') || upper.startsWith('LD_') || DENIED_ENV_NAMES.includes(upper)) {
      return `${name} can change which code the CLI loads or where its traffic goes, and is not allowed.`;
    }
    if (seen.has(upper)) return `${name} is declared more than once.`;
    seen.add(upper);
    if (variable.value.length > 4096) {
      return `The value for ${name} exceeds the 4096 byte limit.`;
    }
  }
  return null;
}
