import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import HoldToDeleteButton from './hold-to-delete-button';

// Keep animation scheduling out of the input-ownership tests.
vi.mock('motion/react', async () => {
  const { forwardRef, createElement } = await import('react');
  const element = (tag: string) => forwardRef<HTMLElement, Record<string, unknown>>(
    ({ animate: _animate, ...props }, ref) => createElement(tag, { ...props, ref }),
  );
  return {
    motion: { button: element('button'), span: element('span') },
    animate: vi.fn(), useAnimationControls: () => ({ start: vi.fn() }),
    useMotionValue: () => ({ set: vi.fn() }), useReducedMotion: () => true,
    useTransform: () => undefined,
  };
});

describe('HoldToDeleteButton input ownership', () => {
  let container: HTMLDivElement;
  let root: Root;
  let button: HTMLButtonElement;
  const onDelete = vi.fn();
  const status = () => container.querySelector('[aria-live]')?.textContent;
  const pointer = async (type: string, id = 1, buttonNumber = 0) => {
    await act(async () => {
      const event = new MouseEvent(type, { bubbles: true, button: buttonNumber });
      Object.defineProperty(event, 'pointerId', { value: id });
      Object.defineProperty(event, 'isPrimary', { value: true });
      button.dispatchEvent(event);
    });
  };
  const click = async (detail = 0) => {
    await act(async () => {
      if (detail === 0) button.click();
      else button.dispatchEvent(new MouseEvent('click', { bubbles: true, detail }));
    });
  };
  const key = async (type: string, value: string, repeat = false) => {
    await act(async () => button.dispatchEvent(new KeyboardEvent(type, { key: value, repeat, bubbles: true, cancelable: true })));
  };
  const releaseClick = async () => {
    await pointer('pointerdown');
    await pointer('pointerup');
    await click(1);
  };

  beforeEach(async () => {
    vi.useFakeTimers();
    onDelete.mockReset();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(<HoldToDeleteButton label="Delete history" confirmLabel="Confirm deletion" holdDuration={1200} onDelete={onDelete} />));
    button = container.querySelector('button')!;
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  it.each([1, 2])('ignores mouse button %s for both taps and a full hold', async (number) => {
    await pointer('pointerdown', 1, number);
    await pointer('pointerup', 1, number);
    expect(status()).toBe('Delete history…');
    await pointer('pointerdown', 1, number);
    await act(async () => vi.advanceTimersByTime(1300));
    expect(onDelete).not.toHaveBeenCalled();
  });

  it('ignores a non-primary pointer and does not mix keyboard and pointer ownership', async () => {
    await act(async () => button.dispatchEvent(new PointerEvent('pointerdown', { pointerId: 3, button: 0, isPrimary: false, bubbles: true })));
    expect(status()).toBe('Delete history…');
    await pointer('pointerdown', 7);
    await key('keydown', 'Enter');
    await key('keyup', 'Enter');
    expect(status()).toBe('Keep holding');
    await pointer('pointerup', 7);
    expect(status()).toBe('Confirm deletion…');
    expect(onDelete).not.toHaveBeenCalled();
  });

  it('requires the starting pointer and ignores competing presses and releases', async () => {
    await pointer('pointerdown', 7);
    await pointer('pointerdown', 9);
    await pointer('pointerup', 9);
    await pointer('pointerup', 7, 2);
    expect(status()).toBe('Keep holding');
    await pointer('pointerup', 7);
    expect(status()).toBe('Confirm deletion…');
    expect(onDelete).not.toHaveBeenCalled();
  });

  it('supports two click-only assistive activations without pointer or key events', async () => {
    await click();
    expect(status()).toBe('Confirm deletion…');
    await click();
    await click();
    expect(onDelete).toHaveBeenCalledTimes(1);
  });

  it('counts pointer release and its generated click once', async () => {
    await releaseClick();
    expect(status()).toBe('Confirm deletion…');
    expect(onDelete).not.toHaveBeenCalled();
    await releaseClick();
    expect(onDelete).toHaveBeenCalledTimes(1);
  });

  it.each([' '])('retains %s arm/confirm and suppresses its generated click', async (value) => {
    await key('keydown', value);
    await key('keydown', value, true);
    await key('keyup', value);
    await click();
    expect(status()).toBe('Confirm deletion…');
    expect(onDelete).not.toHaveBeenCalled();
    await key('keydown', value);
    await key('keyup', value);
    await click();
    expect(onDelete).toHaveBeenCalledTimes(1);
  });

  it.each(['pointercancel', 'lostpointercapture', 'pointerout'])('cancels on %s and rejects the trailing click', async (type) => {
    await pointer('pointerdown');
    await pointer(type);
    await pointer('pointerup');
    await click(1);
    await act(async () => vi.advanceTimersByTime(1300));
    expect(status()).toBe('Delete history…');
    expect(onDelete).not.toHaveBeenCalled();
  });

  it('ignores cancellation from another pointer', async () => {
    await pointer('pointerdown', 7);
    await pointer('pointercancel', 9);
    expect(status()).toBe('Keep holding');
    await act(async () => vi.advanceTimersByTime(1200));
    await pointer('pointerup', 7);
    await click(1);
    expect(onDelete).toHaveBeenCalledTimes(1);
  });

  it('Escape disarms click-only confirmation and cancels keyboard holding', async () => {
    await click();
    await key('keydown', 'Escape');
    expect(status()).toBe('Delete history…');
    await key('keydown', 'Enter');
    await key('keydown', 'Escape');
    await key('keyup', 'Enter');
    await click();
    await act(async () => vi.advanceTimersByTime(1300));
    expect(onDelete).not.toHaveBeenCalled();
  });

  it('disarms after timeout, blur, and disabling during a hold', async () => {
    await click();
    await act(async () => vi.advanceTimersByTime(4000));
    expect(status()).toBe('Delete history…');
    await click();
    await act(async () => button.dispatchEvent(new FocusEvent('focusout', { bubbles: true })));
    expect(status()).toBe('Delete history…');
    await pointer('pointerdown');
    await act(async () => root.render(<HoldToDeleteButton label="Delete history" disabled onDelete={onDelete} />));
    await act(async () => vi.advanceTimersByTime(2500));
    expect(onDelete).not.toHaveBeenCalled();
  });
});
