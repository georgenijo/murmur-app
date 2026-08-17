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
  await expect.poll(() => page.locator('.ui-window-wordmark').evaluate((element) => (
    element.getBoundingClientRect().left
  ))).toBe(80);
});

for (const state of ['recording', 'update-recovering', 'settings'] as const) {
  test(`${state} title-bar controls share the native traffic-light centerline`, async ({ page }) => {
    await page.goto(`/visual-fixtures.html?state=${state}&appearance=light`);

    const header = page.locator('.ui-window-header');
    const items = [
      header.locator('.ui-window-wordmark'),
      header.getByTestId('main-status-chip'),
      ...(state === 'settings'
        ? [
            header.getByText('Settings', { exact: true }),
            header.getByRole('button', { name: 'Done' }),
          ]
        : [
            header.getByTestId('record-pill'),
            header.getByRole('button', { name: 'Open settings' }),
          ]),
    ];
    const [headerBox, centers] = await Promise.all([
      header.boundingBox(),
      Promise.all(items.map((item) => item.evaluate((element) => {
        const box = element.getBoundingClientRect();
        return box.top + box.height / 2;
      }))),
    ]);

    expect(Math.max(...centers) - Math.min(...centers)).toBeLessThanOrEqual(0.5);
    expect(headerBox).not.toBeNull();
    expect((headerBox!.y + headerBox!.height / 2) - centers[0]).toBeCloseTo(2, 1);
  });
}

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

test('settings editors preserve the primary hierarchy and provide a real back action', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=settings&appearance=light');
  await page.getByRole('button', { name: 'Text' }).click();
  await page.getByRole('button', { name: /^Aliases\b/ }).click();

  const fixture = page.locator('[data-visual-ready="true"]');
  await expect(page.getByRole('navigation', { name: 'Settings pages' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Text & Vocabulary', exact: true })).toHaveAttribute('aria-current', 'page');
  await expect(page.getByRole('navigation', { name: 'Settings editors' })).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'Aliases', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Back to Text settings' })).toBeVisible();
  await expect(fixture).toHaveScreenshot('light-settings-aliases.png');

  await page.keyboard.press('Escape');
  await expect(page.getByRole('heading', { name: 'Text & Vocabulary' })).toBeVisible();
});

test('appearance matches the compact collection-card layout', async ({ page }) => {
  const source = {
    kind: 'open-vsx',
    extensionId: 'h1dr0n.claude-theme',
    version: '1.0.0',
    license: 'MIT',
  } as const;
  const collection = { id: 'open-vsx:h1dr0n.claude-theme', label: 'Claude Theme' };
  const entry = (
    id: string,
    label: string,
    modes: Array<'light' | 'dark'>,
    theme: Record<string, unknown>,
  ) => ({
    version: 1,
    id,
    label,
    modes,
    theme: { version: 1, presetId: 'custom', ...theme },
    source,
    collection,
  });
  const library = {
    version: 1,
    revision: 1,
    themes: [
      entry('claude-classic', 'Claude Classic', ['light', 'dark'], {
        light: { background: '#f1efe7', foreground: '#141413', accent: '#0060a4' },
        dark: { background: '#1a1d23', foreground: '#d6d6d6', accent: '#74a9d8' },
      }),
      entry('claude-dusk', 'Claude Dusk (Deep Slate)', ['dark'], {
        dark: { background: '#1a1d23', foreground: '#d6d6d6', accent: '#74a9d8' },
      }),
      entry('claude-midnight', 'Claude Midnight (OLED Black)', ['dark'], {
        dark: { background: '#000000', foreground: '#f5f5f5', accent: '#74a9d8' },
      }),
      entry('claude-midnight-light', 'Claude Midnight Light (Pure High-Contrast)', ['light'], {
        light: { background: '#ffffff', foreground: '#000000', accent: '#000000' },
      }),
    ],
  };
  await page.addInitScript((value) => {
    localStorage.setItem('murmur-theme-library', JSON.stringify(value));
  }, library);
  await page.goto('/visual-fixtures.html?state=settings-appearance&appearance=light');
  await page.getByRole('button', { name: 'Appearance', exact: true }).click();
  await page.getByRole('button', { name: 'Use Claude Theme theme' }).click();

  const fixture = page.locator('[data-visual-ready="true"]');
  const sonicCard = page.locator('[data-theme-collection="Sonic"]');
  const claudeCard = page.locator('[data-theme-collection="Claude Theme"]');
  await expect(claudeCard).toHaveCount(1);
  await expect(sonicCard).toHaveCSS('width', '208px');
  await expect(sonicCard).toHaveCSS('height', '94px');
  await expect(claudeCard).toHaveCSS('width', '208px');
  await expect(claudeCard).toHaveCSS('height', '94px');
  await expect(claudeCard.locator('button[aria-label*="light variant"][aria-pressed="true"]')).toBeVisible();
  await expect(claudeCard.locator('button[aria-label*="dark variant"][aria-pressed="true"]')).toBeVisible();
  await expect(page.getByText('Active theme', { exact: false })).toHaveCount(0);
  await expect(page.getByText('Partly active')).toHaveCount(0);
  await expect(page.getByText('Choose dark style')).toHaveCount(0);
  await expect(page.getByText('Import a file or browse Open VSX')).toHaveCount(0);
  await expect(fixture).toHaveScreenshot('light-settings-appearance.png');

  await page.getByRole('radio', { name: /dark/i }).click();
  await expect(claudeCard.locator('button[aria-label*="dark variant"][aria-pressed="true"]')).toBeVisible();
  await expect(fixture).toHaveScreenshot('dark-settings-appearance.png');

  const cardHeight = await claudeCard.evaluate((element) => element.getBoundingClientRect().height);
  await claudeCard.locator('button[aria-label^="Choose dark variant"]').hover();
  const midnight = page.getByRole('button', { name: /Use .*Midnight.* for dark mode/ });
  await expect(midnight).toBeVisible();
  await expect.poll(() => claudeCard.evaluate((element) => element.getBoundingClientRect().height)).toBe(cardHeight);
  await midnight.click();
  await expect(claudeCard.locator('button[aria-label*="dark variant"][aria-label*="Midnight"]')).toBeVisible();
  await expect.poll(() => claudeCard.evaluate((element) => element.getBoundingClientRect().height)).toBe(cardHeight);
});

test('the main recording waveform reacts to audio without pulse animation', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=recording&appearance=dark');
  const waveform = page.getByTestId('main-recording-waveform');

  await expect(waveform.locator('span')).toHaveCount(5);
  await expect(waveform.locator('.animate-pulse')).toHaveCount(0);
  const heights = await waveform.locator('span').evaluateAll((bars) => (
    bars.map((bar) => getComputedStyle(bar).height)
  ));
  expect(new Set(heights).size).toBeGreaterThan(1);
});
