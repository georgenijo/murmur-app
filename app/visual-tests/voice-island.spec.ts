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
          await expect(fixture.locator('[data-variant="words"] .bezel-wheel')).toBeVisible();
          await expect(fixture.locator('[data-variant="words"] .overlay-island')).toHaveCSS('height', `${geometry.pillWords.collapsedH}px`);
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

type LineState = {
  top: number; bottom: number; left: number; right: number;
  opacity: number; visible: boolean; front: boolean; blur: string;
  transform: string; scrollWidth: number; clientWidth: number; text: string;
};

/** Every wheel line's projected box and effective visibility. */
async function wheelLines(page: import('@playwright/test').Page) {
  return page.locator('.bezel-wheel').evaluate((wheel) => {
    const lines = [...wheel.querySelectorAll<HTMLElement>('.bezel-wheel-line')].map((el): LineState => {
      const r = el.getBoundingClientRect();
      const cs = getComputedStyle(el);
      return {
        top: r.top, bottom: r.bottom, left: r.left, right: r.right,
        opacity: Number(cs.opacity), visible: cs.visibility === 'visible', front: el.hasAttribute('data-front'),
        blur: cs.filter, transform: cs.transform, scrollWidth: el.scrollWidth, clientWidth: el.clientWidth,
        text: el.textContent ?? '',
      };
    });
    const w = wheel.getBoundingClientRect();
    return { wheel: { top: w.top, bottom: w.bottom, left: w.left, right: w.right, height: w.height }, lines };
  });
}

for (const motion of ['no-preference', 'reduce'] as const) {
  for (const appearance of ['light', 'dark'] as const) {
    for (const [name, text] of Object.entries(WORDS)) {
      test(`floating pill wheel keeps ${name} words inside the fixed ${appearance} capsule (${motion})`, async ({ page }) => {
        await page.emulateMedia({ reducedMotion: motion, colorScheme: appearance });
        await page.setViewportSize({ width: 420, height: 200 });
        await page.goto(`/visual-fixtures.html?state=voice-island&placement=pill&variant=words&appearance=${appearance}&text=${encodeURIComponent(text)}`);
        const pill = page.locator('.overlay-island');
        const wheel = pill.locator('.bezel-wheel');
        await expect(wheel).toBeVisible();
        await page.evaluate(() => document.fonts.ready);
        // Let the word fade-ins finish so opacity and blur are at rest.
        await page.waitForTimeout(motion === 'reduce' ? 300 : 600);
        const [capsule, glyph, wave] = await Promise.all([
          pill.boundingBox(), pill.locator('.bezel-glyph').boundingBox(), pill.locator('.overlay-waveform').boundingBox(),
        ]);
        // The capsule keeps its fixed slim size whatever the text.
        expect(capsule!.width).toBe(geometry.pillWords.windowW);
        expect(capsule!.height).toBe(geometry.pillWords.collapsedH);
        const { wheel: w, lines } = await wheelLines(page);
        // Dot, waveform and the wheel sit side by side, inside the capsule.
        expect(wave!.x).toBeGreaterThanOrEqual(glyph!.x + glyph!.width);
        expect(w.left).toBeGreaterThanOrEqual(wave!.x + wave!.width);
        expect(w.right).toBeLessThanOrEqual(capsule!.x + capsule!.width - 1);
        expect(w.top).toBeGreaterThanOrEqual(capsule!.y);
        expect(w.bottom).toBeLessThanOrEqual(capsule!.y + capsule!.height);
        const shown = lines.filter(l => l.visible && l.opacity > 0.02);
        expect(lines.length).toBeLessThanOrEqual(4);
        // Every visible line stays inside the wheel and never overflows sideways
        // (a long word or CJK is cut into lines, never wider than the pill).
        for (const line of shown) {
          expect(line.left).toBeGreaterThanOrEqual(w.left - 0.5);
          expect(line.right).toBeLessThanOrEqual(w.right + 0.5);
          expect(line.top).toBeGreaterThanOrEqual(w.top - 0.5);
          expect(line.bottom).toBeLessThanOrEqual(w.bottom + 0.5);
          expect(line.scrollWidth).toBeLessThanOrEqual(line.clientWidth + 1);
        }
        // The front line is the newest words, fully lit, sharp, vertically
        // centred in the slim pill and inside the mask's opaque band (22% to 78%).
        const front = shown.filter(l => l.front);
        expect(front).toHaveLength(1);
        expect(front[0].opacity).toBe(1);
        expect(front[0].blur).toBe('none');
        expect(front[0].top).toBeGreaterThanOrEqual(w.top + 0.22 * w.height - 0.5);
        expect(front[0].bottom).toBeLessThanOrEqual(w.top + 0.78 * w.height + 0.5);
        expect(Math.abs((front[0].top + front[0].bottom) / 2 - (capsule!.y + capsule!.height / 2))).toBeLessThanOrEqual(1);
        expect(front[0].bottom - front[0].top).toBeLessThanOrEqual(16);
        expect(text.replace(/\s+/g, ' ').trim().endsWith(front[0].text.replace(/\s+/g, ' ').trim().slice(-8))).toBe(true);
        // At rest only the front line is drawn: the previous line has rolled
        // out of the top and is gone, not a blurred smudge above it.
        expect(shown.filter(l => !l.front)).toHaveLength(0);
        // No selection band or any other box behind the words.
        expect(await wheel.evaluate(el => [...el.children].every(c => c.matches('[role="status"], .bezel-wheel-line')))).toBe(true);
        await expect(page.locator('[class*="band"]')).toHaveCount(0);
        // Reduce Motion: flat lines, no 3D transform anywhere in the wheel.
        if (motion === 'reduce') {
          for (const line of lines) expect(line.transform).not.toContain('matrix3d');
          expect(await wheel.evaluate(el => el.querySelectorAll('[style*="rotate"], [style*="translateZ"]').length)).toBe(0);
        }
        // No separate words box exists anywhere.
        await expect(page.locator('.live-words-card, .live-words-window')).toHaveCount(0);
      });
    }
  }
}

test('streamed words roll finished lines up and back over the wheel', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference', colorScheme: 'dark' });
  await page.setViewportSize({ width: 420, height: 200 });
  const text = WORDS.sentence + ' and then a few more words so that it keeps on rolling';
  await page.goto(`/visual-fixtures.html?state=voice-island&placement=pill&variant=words&stream=60&text=${encodeURIComponent(text)}`);
  await expect(page.locator('.bezel-wheel')).toBeVisible();
  // Sample the roll while words arrive: the outgoing line turns through 3D
  // above the front line and is glimpsed only while it leaves.
  let rolling = false;
  for (let i = 0; i < 60 && !rolling; i++) {
    const { lines } = await wheelLines(page);
    const front = lines.find(l => l.front);
    rolling = !!front && lines.some(l => !l.front && l.visible && l.opacity > 0.02
      && l.transform.startsWith('matrix3d') && l.top < front.top);
    if (!rolling) await page.waitForTimeout(30);
  }
  expect(rolling).toBe(true);
  await page.waitForFunction(() => (window as unknown as { __wheelStream?: string }).__wheelStream === 'done');
  await page.waitForTimeout(1200);
  const { wheel: w, lines } = await wheelLines(page);
  const front = lines.filter(l => l.front);
  expect(front).toHaveLength(1);
  expect(front[0].opacity).toBe(1);
  expect(front[0].text.trim().endsWith('rolling')).toBe(true);
  expect(lines.filter(l => l.visible && l.opacity > 0.02 && !l.front)).toHaveLength(0);
  for (const line of lines.filter(l => l.visible && l.opacity > 0.02)) {
    expect(line.top).toBeGreaterThanOrEqual(w.top - 0.5);
    expect(line.bottom).toBeLessThanOrEqual(w.bottom + 0.5);
  }
});

test('the warning widens the slim pill and its phrase fits; every pill size is 34pt tall', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.setViewportSize({ width: 420, height: 320 });
  await page.goto('/visual-fixtures.html?state=voice-island&placement=pill&appearance=dark');
  const boxOf = async (variant: string) =>
    (await page.locator(`[data-variant="${variant}"] .overlay-island`).boundingBox())!;
  const words = await boxOf('words');
  const warning = await boxOf('warning');
  const compact = await boxOf('recording');
  expect([compact.width, compact.height]).toEqual([geometry.pillRecording.windowW, geometry.pillRecording.collapsedH]);
  expect([words.width, words.height]).toEqual([geometry.pillWords.windowW, geometry.pillWords.collapsedH]);
  expect([warning.width, warning.height]).toEqual([geometry.pillWarning.windowW, geometry.pillWarning.collapsedH]);
  expect([words.width, words.height]).toEqual([224, 34]);
  expect(compact.width).toBeLessThan(words.width);
  expect(words.width).toBeLessThan(warning.width);
  expect(new Set([compact.height, words.height, warning.height])).toEqual(new Set([34]));
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
  await expect(row.locator('.bezel-wheel')).toHaveCount(0);
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
