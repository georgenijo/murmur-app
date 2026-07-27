import { describe, it, expect } from 'vitest';
import { mainWindowShortcut, type ShortcutEvent } from './keyboardShortcuts';

function event(overrides: Partial<ShortcutEvent> & { key: string }): ShortcutEvent {
  return { metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...overrides };
}

describe('mainWindowShortcut', () => {
  it('maps the four bound keys under Command', () => {
    expect(mainWindowShortcut(event({ key: 'k', metaKey: true }))).toBe('palette');
    expect(mainWindowShortcut(event({ key: 'f', metaKey: true }))).toBe('search');
    expect(mainWindowShortcut(event({ key: ',', metaKey: true }))).toBe('settings');
    expect(mainWindowShortcut(event({ key: 'l', metaKey: true }))).toBe('logs');
  });

  it('accepts Control for keyboard-only setups', () => {
    expect(mainWindowShortcut(event({ key: 'k', ctrlKey: true }))).toBe('palette');
  });

  it('is case-insensitive', () => {
    expect(mainWindowShortcut(event({ key: 'K', metaKey: true }))).toBe('palette');
  });

  it('passes through plain keys', () => {
    expect(mainWindowShortcut(event({ key: 'k' }))).toBeNull();
    expect(mainWindowShortcut(event({ key: 'f' }))).toBeNull();
  });

  it('passes through when Option or Shift is held', () => {
    expect(mainWindowShortcut(event({ key: 'k', metaKey: true, altKey: true }))).toBeNull();
    expect(mainWindowShortcut(event({ key: 'f', metaKey: true, shiftKey: true }))).toBeNull();
  });

  it('passes through unbound letters', () => {
    for (const key of ['a', 'c', 'v', 'z', 'q', 'w', 'r', 'Enter', 'Escape']) {
      expect(mainWindowShortcut(event({ key, metaKey: true }))).toBeNull();
    }
  });
});
