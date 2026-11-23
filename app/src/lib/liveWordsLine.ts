/** Characters kept for the pill's single line. The line is about 20 characters
 * wide, so this is generous; it only bounds the DOM for a long dictation. */
export const LIVE_WORDS_LINE_MAX_CHARS = 160;

/** The newest words of a partial transcript, cut at a word boundary when there
 * is one. Text without spaces (CJK, one very long word) is cut anywhere. */
export function liveWordsTail(text: string, max = LIVE_WORDS_LINE_MAX_CHARS): string {
  const trimmed = text.replace(/\s+/g, ' ').trim();
  if (trimmed.length <= max) return trimmed;
  const tail = trimmed.slice(-max);
  const boundary = tail.indexOf(' ');
  return boundary === -1 ? tail : tail.slice(boundary + 1);
}
