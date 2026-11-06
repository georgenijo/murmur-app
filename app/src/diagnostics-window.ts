import { applyResolvedTheme, loadAppearanceDocument, resolveTheme } from './lib/appearance';
import { PRESENTATION_CHANGED } from './lib/presentationStorage';

// Diagnostics follows the shared theme read-only, including scheme changes.
// Its DOM mutation then drives the same native background bridge as main.
const media = window.matchMedia('(prefers-color-scheme: dark)');
const applyAppearance = () => {
  const { document } = loadAppearanceDocument();
  const appearance = document.mode === 'system' ? (media.matches ? 'dark' : 'light') : document.mode;
  applyResolvedTheme(resolveTheme(document.theme, appearance));
};
window.addEventListener('storage', applyAppearance);
window.addEventListener(PRESENTATION_CHANGED, applyAppearance);
media.addEventListener('change', applyAppearance);
applyAppearance();
