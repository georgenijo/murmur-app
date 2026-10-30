import { defineConfig } from '@playwright/test';
import base from './playwright.config';

// Parallel epic checkouts share this Mac. Never reuse another worker's Vite
// server: screenshots and measurements must exercise this checkout's code.
export default defineConfig({
  ...base,
  use: { ...base.use, baseURL: 'http://127.0.0.1:14854' },
  webServer: {
    command: 'npm run dev -- --host 127.0.0.1 --port 14854',
    url: 'http://127.0.0.1:14854/visual-fixtures.html',
    reuseExistingServer: false,
  },
});
