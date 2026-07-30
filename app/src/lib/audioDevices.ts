export interface AudioDeviceDescriptor {
  /** Backend-native stable ID; raw CoreAudio UID on macOS. */
  id: string;
  /** Presentation-only display name. */
  name: string;
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
