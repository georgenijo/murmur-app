import { expect, test } from '@playwright/test';

// Selection mode adds a checkbox column and a bottom bar to Home history.
// These assertions catch the layout breaking: checkbox overlapping the time,
// the body losing its column, or the bar overflowing the history width.
for (const appearance of ['light', 'dark'] as const) {
  for (const size of [{ width: 1100, height: 780 }, { width: 840, height: 620 }]) {
    test.describe(`History select to combine ${appearance} ${size.width}x${size.height}`, () => {
      test.use({ viewport: size, colorScheme: appearance, timezoneId: 'America/New_York' });
      test.beforeEach(async ({ page }) => {
        await page.clock.setFixedTime(new Date('2026-09-04T20:00:00Z'));
        await page.goto(`/visual-fixtures.html?state=idle&appearance=${appearance}`);
        await page.getByRole('button', { name: 'More history actions' }).click();
        await page.getByRole('menuitem', { name: 'Select transcripts to combine…' }).click();
      });

      test('checkbox, time and text sit in separate columns', async ({ page }) => {
        const row = page.locator('.transcript-card').first();
        const box = await row.locator('.transcript-select').boundingBox();
        const time = await row.locator('.transcript-time').boundingBox();
        const body = await row.locator('.transcript-body').boundingBox();
        const card = await row.boundingBox();
        expect(box && time && body && card).toBeTruthy();
        expect(box!.x).toBeGreaterThanOrEqual(card!.x);
        expect(box!.x + box!.width).toBeLessThanOrEqual(time!.x + 0.5);
        expect(time!.x + time!.width).toBeLessThanOrEqual(body!.x + 0.5);
        expect(body!.x + body!.width).toBeLessThanOrEqual(card!.x + card!.width + 0.5);
        await expect(row.locator('.transcript-actions')).toBeHidden();
      });

      test('selecting rows fills them and the bar stays within the history column', async ({ page }) => {
        const rows = page.locator('.transcript-card');
        await rows.nth(0).click();
        await rows.nth(1).click();
        await expect(rows.nth(0)).toHaveAttribute('data-selected', 'true');
        const fill = await rows.nth(0).evaluate((n) => getComputedStyle(n).backgroundColor);
        expect(fill).not.toBe('rgba(0, 0, 0, 0)');

        const bar = page.getByRole('group', { name: 'Selected transcripts', exact: true });
        await expect(bar).toContainText('2 selected');
        await expect(bar.getByRole('button', { name: 'Copy combined' })).toBeEnabled();
        const barBox = await bar.boundingBox();
        const listBox = await page.locator('.home-history .history-list').boundingBox();
        expect(barBox!.x).toBeGreaterThanOrEqual(listBox!.x - 0.5);
        expect(barBox!.x + barBox!.width).toBeLessThanOrEqual(listBox!.x + listBox!.width + 0.5);
        const overflow = await bar.evaluate((n) => n.scrollWidth - n.clientWidth);
        expect(overflow).toBeLessThanOrEqual(0);
        // Pinned to the bottom of the viewport when the list is taller than it.
        expect(barBox!.y + barBox!.height).toBeLessThanOrEqual(size.height + 0.5);
      });

      test('Done restores the normal row layout', async ({ page }) => {
        await page.getByRole('group', { name: 'Selected transcripts', exact: true }).getByRole('button', { name: 'Done' }).click();
        await expect(page.locator('.transcript-select')).toHaveCount(0);
        await expect(page.getByRole('group', { name: 'Selected transcripts', exact: true })).toHaveCount(0);
      });
    });
  }
}
