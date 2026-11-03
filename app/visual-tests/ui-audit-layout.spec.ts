import { expect, test } from '@playwright/test';
import { STREAMS } from '../src/lib/events';

// Production components with synthetic IPC. Native placement/focus and durable
// review persistence are separate bundled-app checks, not proven by this suite.
for (const appearance of ['light', 'dark']) {
  for (const width of [720, 880, 1120]) {
    test(`meeting action editing fits its pane at ${width}px (${appearance})`, async ({ page }, testInfo) => {
      await page.setViewportSize({ width, height: width === 720 ? 560 : 820 });
      await page.goto(`/visual-fixtures.html?state=meetings-review&appearance=${appearance}`);
      const evidence = page.locator('.meeting-transcript-row');
      await expect(evidence.first()).toBeVisible();
      const canonical = await evidence.allTextContents();
      await page.getByRole('button', { name: 'Edit review…', exact: true }).click();
      const task = page.getByRole('textbox', { name: 'Action item 1', exact: true });
      const owner = page.getByRole('textbox', { name: 'Action owner 1', exact: true });
      const date = page.getByLabel('Action due date 1', { exact: true });
      await task.scrollIntoViewIfNeeded();
      const layout = await task.evaluate(element => {
        const editor = element.closest('.meeting-action-editor')!;
        const taskRect = element.getBoundingClientRect();
        const editorRect = editor.getBoundingClientRect();
        return { taskWidth: taskRect.width, editorWidth: editorRect.width, overflow: editor.scrollWidth - editor.clientWidth };
      });
      expect(layout.taskWidth).toBeGreaterThan(160);
      expect(layout.taskWidth).toBeGreaterThan(layout.editorWidth * 0.85);
      expect(layout.overflow).toBeLessThanOrEqual(1);
      for (const field of [owner, date, page.getByRole('button', { name: 'Remove action item 1', exact: true })]) {
        await field.scrollIntoViewIfNeeded();
        await expect(field).toBeInViewport();
        const box = await field.boundingBox();
        expect(box!.width).toBeGreaterThan(35);
        expect(box!.x).toBeGreaterThanOrEqual(0);
        expect(box!.x + box!.width).toBeLessThanOrEqual(width);
      }
      await task.fill('Draft task that will be cancelled');
      await owner.fill('Casey');
      await date.fill('2026-11-06');
      await page.getByRole('button', { name: 'Cancel', exact: true }).click();
      await page.getByRole('button', { name: 'Edit review…', exact: true }).click();
      await expect(task).toHaveValue('Verify every export format.');
      await expect(owner).toHaveValue('Alex');
      await expect(date).toHaveValue('');
      await task.fill('Verify the reviewed plan and all milestones');
      await owner.fill('Casey');
      await date.fill('2026-11-07');
      await task.scrollIntoViewIfNeeded();
      if (width !== 1120) await page.screenshot({ path: testInfo.outputPath('meeting-action-editor.png') });
      await page.getByRole('button', { name: 'Save review', exact: true }).click();
      await expect(page.getByRole('form', { name: 'Edit meeting review' })).toHaveCount(0);
      expect(await evidence.allTextContents()).toEqual(canonical);
    });

    test(`Insights calendar keeps dates, values, and bounded proportions at ${width}px (${appearance})`, async ({ page }, testInfo) => {
      await page.setViewportSize({ width, height: width === 720 ? 560 : 820 });
      await page.goto(`/visual-fixtures.html?state=insights&appearance=${appearance}`);
      const chart = page.getByRole('figure', { name: 'Words per day heatmap', exact: true });
      const grid = chart.locator('.ui-day-chart-heatmap');
      await grid.scrollIntoViewIfNeeded();
      await expect(chart.locator('.ui-day-chart-date-range')).toContainText(/\d{4}/);
      await expect(chart.locator('.ui-day-chart-weekdays span')).toHaveCount(7);
      await expect(grid.getByRole('button')).toHaveCount(56);
      const bounds = await chart.evaluate(element => {
        const plot = element.querySelector('.ui-day-chart-heatmap')!.getBoundingClientRect();
        const calendar = element.querySelector('.ui-day-chart-calendar')!.getBoundingClientRect();
        const cell = element.querySelector('.ui-day-chart-heatmap button')!.getBoundingClientRect();
        return { height: plot.height, width: plot.width, cellWidth: cell.width, calendarRight: calendar.right, overflow: element.scrollWidth - element.clientWidth };
      });
      expect(bounds.width).toBeLessThanOrEqual(256);
      expect(bounds.height).toBeLessThanOrEqual(224);
      expect(bounds.cellWidth).toBeGreaterThanOrEqual(16);
      expect(bounds.calendarRight).toBeLessThanOrEqual(width);
      expect(bounds.overflow).toBeLessThanOrEqual(1);
      const cell = grid.getByRole('button', { name: /760 words, 6 recordings/ });
      await cell.focus();
      await expect(chart.getByRole('status')).toContainText('760 words · 6 recordings');
      await page.keyboard.press('Escape');
      await expect(chart.getByRole('status')).toContainText('Focus a day');
      if (width !== 1120) await page.screenshot({ path: testInfo.outputPath('insights-calendar.png') });
      await chart.getByText('Daily values', { exact: true }).click();
      await expect(chart.getByRole('table')).toBeVisible();
      await expect(chart.locator('tbody tr')).toHaveCount(56);
      await expect(chart.locator('tbody')).toContainText('760');
    });
  }

  test(`diagnostics stream filters wrap and remain pointer/keyboard reachable (${appearance})`, async ({ page }) => {
    await page.setViewportSize({ width: 720, height: 560 });
    await page.goto(`/visual-fixtures.html?state=settings-folding&page=diagnostics&appearance=${appearance}`);
    const tabs = page.getByRole('tablist', { name: 'Diagnostics views' });
    await expect(tabs).toBeVisible();
    const workspace = tabs.locator('..').locator('..').locator('..');
    const bounds = await workspace.boundingBox();
    for (const stream of STREAMS) {
      const chip = workspace.getByRole('button', { name: stream, exact: true });
      await expect(chip).toBeInViewport();
      const box = await chip.boundingBox();
      expect(box!.x).toBeGreaterThanOrEqual(bounds!.x);
      expect(box!.x + box!.width).toBeLessThanOrEqual(bounds!.x + bounds!.width + 1);
      const pressed = await chip.getAttribute('aria-pressed');
      await chip.click();
      await expect(chip).toHaveAttribute('aria-pressed', pressed === 'true' ? 'false' : 'true');
      await chip.focus();
      await page.keyboard.press('Space');
      await expect(chip).toHaveAttribute('aria-pressed', pressed!);
    }
    await tabs.getByRole('tab').last().click();
    await expect(tabs.getByRole('tab').last()).toHaveAttribute('aria-selected', 'true');
  });

  test(`Home setup copy stays source-owned and fully readable (${appearance})`, async ({ page }) => {
    for (const width of [720, 880]) {
      await page.setViewportSize({ width, height: width === 720 ? 560 : 720 });
      await page.goto(`/visual-fixtures.html?state=idle&appearance=${appearance}`);
      const checklist = page.locator('.discovery-checklist');
      await expect(checklist).toBeVisible();
      await expect(checklist.locator('li')).toHaveCount(7);
      const labels = checklist.locator('strong, small');
      for (const label of await labels.all()) {
        const fit = await label.evaluate(element => ({
          overflow: element.scrollWidth - element.clientWidth,
          ellipsis: getComputedStyle(element).textOverflow,
          before: getComputedStyle(element, '::before').content,
          after: getComputedStyle(element, '::after').content,
        }));
        expect(fit.overflow).toBeLessThanOrEqual(1);
        expect(fit.ellipsis).not.toBe('ellipsis');
        expect(['none', 'normal']).toContain(fit.before);
        expect(['none', 'normal']).toContain(fit.after);
      }
      for (const row of await checklist.locator('li').all()) {
        const action = row.getByRole('button');
        await action.scrollIntoViewIfNeeded();
        const copyBounds = await row.locator('small').boundingBox();
        const actionBounds = await action.boundingBox();
        expect(copyBounds!.x + copyBounds!.width).toBeLessThanOrEqual(actionBounds!.x);
        await expect(action).toBeInViewport();
      }
      await checklist.getByRole('button', { name: 'Dismiss Get more from Murmur' }).click();
      // The fixture's dismiss callback is a no-op; component coverage verifies
      // callback delivery, while this check establishes the pointer target fits.
    }
  });
}

for (const geometry of ['notched', 'external', 'fallback', 'pill']) {
  test(`supported ${geometry} overlay reserves status separately from Mode and quick actions`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    for (const state of ['idle', 'starting', 'still-connecting', 'recording', 'recovering']) {
      const placement = geometry === 'pill' ? 'pill' : 'notch';
      await page.goto(`/visual-fixtures.html?state=voice-island&variant=expanded&placement=${placement}&display=${geometry}&controls-state=${state}`);
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
    }
  });
}
