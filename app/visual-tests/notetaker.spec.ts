import { expect, test } from '@playwright/test';

for (const appearance of ['light', 'dark']) {
  for (const [width, height] of [[1100, 780], [840, 620]]) {
    test(`Notetaker state geometry ${appearance} ${width}`, async ({ page }) => {
      await page.setViewportSize({ width, height });
      for (const state of ['meetings-many', 'meetings-empty', 'meetings-recording', 'meetings-permission-denied']) {
        await page.goto(`/visual-fixtures.html?state=${state}&appearance=${appearance}`);
        await expect(page.getByRole('heading', { name: 'Notetaker', exact: true })).toBeVisible();
        await expect(page.locator('.meetings-statusbar')).toHaveCSS('height', width === 840 ? '75px' : '57px');
        const overflow = await page.locator('.meetings-panel').evaluate((panel) => [...panel.querySelectorAll('.meetings-list,.meeting-workspace'), panel].some((element) => element.scrollWidth > element.clientWidth + 1));
        expect(overflow).toBe(false);
        const status = await page.locator('.meetings-statusbar').boundingBox();
        const phase = await page.locator('.meetings-phase').boundingBox();
        expect(phase!.y).toBeGreaterThanOrEqual(status!.y);
        expect(phase!.y + phase!.height).toBeLessThanOrEqual(status!.y + status!.height);
        if (state === 'meetings-empty') {
          await expect(page.getByRole('button', { name: 'Start meeting', exact: true })).toHaveCount(1);
          await expect(page.getByRole('heading', { name: 'Record your first meeting' })).toBeVisible();
          await expect(page.locator('.meetings-split')).toHaveCount(0);
        } else {
          const actions = await page.locator('.meeting-header-actions').boundingBox();
          const transcript = await page.locator('.meeting-transcript').boundingBox();
          expect(actions!.x + actions!.width).toBeCloseTo(transcript!.x + transcript!.width, 1);
        }
        if (state === 'meetings-recording') {
          await expect(page.getByRole('button', { name: 'Stop meeting', exact: true })).toBeVisible();
          await expect(page.locator('.meeting-more')).toHaveCount(0);
          await expect(page.locator('.meeting-transcript-row')).toHaveCount(3);
        }
        if (state === 'meetings-permission-denied') {
          await expect(page.getByRole('button', { name: 'Open System Settings…' })).toBeVisible();
          await expect(page.getByRole('button', { name: 'Start meeting', exact: true })).toBeEnabled();
          await expect(page.locator('.meetings-statusbar-detail')).toHaveCount(0);
        }
      }
    });
  }

  test(`Notetaker play affordance and keyboard menus ${appearance}`, async ({ page }) => {
    await page.setViewportSize({ width: 1100, height: 780 });
    await page.goto(`/visual-fixtures.html?state=meetings-many&appearance=${appearance}`);
    const play = page.locator('.meeting-transcript-time[data-playable]').first();
    const glyph = play.locator('.meeting-segment-play');
    const resting = await play.boundingBox();
    await expect(glyph).toBeHidden();
    await play.hover();
    await expect(glyph).toBeVisible();
    expect((await play.boundingBox())!.x).toBe(resting!.x);
    await page.mouse.move(0, 0);
    await play.focus();
    await expect(glyph).toBeVisible();
    expect((await play.boundingBox())!.x).toBe(resting!.x);
    await page.locator('.meeting-more').focus();
    await page.keyboard.press('Enter');
    await page.getByRole('menuitem', { name: 'Copy as', exact: true }).focus();
    await page.keyboard.press('ArrowRight');
    await expect(page.getByRole('menuitem', { name: 'JSON', exact: true })).toBeVisible();
    await page.keyboard.press('Escape');
    await page.keyboard.press('Escape');
    await expect(page.locator('.meeting-more')).toBeFocused();
    await page.locator('.meeting-more').click();
    await page.getByRole('menuitem', { name: 'Edit speakers…', exact: true }).click();
    await expect(page.getByLabel('Me speaker label')).toBeFocused();
    const panel = await page.locator('.meeting-speakers-panel').boundingBox();
    const detail = await page.locator('.meetings-detail-column').boundingBox();
    expect(panel!.x).toBeGreaterThanOrEqual(detail!.x);
    expect(panel!.x + panel!.width).toBeLessThanOrEqual(detail!.x + detail!.width);
  });
}
