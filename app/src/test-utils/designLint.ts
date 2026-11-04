import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import postcss from 'postcss';
import ts from 'typescript';

export type DesignRule = 'literal-color' | 'font-size' | 'arbitrary-px' | 'prose-mono';
export interface DesignViolation {
  file: string;
  rule: DesignRule;
  pattern: string;
  context: string;
  occurrence: number;
}

// Paths are relative to app/src. Exemptions are deliberately structural, not
// component-wide: a diagnostics pane can contain both code and prose.
export function isDesignSource(file: string): boolean {
  return /\.(tsx|css)$/.test(file)
    && !/(^|\/)(?:__tests__|__fixtures__|test-utils|visual-fixtures)(?:\/|\.)/.test(file)
    && !/\.(?:test|spec|fixture|fixtures)\./.test(file);
}

const sizeToken = /^var\(--ui-font-(?!sans\b|mono\b)[\w-]+\)$/;
const technicalTags = new Set(['code', 'pre', 'kbd', 'samp']);
const palette = '(?:slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose)-\\d{2,3}|black|white';
// CSS named colors, excluding transparent/currentColor (semantic paint values).
const namedColors = new Set(('aliceblue antiquewhite aqua aquamarine azure beige bisque black blanchedalmond blue blueviolet brown burlywood cadetblue chartreuse chocolate coral cornflowerblue cornsilk crimson cyan darkblue darkcyan darkgoldenrod darkgray darkgreen darkgrey darkkhaki darkmagenta darkolivegreen darkorange darkorchid darkred darksalmon darkseagreen darkslateblue darkslategray darkslategrey darkturquoise darkviolet deeppink deepskyblue dimgray dimgrey dodgerblue firebrick floralwhite forestgreen fuchsia gainsboro ghostwhite gold goldenrod gray green greenyellow grey honeydew hotpink indianred indigo ivory khaki lavender lavenderblush lawngreen lemonchiffon lightblue lightcoral lightcyan lightgoldenrodyellow lightgray lightgreen lightgrey lightpink lightsalmon lightseagreen lightskyblue lightslategray lightslategrey lightsteelblue lightyellow lime limegreen linen magenta maroon mediumaquamarine mediumblue mediumorchid mediumpurple mediumseagreen mediumslateblue mediumspringgreen mediumturquoise mediumvioletred midnightblue mintcream mistyrose moccasin navajowhite navy oldlace olive olivedrab orange orangered orchid palegoldenrod palegreen paleturquoise palevioletred papayawhip peachpuff peru pink plum powderblue purple rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown seagreen seashell sienna silver skyblue slateblue slategray slategrey snow springgreen steelblue tan teal thistle tomato turquoise violet wheat white whitesmoke yellow yellowgreen').split(' '));

// CSS values, selectors and Tailwind variants all contain nested delimiters.
// Only split at the outer level, preserving strings and escaped characters.
function splitOutside(value: string, separators: RegExp): string[] {
  const parts: string[] = [];
  let start = 0;
  let depth = 0;
  let quote = '';
  for (let i = 0; i < value.length; i++) {
    const char = value[i];
    if (char === '\\') { i++; continue; }
    if (quote) { if (char === quote) quote = ''; continue; }
    if (char === '"' || char === "'") { quote = char; continue; }
    if (char === '(' || char === '[') depth++;
    else if (char === ')' || char === ']') depth--;
    else if (!depth && separators.test(char)) {
      parts.push(value.slice(start, i).trim());
      start = i + 1;
    }
  }
  parts.push(value.slice(start).trim());
  return parts.filter(Boolean);
}

function closingParen(value: string, open: number): number {
  let depth = 1;
  let quote = '';
  for (let i = open + 1; i < value.length; i++) {
    const char = value[i];
    if (char === '\\') { i++; continue; }
    if (quote) { if (char === quote) quote = ''; continue; }
    if (char === '"' || char === "'") quote = char;
    else if (char === '(') depth++;
    else if (char === ')' && !--depth) return i;
  }
  return value.length - 1;
}

function colors(value: string, includeNames: boolean): string[] {
  const literals: string[] = [];
  let plain = '';
  for (let i = 0; i < value.length;) {
    const rest = value.slice(i);
    const quoted = rest.match(/^["'][^"']*["']/);
    if (quoted) { i += quoted[0].length; continue; }
    const fn = rest.match(/^([a-z-]+)\(/i);
    if (!fn) { plain += value[i++]; continue; }
    const end = closingParen(value, i + fn[0].length - 1);
    const args = value.slice(i + fn[0].length, end);
    const name = fn[1].toLowerCase();
    if (name === 'var') {
      // The custom-property name is never a color. Only its fallback is paint.
      const comma = splitOutside(args, /,/);
      literals.push(...colors(comma.slice(1).join(','), includeNames));
    } else if (name !== 'url') {
      if (/^(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)$/.test(name) && !/\bvar\(/i.test(args)) {
        literals.push(value.slice(i, end + 1));
      } else {
        // Token-supplied channels are semantic paint; nested literal fallbacks
        // (and explicit channels alongside tokens) must still be inspected.
        literals.push(...colors(args, includeNames));
        if (/^(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)$/.test(name)) {
          let channels = '';
          for (let j = 0; j < args.length; j++) {
            if (args.slice(j).startsWith('var(')) j = closingParen(args, j + 3);
            else channels += args[j];
          }
          if (/\d/.test(channels)) literals.push(value.slice(i, end + 1));
        }
      }
    }
    plain += ' ';
    i = end + 1;
  }
  literals.push(...[...plain.matchAll(/#[\da-f]{3,8}\b/gi)].map(m => m[0]));
  if (includeNames) {
    for (const word of plain.matchAll(/(?<![\w-])[a-z]+(?![\w-])/gi)) {
      if (namedColors.has(word[0].toLowerCase())) literals.push(word[0]);
    }
  }
  return literals;
}

function technicalSelector(selector: string): boolean {
  const parts = splitOutside(selector, /,/);
  return parts.length > 0 && parts.every(part => {
    // :is/:where describe the subject, whereas :not/:has do not establish it.
    let expanded = '';
    for (let i = 0; i < part.length; i++) {
      const fn = part.slice(i).match(/^:([\w-]+)\(/);
      if (!fn) { expanded += part[i]; continue; }
      const end = closingParen(part, i + fn[0].length - 1);
      if (/^(?:is|where)$/.test(fn[1]) && technicalSelector(part.slice(i + fn[0].length, end))) expanded += 'code';
      i = end;
    }
    // A technical sibling does not make the following prose technical.
    const subject = splitOutside(expanded, /[+~]/).pop() ?? '';
    return /(?:^|[\s>])(?:code|pre|kbd|samp)(?=$|[\s.#[:>])/i.test(subject.trim());
  });
}

function resolvedSelector(node: postcss.Container | postcss.Document | undefined): string {
  if (!node) return '';
  const parent = resolvedSelector(node.parent);
  if (node.type !== 'rule') return parent;
  const children = splitOutside((node as postcss.Rule).selector, /,/);
  if (!parent) return children.join(', ');
  return splitOutside(parent, /,/).flatMap(ancestor => children.map(child =>
    child.includes('&') ? child.replace(/&/g, ancestor) : `${ancestor} ${child}`)).join(', ');
}

const paintProperty = /^(?:color|background(?:Color|Image)?|background-(?:color|image)|(?:border|outline)(?:[A-Z][a-z]+|-[a-z]+)*|(?:box|text)(?:Shadow|-shadow)|fill|stroke|stopColor|stop-color|floodColor|flood-color|lightingColor|lighting-color|caretColor|caret-color|accent|accentColor|accent-color)$/;

function tokenFontShorthand(value: string): boolean {
  const components = splitOutside(value.replace(/\s*!important$/, ''), /[\s/]/);
  const size = components.find(component => !/^(?:normal|italic|oblique|small-caps|bold|bolder|lighter|[1-9]00|(?:ultra-|extra-|semi-)?(?:condensed|expanded))$/.test(component));
  return size !== undefined && sizeToken.test(size);
}

export function scanDesignSource(file: string, source: string): DesignViolation[] {
  if (!isDesignSource(file)) return [];
  const violations: DesignViolation[] = [];
  const occurrences = new Map<string, number>();
  const add = (rule: DesignRule, pattern: string, context: string) => {
    const key = JSON.stringify([rule, pattern, context]);
    const occurrence = (occurrences.get(key) ?? 0) + 1;
    occurrences.set(key, occurrence);
    violations.push({ file, rule, pattern, context, occurrence });
  };
  const declaration = (property: string, value: string, context: string, technical: boolean, prefix = '', utility?: string, emit = add) => {
    if (property.startsWith('--')) return; // Token definitions, not token consumers.
    const report = (rule: DesignRule, detail = value) => emit(rule, utility ?? `${prefix}${property}: ${detail}`, context);
    if (paintProperty.test(property)) {
      for (const color of colors(value, true)) report('literal-color', color);
    }
    if (property === 'font-size' || property === 'fontSize') {
      if (!sizeToken.test(value.replace(/\s*!important$/, '').trim())) report('font-size');
    }
    if (property === 'font' && !tokenFontShorthand(value)) report('font-size');
    if (!technical && /\bfont(?:-family|Family)?$/.test(property)
      && /--(?:home|ui)-font-mono\b|\bmonospace\b/.test(value)) report('prose-mono');
  };
  const classes = (value: string, technical: boolean, context: string, emit = add) => {
    for (const original of value.split(/\s+/)) {
      if (!original) continue;
      const token = (splitOutside(original, /:/).pop() ?? '').replace(/^!|!$/g, '');
      if (!token) continue;
      const report = (rule: DesignRule) => emit(rule, original, context);
      if (/\[[^\]]*(?:\d|\.)px\b[^\]]*\]/.test(token)) report('arbitrary-px');
      const property = token.match(/^\[([\w-]+):(.+)\]$/);
      if (property) {
        declaration(property[1], property[2].replace(/_/g, ' '), context, technical, '', original, emit);
        continue;
      }
      if (/^text-(?:xs|sm|base|lg|xl|[2-9]xl)(?:\/[^\s]+)?$/.test(token)
        || /^text-\[(?:length:)?(?:[\d.]|calc\(|clamp\(|min\(|max\()/i.test(token)
        || (/^text-(?:\[length:|\(length:)/.test(token) && !/--ui-font-(?!sans\b|mono\b)[\w-]+/.test(token))) report('font-size');
      if (new RegExp(`^(?:text|bg|border(?:-[trblxyse])?|ring|outline|fill|stroke|shadow|from|via|to|decoration|accent|caret)-(?:${palette})(?:/[^\\s]+)?$`).test(token)) report('literal-color');
      const arbitraryColor = token.match(/^(?:text|bg|border|ring|outline|fill|stroke|shadow|from|via|to|decoration|accent|caret)-\[(?:color:)?(.+)\]/i);
      if (arbitraryColor && colors(arbitraryColor[1].replace(/_/g, ' '), true).length) report('literal-color');
      if (!technical && (/^font-mono$/.test(token) || /--(?:home|ui)-font-mono\b/.test(token))) report('prose-mono');
    }
  };

  if (file.endsWith('.css')) {
    const root = postcss.parse(source, { from: file });
    const contextFor = (node: postcss.Container | postcss.Document | undefined) => {
      const atRules: string[] = [];
      for (let ancestor = node; ancestor; ancestor = ancestor.parent) {
        if (ancestor.type === 'atrule') {
          const at = ancestor as postcss.AtRule;
          atRules.unshift(`@${at.name} ${at.params}`);
        }
      }
      return [...atRules, resolvedSelector(node)].join(' | ').replace(/\s+/g, ' ').trim();
    };
    root.walkDecls(decl => {
      const selector = resolvedSelector(decl.parent).replace(/\s+/g, ' ').trim();
      declaration(decl.prop, decl.value, contextFor(decl.parent), technicalSelector(selector), `${selector} | `);
    });
    root.walkAtRules('apply', rule => {
      classes(rule.params, technicalSelector(resolvedSelector(rule.parent)), contextFor(rule.parent));
    });
  } else {
    const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
    const literal = (node: ts.Node | undefined): string | undefined => {
      if (!node) return undefined;
      if (ts.isJsxExpression(node)) return literal(node.expression);
      if (ts.isStringLiteralLike(node) || ts.isNumericLiteral(node)) return node.text;
      if (ts.isPrefixUnaryExpression(node) && ts.isNumericLiteral(node.operand)) return node.getText(ast);
      return undefined;
    };
    const bindings = new Map<string, ts.VariableDeclaration[]>();
    const collectBindings = (node: ts.Node) => {
      if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name)) {
        bindings.set(node.name.text, [...(bindings.get(node.name.text) ?? []), node]);
      }
      ts.forEachChild(node, collectBindings);
    };
    collectBindings(ast);
    const scope = (node: ts.Node): ts.Node => {
      for (let parent = node.parent; parent; parent = parent.parent) {
        if (ts.isBlock(parent) || ts.isSourceFile(parent) || ts.isFunctionLike(parent)) return parent;
      }
      return ast;
    };
    const paintValues = (node: ts.Node, seen = new Set<ts.Node>()): string[] => {
      if (seen.has(node)) return [];
      seen = new Set([...seen, node]);
      const value = literal(node);
      if (value !== undefined) return [value];
      if (ts.isIdentifier(node)) {
        // Resolve local paint constants only when consumed by a styling prop.
        // Walk outward so an inner binding takes precedence over a module one.
        for (let parent: ts.Node | undefined = node.parent; parent; parent = parent.parent) {
          const binding = bindings.get(node.text)?.find(candidate => scope(candidate) === parent);
          if (binding?.initializer) return paintValues(binding.initializer, seen);
          if (ts.isFunctionLike(parent) && parent.parameters.some(parameter => parameter.name.getText(ast) === node.text)) return [];
        }
        return [];
      }
      if (ts.isConditionalExpression(node)) return [...paintValues(node.whenTrue, seen), ...paintValues(node.whenFalse, seen)];
      if (ts.isTemplateHead(node) || ts.isTemplateMiddle(node) || ts.isTemplateTail(node)) return [node.text];
      const values: string[] = [];
      ts.forEachChild(node, child => { values.push(...paintValues(child, seen)); });
      return values;
    };
    const checkDeclaration = (property: string, initializer: ts.Node | undefined, context: string, technical: boolean) => {
      if (!initializer) return;
      if (paintProperty.test(property)) {
        for (const value of paintValues(initializer)) declaration(property, value, context, technical);
      } else {
        declaration(property, literal(initializer) ?? initializer.getText(ast), context, technical);
      }
    };
    const visit = (node: ts.Node, technical: boolean, path: string[]) => {
      if ((ts.isFunctionDeclaration(node) || ts.isClassDeclaration(node) || ts.isVariableDeclaration(node) || ts.isPropertyAssignment(node)) && node.name) {
        path = [...path, node.name.getText(ast)];
      }
      if (ts.isJsxElement(node) || ts.isJsxSelfClosingElement(node)) {
        const element = ts.isJsxElement(node) ? node.openingElement : node;
        const tag = element.tagName.getText(ast);
        technical ||= technicalTags.has(tag);
        const labels = element.attributes.properties.flatMap(attr => {
          if (!ts.isJsxAttribute(attr)) return [];
          const name = attr.name.getText(ast);
          const value = literal(attr.initializer);
          if (value === undefined) return [];
          if (['id', 'name', 'data-testid'].includes(name)) return [`${name}=${JSON.stringify(value)}`];
          if (name !== 'className') return [];
          // Use non-violating neighbour classes as anchors. Fixing one utility
          // must not change the identity of other violations on that element.
          const neighbours = value.split(/\s+/).filter(token => {
            let clean = true;
            classes(token, false, '', () => { clean = false; });
            return clean;
          }).filter(Boolean);
          return neighbours.length ? [`class=${JSON.stringify(neighbours.join(' '))}`] : [];
        });
        path = [...path, `<${tag}${labels.length ? ' ' + labels.join(' ') : ''}>`];
      }
      const context = path.join(' > ') || 'module';
      if (ts.isPropertyAssignment(node)) {
        const property = node.name.getText(ast).replace(/^["']|["']$/g, '');
        checkDeclaration(property, node.initializer, context, technical);
      }
      if (ts.isJsxAttribute(node)) {
        const initializer = node.initializer;
        checkDeclaration(node.name.getText(ast), initializer && ts.isJsxExpression(initializer) ? initializer.expression : initializer, context, technical);
      }
      if (ts.isStringLiteralLike(node) || ts.isTemplateHead(node) || ts.isTemplateMiddle(node) || ts.isTemplateTail(node)) {
        // Text, URLs and other non-styling JSX attributes are never paint.
        let parent: ts.Node = node.parent;
        while (ts.isJsxExpression(parent) || ts.isParenthesizedExpression(parent)) parent = parent.parent;
        if (!ts.isJsxAttribute(parent) || parent.name.getText(ast) === 'className') classes(node.text, technical, context);
      }
      ts.forEachChild(node, child => visit(child, technical, path));
    };
    visit(ast, false, []);
  }
  return violations;
}

export function scanDesignTree(root: string): DesignViolation[] {
  const walk = (directory: string): DesignViolation[] => readdirSync(directory, { withFileTypes: true })
    .sort((a, b) => a.name.localeCompare(b.name, 'en'))
    .flatMap(entry => {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) return walk(path);
      const file = relative(root, path).split('\\').join('/');
      return isDesignSource(file) ? scanDesignSource(file, readFileSync(path, 'utf8')) : [];
    });
  return walk(root);
}

function key(entry: DesignViolation): string {
  return JSON.stringify([entry.file, entry.rule, entry.pattern, entry.context, entry.occurrence]);
}

export function compareDesignBaseline(actual: DesignViolation[], baseline: DesignViolation[]) {
  const actualKeys = new Set(actual.map(key));
  const baselineKeys = new Set(baseline.map(key));
  return {
    added: actual.filter(entry => !baselineKeys.has(key(entry))),
    stale: baseline.filter(entry => !actualKeys.has(key(entry))),
  };
}

export function assertDesignBaseline(actual: DesignViolation[], baseline: DesignViolation[], shrink = false): void {
  const { added, stale } = compareDesignBaseline(actual, baseline);
  const format = (entries: DesignViolation[]) => entries.map(entry =>
    `  ${entry.file} [${entry.rule}] ${entry.pattern} (occurrence ${entry.occurrence}; ${entry.context})`).join('\n');
  if (added.length || (!shrink && stale.length)) {
    throw new Error([
      'UI design ratchet failed. See docs/features/ui-design-system.md.',
      added.length ? `New violations (fix these; regeneration cannot add them):\n${format(added)}` : '',
      stale.length ? `Stale baseline entries (remove with npm run design-lint:shrink):\n${format(stale)}` : '',
    ].filter(Boolean).join('\n'));
  }
}

export function serializeDesignBaseline(entries: DesignViolation[]): string {
  return `[\n${entries.map(entry => `  ${JSON.stringify(entry)}`).join(',\n')}\n]\n`;
}
