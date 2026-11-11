import { expect, test, type Page } from '@playwright/test';
import { surfaceCases } from './surface-matrix';

async function settings(page: Page, editor: string, appearance: 'light' | 'dark') {
  await page.goto(`/visual-fixtures.html?state=settings-copy&page=text&editor=${editor}&appearance=${appearance}`);
  await expect(page.locator('.settings-editor-sheet > div:first-child h2')).toBeVisible();
}

for (const { appearance, width, height, label } of surfaceCases()) {
  test.describe(`Settings editors ${label}`, () => {
    test.use({ viewport: { width, height }, colorScheme: appearance });
    test('Armed alias deletion blocks every modified Return activation', async ({ page }) => {
      await settings(page, 'aliases', appearance);
      await page.getByRole('button', { name: 'Delete Murmur…', exact: true }).click();
      const confirm = page.getByRole('button', { name: 'Confirm delete Murmur…', exact: true });
      await expect(confirm).toBeFocused();
      for (const key of ['Enter', 'Shift+Enter', 'Meta+Enter', 'Control+Enter', 'Alt+Enter']) {
        await page.keyboard.press(key);
        await expect(confirm).toBeVisible();
        await expect(confirm).toBeFocused();
      }
      await page.keyboard.press('Space');
      await expect(confirm).toHaveCount(0);
    });
  });
}
