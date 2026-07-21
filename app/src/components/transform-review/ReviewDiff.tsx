import { computeWordDiff, pickDiffLayout } from './wordDiff';
import type { DiffToken } from './wordDiff';

interface ReviewDiffProps {
  original: string;
  proposed: string;
}

function tokenClassName(kind: DiffToken['kind'], side: 'original' | 'proposed'): string {
  if (kind === 'same') return '';
  if (kind === 'removed') return side === 'original' ? 'line-through text-red-400/80' : '';
  return side === 'proposed' ? 'text-emerald-400/90' : '';
}

/**
 * Word-level diff view: original struck/red, proposed green. Renders unified
 * (single inline block) under ~200 combined chars, side-by-side above —
 * `pickDiffLayout` decides which. Purely presentational over `computeWordDiff`.
 */
export function ReviewDiff({ original, proposed }: ReviewDiffProps) {
  const layout = pickDiffLayout(original, proposed);
  const tokens = computeWordDiff(original, proposed);

  if (layout === 'unified') {
    return (
      <div className="px-3 py-2 text-[12px] leading-relaxed text-white/80 max-h-40 overflow-y-auto">
        {tokens.map((t, i) => (
          <span key={i} className={tokenClassName(t.kind, t.kind === 'removed' ? 'original' : 'proposed')}>
            {t.text}
          </span>
        ))}
      </div>
    );
  }

  return (
    <div className="px-3 py-2 grid grid-cols-2 gap-3 text-[12px] leading-relaxed max-h-40 overflow-y-auto">
      <div className="text-white/70 border-r border-white/10 pr-3">
        {tokens.filter((t) => t.kind !== 'added').map((t, i) => (
          <span key={i} className={tokenClassName(t.kind, 'original')}>{t.text}</span>
        ))}
      </div>
      <div className="text-white/90 pl-1">
        {tokens.filter((t) => t.kind !== 'removed').map((t, i) => (
          <span key={i} className={tokenClassName(t.kind, 'proposed')}>{t.text}</span>
        ))}
      </div>
    </div>
  );
}
