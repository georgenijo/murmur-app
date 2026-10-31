import { defineConfig } from '@playwright/test';

const port = Number(process.env.MURMUR_VISUAL_PORT ?? 1420);

export default defineConfig({
  testDir: './visual-tests',
  fullyParallel: true,
  reporter: 'line',
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    viewport: { width: 880, height: 720 },
    deviceScaleFactor: 1,
  },
  expect: {
    toHaveScreenshot: {
      animations: 'disabled',
      maxDiffPixelRatio: 0.01,
    },
  },
  webServer: {
    command: `npm run dev -- --host 127.0.0.1 --port ${port}`,
    url: `http://127.0.0.1:${port}/visual-fixtures.html`,
    reuseExistingServer: !process.env.CI,
  },
});
