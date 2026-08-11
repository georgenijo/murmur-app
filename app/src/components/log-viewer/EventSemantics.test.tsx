import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { STREAMS } from '../../lib/events';
import { EventRow } from './EventRow';
import { StreamChips } from './StreamChips';

describe('log viewer event semantics', () => {
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

  it('renders six distinct active stream treatments with semantic markers', async () => {
    await act(async () => {
      root.render(
        <StreamChips active={new Set(STREAMS)} onToggle={vi.fn()} />,
      );
    });

    const chips = Array.from(container.querySelectorAll('button'));
    expect(chips).toHaveLength(STREAMS.length);
    expect(new Set(chips.map((chip) => chip.className)).size).toBe(STREAMS.length);
    for (const chip of chips) {
      const marker = chip.querySelector('[aria-hidden="true"]');
      expect(marker).not.toBeNull();
      expect(marker!.className).not.toContain('surface-container-highest');
    }
  });

  it('shows the stream marker and warning token in an event row', async () => {
    await act(async () => {
      root.render(
        <EventRow
          event={{
            timestamp: '2026-07-28T12:34:56Z',
            stream: 'transform',
            level: 'warn',
            summary: 'Transform needs attention',
            data: {},
          }}
        />,
      );
    });

    const warning = Array.from(container.querySelectorAll('span'))
      .find((element) => element.textContent === 'warn')!;
    const stream = Array.from(container.querySelectorAll('span'))
      .find((element) => element.textContent?.trim() === 'transform'
        && element.classList.contains('bg-warning/10'))!;

    expect(warning.classList).toContain('text-warning');
    expect(stream.classList).toContain('text-warning');
    expect(stream.querySelector('[aria-hidden="true"]')?.classList).toContain('bg-warning');
  });

  it('keeps error-row text on an opaque surface with a strong error border', async () => {
    await act(async () => {
      root.render(
        <EventRow
          event={{
            timestamp: '2026-07-28T12:34:56Z',
            stream: 'system',
            level: 'error',
            summary: 'Startup failed',
            data: { retryable: true },
          }}
        />,
      );
    });

    const errorRow = container.firstElementChild as HTMLElement;
    expect(errorRow.classList).toContain('border-l-4');
    expect(errorRow.classList).toContain('border-error');
    expect(errorRow.classList).toContain('bg-surface-container-lowest');
    expect(errorRow.classList).not.toContain('bg-error/10');
    expect(errorRow.querySelector('.text-error')?.textContent).toBe('error');
    expect(errorRow.querySelector('.text-on-surface')?.textContent).toContain(
      'Startup failed',
    );
    expect(errorRow.querySelector('.text-on-surface-variant')).not.toBeNull();
    expect(errorRow.querySelector('.bg-error\\/10')).toBeNull();
  });
});
