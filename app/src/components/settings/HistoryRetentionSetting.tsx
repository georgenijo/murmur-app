import { useState } from 'react';
import { Button } from '../ui/FormControls';
import { Select } from '../ui/Select';
import { SettingsRow } from './SettingsRow';
import {
  DEFAULT_HISTORY_RETENTION_DAYS,
  HISTORY_RETENTION_OPTIONS,
  isHistoryRetentionDays,
  type HistoryRetentionDays,
} from '../../lib/history';

const ITEMS = HISTORY_RETENTION_OPTIONS.map((option) => ({ value: String(option.value), label: option.label }));

function ageLabel(days: HistoryRetentionDays): string {
  return HISTORY_RETENTION_OPTIONS.find((option) => option.value === days)?.age ?? '';
}

function transcriptsLabel(count: number): string {
  return `${count} ${count === 1 ? 'transcript' : 'transcripts'}`;
}

/**
 * Opt-in automatic deletion of old transcripts. Off ("Never") by default.
 * A choice that would delete transcripts already in History waits for an
 * explicit confirmation that names how many; a choice that deletes nothing
 * right now (including "Never") applies immediately.
 */
export function HistoryRetentionSetting({ value, onChange, countExpired }: {
  value: HistoryRetentionDays;
  onChange: (days: HistoryRetentionDays) => void;
  /** Entries the given window would delete now. Absent means none can be counted. */
  countExpired?: (days: HistoryRetentionDays) => number;
}) {
  const [pending, setPending] = useState<{ days: HistoryRetentionDays; count: number } | null>(null);

  const choose = (raw: string) => {
    const days = Number(raw);
    if (!isHistoryRetentionDays(days)) return;
    const count = days === 0 ? 0 : countExpired?.(days) ?? 0;
    if (count > 0) {
      setPending({ days, count });
      return;
    }
    setPending(null);
    if (days !== value) onChange(days);
  };

  const confirm = () => {
    if (!pending) return;
    setPending(null);
    onChange(pending.days);
  };

  const description = value === 0
    ? 'Keep transcripts until you delete them. Pinned transcripts are always kept.'
    : `Transcripts older than ${ageLabel(value)} are deleted from this Mac. Pinned transcripts are always kept.`;

  return (
    <SettingsRow
      targetId="history-retention"
      title="Delete transcripts automatically"
      description={description}
      onReset={value !== DEFAULT_HISTORY_RETENTION_DAYS ? () => choose(String(DEFAULT_HISTORY_RETENTION_DAYS)) : undefined}
      control={<Select value={String(pending?.days ?? value)} onChange={choose} items={ITEMS} />}
    >
      {pending && (
        <div role="alert" data-testid="history-retention-confirm" className="mt-2 flex flex-wrap items-center gap-2 text-[length:var(--ui-font-caption)] text-on-surface">
          <span>
            {`This deletes ${transcriptsLabel(pending.count)} older than ${ageLabel(pending.days)} now. Pinned transcripts are kept.`}
          </span>
          <span className="flex gap-2">
            <Button type="button" size="xs" variant="primary" onClick={confirm}>
              {`Delete ${transcriptsLabel(pending.count)}`}
            </Button>
            <Button type="button" size="xs" variant="ghost" onClick={() => setPending(null)}>Cancel</Button>
          </span>
        </div>
      )}
    </SettingsRow>
  );
}
