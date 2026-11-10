import { expect, test } from '@playwright/test';

for (const appearance of ['light', 'dark']) {
  for (const [width, height] of [[1100, 780], [840, 620]]) {
    test(`Notetaker state geometry ${appearance} ${width}`, async ({ page }) => {
      await page.setViewportSize({ width, height });
      for (const state of ['meetings-many', 'meetings-empty', 'meetings-recording', 'meetings-stopping', 'meetings-finishing', 'meetings-permission-denied']) {
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
        if (state === 'meetings-stopping' || state === 'meetings-finishing') {
          const label = state === 'meetings-stopping' ? 'Stopping…' : 'Finishing…';
          await expect(page.getByRole('button', { name: label, exact: true })).toBeDisabled();
          await expect(page.locator('.meetings-row[data-live="true"] .meetings-row-preview')).toContainText(label);
          await expect(page.locator('.meeting-detail-meta')).toContainText(label);
          await expect(page.locator('.meeting-listening')).toContainText(state === 'meetings-stopping' ? 'Stopping capture…' : 'Finishing pending transcript chunks…');
          await expect(page.locator('.meetings-row-preview, .meeting-detail-meta, .meeting-listening').filter({ hasText: /Recording|Listening/ })).toHaveCount(0);
        }
        if (state === 'meetings-permission-denied') {
          await expect(page.getByRole('button', { name: 'Open System Settings…' })).toBeVisible();
          await expect(page.getByRole('button', { name: 'Start meeting', exact: true })).toBeEnabled();
          await expect(page.locator('.meetings-statusbar-detail')).toHaveCount(0);
        }
      }
    });
  }

  test(`Notetaker destructive keyboard activation ${appearance}`, async ({ page }) => {
    await page.setViewportSize({ width: 1100, height: 780 });
    await page.goto(`/visual-fixtures.html?state=meetings-many&appearance=${appearance}`);
    await page.locator('.meeting-more').click();
    const deletion = page.getByRole('menuitem', { name: 'Delete meeting…', exact: true });
    await deletion.focus();
    await page.keyboard.down('Enter');
    await page.keyboard.down('Enter');
    await page.keyboard.down('Enter');
    await page.keyboard.up('Enter');
    await expect(deletion).toBeFocused();
    await expect(page.locator('html')).not.toHaveAttribute('data-meeting-deletions');
    // A held Space is one activation. Release, then press again to confirm.
    await page.keyboard.down('Space');
    await page.keyboard.down('Space');
    await page.keyboard.down('Space');
    await page.keyboard.up('Space');
    const confirm = page.getByRole('menuitem', { name: 'Confirm delete…', exact: true });
    await expect(confirm).toBeVisible();
    await page.keyboard.down('Enter');
    await page.keyboard.down('Enter');
    await page.keyboard.up('Enter');
    await expect(page.locator('html')).not.toHaveAttribute('data-meeting-deletions');
    await page.keyboard.press('Space');
    await expect(page.locator('html')).toHaveAttribute('data-meeting-deletions', '1');
  });

  for (const dismissal of ['Escape', 'outside click', 'trigger toggle', 'another action']) {
    test(`Notetaker disarms after ${dismissal} ${appearance}`, async ({ page }) => {
      await page.goto(`/visual-fixtures.html?state=meetings-many&appearance=${appearance}`);
      const more = page.locator('.meeting-more');
      await more.click();
      await page.getByRole('menuitem', { name: 'Delete meeting…', exact: true }).click();
      await expect(page.getByRole('menuitem', { name: 'Confirm delete…', exact: true })).toBeVisible();
      if (dismissal === 'Escape') await page.keyboard.press('Escape');
      if (dismissal === 'outside click') {
        // A real outside click hits Base UI's modal backdrop over the heading.
        const heading = await page.getByRole('heading', { name: 'Notetaker', exact: true }).boundingBox();
        await page.mouse.click(heading!.x + heading!.width / 2, heading!.y + heading!.height / 2);
      }
      if (dismissal === 'trigger toggle') await more.click();
      if (dismissal === 'another action') await page.getByRole('menuitem', { name: 'Edit speakers…', exact: true }).click();
      await expect(more).toHaveAttribute('aria-expanded', 'false');
      await more.click();
      // Check before the four-second arm timeout can hide a stale confirmation.
      await expect(page.getByRole('menuitem', { name: 'Delete meeting…', exact: true })).toBeVisible({ timeout: 500 });
      await page.getByRole('menuitem', { name: 'Delete meeting…', exact: true }).click();
      await expect(page.getByRole('menuitem', { name: 'Confirm delete…', exact: true })).toBeVisible();
      await expect(page.locator('html')).not.toHaveAttribute('data-meeting-deletions');
    });
  }

  for (const movement of ['keyboard', 'pointer']) {
    test(`Notetaker disarms on ${movement} item movement ${appearance}`, async ({ page }) => {
      await page.goto(`/visual-fixtures.html?state=meetings-many&appearance=${appearance}`);
      await page.locator('.meeting-more').click();
      await page.getByRole('menuitem', { name: 'Delete meeting…', exact: true }).click();
      const armed = page.getByRole('menuitem', { name: 'Confirm delete…', exact: true });
      await expect(armed).toBeVisible();
      if (movement === 'keyboard') {
        await armed.focus();
        await page.keyboard.press('ArrowUp');
        await expect(page.getByRole('menuitem', { name: 'Copy as', exact: true })).toBeFocused();
        await page.keyboard.press('ArrowDown');
        await page.keyboard.press('Space');
      } else {
        await page.getByRole('menuitem', { name: 'Edit speakers…', exact: true }).hover();
        await page.getByRole('menuitem', { name: 'Delete meeting…', exact: true }).click();
      }
      await expect(page.getByRole('menuitem', { name: 'Confirm delete…', exact: true })).toBeVisible();
      await expect(page.locator('html')).not.toHaveAttribute('data-meeting-deletions');
    });
  }

  test(`Notetaker double-click only arms ${appearance}`, async ({ page }) => {
    await page.goto(`/visual-fixtures.html?state=meetings-many&appearance=${appearance}`);
    await page.locator('.meeting-more').click();
    await page.getByRole('menuitem', { name: 'Delete meeting…', exact: true }).dblclick();
    const confirm = page.getByRole('menuitem', { name: 'Confirm delete…', exact: true });
    await expect(confirm).toBeVisible();
    await expect(page.locator('html')).not.toHaveAttribute('data-meeting-deletions');
    // A new single-click sequence (detail 1), after the system multi-click window, confirms.
    await page.waitForTimeout(600);
    await confirm.click();
    await expect(page.locator('html')).toHaveAttribute('data-meeting-deletions', '1');
  });

  test(`Notetaker empty Calendar lookup focus ${appearance}`, async ({ page }) => {
    await page.goto(`/visual-fixtures.html?state=meetings-many&appearance=${appearance}&calendar=empty&calendarDelay=2000`);
    const more = page.locator('.meeting-more');
    await more.click();
    await page.getByRole('menuitem', { name: 'Name from calendar…', exact: true }).click();
    await expect(more).toBeFocused();
    await expect(page.getByRole('button', { name: 'Cancel calendar lookup', exact: true })).toBeFocused();
    await expect(page.getByRole('button', { name: 'Apply event', exact: true })).toBeDisabled();
  });

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
