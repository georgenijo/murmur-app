import { expect, test } from '@playwright/test';

for (const appearance of ['light', 'dark']) {
  test(`update controls download, defer, and restart in ${appearance} appearance`, async ({ page }) => {
    await page.goto(`/visual-fixtures.html?state=update-flow&appearance=${appearance}`);
    await page.locator('header').getByRole('button', { name: 'Check for Updates', exact: true }).click();
    const available = page.getByRole('dialog', { name: 'Update Available', exact: true });
    await expect(available.getByRole('button', { name: 'Skip This Version' })).toHaveCount(0);
    await available.getByRole('button', { name: 'Later', exact: true }).click();
    await expect(available).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Check for Updates', exact: true })).toHaveCount(0);
    await expect(page.locator('[data-visual-ready="true"]')).toHaveScreenshot(`${appearance}-update-settings-available.png`);

    await page.getByRole('button', { name: 'Download Update', exact: true }).click();
    await expect(page.getByRole('progressbar', { name: 'Download progress' })).toHaveAttribute('aria-valuenow', '65');
    await expect(page.getByRole('dialog', { name: 'Ready to Restart' })).toBeVisible();
    await expect(page.locator('html')).not.toHaveAttribute('data-update-installs');
    await expect(page.locator('html')).not.toHaveAttribute('data-update-restarts');
    await page.getByRole('dialog').getByRole('button', { name: 'Later', exact: true }).click();
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Restart Now', exact: true })).toBeEnabled();
    await expect(page.locator('[data-visual-ready="true"]')).toHaveScreenshot(`${appearance}-update-settings-ready.png`);
    await page.getByRole('button', { name: 'Restart Now. Murmur 0.44.2 is ready to install', exact: true }).click();
    await expect(page.locator('html')).toHaveAttribute('data-update-installs', '1');
    await expect(page.locator('html')).toHaveAttribute('data-update-restarts', '1');
  });

  for (const state of ['downloading', 'ready']) {
    test(`${appearance} update ${state} screen stays compact`, async ({ page }) => {
      await page.setViewportSize({ width: 720, height: 560 });
      await page.goto(`/visual-fixtures.html?state=update-${state}&appearance=${appearance}`);
      await expect(page.getByRole('dialog')).toBeVisible();
      await expect(page.locator('[data-visual-ready="true"]')).toHaveScreenshot(`${appearance}-update-${state}.png`);
    });
  }
}

for (const width of [640, 880]) {
  test(`Home update check sits immediately left of Settings at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 720 });
    await page.goto('/visual-fixtures.html?state=home-update-check&appearance=dark');
    const check = page.getByRole('button', { name: 'Check for Updates', exact: true });
    const settings = page.getByRole('button', { name: 'Open customization and settings', exact: true });
    await expect(check).toBeVisible();
    await expect(check).toBeEnabled();
    await expect(check).toHaveAttribute('title', 'Check for Updates');
    const checkBounds = await check.boundingBox();
    const settingsBounds = await settings.boundingBox();
    expect(checkBounds).not.toBeNull();
    expect(settingsBounds).not.toBeNull();
    expect(settingsBounds!.x - (checkBounds!.x + checkBounds!.width)).toBeGreaterThanOrEqual(0);
    expect(settingsBounds!.x - (checkBounds!.x + checkBounds!.width)).toBeLessThanOrEqual(16);
    expect(checkBounds!.y).toBe(settingsBounds!.y);
  });
}
