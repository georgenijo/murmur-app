/**
 * Canonical, small type guards reused across `lib/*` sanitizers and IPC
 * payload validators. Each was previously re-declared identically (or with
 * only a different name) in more than a dozen modules; see F11 cleanup
 * context. Only guards that are byte-identical in every prior call site were
 * unified here — anything with different semantics (e.g. accepting arrays,
 * or a differently bounded integer) stays local to its module.
 */

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

export function isNonNegativeInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 0;
}

export function isPositiveInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) > 0;
}
