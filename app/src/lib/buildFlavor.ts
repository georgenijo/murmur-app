/**
 * Compile-time marker for the private Murmur Bench flavor.
 *
 * Vite replaces this expression while bundling, allowing dead-code elimination
 * to remove the personal-corpus UI from normal consumer builds.
 */
export const INTERNAL_BENCHMARK_BUILD =
  import.meta.env.VITE_MURMUR_INTERNAL_BENCHMARK === '1';

/**
 * Compile-time marker for the "Murmur Speed Test" experiment variant
 * (`tauri.speed-test.conf.json`). It is a separate app with its own identifier,
 * data and logs. It never self-updates, so an over-the-air release cannot
 * replace it, and it never registers its own login item.
 */
export const SPEED_TEST_BUILD = import.meta.env.VITE_MURMUR_SPEED_TEST === '1';

/** Flavours that manage neither their own updates nor a login item. */
export const NO_SELF_MANAGEMENT_BUILD = INTERNAL_BENCHMARK_BUILD || SPEED_TEST_BUILD;

/** Header label that names an experiment variant; `undefined` for the production flavour. */
export const SPEED_TEST_BADGE = SPEED_TEST_BUILD ? 'Speed Test build' : undefined;
