import { expect, test } from '@playwright/test';
import geometry from '../src/components/overlay/overlay-geometry.fixture.json' with { type: 'json' };

for (const placement of ['notch', 'below']) {
  for (const appearance of ['light', 'dark']) {
    for (const motion of ['no-preference', 'reduce'] as const) {
      test(`${placement} ${appearance} ${motion} production island states`, async ({ page }) => {
        await page.emulateMedia({ reducedMotion: motion });
        await page.setViewportSize({ width: 680, height: 900 });
        await page.goto(`/visual-fixtures.html?state=voice-island&placement=${placement}&appearance=${appearance}`);
        const fixture = page.locator('[data-fixture-synthetic="voice-island"]');
        await expect(fixture).toBeVisible();
        for (const [variant, key] of [
          ['idle', placement === 'below' ? 'belowIdle' : 'idleNotched'],
          ['recording', placement === 'below' ? 'belowRecording' : 'recordingNotched'],
          ['flow', placement === 'below' ? 'belowProcessing' : 'processingNotched'],
          ['expanded', placement === 'below' ? 'belowExpanded' : 'expandedNotched'],
          ['meeting', placement === 'below' ? 'belowMeetingSuggestion' : 'expandedMeetingNotched'],
        ] as const) {
          const row = fixture.locator(`[data-variant="${variant}"]`);
          const g = geometry[key];
          const box = await row.locator('.overlay-island').boundingBox();
          expect(box!.width).toBe(g.windowW);
          expect(box!.height).toBe(variant === 'expanded' || variant === 'meeting' ? g.expandedH : g.collapsedH);
          await expect(row.locator('.overlay-island')).toHaveCSS('background-color', 'rgb(0, 0, 0)');
        }
        for (const priority of ['cancelled', 'secure', 'miss', 'microphone', 'delivery']) {
          await expect(fixture.locator(`[data-variant="${priority}"] .overlay-field-slot`)).toHaveCSS('visibility', 'hidden');
        }
        const plate = await fixture.locator('[data-variant="recording"] .overlay-field-slot').boundingBox();
        expect(plate!.height).toBe(26);
        const canvas = fixture.locator('[data-variant="recording"] canvas');
        await expect(canvas).toHaveAttribute('data-columns', '43');
        await expect(canvas).toHaveAttribute('data-rows', '9');
        const words = fixture.locator('.live-words-pill');
        await expect(words).toHaveCSS('height', '32px');
        await expect(words).toHaveCSS('padding-left', '14px');
        await expect(words).toHaveCSS('font-size', '13px');
        await expect(words.locator('button, input, textarea, [title], [contenteditable]')).toHaveCount(0);
        await expect(fixture).toHaveScreenshot(`${placement}-${appearance}-${motion}.png`);
      });
    }
  }
}

test('external pretend-notch geometry and narrow tentative words remain bounded', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/visual-fixtures.html?state=voice-island&display=external&variant=recording');
  const island = page.locator('.overlay-island');
  const box = await island.boundingBox();
  expect([box!.width, box!.height]).toEqual([241, 70]);
  await expect(island).toHaveScreenshot('external-recording.png');
  await page.setViewportSize({ width: 560, height: 120 });
  await page.goto('/visual-fixtures.html?state=dictation-preview&text=' + encodeURIComponent('Synthetic provisional words '.repeat(24)));
  const words = page.locator('.live-words-pill');
  await expect(words).toBeVisible();
  await expect(words.locator('.live-words-stable')).toHaveCount(0);
  expect(await words.evaluate(el => el.scrollWidth)).toBeLessThanOrEqual(560);
  await expect(page).toHaveScreenshot('tentative-live-words.png');
});

test('placement, live words and processing style reuse general settings and Advanced', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/visual-fixtures.html?state=settings-folding&page=general');
  const placement = page.getByRole('combobox', { name: 'Recording overlay placement' });
  await placement.click();
  await page.getByRole('option', { name: 'Below-menu-bar pill' }).click();
  await expect(placement).toContainText('Below-menu-bar pill');
  const live = page.getByRole('switch', { name: 'Live words', exact: true });
  await expect(live).not.toBeChecked();
  await live.check();
  await expect(live).toBeChecked();
  const style = page.getByRole('combobox', { name: 'Processing animation', exact: true });
  await expect(style).toHaveCount(0);
  await page.getByRole('button', { name: /^Advanced/ }).click();
  await style.click();
  await page.getByRole('option', { name: 'Pulse', exact: true }).click();
  await expect(style).toContainText('Pulse');
  await expect(style).toBeFocused();
});
