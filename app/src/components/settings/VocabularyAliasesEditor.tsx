import { useEffect, useMemo, useState } from 'react';
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
      <div className="flex items-start justify-between gap-4">
        <div>
          <label className="block text-sm font-medium text-on-surface">Custom Vocabulary</label>
          <p className="mt-1 text-xs text-on-surface-variant">
            Add the written term you want, then exact variants Murmur may hear. Aliases run locally on every model.
          </p>
        </div>
        <button
          type="button"
          onClick={() => update([...draft, {
            id: newEntryId(),
            written: '',
            aliases: [],
            enabled: true,
            scope: { kind: 'global' },
          }])}
          className="shrink-0 rounded-lg border border-outline-variant/30 bg-surface-container-lowest px-3 py-1.5 text-xs font-medium text-on-surface hover:border-primary/50"
        >
          Add term
        </button>
      </div>

      <div className="mt-3 space-y-3">
        {draft.length === 0 && (
          <div className="rounded-lg border border-dashed border-outline-variant/30 px-3 py-4 text-center text-xs text-on-surface-variant">
            No custom terms yet. Add one to teach Murmur a canonical spelling and spoken aliases.
          </div>
        )}
        {draft.map((entry, index) => (
          <div key={entry.id} className={`rounded-xl border p-3 ${entry.enabled ? 'border-outline-variant/30 bg-surface-container-lowest' : 'border-outline-variant/20 bg-surface-container-low opacity-70'}`}>
            <div className="flex items-center gap-2">
              <input
                aria-label={`Written form ${index + 1}`}
                value={entry.written}
                onChange={(event) => patchEntry(index, { written: event.target.value })}
                placeholder="Written form, e.g. Tauri"
                autoComplete="off"
                autoCorrect="off"
                spellCheck={false}
                className="min-w-0 flex-1 rounded-lg border border-outline-variant/30 bg-surface-container px-3 py-2 text-xs text-on-surface focus:outline-none focus:ring-2 focus:ring-primary"
              />
              <span className="rounded-full bg-primary/10 px-2 py-1 text-[10px] font-medium text-primary">Global</span>
              <button
                type="button"
                role="switch"
                aria-label={`${entry.enabled ? 'Disable' : 'Enable'} ${entry.written || `term ${index + 1}`}`}
                aria-checked={entry.enabled}
                onClick={() => patchEntry(index, { enabled: !entry.enabled })}
                className={`relative inline-flex h-5 w-9 shrink-0 items-center rounded-full ${entry.enabled ? 'bg-primary' : 'bg-surface-container-highest'}`}
              >
                <span className={`inline-block h-3.5 w-3.5 rounded-full bg-on-primary shadow transition-transform ${entry.enabled ? 'translate-x-4' : 'translate-x-1'}`} />
              </button>
              <button
                type="button"
                aria-label={`Delete ${entry.written || `term ${index + 1}`}`}
                onClick={() => update(draft.filter((_, entryIndex) => entryIndex !== index))}
                className="text-xs text-on-surface-variant underline hover:text-red-600"
              >
                Delete
              </button>
            </div>
            <label className="mt-2 block text-[11px] font-medium text-on-surface-variant">
              Spoken aliases
            </label>
            <input
              aria-label={`Spoken aliases for ${entry.written || `term ${index + 1}`}`}
              value={entry.aliases.join(', ')}
              onChange={(event) => patchEntry(index, {
                aliases: event.target.value.trim()
                  ? event.target.value.split(',').map((alias) => alias.trim())
                  : [],
              })}
              placeholder="Tori, Tory"
              autoComplete="off"
              autoCorrect="off"
              spellCheck={false}
              className="mt-1 w-full rounded-lg border border-outline-variant/30 bg-surface-container px-3 py-2 text-xs text-on-surface focus:outline-none focus:ring-2 focus:ring-primary"
            />
          </div>
        ))}
      </div>

      {error && (
        <p role="alert" className="mt-2 rounded-lg bg-red-500/10 px-3 py-2 text-xs text-red-700 dark:text-red-300">
          {error}
        </p>
      )}

      <div className="mt-3 rounded-xl border border-outline-variant/30 bg-surface-container-lowest p-3">
        <div className="flex items-center justify-between gap-3">
          <div>
            <p className="text-xs font-medium text-on-surface">Try your aliases</p>
            <p className="mt-0.5 text-[11px] text-on-surface-variant">Runs locally in memory. Nothing is recorded, copied, or logged.</p>
          </div>
          <label className="flex items-center gap-2 text-[11px] text-on-surface-variant">
            <input type="checkbox" checked={includeCli} onChange={(event) => setIncludeCli(event.target.checked)} />
            Include CLI formatting
          </label>
        </div>
        <div className="mt-2 flex gap-2">
          <input
            aria-label="Alias preview input"
            value={previewInput}
            onChange={(event) => { setPreviewInput(event.target.value); setPreviewOutput(''); }}
            className="min-w-0 flex-1 rounded-lg border border-outline-variant/30 bg-surface-container px-3 py-2 text-xs text-on-surface focus:outline-none focus:ring-2 focus:ring-primary"
          />
          <button
            type="button"
            disabled={previewing || !previewInput.trim()}
            onClick={() => void runPreview()}
            className="rounded-lg bg-primary px-3 py-2 text-xs font-medium text-on-primary disabled:opacity-50"
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

      {enabledPrompt && (
        <p className="mt-2 text-[11px] text-on-surface-variant">
          Enabled written terms also bias Whisper; spoken aliases are post-model only.
        </p>
      )}
    </div>
  );
}
