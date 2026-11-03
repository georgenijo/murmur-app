import { expect, test } from '@playwright/test';

for (const appearance of ['light', 'dark']) {
  test(`Voice commands search reveals without switch focus and editor link focuses once in ${appearance}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width: 840, height: 620 });
    await page.goto(`/visual-fixtures.html?state=settings-copy&page=text&appearance=${appearance}`);
    const search = page.getByRole('searchbox', { name: 'Search all settings' });
    await search.fill('Voice commands');
    await page.getByRole('button', { name: /Voice commands & spoken formatting.*Enable custom commands/ }).click();
    const toggle = page.getByRole('switch', { name: 'Voice commands & spoken formatting', exact: true });
    await expect(toggle).toBeVisible();
    await expect(toggle).not.toBeFocused();
    await expect(toggle).toHaveAttribute('aria-checked', 'false');
    await page.getByRole('button', { name: 'Voice commands Create exact spoken replacements and snippets', exact: true }).click();
    await page.getByRole('button', { name: 'Open Voice commands & spoken formatting', exact: true }).click();
    await expect(toggle).toBeVisible();
    await expect(toggle).toBeFocused();
    await expect(toggle).toHaveAttribute('aria-checked', 'false');
    await testInfo.attach(`voice-commands-${appearance}.png`, { body: await page.screenshot(), contentType: 'image/png' });
    await search.click();
    // Outlast the bounded reveal retry, so a delayed refocus would be detected.
    await page.waitForTimeout(550);
    await expect(search).toBeFocused();
  });

  test(`Sensitivity keyboard and pointer values survive navigation in ${appearance}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width: 840, height: 620 });
    await page.goto(`/visual-fixtures.html?state=settings-copy&page=recording&appearance=${appearance}`);
    const slider = page.getByRole('slider', { name: 'Sensitivity', exact: true });
    await slider.focus();
    await page.keyboard.press('Home');
    await expect(slider).toHaveValue('0');
    await expect(slider).toHaveAttribute('aria-valuetext', 'Off');
    await page.keyboard.press('ArrowRight');
    await expect(slider).toHaveValue('5');
    await page.getByRole('button', { name: 'General', exact: true }).click();
    await page.getByRole('button', { name: 'Recording', exact: true }).click();
    await expect(slider).toHaveValue('5');
    const bounds = (await slider.boundingBox())!;
    await slider.click({ position: { x: bounds.width * 0.75, y: bounds.height / 2 } });
    const pointerValue = await slider.inputValue();
    expect(Number(pointerValue)).toBeGreaterThan(5);
    await page.getByRole('button', { name: 'General', exact: true }).click();
    await page.getByRole('button', { name: 'Recording', exact: true }).click();
    await expect(slider).toHaveValue(pointerValue);
    await testInfo.attach(`sensitivity-${appearance}.png`, { body: await page.screenshot(), contentType: 'image/png' });
  });
}
