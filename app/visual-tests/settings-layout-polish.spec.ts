import { expect, test, type Locator, type Page } from '@playwright/test';

const shots = process.env.LAYOUT_SHOTS;

async function open(page: Page, appearance: 'light' | 'dark', state: string, pane: string) {
  await page.emulateMedia({ colorScheme: appearance, reducedMotion: 'reduce' });
  await page.goto(`/visual-fixtures.html?state=${state}&appearance=${appearance}&smartTurn=ready`);
  await expect(page.locator('[data-visual-ready="true"]')).toBeVisible();
  await page.locator('.settings-sidebar').getByRole('button', { name: pane, exact: true }).click();
  await page.evaluate(() => document.fonts.ready);
}

async function box(locator: Locator) {
  const b = await locator.boundingBox();
  if (!b) throw new Error('element has no box');
  return { ...b, right: b.x + b.width, bottom: b.y + b.height };
}

async function snap(page: Page, locator: Locator, name: string) {
  if (shots) await locator.screenshot({ path: `${shots}/${name}.png` });
}

for (const appearance of ['dark', 'light'] as const) {
  test(`${appearance} General Advanced rows sit inside the card inset with row spacing`, async ({ page }) => {
    await open(page, appearance, 'settings', 'General');
    const card = page.locator('.settings-card').filter({ has: page.locator('.settings-advanced') });
    const toggle = card.locator('.settings-advanced-toggle');
    await toggle.click();
    const calibration = page.locator('[data-setting-target="overlay-calibration"]');
    const reset = page.locator('[data-setting-target="reset-statistics"]');
    await expect(reset).toBeVisible();
    await calibration.scrollIntoViewIfNeeded();
    await snap(page, card, `general-advanced-${appearance}`);
    const c = await box(card);
    const [t, cal, res] = [await box(toggle), await box(calibration), await box(reset)];
    for (const inner of [cal, res]) {
      expect(inner.x).toBeGreaterThanOrEqual(c.x + 12);
      expect(inner.right).toBeLessThanOrEqual(c.right - 12);
    }
    expect(Math.abs(cal.x - t.x)).toBeLessThanOrEqual(1);
    expect(Math.abs(res.x - t.x)).toBeLessThanOrEqual(1);
    expect(res.y - cal.bottom).toBeGreaterThanOrEqual(8);
    // The calibration card keeps its own padding, so its button sits inside it.
    await expect(calibration).not.toHaveCSS('padding-top', '0px');
    const calibrate = await box(calibration.getByRole('button', { name: 'Calibrate' }));
    expect(calibrate.bottom).toBeLessThanOrEqual(cal.bottom - 4);
  });

  test(`${appearance} Delivery Advanced keeps the app-overrides button styling`, async ({ page }) => {
    await open(page, appearance, 'settings', 'Delivery');
    await page.locator('.settings-advanced-toggle').first().click();
    const button = page.getByRole('button', { name: /Manage app overrides/ });
    await expect(button).toBeVisible();
    const style = await button.evaluate((e) => {
      const s = getComputedStyle(e);
      return { left: s.paddingLeft, right: s.paddingRight, top: s.borderTopWidth, bottom: s.borderBottomWidth };
    });
    expect(parseFloat(style.left)).toBeGreaterThan(0);
    expect(parseFloat(style.right)).toBeGreaterThan(0);
    expect(style.top).toBe(style.bottom);
  });

  test(`${appearance} Smart Auto microphone list stays inside the Recording card`, async ({ page }) => {
    await open(page, appearance, 'settings-smart-auto', 'Recording');
    await page.getByRole('button', { name: 'Smart Auto microphones' }).click();
    const list = page.getByRole('group', { name: 'Smart Auto microphone inclusion' });
    await expect(list).toBeVisible();
    const card = page.locator('.settings-card').filter({ has: list });
    await snap(page, card, `microphone-expanded-${appearance}`);
    const c = await box(card);
    const l = await box(list);
    expect(l.x).toBeGreaterThanOrEqual(c.x);
    expect(l.right).toBeLessThanOrEqual(c.right);
    const meter = await box(page.getByRole('meter', { name: 'Live microphone input level' }));
    expect(meter.right).toBeLessThanOrEqual(c.right);
    // The list and meter span the row width, not the narrow control column.
    expect(l.width).toBeGreaterThan(c.width * 0.8);
    // The label aligns to the top with the dropdown rather than floating mid-row.
    const label = await box(card.getByText('Microphone', { exact: true }).first());
    const trigger = await box(page.getByRole('button', { name: 'Microphone input' }));
    expect(Math.abs(label.y + label.height / 2 - (trigger.y + trigger.height / 2))).toBeLessThanOrEqual(8);
  });

  test(`${appearance} Advanced disclosure matches the row rhythm`, async ({ page }) => {
    await open(page, appearance, 'settings', 'Recording');
    const toggle = page.locator('.settings-advanced-toggle').first();
    const row = page.locator('[data-setting-target="stop-on-silence"]');
    const [t, r] = [await box(toggle), await box(row.locator('.settings-row-title'))];
    const wrapper = await box(page.locator('.settings-advanced').first());
    expect(wrapper.height).toBeGreaterThanOrEqual(40);
    expect(t.height).toBeGreaterThanOrEqual(20);
    expect(Math.abs(t.x - r.x)).toBeLessThanOrEqual(1);
    await expect(toggle).toHaveCSS('font-size', '13px');
    await expect(toggle.locator('svg')).toHaveCount(1);
    await snap(page, page.locator('.settings-card').filter({ has: toggle }), `advanced-toggle-${appearance}`);
    const closed = await toggle.locator('svg').evaluate((e) => getComputedStyle(e).transform);
    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-expanded', 'true');
    const opened = await toggle.locator('svg').evaluate((e) => getComputedStyle(e).transform);
    expect(opened).not.toBe(closed);
  });

  test(`${appearance} remote speaker details fold with Label remote speakers`, async ({ page }) => {
    await open(page, appearance, 'settings', 'Meetings');
    await page.locator('.settings-advanced-toggle').first().click();
    const speakers = page.locator('[data-setting-target="meeting-speakers"]');
    const toggle = speakers.getByRole('switch', { name: 'Label remote speakers' });
    const branch = speakers.locator('.settings-dependent-branch');
    const model = speakers.getByText('Remote speaker model', { exact: true });
    await expect(toggle).toHaveAttribute('aria-checked', 'false');
    await expect(branch).toHaveAttribute('inert', '');
    await expect(model).toBeHidden();
    await expect(speakers.getByText(/Labels stay on this Mac/)).toBeHidden();
    await toggle.click();
    await expect(branch).not.toHaveAttribute('inert', '');
    await expect(model).toBeVisible();
    await expect(speakers.getByText(/Labels stay on this Mac/)).toBeVisible();
    await expect(speakers.getByRole('button', { name: /Download model|Remove model/ })).toBeVisible();
    await toggle.click();
    await expect(model).toBeHidden();
    await expect(branch).toHaveAttribute('aria-hidden', 'true');
  });

  test(`${appearance} model card folds away while Smart Auto-Stop is off`, async ({ page }) => {
    await open(page, appearance, 'settings', 'Recording');
    await page.locator('.settings-advanced-toggle').first().click();
    const smart = page.locator('[data-setting-target="smart-auto-stop"]');
    await smart.scrollIntoViewIfNeeded();
    const parentRow = smart.locator('.settings-setting-row').first();
    await snap(page, smart, `smart-auto-stop-${appearance}`);
    await expect(parentRow).toHaveAttribute('data-disabled', 'true');
    await expect(smart.locator('[data-smart-auto-model]')).toBeHidden();
    await expect(smart.getByRole('button', { name: /Remove model|Download model/ })).toHaveCount(0);
  });

  test(`${appearance} model card appears once Smart Auto-Stop is on`, async ({ page }) => {
    await open(page, appearance, 'settings', 'Recording');
    await page.locator('[data-setting-target="stop-on-silence"]').getByRole('button', { name: '1.5s' }).click();
    await page.locator('.settings-advanced-toggle').first().click();
    const smart = page.locator('[data-setting-target="smart-auto-stop"]');
    const parentRow = smart.locator('.settings-setting-row').first();
    const model = smart.locator('[data-smart-auto-model]');
    await expect(parentRow).not.toHaveAttribute('data-disabled', /.*/);
    await expect(model).toBeHidden();
    await smart.getByRole('switch', { name: 'Smart Auto-Stop' }).click();
    await expect(model).toBeVisible();
    await expect(model).not.toHaveAttribute('data-disabled', /.*/);
    await expect(smart.getByRole('button', { name: 'Remove model' })).toBeEnabled();
    // The card keeps its own inset box inside the row.
    const row = await box(parentRow);
    const card = await box(model);
    expect(card.y).toBeGreaterThanOrEqual(row.bottom);
    expect(Math.abs(card.x - row.x)).toBeLessThanOrEqual(1);
  });
}
