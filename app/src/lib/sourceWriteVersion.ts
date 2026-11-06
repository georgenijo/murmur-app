// A fresh renderer module session owns its own counter. Never persist either
// value: reloads and independent webviews must not reuse a generation namespace.
export const sourceSession = crypto.randomUUID();
let generation = 0;
export function nextSourceWriteVersion(): { sourceSession: string; intentGeneration: number } {
  return { sourceSession, intentGeneration: ++generation };
}
