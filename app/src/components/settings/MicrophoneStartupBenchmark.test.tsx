import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { MicrophoneStartupBenchmarkReport } from '../../lib/microphoneStartupBenchmark';

const mocks = vi.hoisted(() => {
  const listeners = new Map<string, (event: { payload: unknown }) => void>();
  return {
    listeners,
    invoke: vi.fn(),
    listen: vi.fn(async (event: string, handler: (event: { payload: unknown }) => void) => {
      listeners.set(event, handler);
      return () => listeners.delete(event);
    }),
  };
});

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));

import { MicrophoneStartupBenchmark } from './MicrophoneStartupBenchmark';

const RUN_ID = '888d2ca8-7f8c-4f1c-bc52-a60ee41ff901';

function report(overrides: Partial<MicrophoneStartupBenchmarkReport> = {}): MicrophoneStartupBenchmarkReport {
  return {
    schemaVersion: 1,
    runId: RUN_ID,
    benchmarkRunId: 41,
    appVersion: '0.34.0',
    platform: 'macos',
    deviceSelection: 'pinned',
    requestedCycles: 5,
    completedCycles: 1,
    cancelled: true,
    startedAt: '2026-08-14T12:00:00.000Z',
    finishedAt: '2026-08-14T12:00:01.000Z',
    cycles: [{
      cycle: 1,
      outcome: 'ready',
      cycleStartToFirstPcmMs: 310,
      backend: 'cpal',
      backendOrder: ['auhal', 'cpal'],
      backendOrderSource: 'session_first_pcm_memo',
      fallbackOccurred: true,
      failureKind: null,
      lastSetupStep: 'awaiting_first_callback',
      lastSetupTransition: 'completed',
      attempts: [
        {
          resolutionPass: 1,
          attemptIndex: 1,
          backend: 'auhal',
          outcome: 'failed',
          attemptStartToFirstPcmMs: null,
          activeElapsedMs: 140,
          failureKind: 'backend_error',
          failurePhase: 'stream_build',
          lastSetupStep: 'stream_build',
          lastSetupTransition: 'entered',
          attemptBudgetMs: 8_000,
        },
        {
          resolutionPass: 1,
          attemptIndex: 2,
          backend: 'cpal',
          outcome: 'ready',
          attemptStartToFirstPcmMs: 160,
          activeElapsedMs: 165,
          failureKind: null,
          failurePhase: null,
          lastSetupStep: 'awaiting_first_callback',
          lastSetupTransition: 'completed',
          attemptBudgetMs: 16_000,
        },
      ],
    }],
    ...overrides,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe('MicrophoneStartupBenchmark', () => {
  let container: HTMLDivElement;
  let root: Root;
  let runningChanges: boolean[];
  let unmounted: boolean;

  async function render(overrides: Partial<Parameters<typeof MicrophoneStartupBenchmark>[0]> = {}) {
    await act(async () => {
      root.render(<MicrophoneStartupBenchmark
        status="idle"
        deviceId="missing-pinned-id"
        audioInventory={{
          schemaVersion: 1,
          revision: 1,
          status: 'available',
          devices: [],
          defaultInputId: null,
          errorCode: null,
        }}
        modelBenchmarkRunning={false}
        fileTranscribing={false}
        corpusBusy={false}
        outputDir=""
        autoSave={false}
        onRunningChange={(running) => runningChanges.push(running)}
        {...overrides}
      />);
      await Promise.resolve();
      await Promise.resolve();
    });
  }

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.listeners.clear();
    localStorage.clear();
    runningChanges = [];
    unmounted = false;
    vi.stubGlobal('crypto', { randomUUID: vi.fn(() => RUN_ID) });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    if (!unmounted) await act(async () => root.unmount());
    vi.unstubAllGlobals();
    container.remove();
  });

  it('runs a missing pinned device diagnostically and filters progress by both run identifiers', async () => {
    const run = deferred<MicrophoneStartupBenchmarkReport>();
    mocks.invoke.mockImplementation((command: string) => {
      if (command === 'run_microphone_startup_benchmark') return run.promise;
      throw new Error(`unexpected command: ${command}`);
    });
    await render();

    expect(container.textContent).toContain('Saved microphone (currently unavailable)');
    const button = Array.from(container.querySelectorAll('button'))
      .find((item) => item.textContent === 'Test microphone startup') as HTMLButtonElement;
    expect(button.disabled).toBe(false);
    await act(async () => button.click());

    expect(mocks.invoke).toHaveBeenCalledWith('run_microphone_startup_benchmark', {
      request: { runId: RUN_ID, deviceId: 'missing-pinned-id', cycles: 5 },
    });
    expect(runningChanges).toEqual([true]);

    const progress = {
      schemaVersion: 1,
      runId: RUN_ID,
      benchmarkRunId: 41,
      completedCycles: 1,
      totalCycles: 5,
      currentCycle: 2,
      phase: 'capturing',
      backend: 'cpal',
      fallbackOccurred: true,
      lastSetupStep: 'stream_build',
      lastSetupTransition: 'entered',
    };
    await act(async () => {
      mocks.listeners.get('microphone-startup-benchmark-progress')?.({
        payload: { ...progress, runId: 'older-run' },
      });
    });
    expect(container.textContent).not.toContain('Cycle 2 of 5');
    await act(async () => {
      mocks.listeners.get('microphone-startup-benchmark-progress')?.({ payload: progress });
    });
    expect(container.textContent).toContain('Cycle 2 of 5');
    expect(container.textContent).toContain('CPAL');
    expect(container.textContent).toContain('fallback');

    await act(async () => {
      mocks.listeners.get('microphone-startup-benchmark-progress')?.({
        payload: { ...progress, benchmarkRunId: 42, currentCycle: 4 },
      });
    });
    expect(container.textContent).toContain('Cycle 2 of 5');
    expect(container.textContent).not.toContain('Cycle 4 of 5');

    await act(async () => run.resolve(report()));
    expect(runningChanges).toEqual([true, false]);
    expect(container.textContent).toContain('Startup results');
    expect(container.textContent).toContain('1/5 cycles · cancelled early');
    expect(container.textContent).toContain('AUHAL won 0; CPAL won 1');
    expect(container.textContent).toContain('P1 AUHAL · failed');
    expect(container.textContent).toContain('P1 CPAL · ready in 160 ms');
  });

  it('keeps Run disabled until the correlated progress listener is installed', async () => {
    const listener = deferred<() => boolean>();
    mocks.listen.mockImplementationOnce(() => listener.promise);
    await render();
    const button = Array.from(container.querySelectorAll('button'))
      .find((item) => item.textContent === 'Test microphone startup') as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(container.textContent).toContain('Preparing microphone diagnostics…');

    await act(async () => listener.resolve(() => false));
    expect(button.disabled).toBe(false);
  });

  it('cancels only the exact active run and stays busy until the run resolves post-recovery', async () => {
    const run = deferred<MicrophoneStartupBenchmarkReport>();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === 'run_microphone_startup_benchmark') return run.promise;
      if (command === 'cancel_microphone_startup_benchmark') return Promise.resolve(true);
      throw new Error(`unexpected command: ${command} ${String(args)}`);
    });
    await render();
    const runButton = Array.from(container.querySelectorAll('button'))
      .find((item) => item.textContent === 'Test microphone startup') as HTMLButtonElement;
    await act(async () => runButton.click());
    const cancelButton = Array.from(container.querySelectorAll('button'))
      .find((item) => item.textContent === 'Cancel microphone test') as HTMLButtonElement;
    await act(async () => cancelButton.click());

    expect(mocks.invoke).toHaveBeenCalledWith('cancel_microphone_startup_benchmark', { runId: RUN_ID });
    expect(container.textContent).toContain('Stopping safely…');
    expect(runningChanges).toEqual([true]);

    await act(async () => run.resolve(report({ completedCycles: 0, cycles: [] })));
    expect(runningChanges).toEqual([true, false]);
    expect(container.textContent).toContain('cancelled before its first cycle completed');
  });

  it('cancels the exact active run once when the component unmounts', async () => {
    const run = deferred<MicrophoneStartupBenchmarkReport>();
    mocks.invoke.mockImplementation((command: string) => {
      if (command === 'run_microphone_startup_benchmark') return run.promise;
      if (command === 'cancel_microphone_startup_benchmark') return Promise.resolve(true);
      throw new Error(`unexpected command: ${command}`);
    });
    await render();
    const runButton = Array.from(container.querySelectorAll('button'))
      .find((item) => item.textContent === 'Test microphone startup') as HTMLButtonElement;
    await act(async () => runButton.click());

    await act(async () => root.unmount());
    unmounted = true;
    expect(mocks.invoke.mock.calls.filter(([command]) => command === 'cancel_microphone_startup_benchmark'))
      .toEqual([['cancel_microphone_startup_benchmark', { runId: RUN_ID }]]);
  });

  it('shows local refusal reasons before invoking Rust', async () => {
    await render({ status: 'recording' });
    expect(container.textContent).toContain('Finish the current recording first.');
    const button = Array.from(container.querySelectorAll('button'))
      .find((item) => item.textContent === 'Test microphone startup') as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it('rejects malformed command output and does not persist it', async () => {
    mocks.invoke.mockResolvedValue({ ...report(), rawDeviceId: 'secret-id' });
    await render();
    const button = Array.from(container.querySelectorAll('button'))
      .find((item) => item.textContent === 'Test microphone startup') as HTMLButtonElement;
    await act(async () => button.click());
    expect(container.textContent).toContain('invalid microphone startup report');
    expect(localStorage.getItem('murmur-microphone-startup-benchmark-reports')).toBeNull();
  });

  it('does not auto-save cancelled partial results', async () => {
    mocks.invoke.mockResolvedValue(report());
    await render({ autoSave: true });
    const button = Array.from(container.querySelectorAll('button'))
      .find((item) => item.textContent === 'Test microphone startup') as HTMLButtonElement;
    await act(async () => button.click());
    expect(mocks.invoke.mock.calls.map(([command]) => command))
      .not.toContain('save_microphone_startup_benchmark_report');
    expect(localStorage.getItem('murmur-microphone-startup-benchmark-reports')).toBeNull();
  });

  it('auto-saves only a complete report through the dedicated typed command', async () => {
    const complete = report({ requestedCycles: 1, completedCycles: 1, cancelled: false });
    mocks.invoke.mockImplementation((command: string) => {
      if (command === 'run_microphone_startup_benchmark') return Promise.resolve(complete);
      if (command === 'save_microphone_startup_benchmark_report') return Promise.resolve('/tmp/report.json');
      throw new Error(`unexpected command: ${command}`);
    });
    await render({ autoSave: true, outputDir: '/tmp/reports' });
    const button = Array.from(container.querySelectorAll('button'))
      .find((item) => item.textContent === 'Test microphone startup') as HTMLButtonElement;
    await act(async () => button.click());
    expect(mocks.invoke).toHaveBeenCalledWith('save_microphone_startup_benchmark_report', {
      report: complete,
      outputDir: '/tmp/reports',
    });
  });

  it('does not persist an earlier partial report when a later full run completes', async () => {
    const secondRunId = '11111111-1111-4111-8111-111111111111';
    const complete = report({
      runId: secondRunId,
      benchmarkRunId: 42,
      requestedCycles: 1,
      completedCycles: 1,
      cancelled: false,
      startedAt: '2026-08-14T12:01:00.000Z',
      finishedAt: '2026-08-14T12:01:01.000Z',
    });
    vi.mocked(crypto.randomUUID)
      .mockReturnValueOnce(RUN_ID)
      .mockReturnValueOnce(secondRunId);
    mocks.invoke
      .mockResolvedValueOnce(report())
      .mockResolvedValueOnce(complete);
    await render();
    const runButton = Array.from(container.querySelectorAll('button'))
      .find((item) => item.textContent === 'Test microphone startup') as HTMLButtonElement;

    await act(async () => runButton.click());
    await act(async () => runButton.click());

    const saved = JSON.parse(localStorage.getItem('murmur-microphone-startup-benchmark-reports') ?? '[]');
    expect(saved).toEqual([complete]);
  });
});
