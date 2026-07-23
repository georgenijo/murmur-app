import { invoke } from '@tauri-apps/api/core';

/** Write a log line to frontend.dev.log (or frontend.log in production). */
function write(level: string, tag: string, message: string, data?: Record<string, unknown>) {
  const line = data
    ? `[${tag}] ${message} ${JSON.stringify(data)}`
    : `[${tag}] ${message}`;
  const candidatePassId = data?.transform_pass_id;
  const transformPassId =
    typeof candidatePassId === 'number'
      && Number.isSafeInteger(candidatePassId)
      && candidatePassId > 0
      ? candidatePassId
      : null;
  // Fire-and-forget — don't await, don't block the caller
  invoke('log_frontend', { level, message: line, transformPassId }).catch(() => {});
}

export const flog = {
  info: (tag: string, message: string, data?: Record<string, unknown>) => write('INFO', tag, message, data),
  warn: (tag: string, message: string, data?: Record<string, unknown>) => write('WARN', tag, message, data),
  error: (tag: string, message: string, data?: Record<string, unknown>) => write('ERROR', tag, message, data),
};
