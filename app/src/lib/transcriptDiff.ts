/**
 * Word-level comparison between a history entry's raw recognition and the
 * text Murmur delivered. Display-only: nothing here feeds delivery, history
 * writes, statistics, or logs.
 */
import type { HistoryStageResult } from './history';

export type TranscriptDiffKind = 'same' | 'removed' | 'added';

export interface TranscriptDiffSegment {
  kind: TranscriptDiffKind;
  text: string;
}

/**
 * Upper bound on the LCS table (raw tokens × delivered tokens). 250k cells
 * keeps the comparison well under a frame for ordinary dictations; a pair
 * beyond it is reported as too long instead of blocking the main thread.
 */
export const MAX_DIFF_CELLS = 250_000;

function tokenize(text: string): string[] {
  return text.match(/\s+|\S+/g) ?? [];
}

const isSpace = (token: string) => /^\s+$/.test(token);

/**
 * Compare `raw` with `delivered` word by word. Returns `null` when the pair
 * is too long to compare within `MAX_DIFF_CELLS`.
 *
 * Whitespace-only differences are folded into unchanged text unless they add
 * or remove a line break, so a reflowed space never shows as an edit while a
 * spoken "new line" still does.
 */
export function diffTranscript(raw: string, delivered: string): TranscriptDiffSegment[] | null {
  const a = tokenize(raw);
  const b = tokenize(delivered);
  if ((a.length + 1) * (b.length + 1) > MAX_DIFF_CELLS) return null;

  const width = b.length + 1;
  const lcs = new Uint32Array((a.length + 1) * width);
  for (let i = a.length - 1; i >= 0; i -= 1) {
    for (let j = b.length - 1; j >= 0; j -= 1) {
      lcs[i * width + j] = a[i] === b[j]
        ? lcs[(i + 1) * width + j + 1] + 1
        : Math.max(lcs[(i + 1) * width + j], lcs[i * width + j + 1]);
    }
  }

  const raws: TranscriptDiffSegment[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length || j < b.length) {
    if (i < a.length && j < b.length && a[i] === b[j]) {
      raws.push({ kind: 'same', text: b[j] });
      i += 1;
      j += 1;
    } else if (i < a.length && (j === b.length || lcs[(i + 1) * width + j] >= lcs[i * width + j + 1])) {
      // Ties take the removal first so a replacement reads "old → new".
      raws.push({ kind: 'removed', text: a[i] });
      i += 1;
    } else {
      raws.push({ kind: 'added', text: b[j] });
      j += 1;
    }
  }

  // Merge first so spaces between removed or added words stay inside their
  // run ("um so" reads as one removal), then fold the runs that are only
  // a spacing change: a removed space disappears and an added one is plain
  // spacing in the delivered text.
  const folded = merge(raws).flatMap((segment): TranscriptDiffSegment[] => {
    if (segment.kind === 'same' || !isSpace(segment.text) || segment.text.includes('\n')) return [segment];
    return segment.kind === 'removed' ? [] : [{ kind: 'same', text: segment.text }];
  });
  return merge(folded);
}

function merge(segments: readonly TranscriptDiffSegment[]): TranscriptDiffSegment[] {
  const merged: TranscriptDiffSegment[] = [];
  for (const segment of segments) {
    const last = merged[merged.length - 1];
    if (last && last.kind === segment.kind) {
      last.text += segment.text;
    } else {
      merged.push({ ...segment });
    }
  }
  return merged;
}

/** Count of word-level edits, for a compact summary line. */
export function countDiffChanges(segments: readonly TranscriptDiffSegment[]): number {
  let count = 0;
  for (let index = 0; index < segments.length; index += 1) {
    const segment = segments[index];
    if (segment.kind === 'same') continue;
    // An adjacent removed→added pair is one replacement, not two edits.
    if (segment.kind === 'added' && segments[index - 1]?.kind === 'removed') continue;
    count += 1;
  }
  return count;
}

const STAGE_LABELS: Record<string, string> = {
  cleanup: 'Cleanup',
  smart_correction: 'Smart correction',
  homophone_context: 'Context fixes',
  smart_formatting: 'Smart formatting',
  spoken_structure: 'Spoken formatting',
  spoken_numbers: 'Spoken numbers',
  ide_context: 'IDE context',
  cli_command: 'Command formatting',
};

/** Human-readable names of the stages that changed the text, in pipeline order. */
export function changedStageLabels(stages: readonly HistoryStageResult[] | undefined): string[] {
  if (!stages) return [];
  const labels: string[] = [];
  for (const stage of stages) {
    if (!stage.changed) continue;
    const label = STAGE_LABELS[stage.stage]
      ?? stage.stage.replace(/_/g, ' ').replace(/^./, (first) => first.toUpperCase());
    if (!labels.includes(label)) labels.push(label);
  }
  return labels;
}
