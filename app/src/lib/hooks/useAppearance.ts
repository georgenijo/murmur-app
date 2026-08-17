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
import { emit } from '@tauri-apps/api/event';
import {
  applyResolvedTheme,
  appearanceSelection,
  availableThemeId,
  composeThemeSelection,
  createAppearanceDocument,
  emptyThemeLibrary,
  exportAppearanceText,
  exportThemeLibraryEntryText,
  installThemeLibraryEntries,
  loadAppearanceDocument,
  loadThemeLibrary,
  makeLocalThemeEntry,
  nextAppearanceRevision,
  previewAppearanceImport,
  previewThemeLibrarySelection,
  readAppearancePreview,
  removeThemeLibraryEntries,
  replaceThemeLibraryCollection,
  resolveTheme,
  sanitizeMode,
  sanitizeTheme,
  writeAppearanceDocument,
  writeAppearanceExport,
  writeThemeLibrary,
  type AppearanceChangeReason,
  type AppearanceChangedEvent,
  type AppearanceController,
  type AppearanceDocumentV1,
  type AppearanceMode,
  type AppearanceSelectionV1,
  type ResolvedAppearance,
  type ThemeAdjustment,
  type ThemeConfigV1,
  type ThemeImportPreview,
  type ThemeLibraryDocumentV1,
  type ThemeLibraryEntryV1,
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
  const initialLibraryRef = useRef<ReturnType<typeof loadThemeLibrary> | null>(null);
  if (initialLibraryRef.current === null) initialLibraryRef.current = loadThemeLibrary();
  const [libraryDocument, setLibraryDocument] = useState(
    initialLibraryRef.current.status === 'ready'
      ? initialLibraryRef.current.document
      : emptyThemeLibrary(),
  );
  const libraryDocumentRef = useRef(libraryDocument);
  libraryDocumentRef.current = libraryDocument;
  const [libraryError, setLibraryError] = useState<string | null>(
    initialLibraryRef.current.status === 'unavailable' ? initialLibraryRef.current.error : null,
  );
  const initializedRef = useRef(false);
  const libraryInitializedRef = useRef(false);
  const selectionReconciledRef = useRef(false);
  const operationQueueRef = useRef<Promise<void>>(Promise.resolve());
  const libraryOperationQueueRef = useRef<Promise<void>>(Promise.resolve());
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
        current.selection,
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

  useEffect(() => {
    if (libraryInitializedRef.current) return;
    libraryInitializedRef.current = true;
    const initial = initialLibraryRef.current!;
    if (initial.status !== 'ready' || !initial.needsRepair) return;
    const repaired: ThemeLibraryDocumentV1 = {
      ...initial.document,
      revision: nextAppearanceRevision(initial.document.revision),
    };
    try {
      writeThemeLibrary(repaired);
      libraryDocumentRef.current = repaired;
      setLibraryDocument(repaired);
    } catch (cause) {
      setLibraryError(`Failed to repair the theme library: ${String(cause)}`);
    }
  }, []);

  useSystemAppearance(document, (appearance, nextAdjustments) => {
    setResolvedAppearance(appearance);
    setAdjustments(nextAdjustments);
  });

  const commit = useCallback((
    reason: AppearanceChangeReason,
    derive: (
      current: AppearanceDocumentV1,
    ) => Pick<AppearanceDocumentV1, 'mode' | 'theme' | 'selection'>,
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
          configuration.selection,
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

  useEffect(() => {
    if (selectionReconciledRef.current) return;
    selectionReconciledRef.current = true;
    const current = documentRef.current;
    const selection = appearanceSelection(current);
    const library = libraryDocumentRef.current;
    const available = (owner: string, appearance: ResolvedAppearance) =>
      owner === 'sonic'
      || owner === 'custom'
      || library.themes.some((entry) => entry.id === owner && entry.modes.includes(appearance));
    const nextSelection: AppearanceSelectionV1 = {
      light: available(selection.light, 'light') ? selection.light : 'sonic',
      dark: available(selection.dark, 'dark') ? selection.dark : 'sonic',
    };
    const theme = composeThemeSelection(current, library, nextSelection);
    if (
      JSON.stringify(theme) === JSON.stringify(current.theme)
      && JSON.stringify(nextSelection) === JSON.stringify(current.selection ?? appearanceSelection(current))
    ) return;
    void commit('repair', (document) => ({
      mode: document.mode,
      theme,
      selection: nextSelection,
    })).catch(() => {});
  }, [commit]);

  const setMode = useCallback((mode: AppearanceMode) =>
    commit('user', (current) => ({
      mode: sanitizeMode(mode),
      theme: current.theme,
      selection: current.selection,
    })),
  [commit]);

  const updateTheme = useCallback((updates: Partial<ThemeConfigV1>) =>
    commit('user', (current) => {
      const selection = appearanceSelection(current);
      const hasCompiledOverrides = Boolean(current.theme.light || current.theme.dark);
      const isLibraryOwned = selection.light !== 'custom' || selection.dark !== 'custom';
      const rendered = current.cache[concreteAppearance(current.mode)];
      const editableBase: ThemeConfigV1 = hasCompiledOverrides || isLibraryOwned
        ? {
            version: 1,
            presetId: 'custom',
            accent: rendered.primary,
            background: rendered.background,
            foreground: rendered['on-surface'],
            ...(current.theme.contrast === undefined ? {} : { contrast: current.theme.contrast }),
          }
        : current.theme;
      return {
        mode: current.mode,
        theme: sanitizeTheme({ ...editableBase, ...updates, version: 1, presetId: 'custom' }),
        selection: { light: 'custom', dark: 'custom' },
      };
    }),
  [commit]);

  const reset = useCallback(() =>
    commit('reset', (current) => ({
      mode: current.mode,
      theme: { version: 1, presetId: 'sonic' },
      selection: { light: 'sonic', dark: 'sonic' },
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
      selection: preview.selection ?? { light: 'custom', dark: 'custom' },
    })),
  [commit]);

  const runLibraryMutation = useCallback(<T,>(
    operation: (current: ThemeLibraryDocumentV1) => { document: ThemeLibraryDocumentV1; value: T },
  ): Promise<T> => {
    pendingOperationsRef.current += 1;
    setBusy(true);
    let resolveResult!: (value: T | PromiseLike<T>) => void;
    let rejectResult!: (reason?: unknown) => void;
    const result = new Promise<T>((resolve, reject) => {
      resolveResult = resolve;
      rejectResult = reject;
    });
    const queued = libraryOperationQueueRef.current
      .catch(() => {})
      .then(() => {
        const output = operation(libraryDocumentRef.current);
        libraryDocumentRef.current = output.document;
        setLibraryDocument(output.document);
        setLibraryError(null);
        resolveResult(output.value);
      })
      .catch((cause) => {
        const message = cause instanceof Error ? cause.message : String(cause);
        setLibraryError(message);
        rejectResult(new Error(message));
      })
      .finally(() => {
        pendingOperationsRef.current -= 1;
        if (pendingOperationsRef.current === 0) setBusy(false);
      });
    libraryOperationQueueRef.current = queued;
    return result;
  }, []);

  const installLibraryEntries = useCallback((entries: readonly ThemeLibraryEntryV1[]) =>
    runLibraryMutation((current) => ({
      document: installThemeLibraryEntries(current.revision, entries),
      value: undefined,
    })),
  [runLibraryMutation]);

  const saveCurrentTheme = useCallback((label: string) =>
    runLibraryMutation((current) => {
      const occupiedIds = new Set(current.themes.map((theme) => theme.id));
      const entry = makeLocalThemeEntry(
        availableThemeId(label, occupiedIds),
        label,
        documentRef.current.theme,
      );
      return {
        document: installThemeLibraryEntries(current.revision, [entry]),
        value: entry,
      };
    }),
  [runLibraryMutation]);

  const savePreviewTheme = useCallback((label: string, preview: ThemeImportPreview) =>
    runLibraryMutation((current) => {
      const occupiedIds = new Set(current.themes.map((theme) => theme.id));
      const entry = makeLocalThemeEntry(
        availableThemeId(label, occupiedIds),
        label,
        preview.theme,
        preview.modes ?? ['light', 'dark'],
      );
      return {
        document: installThemeLibraryEntries(current.revision, [entry]),
        value: entry,
      };
    }),
  [runLibraryMutation]);

  const replaceLibraryCollection = useCallback((
    collectionId: string,
    entries: readonly ThemeLibraryEntryV1[],
    expectedCollection: readonly ThemeLibraryEntryV1[],
  ) => runLibraryMutation((current) => ({
    document: replaceThemeLibraryCollection(
      current.revision,
      collectionId,
      entries,
      expectedCollection,
    ),
    value: undefined,
  })).then(async () => {
    const selection = appearanceSelection(documentRef.current);
    const previousIds = new Set(expectedCollection.map((entry) => entry.id));
    if (!previousIds.has(selection.light) && !previousIds.has(selection.dark)) return;
    const replacementIds = new Set(entries.map((entry) => entry.id));
    const nextSelection: AppearanceSelectionV1 = {
      light: previousIds.has(selection.light) && !replacementIds.has(selection.light)
        ? 'sonic'
        : selection.light,
      dark: previousIds.has(selection.dark) && !replacementIds.has(selection.dark)
        ? 'sonic'
        : selection.dark,
    };
    const theme = composeThemeSelection(
      documentRef.current,
      libraryDocumentRef.current,
      nextSelection,
    );
    await commit('library', (current) => ({ mode: current.mode, theme, selection: nextSelection }));
  }), [commit, runLibraryMutation]);

  const removeLibraryThemes = useCallback(async (themeIds: readonly string[]) => {
    const removed = new Set(themeIds);
    const current = documentRef.current;
    const selection = appearanceSelection(current);
    const nextSelection: AppearanceSelectionV1 = {
      light: removed.has(selection.light) ? 'sonic' : selection.light,
      dark: removed.has(selection.dark) ? 'sonic' : selection.dark,
    };
    await runLibraryMutation((library) => ({
      document: removeThemeLibraryEntries(library.revision, themeIds),
      value: undefined,
    }));
    if (nextSelection.light !== selection.light || nextSelection.dark !== selection.dark) {
      const theme = composeThemeSelection(current, libraryDocumentRef.current, nextSelection);
      await commit('library', (document) => ({
        mode: document.mode,
        theme,
        selection: nextSelection,
      }));
    }
  }, [commit, runLibraryMutation]);

  const previewLibrarySelection = useCallback((
    themeId: string,
    appearance?: ResolvedAppearance,
  ) => previewThemeLibrarySelection(
    documentRef.current,
    libraryDocumentRef.current,
    themeId,
    appearance,
  ), []);

  const exportLibraryEntryToPath = useCallback(async (
    entry: ThemeLibraryEntryV1,
    path: string,
  ): Promise<void> => {
    setBusy(true);
    try {
      await invoke<void>('write_theme_file', {
        path,
        contents: exportThemeLibraryEntryText(entry),
      });
      setLibraryError(null);
    } catch (cause) {
      const message = `Failed to export theme: ${String(cause)}`;
      setLibraryError(message);
      throw new Error(message);
    } finally {
      setBusy(false);
    }
  }, []);

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
    library: {
      document: libraryDocument,
      error: libraryError,
      saveCurrent: saveCurrentTheme,
      savePreview: savePreviewTheme,
      install: installLibraryEntries,
      replaceCollection: replaceLibraryCollection,
      remove: removeLibraryThemes,
      previewSelection: previewLibrarySelection,
      exportEntryToPath: exportLibraryEntryToPath,
      clearError: useCallback(() => setLibraryError(null), []),
    },
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
