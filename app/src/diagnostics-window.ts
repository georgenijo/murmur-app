import { startNativeWindowLifecycle } from './lib/windowLifecycle';
import { applyResolvedTheme, loadAppearanceDocument, resolveTheme } from './lib/appearance';

// Diagnostics follows the shared theme read-only, including scheme changes.
// Its DOM mutation then drives the same native background bridge as main.
const media = window.matchMedia('(prefers-color-scheme: dark)');
const applyAppearance = () => {
  const { document } = loadAppearanceDocument();
  const appearance = document.mode === 'system' ? (media.matches ? 'dark' : 'light') : document.mode;
  applyResolvedTheme(resolveTheme(document.theme, appearance));
};
window.addEventListener('storage', applyAppearance);
media.addEventListener('change', applyAppearance);
applyAppearance();

// Diagnostics is created hidden and opened on demand. Its own webview owns its
// zoom and measurements; readiness must never open it at application startup.
const root = document.getElementById('root')!;
const observer = new MutationObserver(() => {
  if (!root.childElementCount) return;
  observer.disconnect();
  startNativeWindowLifecycle(false);
});
observer.observe(root, { childList: true });
if (root.childElementCount) {
  observer.disconnect();
  startNativeWindowLifecycle(false);
}
