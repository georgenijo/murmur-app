import type { PersonalizationSummary } from '../../lib/homeDashboard';

interface PersonalizationCardProps {
  summary: PersonalizationSummary;
  expanded?: boolean;
  onOpenVocabulary: () => void;
  onOpenStyles: () => void;
}

export function PersonalizationCard({
  summary,
  expanded = false,
  onOpenVocabulary,
  onOpenStyles,
}: PersonalizationCardProps) {
  return (
    <section className={`personalization-card ${expanded ? 'is-expanded' : ''}`} aria-labelledby={expanded ? 'personalization-title-expanded' : 'personalization-title'}>
      <div className="personalization-heading">
        <div>
          <p className="dashboard-eyebrow">{expanded ? 'Personalization' : 'Voice profile'}</p>
          <h2 id={expanded ? 'personalization-title-expanded' : 'personalization-title'}>{summary.stage}</h2>
        </div>
        <span className="personalization-stage">{summary.completed} of {summary.total} set up</span>
      </div>

      {expanded && (
        <div className="personalization-ladder" aria-label="Personalization stages">
          {(['Learning', 'Developing', 'Personalized'] as const).map((stage) => (
            <span key={stage} data-active={summary.stage === stage}>{stage}</span>
          ))}
        </div>
      )}

      <ul className="personalization-milestones">
        {summary.milestones.map((milestone) => (
          <li key={milestone.id}>
            <span className="personalization-check" data-complete={milestone.complete} aria-hidden="true">
              {milestone.complete ? '✓' : ''}
            </span>
            <span>
              <strong>{milestone.label}</strong>
              <small>{milestone.detail}</small>
            </span>
            {expanded && milestone.id === 'vocabulary' && !milestone.complete && (
              <button type="button" onClick={onOpenVocabulary}>Add term</button>
            )}
            {expanded && milestone.id === 'styles' && !milestone.complete && (
              <button type="button" onClick={onOpenStyles}>Set style</button>
            )}
          </li>
        ))}
      </ul>

      <p className="personalization-next"><strong>Next</strong> · {summary.nextAction}</p>
      {expanded && (
        <p className="personalization-privacy">These are explicit setup milestones from data stored on this Mac—not a voice-training or confidence score.</p>
      )}
    </section>
  );
}
