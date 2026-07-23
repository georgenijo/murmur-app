import type { AppEvent } from './events';

export function transformPassIdOf(event: AppEvent): number | null {
  const value = event.data?.transform_pass_id;
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0
    ? value
    : null;
}

export function matchesTransformPassId(event: AppEvent, query: string): boolean {
  const trimmed = query.trim();
  if (trimmed === '') return true;
  if (!/^[1-9]\d*$/.test(trimmed)) return false;
  return transformPassIdOf(event) === Number(trimmed);
}

export function formatEventForCopy(event: AppEvent): string {
  const data = Object.keys(event.data ?? {}).length > 0
    ? ` ${JSON.stringify(event.data)}`
    : '';
  return `${event.timestamp} [${event.stream}] ${event.level.toUpperCase()} ${event.summary}${data}`;
}
