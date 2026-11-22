import { describe, expect, it, vi } from 'vitest';

const invoke = vi.hoisted(() => vi.fn(async () => undefined));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import { buildConfigureOptions, cancelRecording, setAgentCommandShortcut, startRecording, stopRecording } from './dictation';
import { DEFAULT_SETTINGS } from './settings';

describe('buildConfigureOptions', () => {
  it('sends the opt-in homophone tier to the recording configuration', () => {
    expect(buildConfigureOptions(DEFAULT_SETTINGS).homophoneContextEnabled).toBe(false);
    expect(buildConfigureOptions({ ...DEFAULT_SETTINGS, homophoneContextEnabled: true }).homophoneContextEnabled).toBe(true);
  });

  it('sends the experimental streaming pipeline setting, off by default', () => {
    expect(buildConfigureOptions(DEFAULT_SETTINGS).streamingPipelineEnabled).toBe(false);
    expect(buildConfigureOptions({ ...DEFAULT_SETTINGS, streamingPipelineEnabled: true }).streamingPipelineEnabled).toBe(true);
  });

  it('sends mute speakers while dictating, off by default', () => {
    expect(buildConfigureOptions(DEFAULT_SETTINGS).muteSpeakersWhileDictating).toBe(false);
    expect(buildConfigureOptions({ ...DEFAULT_SETTINGS, muteSpeakersWhileDictating: true }).muteSpeakersWhileDictating).toBe(true);
  });

  it('sends smart formatting and its independent per-app override to Rust', () => {
    const options = buildConfigureOptions({
      ...DEFAULT_SETTINGS,
      smartPunctuation: false,
      smartFormattingEnabled: true,
      smartDestinationEnabled: true,
      appProfiles: [
        {
          bundleId: 'com.apple.Terminal',
          label: 'Terminal',
          autoPasteOverride: null,
          cleanupOverride: null,
          smartFormattingOverride: false,
          cliFormattingOverride: true,
          writingStyle: 'code_technical',
          ideContextEnabled: false,
          ideProjectRoots: [],
        },
      ],
    });

    expect(options.smartFormattingEnabled).toBe(true);
    expect(options.smartDestinationEnabled).toBe(true);
    expect(options.smartPunctuation).toBe(false);
    expect(options.appProfiles?.[0].smartFormattingOverride).toBe(false);
    expect(options.appProfiles?.[0].writingStyle).toBe('code_technical');
  });
});

describe('cancelRecording', () => {
  it('scopes cancellation to an expected generation only when one is given', async () => {
    await cancelRecording();
    await cancelRecording(9);
    expect(invoke).toHaveBeenNthCalledWith(1, 'cancel_native_recording');
    expect(invoke).toHaveBeenNthCalledWith(2, 'cancel_native_recording', { expectedRecordingId: 9 });
  });
});

describe('startRecording gesture correlation', () => {
  it('carries only the originating keyboard token; a later toolbar call omits it', async () => {
    invoke.mockClear();
    await startRecording(undefined, 'toggle', null, '18446744073709551614');
    await startRecording();
    expect(invoke).toHaveBeenNthCalledWith(1, 'start_native_recording', {
      deviceName: null, smartAuto: null, origin: 'toggle', gestureToken: '18446744073709551614',
    });
    expect(invoke).toHaveBeenNthCalledWith(2, 'start_native_recording', {
      deviceName: null, smartAuto: null, origin: 'toggle',
    });
  });
});

describe('agent commands', () => {
  it('starts an agent command by its origin, with no other argument', async () => {
    invoke.mockClear();
    await startRecording(undefined, 'agent_command');
    await startRecording(undefined, 'hold');
    expect(invoke).toHaveBeenNthCalledWith(1, 'start_native_recording', {
      deviceName: null, smartAuto: null, origin: 'agent_command',
    });
    expect(invoke).toHaveBeenNthCalledWith(2, 'start_native_recording', {
      deviceName: null, smartAuto: null, origin: 'hold',
    });
  });

  it('binds and unbinds the agent command key', async () => {
    invoke.mockClear();
    await setAgentCommandShortcut('alt_r');
    await setAgentCommandShortcut(null);
    expect(invoke).toHaveBeenNthCalledWith(1, 'set_agent_command_shortcut', { shortcut: 'alt_r' });
    expect(invoke).toHaveBeenNthCalledWith(2, 'set_agent_command_shortcut', { shortcut: null });
  });

  it('configures the dispatcher but not the key, which the native listener owns', () => {
    const options = buildConfigureOptions({
      ...DEFAULT_SETTINGS,
      agentCommandEnabled: true,
      agentCommandDispatchProject: '/tmp/agent-dispatch',
    });
    expect(options).toMatchObject({
      agentCommandEnabled: true,
      agentCommandDispatchProject: '/tmp/agent-dispatch',
      agentCommandProvider: 'claude_cpa',
      agentCommandModel: 'claude-sonnet-5-5',
    });
    expect(options).not.toHaveProperty('agentCommandShortcut');
  });
});

describe('stopRecording', () => {
  it('sends the stop trigger explicitly and keeps legacy callers conservative', async () => {
    await stopRecording('toggle');
    expect(invoke).toHaveBeenLastCalledWith('stop_native_recording', { stopTrigger: 'toggle' });
    await stopRecording();
    expect(invoke).toHaveBeenLastCalledWith('stop_native_recording', { stopTrigger: 'other' });
  });
});
