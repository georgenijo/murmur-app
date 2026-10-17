import { expect, test } from '@playwright/test';

for (const size of [{ width: 1120, height: 820 }, { width: 880, height: 720 }, { width: 720, height: 560 }]) {
  test(`window chrome stays fixed and sidebars agree at ${size.width}x${size.height}`, async ({ page }) => {
    await page.setViewportSize(size);
    let sidebarWidth: number | undefined;
    for (const state of ['idle', 'settings', 'assistant-long']) {
      await page.goto(`/visual-fixtures.html?state=${state}&appearance=dark`);
      const sidebar = page.locator(state === 'settings' ? '.settings-sidebar' : '.home-sidebar');
      const before = await sidebar.boundingBox();
      expect(before).not.toBeNull();
      sidebarWidth ??= before!.width;
      expect(before!.width).toBe(sidebarWidth);
      expect(before!.y + before!.height).toBe(size.height);
      if (state !== 'settings') for (const label of ['Home', 'Notetaker', 'Assistant', 'Insights']) {
        const button = await sidebar.getByRole('button', { name: label, exact: true }).boundingBox();
        expect(button!.y).toBeGreaterThanOrEqual(before!.y);
        expect(button!.y + button!.height).toBeLessThanOrEqual(size.height);
      }
      const pane = page.locator(state === 'idle' ? '.home-dashboard' : state === 'settings' ? '[data-testid="settings-content"]' : '.assistant-message-list');
      await expect(pane).toBeVisible();
      await pane.hover();
      for (const delta of [100000, 100000, -100000, -100000]) {
        await page.mouse.wheel(0, delta);
        await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
        const scroll = await pane.evaluate((el) => ({ top: el.scrollTop, range: el.scrollHeight - el.clientHeight }));
        if (scroll.range > 1 && delta > 0) expect(scroll.top).toBeGreaterThan(0);
        if (delta < 0) expect(scroll.top).toBeLessThanOrEqual(1);
        expect(await sidebar.boundingBox()).toEqual(before);
        expect(await page.evaluate(() => ({ x: window.scrollX, y: window.scrollY, height: document.documentElement.scrollHeight }))).toEqual({ x: 0, y: 0, height: size.height });
      }
      if (state === 'idle') {
        expect(await page.locator('.history-list').evaluate((el) => getComputedStyle(el).overflowY)).toBe('visible');
        const search = await page.locator('.history-search').boundingBox();
        const filter = await page.getByRole('button', { name: 'Filter transcripts', exact: true }).boundingBox();
        expect(search!.height).toBe(filter!.height);
        expect(search!.y).toBe(filter!.y);
        await page.getByRole('searchbox', { name: 'Search transcripts' }).focus();
        expect(await page.locator('.history-search').boundingBox()).toEqual(search);
      }
    }
  });
}

test('resizing across the old compact breakpoint preserves the shared sidebar', async ({ page }) => {
  for (const state of ['idle', 'settings']) {
    await page.goto(`/visual-fixtures.html?state=${state}&checklist=hidden`);
    for (const width of [1120, 760, 761, 720, 1120]) {
      await page.setViewportSize({ width, height: 560 });
      await expect(page.locator(state === 'settings' ? '.settings-sidebar' : '.home-sidebar')).toHaveCSS('width', width <= 760 ? '184px' : '200px');
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
      expect(await page.evaluate(() => document.documentElement.scrollHeight)).toBe(560);
    }
  }
});

for (const state of ['first-run-model', 'first-run-onboarding']) {
  test(`${state} remains accessible in a short window`, async ({ page }) => {
    await page.setViewportSize({ width: 720, height: 560 });
    await page.goto(`/visual-fixtures.html?state=${state}&appearance=dark`);
    await expect(page.locator('.main-first-run')).toBeVisible();
    const first = page.locator('.main-first-run button').first();
    const last = page.locator('.main-first-run button').last();
    for (const control of [last, first]) {
      await control.focus();
      const bounds = await control.boundingBox();
      expect(bounds!.y).toBeGreaterThanOrEqual(0);
      expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(560);
      expect(await page.evaluate(() => window.scrollY)).toBe(0);
    }
  });
}
