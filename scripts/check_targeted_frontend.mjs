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
  if (ts.isParenthesizedExpression(node) || ts.isAsExpression(node) || ts.isTypeAssertionExpression(node) ||
      ts.isNonNullExpression(node) || ts.isSatisfiesExpression(node)) return constantString(node.expression);
  if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.PlusToken) {
    const left = constantString(node.left), right = constantString(node.right);
    if (left !== undefined && right !== undefined) return left + right;
  }
}
// A known prefix is enough to reject `plugin:event|emit${dynamic}`. Do not
// pretend to evaluate variable bindings, arbitrary expressions or runtime code.
function stringPrefix(node) {
  const value = constantString(node);
  if (value !== undefined) return value;
  if (ts.isParenthesizedExpression(node) || ts.isAsExpression(node) || ts.isTypeAssertionExpression(node) ||
      ts.isNonNullExpression(node) || ts.isSatisfiesExpression(node)) return stringPrefix(node.expression);
  if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.PlusToken) {
    const left = constantString(node.left);
    return left === undefined ? stringPrefix(node.left) : left + (stringPrefix(node.right) ?? '');
  }
  if (ts.isTemplateExpression(node)) {
    let prefix = node.head.text;
    for (const span of node.templateSpans) {
      const expression = constantString(span.expression);
      if (expression === undefined) return prefix + (stringPrefix(span.expression) ?? '');
      prefix += expression + span.literal.text;
    }
    return prefix;
  }
}
export function violations(source, filename = 'source.tsx') {
  const file = ts.createSourceFile(filename, source, ts.ScriptTarget.Latest, true);
  const failures = [];
  const targeted = new Set(['emitTo', 'sendTo']);
  const importedTransports = new Set();
  const labels = new Set(['main', 'overlay', 'dictation-preview', 'diagnostics']);
  const adapter = filename === 'app/src/lib/targetedEvents.ts';
  for (const statement of file.statements) {
    if (ts.isImportDeclaration(statement)) {
      const bindings = statement.importClause?.namedBindings;
      if (bindings && ts.isNamedImports(bindings)) {
        for (const item of bindings.elements) {
          if (targeted.has((item.propertyName ?? item.name).text)) {
            targeted.add(item.name.text);
            importedTransports.add(item.name.text);
          }
        }
      }
    }
  }
  const test = /(?:\.(?:test|spec)\.[cm]?[jt]sx?$|(?:^|\/)test-utils\/)/.test(filename);
  const report = (node, reason) => failures.push(`${file.getLineAndCharacterOfPosition(node.getStart(file)).line + 1}: ${reason}`);
  for (const diagnostic of file.parseDiagnostics) {
    report(file, `unparseable source: ${ts.flattenDiagnosticMessageText(diagnostic.messageText, ' ')}`);
  }
  function visit(node) {
    if (!adapter && (ts.isStringLiteralLike(node) || ts.isBinaryExpression(node) || ts.isTemplateExpression(node)) &&
        stringPrefix(node)?.startsWith('plugin:event|emit')) {
      report(node, 'raw event emit IPC must stay in the sanctioned adapter');
    }
    // Reserve the transport token in shipped sources. This catches references,
    // destructuring, aliases, optional calls and Window/WebviewWindow methods.
    if (!test && ts.isIdentifier(node) && node.text === 'emit') report(node, 'broadcast emit token');
    if (!test && (ts.isStringLiteralLike(node) || ts.isBinaryExpression(node) || ts.isTemplateExpression(node)) && constantString(node) === 'emit') report(node, 'computed broadcast emit name');
    if (!test && ts.isElementAccessExpression(node) && constantString(node.argumentExpression) === 'emit') {
      report(node, 'computed broadcast emit member');
    }
    // Imports introduce checked names; every other occurrence must be the
    // callee of a direct call. Reject extraction, arguments, bind/call/apply,
    // destructuring and export facades instead of trying to track aliases.
    const token = ts.isIdentifier(node) ? node.text : constantString(node);
    if (!adapter && targeted.has(token) && (!test || importedTransports.has(token) || ts.isExportSpecifier(node.parent))) {
      const parent = node.parent;
      const imported = ts.isImportSpecifier(parent);
      const member = ts.isPropertyAccessExpression(parent) && parent.name === node ||
        ts.isElementAccessExpression(parent) && parent.argumentExpression === node;
      const callee = member ? parent : node;
      const directCall = ts.isCallExpression(callee.parent) && callee.parent.expression === callee;
      if (!imported && !directCall) report(node, 'event transport must only be used in a direct targeted call');
    }
    if (ts.isCallExpression(node)) {
      const name = ts.isIdentifier(node.expression) ? node.expression.text :
        ts.isPropertyAccessExpression(node.expression) ? node.expression.name.text :
        ts.isElementAccessExpression(node.expression) ? constantString(node.expression.argumentExpression) : undefined;
      if (targeted.has(name)) {
        const target = node.arguments[0];
        const fixedLabel = target && ts.isStringLiteral(target) && labels.has(target.text);
        const helperLabel = adapter && target && ts.isIdentifier(target) && target.text === 'label';
        if (!fixedLabel && !helperLabel) report(node, 'event transport requires an explicit consumer window label');
      }
    }
    if ((ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) && node.moduleSpecifier) {
      const api = constantString(node.moduleSpecifier);
      if (/^@tauri-apps\/api\/(?:event|window|webviewWindow)(?:\.js)?$/.test(api ?? '') ||
          /(?:^|\/)targetedEvents(?:\.[cm]?[jt]s)?$/.test(api ?? '')) {
        const binding = ts.isImportDeclaration(node) ? node.importClause?.namedBindings : node.exportClause;
        if (ts.isImportDeclaration(node) && node.importClause?.name) report(node, 'default event transport import');
        if (!binding || ts.isNamespaceImport(binding) || ts.isNamespaceExport(binding)) {
          report(node, 'event transport namespace/glob hides methods');
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
      if (api?.startsWith('@tauri-apps/api/') || /(?:^|\/)targetedEvents(?:\.[cm]?[jt]s)?$/.test(api ?? '')) {
        report(node, 'dynamic event transport import');
      }
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
  else console.log('frontend: broadcast transports and raw emit IPC rejected; direct transport calls require consumer labels');
}
