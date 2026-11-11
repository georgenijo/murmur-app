import { expect, test, type Page } from '@playwright/test';

/** Geometry contracts for the Settings polish pass: the speaker-labels rhythm,
 *  one chevron size on one right-edge axis, and a collapse that never jumps.
 *  No screenshots: every check reads rendered boxes. */
test.use({ viewport: { width: 1180, height: 900 } });

async function open(page: Page, pane: string, reducedMotion: 'reduce' | 'no-preference' = 'reduce', appearance = 'light') {
  await page.emulateMedia({ reducedMotion });
  await page.goto(`/visual-fixtures.html?state=settings&appearance=${appearance}`);
  await expect(page.locator('[data-visual-ready="true"]')).toBeVisible();
  await page.locator('.settings-sidebar').getByRole('button', { name: pane, exact: true }).click();
  await page.evaluate(() => document.fonts.ready);
}

test.describe('speaker labels rhythm', () => {
  for (const appearance of ['light', 'dark']) {
    test(`${appearance} disclosure follows the switch row with the normal row gap`, async ({ page }) => {
      await open(page, 'Meetings', 'reduce', appearance);
      const group = page.getByRole('heading', { name: 'Speaker labels', exact: true }).locator('..');
      const card = group.locator('.settings-card');
      const switchRow = group.locator('[data-setting-target="meeting-speakers"]');
      const toggle = group.getByRole('switch', { name: 'Label remote speakers' });
      const disclosure = group.getByRole('button', { name: 'How speaker labeling works' });
      const model = group.locator('.settings-setting-row', { hasText: 'Remote speaker model' });
      const box = async (l: ReturnType<Page['locator']>) => (await l.boundingBox())!;

      // Off: the branch reserves no height at all.
      const off = await box(card);
      expect(off.height).toBeLessThanOrEqual((await box(switchRow)).height + 2);

      await toggle.click();
      await expect(model).toBeVisible();
      for (const expanded of [false, true]) {
        if (expanded) {
          await disclosure.click();
          await expect(group.getByText(/temporarily saves system audio/)).toBeVisible();
        }
        const [s, d, m] = [await box(switchRow), await box(disclosure), await box(model)];
        const gapAbove = d.y - (s.y + s.height);
        // Neighbouring rows in a card touch, separated only by a 1px divider.
        expect(gapAbove, `switch row to disclosure (expanded=${expanded})`).toBeLessThanOrEqual(2);
        expect(gapAbove).toBeGreaterThanOrEqual(0);
        if (!expanded) {
          const gapBelow = m.y - (d.y + d.height);
          expect(Math.abs(gapAbove - gapBelow), 'same gap above and below the disclosure').toBeLessThanOrEqual(2);
        }
      }
    });
  }
});

test('Calendar details follows its switch row with the normal row gap', async ({ page }) => {
  await open(page, 'Meetings');
  const row = page.locator('[data-setting-target="meeting-suggestions"]');
  const toggle = row.getByRole('switch');
  if ((await toggle.getAttribute('aria-checked')) !== 'true') await toggle.click();
  const disclosure = page.getByRole('button', { name: 'Calendar details' });
  await expect(disclosure).toBeVisible();
  const [r, d] = [(await row.boundingBox())!, (await disclosure.boundingBox())!];
  expect(d.y - (r.y + r.height)).toBeLessThanOrEqual(2);
  expect(d.y - (r.y + r.height)).toBeGreaterThanOrEqual(0);
});

test.describe('settings chevrons', () => {
  for (const pane of ['General', 'Recording', 'Delivery', 'Meetings', 'Text & vocabulary', 'AI & models']) {
    test(`${pane} chevrons share one size and one right edge`, async ({ page }) => {
      await open(page, pane);
      for (let pass = 0; pass < 2; pass++) {
        for (const button of await page.locator('.settings-advanced-toggle[aria-expanded=false]').all()) {
          if (await button.isVisible()) await button.click();
        }
      }
      // The trailing mark of every navigation row and every Advanced toggle,
      // whatever element draws it, so a text glyph cannot hide from the check.
      const chevrons = await page.locator('[data-testid="settings-content"] :is(button.settings-setting-row, .settings-advanced-toggle) > :last-child:not(.settings-advanced-copy)').evaluateAll(els => els
        .filter(el => el.getBoundingClientRect().width > 0)
        .map(el => { const r = el.getBoundingClientRect(); return { w: r.width, h: r.height, right: r.right, card: el.closest('.settings-card')?.getBoundingClientRect().right ?? 0 }; }));
      expect(chevrons.length).toBeGreaterThan(0);
      for (const c of chevrons) {
        expect(Math.abs(c.w - 16)).toBeLessThanOrEqual(.5);
        expect(Math.abs(c.h - 16)).toBeLessThanOrEqual(.5);
        // The same inset from the card edge puts every chevron on one axis.
        expect(Math.abs((c.card - c.right) - (chevrons[0].card - chevrons[0].right))).toBeLessThanOrEqual(.5);
      }
      // No text-glyph chevron is left in a navigation row.
      await expect(page.locator('[data-testid="settings-content"] button.settings-setting-row', { hasText: '›' })).toHaveCount(0);
    });
  }
});

test.describe('Advanced collapse', () => {
  for (const pane of ['Delivery', 'Meetings', 'Text & vocabulary']) {
    test(`${pane} collapse is smooth from the first frame`, async ({ page }) => {
      await open(page, pane, 'no-preference');
      const toggle = page.getByRole('button', { name: /^Advanced/ }).first();
      await toggle.click();
      await expect(toggle).toHaveAttribute('aria-expanded', 'true');
      await page.waitForTimeout(700);
      const frames = await toggle.evaluate(async (el: HTMLElement) => {
        const adv = el.closest('.settings-advanced')!;
        const branch = document.getElementById(el.getAttribute('aria-controls')!)!;
        const row = branch.querySelector<HTMLElement>('.settings-setting-row, button')!;
        const out: { t: number; height: number; branch: number; rowTop: number; gap: number }[] = [];
        const start = performance.now();
        const read = () => {
          const r = row.isConnected ? row.getBoundingClientRect() : null;
          out.push({ t: performance.now() - start, height: adv.getBoundingClientRect().height,
            branch: branch.getBoundingClientRect().height, rowTop: r?.top ?? NaN, gap: r ? r.top - el.getBoundingClientRect().bottom : NaN });
        };
        read();
        el.click();
        await Promise.resolve();
        await new Promise<void>(resolve => {
          const frame = () => { read(); if (performance.now() - start < 700) requestAnimationFrame(frame); else resolve(); };
          requestAnimationFrame(frame);
        });
        return out;
      });
      await expect(toggle).toHaveAttribute('aria-expanded', 'false');
      const mounted = frames.filter(f => Number.isFinite(f.rowTop) && f.branch > 0);
      expect(mounted.length).toBeGreaterThan(3);
      // Animated, not snapped: several intermediate heights.
      expect(frames.filter(f => f.height < frames[0].height - 1 && f.height > frames.at(-1)!.height + 1).length).toBeGreaterThan(2);
      for (let i = 1; i < frames.length; i++) {
        // The disclosure only ever shrinks, and the row after it never moves back down.
        expect(frames[i].height, `height at frame ${i}`).toBeLessThanOrEqual(frames[i - 1].height + .1);
        if (Number.isFinite(frames[i].rowTop) && Number.isFinite(frames[i - 1].rowTop)) {
          expect(frames[i].rowTop, `row top at frame ${i}`).toBeLessThanOrEqual(frames[i - 1].rowTop + .1);
        }
      }
      // The row's offset below the toggle holds until the fold clips it away;
      // a snapping inset would show as the row jumping before the fold moves.
      for (const f of mounted) expect(Math.abs(f.gap - mounted[0].gap), `gap at ${f.t.toFixed(0)}ms`).toBeLessThanOrEqual(.5);
      // The very first painted frame has not yet lost more than a pixel or two.
      expect(frames[0].height - frames[1].height, 'first frame drop').toBeLessThanOrEqual(2);
    });

    test(`${pane} reduced motion still collapses instantly`, async ({ page }) => {
      await open(page, pane, 'reduce');
      const toggle = page.getByRole('button', { name: /^Advanced/ }).first();
      await toggle.click();
      await expect(toggle).toHaveAttribute('aria-expanded', 'true');
      const closedHeight = await toggle.evaluate(async (el: HTMLElement) => {
        const branch = document.getElementById(el.getAttribute('aria-controls')!)!;
        el.click();
        await new Promise(requestAnimationFrame);
        await new Promise(requestAnimationFrame);
        return branch.getBoundingClientRect().height;
      });
      expect(closedHeight).toBe(0);
    });
  }
});
