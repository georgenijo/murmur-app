import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  Settings, ModelOption, HotkeyOption, RecordingMode,
  MODEL_OPTIONS, HOTKEY_OPTIONS, DOUBLE_TAP_KEY_OPTIONS, RECORDING_MODE_OPTIONS,
} from '../../lib/settings';
import type { DictationStatus } from '../../lib/types';

interface SettingsPanelProps {
  isOpen: boolean;
  onClose: () => void;
  settings: Settings;
  onUpdateSettings: (updates: Partial<Settings>) => void;
  status: DictationStatus;
}

export function SettingsPanel({ isOpen, onClose, settings, onUpdateSettings, status }: SettingsPanelProps) {
  const [accessibilityGranted, setAccessibilityGranted] = useState<boolean | null>(null);

  const checkAccessibility = async () => {
    try {
      const granted = await invoke<boolean>('check_accessibility_permission');
      setAccessibilityGranted(granted);
    } catch {
      setAccessibilityGranted(false);
    }
  };

  // Check when panel opens
  useEffect(() => {
    if (isOpen) checkAccessibility();
  }, [isOpen]);

  // Re-check when window regains focus
  useEffect(() => {
    const handleFocus = () => { if (isOpen) checkAccessibility(); };
    window.addEventListener('focus', handleFocus);
    return () => window.removeEventListener('focus', handleFocus);
  }, [isOpen]);

  const handleRequestPermission = () => invoke('request_accessibility_permission');

  const isDoubleTap = settings.recordingMode === 'double_tap';
  const keyOptions = isDoubleTap ? DOUBLE_TAP_KEY_OPTIONS : HOTKEY_OPTIONS;
  const keyLabel = isDoubleTap ? 'Double-Tap Key' : 'Recording Hotkey';
  const keyHelpText = isDoubleTap
    ? 'Double-tap to start recording, single tap to stop'
    : 'Press this combo to toggle recording on/off';
  const isRecording = status !== 'idle';

  return (
    <>
      {/* Backdrop */}
      <div
        className={`fixed inset-0 bg-black/50 transition-opacity z-40 ${
          isOpen ? 'opacity-100' : 'opacity-0 pointer-events-none'
        }`}
        onClick={onClose}
      />

      {/* Panel */}
      <div
        className={`fixed right-0 top-0 h-full w-80 bg-white dark:bg-gray-800 shadow-xl z-50 transform transition-transform duration-300 ease-in-out ${
          isOpen ? 'translate-x-0' : 'translate-x-full'
        }`}
      >
        {/* Header */}
        <div className="flex items-center justify-between p-4 border-b border-gray-200 dark:border-gray-700">
          <h2 className="text-lg font-semibold text-gray-900 dark:text-white">Settings</h2>
          <button
            onClick={onClose}
            className="p-1 rounded-lg hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors"
          >
            <svg className="w-5 h-5 text-gray-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Content */}
        <div className="p-4 space-y-6">
          {/* Model Selector */}
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
              Transcription Model
            </label>
            <select
              value={settings.model}
              onChange={(e) => onUpdateSettings({ model: e.target.value as ModelOption })}
              className="w-full px-3 py-2 rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-2 focus:ring-blue-500 focus:border-transparent"
            >
              {MODEL_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label} ({option.size})
                </option>
              ))}
            </select>
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
              Larger models are more accurate but slower
            </p>
          </div>

          {/* Recording Trigger */}
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
              Recording Trigger
            </label>
            <div className="flex gap-2">
              {RECORDING_MODE_OPTIONS.map((option) => (
                <button
                  key={option.value}
                  disabled={isRecording}
                  onClick={() => onUpdateSettings({ recordingMode: option.value as RecordingMode })}
                  className={`flex-1 px-3 py-2 rounded-lg text-sm font-medium border transition-colors ${
                    settings.recordingMode === option.value
                      ? 'bg-blue-500 text-white border-blue-500'
                      : 'bg-white dark:bg-gray-700 text-gray-700 dark:text-gray-300 border-gray-300 dark:border-gray-600 hover:bg-gray-50 dark:hover:bg-gray-600'
                  } ${isRecording ? 'opacity-50 cursor-not-allowed' : ''}`}
                >
                  {option.label}
                </button>
              ))}
            </div>
            {isRecording && (
              <p className="mt-1 text-xs text-amber-600 dark:text-amber-400">
                Stop recording before changing mode
              </p>
            )}
          </div>

          {/* Accessibility notice for double-tap mode */}
          {isDoubleTap && accessibilityGranted !== null && !accessibilityGranted && (
            <div className="flex items-center gap-2 px-3 py-2 text-xs bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-lg text-amber-700 dark:text-amber-400">
              <span className="w-2 h-2 rounded-full bg-amber-500 flex-shrink-0" />
              <span>Accessibility permission required for double-tap mode</span>
              <button
                onClick={handleRequestPermission}
                className="underline hover:no-underline ml-auto flex-shrink-0"
              >
                Grant
              </button>
            </div>
          )}

          {/* Hotkey / Double-Tap Key Selector */}
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
              {keyLabel}
            </label>
            <select
              value={settings.hotkey}
              onChange={(e) => onUpdateSettings({ hotkey: e.target.value as HotkeyOption })}
              disabled={isRecording}
              className={`w-full px-3 py-2 rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-2 focus:ring-blue-500 focus:border-transparent ${isRecording ? 'opacity-50 cursor-not-allowed' : ''}`}
            >
              {keyOptions.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
              {keyHelpText}
            </p>
          </div>

          {/* Auto-Paste Toggle */}
          <div>
            <div className="flex items-center justify-between">
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300">
                  Auto-Paste
                </label>
                <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
                  Automatically paste transcription (requires Accessibility permission)
                </p>
              </div>
              <button
                type="button"
                onClick={() => onUpdateSettings({ autoPaste: !settings.autoPaste })}
                className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 ${
                  settings.autoPaste ? 'bg-blue-500' : 'bg-gray-300 dark:bg-gray-600'
                }`}
              >
                <span
                  className={`inline-block h-4 w-4 transform rounded-full bg-white shadow transition-transform ${
                    settings.autoPaste ? 'translate-x-6' : 'translate-x-1'
                  }`}
                />
              </button>
            </div>
            {/* Permission Status - only show when auto-paste is ON */}
            {settings.autoPaste && accessibilityGranted !== null && (
              <div className={`mt-2 flex items-center gap-2 text-xs ${
                accessibilityGranted
                  ? 'text-green-600 dark:text-green-400'
                  : 'text-amber-600 dark:text-amber-400'
              }`}>
                <span className={`w-2 h-2 rounded-full ${
                  accessibilityGranted ? 'bg-green-500' : 'bg-amber-500'
                }`} />
                <span>
                  {accessibilityGranted
                    ? 'Accessibility permission granted'
                    : 'Accessibility permission required'}
                </span>
                {!accessibilityGranted && (
                  <button
                    onClick={handleRequestPermission}
                    className="underline hover:no-underline"
                  >
                    Grant
                  </button>
                )}
              </div>
            )}
          </div>

          {/* Model Info */}
          <div className="pt-4 border-t border-gray-200 dark:border-gray-700">
            <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
              Current Model
            </h3>
            <div className="text-sm text-gray-600 dark:text-gray-400">
              <p><strong>Model:</strong> {MODEL_OPTIONS.find(m => m.value === settings.model)?.label}</p>
              <p><strong>Size:</strong> {MODEL_OPTIONS.find(m => m.value === settings.model)?.size}</p>
            </div>
          </div>
        </div>
      </div>
    </>
  );
}
