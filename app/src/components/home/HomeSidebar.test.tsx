import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HomeSidebar } from './HomeSidebar';

function mockMatchMedia(matches: boolean) {
  const listeners = new Set<() => void>();
  window.matchMedia = vi.fn().mockImplementation((query: string) => ({
    matches,
    media: query,
    onchange: null,
    addEventListener: (_event: string, listener: () => void) => listeners.add(listener),
    removeEventListener: (_event: string, listener: () => void) => listeners.delete(listener),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
  })) as unknown as typeof window.matchMedia;
}

describe('HomeSidebar tooltips', () => {
  let container: HTMLDivElement;
  let root: Root;
  const originalMatchMedia = window.matchMedia;

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    // The sidebar disables tooltips once labels are visible (>=760px); force
    // the compact/icon-only layout so hover tooltips remain testable in jsdom,
    // which otherwise has no real layout to derive this from.
    mockMatchMedia(true);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
    window.matchMedia = originalMatchMedia;
  });

  it('keeps aria-labels, refs, and click behavior on the wrapped nav buttons', async () => {
    const onNavigate = vi.fn();
    await act(async () => root.render(
      <HomeSidebar active="home" onNavigate={onNavigate} />,
    ));

    const buttons = Array.from(container.querySelectorAll('nav button')) as HTMLButtonElement[];
    expect(buttons.map((button) => button.getAttribute('aria-label'))).toEqual([
      'Home', 'Notetaker', 'Queries', 'Insights',
    ]);
    // Still exactly one button per destination — the tooltip trigger wraps
    // the existing button rather than introducing a duplicate nested one.
    expect(buttons).toHaveLength(4);

    const queriesButton = buttons.find((button) => button.getAttribute('aria-label') === 'Queries')!;
    await act(async () => queriesButton.click());
    expect(onNavigate).toHaveBeenCalledWith('queries');
  });

  it('shows the button label in a tooltip after the pointer rests on it', async () => {
    vi.useFakeTimers();
    await act(async () => root.render(
      <HomeSidebar active="home" onNavigate={() => {}} />,
    ));

    // Base UI's Tooltip.Trigger applies its interaction wiring (id,
    // data-base-ui-tooltip-trigger, hover/focus handlers) directly to the
    // element FluidTooltip.Trigger wraps — MainNavItem (owned by
    // ui/DashboardPrimitives, read-only here) now forwards unknown props
    // (including that wiring) straight onto its own <button>, so no wrapper
    // span is needed and the trigger element IS the button itself. The real
    // invariant: the two lookups must resolve to the same DOM node, not a
    // 0x0 `display: contents` ancestor that Base UI would anchor tooltips
    // to at (0,0).
    const insightsButton = Array.from(container.querySelectorAll('nav button'))
      .find((el) => el.getAttribute('aria-label') === 'Insights') as HTMLButtonElement;
    const insightsTrigger = Array.from(container.querySelectorAll('[data-base-ui-tooltip-trigger]'))
      .find((el) => el.getAttribute('aria-label') === 'Insights') as HTMLElement;
    expect(insightsTrigger).toBeTruthy();
    expect(insightsTrigger).toBe(insightsButton);
    expect(insightsTrigger.hasAttribute('data-popup-open')).toBe(false);

    // Base UI's hover-open path is driven by a React-level onMouseMove "rest"
    // timer (not pointerenter/pointermove), gated by a native mouseenter
    // listener that arms it — reproduced here with real DOM events plus the
    // group's configured openDelay (350ms) under fake timers.
    await act(async () => {
      insightsTrigger.dispatchEvent(new MouseEvent('mouseenter', { bubbles: false, clientX: 5, clientY: 5 }));
      insightsTrigger.dispatchEvent(new MouseEvent('mousemove', { bubbles: true, clientX: 5, clientY: 5 }));
    });
    await act(async () => {
      vi.advanceTimersByTime(1000);
    });

    expect(insightsTrigger.getAttribute('data-popup-open')).toBe('');
    const openTooltipText = Array.from(document.querySelectorAll('[data-base-ui-portal] [data-current]'))
      .map((node) => node.textContent)
      .join(' ');
    expect(openTooltipText).toContain('Insights');
  });
});
