import { beforeEach, describe, expect, it, vi } from 'vitest';
import { getMeetingAudioManifest, readMeetingAudioChunk } from './meetingAudio';

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
beforeEach(() => { mocks.invoke.mockReset(); });

function deferred<T>() {
  let resolve: (value: T) => void = () => {};
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}
async function settle() { for (let i = 0; i < 20; i += 1) await Promise.resolve(); }

describe('bounded retained audio IPC', () => {
  it('assembles binary ranges up to the exact file ceiling and checks EOF once', async () => {
    mocks.invoke.mockImplementation(async (_command, request) => new ArrayBuffer(request.offsetBytes < 512 * 1024 ? 64 * 1024 : 0));
    const result = await readMeetingAudioChunk('session', 1, () => true);
    expect(result?.byteLength).toBe(512 * 1024);
    expect(mocks.invoke).toHaveBeenCalledTimes(9);
    expect(mocks.invoke).toHaveBeenLastCalledWith('read_meeting_audio_range', {
      sessionId: 'session', segmentId: 1, offsetBytes: 512 * 1024, lengthBytes: 64 * 1024,
    });
  });

  it('rejects oversized and JSON-encoded responses instead of widening the memory bound', async () => {
    mocks.invoke.mockResolvedValue(new ArrayBuffer(64 * 1024 + 1));
    await expect(readMeetingAudioChunk('session', 1, () => true)).rejects.toThrow('Invalid meeting audio range');
    mocks.invoke.mockResolvedValue([1, 2, 3]);
    await expect(readMeetingAudioChunk('session', 1, () => true)).rejects.toThrow('Invalid meeting audio range');
    mocks.invoke.mockResolvedValue(new ArrayBuffer(64 * 1024));
    await expect(readMeetingAudioChunk('session', 1, () => true)).rejects.toThrow('Invalid meeting audio range');
  });

  it('limits all IPC across old and new sessions to two readers and drops cancelled work', async () => {
    const first = deferred<ArrayBuffer>();
    const second = deferred<ArrayBuffer>();
    mocks.invoke.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise).mockResolvedValue(new ArrayBuffer(8));
    let current = true;
    const old = readMeetingAudioChunk('old', 1, () => current);
    const other = readMeetingAudioChunk('old', 2, () => current);
    const queued = readMeetingAudioChunk('old', 3, () => current);
    const next = readMeetingAudioChunk('new', 4, () => true);
    await settle();
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
    current = false;
    first.resolve(new ArrayBuffer(64 * 1024));
    second.resolve(new ArrayBuffer(64 * 1024));
    expect(await old).toBeNull();
    expect(await other).toBeNull();
    expect(await queued).toBeNull();
    expect((await next)?.byteLength).toBe(8);
    expect(mocks.invoke).toHaveBeenCalledTimes(3);
    expect(mocks.invoke.mock.calls.map(call => call[1].segmentId)).toEqual([1, 2, 4]);
  });

  it('validates metadata identity, ordering and cursor progress at the IPC boundary', async () => {
    const request = { sessionId: 'session', fromMs: 0, channel: 'all', cursor: null, limit: 128 } satisfies Parameters<typeof getMeetingAudioManifest>[0];
    const manifest = {
      kind: 'available', sessionId: 'session', durationMs: 1000,
      chunks: [{ segmentId: 1, channel: 'me', startMs: 0, endMs: 1000 }], nextCursor: null,
    };
    mocks.invoke.mockResolvedValue(manifest);
    await expect(getMeetingAudioManifest(request)).resolves.toEqual(manifest);
    mocks.invoke.mockResolvedValue({ ...manifest, sessionId: 'other' });
    await expect(getMeetingAudioManifest(request)).rejects.toThrow();
    mocks.invoke.mockResolvedValue({ ...manifest, nextCursor: { segmentId: 4, startMs: 2000 } });
    await expect(getMeetingAudioManifest(request)).rejects.toThrow('Invalid meeting audio cursor');
    mocks.invoke.mockResolvedValue(manifest);
    await expect(getMeetingAudioManifest({ ...request, cursor: { startMs: 0, segmentId: 1 } })).rejects.toThrow('Invalid meeting audio page');
  });
});
