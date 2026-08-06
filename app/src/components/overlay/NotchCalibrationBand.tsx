import { useEffect, useRef, useState } from 'react';
import type { OverlayGeometry } from '../../lib/overlayGeometry';

interface NotchCalibrationBandProps {
  geometry: OverlayGeometry;
  active: boolean;
  onCommit: (offset: number) => void;
  onCancel: () => void;
}

/** Full-width drag surface used to calibrate the overlay's native Y position. */
export function NotchCalibrationBand({
  geometry,
  active,
  onCommit,
  onCancel,
}: NotchCalibrationBandProps) {
  const [dragOffset, setDragOffset] = useState(0);
  const dragOffsetRef = useRef(0);
  const startYRef = useRef(0);
  const bandRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (active) bandRef.current?.focus();
    else {
      dragOffsetRef.current = 0;
      setDragOffset(0);
    }
  }, [active]);

  if (!active) return null;

  return (
    <div
      ref={bandRef}
      role="dialog"
      aria-label="Calibrate overlay position"
      tabIndex={0}
      onKeyDown={(event) => {
        if (event.key === 'Escape') onCancel();
        if (event.key === 'Enter') onCommit(dragOffsetRef.current);
      }}
      onPointerDown={(event) => {
        event.stopPropagation();
        startYRef.current = event.clientY - dragOffset;
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onPointerMove={(event) => {
        event.stopPropagation();
        if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
        const next = Math.max(-12, Math.min(48, event.clientY - startYRef.current));
        dragOffsetRef.current = next;
        setDragOffset(next);
      }}
      onPointerUp={(event) => {
        event.stopPropagation();
        if (event.currentTarget.hasPointerCapture(event.pointerId)) {
          event.currentTarget.releasePointerCapture(event.pointerId);
        }
        onCommit(dragOffsetRef.current);
      }}
      onClick={(event) => event.stopPropagation()}
      style={{
        position: 'absolute',
        left: 0,
        width: geometry.windowW,
        top: 0,
        height: geometry.collapsedH,
        transform: `translateY(${dragOffset}px)`,
        background: 'rgba(146, 219, 254, 0.16)',
        border: '1px dashed rgba(146, 219, 254, 0.8)',
        borderRadius: '0 0 12px 12px',
        color: '#dbe4e9',
        cursor: 'ns-resize',
        zIndex: 20,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        fontSize: 10,
        fontWeight: 700,
        letterSpacing: '0.02em',
        backdropFilter: 'blur(12px)',
      }}
    >
      ↕ Drag to position
    </div>
  );
}
