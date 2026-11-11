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
    test('mode deletion keeps its explicit danger color when armed', async ({ page }) => {
      await page.goto(`/visual-fixtures.html?state=settings&appearance=${appearance}`);
      await page.getByRole('navigation', { name: 'Settings pages' }).getByRole('button', { name: 'Modes', exact: true }).click();
      await page.getByRole('button', { name: 'Create mode', exact: true }).click();
      const remove = page.locator('.settings-modes-danger').first();
      await danger(remove);
      await remove.click();
      await page.mouse.move(0, 0);
      await expect(remove).toHaveText('Confirm delete…');
      await danger(remove);
    });
    test('knowledge actions and their confirmation retain layered danger utilities', async ({ page }) => {
      await page.goto(`/visual-fixtures.html?state=settings-copy&page=text&editor=knowledge&appearance=${appearance}`);
      await danger(page.getByRole('button', { name: 'Delete all shown…', exact: true }));
      const remove = page.locator('button.text-error[aria-label^="Delete "]').first();
      await danger(remove);
      await remove.click();
      await danger(page.getByRole('dialog').getByRole('button', { name: 'Delete…', exact: true }));
    });
    test('real General actions and quiet actions use the shared control boundary', async ({ page }) => {
      await page.goto(`/visual-fixtures.html?state=settings-general&appearance=${appearance}`);
      const setup = page.getByRole('button', { name: 'Open setup assistant', exact: true });
      await expect(setup).toHaveCSS('border-top-color', await semanticColor(setup, '--ui-control-border'));
      await page.getByRole('button', { name: /^Advanced/ }).click();
      const reset = page.getByRole('button', { name: 'Reset statistics', exact: true });
      await expect(reset).toHaveCSS('border-top-color', await semanticColor(reset, '--ui-control-border'));
      await reset.click();
      await danger(page.getByRole('button', { name: 'Confirm reset statistics', exact: true }));
      await page.goto(`/visual-fixtures.html?state=settings-copy&page=text&editor=knowledge&appearance=${appearance}`);
      const quiet = page.getByRole('button', { name: 'Enable all shown', exact: true });
      await expect(quiet).toHaveCSS('border-top-color', await semanticColor(quiet, '--ui-control-border'));
      await page.goto(`/visual-fixtures.html?state=settings-copy&page=text&editor=knowledge&knowledge=empty&appearance=${appearance}`);
      const disabledQuiet = page.getByRole('button', { name: 'Enable all shown', exact: true });
      await expect(disabledQuiet).toBeDisabled();
      const disabledRestingFill = await disabledQuiet.evaluate(element => getComputedStyle(element).backgroundColor);
      await disabledQuiet.hover();
      await expect(disabledQuiet).toHaveCSS('background-color', disabledRestingFill);
      await page.goto(`/visual-fixtures.html?state=settings-copy&page=text&editor=knowledge&appearance=${appearance}`);
      const enabledQuiet = page.getByRole('button', { name: 'Enable all shown', exact: true });
      await sharedHoverFill(enabledQuiet);
      const close = page.getByRole('button', { name: 'Close editor', exact: true });
      await expect(close).toBeVisible();
      await sharedHoverFill(close);
    });
  });
}
