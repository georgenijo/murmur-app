import { expect, test, type Page } from '@playwright/test';

test.use({ timezoneId: 'America/New_York' });

test.beforeEach(async ({ page }) => {
  await page.clock.setFixedTime(new Date('2026-09-04T20:00:00Z'));
});

// Resolved OpenCode roles after the mutable-palette contrast repair
// (docs/features/appearance.md, "OpenCode built-in theme").
const resolved = {
  light: {
    background: '#f8f8f8',
    surface: '#f8f8f8',
    'surface-container-low': '#fafafa',
    'surface-container': '#f2f2f2',
    'surface-container-high': '#eeeeee',
    'surface-container-lowest': '#ffffff',
    'surface-container-highest': '#dbdbdb',
    primary: '#304ce6',
    'primary-dim': '#2840cf',
    'on-primary': '#ffffff',
    'on-surface': '#161616',
    'on-surface-variant': '#5c5c5c',
    'outline-variant': '#cccccc',
    error: '#a91b29',
    success: '#00642b',
    warning: '#705300',
  },
  dark: {
    background: '#121212',
    surface: '#121212',
    'surface-container-low': '#242424',
    'surface-container': '#2e2e2e',
    'surface-container-high': '#3a3a3a',
    'surface-container-lowest': '#262626',
    'surface-container-highest': '#3c3c3c',
    primary: '#a2bcff',
    'primary-dim': '#8eacff',
    'on-primary': '#161616',
    'on-surface': '#fafafa',
    'on-surface-variant': '#aeaeae',
    'outline-variant': '#454545',
    error: '#ff9a95',
    success: '#6bd586',
    warning: '#f2cf76',
  },
} as const;

async function murmurTokens(page: Page) {
  return page.evaluate((names) => {
    const style = getComputedStyle(document.documentElement);
    return Object.fromEntries(names.map((name) => [
      name,
      style.getPropertyValue(`--murmur-${name}`).trim(),
    ]));
  }, Object.keys(resolved.light));
}

for (const appearance of ['light', 'dark'] as const) {
  test(`OpenCode ${appearance} applies from its built-in Appearance card`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: appearance });
    await page.goto(`/visual-fixtures.html?state=settings-appearance&appearance=${appearance}`);
    await page.getByRole('button', { name: 'Appearance', exact: true }).click();
    await page.getByRole('radio', { name: new RegExp(appearance, 'i') }).click();

    const card = page.locator('[data-theme-collection="OpenCode"]');
    await expect(card).toHaveCSS('width', '208px');
    await expect(card).toHaveCSS('height', '94px');
    await expect(card.getByRole('button', { name: 'Use OpenCode for light mode' })).toBeVisible();
    await expect(card.getByRole('button', { name: 'Use OpenCode for dark mode' })).toBeVisible();
    await expect(card.getByRole('button', { name: 'Export OpenCode' })).toBeVisible();
    await expect(card.getByRole('button', { name: 'Duplicate OpenCode' })).toBeVisible();
    await expect(card.getByRole('button', { name: 'Remove OpenCode' })).toHaveCount(0);

    await card.getByRole('button', { name: 'Use OpenCode theme' }).click();
    await expect(page.locator('html')).toHaveAttribute('data-appearance', appearance);
    await expect(card).toHaveAttribute('data-active', 'true');
    await expect(card.getByRole('button', { name: `Use OpenCode for ${appearance} mode` }))
      .toHaveAttribute('aria-pressed', 'true');
    expect(await murmurTokens(page)).toEqual(resolved[appearance]);

    const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('murmur-appearance') ?? '{}'));
    expect(stored.selection).toEqual({ light: 'opencode', dark: 'opencode' });
    expect(stored.cache[appearance]).toEqual(resolved[appearance]);

    await page.mouse.move(0, 0);
    await expect(page.locator('[data-visual-ready=true]')).toHaveScreenshot(`opencode-${appearance}-appearance.png`);
  });

  for (const state of ['idle', 'settings'] as const) {
    const destination = state === 'idle' ? 'home' : 'settings';
    test(`OpenCode ${appearance} renders ${destination} with its resolved roles`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: appearance });
      await page.goto(`/visual-fixtures.html?state=${state}&appearance=${appearance}&theme=opencode`);
      const fixture = page.locator('[data-visual-ready="true"]');
      await expect(fixture).toHaveAttribute('data-theme-fixture', 'opencode');
      expect(await murmurTokens(page)).toEqual(resolved[appearance]);
      await expect(fixture).toHaveScreenshot(`opencode-${appearance}-${destination}.png`);
    });
  }
}

test('duplicating OpenCode installs a removable local copy and keeps the built-in', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=settings-appearance&appearance=light');
  await page.getByRole('button', { name: 'Appearance', exact: true }).click();
  await page.getByRole('button', { name: 'Duplicate OpenCode', exact: true }).click();

  const copy = page.locator('[data-theme-collection="Copy of OpenCode"]');
  await expect(copy).toHaveCount(1);
  await expect(copy.getByRole('button', { name: 'Remove Copy of OpenCode' })).toBeVisible();
  const library = await page.evaluate(() => JSON.parse(localStorage.getItem('murmur-theme-library') ?? '{}'));
  expect(library.themes.map((entry: { id: string }) => entry.id)).toEqual(['copy-of-opencode']);
  expect(library.themes[0].modes).toEqual(['light', 'dark']);

  await copy.getByRole('button', { name: 'Remove Copy of OpenCode' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Remove', exact: true }).click();
  await expect(copy).toHaveCount(0);
  await expect(page.locator('[data-theme-collection="OpenCode"]')).toHaveCount(1);
});
