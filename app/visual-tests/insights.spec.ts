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

      await page.goto(`/visual-fixtures.html?state=insights-empty&appearance=${appearance}`);
      await expect(view.locator('.insights-empty-message')).toHaveText('Dictate something and your activity will show up here.');
      await expect(view.locator('.ui-day-chart-plot button, .ui-day-chart-plot svg')).toHaveCount(0);
      await expect(view.locator('.ui-day-chart-heatmap button[data-intensity="0"]')).toHaveCount(56);
      await expect(view.locator('.insights-more-statistics, .insights-streak')).toHaveCount(0);
      await speed.click();
      await expect(view.getByRole('figure', { name: /trend line: no dictations yet/ })).toHaveCount(1);
    });
  }
}
