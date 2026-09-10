import { describe, expect, it, vi } from 'vitest';
import { pollQuerySignIn } from './queryProviders';

function fakeSleep(calls: number[]) {
  return async (ms: number) => {
    calls.push(ms);
  };
}

describe('pollQuerySignIn', () => {
  it('launches, probes on an interval, and resolves once a probe result is signed in', async () => {
    const sleeps: number[] = [];
    let probeCount = 0;
    const probeResults: boolean[] = [];
    const onLaunched = vi.fn();
    const onSignedIn = vi.fn();
    const onPending = vi.fn();

    await pollQuerySignIn<boolean>({
      launch: async () => {},
      onLaunched,
      probe: async () => {
        probeCount += 1;
        const signedIn = probeCount >= 3;
        probeResults.push(signedIn);
        return signedIn;
      },
      isSignedIn: (result) => result,
      onSignedIn,
      onPending,
      ownsAttempt: () => true,
      intervalMs: 2000,
      timeoutMs: 60_000,
      now: () => 0,
      sleep: fakeSleep(sleeps),
    });

    expect(probeCount).toBe(3);
    expect(probeResults).toEqual([false, false, true]);
    expect(onLaunched).toHaveBeenCalledTimes(1);
    expect(onSignedIn).toHaveBeenCalledTimes(1);
    expect(onPending).not.toHaveBeenCalled();
    expect(sleeps).toEqual([2000, 2000, 2000]);
  });

  it('calls onPending once the deadline elapses without a signed-in result', async () => {
    let clock = 0;
    const onPending = vi.fn();
    const onSignedIn = vi.fn();

    await pollQuerySignIn<boolean>({
      launch: async () => {},
      probe: async () => false,
      isSignedIn: (result) => result,
      onSignedIn,
      onPending,
      ownsAttempt: () => true,
      intervalMs: 2000,
      timeoutMs: 5000,
      now: () => {
        const value = clock;
        clock += 2000;
        return value;
      },
      sleep: async () => {},
    });

    expect(onSignedIn).not.toHaveBeenCalled();
    expect(onPending).toHaveBeenCalledTimes(1);
  });

  it('stops silently and skips onLaunched once ownership is lost right after launch', async () => {
    let owns = true;
    const onLaunched = vi.fn();
    const onPending = vi.fn();
    const probe = vi.fn(async () => true);

    await pollQuerySignIn<boolean>({
      launch: async () => { owns = false; },
      onLaunched,
      probe,
      isSignedIn: (result) => result,
      onPending,
      ownsAttempt: () => owns,
      sleep: async () => {},
    });

    expect(onLaunched).not.toHaveBeenCalled();
    expect(probe).not.toHaveBeenCalled();
    expect(onPending).not.toHaveBeenCalled();
  });

  it('stops silently and skips onSignedIn/onPending once ownership is lost after a probe result', async () => {
    let owns = true;
    let probeCount = 0;
    const onSignedIn = vi.fn();
    const onPending = vi.fn();
    const onProbeResult = vi.fn();

    await pollQuerySignIn<boolean>({
      launch: async () => {},
      probe: async () => {
        probeCount += 1;
        owns = false;
        return true;
      },
      isSignedIn: (result) => result,
      onProbeResult,
      onSignedIn,
      onPending,
      ownsAttempt: () => owns,
      sleep: async () => {},
    });

    expect(probeCount).toBe(1);
    expect(onProbeResult).not.toHaveBeenCalled();
    expect(onSignedIn).not.toHaveBeenCalled();
    expect(onPending).not.toHaveBeenCalled();
  });

  it('propagates a launch error to the caller without calling onLaunched or probe', async () => {
    const onLaunched = vi.fn();
    const probe = vi.fn(async () => true);

    await expect(pollQuerySignIn<boolean>({
      launch: async () => { throw new Error('launch failed'); },
      onLaunched,
      probe,
      isSignedIn: (result) => result,
      ownsAttempt: () => true,
      sleep: async () => {},
    })).rejects.toThrow('launch failed');

    expect(onLaunched).not.toHaveBeenCalled();
    expect(probe).not.toHaveBeenCalled();
  });

  it('propagates a probe error to the caller', async () => {
    await expect(pollQuerySignIn<boolean>({
      launch: async () => {},
      probe: async () => { throw new Error('probe failed'); },
      isSignedIn: (result) => result,
      ownsAttempt: () => true,
      sleep: async () => {},
    })).rejects.toThrow('probe failed');
  });
});
