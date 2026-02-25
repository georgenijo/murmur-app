import { invoke } from '@tauri-apps/api/core';

/** Write a log line to frontend.dev.log (or frontend.log in production). */
function write(level: string, tag: string, message: string, data?: Record<string, unknown>) {
  const line = data
    ? `[${tag}] ${message} ${JSON.stringify(data)}`
    : `[${tag}] ${message}`;
  // Fire-and-forget — don't await, don't block the caller
  invoke('log_frontend', { level, message: line }).catch(() => {});
}

export const flog = {
  info: (tag: string, message: string, data?: Record<string, unknown>) => write('INFO', tag, message, data),
  warn: (tag: string, message: string, data?: Record<string, unknown>) => write('WARN', tag, message, data),
  error: (tag: string, message: string, data?: Record<string, unknown>) => write('ERROR', tag, message, data),
};
