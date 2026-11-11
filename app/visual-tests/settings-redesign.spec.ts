import { expect, test } from '@playwright/test';

for (const appearance of ['light', 'dark'] as const) {
  for (const width of [1100, 840]) {
    test(`${appearance} approved Settings groups and binding axis at ${width}`, async ({ page }) => {
      await page.setViewportSize({ width, height: width === 1100 ? 780 : 620 });
      await page.emulateMedia({ colorScheme: appearance, reducedMotion: 'reduce' });
      await page.goto(`/visual-fixtures.html?state=settings-general&appearance=${appearance}`);
      await expect(page.locator('.settings-group-heading')).toHaveText(['Startup', 'Dictation', 'Maintenance']);
      await expect(page.locator('.settings-sidebar')).toHaveCSS('width', '208px');
      const title = await page.locator('.settings-content-topbar h1').boundingBox();
      const card = await page.locator('.settings-card').first().boundingBox();
      expect(card!.x).toBe(title!.x);
      expect(card!.width).toBe(width === 1100 ? 680 : 584);
      const windowTitle = await page.locator('.settings-window-title').boundingBox();
      expect(windowTitle!.x + windowTitle!.width / 2).toBeCloseTo(width / 2);
      await expect(page.locator('[data-setting-target="discovery-checklist"]')).toHaveCount(0);
      await page.getByRole('button', { name: /Keyboard shortcuts/ }).click();
      const rightEdges = await page.locator('.settings-shortcut-key').evaluateAll(keys => keys.map(key => key.getBoundingClientRect().right));
      expect(rightEdges).toHaveLength(5);
      await expect(page.locator('[data-shortcut-id="paste-last"] .settings-shortcut-state')).toHaveText('Off');
      expect(Math.max(...rightEdges) - Math.min(...rightEdges)).toBeLessThan(1);
      await expect(page.locator('.settings-shortcut-hold')).toHaveText('Hold');
      await page.getByRole('button', { name: 'Configure in Recording' }).click();
      await expect(page.getByRole('heading', { level: 1 })).toHaveText('Recording');
      await expect(page.locator('[data-setting-target="trigger-key"]')).toBeVisible();
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    });

    test(`${appearance} approved Appearance groups use real per-mode selection at ${width}`, async ({ page }) => {
      await page.setViewportSize({ width, height: width === 1100 ? 780 : 620 });
      await page.emulateMedia({ colorScheme: appearance, reducedMotion: 'reduce' });
      await page.goto(`/visual-fixtures.html?state=settings-appearance&appearance=${appearance}`);
      await expect(page.locator('.settings-appearance-mode')).toHaveCSS('border-radius', '12px');
      await expect(page.locator('.settings-segmented')).toHaveCSS('height', '34px');
      await expect(page.locator('.settings-segmented-btn').first()).toHaveCSS('border-radius', '8px');
      const initialShowing = page.locator('.settings-theme-variant[data-showing-now]');
      await expect(initialShowing).toHaveCount(1);
      const currentMode = /for (light|dark) mode/.exec(await initialShowing.getAttribute('aria-label') ?? '')?.[1] as 'light' | 'dark';
      const otherMode = currentMode === 'light' ? 'dark' : 'light';
      const sonic = page.locator('[data-theme-collection="Sonic"]');
      const opencode = page.locator('[data-theme-collection="OpenCode"]');
      const sonicCurrent = sonic.getByRole('button', { name: new RegExp(`Use Sonic for ${currentMode} mode`) });
      const opencodeCurrent = opencode.getByRole('button', { name: new RegExp(`Use OpenCode for ${currentMode} mode`) });
      const opencodeOther = opencode.getByRole('button', { name: new RegExp(`Use OpenCode for ${otherMode} mode`) });
      await sonicCurrent.click();
      await expect(sonicCurrent).toHaveAttribute('data-showing-now', 'true');
      await opencodeOther.click();
      await expect(opencodeOther).toHaveAttribute('data-showing-now', 'true');
      await expect(opencodeOther).toHaveAttribute('data-assigned', 'true');
      await expect(sonicCurrent).toHaveAttribute('data-assigned', 'true');
      await expect(sonic.locator('.settings-theme-variant-check, .settings-theme-variant-assigned')).toHaveCount(0);
      await page.getByRole('radio', { name: `Use ${currentMode} mode`, exact: true }).click();
      await expect(sonicCurrent).toHaveAttribute('data-showing-now', 'true');
      await expect(opencode.locator('.settings-theme-variant-check, .settings-theme-variant-assigned')).toHaveCount(0);
      await page.getByRole('radio', { name: 'Follow the system appearance', exact: true }).click();
      await expect(sonicCurrent).toHaveAttribute('data-showing-now', 'true');
      await page.getByRole('button', { name: `Use OpenCode for ${currentMode} mode`, exact: false }).click();
      await expect(opencodeCurrent).toHaveAttribute('data-showing-now', 'true');
      await expect(page.locator('.settings-theme-variant-check, .settings-theme-variant-assigned')).toHaveCount(0);
      await expect(page.locator('.settings-theme-caption, .settings-theme-mode-status')).toHaveCount(0);
      expect(await sonic.evaluate((card) => card.getBoundingClientRect().height))
        .toBe(await opencode.evaluate((card) => card.getBoundingClientRect().height));
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    });
  }
}
