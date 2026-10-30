import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { MiddleTruncate, splitForMiddleTruncate } from './middle-truncate';

describe('MiddleTruncate', () => {
  let root: Root | undefined;
  let container: HTMLDivElement | undefined;
  afterEach(() => {
    act(() => root?.unmount());
    container?.remove();
    vi.unstubAllGlobals();
  });

  it('keeps short paths whole and preserves a bounded filename for long paths', () => {
    expect(splitForMiddleTruncate('readme.md')).toBeNull();
    expect(splitForMiddleTruncate('/work/project/transcription.md')?.tail).toBe('transcription.md');
    expect(Array.from(splitForMiddleTruncate('/work/project/a-very-long-filename.ts')!.tail).length).toBeLessThanOrEqual(16);
    expect(splitForMiddleTruncate('/work/project/readme.md', 0)).toBeNull();
  });

  it('splits emoji by code point rather than splitting a surrogate pair', () => {
    expect(splitForMiddleTruncate('project/😀😀😀😀😀😀😀😀😀😀😀😀', 4)).toEqual({
      head: 'project/😀😀😀😀😀😀😀😀', tail: '😀😀😀😀',
    });
  });

  it('updates full text/title and releases observation when switching to a short path', () => {
    const observe = vi.fn();
    const disconnect = vi.fn();
    vi.stubGlobal('ResizeObserver', class { observe = observe; disconnect = disconnect; });
    container = document.createElement('div');
    document.body.append(container);
    root = createRoot(container);
    act(() => root!.render(<MiddleTruncate value="/project/deep/location/transcription.md" />));
    expect(container.textContent).toBe('/project/deep/location/transcription.md');
    expect(container.firstElementChild?.getAttribute('title')).toBe('/project/deep/location/transcription.md');
    expect(observe).toHaveBeenCalled();
    act(() => root!.render(<MiddleTruncate value="short.md" showTitle={false} />));
    expect(container.textContent).toBe('short.md');
    expect(container.firstElementChild?.hasAttribute('title')).toBe(false);
    expect(container.querySelector('[data-overflow]')).toBeNull();
    expect(disconnect).toHaveBeenCalledTimes(1);
  });
});
