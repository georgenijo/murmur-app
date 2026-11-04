import { expect, test } from '@playwright/test';
import { surfaceCases } from './surface-matrix';
import { STREAMS } from '../src/lib/events';

for (const { appearance, width, height, label } of surfaceCases()) {
  test.describe(`Settings diagnostics ${label}`, () => {
    test.use({ viewport: { width, height }, colorScheme: appearance });
    for (const panel of ['transform', 'dictation', 'runs']) {
      test(`${panel} destructive dialog cancels, rejects modified Return, and deletes the captured identity`, async ({ page }) => {
        await page.emulateMedia({ colorScheme: appearance as 'light' | 'dark' });
        await page.clock.setFixedTime(new Date('2026-09-04T20:00:00Z'));
        await page.goto(`/visual-fixtures.html?state=copy-diagnostics&panel=${panel}&appearance=${appearance}`);
        if (panel === 'dictation') await page.getByRole('button', { name: 'Review', exact: true }).click();
        const opener = panel === 'dictation' ? page.getByRole('button', { name: 'Delete local copy…', exact: true }) : panel === 'runs' ? page.getByRole('button', { name: 'Clear performance data…', exact: true })
          : page.getByRole('button', { name: 'Delete…', exact: true });
        const attribute = panel === 'runs' ? 'data-copy-cleared-runs' : 'data-copy-deleted-capture';
        await opener.click();
        const dialog = page.locator('[role="dialog"]:not(.settings-editor-sheet)');
        await expect(dialog.getByRole('button', { name: 'Cancel', exact: true })).toBeFocused();
        await page.keyboard.press('Escape');
        await expect(dialog).toHaveCount(0);
        await expect(opener).toBeFocused();
        await expect(page.locator('html')).not.toHaveAttribute(attribute);
        await page.keyboard.press('Space');
        await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
        await expect(dialog).toHaveCount(0);
        await expect(page.locator('html')).not.toHaveAttribute(attribute);
        await page.keyboard.press('Space');
        await page.keyboard.press('Shift+Tab');
        const confirm = dialog.getByRole('button').last();
        await expect(confirm).toBeFocused();
        for (const key of ['Enter', 'Shift+Enter', 'Meta+Enter', 'Control+Enter', 'Alt+Enter']) {
          await page.keyboard.press(key);
          await expect(dialog).toBeVisible();
          await expect(page.locator('html')).not.toHaveAttribute(attribute);
        }
        await expect(dialog).toHaveScreenshot(`${appearance}-${width}x${height}-${panel}-confirm.png`);
        if (panel !== 'runs') {
          await page.evaluate(() => document.documentElement.setAttribute('data-copy-replace-captures', 'true'));
          if (panel === 'transform') await page.getByRole('button', { name: 'Refresh', exact: true }).evaluate((button: HTMLButtonElement) => button.click());
          await expect(page.locator('html')).toHaveAttribute('data-copy-listed-capture', 'capture-two');
          if (panel === 'transform') await expect(page.getByText('Pass 52', { exact: true })).toBeVisible();
          await expect(dialog).toContainText(panel === 'dictation' ? 'recording 41' : 'pass 51');
        }
        await page.keyboard.press('Space');
        await expect(dialog).toHaveCount(0);
        await expect(page.locator('html')).toHaveAttribute(attribute, panel === 'runs' ? 'true' : 'capture-one');
        if (panel !== 'runs') await expect(page.getByRole('heading', { name: panel === 'transform' ? 'Transform diagnostics' : 'Private dictation capture', exact: true })).toBeFocused();
      });
    }
    test('Delayed direct deletion refresh retains list focus', async ({ page }) => {
      await page.goto(`/visual-fixtures.html?state=copy-diagnostics&panel=transform&appearance=${appearance}`);
      await page.getByRole('button', { name: 'Delete…', exact: true }).click();
      await page.evaluate(() => document.documentElement.setAttribute('data-copy-delay-refresh', 'true'));
      await page.keyboard.press('Shift+Tab');
      await page.keyboard.press('Space');
      await expect(page.locator('html')).toHaveAttribute('data-copy-refresh-waiting', 'true');
      await expect(page.locator('[role="dialog"]:not(.settings-editor-sheet)')).toHaveCount(0);
      await expect(page.getByRole('heading', { name: 'Transform diagnostics', exact: true })).toBeFocused();
      await page.evaluate(() => window.dispatchEvent(new Event('copy-release-refresh')));
      await expect(page.locator('html')).not.toHaveAttribute('data-copy-refresh-waiting');
      await expect(page.getByRole('heading', { name: 'Transform diagnostics', exact: true })).toBeFocused();
      await expect(page.locator('html')).toHaveAttribute('data-copy-deleted-capture', 'capture-one');
    });
    test('Cancelling after capture replacement restores the list heading', async ({ page }) => {
      await page.goto(`/visual-fixtures.html?state=copy-diagnostics&panel=transform&appearance=${appearance}`);
      await page.getByRole('button', { name: 'Delete…', exact: true }).click();
      await page.evaluate(() => document.documentElement.setAttribute('data-copy-replace-captures', 'true'));
      await page.getByRole('button', { name: 'Refresh', exact: true }).evaluate((button: HTMLButtonElement) => button.click());
      await expect(page.getByText('Pass 52', { exact: true })).toBeVisible();
      await page.keyboard.press('Escape');
      await expect(page.locator('[role="dialog"]:not(.settings-editor-sheet)')).toHaveCount(0);
      await expect(page.getByRole('heading', { name: 'Transform diagnostics', exact: true })).toBeFocused();
      await expect(page.locator('html')).not.toHaveAttribute('data-copy-deleted-capture');
    });
    test('Deleting from review returns focus to the resulting list', async ({ page }) => {
      await page.goto(`/visual-fixtures.html?state=copy-diagnostics&panel=transform&appearance=${appearance}`);
      await page.getByRole('button', { name: 'Review', exact: true }).click();
      await page.getByRole('button', { name: 'Delete capture…', exact: true }).click();
      await page.keyboard.press('Shift+Tab');
      await page.keyboard.press('Space');
      await expect(page.locator('[role="dialog"]:not(.settings-editor-sheet)')).toHaveCount(0);
      await expect(page.getByRole('heading', { name: 'Transform diagnostics', exact: true })).toBeFocused();
      await expect(page.locator('html')).toHaveAttribute('data-copy-deleted-capture', 'capture-one');
    });
    test('Stream filters wrap and remain pointer and keyboard reachable', async ({ page }) => {
      await page.goto(`/visual-fixtures.html?state=settings-folding&page=diagnostics&appearance=${appearance}`);
      const tabs = page.getByRole('tablist', { name: 'Diagnostics views' });
      await expect(tabs).toBeVisible();
      const workspace = tabs.locator('..').locator('..').locator('..');
      const bounds = await workspace.boundingBox();
      for (const stream of STREAMS) {
        const chip = workspace.getByRole('button', { name: stream, exact: true });
        await expect(chip).toBeInViewport();
        const box = await chip.boundingBox();
        expect(box!.x).toBeGreaterThanOrEqual(bounds!.x);
        expect(box!.x + box!.width).toBeLessThanOrEqual(bounds!.x + bounds!.width + 1);
        const pressed = await chip.getAttribute('aria-pressed');
        await chip.click();
        await expect(chip).toHaveAttribute('aria-pressed', pressed === 'true' ? 'false' : 'true');
        await chip.focus();
        await page.keyboard.press('Space');
        await expect(chip).toHaveAttribute('aria-pressed', pressed!);
      }
      await tabs.getByRole('tab').last().click();
      await expect(tabs.getByRole('tab').last()).toHaveAttribute('aria-selected', 'true');
    });
  });
}
