import { TRANSFORM_KEY_OPTIONS, type Settings, type TransformKey } from '../../lib/settings';
import { TRANSFORM_MODEL_SIZE_LABEL } from '../../lib/transformSettings';
import type { useTransformModelSettings } from '../../lib/hooks/useTransformModelSettings';
import type { SettingsEditorTab } from './SettingsEditorsWindow';
import { Select } from '../ui/Select';
import { SettingToggle } from './SettingToggle';
import { SettingsBranch } from './SettingsBranch';
import { SettingsCallout } from './SettingsLayout';
import { SettingsSection } from './SettingsSection';
import { TransformPractice } from './TransformPractice';

interface TransformModelSettingsProps {
  settings: Settings;
  onUpdateSettings: (updates: Partial<Settings>) => void;
  activePage: string;
  accessibilityGranted: boolean | null;
  onRequestAccessibility: () => void;
  onOpenEditor: (tab: SettingsEditorTab) => void;
  vm: ReturnType<typeof useTransformModelSettings>;
}

/** Selected-Text Rewrite settings page (F7): on-device model, shortcut, and
 *  saved transforms. Extracted from `SettingsPanel.tsx`; all state and
 *  handlers live in `useTransformModelSettings`. */
export function TransformModelSettings({
  settings,
  onUpdateSettings,
  activePage,
  accessibilityGranted,
  onRequestAccessibility,
  onOpenEditor,
  vm,
}: TransformModelSettingsProps) {
  return (
    <SettingsSection pageId="ai-transform" activePage={activePage} title="Selected-Text Rewrite" subtitle="On-device rewriting, shortcut, and saved instructions">
      <SettingsCallout tone="accent" targetId="rewrite-model" title="Local only · Apple Silicon">
        Hold a dedicated shortcut, speak an instruction, and review a proposed rewrite before
        anything is written. The model stays on-device ({TRANSFORM_MODEL_SIZE_LABEL} download).
        Never auto-applies.
      </SettingsCallout>
      <SettingToggle
        targetId="correction-shortcut"
        title="Correct last dictation shortcut"
        description="Press ⌘⇧E to speak a correction to your latest dictation. Press again to finish, then review and copy or replace a matching selection. Uses the local model below."
        checked={settings.correctionShortcutEnabled}
        onChange={() => onUpdateSettings({ correctionShortcutEnabled: !settings.correctionShortcutEnabled })}
      />
      {settings.correctionShortcutEnabled && accessibilityGranted === false && (
        <p className="text-xs text-on-surface-variant">Accessibility access is required for ⌘⇧E. You can also start correction from the ⌘K command palette.</p>
      )}
      <SettingToggle
        targetId="transform-shortcut"
        title="Enable Transform Shortcut"
        description="Hold the transform key while text is selected to capture a rewrite instruction."
        checked={settings.transformHoldKey !== null}
        onChange={() => {
          void vm.updateTransformHoldKey(
            settings.transformHoldKey === null ? 'alt_r' : null,
          );
        }}
      />
      <SettingsBranch open={settings.transformHoldKey !== null}>
        <div className="settings-field">
          <label className="block text-sm font-medium text-on-surface">Hold key</label>
          <Select
            value={settings.transformHoldKey ?? 'alt_r'}
            onChange={(value) => {
              void vm.updateTransformHoldKey(value as TransformKey);
            }}
            items={TRANSFORM_KEY_OPTIONS}
          />
          <p className="text-xs text-on-surface-variant">
            Dictation hold keys are rejected. Right Option / Left Control / Right Shift only.
          </p>
          {vm.transformKeyError && (
            <p className="text-xs text-error">{vm.transformKeyError}</p>
          )}
          {accessibilityGranted === false && (
            <div className="settings-callout flex items-center gap-2 text-xs text-on-surface" data-tone="accent">
              <span>Accessibility permission is required for transform capture and apply.</span>
              <button type="button" onClick={onRequestAccessibility} className="ml-auto underline">Grant</button>
            </div>
          )}
        </div>
      </SettingsBranch>
      <TransformPractice
        transformHoldKey={settings.transformHoldKey}
        modelReady={vm.transformModel?.state === 'ready'}
      />
      <div>
        <h2 className="text-sm font-medium text-on-surface">On-device model</h2>
        <p className="mt-0.5 text-xs leading-relaxed text-on-surface-variant">
          Qwen2.5-1.5B Instruct (Q4_K_M), {TRANSFORM_MODEL_SIZE_LABEL}. Downloaded to
          Application Support; verified by size and SHA-256. Apple Silicon only.
        </p>
        <div className="settings-field mt-3">
        {vm.transformModel && (
          <p className="text-xs text-on-surface-variant" data-testid="transform-model-status">
            Status:{' '}
            {vm.transformModel.state === 'ready'
              ? 'Ready'
              : vm.transformModel.state === 'downloading'
                ? 'Downloading…'
                : 'Not downloaded'}
          </p>
        )}
        {vm.transformDownloadPct !== null && (
          <div>
            <div className="mb-1 flex justify-between text-xs text-on-surface-variant">
              <span>Downloading transform model</span>
              <span>{vm.transformDownloadPct}%</span>
            </div>
            <div className="h-1.5 overflow-hidden rounded-full bg-surface-container-highest">
              <div
                className="h-full rounded-full bg-primary transition-all duration-200"
                style={{ width: `${vm.transformDownloadPct}%` }}
              />
            </div>
          </div>
        )}
        {vm.transformModelError && (
          <p className="text-xs text-error">{vm.transformModelError}</p>
        )}
        <div className="flex flex-wrap gap-2">
          {vm.transformModel?.state !== 'ready' && (
            <button
              type="button"
              disabled={vm.transformModelBusy || vm.transformModel?.state === 'downloading'}
              onClick={() => void vm.downloadTransform()}
              className="rounded-(--ui-radius-pill) bg-primary shadow-(--ui-shadow-accent) px-3 py-1.5 text-xs font-medium text-on-primary disabled:opacity-50"
            >
              {vm.transformModelBusy || vm.transformModel?.state === 'downloading' ? 'Working…' : 'Download'}
            </button>
          )}
          {vm.transformModel?.state === 'ready' && (
            <button
              type="button"
              disabled={vm.transformModelBusy}
              onClick={() => void vm.removeTransform()}
              onBlur={() => vm.setConfirmRemoveTransform(false)}
              className="settings-quiet-btn px-3 py-1.5 text-xs font-medium text-on-surface-variant disabled:opacity-50"
            >
              {vm.confirmRemoveTransform ? 'Confirm remove' : 'Remove'}
            </button>
          )}
          {vm.transformModel?.runtimeDisabled && (
            <button
              type="button"
              disabled={vm.transformModelBusy}
              onClick={() => void vm.resetTransform()}
              className="rounded-lg border border-outline-variant/30 px-3 py-1.5 text-xs font-medium text-on-surface-variant disabled:opacity-50"
              title="Clear the circuit breaker if the transform runtime was disabled after repeated faults"
            >
              Reset runtime
            </button>
          )}
        </div>
        {vm.transformModel?.runtimeDisabled && (
          <p className="text-xs text-primary">
            The transform runtime was disabled after repeated faults. Reset it to try again.
          </p>
        )}
        </div>
      </div>
      <div>
        <div className="flex items-center justify-between gap-4">
          <div>
            <h2 className="text-sm font-medium text-on-surface">Saved transforms</h2>
            <p className="mt-0.5 text-xs leading-relaxed text-on-surface-variant">Create reusable spoken rewrite instructions.</p>
          </div>
          <button type="button" onClick={() => onOpenEditor('transforms')} className="rounded-lg bg-surface-container-high px-3 py-2 text-xs font-semibold text-on-surface hover:text-primary">Manage</button>
        </div>
      </div>
    </SettingsSection>
  );
}
