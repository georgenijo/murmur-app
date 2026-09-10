import {
  QUERY_CONTEXT_LEVEL_OPTIONS,
  QUERY_KEY_OPTIONS,
  type QueryKey,
  type QueryProviderId,
  type Settings,
} from '../../lib/settings';
import { isIncompleteCodexProbe, queryProviderTestMessage } from '../../lib/voiceQuerySettings';
import type { useVoiceQuerySettings } from '../../lib/hooks/useVoiceQuerySettings';
import { Select } from '../ui/Select';
import { QueryCapabilities } from './QueryCapabilities';
import { SettingToggle } from './SettingToggle';
import { SettingsCallout } from './SettingsLayout';
import { SettingsSection } from './SettingsSection';

interface VoiceQuerySettingsProps {
  settings: Settings;
  onUpdateSettings: (updates: Partial<Settings>) => void;
  activePage: string;
  accessibilityGranted: boolean | null;
  onRequestAccessibility: () => void;
  vm: ReturnType<typeof useVoiceQuerySettings>;
}

/** Voice Query settings page (F6): provider, privacy, shortcut, and response
 *  behavior. Extracted from `SettingsPanel.tsx`; all state and handlers live
 *  in `useVoiceQuerySettings`. */
export function VoiceQuerySettings({
  settings,
  onUpdateSettings,
  activePage,
  accessibilityGranted,
  onRequestAccessibility,
  vm,
}: VoiceQuerySettingsProps) {
  return (
    <SettingsSection pageId="ai-query" activePage={activePage} title="Voice Query" subtitle="Provider, privacy, shortcut, and response behavior">
      <SettingsCallout tone="warning" title="You control where the question goes">
        Murmur transcribes your question locally, then gives it to the exact CLI executable below.
        That CLI may send the question or answer to cloud services according to its own configuration, and may also send any optional app context you enable.
        Murmur cannot verify or prevent that network egress.
      </SettingsCallout>

      <div data-setting-target="voice-query-provider" className="settings-field rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
        <label className="block text-sm font-medium text-on-surface">Provider</label>
        <Select
          value={settings.queryProvider}
          onChange={(value) => void vm.selectQueryProvider(value as QueryProviderId)}
          items={vm.queryProviderItems.length > 0
            ? vm.queryProviderItems
            : [{ value: 'custom', label: 'Custom' }]}
        />
        {vm.selectedQueryPreset && settings.queryProvider !== 'custom' && (
          <p className="text-xs text-on-surface-variant">
            {vm.selectedQueryPreset.discoveredExecutable
              ? `Found ${vm.selectedQueryPreset.discoveredExecutable}`
              : `Not found in ${vm.selectedQueryPreset.discoveryPaths.join(', ')}`}
          </p>
        )}
        {settings.queryProvider === 'custom' && (
          <p className="text-xs text-on-surface-variant">
            Choose an absolute executable and its fixed arguments below. For a local smoke test,
            use <code>/usr/bin/printf</code> with one fixed argument: <code>%s</code>.
          </p>
        )}
      </div>

      <div><QueryCapabilities command={vm.queryCommand} /></div>

      <SettingToggle
        title="Enable Voice Query"
        description="Double-tap a dedicated key to record; tap once to finish. No spoken keyword is used."
        checked={settings.queryHotkey !== null}
        disabled={vm.queryConfigBusy}
        onChange={() => void vm.toggleVoiceQuery()}
      />
      <SettingToggle
        targetId="voice-query-copy"
        title="Automatically copy answers"
        description="Copy successful final answers to the clipboard. Voice Query never auto-pastes."
        checked={settings.queryAutomaticallyCopyAnswers}
        onChange={() => onUpdateSettings({
          queryAutomaticallyCopyAnswers: !settings.queryAutomaticallyCopyAnswers,
        })}
      />
      {vm.queryConfigError && <p role="alert" className="text-xs text-error">{vm.queryConfigError}</p>}
      {vm.queryConfigNotice && (
        <p role="status" className="text-xs text-on-surface-variant">{vm.queryConfigNotice}</p>
      )}

      <div className="settings-field">
          <label htmlFor="query-executable" className="block text-sm font-medium text-on-surface">CLI executable</label>
          <div className="flex gap-2">
            <input
              id="query-executable"
              type="text"
              value={settings.queryExecutable}
              onChange={(event) => vm.updateQueryExecutableText(event.target.value)}
              placeholder="/absolute/path/to/agent"
              spellCheck={false}
              className="min-w-0 flex-1 rounded-lg border border-outline-variant bg-surface-container-lowest px-3 py-2 font-mono text-xs text-on-surface outline-none focus:border-primary"
            />
            <button type="button" onClick={() => void vm.chooseQueryExecutable()} className="rounded-lg border border-outline-variant/30 px-3 py-2 text-xs font-semibold text-on-surface hover:bg-surface-container">
              Browse…
            </button>
          </div>
          <p className="text-xs text-on-surface-variant">
            Must be an absolute path to an executable file. No shell is ever invoked.
          </p>
      </div>

      <div className="settings-field">
          <label htmlFor="query-arguments" className="block text-sm font-medium text-on-surface">Fixed arguments</label>
          <textarea
            id="query-arguments"
            rows={3}
            value={settings.queryArguments.join('\n')}
            onChange={(event) => vm.updateQueryArgumentsText(event.target.value)}
            placeholder={'One argument per line\n--print'}
            spellCheck={false}
            className="w-full resize-y rounded-lg border border-outline-variant bg-surface-container-lowest px-3 py-2 font-mono text-xs leading-relaxed text-on-surface outline-none focus:border-primary"
          />
          <p className="text-xs text-on-surface-variant">
            Each line stays one argument. The transcript is appended as exactly one final argument, including spaces and punctuation.
          </p>
      </div>

      <div>
        <div className="settings-card p-3">
          <div className="flex items-center gap-3">
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium text-on-surface">Provider preflight</p>
              <p className="mt-0.5 text-xs leading-relaxed text-on-surface-variant">
                Runs the preset’s bounded authentication probe through the same direct-spawn and cleared-environment path as a query.
              </p>
            </div>
            <button
              type="button"
              disabled={vm.queryTestBusy || !settings.queryExecutable.trim()}
              onClick={() => void vm.runQueryTest()}
              className="rounded-(--ui-radius-pill) bg-primary shadow-(--ui-shadow-accent) px-3 py-1.5 text-xs font-semibold text-on-primary hover:bg-primary-dim disabled:cursor-not-allowed disabled:opacity-50"
            >
              {vm.queryTestBusy ? 'Testing…' : 'Test'}
            </button>
          </div>
          {vm.queryTestResult && (
            <div className="mt-3 space-y-2 text-xs">
              <p className={vm.queryTestResult.ok ? 'text-primary' : 'text-error'}>
                {queryProviderTestMessage(settings.queryProvider, vm.queryTestResult)}
              </p>
              {vm.queryTestResult.stdout && !isIncompleteCodexProbe(settings.queryProvider, vm.queryTestResult) && (
                <div>
                  <p className="font-semibold text-on-surface-variant">stdout{vm.queryTestResult.stdoutTruncated ? ' · tail only' : ''}</p>
                  <pre className="mt-1 max-h-32 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-surface-container-lowest p-2 font-mono text-[11px] text-on-surface">
                    {vm.queryTestResult.stdout}
                  </pre>
                </div>
              )}
              {vm.queryTestResult.stderr && !isIncompleteCodexProbe(settings.queryProvider, vm.queryTestResult) && (
                <div>
                  <p className="font-semibold text-on-surface-variant">stderr{vm.queryTestResult.stderrTruncated ? ' · tail only' : ''}</p>
                  <pre className="mt-1 max-h-32 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-surface-container-lowest p-2 font-mono text-[11px] text-on-surface">
                    {vm.queryTestResult.stderr}
                  </pre>
                </div>
              )}
              {!vm.queryTestResult.ok && vm.queryTestResult.signInFix && (
                <button
                  type="button"
                  onClick={() => void vm.signInQueryProvider()}
                  className="rounded-lg border border-outline-variant/30 px-3 py-1.5 font-semibold text-on-surface hover:bg-surface-container"
                >
                  Sign in…
                </button>
              )}
            </div>
          )}
          {vm.querySignInStatus && (
            <p aria-live="polite" className="mt-2 text-xs text-on-surface-variant">
              {vm.querySignInStatus}
            </p>
          )}
        </div>
      </div>

      {vm.selectedQueryPreset && (
          vm.selectedQueryPreset.permittedEnvironmentVariables.length > 0
          || vm.queryEnvironmentNeedsRepair
          || vm.queryEnvironmentStatus !== null
      ) && (
        <div>
          <div className="settings-card p-3">
            <p className="text-sm font-medium text-on-surface">
              {vm.selectedQueryPreset.permittedEnvironmentVariables.length > 0
                ? 'Declared config directories'
                : 'Voice Query environment'}
            </p>
            {vm.selectedQueryPreset.permittedEnvironmentVariables.length > 0 && (
              <p className="mt-0.5 text-xs leading-relaxed text-on-surface-variant">
                Optional absolute directory paths are added to the cleared child environment.
                HOME and the base allowlist cannot be overridden. API keys, tokens, and other
                secret variables are not accepted. Values live only in Rust-owned app data,
                never localStorage.
              </p>
            )}
            {vm.selectedQueryPreset.permittedEnvironmentVariables.length > 0 && (
              <div className="settings-stack mt-3">
                {vm.selectedQueryPreset.permittedEnvironmentVariables.map((name) => (
                  <div key={name} className="settings-field">
                    <label htmlFor={`query-env-${name}`} className="block font-mono text-xs font-medium text-on-surface">
                      {name}{vm.configuredQueryEnvironment.includes(name) ? ' · configured' : ''}
                    </label>
                    <input
                      id={`query-env-${name}`}
                      type="text"
                      value={vm.queryEnvironment.find((variable) => variable.name === name)?.value ?? ''}
                      onChange={(event) => vm.updateQueryEnvironmentDraft(name, event.target.value)}
                      placeholder={vm.configuredQueryEnvironment.includes(name)
                        ? 'Enter a replacement path'
                        : '/absolute/path/to/config'}
                      spellCheck={false}
                      className="w-full rounded-lg border border-outline-variant bg-surface-container-lowest px-3 py-2 font-mono text-xs text-on-surface outline-none focus:border-primary"
                    />
                  </div>
                ))}
              </div>
            )}
            <div className="mt-3 flex items-center gap-3">
              {vm.selectedQueryPreset.permittedEnvironmentVariables.length > 0 && (
                <button
                  type="button"
                  onClick={() => void vm.saveDeclaredEnvironment()}
                  className="rounded-lg border border-outline-variant/30 px-3 py-1.5 text-xs font-semibold text-on-surface hover:bg-surface-container"
                >
                  Save environment
                </button>
              )}
              {(vm.configuredQueryEnvironment.length > 0 || vm.queryEnvironmentNeedsRepair) && (
                <button
                  type="button"
                  onClick={() => void vm.clearDeclaredEnvironment()}
                  className="rounded-lg border border-outline-variant/30 px-3 py-1.5 text-xs font-semibold text-on-surface hover:bg-surface-container"
                >
                  Clear saved values
                </button>
              )}
              {vm.queryEnvironmentStatus && (
                <span className="text-xs text-on-surface-variant">{vm.queryEnvironmentStatus}</span>
              )}
            </div>
          </div>
        </div>
      )}

      <div className="settings-field">
          <label className="block text-sm font-medium text-on-surface">Context shared with the CLI</label>
          <Select
            value={settings.queryContextLevel}
            onChange={vm.updateQueryContextLevel}
            items={QUERY_CONTEXT_LEVEL_OPTIONS}
          />
          <p className="text-xs text-on-surface-variant">
            Off by default. App &amp; window adds only the frontmost app name and window title.
            Choose App, window &amp; selection (the third option) to also add up to 8 KiB of
            selected text after secure-field checks. The popover always shows what kind of
            context was included, and per-app exclusions take precedence.
          </p>
      </div>

      <div className="settings-field">
        <SettingToggle
          title="Keep Voice Query history on this Mac"
          description="Off by default. When on, Murmur keeps up to 200 recognized questions, answers, provider IDs, token counts, durations, and stable errors in a separate Rust-owned local store. This includes queries that shared app context: context is not stored as a separate field, but a saved answer may quote it. Turning history off affects new queries; existing entries remain until you delete them from History → Queries."
          checked={settings.retainQueryHistory}
          onChange={() => onUpdateSettings({ retainQueryHistory: !settings.retainQueryHistory })}
        />
        <p className="text-xs text-on-surface-variant">
          Voice Query counters and token usage appear under Insights in the main-window footer.
        </p>
      </div>

      <div className="grid grid-cols-2 gap-4">
          <div className="settings-field">
            <label className="block text-sm font-medium text-on-surface">Query shortcut</label>
            <Select
              value={settings.queryHotkey ?? QUERY_KEY_OPTIONS.find((option) => option.value !== settings.transformHoldKey)?.value ?? 'shift_r'}
              disabled={settings.queryHotkey === null}
              onChange={(value) => vm.updateQueryHotkey(value as QueryKey)}
              items={QUERY_KEY_OPTIONS}
            />
          </div>
          <div className="settings-field">
            <label className="block text-sm font-medium text-on-surface">Timeout</label>
            <Select
              value={String(settings.queryTimeoutSeconds)}
              onChange={vm.updateQueryTimeoutSeconds}
              items={[
                { value: '30', label: '30 seconds' },
                { value: '60', label: '1 minute' },
                { value: '120', label: '2 minutes' },
                { value: '300', label: '5 minutes' },
              ]}
            />
          </div>
      </div>

      {accessibilityGranted === false && settings.queryHotkey !== null && (
        <div>
          <div className="settings-callout flex items-center gap-2 text-xs text-on-surface" data-tone="accent">
            <span>Accessibility permission is required for the global query shortcut.</span>
            <button type="button" onClick={onRequestAccessibility} className="ml-auto underline">Grant</button>
          </div>
        </div>
      )}

      <div className="text-xs leading-relaxed text-on-surface-variant">
        Answers stream into a popover. Successful final answers are copied to the clipboard when automatic
        copying is enabled; otherwise, use Copy in the popover. They are never auto-pasted. Question and answer content enters only the separate local query store when you explicitly enable it.
        Context is never stored as a separate history field, but a saved answer may quote context shared with the CLI.
        Context content never enters saved files, usage stats, diagnostics, logs, or telemetry.
      </div>
    </SettingsSection>
  );
}
