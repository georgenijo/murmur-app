import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import postcss from 'postcss';
import ts from 'typescript';

export type DesignRule = 'literal-color' | 'font-size' | 'arbitrary-px' | 'prose-mono';
export interface DesignViolation {
  file: string;
  rule: DesignRule;
  pattern: string;
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

function colors(value: string, includeNames: boolean): string[] {
  // Do not mistake token names, URL fragments, or quoted text for named colors.
  const paint = value.replace(/url\([^)]*\)|var\(\s*--[\w-]+\s*\)|["'][^"']*["']/g, '');
  const literals = [...paint.matchAll(/#[\da-f]{3,8}\b|\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\([^)]*\)/gi)].map(m => m[0]);
  if (includeNames) {
    for (const word of paint.matchAll(/\b[a-z]+\b/gi)) {
      if (namedColors.has(word[0].toLowerCase())) literals.push(word[0]);
    }
  }
  return literals;
}

function technicalSelector(selector: string): boolean {
  return selector.split(',').every(part => {
    // A code sibling or a :has(code) condition does not make prose technical.
    const subject = part.split(/[+~]/).pop()!.replace(/:[\w-]+\([^)]*\)/g, '').trim();
    return /(?:^|[\s>])(?:code|pre|kbd|samp)(?=$|[\s.#[:>])/i.test(subject);
  });
}

export function scanDesignSource(file: string, source: string): DesignViolation[] {
  if (!isDesignSource(file)) return [];
  const violations: DesignViolation[] = [];
  const occurrences = new Map<string, number>();
  const add = (rule: DesignRule, pattern: string) => {
    const key = JSON.stringify([rule, pattern]);
    const occurrence = (occurrences.get(key) ?? 0) + 1;
    occurrences.set(key, occurrence);
    violations.push({ file, rule, pattern, occurrence });
  };
  const declaration = (property: string, value: string, context: string, technical: boolean) => {
    if (property.startsWith('--')) return; // Token definitions, not token consumers.
    for (const color of colors(value, /color|background|border|shadow|fill|stroke|outline|caret|accent/i.test(property))) {
      add('literal-color', `${context}${property}: ${color}`);
    }
    if (property === 'font-size' || property === 'fontSize') {
      if (!sizeToken.test(value.replace(/\s*!important$/, '').trim())) add('font-size', `${context}${property}: ${value}`);
    }
    if (property === 'font') {
      // A shorthand can hide a local size as well as a mono family.
      if (!/var\(--ui-font-(?!sans\b|mono\b)[\w-]+\)/.test(value)) add('font-size', `${context}${property}: ${value}`);
    }
    if (!technical && /\bfont(?:-family|Family)?$/.test(property)
      && /--(?:home|ui)-font-mono\b|\bmonospace\b/.test(value)) {
      add('prose-mono', `${context}${property}: ${value}`);
    }
  };
  const classes = (value: string, technical: boolean) => {
    for (const token of value.split(/\s+/)) {
      if (!token) continue;
      if (/\[[^\]]*(?:\d|\.)px\b[^\]]*\]/.test(token)) add('arbitrary-px', token);
      if (/(?:^|:)text-(?:xs|sm|base|lg|xl|[2-9]xl)(?:\/[^\s]+)?$/.test(token)
        || /(?:^|:)text-\[(?:length:)?(?:[\d.]|calc\(|clamp\(|min\(|max\()/i.test(token)
        || (/(?:^|:)text-(?:\[length:|\(length:)/.test(token) && !/--ui-font-(?!sans\b|mono\b)[\w-]+/.test(token))) {
        add('font-size', token);
      }
      if (new RegExp(`(?:^|:)(?:text|bg|border(?:-[trblxyse])?|ring|outline|fill|stroke|shadow|from|via|to|decoration|accent|caret)-(?:${palette})(?:/[^\\s]+)?$`).test(token)) {
        add('literal-color', token);
      }
      const arbitraryColor = token.match(/(?:^|:)(?:text|bg|border|ring|outline|fill|stroke|shadow|from|via|to|decoration|accent|caret)-\[(?:color:)?([a-z]+)\]/i);
      if (arbitraryColor && namedColors.has(arbitraryColor[1].toLowerCase())) add('literal-color', token);
      if (!technical && /(?:^|:)font-mono$/.test(token)) add('prose-mono', token);
      if (!technical && /--(?:home|ui)-font-mono\b/.test(token)) add('prose-mono', token);
    }
  };

  if (file.endsWith('.css')) {
    const root = postcss.parse(source, { from: file });
    root.walkDecls(decl => {
      const parent = decl.parent;
      const selector = parent?.type === 'rule' ? parent.selector.replace(/\s+/g, ' ').trim() : '';
      declaration(decl.prop, decl.value, `${selector} | `, technicalSelector(selector));
    });
    root.walkAtRules('apply', rule => {
      classes(rule.params, technicalSelector(rule.parent?.type === 'rule' ? rule.parent.selector : ''));
      for (const color of colors(rule.params, false)) add('literal-color', `@apply: ${color}`);
    });
  } else {
    const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
    const visit = (node: ts.Node, technical: boolean) => {
      if (ts.isJsxElement(node) || ts.isJsxSelfClosingElement(node)) {
        const tag = ts.isJsxElement(node) ? node.openingElement.tagName : node.tagName;
        technical ||= technicalTags.has(tag.getText(ast));
      }
      if (ts.isPropertyAssignment(node)) {
        const property = node.name.getText(ast).replace(/^["']|["']$/g, '');
        const value = ts.isStringLiteralLike(node.initializer) ? node.initializer.text : node.initializer.getText(ast);
        declaration(property, value, '', technical);
      }
      if (ts.isJsxAttribute(node) && node.initializer && ts.isStringLiteral(node.initializer)) {
        declaration(node.name.getText(ast), node.initializer.text, '', technical);
      }
      if (ts.isStringLiteralLike(node) || ts.isTemplateHead(node) || ts.isTemplateMiddle(node) || ts.isTemplateTail(node)) {
        classes(node.text, technical);
        // Object properties were handled above, including their color values.
        if ((!ts.isPropertyAssignment(node.parent) || node.parent.initializer !== node)
          && !ts.isJsxAttribute(node.parent)) {
          for (const color of colors(node.text, false)) add('literal-color', color);
        }
      }
      ts.forEachChild(node, child => visit(child, technical));
    };
    visit(ast, false);
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
  return JSON.stringify([entry.file, entry.rule, entry.pattern, entry.occurrence]);
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
    `  ${entry.file} [${entry.rule}] ${entry.pattern} (occurrence ${entry.occurrence})`).join('\n');
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
