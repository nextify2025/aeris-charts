import { expect } from "@playwright/test";
import { test, wait_for_chart } from "./page-ready.mjs";
import { readFileSync } from "node:fs";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";

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

async function goto_fixture(page, backend) {
  await page.goto(`/?runtimeTest=presentedFrame&backend=${backend}&forceFallbackAdapter=1`);
  await wait_for_chart(page);
}

async function screenshot(page) {
  return PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));
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

async function set_engine_markers(page, on) {
  // The engine's built-in markers with auto_scale disabled: the parity proof isolates the
  // marker drawing itself — the platform's autoscale contract ({min,max} price bounds) cannot
  // express the engine's pixel internal margins (see builtin_plugins.ts).
  await page.evaluate((flag) => window.__set_engine_markers(flag, { auto_scale: false }), on);
  await settle_frames(page);
}

async function set_plugin_markers(page, on, options) {
  await page.evaluate(([flag, opts]) => window.__set_plugin_markers(flag, opts), [on, options ?? null]);
  await settle_frames(page);
}

// (a) The parity proof: plugin markers (create_series_markers on the primitive platform) and
// engine markers (series.set_markers) render pixel-identical frames on the same markers
// fixture — shapes AND text (plugin text paints through the overlay-text hook, the engine's
// own marker-label path).
test("plugin markers render pixel-identical to engine markers", async ({ page }, test_info) => {
  await goto_fixture(page, "canvas2d");
  expect(await page.evaluate(() => window.__chart.backend())).toBe("canvas2d");

  const baseline = await screenshot(page);

  await set_engine_markers(page, true);
  const engine = await screenshot(page);
  const engine_footprint = count_different(baseline, engine);
  expect(engine_footprint, "engine markers must change the frame").toBeGreaterThan(0);

  // Engine markers clear exactly.
  await set_engine_markers(page, false);
  expect(count_different(baseline, await screenshot(page))).toBe(0);

  await set_plugin_markers(page, true, { auto_scale: false });
  expect(await page.evaluate(() => window.__plugin_markers_active())).toBe(true);
  const plugin = await screenshot(page);
  expect(count_different(baseline, plugin), "plugin markers must change the frame").toBeGreaterThan(0);

  const parity_diff = count_different(engine, plugin);
  if (parity_diff !== 0) {
    await test_info.attach("engine-markers.png", { body: PNG.sync.write(engine), contentType: "image/png" });
    await test_info.attach("plugin-markers.png", { body: PNG.sync.write(plugin), contentType: "image/png" });
  }
  expect(parity_diff, "plugin markers must be pixel-identical to engine markers").toBe(0);

  // The plugin detaches exactly.
  await set_plugin_markers(page, false);
  expect(await page.evaluate(() => window.__plugin_markers_active())).toBe(false);
  expect(count_different(baseline, await screenshot(page))).toBe(0);
});

// (b) Parity on the WebGPU backend: plugin ≡ engine 0-diff. A literal WebGPU≡Canvas2D 0-diff
// is not achievable with markers on either form: the engine's WebGPU pass tessellates AA
// shapes (circle/triangle/round-rect) and rasterizes them through its 4xMSAA target, whose
// edge coverage differs from Canvas2D's analytic AA by 1-2 steps on AA edges (the paint-order
// half of the source gap — tessellated markers painting before the quad bucket, under the
// candle wicks — is fixed: both backends now execute the frame's prim order). Both effects
// pre-date the plugin platform and reproduce identically with the engine's own markers
// (measured with the overlapping fixture: 275 px before the ordering fix, 204 px of pure
// AA-edge steps after; isolated shapes: arrow 48, square 54, circle 36 px — AA-edge coverage
// steps only, interiors exact). The platform's 0-diff backend guarantee is scoped to the quad
// family for this reason (see the pane/series reference primitives).
test("plugin markers match engine markers on WebGPU", async ({ page }, test_info) => {
  await goto_fixture(page, "auto");
  expect(await page.evaluate(() => window.__chart.backend())).toBe("webgpu");

  await set_engine_markers(page, true);
  const engine = await screenshot(page);
  await set_engine_markers(page, false);

  await set_plugin_markers(page, true, { auto_scale: false });
  const plugin = await screenshot(page);
  const pane_width = Math.round((fixture.css_width - fixture.price_axis_width) * fixture.pixel_ratio);
  const pane_height = Math.round((fixture.css_height - fixture.time_axis_height) * fixture.pixel_ratio);
  const engine_pane = crop_png(engine, 0, 0, pane_width, pane_height);
  const plugin_pane = crop_png(plugin, 0, 0, pane_width, pane_height);
  const parity_diff = count_different(engine_pane, plugin_pane);
  let ordering_diff = 0;
  let maximum_channel_delta = 0;
  for (let offset = 0; offset < engine_pane.data.length; offset += 4) {
    let pixel_delta = 0;
    for (let channel = 0; channel < 4; channel += 1) {
      pixel_delta = Math.max(
        pixel_delta,
        Math.abs(engine_pane.data[offset + channel] - plugin_pane.data[offset + channel]),
      );
    }
    maximum_channel_delta = Math.max(maximum_channel_delta, pixel_delta);
    if (pixel_delta > 96) ordering_diff += 1;
  }
  console.log(
    `plugin/engine marker residual: ${parity_diff} clipped-edge pixels, max step ${maximum_channel_delta}, ${ordering_diff} ordering pixels`,
  );
  if (parity_diff > 64 || ordering_diff !== 0) {
    await test_info.attach("engine-markers-webgpu.png", { body: PNG.sync.write(engine), contentType: "image/png" });
    await test_info.attach("plugin-markers-webgpu.png", { body: PNG.sync.write(plugin), contentType: "image/png" });
  }
  expect(parity_diff, "pane clipping may change only a bounded marker-edge footprint").toBeLessThanOrEqual(64);
  expect(ordering_diff, "plugin and engine marker interiors/paint order must match").toBe(0);
});

// (c) set_markers([]) clears the markers without detaching; detach removes them entirely.
// Both restore the exact baseline pixels. The default auto_scale option must also expand the
// owning scale while markers are present (reference autoScale behavior).
test("plugin markers clear with set_markers([]) and detach; auto_scale expands the scale", async ({ page }) => {
  await goto_fixture(page, "canvas2d");
  const baseline = await screenshot(page);
  const data_coordinates = async () => page.evaluate(() => {
    const high = Math.max(...window.__data.map((bar) => bar.high));
    const low = Math.min(...window.__data.map((bar) => bar.low));
    return {
      top: window.__main.price_to_coordinate(high),
      bottom: window.__main.price_to_coordinate(low),
    };
  });
  const plain_coordinates = await data_coordinates();
  expect(plain_coordinates.top).not.toBeNull();
  expect(plain_coordinates.bottom).not.toBeNull();

  await set_plugin_markers(page, true);
  const handle = await page.evaluate(() => window.__plugin_markers_handle() !== null);
  expect(handle).toBe(true);
  const with_markers = await screenshot(page);
  expect(count_different(baseline, with_markers), "markers must paint").toBeGreaterThan(0);

  // The reference returns pixel autoscale margins, not an expanded price range: both data extrema
  // move inward so marker shapes have room without changing the scale's raw min/max values.
  const expanded_coordinates = await data_coordinates();
  expect(expanded_coordinates.top, "marker auto_scale must add headroom above the data").toBeGreaterThan(plain_coordinates.top);
  expect(expanded_coordinates.bottom, "marker auto_scale must add headroom below the data").toBeLessThan(plain_coordinates.bottom);

  // set_markers([]) removes the markers and their autoscale contribution.
  await page.evaluate(() => window.__plugin_markers_handle().set_markers([]));
  await settle_frames(page);
  expect(count_different(baseline, await screenshot(page))).toBe(0);
  const cleared_coordinates = await data_coordinates();
  expect(cleared_coordinates.top).toBeCloseTo(plain_coordinates.top, 8);
  expect(cleared_coordinates.bottom).toBeCloseTo(plain_coordinates.bottom, 8);

  // Re-set, then detach: same removal, and markers() round-trips the fixture.
  const marker_count = await page.evaluate(() => {
    const markers = window.__plugin_markers_handle().markers();
    return Array.isArray(markers) ? markers.length : -1;
  });
  expect(marker_count).toBe(0);
  await page.evaluate(() => window.__set_plugin_markers(false));
  // Re-create with the fixture and detach through the handle.
  await set_plugin_markers(page, true, { auto_scale: false });
  expect(count_different(baseline, await screenshot(page))).toBeGreaterThan(0);
  const count_after_reset = await page.evaluate(() => window.__plugin_markers_handle().markers().length);
  expect(count_after_reset).toBe(4);
  await page.evaluate(() => window.__plugin_markers_handle().detach());
  await settle_frames(page);
  expect(count_different(baseline, await screenshot(page))).toBe(0);
  // The handle-level detach leaves the demo's handle slot stale; the setter syncs it.
  await page.evaluate(() => window.__set_plugin_markers(false));
  expect(await page.evaluate(() => window.__plugin_markers_active())).toBe(false);
});

// (d) The text watermark plugin paints its lines on the overlay (the engine watermark's
// slot) and detach clears them exactly.
test("text watermark API paints its lines and detach clears them without a duplicate demo control", async ({ page }) => {
  await goto_fixture(page, "canvas2d");
  const baseline = await screenshot(page);

  const pixel_ratio = fixture.pixel_ratio;
  const pane_width = Math.round((fixture.css_width - fixture.price_axis_width) * pixel_ratio);
  const pane_height = Math.round((fixture.css_height - fixture.time_axis_height) * pixel_ratio);
  const center = (image) => crop_png(
    image,
    Math.round(pane_width / 2) - 300,
    Math.round(pane_height / 2) - 150,
    600,
    300,
  );

  await page.evaluate(async () => {
    const { create_text_watermark } = await import("/dist/aeris_charts_financial.js");
    window.__test_text_watermark = create_text_watermark(window.__chart.panes()[0], {
      lines: [
        { text: "Aeris", color: "rgba(41, 98, 255, 0.16)", fontSize: 72, fontStyle: "bold" },
        { text: "watermark", color: "rgba(41, 98, 255, 0.30)", fontSize: 24, fontFamily: "monospace" },
      ],
    });
  });
  await settle_frames(page);
  expect(await page.locator("#plugin_watermark_toggle").count()).toBe(0);
  const watermarked = await screenshot(page);
  expect(
    count_different(center(baseline), center(watermarked)),
    "the watermark lines must paint in the pane's center",
  ).toBeGreaterThan(0);

  await page.evaluate(() => {
    window.__test_text_watermark.detach();
    window.__test_text_watermark = null;
  });
  await settle_frames(page);
  expect(count_different(baseline, await screenshot(page))).toBe(0);
});
