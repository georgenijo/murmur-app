import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import {
  exportExtension,
  formatHistoryExport,
  historyExportFileName,
  type HistoryEntry,
  type HistoryExportFormat,
} from './history';

export interface PreparedExport {
  contents: string;
  fileName: string;
  extension: string;
}

/** Render the payload and the suggested file name for one export. Pure, so the
 *  format and naming rules stay unit-testable away from the dialog/IO layer. */
export function prepareHistoryExport(
  entries: HistoryEntry[],
  format: HistoryExportFormat,
  at: Date,
): PreparedExport {
  return {
    contents: formatHistoryExport(entries, format, at),
    fileName: historyExportFileName(format, at),
    extension: exportExtension(format),
  };
}

/** Copy the rendered export to the clipboard. Resolves to the entry count. */
export async function copyHistoryExport(
  entries: HistoryEntry[],
  format: HistoryExportFormat,
  at: Date = new Date(),
): Promise<number> {
  const { contents } = prepareHistoryExport(entries, format, at);
  await navigator.clipboard.writeText(contents);
  return entries.length;
}

/**
 * Ask for a destination through the native save dialog and write the export
 * there. Resolves to the written path, or `null` when the user cancels.
 */
export async function saveHistoryExport(
  entries: HistoryEntry[],
  format: HistoryExportFormat,
  at: Date = new Date(),
): Promise<string | null> {
  const { contents, fileName, extension } = prepareHistoryExport(entries, format, at);
  const path = await save({
    defaultPath: fileName,
    filters: [{ name: 'Murmur history', extensions: [extension] }],
  });
  if (typeof path !== 'string' || path.length === 0) return null;
  await invoke('save_text_export', { path, contents });
  return path;
}
