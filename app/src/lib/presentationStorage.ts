// Rust source snapshots remain usable for this window even if WebKit refuses
// cache writes. This layer never imports legacy data or publishes source state.
const fallback = new Map<string, string | null>();
let reported = false;
function cacheFailure(): void {
  if (reported) return;
  reported = true;
  console.warn('Presentation cache unavailable; using in-memory state.');
}
export const presentationStorage = {
  getItem(key: string): string | null {
    if (fallback.has(key)) return fallback.get(key) ?? null;
    return localStorage.getItem(key);
  },
  setItem(key: string, value: string): void {
    try { localStorage.setItem(key, value); fallback.delete(key); }
    catch { fallback.set(key, value); cacheFailure(); }
  },
  removeItem(key: string): void {
    try { localStorage.removeItem(key); fallback.delete(key); }
    catch { fallback.set(key, null); cacheFailure(); }
  },
};
