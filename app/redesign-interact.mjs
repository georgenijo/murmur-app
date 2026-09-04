// Interaction-proof screenshots for the three homepage redesign variants.
// usage: node redesign-interact.mjs
import { chromium } from '@playwright/test';
import { mkdirSync } from 'fs';

const OUT_DIR = '/tmp/murmur-redesign/interact';
mkdirSync(OUT_DIR, { recursive: true });

const BASE = 'http://127.0.0.1:1420/visual-fixtures.html';

const report = [];

function log(name, detail, errors) {
  const errText = errors && errors.length ? ` ERRORS: ${errors.join(' | ').slice(0, 500)}` : '';
  const line = `${name} :: ${detail}${errText}`;
  report.push(line);
  console.log(line);
}

async function openPage(browser, { state, appearance = 'light', width = 1120, height = 800 }) {
  const page = await browser.newPage({
    viewport: { width, height },
    deviceScaleFactor: 2,
    reducedMotion: 'no-preference',
  });
  const errors = [];
  page.on('pageerror', (e) => errors.push(`[pageerror] ${String(e)}`));
  page.on('console', (m) => {
    if (m.type() === 'error') errors.push(`[console] ${m.text()}`);
  });
  await page.goto(`${BASE}?state=${state}&appearance=${appearance}`, { waitUntil: 'networkidle' });
  await page.locator('[data-visual-ready="true"]').waitFor();
  await page.waitForTimeout(300);
  return { page, errors };
}

async function shot(page, name, errors, detail) {
  await page.waitForTimeout(700);
  const out = `${OUT_DIR}/${name}.png`;
  await page.screenshot({ path: out });
  log(name, detail ?? 'captured', errors);
  errors.length = 0; // reset so later shots on the same page only report new errors
}

const browser = await chromium.launch();

/* ────────────────────────────── Variant A ────────────────────────────── */
{
  const { page, errors } = await openPage(browser, { state: 'redesign-a' });

  // 1. Row overflow menu (SmartOverflow "..." trigger) open.
  const rowTrigger = page.locator('.variant-a-history-list [data-slot="smart-overflow-trigger"]').first();
  await rowTrigger.click();
  await page.waitForTimeout(400);
  const rowMenuOpen = await page.evaluate(() => !!document.querySelector('[data-popup-open]'));
  const rowMenuHasItems = await page.getByText('Correct & Teach').first().isVisible().catch(() => false);
  await shot(page, 'a-row-overflow-open-light', errors, `data-popup-open present=${rowMenuOpen}, "Correct & Teach" item visible=${rowMenuHasItems}`);
  await page.keyboard.press('Escape');
  await page.waitForTimeout(200);

  // 1b. Dark variant of the same state (most interesting state for A).
  await page.close();
  const darkA = await openPage(browser, { state: 'redesign-a', appearance: 'dark' });
  const rowTriggerDark = darkA.page.locator('.variant-a-history-list [data-slot="smart-overflow-trigger"]').first();
  await rowTriggerDark.click();
  await darkA.page.waitForTimeout(400);
  const rowMenuOpenDark = await darkA.page.evaluate(() => !!document.querySelector('[data-popup-open]'));
  await shot(darkA.page, 'a-row-overflow-open-dark', darkA.errors, `data-popup-open present=${rowMenuOpenDark}`);
  await darkA.page.close();

  // Fresh page for the remaining light-mode Variant A states.
  const a2 = await openPage(browser, { state: 'redesign-a' });

  // 2. "Mic" fluid-tab selected.
  await a2.page.getByRole('tab', { name: 'Mic' }).click();
  await a2.page.waitForTimeout(200);
  const micSelected = await a2.page.getByRole('tab', { name: 'Mic' }).getAttribute('data-selected');
  await shot(a2.page, 'a-mic-tab-selected-light', a2.errors, `Mic tab data-selected=${micSelected}`);

  // 3. Keyboard focus on Start Recording (Tab through).
  await a2.page.evaluate(() => (document.activeElement)?.blur());
  let focusedTestId = null;
  for (let i = 0; i < 40; i += 1) {
    await a2.page.keyboard.press('Tab');
    focusedTestId = await a2.page.evaluate(() => document.activeElement?.getAttribute('data-testid') ?? null);
    if (focusedTestId === 'home-record-button') break;
  }
  await shot(a2.page, 'a-start-recording-focus-light', a2.errors, `reached data-testid="home-record-button" via Tab=${focusedTestId === 'home-record-button'}`);

  // 4. Hover on Start Recording.
  await a2.page.locator('[data-testid="home-record-button"]').hover();
  await shot(a2.page, 'a-start-recording-hover-light', a2.errors, 'hovered [data-testid="home-record-button"]');
  await a2.page.close();

  // 880x720 scroll-to-bottom check.
  const a3 = await openPage(browser, { state: 'redesign-a', width: 880, height: 720 });
  const scrollInfo = await a3.page.evaluate(() => {
    const list = document.querySelector('.variant-a-history-list');
    if (!list) return null;
    list.scrollTop = list.scrollHeight;
    return { scrollTop: list.scrollTop, scrollHeight: list.scrollHeight, clientHeight: list.clientHeight };
  });
  await shot(a3.page, 'a-scroll-bottom-880x720-light', a3.errors, `scrollInfo=${JSON.stringify(scrollInfo)}`);
  await a3.page.close();
}

/* ────────────────────────────── Variant B ────────────────────────────── */
{
  const { page, errors } = await openPage(browser, { state: 'redesign-b' });

  // 1. Hover the Start expanding-action (per spec: hover should expand it).
  const startBtn = page.getByRole('button', { name: 'Start', exact: true });
  await startBtn.hover();
  await page.waitForTimeout(300);
  const expandedAfterHover = await page.evaluate(() => !!document.querySelector('.vb-expanding [aria-label="Close start options"]'));
  await shot(page, 'b-start-action-hover-light', errors, `expanded after hover=${expandedAfterHover} (ExpandingAction source only expands on click, not hover)`);

  // 1b. Click to actually reach the expanded state, since hover does not.
  await startBtn.click();
  await page.waitForTimeout(400);
  const expandedAfterClick = await page.evaluate(() => !!document.querySelector('.vb-expanding [aria-label="Close start options"]'));
  await shot(page, 'b-start-action-expanded-click-light', errors, `expanded after click=${expandedAfterClick}`);
  // Close it back down before the next interaction.
  await page.getByRole('button', { name: 'Close start options' }).click().catch(() => {});
  await page.waitForTimeout(200);

  // Dark variant of the most interesting state (expanded start action).
  await page.close();
  const darkB = await openPage(browser, { state: 'redesign-b', appearance: 'dark' });
  const startBtnDark = darkB.page.getByRole('button', { name: 'Start', exact: true });
  await startBtnDark.click();
  await darkB.page.waitForTimeout(400);
  const expandedDark = await darkB.page.evaluate(() => !!document.querySelector('.vb-expanding [aria-label="Close start options"]'));
  await shot(darkB.page, 'b-start-action-expanded-dark', darkB.errors, `expanded after click=${expandedDark}`);
  await darkB.page.close();

  const b2 = await openPage(browser, { state: 'redesign-b' });

  // 2. Hover a spotlight-card stat tile, mouse at its center.
  const tile = b2.page.locator('.vb-tile').first();
  await tile.hover();
  await b2.page.waitForTimeout(300);
  const tileLabel = await tile.locator('.vb-tile-label').textContent().catch(() => null);
  await shot(b2.page, 'b-spotlight-tile-hover-light', b2.errors, `hovered tile "${tileLabel}"`);

  // 3. Hover an activity-graph cell so its tooltip shows.
  const cell = b2.page.locator('.vb-graph [data-slot="activity-graph-cell"]').last();
  await cell.hover();
  await b2.page.waitForTimeout(300);
  const tooltipVisible = await b2.page.locator('[data-slot="activity-graph-tooltip"]').first().isVisible().catch(() => false);
  const tooltipText = await b2.page.locator('[data-slot="activity-graph-tooltip-content"]').first().textContent().catch(() => null);
  await shot(b2.page, 'b-activity-graph-tooltip-light', b2.errors, `tooltip visible=${tooltipVisible}, text="${tooltipText}"`);

  // 4. "File" tab selected.
  await b2.page.getByRole('tab', { name: 'File' }).click();
  await b2.page.waitForTimeout(200);
  const fileSelected = await b2.page.getByRole('tab', { name: 'File' }).getAttribute('data-selected');
  await shot(b2.page, 'b-file-tab-selected-light', b2.errors, `File tab data-selected=${fileSelected}`);
  await b2.page.close();

  // 880x720 scroll-to-bottom check.
  const b3 = await openPage(browser, { state: 'redesign-b', width: 880, height: 720 });
  const scrollInfoB = await b3.page.evaluate(() => {
    const list = document.querySelector('.vb-list');
    if (!list) return null;
    list.scrollTop = list.scrollHeight;
    return { scrollTop: list.scrollTop, scrollHeight: list.scrollHeight, clientHeight: list.clientHeight };
  });
  await shot(b3.page, 'b-scroll-bottom-880x720-light', b3.errors, `scrollInfo=${JSON.stringify(scrollInfoB)}`);
  await b3.page.close();
}

/* ────────────────────────────── Variant C ────────────────────────────── */
{
  const { page, errors } = await openPage(browser, { state: 'redesign-c' });

  // 1. "Meetings" tab selected.
  await page.getByRole('tab', { name: 'Meetings' }).click();
  await page.waitForTimeout(200);
  const meetingsSelected = await page.getByRole('tab', { name: 'Meetings' }).getAttribute('data-selected');
  await shot(page, 'c-meetings-tab-selected-light', errors, `Meetings tab data-selected=${meetingsSelected}`);

  // 2. "Queries" tab selected.
  await page.getByRole('tab', { name: 'Queries' }).click();
  await page.waitForTimeout(200);
  const queriesSelected = await page.getByRole('tab', { name: 'Queries' }).getAttribute('data-selected');
  await shot(page, 'c-queries-tab-selected-light', errors, `Queries tab data-selected=${queriesSelected}`);

  // Back to "Recent" for the row-level interactions.
  await page.getByRole('tab', { name: 'Recent' }).click();
  await page.waitForTimeout(200);

  // 3. Hover a transcript row.
  const row = page.locator('.vc-card').first();
  await row.hover();
  await page.waitForTimeout(300);
  const actionsOpacity = await page.evaluate(() => {
    const el = document.querySelector('.vc-card-actions');
    return el ? getComputedStyle(el).opacity : null;
  });
  await shot(page, 'c-transcript-row-hover-light', errors, `.vc-card-actions opacity on hover=${actionsOpacity}`);

  // 4. Overflow/actions on the row revealed (the code reveals the action
  //    bar on hover/focus-within via CSS, then the "..." trigger opens the
  //    overflow menu on click).
  const overflowTrigger = page.locator('.vc-card').first().locator('[data-slot="smart-overflow-trigger"]');
  await overflowTrigger.click();
  await page.waitForTimeout(400);
  const overflowOpen = await page.evaluate(() => !!document.querySelector('[data-popup-open]'));
  await shot(page, 'c-row-overflow-open-light', errors, `data-popup-open present=${overflowOpen}`);
  await page.keyboard.press('Escape');
  await page.waitForTimeout(200);

  // Dark variant of the most interesting state (row overflow open).
  await page.close();
  const darkC = await openPage(browser, { state: 'redesign-c', appearance: 'dark' });
  const rowDark = darkC.page.locator('.vc-card').first();
  await rowDark.hover();
  await darkC.page.waitForTimeout(200);
  const overflowTriggerDark = rowDark.locator('[data-slot="smart-overflow-trigger"]');
  await overflowTriggerDark.click();
  await darkC.page.waitForTimeout(400);
  const overflowOpenDark = await darkC.page.evaluate(() => !!document.querySelector('[data-popup-open]'));
  await shot(darkC.page, 'c-row-overflow-open-dark', darkC.errors, `data-popup-open present=${overflowOpenDark}`);
  await darkC.page.close();

  const c2 = await openPage(browser, { state: 'redesign-c' });

  // 5. Hover an activity-graph cell.
  const cellC = c2.page.locator('.vc-activity [data-slot="activity-graph-cell"]').last();
  await cellC.hover();
  await c2.page.waitForTimeout(300);
  const tooltipVisibleC = await c2.page.locator('[data-slot="activity-graph-tooltip"]').first().isVisible().catch(() => false);
  await shot(c2.page, 'c-activity-graph-hover-light', c2.errors, `tooltip visible=${tooltipVisibleC} (VariantC does not pass showTooltip to ActivityGraph, so no tooltip is expected; only the hover brighten effect should show)`);
  await c2.page.close();

  // 880x720 scroll-to-bottom check.
  const c3 = await openPage(browser, { state: 'redesign-c', width: 880, height: 720 });
  const scrollInfoC = await c3.page.evaluate(() => {
    const list = document.querySelector('.vc-list');
    if (!list) return null;
    list.scrollTop = list.scrollHeight;
    return { scrollTop: list.scrollTop, scrollHeight: list.scrollHeight, clientHeight: list.clientHeight };
  });
  await shot(c3.page, 'c-scroll-bottom-880x720-light', c3.errors, `scrollInfo=${JSON.stringify(scrollInfoC)}`);
  await c3.page.close();
}

await browser.close();

console.log('\n=== SUMMARY ===');
for (const line of report) console.log(line);
