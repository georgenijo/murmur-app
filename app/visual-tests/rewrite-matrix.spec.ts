import { expect, test } from '@playwright/test';
import { surfaceCases } from './surface-matrix';
import { readableText } from './surface-contrast';

for (const { appearance, width, height, label } of surfaceCases([{ width: 420, height: 360 }])) {
  test.describe(`Rewrite ${label}`, () => {
    test.use({ viewport: { width, height }, colorScheme: appearance });
    for (const backdrop of ['black', 'white']) {
      test(`${backdrop} backdrop: opaque labels, shortcuts, diff and hovered shortcuts retain contrast`, async ({ page }, testInfo) => {
        await page.goto('/transform-review.html?mock=ready');
        await page.evaluate(color => { document.documentElement.style.background = color; document.body.style.background = color; }, backdrop);
        const surface = page.locator('.transform-review-surface');
        await expect(surface).toHaveCSS('opacity', '1');
        await expect(surface).toHaveCSS('background-color', 'rgb(20, 20, 20)');
        expect(await surface.evaluate(element => {
          const style = getComputedStyle(element);
          return style.backdropFilter === 'none' && !/blur/.test(style.getPropertyValue('-webkit-backdrop-filter'));
        })).toBe(true);
        await testInfo.attach(`opaque-rewrite-${backdrop}.png`, { body: await page.screenshot(), contentType: 'image/png' });
        const targets = [
          surface.getByText('On-device', { exact: true }),
          surface.getByText('⌘R', { exact: true }),
          surface.getByText('⏎', { exact: true }),
          surface.getByText('esc', { exact: true }),
          surface.getByRole('button', { name: /Retry/ }),
          surface.getByRole('button', { name: /Approve/ }),
          surface.getByRole('button', { name: /Cancel/ }),
          surface.locator('.line-through').first(),
          surface.locator('[class*="text-emerald-400"]').first(),
        ];
        const evidence = [];
        for (const target of targets) {
          const measured = await readableText(target);
          evidence.push(measured);
          expect(measured.contrast, measured.text ?? '').toBeGreaterThanOrEqual(4.5);
        }
        // Shortcut backgrounds change on hover; the same contract still applies.
        for (const [button, shortcut] of [['Retry', '⌘R'], ['Approve', '⏎'], ['Cancel', 'esc']]) {
          await surface.getByRole('button', { name: new RegExp(button) }).hover();
          // Wait for the actual hover transition, not a pre-transition sample.
          await page.waitForTimeout(200);
          const hovered = await readableText(surface.getByText(shortcut, { exact: true }));
          evidence.push(hovered);
          expect(hovered.contrast).toBeGreaterThanOrEqual(4.5);
        }
        await testInfo.attach('computed-rewrite-text.json', { body: JSON.stringify(evidence, null, 2), contentType: 'application/json' });
      });
    }
  });
}
