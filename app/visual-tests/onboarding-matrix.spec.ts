import { expect, test } from '@playwright/test';
import { surfaceCases } from './surface-matrix';

for (const { appearance, width, height, label } of surfaceCases()) {
  test.describe(`Onboarding ${label}`, () => {
    test.use({ viewport: { width, height }, colorScheme: appearance });
    for (const permission of ['unknown', 'granted', 'denied', 'loading', 'restricted', 'unsupported']) {
      test(`${permission} permission steps bound primary actions and only enabled Continue is a default`, async ({ page }) => {
        await page.emulateMedia({ colorScheme: appearance as 'light' | 'dark' });
        await page.goto(`/visual-fixtures.html?state=copy-onboarding&permission=${permission}&appearance=${appearance}`);
        await page.getByRole('button', { name: 'Get started', exact: true }).click();
        for (const title of ['Microphone access', 'Accessibility access', 'System audio access', 'Calendar access']) {
          await expect(page.getByRole('heading', { name: title, exact: true })).toBeVisible();
          const next = page.getByRole('button', { name: 'Continue', exact: true });
          const primary = page.locator('button').filter({ hasNotText: /^$/ });
          const assertPrimary = async () => {
            const styles = await primary.evaluateAll(buttons => buttons.map(button => ({
              text: button.textContent?.trim(), primary: button.className.includes('text-on-primary'),
              background: getComputedStyle(button).backgroundImage,
            })));
            expect(styles.filter(button => button.primary).length).toBeLessThanOrEqual(1);
            if (permission === 'granted') expect(styles.filter(button => button.primary)).toHaveLength(1);
            const continueStyle = styles.find(button => button.text === 'Continue')!;
            expect(continueStyle.primary).toBe(permission === 'granted');
            expect(continueStyle.background === 'none').toBe(permission !== 'granted');
          };
          await assertPrimary();
          if (permission === 'unknown') await expect(page.locator('.main-first-run-onboarding')).toHaveScreenshot(`${appearance}-${width}x${height}-${title.split(' ')[0].toLowerCase()}-permission.png`);
          if (permission === 'loading' && ['System audio access', 'Calendar access'].includes(title)) {
            await page.getByRole('button', { name: title === 'Calendar access' ? 'Allow Calendar access' : 'Allow system audio access', exact: true }).click();
            await expect(page.getByRole('button', { name: 'Waiting for macOS…', exact: true })).toBeDisabled();
            await assertPrimary();
          }
          if (permission === 'loading' && ['Microphone access', 'Accessibility access'].includes(title)) {
            await page.getByRole('button', { name: title === 'Microphone access' ? 'Allow microphone access…' : 'Grant accessibility access…', exact: true }).click();
            if (title === 'Microphone access') {
              await expect(page.getByText('Waiting for your answer in the macOS dialog…', { exact: true })).toBeVisible();
            } else {
              // The visual fixture runs on Vite's dev server, so recovery must
              // guide the launching terminal rather than reset Murmur's bundle.
              await expect(page.getByText(/^In this development session, grant access to the terminal app/)).toBeVisible();
              await expect(page.getByRole('button', { name: /Reset the permission/ })).toHaveCount(0);
            }
            await assertPrimary();
          }
          const back = page.getByRole('button', { name: 'Go back to the previous setup step' });
          await back.focus();
          await page.keyboard.press('Enter');
          if (permission !== 'granted') {
            await expect(page.getByRole('heading', { name: title, exact: true })).toBeVisible();
            await expect(next).toBeDisabled();
            await page.getByRole('button', { name: /^Skip / }).click();
          }
        }
        await expect(page.getByRole('heading', { name: 'Transcription model', exact: true })).toBeVisible();
      });
    }
  });
}
