import { defineConfig, devices } from "@playwright/test";

const port = Number.parseInt(process.env.AERIS_CHARTS_TEST_PORT ?? "4174", 10);
const portable_browser = process.env.AERIS_CHARTS_PORTABLE_BROWSER === "1";

export default defineConfig({
  testDir: "./tests",
  // GitHub's shared Windows runners are substantially slower than release developer machines.
  // Keep the local feedback ceiling tight while allowing the same assertions to finish in CI.
  timeout: process.env.CI ? 60_000 : 30_000,
  fullyParallel: false,
  workers: 1,
  reporter: [["list"], ["html", { open: "never" }]],
  outputDir: "test-results",
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    viewport: { width: 1280, height: 720 },
    deviceScaleFactor: 1.5,
    colorScheme: "light",
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
  },
  projects: [
    {
      // Chromium runs the full suite: the WebGPU backend (SwiftShader adapter) plus the shared
      // Canvas2D-fallback smoke. The WebGPU launch flags are Chromium-specific.
      name: "chromium",
      // Timing budgets and the native GPUI matrix remain machine evidence. Deterministic
      // browser/backend pixel comparisons run in the portable publication gate.
      testIgnore: portable_browser
        ? /(engine-bench|gpui-webgpu-matrix|perf-gate)\.spec\.mjs/
        : undefined,
      grepInvert: portable_browser ? /@machine/ : undefined,
      use: {
        channel: "chromium",
        launchOptions: {
          args: [
            "--enable-unsafe-webgpu",
            "--enable-unsafe-swiftshader",
            "--use-webgpu-adapter=swiftshader",
            "--enable-dawn-features=allow_unsafe_apis",
            "--disable-dawn-features=use_dxc",
            "--enable-webgpu-developer-features",
            "--use-gpu-in-tests",
            "--enable-accelerated-2d-canvas",
          ],
        },
      },
    },
    {
      // Firefox and WebKit have no headless WebGPU here, so they run only the Canvas2D-fallback
      // smoke — confirming the library loads and renders on those engines.
      name: "firefox",
      use: { ...devices["Desktop Firefox"] },
      testMatch: /(cross-browser|financial-compatibility|general-charts|unified-interaction-accessibility|public-reference-interaction)\.spec\.mjs/,
    },
    {
      name: "webkit",
      use: { ...devices["Desktop Safari"] },
      testMatch: /(cross-browser|financial-compatibility|general-charts|unified-interaction-accessibility|public-reference-interaction)\.spec\.mjs/,
    },
  ],
  webServer: {
    command: "node test_server.mjs",
    url: `http://127.0.0.1:${port}`,
    reuseExistingServer: !process.env.CI,
    timeout: 30_000,
  },
});
