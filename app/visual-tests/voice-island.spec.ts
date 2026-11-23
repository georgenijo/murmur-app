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
          .toHaveCount(pill ? 12 : 7);
        for (const priority of ['cancelled', 'miss', 'microphone']) {
          // The pill swaps the waveform for a word; the notch fades it out.
          if (pill) await expect(fixture.locator(`[data-variant="${priority}"] .overlay-waveform`)).toHaveCount(0);
          else await expect(fixture.locator(`[data-variant="${priority}"] .overlay-waveform`)).toHaveCSS('opacity', '0');
        }
        await expect(fixture.locator('[data-variant="delivery"] .overlay-waveform')).toHaveCount(0);
        await expect(fixture.locator('[data-variant="delivery"]').getByRole('button', { name: 'Try delivery again' })).toBeVisible();
        const words = fixture.locator('.live-words-card');
        if (pill) {
          // One object: the floating pill has no separate words box at all.
          await expect(words).toHaveCount(0);
          await expect(fixture.locator('.live-words-window')).toHaveCount(0);
          await expect(fixture.locator('[data-variant="words"] .bezel-words')).toBeVisible();
        } else {
          await expect(words).toHaveCSS('padding-left', '14px');
          await expect(words).toHaveCSS('font-size', '13px');
          await expect(words.locator('button, input, textarea, [title], [contenteditable]')).toHaveCount(0);
        }
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
  await page.setViewportSize({ width: 560, height: 160 });
  await page.goto('/visual-fixtures.html?state=dictation-preview&text=' + encodeURIComponent('Synthetic provisional words '.repeat(24)));
  const words = page.locator('.live-words-card');
  await expect(words).toBeVisible();
  await expect(words.locator('.live-words-stable')).toHaveCount(0);
  const box = await words.boundingBox();
  expect(box!.height).toBeLessThanOrEqual(104);
  expect(await words.evaluate(el => el.scrollWidth)).toBeLessThanOrEqual(460);
});

const WORDS = {
  short: 'okay so',
  sentence: 'And it did not show up on the pill, so I think we can make it more minimal',
  longWord: 'Pneumonoultramicroscopicsilicovolcanoconiosis'.repeat(3),
  cjk: '今日は天気がとても良いので、公園まで散歩に行きたいと思います。'.repeat(4),
  fast: 'one two three four five six seven eight nine ten eleven twelve '.repeat(10),
};

for (const appearance of ['light', 'dark'] as const) {
  for (const [name, text] of Object.entries(WORDS)) {
    test(`floating pill keeps ${name} words on one line inside a fixed ${appearance} capsule`, async ({ page }) => {
      await page.emulateMedia({ reducedMotion: 'reduce', colorScheme: appearance });
      await page.setViewportSize({ width: 420, height: 200 });
      await page.goto(`/visual-fixtures.html?state=voice-island&placement=pill&variant=words&appearance=${appearance}&text=${encodeURIComponent(text)}`);
      const pill = page.locator('.overlay-island');
      const row = pill.locator('.bezel-row');
      const line = pill.locator('.bezel-words');
      await expect(line).toBeVisible();
      await page.evaluate(() => document.fonts.ready);
      const [capsule, rowBox, glyph, wave, lineBox] = await Promise.all([
        pill.boundingBox(), row.boundingBox(), pill.locator('.bezel-glyph').boundingBox(),
        pill.locator('.overlay-waveform').boundingBox(), line.boundingBox(),
      ]);
      // The capsule keeps its fixed words width and slim height whatever the text.
      expect(capsule!.width).toBe(geometry.pillWords.windowW);
      expect(capsule!.height).toBe(geometry.pillWords.collapsedH);
      // Dot, waveform and words sit side by side, inside the capsule.
      expect(glyph!.x).toBeGreaterThanOrEqual(capsule!.x + 1);
      expect(wave!.x).toBeGreaterThanOrEqual(glyph!.x + glyph!.width);
      expect(lineBox!.x).toBeGreaterThanOrEqual(wave!.x + wave!.width);
      expect(lineBox!.x + lineBox!.width).toBeLessThanOrEqual(capsule!.x + capsule!.width - 1);
      expect(rowBox!.x + rowBox!.width).toBeLessThanOrEqual(capsule!.x + capsule!.width);
      // One line: the text never wraps, so the line is never taller than the capsule's row.
      const metrics = await line.evaluate(el => {
        const span = el.querySelector('span')!;
        const range = document.createRange();
        range.selectNodeContents(span);
        const rects = [...range.getClientRects()];
        const tops = new Set(rects.map(r => Math.round(r.top)));
        return { lines: tops.size, whiteSpace: getComputedStyle(el).whiteSpace, overflow: el.getAttribute('data-overflow') };
      });
      expect(metrics.lines).toBeLessThanOrEqual(1);
      expect(metrics.whiteSpace).toBe('nowrap');
      // Text that does not fit fades in from the left and shows its newest end.
      const scrolled = await line.evaluate(el => ({ end: Math.abs(el.scrollWidth - el.clientWidth - el.scrollLeft) < 2, wide: el.scrollWidth > el.clientWidth }));
      if (scrolled.wide) {
        expect(metrics.overflow).toBe('true');
        expect(scrolled.end).toBe(true);
        expect(await line.evaluate(el => getComputedStyle(el).maskImage)).toContain('linear-gradient');
      } else {
        expect(metrics.overflow).toBe('false');
      }
      // No separate words box exists anywhere.
      await expect(page.locator('.live-words-card, .live-words-window')).toHaveCount(0);
    });
  }
}

test('the warning widens the floating pill and its phrase fits; the normal pill is narrower again', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.setViewportSize({ width: 420, height: 320 });
  await page.goto('/visual-fixtures.html?state=voice-island&placement=pill&appearance=dark');
  const widthOf = async (variant: string) =>
    (await page.locator(`[data-variant="${variant}"] .overlay-island`).boundingBox())!.width;
  const normal = await widthOf('words');
  const warning = await widthOf('warning');
  const compact = await widthOf('recording');
  expect(compact).toBe(geometry.pillRecording.windowW);
  expect(normal).toBe(geometry.pillWords.windowW);
  expect(warning).toBe(geometry.pillWarning.windowW);
  expect(compact).toBeLessThan(normal);
  expect(normal).toBeLessThan(warning);
  const island = page.locator('[data-variant="warning"] .overlay-island');
  const phrase = island.getByText('Model incomplete', { exact: true });
  const chip = island.locator('.bezel-chip');
  await expect(phrase).toBeVisible();
  await expect(chip).toHaveText('Fix');
  const [capsule, phraseBox, chipBox] = await Promise.all([island.boundingBox(), phrase.boundingBox(), chip.boundingBox()]);
  expect(phraseBox!.x).toBeGreaterThanOrEqual(capsule!.x);
  expect(chipBox!.x + chipBox!.width).toBeLessThanOrEqual(capsule!.x + capsule!.width - 1);
  expect(await island.locator('.bezel-row').evaluate(row => row.scrollWidth <= row.clientWidth)).toBe(true);
});

test('floating pill words need no separate native window box and Live Words off leaves only the dot and waveform', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/visual-fixtures.html?state=voice-island&placement=pill&variant=recording');
  const row = page.locator('[data-variant="recording"] .bezel-row');
  await expect(row.locator('.bezel-words')).toHaveCount(0);
  await expect(row.locator('> *')).toHaveCount(2);
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
        test(`Expanded ${geometry} ${state} reserves status separately from quick actions`, async ({ page }) => {
          await page.emulateMedia({ reducedMotion: 'reduce' });
          const placement = geometry === 'pill' ? 'pill' : 'notch';
          await page.goto(`/visual-fixtures.html?state=voice-island&variant=expanded&placement=${placement}&display=${geometry}&controls-state=${state}&appearance=${appearance}`);
          const controls = page.getByRole('group', { name: 'Quick settings' });
          await expect(controls).toBeVisible();
          const row = controls.locator('[data-testid="overlay-controls-row"]');
          await page.evaluate(() => document.fonts.ready);
          await expect(row).toHaveCSS('display', 'flex');
          await expect(row.getByRole('button', { name: /Mode:/ })).toHaveCount(0);
          const positions = await row.locator('button').evaluateAll(elements => elements.map(element => {
            const { x, right, y, bottom } = element.getBoundingClientRect();
            return { x, right, y, bottom };
          }));
          expect(positions).toHaveLength(3);
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
