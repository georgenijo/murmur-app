import { expect, test } from '@playwright/test';

const states = ['idle', 'recording', 'processing', 'settings'] as const;
const appearances = ['light', 'dark'] as const;

for (const appearance of appearances) {
  for (const state of states) {
    test(`${appearance} ${state} matches the canonical 720x560 surface`, async ({ page }) => {
      const pageErrors: string[] = [];
      page.on('pageerror', (error) => pageErrors.push(error.message));
      await page.emulateMedia({ colorScheme: appearance });
      await page.goto(`/visual-fixtures.html?state=${state}&appearance=${appearance}`);
      expect(pageErrors).toEqual([]);
      const fixture = page.locator('[data-visual-ready="true"]');
      await expect(fixture).toBeVisible();
      await expect(fixture).toHaveScreenshot(`${appearance}-${state}.png`);
    });
  }
}
