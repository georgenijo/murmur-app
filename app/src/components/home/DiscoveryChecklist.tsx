import { Check, X } from 'lucide-react';
import type { ChecklistItemId } from '../../lib/discovery';

interface ChecklistItem {
  id: ChecklistItemId;
  title: string;
  detail: string;
  action: string;
}

const ITEMS: readonly ChecklistItem[] = [
  { id: 'command_palette', title: 'Open the command palette', detail: 'Jump anywhere with ⌘K', action: 'Open ⌘K' },
  { id: 'mode_binding', title: 'Bind a mode to an app', detail: 'Give one app its own writing behavior', action: 'Choose app' },
  { id: 'transform', title: 'Transform sample text', detail: 'Practice an on-device selected-text rewrite', action: 'Try it' },
  { id: 'voice_query', title: 'Set up Voice Query', detail: 'Ask through a CLI provider you choose', action: 'Set up' },
  { id: 'meeting', title: 'Record a meeting', detail: 'Capture local Me and Them transcripts', action: 'Open Notetaker' },
  { id: 'correction', title: 'Teach a correction', detail: 'Fix the latest dictation and remember it', action: 'Teach' },
  { id: 'shortcuts', title: 'See keyboard shortcuts', detail: 'Keep every Murmur action close', action: 'View' },
] as const;

interface DiscoveryChecklistProps {
  completed: readonly ChecklistItemId[];
  onAction: (id: ChecklistItemId) => void;
  onDismiss: () => void;
}

export function DiscoveryChecklist({ completed, onAction, onDismiss }: DiscoveryChecklistProps) {
  const completedItems = new Set(completed);
  const totalComplete = ITEMS.filter((item) => completedItems.has(item.id)).length;
  return (
    <section className="discovery-checklist" aria-labelledby="discovery-checklist-title">
      <div className="discovery-checklist-heading">
        <div>
          <p className="dashboard-eyebrow">Get more from Murmur</p>
          <h2 id="discovery-checklist-title">Try what is ready when you are</h2>
        </div>
        <div className="discovery-checklist-status">
          <span className="discovery-progress" aria-hidden="true">
            <span style={{ width: `${(totalComplete / ITEMS.length) * 100}%` }} />
          </span>
          <span className="discovery-checklist-count" aria-label={`${totalComplete} of ${ITEMS.length} complete`}>
            {totalComplete}/{ITEMS.length}
          </span>
          <button type="button" onClick={onDismiss} aria-label="Dismiss Get more from Murmur" className="ui-icon-button">
            <X aria-hidden="true" strokeWidth={2} />
          </button>
        </div>
      </div>
      <ul className="discovery-checklist-items">
        {ITEMS.map((item) => {
          const complete = completedItems.has(item.id);
          return (
            <li key={item.id} data-complete={complete}>
              <span className="discovery-check" aria-hidden="true">{complete && <Check strokeWidth={3} />}</span>
              <span className="min-w-0 flex-1">
                <strong>{item.title}</strong>
                <small>{item.detail}</small>
              </span>
              <button type="button" onClick={() => onAction(item.id)}>{complete ? 'Open again' : item.action}</button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
