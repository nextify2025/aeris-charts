import { expect } from "@playwright/test";
import { test, monitor_page, wait_for_chart } from "./page-ready.mjs";
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

// The ported reference vertical-line plugin (fixture_canvas_plugin.js), toggled through the
// demo's own checkbox path.
async function set_vert_line(page, on) {
  await page.evaluate((flag) => window.__set_vert_line(flag), on);
  await settle_frames(page);
}

// A canvas primitive drawing in MEDIA space — the `useMediaCoordinateSpace` counterpart to the
// ported plugin's bitmap-space renderer. Draws a known rect (see the DPR test's pixel probes).
function media_rect_primitive_factory() {
  return {
    pane_views: () => [
      {
        renderer(target) {
          target.useMediaCoordinateSpace((scope) => {
            const ctx = scope.context;
            ctx.fillStyle = "#2962ff";
            ctx.fillRect(120.5, 60, 40, 30);
          });
        },
      },
    ],
  };
}

async function attach_media_rect_primitive(page) {
  await page.evaluate((factory_source) => {
    // eslint-disable-next-line no-eval
    const factory = eval(`(${factory_source})`);
    window.__media_rect_handle = window.__chart.panes()[0].attach_canvas_primitive(factory());
  }, media_rect_primitive_factory.toString());
  await settle_frames(page);
}

// (a) Attaching the ported reference plugin paints on BOTH backends (pane-region diff > 0).
// (b) Detaching removes the paint (0-diff vs the same-backend baseline).
test("canvas primitive paints on WebGPU and Canvas2D, and detach restores the baseline", async ({ page }) => {
  const pixel_ratio = fixture.pixel_ratio;
  const pane_width = Math.round((fixture.css_width - fixture.price_axis_width) * pixel_ratio);
  const pane_height = Math.round((fixture.css_height - fixture.time_axis_height) * pixel_ratio);

  for (const backend of ["canvas2d", "auto"]) {
    await page.goto(`/?runtimeTest=presentedFrame&backend=${backend}&forceFallbackAdapter=1`);
    await wait_for_chart(page);
    if (backend === "auto") {
      expect(await page.evaluate(() => window.__chart.backend())).toBe("webgpu");
    } else {
      expect(await page.evaluate(() => window.__chart.backend())).toBe("canvas2d");
    }
    const baseline = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));

    await set_vert_line(page, true);
    expect(await page.evaluate(() => window.__vert_line_active())).toBe(true);
    const attached = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));
    const pane_diff = count_different(
      crop_png(baseline, 0, 0, pane_width, pane_height),
      crop_png(attached, 0, 0, pane_width, pane_height),
    );
    expect(pane_diff, `pane region must change where the canvas primitive draws (${backend})`).toBeGreaterThan(0);
    expect(count_different(
      crop_png(baseline, pane_width, 0, baseline.width - pane_width, pane_height),
      crop_png(attached, pane_width, 0, attached.width - pane_width, pane_height),
    ), `canvas primitive must not cover the price axis (${backend})`).toBe(0);
    expect(count_different(
      crop_png(baseline, 0, pane_height, pane_width, baseline.height - pane_height),
      crop_png(attached, 0, pane_height, pane_width, attached.height - pane_height),
    ), `canvas primitive must not cover the time axis (${backend})`).toBe(0);

    await set_vert_line(page, false);
    expect(await page.evaluate(() => window.__vert_line_active())).toBe(false);
    const restored = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));
    expect(count_different(baseline, restored), `detach must restore the exact prior pixels (${backend})`).toBe(0);
  }
});

// (c) DPR 2 render has no blur/offset: sample the plugin canvas backing store at the known
// coordinates both coordinate spaces must land on.
test("canvas primitive renders crisp at DPR 2 in bitmap and media space", async ({ browser }) => {
  const context = await browser.newContext({
    viewport: { width: fixture.css_width, height: fixture.css_height },
    deviceScaleFactor: 2,
    colorScheme: "light",
  });
  const page = await context.newPage();
  page.once("close", monitor_page(page));
  page.on("console", (message) => console.log(`[browser:${message.type()}] ${message.text()}`));
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d&forceFallbackAdapter=1&dpr=2");
  await wait_for_chart(page);

  await set_vert_line(page, true);
  await attach_media_rect_primitive(page);

  const probe = await page.evaluate(() => {
    const chart = window.__chart;
    const time = window.__data[Math.floor(window.__data.length / 2)].time;
    const x_media = chart.time_scale().time_to_coordinate(time);
    const canvas = window.__plugin_canvas();
    const rect = canvas.getBoundingClientRect();
    const ctx = canvas.getContext("2d");
    const hpr = canvas.width / rect.width;
    // reference positionsLine(x_media, hpr, 3): the exact bitmap span the ported renderer fills.
    const line_width = Math.round(3 * hpr);
    const line_position = Math.round(hpr * x_media) - Math.floor(line_width * 0.5);
    const read_row = (y, x0, x1) => {
      const out = [];
      const data = ctx.getImageData(x0, y, x1 - x0, 1).data;
      for (let offset = 0; offset < data.length; offset += 4) {
        out.push([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]);
      }
      return out;
    };
    return {
      canvas_size: [canvas.width, canvas.height],
      hpr,
      line_width,
      line_position,
      // Three rows down the pane (top, middle, bottom of the pane strip at DPR 2).
      line_rows: [50, 700, 1300].map((y) => read_row(y, line_position - 2, line_position + line_width + 2)),
      // The media-space rect (120.5, 60, 40, 30) at DPR 2 → bitmap (241, 120, 80, 60).
      media_rect: {
        left_edge: read_row(130, 239, 244),
        right_edge: read_row(130, 319, 324),
        top_edge: ctx.getImageData(250, 118, 1, 5).data.join(","),
      },
    };
  });

  expect(probe.canvas_size).toEqual([Math.round(fixture.css_width * 2), Math.round(fixture.css_height * 2)]);
  expect(probe.hpr).toBe(2);
  expect(probe.line_width).toBe(6);
  // No blur and no offset: transparent outside the exact span, fully opaque #e91e63 inside it,
  // on every sampled row (a half-pixel smear would show up as a partial alpha at either edge).
  for (const [row_index, row] of probe.line_rows.entries()) {
    expect(row[0], `line row ${row_index}: two px before the span must be empty`).toEqual([0, 0, 0, 0]);
    expect(row[1], `line row ${row_index}: one px before the span must be empty`).toEqual([0, 0, 0, 0]);
    for (let i = 2; i < 2 + probe.line_width; i += 1) {
      expect(row[i], `line row ${row_index}: px ${i - 2} inside the span must be solid`).toEqual([233, 30, 99, 255]);
    }
    expect(row[2 + probe.line_width], `line row ${row_index}: one px after the span must be empty`).toEqual([0, 0, 0, 0]);
    expect(row[2 + probe.line_width + 1], `line row ${row_index}: two px after the span must be empty`).toEqual([0, 0, 0, 0]);
  }
  // Media space: (120.5 css, 60 css) → bitmap (241, 120) exactly; 40 css → 80 bitmap px wide.
  expect(probe.media_rect.left_edge[0]).toEqual([0, 0, 0, 0]); // x = 239
  expect(probe.media_rect.left_edge[1]).toEqual([0, 0, 0, 0]); // x = 240
  expect(probe.media_rect.left_edge[2]).toEqual([41, 98, 255, 255]); // x = 241
  expect(probe.media_rect.left_edge[3]).toEqual([41, 98, 255, 255]); // x = 242
  expect(probe.media_rect.right_edge[0]).toEqual([41, 98, 255, 255]); // x = 319
  expect(probe.media_rect.right_edge[1]).toEqual([41, 98, 255, 255]); // x = 320
  expect(probe.media_rect.right_edge[2]).toEqual([0, 0, 0, 0]); // x = 321
  expect(probe.media_rect.right_edge[3]).toEqual([0, 0, 0, 0]); // x = 322
  // y: 60 css → 120 bitmap; rows 118/119 empty, 120/121 filled.
  const top_column = probe.media_rect.top_edge.split(",").map(Number);
  expect(top_column.slice(0, 8)).toEqual([0, 0, 0, 0, 0, 0, 0, 0]);
  expect(top_column.slice(8, 16)).toEqual([41, 98, 255, 255, 41, 98, 255, 255]);

  await context.close();
});

// (d) Resize repaints correctly: the auto-resize observer path (viewport change) and a manual
// `chart.resize` both leave the primitive painted at the recomputed geometry.
test("canvas primitive repaints after auto-resize and chart.resize", async ({ page }) => {
  // ---- autoSize path (no runtimeTest → the demo enables autoSize) ----
  await page.goto("/?backend=canvas2d");
  await wait_for_chart(page);
  // This geometry fixture needs the source range in view. The interactive demo deliberately no
  // longer auto-fits at startup, so the test requests that viewport explicitly.
  await page.evaluate(() => window.__chart.time_scale().fit_content());
  await settle_frames(page);
  await set_vert_line(page, true);

  const painted_alpha = () => page.evaluate(() => {
    const canvas = window.__plugin_canvas();
    const ctx = canvas.getContext("2d");
    const data = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
    let painted = 0;
    for (let offset = 3; offset < data.length; offset += 4) {
      if (data[offset] !== 0) painted += 1;
    }
    return { painted, width: canvas.width, height: canvas.height };
  });

  const before = await painted_alpha();
  expect(before.painted, "the ported plugin must paint on the plugin canvas").toBeGreaterThan(0);

  // The container tracks the viewport; the engine's ResizeObserver re-renders and the package's
  // observer re-runs the canvas pass on the new size.
  await page.setViewportSize({ width: 900, height: 600 });
  await page.waitForFunction((old_width) => window.__plugin_canvas().width !== old_width, before.width);
  await settle_frames(page);
  const shrunk = await painted_alpha();
  expect(shrunk.width).toBeLessThan(before.width);
  expect(shrunk.painted, "the plugin must repaint after the auto-resize").toBeGreaterThan(0);
  // The plugin canvas backing store tracks the engine-sized overlay exactly.
  const canvas_sizes = await page.evaluate(() => {
    const canvases = document.querySelectorAll("#chart_container canvas");
    return { plugin: canvases[2].width, overlay: canvases[3].width };
  });
  expect(canvas_sizes.plugin).toBe(canvas_sizes.overlay);

  // ---- manual chart.resize path (fixture page, autoSize off) ----
  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d&forceFallbackAdapter=1");
  await wait_for_chart(page);
  await set_vert_line(page, true);
  expect((await painted_alpha()).painted).toBeGreaterThan(0);
  await page.evaluate(() => window.__chart.resize(900, 600, 1.5));
  await settle_frames(page);
  const resized = await painted_alpha();
  expect(resized.painted, "the plugin must repaint after chart.resize").toBeGreaterThan(0);
  const manual_sizes = await page.evaluate(() => [...document.querySelectorAll("#chart_container canvas")].map((canvas) => ({
    bitmap: [canvas.width, canvas.height],
    css: [canvas.getBoundingClientRect().width, canvas.getBoundingClientRect().height],
  })));
  expect(manual_sizes).toEqual(Array.from({ length: 4 }, () => ({
    bitmap: [1350, 900],
    css: [900, 600],
  })));
  // The line follows the resized time scale: probe the column the converter now reports.
  const column = await page.evaluate(() => {
    const chart = window.__chart;
    const time = window.__data[Math.floor(window.__data.length / 2)].time;
    const x_media = chart.time_scale().time_to_coordinate(time);
    const canvas = window.__plugin_canvas();
    const ctx = canvas.getContext("2d");
    const hpr = canvas.width / canvas.getBoundingClientRect().width;
    const line_width = Math.round(3 * hpr);
    const position = Math.round(hpr * x_media) - Math.floor(line_width * 0.5);
    const data = ctx.getImageData(position, 0, line_width, canvas.height).data;
    let painted = 0;
    for (let offset = 3; offset < data.length; offset += 4) {
      if (data[offset] !== 0) painted += 1;
    }
    return { painted, position };
  });
  expect(column.painted, "the line must paint at the post-resize coordinate").toBeGreaterThan(0);
});

// Lifecycle: attached/update_all_views/detached fire in order, and a throwing renderer is
// contained (the chart and the other views still paint).
test("canvas primitive lifecycle hooks fire and renderer errors are contained", async ({ page }) => {
  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d&forceFallbackAdapter=1");
  await wait_for_chart(page);
  const calls = await page.evaluate(() => {
    window.__calls = [];
    const record = (name) => window.__calls.push(name);
    const handle = window.__chart.panes()[0].attach_canvas_primitive({
      attached(params) { record(`attached:${params.pane_index}`); },
      detached() { record("detached"); },
      update_all_views() { record("update_all_views"); },
      pane_views() {
        record("pane_views");
        return [
          { renderer() { throw new Error("boom"); } },
          {
            z_order: "top",
            renderer(target) {
              target.useBitmapCoordinateSpace((scope) => {
                scope.context.fillStyle = "#e91e63";
                scope.context.fillRect(10, 10, 20, 20);
              });
            },
          },
        ];
      },
    });
    window.__lifecycle_handle = handle;
    return window.__calls;
  });
  expect(calls[0]).toBe("attached:0");
  expect(calls).toContain("update_all_views");
  expect(calls).toContain("pane_views");
  // The throwing view did not take the later `top` view down with it.
  const painted = await page.evaluate(() => {
    const canvas = window.__plugin_canvas();
    const data = canvas.getContext("2d").getImageData(10, 10, 20, 20).data;
    return data[3];
  });
  expect(painted).toBe(255);
  const after_detach = await page.evaluate(() => {
    window.__lifecycle_handle.detach();
    return window.__calls;
  });
  expect(after_detach[after_detach.length - 1]).toBe("detached");
  // Detach cleared the paint.
  const cleared = await page.evaluate(() => {
    const canvas = window.__plugin_canvas();
    const data = canvas.getContext("2d").getImageData(10, 10, 20, 20).data;
    return data[3];
  });
  expect(cleared).toBe(0);
});
