import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { TypingSpeedCard } from './TypingSpeedCard';
import { InsightsView } from './InsightsView';
import { TYPING_CHECK_PASSAGES } from '../../lib/typingSpeed';

let container: HTMLDivElement;
let root: Root;
let now = 0;
const clock = () => now;

async function render(node: React.ReactNode) {
  await act(async () => root.render(node));
}

const card = () => container.querySelector<HTMLElement>('.insights-typing')!;
const field = () => container.querySelector<HTMLTextAreaElement>('textarea')!;
const button = (label: string) => Array.from(container.querySelectorAll<HTMLButtonElement>('button')).find((element) => element.textContent === label);

async function click(label: string) {
  const target = button(label);
  if (!target) throw new Error(`no button ${label}`);
  await act(async () => target.click());
}

async function type(value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!;
  await act(async () => {
    setter.call(field(), value);
    field().dispatchEvent(new Event('input', { bubbles: true }));
  });
}

async function key(name: string) {
  await act(async () => {
    field().dispatchEvent(new KeyboardEvent('keydown', { key: name, bubbles: true, cancelable: true }));
  });
}

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  localStorage.clear();
  now = 1000;
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

describe('TypingSpeedCard', () => {
  it('starts off with a Measure button and the assumed speed explained', async () => {
    await render(<TypingSpeedCard typingWpm={0} dictationWpm={130} onTypingWpmChange={() => {}} clock={clock} />);
    expect(card().dataset.typingState).toBe('off');
    expect(card().textContent).toContain('assumed 40 wpm');
    expect(card().textContent).toContain('never what you type');
    expect(field()).toBeNull();
    expect(button('Measure')).toBeDefined();
  });

  it('measures a full sentence, finishes on the last character and saves only the speed', async () => {
    const onChange = vi.fn();
    await render(<TypingSpeedCard typingWpm={0} dictationWpm={130} onTypingWpmChange={onChange} clock={clock} />);
    await click('Measure');
    expect(card().dataset.typingState).toBe('typing');
    const passage = TYPING_CHECK_PASSAGES[0];
    expect(container.querySelector('.insights-typing-passage')?.textContent).toBe(passage);
    expect(document.activeElement).toBe(field());
    expect(button('Done')?.disabled).toBe(true);

    await type(passage.slice(0, 1));
    expect(button('Done')?.disabled).toBe(false);
    now += 18_000;
    await type(passage);

    expect(card().dataset.typingState).toBe('scored');
    const expected = Math.round((passage.length / 5) / (18_000 / 60_000));
    expect(container.querySelector('[role="status"]')?.textContent).toBe(`${expected} wpm at 100% accuracy`);
    expect(onChange).not.toHaveBeenCalled();
    await click(`Use ${expected} wpm`);
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenCalledWith(expected);
    // Nothing typed is kept anywhere the card can reach.
    expect(localStorage.length).toBe(0);
  });

  it('Enter finishes early, Escape cancels, and a short run is refused', async () => {
    const onChange = vi.fn();
    await render(<TypingSpeedCard typingWpm={0} dictationWpm={0} onTypingWpmChange={onChange} clock={clock} />);
    await click('Measure');
    await type('Th');
    await key('Escape');
    expect(card().dataset.typingState).toBe('off');

    await click('Measure');
    // A new attempt uses the next sentence and starts empty.
    expect(container.querySelector('.insights-typing-passage')?.textContent).toBe(TYPING_CHECK_PASSAGES[1]);
    expect(field().value).toBe('');
    await type('Pl');
    now += 1000;
    await type('Please send');
    await key('Enter');
    expect(card().dataset.typingState).toBe('scored');
    expect(container.querySelector('[role="status"]')?.textContent).toContain('too short');
    expect(button('Use')).toBeUndefined();
    await click('Cancel');
    expect(card().dataset.typingState).toBe('off');
    expect(onChange).not.toHaveBeenCalled();
  });

  it('refuses a run that is mostly typos', async () => {
    await render(<TypingSpeedCard typingWpm={0} dictationWpm={0} onTypingWpmChange={() => {}} clock={clock} />);
    await click('Measure');
    await type('x');
    now += 10_000;
    await type('x'.repeat(40));
    await click('Done');
    expect(container.querySelector('[role="status"]')?.textContent).toContain('% of characters matched');
  });

  it('blocks pasting the sentence', async () => {
    await render(<TypingSpeedCard typingWpm={0} dictationWpm={0} onTypingWpmChange={() => {}} clock={clock} />);
    await click('Measure');
    const paste = new Event('paste', { bubbles: true, cancelable: true });
    await act(async () => { field().dispatchEvent(paste); });
    expect(paste.defaultPrevented).toBe(true);
  });

  it('shows a saved speed against dictation and can clear it', async () => {
    const onChange = vi.fn();
    await render(<TypingSpeedCard typingWpm={62} dictationWpm={135} onTypingWpmChange={onChange} clock={clock} />);
    expect(card().dataset.typingState).toBe('set');
    expect(container.querySelector('.insights-typing-summary')?.textContent).toBe('62 wpm typing · dictation averages 135 wpm, 2.2× your typing');
    await click('Clear');
    expect(onChange).toHaveBeenCalledWith(0);
  });

  it('omits the comparison when there is no dictation speed yet', async () => {
    await render(<TypingSpeedCard typingWpm={62} dictationWpm={0} onTypingWpmChange={() => {}} clock={clock} />);
    expect(container.querySelector('.insights-typing-summary')?.textContent).toBe('62 wpm typing');
  });
});

describe('InsightsView typing speed', () => {
  const headline = () => container.querySelector('[data-stats="headline"]')?.textContent ?? '';

  it('keeps the 40 wpm assumption and shows no card by default', async () => {
    await render(<InsightsView statsVersion={0} onBackToHome={() => {}} />);
    expect(container.querySelector('.insights-typing')).toBeNull();
    expect(headline()).toContain('vs typing at 40 wpm');
    await render(<InsightsView statsVersion={0} onTypingSpeedWpmChange={() => {}} onBackToHome={() => {}} />);
    expect(container.querySelector('.insights-typing')?.getAttribute('data-typing-state')).toBe('off');
    expect(headline()).toContain('vs typing at 40 wpm');
  });

  it('uses a measured speed for time saved', async () => {
    await render(<InsightsView statsVersion={0} typingSpeedWpm={60} onTypingSpeedWpmChange={() => {}} onBackToHome={() => {}} />);
    expect(container.querySelector('.insights-typing')?.getAttribute('data-typing-state')).toBe('set');
    expect(headline()).toContain('vs your typing at 60 wpm');
  });
});
