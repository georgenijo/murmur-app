/** Presentation types for the Rust-owned updater protocol. */
export interface CompletedUpdate { version: string; notes: string }
export interface UpdaterSnapshot {
  revision: number;
  updateStatus: UpdateStatus;
  completedUpdate: CompletedUpdate | null;
  isUpdateDialogOpen: boolean;
}

export type UpdateStatus =
  | { phase: 'idle' }
  | { phase: 'checking' }
  | { phase: 'available'; version: string; notes: string; isForced: boolean }
  | { phase: 'preparing'; version: string }
  | { phase: 'downloading'; version: string; progress: number | null }
  | { phase: 'ready'; version: string; isForced: boolean }
  | { phase: 'restarting'; version: string }
  | {
      phase: 'error';
      stage: 'check' | 'install' | 'restart';
      message: string;
      isForced: boolean;
      recovery?: 'reinstall';
    }
  | { phase: 'up-to-date' };
