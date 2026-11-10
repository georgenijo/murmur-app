import { expect, test, type Locator, type Page } from '@playwright/test';

type Box = { x: number; y: number; width: number; height: number };

async function box(control: Locator): Promise<Box> {
  const value = await control.boundingBox();
  expect(value).not.toBeNull();
  return value!;
}

async function openWizardModel(page: Page, appearance: 'light' | 'dark', download?: 'indeterminate' | 'failed') {
  const query = new URLSearchParams({ state: 'first-run-onboarding', permission: 'granted', appearance });
  if (download) query.set('download', download);
  await page.goto(`/visual-fixtures.html?${query}`);
  await page.getByRole('button', { name: 'Get started', exact: true }).click();
  for (const heading of ['Microphone access', 'Accessibility access', 'System audio access', 'Calendar access']) {
    await expect(page.getByRole('heading', { name: heading, exact: true })).toBeVisible();
    await page.getByRole('navigation', { name: 'Setup step actions' }).getByRole('button', { name: 'Continue', exact: true }).click();
  }
  await expect(page.getByRole('heading', { name: 'Speech model', exact: true })).toBeVisible();
  return page.getByRole('navigation', { name: 'Setup step actions' });
}

async function openStandaloneModel(page: Page, appearance: 'light' | 'dark', download?: 'indeterminate' | 'failed') {
  const query = new URLSearchParams({ state: 'first-run-model', appearance });
  if (download) query.set('download', download);
  await page.goto(`/visual-fixtures.html?${query}`);
  await expect(page.getByRole('heading', { name: 'Speech model', exact: true })).toBeVisible();
}

for (const { width, height } of [{ width: 1100, height: 780 }, { width: 840, height: 620 }]) {
  for (const appearance of ['light', 'dark'] as const) {
    test(`model primary actions keep their boxes at ${width}x${height} (${appearance})`, async ({ page }) => {
      await page.setViewportSize({ width, height });
      await page.emulateMedia({ colorScheme: appearance, reducedMotion: 'reduce' });

      const wizardIdle = await openWizardModel(page, appearance);
      const wizardIdleBox = await box(wizardIdle.getByRole('button', { name: /^Download ·/, exact: false }));

      const wizardDownloading = await openWizardModel(page, appearance, 'indeterminate');
      await wizardDownloading.getByRole('button', { name: /^Download ·/, exact: false }).click();
      const wizardDownloadingBox = await box(wizardDownloading.getByRole('button', { name: 'Cancel', exact: true }));

      const wizardFailed = await openWizardModel(page, appearance, 'failed');
      await wizardFailed.getByRole('button', { name: /^Download ·/, exact: false }).click();
      const wizardFailedBox = await box(wizardFailed.getByRole('button', { name: 'Try again', exact: true }));

      expect(wizardDownloadingBox).toEqual(wizardIdleBox);
      expect(wizardFailedBox).toEqual(wizardIdleBox);

      await openStandaloneModel(page, appearance);
      const standaloneIdleBox = await box(page.getByRole('button', { name: /^Download ·/, exact: false }));

      await openStandaloneModel(page, appearance, 'indeterminate');
      await page.getByRole('button', { name: /^Download ·/, exact: false }).click();
      const standaloneDownloadingBox = await box(page.getByRole('button', { name: 'Cancel', exact: true }));

      await openStandaloneModel(page, appearance, 'failed');
      await page.getByRole('button', { name: /^Download ·/, exact: false }).click();
      const standaloneFailedBox = await box(page.getByRole('button', { name: 'Try again', exact: true }));

      expect(standaloneDownloadingBox).toEqual(standaloneIdleBox);
      expect(standaloneFailedBox).toEqual(standaloneIdleBox);
    });
  }
}
