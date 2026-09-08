export interface AudioDeviceDescriptor {
  /** Backend-native stable ID; raw CoreAudio UID on macOS. */
  id: string;
  /** Presentation-only display name. */
  name: string;
  /** Core Audio transport classification; never inferred from the label. */
  kind: 'builtIn' | 'external' | 'continuity' | 'unknown';
  /** Core Audio's device-alive property at the cached inventory read. */
  connected: boolean;
  /** Native input-scope enumeration found an input stream. */
  hasInput: boolean;
}

export type AudioInputLidState = 'open' | 'closed' | 'unknown';
export type AudioDeviceKind = AudioDeviceDescriptor['kind'];

export type AudioInputInventoryStatus = 'available' | 'stale' | 'unavailable';
export type AudioInputInventoryErrorCode =
  | null
  | 'notInitialized'
  | 'captureActive'
  | 'enumerationFailed'
  | 'refreshPending';

export interface AudioInputInventoryV2 {
  schemaVersion: 2;
  revision: number;
  status: AudioInputInventoryStatus;
  devices: AudioDeviceDescriptor[];
  defaultInputId: string | null;
  lidState: AudioInputLidState;
  errorCode: AudioInputInventoryErrorCode;
}


const INVENTORY_KEYS = [
  'schemaVersion', 'revision', 'status', 'devices', 'defaultInputId', 'lidState', 'errorCode',
] as const;
const DEVICE_KEYS = ['id', 'name', 'kind', 'connected', 'hasInput'] as const;
const ERROR_CODES = new Set(['notInitialized', 'captureActive', 'enumerationFailed', 'refreshPending']);
const DEVICE_KINDS = new Set<string>(['builtIn', 'external', 'continuity', 'unknown']);
const LID_STATES = new Set<string>(['open', 'closed', 'unknown']);

function hasExactKeys(value: Record<string, unknown>, keys: readonly string[]): boolean {
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  return actual.length === expected.length
    && actual.every((key, index) => key === expected[index]);
}

function isBoundedDeviceField(value: unknown, maxBytes: number): value is string {
  return typeof value === 'string'
    && value.length > 0
    && !value.includes('\0')
    && new TextEncoder().encode(value).length <= maxBytes;
}

function isDeviceKind(value: unknown): value is AudioDeviceKind {
  return typeof value === 'string' && DEVICE_KINDS.has(value);
}

function isLidState(value: unknown): value is AudioInputLidState {
  return typeof value === 'string' && LID_STATES.has(value);
}

/** Strict boundary validation for the shared Rust command/event payload. */
export function parseAudioInputInventory(value: unknown): AudioInputInventoryV2 | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (!hasExactKeys(record, INVENTORY_KEYS)
    || record.schemaVersion !== 2
    || !Number.isSafeInteger(record.revision)
    || (record.revision as number) < 0
    || typeof record.status !== 'string'
    || !['available', 'stale', 'unavailable'].includes(record.status)
    || !Array.isArray(record.devices)
    || record.devices.length > 256
    || !(record.defaultInputId === null || isBoundedDeviceField(record.defaultInputId, 4096))
    || !isLidState(record.lidState)
    || !(record.errorCode === null || (typeof record.errorCode === 'string' && ERROR_CODES.has(record.errorCode)))) return null;

  const devices: AudioDeviceDescriptor[] = [];
  const ids = new Set<string>();
  for (const item of record.devices) {
    if (!item || typeof item !== 'object' || Array.isArray(item)) return null;
    const device = item as Record<string, unknown>;
    if (!hasExactKeys(device, DEVICE_KEYS)
      || !isBoundedDeviceField(device.id, 4096)
      || !isBoundedDeviceField(device.name, 512)
      || !isDeviceKind(device.kind)
      || typeof device.connected !== 'boolean'
      || typeof device.hasInput !== 'boolean'
      || ids.has(device.id)) return null;
    ids.add(device.id);
    devices.push({
      id: device.id,
      name: device.name,
      kind: device.kind,
      connected: device.connected,
      hasInput: device.hasInput,
    });
  }

  const status = record.status as AudioInputInventoryStatus;
  const errorCode = record.errorCode as AudioInputInventoryErrorCode;
  if ((record.defaultInputId !== null && !ids.has(record.defaultInputId as string))
    || (status === 'available' && errorCode !== null)
    || (status === 'stale' && errorCode !== 'enumerationFailed' && errorCode !== 'refreshPending')
    || (status === 'unavailable' && (devices.length !== 0 || record.defaultInputId !== null || errorCode === null))) return null;

  return {
    schemaVersion: 2,
    revision: record.revision as number,
    status,
    devices,
    defaultInputId: record.defaultInputId as string | null,
    lidState: record.lidState,
    errorCode,
  };
}

export interface AudioDeviceSelectOption {
  value: string;
  label: string;
}

export const FOLLOW_SYSTEM_DEFAULT_LABEL = 'Follow macOS Default';

/** Preserve concise unique labels; append the stable ID only for collisions. */
export function audioDeviceSelectOptions(
  devices: AudioDeviceDescriptor[],
): AudioDeviceSelectOption[] {
  const nameCounts = new Map<string, number>();
  for (const device of devices) {
    nameCounts.set(device.name, (nameCounts.get(device.name) ?? 0) + 1);
  }
  return devices.map((device) => ({
    value: device.id,
    label: nameCounts.get(device.name) === 1
      ? device.name
      : `${device.name} (${device.id})`,
  }));
}

/** Make automatic selection explicit and show the device macOS resolves now. */
export function followSystemDefaultOptionLabel(
  devices: AudioDeviceDescriptor[],
  defaultInputId: string | null,
): string {
  const resolved = audioDeviceSelectOptions(devices)
    .find((device) => device.value === defaultInputId);
  return resolved
    ? `${FOLLOW_SYSTEM_DEFAULT_LABEL} — ${resolved.label}`
    : FOLLOW_SYSTEM_DEFAULT_LABEL;
}

/**
 * Migrate the pre-CPAL-0.18 display-name setting only when it identifies one
 * device unambiguously. Duplicate names fail closed and remain unresolved.
 */
export function migrateLegacyMicrophoneId(
  current: string,
  devices: AudioDeviceDescriptor[],
): string {
  if (current === 'system_default' || devices.some((device) => device.id === current)) {
    return current;
  }
  const matches = devices.filter((device) => device.name === current);
  return matches.length === 1 ? matches[0].id : current;
}

export function selectedDeviceExists(
  current: string,
  devices: AudioDeviceDescriptor[],
): boolean {
  return current === 'system_default'
    || devices.some((device) => device.id === current);
}

export interface SmartAutoPreviewRequest {
  approvedDeviceIds: string[];
  preferredDeviceIds: string[];
  allowContinuity: boolean;
}

export type SmartAutoPreviewReason =
  | 'preferred_approved'
  | 'approved_macos_default'
  | 'approved_external_fallback'
  | 'approved_continuity_fallback';

export interface SmartAutoPreviewSelection {
  device: AudioDeviceDescriptor;
  reason: SmartAutoPreviewReason;
}

function isSmartAutoEligible(
  device: AudioDeviceDescriptor,
  approved: Set<string>,
  lidState: AudioInputLidState,
  allowContinuity: boolean,
): boolean {
  if (!approved.has(device.id) || device.connected !== true || device.hasInput !== true) return false;
  if (device.kind === 'external') return true;
  if (device.kind === 'continuity') return allowContinuity;
  return device.kind === 'builtIn' && lidState === 'open';
}

/** Mirrors Rust's cache-only policy for Settings presentation. The recording
 * command resolves again from the authoritative cache and freezes that result. */
export function previewSmartAutoSelection(
  request: SmartAutoPreviewRequest,
  devices: AudioDeviceDescriptor[],
  defaultInputId: string | null,
  lidState: AudioInputLidState,
): SmartAutoPreviewSelection | null {
  const approved = new Set(request.approvedDeviceIds);
  const eligible = (device: AudioDeviceDescriptor) => isSmartAutoEligible(
    device, approved, lidState, request.allowContinuity,
  );
  for (const id of request.preferredDeviceIds) {
    const device = devices.find((candidate) => candidate.id === id && eligible(candidate));
    if (device) return { device, reason: 'preferred_approved' };
  }
  const macosDefault = devices.find((device) => device.id === defaultInputId && eligible(device));
  if (macosDefault) return { device: macosDefault, reason: 'approved_macos_default' };
  const fallback = (kind: 'external' | 'continuity') => devices
    .filter((device) => device.kind === kind && eligible(device))
    .sort((left, right) => left.id.localeCompare(right.id))[0];
  const external = fallback('external');
  if (external) return { device: external, reason: 'approved_external_fallback' };
  const continuity = fallback('continuity');
  return continuity ? { device: continuity, reason: 'approved_continuity_fallback' } : null;
}
