import type { MainDestination } from '../../lib/homeDashboard';
import type { SettingsEditorTab } from '../settings/SettingsEditorsWindow';

interface SettingsLink {
  page: string;
  editorTab?: SettingsEditorTab;
  target?: string;
}

interface HomeSidebarProps {
  active: MainDestination;
  onNavigate: (destination: MainDestination) => void;
  onOpenSettings: (link: SettingsLink) => void;
}

type IconName = 'home' | 'meeting' | 'query' | 'insights' | 'text' | 'commands' | 'styles' | 'transforms' | 'settings';

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
    settings: <><circle cx="12" cy="12" r="3" /><path d="M19 12a7 7 0 1 1-14 0 7 7 0 0 1 14 0Z" /></>,
  };
  return (
    <svg className="home-nav-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.7} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      {paths[name]}
    </svg>
  );
}

function NavigationButton({
  label,
  icon,
  selected = false,
  badge,
  onClick,
}: {
  label: string;
  icon: IconName;
  selected?: boolean;
  badge?: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={selected ? 'page' : undefined}
      aria-label={label}
      className="home-nav-item"
    >
      <NavIcon name={icon} />
      <span className="home-nav-label">{label}</span>
      {badge && <span className="home-nav-badge">{badge}</span>}
    </button>
  );
}

export function HomeSidebar({ active, onNavigate, onOpenSettings }: HomeSidebarProps) {
  return (
    <aside className="home-sidebar" aria-label="Murmur navigation">
      <div className="home-brand" aria-label="Murmur">
        <span className="home-brand-mark" aria-hidden="true">
          <span /><span /><span /><span />
        </span>
        <span className="home-brand-name">Murmur</span>
      </div>

      <nav className="home-nav" aria-label="Main destinations">
        <NavigationButton label="Home" icon="home" selected={active === 'home'} onClick={() => onNavigate('home')} />
        <NavigationButton label="Notetaker" icon="meeting" selected={active === 'meetings'} onClick={() => onNavigate('meetings')} />
        <NavigationButton label="Queries" icon="query" selected={active === 'queries'} onClick={() => onNavigate('queries')} />
        <NavigationButton label="Insights" icon="insights" selected={active === 'insights'} onClick={() => onNavigate('insights')} />

        <p className="home-nav-section">Customize</p>
        <NavigationButton label="Text & Vocabulary" icon="text" onClick={() => onOpenSettings({ page: 'text' })} />
        <NavigationButton label="Voice Commands" icon="commands" onClick={() => onOpenSettings({ page: 'text', editorTab: 'commands' })} />
        <NavigationButton label="Styles" icon="styles" onClick={() => onOpenSettings({ page: 'delivery', target: 'app-overrides' })} />
        <NavigationButton label="Transforms" icon="transforms" onClick={() => onOpenSettings({ page: 'ai-transform' })} />
      </nav>

      <div className="home-sidebar-bottom">
        <div className="home-privacy-note">
          <span className="home-privacy-dot" aria-hidden="true" />
          <span className="home-nav-label">Everything stays on this Mac.</span>
        </div>
        <NavigationButton label="Settings" icon="settings" onClick={() => onOpenSettings({ page: 'general' })} />
      </div>
    </aside>
  );
}
