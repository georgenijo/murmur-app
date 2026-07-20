import { useMemo, useState } from 'react';
import type { AppProfile } from '../../lib/settings';
import {
  deleteKnowledge,
  payloadDetail,
  payloadTitle,
  previewVoiceCommand,
  setKnowledgeEnabled,
  upsertKnowledge,
  type KnowledgeDraft,
  type KnowledgeEntry,
  type KnowledgeScope,
  type VoiceCommandKind,
  type VoiceCommandPreviewResponse,
} from '../../lib/knowledge';
import { useKnowledge } from '../../lib/hooks/useKnowledge';

interface Props {
  active: boolean;
  globallyEnabled: boolean;
  profiles: AppProfile[];
}

function entryKind(entry: KnowledgeEntry | null): VoiceCommandKind {
  return entry?.voiceCommand?.commandType ?? 'text_replacement';
}

function entryPhrase(entry: KnowledgeEntry | null) {
  if (!entry) return '';
  if (entry.payload.kind === 'replacement_rule') return entry.payload.source;
  if (entry.payload.kind === 'snippet') return entry.payload.trigger;
  return '';
}

function entryContent(entry: KnowledgeEntry | null) {
  if (!entry) return '';
  if (entry.payload.kind === 'replacement_rule') return entry.payload.replacement;
  if (entry.payload.kind === 'snippet') return entry.payload.body;
  return '';
}

function CommandEditor({ entry, profiles, onClose, onSaved }: {
  entry: KnowledgeEntry | null;
  profiles: AppProfile[];
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const initialKind = entryKind(entry);
  const [kind, setKind] = useState<VoiceCommandKind>(initialKind);
  const [phrase, setPhrase] = useState(entryPhrase(entry));
  const [content, setContent] = useState(entryContent(entry));
  const [scopeKind, setScopeKind] = useState<'global' | 'app'>(entry?.scope.kind === 'app' ? 'app' : 'global');
  const [bundleId, setBundleId] = useState(entry?.scope.kind === 'app' ? entry.scope.bundleId : profiles[0]?.bundleId ?? '');
  const [enabled, setEnabled] = useState(entry?.enabled ?? true);
  const [allowClipboardRead, setAllowClipboardRead] = useState(entry?.voiceCommand?.allowClipboardRead ?? false);
  const [previewText, setPreviewText] = useState(entryPhrase(entry));
  const [readClipboardPreview, setReadClipboardPreview] = useState(false);
  const [preview, setPreview] = useState<VoiceCommandPreviewResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const draft = useMemo<KnowledgeDraft>(() => {
    const scope: KnowledgeScope = scopeKind === 'app'
      ? { kind: 'app', bundleId: bundleId.trim() }
      : { kind: 'global' };
    return {
      id: entry?.id,
      expectedRevision: entry?.revision,
      payload: kind === 'snippet'
        ? { kind: 'snippet', trigger: phrase.trim(), body: content }
        : { kind: 'replacement_rule', source: phrase.trim(), replacement: content },
      enabled,
      scope,
      voiceCommand: { commandType: kind, allowClipboardRead: kind === 'snippet' && allowClipboardRead },
    };
  }, [allowClipboardRead, bundleId, content, enabled, entry, kind, phrase, scopeKind]);

  const validate = () => {
    if (!phrase.trim()) return 'Enter a spoken phrase.';
    if (kind === 'snippet' && !content) return 'Enter a snippet body.';
    if (scopeKind === 'app' && !bundleId.trim()) return 'Choose an app profile.';
    if (kind === 'snippet' && content.includes('{{clipboard}}') && !allowClipboardRead) {
      return 'Allow clipboard reading before saving a snippet that uses {{clipboard}}.';
    }
    return null;
  };

  const save = async () => {
    const message = validate();
    if (message) { setError(message); return; }
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

  const runPreview = async () => {
    const message = validate();
    if (message) { setError(message); return; }
    setError(null);
    try {
      setPreview(await previewVoiceCommand(draft, previewText, readClipboardPreview));
    } catch (cause) {
      setPreview(null);
      setError(String(cause));
    }
  };

  const insertVariable = (variable: 'date' | 'time' | 'clipboard') => {
    setKind('snippet');
    setContent((current) => `${current}{{${variable}}}`);
  };

  const inputClass = 'w-full rounded-lg border border-outline-variant/40 bg-surface-container-lowest px-3 py-2 text-sm text-on-surface outline-none focus:border-primary focus:ring-1 focus:ring-primary';
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-6" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <div role="dialog" aria-modal="true" aria-labelledby="voice-command-editor-title" className="max-h-[88vh] w-full max-w-[620px] overflow-y-auto rounded-2xl border border-outline-variant/30 bg-surface p-5 shadow-2xl">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 id="voice-command-editor-title" className="text-base font-semibold text-on-surface">{entry ? 'Edit Voice Command' : 'Create Voice Command'}</h2>
            <p className="mt-1 text-xs text-on-surface-variant">Matching and expansion stay local. Commands insert text only.</p>
          </div>
          <button type="button" onClick={onClose} aria-label="Close Voice Command editor" className="rounded-md px-2 py-1 text-on-surface-variant hover:bg-surface-container">✕</button>
        </div>

        <div className="mt-5 space-y-4">
          <div className="grid gap-3 sm:grid-cols-2">
            <label className="text-xs font-medium text-on-surface">Command type
              <select aria-label="Voice Command type" value={kind} onChange={(event) => { const next = event.target.value as VoiceCommandKind; setKind(next); if (next === 'text_replacement') { setAllowClipboardRead(false); setReadClipboardPreview(false); } }} className={`${inputClass} mt-1`}>
                <option value="text_replacement">Text replacement</option>
                <option value="snippet">Multiline snippet</option>
              </select>
            </label>
            <label className="text-xs font-medium text-on-surface">Scope
              <select aria-label="Voice Command scope" value={scopeKind} onChange={(event) => setScopeKind(event.target.value as 'global' | 'app')} className={`${inputClass} mt-1`}>
                <option value="global">All apps</option>
                <option value="app">One configured app</option>
              </select>
            </label>
          </div>

          {scopeKind === 'app' && (
            <label className="block text-xs font-medium text-on-surface">Application
              <select aria-label="Voice Command application" value={bundleId} onChange={(event) => setBundleId(event.target.value)} className={`${inputClass} mt-1`}>
                {profiles.length === 0 && <option value="">Create a per-app profile first</option>}
                {profiles.map((profile) => <option key={profile.bundleId} value={profile.bundleId}>{profile.label || profile.bundleId} · {profile.bundleId}</option>)}
              </select>
            </label>
          )}

          <label className="block text-xs font-medium text-on-surface">Spoken phrase
            <input autoFocus aria-label="Voice Command phrase" value={phrase} onChange={(event) => { setPhrase(event.target.value); if (!previewText) setPreviewText(event.target.value); }} maxLength={256} className={`${inputClass} mt-1`} placeholder="insert standup" />
          </label>

          <label className="block text-xs font-medium text-on-surface">{kind === 'snippet' ? 'Snippet body' : 'Replacement text'}
            {kind === 'snippet' ? (
              <textarea aria-label="Voice Command content" value={content} onChange={(event) => setContent(event.target.value)} className={`${inputClass} mt-1 min-h-36 resize-y whitespace-pre-wrap font-mono`} maxLength={65_536} placeholder={'Yesterday:\n- …\nToday:\n- …\nBlocked:\n- …'} />
            ) : (
              <input aria-label="Voice Command content" value={content} onChange={(event) => setContent(event.target.value)} className={`${inputClass} mt-1`} maxLength={4096} placeholder="Replacement text" />
            )}
          </label>

          {kind === 'snippet' && (
            <div className="rounded-xl border border-outline-variant/30 bg-surface-container-low p-3">
              <p className="text-xs font-medium text-on-surface">Safe local variables</p>
              <div className="mt-2 flex flex-wrap gap-2">
                {(['date', 'time', 'clipboard'] as const).map((variable) => <button key={variable} type="button" onClick={() => insertVariable(variable)} className="rounded-md border border-outline-variant/40 bg-surface-container-lowest px-2 py-1 font-mono text-xs text-primary">{'{{'}{variable}{'}}'}</button>)}
              </div>
              <p className="mt-2 text-[11px] text-on-surface-variant">Date uses YYYY-MM-DD; time uses local 24-hour HH:mm. Both use one timestamp per command expansion.</p>
              <label className="mt-3 flex items-start gap-2 text-xs text-on-surface">
                <input aria-label="Allow clipboard reading" type="checkbox" checked={allowClipboardRead} onChange={(event) => { setAllowClipboardRead(event.target.checked); if (!event.target.checked) setReadClipboardPreview(false); }} className="mt-0.5 accent-primary" />
                <span><strong>Allow this command to read clipboard text.</strong><span className="mt-0.5 block text-on-surface-variant">Required only for {'{{clipboard}}'}. Murmur reads it after this exact phrase matches; clipboard-first output is unchanged.</span></span>
              </label>
            </div>
          )}

          <label className="flex items-center gap-2 text-sm text-on-surface"><input type="checkbox" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} className="accent-primary" />Command enabled</label>

          <div className="rounded-xl border border-outline-variant/30 p-3">
            <div className="flex items-center justify-between gap-3"><div><p className="text-xs font-medium text-on-surface">Test phrase and preview</p><p className="mt-0.5 text-[11px] text-on-surface-variant">Runs the real local matcher without copying or pasting.</p></div><button type="button" onClick={() => void runPreview()} className="rounded-lg bg-primary px-3 py-1.5 text-xs font-semibold text-on-primary">Test</button></div>
            <textarea aria-label="Voice Command test phrase" value={previewText} onChange={(event) => setPreviewText(event.target.value)} className={`${inputClass} mt-3 min-h-16 resize-y`} placeholder="Type an utterance containing the spoken phrase" />
            {kind === 'snippet' && allowClipboardRead && content.includes('{{clipboard}}') && <label className="mt-2 flex items-center gap-2 text-xs text-on-surface"><input aria-label="Read clipboard for this preview" type="checkbox" checked={readClipboardPreview} onChange={(event) => setReadClipboardPreview(event.target.checked)} className="accent-primary" />Read current clipboard for this preview</label>}
            {preview && <div className="mt-3 rounded-lg bg-surface-container-lowest px-3 py-2"><p className="text-[10px] font-semibold uppercase tracking-wide text-on-surface-variant">Preview</p><pre className="mt-1 whitespace-pre-wrap break-words font-sans text-sm text-on-surface">{preview.output}</pre>{preview.clipboardRequired && !preview.clipboardRead && <p className="mt-2 text-xs text-amber-700 dark:text-amber-300">Phrase matched, but clipboard preview was not explicitly enabled or readable.</p>}</div>}
          </div>

          {error && <p role="alert" className="rounded-lg border border-red-300 bg-red-50 px-3 py-2 text-xs text-red-700 dark:border-red-800 dark:bg-red-950/40 dark:text-red-300">{error}</p>}
          <div className="flex justify-end gap-2 border-t border-outline-variant/25 pt-4"><button type="button" onClick={onClose} className="rounded-lg border border-outline-variant/40 px-3 py-2 text-xs font-medium">Cancel</button><button type="button" onClick={() => void save()} disabled={saving} className="rounded-lg bg-primary px-3 py-2 text-xs font-semibold text-on-primary disabled:opacity-50">{saving ? 'Saving…' : 'Save command'}</button></div>
        </div>
      </div>
    </div>
  );
}

export function VoiceCommandsManager({ active, globallyEnabled, profiles }: Props) {
  const knowledge = useKnowledge({ voiceCommand: true }, active);
  const [editing, setEditing] = useState<KnowledgeEntry | null | undefined>(undefined);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async (action: () => Promise<unknown>) => {
    setError(null);
    try { await action(); await knowledge.refresh(); } catch (cause) { setError(String(cause)); }
  };

  return (
    <div className="mt-4 space-y-3">
      {!globallyEnabled && <p className="rounded-lg border border-amber-300/70 bg-amber-50 px-3 py-2 text-xs text-amber-900 dark:border-amber-800 dark:bg-amber-950/30 dark:text-amber-200">Commands are stored but will not run until the Voice Commands switch is enabled.</p>}
      <div className="flex items-start justify-between gap-3"><div><p className="text-sm font-medium text-on-surface">Custom commands</p><p className="mt-1 text-xs text-on-surface-variant">Text-only local commands. App commands override a global command with the same phrase in that app.</p></div><button type="button" onClick={() => setEditing(null)} disabled={knowledge.status.availability === 'unavailable'} className="rounded-lg bg-primary px-3 py-2 text-xs font-semibold text-on-primary disabled:opacity-40">New command</button></div>
      {(error || knowledge.error) && <p role="alert" className="rounded-lg border border-red-300 bg-red-50 px-3 py-2 text-xs text-red-700 dark:border-red-800 dark:bg-red-950/40 dark:text-red-300">{error ?? knowledge.error}</p>}
      {knowledge.entries.length === 0 && !knowledge.loading ? <p className="rounded-lg border border-dashed border-outline-variant/40 px-3 py-6 text-center text-xs text-on-surface-variant">No custom Voice Commands yet.</p> : <ul className="space-y-2">{knowledge.entries.map((entry) => <li key={entry.id} className={`flex items-start gap-3 rounded-xl border border-outline-variant/30 bg-surface-container-lowest p-3 ${entry.enabled ? '' : 'opacity-60'}`}>
        <button type="button" role="switch" aria-checked={entry.enabled} aria-label={`${entry.enabled ? 'Disable' : 'Enable'} Voice Command ${payloadTitle(entry.payload)}`} onClick={() => void run(() => setKnowledgeEnabled(entry, !entry.enabled))} className={`relative mt-0.5 inline-flex h-5 w-9 shrink-0 items-center rounded-full ${entry.enabled ? 'bg-primary' : 'bg-surface-container-highest'}`}><span className={`h-3.5 w-3.5 rounded-full bg-on-primary shadow transition-transform ${entry.enabled ? 'translate-x-4' : 'translate-x-1'}`} /></button>
        <button type="button" onClick={() => setEditing(entry)} className="min-w-0 flex-1 text-left"><div className="flex flex-wrap items-center gap-2"><strong className="text-sm text-on-surface">“{payloadTitle(entry.payload)}”</strong><span className="rounded bg-surface-container px-1.5 py-0.5 text-[10px] uppercase text-on-surface-variant">{entry.voiceCommand?.commandType === 'snippet' ? 'Snippet' : 'Text'}</span>{entry.voiceCommand?.allowClipboardRead && <span className="rounded bg-amber-100 px-1.5 py-0.5 text-[10px] text-amber-900 dark:bg-amber-950/50 dark:text-amber-200">Clipboard access</span>}</div><p className="mt-1 line-clamp-3 whitespace-pre-wrap text-xs text-on-surface-variant">{payloadDetail(entry.payload) || '(empty replacement)'}</p><p className="mt-1 text-[11px] text-on-surface-variant">{entry.scope.kind === 'global' ? 'All apps' : `Only ${entry.scope.bundleId}`}</p></button>
        <div className="flex shrink-0 gap-1"><button type="button" onClick={() => setEditing(entry)} className="rounded-md px-2 py-1 text-xs font-medium hover:bg-surface-container">Edit</button><button type="button" onClick={() => confirmDelete === entry.id ? void run(() => deleteKnowledge(entry)).then(() => setConfirmDelete(null)) : setConfirmDelete(entry.id)} className="rounded-md px-2 py-1 text-xs font-medium text-red-700 hover:bg-red-50 dark:text-red-300">{confirmDelete === entry.id ? 'Confirm delete' : 'Delete'}</button></div>
      </li>)}</ul>}
      {knowledge.nextOffset !== null && <button type="button" onClick={() => void knowledge.loadMore()} className="w-full rounded-lg border border-outline-variant/30 px-3 py-2 text-xs font-medium">Load more</button>}
      {editing !== undefined && <CommandEditor entry={editing} profiles={profiles} onClose={() => setEditing(undefined)} onSaved={knowledge.refresh} />}
    </div>
  );
}
