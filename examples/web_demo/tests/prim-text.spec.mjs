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

// Primitive exercising `ctx.text` (the Prim::Text path): z-order probes (text over an opaque
// rect, and a later rect covering an earlier text — both inside ONE view, so paint order is the
// only thing that can show), a direct-on-pane pink run (the over-series + cache probes), and a
// right-aligned run. All positions are integer bitmap px so the raster phase is deterministic.
// Unique rasterizations: Aeris OVER + Aeris UNDER + Aeris CACHE + RIGHT = 4 (the doubled Aeris
// CACHE call shares one cache key). Colors are inlined: the factory is serialized into the page.
function text_primitive_factory() {
  return {
    pane_views: () => [
      {
        z_order: "normal",
        renderer(ctx) {
          const x = ctx.pane_left + 60;
          const y = ctx.pane_top + 60;
          // (c1) text ABOVE the rect painted just before it (same view).
          ctx.rect(x, y, 220, 44, "#2962ff");
          ctx.text(x + 10, y + 22, "Aeris OVER", { color: "#ffffff", size: 26, bold: true });
          // (c2) this text is COVERED by the rect painted after it (same view).
          ctx.text(x + 10, y + 78, "Aeris UNDER", { color: "#ffffff", size: 26, bold: true });
          ctx.rect(x, y + 56, 220, 44, "#2962ff");
          // (a)/(d) pink run straight on the pane; recorded twice — one rasterization.
          ctx.text(x + 10, y + 536, "Aeris CACHE", { color: "#c2185b", size: 24 });
          ctx.text(x + 10, y + 536, "Aeris CACHE", { color: "#c2185b", size: 24 });
          // align probe.
          ctx.text(x + 460, y + 536, "RIGHT", { color: "#1e88e5", size: 24, align: "right" });
        },
      },
    ],
  };
}

// Band layout in bitmap px (= screenshot px at the fixture's 1.5 ratio): mirrors the factory.
const BAND = { x: 60, y: 60, w: 220, h: 44 };
const OVER_BAND = { x: BAND.x, y: BAND.y, w: BAND.w, h: BAND.h };
const UNDER_BAND = { x: BAND.x, y: BAND.y + 56, w: BAND.w, h: BAND.h };
const OVER_ANCHOR = { x: BAND.x + 10, y: BAND.y + 22 };
const CACHE_BAND = { x: BAND.x, y: BAND.y + 512, w: 520, h: 48 };

async function goto_fixture(page, backend) {
  await page.goto(`/?runtimeTest=presentedFrame&backend=${backend}&forceFallbackAdapter=1`);
  await wait_for_chart(page);
  // This probe compares complete backend frames outside the text region. Keep its frame free of
  // grid rasterization differences; the reference-fidelity suite owns the explicit grid fixture.
  await page.evaluate(() => window.__chart.apply_options({
    grid: { vertLines: { visible: false }, horzLines: { visible: false } },
  }));
  await settle_frames(page);
}

async function attach_text_primitive(page) {
  await page.evaluate((factory_source) => {
    // eslint-disable-next-line no-eval
    const factory = eval(`(${factory_source})`);
    window.__text_primitive_handle = window.__chart.panes()[0].attach_primitive(factory());
  }, text_primitive_factory.toString());
  await settle_frames(page);
}

async function detach_text_primitive(page) {
  await page.evaluate(() => {
    window.__text_primitive_handle.detach();
    window.__text_primitive_handle = null;
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

// Strict diff with the per-pixel maximum channel delta (pixelmatch counts but hides magnitudes).
function analyze_diff(a, b) {
  expect([a.width, a.height]).toEqual([b.width, b.height]);
  let count = 0;
  let max_delta = 0;
  for (let i = 0; i < a.data.length; i += 4) {
    const delta = Math.max(
      Math.abs(a.data[i] - b.data[i]),
      Math.abs(a.data[i + 1] - b.data[i + 1]),
      Math.abs(a.data[i + 2] - b.data[i + 2]),
      Math.abs(a.data[i + 3] - b.data[i + 3]),
    );
    if (delta !== 0) {
      count += 1;
      max_delta = Math.max(max_delta, delta);
    }
  }
  return { count, max_delta };
}

// Count pixels inside `region` of `png` whose channels all pass `test(r, g, b, a)`.
function count_pixels(png, region, test_fn) {
  let count = 0;
  for (let y = region.y; y < region.y + region.h; y += 1) {
    for (let x = region.x; x < region.x + region.w; x += 1) {
      const i = (png.width * y + x) * 4;
      if (test_fn(png.data[i], png.data[i + 1], png.data[i + 2], png.data[i + 3])) count += 1;
    }
  }
  return count;
}

const near_white = (r, g, b) => r > 230 && g > 230 && b > 230;
const pinkish = (r, g, b) => r > 150 && g < 100 && b > 60 && b < 150;
const solid_blue = (r, g, b) => Math.abs(r - 0x29) <= 2 && Math.abs(g - 0x62) <= 2 && Math.abs(b - 0xff) <= 2;

test("prim text paints on both backends, is pixel-identical, z-orders, caches, and detaches", async ({ page }, test_info) => {
  const pixel_ratio = fixture.pixel_ratio;
  const pane_width = Math.round((fixture.css_width - fixture.price_axis_width) * pixel_ratio);
  const pane_height = Math.round((fixture.css_height - fixture.time_axis_height) * pixel_ratio);

  // ---- Canvas2D: baseline → attach → probes ----
  await goto_fixture(page, "canvas2d");
  expect(await page.evaluate(() => window.__chart.backend())).toBe("canvas2d");
  const canvas_before = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));
  await attach_text_primitive(page);
  const canvas_attached = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));

  // (a) The text bands actually painted.
  const over_diff = count_different(
    crop_png(canvas_before, OVER_BAND.x, OVER_BAND.y, OVER_BAND.w, OVER_BAND.h),
    crop_png(canvas_attached, OVER_BAND.x, OVER_BAND.y, OVER_BAND.w, OVER_BAND.h),
  );
  expect(over_diff, "OVER band must change where rect+text draw").toBeGreaterThan(0);
  const cache_diff = count_different(
    crop_png(canvas_before, CACHE_BAND.x, CACHE_BAND.y, CACHE_BAND.w, CACHE_BAND.h),
    crop_png(canvas_attached, CACHE_BAND.x, CACHE_BAND.y, CACHE_BAND.w, CACHE_BAND.h),
  );
  expect(cache_diff, "CACHE band must change where the pink text draws").toBeGreaterThan(0);

  // (a) Content probes at the text bounds: the white glyphs of "Aeris OVER" land exactly inside
  // the browser-measured ink box (same font spec the host builds: 700 26px <layout family>),
  // and nowhere else inside the blue band.
  const bounds = await page.evaluate(([size, weight]) => {
    const layout = window.__chart.options().layout;
    const measure = document.createElement("canvas").getContext("2d");
    measure.font = `${weight} ${size}px ${layout.fontFamily}`;
    measure.textBaseline = "middle";
    const m = measure.measureText("Aeris OVER");
    return {
      abl: m.actualBoundingBoxLeft,
      abr: m.actualBoundingBoxRight,
      asc: m.actualBoundingBoxAscent,
      desc: m.actualBoundingBoxDescent,
    };
  }, [26, 700]);
  const ink = {
    x0: OVER_ANCHOR.x - bounds.abl,
    x1: OVER_ANCHOR.x + bounds.abr,
    y0: OVER_ANCHOR.y - bounds.asc,
    y1: OVER_ANCHOR.y + bounds.desc,
  };
  const ink_white = count_pixels(
    canvas_attached,
    { x: Math.floor(ink.x0), y: Math.floor(ink.y0), w: Math.ceil(ink.x1) - Math.floor(ink.x0), h: Math.ceil(ink.y1) - Math.floor(ink.y0) },
    near_white,
  );
  expect(ink_white, "glyph ink must appear inside the measured text bounds").toBeGreaterThan(100);
  // Margin strips between the band edge and the ink box must stay solid blue (no stray glyphs).
  const margins = [
    { x: OVER_BAND.x + 2, y: OVER_BAND.y + 2, w: Math.floor(ink.x0) - OVER_BAND.x - 4, h: OVER_BAND.h - 4 },
    { x: Math.ceil(ink.x1) + 2, y: OVER_BAND.y + 2, w: OVER_BAND.x + OVER_BAND.w - Math.ceil(ink.x1) - 4, h: OVER_BAND.h - 4 },
    { x: Math.floor(ink.x0), y: OVER_BAND.y + 2, w: Math.ceil(ink.x1) - Math.floor(ink.x0), h: Math.floor(ink.y0) - OVER_BAND.y - 3 },
  ].filter((r) => r.w > 0 && r.h > 0);
  for (const margin of margins) {
    expect(
      count_pixels(canvas_attached, margin, (r, g, b) => !solid_blue(r, g, b)),
      `margin ${JSON.stringify(margin)} must stay solid band blue`,
    ).toBe(0);
  }

  // (c2) Z-order within the layer: the UNDER band's earlier text is covered by the later rect.
  expect(
    count_pixels(canvas_attached, UNDER_BAND, near_white),
    "the later rect must fully cover the earlier text (prim order preserved)",
  ).toBe(0);
  expect(count_pixels(canvas_attached, UNDER_BAND, solid_blue)).toBeGreaterThan(UNDER_BAND.w * UNDER_BAND.h * 0.95);

  // (c1) Text above the series: candle pixels inside the pink text's band become text-colored.
  const candle_probe = { x: CACHE_BAND.x + 8, y: CACHE_BAND.y + 8, w: 200, h: CACHE_BAND.h - 16 };
  const not_background = (r, g, b) => !(r > 235 && g > 235 && b > 235);
  const candle_px_before = count_pixels(canvas_before, candle_probe, not_background);
  expect(candle_px_before, "fixture must have series pixels under the pink text for the probe").toBeGreaterThan(0);
  // After painting, the pink glyphs sit ON TOP: pink pixels appear where the probe had content.
  expect(count_pixels(canvas_attached, candle_probe, pinkish)).toBeGreaterThan(50);

  // (c3) Z-order vs engine chrome: the engine emits the crosshair lines at the end of the
  // pane's `main` layer and plugin normal-layer prims append after it (FramePane.top_prims is
  // the above-crosshair slot), so house order paints the primitive's text ABOVE the crosshair
  // line — identical on both backends because they share one frame. Hover into the band: the
  // line column inside the band keeps the band/text pixels (the line is only visible above
  // and below the band); the axis strips, a separate overlay composited above the pane, stay
  // untouched (text is structurally below the axis chrome).
  await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 0 } }));
  await settle_frames(page);
  // The first pointer move of a session doesn't paint the crosshair (warm-up needed; the
  // same pattern the hit-testing specs rely on), so prime the hover path before probing.
  await page.mouse.move(500, 300);
  await settle_frames(page);
  const hover_bitmap_x = OVER_ANCHOR.x + 30; // inside the measured "Aeris OVER" ink
  await page.mouse.move(hover_bitmap_x / pixel_ratio, (OVER_ANCHOR.y + 1) / pixel_ratio);
  await settle_frames(page);
  const canvas_hover = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));
  const line_column = { x: hover_bitmap_x - 1, y: OVER_BAND.y + 2, w: 3, h: OVER_BAND.h - 4 };
  // This fixture uses the light theme, whose border-derived crosshair raster is near black.
  const crosshairish = (r, g, b) => r < 80 && g < 80 && b < 80;
  expect(
    count_pixels(canvas_hover, line_column, crosshairish),
    "house order: primitive main-layer text paints above the crosshair line (main-layer end)",
  ).toBe(0);
  // Sanity that the crosshair is actually hovering (line visible just below the band).
  const line_below = { x: hover_bitmap_x - 1, y: UNDER_BAND.y + UNDER_BAND.h + 8, w: 3, h: 40 };
  expect(count_pixels(canvas_hover, line_below, crosshairish)).toBeGreaterThan(0);
  // Axis strips are a separate overlay above the pane; in-pane text never reaches them.
  const price_axis_diff = count_different(
    crop_png(canvas_before, pane_width, 0, canvas_before.width - pane_width, pane_height),
    crop_png(canvas_attached, pane_width, 0, canvas_attached.width - pane_width, pane_height),
  );
  expect(price_axis_diff, "price axis strip must be untouched by pane text").toBe(0);
  const time_axis_diff = count_different(
    crop_png(canvas_before, 0, pane_height, pane_width, canvas_before.height - pane_height),
    crop_png(canvas_attached, 0, pane_height, pane_width, canvas_attached.height - pane_height),
  );
  expect(time_axis_diff, "time axis strip must be untouched by pane text").toBe(0);
  // Drop the crosshair back out (pointerleave hides it, reference `mouseLeaveEvent`) so the
  // detach probe compares like with like.
  await page.evaluate(() => {
    for (const canvas of document.querySelectorAll("canvas")) {
      canvas.dispatchEvent(new PointerEvent("pointerleave", { pointerType: "mouse" }));
    }
  });
  await settle_frames(page);

  // (e) Detach restores the exact prior pixels.
  await detach_text_primitive(page);
  const canvas_restored = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));
  expect(count_different(canvas_before, canvas_restored)).toBe(0);

  // ---- WebGPU: same chart + same primitive ----
  await goto_fixture(page, "auto");
  expect(await page.evaluate(() => window.__chart.backend())).toBe("webgpu");
  const gpu_before = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));
  const cache_before = await page.evaluate(() => JSON.parse(window.__chart.wasm.text_cache_debug()));
  await attach_text_primitive(page);
  const gpu_attached = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));

  // (a) The GPU backend paints the same probes: white ink in the OVER band, pink in the CACHE band.
  expect(count_pixels(gpu_attached, { x: Math.floor(ink.x0), y: Math.floor(ink.y0), w: Math.ceil(ink.x1) - Math.floor(ink.x0), h: Math.ceil(ink.y1) - Math.floor(ink.y0) }, near_white)).toBeGreaterThan(100);
  expect(count_pixels(gpu_attached, candle_probe, pinkish)).toBeGreaterThan(50);
  expect(count_pixels(gpu_attached, UNDER_BAND, near_white)).toBe(0);

  // (b) Cross-backend text parity. Both backends rasterize the same run with the same browser
  // font/AA (Canvas2D `fillText` direct; WebGPU via the offscreen-atlas quad), so glyph shapes
  // and placement are identical — nothing is shifted by a whole pixel. The measured strict
  // residual is confined to per-channel ±1/255 rounding on AA edge pixels (the documented
  // sp=0 premultiplied-blend class, blend.rs; the doubled Aeris CACHE call blends twice, so
  // its AA edges carry the residual twice). Current Windows SwiftShader measurement: 176
  // strict-diff pixels, EVERY differing channel exactly ±1.
  const text_region = { x: OVER_BAND.x - 4, y: OVER_BAND.y - 4, w: CACHE_BAND.w + 8, h: CACHE_BAND.y + CACHE_BAND.h - OVER_BAND.y + 8 };
  const text_analysis = analyze_diff(
    crop_png(gpu_attached, text_region.x, text_region.y, text_region.w, text_region.h),
    crop_png(canvas_attached, text_region.x, text_region.y, text_region.w, text_region.h),
  );
  if (text_analysis.count !== 0) {
    await test_info.attach("webgpu-text.png", { body: PNG.sync.write(crop_png(gpu_attached, text_region.x, text_region.y, text_region.w, text_region.h)), contentType: "image/png" });
    await test_info.attach("canvas2d-text.png", { body: PNG.sync.write(crop_png(canvas_attached, text_region.x, text_region.y, text_region.w, text_region.h)), contentType: "image/png" });
  }
  console.log(`text-region parity: ${text_analysis.count} strict-diff px, max channel delta ${text_analysis.max_delta}/255`);
  expect(text_analysis.count, "text-region residual must stay in the measured ±1 class").toBeLessThanOrEqual(200);
  expect(text_analysis.max_delta, "no pixel may differ by more than 1/255 (no whole-pixel shifts)").toBeLessThanOrEqual(1);

  // The primitive must not introduce any cross-backend differences outside its text region.
  // Compare against the baseline backend residual so ordinary candle rasterization differences
  // do not get misattributed to the primitive.
  const frame_diff = count_different(gpu_attached, canvas_attached);
  const baseline_frame_diff = count_different(gpu_before, canvas_before);
  const baseline_region_diff = count_different(
    crop_png(gpu_before, text_region.x, text_region.y, text_region.w, text_region.h),
    crop_png(canvas_before, text_region.x, text_region.y, text_region.w, text_region.h),
  );
  const outside_text_diff = frame_diff - text_analysis.count;
  const baseline_outside_text_diff = baseline_frame_diff - baseline_region_diff;
  if (outside_text_diff !== baseline_outside_text_diff) {
    await test_info.attach("webgpu.png", { body: PNG.sync.write(gpu_attached), contentType: "image/png" });
    await test_info.attach("canvas2d.png", { body: PNG.sync.write(canvas_attached), contentType: "image/png" });
  }
  expect(outside_text_diff, "the primitive must not change the baseline outside its text region").toBe(baseline_outside_text_diff);

  // (d) Cache: the four unique runs rasterized once; the doubled Aeris CACHE call shared one
  // entry; further frames hit the cache (no re-rasterization).
  const stats = await page.evaluate(() => JSON.parse(window.__chart.wasm.text_cache_debug()));
  expect(stats.rasterizations - cache_before.rasterizations, "4 unique runs → 4 rasterizations (duplicate shares one entry)").toBe(4);
  expect(stats.entries - cache_before.entries).toBe(4);
  await page.evaluate(() => window.__chart.wasm.render());
  await settle_frames(page);
  const stats_after = await page.evaluate(() => JSON.parse(window.__chart.wasm.text_cache_debug()));
  expect(stats_after.rasterizations, "steady-state frames must hit the cache").toBe(stats.rasterizations);
});
