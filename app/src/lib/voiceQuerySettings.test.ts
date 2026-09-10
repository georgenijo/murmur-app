import { describe, expect, it } from 'vitest';
import { DEFAULT_SETTINGS } from './settings';
import type { QueryProviderTestResult } from './queryProviders';
import {
  QUERY_SWITCH_INTERRUPTED_NOTICE,
  isIncompleteCodexProbe,
  queryCommand,
  queryCommandFingerprintFor,
  queryConfigurationMessage,
  queryProviderTestMessage,
} from './voiceQuerySettings';

function testResult(overrides: Partial<QueryProviderTestResult> = {}): QueryProviderTestResult {
  return {
    ok: false,
    authenticated: false,
    errorCode: null,
    stdout: '',
    stderr: '',
    stdoutTruncated: false,
    stderrTruncated: false,
    signInFix: null,
    ...overrides,
  };
}

describe('queryConfigurationMessage', () => {
  it('maps known Rust error codes to human-readable copy', () => {
    expect(queryConfigurationMessage('invalid_executable')).toMatch(/absolute path/);
    expect(queryConfigurationMessage('invalid_arguments')).toMatch(/safety limits/);
    expect(queryConfigurationMessage('invalid_timeout')).toMatch(/5 seconds and 5 minutes/);
    expect(queryConfigurationMessage('invalid_environment')).toMatch(/config-directory paths/);
    expect(queryConfigurationMessage('environment_unavailable')).toMatch(/protected Voice Query environment file/);
  });

  it('falls back to a generic message for unknown errors', () => {
    expect(queryConfigurationMessage('boom')).toBe('Murmur could not validate this Voice Query configuration.');
  });
});

describe('isIncompleteCodexProbe', () => {
  it('is false for non-codex providers regardless of output', () => {
    expect(isIncompleteCodexProbe('claude', testResult({
      errorCode: 'probe_failed',
      stdout: 'The Codex CLI installation is incomplete',
    }))).toBe(false);
  });

  it('is false when the error code is not probe_failed', () => {
    expect(isIncompleteCodexProbe('codex', testResult({
      errorCode: 'provider_not_authenticated',
      stdout: 'The Codex CLI installation is incomplete',
    }))).toBe(false);
  });

  it('recognizes the explicit incomplete-install message', () => {
    expect(isIncompleteCodexProbe('codex', testResult({
      errorCode: 'probe_failed',
      stdout: 'The Codex CLI installation is incomplete',
    }))).toBe(true);
  });

  it('recognizes the ENOENT vendor-binary signature', () => {
    expect(isIncompleteCodexProbe('codex', testResult({
      errorCode: 'probe_failed',
      stderr: 'Error: ENOENT, open \'/x/@openai/codex-darwin-arm64/vendor/x86_64-apple-darwin/codex/codex\'',
    }))).toBe(true);
  });

  it('is false for an unrelated probe failure', () => {
    expect(isIncompleteCodexProbe('codex', testResult({
      errorCode: 'probe_failed',
      stderr: 'permission denied',
    }))).toBe(false);
  });
});

describe('queryProviderTestMessage', () => {
  it('prioritizes the incomplete-codex-install message', () => {
    const result = testResult({
      errorCode: 'probe_failed',
      stdout: 'The Codex CLI installation is incomplete',
    });
    expect(queryProviderTestMessage('codex', result)).toMatch(/Reinstall or update Codex/);
  });

  it('reports custom providers as validated without an auth probe', () => {
    expect(queryProviderTestMessage('custom', testResult({ authenticated: null }))).toMatch(/do not have a built-in authentication probe/);
  });

  it('reports success when ok', () => {
    expect(queryProviderTestMessage('claude', testResult({ ok: true, authenticated: true }))).toBe('Authenticated and ready.');
  });

  it('uses the provided sign-in fix for an unauthenticated provider', () => {
    expect(queryProviderTestMessage('claude', testResult({
      errorCode: 'provider_not_authenticated',
      signInFix: 'Run `claude login`.',
    }))).toBe('Run `claude login`.');
  });

  it('falls back to a generic probe-failure message', () => {
    expect(queryProviderTestMessage('claude', testResult({ errorCode: 'probe_failed' }))).toMatch(/probe failed/);
  });
});

describe('queryCommand', () => {
  it('extracts exactly the command-relevant settings fields', () => {
    const settings = {
      ...DEFAULT_SETTINGS,
      queryProvider: 'claude' as const,
      queryExecutable: '/usr/local/bin/claude',
      queryArguments: ['--print'],
      queryTimeoutSeconds: 60,
      retainQueryHistory: true,
    };
    expect(queryCommand(settings)).toEqual({
      provider: 'claude',
      executable: '/usr/local/bin/claude',
      arguments: ['--print'],
      timeoutSeconds: 60,
      contextLevel: settings.queryContextLevel,
      retainQueryHistory: true,
    });
  });
});

describe('queryCommandFingerprintFor', () => {
  it('changes when any tracked field changes', () => {
    const command = queryCommand(DEFAULT_SETTINGS);
    const base = queryCommandFingerprintFor(command, null);
    expect(queryCommandFingerprintFor({ ...command, executable: '/other' }, null)).not.toBe(base);
    expect(queryCommandFingerprintFor(command, 'alt_r')).not.toBe(base);
  });

  it('is stable for identical inputs', () => {
    const command = queryCommand(DEFAULT_SETTINGS);
    expect(queryCommandFingerprintFor(command, null)).toBe(queryCommandFingerprintFor(command, null));
  });
});

describe('QUERY_SWITCH_INTERRUPTED_NOTICE', () => {
  it('is a stable, non-empty notice string', () => {
    expect(QUERY_SWITCH_INTERRUPTED_NOTICE.length).toBeGreaterThan(0);
  });
});
