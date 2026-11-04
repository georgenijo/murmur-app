import { expect, test } from '@playwright/test';

// Web-layout evidence only. Native transitions and AppKit's hover strip are
// exercised separately in the signed app; Chromium cannot reproduce them.
for (const appearance of ['light', 'dark']) {
  for (const workspace of ['Home expanded', 'Home collapsed', 'Settings', 'Diagnostics']) {
    for (const zoom of [0.5, 1, 1.2]) {
      test(`${workspace} fullscreen inset at ${zoom} zoom (${appearance})`, async ({ page }) => {
        await page.goto(`/visual-fixtures.html?state=${workspace.startsWith('Home') ? 'idle' : 'settings'}&appearance=${appearance}&checklist=hidden`);
        await expect(page.locator('[data-visual-ready="true"]')).toBeVisible();
        if (workspace === 'Diagnostics') {
          await page.evaluate(async () => {
            const paths = ['/node_modules/.vite/deps/react.js', '/node_modules/.vite/deps/react-dom_client.js', '/src/components/log-viewer/DiagnosticsWindowApp.tsx'];
            const [{ default: React }, { default: ReactDOM }, { DiagnosticsWindowApp }] = await Promise.all(paths.map(path => import(path)));
            document.getElementById('root')!.style.display = 'none';
            const host = document.createElement('div');
            host.id = 'fullscreen-diagnostics';
            document.body.append(host);
            ReactDOM.createRoot(host).render(React.createElement(DiagnosticsWindowApp));
          });
          await expect(page.locator('#fullscreen-diagnostics .ui-window-header')).toContainText('Diagnostics');
        }
        if (workspace === 'Home collapsed') await page.getByRole('button', { name: 'Collapse sidebar', exact: true }).click();
        await page.evaluate(async zoom => {
          const path = '/src/lib/windowLifecycle.tsx';
          await import(path); // Load the production lifecycle CSS.
          document.documentElement.style.setProperty('--ui-window-page-zoom', String(zoom));
        }, zoom);
        const header = page.locator(workspace === 'Diagnostics' ? '#fullscreen-diagnostics .ui-window-header' : '#root .ui-window-header');
        expect(await header.evaluate(el => parseFloat(getComputedStyle(el).paddingLeft))).toBeCloseTo(80 / zoom, 2);
        for (const fullscreen of [true, false, true]) {
          await page.evaluate(fullscreen => { document.documentElement.dataset.fullscreen = String(fullscreen); }, fullscreen);
          expect(await header.evaluate(el => parseFloat(getComputedStyle(el).paddingLeft))).toBeCloseTo(fullscreen ? 0 : 80 / zoom, 2);
          // Chromium quantizes layout positions to 1/64 CSS px.
          expect(await header.locator('.ui-window-header-content').evaluate(el => el.getBoundingClientRect().left)).toBeCloseTo(fullscreen ? 0 : 80 / zoom, 1);
          await expect(header).toHaveAttribute('data-tauri-drag-region', 'true');
          await expect(header.locator('.ui-window-header-content')).toHaveAttribute('data-tauri-drag-region', 'true');
        }
        if (workspace.startsWith('Home')) {
          const toggle = header.getByRole('button', { name: /sidebar/ });
          await expect(toggle).not.toHaveAttribute('data-tauri-drag-region');
          await toggle.click();
          await expect(toggle).toHaveAttribute('aria-expanded', String(workspace === 'Home collapsed'));
        }
      });
    }
  }
}
