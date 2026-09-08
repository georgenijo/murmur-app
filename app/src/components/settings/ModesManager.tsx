import { useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  AVAILABLE_MODEL_OPTIONS,
  BUILTIN_MODES,
  LANGUAGE_OPTIONS,
  WRITING_STYLE_OPTIONS,
  SITE_MODE_BROWSERS,
  normalizeSiteHost,
  type AppProfile,
  type BrowserSiteRule,
  type MurmurMode,
  type WritingStyle,
} from '../../lib/settings';

const nextModeId = () => `mode.${Date.now().toString(36)}.${Math.random().toString(36).slice(2, 8)}`;
const nextSiteRuleId = () => `site.${Date.now().toString(36)}.${Math.random().toString(36).slice(2, 8)}`;

interface BrowserSiteProbe {
  status: 'available' | 'disabled' | 'unsupported_browser' | 'unavailable';
  browserBundleId: string | null;
  host: string | null;
}

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

export function ModesManager({ modes, profiles, siteLookupEnabled, siteRules, onChange }: {
  modes: MurmurMode[];
  profiles: AppProfile[];
  siteLookupEnabled: boolean;
  siteRules: BrowserSiteRule[];
  onChange: (next: {
    modes: MurmurMode[];
    appProfiles: AppProfile[];
    siteModeLookupEnabled: boolean;
    browserSiteRules: BrowserSiteRule[];
  }) => void;
}) {
  const allModes = useMemo(() => [...BUILTIN_MODES, ...modes], [modes]);
  const [selectedId, setSelectedId] = useState(allModes[0]?.id ?? '');
  const [sample, setSample] = useState('um draft a concise project update');
  const [siteBrowser, setSiteBrowser] = useState(SITE_MODE_BROWSERS[0].bundleId as string);
  const [siteHost, setSiteHost] = useState('');
  const [siteError, setSiteError] = useState<string | null>(null);
  const [siteProbe, setSiteProbe] = useState<string | null>(null);
  const [siteTesting, setSiteTesting] = useState(false);
  const [siteRuleDrafts, setSiteRuleDrafts] = useState<Record<string, string>>({});
  const selected = allModes.find((mode) => mode.id === selectedId) ?? allModes[0];
  const custom = selected && !selected.builtIn;
  const commit = (update: Partial<{
    modes: MurmurMode[];
    appProfiles: AppProfile[];
    siteModeLookupEnabled: boolean;
    browserSiteRules: BrowserSiteRule[];
  }>) => onChange({
    modes, appProfiles: profiles, siteModeLookupEnabled: siteLookupEnabled,
    browserSiteRules: siteRules, ...update,
  });

  const updateMode = (update: Partial<MurmurMode>) => {
    if (!selected || selected.builtIn) return;
    commit({ modes: modes.map((mode) => mode.id === selected.id ? { ...mode, ...update } : mode) });
  };
  const create = (source?: MurmurMode) => {
    const mode = source ? { ...source, id: nextModeId(), name: `${source.name} Copy`, builtIn: false, enabled: true } : blankMode();
    commit({ modes: [...modes, mode] });
    setSelectedId(mode.id);
  };
  const remove = () => {
    if (!selected || selected.builtIn) return;
    commit({
      modes: modes.filter((mode) => mode.id !== selected.id),
      appProfiles: profiles.map((profile) => profile.modeId === selected.id ? { ...profile, modeId: null } : profile),
      browserSiteRules: siteRules.filter((rule) => rule.modeId !== selected.id),
    });
    setSelectedId(BUILTIN_MODES[0].id);
  };
  const bind = (bundleId: string, checked: boolean) => commit({
    appProfiles: profiles.map((profile) => profile.bundleId === bundleId
      ? { ...profile, modeId: checked ? selected?.id ?? null : null }
      : profile),
  });
  const addSiteRule = () => {
    if (!selected) return;
    const host = normalizeSiteHost(siteHost);
    if (!host) { setSiteError('Enter an exact host such as github.com.'); return; }
    if (siteRules.some((rule) => rule.browserBundleId === siteBrowser && rule.host === host)) {
      setSiteError('That browser and host already have a rule.'); return;
    }
    commit({ browserSiteRules: [...siteRules, {
      id: nextSiteRuleId(), browserBundleId: siteBrowser, host,
      modeId: selected.id, enabled: true,
    }] });
    setSiteHost('');
    setSiteError(null);
  };
  const updateSiteRule = (rule: BrowserSiteRule, update: Partial<BrowserSiteRule>) => {
    const next = { ...rule, ...update };
    if (siteRules.some((item) => item.id !== rule.id
      && item.browserBundleId === next.browserBundleId && item.host === next.host)) {
      setSiteError('That browser and host already have a rule.');
      setSiteRuleDrafts((drafts) => ({ ...drafts, [rule.id]: rule.host }));
      return;
    }
    setSiteError(null);
    setSiteRuleDrafts((drafts) => ({ ...drafts, [rule.id]: next.host }));
    commit({ browserSiteRules: siteRules.map((item) => item.id === rule.id ? next : item) });
  };
  const testCurrentSite = async () => {
    setSiteTesting(true);
    setSiteProbe('Switch to the browser and site you want to test. Murmur will wait up to five seconds.');
    try {
      const result = await invoke<BrowserSiteProbe>('probe_browser_site');
      if (result.status === 'available' && result.host && result.browserBundleId) {
        setSiteBrowser(result.browserBundleId);
        setSiteHost(result.host);
        const match = siteRules.find((rule) => rule.enabled
          && rule.browserBundleId === result.browserBundleId && rule.host === result.host);
        setSiteProbe(match ? `Current site matches ${allModes.find((mode) => mode.id === match.modeId)?.name ?? 'an unavailable Mode'}.` : `Current site detected: ${result.host}.`);
      } else {
        setSiteProbe(result.status === 'disabled' ? 'Enable site activation before testing.' : 'No supported frontmost browser site is available.');
      }
    } catch { setSiteProbe('Murmur could not read the current browser host.'); }
    finally { setSiteTesting(false); }
  };

  if (!selected) return null;
  return (
    <section aria-labelledby="modes-title" className="settings-card p-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div><h2 id="modes-title" className="text-sm font-semibold text-on-surface">Modes</h2><p className="mt-1 text-xs text-on-surface-variant">Reusable local behavior for any number of apps.</p></div>
        <button type="button" onClick={() => create()} className="rounded-(--ui-radius-pill) bg-primary shadow-(--ui-shadow-accent) px-3 py-2 text-xs font-semibold text-on-primary">Create Mode</button>
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
          <fieldset className="space-y-3 rounded-lg border border-outline-variant/25 p-3">
            <legend className="px-1 text-xs font-semibold text-on-surface">Browser sites</legend>
            <label className="flex items-start justify-between gap-4 text-xs text-on-surface">
              <span><span className="block font-medium">Use frontmost browser host</span><span className="mt-1 block leading-relaxed text-on-surface-variant">Off by default. Reads only the current URL from an allowed browser, immediately discards everything except its exact host, and never reads page text or history. A host is saved only when you add a rule.</span></span>
              <input type="checkbox" role="switch" aria-label="Use browser site Mode rules" checked={siteLookupEnabled} onChange={(event) => commit({ siteModeLookupEnabled: event.target.checked })} />
            </label>
            {siteLookupEnabled && <>
              <div className="grid gap-2 sm:grid-cols-[160px_minmax(0,1fr)_auto]">
                <select aria-label="Site rule browser" value={siteBrowser} onChange={(event) => setSiteBrowser(event.target.value)} className="rounded-lg border border-outline-variant bg-surface-container-lowest px-2 py-2 text-xs">{SITE_MODE_BROWSERS.map((browser) => <option key={browser.bundleId} value={browser.bundleId}>{browser.label}</option>)}</select>
                <input aria-label="Site rule host" value={siteHost} onChange={(event) => setSiteHost(event.target.value)} placeholder="github.com" className="rounded-lg border border-outline-variant bg-surface-container-lowest px-3 py-2 text-xs" />
                <button type="button" onClick={addSiteRule} className="rounded-(--ui-radius-pill) bg-primary shadow-(--ui-shadow-accent) px-3 py-2 text-xs font-semibold text-on-primary">Add</button>
              </div>
              {siteError && <p role="alert" className="text-xs text-error">{siteError}</p>}
              <div className="space-y-2">{siteRules.filter((rule) => rule.modeId === selected.id).map((rule) => <div key={rule.id} className="grid items-center gap-2 rounded-lg bg-surface-container-low p-2 sm:grid-cols-[auto_150px_minmax(0,1fr)_auto]">
                <input type="checkbox" aria-label={`Enable ${rule.host}`} checked={rule.enabled} onChange={(event) => updateSiteRule(rule, { enabled: event.target.checked })} />
                <select aria-label={`Browser for ${rule.host}`} value={rule.browserBundleId} onChange={(event) => updateSiteRule(rule, { browserBundleId: event.target.value })} className="rounded-lg border border-outline-variant bg-surface-container-lowest px-2 py-1.5 text-xs">{SITE_MODE_BROWSERS.map((browser) => <option key={browser.bundleId} value={browser.bundleId}>{browser.label}</option>)}</select>
                <input aria-label={`Host for ${rule.host}`} value={siteRuleDrafts[rule.id] ?? rule.host} onChange={(event) => setSiteRuleDrafts((drafts) => ({ ...drafts, [rule.id]: event.target.value }))} onBlur={(event) => { const host = normalizeSiteHost(event.currentTarget.value); if (host) updateSiteRule(rule, { host }); else { setSiteRuleDrafts((drafts) => ({ ...drafts, [rule.id]: rule.host })); setSiteError('Enter an exact host such as github.com.'); } }} className="rounded-lg border border-outline-variant bg-surface-container-lowest px-2 py-1.5 text-xs" />
                <button type="button" aria-label={`Remove ${rule.host}`} onClick={() => commit({ browserSiteRules: siteRules.filter((item) => item.id !== rule.id) })} className="rounded-lg px-2 py-1.5 text-xs text-error">Remove</button>
              </div>)}</div>
              <button type="button" disabled={siteTesting} onClick={() => void testCurrentSite()} className="rounded-lg border border-outline-variant/40 px-3 py-2 text-xs font-semibold text-on-surface disabled:opacity-50">{siteTesting ? 'Waiting for browser…' : 'Test current site'}</button>
              {siteProbe && <p role="status" className="text-xs text-on-surface-variant">{siteProbe}</p>}
            </>}
          </fieldset>
          <div><label className="text-xs font-semibold text-on-surface">Before / after test<textarea aria-label="Mode test input" value={sample} onChange={(e) => setSample(e.target.value)} className="mt-1 min-h-20 w-full rounded-lg border border-outline-variant bg-surface-container-lowest p-3 text-sm" /></label><div className="mt-2 grid gap-2 sm:grid-cols-2"><pre className="whitespace-pre-wrap rounded-lg bg-surface-container-low p-3 text-xs">{sample}</pre><pre data-testid="mode-preview" className="whitespace-pre-wrap rounded-lg bg-primary-container/40 p-3 text-xs">{previewModeText(sample, selected)}</pre></div><p className="mt-1 text-[10px] text-on-surface-variant">Preview stays in this window. It never copies, pastes, or injects into another app.</p></div>
        </div>
      </div>
    </section>
  );
}
