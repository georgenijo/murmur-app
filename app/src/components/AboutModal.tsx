import { useState, useEffect } from 'react';
import { getVersion } from '@tauri-apps/api/app';
import { motion, useReducedMotion } from 'motion/react';

interface AboutModalProps {
  isOpen: boolean;
  onClose: () => void;
}

export function AboutModal({ isOpen, onClose }: AboutModalProps) {
  const [version, setVersion] = useState<string>('...');
  const shouldReduceMotion = useReducedMotion();

  useEffect(() => {
    if (isOpen) {
      getVersion().then(setVersion).catch(() => setVersion('—'));
    }
  }, [isOpen]);

  if (!isOpen) return null;

  return (
    <>
      {/* Backdrop */}
      <div
        className="dialog-backdrop fixed inset-0 z-50"
        onClick={onClose}
      />

      {/* Modal */}
      <div className="fixed inset-0 flex items-center justify-center z-50 pointer-events-none">
        <motion.div
          initial={shouldReduceMotion ? false : { opacity: 0, scale: 0.98 }}
          animate={{ opacity: 1, scale: 1 }}
          transition={{ duration: shouldReduceMotion ? 0 : 0.16, ease: [0.23, 1, 0.32, 1] }}
          className="dialog-popover w-72 p-6 text-center pointer-events-auto"
        >
          {/* Icon */}
          <div className="mx-auto mb-4 flex h-16 w-16 items-center justify-center rounded-[var(--ui-radius-card)] bg-[linear-gradient(140deg,var(--murmur-primary),var(--murmur-primary-dim))] shadow-[var(--ui-shadow-accent)]">
            <svg className="w-10 h-10 text-on-primary" fill="currentColor" viewBox="0 0 24 24">
              <path d="M12 14c1.66 0 3-1.34 3-3V5c0-1.66-1.34-3-3-3S9 3.34 9 5v6c0 1.66 1.34 3 3 3z"/>
              <path d="M17 11c0 2.76-2.24 5-5 5s-5-2.24-5-5H5c0 3.53 2.61 6.43 6 6.92V21h2v-3.08c3.39-.49 6-3.39 6-6.92h-2z"/>
            </svg>
          </div>

          <h2 className="text-xl font-bold tracking-[var(--ui-track-title,-0.022em)] text-on-surface mb-1">
            Murmur
          </h2>
          <p className="text-sm text-on-surface-variant mb-4">
            Version {version}
          </p>

          <p className="text-sm text-on-surface mb-4">
            Privacy-first voice-to-text powered by Whisper AI. All processing happens locally on your device — audio never leaves this Mac.
          </p>

          <p className="text-xs text-on-surface-variant">
            © 2026 Murmur
          </p>

          <button
            onClick={onClose}
            className="dialog-pill-btn mt-4 px-4 py-2 text-on-surface hover:bg-surface-container focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
          >
            Close
          </button>
        </motion.div>
      </div>
    </>
  );
}
