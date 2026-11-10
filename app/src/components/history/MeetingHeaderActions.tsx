import { useRef, type KeyboardEvent, type ReactNode } from 'react';
import { Menu } from '@base-ui/react/menu';
import { ChevronDown, ChevronRight, Copy } from 'lucide-react';
import { MEETING_EXPORT_FORMATS, type MeetingReviewExportFormat } from '../../lib/meetings';
import { Button } from '../ui/FormControls';

interface MeetingHeaderActionsProps {
  active: boolean;
  busy: boolean;
  calendarDisabled: boolean;
  retainedAudio: boolean;
  bundleBusy: boolean;
  confirmingDelete?: boolean;
  onRename: () => void;
  onCalendar: () => void;
  onSpeakers: () => void;
  onCopy: (format: MeetingReviewExportFormat) => void;
  onExport: (format: MeetingReviewExportFormat) => void;
  onBundle: () => void;
  onDelete?: () => void;
  onDisarmDelete?: () => void;
}

function Popup({ children, submenu = false, finalFocus }: { children: ReactNode; submenu?: boolean; finalFocus?: () => HTMLElement | boolean }) {
  return <Menu.Portal><Menu.Positioner side={submenu ? 'right' : 'bottom'} align={submenu ? 'start' : 'end'} sideOffset={6} className="z-50">
    <Menu.Popup finalFocus={finalFocus} className="ui-menu-popup meeting-action-menu">{children}</Menu.Popup>
  </Menu.Positioner></Menu.Portal>;
}

/** Keyboard activation is handled here rather than delegated to the menu
 * primitive. The item remains open while armed, so it must own both presses. */
function isInvalidDestructiveKey(event: KeyboardEvent) {
  return event.repeat || event.nativeEvent.isComposing
    || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey;
}

function guardDestructiveKey(event: KeyboardEvent, onActivate: () => void) {
  if (event.key !== 'Enter' && event.key !== ' ') return;
  event.preventDefault();
  event.stopPropagation();
  if (!isInvalidDestructiveKey(event)) onActivate();
}

/** All menu items dispatch the same local export/naming handlers as the workspace. */
export function MeetingHeaderActions(props: MeetingHeaderActionsProps) {
  const moreTriggerRef = useRef<HTMLButtonElement>(null);
  const actionsRef = useRef<HTMLDivElement>(null);
  const focusAfterClose = useRef<string | null>(null);
  const formats = (action: MeetingHeaderActionsProps['onCopy']) => MEETING_EXPORT_FORMATS.map((format) => (
    <Menu.Item key={format.value} className="ui-menu-item" disabled={props.active && (format.value === 'srt' || format.value === 'vtt')} onClick={() => action(format.value)}>{format.label}</Menu.Item>
  ));
  return <div ref={actionsRef} className="meeting-header-actions">
    <Button size="sm" className="meeting-header-action meeting-copy" aria-label="Copy transcript" onClick={() => props.onCopy('markdown')}>
      <Copy aria-hidden="true" className="meeting-copy-icon h-3.5 w-3.5" /><span className="meeting-copy-label">Copy transcript</span>
    </Button>
    <Menu.Root>
      <Menu.Trigger className="ui-form-button meeting-header-action">Export <ChevronDown aria-hidden="true" className="h-3.5 w-3.5" /></Menu.Trigger>
      <Popup>{formats(props.onExport)}
        {props.retainedAudio && <><Menu.Separator className="ui-menu-separator" />
          <Menu.Item className="ui-menu-item" disabled={props.active || props.busy || props.bundleBusy} onClick={props.onBundle}>Export with audio…</Menu.Item>
        </>}
      </Popup>
    </Menu.Root>
    {!props.busy && <Menu.Root onOpenChange={(open) => {
      props.onDisarmDelete?.();
      if (open) focusAfterClose.current = null;
    }}>
      <Menu.Trigger ref={moreTriggerRef} aria-label="More meeting actions" className="ui-form-button meeting-header-action meeting-more">···</Menu.Trigger>
      <Popup finalFocus={() => {
        const workspace = actionsRef.current?.closest('.meeting-workspace');
        return focusAfterClose.current ? workspace?.querySelector<HTMLElement>(focusAfterClose.current) ?? moreTriggerRef.current ?? true : true;
      }}>
        <Menu.Item className="ui-menu-item" onClick={() => { focusAfterClose.current = 'form[aria-label="Name meeting"] input'; props.onRename(); }}>Rename…</Menu.Item>
        <Menu.Item className="ui-menu-item" disabled={props.calendarDisabled} onClick={() => { focusAfterClose.current = '.meeting-detail-header-panel input:not(:disabled), .meeting-detail-header-panel button:not(:disabled), form[aria-label="Name meeting"] input:not(:disabled)'; props.onCalendar(); }}>Name from calendar…</Menu.Item>
        <Menu.Item className="ui-menu-item" onClick={() => { focusAfterClose.current = '.meeting-speakers input'; props.onSpeakers(); }}>Edit speakers…</Menu.Item>
        <Menu.SubmenuRoot>
          <Menu.SubmenuTrigger className="ui-menu-item">Copy as <ChevronRight aria-hidden="true" className="ml-auto h-4 w-4" /></Menu.SubmenuTrigger>
          <Popup submenu>{formats(props.onCopy)}</Popup>
        </Menu.SubmenuRoot>
        {props.onDelete && <><Menu.Separator className="ui-menu-separator" />
          <Menu.Item className="ui-menu-item" data-variant="destructive" closeOnClick={false} onKeyDownCapture={(event) => guardDestructiveKey(event, () => props.onDelete?.())}
            onBlur={props.onDisarmDelete} onPointerLeave={props.onDisarmDelete} onClick={(event) => {
              if (event.detail > 1) return;
              props.onDelete?.();
            }}>{props.confirmingDelete ? 'Confirm delete…' : 'Delete meeting…'}</Menu.Item>
        </>}
      </Popup>
    </Menu.Root>}
  </div>;
}
