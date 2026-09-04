// jsdom under vitest 4 does not expose a spec-complete `localStorage`
// (notably `clear()` is missing), so tests that persist via localStorage fail
// with "localStorage.clear is not a function". Provide a small in-memory Storage
// so localStorage-backed code (settings, updater check-interval) is deterministic.
class MemoryStorage implements Storage {
  private store = new Map<string, string>();
  get length(): number {
    return this.store.size;
  }
  clear(): void {
    this.store.clear();
  }
  getItem(key: string): string | null {
    return this.store.get(key) ?? null;
  }
  key(index: number): string | null {
    return Array.from(this.store.keys())[index] ?? null;
  }
  removeItem(key: string): void {
    this.store.delete(key);
  }
  setItem(key: string, value: string): void {
    this.store.set(key, String(value));
  }
}

const storage = new MemoryStorage();
Object.defineProperty(globalThis, 'localStorage', { value: storage, configurable: true });
if (typeof window !== 'undefined') {
  Object.defineProperty(window, 'localStorage', { value: storage, configurable: true });
}

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

// jsdom does not implement layout scrolling. Keep browser-only scroll calls
// deterministic so animation-frame callbacks cannot fail after a test ends.
Object.defineProperty(HTMLElement.prototype, 'scrollTo', {
  value: () => {},
  configurable: true,
});

// jsdom does not implement the Pointer Capture API. Several components rely
// on it unguarded (Sona's hold-to-delete control calls
// setPointerCapture/releasePointerCapture on pointerdown/pointerup/
// pointercancel so a small drift off the row doesn't fire pointerleave and
// cancel the hold, and animated-switch calls setPointerCapture on
// pointerdown), so stub all three methods once here as a no-op rather than
// patching them per test file.
if (!('setPointerCapture' in HTMLElement.prototype)) {
  Object.assign(HTMLElement.prototype, {
    setPointerCapture: () => {},
    releasePointerCapture: () => {},
    hasPointerCapture: () => false,
  });
}
