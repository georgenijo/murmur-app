#!/usr/bin/env node
// Read-only candidates, not deletion instructions. Run after `cd app && npm ci`.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const app = path.join(root, 'app');
const require = createRequire(path.join(app, 'package.json'));
const ts = require('typescript');
const tracked = execFileSync('git', ['ls-files', '-z'], { cwd: root, encoding: 'utf8' })
  .split('\0').filter(Boolean);
const sourceFiles = tracked.filter(file => /^app\/.*\.[cm]?[jt]sx?$/.test(file)
  && !file.startsWith('app/src-tauri/') && fs.existsSync(path.join(root, file)));
const config = ts.readConfigFile(path.join(app, 'tsconfig.json'), ts.sys.readFile);
if (config.error) throw new Error(ts.flattenDiagnosticMessageText(config.error.messageText, '\n'));
const parsed = ts.parseJsonConfigFileContent(config.config, ts.sys, app);
const program = ts.createProgram(sourceFiles.map(file => path.join(root, file)), {
  ...parsed.options, allowJs: true, checkJs: false,
});
const checker = program.getTypeChecker();
const relative = file => path.relative(root, file).split(path.sep).join('/');
const sources = program.getSourceFiles().filter(file => sourceFiles.includes(relative(file.fileName)));
const edges = new Map();
const references = new Map();
const localUses = new Map();
const packages = new Set();
const dynamicImports = [];
const unalias = symbol => symbol && (symbol.flags & ts.SymbolFlags.Alias)
  ? checker.getAliasedSymbol(symbol) : symbol;

for (const source of sources) {
  const file = relative(source.fileName);
  const imports = new Set();
  function visit(node) {
    if (ts.isIdentifier(node)) {
      const symbol = unalias(checker.getSymbolAtLocation(node));
      if (symbol) {
        if (!references.has(symbol)) references.set(symbol, new Set());
        references.get(symbol).add(file);
        if (!symbol.declarations?.some(declaration => declaration.name === node)) {
          if (!localUses.has(symbol)) localUses.set(symbol, new Set());
          localUses.get(symbol).add(file);
        }
      }
    }
    const specifier = (ts.isImportDeclaration(node) || ts.isExportDeclaration(node))
      ? node.moduleSpecifier
      : ts.isImportTypeNode(node) && ts.isLiteralTypeNode(node.argument)
        ? node.argument.literal
        : ts.isCallExpression(node) && (node.expression.kind === ts.SyntaxKind.ImportKeyword
          || node.expression.getText(source) === 'require') ? node.arguments[0] : undefined;
    if (specifier && ts.isStringLiteralLike(specifier)) {
      const resolved = ts.resolveModuleName(specifier.text, source.fileName, parsed.options, ts.sys).resolvedModule;
      if (resolved && !resolved.isExternalLibraryImport) imports.add(relative(resolved.resolvedFileName));
      if (!specifier.text.startsWith('.') && !specifier.text.startsWith('@/') && !specifier.text.startsWith('/')) {
        packages.add(specifier.text.startsWith('@') ? specifier.text.split('/').slice(0, 2).join('/') : specifier.text.split('/')[0]);
      }
    } else if (ts.isCallExpression(node) && (node.expression.kind === ts.SyntaxKind.ImportKeyword
      || node.expression.getText(source) === 'import.meta.glob')) {
      dynamicImports.push(`${file}:${source.getLineAndCharacterOfPosition(node.getStart()).line + 1}`);
    }
    ts.forEachChild(node, visit);
  }
  visit(source);
  edges.set(file, imports);
}

const htmlRoots = new Set();
for (const file of tracked.filter(file => /^app\/[^/]+\.html$/.test(file))) {
  for (const match of fs.readFileSync(path.join(root, file), 'utf8').matchAll(/<script\b[^>]*\bsrc=["']([^"']+)["']/g)) {
    if (match[1].startsWith('/src/')) htmlRoots.add(`app${match[1]}`);
  }
}
const testRoots = sourceFiles.filter(file => /\.test\.[jt]sx?$/.test(file) || file.includes('/visual-tests/'));
const toolRoots = sourceFiles.filter(file => !file.startsWith('app/src/'));
function reachable(roots) {
  const visited = new Set();
  function walk(file) {
    if (visited.has(file)) return;
    visited.add(file);
    for (const imported of edges.get(file) ?? []) walk(imported);
  }
  roots.forEach(walk);
  return visited;
}
const live = reachable([...htmlRoots, ...toolRoots]);
const tested = reachable(testRoots);
const production = sourceFiles.filter(file => file.startsWith('app/src/')
  && !/\.test\.[jt]sx?$|\.d\.ts$/.test(file));
const exports = [];
for (const source of sources.filter(file => production.includes(relative(file.fileName)))) {
  const module = checker.getSymbolAtLocation(source);
  if (!module) continue;
  for (const exported of checker.getExportsOfModule(module)) {
    const symbol = unalias(exported);
    const file = relative(source.fileName);
    const consumers = [...(references.get(symbol) ?? [])].filter(consumer => consumer !== file);
    if (consumers.length) continue;
    const declaration = exported.declarations?.[0] ?? symbol.declarations?.[0];
    exports.push({ file, name: exported.name,
      kind: symbol.flags & ts.SymbolFlags.Value ? 'value' : 'type',
      usedInDeclaringFile: localUses.get(symbol)?.has(file) ?? false,
      line: declaration ? declaration.getSourceFile().getLineAndCharacterOfPosition(declaration.getStart()).line + 1 : null });
  }
}
const manifest = JSON.parse(fs.readFileSync(path.join(app, 'package.json'), 'utf8'));
console.log(JSON.stringify({
  revision: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim(),
  workingTreeDirty: execFileSync('git', ['status', '--porcelain'], { cwd: root, encoding: 'utf8' }).trim().length > 0,
  scope: 'Tracked frontend source, all HTML windows, visual fixtures, tests, and app tooling. Type imports count as references.',
  limitations: 'Candidates require review. Glob imports and runtime-computed references are not expanded; CSS, Rust IPC and external consumers need separate checks. Missing package imports may be CLI, CSS or config dependencies.',
  filesScanned: sources.length,
  htmlRoots: [...htmlRoots].sort(),
  unreachableFiles: production.filter(file => !live.has(file) && !tested.has(file)),
  testOnlyFiles: production.filter(file => !live.has(file) && tested.has(file)),
  exportsWithoutExternalReferences: exports,
  dependenciesWithoutModuleImports: Object.keys(manifest.dependencies).filter(name => !packages.has(name)),
  dynamicImportsNeedingReview: dynamicImports,
}, null, 2));
