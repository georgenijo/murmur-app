import type { ReactNode, ComponentPropsWithoutRef } from 'react';
import { Undo2, TriangleAlert } from 'lucide-react';
import { Tooltip } from '@base-ui/react/tooltip';

/** A title always reserves reset space so editing never moves its text. */
export function SettingReset({ onReset, disabled }: { onReset?: () => void; disabled?: boolean }) {
  return (
    <span className="settings-reset-slot">
      {onReset && (
        <Tooltip.Provider>
          <Tooltip.Root>
            <Tooltip.Trigger render={
              <button type="button" className="settings-reset" aria-label="Reset to default" disabled={disabled} onClick={onReset}>
                <Undo2 aria-hidden="true" />
              </button>
            } />
            <Tooltip.Portal>
              <Tooltip.Positioner sideOffset={6}>
                <Tooltip.Popup role="tooltip" className="ui-tooltip">Reset to default</Tooltip.Popup>
              </Tooltip.Positioner>
            </Tooltip.Portal>
          </Tooltip.Root>
        </Tooltip.Provider>
      )}
    </span>
  );
}

export function SettingsRow({ title, description, control, children, onReset, disabled = false, targetId, className = '', ...props }: Omit<ComponentPropsWithoutRef<'div'>, 'title'> & {
  title: ReactNode;
  description?: ReactNode;
  control?: ReactNode;
  onReset?: () => void;
  disabled?: boolean;
  targetId?: string;
}) {
  return (
    <div {...props} data-setting-target={targetId} data-disabled={disabled || undefined} className={`settings-setting-row ${className}`}>
      <div className="settings-row-layout">
        <div className="settings-title-description">
          <div className="settings-row-title">
            <span>{title}</span>
            <SettingReset onReset={onReset} disabled={disabled} />
          </div>
          {description && <p className="settings-row-description">{description}</p>}
          {children}
        </div>
        {control && <div className="settings-row-controls">{control}</div>}
      </div>
    </div>
  );
}

export function SettingsCard({ children, variant = 'grouped', className = '', ...props }: ComponentPropsWithoutRef<'div'> & { variant?: 'grouped' | 'plain' }) {
  return (
    <div {...props} className={`${variant === 'plain' ? 'settings-group-plain' : 'settings-card settings-card-body settings-rows'} ${className}`}>
      {children}
    </div>
  );
}

export function SettingsUnavailableGroup({ message, children }: { message: ReactNode; children: ReactNode }) {
  return (
    <div className="settings-unavailable">
      <div className="settings-unavailable-notice"><TriangleAlert aria-hidden="true" /><p>{message}</p></div>
      <div className="settings-unavailable-rows">{children}</div>
    </div>
  );
}
