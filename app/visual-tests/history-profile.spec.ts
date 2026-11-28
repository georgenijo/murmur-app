import { expect, test } from '@playwright/test';

// Opt-in "Show app profile in History". Geometry only: the profile tag stays
// on the row's metadata line inside the text column, a long label truncates
// instead of widening the row, and the App filter narrows the list.
for (const appearance of ['light', 'dark'] as const) {
  for (const size of [{ width: 1100, height: 780 }, { width: 840, height: 620 }]) {
    test(`history profile tags stay inside their rows (${appearance} ${size.width}x${size.height})`, async ({ page }) => {
      await page.setViewportSize(size);
      await page.clock.setFixedTime(new Date('2026-09-04T20:00:00Z'));
      await page.goto(`/visual-fixtures.html?state=home-history-profile&appearance=${appearance}`);

      const tags = page.locator('.transcript-profile-tag');
      await expect(tags.first()).toBeVisible();
      for (const tag of await tags.all()) {
        const card = tag.locator('xpath=ancestor::article[1]');
        const body = await card.locator('.transcript-body').boundingBox();
        const box = await tag.boundingBox();
        expect(body && box).toBeTruthy();
        expect(box!.x).toBeGreaterThanOrEqual(body!.x - 0.5);
        expect(box!.x + box!.width).toBeLessThanOrEqual(body!.x + body!.width + 0.5);
        // One line: a long label truncates rather than wrapping.
        expect(box!.height).toBeLessThanOrEqual(20);
      }
      const long = page.locator('.transcript-profile-tag').filter({ hasText: 'quarterly planning' }).first();
      const inner = long.locator('.truncate');
      expect(await inner.evaluate((node) => node.scrollWidth > node.clientWidth)).toBe(true);

      expect(await page.locator('.home-dashboard').evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
    });
  }
}

test('the App filter narrows history to one profile', async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 780 });
  await page.goto('/visual-fixtures.html?state=home-history-profile&appearance=light');
  const cards = page.locator('[data-testid="transcript-card"]');
  const total = await cards.count();
  await page.getByRole('button', { name: 'Filter transcripts' }).click();
  await expect(page.getByRole('menuitemradio', { name: 'Any app' })).toBeVisible();
  const menu = page.locator('.history-filter-menu');
  const menuBox = await menu.boundingBox();
  expect(menuBox!.width).toBeLessThanOrEqual(288.5);
  await page.getByRole('menuitemradio', { name: 'Slack', exact: true }).click();
  await page.keyboard.press('Escape');
  await expect(page.locator('.history-filtered-note')).toContainText(/Showing \d+ of/);
  const shown = await cards.count();
  expect(shown).toBeGreaterThan(0);
  expect(shown).toBeLessThan(total);
  for (const card of await cards.all()) {
    await expect(card.locator('.transcript-profile-tag')).toHaveText('App profile: Slack');
  }
});

test('no profile tags or App section while the setting is off', async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 780 });
  await page.goto('/visual-fixtures.html?state=idle&appearance=light');
  await expect(page.locator('[data-testid="transcript-card"]').first()).toBeVisible();
  await expect(page.locator('.transcript-profile-tag')).toHaveCount(0);
  await page.getByRole('button', { name: 'Filter transcripts' }).click();
  await expect(page.getByRole('menuitemradio', { name: 'Any time' })).toBeVisible();
  await expect(page.getByRole('menuitemradio', { name: 'Any app' })).toHaveCount(0);
});
