import { act, useRef } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { OverlayGeometry } from '../../lib/overlayGeometry';
import { OverlayPill } from './OverlayPill';
import type { OverlayIndicator } from './deriveVisual';

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

function CuePill({ indicator }: { indicator: OverlayIndicator }) {
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

  it('renders an accessible, non-interactive manual-paste status', async () => {
    await act(async () => root.render(<CuePill indicator={{ kind: 'clipboardOnly' }} />));

    const status = container.querySelector<HTMLElement>('[role="status"]');
    expect(status?.textContent).toBe('⌘V');
    expect(status?.getAttribute('aria-live')).toBe('polite');
    expect(status?.getAttribute('aria-label')).toBe('Text copied to clipboard. Paste manually.');
    expect(container.querySelector('button')).toBeNull();
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

  it('shows provisional dictation only while recording and keeps it non-interactive', async () => {
    function LivePill() {
      const barRefs = useRef<(HTMLDivElement | null)[]>([]);
      return <OverlayPill
        geometry={geometry}
        visual={{ indicator: { kind: 'recording' }, showTapMissedLabel: false, waveformVisible: true }}
        status="recording"
        partialText="safe live words"
        barRefs={barRefs}
      />;
    }
    await act(async () => root.render(<LivePill />));
    const preview = container.querySelector<HTMLElement>('[aria-label^="Live transcription preview:"]');
    expect(preview?.textContent).toBe('safe live words');
    expect(preview?.getAttribute('aria-live')).toBe('polite');
    expect(container.querySelector('button')).toBeNull();
  });
});
