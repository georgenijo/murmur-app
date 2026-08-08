/**
 * Compile-time marker for the private Murmur Bench flavor.
 *
 * Vite replaces this expression while bundling, allowing dead-code elimination
 * to remove the personal-corpus UI from normal consumer builds.
 */
export const INTERNAL_BENCHMARK_BUILD =
  import.meta.env.VITE_MURMUR_INTERNAL_BENCHMARK === '1';
