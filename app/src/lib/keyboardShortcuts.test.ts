import { describe, it, expect } from 'vitest';
import { isEditableTarget, mainWindowShortcut, type ShortcutEvent } from './keyboardShortcuts';

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

  it('never shadows emacs-style Control bindings while the user is typing', () => {
    for (const key of ['k', 'f', 'l', ',']) {
      expect(mainWindowShortcut(event({ key, ctrlKey: true }), true)).toBeNull();
    }
  });

  it('still accepts the Command form inside an editable field', () => {
    expect(mainWindowShortcut(event({ key: 'k', metaKey: true }), true)).toBe('palette');
    expect(mainWindowShortcut(event({ key: 'f', metaKey: true }), true)).toBe('search');
  });
});

describe('isEditableTarget', () => {
  function target(html: string, selector: string): Element {
    const host = document.createElement('div');
    host.innerHTML = html;
    document.body.appendChild(host);
    return host.querySelector(selector)!;
  }

  it('detects inputs, textareas, selects, and contenteditable', () => {
    expect(isEditableTarget(target('<input />', 'input'))).toBe(true);
    expect(isEditableTarget(target('<textarea></textarea>', 'textarea'))).toBe(true);
    expect(isEditableTarget(target('<select></select>', 'select'))).toBe(true);
    expect(isEditableTarget(target('<div contenteditable="true"></div>', 'div'))).toBe(true);
    expect(isEditableTarget(target('<div contenteditable=""></div>', 'div'))).toBe(true);
  });

  it('detects a descendant of a contenteditable region', () => {
    expect(isEditableTarget(target('<div contenteditable="true"><span>x</span></div>', 'span'))).toBe(true);
  });

  it('is false for ordinary elements, contenteditable="false", and no target', () => {
    expect(isEditableTarget(target('<button></button>', 'button'))).toBe(false);
    expect(isEditableTarget(target('<div contenteditable="false"><span>x</span></div>', 'span'))).toBe(false);
    expect(isEditableTarget(null)).toBe(false);
    expect(isEditableTarget(document)).toBe(false);
  });
});
