import { describe, expect, it } from 'vitest';
import { makeRun } from '../components/log-viewer/testFixtures';
import {
  isMeasurementV1,
  isPerformanceRunV1,
  isPerformanceStoreHealthV1,
  isResourceSampleV1,
  measuredValue,
  PERFORMANCE_STAGES_V1,
  type DeliveryTimingV1,
  type MeasurementV1,
} from './performance';

describe('performance contracts', () => {
  it('accepts historical v1 runs without final inference queue but rejects other missing or duplicate stages', () => {
    const run = makeRun();
    expect(isPerformanceRunV1(run)).toBe(true);
    const legacy = { ...run, stages: run.stages.filter(timing => timing.stage !== 'inferenceQueue') };
    expect(isPerformanceRunV1(legacy)).toBe(true);
    expect(isPerformanceRunV1({ ...legacy, stages: legacy.stages.filter(timing => timing.stage !== 'modelQueue') })).toBe(false);
    expect(isPerformanceRunV1({ ...run, stages: [...run.stages, run.stages[0]] })).toBe(false);
  });

  it('accepts homophone fallback timing and historical runs without that measurement', () => {
    const run = makeRun();
    run.stages = run.stages.map(timing => timing.stage === 'homophoneContext'
      ? { ...timing, durationMs: { status: 'measured', value: 151 }, outcome: 'fallback' }
      : timing);
    expect(isPerformanceRunV1(run)).toBe(true);
    expect(isPerformanceRunV1({ ...run, stages: run.stages.filter(timing => timing.stage !== 'homophoneContext') })).toBe(true);
  });

  it('accepts historical v1 runs without release timing stages, but not other gaps', () => {
    const run = makeRun();
    const additive = new Set(['inferenceQueue', 'releaseToStop', 'releaseToPaste']);
    const legacy = { ...run, stages: run.stages.filter(timing => !additive.has(timing.stage)) };
    expect(isPerformanceRunV1(legacy)).toBe(true);
    expect(isPerformanceRunV1({
      ...legacy,
      stages: legacy.stages.filter(timing => timing.stage !== 'totalProcessing'),
    })).toBe(false);
  });

  it('accepts only the bounded, content-free diagnostics-health schema', () => {
    const health = {
      schemaVersion: 1,
      status: 'available',
      skippedRunCount: 2,
      recommendedAction: 'retry',
      lastFailure: {
        operation: 'begin',
        errorClass: 'busyLocked',
        attemptCount: 3,
        retryExhausted: true,
        atMs: 1_786_720_000_000,
        recordingId: 35,
      },
      lastRecovery: {
        action: 'quarantinedAndReinitialized',
        atMs: 1_786_719_000_000,
      },
    };

    expect(isPerformanceStoreHealthV1(health)).toBe(true);
    expect(isPerformanceStoreHealthV1({
      schemaVersion: 1,
      status: 'unavailable',
      skippedRunCount: 0,
      recommendedAction: 'reinitializeStore',
    })).toBe(true);
    expect(isPerformanceStoreHealthV1({ ...health, schemaVersion: 2 })).toBe(false);
    expect(isPerformanceStoreHealthV1({ ...health, status: 'disabled' })).toBe(false);
    expect(isPerformanceStoreHealthV1({ ...health, skippedRunCount: -1 })).toBe(false);
    expect(isPerformanceStoreHealthV1({
      ...health,
      lastFailure: { ...health.lastFailure, attemptCount: 0, retryExhausted: false },
    })).toBe(false);
    expect(isPerformanceStoreHealthV1({
      ...health,
      lastFailure: { ...health.lastFailure, attemptCount: 4, retryExhausted: false },
    })).toBe(false);
    expect(isPerformanceStoreHealthV1({
      ...health,
      lastFailure: { ...health.lastFailure, attemptCount: 2, retryExhausted: true },
    })).toBe(false);
    expect(isPerformanceStoreHealthV1({
      ...health,
      lastFailure: { ...health.lastFailure, errorClass: 'io', retryExhausted: true },
    })).toBe(false);
    expect(isPerformanceStoreHealthV1({
      ...health,
      lastFailure: { ...health.lastFailure, retryExhausted: false },
    })).toBe(false);
    expect(isPerformanceStoreHealthV1({
      ...health,
      lastFailure: {
        ...health.lastFailure,
        errorClass: 'io',
        attemptCount: 1,
        retryExhausted: false,
      },
    })).toBe(true);
    expect(isPerformanceStoreHealthV1({
      ...health,
      lastFailure: { ...health.lastFailure, errorClass: 'rawSqliteError' },
    })).toBe(false);
    expect(isPerformanceStoreHealthV1({
      ...health,
      lastFailure: { ...health.lastFailure, path: '/private/data' },
    })).toBe(false);
    expect(isPerformanceStoreHealthV1({ ...health, message: 'database is locked' })).toBe(false);
  });

  it('keeps measured zero distinct from unavailable and not-applicable', () => {
    const measured: MeasurementV1<number> = { status: 'measured', value: 0 };
    expect(isMeasurementV1(measured, (value): value is number => typeof value === 'number')).toBe(true);
    expect(measuredValue(measured)).toBe(0);
    expect(measuredValue({ status: 'notApplicable' })).toBeNull();
    expect(measuredValue({ status: 'unavailable', reason: 'noSamples' })).toBeNull();
  });

  it('rejects unknown resource schema versions and zero sentinels', () => {
    const sample = {
      schemaVersion: 1,
      observedAtMs: 1,
      host: { cpuPercent: { status: 'unavailable', reason: 'sampleFailed' } },
      mainProcess: {
        cpuPercent: { status: 'measured', value: 0 },
        rssBytes: { status: 'measured', value: 0 },
        rustHeapBytes: { status: 'notApplicable' },
        ffiNativeHeapBytes: { status: 'unavailable', reason: 'unsupportedPlatform' },
      },
      sidecarProcess: {
        cpuPercent: { status: 'unavailable', reason: 'dependencyPending' },
        rssBytes: { status: 'unavailable', reason: 'dependencyPending' },
      },
    };
    expect(isResourceSampleV1(sample)).toBe(true);
    expect(isResourceSampleV1({ ...sample, schemaVersion: 2 })).toBe(false);
    expect(isResourceSampleV1({
      ...sample,
      host: { cpuPercent: 0 },
    })).toBe(false);
    expect(isResourceSampleV1({ ...sample, question: 'private' })).toBe(false);
    expect(isResourceSampleV1({
      ...sample,
      host: { ...sample.host, context: 'private' },
    })).toBe(false);
    expect(isResourceSampleV1({
      ...sample,
      mainProcess: {
        ...sample.mainProcess,
        rssBytes: { status: 'measured', value: 0, detail: 'private' },
      },
    })).toBe(false);
  });

  it('requires correlation to match the run kind', () => {
    const unavailable = { status: 'unavailable', reason: 'noSamples' };
    const notApplicable = { status: 'notApplicable' };
    const range = {
      start: unavailable,
      average: unavailable,
      peak: unavailable,
      end: unavailable,
    };
    const base = {
      schemaVersion: 1,
      runId: '0123456789abcdef0123456789abcdef',
      kind: 'dictation',
      startedAtMs: 1,
      finishedAtMs: 2,
      appVersion: '1.0.0',
      correlation: { kind: 'dictation', recordingId: 4 },
      outcome: { status: 'success' },
      runtimes: [],
      stages: PERFORMANCE_STAGES_V1.map(stage => ({
        stage,
        durationMs: notApplicable,
        outcome: 'skipped',
      })),
      input: {
        audioDurationMs: { status: 'measured', value: 100 },
        inputSizeBucket: notApplicable,
        outputSizeBucket: { status: 'measured', value: 'small' },
        outputTokenCount: notApplicable,
      },
      resources: {
        sampleCount: 0,
        host: { cpuPercent: range },
        mainProcess: {
          cpuPercent: range,
          rssBytes: range,
          rustHeapBytes: range,
          ffiNativeHeapBytes: range,
        },
        sidecarProcess: {
          cpuPercent: {
            start: notApplicable,
            average: notApplicable,
            peak: notApplicable,
            end: notApplicable,
          },
          rssBytes: {
            start: notApplicable,
            average: notApplicable,
            peak: notApplicable,
            end: notApplicable,
          },
        },
      },
      followUps: [],
    };
    expect(isPerformanceRunV1(base)).toBe(true);
    expect(isPerformanceRunV1({
      ...base,
      correlation: { kind: 'fileTranscription', fileRunId: 4 },
    })).toBe(false);

    const production = {
      schemaVersion: 1,
      cohort: {
        machineId: '123e4567-e89b-12d3-a456-426614174000',
        osVersion: '15.6.1',
        architecture: 'aarch64',
        buildMode: 'release',
        microphoneSelection: 'systemDefault',
        microphoneKind: 'builtIn',
        configurationKey: 'a'.repeat(64),
      },
      capture: {
        helperResolveMs: 0,
        helperSignatureMs: 2,
        helperSpawnMs: 3,
        streamOpenMs: 40,
        firstCallbackWaitMs: 80,
        firstPcmMs: 120,
        readyMs: 150,
        stopToWorkerExitMs: null,
        backend: 'auhal',
        fallbackAttempted: false,
        fallbackSucceeded: null,
        failureKind: null,
        workerInvariantViolation: false,
        zeroSampleSuccess: false,
      },
    };
    expect(isPerformanceRunV1({ ...base, production })).toBe(true);
    const gesture = {
      pressToAcceptedMs: { status: 'measured', value: 160 },
      doubleTapGapMs: { status: 'measured', value: 124 },
      pressToFirstPcmMs: { status: 'measured', value: 330 },
      acceptedToFirstPcmMs: { status: 'measured', value: 170 },
      pressToReadyMs: { status: 'unavailable', reason: 'noSamples' },
      acceptedToReadyMs: { status: 'notApplicable' },
    };
    const withGesture = (change: object) => ({ ...base, production: {
      ...production, capture: { ...production.capture, gesture: { ...gesture, ...change } },
    } });
    expect(isPerformanceRunV1(withGesture({}))).toBe(true);
    expect(isPerformanceRunV1(withGesture({ pressToAcceptedMs: { status: 'measured', value: -1 } }))).toBe(false);
    expect(isPerformanceRunV1(withGesture({ pressToAcceptedMs: { status: 'measured', value: 0 } }))).toBe(true);
    expect(isPerformanceRunV1(withGesture({ transcript: 'reject unknown content' }))).toBe(false);

    const withCapture = (change: object) => ({ ...base, production: {
      ...production, capture: { ...production.capture, ...change },
    } });
    expect(isPerformanceRunV1(withCapture({
      deviceResolutionMs: 0,
      audioUnitNewMs: 116,
      audioUnitInitializeMs: 4,
      streamConfigureMs: 15,
      streamStartMs: 28,
      helperPreparation: 'adopted',
    }))).toBe(true);
    for (const outcome of ['notPrepared', 'stillPreparing', 'identityMismatch', 'expired']) {
      expect(isPerformanceRunV1(withCapture({ helperPreparation: outcome }))).toBe(true);
    }
    expect(isPerformanceRunV1(withCapture({ audioUnitNewMs: -1 }))).toBe(false);
    expect(isPerformanceRunV1(withCapture({ audioUnitInitializeMs: 0 }))).toBe(true);
    expect(isPerformanceRunV1(withCapture({ audioUnitInitializeMs: -1 }))).toBe(false);
    expect(isPerformanceRunV1(withCapture({ audioUnitInitializeMs: Number.MAX_SAFE_INTEGER + 1 }))).toBe(false);
    expect(isPerformanceRunV1(withCapture({ streamStartMs: 1.5 }))).toBe(false);
    expect(isPerformanceRunV1(withCapture({ helperPreparation: 'private' }))).toBe(false);
    expect(isPerformanceRunV1(withCapture({ streamConfigureMs: 3, stderr: 'private' }))).toBe(false);

    expect(isPerformanceRunV1({ ...base, production: null })).toBe(true);
    expect(isPerformanceRunV1({
      ...base,
      production: { ...production, schemaVersion: 2 },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...base,
      production: { ...production, transcript: 'private' },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...base,
      production: {
        ...production,
        cohort: { ...production.cohort, rawDeviceUid: 'private' },
      },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...base,
      production: {
        ...production,
        capture: { ...production.capture, helperResolveMs: -1 },
      },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...base,
      production: {
        ...production,
        capture: { ...production.capture, helperResolveMs: 1.5 },
      },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...base,
      production: {
        ...production,
        cohort: { ...production.cohort, osVersion: 'macOS 15.6' },
      },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...base,
      production: {
        ...production,
        cohort: { ...production.cohort, configurationKey: 'A'.repeat(64) },
      },
    })).toBe(false);

    const voiceQuery = {
      ...base,
      kind: 'voiceQuery',
      correlation: { kind: 'voiceQuery', queryPassId: 9 },
      queryProcess: { exitCode: 0, stderrPresent: false },
    };
    expect(isPerformanceRunV1(voiceQuery)).toBe(true);
    const { queryProcess: _legacyMissingProcess, ...legacyVoiceQuery } = voiceQuery;
    expect(isPerformanceRunV1(legacyVoiceQuery)).toBe(true);
    expect(isPerformanceRunV1({
      ...voiceQuery,
      correlation: { kind: 'selectedTextTransform', transformPassId: 9 },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...voiceQuery,
      queryProcess: { exitCode: 0, stderrPresent: true, stderr: 'secret' },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...voiceQuery,
      queryProcess: { exitCode: '0', stderrPresent: true },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...voiceQuery,
      kind: 'dictation',
      correlation: { kind: 'dictation', recordingId: 9 },
    })).toBe(false);
  });

  describe('delivery timing attribution', () => {
    const measured = (value: number): MeasurementV1<number> => ({ status: 'measured', value });
    const notApplicable: MeasurementV1<number> = { status: 'notApplicable' };
    const pending: MeasurementV1<number> = { status: 'unavailable', reason: 'dependencyPending' };
    const idleCallback = { queueWaitUs: notApplicable, ownerWaitUs: notApplicable, workUs: notApplicable };
    const idleVerification = { elapsedUs: notApplicable, lookupRetryCount: notApplicable };

    function normalPasteTiming(): DeliveryTimingV1 {
      return {
        schemaVersion: 1,
        acceptedRoute: 'normalPaste',
        deliveryRoute: 'normalPaste',
        outcome: 'autoPasted',
        outcomeUs: measured(48_210),
        totalUs: measured(48_900),
        configuredDelayUs: measured(0),
        prepareCallback: idleCallback,
        injectionCallback: {
          queueWaitUs: measured(1_240),
          ownerWaitUs: measured(35),
          workUs: measured(46_700),
        },
        inject: {
          focusedFieldQueryUs: measured(3_100),
          clipboardWriteUs: measured(900),
          beforeFocusVerification: { elapsedUs: measured(2_400), lookupRetryCount: measured(0) },
          afterFocusVerification: { elapsedUs: measured(2_300), lookupRetryCount: measured(0) },
          afterClipboardVerification: { elapsedUs: measured(2_500), lookupRetryCount: measured(1) },
          eventConstructionUs: measured(150),
          finalFocusProofUs: measured(2_200),
          keyEventUs: measured(3_300),
          nativePasteRetryCount: measured(0),
          nativePasteRetryWaitUs: measured(0),
        },
        windowDiagnosticUs: pending,
        smart: {
          prepareOutcome: 'notRun',
          activationWaitUs: notApplicable,
          prepareValidationUs: notApplicable,
          insertValidationUs: notApplicable,
          settleUs: notApplicable,
          readbackUs: notApplicable,
          restoreUs: notApplicable,
          cleanupUs: notApplicable,
          snapshotWindowLookupUs: notApplicable,
        },
      };
    }

    function smartTimingRefusal(): DeliveryTimingV1 {
      return {
        ...normalPasteTiming(),
        acceptedRoute: 'smartTarget',
        deliveryRoute: 'smartTarget',
        outcome: 'clipboardOnlySmartTargetUnavailable',
        prepareCallback: { queueWaitUs: measured(900), ownerWaitUs: measured(12), workUs: measured(8_000) },
        // The Smart route never runs the normal-route preflight verifications.
        inject: {
          ...normalPasteTiming().inject,
          focusedFieldQueryUs: notApplicable,
          beforeFocusVerification: idleVerification,
          afterFocusVerification: idleVerification,
          afterClipboardVerification: idleVerification,
        },
        windowDiagnosticUs: notApplicable,
        smart: {
          prepareOutcome: 'ready',
          activationWaitUs: measured(300_000),
          prepareValidationUs: measured(6_400),
          insertValidationUs: measured(5_100),
          settleUs: measured(0),
          readbackUs: notApplicable,
          restoreUs: notApplicable,
          cleanupUs: measured(1_800),
          snapshotWindowLookupUs: measured(2_900),
        },
      };
    }

    const withTiming = (deliveryTiming: unknown) => ({ ...makeRun(), deliveryTiming });
    // Replace one nested value and keep everything else valid.
    const changed = (change: (timing: Record<string, any>) => void) => {
      const timing = JSON.parse(JSON.stringify(normalPasteTiming()));
      change(timing);
      return withTiming(timing);
    };

    it('accepts the record on the normal and Smart routes, and runs without it', () => {
      expect(isPerformanceRunV1(makeRun())).toBe(true);
      expect(isPerformanceRunV1(withTiming(normalPasteTiming()))).toBe(true);
      expect(isPerformanceRunV1(withTiming(smartTimingRefusal()))).toBe(true);
      // Same tolerance as `production`: the backend omits the key, never nulls it.
      expect(isPerformanceRunV1(withTiming(null))).toBe(true);
    });

    it('keeps measured zero distinct from not-applicable and pending', () => {
      const zero = withTiming({ ...normalPasteTiming(), windowDiagnosticUs: measured(0) });
      expect(isPerformanceRunV1(zero)).toBe(true);
      expect(measuredValue(normalPasteTiming().windowDiagnosticUs)).toBeNull();
      expect(measuredValue(normalPasteTiming().smart.activationWaitUs)).toBeNull();
      expect(measuredValue(measured(0))).toBe(0);
      expect(isPerformanceRunV1(changed(timing => {
        timing.inject.keyEventUs = { status: 'unavailable', reason: 'sampleFailed' };
      }))).toBe(true);
    });

    it('accepts every bounded route, outcome and Smart preparation code', () => {
      for (const route of ['disabled', 'clipboardOnly', 'normalPaste', 'smartTarget']) {
        expect(isPerformanceRunV1(changed(timing => {
          timing.acceptedRoute = route;
          timing.deliveryRoute = route;
        }))).toBe(true);
      }
      for (const outcome of [
        'autoPasted', 'noText', 'clipboardOnlyAutoPasteDisabled',
        'clipboardOnlyAccessibilityDenied', 'clipboardOnlyTargetChanged',
        'clipboardOnlyFocusNotEditable', 'clipboardOnlyClipboardChanged',
        'clipboardOnlyPasteFailed', 'clipboardOnlySmartTargetUnavailable',
        'stoppedCancelled', 'stoppedExpired', 'stoppedAlreadyCommitted',
        'stoppedOperationInFlight', 'clipboardWriteFailed', 'cancelledBeforeInjection',
        'senderDropped', 'timedOut', 'dispatchFailed', 'incomplete',
      ]) {
        expect(isPerformanceRunV1(changed(timing => { timing.outcome = outcome; }))).toBe(true);
      }
      for (const prepareOutcome of [
        'notRun', 'ready', 'waitForActivation', 'refused', 'refusedClipboardOnly',
        'ownerNotCurrent', 'senderDropped', 'timedOut', 'dispatchFailed',
      ]) {
        expect(isPerformanceRunV1(changed(timing => {
          timing.smart.prepareOutcome = prepareOutcome;
        }))).toBe(true);
      }
    });

    it('rejects another schema version and values outside the bounded vocabularies', () => {
      expect(isPerformanceRunV1(changed(timing => { timing.schemaVersion = 2; }))).toBe(false);
      expect(isPerformanceRunV1(changed(timing => { timing.schemaVersion = '1'; }))).toBe(false);
      expect(isPerformanceRunV1(changed(timing => { timing.acceptedRoute = 'terminal'; }))).toBe(false);
      expect(isPerformanceRunV1(changed(timing => { timing.deliveryRoute = 'Safari'; }))).toBe(false);
      expect(isPerformanceRunV1(changed(timing => { timing.outcome = 'pasted into Mail'; }))).toBe(false);
      expect(isPerformanceRunV1(changed(timing => { timing.smart.prepareOutcome = 'activated'; }))).toBe(false);
    });

    it('rejects undeclared keys at every level, so content cannot ride along', () => {
      const secret = 'dictated words and window title';
      expect(isPerformanceRunV1(changed(timing => { timing.text = secret; }))).toBe(false);
      expect(isPerformanceRunV1(changed(timing => { timing.prepareCallback.windowTitle = secret; }))).toBe(false);
      expect(isPerformanceRunV1(changed(timing => { timing.injectionCallback.bundleId = secret; }))).toBe(false);
      expect(isPerformanceRunV1(changed(timing => { timing.inject.clipboard = secret; }))).toBe(false);
      expect(isPerformanceRunV1(changed(timing => {
        timing.inject.afterClipboardVerification.processName = secret;
      }))).toBe(false);
      expect(isPerformanceRunV1(changed(timing => { timing.smart.path = secret; }))).toBe(false);
      expect(isPerformanceRunV1(changed(timing => {
        timing.windowDiagnosticUs = { status: 'measured', value: 1, title: secret };
      }))).toBe(false);
    });

    it('rejects missing required keys instead of reading them as zero', () => {
      expect(isPerformanceRunV1(changed(timing => { delete timing.windowDiagnosticUs; }))).toBe(false);
      expect(isPerformanceRunV1(changed(timing => { delete timing.smart; }))).toBe(false);
      expect(isPerformanceRunV1(changed(timing => { delete timing.inject.keyEventUs; }))).toBe(false);
      expect(isPerformanceRunV1(changed(timing => {
        delete timing.inject.beforeFocusVerification.lookupRetryCount;
      }))).toBe(false);
      expect(isPerformanceRunV1(changed(timing => { delete timing.injectionCallback.ownerWaitUs; }))).toBe(false);
    });

    it('requires every duration and count to be a non-negative safe integer measurement', () => {
      for (const value of [-1, 1.5, Number.MAX_SAFE_INTEGER + 1, Number.NaN, '12']) {
        expect(isPerformanceRunV1(changed(timing => {
          timing.totalUs = { status: 'measured', value };
        }))).toBe(false);
        expect(isPerformanceRunV1(changed(timing => {
          timing.inject.nativePasteRetryCount = { status: 'measured', value };
        }))).toBe(false);
        expect(isPerformanceRunV1(changed(timing => {
          timing.inject.afterFocusVerification.lookupRetryCount = { status: 'measured', value };
        }))).toBe(false);
        expect(isPerformanceRunV1(changed(timing => {
          timing.smart.cleanupUs = { status: 'measured', value };
        }))).toBe(false);
      }
      expect(isPerformanceRunV1(changed(timing => {
        timing.totalUs = { status: 'measured', value: Number.MAX_SAFE_INTEGER };
      }))).toBe(true);
      // A bare number is not a measurement: a zero must say it was measured.
      expect(isPerformanceRunV1(changed(timing => { timing.outcomeUs = 0; }))).toBe(false);
      expect(isPerformanceRunV1(changed(timing => { timing.inject.clipboardWriteUs = null; }))).toBe(false);
      expect(isPerformanceRunV1(changed(timing => {
        timing.injectionCallback.workUs = { status: 'unavailable', reason: 'because' };
      }))).toBe(false);
      expect(isPerformanceRunV1(changed(timing => {
        timing.windowDiagnosticUs = { status: 'notApplicable', value: 0 };
      }))).toBe(false);
    });
  });

  it('rejects undeclared content recursively at every performance-run boundary', () => {
    const unavailable = { status: 'unavailable', reason: 'noSamples' };
    const notApplicable = { status: 'notApplicable' };
    const range = {
      start: unavailable,
      average: unavailable,
      peak: unavailable,
      end: unavailable,
    };
    const run = {
      schemaVersion: 1,
      runId: '0123456789abcdef0123456789abcdef',
      kind: 'voiceQuery',
      startedAtMs: 1,
      finishedAtMs: 2,
      appVersion: '1.0.0',
      correlation: { kind: 'voiceQuery', queryPassId: 9 },
      outcome: { status: 'success' },
      runtimes: [{
        role: 'transcription',
        modelId: 'parakeet-v3',
        backend: 'parakeet',
        accelerator: 'appleNeuralEngine',
        warmState: 'warm',
      }],
      stages: PERFORMANCE_STAGES_V1.map(stage => ({
        stage,
        durationMs: notApplicable,
        outcome: 'skipped',
      })),
      input: {
        audioDurationMs: { status: 'measured', value: 100 },
        inputSizeBucket: notApplicable,
        outputSizeBucket: { status: 'measured', value: 'small' },
        outputTokenCount: notApplicable,
      },
      resources: {
        sampleCount: 0,
        host: { cpuPercent: range },
        mainProcess: {
          cpuPercent: range,
          rssBytes: range,
          rustHeapBytes: range,
          ffiNativeHeapBytes: range,
        },
        sidecarProcess: { cpuPercent: range, rssBytes: range },
      },
      followUps: [{
        kind: 'apply',
        atMs: 2,
        durationMs: notApplicable,
        outcome: 'skipped',
      }],
      queryProcess: { exitCode: 0, stderrPresent: true },
    };
    expect(isPerformanceRunV1(run)).toBe(true);

    const privateValue = 'must not cross the diagnostics boundary';
    expect(isPerformanceRunV1({ ...run, question: privateValue })).toBe(false);
    expect(isPerformanceRunV1({
      ...run,
      correlation: { ...run.correlation, answer: privateValue },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...run,
      outcome: { status: 'success', context: privateValue },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...run,
      runtimes: [{ ...run.runtimes[0], stderr: privateValue }],
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...run,
      stages: run.stages.map((stage, index) => (
        index === 0 ? { ...stage, detail: privateValue } : stage
      )),
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...run,
      input: {
        ...run.input,
        audioDurationMs: { status: 'measured', value: 100, question: privateValue },
      },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...run,
      resources: {
        ...run.resources,
        host: { ...run.resources.host, context: privateValue },
      },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...run,
      resources: {
        ...run.resources,
        mainProcess: {
          ...run.resources.mainProcess,
          rssBytes: { ...run.resources.mainProcess.rssBytes, stderr: privateValue },
        },
      },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...run,
      followUps: [{ ...run.followUps[0], answer: privateValue }],
    })).toBe(false);
  });
});
