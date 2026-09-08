import { useEffect, useState, type Ref } from 'react';
import type { MainDestination } from '../../lib/homeDashboard';
import { MainNavItem } from '../ui/DashboardPrimitives';
import { FluidTooltip } from '../ui/fluid-tooltip/fluid-tooltip';

interface HomeSidebarProps {
  active: MainDestination;
  homeButtonRef?: Ref<HTMLButtonElement>;
  onNavigate: (destination: MainDestination) => void;
}

const COMPACT_QUERY = '(max-width: 760px)';

/** Mirrors the CSS breakpoint that hides `.home-nav-label` — the sidebar's
 *  tooltips are redundant once the text label is visible, so gate them to
 *  the same width. SSR/jsdom-safe: falls back to non-compact when
 *  matchMedia is unavailable, and tests mock it to open tooltips on hover. */
function useIsCompactSidebar(): boolean {
  const [isCompact, setIsCompact] = useState(() => (
    typeof window !== 'undefined' && typeof window.matchMedia === 'function'
      ? window.matchMedia(COMPACT_QUERY).matches
      : false
  ));

  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
    const mql = window.matchMedia(COMPACT_QUERY);
    const handleChange = () => setIsCompact(mql.matches);
    handleChange();
    mql.addEventListener('change', handleChange);
    return () => mql.removeEventListener('change', handleChange);
  }, []);

  return isCompact;
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

export function HomeSidebar({ active, homeButtonRef, onNavigate }: HomeSidebarProps) {
  const isCompact = useIsCompactSidebar();
  return (
    <aside className="home-sidebar" aria-label="Murmur navigation">
      <div className="home-brand" aria-label="Murmur">
        <span className="home-brand-mark" aria-hidden="true">
          <span /><span /><span /><span />
        </span>
        <span className="home-brand-name">Murmur</span>
      </div>

      <nav className="home-nav" aria-label="Main destinations">
        {/* Tooltips are redundant once the sidebar's text labels are visible
            (≥760px); disabling the whole group there avoids a pointless hover
            popup next to text that already says the same thing. */}
        <FluidTooltip.Group orientation="vertical" disabled={!isCompact}>
          <FluidTooltip.Root id="home-nav-home" side="right">
            <FluidTooltip.Trigger>
              <MainNavItem ref={homeButtonRef} label="Home" icon={<NavIcon name="home" />} selected={active === 'home'} onActivate={() => onNavigate('home')} />
            </FluidTooltip.Trigger>
            <FluidTooltip.Content>Home</FluidTooltip.Content>
          </FluidTooltip.Root>
          <FluidTooltip.Root id="home-nav-meetings" side="right">
            <FluidTooltip.Trigger>
              <MainNavItem label="Notetaker" icon={<NavIcon name="meeting" />} selected={active === 'meetings'} onActivate={() => onNavigate('meetings')} />
            </FluidTooltip.Trigger>
            <FluidTooltip.Content>Notetaker</FluidTooltip.Content>
          </FluidTooltip.Root>
          <FluidTooltip.Root id="home-nav-queries" side="right">
            <FluidTooltip.Trigger>
              <MainNavItem label="Queries" icon={<NavIcon name="query" />} selected={active === 'queries'} onActivate={() => onNavigate('queries')} />
            </FluidTooltip.Trigger>
            <FluidTooltip.Content>Queries</FluidTooltip.Content>
          </FluidTooltip.Root>
          <FluidTooltip.Root id="home-nav-insights" side="right">
            <FluidTooltip.Trigger>
              <MainNavItem label="Insights" icon={<NavIcon name="insights" />} selected={active === 'insights'} onActivate={() => onNavigate('insights')} />
            </FluidTooltip.Trigger>
            <FluidTooltip.Content>Insights</FluidTooltip.Content>
          </FluidTooltip.Root>
        </FluidTooltip.Group>
      </nav>

      <div className="home-sidebar-bottom">
        <div className="home-privacy-note">
          <span className="home-privacy-dot" aria-hidden="true" />
          <span className="home-nav-label">Everything stays on this Mac.</span>
        </div>
      </div>
    </aside>
  );
}
