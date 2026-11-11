import { expect, test } from '@playwright/test';

for (const fold of ['advanced', 'microphone'] as const) {
  for (const reducedMotion of ['no-preference', 'reduce'] as const) {
    test(`${fold} keeps its width and returns smoothly from the scroll limit (${reducedMotion})`, async ({ page }) => {
      await page.setViewportSize({ width: 1360, height: 800 });
      await page.emulateMedia({ reducedMotion });
      await page.goto('/visual-fixtures.html?state=settings-smart-auto&appearance=light');
      await page.locator('.settings-sidebar').getByRole('button', { name: 'Recording', exact: true }).click();
      const selector = fold === 'advanced' ? '.settings-advanced-toggle' : '.settings-microphone-disclosure';
      const toggle = page.locator(selector);
      const pane = page.getByTestId('settings-content');
      await expect(toggle).toHaveAttribute('aria-expanded', 'false');
      // Reserve the native scrollbar lane before a fold changes overflow so
      // the card edges remain stationary in WKWebView.
      await expect(pane).toHaveCSS('overflow-y', 'scroll');
      // Exercise the WebKit fallback independently of scrollbar-gutter support.
      await pane.evaluate(el => { el.style.scrollbarGutter = 'auto'; });
      const sample = async () => page.evaluate(async selector => {
        const toggle = document.querySelector<HTMLButtonElement>(selector)!;
        const branch = document.getElementById(toggle.getAttribute('aria-controls')!)!;
        const pane = document.querySelector<HTMLElement>('[data-testid="settings-content"]')!;
        const card = toggle.closest('.settings-card')!;
        const content = branch.querySelector('.settings-dependent-branch-content')!;
        const frames: { time: number; x: number; width: number; scroll: number; height: number }[] = [];
        const start = performance.now();
        const read = () => frames.push({ time: performance.now() - start,
          x: card.getBoundingClientRect().x, width: content.getBoundingClientRect().width,
          scroll: pane.scrollTop, height: branch.getBoundingClientRect().height });
        read();
        toggle.click();
        await Promise.resolve(); // let React install the fold's animation-frame callback first
        await new Promise<void>(resolve => {
          const frame = () => { read(); if (performance.now() - start < 650) requestAnimationFrame(frame); else resolve(); };
          requestAnimationFrame(frame);
        });
        return frames;
      }, selector);

      const opening = await sample();
      await expect(toggle).toHaveAttribute('aria-expanded', 'true');
      // Capture the bottom-scrolled case without the test driver scrolling the
      // offscreen disclosure into view before clicking it.
      await pane.evaluate(el => { el.scrollTop = el.scrollHeight; });
      const closing = await sample();
      await expect(toggle).toHaveAttribute('aria-expanded', 'false');
      const frames = [...opening, ...closing];
      expect(Math.max(...frames.map(f => f.x)) - Math.min(...frames.map(f => f.x))).toBeLessThan(.1);
      expect(Math.max(...frames.map(f => f.width)) - Math.min(...frames.map(f => f.width))).toBeLessThan(.1);
      expect(closing.at(-1)!.height).toBe(0);
      const final = await pane.evaluate(el => ({ scroll: el.scrollTop, max: Math.max(0, el.scrollHeight - el.clientHeight) }));
      expect(final.scroll).toBeLessThanOrEqual(final.max);
      expect(closing[0].scroll).toBeGreaterThan(final.scroll);
      if (reducedMotion === 'no-preference') {
        const intermediate = closing.filter(f => f.height > 1 && f.height < closing[0].height - 1);
        expect(intermediate.length).toBeGreaterThan(2);
        for (const frame of intermediate) {
          // Scroll tracks the visible fold, rather than holding until children
          // disappear or focus restoration snaps the owner to the viewport.
          const expected = final.scroll + (closing[0].scroll - final.scroll) * frame.height / closing[0].height;
          // Closing removes the 7px revealed-rows inset with the fold, so its
          // scroll position may differ by that one spacing unit from a pure
          // height-proportional interpolation.
          expect(Math.abs(frame.scroll - expected)).toBeLessThanOrEqual(8);
        }
      }
    });
  }
}
