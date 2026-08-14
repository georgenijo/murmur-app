import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  parseAudioInputInventory,
  type AudioInputInventoryV1,
} from '../audioDevices';

export interface AudioInputInventoryState {
  inventory: AudioInputInventoryV1 | null;
  loading: boolean;
  error: string | null;
}

const INVALID_MESSAGE = 'The microphone list returned an unsupported response.';
const UNAVAILABLE_MESSAGE = 'The microphone list is temporarily unavailable.';

export function useAudioInputInventory(enabled: boolean): AudioInputInventoryState {
  const [state, setState] = useState<AudioInputInventoryState>({
    inventory: null,
    loading: enabled,
    error: null,
  });
  const generationRef = useRef(0);
  const revisionRef = useRef(-1);

  useEffect(() => {
    const generation = ++generationRef.current;
    if (!enabled) {
      revisionRef.current = -1;
      setState({ inventory: null, loading: false, error: null });
      return;
    }

    let disposed = false;
    let unlisten: (() => void) | null = null;
    let acceptedEvent = false;
    const owns = () => !disposed && generationRef.current === generation;
    const accept = (payload: unknown, source: 'command' | 'event') => {
      if (!owns()) return;
      const inventory = parseAudioInputInventory(payload);
      if (!inventory) {
        if (source === 'command' && acceptedEvent) return;
        setState((current) => ({ ...current, loading: false, error: INVALID_MESSAGE }));
        return;
      }
      if (inventory.revision < revisionRef.current) return;
      if (source === 'event') acceptedEvent = true;
      revisionRef.current = inventory.revision;
      setState({
        inventory,
        loading: false,
        error: inventory.status !== 'available'
          ? UNAVAILABLE_MESSAGE
          : null,
      });
    };

    setState((current) => ({ ...current, loading: true, error: null }));
    listen<unknown>('audio-input-inventory-changed', (event) => accept(event.payload, 'event'))
      .then((stop) => {
        if (!owns()) {
          stop();
          return;
        }
        unlisten = stop;
        // The listener must be installed before requesting the snapshot. This
        // closes the event-before-registration gap between revisions N/N+1.
        invoke<unknown>('get_audio_input_inventory')
          .then((payload) => accept(payload, 'command'))
          .catch(() => {
            if (owns() && !acceptedEvent) {
              setState((current) => ({ ...current, loading: false, error: UNAVAILABLE_MESSAGE }));
            }
          });
      })
      .catch(() => {
        if (owns()) {
          setState((current) => ({ ...current, loading: false, error: UNAVAILABLE_MESSAGE }));
        }
      });

    return () => {
      disposed = true;
      generationRef.current += 1;
      unlisten?.();
    };
  }, [enabled]);

  return state;
}
