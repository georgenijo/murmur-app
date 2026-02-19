import { useState, useEffect, useCallback } from 'react';

interface KeyCaptureInputProps {
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
}

// Format a Tauri shortcut string for display (e.g. "Shift+Alt+D" → "⇧ ⌥ D")
function formatShortcut(shortcut: string): string {
  if (!shortcut) return '';
  const DISPLAY: Record<string, string> = {
    Shift: '⇧',
    Alt: '⌥',
    Ctrl: '⌃',
    Super: '⌘',
  };
  return shortcut
    .split('+')
    .map((part) => DISPLAY[part] ?? part)
    .join(' ');
}

// Map a KeyboardEvent to a Tauri shortcut string like "Shift+Alt+D" or "Ctrl+Space"
function eventToShortcut(e: KeyboardEvent): string | null {
  const MODIFIER_KEYS = new Set(['Shift', 'Alt', 'Control', 'Meta']);
  if (MODIFIER_KEYS.has(e.key)) return null; // ignore lone modifier keydowns

  const parts: string[] = [];
  if (e.ctrlKey) parts.push('Ctrl');
  if (e.shiftKey) parts.push('Shift');
  if (e.altKey) parts.push('Alt');
  if (e.metaKey) parts.push('Super');

  let key = e.key;
  if (key === ' ') key = 'Space';
  else if (key.length === 1) key = key.toUpperCase();
  // Multi-char keys (ArrowUp, F1, etc.) pass through as-is

  parts.push(key);
  return parts.join('+');
}

export function KeyCaptureInput({ value, onChange, disabled = false }: KeyCaptureInputProps) {
  const [capturing, setCapturing] = useState(false);

  const startCapturing = useCallback(() => {
    if (!disabled) setCapturing(true);
  }, [disabled]);

  const stopCapturing = useCallback(() => {
    setCapturing(false);
  }, []);

  useEffect(() => {
    if (!capturing) return;

    const handleKeyDown = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();

      if (e.key === 'Escape') {
        stopCapturing();
        return;
      }

      const shortcut = eventToShortcut(e);
      if (shortcut) {
        onChange(shortcut);
        stopCapturing();
      }
    };

    window.addEventListener('keydown', handleKeyDown, true);
    return () => window.removeEventListener('keydown', handleKeyDown, true);
  }, [capturing, onChange, stopCapturing]);

  // Cancel capture on outside click
  useEffect(() => {
    if (!capturing) return;
    const handleMouseDown = (e: MouseEvent) => {
      const target = e.target as HTMLElement;
      if (!target.closest('[data-key-capture]')) stopCapturing();
    };
    window.addEventListener('mousedown', handleMouseDown);
    return () => window.removeEventListener('mousedown', handleMouseDown);
  }, [capturing, stopCapturing]);

  const displayText = capturing
    ? 'Press any key combo…'
    : value
      ? formatShortcut(value)
      : '';

  return (
    <div
      data-key-capture
      className={`flex items-center gap-2 w-full px-3 py-2 rounded-lg border text-sm transition-colors ${
        disabled
          ? 'opacity-50 cursor-not-allowed border-stone-300 dark:border-stone-600 bg-stone-100 dark:bg-stone-800'
          : capturing
            ? 'border-stone-500 dark:border-stone-400 bg-white dark:bg-stone-700 ring-2 ring-stone-400 dark:ring-stone-500 cursor-text'
            : 'border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-700 hover:border-stone-400 dark:hover:border-stone-500 cursor-pointer'
      }`}
      onClick={startCapturing}
    >
      <span
        className={`flex-1 font-mono ${
          capturing
            ? 'text-stone-400 dark:text-stone-500 italic'
            : value
              ? 'text-stone-900 dark:text-stone-100'
              : 'text-stone-400 dark:text-stone-500 italic'
        }`}
      >
        {displayText || 'Click to set hotkey'}
      </span>

      {value && !capturing && !disabled && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onChange('');
          }}
          className="text-stone-400 hover:text-stone-600 dark:hover:text-stone-300 transition-colors flex-shrink-0"
          title="Clear hotkey"
        >
          <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      )}
    </div>
  );
}
