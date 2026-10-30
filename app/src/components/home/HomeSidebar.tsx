import { useId, useLayoutEffect, useRef, type Ref } from 'react';
import { PanelLeftClose, PanelLeftOpen } from 'lucide-react';
import { FluidTooltip } from '../ui/fluid-tooltip/fluid-tooltip';
import { HOME_SIDEBAR_MIN, HOME_SIDEBAR_MAX, useHomeSidebar } from '../../lib/hooks/useHomeSidebar';
import type { MainDestination } from '../../lib/homeDashboard';
import { MainNavItem } from '../ui/DashboardPrimitives';

interface HomeSidebarProps {
  active: MainDestination;
  initialCollapsed?: boolean;
  suspended?: boolean;
  onWidthChange?: (width: number) => void;
  homeButtonRef?: Ref<HTMLButtonElement>;
  onNavigate: (destination: MainDestination) => void;
}

type IconName = 'home' | 'meeting' | 'query' | 'insights';

function NavIcon({ name }: { name: IconName }) {
  const paths: Record<IconName, React.ReactNode> = {
    home: <><path d="M4 10.5 12 4l8 6.5V20H5V10.5Z" /><path d="M9 20v-6h6v6" /></>,
    meeting: <><rect x="4" y="5" width="16" height="15" rx="2" /><path d="M8 3v4M16 3v4M4 10h16M8 14h3M8 17h6" /></>,
    query: <><path d="M5 5h14v11H9l-4 4V5Z" /><path d="M9 9h6M9 12h4" /></>,
    insights: <><path d="M5 19V9M10 19V5M15 19v-7M20 19V3" /></>,
  };
  return (
    <svg className="home-nav-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.7} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      {paths[name]}
    </svg>
  );
}

export function HomeSidebar({ active, homeButtonRef, onNavigate, initialCollapsed, suspended, onWidthChange }: HomeSidebarProps) {
  const sidebar = useHomeSidebar(initialCollapsed, suspended);
  const navId = useId();
  const toggleRef = useRef<HTMLButtonElement>(null);
  const resizeFocused = useRef(false);
  useLayoutEffect(() => {
    if (sidebar.collapsed && resizeFocused.current) {
      resizeFocused.current = false;
      toggleRef.current?.focus();
    }
  }, [sidebar.collapsed]);
  useLayoutEffect(() => { onWidthChange?.(sidebar.width); }, [sidebar.width, onWidthChange]);
  const destinations = [
    { id: 'home', label: 'Home', icon: 'home' },
    { id: 'meetings', label: 'Notetaker', icon: 'meeting' },
    { id: 'queries', label: 'Assistant', icon: 'query' },
    { id: 'insights', label: 'Insights', icon: 'insights' },
  ] as const;
  return (
    <aside className="home-sidebar" aria-label="Murmur navigation" data-collapsed={sidebar.collapsed} data-resizing={sidebar.resizing} style={{ width: sidebar.width }}>
      <div className="home-sidebar-heading">
        <div className="home-brand" aria-label="Murmur">
          <span className="home-brand-mark" aria-hidden="true">
            <span /><span /><span /><span />
          </span>
          <span className="home-brand-name">Murmur</span>
        </div>
        <button ref={toggleRef} type="button" className="ui-icon-button home-sidebar-toggle"
          aria-label={sidebar.collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
          title={sidebar.collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
          aria-expanded={!sidebar.collapsed} aria-controls={navId} onClick={sidebar.toggle}>
          {sidebar.collapsed ? <PanelLeftOpen aria-hidden="true" size={16} /> : <PanelLeftClose aria-hidden="true" size={16} />}
        </button>
      </div>

      <FluidTooltip.Group orientation="vertical" disabled={suspended}>
        <nav id={navId} className="home-nav" aria-label="Main destinations">
          {destinations.map(({ id, label, icon }) => (
            <FluidTooltip.Root key={id} id={`${navId}-${id}`} disabled={!sidebar.collapsed}>
              <FluidTooltip.Trigger>
                <MainNavItem ref={id === 'home' ? homeButtonRef : undefined} label={label}
                  icon={<NavIcon name={icon} />} selected={active === id} onActivate={() => onNavigate(id)} />
              </FluidTooltip.Trigger>
              <FluidTooltip.Content>{label}</FluidTooltip.Content>
            </FluidTooltip.Root>
          ))}
        </nav>
      </FluidTooltip.Group>

      <div className="home-sidebar-bottom">
        <div className="home-privacy-note" title={sidebar.collapsed ? 'Dictation stays on this Mac.' : undefined}>
          <span className="home-privacy-dot" aria-hidden="true" />
          <span className="home-nav-label">Dictation stays on this Mac.</span>
        </div>
      </div>
      {!sidebar.collapsed && <div className="home-sidebar-resize" role="separator" tabIndex={0}
        aria-label="Sidebar width" aria-orientation="vertical" aria-controls={navId}
        aria-valuemin={HOME_SIDEBAR_MIN} aria-valuemax={HOME_SIDEBAR_MAX} aria-valuenow={sidebar.width}
        aria-valuetext={`${sidebar.width} pixels`} title="Drag to resize sidebar; use Left/Right arrows, Home or End"
        onFocus={() => { resizeFocused.current = true; }} onBlur={() => { resizeFocused.current = false; }}
        {...sidebar.resizeProps} />}
    </aside>
  );
}
