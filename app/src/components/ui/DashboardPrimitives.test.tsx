import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  DashboardAction,
  DashboardSectionHeader,
  DashboardStatGroup,
  DashboardSurface,
  MainNavItem,
  WorkspacePageHeader,
} from './DashboardPrimitives';

describe('dashboard primitives', () => {
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
  });

  it('keeps static surface depth separate from interactive actions', async () => {
    const onActivate = vi.fn();
    await act(async () => root.render(
      <DashboardSurface as="section" variant="outlined" ariaLabel="Summary">
        <DashboardAction variant="quiet" icon="forward" onActivate={onActivate}>Open</DashboardAction>
        <DashboardAction variant="primary" onActivate={vi.fn()} disabled>Disabled</DashboardAction>
      </DashboardSurface>,
    ));

    const surface = container.querySelector('section')!;
    const buttons = container.querySelectorAll('button');
    expect(surface.dataset.surface).toBe('outlined');
    expect(surface.getAttribute('aria-label')).toBe('Summary');
    expect(buttons[0].dataset.action).toBe('quiet');
    expect(buttons[1].disabled).toBe(true);
    await act(async () => buttons[0].click());
    expect(onActivate).toHaveBeenCalledOnce();
  });

  it('exposes every surface and action variant through one bounded contract', async () => {
    await act(async () => root.render(
      <>
        <DashboardSurface variant="flat">Flat</DashboardSurface>
        <DashboardSurface variant="outlined">Outlined</DashboardSurface>
        <DashboardSurface variant="elevated">Elevated</DashboardSurface>
        <DashboardAction variant="primary" onActivate={vi.fn()}>Primary</DashboardAction>
        <DashboardAction variant="secondary" onActivate={vi.fn()}>Secondary</DashboardAction>
        <DashboardAction variant="quiet" onActivate={vi.fn()}>Quiet</DashboardAction>
        <DashboardAction kind="link" href="#details" variant="secondary">Details</DashboardAction>
      </>,
    ));

    expect(Array.from(container.querySelectorAll('.ui-dashboard-surface')).map((surface) => (
      (surface as HTMLElement).dataset.surface
    ))).toEqual(['flat', 'outlined', 'elevated']);
    expect(Array.from(container.querySelectorAll('.ui-dashboard-action')).map((action) => (
      (action as HTMLElement).dataset.action
    ))).toEqual(['primary', 'secondary', 'quiet', 'secondary']);
    expect(container.querySelector('a')?.getAttribute('href')).toBe('#details');
  });

  it('renders one Back action and keeps navigation selection semantic', async () => {
    const onBack = vi.fn();
    const navRef = { current: null as HTMLButtonElement | null };
    await act(async () => root.render(
      <>
        <WorkspacePageHeader
          title="Insights"
          titleId="insights-title"
          description="Local statistics."
          back={{ label: 'Back to Home', onActivate: onBack }}
        />
        <MainNavItem
          ref={navRef}
          label="Home"
          icon={<span aria-hidden="true">H</span>}
          selected
          onActivate={vi.fn()}
        />
      </>,
    ));

    const back = Array.from(container.querySelectorAll('button'))
      .find((button) => button.textContent?.includes('Back to Home'))!;
    await act(async () => back.click());
    expect(onBack).toHaveBeenCalledOnce();
    expect(navRef.current?.getAttribute('aria-current')).toBe('page');
  });

  it('renders shared section and stat structures without fake interaction', async () => {
    await act(async () => root.render(
      <>
        <DashboardSectionHeader eyebrow="This month" title="Usage" titleId="usage-title" />
        <DashboardStatGroup
          kind="tiles"
          ariaLabel="Usage totals"
          items={[
            { id: 'words', label: 'Words', value: '5,168', detail: 'all time' },
            { id: 'wpm', label: 'Average speed', value: '189', detail: 'wpm' },
          ]}
        />
      </>,
    ));

    expect(container.querySelector('h2')?.id).toBe('usage-title');
    expect(container.querySelector('dl')?.dataset.stats).toBe('tiles');
    expect(container.querySelectorAll('dt')).toHaveLength(2);
    expect(Array.from(container.querySelectorAll('.ui-dashboard-stat')).every((item) => (
      Array.from(item.children).every((child) => child.tagName === 'DT' || child.tagName === 'DD')
    ))).toBe(true);
    expect(container.querySelectorAll('dd > small')).toHaveLength(2);
    expect(container.querySelectorAll('button')).toHaveLength(0);
  });
});
