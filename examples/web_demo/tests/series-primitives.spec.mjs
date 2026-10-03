import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";

const fixture = JSON.parse(readFileSync(new URL("../fixtures/d1/candles.json", import.meta.url), "utf8"));

test.beforeEach(async ({ page }) => {
  page.on("console", (message) => console.log(`[browser:${message.type()}] ${message.text()}`));
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
});

async function wait_for_chart(page) {
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

async function settle_frames(page) {
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

// Reference series primitive exercising the series-bound price converter, the z-order layers,
// and both axis-label surfaces. Like the pane-primitive reference it deliberately emits
// quad-family commands only (plus one proven-safe translucent bottom fill), so the
// WebGPU/Canvas2D identity assertion isolates the primitive pipeline. The band price comes
// from the deterministic fixture data, captured at attach time (plugins must not call chart
// APIs from inside mid-render hooks).
function reference_series_primitive_factory(entry_price) {
  const top = entry_price * 1.05;
  const bottom = entry_price * 0.95;
  return {
    pane_views: () => [
      {
        z_order: "bottom",
        renderer(ctx) {
          ctx.rect(ctx.pane_left + 40, ctx.pane_top + 30, 120, 90, "rgba(41, 98, 255, 0.12)");
        },
      },
      {
        z_order: "normal",
        renderer(ctx) {
          const y_top = ctx.price_to_y(top);
          const y_bottom = ctx.price_to_y(bottom);
          if (y_top === null || y_bottom === null) return;
          ctx.hline(y_top, ctx.pane_left, ctx.pane_left + ctx.pane_width, "#7b1fa2", 2, 0);
          ctx.hline(y_bottom, ctx.pane_left, ctx.pane_left + ctx.pane_width, "#7b1fa2", 2, 0);
        },
      },
      {
        z_order: "top",
        renderer(ctx) {
          // Opaque on purpose (see the pane-primitive reference for the blend-tie rationale).
          ctx.rect_frame(ctx.pane_left + 200, ctx.pane_top + 50, 140, 70, "#7b1fa2", 2);
        },
      },
    ],
    // The `price` descriptor form: the host converts it on the owning series' scale.
    price_axis_views: () => [{ text: "SBAND", price: entry_price, color: "#7b1fa2" }],
    time_axis_views: () => [{ text: "S1", coordinate: 150, color: "#7b1fa2" }],
  };
}

async function goto_fixture(page, backend) {
  await page.goto(`/?runtimeTest=presentedFrame&backend=${backend}&forceFallbackAdapter=1`);
  await wait_for_chart(page);
}

async function attach_reference_series_primitive(page) {
  await page.evaluate((factory_source) => {
    // eslint-disable-next-line no-eval
    const factory = eval(`(${factory_source})`);
    const entry = window.__data[Math.floor(window.__data.length / 2)].close;
    window.__reference_series_primitive_handle = window.__main.attach_primitive(factory(entry));
  }, reference_series_primitive_factory.toString());
  await settle_frames(page);
}

async function detach_reference_series_primitive(page) {
  await page.evaluate(() => {
    window.__reference_series_primitive_handle.detach();
    window.__reference_series_primitive_handle = null;
  });
  await settle_frames(page);
}

function crop_png(source, x, y, width, height) {
  const output = new PNG({ width, height });
  PNG.bitblt(source, output, x, y, width, height, 0, 0);
  return output;
}

function count_different(a, b) {
  expect([a.width, a.height]).toEqual([b.width, b.height]);
  return pixelmatch(a.data, b.data, new PNG({ width: a.width, height: a.height }).data, a.width, a.height, {
    threshold: 0,
    includeAA: true,
  });
}

function max_channel_delta(a, b) {
  let max_delta = 0;
  for (let i = 0; i < a.data.length; i += 1) {
    max_delta = Math.max(max_delta, Math.abs(a.data[i] - b.data[i]));
  }
  return max_delta;
}

test("series primitive paints identically on both backends, labels its series' axis, and detaches cleanly", async ({ page }, test_info) => {
  const pixel_ratio = fixture.pixel_ratio;
  const pane_width = Math.round((fixture.css_width - fixture.price_axis_width) * pixel_ratio);
  const pane_height = Math.round((fixture.css_height - fixture.time_axis_height) * pixel_ratio);

  // ---- Canvas2D: baseline → attach → detach ----
  await goto_fixture(page, "canvas2d");
  expect(await page.evaluate(() => window.__chart.backend())).toBe("canvas2d");
  const canvas_before = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));
  await attach_reference_series_primitive(page);
  const canvas_attached = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));

  // (c) The primitive actually painted: the pane region (band lines/frame/fill), the
  // price-axis strip (boxed SBAND label converted from its `price`), and the time strip
  // (boxed S1 label) all changed.
  const pane_diff = count_different(
    crop_png(canvas_before, 0, 0, pane_width, pane_height),
    crop_png(canvas_attached, 0, 0, pane_width, pane_height),
  );
  expect(pane_diff, "pane region must change where the primitive draws").toBeGreaterThan(0);
  const price_axis_diff = count_different(
    crop_png(canvas_before, pane_width, 0, canvas_before.width - pane_width, pane_height),
    crop_png(canvas_attached, pane_width, 0, canvas_attached.width - pane_width, pane_height),
  );
  expect(price_axis_diff, "price axis must gain the primitive's boxed label").toBeGreaterThan(0);
  const time_axis_diff = count_different(
    crop_png(canvas_before, 0, pane_height, pane_width, canvas_before.height - pane_height),
    crop_png(canvas_attached, 0, pane_height, pane_width, canvas_attached.height - pane_height),
  );
  expect(time_axis_diff, "time axis must gain the primitive's boxed label").toBeGreaterThan(0);

  // (d) Detach restores the exact prior pixels (same backend, deterministic render).
  await detach_reference_series_primitive(page);
  const canvas_restored = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));
  expect(count_different(canvas_before, canvas_restored)).toBe(0);

  // ---- WebGPU: same chart + same primitive → identical presented frame ----
  await goto_fixture(page, "auto");
  expect(await page.evaluate(() => window.__chart.backend())).toBe("webgpu");
  await attach_reference_series_primitive(page);
  const gpu_attached = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));

  // (a) Primitive pane geometry remains byte-identical; shared axis text may differ only at
  // bounded fractional-DPR antialiasing edges.
  const pane_backend_diff = count_different(
    crop_png(gpu_attached, 0, 0, pane_width, pane_height),
    crop_png(canvas_attached, 0, 0, pane_width, pane_height),
  );
  expect(pane_backend_diff, "series primitive pane geometry must remain pixel-identical").toBe(0);
  const backend_diff = count_different(gpu_attached, canvas_attached);
  const backend_max_delta = max_channel_delta(gpu_attached, canvas_attached);
  console.log(`series-primitive full-frame residual: ${backend_diff} px, max delta ${backend_max_delta}`);
  if (backend_diff > 2_600 || backend_max_delta > 40) {
    await test_info.attach("webgpu.png", { body: PNG.sync.write(gpu_attached), contentType: "image/png" });
    await test_info.attach("canvas2d.png", { body: PNG.sync.write(canvas_attached), contentType: "image/png" });
  }
  // Windows SwiftShader measurement: 2,307 pixels, max delta 32.
  expect(backend_diff, "full-frame differences must stay confined to bounded AA edges").toBeLessThanOrEqual(2_600);
  expect(backend_max_delta).toBeLessThanOrEqual(40);
});

test("series primitive autoscale_info expands the owning scale and detach restores it", async ({ page }) => {
  await goto_fixture(page, "canvas2d");

  // (b) Visible price range via the public price-scale handle, without vs with the demo's
  // scale-band primitive (its autoscale_info reaches ±10 beyond the data on both ends).
  const plain_range = await page.evaluate(() => window.__chart.price_scale("right").get_visible_range());
  expect(plain_range).not.toBeNull();
  await page.evaluate(() => window.__set_scale_band(true));
  await settle_frames(page);
  expect(await page.evaluate(() => window.__scale_band_active())).toBe(true);
  const expanded_range = await page.evaluate(() => window.__chart.price_scale("right").get_visible_range());
  expect(expanded_range).not.toBeNull();
  expect(expanded_range.from, "band must reach below the data range").toBeLessThan(plain_range.from - 5);
  expect(expanded_range.to, "band must reach above the data range").toBeGreaterThan(plain_range.to + 5);

  // (d) Detaching the primitive restores the data-driven range exactly.
  await page.evaluate(() => window.__set_scale_band(false));
  await settle_frames(page);
  expect(await page.evaluate(() => window.__scale_band_active())).toBe(false);
  const restored_range = await page.evaluate(() => window.__chart.price_scale("right").get_visible_range());
  expect(restored_range).not.toBeNull();
  expect(restored_range.from).toBeCloseTo(plain_range.from, 8);
  expect(restored_range.to).toBeCloseTo(plain_range.to, 8);
});

test("removing the owning series auto-detaches its primitives; later attaches keep working", async ({ page }) => {
  const page_errors = [];
  page.on("pageerror", (error) => page_errors.push(error.message));
  await goto_fixture(page, "canvas2d");

  const spy = await page.evaluate(() => {
    window.__spy = { attached: 0, detached: 0, params: null, request_update: null };
    const spy_primitive = {
      attached(params) {
        window.__spy.attached += 1;
        window.__spy.params = { series_id: params.series_id, pane_index: params.pane_index };
        window.__spy.request_update = params.request_update ?? null;
      },
      detached() {
        window.__spy.detached += 1;
      },
      pane_views: () => [],
    };
    const extra = window.__chart.add_series("line");
    extra.set_data(window.__data.map((bar) => ({ time: bar.time, value: bar.close })));
    const handle = extra.attach_primitive(spy_primitive);
    return {
      extra_id: extra.id,
      attached: window.__spy.attached,
      params: window.__spy.params,
      has_request_update: typeof window.__spy.request_update === "function",
      handle_type: typeof handle.detach,
    };
  });
  expect(spy.attached).toBe(1);
  expect(spy.params.series_id).toBe(spy.extra_id);
  expect(spy.params.pane_index).toBe(0);
  expect(spy.has_request_update).toBe(true);
  expect(spy.handle_type).toBe("function");

  // The injected request_update schedules a repaint; calling it must not throw mid-lifecycle.
  await page.evaluate(() => window.__spy.request_update());
  await settle_frames(page);

  // (e) Removing the owning series auto-detaches the primitive (reference drops it with the series).
  await page.evaluate(() => {
    const extra = window.__chart.series_order().find((series) => series.id === window.__spy.params.series_id);
    window.__chart.remove_series(extra);
  });
  await settle_frames(page);
  expect(await page.evaluate(() => window.__spy.detached)).toBe(1);

  // The next attach (on the surviving main series) works, and its handle detaches normally.
  const reattach = await page.evaluate(() => {
    const handle = window.__main.attach_primitive({
      attached(params) {
        window.__spy.attached += 1;
        window.__spy.params = { series_id: params.series_id, pane_index: params.pane_index };
      },
      detached() {
        window.__spy.detached += 1;
      },
      pane_views: () => [],
    });
    const attached = window.__spy.attached;
    const series_id = window.__spy.params.series_id;
    handle.detach();
    return { attached, series_id, main_id: window.__main.id, detached: window.__spy.detached };
  });
  await settle_frames(page);
  expect(reattach.attached).toBe(2);
  expect(reattach.series_id).toBe(reattach.main_id);
  expect(reattach.detached).toBe(2);

  expect(page_errors, "no page errors across the auto-detach cycle").toEqual([]);
});
