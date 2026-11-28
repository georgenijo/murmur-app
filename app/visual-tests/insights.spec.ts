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

for (const appearance of ['light', 'dark'] as const) {
  for (const viewport of [{ width: 1100, height: 780 }, { width: 840, height: 620 }]) {
    test(`Insights personal bests: ${appearance} ${viewport.width}`, async ({ page }) => {
      await page.clock.setFixedTime(new Date('2026-09-04T20:00:00Z'));
      await page.setViewportSize(viewport);
      await page.goto(`/visual-fixtures.html?state=insights&appearance=${appearance}`);
      const view = page.locator('.insights-view');
      const card = view.locator('.insights-bests');
      await expect(card).toHaveAttribute('data-bests-state', 'off');
      await expect(card.locator('dl')).toHaveCount(0);

      await page.goto(`/visual-fixtures.html?state=insights-bests&appearance=${appearance}`);
      await expect(card).toHaveAttribute('data-bests-state', 'on');
      await card.scrollIntoViewIfNeeded();
      await expect(card.locator('[data-best="words-day"] strong')).toHaveText('900 words');
      await expect(card.locator('[data-best="dictations-day"] strong')).toHaveText('7 dictations');
      await expect(card.locator('[data-best="streak"] strong')).toHaveText('7 days');
      const layout = await view.evaluate(element => {
        const rect = (selector: string) => element.querySelector(selector)!.getBoundingClientRect();
        const headline = rect('[data-stats="headline"]');
        const bests = rect('.insights-bests');
        const sections = rect('.usage-dashboard-sections');
        const cells = Array.from(element.querySelectorAll('.insights-bests-grid > div')).map(cell => {
          const box = cell.getBoundingClientRect();
          const dt = cell.querySelector('dt')!.getBoundingClientRect();
          const dd = cell.querySelector('dd')!.getBoundingClientRect();
          return { left: box.left, top: box.top, dtRight: dt.right, ddLeft: dd.left, ddRight: dd.right, right: box.right };
        });
        return { headlineLeft: headline.left, headlineRight: headline.right, bestsLeft: bests.left, bestsRight: bests.right,
          bestsTop: bests.top, sectionsBottom: sections.bottom, cells,
          overflow: element.scrollWidth - element.clientWidth };
      });
      expect(layout.bestsLeft).toBeCloseTo(layout.headlineLeft, 1);
      expect(layout.bestsRight).toBeCloseTo(layout.headlineRight, 1);
      // Below the charts, so it never shifts the chart under the pointer.
      expect(layout.sectionsBottom).toBeLessThanOrEqual(layout.bestsTop);
      expect(layout.overflow).toBe(0);
      // Two columns, two rows, and no label running into its value.
      expect(layout.cells).toHaveLength(4);
      expect(layout.cells[0].top).toBeCloseTo(layout.cells[1].top, 1);
      expect(layout.cells[1].left).toBeGreaterThan(layout.cells[0].right);
      expect(layout.cells[2].top).toBeGreaterThan(layout.cells[0].top);
      for (const cell of layout.cells) {
        expect(cell.dtRight).toBeLessThanOrEqual(cell.ddLeft);
        expect(cell.ddRight).toBeLessThanOrEqual(cell.right + 0.5);
      }

      // Turning it off without a reload hides the figures again.
      await card.getByRole('switch', { name: 'Show personal bests' }).click();
      await expect(card).toHaveAttribute('data-bests-state', 'off');
    });
  }
}

for (const appearance of ['light', 'dark'] as const) {
  for (const viewport of [{ width: 1100, height: 780 }, { width: 840, height: 620 }]) {
    test(`Insights time of day: ${appearance} ${viewport.width}`, async ({ page }) => {
      await page.setViewportSize(viewport);
      await page.goto(`/visual-fixtures.html?state=insights&appearance=${appearance}`);
      const view = page.locator('.insights-view');
      const card = view.locator('.insights-hours');
      await expect(card).toHaveAttribute('data-hours-state', 'off');
      await expect(card.locator('.insights-hours-bars')).toHaveCount(0);

      await page.goto(`/visual-fixtures.html?state=insights-hours-empty&appearance=${appearance}`);
      await expect(card).toHaveAttribute('data-hours-state', 'empty');

      await page.goto(`/visual-fixtures.html?state=insights-hours&appearance=${appearance}`);
      await expect(card).toHaveAttribute('data-hours-state', 'on');
      await card.scrollIntoViewIfNeeded();
      await expect(card.locator('[data-peak]')).toHaveAttribute('data-hour', '9');
      await expect(card.locator('.insights-hours-summary')).toContainText('From 32 dictations in History');
      const layout = await view.evaluate(element => {
        const rect = (selector: string) => element.querySelector(selector)!.getBoundingClientRect();
        const headline = rect('[data-stats="headline"]');
        const hours = rect('.insights-hours');
        const sections = rect('.usage-dashboard-sections');
        const plot = rect('.insights-hours-bars');
        const bars = Array.from(element.querySelectorAll('.insights-hours-bars > span')).map(bar => bar.getBoundingClientRect());
        const axis = Array.from(element.querySelectorAll('.insights-hours-axis > span')).map(label => label.getBoundingClientRect());
        const periods = Array.from(element.querySelectorAll('.insights-hours-periods > div')).map(cell => {
          const box = cell.getBoundingClientRect();
          return { left: box.left, right: box.right, dtRight: cell.querySelector('dt')!.getBoundingClientRect().right, ddLeft: cell.querySelector('dd')!.getBoundingClientRect().left };
        });
        return { headlineLeft: headline.left, headlineRight: headline.right, hoursLeft: hours.left, hoursRight: hours.right,
          hoursTop: hours.top, sectionsBottom: sections.bottom, plotLeft: plot.left, plotRight: plot.right, plotBottom: plot.bottom,
          bars: bars.map(bar => ({ left: bar.left, right: bar.right, bottom: bar.bottom, height: bar.height })),
          axis: axis.map(label => ({ left: label.left, right: label.right })), periods,
          overflow: element.scrollWidth - element.clientWidth };
      });
      expect(layout.hoursLeft).toBeCloseTo(layout.headlineLeft, 1);
      expect(layout.hoursRight).toBeCloseTo(layout.headlineRight, 1);
      expect(layout.sectionsBottom).toBeLessThanOrEqual(layout.hoursTop);
      expect(layout.overflow).toBe(0);
      // 24 bars in order, inside the plot, sharing a baseline; the peak is the tallest.
      expect(layout.bars).toHaveLength(24);
      for (let index = 1; index < 24; index += 1) expect(layout.bars[index].left).toBeGreaterThanOrEqual(layout.bars[index - 1].right);
      expect(layout.bars[0].left).toBeGreaterThanOrEqual(layout.plotLeft - 0.5);
      expect(layout.bars[23].right).toBeLessThanOrEqual(layout.plotRight + 0.5);
      const heights = layout.bars.map(bar => bar.height);
      expect(Math.max(...heights)).toBe(heights[9]);
      expect(heights[0]).toBe(0);
      // Axis labels start under their hour and never overlap each other.
      expect(layout.axis).toHaveLength(4);
      expect(layout.axis[0].left).toBeCloseTo(layout.bars[0].left, 0);
      for (let index = 1; index < 4; index += 1) expect(layout.axis[index].left).toBeGreaterThan(layout.axis[index - 1].right);
      // Period labels never run into their share.
      expect(layout.periods).toHaveLength(4);
      for (const cell of layout.periods) {
        expect(cell.dtRight).toBeLessThanOrEqual(cell.ddLeft);
        expect(cell.ddLeft).toBeLessThanOrEqual(cell.right);
      }

      await card.getByRole('switch', { name: 'Show when you dictate' }).click();
      await expect(card).toHaveAttribute('data-hours-state', 'off');
    });
  }
}
