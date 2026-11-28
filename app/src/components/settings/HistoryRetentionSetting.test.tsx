import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { HistoryRetentionSetting } from './HistoryRetentionSetting';
import type { HistoryRetentionDays } from '../../lib/history';

// A native select stands in for the custom listbox so the test drives the choice directly.
vi.mock('../ui/Select', () => ({
  SelectLabelContext: { Provider: ({ children }: { children: React.ReactNode }) => children },
  Select: ({ value, onChange, items }: { value: string; onChange: (v: string) => void; items: { value: string; label: string }[] }) => (
    <select data-testid="retention" value={value} onChange={(event) => onChange(event.target.value)}>
      {items.map((item) => <option key={item.value} value={item.value}>{item.label}</option>)}
    </select>
  ),
}));

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

function select(): HTMLSelectElement {
  return container.querySelector('[data-testid="retention"]')!;
}
async function choose(value: string) {
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value')!.set!;
    setter.call(select(), value);
    select().dispatchEvent(new Event('change', { bubbles: true }));
  });
}
function button(label: string): HTMLButtonElement {
  return [...container.querySelectorAll('button')].find((b) => b.textContent === label)!;
}

async function mount(value: HistoryRetentionDays, counts: Partial<Record<HistoryRetentionDays, number>>) {
  const onChange = vi.fn();
  const countExpired = vi.fn((days: HistoryRetentionDays) => counts[days] ?? 0);
  await act(async () => root.render(<HistoryRetentionSetting value={value} onChange={onChange} countExpired={countExpired} />));
  return { onChange, countExpired };
}

it('defaults to Never and explains that nothing is deleted', async () => {
  await mount(0, {});
  expect(select().value).toBe('0');
  expect(container.textContent).toContain('Keep transcripts until you delete them');
  expect(container.querySelector('[data-testid="history-retention-confirm"]')).toBeNull();
});

it('applies a window immediately when it would delete nothing now', async () => {
  const { onChange } = await mount(0, { 90: 0 });
  await choose('90');
  expect(onChange).toHaveBeenCalledWith(90);
  expect(container.querySelector('[data-testid="history-retention-confirm"]')).toBeNull();
});

it('asks before deleting existing transcripts, naming the count', async () => {
  const { onChange } = await mount(0, { 30: 12 });
  await choose('30');
  expect(onChange).not.toHaveBeenCalled();
  const confirm = container.querySelector('[data-testid="history-retention-confirm"]')!;
  expect(confirm.getAttribute('role')).toBe('alert');
  expect(confirm.textContent).toContain('This deletes 12 transcripts older than 30 days now. Pinned transcripts are kept.');
  expect(select().value).toBe('30');

  await act(async () => button('Delete 12 transcripts').click());
  expect(onChange).toHaveBeenCalledExactlyOnceWith(30);
  expect(container.querySelector('[data-testid="history-retention-confirm"]')).toBeNull();
});

it('cancel keeps the current window and deletes nothing', async () => {
  const { onChange } = await mount(90, { 1: 1 });
  await choose('1');
  expect(container.textContent).toContain('This deletes 1 transcript older than 1 day now.');
  await act(async () => button('Cancel').click());
  expect(onChange).not.toHaveBeenCalled();
  expect(select().value).toBe('90');
});

it('turning it off never prompts', async () => {
  const { onChange, countExpired } = await mount(7, { 0: 99 });
  expect(container.textContent).toContain('Transcripts older than 7 days are deleted from this Mac.');
  await choose('0');
  expect(countExpired).not.toHaveBeenCalled();
  expect(onChange).toHaveBeenCalledWith(0);
});
