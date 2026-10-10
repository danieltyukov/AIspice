import { defineConfig, devices } from "@playwright/test";

/*
 * End-to-end tests against the production build served by `vite preview`.
 * The app runs on the in-browser mock backend there (no __TAURI_INTERNALS__),
 * and the tests open it with ?fast so the scripted agent and the simulator
 * answer without artificial delays.
 */
export default defineConfig({
  testDir: "./e2e",
  outputDir: "./e2e/results",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? "line" : "list",
  timeout: 30_000,
  expect: { timeout: 7_000 },
  use: {
    baseURL: "http://127.0.0.1:4173",
    trace: "retain-on-failure",
    viewport: { width: 1440, height: 900 },
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1440, height: 900 } } }],
  webServer: {
    command: "npm run build && npm run preview -- --port 4173 --strictPort --host 127.0.0.1",
    url: "http://127.0.0.1:4173",
    reuseExistingServer: !process.env.CI,
    timeout: 180_000,
  },
});
