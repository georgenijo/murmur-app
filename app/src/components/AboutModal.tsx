import { MurmurMark } from './onboarding/StepHeading';
import { defaultButtonKeyDown } from '../lib/defaultButton';
import { useState, useEffect } from 'react';
import { getVersion } from '@tauri-apps/api/app';
import { motion, useReducedMotion } from 'motion/react';
import { useDialogFocus } from '../lib/hooks/useDialogFocus';

interface AboutModalProps {
  isOpen: boolean;
  onClose: () => void;
}

export function AboutModal({ isOpen, onClose }: AboutModalProps) {
  return isOpen ? <AboutDialog onClose={onClose} /> : null;
}

function AboutDialog({ onClose }: Pick<AboutModalProps, 'onClose'>) {
  const dialogRef = useDialogFocus(onClose);
  const [version, setVersion] = useState<string>('…');
  const shouldReduceMotion = useReducedMotion();

  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion('—'));
  }, []);

  return (
    <>
      {/* Backdrop */}
      <div
        className="dialog-backdrop about-backdrop fixed inset-0 z-50"
        onClick={onClose}
      />

      {/* Modal */}
      <div className="fixed inset-0 flex items-center justify-center z-50 pointer-events-none">
        <motion.div ref={dialogRef} tabIndex={-1} role="dialog" aria-modal="true" aria-label="About Murmur" onKeyDown={defaultButtonKeyDown}
          initial={shouldReduceMotion ? false : { opacity: 0, scale: 0.98 }}
          animate={{ opacity: 1, scale: 1 }}
          transition={{ duration: shouldReduceMotion ? 0 : 0.16, ease: [0.23, 1, 0.32, 1] }}
          className="dialog-popover about-dialog w-72 text-center pointer-events-auto"
        >
          {/* Icon */}
          <div className="about-mark"><MurmurMark /></div>

          <h2 className="font-semibold tracking-[var(--ui-track-title,-0.01em)] text-on-surface mb-1">
            Murmur
          </h2>
          <p className="text-on-surface-variant mb-4">
            Version {version}
          </p>

          <p className="text-on-surface mb-4">
            Private dictation and meeting transcription. Audio and transcripts stay on this Mac.
          </p>

          <p className="text-on-surface-variant">
            © 2026 Murmur
          </p>

          <button
            onClick={onClose} data-default-button
            className="dialog-pill-btn mt-4 px-4 py-2 text-on-surface hover:bg-surface-container focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
          >
            Close
          </button>
        </motion.div>
      </div>
    </>
  );
}
