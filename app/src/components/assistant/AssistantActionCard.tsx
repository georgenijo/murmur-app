import type { AssistantAction } from '../../lib/assistant';

const STATUS_LABELS: Record<AssistantAction['status'], string> = {
  proposed: 'Pending approval',
  cancelled: 'Cancelled',
  expired: 'Expired',
  executing: 'In progress',
  completed: 'Completed',
  failed: 'Failed',
  uncertain: 'Uncertain',
};

function color(action: AssistantAction): string | null {
  const rgb = action.parameters.rgb_color;
  if (rgb === null) return null;
  const names: Record<string, string> = { '255,0,0': 'Red', '0,255,0': 'Green', '0,0,255': 'Blue', '255,255,255': 'White' };
  const coordinates = rgb.join(',');
  return `${names[coordinates] ?? 'Custom color'} (RGB ${rgb.join(', ')})`;
}

function setting(action: AssistantAction): string {
  if (action.parameters.power === 'off') return 'Turn off';
  const details = [
    action.parameters.brightness_pct === null ? null : `${action.parameters.brightness_pct}% brightness`,
    color(action),
  ].filter((value): value is string => value !== null);
  return details.length ? `Turn on · ${details.join(' · ')}` : 'Turn on';
}

export function AssistantActionCard({ action, disabled, onConfirm, onCancel }: {
  action: AssistantAction;
  disabled: boolean;
  onConfirm: (actionId: string) => Promise<void>;
  onCancel: (actionId: string) => Promise<void>;
}) {
  return <section className="assistant-action-card" data-status={action.status} aria-label={`Light action: ${STATUS_LABELS[action.status]}`}>
    <header>
      <div><p>Light action</p><strong>{setting(action)}</strong></div>
      <span className="assistant-action-status">{STATUS_LABELS[action.status]}</span>
    </header>
    <ul className="assistant-action-targets">
      {action.targets.map((target) => <li key={target.entity_id}>
        {target.name && <span>{target.name}</span>}
        <code>{target.entity_id}</code>
      </li>)}
    </ul>
    {action.status === 'proposed' && <div className="assistant-action-buttons">
      <button type="button" className="assistant-primary" disabled={disabled} onClick={() => void onConfirm(action.action_id)}>Confirm</button>
      <button type="button" className="assistant-secondary" disabled={disabled} onClick={() => void onCancel(action.action_id)}>Cancel</button>
    </div>}
    {action.verification && <p className="assistant-action-verification">
      {action.verification.message} Reported by Home Assistant; physical state not independently verified.
    </p>}
    <details className="assistant-action-details">
      <summary>Action details</summary>
      <dl>
        <div><dt>Action ID</dt><dd><code>{action.action_id}</code></dd></div>
        <div><dt>Connection ID</dt><dd><code>{action.connection_id}</code></dd></div>
        <div><dt>Created</dt><dd><time dateTime={action.created_at}>{action.created_at}</time></dd></div>
        <div><dt>Expires</dt><dd><time dateTime={action.expires_at}>{action.expires_at}</time></dd></div>
        {action.verification?.states.map((state) => <div key={state.entity_id}><dt>{state.entity_id}</dt><dd>{state.state}</dd></div>)}
      </dl>
    </details>
  </section>;
}
