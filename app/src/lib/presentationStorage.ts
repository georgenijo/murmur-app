// Hydrated snapshots, writes and removal tombstones stay authoritative for this
// renderer session, including after successful DOM operations. Legacy import
// reads bypass this adapter and remain fail-closed.
const values = new Map<string, string | null>();
let reported = false;
function cacheFailure(): void {
  if (reported) return;
  reported = true;
  console.warn('Presentation cache unavailable; using in-memory state.');
}
export const presentationStorage = {
  /** A committed native notification updates only this renderer's mirror;
   * writing it through could race another renderer's newer optimistic edit. */
  rememberItem(key: string, value: string): void {
    values.set(key, value);
  },
  getItem(key: string): string | null {
    if (values.has(key)) return values.get(key) ?? null;
    try { return localStorage.getItem(key); }
    catch {
      cacheFailure();
      // Unknown is distinct from an authoritative absence (for example, the
      // onboarding gate). Never turn a failed read into a writable default.
      throw new Error('Presentation cache unavailable');
    }
  },
  setItem(key: string, value: string): void {
    values.set(key, value);
    try { localStorage.setItem(key, value); }
    catch { cacheFailure(); }
  },
  removeItem(key: string): void {
    values.set(key, null);
    try { localStorage.removeItem(key); }
    catch { cacheFailure(); }
  },
};

// Successful cache writes in another renderer also update this session's copy.
// Typed native settings notifications separately refresh from the source file,
// so propagation continues when DOM storage itself is unavailable.
window.addEventListener('storage', event => {
  if (event.storageArea !== null && event.storageArea !== localStorage) return;
  if (event.key === null) {
    values.clear();
  } else {
    values.set(event.key, event.newValue);
  }
});
