import { useEffect, useRef, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import type {
  AppearanceMode,
  ThemeAdjustment,
  ThemeConfigV1,
} from "../../lib/appearance";
import { useAppearance } from "../../lib/hooks/useAppearance";
import { CommunityThemeDialog } from "./CommunityThemeDialog";
import { ThemeLibrary } from "./ThemeLibrary";

const MODE_OPTIONS: readonly {
  value: AppearanceMode;
  label: string;
  description: string;
}[] = [
  {
    value: "system",
    label: "System",
    description: "Follow the current macOS appearance.",
  },
  {
    value: "light",
    label: "Light",
    description: "Keep Murmur light regardless of macOS.",
  },
  {
    value: "dark",
    label: "Dark",
    description: "Keep Murmur dark regardless of macOS.",
  },
];

function adjustmentText(
  adjustment: ThemeAdjustment,
  theme: ThemeConfigV1,
): string {
  const reason =
    adjustment.reason === "gamut"
      ? "because it falls outside the displayable color gamut."
      : "to preserve accessible contrast.";
  if (theme.accent && adjustment.token === "primary") {
    return `Your selected accent ${theme.accent} resolves to ${adjustment.to} in ${adjustment.appearance} mode ${reason}`;
  }
  if (theme.background && adjustment.token === "background") {
    return `Your selected background ${theme.background} resolves to ${adjustment.to} in ${adjustment.appearance} mode ${reason}`;
  }
  if (theme.foreground && adjustment.token === "on-surface") {
    return `Your selected foreground ${theme.foreground} resolves to ${adjustment.to} in ${adjustment.appearance} mode ${reason}`;
  }
  return `${adjustment.token} resolves to ${adjustment.to} in ${adjustment.appearance} mode ${reason}`;
}

function ColorControl({
  label,
  description,
  value,
  fallback,
  onCommit,
}: {
  label: string;
  description: string;
  value?: string;
  fallback: string;
  onCommit: (value: string | undefined) => void;
}) {
  const effectiveValue = value ?? fallback;
  const [draft, setDraft] = useState(effectiveValue);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setDraft(effectiveValue);
    setError(null);
  }, [effectiveValue]);

  const commit = (next: string) => {
    const normalized = next.trim().toLowerCase();
    if (!/^#[0-9a-f]{6}$/.test(normalized)) {
      setError("Enter a six-digit hex color, such as #036785.");
      return;
    }
    setError(null);
    setDraft(normalized);
    if (normalized === effectiveValue.toLowerCase()) return;
    onCommit(normalized);
  };

  return (
    <div>
      <div className="flex items-start justify-between gap-4">
        <div>
          <label
            htmlFor={`appearance-${label.toLowerCase()}`}
            className="text-sm font-medium text-on-surface"
          >
            {label}
          </label>
          <p className="mt-1 text-xs text-on-surface-variant">{description}</p>
        </div>
        <input
          aria-label={`${label} color picker`}
          type="color"
          value={/^#[0-9a-fA-F]{6}$/.test(draft) ? draft : fallback}
          onChange={(event) => commit(event.currentTarget.value)}
          className="h-8 w-10 cursor-pointer rounded-md border border-on-surface-variant bg-surface-container-lowest p-0.5"
        />
      </div>
      <div className="mt-2 flex items-center gap-2">
        <input
          id={`appearance-${label.toLowerCase()}`}
          aria-invalid={error ? "true" : undefined}
          value={draft}
          onChange={(event) => setDraft(event.currentTarget.value)}
          onBlur={() => commit(draft)}
          onKeyDown={(event) => {
            if (event.key === "Enter") commit(event.currentTarget.value);
          }}
          spellCheck={false}
          className="w-32 rounded-lg border border-on-surface-variant bg-surface-container-lowest px-2.5 py-1.5 font-mono text-xs text-on-surface outline-none focus-visible:ring-2 focus-visible:ring-primary"
        />
        {value && (
          <button
            type="button"
            onClick={() => onCommit(undefined)}
            className="rounded-md px-2 py-1 text-xs font-medium text-on-surface-variant hover:bg-surface-container"
          >
            Clear override
          </button>
        )}
      </div>
      {error && (
        <p role="alert" className="mt-1 text-xs text-error">
          {error}
        </p>
      )}
    </div>
  );
}

export function AppearanceSettings() {
  const appearance = useAppearance();
  const resolvedTokens =
    appearance.document.cache[appearance.resolvedAppearance];
  const [contrastDraft, setContrastDraft] = useState(
    appearance.document.theme.contrast ?? 0,
  );
  const [localError, setLocalError] = useState<string | null>(null);
  const [communityOpen, setCommunityOpen] = useState(false);
  const contrastDraftRef = useRef(contrastDraft);
  const lastCommittedContrastRef = useRef(
    appearance.document.theme.contrast ?? 0,
  );

  useEffect(() => {
    const contrast = appearance.document.theme.contrast ?? 0;
    setContrastDraft(contrast);
    contrastDraftRef.current = contrast;
    lastCommittedContrastRef.current = contrast;
  }, [appearance.document.theme.contrast]);

  const updateTheme = (patch: Partial<ThemeConfigV1>) => {
    setLocalError(null);
    void Promise.resolve(appearance.updateTheme(patch)).catch((error) =>
      setLocalError(String(error)),
    );
  };

  const runControllerAction = (operation: Promise<void>) => {
    // The appearance hook owns and renders controller failures. Event handlers
    // still consume the rejected promise so a UI-reported failure never also
    // becomes an unhandled rejection.
    void operation.catch(() => {});
  };

  const importTheme = async () => {
    setLocalError(null);
    try {
      const path = await open({
        multiple: false,
        directory: false,
        filters: [
          { name: "Murmur Theme", extensions: ["json", "murmur-theme"] },
        ],
      });
      if (typeof path === "string") {
        const preview = await appearance.importFromPath(path);
        const entry = await appearance.library.savePreview(
          preview.label ?? "Imported theme",
          preview,
        );
        await appearance.commitImport(
          appearance.library.previewSelection(entry.id),
        );
      }
    } catch (error) {
      setLocalError(String(error));
    }
  };

  const exportTheme = async () => {
    setLocalError(null);
    try {
      const path = await save({
        defaultPath: "murmur-theme.json",
        filters: [{ name: "Murmur Theme", extensions: ["json"] }],
      });
      if (typeof path === "string") {
        await appearance.exportToPath(path);
      }
    } catch (error) {
      setLocalError(String(error));
    }
  };

  const selectedAdjustments = appearance.adjustments.filter((adjustment) => {
    if (appearance.document.theme.accent && adjustment.token === "primary") {
      return true;
    }
    if (
      appearance.document.theme.background &&
      adjustment.token === "background"
    ) {
      return true;
    }
    return Boolean(
      appearance.document.theme.foreground &&
        adjustment.token === "on-surface",
    );
  });

  const commitContrast = () => {
    const contrast = contrastDraftRef.current;
    if (contrast === lastCommittedContrastRef.current) return;
    lastCommittedContrastRef.current = contrast;
    updateTheme({ presetId: "custom", contrast });
  };

  return (
    <div className="space-y-5">
      <fieldset>
        <legend className="text-sm font-medium text-on-surface">
          Appearance mode
        </legend>
        <div
          role="radiogroup"
          aria-label="Appearance mode"
          className="mt-2 grid grid-cols-3 gap-2"
        >
          {MODE_OPTIONS.map((option, index) => {
            const selected = appearance.document.mode === option.value;
            return (
              <button
                key={option.value}
                type="button"
                role="radio"
                aria-checked={selected}
                tabIndex={selected ? 0 : -1}
                onClick={() => runControllerAction(appearance.setMode(option.value))}
                onKeyDown={(event) => {
                  const direction =
                    event.key === "ArrowRight" || event.key === "ArrowDown"
                      ? 1
                      : event.key === "ArrowLeft" || event.key === "ArrowUp"
                        ? -1
                        : 0;
                  if (direction === 0) return;
                  event.preventDefault();
                  const nextIndex =
                    (index + direction + MODE_OPTIONS.length) %
                    MODE_OPTIONS.length;
                  const radios = event.currentTarget
                    .closest('[role="radiogroup"]')
                    ?.querySelectorAll<HTMLButtonElement>('[role="radio"]');
                  radios?.[nextIndex]?.focus();
                  runControllerAction(
                    appearance.setMode(MODE_OPTIONS[nextIndex].value),
                  );
                }}
                className={`rounded-xl border p-3 text-left outline-none transition-colors focus-visible:ring-2 focus-visible:ring-primary ${
                  selected
                    ? "border-primary bg-primary/10 text-on-surface"
                    : "border-on-surface-variant bg-surface-container-lowest text-on-surface hover:border-primary hover:bg-surface-container"
                }`}
              >
                <span className="block text-sm font-medium">
                  {option.label}
                </span>
                <span className="mt-1 block text-[11px] leading-4 text-on-surface">
                  {option.description}
                </span>
              </button>
            );
          })}
        </div>
        <p className="mt-2 text-xs text-on-surface-variant">
          Currently rendered as {appearance.resolvedAppearance}. The overlay and
          transform-review glass always stay dark.
        </p>
      </fieldset>

      <ThemeLibrary
        onBrowse={() => setCommunityOpen(true)}
        onImport={() => void importTheme()}
      />

      <div className="rounded-xl border border-on-surface-variant bg-surface-container-lowest p-3">
        <div className="flex items-center justify-between gap-3">
          <div>
            <p className="text-sm font-semibold text-on-surface">Customize current</p>
            <p className="mt-1 text-xs text-on-surface-variant">
              Reset restores Murmur’s exact built-in Sonic palette. Editing
              creates a custom palette from the current theme. Clearing an
              individual color override keeps the theme Custom.
            </p>
          </div>
          <button
            type="button"
            disabled={appearance.busy}
            onClick={() => runControllerAction(appearance.reset())}
            className="rounded-lg border border-on-surface-variant px-3 py-1.5 text-xs font-medium text-on-surface hover:border-primary hover:bg-surface-container disabled:opacity-50"
          >
            Reset to Sonic
          </button>
        </div>
      </div>

      <ColorControl
        label="Accent"
        description="Used for selected controls, progress, and focus."
        value={appearance.document.theme.accent}
        fallback={resolvedTokens.primary}
        onCommit={(accent) => updateTheme({ presetId: "custom", accent })}
      />

      <ColorControl
        label="Background"
        description="Sets the canvas and derives surface ladder."
        value={appearance.document.theme.background}
        fallback={resolvedTokens.background}
        onCommit={(background) =>
          updateTheme({ presetId: "custom", background })
        }
      />

      <ColorControl
        label="Foreground"
        description="Sets primary readable text across derived surfaces."
        value={appearance.document.theme.foreground}
        fallback={resolvedTokens["on-surface"]}
        onCommit={(foreground) =>
          updateTheme({ presetId: "custom", foreground })
        }
      />

      <div>
        <div className="flex items-center justify-between">
          <label
            htmlFor="appearance-contrast"
            className="text-sm font-medium text-on-surface"
          >
            Contrast
          </label>
          <span className="text-xs tabular-nums text-on-surface-variant">
            {contrastDraft > 0 ? "+" : ""}
            {contrastDraft}
          </span>
        </div>
        <input
          id="appearance-contrast"
          type="range"
          min={-100}
          max={100}
          step={1}
          value={contrastDraft}
          onChange={(event) => {
            const contrast = Number(event.currentTarget.value);
            setContrastDraft(contrast);
            contrastDraftRef.current = contrast;
          }}
          onPointerUp={commitContrast}
          onPointerCancel={commitContrast}
          onKeyUp={(event) => {
            if (
              event.key.startsWith("Arrow") ||
              event.key === "Home" ||
              event.key === "End" ||
              event.key === "PageUp" ||
              event.key === "PageDown"
            ) {
              commitContrast();
            }
          }}
          onBlur={commitContrast}
          className="mt-2 h-1.5 w-full cursor-pointer appearance-none rounded-full bg-surface-container-highest accent-primary"
        />
        <p className="mt-1 text-xs text-on-surface-variant">
          Adjusts separation between surfaces while preserving required
          contrast.
        </p>
      </div>

      {selectedAdjustments.length > 0 && (
        <div
          role="status"
          className="rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning"
        >
          <p className="font-medium">Adjusted for accessibility</p>
          <ul className="mt-1 list-disc space-y-0.5 pl-4">
            {selectedAdjustments.map((adjustment, index) => (
              <li key={index}>
                {adjustmentText(adjustment, appearance.document.theme)}
              </li>
            ))}
          </ul>
        </div>
      )}

      <div className="border-t border-outline-variant pt-4">
        <p className="text-sm font-medium text-on-surface">Export current theme</p>
        <p className="mt-1 text-xs text-on-surface-variant">
          Save the active appearance as a portable Murmur theme file.
        </p>
        <div className="mt-3">
          <button
            type="button"
            disabled={appearance.busy}
            onClick={() => void exportTheme()}
            className="rounded-lg border border-on-surface-variant px-3 py-1.5 text-xs font-medium text-on-surface hover:border-primary hover:bg-surface-container disabled:opacity-50"
          >
            Export current
          </button>
        </div>
      </div>

      {(localError || appearance.error) && (
        <p
          role="alert"
          className="rounded-lg border border-error/30 bg-error/10 px-3 py-2 text-xs text-error"
        >
          {localError ?? appearance.error}
        </p>
      )}
      <CommunityThemeDialog open={communityOpen} onClose={() => setCommunityOpen(false)} />
    </div>
  );
}
