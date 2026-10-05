import { test, expect } from "@playwright/test";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";

// Live-bar easing through the public TS API (`series_options.live_bar_easing_ms`): a same-time
// `update()` of the drawn last bar keeps the package's rAF loop presenting frames while the drawn
// close glides, the drawn candle changes across presented frames, every query reads the real
// values at once, and the loop stops when the glide settles. Easing off keeps today's single
// repaint per update.

async function wait_chart(page) {
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

/** Wait `n` animation frames without driving any repaint of our own. */
async function wait_frames(page, n) {
  await page.evaluate(async (count) => {
    for (let i = 0; i < count; i += 1) {
      await new Promise((resolve) => requestAnimationFrame(resolve));
    }
  }, n);
}

async function presented(page) {
  return page.evaluate(() => window.__chart.frame_stats().presented_frames);
}

/** Replace the main series' last bar in place with one closing `delta` above its open. */
async function replace_last_bar(page, delta) {
  return page.evaluate((d) => {
    const data = window.__main.data();
    const last = data[data.length - 1];
    const close = Math.max(last.open, last.close) + d;
    window.__main.update({
      time: last.time,
      open: last.open,
      high: Math.max(last.high, close) + 0.5,
      low: last.low,
      close,
    });
    return { time: last.time, close, real: window.__main.data().at(-1).close };
  }, delta);
}

async function capture(page) {
  const data_url = await page.evaluate(() => window.__chart.take_screenshot().toDataURL("image/png"));
  return PNG.sync.read(Buffer.from(data_url.split(",")[1], "base64"));
}

/** The device-pixel column band around the last bar, full pane height. */
async function last_bar_band(page) {
  return page.evaluate(() => {
    const dpr = window.devicePixelRatio || 1;
    const data = window.__main.data();
    const scale = window.__chart.time_scale();
    const x = scale.logical_to_coordinate(data.length - 1);
    const spacing = Math.abs(x - scale.logical_to_coordinate(data.length - 2));
    return {
      x0: Math.max(0, Math.floor((x - spacing) * dpr)),
      x1: Math.ceil((x + spacing) * dpr),
    };
  });
}

function band_differs(a, b, band) {
  const width = Math.min(band.x1, a.width, b.width) - band.x0;
  const height = Math.min(a.height, b.height);
  if (width <= 0) return false;
  const crop = (png) => {
    const out = new PNG({ width, height });
    for (let y = 0; y < height; y += 1) {
      for (let x = 0; x < width; x += 1) {
        const from = (y * png.width + band.x0 + x) * 4;
        const to = (y * width + x) * 4;
        out.data[to] = png.data[from];
        out.data[to + 1] = png.data[from + 1];
        out.data[to + 2] = png.data[from + 2];
        out.data[to + 3] = png.data[from + 3];
      }
    }
    return out;
  };
  const ca = crop(a);
  const cb = crop(b);
  return pixelmatch(ca.data, cb.data, null, width, height, { threshold: 0.1 }) > 0;
}

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
  await page.goto("/");
  await wait_chart(page);
});

test("the option round-trips, clamps, and rejects negative values before applying anything", async ({ page }) => {
  const result = await page.evaluate(() => {
    const main = window.__main;
    const before = main.options().live_bar_easing_ms;
    main.apply_options({ live_bar_easing_ms: 120 });
    const applied = main.options().live_bar_easing_ms;
    main.apply_options({ live_bar_easing_ms: 5000 });
    const clamped = main.options().live_bar_easing_ms;
    let error = null;
    try {
      main.apply_options({ live_bar_easing_ms: -1, title: "never applied" });
    } catch (e) {
      error = { code: e.code, title: main.options().title };
    }
    main.apply_options({ live_bar_easing_ms: 0 });
    return { before, applied, clamped, error, off: main.options().live_bar_easing_ms };
  });
  expect(result.before).toBe(0);
  expect(result.applied).toBe(120);
  expect(result.clamped).toBe(1000);
  expect(result.error?.code).toBe("invalid_options");
  expect(result.error?.title).not.toBe("never applied");
  expect(result.off).toBe(0);
});

test("a same-time update glides over several presented frames while queries read real values", async ({ page }) => {
  // The longest time constant, so the glide settles six seconds after the tick: a CI runner that
  // spends hundreds of milliseconds per screenshot still captures every frame mid-glide, while on
  // a fast machine the drawn close still moves by several pixels between two captures.
  const TAU_MS = 1000;
  await page.evaluate((tau) => window.__main.apply_options({ live_bar_easing_ms: tau }), TAU_MS);
  await wait_frames(page, 3);
  const idle = await presented(page);
  await wait_frames(page, 4);
  expect(await presented(page), "an idle eased chart presents nothing").toBeLessThanOrEqual(idle + 1);

  const start = await presented(page);
  const tick = await replace_last_bar(page, 6);
  expect(tick.real, "data() reads the real close at once").toBe(tick.close);
  // The tick frame, the clock stamp, then at least two glide frames inside 8 rAFs.
  await wait_frames(page, 8);
  expect(await presented(page)).toBeGreaterThanOrEqual(start + 3);

  // Presented frames draw the last bar differently while the glide runs: three captures, four
  // rAFs apart, and the bar moved between at least one pair of them.
  const band = await last_bar_band(page);
  const frames = [];
  for (let i = 0; i < 3; i += 1) {
    frames.push(await capture(page));
    await wait_frames(page, 4);
  }
  const changes = [
    band_differs(frames[0], frames[1], band),
    band_differs(frames[1], frames[2], band),
    band_differs(frames[0], frames[2], band),
  ];
  expect(changes.filter(Boolean).length, "the drawn last bar moved across presented frames").toBeGreaterThanOrEqual(1);

  // Queries never saw the glide.
  const real = await page.evaluate((expected) => {
    const last = window.__main.data().at(-1);
    return { close: last.close, matches: last.close === expected };
  }, tick.close);
  expect(real.matches).toBe(true);

  // Six time constants after the tick the glide has settled and the loop has stopped.
  await page.waitForTimeout(6 * TAU_MS + 600);
  const settled = await presented(page);
  await wait_frames(page, 6);
  expect(await presented(page), "the rAF loop stops once settled").toBeLessThanOrEqual(settled + 1);
  const still = [await capture(page)];
  await wait_frames(page, 2);
  still.push(await capture(page));
  expect(band_differs(still[0], still[1], band)).toBe(false);
});

test("easing off keeps one repaint per update", async ({ page }) => {
  await page.evaluate(() => window.__main.apply_options({ live_bar_easing_ms: 0 }));
  await wait_frames(page, 3);
  const start = await presented(page);
  await replace_last_bar(page, 6);
  await wait_frames(page, 8);
  expect(await presented(page)).toBeLessThanOrEqual(start + 2);
});
