import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  AppearanceController,
  ThemeImportPreview,
  ThemeLibraryEntryV1,
} from "../../lib/appearance";
import { AppearanceSettings } from "./AppearanceSettings";

const mocks = vi.hoisted(() => ({
  open: vi.fn(),
  save: vi.fn(),
  controller: null as AppearanceController | null,
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: mocks.open,
  save: mocks.save,
}));
vi.mock("../../lib/hooks/useAppearance", () => ({
  useAppearance: () => {
    if (!mocks.controller)
      throw new Error("Missing appearance test controller");
    return mocks.controller;
  },
}));

const tokens = {
  background: "#f7fafc",
  surface: "#f7fafc",
  "surface-container-low": "#eff4f8",
  "surface-container": "#e9eff3",
  "surface-container-high": "#e2e9ee",
  "surface-container-lowest": "#ffffff",
  "surface-container-highest": "#dbe4e9",
  primary: "#036785",
  "primary-dim": "#005a75",
  "on-primary": "#f3faff",
  "on-surface": "#2b3438",
  "on-surface-variant": "#586065",
  "outline-variant": "#abb3b9",
  error: "#a83836",
  success: "#247a52",
  warning: "#8b5d00",
} as const;

function controller(): AppearanceController {
  return {
    document: {
      version: 1,
      revision: 1,
      mode: "system",
      theme: { version: 1, presetId: "sonic" },
      cache: { version: 1, light: tokens, dark: tokens },
    },
    resolvedAppearance: "light",
    adjustments: [],
    busy: false,
    error: null,
    setMode: vi.fn(async () => {}),
    updateTheme: vi.fn(async () => {}),
    reset: vi.fn(async () => {}),
    previewImport: vi.fn(),
    importFromPath: vi.fn(),
    commitImport: vi.fn(async () => {}),
    exportText: vi.fn(() => "{}"),
    exportToPath: vi.fn(async () => {}),
    library: {
      document: { version: 1, revision: 0, themes: [] },
      error: null,
      saveCurrent: vi.fn(),
      savePreview: vi.fn(),
      install: vi.fn(async () => {}),
      replaceCollection: vi.fn(async () => {}),
      remove: vi.fn(async () => {}),
      previewSelection: vi.fn(),
      exportEntryToPath: vi.fn(async () => {}),
      clearError: vi.fn(),
    },
    clearError: vi.fn(),
  };
}

describe("AppearanceSettings", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(async () => {
    mocks.controller = controller();
    mocks.open.mockReset();
    mocks.save.mockReset();
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(<AppearanceSettings />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it("exposes accessible mode cards and resets to Sonic", async () => {
    const radios = Array.from(
      container.querySelectorAll('[role="radio"]'),
    ) as HTMLButtonElement[];
    expect(
      radios.map(
        (radio) =>
          radio.textContent?.trim().split("Follow")[0].split("Keep")[0],
      ),
    ).toEqual(["System", "Light", "Dark"]);
    expect(radios[0].getAttribute("aria-checked")).toBe("true");

    await act(async () => radios[2].click());
    expect(mocks.controller!.setMode).toHaveBeenCalledWith("dark");

    await act(async () => {
      radios[0].focus();
      radios[0].dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }),
      );
    });
    expect(mocks.controller!.setMode).toHaveBeenCalledWith("light");
    expect(document.activeElement).toBe(radios[1]);

    const createTheme = Array.from(container.querySelectorAll("button")).find(
      (button) => button.textContent?.includes("Create theme"),
    )!;
    await act(async () => createTheme.click());
    const reset = Array.from(container.querySelectorAll("button")).find(
      (button) => button.textContent === "Reset to Sonic",
    )!;
    await act(async () => reset.click());
    expect(mocks.controller!.reset).toHaveBeenCalledOnce();
    expect(container.querySelector('[role="dialog"]')?.textContent).toContain("Start from the colors currently on screen");
    expect(container.textContent).not.toContain("Save current");
    expect(container.textContent).not.toContain("Choose dark style");
  });

  it("consumes mode and reset rejections while preserving hook error UI", async () => {
    vi.mocked(mocks.controller!.setMode).mockRejectedValueOnce(
      new Error("native mode failed"),
    );
    const dark = Array.from(
      container.querySelectorAll('[role="radio"]'),
    ).find((radio) => radio.textContent?.includes("Dark")) as HTMLButtonElement;

    await act(async () => {
      dark.click();
      await Promise.resolve();
    });
    expect(mocks.controller!.setMode).toHaveBeenCalledWith("dark");

    vi.mocked(mocks.controller!.reset).mockRejectedValueOnce(
      new Error("reset failed"),
    );
    const createTheme = Array.from(container.querySelectorAll("button")).find(
      (button) => button.textContent?.includes("Create theme"),
    )!;
    await act(async () => createTheme.click());
    const reset = Array.from(container.querySelectorAll("button")).find(
      (button) => button.textContent === "Reset to Sonic",
    )!;
    await act(async () => {
      reset.click();
      await Promise.resolve();
    });
    expect(mocks.controller!.reset).toHaveBeenCalledOnce();

    mocks.controller!.error =
      "Failed to update appearance: Error: reset failed";
    await act(async () => root.render(<AppearanceSettings />));
    expect(container.querySelector('[role="alert"]')?.textContent).toContain(
      "Failed to update appearance",
    );
  });

  it("validates typed hex and coalesces pointer and keyboard contrast commits", async () => {
    const createTheme = Array.from(container.querySelectorAll("button")).find(
      (button) => button.textContent?.includes("Create theme"),
    )!;
    await act(async () => createTheme.click());
    const accent = container.querySelector(
      "#appearance-accent",
    ) as HTMLInputElement;
    const setInputValue = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!;
    await act(async () => {
      setInputValue.call(accent, "not-a-color");
      accent.dispatchEvent(new Event("input", { bubbles: true }));
      accent.dispatchEvent(new FocusEvent("focusout", { bubbles: true }));
    });
    expect(container.querySelector('[role="alert"]')?.textContent).toContain(
      "six-digit hex",
    );
    expect(mocks.controller!.updateTheme).not.toHaveBeenCalled();

    await act(async () => {
      setInputValue.call(accent, "#123456");
      accent.dispatchEvent(new Event("input", { bubbles: true }));
      accent.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
      );
    });
    expect(mocks.controller!.updateTheme).toHaveBeenCalledWith({
      presetId: "custom",
      accent: "#123456",
    });
    vi.mocked(mocks.controller!.updateTheme).mockClear();

    const contrast = container.querySelector(
      "#appearance-contrast",
    ) as HTMLInputElement;
    await act(async () => {
      setInputValue.call(contrast, "100");
      contrast.dispatchEvent(new Event("input", { bubbles: true }));
      setInputValue.call(contrast, "80");
      contrast.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(mocks.controller!.updateTheme).not.toHaveBeenCalled();

    await act(async () => {
      contrast.dispatchEvent(new Event("pointerup", { bubbles: true }));
    });
    expect(mocks.controller!.updateTheme).toHaveBeenCalledWith({
      presetId: "custom",
      contrast: 80,
    });
    expect(mocks.controller!.updateTheme).toHaveBeenCalledTimes(1);

    await act(async () => {
      setInputValue.call(contrast, "-50");
      contrast.dispatchEvent(new Event("input", { bubbles: true }));
      contrast.dispatchEvent(
        new KeyboardEvent("keyup", { key: "ArrowLeft", bubbles: true }),
      );
      contrast.dispatchEvent(new FocusEvent("focusout", { bubbles: true }));
    });
    expect(mocks.controller!.updateTheme).toHaveBeenLastCalledWith({
      presetId: "custom",
      contrast: -50,
    });
    expect(mocks.controller!.updateTheme).toHaveBeenCalledTimes(2);
  });

  it("installs and applies imported files immediately, without a token preview", async () => {
    const preview: ThemeImportPreview = {
      mode: "dark",
      theme: {
        version: 1,
        presetId: "custom",
        accent: "#123456",
        light: { primary: "#112233" },
        dark: { primary: "#ddeeff", "primary-dim": "#ccddee" },
      },
      light: tokens,
      dark: tokens,
      adjustments: [],
    };
    mocks.open.mockResolvedValue("/tmp/in.json");
    mocks.save.mockResolvedValue("/tmp/out.json");
    vi.mocked(mocks.controller!.importFromPath).mockResolvedValue(preview);
    const savedEntry: ThemeLibraryEntryV1 = {
      version: 1,
      id: "imported-theme",
      label: "Imported theme",
      modes: ["light", "dark"],
      theme: preview.theme,
      source: { kind: "local" as const },
    };
    vi.mocked(mocks.controller!.library.savePreview).mockResolvedValue(savedEntry);
    vi.mocked(mocks.controller!.library.previewSelection).mockReturnValue(preview);
    const clipboardWrite = vi.fn();
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: clipboardWrite },
    });

    const importButton = Array.from(container.querySelectorAll("button")).find(
      (button) => button.textContent?.trim().includes("Import theme"),
    )!;
    await act(async () => importButton.click());
    const importFileButton = Array.from(container.querySelectorAll("button")).find(
      (button) => button.textContent?.trim() === "Import file",
    )!;
    await act(async () => importFileButton.click());
    expect(mocks.controller!.importFromPath).toHaveBeenCalledWith(
      "/tmp/in.json",
    );
    expect(mocks.controller!.library.savePreview).toHaveBeenCalledWith(
      "Imported theme",
      preview,
    );
    expect(mocks.controller!.library.previewSelection).toHaveBeenCalledWith(
      "imported-theme",
    );
    expect(mocks.controller!.commitImport).toHaveBeenCalledWith(preview);
    expect(container.textContent).not.toContain("Import preview");
    expect(container.textContent).not.toContain("token override");

    const createTheme = Array.from(container.querySelectorAll("button")).find(
      (button) => button.textContent?.includes("Create theme"),
    )!;
    await act(async () => createTheme.click());
    const exportButton = Array.from(container.querySelectorAll("button")).find(
      (button) => button.textContent === "Export",
    )!;
    await act(async () => exportButton.click());
    expect(mocks.controller!.exportToPath).toHaveBeenCalledWith(
      "/tmp/out.json",
    );
    expect(clipboardWrite).not.toHaveBeenCalled();
  });

  it("applies a theme when its card is clicked", async () => {
    const preview: ThemeImportPreview = {
      mode: "system",
      theme: { version: 1, presetId: "sonic" },
      light: tokens,
      dark: tokens,
      adjustments: [],
    };
    vi.mocked(mocks.controller!.library.previewSelection).mockReturnValue(preview);

    const sonicCard = container.querySelector(
      'button[aria-label="Use Sonic theme"]',
    ) as HTMLButtonElement;
    await act(async () => sonicCard.click());

    expect(mocks.controller!.library.previewSelection).toHaveBeenCalledWith("sonic");
    expect(mocks.controller!.commitImport).toHaveBeenCalledWith(preview);
  });

  it("collapses imported collections into one fixed-height card with direct radial variants", async () => {
    const collection = { id: "open-vsx:h1dr0n.claude-theme", label: "Claude Theme" };
    const entry = (
      id: string,
      label: string,
      mode: "light" | "dark",
      background: `#${string}`,
    ): ThemeLibraryEntryV1 => ({
      version: 1,
      id,
      label,
      modes: [mode],
      theme: {
        version: 1,
        presetId: "custom",
        [mode]: { background, surface: background },
      },
      source: {
        kind: "open-vsx",
        extensionId: "h1dr0n.claude-theme",
        version: "1.0.0",
        license: "MIT",
      },
      collection,
    });
    const light = entry("claude-light", "Claude Dusk Light", "light", "#f1efe7");
    const dark = entry("claude-dark", "Claude Dusk", "dark", "#1a1d23");
    const midnight = entry("claude-midnight", "Claude Midnight", "dark", "#000000");
    mocks.controller!.library.document = {
      version: 1,
      revision: 1,
      themes: [light, dark, midnight],
    };
    mocks.controller!.document.selection = { light: light.id, dark: midnight.id };
    mocks.controller!.resolvedAppearance = "dark";
    await act(async () => root.render(<AppearanceSettings />));

    expect(container.querySelectorAll('[data-theme-collection="Claude Theme"]')).toHaveLength(1);
    expect(container.querySelectorAll('button[aria-label="Use Claude Theme theme"]')).toHaveLength(1);
    const claudeCard = container.querySelector('[data-theme-collection="Claude Theme"]')!;
    expect(claudeCard.querySelector('button[aria-label*="dark variant"]')?.getAttribute("aria-pressed")).toBe("true");
    expect(claudeCard.querySelector('[role="img"][aria-label^="Light preview"]')).not.toBeNull();
    expect(claudeCard.querySelector('button[aria-label="Use Claude Theme theme"]')?.getAttribute("aria-pressed")).toBe("true");
    expect(container.textContent).not.toContain("Active theme");
    expect(container.textContent).not.toContain("Partly active");
    expect(container.textContent).not.toContain("Choose dark style");
    expect(container.textContent).not.toContain("Claude Midnight (OLED Black)");

    const card = container.querySelector(
      'button[aria-label="Use Claude Theme theme"]',
    ) as HTMLButtonElement;
    await act(async () => card.click());
    expect(mocks.controller!.commitImport).toHaveBeenCalledWith(
      expect.objectContaining({
        selection: { light: light.id, dark: midnight.id },
      }),
    );
  });

  it("reports only adjustments attributable to selected controls", async () => {
    mocks.controller!.document.theme = {
      version: 1,
      presetId: "custom",
      accent: "#123456",
      background: "#fefefe",
    };
    mocks.controller!.adjustments = [
      {
        appearance: "light",
        token: "surface-container",
        reason: "contrast",
        from: "#e9eff3",
        to: "#dde5ea",
      },
      {
        appearance: "light",
        token: "primary",
        reason: "contrast",
        from: "#036785",
        to: "#174f75",
      },
      {
        appearance: "light",
        token: "background",
        reason: "gamut",
        from: "#fefefe",
        to: "#fafafa",
      },
    ];
    await act(async () => root.render(<AppearanceSettings />));

    const createTheme = Array.from(container.querySelectorAll("button")).find(
      (button) => button.textContent?.includes("Create theme"),
    )!;
    await act(async () => createTheme.click());

    const notice = container.querySelector('[role="status"]')!;
    expect(notice.textContent).toContain(
      "Your selected accent #123456 resolves to #174f75",
    );
    expect(notice.textContent).toContain("preserve accessible contrast");
    expect(notice.textContent).toContain(
      "Your selected background #fefefe resolves to #fafafa",
    );
    expect(notice.textContent).toContain("outside the displayable color gamut");
    expect(notice.textContent).not.toContain("surface-container");
    expect(notice.textContent).not.toContain("from #036785");
  });
});
