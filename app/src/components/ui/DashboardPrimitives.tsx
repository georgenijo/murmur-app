import { forwardRef, type ReactNode } from 'react';

import { cn } from '@/lib/sona-utils';

export type DashboardSurfaceVariant = 'flat' | 'outlined' | 'elevated';

interface DashboardSurfaceProps {
  as?: 'div' | 'section' | 'article';
  variant: DashboardSurfaceVariant;
  padding?: 'none' | 'compact' | 'standard' | 'roomy';
  children: ReactNode;
  ariaLabel?: string;
  labelledBy?: string;
}

export function DashboardSurface({
  as: Element = 'div',
  variant,
  padding = 'none',
  children,
  ariaLabel,
  labelledBy,
}: DashboardSurfaceProps) {
  return (
    <Element
      className="ui-dashboard-surface"
      data-surface={variant}
      data-padding={padding}
      aria-label={ariaLabel}
      aria-labelledby={labelledBy}
    >
      {children}
    </Element>
  );
}

type DashboardActionCommon = {
  variant: 'primary' | 'secondary' | 'quiet';
  children: ReactNode;
  ariaLabel?: string;
  icon?: 'back' | 'forward';
  tone?: 'default' | 'danger';
  testId?: string;
};

type DashboardActionProps =
  | (DashboardActionCommon & {
      kind?: 'button';
      onActivate: () => void;
      disabled?: boolean;
    })
  | (DashboardActionCommon & {
      kind: 'link';
      href: string;
    });

function DirectionIcon({ direction }: { direction: 'back' | 'forward' }) {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <path
        d={direction === 'back' ? 'm9.75 3.5-4.5 4.5 4.5 4.5' : 'm6.25 3.5 4.5 4.5-4.5 4.5'}
        fill="none"
        stroke="currentColor"
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="1.6"
      />
    </svg>
  );
}

export function DashboardAction(props: DashboardActionProps) {
  const contents = (
    <>
      {props.icon === 'back' && <DirectionIcon direction="back" />}
      {props.children}
      {props.icon === 'forward' && <DirectionIcon direction="forward" />}
    </>
  );
  if (props.kind === 'link') {
    return (
      <a
        href={props.href}
        className="ui-dashboard-action"
        data-action={props.variant}
        data-tone={props.tone ?? 'default'}
        data-testid={props.testId}
        aria-label={props.ariaLabel}
      >
        {contents}
      </a>
    );
  }
  return (
    <button
      type="button"
      onClick={props.onActivate}
      disabled={props.disabled}
      className="ui-dashboard-action"
      data-action={props.variant}
      data-tone={props.tone ?? 'default'}
      data-testid={props.testId}
      aria-label={props.ariaLabel}
    >
      {contents}
    </button>
  );
}

interface WorkspacePageHeaderProps {
  title: string;
  titleId: string;
  description?: string;
  back?: {
    label: string;
    onActivate: () => void;
  };
  trailing?: ReactNode;
}

export function WorkspacePageHeader({
  title,
  titleId,
  description,
  back,
  trailing,
}: WorkspacePageHeaderProps) {
  return (
    <header className="ui-workspace-page-header">
      {back && (
        <DashboardAction variant="quiet" icon="back" onActivate={back.onActivate}>
          {back.label}
        </DashboardAction>
      )}
      <div className="ui-workspace-page-copy">
        <h1 id={titleId}>{title}</h1>
        {description && <p>{description}</p>}
      </div>
      {trailing && <div className="ui-workspace-page-trailing">{trailing}</div>}
    </header>
  );
}

interface MainNavItemProps {
  label: string;
  icon: ReactNode;
  selected: boolean;
  badge?: ReactNode;
  onActivate: () => void;
}

export const MainNavItem = forwardRef<
  HTMLButtonElement,
  MainNavItemProps & React.ButtonHTMLAttributes<HTMLButtonElement>
>(function MainNavItem({
  label,
  icon,
  selected,
  badge,
  onActivate,
  onClick,
  className,
  ...rest
}, ref) {
  return (
    <button
      ref={ref}
      type="button"
      {...rest}
      // Compose rather than overwrite: a tooltip trigger (Base UI's render
      // prop) may inject its own onClick (e.g. for closeOnClick) on this
      // element. Calling both — instead of letting a later spread silently
      // replace onActivate — keeps navigation working under any wrapper.
      // These three props are re-asserted after the spread (rather than
      // spreading `rest` last) so a wrapper-injected aria-current/className
      // can never silently shadow the nav item's own selected state or base
      // styling; className is merged, not overwritten, via cn().
      onClick={(event) => {
        onClick?.(event);
        onActivate();
      }}
      aria-current={selected ? 'page' : undefined}
      aria-label={label}
      className={cn('ui-main-nav-item', className)}
    >
      {icon}
      <span className="home-nav-label">{label}</span>
      {badge && <span className="home-nav-badge">{badge}</span>}
    </button>
  );
});

interface DashboardSectionHeaderProps {
  eyebrow?: string;
  title?: string;
  titleId?: string;
  action?: ReactNode;
}

export function DashboardSectionHeader({ eyebrow, title, titleId, action }: DashboardSectionHeaderProps) {
  return (
    <div className="ui-dashboard-section-header">
      <div>
        {eyebrow && <p className="ui-dashboard-eyebrow">{eyebrow}</p>}
        {title && <h2 id={titleId}>{title}</h2>}
      </div>
      {action}
    </div>
  );
}

export interface DashboardStatItem {
  id: string;
  label: string;
  value: ReactNode;
  detail?: ReactNode;
}

interface DashboardStatGroupProps {
  kind: 'rows' | 'tiles';
  items: readonly DashboardStatItem[];
  ariaLabel: string;
}

export function DashboardStatGroup({ kind, items, ariaLabel }: DashboardStatGroupProps) {
  return (
    <dl className="ui-dashboard-stats" data-stats={kind} aria-label={ariaLabel}>
      {items.map((item) => (
        <div key={item.id} className="ui-dashboard-stat">
          <dt>{item.label}</dt>
          <dd>
            <span className="ui-dashboard-stat-value">{item.value}</span>
            {item.detail && <small>{item.detail}</small>}
          </dd>
        </div>
      ))}
    </dl>
  );
}
