import { expect, test } from '@playwright/test';
import { surfaceCases } from './surface-matrix';

for (const { appearance, width, height, label } of surfaceCases([{ width: 840, height: 620 }, { width: 880, height: 720 }, { width: 720, height: 560 }])) {
  test.describe(`Settings inputs ${label}`, () => {
    test.use({ viewport: { width, height }, colorScheme: appearance });
    test('Voice commands search reveals without switch focus; editor handoff focuses once', async ({ page }) => {
      await page.goto(`/visual-fixtures.html?state=settings-copy&page=text&appearance=${appearance}`);
      const search = page.getByRole('searchbox', { name: 'Search all settings' });
      if (width < 768) await page.getByRole('button', { name: 'Search settings', exact: true }).click();
      await search.fill('Voice commands');
      await page.getByRole('button', { name: /Voice commands & spoken formatting.*Enable custom commands/ }).click();
      const toggle = page.getByRole('switch', { name: 'Voice commands & spoken formatting', exact: true });
      await expect(toggle).toBeVisible();
      await expect(toggle).not.toBeFocused();
      await expect(toggle).toHaveAttribute('aria-checked', 'false');
      await page.getByRole('button', { name: 'Voice commands Create exact spoken replacements and snippets', exact: true }).click();
      await page.getByRole('button', { name: 'Open Voice commands & spoken formatting', exact: true }).click();
      await expect(toggle).toBeVisible();
      await expect(toggle).toBeFocused();
      await expect(toggle).toHaveAttribute('aria-checked', 'false');
      if (width < 768) await page.getByRole('button', { name: 'Search settings', exact: true }).click();
      await search.click();
      // Outlast the bounded reveal retry, so a delayed refocus would be detected.
      await page.waitForTimeout(550);
      await expect(search).toBeFocused();
    });
    test('Sensitivity keyboard and pointer values survive navigation', async ({ page }) => {
      await page.goto(`/visual-fixtures.html?state=settings-copy&page=recording&appearance=${appearance}`);
      const slider = page.getByRole('slider', { name: 'Sensitivity', exact: true });
      await slider.focus();
      await page.keyboard.press('Home');
      await expect(slider).toHaveValue('0');
      await expect(slider).toHaveAttribute('aria-valuetext', 'Off');
      await page.keyboard.press('ArrowRight');
      await expect(slider).toHaveValue('5');
      await page.getByRole('button', { name: 'General', exact: true }).click();
      await page.getByRole('button', { name: 'Recording', exact: true }).click();
      await expect(slider).toHaveValue('5');
      const bounds = (await slider.boundingBox())!;
      await slider.click({ position: { x: bounds.width * 0.75, y: bounds.height / 2 } });
      const pointerValue = await slider.inputValue();
      expect(Number(pointerValue)).toBeGreaterThan(5);
      await page.getByRole('button', { name: 'General', exact: true }).click();
      await page.getByRole('button', { name: 'Recording', exact: true }).click();
      await expect(slider).toHaveValue(pointerValue);
    });
    if (width === 840) {
      test('Auto-paste stays a standard-height row in every file-saving state', async ({ page }) => {
        await page.goto(`/visual-fixtures.html?state=settings-copy&page=delivery&appearance=${appearance}`);
        const autoPaste = page.locator('[data-setting-target="auto-paste"]');
        const description = autoPaste.locator('.settings-row-description');
        const assertRow = async (copy: string) => {
          await expect(description).toHaveText(copy);
          await expect(description).toHaveCSS('height', '16px');
          await expect(autoPaste).toHaveCSS('min-height', '52px');
          expect((await autoPaste.boundingBox())!.height).toBe(52);
        };

        await assertRow('Always copied to clipboard. Also paste into active app.');
        await page.getByRole('switch', { name: 'Auto paste', exact: true }).check();
        await assertRow('Always copied to clipboard. Also paste into active app.');

        const transcriptFile = page.getByRole('switch', { name: 'Save transcript to file', exact: true });
        const audioFile = page.getByRole('switch', { name: 'Save audio to file', exact: true });
        await transcriptFile.check();
        await assertRow('Always copied to clipboard. Auto-paste is paused.');
        await transcriptFile.uncheck();

        await page.getByRole('switch', { name: 'Auto paste', exact: true }).uncheck();
        await audioFile.check();
        await assertRow('Always copied to clipboard. Auto-paste is off.');
        await transcriptFile.check();
        await assertRow('Always copied to clipboard. Auto-paste is off.');
      });
    }
  });
}
