import { describe, expect, it } from 'vitest';

import {
  MAX_QUERY_ENV_VARS,
  presetSelection,
  probeSummary,
  queryEnvVarError,
  type QueryAuthProbeReport,
  type QueryPresetInfo,
} from './queryProviders';

const CLAUDE: QueryPresetInfo = {
  id: 'claude',
  label: 'Claude Code',
  summary: "Anthropic's CLI.",
  binaryName: 'claude',
  recommendedArguments: ['-p'],
  suggestedEnvKeys: ['CLAUDE_CONFIG_DIR'],
  loginHint: 'claude auth login',
  discoveredPath: '/opt/homebrew/bin/claude',
};

function report(overrides: Partial<QueryAuthProbeReport>): QueryAuthProbeReport {
  return {
    verdict: 'authenticated',
    exitCode: 0,
    output: '',
    truncated: false,
    durationMs: 12,
    loginHint: null,
    ...overrides,
  };
}

describe('presetSelection', () => {
  it('fills in the discovered path and the provider’s one-shot arguments', () => {
    expect(presetSelection(CLAUDE, '/old/path')).toEqual({
      executable: '/opt/homebrew/bin/claude',
      arguments: ['-p'],
    });
  });

  it('keeps a manually chosen path when the provider was not found', () => {
    // Re-selecting the same provider must not wipe a path the user browsed to
    // because Murmur cannot see that install location.
    expect(presetSelection({ ...CLAUDE, discoveredPath: null }, '/custom/claude').executable)
      .toBe('/custom/claude');
  });

  it('hands back a copy so editing the arguments cannot mutate the preset', () => {
    const selection = presetSelection(CLAUDE, '');
    selection.arguments.push('--dangerous');
    expect(CLAUDE.recommendedArguments).toEqual(['-p']);
  });
});

describe('probeSummary', () => {
  it('states the verdict plainly for each outcome', () => {
    expect(probeSummary(report({ verdict: 'authenticated' }), 'Claude Code'))
      .toBe('Claude Code is signed in and ready.');
    expect(probeSummary(report({ verdict: 'not_authenticated', exitCode: 1 }), 'Claude Code'))
      .toBe('Claude Code is not signed in.');
  });

  it('never reads an inconclusive check as success', () => {
    const unknown = probeSummary(report({ verdict: 'unknown', exitCode: 7 }), 'Codex');
    expect(unknown).toContain('exit code 7');
    expect(unknown).not.toContain('signed in and ready');
    expect(probeSummary(report({ verdict: 'unknown', exitCode: null }), 'Codex'))
      .toContain('did not report a result');
  });
});

describe('queryEnvVarError', () => {
  it('accepts the configuration pairs the presets suggest', () => {
    expect(queryEnvVarError([
      { name: 'CLAUDE_CONFIG_DIR', value: '/Users/someone/.claude' },
      { name: 'CODEX_HOME', value: '/Users/someone/.codex' },
    ])).toBeNull();
    expect(queryEnvVarError([])).toBeNull();
  });

  it('refuses HOME and the rest of the inherited allowlist', () => {
    for (const name of ['HOME', 'PATH', 'USER', 'LOGNAME', 'TMPDIR', 'LANG', 'LC_ALL', 'LC_CTYPE']) {
      expect(queryEnvVarError([{ name, value: '/tmp/anything' }])).toContain(name);
    }
  });

  it('refuses dynamic-linker injection, malformed names, and duplicates', () => {
    expect(queryEnvVarError([{ name: 'DYLD_INSERT_LIBRARIES', value: '/tmp/x.dylib' }])).not.toBeNull();
    expect(queryEnvVarError([{ name: 'LD_PRELOAD', value: '/tmp/x.so' }])).not.toBeNull();
    expect(queryEnvVarError([{ name: '1BAD', value: 'x' }])).not.toBeNull();
    expect(queryEnvVarError([{ name: 'HAS SPACE', value: 'x' }])).not.toBeNull();
    expect(queryEnvVarError([{ name: '', value: 'x' }])).not.toBeNull();
    expect(queryEnvVarError([
      { name: 'CODEX_HOME', value: '/a' },
      { name: 'CODEX_HOME', value: '/b' },
    ])).toContain('more than once');
  });

  it('enforces the same count and value bounds as the Rust validator', () => {
    const tooMany = Array.from({ length: MAX_QUERY_ENV_VARS + 1 }, (_, index) => ({
      name: `VAR_${index}`,
      value: 'x',
    }));
    expect(queryEnvVarError(tooMany)).not.toBeNull();
    expect(queryEnvVarError([{ name: 'CODEX_HOME', value: 'a'.repeat(4097) }])).not.toBeNull();
  });
});
