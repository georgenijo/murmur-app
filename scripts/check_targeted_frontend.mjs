#!/usr/bin/env node
/** Source-level frontend transport policy. Parse syntax instead of matching JS
 * text: aliases, escaped identifiers, computed properties and template expression
 * bodies must not hide broadcast transports. Runtime code synthesis is out of scope. */
import { createRequire } from 'node:module';
import { readdirSync, readFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
const require = createRequire(import.meta.url);
const ts = require(process.env.TARGETED_EVENTS_TYPESCRIPT || '../app/node_modules/typescript');
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

function constantString(node) {
  if (!node) return undefined;
  if (ts.isStringLiteralLike(node)) return node.text;
  if (ts.isTemplateExpression(node)) {
    let value = node.head.text;
    for (const span of node.templateSpans) {
      const expression = constantString(span.expression);
      if (expression === undefined) return undefined;
      value += expression + span.literal.text;
    }
    return value;
  }
  if (ts.isParenthesizedExpression(node)) return constantString(node.expression);
  if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.PlusToken) {
    const left = constantString(node.left), right = constantString(node.right);
    if (left !== undefined && right !== undefined) return left + right;
  }
}
export function violations(source, filename = 'source.tsx') {
  const file = ts.createSourceFile(filename, source, ts.ScriptTarget.Latest, true);
  const failures = [];
  const test = /(?:\.(?:test|spec)\.[cm]?[jt]sx?$|(?:^|\/)test-utils\/)/.test(filename);
  const report = (node, reason) => failures.push(`${file.getLineAndCharacterOfPosition(node.getStart(file)).line + 1}: ${reason}`);
  for (const diagnostic of file.parseDiagnostics) {
    report(file, `unparseable source: ${ts.flattenDiagnosticMessageText(diagnostic.messageText, ' ')}`);
  }
  function visit(node) {
    // Reserve the transport token in shipped sources. This catches references,
    // destructuring, aliases, optional calls and Window/WebviewWindow methods.
    if (!test && ts.isIdentifier(node) && node.text === 'emit') report(node, 'broadcast emit token');
    if (!test && (ts.isStringLiteralLike(node) || ts.isBinaryExpression(node) || ts.isTemplateExpression(node)) && constantString(node) === 'emit') report(node, 'computed broadcast emit name');
    if (!test && ts.isElementAccessExpression(node) && constantString(node.argumentExpression) === 'emit') {
      report(node, 'computed broadcast emit member');
    }
    if ((ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) && node.moduleSpecifier) {
      const api = constantString(node.moduleSpecifier);
      if (/^@tauri-apps\/api\/(?:event|window|webviewWindow)(?:\.js)?$/.test(api ?? '')) {
        const binding = ts.isImportDeclaration(node) ? node.importClause?.namedBindings : node.exportClause;
        if (ts.isImportDeclaration(node) && node.importClause?.name) report(node, 'default Tauri transport import');
        if (!binding || ts.isNamespaceImport(binding) || ts.isNamespaceExport(binding)) {
          report(node, 'Tauri transport namespace/glob hides methods');
        } else {
          for (const item of binding.elements) {
            if ((item.propertyName ?? item.name).text === 'emit') report(item, 'Tauri broadcast import/re-export');
          }
        }
      }
    }
    if (ts.isCallExpression(node) && (node.expression.kind === ts.SyntaxKind.ImportKeyword ||
        ts.isIdentifier(node.expression) && node.expression.text === 'require')) {
      const api = constantString(node.arguments[0]);
      if (api?.startsWith('@tauri-apps/api/')) report(node, 'dynamic Tauri transport import');
    }
    ts.forEachChild(node, visit);
  }
  visit(file);
  return [...new Set(failures)];
}
export function checkRepository(directory = root) {
  const failures = [];
  function walk(folder) {
    for (const entry of readdirSync(folder, { withFileTypes: true })) {
      const path = join(folder, entry.name);
      if (entry.isDirectory()) walk(path);
      else if (/\.[cm]?[jt]sx?$/.test(entry.name)) {
        const name = relative(directory, path).replaceAll('\\', '/');
        failures.push(...violations(readFileSync(path, 'utf8'), name).map(failure => `${name}:${failure}`));
      }
    }
  }
  walk(join(directory, 'app/src'));
  return failures;
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const failures = checkRepository();
  if (failures.length) { console.error(failures.join('\n')); process.exitCode = 1; }
  else console.log('frontend: no broadcast emit imports, tokens or window methods');
}
