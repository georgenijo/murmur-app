import { expect, type Locator, type Page } from '@playwright/test';

/** Browser layout only: native buttons remain AppKit's responsibility. */
export async function expectNativeChromeClearance(page: Page, header: Locator, zoom: number, expanded?: boolean) {
  const layout = await header.evaluate((element) => {
    const style = getComputedStyle(element);
    const stops = [...style.backgroundImage.matchAll(/([\d.]+)px/g)].map(match => Number(match[1]));
    return {
      strip: Math.max(0, ...stops),
      inset: parseFloat(style.paddingLeft),
      background: style.backgroundImage,
    };
  });
  expect(layout.strip === 0 || layout.strip * zoom >= 70, `painted strip ${layout.strip}px at zoom ${zoom}: ${layout.background}`).toBe(true);
  if (expanded === false) expect(layout.strip).toBe(0);
  if (expanded === true) {
    const sidebar = await page.locator('.home-sidebar').boundingBox();
    expect(layout.strip).toBeCloseTo(sidebar!.width, 1);
  }
  const toggle = header.getByRole('button', { name: /^(Expand|Collapse) sidebar$/ });
  if (expanded !== undefined) {
    await expect(toggle).toBeVisible();
    expect((await toggle.boundingBox())!.x * zoom).toBeGreaterThanOrEqual(layout.inset * zoom - 0.1);
    await expect(toggle).not.toHaveAttribute('data-tauri-drag-region');
    const navId = await page.getByRole('navigation', { name: 'Main destinations' }).getAttribute('id');
    await expect(toggle).toHaveAttribute('aria-controls', navId!);
  } else await expect(toggle).toHaveCount(0);
}
