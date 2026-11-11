import { defaultButtonKeyDown, noDefaultButtonKeyDown } from '../../lib/defaultButton';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useDialogFocus } from '../../lib/hooks/useDialogFocus';
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
import { Button, Input } from '../ui/FormControls';
import { Select } from '../ui/Select';
import AnimatedSwitch from '../ui/animated-switch/animated-switch';

interface Props {
  active: boolean;
  globallyEnabled: boolean;
  profiles: AppProfile[];
  onOpenVoiceCommandSettings?: () => void;
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

function CommandEditor({ entry, duplicate, profiles, onClose, onSaved, restoreFocus }: {
  entry: KnowledgeEntry | null;
  duplicate: boolean;
  profiles: AppProfile[];
  onClose: () => void;
  onSaved: () => Promise<void>;
  restoreFocus: () => HTMLElement | null;
}) {
  const editing = entry !== null && !duplicate;
  const initialKind = entryKind(entry);
  const [kind, setKind] = useState<VoiceCommandKind>(initialKind);
  const [phrase, setPhrase] = useState(duplicate ? '' : entryPhrase(entry));
  const [content, setContent] = useState(entryContent(entry));
  const [scopeKind, setScopeKind] = useState<'global' | 'app'>(entry?.scope.kind === 'app' ? 'app' : 'global');
  const [bundleId, setBundleId] = useState(entry?.scope.kind === 'app' ? entry.scope.bundleId : profiles[0]?.bundleId ?? '');
  const [enabled, setEnabled] = useState(entry?.enabled ?? true);
  const [allowClipboardRead, setAllowClipboardRead] = useState(entry?.voiceCommand?.allowClipboardRead ?? false);
  const [previewText, setPreviewText] = useState(duplicate ? '' : entryPhrase(entry));
  const [duplicatePreviewEdited, setDuplicatePreviewEdited] = useState(false);
  const [readClipboardPreview, setReadClipboardPreview] = useState(false);
  const [preview, setPreview] = useState<VoiceCommandPreviewResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const discardCancelRef = useRef<HTMLButtonElement>(null);
  const editedControlRef = useRef<HTMLElement | null>(null);
  const dirty = kind !== initialKind
    || phrase !== (duplicate ? '' : entryPhrase(entry))
    || content !== entryContent(entry)
    || scopeKind !== (entry?.scope.kind === 'app' ? 'app' : 'global')
    || bundleId !== (entry?.scope.kind === 'app' ? entry.scope.bundleId : profiles[0]?.bundleId ?? '')
    || enabled !== (entry?.enabled ?? true)
    || allowClipboardRead !== (entry?.voiceCommand?.allowClipboardRead ?? false);
  const requestClose = () => {
    if (saving) return;
    if (confirmDiscard) setConfirmDiscard(false);
    else if (dirty) {
      editedControlRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      setConfirmDiscard(true);
    }
    else onClose();
  };
  const dialogRef = useDialogFocus(requestClose, restoreFocus, true);
  useEffect(() => {
    if (confirmDiscard) discardCancelRef.current?.focus();
    else if (editedControlRef.current) {
      (editedControlRef.current.isConnected ? editedControlRef.current : dialogRef.current)?.focus();
      editedControlRef.current = null;
    }
  }, [confirmDiscard, dialogRef]);

  const draft = useMemo<KnowledgeDraft>(() => {
    const scope: KnowledgeScope = scopeKind === 'app'
      ? { kind: 'app', bundleId: bundleId.trim() }
      : { kind: 'global' };
    return {
      ...(editing && entry ? { id: entry.id, expectedRevision: entry.revision } : {}),
      payload: kind === 'snippet'
        ? { kind: 'snippet', trigger: phrase.trim(), body: content }
        : { kind: 'replacement_rule', source: phrase.trim(), replacement: content },
      enabled,
      scope,
      voiceCommand: { commandType: kind, allowClipboardRead: kind === 'snippet' && allowClipboardRead },
    };
  }, [allowClipboardRead, bundleId, content, editing, enabled, entry, kind, phrase, scopeKind]);

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

  const inputClass = 'mt-1 w-full';
  return (
    <div className="dialog-backdrop fixed inset-0 z-50 flex items-center justify-center p-6" onMouseDown={(event) => { if (event.target === event.currentTarget) requestClose(); }}>
      <div ref={dialogRef} tabIndex={-1} role="dialog" onKeyDown={defaultButtonKeyDown} aria-modal="true" aria-labelledby="voice-command-editor-title" className="dialog-popover flex max-h-[88vh] w-full max-w-[620px] flex-col overflow-hidden p-5">
        <div className="flex shrink-0 items-start justify-between gap-4">
          <div>
            <h2 id="voice-command-editor-title" className="text-base font-semibold text-on-surface">{duplicate ? 'Duplicate voice command' : entry ? 'Edit voice command' : 'Create voice command'}</h2>

          </div>
          <button type="button" onClick={requestClose} disabled={saving} aria-label="Close voice command editor" className="rounded-md px-2 py-1 text-on-surface-variant hover:bg-surface-container">✕</button>
        </div>

        <div className="mt-5 min-h-0 flex-1 scroll-py-3 space-y-4 overflow-y-auto" onFocusCapture={event => event.target.scrollIntoView?.({ block: 'nearest' })}>
          <div className="grid gap-3 sm:grid-cols-2">
            <label className="text-xs font-medium text-on-surface">Command type
              <Select aria-label="Voice command type" value={kind} onChange={(value) => { const next = value as VoiceCommandKind; setKind(next); if (next === 'text_replacement') { setAllowClipboardRead(false); setReadClipboardPreview(false); } }} items={[{ value: 'text_replacement', label: 'Text replacement' }, { value: 'snippet', label: 'Multiline snippet' }]} />
            </label>
            <label className="text-xs font-medium text-on-surface">Scope
              <Select aria-label="Voice command scope" value={scopeKind} onChange={(value) => setScopeKind(value as 'global' | 'app')} items={[{ value: 'global', label: 'All apps' }, { value: 'app', label: 'One configured app' }]} />
            </label>
          </div>

          {scopeKind === 'app' && (
            <label className="block text-xs font-medium text-on-surface">Application
              <Select aria-label="Voice command application" value={bundleId} onChange={setBundleId} items={profiles.length === 0 ? [{ value: '', label: 'Create a per-app profile first' }] : profiles.map((profile) => ({ value: profile.bundleId, label: `${profile.label || profile.bundleId} · ${profile.bundleId}` }))} />
            </label>
          )}

          <label className="block text-xs font-medium text-on-surface">Spoken phrase
            <Input aria-label="Voice command phrase" value={phrase} onChange={(event) => { setPhrase(event.target.value); if (duplicate) { if (!duplicatePreviewEdited) setPreviewText(event.target.value); } else if (!previewText) setPreviewText(event.target.value); }} maxLength={256} className={inputClass} placeholder="insert standup" />
          </label>

          <label className="block text-xs font-medium text-on-surface">{kind === 'snippet' ? 'Snippet body' : 'Replacement text'}
            {kind === 'snippet' ? (
              <textarea aria-label="Voice command content" value={content} onChange={(event) => setContent(event.target.value)} className={`${inputClass} mt-1 min-h-36 resize-y whitespace-pre-wrap font-mono`} maxLength={65_536} placeholder={'Yesterday:\n- …\nToday:\n- …\nBlocked:\n- …'} />
            ) : (
              <Input aria-label="Voice command content" value={content} onChange={(event) => setContent(event.target.value)} className={inputClass} maxLength={4096} placeholder="Replacement text" />
            )}
          </label>

          {kind === 'snippet' && (
            <div className="settings-card p-3">
              <p className="text-xs font-medium text-on-surface">Safe local variables</p>
              <div className="mt-2 flex flex-wrap gap-2">
                {(['date', 'time', 'clipboard'] as const).map((variable) => <button key={variable} type="button" onClick={() => insertVariable(variable)} className="rounded-md border border-outline-variant/40 bg-surface-container-lowest px-2 py-1 font-mono text-xs text-primary">{'{{'}{variable}{'}}'}</button>)}
              </div>
              <p className="mt-2 text-[11px] text-on-surface-variant">Date uses YYYY-MM-DD; time uses local 24-hour HH:mm. Both use one timestamp per command expansion.</p>
              <label className="mt-3 flex items-start gap-2 text-xs text-on-surface">
                <AnimatedSwitch aria-label="Allow clipboard reading" checked={allowClipboardRead} onCheckedChange={(checked) => { setAllowClipboardRead(checked); if (!checked) setReadClipboardPreview(false); }} size="sm" className="mt-0.5" />
                <span><strong>Allow this command to read clipboard text.</strong><span className="mt-0.5 block text-on-surface-variant">Required only for {'{{clipboard}}'}. Murmur reads it after this exact phrase matches; clipboard-first output is unchanged.</span></span>
              </label>
            </div>
          )}

          <label className="flex items-center gap-2 text-sm text-on-surface"><AnimatedSwitch aria-label="Use command" checked={enabled} onCheckedChange={setEnabled} size="sm" />Use command</label>

          <div className="settings-card p-3">
            <div className="flex items-center justify-between gap-3"><div><p className="text-xs font-medium text-on-surface">Test phrase and preview</p><p className="mt-0.5 text-[11px] text-on-surface-variant">Run the local matcher without copying or pasting</p></div><Button type="button" size="sm" onClick={() => void runPreview()}>Test</Button></div>
            <textarea aria-label="Voice command test phrase" value={previewText} onChange={(event) => { setPreviewText(event.target.value); if (duplicate) setDuplicatePreviewEdited(true); }} className={`${inputClass} mt-3 min-h-16 resize-y`} placeholder="Type an utterance containing the spoken phrase" />
            {kind === 'snippet' && allowClipboardRead && content.includes('{{clipboard}}') && <label className="mt-2 flex items-center gap-2 text-xs text-on-surface"><AnimatedSwitch aria-label="Read clipboard for this preview" checked={readClipboardPreview} onCheckedChange={setReadClipboardPreview} size="sm" />Read current clipboard for this preview</label>}
            {preview && <div className="mt-3 rounded-lg bg-surface-container-lowest px-3 py-2"><p className="text-[10px] font-semibold uppercase tracking-wide text-on-surface-variant">Preview</p><pre className="mt-1 whitespace-pre-wrap break-words font-sans text-sm text-on-surface">{preview.output}</pre>{preview.clipboardRequired && !preview.clipboardRead && <p className="mt-2 text-xs text-primary">Phrase matched, but clipboard preview was not explicitly enabled or readable.</p>}</div>}
          </div>

          {error && <p role="alert" className="rounded-lg border border-error/30 bg-error/10 px-3 py-2 text-xs text-error">{error}</p>}
        </div>
        <div className="shrink-0 border-t border-outline-variant/25 pt-4">
          {confirmDiscard ? <div role="group" aria-label="Unsaved voice command changes">
            <p className="mb-3 text-sm text-on-surface">Discard unsaved changes to this command?</p>
            <div className="flex justify-end gap-2">
              <Button ref={discardCancelRef} type="button" size="sm" data-default-button onClick={() => setConfirmDiscard(false)}>Keep editing</Button>
              <Button type="button" size="sm" onClick={onClose} data-tone="danger">Discard changes</Button>
            </div>
          </div> : <div className="flex justify-end gap-2">
            <Button type="button" size="sm" onClick={requestClose} disabled={saving}>Cancel</Button>
            <Button type="button" variant="primary" size="sm" onClick={() => void save()} data-default-button disabled={saving}>{saving ? 'Saving…' : 'Save command'}</Button>
          </div>}
        </div>
      </div>
    </div>
  );
}

export function VoiceCommandsManager({ active, globallyEnabled, profiles, onOpenVoiceCommandSettings }: Props) {
  const knowledge = useKnowledge({ voiceCommand: true }, active);
  const openerRef = useRef<HTMLButtonElement | null>(null);
  const newCommandRef = useRef<HTMLButtonElement>(null);
  const [editing, setEditing] = useState<{ entry: KnowledgeEntry | null; duplicate: boolean } | undefined>(undefined);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const openEditor = (entry: KnowledgeEntry | null, duplicate: boolean, opener: HTMLButtonElement) => {
    openerRef.current = opener;
    setEditing({ entry, duplicate });
  };

  const run = async (action: () => Promise<unknown>) => {
    setError(null);
    try { await action(); await knowledge.refresh(); } catch (cause) { setError(String(cause)); }
  };

  return (
    <div className="mt-4 space-y-3">
      {!globallyEnabled && <div className="rounded-lg border border-primary/70 bg-primary/10 px-3 py-2 text-xs text-on-surface">
        <p>Commands are saved but will not run until “Voice commands & spoken formatting” is turned on in Text → Advanced.</p>
        {onOpenVoiceCommandSettings && <Button type="button" variant="ghost" size="xs" onClick={onOpenVoiceCommandSettings} className="mt-2">Open Voice commands &amp; spoken formatting</Button>}
      </div>}
      <div className="flex items-start justify-between gap-3"><div><p className="text-sm font-medium text-on-surface">Custom commands</p></div><Button type="button" variant="primary" size="sm" ref={newCommandRef} onClick={(event) => openEditor(null, false, event.currentTarget)} disabled={knowledge.status.availability === 'unavailable'}>New command…</Button></div>
      {(error || knowledge.error) && <p role="alert" className="rounded-lg border border-error/30 bg-error/10 px-3 py-2 text-xs text-error">{error ?? knowledge.error}</p>}
      {knowledge.entries.length === 0 && !knowledge.loading ? <p className="rounded-lg border border-dashed border-outline-variant/40 px-3 py-6 text-center text-xs text-on-surface-variant">No custom voice commands yet.</p> : <ul className="space-y-2">{knowledge.entries.map((entry) => <li key={entry.id} className={`settings-card flex items-start gap-3 p-3 ${entry.enabled ? '' : 'opacity-60'}`}>
        <AnimatedSwitch checked={entry.enabled} size="sm" aria-label={`${entry.enabled ? 'Disable' : 'Enable'} Voice Command ${payloadTitle(entry.payload)}`} onCheckedChange={() => void run(() => setKnowledgeEnabled(entry, !entry.enabled))} className="mt-0.5" />
        <button type="button" onClick={(event) => openEditor(entry, false, event.currentTarget)} className="min-w-0 flex-1 text-left"><div className="flex flex-wrap items-center gap-2"><strong className="text-sm text-on-surface">“{payloadTitle(entry.payload)}”</strong><span className="rounded bg-surface-container px-1.5 py-0.5 text-[length:var(--ui-font-micro)] font-medium text-on-surface-variant">{entry.voiceCommand?.commandType === 'snippet' ? 'Snippet' : 'Text'}</span>{entry.voiceCommand?.allowClipboardRead && <span className="rounded bg-primary/10 px-1.5 py-0.5 text-[length:var(--ui-font-micro)] text-on-surface">Clipboard access</span>}</div><p className="mt-1 line-clamp-3 whitespace-pre-wrap text-xs text-on-surface-variant">{payloadDetail(entry.payload) || '(empty replacement)'}</p><p className="mt-1 text-[11px] text-on-surface-variant">{entry.scope.kind === 'global' ? 'All apps' : `Only ${entry.scope.bundleId}`}</p></button>
        <div className="flex shrink-0 gap-1"><button type="button" onClick={(event) => openEditor(entry, false, event.currentTarget)} className="rounded-md px-2 py-1 text-xs font-medium hover:bg-surface-container">Edit</button><button type="button" onClick={(event) => openEditor(entry, true, event.currentTarget)} className="rounded-md px-2 py-1 text-xs font-medium hover:bg-surface-container">Duplicate</button><button type="button" onKeyDown={noDefaultButtonKeyDown} onBlur={() => setConfirmDelete(null)} onClick={() => confirmDelete === entry.id ? void run(() => deleteKnowledge(entry)).then(() => setConfirmDelete(null)) : setConfirmDelete(entry.id)} className="rounded-md px-2 py-1 text-xs font-medium text-error hover:bg-error/10">{confirmDelete === entry.id ? 'Confirm delete…' : 'Delete…'}</button></div>
      </li>)}</ul>}
      {knowledge.nextOffset !== null && <button type="button" onClick={() => void knowledge.loadMore()} className="w-full rounded-lg border border-outline-variant/30 px-3 py-2 text-xs font-medium">Load more</button>}
      {editing !== undefined && <CommandEditor entry={editing.entry} duplicate={editing.duplicate} profiles={profiles} onClose={() => setEditing(undefined)} onSaved={knowledge.refresh} restoreFocus={() => openerRef.current?.isConnected ? openerRef.current : newCommandRef.current} />}
    </div>
  );
}
