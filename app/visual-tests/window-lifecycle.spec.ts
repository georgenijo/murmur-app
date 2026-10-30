import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
import type { Page } from '@playwright/test';

const config = JSON.parse(readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
const mainWindow = config.app.windows.find((window: { label: string }) => window.label === 'main');
test.use({ viewport: { width: mainWindow.minWidth!, height: mainWindow.minHeight! } });

async function expectNoHorizontalClipping(page: Page, label: string) {
  const overflow = await page.locator('#root').evaluate((root) => {
    return [...root.querySelectorAll<HTMLElement>('button, input:not([type=hidden]), select, textarea, [role=switch]')]
      .filter((element) => element.checkVisibility())
      .filter((element) => {
        const rect = element.getBoundingClientRect();
        return rect.width > 0 && (rect.right > window.innerWidth + 1 || rect.left < -1);
      }).map((element) => element.textContent?.trim().slice(0, 60) || element.tagName);
  });
  expect(overflow, label).toEqual([]);
  expect(await page.locator('#root').evaluate((root) => root.scrollWidth <= root.clientWidth + 1), label).toBe(true);
}

for (const appearance of ['light', 'dark']) {
  test(`workspace measurements and native theme bridge (${appearance})`, async ({ page }) => {
    await page.goto(`/visual-fixtures.html?state=settings&appearance=${appearance}`);
    await expect(page.locator('[data-visual-ready="true"]')).toBeVisible();
    const measurement = await page.evaluate(async () => {
      const modulePath = '/src/lib/windowLifecycle.tsx';
      const { measureWindowAppearance } = await import(modulePath);
      const header = document.querySelector('.ui-window-header')!;
      return {
        appearance: measureWindowAppearance(),
        headerHeight: header.getBoundingClientRect().height,
        background: getComputedStyle(document.documentElement).getPropertyValue('--murmur-background').trim(),
      };
    });
    expect(measurement.appearance).toEqual({ background: measurement.background, headerHeight: measurement.headerHeight, zoom: 1 });
    expect(measurement.headerHeight).toBeGreaterThan(0);
    expect(measurement.background).toMatch(/^#[a-f0-9]{6}$/i);
    await expect(page).toHaveScreenshot(`window-minimum-${appearance}.png`);
  });

  test(`all settings pages fit the 840×620 native minimum (${appearance})`, async ({ page }) => {
    expect([mainWindow.minWidth, mainWindow.minHeight]).toEqual([840, 620]);
    expect([mainWindow.width, mainWindow.height]).toEqual([1100, 780]);
    expect(mainWindow.visible).toBe(false);
    await page.goto(`/visual-fixtures.html?state=settings&appearance=${appearance}`);
    await expect(page.locator('[data-visual-ready="true"]')).toBeVisible();
    const nav = page.getByRole('navigation', { name: 'Settings pages' });
    const labels = await nav.getByRole('button').allTextContents();
    expect(labels.length).toBeGreaterThan(5);
    for (const label of labels) {
      await page.goto(`/visual-fixtures.html?state=${label === 'Appearance' ? 'settings-appearance' : 'settings'}&appearance=${appearance}`);
      await expect(page.locator('[data-visual-ready="true"]')).toBeVisible();
      await nav.getByRole('button', { name: label, exact: true }).click();
      await expect(page.locator('#root'), label).toContainText(/\S/);
      // Vertical content is intentionally scrollable; no pane may force a
      // horizontal scroll or put its actionable right edge off screen.
      await expectNoHorizontalClipping(page, label);
    }
    for (const detail of ['Speech-to-Text', 'Voice Query', 'Selected-Text Rewrite']) {
      await nav.getByRole('button', { name: 'AI & Models', exact: true }).click();
      await page.locator('main').getByRole('button', { name: new RegExp(`^${detail}`) }).click();
      await expect(page.locator('.settings-card-body')).toBeVisible();
      await expectNoHorizontalClipping(page, detail);
    }
  });
}
