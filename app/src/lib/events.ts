export interface AppEvent {
  timestamp: string;
  stream: StreamName;
  level: LevelName;
  summary: string;
  data: Record<string, unknown>;
}

export type StreamName = 'pipeline' | 'audio' | 'keyboard' | 'transform' | 'meeting' | 'query' | 'system';
export type LevelName = 'trace' | 'debug' | 'info' | 'warn' | 'error';

export const STREAMS: StreamName[] = [
  'pipeline',
  'audio',
  'keyboard',
  'transform',
  'meeting',
  'query',
  'system',
];
export const LEVELS: LevelName[] = ['trace', 'debug', 'info', 'warn', 'error'];

export const STREAM_COLORS: Record<StreamName, { bg: string; text: string; dot: string }> = {
  pipeline: {
    bg: 'bg-surface-container-high',
    text: 'text-on-surface',
    dot: 'bg-on-surface',
  },
  audio: {
    bg: 'bg-primary/10',
    text: 'text-on-surface',
    dot: 'bg-on-surface',
  },
  keyboard: {
    bg: 'bg-surface-container-lowest',
    text: 'text-primary',
    dot: 'bg-primary',
  },
  transform: {
    bg: 'bg-warning/10',
    text: 'text-warning',
    dot: 'bg-warning',
  },
  meeting: {
    bg: 'bg-surface-container-low',
    text: 'text-on-surface',
    dot: 'bg-error',
  },
  query: {
    bg: 'bg-primary/10',
    text: 'text-primary',
    dot: 'bg-primary',
  },
  system: {
    bg: 'bg-success/10',
    text: 'text-success',
    dot: 'bg-success',
  },
};

export const LEVEL_COLORS: Record<LevelName, string> = {
  trace: 'text-on-surface-variant',
  debug: 'text-on-surface-variant',
  info: 'text-on-surface',
  warn: 'text-warning',
  error: 'text-error',
};
