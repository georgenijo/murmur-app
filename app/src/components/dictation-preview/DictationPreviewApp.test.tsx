import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  listen: vi.fn(),
  invoke: vi.fn().mockResolvedValue(undefined),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

import { DictationPreviewApp, DictationPreviewCard, visiblePartial } from './DictationPreviewApp';
import { DEFAULT_SETTINGS, STORAGE_KEY } from '../../lib/settings';

describe('visiblePartial', () => {
  it('keeps short transcripts intact', () => {
    expect(visiblePartial('  okay so one thing  ')).toBe('okay so one thing');
  });

  it('keeps the tail, not the head, once the transcript outgrows the card', () => {
    const long = `${'word '.repeat(200)}final words`;
    const visible = visiblePartial(long);
    expect(visible.endsWith('final words')).toBe(true);
    expect(visible.length).toBeLessThanOrEqual(320);
  });

  it('never starts the visible tail mid-word', () => {
    // The 320-char window lands inside the leading run, so the partial first
    // token is dropped and the tail resumes at the next whole word.
    const long = `${'a'.repeat(200)} supercalifragilistic ${'b'.repeat(200)}`;
    expect(visiblePartial(long).startsWith('supercalifragilistic ')).toBe(true);
    expect(visiblePartial(long).includes('aaa')).toBe(false);
  });

  it('falls back to a hard cut when the tail has no word boundary', () => {
    const unbroken = 'z'.repeat(400);
    expect(visiblePartial(unbroken)).toBe('z'.repeat(320));
  });
});

describe('DictationPreviewApp', () => {
  let root: Root;
  let container: HTMLDivElement;

  beforeEach(async () => {
    localStorage.clear();
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ ...DEFAULT_SETTINGS, overlayLiveWords: true }));
    Object.defineProperty(window, 'matchMedia', { configurable: true, value: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }) });
    mocks.listeners.clear();
    mocks.listen.mockReset();
    mocks.listen.mockImplementation(async (name: string, handler: (event: { payload: unknown }) => void) => {
      mocks.listeners.set(name, handler);
      return () => mocks.listeners.delete(name);
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(<DictationPreviewApp />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('renders nothing until words arrive, so an empty card never shows', () => {
    expect(container.querySelector('[aria-label="Live dictation preview"]')).toBeNull();
  });

  it('renders the live transcript once partials land', async () => {
    await act(async () => {
      mocks.listeners.get('dictation-generation-started')?.({ payload: { recordingId: 3 } });
      mocks.listeners.get('dictation-partial')?.({ payload: { recordingId: 3, text: 'okay so one thing' } });
    });
    const card = container.querySelector('[aria-label="Live dictation preview"]');
    expect(card).not.toBeNull();
    expect(container.querySelector('[aria-label="Words recognized so far"]')?.textContent)
      .toBe('okay so one thing');
    expect(container.querySelector('.live-words-stable')).toBeNull();
    expect(container.querySelector('.live-words-tentative')?.textContent).toBe('okay so one thing');
    expect(container.querySelector('button, input, textarea, [title], [contenteditable]')).toBeNull();
  });

  it('clears instantly when disabled and does not resurrect a stopped generation on re-enable', async () => {
    await act(async () => {
      mocks.listeners.get('dictation-generation-started')?.({ payload: { recordingId: 3 } });
      mocks.listeners.get('dictation-partial')?.({ payload: { recordingId: 3, text: 'synthetic words' } });
    });
    expect(container.textContent).toContain('synthetic words');
    const toggle = async (enabled: boolean) => act(async () => {
      localStorage.setItem(STORAGE_KEY, JSON.stringify({ ...DEFAULT_SETTINGS, overlayLiveWords: enabled }));
      mocks.listeners.get('settings-changed')?.({ payload: null });
    });
    await toggle(false);
    expect(container.textContent).toBe('');
    await act(async () => mocks.listeners.get('recording-status-changed')?.({ payload: 'idle' }));
    await toggle(true);
    await act(async () => mocks.listeners.get('dictation-partial')?.({ payload: { recordingId: 3, text: 'stale' } }));
    expect(container.textContent).toBe('');
  });

  it('styles stable text only when the producer supplies an explicit boundary', async () => {
    await act(async () => root.render(<DictationPreviewCard stableText="committed synthetic words" text="provisional ending" />));
    expect(container.querySelector('.live-words-stable')?.textContent).toBe('committed synthetic words');
    expect(container.querySelector('.live-words-tentative')?.textContent).toBe('provisional ending');
  });
});
