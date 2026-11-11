import { expect, test, type Locator } from '@playwright/test';

async function semanticColor(control: Locator, token: string) {
  const hex = await control.evaluate((element, name) => getComputedStyle(element).getPropertyValue(name).trim(), token);
  return `rgb(${hex.slice(1).match(/../g)!.map(value => parseInt(value, 16)).join(', ')})`;
}

async function danger(control: Locator) {
  await expect(control).toHaveCSS('color', await semanticColor(control, '--murmur-error'));
  await control.hover();
  await expect(control).toHaveCSS('color', await semanticColor(control, '--murmur-error'));
}

async function sharedHoverFill(control: Locator) {
  const resting = await control.evaluate(element => getComputedStyle(element).backgroundColor);
  await control.hover();
  await expect(control).toHaveCSS('background-color', await semanticColor(control, '--ui-dashboard-hover'));
  return resting;
}

for (const appearance of ['light', 'dark'] as const) {
  test.describe(`Shared secondary buttons ${appearance}`, () => {
    test.use({ colorScheme: appearance, reducedMotion: 'reduce' });
    test('meeting deletion retains its danger tone at rest, on hover and while confirming', async ({ page }) => {
      await page.goto(`/visual-fixtures.html?state=meetings-many&appearance=${appearance}`);
      await page.getByRole('region', { name: 'Meeting details' }).getByRole('button', { name: 'More meeting actions', exact: true }).click();
      const remove = page.getByRole('menuitem', { name: 'Delete meeting…', exact: true });
      await danger(remove);
      // Leaving the destructive item disarms it, so arm from keyboard with
      // the pointer already away to check confirmation paint before hover.
      await page.mouse.move(0, 0);
      await remove.focus();
      await remove.press('Space');
      const confirm = page.getByRole('menuitem', { name: 'Confirm delete…', exact: true });
      await danger(confirm);
      const fill = await confirm.evaluate(n => getComputedStyle(n).backgroundColor);
      expect(fill).not.toBe('rgba(0, 0, 0, 0)');
    });
    test('real General actions and the editor close action use the shared control boundary', async ({ page }) => {
      await page.goto(`/visual-fixtures.html?state=settings-general&appearance=${appearance}`);
      const setup = page.getByRole('button', { name: 'Open setup assistant', exact: true });
      await expect(setup).toHaveCSS('border-top-color', await semanticColor(setup, '--ui-control-border'));
      await page.getByRole('button', { name: /^Advanced/ }).click();
      const reset = page.getByRole('button', { name: 'Reset statistics', exact: true });
      await expect(reset).toHaveCSS('border-top-color', await semanticColor(reset, '--ui-control-border'));
      await reset.click();
      await danger(page.getByRole('button', { name: 'Confirm reset statistics', exact: true }));
      await page.goto(`/visual-fixtures.html?state=settings-copy&page=text&editor=aliases&appearance=${appearance}`);
      const close = page.getByRole('button', { name: 'Close editor', exact: true });
      await expect(close).toBeVisible();
      await sharedHoverFill(close);
    });
  });
}
