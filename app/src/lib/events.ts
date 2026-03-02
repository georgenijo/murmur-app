export interface AppEvent {
  timestamp: string;
  stream: StreamName;
  level: LevelName;
  summary: string;
  data: Record<string, unknown>;
}

export type StreamName = 'pipeline' | 'audio' | 'keyboard' | 'system';
export type LevelName = 'trace' | 'debug' | 'info' | 'warn' | 'error';

export const STREAMS: StreamName[] = ['pipeline', 'audio', 'keyboard', 'system'];
export const LEVELS: LevelName[] = ['trace', 'debug', 'info', 'warn', 'error'];

export const STREAM_COLORS: Record<StreamName, { bg: string; text: string; dot: string }> = {
  pipeline: { bg: 'bg-stone-200 dark:bg-stone-700', text: 'text-stone-700 dark:text-stone-300', dot: 'bg-stone-500' },
  audio:    { bg: 'bg-blue-100 dark:bg-blue-900/40', text: 'text-blue-700 dark:text-blue-300', dot: 'bg-blue-500' },
  keyboard: { bg: 'bg-purple-100 dark:bg-purple-900/40', text: 'text-purple-700 dark:text-purple-300', dot: 'bg-purple-500' },
  system:   { bg: 'bg-emerald-100 dark:bg-emerald-900/40', text: 'text-emerald-700 dark:text-emerald-300', dot: 'bg-emerald-500' },
};

export const LEVEL_COLORS: Record<LevelName, string> = {
  trace: 'text-stone-400 dark:text-stone-500',
  debug: 'text-stone-500 dark:text-stone-400',
  info:  'text-stone-700 dark:text-stone-300',
  warn:  'text-amber-600 dark:text-amber-400',
  error: 'text-red-600 dark:text-red-400',
};
