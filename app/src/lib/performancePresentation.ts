import type { CorrelationFilter } from './eventFilters';
import type {
  MeasurementV1,
  PerformanceRunKindV1,
  PerformanceRunV1,
  PerformanceStageV1,
  RunOutcomeV1,
  RuntimeIdentityV1,
  StageTimingV1,
  UnavailableReasonV1,
} from './performance';

const KIND_LABELS: Record<PerformanceRunKindV1, string> = {
  dictation: 'Dictation',
  fileTranscription: 'File transcription',
  selectedTextTransform: 'Selected-text transform',
};

const STAGE_LABELS: Record<PerformanceStageV1, string> = {
  captureFinalization: 'Capture finalization',
  fileDecode: 'File decode',
  vad: 'Voice activity detection',
  modelQueue: 'Model queue',
  modelLoad: 'Model load',
  inferenceDecode: 'Inference / decode',
  transcriptTransform: 'Transcript transforms',
  cleanup: 'Cleanup',
  voiceCommands: 'Voice Commands',
  smartCorrection: 'Smart Correction',
  smartFormatting: 'Smart Formatting',
  ideContext: 'IDE context',
  cliCommand: 'CLI command formatting',
  fileOutput: 'File output',
  clipboardPaste: 'Clipboard / paste',
  fileReturn: 'File return',
  totalProcessing: 'Total processing',
  selectedTextCapture: 'Selected-text capture',
  instructionCapture: 'Instruction capture',
  instructionAsr: 'Instruction ASR',
  sidecarSpawnLoad: 'Sidecar spawn / load',
  generation: 'Generation',
  reviewReady: 'Review ready',
  apply: 'Apply',
  undo: 'Undo',
};

const STAGES_BY_KIND: Record<PerformanceRunKindV1, readonly PerformanceStageV1[]> = {
  dictation: [
    'captureFinalization',
    'vad',
    'modelQueue',
    'modelLoad',
    'inferenceDecode',
    'transcriptTransform',
    'cleanup',
    'voiceCommands',
    'smartCorrection',
    'smartFormatting',
    'ideContext',
    'cliCommand',
    'fileOutput',
    'clipboardPaste',
    'totalProcessing',
  ],
  fileTranscription: [
    'fileDecode',
    'vad',
    'modelQueue',
    'modelLoad',
    'inferenceDecode',
    'transcriptTransform',
    'cleanup',
    'voiceCommands',
    'smartCorrection',
    'smartFormatting',
    'ideContext',
    'cliCommand',
    'fileReturn',
    'totalProcessing',
  ],
  selectedTextTransform: [
    'selectedTextCapture',
    'instructionCapture',
    'instructionAsr',
    'sidecarSpawnLoad',
    'generation',
    'reviewReady',
  ],
};

const UNAVAILABLE_LABELS: Record<UnavailableReasonV1, string> = {
  unsupportedPlatform: 'Unsupported on this platform',
  sampleFailed: 'Sample failed',
  noSamples: 'No samples',
  dependencyPending: 'Unavailable',
};

export interface FormattedMeasurement {
  text: string;
  status: MeasurementV1<unknown>['status'];
  detail?: string;
}

export interface RateDisplay extends FormattedMeasurement {
  label: 'Real-time factor' | 'Throughput';
}

export function kindLabel(kind: PerformanceRunKindV1): string {
  return KIND_LABELS[kind];
}

export function stageLabel(stage: PerformanceStageV1): string {
  return STAGE_LABELS[stage];
}

export function formatMilliseconds(value: number): string {
  if (value < 1_000) return `${Math.round(value)} ms`;
  if (value < 10_000) return `${(value / 1_000).toFixed(2)} s`;
  return `${(value / 1_000).toFixed(1)} s`;
}

export function formatBytes(value: number): string {
  const mib = value / 1_048_576;
  return mib < 10 ? `${mib.toFixed(1)} MiB` : `${Math.round(mib)} MiB`;
}

export function formatPercent(value: number): string {
  return `${value < 10 ? value.toFixed(1) : Math.round(value)}%`;
}

export function formatMeasurement<T>(
  measurement: MeasurementV1<T>,
  format: (value: T) => string,
): FormattedMeasurement {
  if (measurement.status === 'measured') {
    return { text: format(measurement.value), status: 'measured' };
  }
  if (measurement.status === 'notApplicable') {
    return { text: 'Not applicable', status: 'notApplicable' };
  }
  return {
    text: 'Unavailable',
    status: 'unavailable',
    detail: UNAVAILABLE_LABELS[measurement.reason],
  };
}

export function formatTimestamp(timestampMs: number): string {
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
    second: '2-digit',
  }).format(new Date(timestampMs));
}

export function runLatencyMs(run: PerformanceRunV1): number {
  return Math.max(0, run.finishedAtMs - run.startedAtMs);
}

export function runOutcomeLabel(outcome: RunOutcomeV1): string {
  switch (outcome.status) {
    case 'success': return 'Success';
    case 'noSpeech': return 'No speech';
    case 'cancelled': return 'Cancelled';
    case 'timedOut': return 'Timed out';
    case 'failed': return 'Failed';
    case 'interrupted': return 'Interrupted';
  }
}

export function runOutcomeDetail(outcome: RunOutcomeV1): string | null {
  if (outcome.status === 'success' || outcome.status === 'noSpeech') return null;
  if (outcome.status === 'cancelled' || outcome.status === 'timedOut') {
    return stageLabel(outcome.stage);
  }
  return `${stageLabel(outcome.stage)} · ${outcome.errorCode}`;
}

export function orderedStages(run: PerformanceRunV1): StageTimingV1[] {
  const byName = new Map(run.stages.map(stage => [stage.stage, stage]));
  return STAGES_BY_KIND[run.kind].flatMap(stage => {
    const timing = byName.get(stage);
    return timing ? [timing] : [];
  });
}

export function totalProcessingMeasurement(run: PerformanceRunV1): MeasurementV1<number> {
  return run.stages.find(stage => stage.stage === 'totalProcessing')?.durationMs
    ?? { status: 'unavailable', reason: 'noSamples' };
}

function unavailableRate(
  label: RateDisplay['label'],
  measurements: Array<MeasurementV1<unknown>>,
): RateDisplay {
  const unavailable = measurements.find(measurement => measurement.status === 'unavailable');
  if (unavailable?.status === 'unavailable') {
    return {
      label,
      text: 'Unavailable',
      status: 'unavailable',
      detail: UNAVAILABLE_LABELS[unavailable.reason],
    };
  }
  return { label, text: 'Not applicable', status: 'notApplicable' };
}

export function rateForRun(run: PerformanceRunV1): RateDisplay {
  if (run.kind === 'selectedTextTransform') {
    const generation = run.stages.find(stage => stage.stage === 'generation')?.durationMs
      ?? { status: 'unavailable', reason: 'noSamples' } as const;
    const tokens = run.input.outputTokenCount;
    if (generation.status === 'measured'
      && tokens.status === 'measured'
      && generation.value > 0) {
      return {
        label: 'Throughput',
        text: `${(tokens.value / (generation.value / 1_000)).toFixed(1)} tok/s`,
        status: 'measured',
      };
    }
    return unavailableRate('Throughput', [generation, tokens]);
  }

  const total = totalProcessingMeasurement(run);
  const audio = run.input.audioDurationMs;
  if (total.status === 'measured' && audio.status === 'measured' && audio.value > 0) {
    return {
      label: 'Real-time factor',
      text: `${(total.value / audio.value).toFixed(2)}×`,
      status: 'measured',
    };
  }
  return unavailableRate('Real-time factor', [total, audio]);
}

export function inputSummary(run: PerformanceRunV1): FormattedMeasurement {
  if (run.input.audioDurationMs.status === 'measured') {
    return formatMeasurement(run.input.audioDurationMs, formatMilliseconds);
  }
  if (run.input.inputSizeBucket.status === 'measured') {
    return formatMeasurement(run.input.inputSizeBucket, value => `${value} input`);
  }
  return formatMeasurement(run.input.audioDurationMs, formatMilliseconds);
}

export function runtimeSummary(run: PerformanceRunV1): string {
  if (run.runtimes.length === 0) return 'Unavailable';
  return run.runtimes
    .map(runtime => `${runtime.modelId} · ${backendLabel(runtime)} · ${acceleratorLabel(runtime)}`)
    .join(' / ');
}

export function backendLabel(runtime: RuntimeIdentityV1): string {
  const labels: Record<RuntimeIdentityV1['backend'], string> = {
    whisper: 'Whisper',
    parakeet: 'Parakeet',
    coreml: 'Core ML',
    llamaCpp: 'llama.cpp',
  };
  return labels[runtime.backend];
}

export function acceleratorLabel(runtime: RuntimeIdentityV1): string {
  const labels: Record<RuntimeIdentityV1['accelerator'], string> = {
    cpu: 'CPU',
    metalGpu: 'Metal GPU',
    appleNeuralEngine: 'Apple Neural Engine',
    platformFallback: 'Platform fallback',
  };
  return labels[runtime.accelerator];
}

export function correlationFilterForRun(run: PerformanceRunV1): CorrelationFilter {
  switch (run.correlation.kind) {
    case 'dictation':
      return { field: 'recording_id', value: String(run.correlation.recordingId) };
    case 'fileTranscription':
      return { field: 'file_run_id', value: String(run.correlation.fileRunId) };
    case 'selectedTextTransform':
      return { field: 'transform_pass_id', value: String(run.correlation.transformPassId) };
  }
}

export function correlationLabel(run: PerformanceRunV1): string {
  const filter = correlationFilterForRun(run);
  const labels: Record<CorrelationFilter['field'], string> = {
    run_id: 'Run',
    recording_id: 'Recording',
    file_run_id: 'File run',
    transform_pass_id: 'Transform pass',
  };
  return `${labels[filter.field]} ${filter.value}`;
}

export function peakResourceSummary(run: PerformanceRunV1): string {
  const parts: string[] = [];
  const mainRss = formatMeasurement(run.resources.mainProcess.rssBytes.peak, formatBytes);
  const hostCpu = formatMeasurement(run.resources.host.cpuPercent.peak, formatPercent);
  const sidecarRss = formatMeasurement(run.resources.sidecarProcess.rssBytes.peak, formatBytes);
  if (mainRss.status === 'measured') parts.push(`Main ${mainRss.text}`);
  if (sidecarRss.status === 'measured') parts.push(`Sidecar ${sidecarRss.text}`);
  if (hostCpu.status === 'measured') parts.push(`Host ${hostCpu.text}`);
  return parts.join(' · ') || 'Unavailable';
}

export function resourceDelta(
  start: MeasurementV1<number>,
  end: MeasurementV1<number>,
): FormattedMeasurement {
  if (start.status === 'measured' && end.status === 'measured') {
    const delta = end.value - start.value;
    return {
      text: `${delta >= 0 ? '+' : '−'}${formatBytes(Math.abs(delta))}`,
      status: 'measured',
    };
  }
  return unavailableRate('Throughput', [start, end]);
}
