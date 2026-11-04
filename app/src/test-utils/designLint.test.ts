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
      expect.objectContaining({ rule: 'literal-color', pattern: 'bg-[#abcdef]' }),
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

  it('keeps literal JSX expressions attached to their styling attributes', () => {
    expect(scan('<svg fill={"red"} stroke={"blue"}><text fontSize={17} /></svg>').map(v => v.rule))
      .toEqual(['literal-color', 'literal-color', 'font-size']);
    expect(scan('<div style={{ fontSize: 17 }} /><text fontSize={"var(--ui-font-body)"} />'))
      .toEqual([expect.objectContaining({ rule: 'font-size', pattern: 'fontSize: 17' })]);
  });

  it('checks important modifiers and arbitrary CSS property utilities', () => {
    const entries = scan('<div className="text-sm! !text-sm hover:!text-sm hover:text-sm! font-mono! bg-red-500! [font-size:1em] [color:red] [font-family:monospace]" />');
    expect(entries.filter(v => v.rule === 'font-size')).toHaveLength(5);
    expect(entries.filter(v => v.rule === 'literal-color')).toHaveLength(2);
    expect(entries.filter(v => v.rule === 'prose-mono')).toHaveLength(2);
    expect(scan('<code className="!font-mono [font-family:monospace] [font-size:var(--ui-font-body)] [color:var(--murmur-primary)]" />')).toEqual([]);
  });

  it('requires the font shorthand size itself to be a scale token', () => {
    const entries = scan('.x { font: 30px/var(--ui-font-body) sans-serif; font: calc(var(--ui-font-body) * 3) sans-serif; }', 'x.css');
    expect(entries.filter(v => v.rule === 'font-size')).toHaveLength(2);
    expect(scan('.x { font: italic 600 var(--ui-font-body)/1.5 sans-serif; font: var(--ui-font-body) var(--ui-font-sans); }', 'x.css')).toEqual([]);
  });

  it('parses nested color functions and var fallbacks without reading token names as colors', () => {
    expect(scan('.x { color: rgb(var(--theme-primary-rgb)); background: var(--brand-red, var(--murmur-primary)); }', 'x.css')).toEqual([]);
    const entries = scan('.x { color: var(--brand-red, var(--other, red)); background: var(--brand, rgb(1 2 3)); border-color: var(--border, #abc); }', 'x.css');
    expect(entries.map(v => v.pattern)).toEqual(['.x | color: red', '.x | background: rgb(1 2 3)', '.x | border-color: #abc']);
  });

  it('limits literal colors to style declarations and styling attributes', () => {
    expect(scan('<a href="#abc" title="red" data-color="#fff">#fff</a>')).toEqual([]);
    expect(scan('const metadata = { href: "#abc", title: "red" }; <svg fill="#abc" style={{ color: "red" }} />').map(v => v.rule))
      .toEqual(['literal-color', 'literal-color']);
  });

  it('retains conditional styling colors and resolves local paint constants', () => {
    expect(scan('const paint = active ? "#abc" : "#def"; <div style={{ color: paint, background: active ? "red" : "blue" }} /><svg fill={active ? "#123" : "#456"} />').filter(v => v.rule === 'literal-color')).toHaveLength(6);
    expect(scan('const paint = "#abc"; <a href={paint} />')).toEqual([]);
    expect(scan('const paint = "#abc"; function Card(paint: string) { return <svg fill={paint} />; }')).toEqual([]);
    expect(scan('const paint = "#abc"; function Card() { const paint = "#def"; return <svg fill={paint} />; }').map(v => v.pattern)).toEqual(['fill: #def']);
  });

  it('resolves technical selector functions and nested ancestors', () => {
    expect(scan(':is(code, pre) { font-family: monospace; } code:not(.a, .b) { font-family: monospace; } code { & span { font-family: monospace; @apply font-mono; } }', 'x.css')).toEqual([]);
    expect(scan(':is(code, .prose) { font-family: monospace; } code { & + p { font-family: monospace; } }', 'x.css').filter(v => v.rule === 'prose-mono')).toHaveLength(2);
    expect(scan('code, pre { & span, & kbd { font-family: monospace; } }', 'x.css')).toEqual([]);
  });

  it('ignores comments, tests, and fixtures without exempting adjacent production files', () => {
    const source = '<div className="p-[7px] font-mono" />';
    for (const file of ['Example.test.tsx', '__tests__/Example.tsx', '__fixtures__/Example.tsx', 'visual-fixtures.tsx', 'visual-fixtures/Example.tsx', 'Example.fixture.tsx']) {
      expect(scan(source, file)).toEqual([]);
    }
    expect(isDesignSource('components/FixtureEditor.tsx')).toBe(true);
    expect(scan('const separator = ` : ${name}: `;')).toEqual([]);
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

  it('rejects replacement violations on a different element, component, or CSS declaration', () => {
    const original = '<><header className="text-sm" /><footer /></>';
    const replacement = '<><header /><footer className="text-sm" /></>';
    expect(compareDesignBaseline(scan(replacement), scan(original))).toMatchObject({ added: [expect.anything()], stale: [expect.anything()] });
    expect(() => assertDesignBaseline(scan(replacement), scan(original))).toThrow('New violations');
    const neighbours = '<><div className="header text-sm" /><div className="footer" /></>';
    expect(() => assertDesignBaseline(scan('<><div className="header" /><div className="footer text-sm" /></>'), scan(neighbours))).toThrow('New violations');
    expect(() => assertDesignBaseline(scan('function Footer() { return <div className="text-sm" />; }'), scan('function Header() { return <div className="text-sm" />; }'))).toThrow('New violations');
    expect(() => assertDesignBaseline(scan('.footer { @apply text-sm; }', 'x.css'), scan('.header { @apply text-sm; }', 'x.css'))).toThrow('New violations');
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

  it('keeps remaining identities when another violation is fixed', () => {
    const before = scan('<div className="header text-sm p-[7px]" />');
    const after = scan('<div className="header p-[7px]" />');
    expect(compareDesignBaseline(after, before).added).toEqual([]);
    expect(compareDesignBaseline(after, before).stale).toHaveLength(1);
    expect(scan('<><aside /><header className="text-sm" /></>')).toEqual(scan('<><header className="text-sm" /></>'));
  });

  it('allows regeneration only to shrink; new violations still fail in shrink mode', () => {
    expect(() => assertDesignBaseline([], baseline, true)).not.toThrow();
    expect(compareDesignBaseline([], baseline).stale).toEqual(baseline);
    expect(() => assertDesignBaseline(scan('<div className="p-[9px]" />'), baseline, true))
      .toThrow('regeneration cannot add them');
  });
});
