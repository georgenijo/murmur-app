import { useEffect, useId, useRef, useState, type ChangeEvent, type KeyboardEvent } from 'react';
import { ASSUMED_TYPING_WPM } from '../../lib/activityStats';
import {
  TYPING_CHECK_PASSAGES,
  TYPING_SPEED_OFF,
  dictationSpeedup,
  sanitizeTypingSpeedWpm,
  scoreTypingCheck,
  type TypingCheckResult,
} from '../../lib/typingSpeed';
import { Button } from '../ui/FormControls';

interface TypingSpeedCardProps {
  /** Saved typing speed; 0 = not measured (default). */
  typingWpm: number;
  /** Average dictation speed from local samples; 0 when there are none. */
  dictationWpm: number;
  onTypingWpmChange: (wpm: number) => void;
  /** Injectable monotonic clock for tests. */
  clock?: () => number;
}

type Phase =
  | { kind: 'idle' }
  | { kind: 'typing'; passage: string; startedAt: number | null }
  | { kind: 'scored'; result: TypingCheckResult };

const defaultClock = () => performance.now();

export function TypingSpeedCard({ typingWpm: typingWpmSetting, dictationWpm, onTypingWpmChange, clock = defaultClock }: TypingSpeedCardProps) {
  const titleId = useId();
  const passageId = useId();
  const typingWpm = sanitizeTypingSpeedWpm(typingWpmSetting);
  const [phase, setPhase] = useState<Phase>({ kind: 'idle' });
  const [typed, setTyped] = useState('');
  const passageIndex = useRef(0);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (phase.kind === 'typing') inputRef.current?.focus();
  }, [phase.kind]);

  const start = () => {
    const passage = TYPING_CHECK_PASSAGES[passageIndex.current % TYPING_CHECK_PASSAGES.length];
    passageIndex.current++;
    setTyped('');
    setPhase({ kind: 'typing', passage, startedAt: null });
  };

  const finish = (text: string, startedAt: number | null, passage: string) => {
    const elapsed = startedAt === null ? 0 : clock() - startedAt;
    setTyped('');
    setPhase({ kind: 'scored', result: scoreTypingCheck(passage, text, elapsed) });
  };

  const cancel = () => {
    setTyped('');
    setPhase({ kind: 'idle' });
  };

  const save = (result: TypingCheckResult) => {
    if (!result.ok) return;
    setPhase({ kind: 'idle' });
    onTypingWpmChange(result.wpm);
  };

  const onInput = (event: ChangeEvent<HTMLTextAreaElement>) => {
    if (phase.kind !== 'typing') return;
    const text = event.target.value;
    const startedAt = phase.startedAt ?? (text.length > 0 ? clock() : null);
    if (startedAt !== phase.startedAt) setPhase({ ...phase, startedAt });
    if (text.length >= phase.passage.length) {
      finish(text, startedAt, phase.passage);
      return;
    }
    setTyped(text);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (phase.kind !== 'typing') return;
    if (event.key === 'Escape') {
      // Escape belongs to the check here, not to any window-level handler.
      event.preventDefault();
      event.stopPropagation();
      cancel();
    } else if (event.key === 'Enter') {
      // The passage has no line breaks; Enter finishes early instead.
      event.preventDefault();
      finish(typed, phase.startedAt, phase.passage);
    }
  };

  const state = phase.kind === 'typing' ? 'typing' : phase.kind === 'scored' ? 'scored' : typingWpm === TYPING_SPEED_OFF ? 'off' : 'set';
  const speedup = dictationSpeedup(dictationWpm, typingWpm);

  return (
    <section className="insights-typing" aria-labelledby={titleId} data-typing-state={state}>
      <div className="insights-typing-header">
        <h2 id={titleId}>Your typing speed</h2>
        {state === 'off' && <Button size="sm" onClick={start}>Measure</Button>}
        {state === 'set' && (
          <div className="insights-typing-actions">
            <Button size="sm" variant="ghost" onClick={() => onTypingWpmChange(TYPING_SPEED_OFF)}>Clear</Button>
            <Button size="sm" onClick={start}>Measure again</Button>
          </div>
        )}
      </div>

      {state === 'off' && (
        <p className="insights-typing-hint">
          Type one short sentence to replace the assumed {ASSUMED_TYPING_WPM} wpm in time saved. Only the speed is kept, never what you type. Dictation itself is unchanged.
        </p>
      )}

      {state === 'set' && (
        <p className="insights-typing-summary">
          <strong>{typingWpm.toLocaleString()} wpm</strong> typing
          {speedup !== null && <> · dictation averages {dictationWpm.toLocaleString()} wpm, <strong data-typing-speedup>{speedup.toLocaleString(undefined, { minimumFractionDigits: 1, maximumFractionDigits: 1 })}×</strong> your typing</>}
        </p>
      )}

      {phase.kind === 'typing' && (
        <div className="insights-typing-check">
          <p id={passageId} className="insights-typing-passage">{phase.passage}</p>
          <textarea
            ref={inputRef}
            className="ui-form-input insights-typing-input"
            aria-label="Type the sentence above"
            aria-describedby={passageId}
            rows={2}
            value={typed}
            onChange={onInput}
            onKeyDown={onKeyDown}
            onPaste={(event) => event.preventDefault()}
            onDrop={(event) => event.preventDefault()}
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
          />
          <div className="insights-typing-footer">
            <span className="insights-typing-hint">
              {phase.startedAt === null ? 'The timer starts at your first key.' : 'Enter finishes early · Escape cancels'}
            </span>
            <div className="insights-typing-actions">
              <Button size="sm" variant="ghost" onClick={cancel}>Cancel</Button>
              <Button size="sm" onClick={() => finish(typed, phase.startedAt, phase.passage)} disabled={phase.startedAt === null}>Done</Button>
            </div>
          </div>
        </div>
      )}

      {phase.kind === 'scored' && (
        <div className="insights-typing-check">
          <p className="insights-typing-result" role="status">
            {phase.result.ok
              ? <><strong>{phase.result.wpm.toLocaleString()} wpm</strong> at {Math.round(phase.result.accuracy * 100)}% accuracy</>
              : phase.result.reason === 'too-short'
                ? 'That was too short to measure. Type at least half the sentence.'
                : `Only ${Math.round(phase.result.accuracy * 100)}% of characters matched, so the speed would be misleading.`}
          </p>
          <div className="insights-typing-actions">
            <Button size="sm" variant="ghost" onClick={cancel}>{phase.result.ok ? 'Discard' : 'Cancel'}</Button>
            <Button size="sm" variant={phase.result.ok ? 'outline' : 'primary'} onClick={start}>Try again</Button>
            {phase.result.ok && (
              <Button size="sm" variant="primary" onClick={() => save(phase.result)}>
                Use {phase.result.wpm.toLocaleString()} wpm
              </Button>
            )}
          </div>
        </div>
      )}
    </section>
  );
}
