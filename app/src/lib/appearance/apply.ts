import { MURMUR_TOKEN_NAMES, type ResolvedTheme } from './types';

export function applyResolvedTheme(
  resolved: ResolvedTheme,
  root: HTMLElement = document.documentElement,
): void {
  root.dataset.appearance = resolved.appearance;
  root.style.colorScheme = resolved.colorScheme;
  for (const token of MURMUR_TOKEN_NAMES) {
    root.style.setProperty(`--murmur-${token}`, resolved.tokens[token]);
  }
}
