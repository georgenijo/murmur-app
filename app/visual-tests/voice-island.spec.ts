import { expect, test } from '@playwright/test';
import geometry from '../src/components/overlay/overlay-geometry.fixture.json' with { type: 'json' };

for (const placement of ['notch', 'pill'] as const) {
  for (const appearance of ['light', 'dark']) {
    for (const motion of ['no-preference', 'reduce'] as const) {
      test(`${placement} ${appearance} ${motion} production island states`, async ({ page }) => {
        await page.emulateMedia({ reducedMotion: motion });
        await page.setViewportSize({ width: 680, height: 900 });
        await page.goto(`/visual-fixtures.html?state=voice-island&placement=${placement}&appearance=${appearance}`);
        const fixture = page.locator('[data-fixture-synthetic="voice-island"]');
        await expect(fixture).toBeVisible();
        const pill = placement === 'pill';
        for (const [variant, key] of [
          ['idle', pill ? 'pillIdle' : 'idleNotched'],
          ['recording', pill ? 'pillRecording' : 'recordingNotched'],
          ['processing', pill ? 'pillProcessing' : 'processingNotched'],
          ['expanded', pill ? 'pillExpanded' : 'expandedNotched'],
          ['meeting', pill ? 'pillMeetingSuggestion' : 'expandedMeetingNotched'],
        ] as const) {
          const island = fixture.locator(`[data-variant="${variant}"] .overlay-island`);
          const g = geometry[key];
          await expect(island).toHaveCSS('width', `${g.windowW}px`);
          await expect(island).toHaveCSS('height',
            `${variant === 'expanded' || variant === 'meeting' ? g.expandedH : g.collapsedH}px`);
          await expect(island).toHaveCSS('background-color',
            pill ? 'rgba(12, 12, 14, 0.92)' : 'rgba(20, 20, 20, 0.92)');
        }
        // The idle pill is fully hidden; the notch island always shows.
        await expect(fixture.locator('[data-variant="idle"] .overlay-island'))
          .toHaveCSS('opacity', pill ? '0' : '1');
        await expect(fixture.locator('[data-variant="recording"] .overlay-island')).toHaveCSS('opacity', '1');
        await expect(fixture.locator('[data-variant="recording"] .overlay-waveform-bar'))
          .toHaveCount(pill ? 18 : 7);
        for (const priority of ['cancelled', 'secure', 'miss', 'microphone']) {
          await expect(fixture.locator(`[data-variant="${priority}"] .overlay-waveform`)).toHaveCSS('opacity', '0');
        }
        await expect(fixture.locator('[data-variant="delivery"] .overlay-waveform')).toHaveCount(0);
        await expect(fixture.locator('[data-variant="delivery"]').getByRole('button', { name: 'Try delivery again' })).toBeVisible();
        const words = fixture.locator('.live-words-card');
        await expect(words).toHaveCSS('padding-left', '14px');
        await expect(words).toHaveCSS('font-size', '13px');
        await expect(words.locator('button, input, textarea, [title], [contenteditable]')).toHaveCount(0);
        await expect(fixture).toHaveScreenshot(`${placement}-${appearance}-${motion}.png`);
      });
    }
  }
}

test('external pretend-notch geometry and long live words remain bounded', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/visual-fixtures.html?state=voice-island&display=external&variant=recording');
  const island = page.locator('.overlay-island');
  await expect(island).toHaveCSS('width', '241px');
  await expect(island).toHaveCSS('height', '30px');
  await expect(island).toHaveScreenshot('external-recording.png');
  await page.setViewportSize({ width: 560, height: 160 });
  await page.goto('/visual-fixtures.html?state=dictation-preview&text=' + encodeURIComponent('Synthetic provisional words '.repeat(24)));
  const words = page.locator('.live-words-card');
  await expect(words).toBeVisible();
  await expect(words.locator('.live-words-stable')).toHaveCount(0);
  const box = await words.boundingBox();
  expect(box!.height).toBeLessThanOrEqual(104);
  expect(await words.evaluate(el => el.scrollWidth)).toBeLessThanOrEqual(460);
  await expect(page).toHaveScreenshot('tentative-live-words.png');
});

test('placement and live words live in general settings', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/visual-fixtures.html?state=settings-folding&page=general');
  const placement = page.getByRole('combobox', { name: 'Recording overlay placement' });
  await placement.click();
  await page.getByRole('option', { name: 'Floating pill' }).click();
  await expect(placement).toContainText('Floating pill');
  const live = page.getByRole('switch', { name: 'Live words', exact: true });
  await expect(live).toBeChecked();
  await live.uncheck();
  await expect(live).not.toBeChecked();
  await expect(page.getByRole('combobox', { name: 'Processing animation', exact: true })).toHaveCount(0);
});
