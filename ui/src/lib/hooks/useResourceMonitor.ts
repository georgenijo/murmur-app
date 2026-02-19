import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

export interface ResourceReading {
  cpu_percent: number;
  memory_mb: number;
}

const MAX_READINGS = 60;

export function useResourceMonitor(enabled: boolean): ResourceReading[] {
  const [readings, setReadings] = useState<ResourceReading[]>([]);

  useEffect(() => {
    if (!enabled) return;

    const fetchUsage = async () => {
      try {
        const usage = await invoke<ResourceReading>('get_resource_usage');
        setReadings(prev => {
          const next = [...prev, usage];
          return next.length > MAX_READINGS ? next.slice(-MAX_READINGS) : next;
        });
      } catch {
        // ignore errors silently
      }
    };

    fetchUsage();
    const id = setInterval(fetchUsage, 1000);
    return () => clearInterval(id);
  }, [enabled]);

  return readings;
}
