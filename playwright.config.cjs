const { defineConfig } = require('@playwright/test');
module.exports = defineConfig({
  testDir: './e2e', fullyParallel: true, workers: 2,
  timeout: 30000, retries: process.env.CI ? 1 : 0,
  reporter: [['list'], ['html', { open: 'never' }]],
  use: { baseURL: 'http://127.0.0.1:4329', channel: process.env.PW_CHANNEL || undefined, trace: 'retain-on-failure', screenshot: 'only-on-failure' },
  webServer: { command: 'node scripts/ui-test-server.cjs', url: 'http://127.0.0.1:4329', reuseExistingServer: false },
});
