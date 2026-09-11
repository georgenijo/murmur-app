import type { PaletteCommand } from './commandPalette';
import type { ModeRuntimeStatus } from './hooks/useModeRuntime';

export function nextRecordingCommands(
  status: ModeRuntimeStatus,
  setNext: (modeId: string | null) => void,
): PaletteCommand[] {
  return [
    ...(status.available ?? []).map((mode) => ({
      id: `next-recording-${mode.id}`,
      title: `Next recording: ${mode.name}`,
      section: 'Recording',
      keywords: ['mode', 'once', 'override', 'dictation'],
      hint: status.pending?.id === mode.id ? 'pending' : 'once',
      run: () => setNext(mode.id),
    })),
    ...(status.pending ? [{
      id: 'next-recording-clear',
      title: 'Clear next recording override',
      section: 'Recording',
      keywords: ['mode', 'cancel', 'pending'],
      run: () => setNext(null),
    }] : []),
  ];
}
