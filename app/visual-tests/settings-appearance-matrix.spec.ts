import { expect, test } from '@playwright/test';
import { surfaceCases } from './surface-matrix';

for (const { appearance, width, height, label } of surfaceCases()) {
  test.describe(`Settings appearance ${label}`, () => {
    test.use({ viewport: { width, height }, colorScheme: appearance });
    test('Invalid theme colors block Return and Done; valid Return commits', async ({ page }) => {
      await page.emulateMedia({ colorScheme: appearance as 'light' | 'dark' });
      await page.goto(`/visual-fixtures.html?state=settings-copy&page=appearance&appearance=${appearance}`);
      await page.getByRole('button', { name: 'Create theme', exact: true }).click();
      const dialog = page.locator('[role="dialog"]:not(.settings-editor-sheet)');
      const accent = dialog.getByLabel('Accent', { exact: true });
      await accent.fill('not-a-color');
      await page.keyboard.press('Enter');
      await expect(dialog).toBeVisible();
      await expect(accent).toHaveAttribute('aria-invalid', 'true');
      await dialog.getByRole('button', { name: 'Done', exact: true }).click();
      await expect(dialog).toBeVisible();
      await expect(accent).toBeFocused();
      await expect(dialog).toHaveScreenshot(`${appearance}-${width}x${height}-invalid-accent.png`);
      await accent.fill('#123abc');
      await page.keyboard.press('Enter');
      await expect(dialog).toHaveCount(0);
      await page.getByRole('button', { name: 'Create theme', exact: true }).click();
      await expect(dialog.getByLabel('Accent', { exact: true })).toHaveValue('#123abc');
      // Pending invalid fields also block Done when Return starts elsewhere.
      await dialog.getByLabel('Background', { exact: true }).fill('invalid');
      await dialog.getByLabel('Foreground', { exact: true }).focus();
      await page.keyboard.press('Enter');
      await expect(dialog).toBeVisible();
      await expect(dialog.getByLabel('Background', { exact: true })).toBeFocused();
    });
    test('Live theme editor pins real actions and restores Create theme focus', async ({ page }) => {
      await page.goto(`/visual-fixtures.html?state=settings-appearance&appearance=${appearance}`);
      await page.getByRole('button', { name: 'Create theme', exact: true }).click();
      const dialog = page.getByRole('dialog', { name: 'Create theme', exact: true });
      for (const name of ['Reset to Sonic', 'Export', 'Done']) {
        const action = dialog.getByRole('button', { name, exact: true });
        await expect(action).toBeInViewport({ ratio: 1 });
        const box = (await action.boundingBox())!;
        expect(box.y + box.height).toBeLessThanOrEqual(height);
      }
      await expect(dialog.getByRole('button', { name: 'Cancel', exact: true })).toHaveCount(0);
      const contrast = dialog.getByRole('slider', { name: 'Contrast', exact: true });
      await contrast.focus();
      await expect(contrast).toBeInViewport({ ratio: 1 });
      await expect(dialog.getByRole('button', { name: 'Done', exact: true })).toBeInViewport({ ratio: 1 });
      await dialog.getByRole('button', { name: 'Done', exact: true }).click();
      await expect(page.getByRole('button', { name: 'Create theme', exact: true })).toBeFocused();
    });
  });
}
