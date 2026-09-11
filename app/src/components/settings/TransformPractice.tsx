import { useRef, useState } from 'react';
import { TRANSFORM_KEY_OPTIONS, type Settings } from '../../lib/settings';

const SAMPLE_TEXT = 'The project update is quite long and could be made much more direct for the reader.';

interface TransformPracticeProps {
  transformHoldKey: Settings['transformHoldKey'];
  modelReady: boolean;
}

export function TransformPractice({ transformHoldKey, modelReady }: TransformPracticeProps) {
  const sampleRef = useRef<HTMLTextAreaElement>(null);
  const [selected, setSelected] = useState(false);
  const keyLabel = TRANSFORM_KEY_OPTIONS.find((option) => option.value === transformHoldKey)?.label;
  const ready = transformHoldKey !== null && modelReady;

  const selectSample = () => {
    const sample = sampleRef.current;
    if (!sample) return;
    sample.focus();
    sample.setSelectionRange(0, sample.value.length);
    setSelected(true);
  };

  return (
    <section data-setting-target="transform-practice" className="settings-card transform-practice" aria-labelledby="transform-practice-title">
      <div>
        <h2 id="transform-practice-title" className="text-sm font-medium text-on-surface">Practice on sample text</h2>
        <p className="mt-0.5 text-xs leading-relaxed text-on-surface-variant">
          This uses the real selected-text flow. Murmur changes nothing until you approve the review.
        </p>
      </div>
      <textarea ref={sampleRef} defaultValue={SAMPLE_TEXT} aria-label="Transform practice sample text" rows={3} />
      <div className="flex flex-wrap items-center gap-2">
        <button type="button" onClick={selectSample} disabled={!ready} className="rounded-(--ui-radius-pill) bg-primary px-3 py-1.5 text-xs font-semibold text-on-primary shadow-(--ui-shadow-accent) disabled:cursor-not-allowed disabled:opacity-50">
          Select sample text
        </button>
        <p role="status" aria-live="polite" className="text-xs text-on-surface-variant">
          {!modelReady
            ? 'Download the on-device model below first.'
            : transformHoldKey === null
              ? 'Enable the Transform shortcut above first.'
              : selected
                ? `Keep this text selected. Hold ${keyLabel}, say “make this shorter,” then release.`
                : `Select the sample, then hold ${keyLabel} and speak a rewrite instruction.`}
        </p>
      </div>
    </section>
  );
}
