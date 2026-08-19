import { useMemo, useState } from 'react';
import {
  AVAILABLE_MODEL_OPTIONS,
  BUILTIN_MODES,
  LANGUAGE_OPTIONS,
  WRITING_STYLE_OPTIONS,
  type AppProfile,
  type MurmurMode,
  type WritingStyle,
} from '../../lib/settings';

const nextModeId = () => `mode.${Date.now().toString(36)}.${Math.random().toString(36).slice(2, 8)}`;

export function summarizeMode(mode: MurmurMode): string {
  const parts = [mode.writingStyle?.replace('_', ' ') ?? 'global style'];
  if (mode.modelId) parts.push(mode.modelId);
  if (mode.language) parts.push(mode.language);
  if (mode.vocabularyPolicy !== 'inherit') parts.push(`${mode.vocabularyPolicy} vocabulary`);
  if (mode.contextPolicy === 'project') parts.push('project context');
  if (mode.autoPaste != null) parts.push(mode.autoPaste ? 'auto-paste' : 'clipboard only');
  return parts.join(' · ');
}

/** Content stays in React memory. This preview never calls Tauri, clipboard, or paste APIs. */
export function previewModeText(text: string, mode: MurmurMode): string {
  if (mode.writingStyle === 'verbatim' || !text.trim()) return text;
  let next = text.trim().replace(/\s+/g, ' ');
  if (mode.writingStyle !== 'code_technical') {
    next = next.replace(/\b(?:um|uh)\b[ ,]*/gi, '');
  }
  if (mode.writingStyle === 'notes') return next.replace(/\bnew line\b/gi, '\n');
  return next.charAt(0).toUpperCase() + next.slice(1);
}

function blankMode(name = 'New Mode'): MurmurMode {
  return {
    id: nextModeId(), name, builtIn: false, enabled: true, writingStyle: null,
    cleanupEnabled: null, smartFormattingEnabled: null, cliFormattingEnabled: null,
    vocabularyPolicy: 'inherit', contextPolicy: 'none', modelId: null,
    language: null, autoPaste: null,
  };
}

export function ModesManager({ modes, profiles, onChange }: {
  modes: MurmurMode[];
  profiles: AppProfile[];
  onChange: (next: { modes: MurmurMode[]; appProfiles: AppProfile[] }) => void;
}) {
  const allModes = useMemo(() => [...BUILTIN_MODES, ...modes], [modes]);
  const [selectedId, setSelectedId] = useState(allModes[0]?.id ?? '');
  const [sample, setSample] = useState('um draft a concise project update');
  const selected = allModes.find((mode) => mode.id === selectedId) ?? allModes[0];
  const custom = selected && !selected.builtIn;

  const updateMode = (update: Partial<MurmurMode>) => {
    if (!selected || selected.builtIn) return;
    onChange({ modes: modes.map((mode) => mode.id === selected.id ? { ...mode, ...update } : mode), appProfiles: profiles });
  };
  const create = (source?: MurmurMode) => {
    const mode = source ? { ...source, id: nextModeId(), name: `${source.name} Copy`, builtIn: false, enabled: true } : blankMode();
    onChange({ modes: [...modes, mode], appProfiles: profiles });
    setSelectedId(mode.id);
  };
  const remove = () => {
    if (!selected || selected.builtIn) return;
    onChange({
      modes: modes.filter((mode) => mode.id !== selected.id),
      appProfiles: profiles.map((profile) => profile.modeId === selected.id ? { ...profile, modeId: null } : profile),
    });
    setSelectedId(BUILTIN_MODES[0].id);
  };
  const bind = (bundleId: string, checked: boolean) => onChange({
    modes,
    appProfiles: profiles.map((profile) => profile.bundleId === bundleId
      ? { ...profile, modeId: checked ? selected?.id ?? null : null }
      : profile),
  });

  if (!selected) return null;
  return (
    <section aria-labelledby="modes-title" className="rounded-xl border border-outline-variant/30 bg-surface-container-lowest p-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div><h2 id="modes-title" className="text-sm font-semibold text-on-surface">Modes</h2><p className="mt-1 text-xs text-on-surface-variant">Reusable local behavior for any number of apps.</p></div>
        <button type="button" onClick={() => create()} className="rounded-lg bg-primary px-3 py-2 text-xs font-semibold text-on-primary">Create Mode</button>
      </div>
      <div className="mt-4 grid gap-4 lg:grid-cols-[220px_minmax(0,1fr)]">
        <div className="space-y-1" role="list" aria-label="Modes">
          {allModes.map((mode) => <button key={mode.id} type="button" onClick={() => setSelectedId(mode.id)} className={`w-full rounded-lg px-3 py-2 text-left text-xs ${mode.id === selected.id ? 'bg-primary-container text-on-primary-container' : 'text-on-surface hover:bg-surface-container'}`}><span className="block font-semibold">{mode.name}</span><span className="block truncate text-[10px] opacity-70">{mode.builtIn ? 'Built-in' : mode.enabled ? 'Enabled' : 'Disabled'}</span></button>)}
        </div>
        <div className="min-w-0 space-y-4">
          <div className="flex flex-wrap gap-2">
            {custom && <button type="button" onClick={() => updateMode({ enabled: !selected.enabled })} className="rounded-lg bg-surface-container-high px-3 py-1.5 text-xs">{selected.enabled ? 'Disable' : 'Enable'}</button>}
            <button type="button" onClick={() => create(selected)} className="rounded-lg bg-surface-container-high px-3 py-1.5 text-xs">Duplicate</button>
            {custom && <button type="button" onClick={remove} className="rounded-lg bg-error-container px-3 py-1.5 text-xs text-on-error-container">Delete</button>}
          </div>
          {custom ? <input aria-label="Mode name" value={selected.name} maxLength={128} onChange={(event) => updateMode({ name: event.target.value })} className="w-full rounded-lg border border-outline-variant bg-surface-container-lowest px-3 py-2 text-sm" /> : <h3 className="text-base font-semibold text-on-surface">{selected.name}</h3>}
          <p className="rounded-lg bg-surface-container-low px-3 py-2 text-xs text-on-surface-variant">{summarizeMode(selected)}</p>
          {custom && <div className="grid gap-3 sm:grid-cols-2">
            <label className="text-xs">Writing style<select aria-label="Writing style" value={selected.writingStyle ?? ''} onChange={(e) => updateMode({ writingStyle: (e.target.value || null) as WritingStyle | null })} className="mt-1 w-full rounded-lg border border-outline-variant bg-surface-container-lowest px-2 py-2"><option value="">Use global</option>{WRITING_STYLE_OPTIONS.filter((o) => o.value !== 'inherit').map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}</select></label>
            <label className="text-xs">Model<select aria-label="Mode model" value={selected.modelId ?? ''} onChange={(e) => updateMode({ modelId: (e.target.value || null) as MurmurMode['modelId'] })} className="mt-1 w-full rounded-lg border border-outline-variant bg-surface-container-lowest px-2 py-2"><option value="">Use global</option>{AVAILABLE_MODEL_OPTIONS.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}</select></label>
            <label className="text-xs">Language<select aria-label="Mode language" value={selected.language ?? ''} onChange={(e) => updateMode({ language: e.target.value || null })} className="mt-1 w-full rounded-lg border border-outline-variant bg-surface-container-lowest px-2 py-2"><option value="">Use global</option>{LANGUAGE_OPTIONS.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}</select></label>
            <label className="text-xs">Vocabulary<select aria-label="Vocabulary policy" value={selected.vocabularyPolicy} onChange={(e) => updateMode({ vocabularyPolicy: e.target.value as MurmurMode['vocabularyPolicy'] })} className="mt-1 w-full rounded-lg border border-outline-variant bg-surface-container-lowest px-2 py-2"><option value="inherit">Inherit</option><option value="general">General</option><option value="technical">Technical</option></select></label>
          </div>}
          <fieldset><legend className="text-xs font-semibold text-on-surface">Bound applications</legend><div className="mt-2 flex flex-wrap gap-2">{profiles.length ? profiles.map((profile) => <label key={profile.bundleId} className="inline-flex items-center gap-2 rounded-lg bg-surface-container-low px-3 py-2 text-xs"><input type="checkbox" checked={profile.modeId === selected.id} onChange={(e) => bind(profile.bundleId, e.target.checked)} />{profile.label || profile.bundleId}</label>) : <span className="text-xs text-on-surface-variant">Add app overrides to bind this Mode.</span>}</div></fieldset>
          <div><label className="text-xs font-semibold text-on-surface">Before / after test<textarea aria-label="Mode test input" value={sample} onChange={(e) => setSample(e.target.value)} className="mt-1 min-h-20 w-full rounded-lg border border-outline-variant bg-surface-container-lowest p-3 text-sm" /></label><div className="mt-2 grid gap-2 sm:grid-cols-2"><pre className="whitespace-pre-wrap rounded-lg bg-surface-container-low p-3 text-xs">{sample}</pre><pre data-testid="mode-preview" className="whitespace-pre-wrap rounded-lg bg-primary-container/40 p-3 text-xs">{previewModeText(sample, selected)}</pre></div><p className="mt-1 text-[10px] text-on-surface-variant">Preview stays in this window. It never copies, pastes, or injects into another app.</p></div>
        </div>
      </div>
    </section>
  );
}
