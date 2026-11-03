import { createContext, useContext, useState, useRef, useEffect, useLayoutEffect, useId } from 'react';

/** Settings rows supply their visible title; standalone pickers supply an explicit name. */
export const SelectLabelContext = createContext<string | undefined>(undefined);

export interface SelectOption<T extends string = string> {
  value: T;
  label: string;
  badge?: string;
  badgeTone?: 'accent' | 'warning';
}

export interface SelectGroup<T extends string = string> {
  label: string;
  options: SelectOption<T>[];
}

export type SelectItems<T extends string = string> =
  | SelectOption<T>[]
  | SelectGroup<T>[];

export interface SelectProps<T extends string = string> {
  value: T;
  onChange: (value: T) => void;
  items: SelectItems<T>;
  disabled?: boolean;
  size?: 'default' | 'sm' | 'xs';
  placeholder?: string;
  'aria-label'?: string;
  'aria-labelledby'?: string;
  'aria-describedby'?: string;
}

function isGrouped<T extends string>(
  items: SelectItems<T>,
): items is SelectGroup<T>[] {
  return items.length > 0 && 'options' in items[0];
}

export function Select<T extends string = string>({
  value,
  onChange,
  items,
  disabled,
  size = 'default',
  placeholder,
  'aria-label': ariaLabel,
  'aria-labelledby': ariaLabelledBy,
  'aria-describedby': ariaDescribedBy,
}: SelectProps<T>) {
  const id = useId();
  const rowLabel = useContext(SelectLabelContext);
  const labelledBy = ariaLabelledBy ?? (ariaLabel ? undefined : rowLabel);
  const [placement, setPlacement] = useState({ side: 'bottom', maxHeight: 240 });
  const [isOpen, setIsOpen] = useState(false);
  const [highlightedIndex, setHighlightedIndex] = useState(-1);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const listboxRef = useRef<HTMLUListElement>(null);

  const flatOptions: SelectOption<T>[] = isGrouped(items)
    ? items.flatMap((g) => g.options)
    : items;

  // Callers commonly rebuild items on render. Only real content changes should
  // reconnect placement observers (including labels, badges, and group headings).
  const placementContentKey = JSON.stringify(items);
  const selectedOption = flatOptions.find((o) => o.value === value);
  const displayLabel = selectedOption?.label ?? placeholder ?? '';

  function badge(option: SelectOption<T>) {
    if (!option.badge) return null;
    return (
      <span
        className={`ml-2 shrink-0 rounded-full px-2 py-0.5 text-xs font-semibold ${
          option.badgeTone === 'warning'
            ? 'bg-warning/15 text-warning'
            : 'bg-[var(--ui-tint-accent-strong)] text-on-surface'
        }`}
      >
        {option.badge}
      </span>
    );
  }

  // Click-outside handler
  useEffect(() => {
    if (!isOpen) return;

    const handleMouseDown = (e: MouseEvent) => {
      if (
        triggerRef.current?.contains(e.target as Node) ||
        listboxRef.current?.contains(e.target as Node)
      ) {
        return;
      }
      setIsOpen(false);
    };

    document.addEventListener('mousedown', handleMouseDown);
    return () => document.removeEventListener('mousedown', handleMouseDown);
  }, [isOpen]);

  // Keep the menu inside both the window and any clipping Settings pane.
  useLayoutEffect(() => {
    if (!isOpen || !triggerRef.current || !listboxRef.current) return;
    const place = () => {
      const trigger = triggerRef.current;
      const listbox = listboxRef.current;
      if (!trigger || !listbox) return;
      const rect = trigger.getBoundingClientRect();
      let top = 0;
      let bottom = window.innerHeight;
      for (let parent = trigger.parentElement; parent; parent = parent.parentElement) {
        const { overflowY } = getComputedStyle(parent);
        if (!/auto|scroll|hidden|clip/.test(overflowY)) continue;
        const bounds = parent.getBoundingClientRect();
        top = Math.max(top, bounds.top);
        bottom = Math.min(bottom, bounds.bottom);
      }
      const above = Math.max(0, rect.top - top - 4);
      const below = Math.max(0, bottom - rect.bottom - 4);
      const desired = Math.min(240, listbox.scrollHeight);
      const side = below < desired && above > below ? 'top' : 'bottom';
      const maxHeight = Math.min(240, side === 'top' ? above : below);
      setPlacement(previous => previous.side === side && previous.maxHeight === maxHeight
        ? previous : { side, maxHeight });
    };
    place();
    window.addEventListener('resize', place);
    // Capturing observes nested pane scrolls without moving the pane itself.
    window.addEventListener('scroll', place, true);
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(place);
    const clippingObserver = new MutationObserver(place);
    observer?.observe(triggerRef.current);
    // A folding ancestor can stop clipping without resizing the control row.
    // Observe its settled class/style as well as geometry while the menu is open.
    for (let parent = triggerRef.current.parentElement; parent; parent = parent.parentElement) {
      observer?.observe(parent);
      clippingObserver.observe(parent, { attributes: true, attributeFilter: ['class', 'style'] });
    }
    return () => {
      window.removeEventListener('resize', place);
      window.removeEventListener('scroll', place, true);
      observer?.disconnect();
      clippingObserver.disconnect();
    };
  }, [isOpen, placementContentKey, size]);

  // Scroll only the listbox, not its surrounding Settings pane.
  useLayoutEffect(() => {
    const listbox = listboxRef.current;
    if (highlightedIndex < 0 || !listbox) return;
    const option = listbox.querySelector<HTMLElement>(`[data-index="${highlightedIndex}"]`);
    if (!option) return;
    const bounds = listbox.getBoundingClientRect();
    const rect = option.getBoundingClientRect();
    if (rect.top < bounds.top) listbox.scrollTop -= bounds.top - rect.top;
    else if (rect.bottom > bounds.bottom) listbox.scrollTop += rect.bottom - bounds.bottom;
  }, [highlightedIndex, placement]);

  useEffect(() => {
    if (disabled) { setIsOpen(false); setHighlightedIndex(-1); }
  }, [disabled]);

  const open = () => {
    if (disabled) return;
    // WebKit does not focus buttons on pointer click; keyboard navigation and
    // aria-activedescendant must still belong to the opened combobox.
    triggerRef.current?.focus({ preventScroll: true });
    setIsOpen(true);
    const idx = flatOptions.findIndex((o) => o.value === value);
    setHighlightedIndex(idx >= 0 ? idx : 0);
  };

  const close = () => {
    setIsOpen(false);
    setHighlightedIndex(-1);
    triggerRef.current?.focus();
  };

  const selectOption = (option: SelectOption<T>) => {
    onChange(option.value);
    close();
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (disabled) return;

    switch (e.key) {
      case 'Enter':
      case ' ': {
        e.preventDefault();
        if (!isOpen) {
          open();
        } else if (highlightedIndex >= 0 && highlightedIndex < flatOptions.length) {
          selectOption(flatOptions[highlightedIndex]);
        }
        break;
      }
      case 'ArrowDown': {
        e.preventDefault();
        if (!isOpen) {
          open();
        } else {
          setHighlightedIndex((prev) =>
            prev < flatOptions.length - 1 ? prev + 1 : 0,
          );
        }
        break;
      }
      case 'ArrowUp': {
        e.preventDefault();
        if (!isOpen) {
          open();
          setHighlightedIndex(flatOptions.length - 1);
        } else {
          setHighlightedIndex((prev) =>
            prev > 0 ? prev - 1 : flatOptions.length - 1,
          );
        }
        break;
      }
      case 'Home': {
        if (isOpen) {
          e.preventDefault();
          setHighlightedIndex(0);
        }
        break;
      }
      case 'End': {
        if (isOpen) {
          e.preventDefault();
          setHighlightedIndex(flatOptions.length - 1);
        }
        break;
      }
      case 'Escape': {
        if (isOpen) {
          e.preventDefault();
          close();
        }
        break;
      }
      case 'Tab': {
        if (isOpen) {
          setIsOpen(false);
          setHighlightedIndex(-1);
        }
        break;
      }
    }
  };

  function renderOption(option: SelectOption<T>, index: number) {
    const isSelected = option.value === value;
    const isHighlighted = index === highlightedIndex;
    return (
      <li
        key={option.value}
        role="option"
        id={`${id}-option-${index}`}
        data-index={index}
        aria-selected={isSelected}
        onClick={() => selectOption(option)}
        onMouseEnter={() => setHighlightedIndex(index)}
        className={`flex cursor-pointer items-center justify-between px-3 py-2 text-sm text-on-surface ${
          isHighlighted ? 'bg-surface-container' : ''
        } ${isSelected ? 'font-medium' : ''}`}
      >
        <span className="flex min-w-0 flex-1 items-center">
          <span className="truncate">{option.label}</span>
          {badge(option)}
        </span>
        {isSelected && (
          <svg
            className="ml-2 h-4 w-4 shrink-0 text-primary"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M5 13l4 4L19 7"
            />
          </svg>
        )}
      </li>
    );
  }

  // Precompute flat index offsets for grouped items
  const groupOffsets = isGrouped(items)
    ? items.reduce<number[]>((offsets, _, i) => {
        offsets.push(i === 0 ? 0 : offsets[i - 1] + items[i - 1].options.length);
        return offsets;
      }, [])
    : [];

  return (
    <div className="ui-select relative w-full" onKeyDown={handleKeyDown}>
      <button
        ref={triggerRef}
        type="button"
        role="combobox"
        data-size={size}
        aria-expanded={isOpen}
        aria-haspopup="listbox"
        aria-controls={`${id}-listbox`}
        aria-activedescendant={
          isOpen && highlightedIndex >= 0
            ? `${id}-option-${highlightedIndex}`
            : undefined
        }
        aria-label={ariaLabel}
        aria-labelledby={labelledBy}
        aria-describedby={ariaDescribedBy}
        disabled={disabled}
        onClick={() => (isOpen ? close() : open())}
        className={`ui-select-trigger flex h-8 w-full items-center justify-between rounded-(--ui-radius-control) border border-(--ui-hairline) bg-(--ui-tint-raised) shadow-(--ui-shadow-1) px-3 text-left text-sm text-on-surface transition-colors focus-visible:border-transparent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary ${
          disabled ? 'cursor-not-allowed' : ''
        }`}
      >
        <span className="flex min-w-0 flex-1 items-center">
          <span className="truncate">{displayLabel}</span>
          {selectedOption && badge(selectedOption)}
        </span>
        <svg
          className={`ml-2 h-4 w-4 shrink-0 text-on-surface-variant transition-transform ${
            isOpen ? 'rotate-180' : ''
          }`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M19 9l-7 7-7-7"
          />
        </svg>
      </button>

      {isOpen && (
        <ul
          ref={listboxRef}
          role="listbox"
          id={`${id}-listbox`}
          aria-label={ariaLabel}
          aria-labelledby={labelledBy}
          data-side={placement.side}
          style={{
            top: placement.side === 'bottom' ? 'calc(100% + 4px)' : undefined,
            bottom: placement.side === 'top' ? 'calc(100% + 4px)' : undefined,
            maxHeight: placement.maxHeight,
          }}
          className="absolute z-10 w-full overflow-auto overscroll-contain rounded-(--ui-radius-popover) border border-(--ui-hairline-strong) bg-surface-container-lowest py-1 shadow-(--ui-shadow-3)"
        >
          {isGrouped(items)
            ? items.map((group, gi) => (
                  <li
                    key={gi}
                    role="group"
                    aria-labelledby={`${id}-group-${gi}`}
                  >
                    <span
                      id={`${id}-group-${gi}`}
                      className="block select-none px-3 py-1.5 text-xs font-semibold uppercase tracking-wider text-on-surface-variant"
                    >
                      {group.label}
                    </span>
                    <ul role="none">
                      {group.options.map((option, oi) =>
                        renderOption(option, groupOffsets[gi] + oi),
                      )}
                    </ul>
                  </li>
                ))
            : flatOptions.map((option, idx) => renderOption(option, idx))}
        </ul>
      )}
    </div>
  );
}
