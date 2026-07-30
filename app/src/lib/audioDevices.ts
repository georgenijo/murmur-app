export interface AudioDeviceDescriptor {
  /** Backend-native stable ID; raw CoreAudio UID on macOS. */
  id: string;
  /** Presentation-only display name. */
  name: string;
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
