import { useEffect, useRef } from 'react';
import Markdown from 'react-markdown';
import rehypeSanitize from 'rehype-sanitize';
import { motion, useReducedMotion } from 'motion/react';
import type { CompletedUpdate } from '../lib/updater';

interface WhatsNewModalProps {
  update: CompletedUpdate | null;
  onDismiss: () => void;
}

export function WhatsNewModal({ update, onDismiss }: WhatsNewModalProps) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const doneRef = useRef<HTMLButtonElement>(null);
  const onDismissRef = useRef(onDismiss);
  onDismissRef.current = onDismiss;
  const shouldReduceMotion = useReducedMotion();

  useEffect(() => {
    if (!update) return;

    const previouslyFocused =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        onDismissRef.current();
      } else if (event.key === 'Tab') {
        const focusable = Array.from(
          dialogRef.current?.querySelectorAll<HTMLElement>(
            'a[href], button:not(:disabled), [tabindex]:not([tabindex="-1"])',
          ) ?? [],
        );
        if (focusable.length === 0) return;

        event.preventDefault();
        const currentIndex = focusable.indexOf(document.activeElement as HTMLElement);
        const nextIndex = event.shiftKey
          ? (currentIndex <= 0 ? focusable.length - 1 : currentIndex - 1)
          : (currentIndex === focusable.length - 1 ? 0 : currentIndex + 1);
        focusable[nextIndex]?.focus();
      }
    };
    document.addEventListener('keydown', onKeyDown);
    const focusTimer = setTimeout(() => doneRef.current?.focus(), 60);

    return () => {
      document.removeEventListener('keydown', onKeyDown);
      clearTimeout(focusTimer);
      previouslyFocused?.focus();
    };
  }, [update]);

  if (!update) return null;

  return (
    <div
      className="dialog-backdrop fixed inset-0 z-50 flex items-center justify-center p-5 backdrop-blur-[2px]"
      onClick={(event) => {
        if (event.target === event.currentTarget) onDismiss();
      }}
    >
      <motion.div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="whats-new-title"
        aria-describedby="whats-new-description"
        initial={shouldReduceMotion ? false : { opacity: 0, scale: 0.98 }}
        animate={{ opacity: 1, scale: 1 }}
        transition={{ duration: shouldReduceMotion ? 0 : 0.16, ease: [0.23, 1, 0.32, 1] }}
        className="dialog-popover flex max-h-[82vh] w-full max-w-[520px] flex-col overflow-hidden"
      >
        <div className="relative shrink-0 overflow-hidden border-b border-[var(--ui-hairline)] px-6 pb-5 pt-6">
          <div className="absolute -right-12 -top-16 h-44 w-44 rounded-full bg-primary/10 blur-3xl" />
          <div className="relative">
            <div className="mb-4 flex items-start justify-between gap-4">
              <div className="flex h-12 w-12 items-center justify-center rounded-[var(--ui-radius-card)] bg-[linear-gradient(140deg,var(--murmur-primary),var(--murmur-primary-dim))] text-on-primary shadow-[var(--ui-shadow-accent)]">
                <svg
                  className="h-6 w-6"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                  aria-hidden="true"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={1.8}
                    d="M12 3l1.4 4.1L17.5 8.5l-4.1 1.4L12 14l-1.4-4.1-4.1-1.4 4.1-1.4L12 3zm6 10 .9 2.6 2.6.9-2.6.9L18 20l-.9-2.6-2.6-.9 2.6-.9L18 13z"
                  />
                </svg>
              </div>
              <span className="rounded-full border border-success/25 bg-success/10 px-2.5 py-1 text-[11px] font-semibold text-success">
                Updated successfully
              </span>
            </div>

            <h2 id="whats-new-title" className="text-2xl font-semibold tracking-[var(--ui-track-title,-0.022em)] text-on-surface">
              What&apos;s new in Murmur {update.version}
            </h2>
            <p id="whats-new-description" className="mt-1.5 text-sm text-on-surface-variant">
              Here&apos;s what changed since your last version.
            </p>
          </div>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
          {update.notes.trim() ? (
            <div className="text-sm leading-6 text-on-surface-variant [&_a]:font-medium [&_a]:text-primary [&_a]:underline [&_a]:underline-offset-2 [&_code]:rounded [&_code]:bg-surface-container-high [&_code]:px-1 [&_h1]:mb-2 [&_h1]:mt-5 [&_h1]:text-base [&_h1]:font-semibold [&_h1]:text-on-surface [&_h1:first-child]:mt-0 [&_h2]:mb-2 [&_h2]:mt-5 [&_h2]:text-base [&_h2]:font-semibold [&_h2]:text-on-surface [&_h2:first-child]:mt-0 [&_h3]:mb-1.5 [&_h3]:mt-4 [&_h3]:text-sm [&_h3]:font-semibold [&_h3]:text-on-surface [&_li]:my-1 [&_ol]:my-2 [&_ol]:list-decimal [&_ol]:pl-5 [&_p]:my-2 [&_strong]:font-semibold [&_strong]:text-on-surface [&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-5">
              <Markdown rehypePlugins={[rehypeSanitize]}>{update.notes}</Markdown>
            </div>
          ) : (
            <p className="dialog-card px-4 py-3 text-sm text-on-surface-variant">
              Murmur is up to date with the latest features and fixes.
            </p>
          )}
        </div>

        <div className="shrink-0 border-t border-[var(--ui-hairline)] bg-[var(--ui-tint-sunken)] px-6 py-4">
          <button
            ref={doneRef}
            type="button"
            onClick={onDismiss}
            className="w-full rounded-[var(--ui-radius-pill)] bg-[linear-gradient(140deg,var(--murmur-primary),var(--murmur-primary-dim))] px-4 py-2.5 text-sm font-semibold text-on-primary shadow-[var(--ui-shadow-accent)] transition-colors hover:brightness-105 focus:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2"
          >
            Start using Murmur
          </button>
        </div>
      </motion.div>
    </div>
  );
}
