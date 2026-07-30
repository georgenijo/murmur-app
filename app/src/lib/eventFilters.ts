import type { AppEvent } from './events';

export type CorrelationField =
  | 'run_id'
  | 'recording_id'
  | 'file_run_id'
  | 'transform_pass_id';

export interface CorrelationFilter {
  field: CorrelationField;
  value: string;
}

export const CORRELATION_FIELD_LABELS: Record<CorrelationField, string> = {
  run_id: 'Run ID',
  recording_id: 'Recording ID',
  file_run_id: 'File run ID',
  transform_pass_id: 'Transform pass ID',
};

const NUMERIC_CORRELATION_FIELDS = new Set<CorrelationField>([
  'recording_id',
  'file_run_id',
  'transform_pass_id',
]);

export function matchesCorrelation(
  event: AppEvent,
  filter: CorrelationFilter | null,
): boolean {
  if (!filter || filter.value.trim() === '') return true;
  const query = filter.value.trim();
  const candidate = event.data?.[filter.field];
  if (NUMERIC_CORRELATION_FIELDS.has(filter.field)) {
    return /^[1-9]\d*$/.test(query)
      && typeof candidate === 'number'
      && Number.isSafeInteger(candidate)
      && candidate === Number(query);
  }
  return typeof candidate === 'string' && candidate === query;
}

export function transformPassIdOf(event: AppEvent): number | null {
  const value = event.data?.transform_pass_id;
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0
    ? value
    : null;
}

export function matchesTransformPassId(event: AppEvent, query: string): boolean {
  return matchesCorrelation(event, {
    field: 'transform_pass_id',
    value: query,
  });
}

export function formatEventForCopy(event: AppEvent): string {
  const data = Object.keys(event.data ?? {}).length > 0
    ? ` ${JSON.stringify(event.data)}`
    : '';
  return `${event.timestamp} [${event.stream}] ${event.level.toUpperCase()} ${event.summary}${data}`;
}
