import { useEffect, useMemo, useRef, useState } from 'react';
import type { VocabularyEntry, VoiceCommand } from '../../lib/settings';
import { vocabularyPrompt } from '../../lib/settings';
import { previewVocabularyAliases } from '../../lib/dictation';
import { validateVocabularyEntries } from '../../lib/vocabulary';

function newEntryId(): string {
  return globalThis.crypto?.randomUUID?.() ?? `vocabulary-${Date.now()}-${Math.random()}`;
}

export function VocabularyAliasesEditor({
  entries,
  voiceCommands,
  onChange,
}: {
  entries: VocabularyEntry[];
  voiceCommands: VoiceCommand[];
  onChange: (entries: VocabularyEntry[]) => void;
}) {
  const [draft, setDraft] = useState(entries);
  const [error, setError] = useState<string | null>(null);
  const [previewInput, setPreviewInput] = useState('npm run Tori dev');
  const [previewOutput, setPreviewOutput] = useState('');
  const [previewing, setPreviewing] = useState(false);
  const [includeCli, setIncludeCli] = useState(true);
  const [previewExpanded, setPreviewExpanded] = useState(false);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => { setDraft(entries); }, [entries]);

  const enabledPrompt = useMemo(() => vocabularyPrompt(draft), [draft]);

  const update = (next: VocabularyEntry[]) => {
    setDraft(next);
    setPreviewOutput('');
    const validationError = validateVocabularyEntries(next, voiceCommands);
    setError(validationError);
    if (!validationError) onChange(next);
  };

  const patchEntry = (index: number, patch: Partial<VocabularyEntry>) => {
    update(draft.map((entry, entryIndex) => entryIndex === index ? { ...entry, ...patch } : entry));
  };

  const addEntry = () => {
    setError(null);
    setPreviewOutput('');
    setDraft((current) => [...current, {
      id: newEntryId(),
      written: '',
      aliases: [],
      enabled: true,
      scope: { kind: 'global' },
    }]);
    requestAnimationFrame(() => {
      const list = listRef.current;
      list?.scrollTo({ top: list.scrollHeight, behavior: 'smooth' });
    });
  };

  const runPreview = async () => {
    const validationError = validateVocabularyEntries(draft, voiceCommands);
    if (validationError) { setError(validationError); return; }
    setPreviewing(true);
    setError(null);
    try {
      setPreviewOutput(await previewVocabularyAliases(draft, voiceCommands, previewInput, includeCli));
    } catch (previewError) {
      setError(String(previewError));
    } finally {
      setPreviewing(false);
    }
  };

  return (
    <div>
      <div className="flex items-center justify-between gap-4">
        <div>
          <p className="text-xs font-medium text-on-surface">
            {draft.length === 0 ? 'No saved spellings' : `${draft.length} saved ${draft.length === 1 ? 'spelling' : 'spellings'}`}
          </p>
          <p className="mt-0.5 text-[11px] text-on-surface-variant">
            If Murmur hears it more than one way, separate them with commas.
          </p>
        </div>
        <button
          type="button"
          onClick={addEntry}
          className="shrink-0 rounded-(--ui-radius-pill) bg-primary shadow-(--ui-shadow-accent) px-3 py-1.5 text-xs font-medium text-on-primary hover:brightness-110"
        >
          + Add spelling
        </button>
      </div>

      {draft.length === 0 ? (
        <div className="mt-3 rounded-xl border border-dashed border-outline-variant/40 bg-surface-container-lowest px-4 py-5 text-center">
          <p className="text-xs font-medium text-on-surface">Everything spelled correctly?</p>
          <p className="mt-1 text-[11px] text-on-surface-variant">
            Add a name or unusual word when Murmur gets it wrong.
          </p>
        </div>
      ) : (
        <div className="settings-card mt-3 overflow-hidden">
          <div className="grid grid-cols-[minmax(0,1fr)_20px_minmax(0,1fr)_76px] items-center gap-2 border-b border-outline-variant/30 bg-surface-container-low px-3 py-2 text-[10px] font-semibold uppercase tracking-wide text-on-surface-variant">
            <span>Murmur hears <span className="font-normal normal-case tracking-normal">(optional)</span></span>
            <span aria-hidden="true" />
            <span>Murmur types</span>
            <span className="sr-only">Actions</span>
          </div>
          <div
            ref={listRef}
            aria-label="Saved spellings"
            className="max-h-[286px] divide-y divide-outline-variant/20 overflow-y-auto overscroll-contain"
          >
            {draft.map((entry, index) => (
              <div
                key={entry.id}
                className={`grid grid-cols-[minmax(0,1fr)_20px_minmax(0,1fr)_76px] items-center gap-2 px-3 py-2 ${entry.enabled ? '' : 'bg-surface-container-low opacity-60'}`}
              >
                <input
                  aria-label={`Spoken aliases for ${entry.written || `term ${index + 1}`}`}
                  value={entry.aliases.join(', ')}
                  onChange={(event) => patchEntry(index, {
                    aliases: event.target.value.trim()
                      ? event.target.value.split(',').map((alias) => alias.trim())
                      : [],
                  })}
                  placeholder="e.g. Tori, Tory"
                  autoComplete="off"
                  autoCorrect="off"
                  spellCheck={false}
                  className="min-w-0 rounded-(--ui-radius-control) border-(--ui-hairline) bg-(--ui-tint-raised) px-2.5 py-1.5 text-xs text-on-surface placeholder:text-on-surface-variant/70 focus:outline-none focus:ring-2 focus:ring-primary"
                />
                <svg
                  aria-hidden="true"
                  viewBox="0 0 20 20"
                  className="h-4 w-4 justify-self-center text-on-surface-variant"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.5"
                >
                  <path d="M3 10h13m-4-4 4 4-4 4" strokeLinecap="round" strokeLinejoin="round" />
                </svg>
                <input
                  aria-label={`Written form ${index + 1}`}
                  value={entry.written}
                  onChange={(event) => patchEntry(index, { written: event.target.value })}
                  placeholder="e.g. Tauri"
                  autoComplete="off"
                  autoCorrect="off"
                  spellCheck={false}
                  className="min-w-0 rounded-(--ui-radius-control) border-(--ui-hairline) bg-(--ui-tint-raised) px-2.5 py-1.5 text-xs font-medium text-on-surface placeholder:font-normal placeholder:text-on-surface-variant/70 focus:outline-none focus:ring-2 focus:ring-primary"
                />
                <div className="flex items-center justify-end gap-2">
                  <button
                    type="button"
                    role="switch"
                    aria-label={`${entry.enabled ? 'Disable' : 'Enable'} ${entry.written || `term ${index + 1}`}`}
                    aria-checked={entry.enabled}
                    title={entry.enabled ? 'Turn off' : 'Turn on'}
                    onClick={() => patchEntry(index, { enabled: !entry.enabled })}
                    className={`relative inline-flex h-5 w-9 shrink-0 items-center rounded-full ${entry.enabled ? 'bg-primary' : 'bg-surface-container-highest'}`}
                  >
                    <span className={`inline-block h-3.5 w-3.5 rounded-full shadow transition-transform ${entry.enabled ? 'translate-x-4 bg-on-primary' : 'translate-x-1 bg-on-surface-variant'}`} />
                  </button>
                  <button
                    type="button"
                    aria-label={`Delete ${entry.written || `term ${index + 1}`}`}
                    title="Delete"
                    onClick={() => update(draft.filter((_, entryIndex) => entryIndex !== index))}
                    className="rounded p-1 text-on-surface-variant hover:bg-error/10 hover:text-error"
                  >
                    <svg aria-hidden="true" viewBox="0 0 20 20" className="h-4 w-4" fill="none" stroke="currentColor" strokeWidth="1.5">
                      <path d="M4.5 6h11M8 3.5h4M6.5 6l.6 10h5.8l.6-10M8.5 8.5v5M11.5 8.5v5" strokeLinecap="round" strokeLinejoin="round" />
                    </svg>
                  </button>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {error && (
        <p role="alert" className="mt-2 rounded-lg bg-error/10 px-3 py-2 text-xs text-error">
          {error}
        </p>
      )}

      <div className="settings-card mt-3">
        <button
          type="button"
          aria-expanded={previewExpanded}
          onClick={() => setPreviewExpanded((expanded) => !expanded)}
          className="flex w-full items-center justify-between gap-3 px-3 py-2.5 text-left"
        >
          <div>
            <p className="text-xs font-medium text-on-surface">Test a phrase</p>
            <p className="mt-0.5 text-[11px] text-on-surface-variant">See the result without recording anything.</p>
          </div>
          <svg aria-hidden="true" viewBox="0 0 20 20" className={`h-4 w-4 shrink-0 text-on-surface-variant transition-transform ${previewExpanded ? 'rotate-180' : ''}`} fill="none" stroke="currentColor" strokeWidth="1.5">
            <path d="m5 7.5 5 5 5-5" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </button>
        {previewExpanded && (
          <div className="border-t border-outline-variant/20 px-3 pb-3 pt-2">
            <div className="flex items-center justify-between gap-3">
              <p className="text-[11px] text-on-surface-variant">Runs locally in memory. Nothing is copied or logged.</p>
              <label className="flex shrink-0 items-center gap-2 text-[11px] text-on-surface-variant">
                <input type="checkbox" checked={includeCli} onChange={(event) => setIncludeCli(event.target.checked)} />
                Include CLI formatting
              </label>
            </div>
            <div className="mt-2 flex gap-2">
              <input
                aria-label="Alias preview input"
                value={previewInput}
                onChange={(event) => { setPreviewInput(event.target.value); setPreviewOutput(''); }}
                className="min-w-0 flex-1 rounded-(--ui-radius-control) border-(--ui-hairline) bg-(--ui-tint-raised) px-3 py-2 text-xs text-on-surface focus:outline-none focus:ring-2 focus:ring-primary"
              />
              <button
                type="button"
                disabled={previewing || !previewInput.trim()}
                onClick={() => void runPreview()}
                className="rounded-(--ui-radius-pill) bg-primary shadow-(--ui-shadow-accent) px-3 py-2 text-xs font-medium text-on-primary disabled:opacity-50"
              >
                {previewing ? 'Testing…' : 'Test'}
              </button>
            </div>
            {previewOutput && (
              <output aria-label="Alias preview output" className="mt-2 block rounded-lg bg-surface-container px-3 py-2 font-mono text-xs text-on-surface">
                {previewOutput}
              </output>
            )}
          </div>
        )}
      </div>

      {enabledPrompt && (
        <p className="mt-2 text-[11px] text-on-surface-variant">
          Your saved spellings work locally with every transcription model.
        </p>
      )}
    </div>
  );
}
