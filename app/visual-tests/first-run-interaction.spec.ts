import { expect, test, type Page } from '@playwright/test';
import { expectConcentricSegments } from './segmented-geometry';

async function expectTitleBarHitTarget(page: Page) {
  const hits = await page.evaluate(() => {
    const header = document.querySelector('.ui-window-header')!;
    const bounds = header.getBoundingClientRect();
    return [120, innerWidth / 2, innerWidth - 24].map((x) => {
      const hit = document.elementFromPoint(x, bounds.top + bounds.height / 2);
      return !!hit && header.contains(hit) && hit.hasAttribute('data-tauri-drag-region');
    });
  });
  expect(hits).toEqual([true, true, true]);
}

for (const size of [{ width: 1100, height: 780 }, { width: 840, height: 620 }]) {
  for (const appearance of ['light', 'dark'] as const) {
    test(`first-run native chrome and model focus stay reachable at ${size.width}x${size.height} (${appearance})`, async ({ page }) => {
      await page.setViewportSize(size);
      await page.emulateMedia({ colorScheme: appearance, reducedMotion: 'reduce' });
      await page.goto(`/visual-fixtures.html?state=copy-onboarding&permission=granted&installed=true&appearance=${appearance}`);
      await expect(page.getByRole('heading', { name: 'Welcome to Murmur', exact: true })).toBeVisible();
      await expectTitleBarHitTarget(page);
      // Raising the header must leave the approved Welcome composition intact.
      expect(await page.locator('.onboarding-content').boundingBox()).toEqual({
        x: (size.width - 440) / 2, y: (size.height - 48 - 254) / 2, width: 440, height: 254,
      });
      await page.getByRole('button', { name: 'Get started', exact: true }).click();
      for (const title of ['Microphone access', 'Accessibility access', 'System audio access', 'Calendar access', 'Speech model', 'Recording shortcut', "You're all set"]) {
        await expect(page.getByRole('heading', { name: title, exact: true })).toBeVisible();
        await expectTitleBarHitTarget(page);
        if (title === 'Speech model') {
          const cards = page.locator('.model-option');
          await page.keyboard.press('Tab'); // Establish keyboard modality before moving focus.
          for (const card of [cards.first(), cards.last()]) {
            await card.focus();
            await expect(card).toBeFocused();
            const focus = await card.evaluate((node) => {
              const style = getComputedStyle(node);
              const bounds = node.getBoundingClientRect();
              const clip = node.closest('.onboarding-content')!.getBoundingClientRect();
              const reach = parseFloat(style.outlineWidth) + parseFloat(style.outlineOffset);
              return {
                visible: node.matches(':focus-visible'), width: style.outlineWidth,
                inside: reach <= 0 && bounds.left >= clip.left && bounds.right <= clip.right,
                withinViewport: bounds.top >= 0 && bounds.bottom <= innerHeight,
                cardWidth: bounds.width,
              };
            });
            expect(focus).toEqual({ visible: true, width: '2px', inside: true, withinViewport: true, cardWidth: 620 });
          }
        }
        if (title === 'Recording shortcut') await expectConcentricSegments(page.getByRole('group', { name: 'Recording trigger' }));
        if (title === "You're all set") await expect(page.getByText('Try it out', { exact: true })).toHaveCount(0);
        if (title !== "You're all set") await page.getByRole('navigation', { name: 'Setup step actions' }).getByRole('button', { name: 'Continue', exact: true }).click();
      }
      await page.goto(`/visual-fixtures.html?state=first-run-model&appearance=${appearance}`);
      await expect(page.getByRole('heading', { name: 'Speech model', exact: true })).toBeVisible();
      await expectTitleBarHitTarget(page);
      expect(await page.locator('.model-options').boundingBox()).toEqual({ x: (size.width - 620) / 2, y: size.height / 2 - 122, width: 620, height: 258 });
    });
  }
}
