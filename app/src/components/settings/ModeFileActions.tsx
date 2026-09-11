import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open, save } from '@tauri-apps/plugin-dialog';
import {
  commitModeImport, exportModeFile, modeSettingsSnapshot, parseModeFile, previewModeImport,
  type ModeExchangeSettings, type ModeImportPreview,
} from '../../lib/modeExchange';

type ExchangeState =
  | { kind: 'idle' }
  | { kind: 'busy' }
  | { kind: 'preview'; preview: ModeImportPreview }
  | { kind: 'message'; message: string }
  | { kind: 'error'; message: string };

export function ModeFileActions({ settings, onChange }: {
  settings: ModeExchangeSettings;
  onChange: (next: ModeExchangeSettings) => void;
}) {
  const [state, setState] = useState<ExchangeState>({ kind: 'idle' });
  const current = useRef(settings);
  current.current = settings;
  const mounted = useRef(true);
  const busy = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  const importFile = async () => {
    if (busy.current) return;
    busy.current = true;
    setState({ kind: 'busy' });
    try {
      const path = await open({ multiple: false, directory: false, filters: [{ name: 'Murmur Modes', extensions: ['json'] }] });
      if (!mounted.current) return;
      if (typeof path !== 'string') { setState({ kind: 'idle' }); return; }
      const contents = await invoke<string>('read_modes_file', { path });
      if (!mounted.current) return;
      const file = parseModeFile(contents);
      setState({ kind: 'preview', preview: previewModeImport(file, current.current) });
    } catch (error) {
      if (mounted.current) setState({ kind: 'error', message: error instanceof Error ? error.message : 'Murmur could not read the Mode file. Choose a regular UTF-8 JSON file, 256 KiB or smaller.' });
    } finally { busy.current = false; }
  };
  const exportFile = async () => {
    if (busy.current) return;
    busy.current = true;
    setState({ kind: 'busy' });
    try {
      const contents = exportModeFile(current.current);
      const path = await save({ defaultPath: 'murmur-modes.json', filters: [{ name: 'Murmur Modes', extensions: ['json'] }] });
      if (!mounted.current) return;
      if (path === null) { setState({ kind: 'idle' }); return; }
      await invoke('save_text_export', { path, contents });
      if (mounted.current) setState({ kind: 'message', message: 'Custom Modes exported.' });
    } catch {
      if (mounted.current) setState({ kind: 'error', message: 'Murmur could not export custom Modes. Check the destination and that Modes and bindings are valid.' });
    } finally { busy.current = false; }
  };
  const confirm = () => {
    if (state.kind !== 'preview' || busy.current) return;
    busy.current = true;
    try {
      const next = commitModeImport(state.preview, current.current);
      onChange(next);
      setState({ kind: 'message', message: 'Import applied. Browser-site activation preference was preserved.' });
    } catch (error) {
      setState({ kind: 'error', message: error instanceof Error ? error.message : 'Import could not be applied.' });
    } finally { busy.current = false; }
  };
  const stale = state.kind === 'preview' && state.preview.baseline !== modeSettingsSnapshot(settings);
  return <div className="mt-3 space-y-2">
    <div className="flex flex-wrap gap-2">
      <button type="button" disabled={state.kind === 'busy'} onClick={() => void exportFile()} className="rounded-lg bg-surface-container-high px-3 py-2 text-xs disabled:opacity-50">Export custom Modes</button>
      <button type="button" disabled={state.kind === 'busy'} onClick={() => void importFile()} className="rounded-lg bg-surface-container-high px-3 py-2 text-xs disabled:opacity-50">Import Modes</button>
    </div>
    {state.kind === 'busy' && <p role="status" className="text-xs text-on-surface-variant">Opening Mode file…</p>}
    {state.kind === 'message' && <p role="status" className="text-xs text-on-surface-variant">{state.message}</p>}
    {state.kind === 'error' && <p role="alert" className="text-xs text-error">{state.message}</p>}
    {state.kind === 'preview' && <div role="region" aria-label="Mode import preview" className="space-y-2 rounded-lg border border-outline-variant p-3 text-xs">
      <p>{state.preview.counts.modes} new Modes, {state.preview.counts.bindings} app bindings, {state.preview.counts.sites} site rules.</p>
      <p>{state.preview.counts.duplicates} exact duplicates skipped. {state.preview.counts.missingApps} bindings skipped because the app has no local profile.</p>
      <p>Existing app profiles keep their fine-tuning and project roots. Browser-site activation stays {settings.siteModeLookupEnabled ? 'on' : 'off'}.</p>
      {state.preview.kind === 'conflict' && <><p role="alert" className="text-error">{state.preview.conflicts.length} conflicts. Nothing will be imported.</p><ul className="list-inside list-disc">{state.preview.conflicts.map((conflict, index) => <li key={index}>{conflict}</li>)}</ul></>}
      {stale && <p role="alert" className="text-error">Modes or bindings changed. Choose the file again to review a fresh preview.</p>}
      <div className="flex gap-2">
        <button type="button" disabled={stale || state.preview.kind === 'conflict'} onClick={confirm} className="rounded-lg bg-primary px-3 py-2 font-semibold text-on-primary disabled:opacity-50">Confirm import</button>
        <button type="button" onClick={() => setState({ kind: 'idle' })} className="rounded-lg bg-surface-container-high px-3 py-2">Cancel import</button>
      </div>
    </div>}
  </div>;
}
