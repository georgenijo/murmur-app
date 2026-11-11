import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.clock.setFixedTime(new Date('2026-09-04T20:00:00Z'));
  await page.setViewportSize({ width: 1120, height: 820 });
});

for (const theme of ['sonic', 'flat-dark', 'open-vsx-low-contrast', 'open-vsx-high-saturation']) {
  test(`${theme} separates dark transcript days and rows and keeps recording compact`, async ({ page }) => {
    await page.goto(`/visual-fixtures.html?state=recording&appearance=dark&theme=${theme}`);
    const cards = page.locator('.home-history .transcript-card');
    const dayStart = cards.nth(0);
    const row = cards.nth(1);
    await expect(dayStart).toHaveAttribute('data-day-start', 'true');
    await expect(row).toHaveAttribute('data-day-start', 'false');
    const dayEnd = page.locator('.home-history .transcript-card[data-day-end="true"]').first();
    const sample = (card: typeof row) => card.evaluate((element) => {
      const css = getComputedStyle(element);
      const canvas = document.createElement('canvas');
      canvas.width = canvas.height = 1;
      const context = canvas.getContext('2d');
      if (!context) throw new Error('Canvas color sampling unavailable');
      const paint = (background: string, foreground?: string) => {
        context.clearRect(0, 0, 1, 1);
        context.fillStyle = background;
        context.fillRect(0, 0, 1, 1);
        if (foreground) {
          context.fillStyle = foreground;
          context.fillRect(0, 0, 1, 1);
        }
        return Array.from(context.getImageData(0, 0, 1, 1).data).slice(0, 3);
      };
      const luminance = (rgb: number[]) => {
        const [r, g, b] = rgb.map((channel) => {
          const value = channel / 255;
          return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
        });
        return 0.2126 * r + 0.7152 * g + 0.0722 * b;
      };
      const contrast = (a: number[], b: number[]) => {
        const x = luminance(a);
        const y = luminance(b);
        return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
      };
      const background = getComputedStyle(document.documentElement).getPropertyValue('--murmur-background');
      const divider = getComputedStyle(element, '::before');
      return {
        surfaceContrast: contrast(paint(background), paint(background, css.backgroundColor)),
        dividerContrast: contrast(paint(background), paint(background, divider.backgroundColor)),
        borderWidth: parseFloat(css.borderTopWidth),
        radius: parseFloat(css.borderTopLeftRadius),
        shadow: css.boxShadow,
      };
    });
    const top = await sample(dayStart);
    const divider = await sample(row);
    const bottom = await sample(dayEnd);
    // Space separates days; resting rows stay on the page without an outer border.
    expect(top.surfaceContrast).toBe(1);
    expect(top.borderWidth).toBe(0);
    expect(top.radius).toBeGreaterThanOrEqual(8);
    expect(bottom.radius).toBeGreaterThanOrEqual(8);
    expect(bottom.shadow).toBe('none');
    // Rows inside a day stay separated by a hairline divider.
    expect(divider.dividerContrast).toBeGreaterThan(1.1);
    const labels = page.locator('.history-date-label');
    const first = await labels.first().boundingBox();
    const firstBody = await dayStart.locator('.transcript-body').boundingBox();
    expect(firstBody!.y - first!.y - first!.height).toBe(8);
    const later = await labels.nth(1).boundingBox();
    const previousBody = await dayEnd.locator('.transcript-body').boundingBox();
    expect(later!.y - previousBody!.y - previousBody!.height).toBe(28);
    const recording = page.locator('.home-talk');
    await expect(recording).toContainText('Listening');
    expect((await page.getByTestId('home-record-button').boundingBox())!.height).toBeLessThanOrEqual(56);
    await page.setViewportSize({ width: 720, height: 560 });
    const box = await recording.boundingBox();
    expect(box).not.toBeNull();
    if (box) expect(box.x + box.width).toBeLessThanOrEqual(720);
  });
}

test('a dark-only theme applies from Light and keeps Appearance and Customize legible', async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'light' });
  await page.addInitScript(() => {
    localStorage.setItem('murmur-theme-library', JSON.stringify({
      version: 1, revision: 1,
      themes: [{
        version: 1, id: 'flat-dark', label: 'Flat Dark', modes: ['dark'], source: { kind: 'local' },
        theme: {
          version: 1, presetId: 'custom',
          dark: {
            background: '#111111', surface: '#111111',
            'surface-container-low': '#111111', 'surface-container-lowest': '#111111',
            'surface-container': '#111111', 'surface-container-high': '#111111',
            'surface-container-highest': '#111111', 'outline-variant': '#111111',
            'on-surface': '#c4c4cc', 'on-surface-variant': '#a0a0a8', primary: '#999999',
          },
        },
      }],
    }));
  });
  await page.goto('/visual-fixtures.html?state=settings-appearance&appearance=light');
  await page.getByRole('button', { name: 'Appearance', exact: true }).click();
  await expect(page.locator('html')).toHaveAttribute('data-appearance', 'light');
  await page.getByRole('button', { name: 'Use Flat Dark theme', exact: true }).click();
  await expect(page.locator('html')).toHaveAttribute('data-appearance', 'dark');
  await expect(page.getByRole('radio', { name: /dark/i })).toBeChecked();
  await page.mouse.move(0, 0);
  await page.getByRole('button', { name: 'Customize', exact: true }).click();
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-appearance', 'dark');
  await page.getByRole('button', { name: 'Appearance', exact: true }).click();
  await page.getByRole('button', { name: /^Use Sonic for light mode/ }).click();
  await expect(page.locator('html')).toHaveAttribute('data-appearance', 'light');
  await page.getByRole('group', { name: 'Flat Dark light and dark styles' })
    .getByRole('button', { name: /for dark mode/ }).click();
  await expect(page.locator('html')).toHaveAttribute('data-appearance', 'dark');
});
