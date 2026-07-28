import {
  createContext,
  createElement,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type PropsWithChildren,
} from 'react';
import { setTheme } from '@tauri-apps/api/app';
import { invoke } from '@tauri-apps/api/core';
import { emit, listen } from '@tauri-apps/api/event';
import {
  applyResolvedTheme,
  createAppearanceDocument,
  exportAppearanceText,
  isNewerAppearanceRevision,
  loadAppearanceDocument,
  nextAppearanceRevision,
  previewAppearanceImport,
  readAppearancePreview,
  resolveTheme,
  sanitizeMode,
  sanitizeTheme,
  writeAppearanceDocument,
  writeAppearanceExport,
  type AppearanceChangeReason,
  type AppearanceChangedEvent,
  type AppearanceController,
  type AppearanceDocumentV1,
  type AppearanceMode,
  type ResolvedAppearance,
  type ThemeAdjustment,
  type ThemeConfigV1,
  type ThemeImportPreview,
} from '../appearance';

export const APPEARANCE_CHANGED_EVENT = 'appearance-changed';
const SYSTEM_DARK_QUERY = '(prefers-color-scheme: dark)';

function systemAppearance(): ResolvedAppearance {
  return typeof window.matchMedia === 'function' && window.matchMedia(SYSTEM_DARK_QUERY).matches
    ? 'dark'
    : 'light';
}

export function concreteAppearance(mode: AppearanceMode): ResolvedAppearance {
  return mode === 'system' ? systemAppearance() : mode;
}

function applyDocument(document: AppearanceDocumentV1): {
  resolvedAppearance: ResolvedAppearance;
  adjustments: ThemeAdjustment[];
} {
  const resolvedAppearance = concreteAppearance(document.mode);
  const resolved = resolveTheme(document.theme, resolvedAppearance);
  applyResolvedTheme(resolved);
  return { resolvedAppearance, adjustments: resolved.adjustments };
}

async function applyNativeTheme(mode: AppearanceMode): Promise<void> {
  await setTheme(mode === 'system' ? null : mode);
}

function validEventPayload(value: unknown): value is AppearanceChangedEvent {
  if (typeof value !== 'object' || value === null) return false;
  const payload = value as Partial<AppearanceChangedEvent>;
  return Number.isSafeInteger(payload.revision)
    && (payload.revision ?? -1) >= 0
    && (
      payload.reason === 'user'
      || payload.reason === 'repair'
      || payload.reason === 'reset'
      || payload.reason === 'import'
    );
}

function useSystemAppearance(
  document: AppearanceDocumentV1,
  onApplied: (appearance: ResolvedAppearance, adjustments: ThemeAdjustment[]) => void,
): void {
  const documentRef = useRef(document);
  documentRef.current = document;
  const onAppliedRef = useRef(onApplied);
  onAppliedRef.current = onApplied;

  useEffect(() => {
    if (document.mode !== 'system' || typeof window.matchMedia !== 'function') return;
    const media = window.matchMedia(SYSTEM_DARK_QUERY);
    const applySystemChange = () => {
      const current = documentRef.current;
      if (current.mode !== 'system') return;
      const applied = applyDocument(current);
      onAppliedRef.current(applied.resolvedAppearance, applied.adjustments);
    };
    media.addEventListener('change', applySystemChange);
    return () => media.removeEventListener('change', applySystemChange);
  }, [document.mode]);
}

/**
 * Main-window-only appearance controller. It is the sole writer/emitter and
 * owns the application-level native theme.
 */
function useMainAppearanceController(): AppearanceController {
  const initialLoadRef = useRef<ReturnType<typeof loadAppearanceDocument> | null>(null);
  if (initialLoadRef.current === null) initialLoadRef.current = loadAppearanceDocument();
  const [document, setDocument] = useState(initialLoadRef.current.document);
  const documentRef = useRef(document);
  documentRef.current = document;
  const initialRuntimeRef = useRef<{
    appearance: ResolvedAppearance;
    adjustments: ThemeAdjustment[];
  } | null>(null);
  if (initialRuntimeRef.current === null) {
    const appearance = concreteAppearance(document.mode);
    initialRuntimeRef.current = {
      appearance,
      adjustments: resolveTheme(document.theme, appearance).adjustments,
    };
  }
  const [resolvedAppearance, setResolvedAppearance] = useState(
    initialRuntimeRef.current.appearance,
  );
  const [adjustments, setAdjustments] = useState(
    initialRuntimeRef.current.adjustments,
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(initialLoadRef.current.error);
  const initializedRef = useRef(false);
  const operationQueueRef = useRef<Promise<void>>(Promise.resolve());
  const pendingOperationsRef = useRef(0);

  const reflect = useCallback((next: AppearanceDocumentV1) => {
    documentRef.current = next;
    setDocument(next);
    const applied = applyDocument(next);
    setResolvedAppearance(applied.resolvedAppearance);
    setAdjustments(applied.adjustments);
  }, []);

  useEffect(() => {
    if (initializedRef.current) return;
    initializedRef.current = true;
    const initial = initialLoadRef.current!;
    let current = initial.document;
    reflect(current);
    if (initial.needsRepair) {
      current = createAppearanceDocument(
        current.mode,
        current.theme,
        nextAppearanceRevision(current.revision),
      );
      try {
        writeAppearanceDocument(current);
        reflect(current);
        void emit<AppearanceChangedEvent>(APPEARANCE_CHANGED_EVENT, {
          revision: current.revision,
          reason: 'repair',
        }).catch((cause) => setError(`Failed to synchronize repaired appearance: ${String(cause)}`));
      } catch (cause) {
        setError(`Failed to repair appearance storage: ${String(cause)}`);
      }
    }
    void applyNativeTheme(current.mode)
      .catch((cause) => setError(`Failed to apply native appearance: ${String(cause)}`));
  }, [reflect]);

  useSystemAppearance(document, (appearance, nextAdjustments) => {
    setResolvedAppearance(appearance);
    setAdjustments(nextAdjustments);
  });

  const commit = useCallback((
    reason: AppearanceChangeReason,
    derive: (current: AppearanceDocumentV1) => Pick<AppearanceDocumentV1, 'mode' | 'theme'>,
  ): Promise<void> => {
    pendingOperationsRef.current += 1;
    setBusy(true);
    const operation = operationQueueRef.current
      .catch(() => {})
      .then(async () => {
        const current = documentRef.current;
        const configuration = derive(current);
        const revision = nextAppearanceRevision(current.revision);
        const eventReason: AppearanceChangeReason = revision <= current.revision
          ? 'repair'
          : reason;
        const next = createAppearanceDocument(
          configuration.mode,
          configuration.theme,
          revision,
        );
        writeAppearanceDocument(next);
        reflect(next);
        await Promise.all([
          applyNativeTheme(next.mode),
          emit<AppearanceChangedEvent>(APPEARANCE_CHANGED_EVENT, {
            revision: next.revision,
            reason: eventReason,
          }),
        ]);
        setError(null);
      });
    const reportedOperation = operation
      .catch((cause) => {
        const message = `Failed to update appearance: ${String(cause)}`;
        setError(message);
        throw new Error(message);
      })
      .finally(() => {
        pendingOperationsRef.current -= 1;
        if (pendingOperationsRef.current === 0) setBusy(false);
      });
    operationQueueRef.current = reportedOperation.catch(() => {});
    return reportedOperation;
  }, [reflect]);

  const setMode = useCallback((mode: AppearanceMode) =>
    commit('user', (current) => ({ mode: sanitizeMode(mode), theme: current.theme })),
  [commit]);

  const updateTheme = useCallback((updates: Partial<ThemeConfigV1>) =>
    commit('user', (current) => ({
      mode: current.mode,
      theme: sanitizeTheme({ ...current.theme, ...updates, version: 1 }),
    })),
  [commit]);

  const reset = useCallback(() =>
    commit('reset', (current) => ({
      mode: current.mode,
      theme: { version: 1, presetId: 'sonic' },
    })),
  [commit]);

  const previewImport = useCallback((text: string) => previewAppearanceImport(text), []);

  const importFromPath = useCallback(async (path: string): Promise<ThemeImportPreview> => {
    setBusy(true);
    try {
      const preview = await readAppearancePreview(
        path,
        (selectedPath) => invoke<string>('read_theme_file', { path: selectedPath }),
      );
      setError(null);
      return preview;
    } catch (cause) {
      const message = `Failed to import theme: ${String(cause)}`;
      setError(message);
      throw new Error(message);
    } finally {
      setBusy(false);
    }
  }, []);

  const commitImport = useCallback((preview: ThemeImportPreview) =>
    commit('import', () => ({
      mode: preview.mode,
      theme: sanitizeTheme(preview.theme),
    })),
  [commit]);

  const exportText = useCallback(() => exportAppearanceText(documentRef.current), []);

  const exportToPath = useCallback(async (path: string): Promise<void> => {
    setBusy(true);
    try {
      await writeAppearanceExport(
        path,
        documentRef.current,
        (selectedPath, contents) =>
          invoke<void>('write_theme_file', { path: selectedPath, contents }),
      );
      setError(null);
    } catch (cause) {
      const message = `Failed to export theme: ${String(cause)}`;
      setError(message);
      throw new Error(message);
    } finally {
      setBusy(false);
    }
  }, []);

  return {
    document,
    resolvedAppearance,
    adjustments,
    busy,
    error,
    setMode,
    updateTheme,
    reset,
    previewImport,
    importFromPath,
    commitImport,
    exportText,
    exportToPath,
    clearError: useCallback(() => setError(null), []),
  };
}

const AppearanceContext = createContext<AppearanceController | null>(null);

export function AppearanceProvider({ children }: PropsWithChildren) {
  const controller = useMainAppearanceController();
  return createElement(AppearanceContext.Provider, { value: controller }, children);
}

export function useAppearance(): AppearanceController {
  const controller = useContext(AppearanceContext);
  if (controller === null) {
    throw new Error('useAppearance must be used within AppearanceProvider.');
  }
  return controller;
}

/**
 * Reader runtime for the log-viewer. It never writes storage or emits events.
 */
export function useAppearanceReader(): void {
  const [document, setDocument] = useState(() => loadAppearanceDocument().document);
  const documentRef = useRef(document);
  documentRef.current = document;

  useEffect(() => {
    applyDocument(documentRef.current);
  }, []);

  useSystemAppearance(document, () => {});

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    const delays = [0, 10, 25, 50, 100, 200];

    const applyLatestIfNewer = (): boolean => {
      if (cancelled) return false;
      const loaded = loadAppearanceDocument().document;
      if (!isNewerAppearanceRevision(documentRef.current.revision, loaded.revision)) return false;
      documentRef.current = loaded;
      setDocument(loaded);
      applyDocument(loaded);
      return true;
    };

    void listen<AppearanceChangedEvent>(APPEARANCE_CHANGED_EVENT, (event) => {
      if (cancelled || !validEventPayload(event.payload)) return;
      if (!isNewerAppearanceRevision(documentRef.current.revision, event.payload.revision)) return;
      void (async () => {
        for (const delay of delays) {
          if (delay > 0) await new Promise((resolve) => window.setTimeout(resolve, delay));
          if (cancelled) return;
          if (applyLatestIfNewer() && documentRef.current.revision >= event.payload.revision) return;
        }
      })();
    }).then((dispose) => {
      if (cancelled) dispose();
      else {
        unlisten = dispose;
        // Close the snapshot/subscribe race: a main-window write can land
        // after our initial load but before the async listener is registered.
        applyLatestIfNewer();
      }
    }).catch(() => {});

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);
}
