import { describe, expect, it } from 'vitest';
import { LIVE_WORDS_LINE_MAX_CHARS, liveWordsTail } from './liveWordsLine';

describe('liveWordsTail', () => {
  it('collapses whitespace and trims, keeping short text whole', () => {
    expect(liveWordsTail('  one\n two   three ')).toBe('one two three');
    expect(liveWordsTail('')).toBe('');
  });

  it('keeps the newest words, starting at a word boundary', () => {
    const words = Array.from({ length: 80 }, (_, i) => `word${i}`).join(' ');
    const tail = liveWordsTail(words);
    expect(tail.length).toBeLessThanOrEqual(LIVE_WORDS_LINE_MAX_CHARS);
    expect(words.endsWith(tail)).toBe(true);
    expect(tail.startsWith('word')).toBe(true);
    expect(tail.endsWith('word79')).toBe(true);
  });

  it('cuts unbroken text (CJK, one very long word) anywhere without growing', () => {
    const cjk = '今日は天気がとても良いので散歩に行きます'.repeat(20);
    expect(liveWordsTail(cjk)).toHaveLength(LIVE_WORDS_LINE_MAX_CHARS);
    const long = 'a'.repeat(500);
    expect(liveWordsTail(long)).toHaveLength(LIVE_WORDS_LINE_MAX_CHARS);
  });
});
