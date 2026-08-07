import { expect, test } from '@playwright/test';

const states = ['idle', 'recording', 'processing', 'update-recovering', 'settings'] as const;
const appearances = ['light', 'dark'] as const;

for (const appearance of appearances) {
  for (const state of states) {
    test(`${appearance} ${state} matches the canonical 880x720 surface`, async ({ page }) => {
      const pageErrors: string[] = [];
      page.on('pageerror', (error) => pageErrors.push(error.message));
      await page.emulateMedia({ colorScheme: appearance });
      await page.goto(`/visual-fixtures.html?state=${state}&appearance=${appearance}`);
      expect(pageErrors).toEqual([]);
      const fixture = page.locator('[data-visual-ready="true"]');
      await expect(fixture).toBeVisible();
      await expect(fixture).toHaveScreenshot(`${appearance}-${state}.png`);
    });
  }
}

test('selected history filters remain selected while hovered', async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.goto('/visual-fixtures.html?state=idle&appearance=dark');
  const mic = page.getByRole('button', { name: 'Mic' });
  await mic.hover();
  await mic.click();
  await expect(mic).toHaveAttribute('aria-pressed', 'true');

  await expect.poll(() => mic.evaluate((element) => {
    const selected = getComputedStyle(element);
    const probe = document.createElement('span');
    probe.style.color = 'var(--murmur-on-surface)';
    document.body.appendChild(probe);
    const expectedBackground = getComputedStyle(probe).color;
    probe.style.color = 'var(--murmur-background)';
    const expectedForeground = getComputedStyle(probe).color;
    probe.remove();
    return {
      backgroundMatches: selected.backgroundColor === expectedBackground,
      foregroundMatches: selected.color === expectedForeground,
    };
  })).toEqual({ backgroundMatches: true, foregroundMatches: true });
});

test('the transcript search placeholder fits beside its shortcut badge', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=idle&appearance=dark');
  const search = page.getByRole('searchbox', { name: 'Search transcripts' });
  const fit = await search.evaluate((input: HTMLInputElement) => {
    const style = getComputedStyle(input);
    const canvas = document.createElement('canvas');
    const context = canvas.getContext('2d')!;
    context.font = style.font;
    const textWidth = context.measureText(input.placeholder).width;
    const availableWidth = input.clientWidth
      - parseFloat(style.paddingLeft)
      - parseFloat(style.paddingRight);
    return { textWidth, availableWidth };
  });

  expect(fit.textWidth).toBeLessThanOrEqual(fit.availableWidth);
});

test('the window header flows into the history toolbar without a divider', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=idle&appearance=light');
  const header = page.locator('.ui-window-header');
  await expect(header).toBeVisible();
  await expect.poll(() => header.evaluate((element) => (
    getComputedStyle(element).borderBottomWidth
  ))).toBe('0px');
});

test('update discovery cannot expand or wrap the recovering header', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=update-recovering&appearance=light');

  const header = page.locator('.ui-window-header');
  const update = page.getByTestId('update-indicator');
  const hotkey = page.getByTestId('hotkey-hint');
  const record = page.getByTestId('record-pill');

  await expect(header).toBeVisible();
  await expect(update).toHaveAccessibleName('Murmur v0.27.1 is available. View update');
  await expect(record).toHaveAccessibleName('Recovering');
  await expect(record).toContainText('Wait');

  const geometry = await Promise.all([
    header.boundingBox(),
    update.boundingBox(),
    hotkey.boundingBox(),
    record.boundingBox(),
  ]);
  const [headerBox, updateBox, hotkeyBox, recordBox] = geometry;

  expect(updateBox?.width).toBeLessThanOrEqual(26);
  expect(updateBox?.height).toBeLessThanOrEqual(26);
  expect(hotkeyBox?.height).toBeLessThanOrEqual(18);
  expect(recordBox?.width).toBe(72);
  expect(recordBox?.height).toBeLessThanOrEqual(26);
  expect(headerBox?.height).toBe(42);
});
