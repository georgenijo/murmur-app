import { expect, test } from '@playwright/test';
import { surfaceCases } from './surface-matrix';
import { readableText } from './surface-contrast';

for (const { appearance, width, height, label } of surfaceCases([{ width: 440, height: 340 }])) {
  test.describe(`Voice Query ${label}`, () => {
    test.use({ viewport: { width, height }, colorScheme: appearance });
    test('Question, answer, details and pinned actions fit the native frame; long answer scrolls', async ({ page }, testInfo) => {
      await page.goto(`/visual-fixtures.html?state=query-review&appearance=${appearance}`);
      await expect(page.getByLabel('Your question')).toContainText('Is the living-room light on?');
      await expect(page.getByLabel('Query answer')).toContainText('Yes—the living-room light is on.');
      await expect(page.getByLabel('Query context')).not.toBeVisible();
      await expect(page.getByRole('button', { name: 'Open in Assistant' })).toBeVisible();
      await expect(page.getByRole('button', { name: 'Copy answer' })).toBeVisible();
      const metrics = await page.locator('.query-review-surface').evaluate(el => {
        const footer = el.querySelector('footer')!.getBoundingClientRect();
        const buttons = Array.from(el.querySelectorAll('button')).map(b => b.getBoundingClientRect());
        return { width: el.scrollWidth, height: el.getBoundingClientRect().height, footerBottom: footer.bottom,
          contained: buttons.every(b => b.left >= 0 && b.right <= 440 && b.top >= 0 && b.bottom <= 340) };
      });
      expect(metrics.width).toBeLessThanOrEqual(440);
      expect(metrics.height).toBe(340);
      expect(metrics.footerBottom).toBeLessThanOrEqual(340);
      expect(metrics.contained).toBe(true);
      await page.screenshot({ path: testInfo.outputPath('query.png') });
      await page.locator('.query-review-details summary').click();
      await expect(page.getByLabel('Query context')).toBeVisible();
      await expect(page.getByLabel('Query capabilities')).toBeVisible();
      await page.goto(`/visual-fixtures.html?state=query-review&appearance=${appearance}&long=1`);
      const overflow = await page.locator('.query-review-content').evaluate(el => el.scrollHeight > el.clientHeight);
      expect(overflow).toBe(true);
      await expect(page.getByRole('button', { name: 'Open in Assistant' })).toBeVisible();
    });
  });
}

for (const { appearance, width, height, label } of surfaceCases([{ width: 880, height: 720 }, { width: 440, height: 340 }])) {
  test.describe(`Voice Query contrast ${label}`, () => {
    test.use({ viewport: { width, height }, colorScheme: appearance });
    test('Secondary text stays readable on the opaque surface', async ({ page }, testInfo) => {
      await page.goto(`/visual-fixtures.html?state=query-review&appearance=${appearance}`);
      const surface = page.locator('.query-review-surface');
      await expect(surface).toBeVisible();
      const targets = [
        surface.locator('.query-review-status'),
        surface.locator('.query-review-details summary'),
        surface.getByRole('button', { name: 'Close', exact: true }),
        surface.getByRole('button', { name: 'Ask follow-up', exact: true }),
      ];
      const evidence = [];
      for (const target of targets) {
        const measured = await readableText(target);
        evidence.push(measured);
        expect(measured.contrast, measured.text ?? '').toBeGreaterThanOrEqual(4.5);
      }
      await testInfo.attach('computed-query-text.json', { body: JSON.stringify(evidence, null, 2), contentType: 'application/json' });
    });
  });
}
