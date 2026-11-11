import { useLayoutEffect, useRef } from 'react';
import { SettingsGroup } from './SettingsLayout';

export type CustomizationDestination = 'text' | 'commands' | 'styles' | 'modes';

const DESTINATIONS: ReadonlyArray<{
  id: CustomizationDestination;
  title: string;
  description: string;
}> = [
  {
    id: 'text',
    title: 'Text & vocabulary',
    description: 'Tune cleanup, punctuation, preferred spellings, and local vocabulary',
  },
  {
    id: 'commands',
    title: 'Voice commands',
    description: 'Create exact spoken replacements and reusable snippets',
  },
  {
    id: 'styles',
    title: 'App overrides',
    description: 'Choose how Murmur writes and delivers text in each app',
  },
  {
    id: 'modes',
    title: 'Modes',
    description: 'Build reusable behavior and assign it to apps or browser sites',
  },
];

interface CustomizationHubProps {
  focusDestination: CustomizationDestination | null;
  onOpen: (destination: CustomizationDestination, opener: HTMLButtonElement) => void;
}

export function CustomizationHub({ focusDestination, onOpen }: CustomizationHubProps) {
  const rowRefs = useRef<Partial<Record<CustomizationDestination, HTMLButtonElement>>>({});

  useLayoutEffect(() => {
    if (focusDestination) rowRefs.current[focusDestination]?.focus();
  }, [focusDestination]);

  return (
    <section aria-label="Customize Murmur">
      <SettingsGroup>
        <ul aria-label="Customization destinations" className="settings-destination-list m-0 list-none p-0">
          {DESTINATIONS.map((destination) => (
            <li key={destination.id}>
              <button
                ref={(node) => {
                  if (node) rowRefs.current[destination.id] = node;
                  else delete rowRefs.current[destination.id];
                }}
                type="button"
                onClick={(event) => onOpen(destination.id, event.currentTarget)}
                className="settings-setting-row flex w-full items-center justify-between gap-6 rounded-lg text-left transition-colors hover:bg-surface-container-low focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
              >
                <span className="settings-title-description">
                  <span className="block text-[length:var(--ui-font-label)] font-medium text-on-surface">{destination.title}</span>
                  <span className="mt-0.5 block text-[length:var(--ui-font-caption)] leading-relaxed text-on-surface-variant">
                    {destination.description}
                  </span>
                </span>
                <span aria-hidden="true" className="text-on-surface-variant">›</span>
              </button>
            </li>
          ))}
        </ul>
      </SettingsGroup>
    </section>
  );
}
