import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { QueryCommandConfig } from '../../lib/queryProviders';

type CapabilityStatus = { profile: 'restricted' | 'trusted_read_only'; directory: string | null };

export function QueryCapabilities({ command }: { command: QueryCommandConfig }) {
  const [status, setStatus] = useState<CapabilityStatus | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const generation = useRef(0);

  useEffect(() => {
    let disposed = false;
    void invoke<CapabilityStatus>('get_query_capabilities').then((value) => {
      if (!disposed) setStatus(value);
    }).catch(() => { if (!disposed) setError('Capability status is unavailable. Reopen Settings to try again.'); });
    return () => { disposed = true; generation.current += 1; };
  }, []);

  const choose = async () => {
    const ticket = ++generation.current;
    setBusy(true); setError(null); setPending(null);
    try {
      const directory = await invoke<string | null>('choose_query_workspace');
      if (ticket === generation.current) setPending(directory);
    } catch (cause) { if (ticket === generation.current) setError(String(cause)); }
    finally { if (ticket === generation.current) setBusy(false); }
  };

  const confirm = async () => {
    const ticket = ++generation.current;
    setBusy(true); setError(null);
    try {
      const value = await invoke<CapabilityStatus>('confirm_query_workspace', { command, expectedDirectory: pending });
      if (ticket === generation.current) { setStatus(value); setPending(null); }
    } catch (cause) { if (ticket === generation.current) setError(String(cause)); }
    finally { if (ticket === generation.current) setBusy(false); }
  };

  const revoke = async () => {
    ++generation.current;
    setPending(null); setError(null); setBusy(false);
    try { setStatus(await invoke<CapabilityStatus>('revoke_query_capabilities')); }
    catch { setError('Could not revoke workspace access. Quit Murmur to revoke it.'); }
  };

  return <div className="space-y-3 rounded-(--ui-radius-control) border border-outline-variant/30 p-3">
    <p className="text-sm font-medium text-on-surface">Capability profile: {status?.profile === 'trusted_read_only' ? 'Trusted read-only' : status ? 'Restricted' : 'Loading…'}</p>
    <p className="text-xs text-on-surface-variant">Inference may use the provider's network connection in every profile. Restricted grants no project folder or extra tools. Your chosen executable and its own configuration remain trusted software; an empty working directory is not an operating-system sandbox.</p>
    <table className="w-full text-left text-xs text-on-surface-variant">
      <thead><tr><th>Capability</th><th>Trusted read-only, Claude only</th></tr></thead>
      <tbody>
        <tr><td>Local file reads</td><td>Confirmed folder only</td></tr>
        <tr><td>Web / search</td><td>Off</td></tr>
        <tr><td>Commands / file writes</td><td>Off</td></tr>
        <tr><td>MCP / plugins / project instructions</td><td>Off</td></tr>
      </tbody>
    </table>
    <p className="text-xs text-on-surface-variant">Claude may transmit your question, app context, and any files it reads to its inference provider. Choose a folder without secrets. The CLI enforces read scope; Murmur does not sandbox the executable. Expanded access for Codex, Cursor, Grok, and Custom is unsupported.</p>
    {status?.directory && <p className="break-all text-xs text-on-surface">Trusted folder: {status.directory}</p>}
    <p className="text-xs text-on-surface-variant">Consent lasts until you quit Murmur. Changing provider or command requires revocation and new consent. Changes apply to the next query; cancel a running query to stop its existing access.</p>
    <div className="flex flex-wrap gap-2">
      <button type="button" className="rounded-lg bg-surface-container-high px-3 py-2 text-xs" disabled={busy || command.provider !== 'claude' || !status} onClick={() => void choose()}>Choose trusted folder…</button>
      <button type="button" className="rounded-lg bg-surface-container-high px-3 py-2 text-xs" onClick={() => void revoke()}>Revoke access · use Restricted</button>
    </div>
    {pending && <div className="space-y-2 rounded-lg border border-warning p-3">
      <p className="break-all text-xs">Allow Claude to read and transmit files in this canonical folder for this app session? {pending}</p>
      <button type="button" disabled={busy || command.provider !== 'claude'} className="rounded-lg bg-primary px-3 py-2 text-xs text-on-primary" onClick={() => void confirm()}>Confirm read-only folder access</button>
    </div>}
    {error && <p role="alert" className="text-xs text-error">{error}</p>}
  </div>;
}
