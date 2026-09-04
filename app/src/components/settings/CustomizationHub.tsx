import { useLayoutEffect, useRef } from 'react';

export type CustomizationDestination = 'text' | 'commands' | 'styles' | 'transforms';

const DESTINATIONS: ReadonlyArray<{
  id: CustomizationDestination;
  title: string;
  description: string;
}> = [
  {
    id: 'text',
    title: 'Text & Vocabulary',
    description: 'Tune cleanup, punctuation, preferred spellings, and local vocabulary.',
  },
  {
    id: 'commands',
    title: 'Voice Commands',
    description: 'Create exact spoken replacements and reusable snippets.',
  },
  {
    id: 'styles',
    title: 'Styles',
    description: 'Choose how Murmur writes and delivers text in each app.',
  },
  {
    id: 'transforms',
    title: 'Transforms',
    description: 'Configure on-device rewrites and saved instructions.',
  },
];

interface CustomizationHubProps {
  focusDestination: CustomizationDestination | null;
  onOpen: (destination: CustomizationDestination) => void;
}

export function CustomizationHub({ focusDestination, onOpen }: CustomizationHubProps) {
  const rowRefs = useRef<Partial<Record<CustomizationDestination, HTMLButtonElement>>>({});

  useLayoutEffect(() => {
    if (focusDestination) rowRefs.current[focusDestination]?.focus();
  }, [focusDestination]);

  return (
    <section aria-labelledby="customization-title">
      <p className="settings-eyebrow">
        Make it yours
      </p>
      <h1 id="customization-title" className="settings-page-title mt-2">
        Customize Murmur
      </h1>
      <p className="mt-1 max-w-2xl text-sm leading-relaxed text-on-surface-variant">
        Shape what Murmur writes, how spoken shortcuts behave, and how results adapt to your apps.
      </p>

      <ol
        aria-label="Customization destinations"
        className="settings-hub-list mt-6"
      >
        {DESTINATIONS.map((destination, index) => (
          <li key={destination.id} className="settings-hub-card">
            <button
              ref={(node) => {
                if (node) rowRefs.current[destination.id] = node;
                else delete rowRefs.current[destination.id];
              }}
              type="button"
              onClick={() => onOpen(destination.id)}
              className="group flex min-h-[76px] w-full items-center gap-4 px-4 py-3 text-left transition-colors hover:bg-surface-container-low focus:outline-none focus-visible:bg-surface-container-low focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary"
            >
              <span className="w-6 shrink-0 font-mono text-[11px] font-semibold tabular-nums text-on-surface-variant">
                {String(index + 1).padStart(2, '0')}
              </span>
              <span className="min-w-0 flex-1">
                <span className="block text-sm font-semibold text-on-surface">{destination.title}</span>
                <span className="mt-1 block text-xs leading-relaxed text-on-surface-variant">
                  {destination.description}
                </span>
              </span>
              <span aria-hidden="true" className="text-lg text-on-surface-variant transition-transform group-hover:translate-x-0.5 group-hover:text-on-surface">
                ›
              </span>
            </button>
          </li>
        ))}
      </ol>

      <p className="mt-4 text-xs leading-relaxed text-on-surface-variant">
        Everything is stored and processed on this Mac.
      </p>
    </section>
  );
}
