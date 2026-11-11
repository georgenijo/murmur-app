import { expect, test, type Page } from '@playwright/test';

async function open(page: Page, pane: 'Text & vocabulary' | 'Recording') {
  await page.emulateMedia({ colorScheme: 'dark', reducedMotion: 'reduce' });
  await page.goto('/visual-fixtures.html?state=settings&appearance=dark');
  await expect(page.locator('[data-visual-ready="true"]')).toBeVisible();
  await page.locator('.settings-sidebar').getByRole('button', { name: pane, exact: true }).click();
  await page.evaluate(() => document.fonts.ready);
}

async function box(page: Page) {
  const content = await page.locator('.settings-content-container').boundingBox();
  if (!content) throw new Error('Settings content has no layout box');
  return content;
}

for (const pane of ['Text & vocabulary', 'Recording'] as const) {
  test(`${pane} keeps its content geometry when Advanced opens`, async ({ page }) => {
    await open(page, pane);
    const before = await box(page);
    const advanced = page.getByRole('button', { name: /^Advanced/ });
    await advanced.click();
    await expect(advanced).toHaveAttribute('aria-expanded', 'true');
    const after = await box(page);

    expect(after.x).toBeCloseTo(before.x, 5);
    expect(after.width).toBeCloseTo(before.width, 5);
  });
}
