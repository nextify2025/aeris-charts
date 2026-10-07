import { test as base } from "@playwright/test";

const failures = new WeakMap();

// Install before navigation, so a failed static import cannot disappear before the ready wait.
export const test = base.extend({
  page: async ({ page }, use) => {
    const stop = monitor_page(page);
    try {
      await use(page);
    } finally {
      stop();
    }
  },
});

export function monitor_page(page) {
  const state = { error: null, reject: null };
  const fail = (error) => {
    if (!state.error) state.error = error;
    state.reject?.(state.error);
  };
  const request_failed = (request) =>
    fail(new Error(`Request failed: ${request.url()} (${request.failure()?.errorText ?? "unknown error"})`));
  const page_error = (error) => fail(error);
  const navigated = (frame) => {
    if (frame === page.mainFrame()) state.error = null;
  };
  page.on("requestfailed", request_failed);
  page.on("pageerror", page_error);
  page.on("framenavigated", navigated);
  failures.set(page, state);
  return () => {
    page.off("requestfailed", request_failed);
    page.off("pageerror", page_error);
    page.off("framenavigated", navigated);
    failures.delete(page);
  };
}

export async function wait_for_chart(page, { grid = false, settle = true } = {}) {
  const state = failures.get(page);
  if (!state) throw new Error("page-ready wait requires the shared Playwright test fixture");
  if (state.error) throw state.error;
  let reject_failure;
  const failed = new Promise((_, reject) => { reject_failure = reject; });
  state.reject = reject_failure;
  try {
    await Promise.race([
      page.waitForFunction((needs_grid) =>
        (!needs_grid || window.__grid !== undefined) && window.__chart?.backend?.() !== undefined, grid),
      failed,
    ]);
  } finally {
    state.reject = null;
  }
  if (state.error) throw state.error;
  if (settle) {
    await page.evaluate(() => new Promise((resolve) => {
      requestAnimationFrame(() => requestAnimationFrame(resolve));
    }));
  }
}
