import { useState, useEffect } from 'react';
import {
  getPerformanceResourceWindow,
  measuredValue,
  onPerformanceResourceSample,
  type ResourceSampleV1,
} from '../performance';

export interface ResourceReading {
  observed_at_ms: number;
  host_cpu_percent: number | null;
  process_cpu_percent: number | null;
  rss_mb: number | null;
  rust_heap_mb: number | null;
  ffi_heap_mb: number | null;
}

const MAX_READINGS = 60;

function toMb(bytes: number | null): number | null {
  return bytes === null ? null : Math.round(bytes / 1_048_576);
}

function toReading(sample: ResourceSampleV1): ResourceReading {
  return {
    observed_at_ms: sample.observedAtMs,
    host_cpu_percent: measuredValue(sample.host.cpuPercent),
    process_cpu_percent: measuredValue(sample.mainProcess.cpuPercent),
    rss_mb: toMb(measuredValue(sample.mainProcess.rssBytes)),
    rust_heap_mb: toMb(measuredValue(sample.mainProcess.rustHeapBytes)),
    ffi_heap_mb: toMb(measuredValue(sample.mainProcess.ffiNativeHeapBytes)),
  };
}

export function useResourceMonitor(enabled: boolean): ResourceReading[] {
  const [readings, setReadings] = useState<ResourceReading[]>([]);

  useEffect(() => {
    if (!enabled) return;

    let cancelled = false;
    const append = (sample: ResourceSampleV1) => {
      if (cancelled) return;
      const reading = toReading(sample);
      setReadings(prev => {
        const withoutDuplicate = prev.filter(
          existing => existing.observed_at_ms !== reading.observed_at_ms,
        );
        const next = [...withoutDuplicate, reading]
          .sort((left, right) => left.observed_at_ms - right.observed_at_ms);
        return next.length > MAX_READINGS ? next.slice(-MAX_READINGS) : next;
      });
    };
    const hydrate = async () => {
      try {
        const window = await getPerformanceResourceWindow();
        if (!cancelled) setReadings(window.slice(-MAX_READINGS).map(toReading));
      } catch (e) {
        if (import.meta.env.DEV) console.debug('[useResourceMonitor]', e);
      }
    };

    void hydrate();
    const unlisten = onPerformanceResourceSample(append);
    return () => {
      cancelled = true;
      void unlisten.then(stop => stop());
    };
  }, [enabled]);

  return readings;
}
