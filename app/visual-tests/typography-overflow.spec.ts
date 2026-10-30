import { expect, test, type Page } from '@playwright/test';

const destinations = [
  ['General'], ['General', 'Keyboard shortcuts'], ['Recording'], ['Customize'], ['Delivery'], ['Meetings'],
  ['Text & vocabulary'], ['AI & models'], ['Modes'], ['Appearance'],
  ['Performance Lab'], ['Diagnostics'],
  ['AI & models', 'Speech-to-text'], ['AI & models', 'Voice Query'],
  ['AI & models', 'Selected-text rewrite'],
  ['Customize', 'Voice commands'], ['Customize', 'Styles'], ['Customize', 'Transforms'],
] as const;

async function audit(page: Page) {
  return page.locator('.settings-workspace').evaluate(workspace => {
    const clipped: string[] = [];
    const tracking: string[] = [];
    const walker = document.createTreeWalker(workspace, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const element = node.parentElement;
      if (!element || element.closest('.sr-only, .ui-visually-hidden') || !node.textContent?.trim() || !element.checkVisibility({ visibilityProperty: true })) continue;
      const style = getComputedStyle(element);
      if (parseFloat(style.fontSize) < 18 && (parseFloat(style.letterSpacing) || 0) !== 0) tracking.push(node.textContent.trim());
      // Ignore intentional one-line truncation; check every text box, even
      // below the vertical scroll viewport, for horizontal clipping.
      if (style.textOverflow === 'ellipsis' || element.closest('.truncate, .line-clamp-2, .line-clamp-3, .peer')) continue;
      const range = document.createRange();
      range.selectNodeContents(node);
      for (const rect of range.getClientRects()) {
        if (!rect.width || !rect.height) continue;
        for (let parent: Element | null = element; parent && parent !== workspace; parent = parent.parentElement) {
          const css = getComputedStyle(parent);
          if (!['hidden', 'clip', 'auto', 'scroll'].includes(css.overflowX)) continue;
          // Horizontally scrolling tab strips remain reachable by scrolling.
          if (['auto', 'scroll'].includes(css.overflowX) && parent.scrollWidth > parent.clientWidth + 1) break;
          const bounds = parent.getBoundingClientRect();
          if (rect.right > bounds.right + 1 || rect.left < bounds.left - 1) {
            clipped.push(node.textContent.trim());
            break;
          }
        }
      }
    }
    return { clipped: [...new Set(clipped)], tracking: [...new Set(tracking)], horizontalOverflow: workspace.scrollWidth - workspace.clientWidth };
  });
}

for (const appearance of ['light', 'dark']) {
  for (const size of [{ width: 880, height: 720 }, { width: 720, height: 560 }]) {
    test(`${appearance} settings panes and menus fit ${size.width}x${size.height}`, async ({ page }, testInfo) => {
      test.setTimeout(90_000);
      await page.setViewportSize(size);
      await page.emulateMedia({ colorScheme: appearance, reducedMotion: 'reduce' });
      const report: unknown[] = [];
      for (const path of destinations) {
        await page.goto(`/visual-fixtures.html?state=settings-appearance&appearance=${appearance}`);
        await page.locator('.settings-workspace aside').getByRole('button', { name: path[0], exact: true }).click();
        if (path.length > 1) await page.locator('main').getByRole('button', { name: new RegExp(path[1]) }).first().click();
        const result = await audit(page);
        report.push({ pane: path.join(' / '), ...result });
        expect.soft(result.tracking, path.join(' / ')).toEqual([]);
        expect.soft(result.clipped, path.join(' / ')).toEqual([]);
        expect.soft(result.horizontalOverflow, path.join(' / ')).toBe(0);
        // Inspect every exposed select, without changing the selected value.
        for (const trigger of await page.getByRole('combobox').all()) {
          if (!await trigger.isVisible() || !await trigger.isEnabled()) continue;
          await trigger.click();
          const list = page.getByRole('listbox');
          if (await list.count()) {
            expect.soft(await list.evaluate(el => el.scrollWidth - el.clientWidth), `${path.join(' / ')} listbox`).toBe(0);
            await page.keyboard.press('Escape');
          }
        }
      }
      await page.goto(`/visual-fixtures.html?state=idle&appearance=${appearance}`);
      for (const name of [/^Filter transcripts/, 'More history actions', 'More transcript actions']) {
        const trigger = page.getByRole('button', { name }).first();
        if (name === 'More transcript actions') await page.locator('.transcript-card').first().hover();
        await trigger.click();
        const menu = page.getByRole('menu');
        await expect(menu).toBeVisible();
        expect.soft(await menu.evaluate(el => el.scrollWidth - el.clientWidth)).toBe(0);
        await page.keyboard.press('Escape');
      }
      await testInfo.attach('settings-text-audit', { body: JSON.stringify(report, null, 2), contentType: 'application/json' });
    });
  }
}
