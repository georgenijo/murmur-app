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
    expect(insightsButton.querySelectorAll('.home-nav-label')).toHaveLength(1);
    expect(insightsButton.getAttribute('aria-describedby')).toBeNull();
  });
});
