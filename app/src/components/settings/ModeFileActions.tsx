import { defaultButtonKeyDown } from '../../lib/defaultButton';
import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '../ui/FormControls';
import { SettingsRow } from './SettingsRow';
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
      if (mounted.current) setState({ kind: 'message', message: 'Custom modes exported.' });
    } catch {
      if (mounted.current) setState({ kind: 'error', message: 'Murmur could not export custom modes. Check the destination and that modes and bindings are valid.' });
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
  const busyNow = state.kind === 'busy';
  return <SettingsRow title="Import and export" description="Share custom modes with their app and site bindings as a JSON file"
    control={<div className="settings-modes-actions">
      <Button size="sm" disabled={busyNow} onClick={() => void importFile()}>Import modes…</Button>
      <Button size="sm" disabled={busyNow} onClick={() => void exportFile()}>Export custom modes…</Button>
    </div>}>
    {busyNow && <p role="status" className="settings-modes-note">Opening mode file…</p>}
    {state.kind === 'message' && <p role="status" className="settings-modes-note">{state.message}</p>}
    {state.kind === 'error' && <p role="alert" className="settings-modes-note settings-modes-error">{state.message}</p>}
    {state.kind === 'preview' && <div data-default-scope onKeyDown={defaultButtonKeyDown} role="region" aria-label="Mode import preview" className="settings-callout settings-modes-import">
      <p>{state.preview.counts.modes} new modes, {state.preview.counts.bindings} app bindings, {state.preview.counts.sites} site rules.</p>
      <p>{state.preview.counts.duplicates} exact duplicates skipped. {state.preview.counts.missingApps} bindings skipped because the app has no local profile.</p>
      <p>Existing app profiles keep their fine-tuning and project roots. Browser-site activation stays {settings.siteModeLookupEnabled ? 'on' : 'off'}.</p>
      {state.preview.kind === 'conflict' && <><p role="alert" className="settings-modes-error">{state.preview.conflicts.length} conflicts. Nothing will be imported.</p><ul className="list-inside list-disc">{state.preview.conflicts.map((conflict, index) => <li key={index}>{conflict}</li>)}</ul></>}
      {stale && <p role="alert" className="settings-modes-error">Modes or bindings changed. Choose the file again to review a fresh preview.</p>}
      <div className="settings-modes-actions">
        <Button size="sm" disabled={stale || state.preview.kind === 'conflict'} onClick={confirm} data-default-button>Confirm import</Button>
        <Button size="sm" onClick={() => setState({ kind: 'idle' })}>Cancel import</Button>
      </div>
    </div>}
  </SettingsRow>;
}
