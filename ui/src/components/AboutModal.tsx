interface AboutModalProps {
  isOpen: boolean;
  onClose: () => void;
}

export function AboutModal({ isOpen, onClose }: AboutModalProps) {
  if (!isOpen) return null;

  return (
    <>
      {/* Backdrop */}
      <div
        className="fixed inset-0 bg-stone-900/50 z-50"
        onClick={onClose}
      />

      {/* Modal */}
      <div className="fixed inset-0 flex items-center justify-center z-50 pointer-events-none">
        <div className="bg-white dark:bg-stone-800 rounded-2xl shadow-xl p-6 w-72 text-center pointer-events-auto">
          {/* Icon */}
          <div className="w-16 h-16 mx-auto mb-4 bg-stone-800 rounded-2xl flex items-center justify-center">
            <svg className="w-10 h-10 text-white" fill="currentColor" viewBox="0 0 24 24">
              <path d="M12 14c1.66 0 3-1.34 3-3V5c0-1.66-1.34-3-3-3S9 3.34 9 5v6c0 1.66 1.34 3 3 3z"/>
              <path d="M17 11c0 2.76-2.24 5-5 5s-5-2.24-5-5H5c0 3.53 2.61 6.43 6 6.92V21h2v-3.08c3.39-.49 6-3.39 6-6.92h-2z"/>
            </svg>
          </div>

          <h2 className="text-xl font-bold text-stone-900 dark:text-stone-100 mb-1">
            Local Dictation
          </h2>
          <p className="text-sm text-stone-500 dark:text-stone-400 mb-4">
            Version 0.1.0
          </p>

          <p className="text-sm text-stone-600 dark:text-stone-300 mb-4">
            Privacy-first voice-to-text powered by Whisper AI. All processing happens locally on your device.
          </p>

          <p className="text-xs text-stone-400 dark:text-stone-500">
            © 2024 Local Dictation
          </p>

          <button
            onClick={onClose}
            className="mt-4 px-4 py-2 bg-stone-100 dark:bg-stone-700 text-stone-700 dark:text-stone-300 rounded-lg hover:bg-stone-200 dark:hover:bg-stone-600 transition-colors"
          >
            Close
          </button>
        </div>
      </div>
    </>
  );
}
