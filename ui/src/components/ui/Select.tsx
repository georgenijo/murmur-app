import { useState, useRef, useEffect, useId } from 'react';

export interface SelectOption<T extends string = string> {
  value: T;
  label: string;
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
  placeholder?: string;
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
  placeholder,
}: SelectProps<T>) {
  const id = useId();
  const [isOpen, setIsOpen] = useState(false);
  const [highlightedIndex, setHighlightedIndex] = useState(-1);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const listboxRef = useRef<HTMLUListElement>(null);

  const flatOptions: SelectOption<T>[] = isGrouped(items)
    ? items.flatMap((g) => g.options)
    : items;

  const selectedOption = flatOptions.find((o) => o.value === value);
  const displayLabel = selectedOption?.label ?? placeholder ?? '';

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

  // Scroll highlighted option into view
  useEffect(() => {
    if (highlightedIndex < 0 || !listboxRef.current) return;
    const el = listboxRef.current.querySelector(
      `[data-index="${highlightedIndex}"]`,
    );
    el?.scrollIntoView({ block: 'nearest' });
  }, [highlightedIndex]);

  const open = () => {
    if (disabled) return;
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
        className={`px-3 py-2 text-sm cursor-pointer flex items-center justify-between ${
          isHighlighted ? 'bg-stone-100 dark:bg-stone-600' : ''
        } ${isSelected ? 'font-medium' : ''} text-stone-900 dark:text-stone-100`}
      >
        <span className="truncate">{option.label}</span>
        {isSelected && (
          <svg
            className="w-4 h-4 text-stone-500 dark:text-stone-400 shrink-0 ml-2"
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

  // Build a flat index offset for grouped items
  let flatIndex = 0;

  return (
    <div className="relative w-full" onKeyDown={handleKeyDown}>
      <button
        ref={triggerRef}
        type="button"
        role="combobox"
        aria-expanded={isOpen}
        aria-haspopup="listbox"
        aria-controls={`${id}-listbox`}
        aria-activedescendant={
          isOpen && highlightedIndex >= 0
            ? `${id}-option-${highlightedIndex}`
            : undefined
        }
        disabled={disabled}
        onClick={() => (isOpen ? close() : open())}
        className={`w-full px-3 py-2 rounded-lg border border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-700 text-stone-900 dark:text-stone-100 text-sm text-left focus:ring-2 focus:ring-stone-500 focus:border-transparent transition-colors flex items-center justify-between ${
          disabled ? 'opacity-50 cursor-not-allowed' : ''
        }`}
      >
        <span className="truncate">{displayLabel}</span>
        <svg
          className={`w-4 h-4 text-stone-400 shrink-0 ml-2 transition-transform ${
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
          className="absolute z-10 mt-1 w-full max-h-60 overflow-auto rounded-lg border border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-700 shadow-lg py-1"
        >
          {isGrouped(items)
            ? items.map((group, gi) => {
                const startIndex = flatIndex;
                flatIndex += group.options.length;
                return (
                  <li
                    key={gi}
                    role="group"
                    aria-labelledby={`${id}-group-${gi}`}
                  >
                    <span
                      id={`${id}-group-${gi}`}
                      className="block px-3 py-1.5 text-xs font-semibold text-stone-500 dark:text-stone-400 uppercase tracking-wider select-none"
                    >
                      {group.label}
                    </span>
                    <ul role="none">
                      {group.options.map((option, oi) =>
                        renderOption(option, startIndex + oi),
                      )}
                    </ul>
                  </li>
                );
              })
            : flatOptions.map((option, idx) => renderOption(option, idx))}
        </ul>
      )}
    </div>
  );
}
