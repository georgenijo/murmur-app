import { expect, test } from '@playwright/test';
import { surfaceCases } from './surface-matrix';

const titles = ['Microphone access', 'Accessibility access', 'System audio access', 'Calendar access'] as const;
const permissions = ['unknown', 'granted', 'denied', 'loading', 'restricted', 'unsupported'] as const;

for (const { appearance, width, height, label } of surfaceCases()) {
  test.describe(`Onboarding ${label}`, () => {
    test.use({ viewport: { width, height }, colorScheme: appearance });
    for (const permission of permissions) {
      test(`${permission} permission steps have one primary and safe Return behavior`, async ({ page }) => {
        await page.emulateMedia({ colorScheme: appearance });
        await page.goto(`/visual-fixtures.html?state=copy-onboarding&permission=${permission}&appearance=${appearance}`);
        await page.getByRole('button', { name: 'Get started', exact: true }).click();
        let progressBounds: { x: number; y: number; width: number; height: number } | null = null;
        let navBounds: typeof progressBounds = null;
        for (const [index, title] of titles.entries()) {
          await expect(page.getByRole('heading', { name: title, exact: true })).toBeVisible();
          await expect(page.getByRole('progressbar', { name: `Step ${index + 2} of 8` })).toHaveAttribute('aria-valuenow', String(index + 2));
          await expect(page.locator('.onboarding-progress-wrap')).toContainText(`Step ${index + 2} of 8`);
          // Wait for the fixture's asynchronous probes before checking the actions.
          const status = page.locator('.onboarding-status').first();
          const expectedStatus = index === 0
            ? permission === 'granted' ? 'Microphone access granted' : permission === 'denied' ? 'Microphone access denied' : 'Microphone access not yet granted'
            : index === 1
              ? permission === 'granted' ? 'Accessibility access granted' : 'Accessibility access not granted'
              : index === 2
                ? permission === 'granted' ? 'System audio access granted' : permission === 'denied' ? 'System audio access denied' : 'System audio access not yet granted'
                : permission === 'granted' ? 'Calendar access granted' : permission === 'denied' ? 'Calendar access denied' : permission === 'restricted' ? 'Calendar access restricted' : permission === 'unsupported' ? 'Calendar access unavailable' : 'Calendar access not yet requested';
          await expect(status).toHaveText(expectedStatus);

          const isNavigation = permission === 'granted' || (index === 3 && ['restricted', 'unsupported'].includes(permission));
          const label = (index === 1 && permission !== 'granted') || (permission === 'denied' && index !== 1) ? 'Open System Settings…' : 'Continue';
          const scope = page.locator('[data-default-scope]');
          const primary = scope.locator('button[data-ui-button][data-variant="primary"]');
          await expect(primary).toHaveCount(1);
          await expect(primary).toHaveText(label);
          await expect(primary).toBeEnabled();
          await expect(scope.locator('button[data-default-button]')).toHaveCount(isNavigation ? 1 : 0);
          if (isNavigation) await expect(primary).toHaveAttribute('data-default-button');
          else await expect(primary).not.toHaveAttribute('data-default-button');
          await expect(page.getByRole('button', { name: /^(Allow microphone access…|Grant accessibility access…|Allow Calendar access|Allow system audio access)$/ })).toHaveCount(0);

          const progress = await page.locator('.onboarding-progress-wrap').boundingBox();
          const nav = await page.locator('.onboarding-nav').boundingBox();
          expect(progress).not.toBeNull();
          expect(nav).not.toBeNull();
          if (index === 0) { progressBounds = progress; navBounds = nav; }
          else { expect(progress).toEqual(progressBounds); expect(nav).toEqual(navBounds); }
          if (permission === 'unknown') await expect(page.locator('.main-first-run-onboarding')).toHaveScreenshot(`${appearance}-${width}x${height}-${title.split(' ')[0].toLowerCase()}-permission.png`);

          if (permission === 'loading') {
            await primary.click();
            if (index >= 2) {
              await expect(status).toHaveText('Waiting for macOS…');
              await expect(primary).toBeDisabled();
              await expect(primary).not.toHaveAttribute('data-default-button');
            } else if (index === 0) {
              await expect(status).toHaveText('Waiting for your answer in the macOS dialog…');
            } else {
              await expect(page.getByText(/^In this development session, grant access to the terminal app/)).toBeVisible();
              await expect(page.getByRole('button', { name: /Reset the permission/ })).toHaveCount(0);
            }
          }
          const back = page.getByRole('button', { name: 'Go back to the previous setup step' });
          await back.focus();
          await page.keyboard.press('Enter');
          if (!isNavigation) {
            await expect(page.getByRole('heading', { name: title, exact: true })).toBeVisible();
            await page.getByRole('button', { name: /^Skip / }).click();
          }
          await expect(page.getByRole('heading', { name: titles[index + 1] ?? 'Transcription model', exact: true })).toBeVisible();
        }
      });
    }
  });
}
