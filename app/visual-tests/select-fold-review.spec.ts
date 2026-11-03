import { expect, test } from '@playwright/test';

test('Select recovers its usable height when an opening ancestor stops clipping', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.goto('/visual-fixtures.html?state=select-fold-review');
  const trigger = page.getByRole('combobox', { name: 'Timeout' });
  await page.getByRole('button', { name: 'Expand timeout' }).click();
  // Invoke the real trigger during the fold, without Playwright waiting for
  // animation stability and thereby skipping the failing lifecycle window.
  const duringFold = await trigger.evaluate(element => {
    const clip = element.closest('.settings-dependent-branch-clip')!;
    const folding = !clip.classList.contains('settings-dependent-branch-settled');
    (element as HTMLButtonElement).click();
    return folding;
  });
  expect(duringFold).toBe(true);
  await expect(page.locator('.settings-dependent-branch-clip')).toHaveClass(/settings-dependent-branch-settled/);
  const menu = page.getByRole('listbox', { name: 'Timeout' });
  await expect.poll(async () => (await menu.boundingBox())?.height ?? 0).toBeGreaterThan(120);
  await page.keyboard.press('End');
  await page.keyboard.press('Enter');
  await expect(trigger).toHaveText('300 seconds');
  await expect(menu).toHaveCount(0);
});
