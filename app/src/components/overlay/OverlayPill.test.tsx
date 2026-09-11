import { act, useRef } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { OverlayGeometry } from '../../lib/overlayGeometry';
import { OverlayPill, type OverlaySmartAutoSummary } from './OverlayPill';
import { BAR_COUNT } from '../../lib/hooks/useWaveform';
import type { OverlayIndicator } from './deriveVisual';
import type { OverlayDeliveryCue } from '../../lib/hooks/useOverlayRuntime';

const geometry: OverlayGeometry = {
  windowW: 257,
  collapsedH: 32,
  expandedH: 76,
  pillIdleW: 221,
  pillActiveW: 257,
  pillMarginIdle: 0,
  pillMarginActive: 0,
  dropdownH: 44,
  wingW: 36,
};

function CuePill({
  indicator,
  smartAutoSummary,
  deliveryCue,
  onRetryDelivery,
  onPauseDeliveryTimer,
  onResumeDeliveryTimer,
}: {
  indicator: OverlayIndicator;
  smartAutoSummary?: OverlaySmartAutoSummary;
  deliveryCue?: OverlayDeliveryCue;
  onRetryDelivery?: () => void;
  onPauseDeliveryTimer?: () => void;
  onResumeDeliveryTimer?: () => void;
}) {
  const barRefs = useRef<(HTMLDivElement | null)[]>([]);
  return (
    <OverlayPill
      geometry={geometry}
      visual={{
        indicator,
        showTapMissedLabel: false,
        waveformVisible: false,
      }}
      status="idle"
      barRefs={barRefs}
      smartAutoSummary={smartAutoSummary}
      deliveryCue={deliveryCue}
      onRetryDelivery={onRetryDelivery}
      onPauseDeliveryTimer={onPauseDeliveryTimer}
      onResumeDeliveryTimer={onResumeDeliveryTimer}
    />
  );
}

describe('OverlayPill transient cues', () => {
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

  it('renders an accessible manual-paste status with a non-focusing retry action', async () => {
    const onRetryDelivery = vi.fn();
    const onPauseDeliveryTimer = vi.fn();
    const onResumeDeliveryTimer = vi.fn();
    await act(async () => root.render(
      <CuePill
        indicator={{ kind: 'clipboardOnly' }}
        deliveryCue={{ kind: 'confirmed_clipboard', message: 'Text copied. Try again.' }}
        onRetryDelivery={onRetryDelivery}
        onPauseDeliveryTimer={onPauseDeliveryTimer}
        onResumeDeliveryTimer={onResumeDeliveryTimer}
      />,
    ));

    const status = container.querySelector<HTMLElement>('[role="status"]');
    expect(status?.textContent).toBe('⌘V');
    expect(status?.getAttribute('aria-live')).toBe('polite');
    expect(status?.getAttribute('aria-label')).toBe('Text copied. Try again.');
    const action = container.querySelector<HTMLButtonElement>('[aria-label="Try delivery again"]')!;
    expect(action.textContent).toBe('Try again');
    expect(action.tabIndex).toBe(-1);
    expect(action.style.fontSize).toBe('8px');
    expect(action.style.fontWeight).toBe('600');
    expect(action.style.lineHeight).toBe('1');
    expect(action.style.letterSpacing).toBe('-0.03em');
    const pointerDown = new MouseEvent('pointerdown', { bubbles: true, cancelable: true });
    action.dispatchEvent(pointerDown);
    expect(pointerDown.defaultPrevented).toBe(true);
    expect(onPauseDeliveryTimer).toHaveBeenCalledOnce();
    const mouseDown = new MouseEvent('mousedown', { bubbles: true, cancelable: true });
    action.dispatchEvent(mouseDown);
    expect(mouseDown.defaultPrevented).toBe(true);
    await act(async () => action.click());
    expect(onRetryDelivery).toHaveBeenCalledOnce();

    action.dispatchEvent(new MouseEvent('pointerout', { bubbles: true }));
    expect(onResumeDeliveryTimer).toHaveBeenCalledOnce();
  });

  it.each([
    [{ kind: 'auto_pasted', message: 'Pasted.' } as const, '✓', 'Pasted', false],
    [{ kind: 'retrying', message: 'Trying.' } as const, '…', 'Trying…', false],
    [{ kind: 'clipboard_only', message: 'Clipboard changed.' } as const, '↻', 'Try again', true],
    [{ kind: 'empty', message: 'Nothing.' } as const, '—', 'Nothing', false],
    [{ kind: 'busy', message: 'Busy.' } as const, '…', 'Try again', true],
    [{ kind: 'failed', message: 'Failed.' } as const, '!', 'Try again', true],
  ])('renders a sensible %s delivery result', async (deliveryCue, icon, label, retryable) => {
    await act(async () => root.render(
      <CuePill indicator={{ kind: 'clipboardOnly' }} deliveryCue={deliveryCue} />,
    ));
    expect(container.querySelector<HTMLElement>('[role="status"]')?.textContent).toBe(icon);
    expect(container.textContent).toContain(label);
    expect(Boolean(container.querySelector('[aria-label="Try delivery again"]'))).toBe(retryable);
  });

  it('renders an actionable, non-interactive mic-off status for an unavailable device', async () => {
    await act(async () => root.render(
      <CuePill indicator={{ kind: 'microphoneFailure', failure: 'chooseMicrophone' }} />,
    ));

    const status = container.querySelector<HTMLElement>('[role="status"]');
    expect(status?.getAttribute('aria-live')).toBe('assertive');
    expect(status?.getAttribute('aria-label')).toBe(
      'Selected microphone unavailable. Open Settings to choose another.',
    );
    expect(status?.querySelector('svg')).not.toBeNull();
    expect(status?.querySelector('svg')?.getAttribute('aria-hidden')).toBe('true');
    expect(container.querySelector('button')).toBeNull();
  });

  it('keeps other microphone failures generic and truthful', async () => {
    await act(async () => root.render(
      <CuePill indicator={{ kind: 'microphoneFailure', failure: 'retry' }} />,
    ));

    const status = container.querySelector<HTMLElement>('[role="status"]');
    expect(status?.textContent).toBe('!');
    expect(status?.getAttribute('aria-label')).toBe(
      'Microphone capture failed. Try recording again.',
    );
  });

  it('renders the exact permission and partial-transcription actions', async () => {
    await act(async () => root.render(
      <CuePill indicator={{ kind: 'microphoneFailure', failure: 'openMicrophoneSettings' }} />,
    ));
    expect(container.querySelector<HTMLElement>('[role="status"]')?.getAttribute('aria-label'))
      .toBe('Microphone access denied. Open System Settings to grant access.');

    await act(async () => root.render(
      <CuePill indicator={{
        kind: 'microphoneFailure',
        failure: 'waitForPartialTranscription',
      }} />,
    ));
    expect(container.querySelector<HTMLElement>('[role="status"]')?.getAttribute('aria-label'))
      .toBe('Microphone capture was interrupted. Waiting for the partial transcription.');
  });

  it('identifies a verified Smart Auto route without exposing its device ID', async () => {
    await act(async () => root.render(
      <CuePill
        indicator={{ kind: 'idle', dimmed: false }}
        smartAutoSummary={{ kind: 'ready', deviceName: 'USB Microphone', reason: 'preferred included microphone' }}
      />,
    ));

    const status = container.querySelector<HTMLElement>('[role="status"]');
    expect(status?.getAttribute('aria-label')).toBe('Smart Auto next capture ready with USB Microphone. preferred included microphone.');
    expect(status?.getAttribute('title')).toBe('Smart Auto: USB Microphone. preferred included microphone.');
    expect(status?.getAttribute('aria-live')).toBe('polite');
    expect(status?.textContent).not.toContain('usb-device-id');
  });

  it('shows when Smart Auto needs verification while leaving higher-priority cues alone', async () => {
    await act(async () => root.render(
      <CuePill indicator={{ kind: 'idle', dimmed: false }} smartAutoSummary={{ kind: 'blocked', retryAfterMs: null }} />,
    ));
    expect(container.querySelector<HTMLElement>('[role="status"]')?.getAttribute('aria-label'))
      .toBe('Smart Auto next capture blocked. Open Settings to retry, verify signal, or pin an input.');

    await act(async () => root.render(
      <CuePill indicator={{ kind: 'clipboardOnly' }} smartAutoSummary={{ kind: 'blocked', retryAfterMs: null }} />,
    ));
    expect(container.querySelector<HTMLElement>('[role="status"]')?.getAttribute('aria-label'))
      .toBe('Text copied to clipboard. Paste manually or try again.');
  });

  it('describes a background probe without presenting it as a manual preview', async () => {
    await act(async () => root.render(
      <CuePill
        indicator={{ kind: 'idle', dimmed: false }}
        smartAutoSummary={{ kind: 'probing', deviceName: 'USB Microphone', phase: 'checking signal' }}
      />,
    ));
    expect(container.querySelector<HTMLElement>('[role="status"]')?.getAttribute('aria-label'))
      .toBe('Smart Auto background check for USB Microphone: checking signal. Audio is not transcribed or saved.');
    expect(container.textContent).not.toContain('preview');
  });

  it('describes a cooldown without promising that background probing is enabled', async () => {
    await act(async () => root.render(
      <CuePill indicator={{ kind: 'idle', dimmed: false }} smartAutoSummary={{ kind: 'blocked', retryAfterMs: 60_000 }} />,
    ));
    const status = container.querySelector<HTMLElement>('[role="status"]');
    expect(status?.getAttribute('aria-label')).toBe('Smart Auto next capture blocked. Cooldown: about 60 seconds remaining.');
    expect(status?.getAttribute('title')).toBe('Smart Auto blocked. Waiting for cooldown.');
    expect(status?.getAttribute('aria-label')).not.toContain('scheduled');
  });

  it('keeps the wing on the waveform while recording — transcript text lives in the preview popover', async () => {
    function LivePill() {
      const barRefs = useRef<(HTMLDivElement | null)[]>([]);
      return <OverlayPill
        geometry={geometry}
        visual={{ indicator: { kind: 'recording' }, showTapMissedLabel: false, waveformVisible: true }}
        status="recording"
        barRefs={barRefs}
      />;
    }
    await act(async () => root.render(<LivePill />));
    // A 36pt wing can only show ~4 head-truncated characters, so provisional
    // text renders in the `dictation-preview` window instead.
    expect(container.querySelector('[aria-label^="Live transcription preview:"]')).toBeNull();
    expect(container.querySelectorAll('.rounded-full.bg-white\\/90').length).toBe(BAR_COUNT);
    expect(container.querySelector('button')).toBeNull();
  });
});
