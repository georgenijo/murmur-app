import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './visual-tests',
  fullyParallel: true,
  reporter: 'line',
  use: {
    baseURL: 'http://127.0.0.1:1420',
    viewport: { width: 1180, height: 760 },
    deviceScaleFactor: 1,
  },
  expect: {
    toHaveScreenshot: {
      animations: 'disabled',
      maxDiffPixelRatio: 0.01,
    },
  },
  webServer: {
    command: 'npm run dev -- --host 127.0.0.1',
    url: 'http://127.0.0.1:1420/visual-fixtures.html',
    reuseExistingServer: !process.env.CI,
  },
});
