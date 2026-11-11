import { expect, test, type Page } from '@playwright/test';

const panes = [
  ['General'], ['General', 'Keyboard shortcuts'], ['Recording'], ['Customize'],
  ['Delivery'], ['Meetings'], ['Text & vocabulary'], ['AI & models'], ['Modes'],
  ['Appearance'], ['Performance Lab'], ['Diagnostics'],
  ['AI & models', 'Speech-to-text'],
  ['Customize', 'Voice commands'],
  ['Customize', 'App overrides'],
  ['Text & vocabulary', 'Vocabulary'], ['Text & vocabulary', 'Aliases'],
  ['Text & vocabulary', 'Knowledge'], ['Text & vocabulary', 'Manage project scan'],
] as const;

async function openPane(page: Page, path: readonly string[], appearance: string) {
  await page.goto(`/visual-fixtures.html?state=${path[0] === 'Appearance' ? 'settings-appearance' : 'settings'}&appearance=${appearance}`);
  await expect(page.locator('[data-visual-ready="true"]')).toBeVisible();
  await page.locator('.settings-sidebar').getByRole('button', { name: path[0], exact: true }).click();
  if (path[1] === 'Manage project scan') { await page.getByRole('button', { name: /^Advanced/ }).click(); await page.getByRole('switch', { name: 'Developer terms', exact: true }).check(); }
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
      await expect(page.locator('main')).toBeAttached();
      if (path[0] === 'Appearance') await expect(page.getByRole('button', { name: 'Use OpenCode theme', exact: true })).toBeVisible();
      if (path.length === 1 && path[0] === 'Customize') {
        const rows = page.locator('[aria-label="Customization destinations"] > li > .settings-setting-row');
        await expect(rows).toHaveCount(4);
        for (const row of await rows.all()) expect((await row.boundingBox())!.height).toBeGreaterThanOrEqual(52);
      }
      if (path[0] === 'Performance Lab') {
        // Two-line test-length options must stay inside their button and the track.
        await expect(page.locator('.settings-segmented-stacked > .settings-segmented-btn')).toHaveCount(3);
        // Measure in one pass so a late layout shift cannot fall between two readings.
        const { track, options } = await page.locator('.settings-segmented-stacked').evaluate(element => {
          const box = (node: Element) => { const rect = node.getBoundingClientRect(); return { top: rect.top, bottom: rect.bottom }; };
          return {
            track: box(element),
            options: Array.from(element.querySelectorAll(':scope > .settings-segmented-btn'), button => ({
              button: box(button),
              lines: Array.from(button.querySelectorAll('span'), box),
            })),
          };
        });
        for (const { button, lines } of options) {
          expect(button.bottom).toBeLessThanOrEqual(track.bottom);
          expect(lines).toHaveLength(2);
          for (const line of lines) {
            expect(line.top).toBeGreaterThanOrEqual(button.top);
            expect(line.bottom).toBeLessThanOrEqual(button.bottom);
          }
        }
      }
    });
  }
}
