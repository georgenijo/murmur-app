import { Menu } from '@base-ui/react/menu';
import { Check, ChevronDown, ListFilter } from 'lucide-react';
import {
  HISTORY_DATE_FILTER_OPTIONS,
  HISTORY_FILTER_OPTIONS,
  type HistoryDateFilter,
  type HistoryFilter,
  type HistoryPinnedFilter,
  type HistoryProfileFilter,
  profileFilterLabel,
  profileFilterValue,
} from '../../lib/history';
import {
  AnimatedDropdown,
  AnimatedDropdownContent,
  AnimatedDropdownItem,
  AnimatedDropdownSeparator,
  AnimatedDropdownTrigger,
} from '@/components/ui/animated-dropdown/animated-dropdown';

interface HistoryFilterMenuProps {
  disabled?: boolean;
  filter: HistoryFilter;
  dateFilter: HistoryDateFilter;
  pinnedFilter: HistoryPinnedFilter;
  onFilterChange: (value: HistoryFilter) => void;
  onDateFilterChange: (value: HistoryDateFilter) => void;
  onPinnedFilterChange: (value: HistoryPinnedFilter) => void;
  /** Profile labels to offer. The App section is hidden when this is empty. */
  profiles?: readonly string[];
  profileFilter?: HistoryProfileFilter;
  onProfileFilterChange?: (value: HistoryProfileFilter) => void;
  onClear: () => void;
}

function isHistoryFilter(value: unknown): value is HistoryFilter {
  return HISTORY_FILTER_OPTIONS.some((option) => option.value === value);
}

function isHistoryDateFilter(value: unknown): value is HistoryDateFilter {
  return HISTORY_DATE_FILTER_OPTIONS.some((option) => option.value === value);
}

const NO_PROFILE_LABEL = 'No app profile';

function activeFilterNames(
  filter: HistoryFilter,
  dateFilter: HistoryDateFilter,
  pinnedFilter: HistoryPinnedFilter,
  profileFilter: HistoryProfileFilter,
): string[] {
  const parts: string[] = [];
  if (filter !== 'all') parts.push(HISTORY_FILTER_OPTIONS.find((option) => option.value === filter)?.label ?? filter);
  if (dateFilter !== 'all') parts.push(HISTORY_DATE_FILTER_OPTIONS.find((option) => option.value === dateFilter)?.label ?? dateFilter);
  if (pinnedFilter === 'pinned') parts.push('Pinned');
  if (profileFilter === 'none') parts.push(NO_PROFILE_LABEL);
  else if (profileFilter !== 'any') parts.push(profileFilterLabel(profileFilter) ?? profileFilter);
  return parts;
}

/**
 * Summary shown on the trigger: "Filter" when nothing is narrowed, the choice's
 * name when one is active, and a count beyond that so the toolbar stays on one line.
 */
export function historyFilterLabel(
  filter: HistoryFilter,
  dateFilter: HistoryDateFilter,
  pinnedFilter: HistoryPinnedFilter,
  profileFilter: HistoryProfileFilter = 'any',
): string {
  const parts = activeFilterNames(filter, dateFilter, pinnedFilter, profileFilter);
  if (parts.length === 0) return 'Filter';
  return parts.length === 1 ? parts[0] : `${parts.length} filters`;
}

export function HistoryFilterMenu({
  disabled = false,
  filter,
  dateFilter,
  pinnedFilter,
  onFilterChange,
  onDateFilterChange,
  onPinnedFilterChange,
  profiles = [],
  profileFilter = 'any',
  onProfileFilterChange = () => {},
  onClear,
}: HistoryFilterMenuProps) {
  const active = filter !== 'all' || dateFilter !== 'all' || pinnedFilter !== 'all' || profileFilter !== 'any';
  const label = historyFilterLabel(filter, dateFilter, pinnedFilter, profileFilter);
  const names = activeFilterNames(filter, dateFilter, pinnedFilter, profileFilter).join(' · ');
  const profileValues = new Set<string>(['any', 'none', ...profiles.map(profileFilterValue)]);

  return (
    <AnimatedDropdown disabled={disabled}>
      <AnimatedDropdownTrigger
        className={active ? 'history-filter-trigger history-filter-trigger-active' : 'history-filter-trigger'}
        aria-label={active ? `Filter transcripts: ${label}${label === names ? '' : `, ${names}`}` : 'Filter transcripts'}
      >
        <ListFilter aria-hidden="true" size={14} />
        <span className="truncate">{label}</span>
        <ChevronDown aria-hidden="true" size={14} className="opacity-60" />
      </AnimatedDropdownTrigger>
      <AnimatedDropdownContent align="end" side="bottom" className="history-filter-menu min-w-48 max-w-72">
        <Menu.RadioGroup value={filter} onValueChange={(value) => { if (isHistoryFilter(value)) onFilterChange(value); }}>
          <Menu.GroupLabel className="ui-menu-label">Show</Menu.GroupLabel>
          {HISTORY_FILTER_OPTIONS.map((option) => (
            <Menu.RadioItem key={option.value} value={option.value} className="ui-menu-item history-filter-option">
              <span className="history-filter-radio" aria-hidden="true" />
              {option.label}
            </Menu.RadioItem>
          ))}
        </Menu.RadioGroup>
        <AnimatedDropdownSeparator />
        <Menu.RadioGroup value={dateFilter} onValueChange={(value) => { if (isHistoryDateFilter(value)) onDateFilterChange(value); }}>
          <Menu.GroupLabel className="ui-menu-label">From</Menu.GroupLabel>
          {HISTORY_DATE_FILTER_OPTIONS.map((option) => (
            <Menu.RadioItem key={option.value} value={option.value} className="ui-menu-item history-filter-option">
              <span className="history-filter-radio" aria-hidden="true" />
              {option.label}
            </Menu.RadioItem>
          ))}
        </Menu.RadioGroup>
        <AnimatedDropdownSeparator />
        <Menu.CheckboxItem
          checked={pinnedFilter === 'pinned'}
          onCheckedChange={(checked) => onPinnedFilterChange(checked ? 'pinned' : 'all')}
          className="ui-menu-item history-filter-option"
        >
          <span className="history-filter-check" aria-hidden="true"><Check size={11} strokeWidth={3} /></span>
          Pinned only
        </Menu.CheckboxItem>
        {profiles.length > 0 && (
          <>
            <AnimatedDropdownSeparator />
            <Menu.RadioGroup
              value={profileFilter}
              onValueChange={(value) => {
                if (typeof value === 'string' && profileValues.has(value)) onProfileFilterChange(value as HistoryProfileFilter);
              }}
            >
              <Menu.GroupLabel className="ui-menu-label">App</Menu.GroupLabel>
              <Menu.RadioItem value="any" className="ui-menu-item history-filter-option">
                <span className="history-filter-radio" aria-hidden="true" />
                Any app
              </Menu.RadioItem>
              {profiles.map((profile) => (
                <Menu.RadioItem key={profile} value={profileFilterValue(profile)} className="ui-menu-item history-filter-option">
                  <span className="history-filter-radio" aria-hidden="true" />
                  <span className="min-w-0 truncate">{profile}</span>
                </Menu.RadioItem>
              ))}
              <Menu.RadioItem value="none" className="ui-menu-item history-filter-option">
                <span className="history-filter-radio" aria-hidden="true" />
                {NO_PROFILE_LABEL}
              </Menu.RadioItem>
            </Menu.RadioGroup>
          </>
        )}
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
