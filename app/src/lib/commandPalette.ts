/**
 * Command registry and matching for the ⌘K palette.
 *
 * Ranking is deliberately explicit rather than a generic fuzzy library: with a
 * fixed, small command set, predictable tiers ("prefix beats substring beats
 * subsequence") make the first result feel obvious, and they are easy to pin
 * down in tests.
 */

export interface PaletteCommand {
  id: string;
  /** What the row reads as. Matched against with the highest weight. */
  title: string;
  /** Group label shown on the right of the row. */
  section: string;
  /** Extra spoken/typed synonyms that should find this command. */
  keywords?: string[];
  /** Short right-hand hint, e.g. a keyboard shortcut or current state. */
  hint?: string;
  run: () => void | Promise<void>;
}

/** Score tiers, highest first. Sums across query tokens. */
const EXACT_TITLE = 1000;
const TITLE_PREFIX = 800;
const TITLE_WORD_PREFIX = 700;
const TITLE_SUBSTRING = 600;
const ALIAS_SUBSTRING = 400;
const TITLE_SUBSEQUENCE = 200;

export function paletteTokens(query: string): string[] {
  return query.toLowerCase().split(/\s+/).filter(Boolean);
}

/** True when every character of `needle` appears in `haystack` in order. */
export function isSubsequence(needle: string, haystack: string): boolean {
  if (needle.length === 0) return true;
  let index = 0;
  for (const char of haystack) {
    if (char === needle[index]) index++;
    if (index === needle.length) return true;
  }
  return false;
}

function tokenScore(command: PaletteCommand, token: string): number | null {
  const title = command.title.toLowerCase();
  if (title === token) return EXACT_TITLE;
  if (title.startsWith(token)) return TITLE_PREFIX;
  if (title.split(/\s+/).some((word) => word.startsWith(token))) return TITLE_WORD_PREFIX;
  if (title.includes(token)) return TITLE_SUBSTRING;
  const aliases = [command.section, ...(command.keywords ?? [])].map((alias) => alias.toLowerCase());
  if (aliases.some((alias) => alias.includes(token))) return ALIAS_SUBSTRING;
  if (isSubsequence(token, title)) return TITLE_SUBSEQUENCE;
  return null;
}

/**
 * Total score for a command against a query, or `null` when any token fails to
 * match (tokens are ANDed, so "set del" narrows rather than widens).
 */
export function scoreCommand(command: PaletteCommand, query: string): number | null {
  const tokens = paletteTokens(query);
  if (tokens.length === 0) return 0;
  let total = 0;
  for (const token of tokens) {
    const score = tokenScore(command, token);
    if (score === null) return null;
    total += score;
  }
  // Shorter titles win ties: "Settings" should outrank "Settings: Transcription".
  return total - Math.min(command.title.length, 99) / 100;
}

/** Matching commands, best first; declaration order breaks ties. */
export function filterCommands(commands: PaletteCommand[], query: string): PaletteCommand[] {
  return commands
    .map((command, index) => ({ command, index, score: scoreCommand(command, query) }))
    .filter((candidate): candidate is { command: PaletteCommand; index: number; score: number } =>
      candidate.score !== null)
    .sort((a, b) => (b.score - a.score) || (a.index - b.index))
    .map(({ command }) => command);
}

/** Move the highlighted row, wrapping at both ends. */
export function moveSelection(current: number, delta: number, length: number): number {
  if (length === 0) return 0;
  return ((current + delta) % length + length) % length;
}
