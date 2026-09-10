import { Menu } from '@base-ui/react/menu';
import { Check, ChevronDown, ListFilter } from 'lucide-react';
import {
  HISTORY_DATE_FILTER_OPTIONS,
  HISTORY_FILTER_OPTIONS,
  type HistoryDateFilter,
  type HistoryFilter,
  type HistoryPinnedFilter,
} from '../../lib/history';
import {
  AnimatedDropdown,
  AnimatedDropdownContent,
  AnimatedDropdownItem,
  AnimatedDropdownSeparator,
  AnimatedDropdownTrigger,
} from '@/components/ui/animated-dropdown/animated-dropdown';

interface HistoryFilterMenuProps {
  filter: HistoryFilter;
  dateFilter: HistoryDateFilter;
  pinnedFilter: HistoryPinnedFilter;
  onFilterChange: (value: HistoryFilter) => void;
  onDateFilterChange: (value: HistoryDateFilter) => void;
  onPinnedFilterChange: (value: HistoryPinnedFilter) => void;
  onClear: () => void;
}

function isHistoryFilter(value: unknown): value is HistoryFilter {
  return HISTORY_FILTER_OPTIONS.some((option) => option.value === value);
}

function isHistoryDateFilter(value: unknown): value is HistoryDateFilter {
  return HISTORY_DATE_FILTER_OPTIONS.some((option) => option.value === value);
}

function activeFilterNames(filter: HistoryFilter, dateFilter: HistoryDateFilter, pinnedFilter: HistoryPinnedFilter): string[] {
  const parts: string[] = [];
  if (filter !== 'all') parts.push(HISTORY_FILTER_OPTIONS.find((option) => option.value === filter)?.label ?? filter);
  if (dateFilter !== 'all') parts.push(HISTORY_DATE_FILTER_OPTIONS.find((option) => option.value === dateFilter)?.label ?? dateFilter);
  if (pinnedFilter === 'pinned') parts.push('Pinned');
  return parts;
}

/**
 * Summary shown on the trigger: "Filter" when nothing is narrowed, the choice's
 * name when one is active, and a count beyond that so the toolbar stays on one line.
 */
export function historyFilterLabel(filter: HistoryFilter, dateFilter: HistoryDateFilter, pinnedFilter: HistoryPinnedFilter): string {
  const parts = activeFilterNames(filter, dateFilter, pinnedFilter);
  if (parts.length === 0) return 'Filter';
  return parts.length === 1 ? parts[0] : `${parts.length} filters`;
}

export function HistoryFilterMenu({
  filter,
  dateFilter,
  pinnedFilter,
  onFilterChange,
  onDateFilterChange,
  onPinnedFilterChange,
  onClear,
}: HistoryFilterMenuProps) {
  const active = filter !== 'all' || dateFilter !== 'all' || pinnedFilter !== 'all';
  const label = historyFilterLabel(filter, dateFilter, pinnedFilter);
  const names = activeFilterNames(filter, dateFilter, pinnedFilter).join(' · ');

  return (
    <AnimatedDropdown>
      <AnimatedDropdownTrigger
        className={active ? 'history-filter-trigger history-filter-trigger-active' : 'history-filter-trigger'}
        aria-label={active ? `Filter transcripts: ${names}` : 'Filter transcripts'}
      >
        <ListFilter aria-hidden="true" size={13} strokeWidth={2.2} />
        <span className="truncate">{label}</span>
        <ChevronDown aria-hidden="true" size={12} strokeWidth={2.2} className="opacity-60" />
      </AnimatedDropdownTrigger>
      <AnimatedDropdownContent align="end" side="bottom" className="history-filter-menu min-w-48">
        <Menu.RadioGroup value={filter} onValueChange={(value) => { if (isHistoryFilter(value)) onFilterChange(value); }}>
          <Menu.GroupLabel className="history-export-label">Show</Menu.GroupLabel>
          {HISTORY_FILTER_OPTIONS.map((option) => (
            <Menu.RadioItem key={option.value} value={option.value} className="history-filter-option">
              <span className="history-filter-radio" aria-hidden="true" />
              {option.label}
            </Menu.RadioItem>
          ))}
        </Menu.RadioGroup>
        <AnimatedDropdownSeparator />
        <Menu.RadioGroup value={dateFilter} onValueChange={(value) => { if (isHistoryDateFilter(value)) onDateFilterChange(value); }}>
          <Menu.GroupLabel className="history-export-label">From</Menu.GroupLabel>
          {HISTORY_DATE_FILTER_OPTIONS.map((option) => (
            <Menu.RadioItem key={option.value} value={option.value} className="history-filter-option">
              <span className="history-filter-radio" aria-hidden="true" />
              {option.label}
            </Menu.RadioItem>
          ))}
        </Menu.RadioGroup>
        <AnimatedDropdownSeparator />
        <Menu.CheckboxItem
          checked={pinnedFilter === 'pinned'}
          onCheckedChange={(checked) => onPinnedFilterChange(checked ? 'pinned' : 'all')}
          className="history-filter-option"
        >
          <span className="history-filter-check" aria-hidden="true"><Check size={11} strokeWidth={3} /></span>
          Pinned only
        </Menu.CheckboxItem>
        {active && (
          <>
            <AnimatedDropdownSeparator />
            <AnimatedDropdownItem onClick={onClear}>Clear filters</AnimatedDropdownItem>
          </>
        )}
      </AnimatedDropdownContent>
    </AnimatedDropdown>
  );
}
