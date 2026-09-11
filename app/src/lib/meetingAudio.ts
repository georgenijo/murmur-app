import { invoke } from '@tauri-apps/api/core';
import type { MeetingSpeaker } from './meetings';

export type MeetingAudioChannel = 'all' | MeetingSpeaker;
export type MeetingAudioUnavailableReason = 'notRetained' | 'notFinished' | 'noAudio';
export interface MeetingAudioCursor { startMs: number; segmentId: number }
export interface MeetingAudioChunk extends MeetingAudioCursor { channel: MeetingSpeaker; endMs: number }
export type MeetingAudioManifest =
  | { kind: 'unavailable'; reason: MeetingAudioUnavailableReason }
  | { kind: 'available'; sessionId: string; durationMs: number; chunks: MeetingAudioChunk[]; nextCursor: MeetingAudioCursor | null };
export interface MeetingAudioManifestRequest {
  sessionId: string;
  fromMs: number;
  channel: MeetingAudioChannel;
  cursor: MeetingAudioCursor | null;
  limit: number;
}

export async function getMeetingAudioCaptureBusy(): Promise<boolean> {
  const value = await invoke<unknown>('get_meeting_audio_capture_busy');
  if (typeof value !== 'boolean') throw new Error('Invalid capture ownership response');
  return value;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
function unsigned(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
}
function cursor(value: unknown): value is MeetingAudioCursor {
  return record(value) && unsigned(value.startMs) && unsigned(value.segmentId);
}
function chunk(value: unknown): value is MeetingAudioChunk {
  return record(value) && cursor(value) && 'endMs' in value && unsigned(value.endMs)
    && value.endMs > value.startMs && 'channel' in value && (value.channel === 'me' || value.channel === 'them');
}

export async function getMeetingAudioManifest(request: MeetingAudioManifestRequest): Promise<MeetingAudioManifest> {
  const release = await acquireReader();
  let value: unknown;
  try { value = await invoke<unknown>('get_meeting_audio_manifest', { ...request }); }
  finally { release(); }
  if (record(value) && value.kind === 'unavailable'
    && (value.reason === 'notRetained' || value.reason === 'notFinished' || value.reason === 'noAudio')) {
    return { kind: 'unavailable', reason: value.reason };
  }
  if (record(value) && value.kind === 'available' && value.sessionId === request.sessionId
    && unsigned(value.durationMs) && Array.isArray(value.chunks) && value.chunks.length <= request.limit
    && value.chunks.every(chunk) && (value.nextCursor === null || cursor(value.nextCursor))) {
    const chunks: MeetingAudioChunk[] = value.chunks;
    const durationMs = value.durationMs;
    if (chunks.some((item, index) => item.endMs > durationMs
      || item.endMs <= request.fromMs || (request.channel !== 'all' && item.channel !== request.channel)
      || (index > 0 && (chunks[index - 1].startMs > item.startMs
        || (chunks[index - 1].startMs === item.startMs && chunks[index - 1].segmentId >= item.segmentId))))) {
      throw new Error('Invalid meeting audio metadata');
    }
    const last = chunks[chunks.length - 1];
    if (value.nextCursor !== null && (!last || value.nextCursor.startMs !== last.startMs
      || value.nextCursor.segmentId !== last.segmentId)) throw new Error('Invalid meeting audio cursor');
    const previous = request.cursor;
    if (previous && chunks.some(item => item.startMs < previous.startMs
      || (item.startMs === previous.startMs && item.segmentId <= previous.segmentId))) {
      throw new Error('Invalid meeting audio page');
    }
    return { kind: 'available', sessionId: request.sessionId, durationMs: value.durationMs, chunks, nextCursor: value.nextCursor };
  }
  throw new Error('Invalid meeting audio response');
}

const RANGE_BYTES = 64 * 1024;
const MAX_WAV_BYTES = 512 * 1024;
let readers = 0;
const waitingReaders: (() => void)[] = [];

async function acquireReader(): Promise<() => void> {
  if (readers >= 2) await new Promise<void>(resolve => waitingReaders.push(resolve));
  else readers += 1;
  return () => {
    const next = waitingReaders.shift();
    if (next) next();
    else readers -= 1;
  };
}

export async function readMeetingAudioChunk(
  sessionId: string, segmentId: number, current: () => boolean,
): Promise<ArrayBuffer | null> {
  const parts: Uint8Array[] = [];
  let size = 0;
  for (let request = 0; request < 9; request += 1) {
    const release = await acquireReader();
    let value: unknown;
    try {
      if (!current()) return null;
      value = await invoke<unknown>('read_meeting_audio_range', {
        sessionId, segmentId, offsetBytes: size, lengthBytes: RANGE_BYTES,
      });
    } finally { release(); }
    if (!current()) return null;
    if (!(value instanceof ArrayBuffer) || value.byteLength > RANGE_BYTES
      || size + value.byteLength > MAX_WAV_BYTES) throw new Error('Invalid meeting audio range');
    parts.push(new Uint8Array(value));
    size += value.byteLength;
    if (value.byteLength < RANGE_BYTES) {
      if (size === 0) throw new Error('Meeting audio is missing');
      const wav = new Uint8Array(size);
      let offset = 0;
      for (const part of parts) { wav.set(part, offset); offset += part.byteLength; }
      return wav.buffer;
    }
  }
  throw new Error('Meeting audio exceeds the chunk limit');
}
