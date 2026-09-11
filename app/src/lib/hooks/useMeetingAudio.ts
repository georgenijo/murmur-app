import { useEffect, useMemo, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import {
  getMeetingAudioCaptureBusy, getMeetingAudioManifest, readMeetingAudioChunk,
  type MeetingAudioChannel, type MeetingAudioChunk, type MeetingAudioCursor,
  type MeetingAudioManifest, type MeetingAudioManifestRequest, type MeetingAudioUnavailableReason,
} from '../meetingAudio';
import type { MeetingSpeaker } from '../meetings';

export interface MeetingAudioState {
  status: 'loading' | 'ready' | 'buffering' | 'playing' | 'paused' | 'unavailable' | 'error';
  unavailableReason: MeetingAudioUnavailableReason | null;
  error: string | null;
  durationMs: number;
  positionMs: number;
  channel: MeetingAudioChannel;
}

interface PlaybackDependencies {
  manifest(request: MeetingAudioManifestRequest): Promise<MeetingAudioManifest>;
  read(sessionId: string, segmentId: number, current: () => boolean): Promise<ArrayBuffer | null>;
  context(): AudioContext;
  captureBusy(): Promise<boolean>;
}
interface Track {
  channel: MeetingSpeaker;
  chunks: MeetingAudioChunk[];
  cursor: MeetingAudioCursor | null;
  loading: boolean;
  pending: MeetingAudioChunk | null;
  pageEndMs: number;
  sources: Map<number, { source: AudioBufferSourceNode | null; chunk: MeetingAudioChunk; buffer: AudioBuffer }>;
}

const INITIAL_STATE: MeetingAudioState = {
  status: 'loading', unavailableReason: null, error: null, durationMs: 0, positionMs: 0, channel: 'all',
};
const LOOKAHEAD_MS = 2000;
const dependencies: PlaybackDependencies = {
  manifest: getMeetingAudioManifest,
  read: readMeetingAudioChunk,
  context: () => new AudioContext(),
  captureBusy: getMeetingAudioCaptureBusy,
};

export class MeetingAudioPlayback {
  state: MeetingAudioState = { ...INITIAL_STATE };
  private generation = 0;
  private context: AudioContext | null = null;
  private tracks: Track[] = [];
  private anchor = 0;
  private timer: ReturnType<typeof setInterval> | null = null;
  private busy = false;
  private disposed = false;
  private invalidated = false;
  private monitoredBusy = false;
  private monitoringReady = true;
  private stalled = false;
  private monitoringError = false;
  private manifestRequest = 0;
  private refreshWhenIdle = false;

  constructor(
    private readonly sessionId: string | null,
    private readonly changed: (state: MeetingAudioState) => void,
    private readonly io: PlaybackDependencies = dependencies,
  ) {}

  private update(patch: Partial<MeetingAudioState>) {
    if (this.disposed) return;
    this.state = { ...this.state, ...patch };
    this.changed(this.state);
  }

  async load() {
    if (this.disposed) {
      this.disposed = false;
      this.state = { ...INITIAL_STATE };
    }
    const generation = this.generation;
    const request = ++this.manifestRequest;
    if (!this.sessionId) {
      this.update({ status: 'unavailable', unavailableReason: 'notRetained' });
      return;
    }
    try {
      const manifest = await this.io.manifest({ sessionId: this.sessionId, fromMs: 0, channel: 'all', cursor: null, limit: 1 });
      if (!this.current(generation) || request !== this.manifestRequest) return;
      if (manifest.kind === 'unavailable') {
        this.update({ status: 'unavailable', unavailableReason: manifest.reason });
      } else this.update({ status: 'ready', durationMs: manifest.durationMs, error: null, unavailableReason: null });
    } catch {
      if (this.current(generation) && request === this.manifestRequest) this.fail('Retained meeting audio could not be opened.');
    }
  }

  private current(generation: number) { return !this.disposed && !this.invalidated && this.generation === generation; }
  private position() {
    return this.state.status === 'playing' && this.context
      ? Math.min(this.state.durationMs, Math.max(0, (this.context.currentTime - this.anchor) * 1000))
      : this.state.positionMs;
  }
  private stop() {
    const positionMs = this.position();
    this.generation += 1;
    if (this.timer !== null) clearInterval(this.timer);
    this.timer = null;
    this.stalled = false;
    for (const track of this.tracks) {
      for (const { source } of track.sources.values()) {
        this.stopSource(source);
      }
      track.sources.clear();
    }
    this.tracks = [];
    return positionMs;
  }
  private stopSource(source: AudioBufferSourceNode | null) {
    if (!source) return;
    source.onended = null;
    try { source.stop(); } catch { /* Already ended. */ }
    source.disconnect();
    source.buffer = null;
  }
  private fail(message: string) {
    const positionMs = this.stop();
    this.update({ status: 'error', error: message, positionMs });
  }
  setBusy(busy: boolean) {
    this.busy = busy;
    if (busy) this.pause();
    else this.refreshFinishedMeeting();
  }
  setMonitoredBusy(busy: boolean) {
    this.monitoredBusy = busy;
    if (busy) this.pause();
    else this.refreshFinishedMeeting();
  }
  beginMonitoring() {
    this.monitoringReady = false;
    this.setMonitoredBusy(true);
  }
  meetingFinished() {
    if (this.state.status === 'loading' || this.state.unavailableReason === 'notFinished') {
      this.refreshWhenIdle = true;
      this.refreshFinishedMeeting();
    }
  }
  private refreshFinishedMeeting() {
    if (!this.refreshWhenIdle || this.busy || this.monitoredBusy || this.disposed || this.invalidated) return;
    this.refreshWhenIdle = false;
    void this.load();
  }
  monitoringFailed() {
    this.monitoringReady = false;
    this.monitoredBusy = true;
    this.monitoringError = true;
    this.fail('Playback safety checks are unavailable. Reopen this meeting to retry.');
  }
  monitoringRecovered(busy: boolean) {
    this.monitoringReady = true;
    this.setMonitoredBusy(busy);
    if (this.monitoringError) {
      this.monitoringError = false;
      this.update({ status: 'paused', error: null });
      void this.load();
    }
  }
  pause = () => {
    if (this.state.status !== 'playing' && this.state.status !== 'buffering') return;
    const positionMs = this.stop();
    this.update({ status: 'paused', positionMs });
  };
  invalidate = () => {
    this.stop();
    this.invalidated = true;
    this.update({ status: 'unavailable', unavailableReason: 'noAudio', error: null });
  };
  dispose() {
    this.stop();
    this.disposed = true;
    void this.context?.close().catch(() => {});
    this.context = null;
  }
  playAll = () => this.start('all', 0);
  playSegment = (segment: { speaker: MeetingSpeaker; startMs: number }) => this.start(segment.speaker, segment.startMs);
  play = () => this.start(this.state.channel, this.state.positionMs >= this.state.durationMs ? 0 : this.state.positionMs);
  seek = (positionMs: number) => {
    if (!Number.isFinite(positionMs)) return;
    const position = Math.min(this.state.durationMs, Math.max(0, positionMs));
    if (this.state.status === 'playing' || this.state.status === 'buffering') this.start(this.state.channel, position);
    else this.update({ positionMs: position });
  };
  setChannel = (channel: MeetingAudioChannel) => {
    if (channel === this.state.channel) return;
    if (this.state.status === 'playing' || this.state.status === 'buffering') this.start(channel, this.position());
    else this.update({ channel });
  };

  private start(channel: MeetingAudioChannel, positionMs: number) {
    if (this.busy || !this.monitoringReady || this.disposed || this.invalidated || !this.sessionId
      || this.state.status === 'loading' || this.state.status === 'unavailable') return;
    this.stop();
    const generation = this.generation;
    this.update({ status: 'buffering', channel, positionMs: Math.min(this.state.durationMs, Math.max(0, positionMs)), error: null });
    try {
      this.context ??= this.io.context();
      // WebKit requires resume to originate in the click, before the first IPC await.
      const resumed = this.context.resume();
      void this.begin(generation, resumed).catch(() => {
        if (this.current(generation)) this.fail('Meeting audio could not be played. Try playing again.');
      });
    } catch { this.fail('Audio playback is unavailable. Try playing again.'); }
  }

  private async begin(generation: number, resumed: Promise<void>) {
    const [busy] = await Promise.all([this.io.captureBusy(), resumed]);
    if (!this.current(generation)) return;
    this.monitoredBusy = busy;
    if (busy || this.busy || !this.monitoringReady) { this.pause(); return; }
    const sessionId = this.sessionId;
    const context = this.context;
    if (!sessionId || !context || context.state !== 'running') throw new Error('Audio output is suspended');
    const channels: MeetingSpeaker[] = this.state.channel === 'all' ? ['me', 'them'] : [this.state.channel];
    const positionMs = this.state.positionMs;
    const tracks = await Promise.all(channels.map(async channel => {
      const manifest = await this.io.manifest({ sessionId, fromMs: positionMs, channel, cursor: null, limit: 128 });
      if (manifest.kind === 'unavailable') throw new Error('Meeting audio is unavailable');
      return {
        channel, chunks: manifest.chunks, cursor: manifest.nextCursor, loading: false, pending: null,
        pageEndMs: Math.max(positionMs, ...manifest.chunks.map(chunk => chunk.endMs)), sources: new Map(),
      } satisfies Track;
    }));
    if (!this.current(generation)) return;
    this.tracks = tracks;
    // Decode the initial audible chunks before starting one clock shared by both channels.
    const initial = await Promise.all(tracks.map(async track => {
      const chunk = track.chunks[0];
      if (!chunk || chunk.startMs > positionMs + LOOKAHEAD_MS) return null;
      track.chunks.shift();
      const buffer = await this.decode(chunk, generation, context);
      return buffer ? { track, chunk, buffer } : null;
    }));
    if (!this.current(generation)) return;
    const busyBeforeOutput = await this.io.captureBusy();
    if (!this.current(generation)) return;
    if (busyBeforeOutput || this.busy || !this.monitoringReady) { this.pause(); return; }
    this.anchor = context.currentTime - positionMs / 1000;
    this.update({ status: 'playing' });
    for (const item of initial) if (item) this.schedule(item.track, item.chunk, item.buffer, context);
    this.tick(generation);
    if (this.current(generation)) this.timer = setInterval(() => this.tick(generation), 100);
  }

  private async decode(chunk: MeetingAudioChunk, generation: number, context: AudioContext) {
    if (!this.sessionId) return null;
    const bytes = await this.io.read(this.sessionId, chunk.segmentId, () => this.current(generation));
    if (!bytes || !this.current(generation)) return null;
    const buffer = await context.decodeAudioData(bytes);
    if (!this.current(generation)) return null;
    if (buffer.numberOfChannels !== 1 || !Number.isFinite(buffer.duration) || buffer.duration <= 0
      || buffer.duration > 16) throw new Error('Invalid retained audio');
    return buffer;
  }
  private schedule(track: Track, chunk: MeetingAudioChunk, buffer: AudioBuffer, context: AudioContext) {
    if (this.stalled) {
      track.sources.set(chunk.segmentId, { source: null, chunk, buffer });
      return;
    }
    const positionMs = this.position();
    const offset = Math.max(0, (positionMs - chunk.startMs) / 1000);
    const duration = Math.min(buffer.duration, (chunk.endMs - chunk.startMs) / 1000) - offset;
    if (duration <= 0) return;
    const source = context.createBufferSource();
    source.buffer = buffer;
    source.connect(context.destination);
    track.sources.set(chunk.segmentId, { source, chunk, buffer });
    source.onended = () => {
      source.disconnect();
      source.buffer = null;
      track.sources.delete(chunk.segmentId);
    };
    source.start(Math.max(context.currentTime, this.anchor + chunk.startMs / 1000), offset, duration);
  }
  private missingAt(positionMs: number): number | null {
    const boundaries = this.tracks.flatMap(track => {
      const next = track.pending ?? track.chunks[0];
      const boundary = next ? next.startMs : track.cursor ? track.pageEndMs : null;
      return boundary !== null && boundary <= positionMs ? [boundary] : [];
    });
    return boundaries.length ? Math.max(this.state.positionMs, Math.min(...boundaries)) : null;
  }
  private bufferAt(positionMs: number) {
    this.stalled = true;
    this.update({ status: 'buffering', positionMs });
    for (const track of this.tracks) {
      for (const [id, item] of [...track.sources]) {
        this.stopSource(item.source);
        if (item.chunk.endMs <= positionMs) track.sources.delete(id);
        else item.source = null;
      }
    }
  }
  private resumeBuffered() {
    const context = this.context;
    if (!this.stalled || !context || this.missingAt(this.state.positionMs) !== null) return;
    this.anchor = context.currentTime - this.state.positionMs / 1000;
    this.stalled = false;
    this.update({ status: 'playing' });
    for (const track of this.tracks) {
      for (const [id, item] of [...track.sources]) {
        track.sources.delete(id);
        this.schedule(track, item.chunk, item.buffer, context);
      }
    }
  }
  private tick(generation: number) {
    if (!this.current(generation) || (this.state.status !== 'playing' && !this.stalled)) return;
    let positionMs = this.position();
    const missing = this.missingAt(positionMs);
    if (!this.stalled && missing !== null) {
      this.bufferAt(missing);
      positionMs = missing;
    }
    if (positionMs >= this.state.durationMs) {
      this.pause();
      this.update({ positionMs: this.state.durationMs });
      return;
    }
    this.update({ positionMs });
    for (const track of this.tracks) {
      if (track.loading) continue;
      track.loading = true;
      void this.fill(track, generation).catch(() => {
        if (this.current(generation)) this.fail('Retained audio is missing or unreadable. Try playing again.');
      }).finally(() => {
        track.loading = false;
        if (this.current(generation)) this.resumeBuffered();
      });
    }
  }
  private async fill(track: Track, generation: number) {
    const context = this.context;
    const sessionId = this.sessionId;
    if (!context || !sessionId) return;
    while (this.current(generation) && track.sources.size < 2) {
      if (track.chunks.length === 0 && track.cursor) {
        const manifest = await this.io.manifest({ sessionId, fromMs: this.position(), channel: track.channel, cursor: track.cursor, limit: 128 });
        if (!this.current(generation)) return;
        if (manifest.kind === 'unavailable') throw new Error('Meeting audio is unavailable');
        track.chunks = manifest.chunks;
        track.cursor = manifest.nextCursor;
        track.pageEndMs = Math.max(track.pageEndMs, ...manifest.chunks.map(chunk => chunk.endMs));
      }
      const overdue = this.missingAt(this.position());
      if (!this.stalled && overdue !== null) this.bufferAt(overdue);
      const chunk = track.chunks[0];
      if (!chunk || chunk.startMs > this.position() + LOOKAHEAD_MS) return;
      track.chunks.shift();
      if (chunk.endMs <= this.position()) continue;
      track.pending = chunk;
      const buffer = await this.decode(chunk, generation, context);
      if (!buffer || !this.current(generation)) return;
      const missing = this.missingAt(this.position());
      if (!this.stalled && missing !== null) this.bufferAt(missing);
      track.pending = null;
      this.schedule(track, chunk, buffer, context);
      this.resumeBuffered();
    }
  }
}

export function useMeetingAudio({ sessionId, captureBusy }: { sessionId: string | null; captureBusy: boolean }) {
  const [state, setState] = useState<MeetingAudioState>(INITIAL_STATE);
  const [monitorAttempt, setMonitorAttempt] = useState(0);
  const playback = useMemo(() => new MeetingAudioPlayback(sessionId, setState), [sessionId]);
  useEffect(() => {
    setState(playback.state);
    void playback.load();
    return () => playback.dispose();
  }, [playback]);
  useEffect(() => { playback.setBusy(captureBusy); }, [playback, captureBusy]);
  useEffect(() => {
    let disposed = false;
    let subscriptionsReady = false;
    let ticket = 0;
    const unlisteners: (() => void)[] = [];
    playback.beginMonitoring();
    const hydrate = async () => {
      const current = ++ticket;
      try {
        const busy = await dependencies.captureBusy();
        if (!disposed && current === ticket) playback.monitoringRecovered(busy);
      } catch { if (!disposed && current === ticket) playback.monitoringFailed(); }
    };
    const events = ['transform-capture-starting', 'transform-state-changed', 'transform-review-hidden', 'transform-capture-failed', 'query-state-changed', 'microphone-preview-status', 'microphone-startup-benchmark-progress', 'recording-status-changed', 'meeting-status-changed', 'meeting-summary-status-changed'];
    const subscriptions = events.map(name => listen<unknown>(name, ({ payload }) => {
        playback.pause();
        playback.setMonitoredBusy(true);
        if (name === 'meeting-status-changed' && typeof payload === 'object' && payload !== null
          && 'phase' in payload && (payload.phase === 'idle' || payload.phase === 'failed')
          && 'sessionId' in payload && (payload.sessionId === null || payload.sessionId === sessionId)) {
          playback.meetingFinished();
        }
        if (subscriptionsReady) void hydrate();
      }).then(unlisten => { if (disposed) unlisten(); else unlisteners.push(unlisten); }));
    subscriptions.push(listen<{ sessionId: string | null }>('meeting-audio-invalidated', ({ payload }) => {
      if (payload.sessionId === null || payload.sessionId === sessionId) playback.invalidate();
    }).then(unlisten => { if (disposed) unlisten(); else unlisteners.push(unlisten); }));
    void Promise.all(subscriptions).then(() => {
      subscriptionsReady = true;
      if (!disposed) void hydrate();
    }).catch(() => {
      if (!disposed) playback.monitoringFailed();
    });
    return () => { disposed = true; unlisteners.forEach(unlisten => unlisten()); };
  }, [playback, sessionId, monitorAttempt]);
  return {
    ...state, playAll: playback.playAll, playSegment: playback.playSegment, play: playback.play,
    pause: playback.pause, seek: playback.seek, setChannel: playback.setChannel, invalidate: playback.invalidate,
    retry: () => {
      playback.pause();
      playback.beginMonitoring();
      setMonitorAttempt(attempt => attempt + 1);
      void playback.load();
    },
  };
}

export type MeetingAudioController = ReturnType<typeof useMeetingAudio>;
