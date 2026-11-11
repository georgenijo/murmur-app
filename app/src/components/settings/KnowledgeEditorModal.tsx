import { useEffect, useRef, useState } from 'react';
import { returnPressesDefault } from '../../lib/defaultButton';
import type { AppProfile } from '../../lib/settings';
import type {
  KnowledgeDraft,
  KnowledgeEntry,
  KnowledgeKind,
  KnowledgePayload,
  KnowledgeScope,
} from '../../lib/knowledge';
import { Button, Input } from '../ui/FormControls';
import { Select } from '../ui/Select';
import AnimatedSwitch from '../ui/animated-switch/animated-switch';

interface Props {
  entry: KnowledgeEntry | null;
  profiles: AppProfile[];
  onClose: () => void;
  onSave: (draft: KnowledgeDraft) => Promise<void>;
}

function initialPayload(entry: KnowledgeEntry | null): KnowledgePayload {
  return entry?.payload ?? { kind: 'replacement_rule', source: '', replacement: '' };
}

function scopeValues(scope: KnowledgeScope) {
  return {
    kind: scope.kind,
    bundleId: scope.kind === 'global' ? '' : scope.bundleId,
    root: scope.kind === 'project' ? scope.root : '',
  };
}

export function KnowledgeEditorModal({ entry, profiles, onClose, onSave }: Props) {
  const [payload, setPayload] = useState<KnowledgePayload>(() => initialPayload(entry));
  const [scope, setScope] = useState(() => scopeValues(entry?.scope ?? { kind: 'global' }));
  const [enabled, setEnabled] = useState(entry?.enabled ?? true);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const dialogRef = useRef<HTMLDivElement>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  // Save is this dialog's default button; read through a ref so the
  // mount-once key listener always presses the current draft's Save.
  const saveRef = useRef<() => void>(() => {});

  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        onCloseRef.current();
        return;
      }
      if (!event.defaultPrevented && dialogRef.current?.contains(event.target as Node) && returnPressesDefault(event, () => saveRef.current())) return;
      if (event.key !== 'Tab') return;
      const nodes = Array.from(dialogRef.current?.querySelectorAll<HTMLElement>(
        'button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled])',
      ) ?? []);
      if (nodes.length === 0) return;
      const first = nodes[0];
      const last = nodes[nodes.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener('keydown', onKey);
    const timer = window.setTimeout(() => dialogRef.current?.querySelector<HTMLElement>('[role="combobox"]')?.focus(), 40);
    return () => {
      document.removeEventListener('keydown', onKey);
      window.clearTimeout(timer);
      previous?.focus();
    };
  }, []);

  const changeKind = (kind: KnowledgeKind) => {
    if (kind === 'replacement_rule') setPayload({ kind, source: '', replacement: '' });
    if (kind === 'vocabulary_term') setPayload({ kind, written: '', aliases: [] });
    if (kind === 'snippet') setPayload({ kind, trigger: '', body: '' });
  };

  const buildScope = (): KnowledgeScope | null => {
    const bundleId = scope.bundleId.trim();
    if (scope.kind === 'global') return { kind: 'global' };
    if (!bundleId) return null;
    if (scope.kind === 'app') return { kind: 'app', bundleId };
    const root = scope.root.trim();
    return root ? { kind: 'project', bundleId, root } : null;
  };

  const validate = () => {
    if (payload.kind === 'replacement_rule' && (!payload.source.trim() || !payload.replacement.trim())) {
      return 'Enter both the heard phrase and its replacement.';
    }
    if (payload.kind === 'vocabulary_term' && !payload.written.trim()) {
      return 'Enter the written vocabulary term.';
    }
    if (payload.kind === 'snippet' && (!payload.trigger.trim() || !payload.body.trim())) {
      return 'Enter both the spoken trigger and snippet body.';
    }


    if (!buildScope()) return scope.kind === 'project'
      ? 'Project scope requires an app bundle ID and project root.'
      : 'App scope requires an app bundle ID.';
    return null;
  };

  const submit = async () => {
    const validation = validate();
    if (validation) {
      setError(validation);
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await onSave({
        id: entry?.id,
        expectedRevision: entry?.revision,
        payload,
        enabled,
        scope: buildScope()!,
        voiceCommand: entry?.voiceCommand ?? null,
      });
    } catch (cause) {
      setError(String(cause));
    } finally {
      setSaving(false);
    }
  };
  saveRef.current = () => {
    if (!saving) void submit();
  };

  const inputClass = 'mt-1';

  return (
    <div
      className="dialog-backdrop fixed inset-0 z-50 flex items-center justify-center p-6 backdrop-blur-[2px]"
      onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="knowledge-editor-title"
        className="dialog-popover max-h-[86vh] w-full max-w-[560px] overflow-y-auto p-5"
      >
        <div className="mb-4 flex items-start justify-between gap-4">
          <div>
            <h2 id="knowledge-editor-title" className="text-base font-semibold text-on-surface">
              {entry ? 'Edit knowledge' : 'Create knowledge…'}
            </h2>
            <p className="mt-1 text-xs text-on-surface-variant">
              Stored only on this Mac. Content is never sent to a cloud service or written to logs.
            </p>
          </div>
          <button type="button" onClick={onClose} aria-label="Close knowledge editor" className="rounded-md px-2 py-1 text-on-surface-variant hover:bg-surface-container">✕</button>
        </div>

        <div className="space-y-4">
          <label className="block text-xs font-medium text-on-surface">
            Type
            <Select
              value={payload.kind}
              onChange={(value) => changeKind(value as KnowledgeKind)}
              disabled={Boolean(entry)}
              aria-label="Knowledge type"
              items={[{ value: 'replacement_rule', label: 'Replacement rule' }, { value: 'vocabulary_term', label: 'Vocabulary term' }, { value: 'snippet', label: 'Snippet' }]}
            />
          </label>

          {payload.kind === 'replacement_rule' && (
            <div className="grid gap-3 sm:grid-cols-2">
              <label className="text-xs font-medium text-on-surface">Heard phrase
                <Input aria-label="Heard phrase" value={payload.source} onChange={(event) => setPayload({ ...payload, source: event.target.value })} className={inputClass} maxLength={256} />
              </label>
              <label className="text-xs font-medium text-on-surface">Replacement
                <Input aria-label="Replacement" value={payload.replacement} onChange={(event) => setPayload({ ...payload, replacement: event.target.value })} className={inputClass} maxLength={4096} />
              </label>
            </div>
          )}
          {payload.kind === 'vocabulary_term' && (
            <div className="space-y-3">
              <label className="block text-xs font-medium text-on-surface">Written form
                <input aria-label="Written form" value={payload.written} onChange={(event) => setPayload({ ...payload, written: event.target.value })} className={`${inputClass} mt-1`} maxLength={256} />
              </label>
              <label className="block text-xs font-medium text-on-surface">Spoken aliases
                <input aria-label="Spoken aliases" value={payload.aliases.join(', ')} onChange={(event) => setPayload({ ...payload, aliases: event.target.value.split(',').map((value) => value.trim()).filter(Boolean) })} className={`${inputClass} mt-1`} placeholder="Tori, Tory" />
                <span className="mt-1 block font-normal text-on-surface-variant">Comma-separated; up to 16 aliases.</span>
              </label>
            </div>
          )}
          {payload.kind === 'snippet' && (
            <div className="space-y-3">
              <label className="block text-xs font-medium text-on-surface">Spoken trigger
                <input aria-label="Spoken trigger" value={payload.trigger} onChange={(event) => setPayload({ ...payload, trigger: event.target.value })} className={`${inputClass} mt-1`} maxLength={256} />
              </label>
              <label className="block text-xs font-medium text-on-surface">Snippet body
                <textarea aria-label="Snippet body" value={payload.body} onChange={(event) => setPayload({ ...payload, body: event.target.value })} className={`${inputClass} mt-1 min-h-24 resize-y font-mono`} maxLength={16_384} />
              </label>
            </div>
          )}


          <div className="grid gap-3 sm:grid-cols-2">
            <label className="text-xs font-medium text-on-surface">Visibility
              <Select aria-label="Knowledge visibility" value={scope.kind} onChange={(value) => setScope((current) => ({ ...current, kind: value as KnowledgeScope['kind'] }))} items={[{ value: 'global', label: 'All apps' }, { value: 'app', label: 'One app' }, { value: 'project', label: 'One project in one app' }]} />
            </label>
            {scope.kind !== 'global' && (
              <label className="text-xs font-medium text-on-surface">App bundle ID
                <Input list="knowledge-profile-ids" aria-label="App bundle ID" value={scope.bundleId} onChange={(event) => setScope((current) => ({ ...current, bundleId: event.target.value }))} className={inputClass} placeholder="com.apple.Terminal" />
                <datalist id="knowledge-profile-ids">{profiles.map((profile) => <option key={profile.bundleId} value={profile.bundleId}>{profile.label}</option>)}</datalist>
              </label>
            )}
          </div>
          {scope.kind === 'project' && (
            <label className="block text-xs font-medium text-on-surface">Project root
              <Input aria-label="Project root" value={scope.root} onChange={(event) => setScope((current) => ({ ...current, root: event.target.value }))} className={inputClass} placeholder="/Users/me/code/project" />
            </label>
          )}

          <label className="flex items-center gap-2 text-sm text-on-surface">
            <AnimatedSwitch aria-label="Make this entry available to features that use it" checked={enabled} onCheckedChange={setEnabled} size="sm" />
            Make this entry available to features that use it
          </label>

          {entry?.voiceCommand && (
            <p className="rounded-lg border border-primary/25 bg-primary/5 px-3 py-2 text-xs text-on-surface">
              This record is also a {entry.voiceCommand.commandType === 'snippet' ? 'snippet' : 'text replacement'} Voice Command. Use the Voice Commands section to change its command type, test it, or review clipboard access.
            </p>
          )}

          {error && <p role="alert" className="rounded-lg border border-error/30 bg-error/10 px-3 py-2 text-xs text-error">{error}</p>}

          <div className="flex justify-end gap-2 border-t border-outline-variant/25 pt-4">
            <Button type="button" size="sm" onClick={onClose}>Cancel</Button>
            <Button type="button" variant="primary" size="sm" onClick={() => void submit()} data-default-button disabled={saving}>{saving ? 'Saving…' : 'Save knowledge'}</Button>
          </div>
        </div>
      </div>
    </div>
  );
}
