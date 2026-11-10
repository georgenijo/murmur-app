import { describe, expect, it } from 'vitest';
import deliveryTimingFixture from './fixtures/delivery-timing-v1.json';
import { makeRun } from '../components/log-viewer/testFixtures';
import {
  isMeasurementV1,
  isPerformanceRunV1,
  isPerformanceStoreHealthV1,
  isResourceSampleV1,
  measuredValue,
  PERFORMANCE_STAGES_V1,
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

  // Adapted from trial PR #1093 (Sonnet), 9db3f505, for our schema.
  describe('delivery timing attribution', () => {
    type Path = string[];
    type TestObject = Record<string, unknown>;
    const measured = (value: unknown) => ({ status: 'measured', value });
    const withTiming = (deliveryTiming: unknown) => ({ ...makeRun(), deliveryTiming });
    const at = (timing: unknown, path: Path): TestObject => path.reduce(
      (value, key) => (value as TestObject)[key], timing,
    ) as TestObject;
    const changed = (path: Path, value: unknown) => {
      const timing = structuredClone(deliveryTimingFixture);
      at(timing, path.slice(0, -1))[path[path.length - 1]] = value;
      return withTiming(timing);
    };
    const objects: Path[] = [];
    const visit = (value: unknown, path: Path) => {
      if (typeof value !== 'object' || value === null) return;
      if (!Array.isArray(value)) objects.push(path);
      for (const [key, child] of Object.entries(value)) visit(child, [...path, key]);
    };
    visit(deliveryTimingFixture, []);
    const durationPaths = objects.filter(path => path[path.length - 1]?.endsWith('Us'));
    const verificationPaths = [
      ...[0, 1, 2].map(i => ['targetVerification', String(i)]),
      ['retryTargetVerification'],
    ];
    const focusPaths = [['focusQuery'], ['retryFocusQuery']];
    const postPaths = [['nativePost'], ['retryPost']];

    it('accepts Smart refusal records and historical absence, but rejects a null companion', () => {
      expect(isPerformanceRunV1(withTiming({
        ...deliveryTimingFixture,
        acceptedRoute: measured('smartHandoff'),
        actualRoute: measured('clipboardOnly'),
        outcome: measured('smartTargetUnavailable'),
        preparation: measured('refusedClipboardOnly'),
      }))).toBe(true);
      expect(isPerformanceRunV1(makeRun())).toBe(true);
      expect(isPerformanceRunV1(withTiming(null))).toBe(false);
      expect(isPerformanceRunV1(changed(['schemaVersion'], '1'))).toBe(false);
    });

    it('preserves measurement states and measured zero through the run guard', () => {
      for (const path of durationPaths) {
        for (const measurement of [
          measured(0), { status: 'notApplicable' },
          ...['unsupportedPlatform', 'sampleFailed', 'noSamples', 'dependencyPending']
            .map(reason => ({ status: 'unavailable', reason })),
        ]) {
          expect(isPerformanceRunV1(changed(path, measurement)), path.join('.')).toBe(true);
          expect(measuredValue(measurement as MeasurementV1<number>))
            .toBe(measurement.status === 'measured' ? 0 : null);
        }
        for (const invalid of [
          0, null, { status: 'pending' }, { status: 'measured' },
          { status: 'unavailable', reason: 'because' },
          { status: 'notApplicable', value: 0 },
          { status: 'measured', value: 0, title: 'synthetic forbidden content' },
        ]) expect(isPerformanceRunV1(changed(path, invalid)), path.join('.')).toBe(false);
      }
    });

    it('accepts only the bounded vocabularies on initial and retry records', () => {
      const vocabularies: [Path[], string[]][] = [
        [[['acceptedRoute'], ['actualRoute']], ['normal', 'smartCurrent', 'smartHandoff', 'clipboardOnly']],
        [[['outcome']], ['noText', 'autoPasted', 'autoPasteDisabled', 'accessibilityDenied',
          'targetChanged', 'focusNotEditable', 'clipboardChanged', 'pasteFailed',
          'smartTargetUnavailable', 'cancelled', 'expired', 'alreadyCommitted',
          'operationInFlight', 'staleOwner', 'clipboardWriteFailed', 'dispatchFailed',
          'replyLost', 'timeout', 'earlyExit']],
        [[['preparation'], ['confirmation'], ['rollback']], ['ready', 'activationRequested',
          'refused', 'refusedClipboardOnly', 'structurallyConfirmed', 'unconfirmed',
          'notObserved', 'restored', 'timeout', 'replyLost', 'staleOwner']],
        [focusPaths.map(p => [...p, 'outcome']), ['editable', 'nonEditable', 'secure', 'unknown']],
        [focusPaths.map(p => [...p, 'transport']), ['native', 'osascript']],
        [verificationPaths.map(p => [...p, 'outcome']), ['verified', 'differentApplication',
          'differentProcess', 'processRelaunched', 'partialIdentityMismatch', 'lookupUnavailable',
          'startIdentityIncomplete', 'startTargetIsSelf', 'staleOwner']],
        [postPaths.map(p => [...p, 'finalFocusProof']), ['liveMatch', 'liveMismatch',
          'identityMismatch', 'frontmostFallbackMatch', 'frontmostFallbackMismatch']],
        [postPaths.map(p => [...p, 'outcome']), ['posted', 'refused', 'stopped', 'constructionFailed']],
        [[['firstFailure']], ['sourceCreate', 'keyDownCreate', 'keyUpCreate']],
      ];
      for (const [paths, values] of vocabularies) for (const path of paths) {
        for (const value of values) {
          expect(isPerformanceRunV1(changed(path, measured(value))), path.join('.')).toBe(true);
        }
        for (const invalid of ['synthetic arbitrary content', '', 0, null, {}]) {
          expect(isPerformanceRunV1(changed(path, measured(invalid))), path.join('.')).toBe(false);
        }
      }
      expect(isPerformanceRunV1(changed(['windowDiagnostic'], 'unavailableOffPath'))).toBe(true);
      expect(isPerformanceRunV1(changed(['windowDiagnostic'], 'synthetic window title'))).toBe(false);
      for (const phase of ['beforeFocus', 'afterFocus', 'afterClipboard']) {
        expect(isPerformanceRunV1(changed(['retryTargetVerification', 'phase'], phase))).toBe(true);
      }
      for (const path of verificationPaths) {
        expect(isPerformanceRunV1(changed([...path, 'phase'], 'arbitrary'))).toBe(false);
      }
      expect(isPerformanceRunV1(changed(['targetVerification'], deliveryTimingFixture.targetVerification.slice(1)))).toBe(false);
      expect(isPerformanceRunV1(changed(['targetVerification'], [...deliveryTimingFixture.targetVerification, deliveryTimingFixture.targetVerification[0]]))).toBe(false);
    });

    it('rejects nested content and missing keys at every object boundary', () => {
      for (const path of objects) {
        const timing = structuredClone(deliveryTimingFixture);
        at(timing, path).windowTitle = 'synthetic forbidden content';
        expect(isPerformanceRunV1(withTiming(timing)), path.join('.')).toBe(false);
        for (const key of Object.keys(at(deliveryTimingFixture, path))) {
          const incomplete = structuredClone(deliveryTimingFixture);
          delete at(incomplete, path)[key];
          expect(isPerformanceRunV1(withTiming(incomplete)), [...path, key].join('.')).toBe(false);
        }
      }
    });

    it('requires safe integer durations and counts, retaining bounded retry counts', () => {
      const lookupPaths = verificationPaths.map(p => [...p, 'lookupRetryCount']);
      for (const path of [...durationPaths, ...lookupPaths]) {
        for (const value of [-1, 1.5, Number.MAX_SAFE_INTEGER + 1, Number.NaN, '12']) {
          expect(isPerformanceRunV1(changed(path, measured(value))), path.join('.')).toBe(false);
        }
        expect(isPerformanceRunV1(changed(path, measured(0))), path.join('.')).toBe(true);
      }
      for (const path of durationPaths) {
        expect(isPerformanceRunV1(changed(path, measured(Number.MAX_SAFE_INTEGER))), path.join('.')).toBe(true);
      }
      for (const path of lookupPaths) {
        expect(isPerformanceRunV1(changed(path, measured(2)))).toBe(true);
        expect(isPerformanceRunV1(changed(path, measured(3)))).toBe(false);
      }
      for (const key of ['configuredDelayMs', 'pasteboardCheckCount', 'nativePasteRetryCount']) {
        for (const value of [-1, 1.5, Number.MAX_SAFE_INTEGER + 1, Number.NaN, '12', measured(0)]) {
          expect(isPerformanceRunV1(changed([key], value)), key).toBe(false);
        }
      }
      expect(isPerformanceRunV1(changed(['nativePasteRetryCount'], 1))).toBe(true);
      expect(isPerformanceRunV1(changed(['nativePasteRetryCount'], 2))).toBe(false);
    });

    it('keeps signed AX status distinct from unsigned durations and requires real booleans', () => {
      for (const path of [
        ...focusPaths.map(p => [...p, 'axStatus']),
        ...postPaths.map(p => [...p, 'finalFocusAxStatus']),
      ]) {
        for (const value of [-2147483648, -25204, 0, 2147483647]) {
          expect(isPerformanceRunV1(changed(path, measured(value)))).toBe(true);
        }
        for (const value of [-2147483649, 2147483648, 1.5, Number.NaN, '0']) {
          expect(isPerformanceRunV1(changed(path, measured(value)))).toBe(false);
        }
      }
      for (const path of [['timedOut'], ['cancelled'], ['expired'], ...focusPaths.map(p => [...p, 'timedOut'])]) {
        for (const value of [true, false]) expect(isPerformanceRunV1(changed(path, value))).toBe(true);
        for (const value of [0, 'true', null]) expect(isPerformanceRunV1(changed(path, value))).toBe(false);
      }
      for (const value of [true, false]) expect(isPerformanceRunV1(changed(['accessibility'], measured(value)))).toBe(true);
      for (const value of [0, 'true', null]) expect(isPerformanceRunV1(changed(['accessibility'], measured(value)))).toBe(false);
    });
  });

});

describe('shadow diagnostics contract', () => {
  const shadow = {
    schemaVersion: 1, pressSample: 16000, pauseSample: 20800,
    trailingSilenceBeforePressMs: 0, trailingSilenceBeforeStopMs: 500,
    pressInputMatches: false, pauseInputMatches: true, previewHeldBackendAtPress: false,
    determinism: 'disabled', decodeMatches: null,
  };
  it('accepts either V1 companion or both while validating each independently', () => {
    for (const companions of [{}, { shadow }, { deliveryTiming: deliveryTimingFixture },
      { shadow, deliveryTiming: deliveryTimingFixture }]) {
      expect(isPerformanceRunV1({ ...makeRun(), ...companions })).toBe(true);
    }
    const combined = { ...makeRun(), shadow, deliveryTiming: deliveryTimingFixture };
    expect(isPerformanceRunV1({ ...combined, shadow: { ...shadow, schemaVersion: 2 } })).toBe(false);
    expect(isPerformanceRunV1({ ...combined,
      deliveryTiming: { ...deliveryTimingFixture, schemaVersion: 2 } })).toBe(false);
    expect(isPerformanceRunV1({ ...combined, shadow: { ...shadow, decodeMatches: true } })).toBe(false);
    expect(isPerformanceRunV1({ ...combined, deliveryTiming: null })).toBe(false);
  });
  it('reads historical runs and validates the versioned content-free companion', () => {
    expect(isPerformanceRunV1(makeRun())).toBe(true);
    expect(isPerformanceRunV1({ ...makeRun(), shadow })).toBe(true);
    expect(isPerformanceRunV1({ ...makeRun(), shadow: { ...shadow, schemaVersion: 2 } })).toBe(false);
    expect(isPerformanceRunV1({ ...makeRun(), shadow: { ...shadow, pressSample: -1 } })).toBe(false);
    expect(isPerformanceRunV1({ ...makeRun(), shadow: { ...shadow, text: 'forbidden' } })).toBe(false);
    expect(isPerformanceRunV1({ ...makeRun(), shadow: { ...shadow, decodeMatches: true } })).toBe(false);
    expect(isPerformanceRunV1({ ...makeRun(), shadow: { ...shadow, determinism: 'compared', decodeMatches: true } })).toBe(true);
  });
});

it('accepts the bounded repeat skip reason and rejects a fabricated comparison', () => {
  const shadow = {
    schemaVersion: 1, pressSample: null, pauseSample: null,
    trailingSilenceBeforePressMs: null, trailingSilenceBeforeStopMs: null,
    pressInputMatches: false, pauseInputMatches: null, previewHeldBackendAtPress: false,
    determinism: 'skippedForegroundLatencyRisk', decodeMatches: null,
  };
  expect(isPerformanceRunV1({ ...makeRun(), shadow })).toBe(true);
  expect(isPerformanceRunV1({ ...makeRun(), shadow: { ...shadow, decodeMatches: false } })).toBe(false);
});

it('accepts not measured in app without inventing a comparison', () => {
  const shadow = {
    schemaVersion: 1, pressSample: null, pauseSample: null,
    trailingSilenceBeforePressMs: null, trailingSilenceBeforeStopMs: null,
    pressInputMatches: null, pauseInputMatches: null, previewHeldBackendAtPress: null,
    determinism: 'notMeasuredInApp', decodeMatches: null,
  };
  expect(isPerformanceRunV1({ ...makeRun(), shadow })).toBe(true);
  expect(isPerformanceRunV1({ ...makeRun(), shadow: { ...shadow, decodeMatches: true } })).toBe(false);
});
