import type { ReviewViewModel } from './deriveReviewState';

const WAVEFORM_BAR_COUNT = 5;

interface ReviewChipProps {
  vm: ReviewViewModel;
}

/**
 * Header row: instruction chip (placeholder while listening, the actual
 * instruction text from `thinking` onward), the listening-only waveform-style
 * indicator, the thinking status line (with the "Still working…" hint after
 * 5s), and the small "on-device" badge. Purely presentational — driven
 * entirely by the `ReviewViewModel` from `deriveReviewState`.
 */
export function ReviewChip({ vm }: ReviewChipProps) {
  return (
    <div className="flex items-start gap-2 px-3 pt-3">
      <div className="flex-1 min-w-0">
        <div className="text-[13px] font-medium text-white/90 truncate">{vm.chipText}</div>
        {vm.subText && <div className="text-[11px] text-white/50 mt-0.5">{vm.subText}</div>}
        {vm.statusText && (
          <div className="text-[11px] text-white/50 mt-0.5 flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 border-[1.5px] border-white/20 border-t-white/70 rounded-full animate-spin inline-block shrink-0" />
            <span>{vm.statusText}</span>
            {vm.showStillWorkingHint && <span className="text-white/35">· Still working…</span>}
          </div>
        )}
      </div>

      {vm.showWaveform && (
        <div className="flex items-center gap-[2px] h-4 shrink-0 mt-0.5" aria-hidden="true">
          {Array.from({ length: WAVEFORM_BAR_COUNT }, (_, i) => (
            <div
              key={i}
              className="w-[2px] rounded-full bg-white/70"
              style={{
                height: `${4 + (i % 3) * 4}px`,
                animation: 'pulse 0.9s ease-in-out infinite',
                animationDelay: `${i * 90}ms`,
              }}
            />
          ))}
        </div>
      )}

      {vm.showOnDeviceBadge && (
        <span className="shrink-0 text-[9px] uppercase tracking-wide text-white/40 border border-white/15 rounded-full px-1.5 py-0.5 mt-0.5">
          On-device
        </span>
      )}
    </div>
  );
}
