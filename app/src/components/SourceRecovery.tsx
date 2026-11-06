import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

/** A failed startup never mounts editors with default data. Restore requires a
 * complete, user-selected backup and preserves all quarantined evidence. */
export function SourceRecovery() {
  const [key, setKey] = useState('dictation-settings');
  const [message, setMessage] = useState('');
  const [busy, setBusy] = useState(false);
  return <main>
    <h1>Saved data needs recovery</h1>
    <p role="alert">Your saved data could not be loaded. Restart to retry an interrupted save. If a destination is missing or quarantined, restore its complete current-version JSON backup below. Existing quarantines are preserved. No defaults will be published.</p>
    <section aria-label="Completion recovery">
      <h2>History and reports</h2>
      <p>If an interrupted History or report save is damaged, preserve its rejected evidence and continue from the readable saved stores. The rejected save will not be replayed. Clear History and retention actions remove this evidence.</p>
      <button disabled={busy} onClick={() => {
        setBusy(true);
        void invoke('recover_completion_pending')
          .then(() => setMessage('Completion recovery finished. Restart Murmur to load your data.'))
          .catch(() => setMessage('Completion recovery could not finish. Preserve the data directory for manual recovery.'))
          .finally(() => setBusy(false));
      }}>Preserve rejected save and recover</button>
    </section>
    <label>Destination <select value={key} disabled={busy} onChange={event => setKey(event.target.value)}>
      <option value="dictation-settings">Settings (settings.json)</option>
      <option value="murmur-theme-library">Theme library (theme-library.json)</option>
    </select></label>
    <label>Restore backup <input type="file" accept=".json,application/json" disabled={busy} onChange={event => {
      const file = event.target.files?.[0];
      if (!file || busy) return;
      const destination = key;
      setBusy(true);
      setMessage('Restoring backup…');
      void file.text().then(blob => invoke('restore_source_backup', { key: destination, blob }))
        .then(() => setMessage('Backup restored. Restart Murmur to load your data.'))
        .catch(() => setMessage('Backup could not be restored. Select a complete current-version backup for a missing or quarantined destination. Your saved files have been preserved.'))
        .finally(() => setBusy(false));
    }} /></label>
    <p role="status">{message}</p>
    <p>If you have no backup, preserve the data directory and quarantines for manual recovery before proceeding.</p>
  </main>;
}
