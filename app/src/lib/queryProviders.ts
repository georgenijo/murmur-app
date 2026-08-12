import { invoke } from '@tauri-apps/api/core';
import type { QueryProviderId } from './settings';

export interface QueryCommandConfig {
  provider: QueryProviderId;
  executable: string;
  arguments: string[];
  timeoutSeconds: number;
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
