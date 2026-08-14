export interface AudioDeviceDescriptor {
  /** Backend-native stable ID; raw CoreAudio UID on macOS. */
  id: string;
  /** Presentation-only display name. */
  name: string;
}

export type AudioInputInventoryStatus = 'available' | 'stale' | 'unavailable';
export type AudioInputInventoryErrorCode =
  | null
  | 'notInitialized'
  | 'captureActive'
  | 'enumerationFailed'
  | 'refreshPending';

export interface AudioInputInventoryV1 {
  schemaVersion: 1;
  revision: number;
  status: AudioInputInventoryStatus;
  devices: AudioDeviceDescriptor[];
  defaultInputId: string | null;
  errorCode: AudioInputInventoryErrorCode;
}

const INVENTORY_KEYS = [
  'schemaVersion', 'revision', 'status', 'devices', 'defaultInputId', 'errorCode',
] as const;
const DEVICE_KEYS = ['id', 'name'] as const;
const ERROR_CODES = new Set(['notInitialized', 'captureActive', 'enumerationFailed', 'refreshPending']);

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

/** Strict boundary validation for the shared Rust command/event payload. */
export function parseAudioInputInventory(value: unknown): AudioInputInventoryV1 | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (!hasExactKeys(record, INVENTORY_KEYS)
    || record.schemaVersion !== 1
    || !Number.isSafeInteger(record.revision)
    || (record.revision as number) < 0
    || typeof record.status !== 'string'
    || !['available', 'stale', 'unavailable'].includes(record.status)
    || !Array.isArray(record.devices)
    || record.devices.length > 256
    || !(record.defaultInputId === null || isBoundedDeviceField(record.defaultInputId, 4096))
    || !(record.errorCode === null || (typeof record.errorCode === 'string' && ERROR_CODES.has(record.errorCode)))) return null;

  const devices: AudioDeviceDescriptor[] = [];
  const ids = new Set<string>();
  for (const item of record.devices) {
    if (!item || typeof item !== 'object' || Array.isArray(item)) return null;
    const device = item as Record<string, unknown>;
    if (!hasExactKeys(device, DEVICE_KEYS)
      || !isBoundedDeviceField(device.id, 4096)
      || !isBoundedDeviceField(device.name, 512)
      || ids.has(device.id)) return null;
    ids.add(device.id);
    devices.push({ id: device.id, name: device.name });
  }

  const status = record.status as AudioInputInventoryStatus;
  const errorCode = record.errorCode as AudioInputInventoryErrorCode;
  if ((record.defaultInputId !== null && !ids.has(record.defaultInputId as string))
    || (status === 'available' && errorCode !== null)
    || (status === 'stale' && errorCode !== 'enumerationFailed' && errorCode !== 'refreshPending')
    || (status === 'unavailable' && (devices.length !== 0 || record.defaultInputId !== null || errorCode === null))) return null;

  return {
    schemaVersion: 1,
    revision: record.revision as number,
    status,
    devices,
    defaultInputId: record.defaultInputId as string | null,
    errorCode,
  };
}

export interface AudioDeviceSelectOption {
  value: string;
  label: string;
}

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
