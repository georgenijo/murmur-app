import { expect, test, type Page } from '@playwright/test';

const panes = [
  ['General'], ['General', 'Keyboard shortcuts'], ['Recording'], ['Customize'],
  ['Delivery'], ['Meetings'], ['Text & vocabulary'], ['AI & models'], ['Modes'],
  ['Appearance'], ['Performance Lab'], ['Diagnostics'],
  ['AI & models', 'Speech-to-text'], ['AI & models', 'Voice Query'],
  ['AI & models', 'Selected-text rewrite'], ['Customize', 'Voice commands'],
  ['Customize', 'Styles'], ['Customize', 'Transforms'],
  ['Text & vocabulary', 'Vocabulary'], ['Text & vocabulary', 'Aliases'],
  ['Text & vocabulary', 'Knowledge'], ['Text & vocabulary', 'Manage project scan'],
] as const;

async function openPane(page: Page, path: readonly string[], appearance: string) {
  await page.goto(`/visual-fixtures.html?state=${path[0] === 'Appearance' ? 'settings-appearance' : 'settings'}&appearance=${appearance}`);
  await expect(page.locator('[data-visual-ready="true"]')).toBeVisible();
  await page.locator('.settings-sidebar').getByRole('button', { name: path[0], exact: true }).click();
  if (path[1] === 'Manage project scan') { await page.locator('summary').filter({ hasText: /^Advanced/ }).click(); await page.getByRole('switch', { name: 'Developer terms', exact: true }).check(); }
  if (path[1]) await page.locator('main').getByRole('button', { name: new RegExp(path[1]) }).first().click();
  await page.evaluate(() => document.fonts.ready);
}

for (const appearance of ['light', 'dark']) {
  for (const path of panes) {
    const name = `${appearance}-${path.join('-').toLowerCase().replace(/[^a-z0-9-]+/g, '-')}`;
    test(`${name} settings pane`, async ({ page }) => {
      const errors: string[] = [];
      page.on('pageerror', error => errors.push(error.message));
      await page.emulateMedia({ colorScheme: appearance as 'light' | 'dark', reducedMotion: 'reduce' });
      await openPane(page, path, appearance);
      expect(errors).toEqual([]);
      await expect(page.locator('.settings-sidebar')).toBeVisible();
      await expect(page.locator('main')).toBeVisible();
      if (path[0] === 'Appearance') await expect(page.getByRole('button', { name: 'Use OpenCode theme', exact: true })).toBeVisible();
      if (path.length === 1 && path[0] === 'Customize') {
        for (const row of await page.locator('.settings-hub-card > button').all()) expect((await row.boundingBox())!.height).toBeGreaterThanOrEqual(66);
      }
      if (process.env.E848_BEFORE) await page.screenshot({ path: `${process.env.E848_BEFORE}/${name}.png` });
      else await expect(page).toHaveScreenshot(`${name}.png`);
    });
  }
}
