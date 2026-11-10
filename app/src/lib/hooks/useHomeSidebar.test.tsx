import { StrictMode, act, useLayoutEffect } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HOME_SIDEBAR_QUERY, useHomeSidebar } from './useHomeSidebar';

describe('useHomeSidebar breakpoint ownership', () => {
  let container: HTMLDivElement;
  let root: Root;
  let media: MediaQueryList;
  let matches: boolean;

  function Harness({ onLayout }: { onLayout?: () => void }) {
    const sidebar = useHomeSidebar();
    useLayoutEffect(() => { onLayout?.(); }, [onLayout]);
    return <>
      <button onClick={sidebar.toggle}>Toggle</button>
      <div data-width={sidebar.width} data-collapsed={sidebar.collapsed}>
        {!sidebar.collapsed && <div role="separator" {...sidebar.resizeProps} />}
      </div>
    </>;
  }

  beforeEach(() => {
    matches = false;
    media = new EventTarget() as MediaQueryList;
    Object.defineProperty(media, 'matches', { get: () => matches });
    vi.stubGlobal('matchMedia', vi.fn((query: string) => {
      expect(query).toBe(HOME_SIDEBAR_QUERY);
      return media;
    }));
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.unstubAllGlobals();
  });

  const width = () => Number(container.querySelector('[data-width]')?.getAttribute('data-width'));
  const toggle = async () => { await act(async () => container.querySelector('button')!.click()); };
  const notify = async (next: boolean) => {
    matches = next;
    await act(async () => media.dispatchEvent(new Event('change')));
  };
  const key = async (value: string) => {
    await act(async () => container.querySelector('[role="separator"]')!.dispatchEvent(
      new KeyboardEvent('keydown', { key: value, bubbles: true, cancelable: true }),
    ));
  };

  it('keeps an explicit narrow expansion and keyboard width when its initial media notification arrives late', async () => {
    matches = true;
    await act(async () => root.render(<StrictMode><Harness /></StrictMode>));
    expect(width()).toBe(48);
    await toggle();
    await key('End');
    expect(width()).toBe(384);
    await key('Home');
    expect(width()).toBe(208);
    // The query already matched during initial render. Its queued notification
    // must not start another narrow visit after the user has expanded it.
    await notify(true);
    expect(width()).toBe(208);
    expect(container.querySelector('[role="separator"]')).not.toBeNull();
  });

  it('reconciles a breakpoint crossed between render and effect subscription', async () => {
    await act(async () => root.render(<Harness onLayout={() => { matches = true; }} />));
    expect(width()).toBe(48);
    await toggle();
    expect(width()).toBe(208);
    await notify(true);
    expect(width()).toBe(208);
  });

  it('collapses each new narrow visit while restoring desktop width and collapse preference', async () => {
    await act(async () => root.render(<Harness />));
    await key('End');
    expect(width()).toBe(384);
    await notify(true);
    expect(width()).toBe(48);
    await toggle();
    expect(width()).toBe(384);
    await notify(false);
    expect(width()).toBe(384);
    await notify(true);
    expect(width()).toBe(48);
    await notify(false);
    await toggle();
    expect(width()).toBe(48);
    await notify(true);
    await toggle();
    expect(width()).toBe(384);
    await notify(false);
    expect(width()).toBe(48);
    await toggle();
    expect(width()).toBe(384);
  });
});
