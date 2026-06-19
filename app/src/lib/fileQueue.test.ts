import { describe, it, expect } from 'vitest';
import {
  hasAudioExtension,
  baseName,
  buildQueueItems,
  updateItem,
  nextQueued,
  summarize,
  hasAnyAudio,
  QueueItem,
} from './fileQueue';

describe('hasAudioExtension', () => {
  it('accepts supported audio extensions (case-insensitive)', () => {
    expect(hasAudioExtension('/a/b.wav')).toBe(true);
    expect(hasAudioExtension('/a/b.mp3')).toBe(true);
    expect(hasAudioExtension('/a/b.m4a')).toBe(true);
    expect(hasAudioExtension('/a/b.M4A')).toBe(true);
    expect(hasAudioExtension('CLIP.WAV')).toBe(true);
  });

  it('rejects unsupported or extension-less paths', () => {
    expect(hasAudioExtension('/a/b.txt')).toBe(false);
    expect(hasAudioExtension('/a/b.flac')).toBe(false);
    expect(hasAudioExtension('/a/noext')).toBe(false);
    expect(hasAudioExtension('')).toBe(false);
  });
});

describe('baseName', () => {
  it('returns the final segment for posix and windows paths', () => {
    expect(baseName('/Users/x/clip.wav')).toBe('clip.wav');
    expect(baseName('C:\\sounds\\clip.mp3')).toBe('clip.mp3');
    expect(baseName('clip.m4a')).toBe('clip.m4a');
  });
});

describe('buildQueueItems', () => {
  it('builds queued items only for supported audio files', () => {
    const items = buildQueueItems(['/a/one.wav', '/a/note.txt', '/a/two.mp3']);
    expect(items).toHaveLength(2);
    expect(items.map((i) => i.name)).toEqual(['one.wav', 'two.mp3']);
    expect(items.every((i) => i.status === 'queued')).toBe(true);
  });

  it('assigns unique ids even for duplicate basenames in different dirs', () => {
    const items = buildQueueItems(['/a/clip.wav', '/b/clip.wav']);
    expect(items).toHaveLength(2);
    expect(items[0].id).not.toBe(items[1].id);
  });

  it('dedupes against existing items and within the new batch', () => {
    const existing = buildQueueItems(['/a/one.wav']);
    const merged = buildQueueItems(['/a/one.wav', '/a/two.wav', '/a/two.wav'], existing);
    // /a/one.wav already queued -> skipped; /a/two.wav added once.
    expect(merged).toHaveLength(1);
    expect(merged[0].name).toBe('two.wav');
    // ordinal continues past existing length, keeping ids distinct.
    expect(merged[0].id).not.toBe(existing[0].id);
  });

  it('returns an empty list when no audio files are present', () => {
    expect(buildQueueItems(['/a/note.txt', '/a/img.png'])).toEqual([]);
  });
});

describe('updateItem', () => {
  it('patches only the targeted item immutably', () => {
    const queue = buildQueueItems(['/a/one.wav', '/a/two.wav']);
    const next = updateItem(queue, queue[0].id, { status: 'done', text: 'hi' });
    expect(next[0]).toMatchObject({ status: 'done', text: 'hi' });
    expect(next[1].status).toBe('queued');
    // original untouched
    expect(queue[0].status).toBe('queued');
    expect(next).not.toBe(queue);
  });

  it('is a no-op for an unknown id', () => {
    const queue = buildQueueItems(['/a/one.wav']);
    const next = updateItem(queue, 'missing', { status: 'error' });
    expect(next[0].status).toBe('queued');
  });
});

describe('nextQueued', () => {
  it('returns the first queued item', () => {
    let queue = buildQueueItems(['/a/one.wav', '/a/two.wav', '/a/three.wav']);
    queue = updateItem(queue, queue[0].id, { status: 'done' });
    const next = nextQueued(queue);
    expect(next?.name).toBe('two.wav');
  });

  it('returns null when nothing is queued', () => {
    let queue = buildQueueItems(['/a/one.wav']);
    queue = updateItem(queue, queue[0].id, { status: 'error', error: 'boom' });
    expect(nextQueued(queue)).toBeNull();
    expect(nextQueued([])).toBeNull();
  });

  it('skips an item currently transcribing', () => {
    let queue = buildQueueItems(['/a/one.wav', '/a/two.wav']);
    queue = updateItem(queue, queue[0].id, { status: 'transcribing' });
    expect(nextQueued(queue)?.name).toBe('two.wav');
  });
});

describe('summarize', () => {
  it('counts statuses and reports unfinished while work remains', () => {
    let queue = buildQueueItems(['/a/one.wav', '/a/two.wav', '/a/three.wav']);
    queue = updateItem(queue, queue[0].id, { status: 'done' });
    queue = updateItem(queue, queue[1].id, { status: 'transcribing' });
    const s = summarize(queue);
    expect(s).toMatchObject({ total: 3, queued: 1, transcribing: 1, done: 1, error: 0 });
    expect(s.finished).toBe(false);
  });

  it('is finished once nothing is queued or transcribing', () => {
    let queue = buildQueueItems(['/a/one.wav', '/a/two.wav']);
    queue = updateItem(queue, queue[0].id, { status: 'done' });
    queue = updateItem(queue, queue[1].id, { status: 'error', error: 'x' });
    const s = summarize(queue);
    expect(s.finished).toBe(true);
    expect(s.done).toBe(1);
    expect(s.error).toBe(1);
  });

  it('reports not finished for an empty queue', () => {
    expect(summarize([]).finished).toBe(false);
  });

  it('handles a queue with one item per status', () => {
    const queue: QueueItem[] = [
      { id: '0', path: '/a.wav', name: 'a.wav', status: 'queued' },
      { id: '1', path: '/b.wav', name: 'b.wav', status: 'transcribing' },
      { id: '2', path: '/c.wav', name: 'c.wav', status: 'done', text: 't' },
      { id: '3', path: '/d.wav', name: 'd.wav', status: 'error', error: 'e' },
    ];
    expect(summarize(queue)).toMatchObject({
      total: 4,
      queued: 1,
      transcribing: 1,
      done: 1,
      error: 1,
      finished: false,
    });
  });
});

describe('hasAnyAudio', () => {
  it('detects at least one audio path', () => {
    expect(hasAnyAudio(['/a/note.txt', '/a/clip.mp3'])).toBe(true);
    expect(hasAnyAudio(['/a/note.txt', '/a/img.png'])).toBe(false);
    expect(hasAnyAudio([])).toBe(false);
  });
});
