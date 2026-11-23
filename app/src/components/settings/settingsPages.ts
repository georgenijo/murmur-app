// Settings page registry. Kept apart from SettingsPanel.tsx so the main
// window can read page ids (command palette, deep links) without pulling the
// whole panel into its entry chunk; App loads SettingsPanel lazily.
import type { SettingsEditorTab } from './SettingsEditorsWindow';

export interface SettingsPageRequest {
  page: string;
  token: number;
  editorTab?: SettingsEditorTab;
  target?: string;
}

export const SETTINGS_CATEGORIES = [
  { id: 'general', label: 'General', icon: 'general' },
  { id: 'recording', label: 'Recording', icon: 'recording' },
  { id: 'customize', label: 'Customize', icon: 'customize' },
  { id: 'delivery', label: 'Delivery', icon: 'delivery' },
  { id: 'meetings', label: 'Meetings', icon: 'meetings' },
  { id: 'text', label: 'Text & vocabulary', icon: 'text' },
  { id: 'ai', label: 'AI & models', icon: 'ai' },
  { id: 'appearance', label: 'Appearance', icon: 'appearance' },
] as const;

const DEFAULT_SETTINGS_PAGE = 'general';

export const SETTINGS_TOOLS = [
  { id: 'performance', label: 'Performance Lab', icon: 'performance' },
  { id: 'diagnostics', label: 'Diagnostics', icon: 'diagnostics' },
] as const;

export const AI_DETAIL_PAGES = ['ai-transcription'] as const;
const GENERAL_DETAIL_PAGES = ['shortcuts'] as const;

/** Coerce a requested page id back to a real page — an unknown id opens
 *  General rather than rendering an empty pane. */
export function resolvePage(page: string | undefined): string {
  if (SETTINGS_CATEGORIES.some((category) => category.id === page)) return page as string;
  if (SETTINGS_TOOLS.some((tool) => tool.id === page)) return page as string;
  if ((AI_DETAIL_PAGES as readonly string[]).includes(page ?? '')) return page as string;
  if ((GENERAL_DETAIL_PAGES as readonly string[]).includes(page ?? '')) return page as string;
  if (page === 'dictation') return 'recording';
  if (page === 'model' || page === 'transcription') return 'ai-transcription';
  if (page === 'benchmark') return 'performance';
  if (page === 'text-vocabulary') return 'text';
  if (page === 'app') return 'general';
  return DEFAULT_SETTINGS_PAGE;
}

export function settingsLatencyView(page: string | undefined): string {
  return `settings.${resolvePage(page)}`;
}
