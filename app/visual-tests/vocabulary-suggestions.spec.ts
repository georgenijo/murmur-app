import { expect, test } from '@playwright/test';
import { surfaceCases } from './surface-matrix';

for (const { appearance, width, height, label } of surfaceCases()) {
  test.describe(`Vocabulary suggestions ${label}`, () => {
    test.use({ viewport: { width, height }, colorScheme: appearance });

    test('lists recurring terms inside the card and adds one as a saved spelling', async ({ page }) => {
      await page.goto(`/visual-fixtures.html?state=settings-copy&page=text&editor=aliases&suggestions=1&appearance=${appearance}`);
      const card = page.locator('[data-setting-target="vocabulary-suggestions"]');
      await expect(card).toBeVisible();
      await expect(card.getByRole('switch', { name: 'Suggest spellings from history' })).toHaveAttribute('aria-checked', 'true');
      const list = card.getByRole('list', { name: 'Suggested spellings' });
      await expect(list.locator('li')).toHaveCount(5);
      await expect(list.locator('li').first()).toContainText('GitHub');

      // Every row's actions stay inside the card at this viewport.
      const cardBox = (await card.boundingBox())!;
      for (const row of await list.locator('li').all()) {
        const add = (await row.getByRole('button', { name: /^Add / }).boundingBox())!;
        expect(add.x + add.width).toBeLessThanOrEqual(cardBox.x + cardBox.width + 0.5);
        expect(add.x).toBeGreaterThanOrEqual(cardBox.x);
      }
      expect(await card.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);

      await card.getByRole('button', { name: 'Add Priya to saved spellings' }).click();
      await expect(list.locator('li')).toHaveCount(4);
      await expect(page.getByRole('textbox', { name: 'Written form 2' })).toHaveValue('Priya');
    });

    test('is off by default with no suggestion list', async ({ page }) => {
      await page.goto(`/visual-fixtures.html?state=settings-copy&page=text&editor=aliases&appearance=${appearance}`);
      const card = page.locator('[data-setting-target="vocabulary-suggestions"]');
      await expect(card.getByRole('switch', { name: 'Suggest spellings from history' })).toHaveAttribute('aria-checked', 'false');
      await expect(card.getByRole('list')).toHaveCount(0);
    });
  });
}
