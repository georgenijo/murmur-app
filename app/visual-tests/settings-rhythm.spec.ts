import { expect, test, type Page } from '@playwright/test';

/**
 * Settings rhythm guard. Measures every settings card top to bottom (card
 * edges, full-width divider lines, and clusters of visible content) and fails
 * when a gap leaves the grid documented in `src/styles/settings.css`:
 *
 *   card edge → content         18px  (6px card padding + 12px row padding)
 *   divider   → content → divider  12px on both sides
 *   divider   → divider         never adjacent (no doubled lines)
 *   content   → content         no stray gap wider than a row break
 *
 * Content is measured by its ink (text line boxes, controls, and boxed or
 * tinted surfaces), so the bounds allow for line-height leading around text.
 * A new setting that brings its own margin, padding, or border fails here
 * instead of shipping a lopsided gap.
 */

test.use({ viewport: { width: 1180, height: 3200 } });

const EDGE = { min: 17, max: 21 };
const DIVIDER = { min: 11.5, max: 15.5 };
const MIN_LINE_GAP = 10;
const MAX_CONTENT_GAP = 20;

const PAGES = [
  { path: ['General'], fixtureState: 'settings' },
  { path: ['Recording'], fixtureState: 'settings' },
  { path: ['Recording'], fixtureState: 'settings-smart-auto', label: 'Recording · Smart Auto' },
  { path: ['Delivery'], fixtureState: 'settings' },
  { path: ['Meetings'], fixtureState: 'settings' },
  { path: ['Text & Vocabulary'], fixtureState: 'settings' },
  { path: ['AI & Models'], fixtureState: 'settings' },
  { path: ['AI & Models', 'Speech-to-Text'], fixtureState: 'settings' },
  { path: ['AI & Models', 'Voice Query'], fixtureState: 'settings' },
  { path: ['AI & Models', 'Selected-Text Rewrite'], fixtureState: 'settings' },
] as const;

type Item = { kind: 'edge' | 'line' | 'ink'; top: number; bottom: number; label: string; color?: string };

async function openPage(page: Page, path: readonly string[], fixtureState: string) {
  await page.goto(`/visual-fixtures.html?state=${fixtureState}&appearance=light`);
  await expect(page.locator('[data-visual-ready="true"]')).toBeVisible();
  await page.getByRole('navigation', { name: 'Settings pages' }).getByRole('button', { name: path[0], exact: true }).click();
  if (path[1]) await page.locator('main').getByRole('button', { name: new RegExp(`^${path[1]}`) }).first().click();
  await expect(page.locator('.settings-card-body')).toBeVisible();
}

/** Turns on every switch that owns a dependent branch and opens every
 *  disclosure, so the measured sequence covers each nested layout. */
async function expandEverything(page: Page) {
  for (let pass = 0; pass < 3; pass += 1) {
    // Nested owners appear only once their parent branch opens, hence passes.
    const clicked = await page.locator('.settings-card-body').evaluate((body) => {
      const owners = [...body.querySelectorAll<HTMLElement>('[role="switch"][aria-checked="false"]:not([disabled])')]
        .filter((element) => (element.closest('.settings-setting-row') ?? element.parentElement)
          ?.nextElementSibling?.classList.contains('settings-dependent-branch'));
      owners.forEach((owner) => owner.click());
      body.querySelectorAll('details').forEach((details) => { details.open = true; });
      return owners.length;
    });
    if (clicked === 0) break;
    await page.waitForTimeout(100);
  }
  // Branch reveal is a grid-row transition; wait for it to settle.
  await page.waitForTimeout(400);
}

function measure(): Item[] {
  const body = document.querySelector('.settings-card-body') as HTMLElement;
  const card = body.getBoundingClientRect();
  const visible = (element: Element, rect: DOMRect) => {
    if (!element.checkVisibility({ opacityProperty: true, visibilityProperty: true })) return false;
    for (let node: Element | null = element; node && node !== body; node = node.parentElement) {
      const style = getComputedStyle(node);
      if (style.position === 'fixed') return false;
      if (style.overflowY !== 'visible') {
        const clip = node.getBoundingClientRect();
        if (rect.bottom <= clip.top + 0.5 || rect.top >= clip.bottom - 0.5) return false;
      }
    }
    return true;
  };

  const ink: { top: number; bottom: number; label: string }[] = [];
  const walker = document.createTreeWalker(body, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = node.textContent?.trim();
    if (!text || !node.parentElement) continue;
    const range = document.createRange();
    range.selectNodeContents(node);
    for (const rect of range.getClientRects()) {
      if (rect.height > 0 && rect.width > 0 && visible(node.parentElement, rect)) ink.push({ top: rect.top, bottom: rect.bottom, label: text });
    }
  }
  body.querySelectorAll('*').forEach((element) => {
    const style = getComputedStyle(element);
    const rect = element.getBoundingClientRect();
    // A card row is layout even when it is a button; only its contents are ink.
    if (element.parentElement?.classList.contains('settings-rows')) return;
    const control = element.matches('input:not([type=hidden]), select, textarea, [role=switch], button, svg');
    const boxed = parseFloat(style.borderLeftWidth) > 0 && parseFloat(style.borderTopWidth) > 0 && rect.height > 4;
    const tinted = style.backgroundColor !== 'rgba(0, 0, 0, 0)' && rect.height > 16 && rect.width > card.width * 0.3;
    if ((control ? rect.height > 2 && rect.width > 2 : boxed || tinted) && visible(element, rect)) {
      ink.push({ top: rect.top, bottom: rect.bottom, label: `[${element.tagName.toLowerCase()}]` });
    }
  });
  ink.sort((a, b) => a.top - b.top);

  const clusters: { top: number; bottom: number; label: string }[] = [];
  for (const rect of ink) {
    const last = clusters[clusters.length - 1];
    if (last && rect.top <= last.bottom + 4) {
      last.bottom = Math.max(last.bottom, rect.bottom);
      if (last.label.startsWith('[') && !rect.label.startsWith('[')) last.label = rect.label;
    } else {
      clusters.push({ ...rect });
    }
  }

  const lines: Item[] = [];
  body.querySelectorAll('*').forEach((element) => {
    const style = getComputedStyle(element);
    const rect = element.getBoundingClientRect();
    if (rect.width < card.width * 0.4 || rect.height === 0 || parseFloat(style.borderLeftWidth) > 0 || !visible(element, rect)) return;
    if (parseFloat(style.borderTopWidth) > 0) lines.push({ kind: 'line', top: rect.top, bottom: rect.top + 1, label: 'divider', color: style.borderTopColor });
    if (parseFloat(style.borderBottomWidth) > 0) lines.push({ kind: 'line', top: rect.bottom - 1, bottom: rect.bottom, label: 'divider', color: style.borderBottomColor });
  });

  return [
    { kind: 'edge', top: card.top, bottom: card.top + 1, label: 'card top' },
    { kind: 'edge', top: card.bottom - 1, bottom: card.bottom, label: 'card bottom' },
    ...lines,
    ...clusters.map((cluster) => ({ kind: 'ink' as const, top: cluster.top, bottom: cluster.bottom, label: cluster.label.slice(0, 48) })),
  ].sort((a, b) => a.top - b.top || Number(a.kind === 'ink') - Number(b.kind === 'ink'));
}

function violations(items: Item[]) {
  const problems: string[] = [];
  let previous: Item | null = null;
  for (const item of items) {
    if (previous) {
      const gap = Math.round((item.top - previous.bottom) * 2) / 2;
      const where = `${previous.label} → ${item.label}: ${gap}px`;
      const rule = (kinds: string[]) => kinds.includes(previous!.kind) && kinds.includes(item.kind);
      if (gap > -1) {
        if (previous.kind !== 'ink' && item.kind !== 'ink') {
          if (gap < MIN_LINE_GAP) problems.push(`doubled line (${where})`);
        } else if (rule(['edge', 'ink']) && (previous.kind === 'edge' || item.kind === 'edge')) {
          if (gap < EDGE.min || gap > EDGE.max) problems.push(`card padding off-grid (${where}, want ${EDGE.min}–${EDGE.max})`);
        } else if (previous.kind === 'line' || item.kind === 'line') {
          if (gap < DIVIDER.min || gap > DIVIDER.max) problems.push(`divider spacing off-grid (${where}, want ${DIVIDER.min}–${DIVIDER.max})`);
        } else if (gap > MAX_CONTENT_GAP) {
          problems.push(`stray gap inside a row (${where}, want ≤ ${MAX_CONTENT_GAP})`);
        }
      }
    }
    if (!previous || item.bottom > previous.bottom) previous = item;
  }
  const colors = new Set(items.filter((item) => item.kind === 'line').map((item) => item.color));
  if (colors.size > 1) problems.push(`dividers use ${colors.size} colors: ${[...colors].join(', ')}`);
  return problems;
}

/** Checks the declared container tokens directly. The ink/line scan above
 * catches doubled dividers and edge drift; this catches a missing 8px field
 * gap or 12px nested-stack gap even when both neighboring items are text. */
function containerViolations() {
  const problems: string[] = [];
  const visibleChildren = (container: Element) => [...container.children].filter((child) => {
    const rect = child.getBoundingClientRect();
    const style = getComputedStyle(child);
    return rect.height > 0 && style.display !== 'none' && style.visibility !== 'hidden';
  });
  const checkMargin = (element: Element, expected: number, label: string) => {
    const actual = Number.parseFloat(getComputedStyle(element).marginTop) || 0;
    if (Math.abs(actual - expected) > 0.1) problems.push(`${label}: ${actual}px, want ${expected}px`);
  };

  document.querySelectorAll('.settings-field').forEach((container, containerIndex) => {
    visibleChildren(container).slice(1).forEach((child, childIndex) => {
      checkMargin(child, 8, `field ${containerIndex + 1}, gap ${childIndex + 1}`);
    });
  });
  document.querySelectorAll('.settings-stack').forEach((container, containerIndex) => {
    visibleChildren(container).slice(1).forEach((child, childIndex) => {
      if (child.classList.contains('settings-dependent-branch')) return;
      checkMargin(child, 12, `stack ${containerIndex + 1}, gap ${childIndex + 1}`);
    });
  });
  document.querySelectorAll('.settings-stack > .settings-dependent-branch-open > .settings-dependent-branch-clip > .settings-dependent-branch-content').forEach((content, index) => {
    checkMargin(content, 12, `open branch ${index + 1}`);
  });
  document.querySelectorAll('.settings-title-description').forEach((container, index) => {
    const description = container.querySelector(':scope > p + p');
    if (description) checkMargin(description, 2, `title/description ${index + 1}`);
  });
  return problems;
}

for (const pageCase of PAGES) {
  for (const state of ['default', 'expanded'] as const) {
    test(`${pageCase.label ?? pageCase.path.join(' › ')} (${state}) stays on the settings rhythm`, async ({ page }) => {
      await openPage(page, pageCase.path, pageCase.fixtureState);
      if (state === 'expanded') await expandEverything(page);
      const items = await page.evaluate(measure);
      expect(items.filter((item) => item.kind === 'ink').length).toBeGreaterThan(0);
      expect(violations(items)).toEqual([]);
      expect(await page.evaluate(containerViolations)).toEqual([]);
    });
  }
}
