import { Settings, ModelOption, HotkeyOption, MODEL_OPTIONS, HOTKEY_OPTIONS } from '../../lib/settings';

interface SettingsPanelProps {
  isOpen: boolean;
  onClose: () => void;
  settings: Settings;
  onUpdateSettings: (updates: Partial<Settings>) => void;
}

export function SettingsPanel({ isOpen, onClose, settings, onUpdateSettings }: SettingsPanelProps) {
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

          {/* Hotkey Selector */}
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
              Recording Hotkey
            </label>
            <select
              value={settings.hotkey}
              onChange={(e) => onUpdateSettings({ hotkey: e.target.value as HotkeyOption })}
              className="w-full px-3 py-2 rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-2 focus:ring-blue-500 focus:border-transparent"
            >
              {HOTKEY_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
              Hold this key to record, release to transcribe
            </p>
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
