import { expect, test } from '@playwright/test';

for (const appearance of ['light', 'dark'] as const) {
  for (const viewport of [{ width: 1100, height: 780 }, { width: 840, height: 620 }]) {
    test(`Insights production states: ${appearance} ${viewport.width}`, async ({ page }) => {
      await page.clock.setFixedTime(new Date('2026-09-04T20:00:00Z'));
      await page.setViewportSize(viewport);
      await page.goto(`/visual-fixtures.html?state=insights&appearance=${appearance}`);
      const view = page.locator('.insights-view');
      const words = page.getByRole('button', { name: 'Words', exact: true });
      const speed = page.getByRole('button', { name: 'Speed', exact: true });
      const plot = view.locator('.ui-day-chart-plot');
      await expect(plot).toHaveAttribute('data-axis-min', '0');
      await expect(plot).toHaveAttribute('data-axis-max', '1000');
      await speed.click();
      await expect(speed).toHaveAttribute('aria-pressed', 'true');
      await expect(plot).toHaveAttribute('data-axis-min', '100');
      await expect(plot).toHaveAttribute('data-axis-max', '200');
      const first = view.locator('.ui-day-chart-line-targets button').first();
      await first.focus();
      await expect(view.locator('.insights-daily [role="status"]')).toContainText('140 WPM');
      await expect(first.locator('span')).toHaveAttribute('style', /60%/);
      await page.keyboard.press('Escape');
      await expect(view.locator('.insights-daily [role="status"]')).toContainText('Focus a day');
      await words.click();
      await expect(view.getByRole('table')).toHaveCount(2);
      await view.locator('summary').click();
      await expect(view.locator('details')).toHaveAttribute('open', '');
      const layout = await view.evaluate(element => {
        const card = element.querySelector('.insights-daily')!.getBoundingClientRect();
        const details = element.querySelector('[data-stats="details"]')!.getBoundingClientRect();
        return { chartWidth: card.width, detailsWidth: details.width, left: card.left - details.left,
          overflow: element.scrollWidth - element.clientWidth };
      });
      expect(layout.detailsWidth).toBeCloseTo(layout.chartWidth, 1);
      expect(layout.left).toBe(0);
      expect(layout.overflow).toBe(0);
      await view.locator('[data-analytics="activity"]').scrollIntoViewIfNeeded();
      await expect(view.locator('.ui-day-chart-heatmap')).toBeInViewport();

      await page.goto(`/visual-fixtures.html?state=insights-gap&appearance=${appearance}`);
      await expect(view.locator('.ui-day-chart-line-targets button')).toHaveCount(7);
      await expect(view.locator('.ui-day-chart-line-targets span')).toHaveCount(6);
      await expect(view.locator('polyline')).toHaveCount(2);
      expect(await view.locator('polyline').evaluateAll(lines => lines.map(line => line.getAttribute('points')!.split(' ').length))).toEqual([3, 3]);

      await page.goto(`/visual-fixtures.html?state=insights-no-speed&appearance=${appearance}`);
      await expect(view.locator('.insights-empty-message')).toHaveText('No speed data for these days.');
      await expect(view.locator('.ui-day-chart-line-targets button')).toHaveCount(7);
      await expect(view.locator('polyline, .ui-day-chart-line-targets span')).toHaveCount(0);
      await view.locator('.ui-day-chart-line-targets button').first().focus();
      await expect(view.locator('.insights-daily [role="status"]')).toContainText('No WPM data · 3 recordings');

      await page.goto(`/visual-fixtures.html?state=insights-empty&appearance=${appearance}`);
      await expect(view.locator('.insights-empty-message')).toHaveText('Dictate something and your activity will show up here.');
      await expect(view.locator('.ui-day-chart-plot button, .ui-day-chart-plot svg')).toHaveCount(0);
      await expect(view.locator('.ui-day-chart-heatmap button[data-intensity="0"]')).toHaveCount(56);
      await expect(view.locator('.insights-more-statistics, .insights-streak')).toHaveCount(0);
      await speed.click();
      await expect(view.getByRole('figure', { name: /trend line: no dictations yet/ })).toHaveCount(1);
    });
  }

  for (const sidebarWidth of [256, 208]) {
    test(`Insights calendar containment: ${appearance}, ${sidebarWidth}px sidebar`, async ({ page }) => {
      await page.clock.setFixedTime(new Date('2026-09-04T20:00:00Z'));
      await page.goto(`/visual-fixtures.html?state=insights-more&appearance=${appearance}`);
      // Exercise both current and upcoming Home sidebar geometry without owning
      // its implementation. Standard rendering captures use the real sidebar.
      await page.evaluate(width => {
        (document.querySelector('.main-app-shell') as HTMLElement).style.setProperty('--home-sidebar-width', `${width}px`);
        (document.querySelector('.home-sidebar') as HTMLElement).style.width = `${width}px`;
      }, sidebarWidth);
      for (const width of [951, 1000]) {
        await page.setViewportSize({ width, height: 780 });
        const bounds = await page.locator('[data-analytics="activity"]').evaluate(card => {
          const rect = card.getBoundingClientRect();
          const style = getComputedStyle(card);
          const calendar = card.querySelector('.ui-day-chart-heatmap')!.getBoundingClientRect();
          const daily = document.querySelector('.insights-daily')!.getBoundingClientRect();
          const details = document.querySelector('[data-stats="details"]')!.getBoundingClientRect();
          return { calendarLeft: calendar.left, calendarRight: calendar.right,
            innerRight: rect.right - parseFloat(style.borderRightWidth) - parseFloat(style.paddingRight),
            cardLeft: rect.left, sidebar: document.querySelector('.home-sidebar')!.getBoundingClientRect().width,
            dailyRight: daily.right, detailsRight: details.right };
        });
        expect(bounds.sidebar).toBe(sidebarWidth);
        expect(bounds.calendarRight).toBeLessThanOrEqual(bounds.innerRight + 0.01);
        expect(bounds.calendarLeft).toBeGreaterThan(bounds.cardLeft);
        expect(bounds.detailsRight).toBeCloseTo(bounds.dailyRight, 1);
      }
    });
  }
}

for (const appearance of ['light', 'dark'] as const) {
  for (const viewport of [{ width: 1100, height: 780 }, { width: 840, height: 620 }]) {
    test(`Insights daily word goal: ${appearance} ${viewport.width}`, async ({ page }) => {
      await page.clock.setFixedTime(new Date('2026-09-04T20:00:00Z'));
      await page.setViewportSize(viewport);
      await page.goto(`/visual-fixtures.html?state=insights&appearance=${appearance}`);
      const view = page.locator('.insights-view');
      const card = view.locator('.insights-goal');
      await expect(card).toHaveAttribute('data-goal-state', 'off');
      await expect(card.getByRole('progressbar')).toHaveCount(0);

      await page.goto(`/visual-fixtures.html?state=insights-goal&appearance=${appearance}`);
      const bar = card.getByRole('progressbar');
      await expect(bar).toHaveAttribute('aria-valuetext', '760 of 1,000 words today');
      await expect(card.locator('.insights-goal-today')).toHaveText('760 of 1,000 words today · 240 to go');
      const layout = await view.evaluate(element => {
        const headline = element.querySelector('[data-stats="headline"]')!.getBoundingClientRect();
        const goal = element.querySelector('.insights-goal')!.getBoundingClientRect();
        const fill = element.querySelector('.insights-goal-bar > span')!.getBoundingClientRect();
        const track = element.querySelector('.insights-goal-bar')!.getBoundingClientRect();
        const sections = element.querySelector('.usage-dashboard-sections')!.getBoundingClientRect();
        return { headlineLeft: headline.left, headlineRight: headline.right, goalLeft: goal.left, goalRight: goal.right,
          goalBottom: goal.bottom, headlineBottom: headline.bottom, sectionsTop: sections.top,
          titleRight: element.querySelector('.insights-goal-header h2')!.getBoundingClientRect().right,
          pickerLeft: element.querySelector('.insights-goal-header .ui-select')!.getBoundingClientRect().left,
          pickerWidth: element.querySelector('.insights-goal-header .ui-select')!.getBoundingClientRect().width,
          fillRatio: fill.width / track.width, overflow: element.scrollWidth - element.clientWidth };
      });
      expect(layout.goalLeft).toBeCloseTo(layout.headlineLeft, 1);
      expect(layout.goalRight).toBeCloseTo(layout.headlineRight, 1);
      expect(layout.goalBottom).toBeLessThanOrEqual(layout.sectionsTop);
      expect(layout.headlineBottom).toBeLessThanOrEqual(layout.goalBottom);
      expect(layout.fillRatio).toBeCloseTo(0.76, 2);
      expect(layout.overflow).toBe(0);
      expect(layout.titleRight).toBeLessThanOrEqual(layout.pickerLeft);
      expect(layout.pickerWidth).toBe(148);

      // Lowering the goal below today's words marks it met without a reload.
      await card.getByRole('combobox', { name: 'Daily word goal' }).click();
      await page.getByRole('option', { name: '500 words' }).click();
      await expect(card).toHaveAttribute('data-goal-state', 'met');
      await expect(card.locator('.insights-goal-today')).toHaveText('Goal met · 760 words today');
    });
  }
}
