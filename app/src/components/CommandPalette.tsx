import { useEffect, useMemo, useRef, useState } from 'react';
import { filterCommands, moveSelection, type PaletteCommand } from '../lib/commandPalette';

interface CommandPaletteProps {
  isOpen: boolean;
  onClose: () => void;
  commands: PaletteCommand[];
}

/**
 * Keyboard-first launcher for everything the main window can do.
 *
 * The palette never performs an action itself — each row owns a `run`
 * callback — so it stays a pure navigation surface and the caller keeps one
 * source of truth for what each command means.
 */
export function CommandPalette({ isOpen, onClose, commands }: CommandPaletteProps) {
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const paletteRef = useRef<HTMLDivElement>(null);
  const previouslyFocusedRef = useRef<HTMLElement | null>(null);
  const restoreFocusRef = useRef(false);

  const results = useMemo(() => filterCommands(commands, query), [commands, query]);

  useEffect(() => {
    if (!isOpen) return;
    previouslyFocusedRef.current =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    restoreFocusRef.current = true;
    const paletteNode = paletteRef.current;
    setQuery('');
    setSelected(0);
    // Focus after paint so the palette wins over whatever had focus before.
    const id = requestAnimationFrame(() => inputRef.current?.focus());
    return () => {
      cancelAnimationFrame(id);
      const activeElement = document.activeElement;
      const focusStayedInPalette =
        activeElement === document.body ||
        (activeElement instanceof Node && paletteNode?.contains(activeElement));
      if (restoreFocusRef.current || focusStayedInPalette) {
        previouslyFocusedRef.current?.focus();
      }
      previouslyFocusedRef.current = null;
      restoreFocusRef.current = false;
    };
  }, [isOpen]);

  useEffect(() => setSelected(0), [query]);

  // Keep the highlighted row in view during keyboard navigation.
  useEffect(() => {
    if (!isOpen) return;
    const row = listRef.current?.querySelector('[aria-selected="true"]');
    // `scrollIntoView` is absent in jsdom and in some embedded webviews.
    if (row && typeof row.scrollIntoView === 'function') row.scrollIntoView({ block: 'nearest' });
  }, [selected, isOpen]);

  if (!isOpen) return null;

  const runSelected = () => {
    const command = results[selected];
    if (!command) return;
    // The command may intentionally move focus to its destination.
    restoreFocusRef.current = false;
    onClose();
    void command.run();
  };

  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === 'Tab') {
      // The input is the dialog's only focusable element, so containing the
      // modal is just refusing to hand focus back to the page behind it.
      event.preventDefault();
    } else if (event.key === 'ArrowDown' || (event.key === 'n' && event.ctrlKey)) {
      event.preventDefault();
      setSelected((current) => moveSelection(current, 1, results.length));
    } else if (event.key === 'ArrowUp' || (event.key === 'p' && event.ctrlKey)) {
      event.preventDefault();
      setSelected((current) => moveSelection(current, -1, results.length));
    } else if (event.key === 'Enter') {
      event.preventDefault();
      runSelected();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      onClose();
    }
  };

  return (
    <div
      ref={paletteRef}
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/50 pt-[12vh] backdrop-blur-[2px]"
      onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        className="w-[min(32rem,90vw)] overflow-hidden rounded-2xl bg-surface-container-lowest shadow-2xl ring-1 ring-outline-variant/30"
        onKeyDown={onKeyDown}
      >
        <div className="flex items-center gap-2 border-b border-outline-variant/20 px-3.5 py-2.5">
          <svg className="h-4 w-4 shrink-0 text-on-surface-variant" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-4.35-4.35M17 11a6 6 0 11-12 0 6 6 0 0112 0z" />
          </svg>
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Type a command…"
            aria-label="Command"
            role="combobox"
            aria-expanded="true"
            aria-autocomplete="list"
            aria-controls="command-palette-results"
            aria-activedescendant={results[selected] ? `command-${results[selected].id}` : undefined}
            className="w-full bg-transparent text-sm text-on-surface placeholder:text-on-surface-variant focus:outline-none"
          />
          <kbd className="shrink-0 rounded border border-outline-variant/40 px-1.5 py-0.5 text-[10px] text-on-surface-variant">esc</kbd>
        </div>

        <ul
          ref={listRef}
          id="command-palette-results"
          role="listbox"
          aria-label="Commands"
          className="max-h-[min(24rem,60vh)] overflow-y-auto py-1"
        >
          {results.length === 0 && (
            <li className="px-3.5 py-6 text-center text-sm text-on-surface-variant">No matching command</li>
          )}
          {results.map((command, index) => (
            <li key={command.id} role="none">
              <button
                id={`command-${command.id}`}
                role="option"
                aria-selected={index === selected}
                type="button"
                tabIndex={-1}
                onMouseMove={() => setSelected(index)}
                onClick={() => {
                  restoreFocusRef.current = false;
                  onClose();
                  void command.run();
                }}
                className={`flex w-full items-center gap-3 px-3.5 py-2 text-left text-sm transition-colors ${index === selected ? 'bg-primary/10 text-on-surface' : 'text-on-surface hover:bg-surface-container'}`}
              >
                <span className="min-w-0 flex-1 truncate">{command.title}</span>
                {command.hint && (
                  <span className="shrink-0 text-[11px] text-on-surface">{command.hint}</span>
                )}
                <span className="shrink-0 rounded-full bg-surface-container px-2 py-0.5 text-[10px] font-medium text-on-surface-variant">
                  {command.section}
                </span>
              </button>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
