import { expect, test } from '@playwright/test';

for (const appearance of ['light', 'dark']) {
  test(`${appearance} changed custom toggles expose their real reset handlers`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: appearance as 'light' | 'dark', reducedMotion: 'reduce' });
    await page.goto(`/visual-fixtures.html?state=settings-general&appearance=${appearance}`);
    const nav = page.getByRole('navigation', { name: 'Settings pages' });
    await nav.getByRole('button', { name: 'Delivery', exact: true }).click();
    const auto = page.locator('[data-setting-target="auto-paste"]');
    await expect(auto.getByRole('button', { name: 'Reset to default' })).toHaveCount(0);
    await auto.getByRole('switch').check();
    await expect(auto.getByRole('button', { name: 'Reset to default' })).toBeVisible();
    await auto.getByRole('button', { name: 'Reset to default' }).click();
    await expect(auto.getByRole('switch')).not.toBeChecked();
    await expect(auto.getByRole('button', { name: 'Reset to default' })).toHaveCount(0);

    await nav.getByRole('button', { name: 'Recording', exact: true }).click();
    const smart = page.locator('[data-setting-target="smart-auto-stop"]');
    await expect(smart.getByRole('button', { name: 'Reset to default' })).toHaveCount(0);
    await expect(smart.getByRole('switch')).toBeDisabled();
    await page.locator('[data-setting-target="stop-on-silence"]').getByRole('button', { name: '1.5s', exact: true }).click();
    await smart.getByRole('switch').check();
    await smart.getByRole('button', { name: 'Reset to default' }).click();
    await expect(smart.getByRole('switch')).not.toBeChecked();
    await expect(smart.getByRole('button', { name: 'Reset to default' })).toHaveCount(0);

    await nav.getByRole('button', { name: 'Meetings', exact: true }).click();
    const speakers = page.locator('[data-setting-target="meeting-speakers"]');
    await expect(speakers.getByRole('button', { name: 'Reset to default' })).toHaveCount(0);
    await speakers.getByRole('switch').check();
    await speakers.getByRole('button', { name: 'Reset to default' }).click();
    await expect(speakers.getByRole('switch')).not.toBeChecked();
    await expect(speakers.getByRole('button', { name: 'Reset to default' })).toHaveCount(0);
  });
}
