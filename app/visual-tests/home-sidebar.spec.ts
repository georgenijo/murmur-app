import { expect, test } from '@playwright/test';

const fixture = (appearance = 'light', state = 'idle') => `/visual-fixtures.html?state=${state}&appearance=${appearance}&checklist=hidden`;

test.beforeEach(async ({ page }) => {
  await page.clock.setFixedTime(new Date('2026-09-04T20:00:00Z'));
});

for (const appearance of ['light', 'dark']) {
  for (const state of ['expanded', 'collapsed', 'narrow']) {
    test(`${appearance} Home sidebar ${state}`, async ({ page }) => {
      await page.setViewportSize(state === 'narrow' ? { width: 720, height: 560 } : { width: 1100, height: 780 });
      await page.goto(fixture(appearance, state === 'collapsed' ? 'home-sidebar-collapsed' : 'idle'));
      const sidebar = page.getByRole('complementary', { name: 'Murmur navigation' });
      const collapsed = state !== 'expanded';
      await expect(sidebar).toHaveCSS('width', collapsed ? '48px' : '256px');
      await expect(sidebar).toHaveAttribute('data-collapsed', String(collapsed));
      await expect(sidebar.getByRole('button', { name: collapsed ? 'Expand sidebar' : 'Collapse sidebar' })).toHaveAttribute('aria-expanded', String(!collapsed));
      for (const label of ['Home', 'Notetaker', 'Assistant', 'Insights']) {
        const item = sidebar.getByRole('button', { name: label, exact: true });
        await expect(item).toBeVisible();
        await expect(item).toBeEnabled();
        if (collapsed) {
          await expect(item).toHaveCSS('width', '32px');
          await expect(item).toHaveCSS('height', '32px');
        }
      }
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(state === 'narrow' ? 720 : 1100);
      await expect(page.locator('[data-visual-ready]')).toHaveScreenshot(`home-sidebar-${appearance}-${state}.png`);
    });
  }
}

test('T3 breakpoint collapses Home only and restores width and desktop preference', async ({ page }) => {
  await page.goto(fixture());
  const sidebar = page.locator('.home-sidebar');
  const resize = page.getByRole('separator', { name: 'Sidebar width' });
  await resize.focus();
  await page.keyboard.press('ArrowRight');
  await expect(sidebar).toHaveCSS('width', '272px');
  await page.getByRole('button', { name: 'Insights', exact: true }).click();
  const insights = page.getByRole('navigation', { name: 'Main destinations' }).getByRole('button', { name: 'Insights', exact: true });
  await insights.focus();
  await page.setViewportSize({ width: 767, height: 620 });
  await expect(sidebar).toHaveCSS('width', '48px');
  await expect(insights).toBeFocused();
  await expect(insights).toHaveAttribute('aria-current', 'page');
  await page.keyboard.press('Tab');
  await page.keyboard.press('Shift+Tab');
  await expect(page.locator('[data-base-ui-portal] [data-current]')).toHaveText('Insights');
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Expand sidebar' }).click();
  await expect(sidebar).toHaveCSS('width', '272px');
  await page.setViewportSize({ width: 768, height: 620 });
  await expect(sidebar).toHaveCSS('width', '272px');
  await page.getByRole('button', { name: 'Collapse sidebar' }).click();
  await page.setViewportSize({ width: 720, height: 560 });
  await page.setViewportSize({ width: 840, height: 620 });
  await expect(sidebar).toHaveCSS('width', '48px');
  await expect(insights).toHaveAttribute('aria-current', 'page');
  // Settings remains owned by #848, including its old compact breakpoint.
  for (const width of [720, 767, 840]) {
    await page.setViewportSize({ width, height: 620 });
    await page.goto('/visual-fixtures.html?state=settings');
    await expect(page.locator('.settings-sidebar')).toHaveCSS('width', width <= 760 ? '184px' : '200px');
  }
});

test('keyboard toggle, bounded separator and collapsed navigation keep accessible names', async ({ page }) => {
  await page.goto(fixture());
  const sidebar = page.locator('.home-sidebar');
  const toggle = page.getByRole('button', { name: 'Collapse sidebar' });
  const separator = page.getByRole('separator', { name: 'Sidebar width' });
  await toggle.focus();
  await page.keyboard.press('Enter');
  await expect(sidebar).toHaveCSS('width', '48px');
  await expect(page.getByRole('separator')).toHaveCount(0);
  await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: 'Home', exact: true })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: 'Notetaker', exact: true })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('navigation', { name: 'Main destinations' }).getByRole('button', { name: 'Notetaker' })).toHaveAttribute('aria-current', 'page');
  await page.getByRole('button', { name: 'Insights', exact: true }).hover();
  await expect(page.locator('[data-base-ui-portal] [data-current]')).toHaveText('Insights');
  await page.getByRole('button', { name: 'Expand sidebar' }).focus();
  await page.keyboard.press('Space');
  await expect(sidebar).toHaveCSS('width', '256px');
  await separator.focus();
  await expect(separator).toHaveAttribute('aria-orientation', 'vertical');
  await expect(separator).toHaveAttribute('aria-valuemin', '256');
  await expect(separator).toHaveAttribute('aria-valuemax', '384');
  for (const [key, width] of [['End', 384], ['ArrowRight', 384], ['Home', 256], ['ArrowLeft', 256], ['ArrowRight', 272], ['Shift+ArrowRight', 304], ['ArrowLeft', 288]] as const) {
    await page.keyboard.press(key);
    await expect(separator).toHaveAttribute('aria-valuenow', String(width));
    await expect(sidebar).toHaveCSS('width', `${width}px`);
  }
  // Automatic collapse returns keyboard focus from the removed separator to the toggle.
  await page.setViewportSize({ width: 720, height: 560 });
  await expect(page.getByRole('button', { name: 'Expand sidebar' })).toBeFocused();
});

test('pointer resizing clamps, commits, and rolls back on cancellation, lost capture, blur and breakpoint', async ({ page }) => {
  await page.goto(fixture());
  const sidebar = page.locator('.home-sidebar');
  const separator = page.getByRole('separator', { name: 'Sidebar width' });
  const start = async () => {
    const box = (await separator.boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + 80);
    await page.mouse.down();
  };
  await start();
  await page.mouse.move(1000, 130);
  await expect(sidebar).toHaveCSS('width', '384px');
  await page.mouse.move(0, 130);
  await expect(sidebar).toHaveCSS('width', '256px');
  await page.mouse.move(300, 130);
  await page.mouse.up();
  const committed = await sidebar.evaluate((el) => el.getBoundingClientRect().width);
  expect(committed).toBeGreaterThan(256);
  await page.getByRole('button', { name: 'Collapse sidebar' }).click();
  await page.getByRole('button', { name: 'Expand sidebar' }).click();
  await expect(sidebar).toHaveCSS('width', `${committed}px`);
  for (const cancel of ['Escape', 'pointercancel', 'lostpointercapture', 'blur', 'breakpoint']) {
    await start();
    await page.mouse.move(380, 130);
    if (cancel === 'Escape') await page.keyboard.press('Escape');
    else if (cancel === 'blur') await page.evaluate(() => window.dispatchEvent(new Event('blur')));
    else if (cancel === 'breakpoint') await page.setViewportSize({ width: 720, height: 560 });
    else await separator.evaluate((el, type) => el.dispatchEvent(new PointerEvent(type, { pointerId: 1, bubbles: true })), cancel);
    await page.mouse.up();
    if (cancel === 'breakpoint') await page.setViewportSize({ width: 880, height: 720 });
    await expect(sidebar, cancel).toHaveCSS('width', `${committed}px`);
    await expect(sidebar).toHaveAttribute('data-resizing', 'false');
  }
});

test('default, minimum and small browser windows measure the real Home resize bounds', async ({ page }) => {
  for (const width of [1100, 840, 768, 720]) {
    await page.setViewportSize({ width, height: width === 720 ? 560 : 620 });
    await page.goto(fixture());
    const sidebar = page.locator('.home-sidebar');
    await expect(sidebar).toHaveCSS('width', width <= 767 ? '48px' : '256px');
    if (width <= 767) await page.getByRole('button', { name: 'Expand sidebar' }).click();
    const separator = page.getByRole('separator', { name: 'Sidebar width' });
    await expect(separator).toHaveCSS('width', '8px');
    await expect(separator).toHaveCSS('touch-action', 'none');
    await expect(page.getByRole('button', { name: 'Collapse sidebar' })).toHaveCSS('width', '32px');
    await expect(sidebar.locator('.home-nav-icon').first()).toHaveCSS('width', '16px');
    await separator.focus();
    for (const [key, expected] of [['End', 384], ['Home', 256]] as const) {
      await page.keyboard.press(key);
      await expect(sidebar).toHaveCSS('width', `${expected}px`);
      const bounds = await page.locator('.main-dashboard-workspace').boundingBox();
      expect(bounds!.x).toBe(expected);
      expect(bounds!.width).toBe(width - expected);
      expect(await page.locator('.home-dashboard').evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    }
  }
});
