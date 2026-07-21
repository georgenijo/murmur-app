import { useMemo, useState } from 'react';
import {
  deleteKnowledge,
  setKnowledgeEnabled,
  upsertKnowledge,
  type KnowledgeDraft,
  type KnowledgeEntry,
} from '../../lib/knowledge';
import { useKnowledge } from '../../lib/hooks/useKnowledge';

interface Props {
  active: boolean;
}

function TransformEditor({
  entry,
  onClose,
  onSaved,
}: {
  entry: KnowledgeEntry | null;
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const [name, setName] = useState(
    entry?.payload.kind === 'transform' ? entry.payload.name : '',
  );
  const [instruction, setInstruction] = useState(
    entry?.payload.kind === 'transform' ? entry.payload.instruction : '',
  );
  const [enabled, setEnabled] = useState(entry?.enabled ?? true);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const draft = useMemo<KnowledgeDraft>(() => ({
    id: entry?.id,
    expectedRevision: entry?.revision,
    payload: { kind: 'transform', name: name.trim(), instruction: instruction.trim() },
    enabled,
    scope: { kind: 'global' },
  }), [enabled, entry, instruction, name]);

  const save = async () => {
    if (!name.trim()) {
      setError('Enter a spoken name (e.g. “meeting notes”).');
      return;
    }
    if (!instruction.trim()) {
      setError('Enter the full rewrite instruction.');
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await upsertKnowledge(draft);
      await onSaved();
      onClose();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="space-y-3 rounded-xl border border-outline-variant/30 bg-surface-container-lowest p-3">
      <div>
        <label className="mb-1 block text-xs font-medium text-on-surface">Spoken name</label>
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          className="w-full rounded-lg border border-outline-variant/30 bg-surface px-3 py-2 text-sm text-on-surface"
          placeholder="e.g. meeting notes"
        />
      </div>
      <div>
        <label className="mb-1 block text-xs font-medium text-on-surface">Instruction</label>
        <textarea
          value={instruction}
          onChange={(e) => setInstruction(e.target.value)}
          rows={4}
          className="w-full rounded-lg border border-outline-variant/30 bg-surface px-3 py-2 text-sm text-on-surface"
          placeholder="Rewrite as concise meeting notes with action items…"
        />
      </div>
      <label className="flex items-center gap-2 text-xs text-on-surface">
        <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} />
        Enabled
      </label>
      {error && <p className="text-xs text-error">{error}</p>}
      <div className="flex gap-2">
        <button
          type="button"
          disabled={saving}
          onClick={() => void save()}
          className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-on-primary disabled:opacity-50"
        >
          {saving ? 'Saving…' : 'Save'}
        </button>
        <button type="button" onClick={onClose} className="rounded-lg px-3 py-1.5 text-xs text-on-surface-variant underline">
          Cancel
        </button>
      </div>
    </div>
  );
}

const TRANSFORM_LIST_REQUEST = { kind: 'transform' as const, limit: 100 };

/**
 * CRUD list for user-defined selected-text transforms (issue #312 D1).
 * Mirrors the snippets/voice-commands manager idiom via the knowledge store.
 */
export function TransformsManager({ active }: Props) {
  const { entries, loading, error, refresh } = useKnowledge(TRANSFORM_LIST_REQUEST, active);
  const [editing, setEditing] = useState<KnowledgeEntry | null | 'new'>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-2">
        <p className="text-xs text-on-surface-variant">
          Speak a saved name during transform hold to expand it to the full instruction.
          Built-ins: Shorten, Bullets, Professional, Fix grammar, Casual.
        </p>
        <button
          type="button"
          onClick={() => setEditing('new')}
          className="shrink-0 rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-on-primary"
        >
          Add
        </button>
      </div>

      {editing !== null && (
        <TransformEditor
          entry={editing === 'new' ? null : editing}
          onClose={() => setEditing(null)}
          onSaved={async () => {
            await refresh();
          }}
        />
      )}

      {(error || actionError) && (
        <p className="text-xs text-error">{error ?? actionError}</p>
      )}
      {loading && entries.length === 0 && (
        <p className="text-xs text-on-surface-variant">Loading…</p>
      )}
      {!loading && entries.length === 0 && editing === null && (
        <p className="text-xs text-on-surface-variant">No saved transforms yet.</p>
      )}

      <ul className="space-y-2">
        {entries.map((entry) => {
          const name = entry.payload.kind === 'transform' ? entry.payload.name : entry.id;
          const instruction =
            entry.payload.kind === 'transform' ? entry.payload.instruction : '';
          return (
            <li
              key={entry.id}
              className="flex items-start justify-between gap-3 rounded-lg border border-outline-variant/20 bg-surface-container-lowest px-3 py-2"
            >
              <div className="min-w-0">
                <p className="truncate text-sm font-medium text-on-surface">
                  {name}
                  {!entry.enabled && (
                    <span className="ml-2 text-xs font-normal text-on-surface-variant">(disabled)</span>
                  )}
                </p>
                <p className="mt-0.5 line-clamp-2 text-xs text-on-surface-variant">{instruction}</p>
              </div>
              <div className="flex shrink-0 flex-col items-end gap-1">
                <button
                  type="button"
                  className="text-xs text-on-surface-variant underline"
                  onClick={() => setEditing(entry)}
                >
                  Edit
                </button>
                <button
                  type="button"
                  className="text-xs text-on-surface-variant underline"
                  onClick={() => {
                    void setKnowledgeEnabled(entry, !entry.enabled)
                      .then(() => refresh())
                      .catch((e) => setActionError(String(e)));
                  }}
                >
                  {entry.enabled ? 'Disable' : 'Enable'}
                </button>
                <button
                  type="button"
                  className="text-xs text-error underline"
                  onClick={() => {
                    if (!window.confirm(`Delete transform “${name}”?`)) return;
                    void deleteKnowledge(entry)
                      .then(() => refresh())
                      .catch((e) => setActionError(String(e)));
                  }}
                >
                  Delete
                </button>
              </div>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
