// usage: node shot.mjs <state> [outPrefix]   -> light/dark x 880x720 + 1120x800
import { chromium } from '@playwright/test';
const [state, prefix = state] = process.argv.slice(2);
const browser = await chromium.launch();
for (const appearance of ['light', 'dark']) {
  for (const [w, h] of [[880, 720], [1120, 800]]) {
    const page = await browser.newPage({ viewport: { width: w, height: h }, deviceScaleFactor: 2, reducedMotion: 'reduce' });
    const errors = [];
    page.on('pageerror', (e) => errors.push(String(e)));
    page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text()); });
    await page.goto(`http://127.0.0.1:1420/visual-fixtures.html?state=${state}&appearance=${appearance}`, { waitUntil: 'networkidle' });
    await page.locator('[data-visual-ready="true"]').waitFor();
    await page.waitForTimeout(600);
    const out = `/tmp/murmur-redesign/${prefix}-${appearance}-${w}x${h}.png`;
    await page.screenshot({ path: out });
    console.log(out, errors.length ? `ERRORS: ${errors.join(' | ').slice(0, 400)}` : 'ok');
    await page.close();
  }
}
await browser.close();
