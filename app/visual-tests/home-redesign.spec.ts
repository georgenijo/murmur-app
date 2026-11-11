import { expect, test } from '@playwright/test';

for (const appearance of ['light', 'dark'] as const) {
  for (const size of [{ width: 1100, height: 780 }, { width: 840, height: 620 }]) {
    test.describe(`Home redesign ${appearance} ${size.width}x${size.height}`, () => {
      test.use({ viewport: size, colorScheme: appearance, timezoneId: 'America/New_York' });
      test.beforeEach(async ({ page }) => {
        await page.clock.setFixedTime(new Date('2026-09-04T20:00:00Z'));
      });
      test('newest and later rows receive the same pointer hover fill', async ({ page }) => {
        await page.goto(`/visual-fixtures.html?state=idle&appearance=${appearance}`);
        const rows = page.locator('.transcript-card');
        const fills: string[] = [];
        for (const index of [0, 1]) {
          const row = rows.nth(index);
          await row.hover();
          await expect(row.locator('.transcript-actions')).toHaveCSS('opacity', '1');
          const fill = await row.evaluate(n => getComputedStyle(n).backgroundColor);
          expect(fill).not.toBe('rgba(0, 0, 0, 0)');
          fills.push(fill);
        }
        expect(fills[0]).toBe(fills[1]);
      });
      test('empty history has one dictation action and no passive usage strip', async ({ page }) => {
        await page.goto(`/visual-fixtures.html?state=home-empty&appearance=${appearance}`);
        await expect(page.getByRole('button', { name: 'Start dictation', exact: true })).toBeEnabled();
        await expect(page.getByRole('heading', { name: 'No dictations yet' })).toBeVisible();
        await expect(page.locator('.home-talk-footer')).toHaveCount(0);
        await expect(page.getByRole('searchbox', { name: 'Search transcripts' })).toBeDisabled();
        await expect(page.getByRole('button', { name: 'Filter transcripts' })).toBeDisabled();
        await expect(page.getByRole('button', { name: 'More history actions' })).toBeDisabled();
      });
      test('confirmed microphone denial has a single recovery action', async ({ page }) => {
        await page.goto(`/visual-fixtures.html?state=home-mic-denied&appearance=${appearance}`);
        await expect(page.getByRole('button', { name: 'Start dictation — microphone access required' })).toBeDisabled();
        await expect(page.getByRole('button', { name: 'Open System Settings…', exact: true })).toBeEnabled();
        await expect(page.locator('.home-talk [role="status"]')).toBeVisible();
        expect(await page.locator('.home-dashboard').evaluate(n => n.scrollWidth <= n.clientWidth)).toBe(true);
      });
    });
  }
}
