import { act, createRef } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HomeSidebar } from './HomeSidebar';

describe('HomeSidebar labeled navigation', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('keeps one labeled native button per destination and navigates once', async () => {
    const onNavigate = vi.fn();
    await act(async () => root.render(
      <HomeSidebar active="home" onNavigate={onNavigate} />,
    ));

    const buttons = Array.from(container.querySelectorAll<HTMLButtonElement>('nav button'));
    const labels = ['Home', 'Notetaker', 'Assistant', 'Insights'];
    expect(buttons.map((button) => button.getAttribute('aria-label'))).toEqual(labels);
    expect(buttons.map((button) => button.querySelector('.home-nav-label')?.textContent)).toEqual(labels);
    expect(buttons).toHaveLength(4);

    for (const [index, destination] of ['home', 'meetings', 'queries', 'insights'].entries()) {
      expect(buttons[index].type).toBe('button');
      expect(buttons[index].tabIndex).toBe(0);
      await act(async () => buttons[index].click());
      expect(onNavigate).toHaveBeenNthCalledWith(index + 1, destination);
    }
    expect(onNavigate).toHaveBeenCalledTimes(4);
  });

  it('preserves the Home focus ref and exposes only the selected destination as current', async () => {
    const homeButtonRef = createRef<HTMLButtonElement>();
    await act(async () => root.render(
      <HomeSidebar active="queries" homeButtonRef={homeButtonRef} onNavigate={() => {}} />,
    ));

    expect(homeButtonRef.current?.getAttribute('aria-label')).toBe('Home');
    homeButtonRef.current?.focus();
    expect(document.activeElement).toBe(homeButtonRef.current);
    expect(Array.from(container.querySelectorAll('nav [aria-current="page"]'))
      .map((button) => button.getAttribute('aria-label'))).toEqual(['Assistant']);
  });

  it('does not duplicate visible labels in hover or focus tooltips', async () => {
    vi.useFakeTimers();
    await act(async () => root.render(
      <HomeSidebar active="home" onNavigate={() => {}} />,
    ));

    const insightsButton = container.querySelector<HTMLButtonElement>('button[aria-label="Insights"]')!;
    await act(async () => {
      insightsButton.focus();
      insightsButton.dispatchEvent(new MouseEvent('mouseenter', { clientX: 5, clientY: 5 }));
      insightsButton.dispatchEvent(new MouseEvent('mousemove', { bubbles: true, clientX: 5, clientY: 5 }));
      vi.advanceTimersByTime(1000);
    });

    expect(document.activeElement).toBe(insightsButton);
    expect(document.querySelector('[role="tooltip"]')).toBeNull();
    expect(container.querySelector('[data-base-ui-tooltip-trigger]')).toBeNull();
    expect(insightsButton.disabled).toBe(false);
    expect(insightsButton.querySelectorAll('.home-nav-label')).toHaveLength(1);
    expect(insightsButton.getAttribute('aria-describedby')).toBeNull();
  });

  function pointer(target: Element, type: string, pointerId: number, clientX: number, options = {}) {
    const event = new MouseEvent(type, { bubbles: true, clientX, cancelable: true, ...options });
    Object.defineProperties(event, { pointerId: { value: pointerId }, isPrimary: { value: !('isPrimary' in options) || options.isPrimary } });
    target.dispatchEvent(event);
  }

  it('ignores foreign pointers and secondary buttons, then cancels only the owning drag', async () => {
    await act(async () => root.render(<HomeSidebar active="home" onNavigate={() => {}} />));
    const separator = container.querySelector<HTMLDivElement>('[role="separator"]')!;
    const sidebar = container.querySelector<HTMLElement>('aside')!;
    const capture = vi.spyOn(separator, 'setPointerCapture');
    await act(async () => pointer(separator, 'pointerdown', 9, 256, { button: 2 }));
    expect(capture).not.toHaveBeenCalled();
    await act(async () => pointer(separator, 'pointerdown', 10, 256, { isPrimary: false }));
    expect(capture).not.toHaveBeenCalled();
    await act(async () => pointer(separator, 'pointerdown', 9, 256));
    expect(capture).toHaveBeenCalledExactlyOnceWith(9);
    await act(async () => pointer(separator, 'pointermove', 9, 300));
    expect(sidebar.style.width).toBe('300px');
    await act(async () => {
      pointer(separator, 'pointerdown', 10, 300);
      pointer(separator, 'pointermove', 10, 380);
      pointer(separator, 'pointercancel', 10, 380);
      pointer(separator, 'pointerup', 10, 380);
    });
    expect(sidebar.style.width).toBe('300px');
    expect(sidebar.dataset.resizing).toBe('true');
    await act(async () => pointer(separator, 'pointercancel', 9, 300));
    expect(sidebar.style.width).toBe('256px');
    expect(sidebar.dataset.resizing).toBe('false');
  });

  it('releases capture and rolls back a draft when Settings suspends Home', async () => {
    const render = (suspended = false) => root.render(<HomeSidebar active="insights" suspended={suspended} onNavigate={() => {}} />);
    await act(async () => render());
    const separator = container.querySelector<HTMLDivElement>('[role="separator"]')!;
    vi.spyOn(separator, 'hasPointerCapture').mockReturnValue(true);
    const release = vi.spyOn(separator, 'releasePointerCapture');
    await act(async () => {
      pointer(separator, 'pointerdown', 4, 256);
      pointer(separator, 'pointermove', 4, 330);
    });
    await act(async () => render(true));
    expect(container.querySelector<HTMLElement>('aside')!.style.width).toBe('256px');
    expect(release).toHaveBeenCalledExactlyOnceWith(4);
    expect(container.querySelector('[aria-current="page"]')?.getAttribute('aria-label')).toBe('Insights');
  });

  it('cleans up pointer capture on unmount without committing a draft', async () => {
    await act(async () => root.render(<HomeSidebar active="home" onNavigate={() => {}} />));
    const separator = container.querySelector<HTMLDivElement>('[role="separator"]')!;
    vi.spyOn(separator, 'hasPointerCapture').mockReturnValue(true);
    const release = vi.spyOn(separator, 'releasePointerCapture');
    await act(async () => pointer(separator, 'pointerdown', 7, 256));
    await act(async () => root.render(null));
    expect(release).toHaveBeenCalledExactlyOnceWith(7);
  });


  it('cancels pointerup when the viewport crosses the breakpoint before its media event arrives', async () => {
    const media = { matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() };
    vi.stubGlobal('matchMedia', vi.fn(() => media));
    await act(async () => root.render(<HomeSidebar active="home" onNavigate={() => {}} />));
    const separator = container.querySelector<HTMLDivElement>('[role="separator"]')!;
    await act(async () => {
      pointer(separator, 'pointerdown', 7, 256);
      pointer(separator, 'pointermove', 7, 340);
    });
    media.matches = true;
    await act(async () => pointer(separator, 'pointerup', 7, 340));
    expect(container.querySelector<HTMLElement>('aside')!.style.width).toBe('256px');
    expect(container.querySelector('aside')!.dataset.resizing).toBe('false');
  });

});
