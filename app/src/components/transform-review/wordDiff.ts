// Small pure word-level diff (LCS over word tokens). No new dependency — the
// texts being diffed are short (a selection + its transform), so an O(n*m)
// LCS table is cheap.

export type DiffTokenKind = 'same' | 'removed' | 'added';

export interface DiffToken {
  kind: DiffTokenKind;
  text: string;
}

/** Layout hint for `ReviewDiff`: unified inline under ~200 combined chars, side-by-side above. */
export type DiffLayout = 'unified' | 'side-by-side';

const SIDE_BY_SIDE_THRESHOLD_CHARS = 200;

/**
 * Above this many tokens per side, the O(n*m) LCS table (and the O(n+m)
 * backtrack through it) gets expensive enough to jank the popover — a single
 * 2000x2000 table is 4M cells, and review text can in principle be an entire
 * pasted document. Beyond the cap we skip the table entirely and degrade to
 * a single removed-run + single added-run (no attempt at a "smart" diff),
 * which is O(n+m) and always fast regardless of input size.
 */
const MAX_DIFF_TOKENS_PER_SIDE = 2000;

/**
 * Split on runs of whitespace, keeping the whitespace itself as tokens so
 * diffed output preserves original spacing exactly.
 */
function tokenize(text: string): string[] {
  if (text.length === 0) return [];
  return text.split(/(\s+)/).filter((t) => t.length > 0);
}

/**
 * Word-level diff between `original` and `proposed` via LCS over whitespace-
 * preserving tokens. Returns a flat sequence of tokens tagged `same` /
 * `removed` (present only in `original`) / `added` (present only in
 * `proposed`) in display order: unchanged runs interleave with removed/added
 * runs the way a standard diff view expects.
 */
export function computeWordDiff(original: string, proposed: string): DiffToken[] {
  const a = tokenize(original);
  const b = tokenize(proposed);
  const n = a.length;
  const m = b.length;

  if (n === 0 && m === 0) return [];

  if (n > MAX_DIFF_TOKENS_PER_SIDE || m > MAX_DIFF_TOKENS_PER_SIDE) {
    return computeDegradedDiff(a, b);
  }

  // dp[i][j] = length of the LCS of a[i..] and b[j..].
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }

  const tokens: DiffToken[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      tokens.push({ kind: 'same', text: a[i] });
      i++;
      j++;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      tokens.push({ kind: 'removed', text: a[i] });
      i++;
    } else {
      tokens.push({ kind: 'added', text: b[j] });
      j++;
    }
  }
  while (i < n) {
    tokens.push({ kind: 'removed', text: a[i] });
    i++;
  }
  while (j < m) {
    tokens.push({ kind: 'added', text: b[j] });
    j++;
  }
  return tokens;
}

/**
 * Degraded shape for inputs beyond `MAX_DIFF_TOKENS_PER_SIDE`: the entire
 * original as one `removed` run followed by the entire proposed as one
 * `added` run. Still round-trips exactly (same reconstruction invariant the
 * real LCS diff guarantees) — just without word-level granularity.
 */
function computeDegradedDiff(a: string[], b: string[]): DiffToken[] {
  const tokens: DiffToken[] = [];
  if (a.length > 0) tokens.push({ kind: 'removed', text: a.join('') });
  if (b.length > 0) tokens.push({ kind: 'added', text: b.join('') });
  return tokens;
}

/** Unified (inline) layout for short diffs; side-by-side once combined length grows. */
export function pickDiffLayout(original: string, proposed: string): DiffLayout {
  return original.length + proposed.length < SIDE_BY_SIDE_THRESHOLD_CHARS
    ? 'unified'
    : 'side-by-side';
}
