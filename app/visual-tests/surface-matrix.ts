// Shared dimensions keep surface × state × appearance × width cases explicit.
// Pass a surface's native size when it is a fixed-size auxiliary window.
export const desktop = { width: 880, height: 720 };
export const compact = { width: 720, height: 560 };
export const nativeWindow = { width: 1120, height: 820 };

export function surfaceCases(sizes = [desktop, compact]) {
  return sizes.flatMap(size => (['light', 'dark'] as const).map(appearance => ({
    ...size,
    appearance,
    label: `${appearance} ${size.width}x${size.height}`,
  })));
}
