import { invoke } from '@tauri-apps/api/core';

/**
 * Event codes the backend keeps (`canonical_event_code` in telemetry.rs). The
 * backend drops any other frontend event, so only these may be sent.
 */
export type FrontendEventCode =
  | 'system.launch_frontend_hydrated'
  | 'updater.check_current'
  | 'updater.check_failed'
  | 'updater.install_blocked'
  | 'updater.install_ready'
  | 'updater.install_failed'
  | 'updater.restart_failed'
  | 'updater.update_available'
  | 'updater.freshness_check_failed'
  | 'updater.download_starting'
  | 'updater.download_started'
  | 'updater.download_finished'
  | 'updater.download_ready';

interface FrontendEventData {
  version?: string;
  selection?: string;
}

/** Record a content-free frontend event in the Rust structured log. */
function write(level: string, eventCode: FrontendEventCode, data?: FrontendEventData) {
  // Fire-and-forget — don't await, don't block the caller
  invoke('log_frontend', {
    level,
    eventCode,
    updaterVersion: data?.version ?? null,
    updaterSelection: data?.selection ?? null,
  }).catch(() => {});
}

export const flog = {
  info: (eventCode: FrontendEventCode, data?: FrontendEventData) => write('INFO', eventCode, data),
  warn: (eventCode: FrontendEventCode, data?: FrontendEventData) => write('WARN', eventCode, data),
  error: (eventCode: FrontendEventCode, data?: FrontendEventData) => write('ERROR', eventCode, data),
};
