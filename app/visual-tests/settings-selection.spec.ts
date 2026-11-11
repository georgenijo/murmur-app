import { expect, test } from '@playwright/test';

const appearances = ['light', 'dark'] as const;
const viewports = [
  { width: 880, height: 720, label: 'desktop' },
  { width: 720, height: 560, label: 'compact' },
] as const;

const navigationLabels = [
  'General',
  'Recording',
  'Customize',
  'Delivery',
  'Meetings',
  'Text & vocabulary',
  'AI & models',
  'Modes',
  'Appearance',
] as const;

for (const appearance of appearances) {
  for (const viewport of viewports) {
    test(`${appearance} Settings selection remains clear at ${viewport.label} width`, async ({ page }) => {
      await page.setViewportSize(viewport);
      await page.goto(`/visual-fixtures.html?state=settings&appearance=${appearance}`);

      const fixture = page.locator('[data-visual-ready="true"]');
      const navigation = page.getByRole('navigation', { name: 'Settings pages' });
      await expect(fixture).toBeVisible();
      await expect(navigation).toBeVisible();
      await expect(navigation.getByRole('button')).toHaveText(navigationLabels);

      await expect(navigation.getByRole('button', { name: 'General', exact: true })).toHaveAttribute('aria-current', 'page');
      await expect(page.getByRole('heading', { name: 'General', exact: true, level: 1 })).toBeVisible();
      const customize = navigation.getByRole('button', { name: 'Customize', exact: true });
      await customize.click();
      await expect(customize).toHaveAttribute('aria-current', 'page');
      await expect(page.getByRole('heading', { name: 'Customize', exact: true, level: 1 })).toBeVisible();

      const customizeIcon = customize.locator('svg').evaluate((svg) => svg.innerHTML);
      const modesIcon = navigation.getByRole('button', { name: 'Modes', exact: true }).locator('svg').evaluate((svg) => svg.innerHTML);
      expect(await customizeIcon).not.toBe(await modesIcon);

      await expect(customize).toHaveCSS('border-top-width', '0px');
      await expect(customize).not.toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');

      await navigation.getByRole('button', { name: 'Modes', exact: true }).click();
      const modeList = page.getByRole('list', { name: 'Modes' });
      const modeRow = modeList.getByRole('button', { name: /Technical Built-in/ }).first();
      await expect(modeRow).toBeVisible();
      await modeRow.click();
      await expect(modeRow).toHaveAttribute('aria-pressed', 'true');

      const selectedModeStyle = await modeRow.evaluate((element) => {
        const style = getComputedStyle(element);
        return {
          borderTopWidth: style.borderTopWidth,
          borderTopColor: style.borderTopColor,
          boxShadow: style.boxShadow,
        };
      });
      expect(selectedModeStyle.borderTopWidth).toBe('1px');
      expect(selectedModeStyle.borderTopColor).not.toBe('rgba(0, 0, 0, 0)');
      expect(selectedModeStyle.boxShadow).toContain('inset');
    });
  }
}
