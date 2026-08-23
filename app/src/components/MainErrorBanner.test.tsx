import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MainErrorBanner } from './MainErrorBanner';

describe('MainErrorBanner', () => {
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

  it('is an accessible alert with a keyboard-operable dismiss control', async () => {
    const onDismiss = vi.fn();
    await act(async () => root.render(
      <MainErrorBanner message="Microphone cleanup is still in progress." onDismiss={onDismiss} />,
    ));

    const alert = container.querySelector('[role="alert"]') as HTMLElement;
    const dismiss = container.querySelector('[aria-label="Dismiss error"]') as HTMLButtonElement;
    expect(alert.textContent).toContain('Microphone cleanup');
    expect(alert.getAttribute('aria-atomic')).toBe('true');
    expect(dismiss.type).toBe('button');

    dismiss.focus();
    expect(document.activeElement).toBe(dismiss);
    await act(async () => dismiss.click());
    expect(onDismiss).toHaveBeenCalledOnce();
  });

  it('uses bounded chrome alignment and permits long text to wrap', async () => {
    await act(async () => root.render(
      <MainErrorBanner
        message="A deliberately long error that must remain readable in a narrow main window without forcing the alert beyond the application chrome."
        onDismiss={vi.fn()}
      />,
    ));

    const alert = container.querySelector('[data-testid="main-error-banner"]') as HTMLElement;
    const message = alert.querySelector('p') as HTMLParagraphElement;
    expect(alert.classList).toContain('main-error-banner');
    expect(message.classList).toContain('min-w-0');
    expect(message.classList).toContain('break-words');
    expect(alert.querySelector('button')?.classList).toContain('shrink-0');
  });
});
