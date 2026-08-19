import { invoke } from '@tauri-apps/api/core';
import type { HistoryStageResult } from './history';

export interface HistoryReformatResult {
  text: string;
  modeId: string;
  stages: HistoryStageResult[];
}

export function reformatHistoryText(rawText: string, modeId: string): Promise<HistoryReformatResult> {
  return invoke('reformat_history_text', { rawText, modeId });
}
