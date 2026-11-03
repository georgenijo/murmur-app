import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SettingsRow } from '../settings/SettingsRow';
import { Select } from './Select';

const items = [
  { value: 'a', label: 'Automatic' },
  { value: 'b', label: 'English' },
  { value: 'c', label: 'French' },
];

function rect(top: number, bottom: number): DOMRect {
  return { top, bottom, left: 0, right: 160, width: 160, height: bottom - top, x: 0, y: top, toJSON() {} };
}

function accessibleName(element: Element): string {
  const labelledBy = element.getAttribute('aria-labelledby');
  return labelledBy
    ? labelledBy.split(/\s+/).map(id => document.getElementById(id)?.textContent ?? '').join(' ')
    : element.getAttribute('aria-label') ?? '';
}

describe('Select', () => {
  let container: HTMLDivElement;
  let root: Root;
  const onChange = vi.fn();

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    onChange.mockReset();
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  const trigger = () => container.querySelector<HTMLButtonElement>('[role="combobox"]')!;
  const menu = () => container.querySelector<HTMLUListElement>('[role="listbox"]')!;
  const key = async (value: string) => act(async () => {
    trigger().dispatchEvent(new KeyboardEvent('keydown', { key: value, bubbles: true }));
  });

  it('uses its visible row title as the combobox and listbox name, excluding reset text', async () => {
    await act(async () => root.render(<SettingsRow title="Language" onReset={() => {}}
      control={<Select value="b" onChange={onChange} items={items} />} />));
    expect(accessibleName(trigger())).toBe('Language');
    expect(trigger().textContent).toBe('English');
    await act(async () => trigger().click());
    expect(accessibleName(menu())).toBe('Language');
    expect(menu().querySelector('[aria-selected="true"]')?.textContent).toBe('English');
  });

  it('preserves an explicit name and supports an explicit visible label outside a row', async () => {
    await act(async () => root.render(<SettingsRow title="Device"
      control={<Select aria-label="Microphone input" value="a" onChange={onChange} items={items} />} />));
    expect(accessibleName(trigger())).toBe('Microphone input');
    await act(async () => root.render(<><span id="custom-label">Provider</span>
      <Select aria-labelledby="custom-label" value="a" onChange={onChange} items={items} /></>));
    expect(accessibleName(trigger())).toBe('Provider');
  });

  it('preserves Arrow, Home, End, selection, dismissal and trigger focus', async () => {
    await act(async () => root.render(<Select aria-label="Language" value="b" onChange={onChange} items={items} />));
    trigger().focus();
    await key('ArrowDown');
    expect(trigger().getAttribute('aria-expanded')).toBe('true');
    const active = () => document.getElementById(trigger().getAttribute('aria-activedescendant')!)?.textContent;
    expect(active()).toBe('English');
    await key('End');
    expect(active()).toBe('French');
    await key('Home');
    expect(active()).toBe('Automatic');
    await key('ArrowUp');
    expect(active()).toBe('French');
    await key('Enter');
    expect(onChange).toHaveBeenCalledExactlyOnceWith('c');
    expect(document.activeElement).toBe(trigger());
    expect(menu()).toBeNull();
    await key(' ');
    await key('Escape');
    expect(menu()).toBeNull();
    expect(document.activeElement).toBe(trigger());
    await key('ArrowDown');
    await key('Tab');
    expect(menu()).toBeNull();
    expect(onChange).toHaveBeenCalledTimes(1);
  });

  it('selects with the pointer and dismisses on outside clicks', async () => {
    await act(async () => root.render(<Select aria-label="Language" value="a" onChange={onChange} items={items} />));
    expect(document.activeElement).not.toBe(trigger());
    await act(async () => trigger().click());
    expect(document.activeElement).toBe(trigger());
    await act(async () => menu().querySelectorAll<HTMLLIElement>('[role="option"]')[1].click());
    expect(onChange).toHaveBeenCalledExactlyOnceWith('b');
    await act(async () => trigger().click());
    await act(async () => document.body.dispatchEvent(new MouseEvent('mousedown', { bubbles: true })));
    expect(menu()).toBeNull();
  });

  it.each([560, 720])('flips above a lower-edge trigger inside a nested scroller at height %s', async (height) => {
    vi.stubGlobal('innerHeight', height);
    const original = HTMLElement.prototype.getBoundingClientRect;
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
      if (this.dataset.testid === 'scroll-pane') return rect(80, height - 40);
      if (this.getAttribute('role') === 'combobox') return rect(height - 110, height - 78);
      return original.call(this);
    });
    vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockReturnValue(160);
    await act(async () => root.render(<div data-testid="scroll-pane" style={{ overflowY: 'auto' }}>
      <Select aria-label="Hold key" value="a" onChange={onChange} items={items} />
    </div>));
    await act(async () => trigger().click());
    expect(menu().dataset.side).toBe('top');
    expect(menu().style.bottom).toBe('calc(100% + 4px)');
    expect(menu().style.maxHeight).toBe('240px');
  });

  it('bounds a menu in a short clipping pane and updates its placement on scroll', async () => {
    let triggerTop = 190;
    const original = HTMLElement.prototype.getBoundingClientRect;
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
      if (this.dataset.testid === 'scroll-pane') return rect(100, 300);
      if (this.getAttribute('role') === 'combobox') return rect(triggerTop, triggerTop + 32);
      return original.call(this);
    });
    vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockReturnValue(400);
    await act(async () => root.render(<div data-testid="scroll-pane" style={{ overflowY: 'auto' }}>
      <Select aria-label="Hold key" value="a" onChange={onChange} items={items} />
    </div>));
    await act(async () => trigger().click());
    expect(menu().dataset.side).toBe('top');
    expect(menu().style.maxHeight).toBe('86px');
    triggerTop = 110;
    await act(async () => container.firstElementChild!.dispatchEvent(new Event('scroll')));
    expect(menu().dataset.side).toBe('bottom');
    expect(menu().style.maxHeight).toBe('154px');
  });

  it('keeps observers across equivalent items and remeasures real content and geometry changes', async () => {
    const resizeCallbacks: (() => void)[] = [];
    const mutationCallbacks: (() => void)[] = [];
    const resizeDisconnect = vi.fn();
    const mutationDisconnect = vi.fn();
    const observe = vi.fn();
    const resize = vi.fn(function (callback: () => void) {
      resizeCallbacks.push(callback);
      return { observe, disconnect: resizeDisconnect };
    });
    const mutation = vi.fn(function (callback: () => void) {
      mutationCallbacks.push(callback);
      return { observe, disconnect: mutationDisconnect };
    });
    vi.stubGlobal('ResizeObserver', resize);
    vi.stubGlobal('MutationObserver', mutation);
    let triggerTop = 190;
    let contentHeight = 60;
    let clipBottom = 300;
    const original = HTMLElement.prototype.getBoundingClientRect;
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
      if (this.dataset.testid === 'scroll-pane') return rect(100, clipBottom);
      if (this.getAttribute('role') === 'combobox') return rect(triggerTop, triggerTop + 32);
      return original.call(this);
    });
    vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockImplementation(() => contentHeight);
    const render = async (label = 'Languages', badge?: string) => act(async () => root.render(
      <div data-testid="scroll-pane" style={{ overflowY: 'auto' }}>
        <Select aria-label="Language" value="a" onChange={onChange}
          items={[{ label, options: items.map(item => ({ ...item, badge })) }]} />
      </div>,
    ));
    await render();
    await act(async () => trigger().click());
    expect(menu().dataset.side).toBe('bottom');
    for (let i = 0; i < 3; i++) await render();
    expect(resize).toHaveBeenCalledTimes(1);
    expect(mutation).toHaveBeenCalledTimes(1);
    expect(resizeDisconnect).not.toHaveBeenCalled();
    expect(mutationDisconnect).not.toHaveBeenCalled();
    expect(observe).toHaveBeenCalledWith(trigger());
    // Same option count, different heading/badge wrapping and therefore height.
    contentHeight = 180;
    await render('Available languages', 'Recommended');
    expect(resize).toHaveBeenCalledTimes(2);
    expect(mutation).toHaveBeenCalledTimes(2);
    expect(menu().dataset.side).toBe('top');
    triggerTop = 110;
    await act(async () => resizeCallbacks[resizeCallbacks.length - 1]());
    expect(menu().dataset.side).toBe('bottom');
    expect(menu().style.maxHeight).toBe('154px');
    clipBottom = 250;
    await act(async () => mutationCallbacks[mutationCallbacks.length - 1]());
    expect(menu().style.maxHeight).toBe('104px');
    await key('Escape');
    expect(resizeDisconnect).toHaveBeenCalledTimes(2);
    expect(mutationDisconnect).toHaveBeenCalledTimes(2);
  });

  it('scrolls highlighted options only within the list, without scrolling Settings', async () => {
    const scrollIntoView = vi.fn();
    Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', { value: scrollIntoView, configurable: true });
    const original = HTMLElement.prototype.getBoundingClientRect;
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
      if (this.getAttribute('role') === 'listbox') return rect(100, 180);
      if (this.dataset.index === '0') return rect(104, 136);
      if (this.dataset.index === '2') return rect(200, 232);
      return original.call(this);
    });
    await act(async () => root.render(<Select aria-label="Language" value="a" onChange={onChange} items={items} />));
    await act(async () => trigger().click());
    await key('End');
    expect(menu().scrollTop).toBe(52);
    expect(scrollIntoView).not.toHaveBeenCalled();
    await act(async () => root.render(<Select disabled aria-label="Language" value="a" onChange={onChange} items={items} />));
    expect(menu()).toBeNull();
    expect(trigger().disabled).toBe(true);
  });
});
