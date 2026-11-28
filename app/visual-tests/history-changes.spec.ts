import { expect, test } from '@playwright/test';

// Opt-in "Show what Murmur changed" panel inside a History row. Geometry only:
// the panel must stay inside its card's text column and never widen the page.
for (const appearance of ['light', 'dark'] as const) {
  for (const size of [{ width: 1100, height: 780 }, { width: 840, height: 620 }]) {
    test(`history comparison stays inside its row (${appearance} ${size.width}x${size.height})`, async ({ page }) => {
      await page.setViewportSize(size);
      await page.clock.setFixedTime(new Date('2026-09-04T20:00:00Z'));
      await page.goto(`/visual-fixtures.html?state=home-history-changes&appearance=${appearance}`);

      const card = page.locator('[data-testid="transcript-card"]').filter({ hasText: 'History owns the window' });
      await card.hover();
      await card.getByRole('button', { name: 'More transcript actions' }).click();
      await page.getByRole('menuitem', { name: 'Show what changed' }).click();

      const panel = card.getByRole('region', { name: 'What Murmur changed' });
      await expect(panel).toBeVisible();
      await expect(panel).toContainText('Changed by Cleanup.');
      await expect(panel.locator('del').first()).toBeVisible();
      await expect(panel.locator('ins').first()).toBeVisible();

      const body = await card.locator('.transcript-body').boundingBox();
      const box = await panel.boundingBox();
      expect(body && box).toBeTruthy();
      expect(box!.x).toBeGreaterThanOrEqual(body!.x - 0.5);
      expect(box!.x + box!.width).toBeLessThanOrEqual(body!.x + body!.width + 0.5);
      // The row's actions column keeps its place to the right of the text.
      const actions = await card.locator('.transcript-actions').boundingBox();
      expect(actions!.x).toBeGreaterThanOrEqual(box!.x + box!.width - 0.5);

      // Removed and added words are distinguishable without colour.
      await expect(panel.locator('del').first()).toHaveCSS('text-decoration-line', 'line-through');
      await expect(panel.locator('ins').first()).toHaveCSS('text-decoration-line', 'underline');

      expect(await page.locator('.home-dashboard').evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
    });
  }
}

test('the comparison action is absent when the setting is off', async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 780 });
  await page.goto('/visual-fixtures.html?state=idle&appearance=light');
  const card = page.locator('[data-testid="transcript-card"]').first();
  await card.hover();
  await card.getByRole('button', { name: 'More transcript actions' }).click();
  await expect(page.getByRole('menuitem', { name: 'Pin transcript' })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: 'Show what changed' })).toHaveCount(0);
});
