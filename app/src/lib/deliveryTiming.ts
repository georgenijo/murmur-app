// Mirrors the strictly bounded native deliveryTiming companion. No free-form strings.
import { isMeasurementV1, type MeasurementV1 } from './performance';

const RouteValues = ['normal', 'smartCurrent', 'smartHandoff', 'clipboardOnly'] as const;
type Route = typeof RouteValues[number];
const OutcomeValues = ['noText', 'autoPasted', 'autoPasteDisabled', 'accessibilityDenied', 'targetChanged', 'focusNotEditable', 'clipboardChanged', 'pasteFailed', 'smartTargetUnavailable', 'cancelled', 'expired', 'alreadyCommitted', 'operationInFlight', 'staleOwner', 'clipboardWriteFailed', 'dispatchFailed', 'replyLost', 'timeout', 'earlyExit'] as const;
type Outcome = typeof OutcomeValues[number];
const FocusKindValues = ['editable', 'nonEditable', 'secure', 'unknown'] as const;
type FocusKind = typeof FocusKindValues[number];
const TransportValues = ['native', 'osascript'] as const;
type Transport = typeof TransportValues[number];
const ProofValues = ['liveMatch', 'liveMismatch', 'identityMismatch', 'frontmostFallbackMatch', 'frontmostFallbackMismatch'] as const;
type Proof = typeof ProofValues[number];
const PostOutcomeValues = ['posted', 'refused', 'stopped', 'constructionFailed'] as const;
type PostOutcome = typeof PostOutcomeValues[number];
const FirstFailureValues = ['sourceCreate', 'keyDownCreate', 'keyUpCreate'] as const;
type FirstFailure = typeof FirstFailureValues[number];
const SmartOutcomeValues = ['ready', 'activationRequested', 'refused', 'refusedClipboardOnly', 'structurallyConfirmed', 'unconfirmed', 'notObserved', 'restored', 'timeout', 'replyLost', 'staleOwner'] as const;
type SmartOutcome = typeof SmartOutcomeValues[number];
const WindowDiagnosticValues = ['unavailableOffPath'] as const;
type WindowDiagnostic = typeof WindowDiagnosticValues[number];
const VerificationPhaseValues = ['beforeFocus', 'afterFocus', 'afterClipboard'] as const;
type VerificationPhase = typeof VerificationPhaseValues[number];
const VerificationOutcomeValues = ['verified', 'differentApplication', 'differentProcess', 'processRelaunched', 'partialIdentityMismatch', 'lookupUnavailable', 'startIdentityIncomplete', 'startTargetIsSelf', 'staleOwner'] as const;
type VerificationOutcome = typeof VerificationOutcomeValues[number];

type Guard = (value: unknown) => boolean;
const object = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const uint: Guard = v => typeof v === 'number' && Number.isSafeInteger(v) && v >= 0;
const int: Guard = v => typeof v === 'number' && Number.isSafeInteger(v) && v >= -2147483648 && v <= 2147483647;
const bool: Guard = v => typeof v === 'boolean';
const enumeration = (values: readonly string[]): Guard => v => typeof v === 'string' && values.includes(v);
const measured = (guard: Guard): Guard => v => isMeasurementV1(v, (x): x is unknown => guard(x));
const shape = (fields: Record<string, Guard>): Guard => v => object(v) && Object.keys(v).length === Object.keys(fields).length && Object.entries(fields).every(([key, guard]) => Object.prototype.hasOwnProperty.call(v, key) && guard(v[key]));

export interface FocusQuery {
  outcome: MeasurementV1<FocusKind>;
  transport: MeasurementV1<Transport>;
  axStatus: MeasurementV1<number>;
  timedOut: boolean;
}

const focusQueryShape: Guard = shape({
  outcome: measured(enumeration(FocusKindValues)),
  transport: measured(enumeration(TransportValues)),
  axStatus: measured(int),
  timedOut: bool,
});
export function isFocusQuery(value: unknown): value is FocusQuery { return focusQueryShape(value); }

export interface TargetVerification {
  phase: VerificationPhase;
  elapsedUs: MeasurementV1<number>;
  lookupRetryCount: MeasurementV1<number>;
  outcome: MeasurementV1<VerificationOutcome>;
}

const targetVerificationShape: Guard = shape({
  phase: enumeration(VerificationPhaseValues),
  elapsedUs: measured(uint),
  lookupRetryCount: measured(v => uint(v) && (v as number) <= 2),
  outcome: measured(enumeration(VerificationOutcomeValues)),
});
export function isTargetVerification(value: unknown): value is TargetVerification { return targetVerificationShape(value); }

export interface NativePost {
  eventConstructionUs: MeasurementV1<number>;
  finalFocusProofUs: MeasurementV1<number>;
  finalFocusProof: MeasurementV1<Proof>;
  finalFocusAxStatus: MeasurementV1<number>;
  keyDownPostUs: MeasurementV1<number>;
  keyHoldUs: MeasurementV1<number>;
  keyUpPostUs: MeasurementV1<number>;
  outcome: MeasurementV1<PostOutcome>;
}

const nativePostShape: Guard = shape({
  eventConstructionUs: measured(uint),
  finalFocusProofUs: measured(uint),
  finalFocusProof: measured(enumeration(ProofValues)),
  finalFocusAxStatus: measured(int),
  keyDownPostUs: measured(uint),
  keyHoldUs: measured(uint),
  keyUpPostUs: measured(uint),
  outcome: measured(enumeration(PostOutcomeValues)),
});
export function isNativePost(value: unknown): value is NativePost { return nativePostShape(value); }

export interface DeliveryTimingV1 {
  schemaVersion: 1;
  acceptedRoute: MeasurementV1<Route>;
  actualRoute: MeasurementV1<Route>;
  outcome: MeasurementV1<Outcome>;
  configuredDelayMs: number;
  accessibility: MeasurementV1<boolean>;
  stageTotalUs: MeasurementV1<number>;
  toPostUs: MeasurementV1<number>;
  configuredDelayElapsedUs: MeasurementV1<number>;
  prepareQueueUs: MeasurementV1<number>;
  prepareOwnerWaitUs: MeasurementV1<number>;
  prepareWorkUs: MeasurementV1<number>;
  activationWaitUs: MeasurementV1<number>;
  injectQueueUs: MeasurementV1<number>;
  injectOwnerWaitUs: MeasurementV1<number>;
  injectWorkUs: MeasurementV1<number>;
  cleanupUs: MeasurementV1<number>;
  receiverAndOutcomeUs: MeasurementV1<number>;
  clipboardWriteUs: MeasurementV1<number>;
  pasteboardCheckUs: MeasurementV1<number>;
  pasteboardCheckCount: number;
  focusQueryUs: MeasurementV1<number>;
  focusQuery: FocusQuery;
  targetVerification: [TargetVerification, TargetVerification, TargetVerification];
  retryTargetVerification: TargetVerification;
  windowDiagnosticUs: MeasurementV1<number>;
  windowDiagnostic: WindowDiagnostic;
  nativePost: NativePost;
  retryPost: NativePost;
  nativePasteRetryCount: number;
  firstFailure: MeasurementV1<FirstFailure>;
  retryWaitUs: MeasurementV1<number>;
  retryFocusQueryUs: MeasurementV1<number>;
  retryFocusQuery: FocusQuery;
  smartPrepareValidationUs: MeasurementV1<number>;
  smartInsertValidationUs: MeasurementV1<number>;
  smartPostSettleUs: MeasurementV1<number>;
  smartReadbackUs: MeasurementV1<number>;
  smartRestoreUs: MeasurementV1<number>;
  preparation: MeasurementV1<SmartOutcome>;
  confirmation: MeasurementV1<SmartOutcome>;
  rollback: MeasurementV1<SmartOutcome>;
  timedOut: boolean;
  cancelled: boolean;
  expired: boolean;
}

const deliveryTimingV1Shape: Guard = shape({
  schemaVersion: (v => v === 1),
  acceptedRoute: measured(enumeration(RouteValues)),
  actualRoute: measured(enumeration(RouteValues)),
  outcome: measured(enumeration(OutcomeValues)),
  configuredDelayMs: uint,
  accessibility: measured(bool),
  stageTotalUs: measured(uint),
  toPostUs: measured(uint),
  configuredDelayElapsedUs: measured(uint),
  prepareQueueUs: measured(uint),
  prepareOwnerWaitUs: measured(uint),
  prepareWorkUs: measured(uint),
  activationWaitUs: measured(uint),
  injectQueueUs: measured(uint),
  injectOwnerWaitUs: measured(uint),
  injectWorkUs: measured(uint),
  cleanupUs: measured(uint),
  receiverAndOutcomeUs: measured(uint),
  clipboardWriteUs: measured(uint),
  pasteboardCheckUs: measured(uint),
  pasteboardCheckCount: uint,
  focusQueryUs: measured(uint),
  focusQuery: isFocusQuery,
  targetVerification: (v => Array.isArray(v) && v.length === 3 && v.every(isTargetVerification) && v.every((x, i) => x.phase === VerificationPhaseValues[i])),
  retryTargetVerification: isTargetVerification,
  windowDiagnosticUs: measured(uint),
  windowDiagnostic: enumeration(WindowDiagnosticValues),
  nativePost: isNativePost,
  retryPost: isNativePost,
  nativePasteRetryCount: (v => uint(v) && (v as number) <= 1),
  firstFailure: measured(enumeration(FirstFailureValues)),
  retryWaitUs: measured(uint),
  retryFocusQueryUs: measured(uint),
  retryFocusQuery: isFocusQuery,
  smartPrepareValidationUs: measured(uint),
  smartInsertValidationUs: measured(uint),
  smartPostSettleUs: measured(uint),
  smartReadbackUs: measured(uint),
  smartRestoreUs: measured(uint),
  preparation: measured(enumeration(SmartOutcomeValues)),
  confirmation: measured(enumeration(SmartOutcomeValues)),
  rollback: measured(enumeration(SmartOutcomeValues)),
  timedOut: bool,
  cancelled: bool,
  expired: bool,
});
export function isDeliveryTimingV1(value: unknown): value is DeliveryTimingV1 { return deliveryTimingV1Shape(value); }
