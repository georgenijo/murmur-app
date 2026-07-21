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

/** Unified (inline) layout for short diffs; side-by-side once combined length grows. */
export function pickDiffLayout(original: string, proposed: string): DiffLayout {
  return original.length + proposed.length < SIDE_BY_SIDE_THRESHOLD_CHARS
    ? 'unified'
    : 'side-by-side';
}
