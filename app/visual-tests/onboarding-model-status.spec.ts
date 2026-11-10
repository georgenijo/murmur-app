import { expect, test } from '@playwright/test';
import { surfaceCases } from './surface-matrix';

const permissionSteps = ['Microphone access', 'Accessibility access', 'System audio access', 'Calendar access'] as const;

for (const { appearance, width, height, label } of surfaceCases()) {
  test.describe(`Onboarding model download status ${label}`, () => {
    test.use({ viewport: { width, height }, colorScheme: appearance });

    test('keeps download status and Cancel visible while the model list scrolls', async ({ page }) => {
      await page.emulateMedia({ colorScheme: appearance, reducedMotion: 'reduce' });
      await page.goto('/visual-fixtures.html?state=copy-onboarding&permission=granted&downloading=true');
      await page.getByRole('button', { name: 'Get started', exact: true }).click();

      for (const title of permissionSteps) {
        await expect(page.getByRole('heading', { name: title, exact: true })).toBeVisible();
        await page.locator('button[data-ui-button][data-variant="primary"]').click();
      }

      await expect(page.getByRole('heading', { name: 'Speech model', exact: true })).toBeVisible();
      await page.getByRole('button', { name: /^Download ·/ }).click();

      const status = page.locator('.onboarding-model-status');
      await expect(page.getByRole('navigation', { name: 'Setup step actions' }).getByRole('button', { name: 'Cancel', exact: true })).toBeVisible();
      await expect(status.getByRole('progressbar')).toBeVisible();
      await expect(status).toContainText('Downloading');

      const statusBox = await status.boundingBox();
      const contentBox = await page.locator('.onboarding-content').boundingBox();
      expect(statusBox).not.toBeNull();
      expect(contentBox).not.toBeNull();
      expect(statusBox!.y).toBeGreaterThanOrEqual(contentBox!.y + contentBox!.height - 1);
      expect(statusBox!.y + statusBox!.height).toBeLessThanOrEqual(height);
      expect(statusBox!.y).toBeGreaterThanOrEqual(0);
    });
  });
}
