import type { QueryProviderId, Settings, TransformKey } from './settings';
import type {
  QueryCommandConfig,
  QueryProviderTestResult,
} from './queryProviders';

/** Shown while a provider-switch preflight is interrupted by a further edit. */
export const QUERY_SWITCH_INTERRUPTED_NOTICE =
  'Configuration changed during provider preflight. Voice Query remains off.';

export function queryConfigurationMessage(error: unknown): string {
  const code = String(error);
  if (code.includes('invalid_executable')) return 'The CLI executable is missing, is not executable, or is not an absolute path.';
  if (code.includes('invalid_arguments')) return 'Fixed arguments exceed the Voice Query safety limits.';
  if (code.includes('invalid_timeout')) return 'Choose a timeout between 5 seconds and 5 minutes.';
  if (code.includes('invalid_environment')) return 'Declared environment values must be absolute config-directory paths.';
  if (code.includes('environment_unavailable')) return 'Murmur could not read the protected Voice Query environment file.';
  if (code.includes('unsupported_capabilities')) return 'This command does not match the trusted Claude profile. Revoke workspace access or reselect Claude.';
  if (code.includes('trusted_workspace_unavailable')) return 'The trusted folder changed or is unavailable. Revoke access and choose it again.';
  return 'Murmur could not validate this Voice Query configuration.';
}

export function isIncompleteCodexProbe(
  provider: QueryProviderId,
  result: QueryProviderTestResult,
): boolean {
  if (provider !== 'codex' || result.errorCode !== 'probe_failed') return false;
  const detail = `${result.stdout}\n${result.stderr}`;
  return detail.includes('The Codex CLI installation is incomplete')
    || (
      /ENOENT/i.test(detail)
      && /@openai\/codex-darwin-/i.test(detail)
      && /\/vendor\//i.test(detail)
      && /\/codex\/codex/i.test(detail)
    );
}

export function queryProviderTestMessage(
  provider: QueryProviderId,
  result: QueryProviderTestResult,
): string {
  if (isIncompleteCodexProbe(provider, result)) {
    return 'The Codex CLI installation is incomplete. Reinstall or update Codex, then choose Test again.';
  }
  if (result.authenticated === null) {
    return 'Executable validated. Custom providers do not have a built-in authentication probe.';
  }
  if (result.ok) return 'Authenticated and ready.';
  if (result.errorCode === 'provider_not_authenticated') {
    return result.signInFix ?? 'The provider is not authenticated.';
  }
  return 'The provider probe failed. Review its output below.';
}

export function queryCommand(settings: Settings): QueryCommandConfig {
  return {
    provider: settings.queryProvider,
    executable: settings.queryExecutable,
    arguments: settings.queryArguments,
    timeoutSeconds: settings.queryTimeoutSeconds,
    contextLevel: settings.queryContextLevel,
    retainQueryHistory: settings.retainQueryHistory,
  };
}

export function queryCommandFingerprintFor(
  command: QueryCommandConfig,
  transformHoldKey: TransformKey | null,
): string {
  return JSON.stringify([
    command.provider,
    command.executable,
    command.arguments,
    command.timeoutSeconds,
    command.contextLevel,
    transformHoldKey,
  ]);
}
