import { expect, test, type Locator, type Page } from '@playwright/test';

async function open(page: Page, state = 'settings-geometry', appearance = 'light') {
  await page.emulateMedia({ colorScheme: appearance as 'light' | 'dark' });
  await page.goto(`/visual-fixtures.html?state=${state}&appearance=${appearance}`);
  await expect(page.locator('[data-visual-ready]')).toBeVisible();
}
async function css(el: Locator, values: Record<string, string>) {
  for (const [property, value] of Object.entries(values)) await expect(el).toHaveCSS(property, value);
}

for (const appearance of ['light', 'dark']) {
  test(`${appearance} real rows, cards, controls and states have exact geometry`, async ({ page }, info) => {
    await page.setViewportSize({ width: 1180, height: 1400 });
    await open(page, 'settings-geometry', appearance);
    await css(page.locator('.settings-section'), { 'margin-bottom': '32px' });
    const card = page.locator('.settings-card').first();
    await css(card, { 'border-radius': '12px', 'border-top-width': '1px', padding: '0px', 'box-shadow': 'rgba(0, 0, 0, 0.05) 0px 1px 2px 0px' });
    expect(await card.evaluate(el => getComputedStyle(el).borderTopColor)).toMatch(/\/ 0\.6\)/);
    expect(await card.evaluate(el => getComputedStyle(el).backgroundColor)).toMatch(/\/ 0\.4\)/);
    const row = page.locator('.settings-setting-row').first();
    await css(row, { 'padding-top': '12px', 'padding-bottom': '12px', 'padding-left': '16px', 'padding-right': '16px', 'border-radius': '0px' });
    const layout = row.locator('.settings-row-layout');
    await css(layout, { display: 'grid', gap: '32px', 'align-items': 'center' });
    const columns = await layout.evaluate(el => getComputedStyle(el).gridTemplateColumns.split(' ').map(parseFloat));
    expect(columns[1]).toBeGreaterThanOrEqual(160);
    await css(row.locator('.settings-row-title'), { 'font-size': '14px', 'line-height': '20px', 'font-weight': '500' });
    await css(row.locator('.settings-row-description'), { 'font-size': '12px', 'line-height': '18px', 'max-width': '672px', 'margin-top': '4px', 'text-wrap-style': 'balance', '-webkit-line-clamp': 'none' });
    const second = page.locator('.settings-card > .settings-setting-row').nth(1);
    await css(second, { 'border-top-width': '1px' });
    expect(await second.evaluate(el => getComputedStyle(el).borderTopColor)).toMatch(/\/ 0\.5\)/);
    const bounds = await second.boundingBox();
    const cardBounds = await card.boundingBox();
    expect(bounds!.x).toBe(cardBounds!.x + 1);
    expect(bounds!.width).toBe(cardBounds!.width - 2);
    const rest = await row.evaluate(el => getComputedStyle(el).backgroundColor);
    await row.hover();
    await expect(row).toHaveCSS('background-color', rest);
    await css(row.locator('.settings-row-controls'), { gap: '8px', 'justify-content': 'flex-end', 'max-width': '320px' });
    await css(row.locator('.settings-reset-slot'), { width: '20px', height: '20px' });
    const reset = row.getByRole('button', { name: 'Reset to default' });
    await css(reset, { width: '20px', height: '20px', 'border-radius': '4px' });
    await css(reset.locator('svg'), { width: '12px', height: '12px' });
    const slotBefore = await row.locator('.settings-reset-slot').boundingBox();
    const toggle = row.getByRole('switch');
    await css(toggle, { width: '30px', height: '18px', padding: '2px', 'border-radius': '999px', 'transition-duration': '0.2s, 0.2s' });
    const thumb = toggle.locator('.ui-switch-thumb');
    await css(thumb, { width: '14px', height: '14px', 'transition-duration': '0.15s, 0.1s' });
    await toggle.hover(); await page.mouse.down();
    await expect(thumb).toHaveCSS('scale', '1.1 1');
    await page.mouse.up();
    await toggle.click();
    await expect(thumb).toHaveCSS('transform', 'matrix(1, 0, 0, 1, 12, 0)');
    const semanticColors = await toggle.evaluate(el => {
      const styles = getComputedStyle(el);
      const resolve = (name: string) => { const div = document.createElement('div'); div.style.color = styles.getPropertyValue(name); el.append(div); const color = getComputedStyle(div).color; div.remove(); return color; };
      return { on: styles.backgroundColor, primary: resolve('--murmur-primary'), knob: getComputedStyle(el.querySelector('.ui-switch-thumb')!).backgroundColor, background: resolve('--murmur-background') };
    });
    expect(semanticColors.on).toBe(semanticColors.primary);
    expect(semanticColors.knob).toBe(semanticColors.background);
    await reset.hover();
    await expect(page.getByRole('tooltip')).toHaveText('Reset to default');
    await reset.click();
    await expect(toggle).not.toBeChecked();
    await expect(thumb).toHaveCSS('transform', 'matrix(1, 0, 0, 1, 0, 0)');
    const offColor = await toggle.evaluate(el => ({ actual: getComputedStyle(el).backgroundColor, expected: getComputedStyle(document.documentElement).getPropertyValue('--murmur-outline-variant').trim() }));
    const resolveColor = await page.evaluate(value => { const el = document.createElement('div'); el.style.color = value; document.body.append(el); const resolved = getComputedStyle(el).color; el.remove(); return resolved; }, offColor.expected);
    await expect(toggle).toHaveCSS('background-color', resolveColor);
    await expect(reset).toHaveCount(0);
    expect(await row.locator('.settings-reset-slot').boundingBox()).toEqual(slotBefore);
    const small = page.getByRole('switch', { name: 'Small switch' });
    await css(small, { width: '26px', height: '16px', padding: '2px' });
    await css(small.locator('.ui-switch-thumb'), { width: '12px', height: '12px', transform: 'matrix(1, 0, 0, 1, 10, 0)' });
    const select = page.getByRole('combobox').first();
    await css(select, { height: '32px', 'min-width': '144px', 'padding-left': '11px', 'padding-right': '11px', 'border-radius': '8px', 'border-top-width': '1px', 'font-size': '14px' });
    await css(select.locator('svg'), { width: '14px', height: '14px', opacity: '0.7' });
    await css(page.getByRole('combobox', { name: 'Small select', exact: true }), { height: '28px' });
    await css(page.getByRole('combobox', { name: 'Extra small select' }), { height: '24px' });
    const input = page.getByRole('textbox', { name: 'Example input' });
    await css(input, { height: '30px', 'padding-left': '11px', 'padding-right': '11px', 'border-radius': '8px' });
    const unfocused = await input.evaluate(el => ({ border: getComputedStyle(el).borderTopColor, bg: getComputedStyle(el).backgroundColor, shadow: getComputedStyle(el).boxShadow }));
    expect(unfocused.shadow).toContain('0px 1px 2px');
    expect(unfocused.shadow).toContain(appearance === 'dark' ? '0px -1px' : '0px 1px');
    await input.focus();
    await expect(input).toHaveCSS('box-shadow', /\/ 0\.24\).*0px 0px 0px 3px/);
    for (const [name, height, padding] of [['Default', '32px', '11px'], ['Small', '28px', '9px'], ['Extra small', '24px', '7px']]) {
      const button = page.getByRole('button', { name, exact: true });
      await css(button, { height, 'padding-left': padding, 'padding-right': padding, 'font-size': '14px', 'font-weight': '500', 'border-radius': '8px', 'border-top-width': '1px' });
      await button.hover(); await page.mouse.down();
      await expect(button).toHaveCSS('transform', 'matrix(0.97, 0, 0, 0.97, 0, 0)');
      await page.mouse.up();
    }
    await css(page.locator('.settings-group-plain'), { gap: '4px' });
    await css(page.locator('.settings-setting-row[data-disabled]').first(), { opacity: '0.64' });
    await css(page.locator('.settings-unavailable-rows'), { opacity: '0.64' });
    await css(page.getByRole('switch', { name: 'Disabled', exact: true }), { opacity: '1' });
    await css(page.getByRole('switch', { name: 'Unavailable', exact: true }), { opacity: '1' });
    expect(await page.locator('.settings-unavailable').evaluate(el => getComputedStyle(el).backgroundColor)).toMatch(/\/ 0\.2\)/);
    await css(page.locator('.settings-unavailable-notice'), { 'font-size': '12px' });
    await css(page.locator('.settings-unavailable-notice svg'), { width: '14px', height: '14px' });
    await css(page.locator('.settings-segmented'), { height: '32px', 'border-radius': '8px' });
    if (appearance === 'dark') expect(await page.locator('.settings-segmented-btn[data-selected]').evaluate(el => getComputedStyle(el).backgroundColor)).toMatch(/\/ 0\.72\)/);
    await expect(page.locator('.settings-segmented-btn[data-selected]')).toHaveCSS('box-shadow', 'rgba(0, 0, 0, 0.1) 0px 1px 2px 0px');
    const measured = await page.locator('main').evaluate(root => {
      const selectors = ['.settings-section', '.settings-page-head', '.settings-section-title', '.settings-card', '.settings-setting-row', '.settings-row-layout', '.settings-row-title', '.settings-row-description', '.settings-row-controls', '.settings-reset-slot', '.ui-switch', '.ui-switch-thumb', '.ui-select-trigger', '.ui-form-input', '.ui-form-button', '.settings-unavailable-notice', '.settings-segmented'];
      const properties = ['width', 'height', 'padding', 'margin-bottom', 'gap', 'font-size', 'font-weight', 'line-height', 'border-radius', 'border-top-width', 'border-top-color', 'background-color', 'box-shadow', 'grid-template-columns', 'opacity', 'transition-duration'];
      return Object.fromEntries(selectors.map(selector => { const el = root.querySelector(selector)!; const css = getComputedStyle(el); return [selector, Object.fromEntries(properties.map(property => [property, css.getPropertyValue(property)]))]; }));
    });
    await info.attach('computed-settings-measurements', { body: JSON.stringify(measured, null, 2), contentType: 'application/json' });
    await info.attach('rendered-control-colors', { body: JSON.stringify({ semanticColors, unfocused }, null, 2), contentType: 'application/json' });
    await expect(page).toHaveScreenshot(`${appearance}-settings-primitives.png`);
  });

  test(`${appearance} responsive nav, content, section and row measurements`, async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await open(page, 'settings-general', appearance);
    const sidebar = page.locator('.settings-sidebar');
    await css(sidebar, { width: '240px', padding: '8px', 'border-right-width': '1px', gap: '16px' });
    await css(sidebar.locator('nav'), { gap: '4px' });
    const nav = sidebar.getByRole('button', { name: 'General', exact: true });
    await css(nav, { height: '32px', 'border-radius': '8px', 'padding-left': '10px', gap: '8px', 'font-size': '14px', 'font-weight': '500' });
    await css(nav.locator('svg'), { width: '16px', height: '16px' });
    await css(sidebar.locator('.settings-tools-eyebrow'), { 'font-size': '12px', 'font-weight': '500' });
    expect(await sidebar.locator('.settings-tools-eyebrow').evaluate(el => getComputedStyle(el).color)).toMatch(/\/ 0\.8\)/);
    await css(page.getByRole('searchbox'), { height: '32px', 'border-radius': '6px' });
    const content = page.locator('.settings-content-container');
    await css(content, { 'max-width': '896px', width: '896px', 'padding-top': '24px', 'padding-left': '24px', 'padding-bottom': '48px' });
    await css(page.locator('.settings-page-head'), { 'min-height': '28px', 'padding-left': '16px', 'margin-bottom': '10px' });
    await css(page.locator('.settings-section-title'), { 'font-size': '16px', 'line-height': '24px', 'font-weight': '600' });
    await css(page.locator('.settings-section'), { 'margin-bottom': '0px' });
    expect(await page.locator('main [data-testid=settings-content] h1').count()).toBe(0);
    for (const width of [768, 767, 640, 639, 560]) {
      await page.setViewportSize({ width, height: 720 });
      await css(sidebar, { width: width < 768 ? '56px' : '240px' });
      expect(await page.locator('.settings-window-header').evaluate(el => getComputedStyle(el).getPropertyValue('--ui-sidebar-width').trim())).toBe(width < 768 ? '56px' : '240px');
      await css(content, { 'padding-left': width < 640 ? '20px' : '24px', 'padding-right': width < 640 ? '20px' : '24px' });
      const row = page.locator('.settings-card > .settings-setting-row').first();
      await css(row, { 'padding-left': width < 640 ? '12px' : '16px' });
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    }
    await page.getByRole('button', { name: 'Search settings', exact: true }).click();
    await expect(page.getByRole('searchbox')).toBeFocused();
    await page.keyboard.press('Escape');
    await css(sidebar, { width: '56px' });
    await expect(page).toHaveScreenshot(`${appearance}-settings-rail.png`);
    await page.setViewportSize({ width: 1180, height: 900 });
    await open(page, 'settings-geometry', appearance);
    const row = page.locator('.settings-setting-row').first();
    // Change the real row's containing card width, independently of viewport.
    for (const [width, stacked] of [[546, false], [545, true]] as const) {
      await page.locator('.settings-card').first().evaluate((el, width) => { (el as HTMLElement).style.width = `${width}px`; }, width);
      await css(row.locator('.settings-row-layout'), { gap: stacked ? '12px' : '32px' });
      const values = await row.evaluate(el => {
        const copy = el.querySelector('.settings-title-description')!.getBoundingClientRect();
        const controls = el.querySelector('.settings-row-controls')!.getBoundingClientRect();
        return { copy: { x: copy.x, bottom: copy.bottom }, controls: { x: controls.x, y: controls.y, width: controls.width }, contentWidth: el.clientWidth - parseFloat(getComputedStyle(el).paddingLeft) * 2 };
      });
      if (stacked) { expect(values.controls.x).toBe(values.copy.x); expect(values.controls.y - values.copy.bottom).toBeCloseTo(12); expect(values.controls.width).toBe(values.contentWidth); }
      else expect(values.controls.x).toBeGreaterThan(values.copy.x);
    }
    await expect(page).toHaveScreenshot(`${appearance}-settings-stacked.png`);
  });
}

test('search highlights exact colors and destination pulses twice at 650ms', async ({ page }) => {
  for (const appearance of ['light', 'dark']) {
    await open(page, 'settings-general', appearance);
    await page.getByRole('searchbox').fill('Launch');
    const match = page.locator('mark').first();
    await css(match, { 'background-color': appearance === 'light' ? 'oklch(0.915 0.11 85)' : 'oklch(0.55 0.1 85)', color: appearance === 'light' ? 'oklch(0.216 0.028 77.4)' : 'oklch(0.97 0 0)' });
    await page.getByRole('button', { name: /Launch at Login/ }).click();
    const target = page.locator('[data-setting-target=launch-login]');
    await css(target, { 'animation-name': 'settings-search-target-pulse', 'animation-duration': '0.65s', 'animation-iteration-count': '2', 'animation-timing-function': 'ease-in-out' });
    const keyframes = await target.evaluate(el => el.getAnimations()[0].effect!.getKeyframes());
    expect(keyframes[1].boxShadow).toContain('2px');
    expect(keyframes[1].boxShadow).toContain('0.7');
  }
});
