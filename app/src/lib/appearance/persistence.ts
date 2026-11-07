import { isRecord } from '../typeGuards';

/** Keep additive source data opaque; known fields always come from validation.
 * Include optional known keys even when validation removes their values.
 * Portable exports must continue to use their explicit projections instead.
 */
export function preserveUnknownFields<T extends object>(
  source: unknown,
  validated: T,
  knownKeys: readonly string[],
): T {
  const opaque = isRecord(source)
    ? Object.fromEntries(Object.entries(source).filter(([key]) => !knownKeys.includes(key)))
    : {};
  return { ...opaque, ...validated };
}

/** JSON object order is not a reason to rewrite durable source state. */
export function sameSource(left: unknown, right: unknown): boolean {
  const stable = (value: unknown): unknown => Array.isArray(value)
    ? value.map(stable)
    : isRecord(value)
      ? Object.fromEntries(Object.keys(value).sort().map(key => [key, stable(value[key])]))
      : value;
  return JSON.stringify(stable(left)) === JSON.stringify(stable(right));
}
