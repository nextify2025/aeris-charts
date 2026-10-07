import { expect } from "@playwright/test";
import { test, wait_for_chart } from "./page-ready.mjs";
import { readFileSync } from "node:fs";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";
import { crop_png, count_different, max_channel_delta } from "./parity-pixels.mjs";

const fixture = JSON.parse(readFileSync(new URL("../fixtures/d1/candles.json", import.meta.url), "utf8"));

test.beforeEach(async ({ page }) => {
  page.on("console", (message) => console.log(`[browser:${message.type()}] ${message.text()}`));
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
});

async function settle_frames(page) {
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

// Reference pane primitive exercising the three z-order layers plus both axis-label surfaces.
// It deliberately emits quad-family commands only (rect/hline/rect_frame): those paint in the
// same bucket order on both backends, so the WebGPU/Canvas2D identity assertion below isolates
// the primitive pipeline itself rather than the engines' tri/quad bucket split.
function reference_primitive_factory() {
  return {
    pane_views: () => [
      {
        z_order: "bottom",
        renderer(ctx) {
          ctx.rect(ctx.pane_left + 60, ctx.pane_top + 10, 150, ctx.pane_height - 20, "rgba(41, 98, 255, 0.12)");
        },
      },
      {
        z_order: "normal",
        renderer(ctx) {
          ctx.hline(ctx.pane_top + 40, ctx.pane_left, ctx.pane_left + ctx.pane_width, "#e91e63", 2, 0);
          ctx.vline(ctx.pane_left + 260, ctx.pane_top, ctx.pane_top + ctx.pane_height, "#e91e63", 2, 0);
        },
      },
      {
        z_order: "top",
        renderer(ctx) {
          // Opaque on purpose: translucent fills can land on a 0.5 alpha-blend rounding tie
          // that the two rasterizers resolve a channel unit apart (a backend artifact the
          // cross-backend fixture never exercises), which is outside this pipeline's scope.
          ctx.rect_frame(ctx.pane_left + 300, ctx.pane_top + 60, 140, 70, "#e91e63", 2);
        },
      },
    ],
    price_axis_views: () => [{ text: "PBAND", coordinate: 40, color: "#e91e63" }],
    time_axis_views: () => [{ text: "P1", coordinate: 120, color: "#e91e63" }],
  };
}

async function goto_fixture(page, backend) {
  await page.goto(`/?runtimeTest=presentedFrame&backend=${backend}&forceFallbackAdapter=1`);
  await wait_for_chart(page);
}

async function attach_reference_primitive(page) {
  await page.evaluate((factory_source) => {
    // eslint-disable-next-line no-eval
    const factory = eval(`(${factory_source})`);
    window.__reference_primitive_handle = window.__chart.panes()[0].attach_primitive(factory());
  }, reference_primitive_factory.toString());
  await settle_frames(page);
}

async function detach_reference_primitive(page) {
  await page.evaluate(() => {
    window.__reference_primitive_handle.detach();
    window.__reference_primitive_handle = null;
  });
  await settle_frames(page);
}

test("pane primitive paints identically on both backends, changes its regions, and detaches cleanly", async ({ page }, test_info) => {
  const pixel_ratio = fixture.pixel_ratio;
  const pane_width = Math.round((fixture.css_width - fixture.price_axis_width) * pixel_ratio);
  const pane_height = Math.round((fixture.css_height - fixture.time_axis_height) * pixel_ratio);

  // ---- Canvas2D: baseline → attach → detach ----
  await goto_fixture(page, "canvas2d");
  expect(await page.evaluate(() => window.__chart.backend())).toBe("canvas2d");
  const canvas_before = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));
  await attach_reference_primitive(page);
  const canvas_attached = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));

  // (b) The primitive actually painted: the pane region (bands/lines/frame), the price-axis
  // strip (boxed PBAND label), and the time strip (boxed P1 label) all changed.
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

  // (c) Detach restores the exact prior pixels (same backend, deterministic render).
  await detach_reference_primitive(page);
  const canvas_restored = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));
  expect(count_different(canvas_before, canvas_restored)).toBe(0);

  // ---- WebGPU: same chart + same primitive → identical presented frame ----
  await goto_fixture(page, "auto");
  expect(await page.evaluate(() => window.__chart.backend())).toBe("webgpu");
  await attach_reference_primitive(page);
  const gpu_attached = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));

  // (a) Primitive pane geometry remains byte-identical; shared axis text may differ only at
  // bounded fractional-DPR antialiasing edges.
  const pane_backend_diff = count_different(
    crop_png(gpu_attached, 0, 0, pane_width, pane_height),
    crop_png(canvas_attached, 0, 0, pane_width, pane_height),
  );
  expect(pane_backend_diff, "pane primitive geometry must remain pixel-identical").toBe(0);
  const backend_diff = count_different(gpu_attached, canvas_attached);
  const backend_max_delta = max_channel_delta(gpu_attached, canvas_attached);
  console.log(`pane-primitive full-frame residual: ${backend_diff} px, max delta ${backend_max_delta}`);
  if (backend_diff > 2_600 || backend_max_delta > 40) {
    await test_info.attach("webgpu.png", { body: PNG.sync.write(gpu_attached), contentType: "image/png" });
    await test_info.attach("canvas2d.png", { body: PNG.sync.write(canvas_attached), contentType: "image/png" });
  }
  // Windows SwiftShader measurement: 2,307 pixels, max delta 32.
  expect(backend_diff, "full-frame differences must stay confined to bounded AA edges").toBeLessThanOrEqual(2_600);
  expect(backend_max_delta).toBeLessThanOrEqual(40);

  // Sanity: the demo's built-in session-bands primitive (z_order "bottom") also toggles.
  await page.evaluate(() => window.__set_day_bands(true));
  await settle_frames(page);
  expect(await page.evaluate(() => window.__day_bands_active())).toBe(true);
  const gpu_bands = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));
  expect(count_different(gpu_attached, gpu_bands)).toBeGreaterThan(0);
  await page.evaluate(() => window.__set_day_bands(false));
  await settle_frames(page);
  expect(await page.evaluate(() => window.__day_bands_active())).toBe(false);
});

test("plugin dashed polylines paint identical dashes on WebGPU and Canvas2D", async ({ page }, test_info) => {
  // The decoder lowers dashed and dotted plugin polylines to solid dash runs clipped to the
  // owning pane (the same shared lowering the engine's own strokes use). No packaged plugin emits a styled polyline,
  // so this test defines its own: a horizontal dashed one, a dotted diagonal that starts left of
  // the pane and leaves past its right edge, and a solid control.
  const run_scenario = async (backend, styles) => {
    await goto_fixture(page, backend);
    await page.evaluate((styles) => {
      window.__dash_handle = window.__chart.panes()[0].attach_primitive({
        pane_views: () => [
          {
            z_order: "top",
            renderer(ctx) {
              const { pane_left: left, pane_top: top, pane_width: w, pane_height: h } = ctx;
              ctx.polyline([left + 20, top + h * 0.25, left + w - 20, top + h * 0.25], "#7b1fa2", 3, styles[0]);
              ctx.polyline([left - 300, top + h * 0.9, left + w + 300, top + h * 0.4], "#e91e63", 3, styles[1]);
              ctx.polyline([left + 20, top + h * 0.6, left + w - 20, top + h * 0.6], "#00897b", 3, 0);
            },
          },
        ],
      });
    }, styles);
    await settle_frames(page);
    return {
      backend: await page.evaluate(() => window.__chart.backend()),
      png: PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false })),
    };
  };
  const dashed = [2, 1];
  const canvas = await run_scenario("canvas2d", dashed);
  expect(canvas.backend).toBe("canvas2d");
  const gpu = await run_scenario("auto", dashed);
  expect(gpu.backend).toBe("webgpu");
  expect([canvas.png.width, canvas.png.height]).toEqual([gpu.png.width, gpu.png.height]);

  // Vacuousness guard: the dashes leave gaps that solid strokes would ink on each backend.
  const solid_canvas = await run_scenario("canvas2d", [0, 0]);
  const solid_gpu = await run_scenario("auto", [0, 0]);
  expect(count_different(solid_canvas.png, canvas.png), "Canvas2D dashes the plugin polylines").toBeGreaterThan(500);
  expect(count_different(solid_gpu.png, gpu.png), "WebGPU dashes the plugin polylines").toBeGreaterThan(500);

  // A dash painted where the other backend leaves a gap (a stroke painted solid) differs by the stroke's full contrast, far above any raster difference. Dash ends may differ
  // by one coverage step between Canvas2D's analytic coverage and WebGPU's faded butt caps, so
  // those stay few and isolated (the same bound as the core-drawing dash parity test).
  let paint_diff = 0;
  let dash_end_diff = 0;
  for (let offset = 0; offset < canvas.png.data.length; offset += 4) {
    let pixel_delta = 0;
    for (let channel = 0; channel < 4; channel += 1) {
      pixel_delta = Math.max(pixel_delta, Math.abs(canvas.png.data[offset + channel] - gpu.png.data[offset + channel]));
    }
    if (pixel_delta > 200) paint_diff += 1;
    else if (pixel_delta > 128) dash_end_diff += 1;
  }
  console.log(`plugin dashed polylines parity: ${dash_end_diff} dash-end coverage pixels, ${paint_diff} paint pixels`);
  if (paint_diff !== 0 || dash_end_diff > 32) {
    const visual = new PNG({ width: canvas.png.width, height: canvas.png.height });
    pixelmatch(canvas.png.data, gpu.png.data, visual.data, canvas.png.width, canvas.png.height, { threshold: 0, includeAA: true });
    await test_info.attach("canvas2d.png", { body: PNG.sync.write(canvas.png), contentType: "image/png" });
    await test_info.attach("webgpu.png", { body: PNG.sync.write(gpu.png), contentType: "image/png" });
    await test_info.attach("diff.png", { body: PNG.sync.write(visual), contentType: "image/png" });
  }
  expect(paint_diff, "plugin dashed polylines paint the same dashes on both executors").toBe(0);
  expect(dash_end_diff, "dash-end coverage steps stay isolated").toBeLessThanOrEqual(32);
});

test("text_views are clipped to their owning pane and cannot cover axis chrome", async ({ page }) => {
  const pixel_ratio = fixture.pixel_ratio;
  const pane_width = Math.round((fixture.css_width - fixture.price_axis_width) * pixel_ratio);
  const pane_height = Math.round((fixture.css_height - fixture.time_axis_height) * pixel_ratio);
  await goto_fixture(page, "canvas2d");
  const before = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));

  await page.evaluate(() => {
    window.__edge_text_handle = window.__chart.panes()[0].attach_primitive({
      text_views: (info) => [{
        text: "CLIPPED AT AXIS",
        x: info.pane_left + info.pane_width - 5,
        y: info.pane_top + 80,
        color: "#ff00ff",
        size: 18,
        align: "left",
        baseline: "middle",
      }],
    });
  });
  await settle_frames(page);
  const attached = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));

  expect(count_different(
    crop_png(before, 0, 0, pane_width, pane_height),
    crop_png(attached, 0, 0, pane_width, pane_height),
  ), "the in-pane edge of the text must remain visible").toBeGreaterThan(0);
  expect(count_different(
    crop_png(before, pane_width, 0, before.width - pane_width, pane_height),
    crop_png(attached, pane_width, 0, attached.width - pane_width, pane_height),
  ), "plugin text must not alter the price-axis strip").toBe(0);

  await page.evaluate(() => window.__edge_text_handle.detach());
  await settle_frames(page);
  const restored = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));
  expect(count_different(before, restored)).toBe(0);
});

test("plugin text_views enter the ordered pane frame used by pane-only screenshots", async ({ page }) => {
  await goto_fixture(page, "canvas2d");
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const handle = chart.panes()[0].attach_primitive({
      text_views: (info) => [{
        text: "FRAME TEXT", x: info.pane_left + 75, y: info.pane_top + 80,
        font: "italic bold 24px Arial", color: "#ff00ff", baseline: "alphabetic",
      }],
    });
    const count_magenta = () => {
      const screenshot = chart.take_screenshot(false, false);
      const pixels = screenshot.getContext("2d").getImageData(0, 0, screenshot.width, screenshot.height).data;
      let magenta = 0;
      for (let index = 0; index < pixels.length; index += 4) {
        if (pixels[index] > 220 && pixels[index + 1] < 80 && pixels[index + 2] > 220 && pixels[index + 3] > 200) magenta++;
      }
      return magenta;
    };
    const painted = count_magenta();
    const cover = chart.panes()[0].attach_primitive({
      pane_views: () => [{
        z_order: "top",
        renderer(ctx) { ctx.rect(ctx.pane_left + 50, ctx.pane_top + 45, 260, 55, "#000000"); },
      }],
    });
    const covered = count_magenta();
    cover.detach();
    handle.detach();
    return { painted, covered };
  });
  expect(result.painted).toBeGreaterThan(30);
  expect(result.covered).toBe(0);
});

test("plugin text_views paint through the WebGPU presented frame", async ({ page }) => {
  await goto_fixture(page, "auto");
  expect(await page.evaluate(() => window.__chart.backend())).toBe("webgpu");
  await page.evaluate(() => {
    window.__frame_text_handle = window.__chart.panes()[0].attach_primitive({
      text_views: (info) => [{
        text: "GPU FRAME", x: info.pane_left + 75, y: info.pane_top + 80,
        font: "italic bold 24px Arial", color: "#ff00ff", baseline: "alphabetic",
      }],
    });
  });
  await settle_frames(page);
  const image = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));
  let magenta = 0;
  for (let index = 0; index < image.data.length; index += 4) {
    if (image.data[index] > 220 && image.data[index + 1] < 80 && image.data[index + 2] > 220 && image.data[index + 3] > 200) magenta++;
  }
  expect(magenta).toBeGreaterThan(30);
});
