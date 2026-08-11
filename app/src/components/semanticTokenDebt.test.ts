import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import * as ts from "typescript";

function productionSources(directory: string): string[] {
  return readdirSync(directory).flatMap((entry: string) => {
    const path = join(directory, entry);
    if (statSync(path).isDirectory()) return productionSources(path);
    return /\.(?:ts|tsx)$/.test(path) && !/\.test\./.test(path) ? [path] : [];
  });
}

const sources = productionSources("./src");
const alwaysDarkPrefixes = [
  "components/overlay/",
  "components/transform-review/",
  "components/query-review/",
] as const;
const alwaysDarkFiles = new Set(["components/OverlayWidget.tsx"]);
const scrimAllowlist = new Set([
  "components/AboutModal.tsx",
  "components/CommandPalette.tsx",
  "components/UpdateModal.tsx",
  "components/WhatsNewModal.tsx",
  "components/history/CorrectAndTeachDialog.tsx",
  "components/settings/KnowledgeEditorModal.tsx",
  "components/settings/KnowledgeManager.tsx",
  "components/settings/VocabTermsModal.tsx",
  "components/settings/VoiceCommandsManager.tsx",
]);
const chartColorAllowlist: Record<string, ReadonlySet<string>> = {
  "components/log-viewer/PerformanceView.tsx": new Set([
    "#d97706",
    "#7c3aed",
    "#2563eb",
  ]),
  "components/log-viewer/RunDetail.tsx": new Set([
    "rgba(120,113,108,0.10)",
  ]),
};

function sourceName(path: string): string {
  return relative("./src", path);
}

describe("semantic theme token debt gate", () => {
  it("keeps palette utilities out of themed production UI", () => {
    const paletteUtility =
      /(?:dark:)?(?:bg|text|border|ring|outline|from|to|via|divide|accent)-(?:stone|amber|red|emerald|green|blue|cyan|sky|slate|gray|zinc|neutral|yellow|orange|teal|purple|violet|white)(?:-[0-9]+)?(?:\/(?:[0-9]+|\[[^\]]+\]))?/g;
    const findings: string[] = [];

    for (const path of sources) {
      const name = sourceName(path);
      if (
        alwaysDarkFiles.has(name) ||
        alwaysDarkPrefixes.some((prefix) => name.startsWith(prefix))
      )
        continue;
      const matches = readFileSync(path, "utf8").match(paletteUtility) ?? [];
      for (const match of matches) findings.push(`${name}: ${match}`);
    }

    expect(findings).toEqual([]);
  });

  it("limits hardcoded visual colors to documented glass, scrim, and chart exceptions", () => {
    const findings: string[] = [];
    for (const path of sources) {
      const name = sourceName(path);
      const source = readFileSync(path, "utf8");
      const isGlass =
        alwaysDarkFiles.has(name) ||
        alwaysDarkPrefixes.some((prefix) => name.startsWith(prefix));
      const withoutComments = source
        .replace(/\/\*[\s\S]*?\*\/|\/\/.*$/gm, "")
        .replace(/&#[0-9]+;/g, "");
      const colors =
        withoutComments.match(
          /#[0-9a-fA-F]{3,8}\b|rgba?\([^)]*\)|bg-black\/\d+/g,
        ) ?? [];

      for (const color of colors) {
        if (isGlass || chartColorAllowlist[name]?.has(color)) continue;
        if (scrimAllowlist.has(name) && /^bg-black\/(?:50|55)$/.test(color))
          continue;
        if (name.startsWith("lib/appearance/")) continue;
        if (
          name === "components/settings/AppearanceSettings.tsx" &&
          color === "#036785"
        )
          continue;
        findings.push(`${name}: ${color}`);
      }
    }
    expect(findings).toEqual([]);
  });

  it("rejects duplicate or conflicting semantic color utilities in static class strings", () => {
    const findings: string[] = [];
    const colorUtility =
      /^(?:(?:dark|hover|focus|focus-visible|active|disabled|group-hover|placeholder):)*(?:bg|text|border|ring|outline|from|to|via|divide|accent)-(?:background|surface(?:-container(?:-(?:low|high|lowest|highest))?)?|primary(?:-dim)?|on-primary|on-surface(?:-variant)?|outline-variant|error|success|warning)(?:\/[^\s]+)?$/;

    for (const path of sources) {
      const name = sourceName(path);
      if (
        alwaysDarkFiles.has(name) ||
        alwaysDarkPrefixes.some((prefix) => name.startsWith(prefix))
      ) {
        continue;
      }
      const source = readFileSync(path, "utf8");
      for (const literal of source.matchAll(/(["'])([^"'`\n]*)\1/g)) {
        const tokens = literal[2].split(/\s+/).filter((token) =>
          colorUtility.test(token),
        );
        const bySlot = new Map<string, string>();
        for (const token of tokens) {
          const withoutOpacity = token.replace(/\/[^\s]+$/, "");
          const parts = withoutOpacity.split(":");
          const base = parts.pop()!;
          const property = base.slice(0, base.indexOf("-"));
          // Semantic tokens already adapt to the resolved mode. Treat `dark:`
          // as the same selector slot so redundant or conflicting overrides fail.
          const selectors = parts.filter((selector) => selector !== "dark");
          const slot = `${selectors.join(":")}:${property}`;
          const previous = bySlot.get(slot);
          if (previous) findings.push(`${name}: ${previous} + ${token}`);
          else bySlot.set(slot, token);
        }
      }
    }

    expect(findings).toEqual([]);
  });

  it("rejects unsupported solid-error and on-primary status pairs", () => {
    const findings: string[] = [];
    for (const path of sources) {
      const name = sourceName(path);
      if (
        alwaysDarkFiles.has(name) ||
        alwaysDarkPrefixes.some((prefix) => name.startsWith(prefix))
      ) {
        continue;
      }
      const source = readFileSync(path, "utf8");
      for (const literal of source.matchAll(/(["'])([^"'`\n]*)\1/g)) {
        const tokens = literal[2].split(/\s+/);
        if (tokens.includes("bg-error") && tokens.includes("text-on-primary")) {
          findings.push(`${name}: bg-error + text-on-primary`);
        }
      }
    }
    expect(findings).toEqual([]);
  });

  it("keeps status foregrounds opaque and primary control hovers contrast-safe", () => {
    const findings: string[] = [];
    const translucentStatusText =
      /\btext-(?:error|success|warning)\/[^\s"'`]+/g;

    for (const path of sources) {
      const name = sourceName(path);
      if (
        alwaysDarkFiles.has(name) ||
        alwaysDarkPrefixes.some((prefix) => name.startsWith(prefix))
      ) {
        continue;
      }
      const source = readFileSync(path, "utf8");
      for (const match of source.match(translucentStatusText) ?? []) {
        findings.push(`${name}: translucent status foreground ${match}`);
      }
      for (const literal of source.matchAll(/(["'])([^"'`\n]*)\1/g)) {
        const tokens = literal[2].split(/\s+/);
        if (
          tokens.includes("bg-primary") &&
          tokens.includes("text-on-primary") &&
          tokens.some((token) => /^hover:opacity-\d+$/.test(token))
        ) {
          findings.push(`${name}: opacity hover on primary control`);
        }
      }
    }

    expect(findings).toEqual([]);
  });

  it("allows only on-surface foreground text on primary tints", () => {
    const findings: string[] = [];
    const primaryTint =
      /^(?:(?:dark|hover|focus|focus-visible|active|disabled|group-hover):)*bg-primary\/(?:5|10|15)$/;
    const unsupportedText =
      /^(?:(?:dark|hover|focus|focus-visible|active|disabled|group-hover):)*text-(?:primary|on-primary|on-surface-variant)(?:\/[^\s]+)?$/;
    const tintInSource = /(?:^|\s)bg-primary\/(?:5|10|15)(?:\s|$)/;
    const textInSource =
      /(?:^|\s)text-(?:primary|on-primary|on-surface-variant)(?:\/[^\s"'`]+)?(?:\s|$)/;

    for (const path of sources) {
      const name = sourceName(path);
      if (
        alwaysDarkFiles.has(name) ||
        alwaysDarkPrefixes.some((prefix) => name.startsWith(prefix))
      ) {
        continue;
      }
      const source = readFileSync(path, "utf8");
      for (const literal of source.matchAll(/(["'])([^"'`\n]*)\1/g)) {
        const tokens = literal[2].split(/\s+/);
        if (
          tokens.some((token) => primaryTint.test(token)) &&
          tokens.some((token) => unsupportedText.test(token))
        ) {
          findings.push(`${name}: unsupported foreground on primary tint`);
        }
      }
      source.split("\n").forEach((line, index) => {
        if (
          /\bbg\s*:/.test(line) &&
          /\btext\s*:/.test(line) &&
          tintInSource.test(line) &&
          textInSource.test(line)
        ) {
          findings.push(`${name}:${index + 1}: composed unsupported primary tint pair`);
        }
      });

      const sourceFile = ts.createSourceFile(
        name,
        source,
        ts.ScriptTarget.Latest,
        true,
        name.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
      );
      const bindings = new Map<string, string>();
      const collectBindings = (node: ts.Node) => {
        if (
          ts.isVariableDeclaration(node) &&
          ts.isIdentifier(node.name) &&
          node.initializer &&
          ts.isStringLiteralLike(node.initializer)
        ) {
          bindings.set(node.name.text, node.initializer.text);
        }
        ts.forEachChild(node, collectBindings);
      };
      collectBindings(sourceFile);

      const classFragments = (node: ts.Node | undefined): string[] => {
        if (!node) return [];
        if (ts.isStringLiteralLike(node)) return [node.text];
        if (ts.isJsxExpression(node)) return classFragments(node.expression);
        if (ts.isIdentifier(node)) {
          const value = bindings.get(node.text);
          return value ? [value] : [];
        }
        if (ts.isTemplateExpression(node)) {
          return [
            node.head.text,
            ...node.templateSpans.flatMap((span) => [
              ...classFragments(span.expression),
              span.literal.text,
            ]),
          ];
        }
        if (ts.isConditionalExpression(node)) {
          return [
            ...classFragments(node.whenTrue),
            ...classFragments(node.whenFalse),
          ];
        }
        return [];
      };

      const classesFor = (
        opening: ts.JsxOpeningElement | ts.JsxSelfClosingElement,
      ): string => {
        const className = opening.attributes.properties.find(
          (property): property is ts.JsxAttribute =>
            ts.isJsxAttribute(property) &&
            property.name.getText(sourceFile) === "className",
        );
        return classFragments(className?.initializer).join(" ");
      };

      const openingFor = (
        node: ts.Node,
      ): ts.JsxOpeningElement | ts.JsxSelfClosingElement | null => {
        if (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) {
          return node;
        }
        if (ts.isJsxElement(node)) return node.openingElement;
        return null;
      };

      const inspectDescendants = (node: ts.Node) => {
        const opening = openingFor(node);
        if (opening) {
          const ownClasses = classesFor(opening);
          const hasUnsupportedText =
            /(?:^|\s)(?:(?:hover|focus|focus-visible|active|disabled|group-hover):)*text-(?:primary|on-primary|on-surface-variant|error|success|warning)(?:\/[^\s]+)?(?:\s|$)/.test(
              ownClasses,
            );
          if (hasUnsupportedText) {
            let cursor: ts.Node | undefined = opening;
            while (cursor) {
              const ancestorOpening = openingFor(cursor);
              if (ancestorOpening) {
                const ancestorClasses = classesFor(ancestorOpening);
                const tokens = ancestorClasses.split(/\s+/);
                if (
                  tokens.some((token) =>
                    /^bg-primary\/(?:5|10|15)$/.test(token),
                  )
                ) {
                  const { line } = sourceFile.getLineAndCharacterOfPosition(
                    opening.getStart(sourceFile),
                  );
                  findings.push(
                    `${name}:${line + 1}: unsupported descendant foreground on primary tint`,
                  );
                  break;
                }
                if (
                  tokens.some(
                    (token) =>
                      /^bg-(?!transparent$)[^/\s]+$/.test(token),
                  )
                ) {
                  break;
                }
              }
              cursor = cursor.parent;
            }
          }
        }
        ts.forEachChild(node, inspectDescendants);
      };
      inspectDescendants(sourceFile);
    }

    expect([...new Set(findings)]).toEqual([]);
  });

  it("gives switch thumbs contrasting checked and unchecked colors", () => {
    const findings: string[] = [];
    const switchBlock =
      /<button(?=[^>]*\brole=["']switch["'])[\s\S]*?<\/button>/g;

    for (const path of sources) {
      const name = sourceName(path);
      if (
        alwaysDarkFiles.has(name) ||
        alwaysDarkPrefixes.some((prefix) => name.startsWith(prefix))
      ) {
        continue;
      }
      const source = readFileSync(path, "utf8");
      for (const match of source.matchAll(switchBlock)) {
        const block = match[0];
        if (
          block.includes("bg-surface-container-highest") &&
          block.includes("bg-on-primary") &&
          !block.includes("bg-on-surface-variant")
        ) {
          findings.push(`${name}: switch thumb lacks unchecked contrast`);
        }
      }
    }

    expect(findings).toEqual([]);
  });

  it("keeps interactive form-control boundaries at full semantic contrast", () => {
    const findings: string[] = [];
    const translucentSemanticBoundary =
      /\b(?:focus:|focus-visible:)?border-(?:background|surface(?:-container(?:-(?:lowest|low|high|highest))?)?|primary(?:-dim)?|on-primary|on-surface(?:-variant)?|outline-variant|error|success|warning)\/[^\s]+\b/;

    const fragments = (
      node: ts.Node | undefined,
      bindings: ReadonlyMap<string, string>,
    ): string[] => {
      if (!node) return [];
      if (ts.isStringLiteralLike(node)) return [node.text];
      if (ts.isJsxExpression(node)) return fragments(node.expression, bindings);
      if (ts.isIdentifier(node)) {
        const value = bindings.get(node.text);
        return value ? [value] : [];
      }
      if (ts.isTemplateExpression(node)) {
        return [
          node.head.text,
          ...node.templateSpans.flatMap((span) => [
            ...fragments(span.expression, bindings),
            span.literal.text,
          ]),
        ];
      }
      if (ts.isConditionalExpression(node)) {
        return [
          ...fragments(node.whenTrue, bindings),
          ...fragments(node.whenFalse, bindings),
        ];
      }
      return [];
    };

    for (const path of sources) {
      const name = sourceName(path);
      if (
        alwaysDarkFiles.has(name) ||
        alwaysDarkPrefixes.some((prefix) => name.startsWith(prefix))
      ) {
        continue;
      }
      const source = readFileSync(path, "utf8");
      const sourceFile = ts.createSourceFile(
        name,
        source,
        ts.ScriptTarget.Latest,
        true,
        name.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
      );
      const bindings = new Map<string, string>();

      const collectBindings = (node: ts.Node) => {
        if (
          ts.isVariableDeclaration(node) &&
          ts.isIdentifier(node.name) &&
          node.initializer &&
          ts.isStringLiteralLike(node.initializer)
        ) {
          bindings.set(node.name.text, node.initializer.text);
        }
        ts.forEachChild(node, collectBindings);
      };
      collectBindings(sourceFile);

      const inspect = (node: ts.Node) => {
        if (
          ts.isJsxOpeningElement(node) ||
          ts.isJsxSelfClosingElement(node)
        ) {
          const tag = node.tagName.getText(sourceFile);
          const role = node.attributes.properties.find(
            (property): property is ts.JsxAttribute =>
              ts.isJsxAttribute(property) &&
              property.name.getText(sourceFile) === "role",
          );
          const roleValue =
            role?.initializer && ts.isStringLiteral(role.initializer)
              ? role.initializer.text
              : null;
          const isControl =
            tag === "input" ||
            tag === "select" ||
            tag === "textarea" ||
            (tag === "button" && roleValue === "combobox");
          if (isControl) {
            const className = node.attributes.properties.find(
              (property): property is ts.JsxAttribute =>
                ts.isJsxAttribute(property) &&
                property.name.getText(sourceFile) === "className",
            );
            const value = fragments(className?.initializer, bindings).join(" ");
            if (translucentSemanticBoundary.test(value)) {
              const { line } = sourceFile.getLineAndCharacterOfPosition(
                node.getStart(sourceFile),
              );
              findings.push(`${name}:${line + 1}: ${tag}`);
            }
          }
        }
        ts.forEachChild(node, inspect);
      };
      inspect(sourceFile);
    }

    expect(findings).toEqual([]);
  });

  it("keeps exact Sonic legacy exceptions out of supported interactive usage", () => {
    const findings: string[] = [];
    const meaningfulOutline =
      /(?:focus|focus-visible|focus-within|selection):(?:border|outline|ring|bg|text)-outline-variant(?:\/[^\s"'`]+)?/g;
    const unsupportedErrorSurface =
      /(?:surface-container-highest[^"'`\n]*text-error|text-error[^"'`\n]*surface-container-highest)/g;
    const unsupportedOnPrimarySurface =
      /(?:bg-surface(?:-container(?:-(?:lowest|low|high|highest))?)?[^"'`\n]*text-on-primary|text-on-primary[^"'`\n]*bg-surface(?:-container(?:-(?:lowest|low|high|highest))?)?)/g;

    for (const path of sources) {
      const name = sourceName(path);
      if (
        alwaysDarkFiles.has(name) ||
        alwaysDarkPrefixes.some((prefix) => name.startsWith(prefix))
      ) {
        continue;
      }
      const source = readFileSync(path, "utf8");
      for (const match of source.match(meaningfulOutline) ?? []) {
        findings.push(`${name}: meaningful state uses ${match}`);
      }
      for (const match of source.match(unsupportedErrorSurface) ?? []) {
        findings.push(`${name}: unsupported Sonic error surface ${match}`);
      }
      for (const match of source.match(unsupportedOnPrimarySurface) ?? []) {
        findings.push(`${name}: unsupported on-primary surface ${match}`);
      }
    }

    expect(findings).toEqual([]);
  });

  it("locks transparent bodies and unsynchronized always-dark glass", () => {
    const css = readFileSync("./src/styles.css", "utf8");
    const overlayHtml = readFileSync("./overlay.html", "utf8");
    const transformHtml = readFileSync("./transform-review.html", "utf8");
    const queryHtml = readFileSync("./query-review.html", "utf8");
    const overlay = readFileSync("./src/components/OverlayWidget.tsx", "utf8");
    const transform = readFileSync(
      "./src/components/transform-review/TransformReviewApp.tsx",
      "utf8",
    );
    const alwaysDarkSource = sources
      .filter((path) => {
        const name = sourceName(path);
        return (
          alwaysDarkFiles.has(name) ||
          alwaysDarkPrefixes.some((prefix) => name.startsWith(prefix))
        );
      })
      .map((path) => readFileSync(path, "utf8"))
      .join("\n");

    expect(css).toMatch(
      /body\.overlay-window\s*\{[^}]*background:\s*transparent/s,
    );
    expect(overlayHtml).toContain('<body class="overlay-window">');
    expect(transformHtml).toContain('<body class="overlay-window">');
    expect(queryHtml).toContain('<body class="overlay-window">');
    expect(overlayHtml).toMatch(
      /html, body, #root[\s\S]*background:\s*transparent/,
    );
    expect(transformHtml).toMatch(
      /html, body, #root[\s\S]*background:\s*transparent/,
    );
    expect(queryHtml).toMatch(
      /html, body, #root[\s\S]*background:\s*transparent/,
    );
    expect(overlay).toContain("background: 'rgba(20, 20, 20, 0.92)'");
    expect(transform).toContain("background: 'rgba(20, 20, 20, 0.92)'");
    expect(alwaysDarkSource).not.toContain("useAppearance");
    expect(alwaysDarkSource).not.toContain("appearance-changed");
    expect(alwaysDarkSource).not.toContain("--murmur-");
  });
});
