import { useMemo, useState } from 'react';
import { loadHistory, type HistoryEntry } from '../../lib/history';
import type { VocabularyEntry } from '../../lib/settings';
import { normalizeVocabularyValue } from '../../lib/vocabulary';
import {
  loadDismissedSuggestions, MIN_SUGGESTION_TRANSCRIPTS, saveDismissedSuggestions, suggestVocabularyTerms,
} from '../../lib/vocabularySuggestions';
import { Button } from '../ui/FormControls';
import AnimatedSwitch from '../ui/animated-switch/animated-switch';

/**
 * Opt-in suggestions for Saved spellings, computed in memory from the local
 * history cache. Suggestions never change delivered text; only an explicit
 * Add creates a vocabulary entry, through the editor's normal validation.
 */
export function VocabularySuggestions({
  enabled,
  onEnabledChange,
  vocabulary,
  onAdd,
  loadEntries = loadHistory,
}: {
  enabled: boolean;
  onEnabledChange: (enabled: boolean) => void;
  vocabulary: VocabularyEntry[];
  onAdd: (term: string) => void;
  /** Injected in tests; defaults to the main window's synchronous history cache. */
  loadEntries?: () => HistoryEntry[];
}) {
  const [dismissed, setDismissed] = useState<Set<string>>(() => loadDismissedSuggestions());
  const [announcement, setAnnouncement] = useState('');
  // Read history once per opt-in, not per keystroke in the editor.
  const history = useMemo(() => (enabled ? loadEntries() : []), [enabled, loadEntries]);
  const suggestions = useMemo(
    () => (enabled ? suggestVocabularyTerms(history, vocabulary, dismissed) : []),
    [enabled, history, vocabulary, dismissed],
  );

  const updateDismissed = (next: Set<string>) => {
    setDismissed(next);
    saveDismissedSuggestions(next);
  };

  return (
    <section aria-labelledby="vocabulary-suggestions-title" className="settings-card mt-3 px-3 py-2.5" data-setting-target="vocabulary-suggestions">
      <div className="flex items-center justify-between gap-3">
        <div>
          <p id="vocabulary-suggestions-title" className="text-[length:var(--ui-font-caption)] font-medium text-on-surface">Suggestions from history</p>
          <p className="mt-0.5 text-[length:var(--ui-font-micro)] text-on-surface-variant">
            Find names and terms you say often in saved transcripts. Runs on this Mac; nothing changes until you add one.
          </p>
        </div>
        <AnimatedSwitch
          aria-label="Suggest spellings from history"
          checked={enabled}
          onCheckedChange={() => onEnabledChange(!enabled)}
          size="sm"
        />
      </div>

      {enabled && (
        <div className="mt-2 border-t border-outline-variant/20 pt-2">
          {suggestions.length === 0 ? (
            <p className="text-[length:var(--ui-font-micro)] text-on-surface-variant">
              {history.length === 0
                ? 'No saved transcripts yet. Suggestions appear once History has entries.'
                : `No suggestions right now. A term is suggested once it appears in ${MIN_SUGGESTION_TRANSCRIPTS} or more saved transcripts.`}
            </p>
          ) : (
            <ul aria-label="Suggested spellings" className="flex flex-col divide-y divide-outline-variant/20">
              {suggestions.map((suggestion) => (
                <li key={suggestion.term} className="flex items-center justify-between gap-3 py-1.5">
                  <div className="min-w-0">
                    <p className="truncate text-[length:var(--ui-font-caption)] font-medium text-on-surface">{suggestion.term}</p>
                    <p className="text-[length:var(--ui-font-micro)] text-on-surface-variant">
                      In {suggestion.transcripts} transcripts
                    </p>
                  </div>
                  <div className="flex shrink-0 items-center gap-1.5">
                    <Button
                      type="button"
                      size="xs"
                      variant="ghost"
                      aria-label={`Dismiss ${suggestion.term}`}
                      onClick={() => {
                        updateDismissed(new Set([...dismissed, normalizeVocabularyValue(suggestion.term)]));
                        setAnnouncement(`Dismissed ${suggestion.term}.`);
                      }}
                    >
                      Dismiss
                    </Button>
                    <Button
                      type="button"
                      size="xs"
                      aria-label={`Add ${suggestion.term} to saved spellings`}
                      onClick={() => {
                        onAdd(suggestion.term);
                        setAnnouncement(`Added ${suggestion.term} to saved spellings.`);
                      }}
                    >
                      Add
                    </Button>
                  </div>
                </li>
              ))}
            </ul>
          )}
          {dismissed.size > 0 && (
            <Button
              type="button"
              size="xs"
              variant="ghost"
              className="mt-1.5"
              onClick={() => { updateDismissed(new Set()); setAnnouncement('Dismissed suggestions restored.'); }}
            >
              Restore {dismissed.size} dismissed
            </Button>
          )}
        </div>
      )}
      <p aria-live="polite" className="sr-only">{announcement}</p>
    </section>
  );
}
