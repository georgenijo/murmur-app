import { ConfirmActionButton } from '../ui/ConfirmActionButton';
import { useId, useMemo, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button, Input } from '../ui/FormControls';
import { Select } from '../ui/Select';
import AnimatedSwitch from '../ui/animated-switch/animated-switch';
import { SettingsBranch } from './SettingsBranch';
import { SettingsCard, SettingsRow } from './SettingsRow';
import { ModeFileActions } from './ModeFileActions';
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

function blankMode(name = 'New mode'): MurmurMode {
  return {
    id: nextModeId(), name, builtIn: false, enabled: true, writingStyle: null,
    cleanupEnabled: null, smartFormattingEnabled: null, cliFormattingEnabled: null,
    vocabularyPolicy: 'inherit', contextPolicy: 'none', modelId: null,
    language: null, autoPaste: null,
  };
}

const VOCABULARY_OPTIONS: { value: MurmurMode['vocabularyPolicy']; label: string }[] = [
  { value: 'inherit', label: 'Inherit' },
  { value: 'general', label: 'General' },
  { value: 'technical', label: 'Technical' },
];
const BROWSER_OPTIONS = SITE_MODE_BROWSERS.map((browser) => ({ value: browser.bundleId as string, label: browser.label }));

/** One headed card on the Modes page, using the shared section rhythm. */
function ModesGroup({ title, children }: { title: string; children: ReactNode }) {
  const id = useId();
  return (
    <section className="settings-section" aria-labelledby={id}>
      <div className="settings-page-head"><h2 id={id} className="settings-section-title">{title}</h2></div>
      <SettingsCard>{children}</SettingsCard>
    </section>
  );
}

export function ModesManager({ modes, profiles, siteLookupEnabled, siteRules, onChange, onAddAppProfile, initialSelectedId }: {
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
  onAddAppProfile?: (modeId: string, opener: HTMLButtonElement) => void;
  initialSelectedId?: string | null;
}) {
  const allModes = useMemo(() => [...BUILTIN_MODES, ...modes], [modes]);
  const [selectedId, setSelectedId] = useState(() => (
    allModes.some((mode) => mode.id === initialSelectedId) ? initialSelectedId ?? '' : allModes[0]?.id ?? ''
  ));
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
    const mode = source ? { ...source, id: nextModeId(), name: `${source.name} copy`, builtIn: false, enabled: true } : blankMode();
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
  const bind = (bundleId: string, checked: boolean) => {
    commit({
      appProfiles: profiles.map((profile) => profile.bundleId === bundleId
        ? { ...profile, modeId: checked ? selected?.id ?? null : null }
        : profile),
    });
  };
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
  const selectedRules = siteRules.filter((rule) => rule.modeId === selected.id);
  const modeName = (id: string | null) => allModes.find((mode) => mode.id === id)?.name ?? 'an unavailable mode';
  return (
    <div className="settings-modes">
      <ModesGroup title="Library">
        <div className="settings-modes-list" role="list" aria-label="Modes">
          {allModes.map((mode) => <button key={mode.id} type="button" aria-pressed={mode.id === selected.id} onClick={() => setSelectedId(mode.id)} className="settings-mode-row">
            <span className="settings-mode-row-name">{mode.name || 'Untitled mode'}</span>
            <span className="settings-mode-row-status">{mode.builtIn ? 'Built-in' : mode.enabled ? 'Enabled' : 'Disabled'}</span>
          </button>)}
        </div>
        <SettingsRow title="Custom mode" description="Start from your global settings, or duplicate a selected mode"
          control={<Button variant="primary" onClick={() => create()}>Create mode</Button>} />
        <ModeFileActions settings={{ modes, appProfiles: profiles, browserSiteRules: siteRules, siteModeLookupEnabled: siteLookupEnabled }} onChange={onChange} />
      </ModesGroup>

      <ModesGroup title="Selected mode">
        <SettingsRow title={selected.name || 'Untitled mode'} description={custom ? summarizeMode(selected) : `Built-in template · ${summarizeMode(selected)}`}
          control={<>
            <Button size="sm" onClick={() => create(selected)}>Duplicate</Button>
            {custom && <ConfirmActionButton key={selected.id} label="Delete" onConfirm={remove} className="ui-form-button settings-modes-danger" data-size="sm" />}
          </>} />
        {custom ? <>
          <SettingsRow title="Enabled" description="A disabled mode stays saved but never applies to apps or sites"
            control={<AnimatedSwitch checked={selected.enabled} aria-label="Enable mode" onCheckedChange={(enabled) => updateMode({ enabled })} />} />
          <SettingsRow title="Name" control={<Input aria-label="Mode name" value={selected.name} maxLength={128} onChange={(event) => updateMode({ name: event.currentTarget.value })} className="settings-picker" />} />
          <SettingsRow title="Writing style" control={<Select aria-label="Writing style" value={selected.writingStyle ?? ''} onChange={(value) => updateMode({ writingStyle: (value || null) as WritingStyle | null })}
            items={[{ value: '', label: 'Use global' }, ...WRITING_STYLE_OPTIONS.filter((option) => option.value !== 'inherit').map(({ value, label }) => ({ value: value as string, label }))]} />} />
          <SettingsRow title="Model" control={<Select aria-label="Mode model" value={selected.modelId ?? ''} onChange={(value) => updateMode({ modelId: (value || null) as MurmurMode['modelId'] })}
            items={[{ value: '', label: 'Use global' }, ...AVAILABLE_MODEL_OPTIONS.map(({ value, label }) => ({ value: value as string, label }))]} />} />
          <SettingsRow title="Language" control={<Select aria-label="Mode language" value={selected.language ?? ''} onChange={(value) => updateMode({ language: value || null })}
            items={[{ value: '', label: 'Use global' }, ...LANGUAGE_OPTIONS]} />} />
          <SettingsRow title="Vocabulary" control={<Select aria-label="Vocabulary policy" value={selected.vocabularyPolicy} onChange={(value) => updateMode({ vocabularyPolicy: value })} items={VOCABULARY_OPTIONS} />} />
        </> : <SettingsRow title="Customize" description="Built-in modes are read-only. Duplicate one to change its style, model, language, or vocabulary." />}
      </ModesGroup>

      <ModesGroup title="Where it applies">
        {profiles.length ? <>
          <SettingsRow title="Bound applications" description={`Dictation in a bound app uses ${selected.name || 'this mode'}`} />
          {profiles.map((profile) => {
            const label = profile.label || profile.bundleId;
            const elsewhere = profile.modeId && profile.modeId !== selected.id;
            return <SettingsRow key={profile.bundleId} title={label} description={elsewhere ? `Uses ${modeName(profile.modeId ?? null)}` : profile.bundleId}
              control={<AnimatedSwitch checked={profile.modeId === selected.id} aria-label={`Bind ${label}`} onCheckedChange={(checked) => bind(profile.bundleId, checked)} />} />;
          })}
        </> : <SettingsRow title="Bound applications" description="Add an app, then choose its Mode here."
          control={onAddAppProfile && <Button onClick={(event) => onAddAppProfile(selected.id, event.currentTarget)}>Add an app</Button>} />}
        <div role="group" aria-label="Browser sites" className="settings-modes-sites">
        <SettingsRow targetId="browser-sites" title="Browser sites"
          description="Reads only the current URL from an allowed browser, immediately discards everything except its exact host, and never reads page text or history. A host is saved only when you add a rule."
          control={<AnimatedSwitch checked={siteLookupEnabled} aria-label="Use browser site mode rules" onCheckedChange={(checked) => commit({ siteModeLookupEnabled: checked })} />} />
        <SettingsBranch open={siteLookupEnabled}>
          <SettingsRow targetId="browser-site-rules" title="Add a site rule" description={`Use ${selected.name || 'this mode'} on an exact host in one browser`}>
            <div className="settings-modes-site-form">
              <Select aria-label="Site rule browser" value={siteBrowser} onChange={setSiteBrowser} items={BROWSER_OPTIONS} />
              <Input aria-label="Site rule host" value={siteHost} onChange={(event) => setSiteHost(event.currentTarget.value)} placeholder="github.com" />
              <Button onClick={addSiteRule}>Add</Button>
            </div>
            {siteError && <p role="alert" className="settings-modes-note settings-modes-error">{siteError}</p>}
          </SettingsRow>
          {selectedRules.map((rule) => <div key={rule.id} className="settings-modes-site-rule">
            <AnimatedSwitch size="sm" checked={rule.enabled} aria-label={`Enable ${rule.host}`} onCheckedChange={(enabled) => updateSiteRule(rule, { enabled })} />
            <Select size="sm" aria-label={`Browser for ${rule.host}`} value={rule.browserBundleId} onChange={(browserBundleId) => updateSiteRule(rule, { browserBundleId })} items={BROWSER_OPTIONS} />
            <Input aria-label={`Host for ${rule.host}`} value={siteRuleDrafts[rule.id] ?? rule.host} onChange={(event) => { const value = event.currentTarget.value; setSiteRuleDrafts((drafts) => ({ ...drafts, [rule.id]: value })); }} onBlur={(event) => { const host = normalizeSiteHost(event.currentTarget.value); if (host) updateSiteRule(rule, { host }); else { setSiteRuleDrafts((drafts) => ({ ...drafts, [rule.id]: rule.host })); setSiteError('Enter an exact host such as github.com.'); } }} />
            <ConfirmActionButton label="Remove" confirmLabel="Confirm remove" aria-label={`Remove ${rule.host}`} onConfirm={() => commit({ browserSiteRules: siteRules.filter((item) => item.id !== rule.id) })} className="ui-form-button settings-modes-danger" data-size="sm" />
          </div>)}
          <SettingsRow title="Current site" description="Detects the frontmost browser host within five seconds"
            control={<Button disabled={siteTesting} onClick={() => void testCurrentSite()}>{siteTesting ? 'Waiting for browser…' : 'Test current site'}</Button>}>
            {siteProbe && <p role="status" className="settings-modes-note">{siteProbe}</p>}
          </SettingsRow>
        </SettingsBranch>
        </div>
      </ModesGroup>

      <ModesGroup title="Before / after test">
        <SettingsRow title="Sample text" description="Preview stays in this window. It never copies, pastes, or injects into another app.">
          <textarea aria-label="Mode test input" value={sample} onChange={(event) => setSample(event.target.value)} className="settings-modes-sample" />
        </SettingsRow>
        <div className="settings-modes-preview">
          <div><p className="settings-modes-preview-label">Before</p><p className="settings-modes-preview-text">{sample}</p></div>
          <div><p className="settings-modes-preview-label">After</p><p data-testid="mode-preview" className="settings-modes-preview-text" data-tone="after">{previewModeText(sample, selected)}</p></div>
        </div>
      </ModesGroup>
    </div>
  );
}
