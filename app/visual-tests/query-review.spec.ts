import { expect, test } from '@playwright/test';

for (const appearance of ['light', 'dark']) {
  test(`query popover fits native geometry in ${appearance}`, async ({ page }) => {
    await page.setViewportSize({ width: 440, height: 340 });
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
    await page.screenshot({ path: `test-results/query-review-${appearance}.png` });
    await page.locator('.query-review-details summary').click();
    await expect(page.getByLabel('Query context')).toBeVisible();
    await expect(page.getByLabel('Query capabilities')).toBeVisible();
    await page.goto(`/visual-fixtures.html?state=query-review&appearance=${appearance}&long=1`);
    const overflow = await page.locator('.query-review-content').evaluate(el => el.scrollHeight > el.clientHeight);
    expect(overflow).toBe(true);
    await expect(page.getByRole('button', { name: 'Open in Assistant' })).toBeVisible();
  });
}
