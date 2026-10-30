import { StrictMode } from 'react';
import type { Root } from 'react-dom/client';
import { hydrateSettingsFromDisk } from '../../lib/settings';
import { DictationPreviewApp } from './DictationPreviewApp';

/** Durable consent must win over a missing or stale WebKit cache before mount. */
export async function mountDictationPreview(root: Root): Promise<void> {
  await hydrateSettingsFromDisk();
  root.render(<StrictMode><DictationPreviewApp /></StrictMode>);
}
