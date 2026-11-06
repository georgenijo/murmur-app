import { useEffect } from 'react';
import { PRESENTATION_CHANGED } from '../presentationStorage';
import { applyResolvedTheme, loadAppearanceDocument, resolveTheme } from '../appearance';

/** Follow the shared appearance without writing settings or changing native focus. */
export function usePopoverAppearance() {
  useEffect(() => {
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    const apply = () => {
      const { document } = loadAppearanceDocument();
      const mode = document.mode === 'system' ? (media.matches ? 'dark' : 'light') : document.mode;
      applyResolvedTheme(resolveTheme(document.theme, mode));
    };
    apply();
    window.addEventListener('storage', apply);
    window.addEventListener(PRESENTATION_CHANGED, apply);
    media.addEventListener('change', apply);
    return () => {
      window.removeEventListener('storage', apply);
      window.removeEventListener(PRESENTATION_CHANGED, apply);
      media.removeEventListener('change', apply);
    };
  }, []);
}
