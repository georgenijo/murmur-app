import { expect, test, type Page } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.clock.setFixedTime(new Date('2026-09-04T20:00:00Z'));
});
test.use({ timezoneId: 'America/New_York' });

async function open(page: Page, state: string, appearance = 'light') {
  await page.emulateMedia({ colorScheme: appearance });
  await page.goto(`/visual-fixtures.html?state=${state}&appearance=${appearance}`);
  await expect(page.locator('[data-visual-ready]')).toBeVisible();
}

for (const appearance of ['light', 'dark']) {
  test(`${appearance} production primitives have exact typography and menu measurements`, async ({ page }) => {
    await open(page, 'typography', appearance);
    await expect(page.locator('.ui-window-wordmark')).toHaveCSS('font-weight', '600');
    await expect(page.locator('h1')).toHaveCSS('letter-spacing', '-0.2px');
    for (const [selector, size, leading] of [['p.text-sm', '14px', '20px'], ['p.text-xs', '12px', '16px'], ['p.text-\\[11px\\]', '11px', '16px']]) {
      await expect(page.locator(selector)).toHaveCSS('font-size', size);
      await expect(page.locator(selector)).toHaveCSS('line-height', leading);
      // Chromium serializes zero letter spacing as "normal".
      expect(await page.locator(selector).evaluate(el => parseFloat(getComputedStyle(el).letterSpacing) || 0)).toBe(0);
    }
    await expect(page.locator('body')).toHaveCSS('font-family', /^-apple-system, (BlinkMacSystemFont|"system-ui"), system-ui, sans-serif$/);
    expect(await page.locator('body').evaluate(el => getComputedStyle(el).getPropertyValue('--ui-font-sans').trim())).toBe('-apple-system, BlinkMacSystemFont, system-ui, sans-serif');
    await expect(page.locator('.font-mono')).toHaveCSS('font-family', '"SF Mono", SFMono-Regular, Menlo, monospace');
    await expect(page.locator('.ui-dashboard-stat-value')).toHaveCSS('font-weight', '600');
    await expect(page.locator('.ui-dashboard-stat-value')).toHaveCSS('font-variant-numeric', 'tabular-nums');
    await expect(page.locator('p.text-sm')).toHaveCSS('text-wrap-mode', 'wrap');
    await expect(page.locator('p.text-sm')).toHaveCSS('text-wrap-style', 'pretty');
    await expect(page.locator('body')).toHaveCSS('overscroll-behavior', 'none');
    const button = page.getByRole('button', { name: 'Primary action', exact: true });
    await expect(button).toHaveCSS('font-weight', '500');
    await expect(button).toHaveCSS('border-radius', '8px');
    await expect(button).toHaveCSS('min-height', '32px');
    await expect(button).toHaveCSS('padding-left', '7px');
    await expect(button).toHaveCSS('border-top-width', '1px');
    await expect(button).toHaveCSS('box-shadow', 'rgba(255, 255, 255, 0.16) 0px 1px 0px 0px inset, rgba(0, 0, 0, 0.05) 0px 1px 2px 0px');
    const compact = page.getByRole('button', { name: 'Copy', exact: true });
    await expect(compact).toHaveCSS('width', '28px');
    await expect(compact).toHaveCSS('height', '28px');
    await expect(compact).toHaveCSS('border-radius', '8px');
    await expect(compact.locator('svg')).toHaveCSS('stroke-width', '1.75px');
    await expect(compact.locator('svg')).toHaveCSS('width', '14px');
    await button.hover();
    await page.mouse.down();
    await expect(button).toHaveCSS('transform', 'matrix(0.97, 0, 0, 0.97, 0, 0)');
    await page.mouse.up();
    await compact.click();
    await expect(compact).not.toHaveCSS('outline-style', 'solid');
    await page.keyboard.press('Tab');
    await expect(page.getByRole('button', { name: 'Fixture menu' })).toBeFocused();
    await expect(page.getByRole('button', { name: 'Fixture menu' })).toHaveCSS('box-shadow', /0px 0px 0px 2px/);
    await page.keyboard.press('Enter');
    const menu = page.getByRole('menu');
    await expect(menu).toHaveCSS('scale', 'none');
    await expect(menu).toHaveCSS('padding', '4px');
    await expect(menu).toHaveCSS('border-radius', '8px');
    await expect(menu).toHaveCSS('border-top-width', '1px');
    await expect(menu).toHaveCSS('-webkit-app-region', 'no-drag');
    await expect(menu).toHaveCSS('overflow-y', 'auto');
    const geometry = await menu.evaluate(el => {
      const css = getComputedStyle(el);
      return { origin: css.transformOrigin, declaredOrigin: css.getPropertyValue('--transform-origin'), maxHeight: css.maxHeight, available: css.getPropertyValue('--available-height'), background: css.backgroundColor, border: css.borderTopColor };
    });
    const origin = geometry.origin.split(' ').map(parseFloat);
    const declaredOrigin = geometry.declaredOrigin.trim().split(' ').map(parseFloat);
    expect(origin).toHaveLength(declaredOrigin.length);
    origin.forEach((value, index) => expect(value).toBeCloseTo(declaredOrigin[index], 3));
    expect(parseFloat(geometry.maxHeight)).toBeCloseTo(parseFloat(geometry.available), 3);
    expect(geometry.background).toMatch(/\/ 0\.8\)/);
    expect(geometry.border).toMatch(/\/ 0\.1\)/);
    await expect(menu).toHaveCSS('backdrop-filter', appearance === 'dark' ? 'blur(16px) saturate(1.08)' : 'blur(12px) saturate(1.14)');
    await expect(menu).toHaveCSS('box-shadow', appearance === 'dark' ? 'rgba(0, 0, 0, 0.8) 0px 18px 44px -18px' : 'rgba(0, 0, 0, 0.55) 0px 16px 40px -18px');
    const item = page.getByRole('menuitem', { name: 'Open folder ⌘O' });
    await expect(item).toHaveCSS('min-height', '28px');
    expect((await item.boundingBox())!.height).toBe(28);
    await expect(item).toHaveCSS('padding', '4px 8px');
    await expect(item).toHaveCSS('gap', '8px');
    await expect(item).toHaveCSS('border-radius', '4px');
    await expect(item).toHaveCSS('font-size', '14px');
    await expect(item).toHaveCSS('line-height', '20px');
    await expect(item.locator('svg')).toHaveCSS('width', '16px');
    const offsets = await item.evaluate(el => {
      const item = el.getBoundingClientRect();
      const icon = el.querySelector('.ui-menu-item-icon')!.getBoundingClientRect();
      const label = el.lastElementChild!.getBoundingClientRect();
      return { inset: icon.left - item.left, gap: label.left - icon.right };
    });
    expect(offsets).toEqual({ inset: 8, gap: 8 });
    await expect(item.locator('.ui-menu-item-icon')).toHaveCSS('opacity', '0.8');
    await expect(page.getByRole('menuitem', { name: 'Unavailable' })).toHaveCSS('opacity', '0.64');
    await expect(menu.getByRole('separator')).toHaveCSS('height', '1px');
    await expect(menu.getByRole('separator')).toHaveCSS('margin', '4px 8px');
    await expect(menu.locator('.ui-menu-shortcut')).toHaveCSS('font-size', '12px');
    await expect(menu.locator('.ui-menu-shortcut')).toHaveCSS('font-weight', '500');
    await expect(menu.locator('.ui-menu-shortcut')).toHaveCSS('letter-spacing', '1.2px');
    await page.keyboard.press('Escape');
    await page.getByRole('button', { name: 'Tooltip trigger' }).hover();
    const tooltip = page.locator('.ui-tooltip');
    await expect(tooltip).toBeVisible();
    await expect(tooltip).toHaveCSS('border-radius', '6px');
    await expect(tooltip).toHaveCSS('font-size', '12px');
    await expect(tooltip).toHaveCSS('line-height', '16px');
    await expect(tooltip).toHaveCSS('text-wrap-style', 'balance');
    await tooltip.evaluate(el => el.setAttribute('data-ending-style', ''));
    await expect(tooltip).toHaveCSS('scale', '0.98');
    await expect(tooltip).toHaveCSS('opacity', '0');
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await expect(tooltip).toHaveCSS('transition-duration', '1e-05s');
    const scrollbar = await page.locator('body').evaluate(el => {
      const lane = getComputedStyle(el, '::-webkit-scrollbar');
      const thumb = getComputedStyle(el, '::-webkit-scrollbar-thumb');
      return { width: lane.width, radius: thumb.borderRadius, color: thumb.backgroundColor };
    });
    expect(scrollbar.width).toBe('6px');
    expect(scrollbar.radius).toBe('3px');
    // Chromium's queried scrollbar pseudo can resolve its hover rule even
    // without a scrollable thumb. Check both exact palette tokens as well.
    expect(appearance === 'dark' ? ['rgba(255, 255, 255, 0.08)', 'rgba(255, 255, 255, 0.12)'] : ['rgb(217, 217, 217)', 'rgb(191, 191, 191)']).toContain(scrollbar.color);
    const palette = await page.locator('body').evaluate(el => {
      const css = getComputedStyle(el);
      return [css.getPropertyValue('--ui-scrollbar-thumb').trim(), css.getPropertyValue('--ui-scrollbar-thumb-hover').trim()];
    });
    expect(palette).toEqual(appearance === 'dark' ? ['rgb(255 255 255 / 8%)', 'rgb(255 255 255 / 12%)'] : ['rgb(217 217 217)', 'rgb(191 191 191)']);
  });


  test(`${appearance} real microphone popover uses the shared glass contract`, async ({ page }) => {
    await open(page, 'settings-general', appearance);
    await page.getByRole('button', { name: 'Recording', exact: true }).click();
    await page.getByRole('button', { name: 'Microphone input', exact: true }).click();
    const popup = page.getByRole('dialog', { name: 'Choose microphone mode' });
    await expect(popup).toBeVisible();
    await expect(popup).toHaveCSS('border-radius', '8px');
    await expect(popup).toHaveCSS('backdrop-filter', appearance === 'dark' ? 'blur(16px) saturate(1.08)' : 'blur(12px) saturate(1.14)');
    await expect(popup).toHaveCSS('-webkit-app-region', 'no-drag');
    await page.keyboard.press('Escape');
    await expect(popup).toBeHidden();
  });

  for (const state of ['idle', 'settings-general', 'settings-appearance', 'menu']) {
    test(`${appearance} ${state} typography golden`, async ({ page }) => {
      await open(page, state === 'menu' ? 'idle' : state, appearance);
      if (state === 'settings-appearance' || state === 'settings-general') {
        await expect(page.locator('.settings-nav-item[aria-current="page"]')).toHaveText(state === 'settings-general' ? 'General' : 'Appearance');
      }
      if (state === 'settings-general') {
        await expect(page.locator('.settings-page-title')).toHaveCSS('font-size', '20px');
        await expect(page.locator('.settings-page-title')).toHaveCSS('line-height', '26px');
        await expect(page.locator('.settings-page-title')).toHaveCSS('font-weight', '600');
      }
      if (state === 'settings-appearance') await expect(page.locator('[data-theme-collection="Sonic"]')).toBeVisible();
      if (state === 'settings-general') await expect(page.locator('.settings-card')).toHaveCSS('border-radius', '12px');
      if (state === 'menu') await page.getByRole('button', { name: /^Filter transcripts/ }).click();
      await expect(page).toHaveScreenshot(`${appearance}-${state}.png`);
    });
  }
}

test('middle truncation preserves the filename and refits on resize', async ({ page }) => {
  await open(page, 'typography');
  const path = page.locator('[data-testid="path-container"] > span');
  const head = path.locator(':scope > span').first();
  await expect(head).toHaveAttribute('data-overflow');
  await expect(path.locator(':scope > span').last()).toHaveText('transcription.md');
  await expect(path).toHaveAttribute('title', '/Users/example/workspaces/murmur-app/docs/features/transcription.md');
  await page.locator('[data-testid="path-container"]').evaluate(el => el.style.width = '800px');
  await expect(head).not.toHaveAttribute('data-overflow');
  await page.locator('[data-testid="path-container"]').evaluate(el => el.style.width = '140px');
  await expect(head).toHaveAttribute('data-overflow');
});
