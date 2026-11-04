import { describe, expect, it } from 'vitest';
import { assertDesignBaseline, compareDesignBaseline, isDesignSource, scanDesignSource } from './designLint';

const scan = (source: string, file = 'components/Example.tsx') => scanDesignSource(file, source);

describe('design lint rules', () => {
  it('finds all four rules in TSX, including variants and inline styles', () => {
    const entries = scan('<div className="hover:p-[7px] text-[11px] font-mono bg-red-500" style={{ color: "#abcdef", fontSize: 15 }} />');
    expect(new Set(entries.map(entry => entry.rule))).toEqual(new Set(['literal-color', 'font-size', 'arbitrary-px', 'prose-mono']));
    expect(entries).toEqual(expect.arrayContaining([
      expect.objectContaining({ rule: 'arbitrary-px', pattern: 'hover:p-[7px]' }),
      expect.objectContaining({ rule: 'font-size', pattern: 'fontSize: 15' }),
      expect.objectContaining({ rule: 'literal-color', pattern: 'color: #abcdef' }),
      expect.objectContaining({ rule: 'literal-color', pattern: 'bg-red-500' }),
    ]));
  });

  it('finds colors, non-scale sizes, and prose mono in CSS, including nested rules', () => {
    const entries = scan(`@media (width > 700px) { .stats { color: red; border: 1px solid #fff; background: rgb(1 2 3); font-size: 14px; font-family: var(--home-font-mono); } }`, 'styles/example.css');
    expect(entries.filter(entry => entry.rule === 'literal-color')).toHaveLength(3);
    expect(entries).toEqual(expect.arrayContaining([
      expect.objectContaining({ rule: 'font-size', pattern: '.stats | font-size: 14px' }),
      expect.objectContaining({ rule: 'prose-mono', pattern: '.stats | font-family: var(--home-font-mono)' }),
    ]));
  });

  it('checks CSS @apply, named SVG colors, built-in type utilities, and non-px sizes', () => {
    expect(scan('.x { @apply hover:w-[calc(100%-7px)] text-sm font-mono; }', 'x.css').map(v => v.rule))
      .toEqual(['arbitrary-px', 'font-size', 'prose-mono']);
    expect(scan('<svg fill="red" stroke="#123456"><text fontSize="1em" /></svg>').map(v => v.rule))
      .toEqual(['literal-color', 'literal-color', 'font-size']);
    expect(scan('<div className="text-[1.2rem] text-[length:var(--local-size)]" />').filter(v => v.rule === 'font-size')).toHaveLength(2);
    expect(scan('<div className="text-[red] bg-[color:rebeccapurple]" />').filter(v => v.rule === 'literal-color')).toHaveLength(2);
    expect(scan('.x { @apply bg-[#abcdef]; }', 'x.css')).toEqual([
      expect.objectContaining({ rule: 'literal-color', pattern: '@apply: #abcdef' }),
    ]);
  });

  it('permits tokens, semantic paints and technical elements, but not prose siblings', () => {
    expect(scan(`:root { --ui-font-body: 14px; --murmur-primary: #fff; }
      .copy { color: var(--murmur-primary); background: transparent; font-size: var(--ui-font-body); }
      pre code, kbd { font-family: var(--home-font-mono); }`, 'styles.css')).toEqual([]);
    expect(scan('<pre><code className="font-mono">code</code></pre><kbd className="font-mono" /><samp className="font-mono" />')).toEqual([]);
    expect(scan('<div className="text-[length:var(--ui-font-body)] text-(length:--ui-font-body)" style={{ fontSize: "var(--ui-font-body)", color: "var(--murmur-primary)" }} />')).toEqual([]);
    expect(scan('<><code className="font-mono" /><p className="font-mono" /></>')).toHaveLength(1);
    expect(scan('code, .prose { font-family: var(--home-font-mono); }', 'x.css')).toHaveLength(1);
    expect(scan('.decode { font-family: var(--home-font-mono); }', 'x.css')).toHaveLength(1);
    expect(scan('code + p, p:has(code) { font-family: var(--home-font-mono); }', 'x.css')).toHaveLength(1);
  });

  it('ignores comments, tests, and fixtures without exempting adjacent production files', () => {
    const source = '<div className="p-[7px] font-mono" />';
    for (const file of ['Example.test.tsx', '__tests__/Example.tsx', '__fixtures__/Example.tsx', 'visual-fixtures.tsx', 'visual-fixtures/Example.tsx', 'Example.fixture.tsx']) {
      expect(scan(source, file)).toEqual([]);
    }
    expect(isDesignSource('components/FixtureEditor.tsx')).toBe(true);
    expect(scan('// "p-[7px] #fff font-mono"\nconst x = <div />;')).toEqual([]);
    expect(scan('/* color: red; font-size: 17px; */ .x { color: var(--murmur-primary); }', 'x.css')).toEqual([]);
  });
});

describe('location-aware design ratchet', () => {
  const legacy = '<div className="p-[7px] p-[7px]" />';
  const baseline = scan(legacy);

  it('fails new violations with a file and rule, even when totals stay equal', () => {
    const changed = scan('<div className="p-[7px] p-[9px]" />');
    expect(() => assertDesignBaseline(changed, baseline)).toThrow('components/Example.tsx [arbitrary-px] p-[9px]');
    expect(() => assertDesignBaseline(scan(legacy, 'Other.tsx'), baseline)).toThrow('Other.tsx [arbitrary-px]');
  });

  it('fails stale entries when a baselined occurrence is removed', () => {
    expect(baseline.map(entry => entry.occurrence)).toEqual([1, 2]);
    expect(() => assertDesignBaseline(scan('<div className="p-[7px]" />'), baseline))
      .toThrow('Stale baseline entries');
    expect(() => assertDesignBaseline([], baseline)).toThrow('components/Example.tsx [arbitrary-px]');
  });

  it('ignores unrelated edits and line shifts', () => {
    expect(scan(`// unrelated comment\n\nconst count = 1;\n${legacy}`)).toEqual(baseline);
    const css = '.x { padding: 1px; font-size: 17px; }';
    expect(scan(`/* comment */\n${css}`, 'x.css')).toEqual(scan(css, 'x.css'));
  });

  it('allows regeneration only to shrink; new violations still fail in shrink mode', () => {
    expect(() => assertDesignBaseline([], baseline, true)).not.toThrow();
    expect(compareDesignBaseline([], baseline).stale).toEqual(baseline);
    expect(() => assertDesignBaseline(scan('<div className="p-[9px]" />'), baseline, true))
      .toThrow('regeneration cannot add them');
  });
});
