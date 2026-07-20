import { useDeferredValue, useMemo, useState } from 'react';
import { open, save } from '@tauri-apps/plugin-dialog';
import type { AppProfile } from '../../lib/settings';
import {
  deleteAllKnowledge,
  deleteKnowledge,
  exportKnowledgeToFile,
  importKnowledgeFromFile,
  inspectKnowledgeImport,
  payloadDetail,
  payloadTitle,
  retryKnowledgeStore,
  scopeLabel,
  setKnowledgeEnabled,
  upsertKnowledge,
  type KnowledgeEntry,
  type KnowledgeImportSummary,
  type KnowledgeKind,
  type KnowledgeListRequest,
} from '../../lib/knowledge';
import { useKnowledge } from '../../lib/hooks/useKnowledge';
import { KnowledgeEditorModal } from './KnowledgeEditorModal';

interface Props {
  active: boolean;
  profiles: AppProfile[];
}

const KIND_LABELS: Record<KnowledgeKind, string> = {
  replacement_rule: 'Replacement',
  vocabulary_term: 'Vocabulary',
  snippet: 'Snippet',
};

function formatUpdated(timestamp: number) {
  return new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(timestamp);
}

function StatusBanner({ availability, message, recoveryAtMs, onRetry }: {
  availability: 'ready' | 'recovered' | 'reinitialized' | 'unavailable';
  message: string | null;
  recoveryAtMs: number | null;
  onRetry: () => void;
}) {
  if (availability === 'ready') return (
    <div className="flex items-center gap-2 rounded-lg border border-emerald-300/60 bg-emerald-50 px-3 py-2 text-xs text-emerald-800 dark:border-emerald-800 dark:bg-emerald-950/30 dark:text-emerald-300" role="status">
      <span className="h-2 w-2 rounded-full bg-emerald-500" /> Local store ready
    </div>
  );
  if (availability === 'unavailable') return (
    <div className="flex items-center justify-between gap-3 rounded-lg border border-red-300 bg-red-50 px-3 py-2 text-xs text-red-800 dark:border-red-800 dark:bg-red-950/30 dark:text-red-300" role="alert">
      <span>{message ?? 'The local knowledge store is unavailable.'}</span>
      <button type="button" onClick={onRetry} className="shrink-0 font-semibold underline">Retry</button>
    </div>
  );
  return (
    <div className="rounded-lg border border-amber-300 bg-amber-50 px-3 py-2 text-xs text-amber-900 dark:border-amber-800 dark:bg-amber-950/30 dark:text-amber-200" role="status">
      <strong>{availability === 'recovered' ? 'Recovered from a local backup.' : 'Started with a new local store.'}</strong>{' '}
      {message}{recoveryAtMs ? ` (${formatUpdated(recoveryAtMs)})` : ''}
    </div>
  );
}

function ConfirmDialog({ title, children, confirmLabel, dangerous = false, disabled = false, onCancel, onConfirm }: {
  title: string;
  children: React.ReactNode;
  confirmLabel: string;
  dangerous?: boolean;
  disabled?: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-6">
      <div role="dialog" aria-modal="true" aria-labelledby="knowledge-confirm-title" className="w-full max-w-md rounded-2xl bg-surface p-5 shadow-2xl">
        <h2 id="knowledge-confirm-title" className="text-base font-semibold text-on-surface">{title}</h2>
        <div className="mt-2 text-sm text-on-surface-variant">{children}</div>
        <div className="mt-5 flex justify-end gap-2">
          <button type="button" onClick={onCancel} className="rounded-lg border border-outline-variant/40 px-3 py-2 text-xs font-medium">Cancel</button>
          <button type="button" disabled={disabled} onClick={onConfirm} className={`rounded-lg px-3 py-2 text-xs font-semibold text-white disabled:opacity-40 ${dangerous ? 'bg-red-600' : 'bg-primary'}`}>{confirmLabel}</button>
        </div>
      </div>
    </div>
  );
}

export function KnowledgeManager({ active, profiles }: Props) {
  const [query, setQuery] = useState('');
  const deferredQuery = useDeferredValue(query);
  const [kind, setKind] = useState<KnowledgeKind | 'all'>('all');
  const [enabled, setEnabled] = useState<'all' | 'enabled' | 'disabled'>('all');
  const [scope, setScope] = useState<'all' | 'global' | 'app' | 'project'>('all');
  const [editing, setEditing] = useState<KnowledgeEntry | null | undefined>(undefined);
  const [deleteTarget, setDeleteTarget] = useState<KnowledgeEntry | null>(null);
  const [deleteAllOpen, setDeleteAllOpen] = useState(false);
  const [deletePhrase, setDeletePhrase] = useState('');
  const [importPreview, setImportPreview] = useState<{ path: string; summary: KnowledgeImportSummary } | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const request = useMemo<KnowledgeListRequest>(() => ({
    query: deferredQuery.trim() || undefined,
    kind: kind === 'all' ? undefined : kind,
    enabled: enabled === 'all' ? undefined : enabled === 'enabled',
    scopeKind: scope === 'all' ? undefined : scope,
  }), [deferredQuery, enabled, kind, scope]);
  const knowledge = useKnowledge(request, active);
  const unavailable = knowledge.status.availability === 'unavailable';

  const run = async (action: () => Promise<unknown>, success: string) => {
    setActionError(null);
    setNotice(null);
    try {
      await action();
      setNotice(success);
      await knowledge.refresh();
    } catch (cause) {
      setActionError(String(cause));
      throw cause;
    }
  };

  const retry = async () => {
    setActionError(null);
    try {
      const status = await retryKnowledgeStore();
      knowledge.setStatus(status);
      if (status.availability !== 'unavailable') await knowledge.refresh();
    } catch (cause) {
      setActionError(String(cause));
    }
  };

  const chooseExport = async () => {
    const path = await save({ defaultPath: 'murmur-personal-knowledge.json', filters: [{ name: 'JSON', extensions: ['json'] }] });
    if (typeof path !== 'string') return;
    await run(async () => {
      const count = await exportKnowledgeToFile(path);
      setNotice(`Exported ${count} ${count === 1 ? 'record' : 'records'}.`);
    }, 'Knowledge exported.');
  };

  const chooseImport = async () => {
    setActionError(null);
    const path = await open({ multiple: false, directory: false, filters: [{ name: 'Murmur knowledge', extensions: ['json'] }] });
    if (typeof path !== 'string') return;
    try {
      setImportPreview({ path, summary: await inspectKnowledgeImport(path) });
    } catch (cause) {
      setActionError(String(cause));
    }
  };

  return (
    <div className="space-y-4">
      <StatusBanner
        availability={knowledge.status.availability}
        message={knowledge.status.message}
        recoveryAtMs={knowledge.status.recoveryAtMs}
        onRetry={() => void retry()}
      />

      <div className="rounded-xl border border-outline-variant/30 bg-surface-container-lowest p-3">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <p className="text-sm font-medium text-on-surface">Personal knowledge</p>
            <p className="mt-1 max-w-xl text-xs text-on-surface-variant">
              Manage local replacement rules, vocabulary, reusable snippets, and the records used by Voice Commands. Voice-enabled records are applied locally during live transcription.
            </p>
          </div>
          <button type="button" onClick={() => setEditing(null)} disabled={unavailable} className="rounded-lg bg-primary px-3 py-2 text-xs font-semibold text-on-primary disabled:opacity-40">Create knowledge</button>
        </div>
        <div className="mt-3 flex flex-wrap gap-2">
          <button type="button" onClick={() => void chooseExport().catch(() => {})} disabled={unavailable} className="rounded-lg border border-outline-variant/40 px-3 py-1.5 text-xs font-medium disabled:opacity-40">Export…</button>
          <button type="button" onClick={() => void chooseImport()} disabled={unavailable} className="rounded-lg border border-outline-variant/40 px-3 py-1.5 text-xs font-medium disabled:opacity-40">Import…</button>
          <button type="button" onClick={() => setDeleteAllOpen(true)} disabled={unavailable || knowledge.status.recordCount === 0} className="rounded-lg border border-red-300 px-3 py-1.5 text-xs font-medium text-red-700 disabled:opacity-40 dark:border-red-800 dark:text-red-300">Delete all…</button>
          <span className="ml-auto self-center text-xs text-on-surface-variant">{knowledge.status.recordCount} stored locally · schema v{knowledge.status.schemaVersion}</span>
        </div>
      </div>

      <div className="grid gap-2 md:grid-cols-[minmax(180px,1fr)_repeat(3,minmax(105px,auto))]">
        <input value={query} onChange={(event) => setQuery(event.target.value)} aria-label="Search personal knowledge" placeholder="Search knowledge…" className="rounded-lg border border-outline-variant/40 bg-surface-container-lowest px-3 py-2 text-xs outline-none focus:border-primary" />
        <select aria-label="Filter knowledge type" value={kind} onChange={(event) => setKind(event.target.value as KnowledgeKind | 'all')} className="rounded-lg border border-outline-variant/40 bg-surface-container-lowest px-2 py-2 text-xs">
          <option value="all">All types</option><option value="replacement_rule">Replacements</option><option value="vocabulary_term">Vocabulary</option><option value="snippet">Snippets</option>
        </select>
        <select aria-label="Filter enabled state" value={enabled} onChange={(event) => setEnabled(event.target.value as typeof enabled)} className="rounded-lg border border-outline-variant/40 bg-surface-container-lowest px-2 py-2 text-xs">
          <option value="all">Any state</option><option value="enabled">Enabled</option><option value="disabled">Disabled</option>
        </select>
        <select aria-label="Filter visibility" value={scope} onChange={(event) => setScope(event.target.value as typeof scope)} className="rounded-lg border border-outline-variant/40 bg-surface-container-lowest px-2 py-2 text-xs">
          <option value="all">Any visibility</option><option value="global">Global</option><option value="app">App</option><option value="project">Project</option>
        </select>
      </div>

      {(actionError || knowledge.error) && <p role="alert" className="rounded-lg border border-red-300 bg-red-50 px-3 py-2 text-xs text-red-800 dark:border-red-800 dark:bg-red-950/30 dark:text-red-300">{actionError ?? knowledge.error}</p>}
      {notice && <p role="status" className="rounded-lg border border-emerald-300 bg-emerald-50 px-3 py-2 text-xs text-emerald-800 dark:border-emerald-800 dark:bg-emerald-950/30 dark:text-emerald-300">{notice}</p>}

      <div className="overflow-hidden rounded-xl border border-outline-variant/30">
        <div className="flex items-center justify-between bg-surface-container px-3 py-2 text-xs text-on-surface-variant">
          <span>{knowledge.loading && knowledge.entries.length === 0 ? 'Loading…' : `${knowledge.total} matching ${knowledge.total === 1 ? 'record' : 'records'}`}</span>
          <button type="button" onClick={() => void knowledge.refresh()} disabled={knowledge.loading || unavailable} className="font-medium underline disabled:opacity-40">Refresh</button>
        </div>
        {knowledge.entries.length === 0 && !knowledge.loading ? (
          <div className="px-4 py-10 text-center text-sm text-on-surface-variant">{unavailable ? 'Retry the store to manage personal knowledge.' : 'No knowledge matches these filters.'}</div>
        ) : (
          <ul className="divide-y divide-outline-variant/25">
            {knowledge.entries.map((entry) => (
              <li key={entry.id} className={`flex items-start gap-3 bg-surface-container-lowest px-3 py-3 ${entry.enabled ? '' : 'opacity-60'}`}>
                <button
                  type="button"
                  role="switch"
                  aria-checked={entry.enabled}
                  aria-label={`${entry.enabled ? 'Disable' : 'Enable'} ${payloadTitle(entry.payload)}`}
                  onClick={() => void run(() => setKnowledgeEnabled(entry, !entry.enabled), entry.enabled ? 'Knowledge disabled.' : 'Knowledge enabled.').catch(() => {})}
                  className={`relative mt-0.5 inline-flex h-5 w-9 shrink-0 items-center rounded-full ${entry.enabled ? 'bg-primary' : 'bg-surface-container-highest'}`}
                ><span className={`h-3.5 w-3.5 rounded-full bg-on-primary shadow transition-transform ${entry.enabled ? 'translate-x-4' : 'translate-x-1'}`} /></button>
                <button type="button" onClick={() => setEditing(entry)} className="min-w-0 flex-1 text-left">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="rounded bg-surface-container px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-on-surface-variant">{KIND_LABELS[entry.payload.kind]}</span>
                    <strong className="truncate text-sm text-on-surface">{payloadTitle(entry.payload)}</strong>
                  </div>
                  <p className="mt-1 line-clamp-2 whitespace-pre-wrap text-xs text-on-surface-variant">{payloadDetail(entry.payload)}</p>
                  <p className="mt-1 truncate text-[11px] text-on-surface-variant">{scopeLabel(entry.scope)} · {entry.provenance.replace('_', ' ')} · Updated {formatUpdated(entry.updatedAtMs)}</p>
                </button>
                <div className="flex shrink-0 gap-1">
                  <button type="button" onClick={() => setEditing(entry)} aria-label={`Edit ${payloadTitle(entry.payload)}`} className="rounded-md px-2 py-1 text-xs font-medium hover:bg-surface-container">Edit</button>
                  <button type="button" onClick={() => setDeleteTarget(entry)} aria-label={`Delete ${payloadTitle(entry.payload)}`} className="rounded-md px-2 py-1 text-xs font-medium text-red-700 hover:bg-red-50 dark:text-red-300 dark:hover:bg-red-950/30">Delete</button>
                </div>
              </li>
            ))}
          </ul>
        )}
        {knowledge.nextOffset !== null && (
          <button type="button" onClick={() => void knowledge.loadMore()} disabled={knowledge.loading} className="w-full border-t border-outline-variant/25 px-3 py-2 text-xs font-semibold text-primary disabled:opacity-40">{knowledge.loading ? 'Loading…' : 'Load 50 more'}</button>
        )}
      </div>

      {editing !== undefined && <KnowledgeEditorModal entry={editing} profiles={profiles} onClose={() => setEditing(undefined)} onSave={async (draft) => {
        await run(() => upsertKnowledge(draft), draft.id ? 'Knowledge updated.' : 'Knowledge created.');
        setEditing(undefined);
      }} />}

      {deleteTarget && <ConfirmDialog title="Delete this knowledge?" confirmLabel="Delete" dangerous onCancel={() => setDeleteTarget(null)} onConfirm={() => void run(() => deleteKnowledge(deleteTarget), 'Knowledge deleted.').then(() => setDeleteTarget(null)).catch(() => {})}>
        This permanently removes “{payloadTitle(deleteTarget.payload)}” from Murmur’s local store.
      </ConfirmDialog>}

      {importPreview && <ConfirmDialog title="Import personal knowledge?" confirmLabel="Import" onCancel={() => setImportPreview(null)} onConfirm={() => void run(async () => {
        const result = await importKnowledgeFromFile(importPreview.path);
        setNotice(`Imported ${result.imported}; skipped ${result.duplicates} duplicate${result.duplicates === 1 ? '' : 's'}.`);
      }, 'Knowledge imported.').then(() => setImportPreview(null)).catch(() => {})}>
        <p>{importPreview.summary.total} records inspected: {importPreview.summary.new} new, {importPreview.summary.duplicates} duplicates, and {importPreview.summary.conflicts} trigger conflicts.</p>
        <p className="mt-2">Existing records are never overwritten. Same-ID conflicts reject the import.</p>
      </ConfirmDialog>}

      {deleteAllOpen && <ConfirmDialog title="Delete all personal knowledge?" confirmLabel="Delete everything" dangerous disabled={deletePhrase !== 'DELETE'} onCancel={() => { setDeleteAllOpen(false); setDeletePhrase(''); }} onConfirm={() => void run(() => deleteAllKnowledge(knowledge.status.storeRevision), 'All personal knowledge deleted.').then(() => { setDeleteAllOpen(false); setDeletePhrase(''); }).catch(() => {})}>
        <p>This permanently deletes all knowledge plus local recovery backups. Export first if you may need it later.</p>
        <label className="mt-3 block text-xs font-medium text-on-surface">Type DELETE to confirm
          <input autoFocus aria-label="Type DELETE to confirm" value={deletePhrase} onChange={(event) => setDeletePhrase(event.target.value)} className="mt-1 w-full rounded-lg border border-red-300 bg-surface-container-lowest px-3 py-2 font-mono text-sm text-on-surface outline-none focus:border-red-500" />
        </label>
      </ConfirmDialog>}
    </div>
  );
}
