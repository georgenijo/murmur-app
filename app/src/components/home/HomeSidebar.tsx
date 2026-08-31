import type { Ref } from 'react';
import type { MainDestination } from '../../lib/homeDashboard';
import type { SettingsEditorTab } from '../settings/SettingsEditorsWindow';
import { MainNavItem } from '../ui/DashboardPrimitives';

interface SettingsLink {
  page: string;
  editorTab?: SettingsEditorTab;
  target?: string;
}

interface HomeSidebarProps {
  active: MainDestination;
  homeButtonRef?: Ref<HTMLButtonElement>;
  onNavigate: (destination: MainDestination) => void;
  onOpenSettings: (link: SettingsLink) => void;
}

type IconName = 'home' | 'meeting' | 'query' | 'insights' | 'text' | 'commands' | 'styles' | 'transforms';

function NavIcon({ name }: { name: IconName }) {
  const paths: Record<IconName, React.ReactNode> = {
    home: <><path d="M4 10.5 12 4l8 6.5V20H5V10.5Z" /><path d="M9 20v-6h6v6" /></>,
    meeting: <><rect x="4" y="5" width="16" height="15" rx="2" /><path d="M8 3v4M16 3v4M4 10h16M8 14h3M8 17h6" /></>,
    query: <><path d="M5 5h14v11H9l-4 4V5Z" /><path d="M9 9h6M9 12h4" /></>,
    insights: <><path d="M5 19V9M10 19V5M15 19v-7M20 19V3" /></>,
    text: <><path d="M5 5h14M8 5v14M5 19h6M15 10h4M15 14h4" /></>,
    commands: <><path d="M6 8h12M6 12h8M6 16h10" /><path d="m17 14 3 2-3 2" /></>,
    styles: <><path d="M4 18 15 7l3 3L7 21H4v-3Z" /><path d="m13 9 3 3M17 4l3 3" /></>,
    transforms: <><path d="m5 16 7-7 7 7M12 9v11" /><path d="M5 5h14" /></>,
  };
  return (
    <svg className="home-nav-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.7} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      {paths[name]}
    </svg>
  );
}

export function HomeSidebar({ active, homeButtonRef, onNavigate, onOpenSettings }: HomeSidebarProps) {
  return (
    <aside className="home-sidebar" aria-label="Murmur navigation">
      <div className="home-brand" aria-label="Murmur">
        <span className="home-brand-mark" aria-hidden="true">
          <span /><span /><span /><span />
        </span>
        <span className="home-brand-name">Murmur</span>
      </div>

      <nav className="home-nav" aria-label="Main destinations">
        <MainNavItem ref={homeButtonRef} label="Home" icon={<NavIcon name="home" />} selected={active === 'home'} onActivate={() => onNavigate('home')} />
        <MainNavItem label="Notetaker" icon={<NavIcon name="meeting" />} selected={active === 'meetings'} onActivate={() => onNavigate('meetings')} />
        <MainNavItem label="Queries" icon={<NavIcon name="query" />} selected={active === 'queries'} onActivate={() => onNavigate('queries')} />
        <MainNavItem label="Insights" icon={<NavIcon name="insights" />} selected={active === 'insights'} onActivate={() => onNavigate('insights')} />

        <p className="home-nav-section">Customize</p>
        <MainNavItem label="Text & Vocabulary" icon={<NavIcon name="text" />} selected={false} onActivate={() => onOpenSettings({ page: 'text' })} />
        <MainNavItem label="Voice Commands" icon={<NavIcon name="commands" />} selected={false} onActivate={() => onOpenSettings({ page: 'text', editorTab: 'commands' })} />
        <MainNavItem label="Styles" icon={<NavIcon name="styles" />} selected={false} onActivate={() => onOpenSettings({ page: 'delivery', target: 'app-overrides' })} />
        <MainNavItem label="Transforms" icon={<NavIcon name="transforms" />} selected={false} onActivate={() => onOpenSettings({ page: 'ai-transform' })} />
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
