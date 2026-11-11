import { expect, test } from '@playwright/test';

test.use({ browserName: 'webkit', viewport: { width: 880, height: 620 }, timezoneId: 'UTC' });

test.beforeEach(async ({ page }) => {
  await page.clock.setFixedTime(new Date('2026-11-11T15:00:00Z'));
  await page.goto('/visual-fixtures.html?state=insights-speed&appearance=light');
});

test('speed tooltip stays inside the visible Insights scroll area near its top', async ({ page }) => {
  const view = page.locator('.insights-view');
  const mark = page.locator('.ui-day-chart-line-targets button').nth(1);
  const point = mark.locator('span');
  await view.evaluate((el, selector) => {
    const point = document.querySelector(selector)!;
    const scroller = el as HTMLElement;
    scroller.scrollTop += point.getBoundingClientRect().top - (el.getBoundingClientRect().top + 18);
  }, '.ui-day-chart-line-targets button:nth-child(2) span');
  const anchor = (await point.boundingBox())!;
  const bounds = (await view.boundingBox())!;
  expect(anchor.y).toBeGreaterThanOrEqual(bounds.y);
  expect(anchor.y).toBeLessThan(bounds.y + 25);
  await page.mouse.move(anchor.x + anchor.width / 2, anchor.y + anchor.height / 2);
  await expect(mark).toHaveAttribute('data-active', 'true');
  const tooltip = page.locator('.insights-daily .ui-day-chart-tooltip');
  await expect.poll(async () => {
    const t = (await tooltip.boundingBox())!;
    const v = (await view.boundingBox())!;
    return t.y - Math.max(0, v.y);
  }).toBeGreaterThanOrEqual(0);
  const t = (await tooltip.boundingBox())!;
  expect(t.y + t.height).toBeLessThanOrEqual(Math.min(620, bounds.y + bounds.height));
  expect(t.x).toBeGreaterThanOrEqual(Math.max(0, bounds.x));
  expect(t.x + t.width).toBeLessThanOrEqual(Math.min(880, bounds.x + bounds.width));
});

test('focused speed tooltip stays centred through sidebar breakpoint resizing', async ({ page }) => {
  const mark = page.locator('.ui-day-chart-line-targets button').nth(3);
  await mark.focus();
  const tooltip = page.locator('.insights-daily .ui-day-chart-tooltip');
  const point = mark.locator('span');
  for (const width of [880, 750, 880]) {
    await page.setViewportSize({ width, height: 620 });
    await expect(page.locator('.home-sidebar')).toHaveAttribute('data-collapsed', String(width < 768));
    await expect.poll(async () => {
      const [t, p] = await Promise.all([tooltip.boundingBox(), point.boundingBox()]);
      if (!t || !p) return Infinity;
      return Math.abs(t.x + t.width / 2 - (p.x + p.width / 2));
    }, { message: `tooltip alignment at ${width}px` }).toBeLessThanOrEqual(3);
    const box = (await tooltip.boundingBox())!;
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(width);
  }
});
