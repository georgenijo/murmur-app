import { expect, test } from '@playwright/test';

for (const appearance of ['light', 'dark'] as const) {
  for (const size of [{ width: 1120, height: 820 }, { width: 880, height: 720 }, { width: 720, height: 560 }]) {
    test(`Assistant composer and controls fit ${appearance} ${size.width}x${size.height}`, async ({ page }) => {
      await page.setViewportSize(size);
      await page.emulateMedia({ colorScheme: appearance });
      await page.goto(`/visual-fixtures.html?state=assistant-listening&appearance=${appearance}`);
      await expect(page.getByRole('heading', { name: 'Assistant', exact: true })).toBeVisible();
      await expect(page.getByRole('button', { name: 'Finish speaking', exact: true })).toBeVisible();
      await expect(page.getByRole('button', { name: 'Stop', exact: true })).toBeVisible();
      await expect(page.getByRole('textbox', { name: 'Message Pi Assistant' })).toHaveValue('What needs my attention tomorrow?');
      await expect(page.getByRole('textbox', { name: 'Message Pi Assistant' })).toHaveAttribute('readonly');
      for (const locator of [page.locator('.assistant-workspace'), page.locator('.assistant-composer'), page.getByRole('button', { name: 'Stop', exact: true })]) {
        const bounds = await locator.boundingBox();
        expect(bounds).not.toBeNull();
        expect(bounds!.x).toBeGreaterThanOrEqual(0);
        expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(size.height + 1);
        expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(size.width + 1);
      }
      expect(await page.locator('.assistant-conversation').evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
      const columns = await page.locator('.assistant-conversation-header h2, .assistant-message-body, .assistant-composer textarea').evaluateAll((els) => els.map((el) => el.getBoundingClientRect().x));
      expect(Math.max(...columns) - Math.min(...columns)).toBeLessThanOrEqual(1);
      for (const surface of ['.assistant-workspace', '.assistant-message', '.assistant-composer']) {
        expect(await page.locator(surface).first().evaluate((el) => getComputedStyle(el).backgroundColor)).toBe('rgba(0, 0, 0, 0)');
      }
      await page.getByText('Connection', { exact: true }).click();
      await expect(page.getByRole('button', { name: 'Disconnect assistant' })).toBeVisible();
      const panel = await page.locator('.assistant-connection-panel').boundingBox();
      expect(panel!.x).toBeGreaterThanOrEqual(0);
      expect(panel!.y).toBeGreaterThanOrEqual(0);
      expect(panel!.x + panel!.width).toBeLessThanOrEqual(size.width);
      await page.getByText('Connection', { exact: true }).press('Escape');
      await expect(page.getByRole('button', { name: 'Disconnect assistant' })).not.toBeVisible();
    });
  }
}
