import { expect, test } from '@playwright/test';
import { surfaceCases } from './surface-matrix';

const titles = ['Microphone access', 'Accessibility access', 'System audio access', 'Calendar access'] as const;
const permissions = ['unknown', 'granted', 'denied', 'loading', 'restricted', 'unsupported'] as const;

for (const { appearance, width, height, label } of surfaceCases()) {
  test.describe(`Onboarding ${label}`, () => {
    test.use({ viewport: { width, height }, colorScheme: appearance });
    for (const permission of permissions) {
      test(`${permission} permission steps have one primary and safe Return behavior`, async ({ page }) => {
        await page.emulateMedia({ colorScheme: appearance, reducedMotion: 'reduce' });
        await page.goto(`/visual-fixtures.html?state=copy-onboarding&permission=${permission}&appearance=${appearance}`);
        await page.getByRole('button', { name: 'Get started', exact: true }).click();
        let progressBounds: { x: number; y: number; width: number; height: number } | null = null;
        let navBounds: typeof progressBounds = null;
        for (const [index, title] of titles.entries()) {
          await expect(page.getByRole('heading', { name: title, exact: true })).toBeVisible();
          await expect(page.locator('.onboarding-stages [aria-current=step]')).toHaveText('Permissions');
          // Wait for the fixture's asynchronous probes before checking the actions.
          const status = page.locator('.onboarding-permission-state');
          const expectedStatus = permission === 'granted' ? 'Allowed'
            : index === 1 ? permission === 'loading' ? 'Waiting for macOS…' : 'Not set'
            : permission === 'denied' ? 'Denied'
            : index === 3 && permission === 'restricted' ? 'Restricted'
            : index === 3 && permission === 'unsupported' ? 'Unavailable' : 'Not set';
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
          const inlineAllow = page.locator('.onboarding-permission-allow');
          const unavailable = index === 3 && ['restricted', 'unsupported'].includes(permission);
          await expect(inlineAllow).toHaveCount(permission === 'granted' || unavailable ? 0 : 1);
          if (permission !== 'granted' && !unavailable) await expect(inlineAllow).toHaveAttribute('aria-label', `Allow ${title.replace(' access', '').toLowerCase()} access`);

          // Wait for Welcome's padding to yield to the pinned-footer layout,
          // then sample the frame and portal footer in one layout pass.
          let geometry: { progress: { x: number; y: number; width: number; height: number }; nav: { x: number; y: number; width: number; height: number }; settled: boolean } | undefined;
          await expect.poll(async () => {
            geometry = await page.locator('.onboarding-frame').evaluate((frame) => {
              const rect = (node: Element) => {
                const { x, y, width, height } = node.getBoundingClientRect();
                return { x, y, width, height };
              };
              const outer = frame.parentElement!;
              const style = getComputedStyle(outer);
              const headerBottom = document.querySelector('.ui-window-header')!.getBoundingClientRect().bottom;
              const available = innerHeight - headerBottom - parseFloat(style.paddingTop) - parseFloat(style.paddingBottom);
              const expectedHeight = Math.min(parseFloat(getComputedStyle(frame).getPropertyValue('--ui-onboarding-frame-height')), available);
              const bounds = frame.getBoundingClientRect();
              const nav = frame.querySelector('.onboarding-nav')!.getBoundingClientRect();
              const expectedTop = headerBottom + parseFloat(style.paddingTop) + (available - expectedHeight) / 2;
              return {
                progress: rect(document.querySelector('.onboarding-stages')!), nav: rect(frame.querySelector('.onboarding-nav')!),
                settled: style.paddingTop === '20px' && style.paddingBottom === '88px' && Math.abs(bounds.top - expectedTop) < 1 && Math.abs(bounds.height - expectedHeight) < 1 && Math.abs(nav.bottom - bounds.bottom) < 1,
              };
            });
            return geometry.settled;
          }).toBe(true);
          const { progress, nav } = geometry!;
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
              // An unknown status and resolved fire-and-forget request keep the manual fallback available.
              await expect(status).toHaveText('Not set');
              await expect(primary).toBeEnabled();
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
          await expect(page.getByRole('heading', { name: titles[index + 1] ?? 'Speech model', exact: true })).toBeVisible();
        }
      });
    }
  });
}
