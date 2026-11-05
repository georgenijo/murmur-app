import { expect, test } from '@playwright/test';
import { surfaceCases } from './surface-matrix';
import geometry from '../src/components/overlay/overlay-geometry.fixture.json' with { type: 'json' };

for (const placement of ['notch', 'pill'] as const) {
  for (const appearance of ['light', 'dark'] as const) {
    for (const motion of ['no-preference', 'reduce'] as const) {
      test(`${placement} ${appearance} ${motion} production island states`, async ({ page }) => {
        const pill = placement === 'pill';
        // Bezel follows the system appearance, as the native pill material does;
        // the notch island is always dark and ignores it.
        await page.emulateMedia({ reducedMotion: motion, colorScheme: pill ? appearance : null });
        await page.setViewportSize({ width: 680, height: 900 });
        await page.goto(`/visual-fixtures.html?state=voice-island&placement=${placement}&appearance=${appearance}`);
        const fixture = page.locator('[data-fixture-synthetic="voice-island"]');
        await expect(fixture).toBeVisible();
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
            // Without the native material (as here) Bezel draws its tone opaque.
            !pill ? 'rgba(20, 20, 20, 0.92)' : appearance === 'dark' ? 'rgb(28, 28, 30)' : 'rgb(246, 246, 248)');
        }
        if (pill) {
          // The always-mounted dropdown must not squeeze the collapsed status row,
          // which fills the island inside its 1px border so it centres exactly.
          const row = fixture.locator('[data-variant="recording"] .overlay-pill-row');
          await expect(row).toHaveCSS('height', `${geometry.pillRecording.collapsedH - 2}px`);
        }
        // The idle pill is fully hidden; the notch island always shows.
        await expect(fixture.locator('[data-variant="idle"] .overlay-island'))
          .toHaveCSS('opacity', pill ? '0' : '1');
        await expect(fixture.locator('[data-variant="recording"] .overlay-island')).toHaveCSS('opacity', '1');
        await expect(fixture.locator('[data-variant="recording"] .overlay-waveform-bar'))
          .toHaveCount(pill ? 18 : 7);
        for (const priority of ['cancelled', 'miss', 'microphone']) {
          // The pill swaps the waveform for a word; the notch fades it out.
          if (pill) await expect(fixture.locator(`[data-variant="${priority}"] .overlay-waveform`)).toHaveCount(0);
          else await expect(fixture.locator(`[data-variant="${priority}"] .overlay-waveform`)).toHaveCSS('opacity', '0');
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


for (const { appearance, width, height, label } of surfaceCases()) {
  test.describe(`Voice overlay controls ${label}`, () => {
    test.use({ viewport: { width, height }, colorScheme: appearance });
    for (const geometry of ['notched', 'external', 'fallback', 'pill']) {
      for (const state of ['idle', 'starting', 'still-connecting', 'recording', 'recovering']) {
        test(`Expanded ${geometry} ${state} reserves status separately from Mode and quick actions`, async ({ page }) => {
          await page.emulateMedia({ reducedMotion: 'reduce' });
          const placement = geometry === 'pill' ? 'pill' : 'notch';
          await page.goto(`/visual-fixtures.html?state=voice-island&variant=expanded&placement=${placement}&display=${geometry}&controls-state=${state}&appearance=${appearance}`);
          const controls = page.getByRole('group', { name: 'Quick settings' });
          await expect(controls).toBeVisible();
          const row = controls.locator('[data-testid="overlay-controls-row"]');
          await page.evaluate(() => document.fonts.ready);
          await expect(row).toHaveCSS('display', 'flex');
          const mode = row.getByRole('button', { name: /Mode: Everyday/ });
          const modeSize = await mode.boundingBox();
          expect(modeSize!.width).toBeGreaterThan(45);
          await expect(mode).toContainText('Everyday');
          const positions = await row.locator('button').evaluateAll(elements => elements.map(element => {
            const { x, right, y, bottom } = element.getBoundingClientRect();
            return { x, right, y, bottom };
          }));
          expect(positions).toHaveLength(4);
          for (let index = 1; index < positions.length; index++) expect(positions[index].x).toBeGreaterThanOrEqual(positions[index - 1].right);
          const frame = await page.locator('[data-native-frame]').boundingBox();
          expect(frame!.width).toBe(241);
          for (const box of positions) {
            expect(box.x).toBeGreaterThanOrEqual(frame!.x);
            expect(box.right).toBeLessThanOrEqual(frame!.x + frame!.width);
          }
          if (state !== 'idle') {
            const status = row.locator('..').locator(':scope > span').last();
            const statusLayout = await status.evaluate(element => {
              const statusRect = element.getBoundingClientRect();
              const row = element.previousElementSibling!;
              const controlsBottom = Math.max(...[...row.querySelectorAll('button')].map(button => button.getBoundingClientRect().bottom));
              const frame = element.closest('[data-native-frame]')!.getBoundingClientRect();
              return { top: statusRect.top, bottom: statusRect.bottom, controlsBottom, frameBottom: frame.bottom };
            });
            expect(statusLayout.top).toBeGreaterThanOrEqual(statusLayout.controlsBottom);
            expect(statusLayout.bottom).toBeLessThanOrEqual(statusLayout.frameBottom + 1);
            expect(await status.textContent()).toBe(state === 'recording' ? '0:00' : state === 'recovering' ? 'Audio recovering' : state === 'starting' ? 'Connecting' : 'Still connecting');
          }
        });
      }
    }
  });
}
