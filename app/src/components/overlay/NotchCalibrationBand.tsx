import type { OverlayGeometry } from '../../lib/overlayGeometry';

interface NotchCalibrationBandProps {
  geometry: OverlayGeometry;
}

/**
 * Dev-build-only visual aid: draws a translucent neon-magenta band over the
 * exact zone the overlay believes the physical notch occupies — the island's
 * center section between the two wings, full notch height (0 to
 * `geometry.collapsedH`). On real notched hardware this band must render
 * fully hidden behind the physical notch; any visible magenta means the
 * geometry or window positioning is wrong.
 *
 * Gated by `import.meta.env.DEV`, the same mechanism the main window's "Dev"
 * banner uses (see App.tsx). That flag is only true under the Vite dev
 * server (`npm run tauri dev`) — it is false in a bundled debug .app, since
 * `vite build` (which produces the bundled frontend for every Tauri build,
 * debug or release) always sets it to false. The band therefore shows only
 * during `tauri dev`, matching the existing dev banner's behavior exactly.
 */
export function NotchCalibrationBand({ geometry }: NotchCalibrationBandProps) {
  if (!import.meta.env.DEV) return null;

  return (
    <div
      aria-hidden="true"
      style={{
        position: 'absolute',
        left: geometry.wingW,
        right: geometry.wingW,
        top: 0,
        height: geometry.collapsedH,
        background: 'rgba(255, 0, 200, 0.5)',
        outline: '1px solid rgba(255, 0, 200, 0.9)',
        pointerEvents: 'none',
        zIndex: 10,
      }}
    />
  );
}
