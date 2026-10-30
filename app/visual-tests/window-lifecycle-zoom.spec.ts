import { expect, test } from '@playwright/test';

for (const appearance of ['light', 'dark']) {
  for (const workspace of ['Settings', 'Diagnostics']) {
    test(`${workspace} reserves native traffic-light width at 50% page zoom (${appearance})`, async ({ page }) => {
      await page.setViewportSize({ width: 840, height: 620 });
      await page.goto(`/visual-fixtures.html?state=settings&appearance=${appearance}`);
      await expect(page.locator('[data-visual-ready="true"]')).toBeVisible();
      if (workspace === 'Diagnostics') {
        // Mount the production Diagnostics window, including its real header.
        // Reuse fixture IPC mocks without changing the shared fixture source.
        await page.evaluate(async () => {
          const reactPath = '/node_modules/.vite/deps/react.js';
          const clientPath = '/node_modules/.vite/deps/react-dom_client.js';
          const diagnosticsPath = '/src/components/log-viewer/DiagnosticsWindowApp.tsx';
          const [{ default: React }, { default: ReactDOM }, { DiagnosticsWindowApp }] = await Promise.all([
            import(reactPath), import(clientPath), import(diagnosticsPath),
          ]);
          document.getElementById('root')!.style.display = 'none';
          const host = document.createElement('div');
          host.id = 'zoom-diagnostics';
          document.body.append(host);
          ReactDOM.createRoot(host).render(React.createElement(DiagnosticsWindowApp));
        });
        await expect(page.locator('#zoom-diagnostics .ui-window-header')).toContainText('Diagnostics');
      }
      const header = page.locator(workspace === 'Settings'
        ? '#root .ui-window-header' : '#zoom-diagnostics .ui-window-header');
      await expect(header).toBeVisible();
      await page.evaluate(async () => {
        const modulePath = '/src/lib/windowLifecycle.tsx';
        const { startWindowLifecycle } = await import(modulePath);
        // The browser has no native webview. Exercise the production bridge's
        // shortcuts and CSS updates, and measure in CSS coordinates below.
        startWindowLifecycle(async () => {}, false);
      });
      const nativeInset = await header.evaluate((element) => parseFloat(getComputedStyle(element).paddingLeft));
      expect(nativeInset).toBe(80);
      for (let count = 0; count < 5; count++) await page.keyboard.press('Control+-');
      // A native 840x620 point window at 50% zoom has a 1680x1240 CSS viewport.
      // Multiply real layout coordinates by zoom to compare with AppKit points.
      // This is headless layout evidence, not native traffic-light interaction.
      await page.setViewportSize({ width: 1680, height: 1240 });
      await expect(header).toHaveCSS('padding-left', '160px');
      const layout = await header.evaluate((element) => {
        const content = element.querySelector('.ui-window-header-content')!;
        const first = content.firstElementChild!;
        return {
          headerHeight: element.getBoundingClientRect().height,
          contentLeft: content.getBoundingClientRect().left,
          firstLeft: first.getBoundingClientRect().left,
          firstWidth: first.getBoundingClientRect().width,
          zoom: document.documentElement.style.getPropertyValue('--ui-window-page-zoom'),
        };
      });
      expect(layout.zoom).toBe('0.5');
      expect(layout.headerHeight).toBeGreaterThan(0);
      expect(layout.contentLeft * 0.5).toBe(nativeInset);
      expect(layout.firstLeft * 0.5).toBeGreaterThanOrEqual(nativeInset);
      expect(layout.firstWidth).toBeGreaterThan(0);
      // Changing the design token must still control the compensated inset;
      // the bridge never duplicates the 5rem measurement in JavaScript.
      await page.evaluate(() => document.documentElement.style.setProperty('--ui-window-controls-inset', 'calc(4rem + 20px)'));
      await expect(header).toHaveCSS('padding-left', '168px');
      await page.keyboard.press('Control+0');
      await page.setViewportSize({ width: 840, height: 620 });
      await expect(header).toHaveCSS('padding-left', '84px');
      await page.keyboard.press('Control++');
      await page.keyboard.press('Control++');
      await expect(header).toHaveCSS('padding-left', '70px');
    });
  }
}
