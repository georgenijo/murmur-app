import type { DiscoveryHintId } from '../lib/discovery';

const HINTS: Record<DiscoveryHintId, { title: string; detail: string; action: string }> = {
  double_tap_timing: {
    title: 'A double-tap arrived too slowly',
    detail: 'The two taps need to land close together. You can turn on visual timing feedback in Recording settings.',
    action: 'Timing settings',
  },
  correct_and_teach: {
    title: 'Fix a word once, then teach Murmur',
    detail: 'Correct & Teach on your latest dictation can remember the spelling for next time.',
    action: 'Correct & Teach',
  },
  browser_modes: {
    title: 'Use a Mode for this site',
    detail: 'Murmur can apply a Mode to an exact browser site without storing the page or its text.',
    action: 'Set up site Modes',
  },
  meeting_summaries: {
    title: 'Your meeting can become a summary',
    detail: 'Open the finished meeting to generate a local summary, decisions, and action items.',
    action: 'View meeting',
  },
};

interface DiscoveryHintToastProps {
  hint: DiscoveryHintId;
  onAction: (hint: DiscoveryHintId) => void;
  onDismiss: (hint: DiscoveryHintId) => void;
}

export function DiscoveryHintToast({ hint, onAction, onDismiss }: DiscoveryHintToastProps) {
  const content = HINTS[hint];
  return (
    <aside className="discovery-hint dialog-toast" aria-labelledby="discovery-hint-title" aria-live="polite">
      <div className="min-w-0 flex-1">
        <p id="discovery-hint-title" className="text-sm font-semibold text-on-surface">{content.title}</p>
        <p className="mt-0.5 text-xs leading-relaxed text-on-surface-variant">{content.detail}</p>
        <button type="button" onClick={() => onAction(hint)} className="mt-2 text-xs font-semibold text-primary hover:underline">
          {content.action}
        </button>
      </div>
      <button type="button" onClick={() => onDismiss(hint)} aria-label={`Dismiss ${content.title}`} className="ui-icon-button shrink-0">×</button>
    </aside>
  );
}
