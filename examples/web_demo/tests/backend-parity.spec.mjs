import { test, expect } from "@playwright/test";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";

const fixture = JSON.parse(readFileSync(new URL("../fixtures/d1/candles.json", import.meta.url), "utf8"));
const reference_baseline = JSON.parse(readFileSync(new URL("../fixtures/d1/reference-baseline.json", import.meta.url), "utf8"));
const reference_matrix = JSON.parse(readFileSync(new URL("../fixtures/d1/reference-matrix.json", import.meta.url), "utf8"));
const reference_features = JSON.parse(readFileSync(new URL("../fixtures/d1/reference-features.json", import.meta.url), "utf8"));
const repository_root = fileURLToPath(new URL("../../..", import.meta.url));
const test_port = Number.parseInt(process.env.AERIS_CHARTS_TEST_PORT ?? "4174", 10);
const test_base_url = `http://127.0.0.1:${test_port}`;

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

// Determinism hardening for pixel comparisons: resolve pending font loads, then run one
// full paint through the real render path and discard it. First-use shaping, atlas upload,
// and backend warmup otherwise leak into the measured capture depending on process history.
async function settle_page(page) {
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ animations: "disabled", fullPage: false });
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

async function capture_presented_frame(page, backend, extra_query = "") {
  await page.goto(`/?runtimeTest=presentedFrame&backend=${backend}&forceFallbackAdapter=1${extra_query}`);
  await wait_for_chart(page);
  return {
    backend: await page.evaluate(() => window.__chart.backend()),
    png: await page.screenshot({ animations: "disabled", fullPage: false }),
  };
}

function render_native_fixture(output) {
  const args = process.platform === "win32"
    ? ["+stable-x86_64-pc-windows-msvc", "run", "-p", "aeris_charts_native", "--example", "parity_fixture", "--", output]
    : ["run", "-p", "aeris_charts_native", "--example", "parity_fixture", "--", output];
  const result = spawnSync("cargo", args, { cwd: repository_root, encoding: "utf8" });
  expect(result.status, `native fixture failed\n${result.stdout}\n${result.stderr}`).toBe(0);
}

function rgba_diff(a, b, tolerance) {
  let different_pixels = 0;
  let maximum_channel_delta = 0;
  let absolute_channel_delta = 0;
  for (let offset = 0; offset < a.length; offset += 4) {
    let pixel_delta = 0;
    for (let channel = 0; channel < 4; channel += 1) {
      const delta = Math.abs(a[offset + channel] - b[offset + channel]);
      pixel_delta = Math.max(pixel_delta, delta);
      maximum_channel_delta = Math.max(maximum_channel_delta, delta);
      absolute_channel_delta += delta;
    }
    if (pixel_delta > tolerance) different_pixels += 1;
  }
  return {
    different_pixels,
    maximum_channel_delta,
    mean_absolute_channel_delta: absolute_channel_delta / a.length,
  };
}

function crop_png(source, x, y, width, height) {
  const output = new PNG({ width, height });
  PNG.bitblt(source, output, x, y, width, height, 0, 0);
  return output;
}

test("scaled four-color image matches WebGPU, Canvas2D, and native", async ({ browser }, test_info) => {
  const context = await browser.newContext({ viewport: { width: 120, height: 120 }, deviceScaleFactor: 1 });
  const page = await context.newPage();
  page.on("console", (message) => console.log(`[image-browser:${message.type()}] ${message.text()}`));
  page.on("pageerror", (error) => console.log(`[image-browser:pageerror] ${error.message}`));
  await page.goto("/");
  const capture = async (backend) => {
    const actual_backend = await page.evaluate(async (requested_backend) => {
      const api = await import("/dist/aeris_charts_financial.js");
      const prior = window.__image_parity_chart;
      prior?.remove();
      document.getElementById("image-parity-host")?.remove();
      const host = document.createElement("div");
      host.id = "image-parity-host";
      host.style.cssText = "position:fixed;left:0;top:0;width:64px;height:64px;background:#fff;z-index:9999";
      document.body.append(host);
      const chart = await api.create_chart(host, {
        autoSize: false,
        backend: requested_backend,
        __force_webgpu_fallback_adapter: true,
        layout: { background: { type: "solid", color: "#ffffff" } },
        grid: { vertLines: { visible: false }, horzLines: { visible: false } },
        leftPriceScale: { visible: false },
        rightPriceScale: { visible: false },
        timeScale: { visible: false },
      });
      chart.resize(64, 64, 1);
      const series = chart.add_series("line");
      series.set_data([{ time: 1, value: 100 }]);
      const image = document.createElement("canvas");
      image.width = image.height = 2;
      const ctx = image.getContext("2d");
      ctx.fillStyle = "#ff0000"; ctx.fillRect(0, 0, 1, 1);
      ctx.fillStyle = "#0000ff"; ctx.fillRect(1, 0, 1, 1);
      ctx.fillStyle = "#00ff00"; ctx.fillRect(0, 1, 1, 1);
      ctx.fillStyle = "#ffffff"; ctx.fillRect(1, 1, 1, 1);
      api.create_image_watermark(series, image, { maxWidth: 20, maxHeight: 20, alpha: 0.72 });
      window.__image_parity_chart = chart;
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      return chart.backend();
    }, backend);
    const image = PNG.sync.read(await page.locator("#image-parity-host").screenshot());
    return { actual_backend, image };
  };
  const gpu = await capture("auto");
  const canvas = await capture("canvas2d");
  expect(gpu.actual_backend).toBe("webgpu");
  expect(canvas.actual_backend).toBe("canvas2d");
  const gpu_center = crop_png(gpu.image, 22, 22, 20, 20);
  const canvas_center = crop_png(canvas.image, 22, 22, 20, 20);
  const native_path = test_info.outputPath("native-image.png");
  const native_run = spawnSync("cargo", ["run", "-p", "aeris_charts_native", "--example", "image_parity_fixture", "--", native_path], {
    cwd: repository_root, encoding: "utf8",
  });
  expect(native_run.status, `${native_run.stdout}\n${native_run.stderr}`).toBe(0);
  const native_center = crop_png(PNG.sync.read(readFileSync(native_path)), 22, 22, 20, 20);
  expect(rgba_diff(gpu_center.data, canvas_center.data, 1).different_pixels).toBe(0);
  expect(rgba_diff(gpu_center.data, native_center.data, 1).different_pixels).toBe(0);
  await context.close();
});

test("dashed and dotted pane paths, stepped series and stroked circles share browser geometry", async ({ page }) => {
  await page.goto("/?backend=canvas2d&forceFallbackAdapter=1");
  const capture = async (backend) => {
    const actual_backend = await page.evaluate(async (requested) => {
      window.__contract_chart?.remove();
      document.getElementById("contract-host")?.remove();
      const api = await import("/dist/aeris_charts_financial.js");
      const host = document.createElement("div");
      host.id = "contract-host";
      host.style.cssText = "position:fixed;left:0;top:0;width:360px;height:220px;background:#fff;z-index:9999";
      document.body.appendChild(host);
      const chart = await api.create_chart(host, {
        backend: requested, autoSize: false,
        layout: { background: { type: "solid", color: "#ffffff" } },
        grid: { vertLines: { visible: false }, horzLines: { visible: false } },
        leftPriceScale: { visible: false }, rightPriceScale: { visible: false },
        timeScale: { visible: false },
      });
      chart.resize(360, 220, 1.5);
      const series = chart.add_series("line", {
        color: "#008a66", line_width: 3, line_type: "stepped",
        price_line_visible: false, last_value_visible: false,
      });
      series.set_data(Array.from({ length: 20 }, (_, i) => ({ time: i + 1, value: 40 + (i % 4) * 10 })));
      chart.time_scale().fit_content();
      chart.panes()[0].attach_primitive({ pane_views: () => [{ z_order: "top", renderer(ctx) {
        const x = ctx.pane_left, y = ctx.pane_top;
        ctx.polyline([x + 20, y + 22, x + 110, y + 32, x + 210, y + 20], "#e91e63", 3, 2);
        ctx.polyline([x + 20, y + 55, x + 110, y + 45, x + 210, y + 60], "#3867ff", 3, 1);
        ctx.circle(x + 270, y + 45, 15, "#8a00ff", "#ff9900", 3);
      } }] });
      window.__contract_chart = chart;
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      return chart.backend();
    }, backend);
    return { backend: actual_backend, png: PNG.sync.read(await page.locator("#contract-host").screenshot()) };
  };
  const canvas = await capture("canvas2d");
  const gpu = await capture("auto");
  expect(canvas.backend).toBe("canvas2d");
  expect(gpu.backend).toBe("webgpu");
  const parity = rgba_diff(gpu.png.data, canvas.png.data, 0);
  console.log(`contract variants: ${parity.different_pixels} residual pixels, max delta ${parity.maximum_channel_delta}`);
  const compare_solid_ink = (source, target, rgb) => {
    const width = source.width, height = source.height;
    const mask = (image) => {
      const out = new Uint8Array(width * height);
      for (let p = 0; p < out.length; p += 1) {
        const offset = p * 4;
        if ([0, 1, 2].every((channel) => Math.abs(image.data[offset + channel] - rgb[channel]) <= 12)) out[p] = 1;
      }
      return out;
    };
    const a = mask(source), b = mask(target);
    let count = 0, missing = 0;
    for (let y = 1; y < height - 1; y += 1) for (let x = 1; x < width - 1; x += 1) {
      if (!a[y * width + x]) continue;
      count += 1;
      let found = false;
      for (let dy = -1; dy <= 1 && !found; dy += 1) for (let dx = -1; dx <= 1; dx += 1) {
        if (b[(y + dy) * width + x + dx]) { found = true; break; }
      }
      if (!found) missing += 1;
    }
    return { count, missing };
  };
  for (const [name, rgb] of [
    ["dashed", [233, 30, 99]], ["dotted", [56, 103, 255]],
    ["stepped", [0, 138, 102]], ["circle", [138, 0, 255]], ["circle stroke", [255, 153, 0]],
  ]) {
    const forward = compare_solid_ink(canvas.png, gpu.png, rgb);
    const backward = compare_solid_ink(gpu.png, canvas.png, rgb);
    console.log(`${name} solid ink: Canvas ${forward.count}, WebGPU ${backward.count}, unmatched ${forward.missing}/${backward.missing}`);
    expect(forward.count, `${name} must paint on Canvas2D`).toBeGreaterThan(20);
    expect(backward.count, `${name} must paint on WebGPU`).toBeGreaterThan(20);
    expect(forward.missing + backward.missing, `${name} geometry must agree within one device pixel`).toBeLessThanOrEqual(20);
  }
});

test("transparent rounded tooltip keeps its outlined interior clear", async ({ browser }) => {
  const context = await browser.newContext({ viewport: { width: 400, height: 240 }, deviceScaleFactor: 2 });
  const page = await context.newPage();
  await page.goto("/");
  for (const backend of ["auto", "canvas2d"]) {
    const geometry = await page.evaluate(async (requested_backend) => {
      const api = await import("/dist/aeris_charts_financial.js");
      window.__round_rect_chart?.remove();
      document.getElementById("round-rect-host")?.remove();
      const host = document.createElement("div");
      host.id = "round-rect-host";
      host.style.cssText = "position:fixed;left:0;top:0;width:320px;height:180px;background:#ffffff;z-index:9999";
      document.body.append(host);
      const chart = await api.create_chart(host, {
        autoSize: false,
        backend: requested_backend,
        __force_webgpu_fallback_adapter: true,
        layout: { background: { type: "solid", color: "rgba(255,255,255,0)" } },
        grid: { vertLines: { visible: false }, horzLines: { visible: false } },
        leftPriceScale: { visible: false },
        rightPriceScale: { visible: false, borderColor: "#ff00ff" },
        timeScale: { visible: false },
      });
      chart.resize(320, 180, 2);
      const series = chart.add_series("line", { color: "#008800", priceLineVisible: false, lastValueVisible: false });
      series.set_data(Array.from({ length: 30 }, (_, index) => ({ time: index + 1, value: 100 + index % 5 })));
      chart.time_scale().fit_content();
      api.create_delta_tooltip(chart, { series, requires_shift_drag: false });
      window.__round_rect_chart = chart;
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      return {
        backend: chart.backend(),
        from: chart.time_scale().logical_to_coordinate(8),
        to: chart.time_scale().logical_to_coordinate(22),
      };
    }, backend);
    expect(geometry.backend).toBe(backend === "auto" ? "webgpu" : "canvas2d");
    await page.mouse.move(geometry.from, 130);
    await page.mouse.down();
    await page.mouse.move(geometry.to, 130, { steps: 5 });
    await page.mouse.up();
    await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    const png = PNG.sync.read(await page.locator("#round-rect-host").screenshot());
    const is_border = (x, y) => {
      const offset = (y * png.width + x) * 4;
      return png.data[offset] > 240 && png.data[offset + 1] < 30 && png.data[offset + 2] > 240;
    };
    let top = null;
    for (let y = 0; y < 150 && top === null; y += 1) {
      for (let x = 0; x < png.width - 50; x += 1) {
        if (Array.from({ length: 50 }, (_, dx) => is_border(x + dx, y)).every(Boolean)) {
          top = { x, y };
          break;
        }
      }
    }
    expect(top, `${geometry.backend} must paint the tooltip border`).not.toBe(null);
    for (let dy = 7; dy < 17; dy += 1) {
      for (let dx = 8; dx < 48; dx += 1) {
        expect(is_border(top.x + dx, top.y + dy), `${geometry.backend} border leaked into tooltip at ${dx},${dy}`).toBe(false);
      }
    }
  }
  await context.close();
});

test("translucent zig-zag joins match Canvas2D on WebGPU", async ({ browser }) => {
  const context = await browser.newContext({ viewport: { width: 320, height: 200 }, deviceScaleFactor: 1 });
  const page = await context.newPage();
  await page.goto("/");
  const capture = async (backend) => {
    const geometry = await page.evaluate(async (requested_backend) => {
      const api = await import("/dist/aeris_charts_financial.js");
      window.__join_chart?.remove();
      document.getElementById("join-host")?.remove();
      const host = document.createElement("div");
      host.id = "join-host";
      host.style.cssText = "position:fixed;left:0;top:0;width:260px;height:160px;background:#fff;z-index:9999";
      document.body.append(host);
      const chart = await api.create_chart(host, {
        autoSize: false, backend: requested_backend, __force_webgpu_fallback_adapter: true,
        layout: { background: { type: "solid", color: "#ffffff" } },
        grid: { vertLines: { visible: false }, horzLines: { visible: false } },
        leftPriceScale: { visible: false }, rightPriceScale: { visible: false },
        timeScale: { visible: false },
      });
      chart.resize(260, 160, 1);
      const series = chart.add_series("line", {
        color: "rgba(0,0,0,0.5)", line_width: 8,
        price_line_visible: false, last_value_visible: false,
      });
      const data = Array.from({ length: 7 }, (_, i) => ({ time: i + 1, value: i % 2 ? 80 : 20 }));
      series.set_data(data);
      chart.time_scale().fit_content();
      window.__join_chart = chart;
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      return {
        backend: chart.backend(),
        joins: [2, 3, 4].map((index) => ({
          x: chart.time_scale().logical_to_coordinate(index),
          y: series.price_to_coordinate(data[index].value),
        })),
      };
    }, backend);
    const png = PNG.sync.read(await page.locator("#join-host").screenshot());
    return { ...geometry, png };
  };
  const gpu = await capture("auto");
  const canvas = await capture("canvas2d");
  expect(gpu.backend).toBe("webgpu");
  expect(canvas.backend).toBe("canvas2d");
  for (let index = 0; index < gpu.joins.length; index += 1) {
    const x = Math.round(gpu.joins[index].x);
    const y = Math.round(gpu.joins[index].y);
    const gpu_join = crop_png(gpu.png, x - 6, y - 6, 13, 13);
    const canvas_join = crop_png(canvas.png, x - 6, y - 6, 13, 13);
    const stats = rgba_diff(gpu_join.data, canvas_join.data, 32);
    expect(stats.different_pixels, `join ${index} at ${x},${y}: max delta ${stats.maximum_channel_delta}`).toBe(0);
  }
  await context.close();
});

test("translucent chart background composites the same on both browser backends", async ({ browser }) => {
  const context = await browser.newContext({ viewport: { width: 300, height: 200 }, deviceScaleFactor: 1 });
  const page = await context.newPage();
  await page.goto("/");
  const capture = async (backend) => {
    const actual = await page.evaluate(async (requested) => {
      const api = await import("/dist/aeris_charts_financial.js");
      window.__alpha_chart?.remove();
      document.getElementById("alpha-host")?.remove();
      const host = document.createElement("div");
      host.id = "alpha-host";
      host.style.cssText = "position:fixed;left:0;top:0;width:260px;height:160px;background:#ff0000;z-index:9999";
      document.body.append(host);
      const chart = await api.create_chart(host, {
        autoSize: false, backend: requested, __force_webgpu_fallback_adapter: true,
        layout: { background: { type: "solid", color: "rgba(0,0,255,0.5)" } },
        grid: { vertLines: { visible: false }, horzLines: { visible: false } },
        leftPriceScale: { visible: false }, rightPriceScale: { visible: false },
        timeScale: { visible: false },
      });
      chart.resize(260, 160, 1);
      window.__alpha_chart = chart;
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      return chart.backend_status();
    }, backend);
    const png = PNG.sync.read(await page.locator("#alpha-host").screenshot());
    return { actual, pixel: Array.from(png.data.subarray((80 * png.width + 130) * 4, (80 * png.width + 130) * 4 + 4)) };
  };
  const canvas = await capture("canvas2d");
  const gpu = await capture("auto");
  expect(canvas.actual.active_backend).toBe("canvas2d");
  expect(gpu.actual.active_backend).toBe("webgpu");
  expect(canvas.pixel[0]).toBeGreaterThan(100);
  expect(canvas.pixel[2]).toBeGreaterThan(100);
  for (let channel = 0; channel < 4; channel += 1) {
    expect(Math.abs(canvas.pixel[channel] - gpu.pixel[channel]), `channel ${channel}: ${canvas.pixel} vs ${gpu.pixel}`).toBeLessThanOrEqual(2);
  }
  await context.close();
});

test("oversized text run remains visible on the WebGPU chart", async ({ browser }) => {
  const context = await browser.newContext({ viewport: { width: 1400, height: 250 }, deviceScaleFactor: 1 });
  const page = await context.newPage();
  await page.goto("/");
  const capture = async (backend) => {
    const actual = await page.evaluate(async (requested) => {
      const api = await import("/dist/aeris_charts_financial.js");
      window.__wide_chart?.remove();
      document.getElementById("wide-host")?.remove();
      const host = document.createElement("div");
      host.id = "wide-host";
      host.style.cssText = "position:fixed;left:0;top:0;width:1300px;height:180px;background:#101820;z-index:9999";
      document.body.append(host);
      const chart = await api.create_chart(host, {
        autoSize: false, backend: requested, __force_webgpu_fallback_adapter: true,
        layout: { background: { type: "solid", color: "#101820" } },
        grid: { vertLines: { visible: false }, horzLines: { visible: false } },
        leftPriceScale: { visible: false }, rightPriceScale: { visible: false },
        timeScale: { visible: false },
      });
      chart.resize(1300, 180, 1);
      const series = chart.add_series("line", {
        color: "#101820", line_width: 1, price_line_visible: false, last_value_visible: false,
      });
      series.set_data([{ time: 1, value: 40 }, { time: 2, value: 60 }]);
      chart.time_scale().fit_content();
      chart.add_drawing("text", [{ logical: 0, price: 50 }], {
        text: "Wide label ".repeat(25), text_size: 32, text_color: "#ffffff",
        text_h_align: "left", text_v_align: "middle",
      });
      window.__wide_chart = chart;
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      return chart.backend();
    }, backend);
    const png = PNG.sync.read(await page.locator("#wide-host").screenshot());
    let white = 0;
    for (let offset = 0; offset < png.data.length; offset += 4) {
      if (png.data[offset] > 200 && png.data[offset + 1] > 200 && png.data[offset + 2] > 200) white += 1;
    }
    let recovery = null;
    if (backend === "auto") {
      recovery = await page.evaluate(async () => {
        const host = document.getElementById("wide-host");
        const visibility = () => Array.from(host.querySelectorAll("canvas"), (canvas) => canvas.style.visibility);
        const before = visibility();
        window.__wide_chart.drawings()[0].remove();
        window.__wide_chart.render();
        await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
        return { before, after: visibility(), backend: window.__wide_chart.backend() };
      });
    }
    return { actual, white, png, recovery };
  };
  const canvas = await capture("canvas2d");
  const gpu = await capture("auto");
  expect(canvas.actual).toBe("canvas2d");
  expect(gpu.actual).toBe("webgpu");
  expect(canvas.white).toBeGreaterThan(100);
  expect(gpu.white, `Canvas2D paints ${canvas.white} white text pixels; WebGPU paints ${gpu.white}`).toBeGreaterThan(canvas.white * 0.8);
  const diff = rgba_diff(canvas.png.data, gpu.png.data, 2);
  expect(diff.different_pixels, `oversized-run fallback max delta ${diff.maximum_channel_delta}`).toBe(0);
  expect(gpu.recovery.backend).toBe("webgpu");
  expect(gpu.recovery.after, "removing the wide label restores the WebGPU surface").not.toEqual(gpu.recovery.before);
  await context.close();
});

function image_stats(a, b) {
  expect([a.width, a.height]).toEqual([b.width, b.height]);
  const exact = rgba_diff(a.data, b.data, 0);
  const visual = new PNG({ width: a.width, height: a.height });
  const perceptual_pixels = pixelmatch(a.data, b.data, visual.data, a.width, a.height, {
    threshold: 0.1,
    includeAA: false,
  });
  return {
    ...exact,
    total_pixels: a.width * a.height,
    different_fraction: exact.different_pixels / (a.width * a.height),
    perceptual_pixels,
    perceptual_fraction: perceptual_pixels / (a.width * a.height),
    visual,
  };
}

function changed_footprint(base, feature) {
  let pixels = 0;
  let min_x = base.width;
  let min_y = base.height;
  let max_x = -1;
  let max_y = -1;
  for (let y = 0; y < base.height; y += 1) {
    for (let x = 0; x < base.width; x += 1) {
      const offset = (y * base.width + x) * 4;
      if (base.data[offset] === feature.data[offset]
        && base.data[offset + 1] === feature.data[offset + 1]
        && base.data[offset + 2] === feature.data[offset + 2]
        && base.data[offset + 3] === feature.data[offset + 3]) continue;
      pixels += 1;
      min_x = Math.min(min_x, x);
      min_y = Math.min(min_y, y);
      max_x = Math.max(max_x, x);
      max_y = Math.max(max_y, y);
    }
  }
  return { pixels, bounds: pixels === 0 ? null : { min_x, min_y, max_x, max_y } };
}

function css_to_device(css, pixel_ratio) {
  return Math.round(css * pixel_ratio);
}

// Regional comparison with per-image axis geometry. Aeris negotiates compact strips while
// the pinned reference keeps its own sizes, so each image is cropped with its own geometry
// ({ price_axis_width, time_axis_height } in CSS px). Windows anchor on shared edges — panes
// and price strips on the top-right (same spacing, same right-anchored last bar, same top
// edge), time strips on the bottom-right (same bottom edge) — so equal content coincides and
// only genuine rendering differences (sizes, glyphs, density) remain in the diff.
function regional_fidelity_report(Aeris, reference, pixel_ratio, Aeris_geom, reference_geom) {
  expect([Aeris.width, Aeris.height]).toEqual([reference.width, reference.height]);
  const layout = (image, geom) => ({
    pane_w: css_to_device(fixture.css_width - geom.price_axis_width, pixel_ratio),
    pane_h: css_to_device(fixture.css_height - geom.time_axis_height, pixel_ratio),
    full_w: image.width,
    full_h: image.height,
  });
  const n = layout(Aeris, Aeris_geom);
  const r = layout(reference, reference_geom);
  // Intersect two windows anchored on shared edges: panes and price strips anchor
  // top-right (common top edge; bars coincide because spacing and the right-anchored last
  // bar match), time strips anchor bottom-right (common bottom edge).
  const pairTopRight = (n_right, n_w, n_h, r_right, r_w, r_h) => {
    const w = Math.min(n_w, r_w);
    const h = Math.min(n_h, r_h);
    return [
      crop_png(Aeris, n_right - w, 0, w, h),
      crop_png(reference, r_right - w, 0, w, h),
    ];
  };
  const pairBottomRight = (n_right, n_w, n_h, r_right, r_w, r_h) => {
    const w = Math.min(n_w, r_w);
    const h = Math.min(n_h, r_h);
    const H = Aeris.height;
    return [
      crop_png(Aeris, n_right - w, H - h, w, h),
      crop_png(reference, r_right - w, H - h, w, h),
    ];
  };
  const W = Aeris.width;
  const regions = {
    full: [Aeris, reference],
    pane: pairTopRight(n.pane_w, n.pane_w, n.pane_h, r.pane_w, r.pane_w, r.pane_h),
    price_axis: pairTopRight(
      W, W - n.pane_w, n.pane_h,
      W, W - r.pane_w, r.pane_h,
    ),
    time_axis: pairBottomRight(
      n.pane_w, n.pane_w, Aeris.height - n.pane_h,
      r.pane_w, r.pane_w, reference.height - r.pane_h,
    ),
  };
  const report = {};
  const visuals = {};
  for (const [name, [Aeris_region, reference_region]] of Object.entries(regions)) {
    const stats = image_stats(Aeris_region, reference_region);
    report[name] = {
      total_pixels: stats.total_pixels,
      different_pixels: stats.different_pixels,
      different_percent: stats.different_fraction * 100,
      perceptual_pixels: stats.perceptual_pixels,
      perceptual_percent: stats.perceptual_fraction * 100,
      maximum_channel_delta: stats.maximum_channel_delta,
      mean_absolute_channel_delta: stats.mean_absolute_channel_delta,
    };
    visuals[name] = stats.visual;
  }
  return { report, visuals };
}

test("public screenshot is deterministic across live backends", async ({ page }) => {
  await page.goto("/?runtimeTest=backendParity&forceFallbackAdapter=1");
  await page.waitForFunction(() => document.documentElement.dataset.backendParity !== undefined);
  const result = await page.evaluate(() => window.__backend_parity_result);

  expect(result.status).toBe("passed");
  expect(result.screenshot_api.status).toBe("passed");
  expect(result.screenshot_api.different_pixels).toBe(0);
});

test("presented WebGPU and Canvas2D frames share geometry with bounded text-AA divergence", async ({ page }, test_info) => {
  const gpu = await capture_presented_frame(page, "auto");
  expect(gpu.backend, "This project is the WebGPU coverage gate; fallback is tested separately").toBe("webgpu");
  const canvas = await capture_presented_frame(page, "canvas2d");
  expect(canvas.backend).toBe("canvas2d");

  const gpu_image = PNG.sync.read(gpu.png);
  const canvas_image = PNG.sync.read(canvas.png);
  expect([gpu_image.width, gpu_image.height]).toEqual([canvas_image.width, canvas_image.height]);

  const diff = new PNG({ width: gpu_image.width, height: gpu_image.height });
  const different_pixels = pixelmatch(
    gpu_image.data,
    canvas_image.data,
    diff.data,
    gpu_image.width,
    gpu_image.height,
    { threshold: 0, includeAA: true },
  );

  if (different_pixels !== 0) {
    await test_info.attach("webgpu.png", { body: gpu.png, contentType: "image/png" });
    await test_info.attach("canvas2d.png", { body: canvas.png, contentType: "image/png" });
    await test_info.attach("diff.png", { body: PNG.sync.write(diff), contentType: "image/png" });
  }
  const parity = rgba_diff(gpu_image.data, canvas_image.data, 0);
  console.log(`WebGPU/Canvas2D shared-frame residual: ${parity.different_pixels} pixels, max channel delta ${parity.maximum_channel_delta}/255`);
  // Both backends consume identical geometry and browser-rasterized text. Fractional-DPR glyph
  // hinting plus analytic Canvas2D versus 4x-MSAA rounded-label edges can leave a bounded AA
  // coverage residual, but no paint-order or solid-interior mismatch (which exceeds this band) is
  // permitted.
  // Windows SwiftShader measurement after the A–E fixes: 2,307 pixels, max delta 32.
  expect(parity.different_pixels).toBeLessThanOrEqual(2_600);
  expect(parity.maximum_channel_delta).toBeLessThanOrEqual(40);

  // The same presented-frame gate with engine markers visible (?feature=markers) — the state
  // that exposed the WebGPU paint-order bug: markers are tri-family shapes emitted after the
  // quad-family candles, so both backends must paint them over the wicks/bodies.
  const marker_gpu = await capture_presented_frame(page, "auto", "&feature=markers");
  expect(marker_gpu.backend, "markers gate: WebGPU must stay active").toBe("webgpu");
  const marker_canvas = await capture_presented_frame(page, "canvas2d", "&feature=markers");
  expect(marker_canvas.backend).toBe("canvas2d");
  const gpu_markers = PNG.sync.read(marker_gpu.png);
  const canvas_markers = PNG.sync.read(marker_canvas.png);
  expect([gpu_markers.width, gpu_markers.height]).toEqual([canvas_markers.width, canvas_markers.height]);

  // The markers must actually paint on both backends, or the gate below is vacuous.
  const marker_pixels = { threshold: 0, includeAA: true };
  expect(
    pixelmatch(gpu_image.data, gpu_markers.data, null, gpu_image.width, gpu_image.height, marker_pixels),
    "engine markers must change the presented WebGPU frame",
  ).toBeGreaterThan(0);
  expect(
    pixelmatch(canvas_image.data, canvas_markers.data, null, canvas_image.width, canvas_image.height, marker_pixels),
    "engine markers must change the presented Canvas2D frame",
  ).toBeGreaterThan(0);

  // Ordering contract: zero pixels may differ by more than an AA coverage step. The shapes'
  // anti-aliased edges legitimately differ by 1-2 steps between SwiftShader's 4xMSAA and
  // Canvas2D's analytic coverage (measured max channel delta 66 on this fixture — not the
  // ordering contract's concern); a paint-order mismatch swaps whole marker/wick/body colors
  // (pre-fix: 67 pixels above this bound, up to 201). This keeps the strict 0-diff gate
  // above untouched while pinning the marker paint order exactly.
  let ordering_diff = 0;
  let edge_diff = 0;
  let maximum_channel_delta = 0;
  for (let offset = 0; offset < gpu_markers.data.length; offset += 4) {
    let pixel_delta = 0;
    for (let channel = 0; channel < 4; channel += 1) {
      pixel_delta = Math.max(pixel_delta, Math.abs(gpu_markers.data[offset + channel] - canvas_markers.data[offset + channel]));
    }
    maximum_channel_delta = Math.max(maximum_channel_delta, pixel_delta);
    if (pixel_delta > 96) ordering_diff += 1;
    else if (pixel_delta !== 0) edge_diff += 1;
  }
  console.log(`markers gate: ${edge_diff} AA-edge pixels (max step ${maximum_channel_delta}), ${ordering_diff} ordering pixels`);
  if (ordering_diff !== 0) {
    const marker_visual = new PNG({ width: gpu_markers.width, height: gpu_markers.height });
    pixelmatch(gpu_markers.data, canvas_markers.data, marker_visual.data, gpu_markers.width, gpu_markers.height, marker_pixels);
    await test_info.attach("webgpu-markers.png", { body: marker_gpu.png, contentType: "image/png" });
    await test_info.attach("canvas2d-markers.png", { body: marker_canvas.png, contentType: "image/png" });
    await test_info.attach("markers-diff.png", { body: PNG.sync.write(marker_visual), contentType: "image/png" });
  }
  expect(ordering_diff, "marker paint order must match Canvas2D (only AA coverage steps may differ)").toBe(0);
});

test("native and browser Canvas2D panes consume the same fixture", async ({ page }, test_info) => {
  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d");
  await wait_for_chart(page);
  const browser_data_url = await page.evaluate(() => window.__chart.take_screenshot().toDataURL("image/png"));
  const browser_full = PNG.sync.read(Buffer.from(browser_data_url.split(",")[1], "base64"));
  const pane_width = Math.round((fixture.css_width - fixture.price_axis_width) * fixture.pixel_ratio);
  const pane_height = Math.round((fixture.css_height - fixture.time_axis_height) * fixture.pixel_ratio);
  expect([browser_full.width, browser_full.height]).toEqual([
    Math.round(fixture.css_width * fixture.pixel_ratio),
    Math.round(fixture.css_height * fixture.pixel_ratio),
  ]);

  const browser_pane = new PNG({ width: pane_width, height: pane_height });
  PNG.bitblt(browser_full, browser_pane, 0, 0, pane_width, pane_height, 0, 0);
  const native_path = test_info.outputPath("native-pane.png");
  render_native_fixture(native_path);
  const native_pane = PNG.sync.read(readFileSync(native_path));
  expect([native_pane.width, native_pane.height]).toEqual([pane_width, pane_height]);

  // This is raw executor output on both sides, before browser compositor scaling, so the shared
  // Canvas2D command stream must be byte-exact even though the rasterizer implementations differ.
  const stats = rgba_diff(native_pane.data, browser_pane.data, 0);
  const fraction = stats.different_pixels / (pane_width * pane_height);
  console.log(`native/browser pane diff: ${stats.different_pixels}/${pane_width * pane_height} (${(fraction * 100).toFixed(4)}%), max ${stats.maximum_channel_delta}, mean ${stats.mean_absolute_channel_delta.toFixed(4)}`);

  if (stats.different_pixels !== 0) {
    const visual_diff = new PNG({ width: pane_width, height: pane_height });
    pixelmatch(native_pane.data, browser_pane.data, visual_diff.data, pane_width, pane_height, {
      threshold: 0.02,
      includeAA: true,
    });
    await test_info.attach("native-pane.png", { body: PNG.sync.write(native_pane), contentType: "image/png" });
    await test_info.attach("browser-pane.png", { body: PNG.sync.write(browser_pane), contentType: "image/png" });
    await test_info.attach("native-browser-diff.png", { body: PNG.sync.write(visual_diff), contentType: "image/png" });
  }
  expect(stats.different_pixels).toBe(0);
});

test("public time and price scale handles are engine-owned and reference-compatible", async ({ page }) => {
  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d");
  await wait_for_chart(page);
  const result = await page.evaluate(async () => {
    const chart = window.__chart;
    const main = window.__main;
    const data = window.__data;
    const time = chart.time_scale();
    const exact_time = data[100].time;
    const x = time.logical_to_coordinate(100);
    const time_result = {
      width: time.width(),
      height: time.height(),
      exact_index: time.time_to_index(exact_time),
      missing_index: time.time_to_index(exact_time + 1),
      nearest_index: time.time_to_index(exact_time + 1, true),
      logical_roundtrip: x === null ? null : time.coordinate_to_logical(x),
      time_coordinate_matches: x === time.time_to_coordinate(exact_time),
    };

    time.apply_options({ bar_spacing: 12, right_offset: 0 });
    time.scroll_to_position(0, false);
    const scrolled = time.scroll_position();
    time.reset_time_scale();
    const reset_options = time.options();
    // With reference `restoreDefault` semantics the applied bar_spacing 12 persists through the reset;
    // restore the fixture spacing for the downstream reference comparisons.
    time.apply_options({ bar_spacing: 6 });
    const query_logical_range = time.get_visible_logical_range();
    const queried_data = main.data();
    const data_scopes = [];
    const on_data_changed = (scope) => data_scopes.push(scope);
    main.subscribe_data_changed(on_data_changed);
    main.set_data(data);
    main.update(data[data.length - 1]);
    main.unsubscribe_data_changed(on_data_changed);
    const series_queries = {
      logical_range: query_logical_range,
      bars: query_logical_range === null ? null : main.bars_in_logical_range(query_logical_range),
      exact: main.data_by_index(100),
      missing: main.data_by_index(-1),
      nearest_right: main.data_by_index(-1, 1),
      length: queried_data.length,
      first: queried_data[0],
      last: queried_data[queried_data.length - 1],
      type: main.series_type(),
      data_scopes,
    };

    const price = main.price_scale();
    const chart_price = chart.price_scale("right");
    const initial_range = price.get_visible_range();
    price.set_visible_range({ from: 90, to: 140 });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const manual_range = price.get_visible_range();
    const manual_options = price.options();
    price.apply_options({ invert_scale: true, scale_margins: { top: 0.25, bottom: 0.15 } });
    const changed_options = price.options();
    price.set_auto_scale(true);
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const auto_range = price.get_visible_range();
    // Compact strips widen the pane, so the same spacing/offset shows more bars than the
    // reference. Pin both charts to one explicit logical range right before each scale-mode
    // measurement (an interior bar boundary both libraries hold exactly), keeping the
    // percentage base and log/indexed ranges identical by construction. Pinning re-fits bar
    // spacing to the range on both sides, so every assertion below stays spacing-agnostic
    // (roundtrips, value ranges, strip widths) by design.
    price.apply_options({ mode: 2 });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    time.set_visible_logical_range({ from: 801, to: 950 });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const percentage_options = price.options();
    const source_price = data[900].close;
    const percentage_coordinate = main.price_to_coordinate(source_price);
    const percentage_roundtrip = percentage_coordinate === null
      ? null
      : main.coordinate_to_price(percentage_coordinate);
    const percentage_range = price.get_visible_range();
    const percentage_width = price.width();
    const percentage_logical_range = time.get_visible_logical_range();
    price.apply_options({ mode: 1 });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    time.set_visible_logical_range({ from: 801, to: 950 });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const log_coordinate = main.price_to_coordinate(source_price);
    const log_roundtrip = log_coordinate === null ? null : main.coordinate_to_price(log_coordinate);
    const log_range = price.get_visible_range();
    price.apply_options({ mode: 3 });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    time.set_visible_logical_range({ from: 801, to: 950 });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const indexed_coordinate = main.price_to_coordinate(source_price);
    const indexed_roundtrip = indexed_coordinate === null
      ? null
      : main.coordinate_to_price(indexed_coordinate);
    const indexed_range = price.get_visible_range();
    const indexed_logical_range = time.get_visible_logical_range();

    return {
      time_result,
      scrolled,
      reset_options,
      series_queries,
      initial_range,
      manual_range,
      manual_options,
      changed_options,
      auto_range,
      percentage_options,
      percentage_range,
      percentage_width,
      percentage_logical_range,
      percentage_coordinate,
      source_price,
      percentage_roundtrip,
      log_coordinate,
      log_roundtrip,
      log_range,
      indexed_coordinate,
      indexed_roundtrip,
      indexed_range,
      indexed_logical_range,
      price_width: price.width(),
      chart_price_width: chart_price.width(),
      overlay_width: chart.price_scale("").width(),
    };
  });

  expect(result.time_result.width).toBeGreaterThan(0);
  expect(result.time_result.height).toBe(fixture.time_axis_height);
  expect(result.time_result.exact_index).toBe(100);
  expect(result.time_result.missing_index).toBeNull();
  expect(result.time_result.nearest_index).toBe(101);
  expect(result.time_result.logical_roundtrip).toBe(100);
  expect(result.time_result.time_coordinate_matches).toBe(true);
  expect(result.scrolled).toBe(0);
  // reference `restoreDefault` restores from the *configured* options (time-scale.ts), so the
  // bar_spacing applied above (12) survives reset_time_scale — options() reports it back.
  expect(result.reset_options).toEqual({
    bar_spacing: 12,
    right_offset: 0,
    min_bar_spacing: 0.5,
    max_bar_spacing: 0,
    right_offset_pixels: null,
    time_visible: true,
    seconds_visible: false,
    fix_left_edge: false,
    fix_right_edge: false,
    lock_visible_time_range_on_resize: false,
    // Measured TradingView default: ordinary wheel zoom keeps the right edge pinned.
    right_bar_stays_on_scroll: true,
    shift_visible_range_on_new_bar: true,
    allow_shift_visible_range_on_whitespace_replacement: false,
    allow_bold_labels: true,
    ticks_visible: false,
    minimum_height: 0,
    tick_mark_max_character_length: 8,
    visible: true,
    // Aeris extensions (defaults): exchange time zone and trading-day start, explicit time-axis
    // marks, the fixed-session logical-range lock, and bars labeled by their open time.
    time_zone: "UTC",
    session_start: 0,
    tick_marks: null,
    lock_visible_logical_range: false,
    bar_time_label: "open",
  });
  expect(result.series_queries.length).toBe(fixture.bar_count);
  expect(result.series_queries.type).toBe("candlestick");
  expect(result.series_queries.missing).toBeNull();
  expect(result.series_queries.nearest_right).toEqual(result.series_queries.first);
  expect(result.series_queries.data_scopes).toEqual(["full", "update"]);
  expect(result.initial_range).not.toBeNull();
  expect(result.manual_range).toEqual({ from: 90, to: 140 });
  expect(result.manual_options.auto_scale).toBe(false);
  expect(result.manual_options.scale_margins).toEqual({ top: 0.2, bottom: 0.1 });
  expect(result.changed_options.invert_scale).toBe(true);
  expect(result.changed_options.scale_margins).toEqual({ top: 0.25, bottom: 0.15 });
  expect(result.auto_range).not.toEqual({ from: 90, to: 140 });
  expect(result.percentage_options.mode).toBe(2);
  expect(result.percentage_options.auto_scale).toBe(true);
  expect(result.percentage_range).not.toBeNull();
  expect(result.percentage_roundtrip).toBeCloseTo(result.source_price, 9);
  expect(result.log_roundtrip).toBeCloseTo(result.source_price, 8);
  expect(result.indexed_roundtrip).toBeCloseTo(result.source_price, 9);
  expect(result.price_width).toBeGreaterThan(0);
  expect(result.chart_price_width).toBe(result.price_width);
  expect(result.overlay_width).toBe(0);

  await page.goto("/reference.html");
  await page.waitForFunction(() => document.documentElement.dataset.ready === "true");
  const reference_modes = await page.evaluate(async (source_price) => {
    const { chart, series } = window.__reference;
    const scale = chart.priceScale("right");
    chart.timeScale().applyOptions({ barSpacing: 6, rightOffset: 0 });
    scale.applyOptions({
      mode: 2,
      invertScale: true,
      scaleMargins: { top: 0.25, bottom: 0.15 },
    });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    chart.timeScale().setVisibleLogicalRange({ from: 801, to: 950 });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const percentage = {
      range: scale.getVisibleRange(),
      coordinate: series.priceToCoordinate(source_price),
      roundtrip: series.coordinateToPrice(series.priceToCoordinate(source_price)),
      width: scale.width(),
      logical_range: chart.timeScale().getVisibleLogicalRange(),
    };
    scale.applyOptions({ mode: 1 });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    chart.timeScale().setVisibleLogicalRange({ from: 801, to: 950 });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const log = {
      range: scale.getVisibleRange(),
      coordinate: series.priceToCoordinate(source_price),
      roundtrip: series.coordinateToPrice(series.priceToCoordinate(source_price)),
    };
    scale.applyOptions({ mode: 3 });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    chart.timeScale().setVisibleLogicalRange({ from: 801, to: 950 });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    return {
      percentage,
      log,
      indexed: {
        range: scale.getVisibleRange(),
        coordinate: series.priceToCoordinate(source_price),
        roundtrip: series.coordinateToPrice(series.priceToCoordinate(source_price)),
        width: scale.width(),
        logical_range: chart.timeScale().getVisibleLogicalRange(),
      },
    };
  }, result.source_price);
  const reference_series_queries = await page.evaluate((logical_range) => {
    const { series } = window.__reference;
    const values = series.data();
    const bars = series.barsInLogicalRange(logical_range);
    return {
      bars: bars === null ? null : {
        bars_before: bars.barsBefore,
        bars_after: bars.barsAfter,
        ...(bars.from === undefined ? {} : { from: bars.from, to: bars.to }),
      },
      exact: series.dataByIndex(100),
      missing: series.dataByIndex(-1),
      nearest_right: series.dataByIndex(-1, 1),
      length: values.length,
      first: values[0],
      last: values[values.length - 1],
      type: series.seriesType().toLowerCase(),
    };
  }, result.series_queries.logical_range);
  expect(result.series_queries.bars).toEqual(reference_series_queries.bars);
  expect(result.series_queries.exact).toEqual(reference_series_queries.exact);
  expect(result.series_queries.missing).toEqual(reference_series_queries.missing);
  expect(result.series_queries.nearest_right).toEqual(reference_series_queries.nearest_right);
  expect(result.series_queries.length).toBe(reference_series_queries.length);
  expect(result.series_queries.first).toEqual(reference_series_queries.first);
  expect(result.series_queries.last).toEqual(reference_series_queries.last);
  expect(result.series_queries.type).toBe(reference_series_queries.type);
  expect(result.percentage_range.from).toBeCloseTo(reference_modes.percentage.range.from, 9);
  expect(result.percentage_range.to).toBeCloseTo(reference_modes.percentage.range.to, 9);
  // Pane heights differ by design (compact 22px time strip vs the reference's), so media
  // coordinates cannot match across libraries; per-side roundtrips prove each mode's math
  // instead (Aeris roundtrips are asserted with the Aeris result above).
  expect(reference_modes.percentage.roundtrip).toBeCloseTo(result.source_price, 9);
  // Compact strips are narrower than the reference's by design; ranges and logical ranges
  // above still match exactly.
  expect(result.percentage_width).toBeLessThan(reference_modes.percentage.width);
  // Both charts pin the requested range with slightly different edge semantics (sub-bar
  // pinning differs); the right edge anchors identically and mode math above matches at
  // 1e-9, so the left edge need only agree within one bar.
  expect(result.percentage_logical_range.to).toBe(reference_modes.percentage.logical_range.to);
  expect(
    Math.abs(result.percentage_logical_range.from - reference_modes.percentage.logical_range.from)
  ).toBeLessThanOrEqual(1);
  expect(reference_modes.log.roundtrip).toBeCloseTo(result.source_price, 9);
  expect(result.log_range).toEqual(reference_modes.log.range);
  expect(result.indexed_range.from).toBeCloseTo(reference_modes.indexed.range.from, 9);
  expect(result.indexed_range.to).toBeCloseTo(reference_modes.indexed.range.to, 9);
  expect(reference_modes.indexed.roundtrip).toBeCloseTo(result.source_price, 9);
  expect(result.price_width).toBeLessThan(reference_modes.indexed.width);
  expect(result.indexed_logical_range.to).toBe(reference_modes.indexed.logical_range.to);
  expect(
    Math.abs(result.indexed_logical_range.from - reference_modes.indexed.logical_range.from)
  ).toBeLessThanOrEqual(1);

  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d&leftScale=1&dpr=1");
  await wait_for_chart(page);
  const Aeris_left = await page.evaluate(async () => {
    const chart = window.__chart;
    const series = window.__main;
    const source_price = window.__data[900].close;
    chart.time_scale().set_visible_logical_range({ from: 801, to: 950 });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    return {
      left_width: chart.price_scale("left").width(),
      right_width: chart.price_scale("right").width(),
      pane_width: chart.time_scale().width(),
      range: chart.price_scale("left").get_visible_range(),
      roundtrip: series.coordinate_to_price(series.price_to_coordinate(source_price)),
      logical_range: chart.time_scale().get_visible_logical_range(),
      source_price,
    };
  });
  expect(Aeris_left.left_width).toBeGreaterThan(0);
  expect(Aeris_left.right_width).toBe(0);
  expect(Aeris_left.pane_width + Aeris_left.left_width).toBe(fixture.css_width);
  expect(Aeris_left.roundtrip).toBeCloseTo(Aeris_left.source_price, 9);

  await page.goto("/reference.html?leftScale=1");
  await page.waitForFunction(() => document.documentElement.dataset.ready === "true");
  const reference_left = await page.evaluate(async (source_price) => {
    const { chart, series } = window.__reference;
    chart.timeScale().setVisibleLogicalRange({ from: 801, to: 950 });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    return {
      left_width: chart.priceScale("left").width(),
      right_width: chart.priceScale("right").width(),
      pane_width: chart.timeScale().width(),
      range: chart.priceScale("left").getVisibleRange(),
      roundtrip: series.coordinateToPrice(series.priceToCoordinate(source_price)),
      logical_range: chart.timeScale().getVisibleLogicalRange(),
    };
  }, Aeris_left.source_price);
  expect(Aeris_left.left_width).toBeLessThan(reference_left.left_width);
  expect(Aeris_left.right_width).toBe(reference_left.right_width);
  expect(Aeris_left.pane_width).toBeGreaterThan(reference_left.pane_width);
  expect(Aeris_left.range).toEqual(reference_left.range);
  // Media coordinates couple to pane height (compact 22px time strip vs the reference's),
  // so each side proves its own mapping with a roundtrip instead.
  expect(reference_left.roundtrip).toBeCloseTo(Aeris_left.source_price, 9);
  expect(Aeris_left.logical_range.to).toBe(reference_left.logical_range.to);
  expect(
    Math.abs(Aeris_left.logical_range.from - reference_left.logical_range.from)
  ).toBeLessThanOrEqual(1);
});

test("browser and GPUI fixture negotiate the same axis width with the same glyph widths", async ({ page }) => {
  await page.addInitScript(() => {
    const native_measure = CanvasRenderingContext2D.prototype.measureText;
    CanvasRenderingContext2D.prototype.measureText = function(text) {
      const metrics = native_measure.call(this, text);
      return new Proxy(metrics, {
        get(target, key) {
          return key === "width" ? String(text).length * 7 : Reflect.get(target, key, target);
        },
      });
    };
  });
  await page.goto("/?backend=canvas2d&forceFallbackAdapter=1");
  await wait_for_chart(page);
  const width = await page.evaluate(async () => {
    const { create_chart } = await import("/dist/aeris_charts_financial.js");
    const host = document.createElement("div");
    host.style.cssText = "position:absolute;left:-10000px;width:800px;height:500px";
    document.body.append(host);
    const chart = await create_chart(host, { backend: "canvas2d", autoSize: false });
    chart.resize(800, 500, 1);
    const series = chart.add_series("candlestick");
    series.set_data([
      { time: 1, open: 101, high: 102, low: 100, close: 101 },
      { time: 2, open: 102, high: 103, low: 101, close: 102 },
    ]);
    chart.time_scale().fit_content();
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const axis_width = chart.wasm.price_axis_width();
    chart.remove();
    host.remove();
    return axis_width;
  });
  expect(width).toBe(54);
});

test("axis cap centering measures the painted axis font size", async ({ page }) => {
  await page.addInitScript(() => {
    window.__axis_cap_fonts = [];
    const measure = CanvasRenderingContext2D.prototype.measureText;
    CanvasRenderingContext2D.prototype.measureText = function(text) {
      if (text === "H") window.__axis_cap_fonts.push(this.font);
      return measure.call(this, text);
    };
  });
  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d");
  await wait_for_chart(page);
  const fonts = await page.evaluate(async () => {
    window.__axis_cap_fonts.length = 0;
    window.__chart.apply_options({ layout: { fontSize: 24 } });
    window.__chart.render();
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    return window.__axis_cap_fonts;
  });
  expect(fonts.some((font) => /\b22px\b/.test(font)), `axis labels must measure at 22px: ${fonts}`).toBe(true);
  expect(fonts.some((font) => /\b24px\b/.test(font)), `layout 24px must not stand in for the axis font: ${fonts}`).toBe(false);
});

test("reference 5.2 reference is deterministic and reports Aeris fidelity @machine", async ({ page }, test_info) => {
  // Pin bar spacing like the matrix cases: compact strips change default-fit spacing, so only
  // equal spacing keeps bars pixel-aligned for the comparison.
  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d&spacing=6");
  await wait_for_chart(page);
  await settle_page(page);
  const Aeris = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));
  const Aeris_axis_width = Number(await page.getAttribute("html", "data-price-axis-width"));

  await page.goto("/reference.html?spacing=6");
  await page.waitForFunction(() => document.documentElement.dataset.ready === "true");
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  await settle_page(page);
  const capture_reference = async () => page.screenshot({ animations: "disabled", fullPage: false });
  const reference_first = await capture_reference();
  const reference_second = await capture_reference();
  expect(reference_second.equals(reference_first), "the pinned reference fixture must itself be deterministic").toBe(true);
  const reference = PNG.sync.read(reference_first);

  // Comparing presented pages puts both libraries through the same Chromium compositor and avoids
  // the unequal public screenshot resolutions (Aeris is device-pixel-sized; reference is CSS-sized).
  const expected_size = [
    Math.round(fixture.css_width * fixture.pixel_ratio),
    Math.round(fixture.css_height * fixture.pixel_ratio),
  ];
  expect([Aeris.width, Aeris.height]).toEqual(expected_size);
  expect([reference.width, reference.height]).toEqual(expected_size);

  const reference_axis_width = Number(await page.getAttribute("html", "data-price-axis-width"));
  const reference_pane_height = await page.evaluate(() => window.__reference.chart.panes()[0].getHeight());

  const { report, visuals } = regional_fidelity_report(
    Aeris,
    reference,
    fixture.pixel_ratio,
    { price_axis_width: Aeris_axis_width, time_axis_height: fixture.time_axis_height },
    {
      price_axis_width: reference_axis_width,
      time_axis_height: fixture.css_height - reference_pane_height,
    },
  );
  await test_info.attach("Aeris.png", { body: PNG.sync.write(Aeris), contentType: "image/png" });
  await test_info.attach("reference-5.2.0.png", { body: PNG.sync.write(reference), contentType: "image/png" });
  await test_info.attach("aeris_charts-reference-diff.png", { body: PNG.sync.write(visuals.full), contentType: "image/png" });
  console.log(`Aeris/reference 5.2 fidelity report: ${JSON.stringify(report)}`);
  await test_info.attach("aeris_charts-reference-report.json", {
    body: Buffer.from(JSON.stringify({ fixture: fixture.name, ref_version: "5.2.0", regions: report }, null, 2)),
    contentType: "application/json",
  });

  // This gate currently establishes a reproducible upstream reference and makes divergence
  // visible. The explicit ceilings prevent fidelity regressions and are lowered region-by-region
  // as Aeris closes each measured gap; they are intentionally not represented as pixel parity.
  // Compact axes (9/2026) intentionally diverge in both axis regions; the pane ceiling still
  // guards chart-content fidelity at its historical level.
  expect(reference_baseline.fixture).toBe(fixture.name);
  expect(reference_baseline.ref_version).toBe("5.2.0");
  for (const [name, ceiling] of Object.entries(reference_baseline.maximum_perceptual_difference)) {
    expect(report[name].perceptual_percent / 100, `${name} exceeded its recorded reference fidelity ceiling`).toBeLessThanOrEqual(ceiling);
  }
});

test("reference spacing, DPR, and theme matrix reports regional fidelity @machine", async ({ browser }, test_info) => {
  expect(reference_matrix.fixture).toBe(fixture.name);
  expect(reference_matrix.ref_version).toBe("5.2.0");
  const cases = reference_matrix.cases;
  const matrix = {};
  for (const entry of cases) {
    const context = await browser.newContext({
      viewport: { width: fixture.css_width, height: fixture.css_height },
      deviceScaleFactor: entry.dpr,
      colorScheme: entry.theme,
    });
    const matrix_page = await context.newPage();
    const query = new URLSearchParams({
      runtimeTest: "presentedFrame",
      backend: "canvas2d",
      dpr: String(entry.dpr),
      spacing: String(entry.spacing),
      theme: entry.theme,
    });
    await matrix_page.goto(`${test_base_url}/?${query}`);
    await wait_for_chart(matrix_page);
    await settle_page(matrix_page);
    const Aeris_spacing = Number(await matrix_page.getAttribute("html", "data-bar-spacing"));
    expect(Aeris_spacing).toBeCloseTo(entry.spacing, 9);
    const Aeris_range = JSON.parse(await matrix_page.getAttribute("html", "data-visible-logical-range"));
    const Aeris_axis_width = Number(await matrix_page.getAttribute("html", "data-price-axis-width"));
    const Aeris_price_extent = await matrix_page.evaluate(() => [
      window.__chart.coordinate_to_price(0),
      window.__chart.coordinate_to_price(window.__chart.wasm.pane_height(0) - 1),
    ]);
    const Aeris = PNG.sync.read(await matrix_page.screenshot({ animations: "disabled", fullPage: false }));

    await matrix_page.goto(`${test_base_url}/reference.html?${query}`);
    await matrix_page.waitForFunction(() => document.documentElement.dataset.ready === "true");
    await matrix_page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    await settle_page(matrix_page);
    const reference_spacing = Number(await matrix_page.getAttribute("html", "data-bar-spacing"));
    expect(reference_spacing).toBeCloseTo(entry.spacing, 9);
    const reference_range = JSON.parse(await matrix_page.getAttribute("html", "data-visible-logical-range"));
    const reference_axis_width = Number(await matrix_page.getAttribute("html", "data-price-axis-width"));
    const reference_pane_height = await matrix_page.evaluate(() => window.__reference.chart.panes()[0].getHeight());
    // Compact axes are intentionally narrower than the reference's strips; the shared
    // content contract is equal ranges, spacing, and price extents (asserted below).
    expect(Aeris_axis_width).toBeLessThan(reference_axis_width);
    const reference_price_extent = await matrix_page.evaluate(() => [
      window.__reference.series.coordinateToPrice(0),
      window.__reference.series.coordinateToPrice(window.__reference.chart.panes()[0].getHeight() - 1),
    ]);
    const reference = PNG.sync.read(await matrix_page.screenshot({ animations: "disabled", fullPage: false }));
    expect([Aeris.width, Aeris.height]).toEqual([
      Math.round(fixture.css_width * entry.dpr),
      Math.round(fixture.css_height * entry.dpr),
    ]);
    const { report, visuals } = regional_fidelity_report(
      Aeris,
      reference,
      entry.dpr,
      { price_axis_width: Aeris_axis_width, time_axis_height: fixture.time_axis_height },
      {
        price_axis_width: reference_axis_width,
        time_axis_height: fixture.css_height - reference_pane_height,
      },
    );
    matrix[entry.name] = report;
    console.log(`${entry.name}: axis ${Aeris_axis_width}px, ranges Aeris ${JSON.stringify(Aeris_range)} reference ${JSON.stringify(reference_range)}, price extents Aeris ${JSON.stringify(Aeris_price_extent)} reference ${JSON.stringify(reference_price_extent)}; ${JSON.stringify(report)}`);
    if (entry.spacing === 50) {
      await test_info.attach(`${entry.name}-Aeris.png`, { body: PNG.sync.write(Aeris), contentType: "image/png" });
      await test_info.attach(`${entry.name}-reference.png`, { body: PNG.sync.write(reference), contentType: "image/png" });
      await test_info.attach(`${entry.name}-diff.png`, { body: PNG.sync.write(visuals.full), contentType: "image/png" });
    }
    await context.close();
  }
  await test_info.attach("aeris_charts-reference-matrix.json", {
    body: Buffer.from(JSON.stringify({ ref_version: "5.2.0", cases: matrix }, null, 2)),
    contentType: "application/json",
  });
  for (const entry of cases) {
    const report = matrix[entry.name];
    for (const [region, ceiling] of Object.entries(entry.maximum)) {
      expect(report[region].perceptual_percent / 100, `${entry.name}/${region} exceeded its reference ceiling`).toBeLessThanOrEqual(ceiling);
    }
  }
});

test("reference marker and overlay-volume fixtures report regional fidelity @machine", async ({ browser }, test_info) => {
  expect(reference_features.fixture).toBe(fixture.name);
  expect(reference_features.ref_version).toBe("5.2.0");
  const feature_reports = {};
  const captures = {};
  for (const feature of ["base", "markers", "volume"]) {
    const context = await browser.newContext({
      viewport: { width: fixture.css_width, height: fixture.css_height },
      deviceScaleFactor: fixture.pixel_ratio,
      colorScheme: "light",
    });
    const feature_page = await context.newPage();
    const query = new URLSearchParams({
      runtimeTest: "presentedFrame",
      backend: "canvas2d",
      dpr: String(fixture.pixel_ratio),
      spacing: "6",
      theme: "light",
      feature,
    });
    await feature_page.goto(`${test_base_url}/?${query}`);
    await wait_for_chart(feature_page);
    await settle_page(feature_page);
    const axis_width = Number(await feature_page.getAttribute("html", "data-price-axis-width"));
    const Aeris_range = JSON.parse(await feature_page.getAttribute("html", "data-visible-logical-range"));
    const Aeris_price_extent = await feature_page.evaluate(() => [
      window.__chart.coordinate_to_price(0),
      window.__chart.coordinate_to_price(window.__chart.wasm.pane_height(0) - 1),
    ]);
    const Aeris = PNG.sync.read(await feature_page.screenshot({ animations: "disabled", fullPage: false }));

    await feature_page.goto(`${test_base_url}/reference.html?${query}`);
    await feature_page.waitForFunction(() => document.documentElement.dataset.ready === "true");
    await feature_page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    await settle_page(feature_page);
    const reference_axis_width = Number(await feature_page.getAttribute("html", "data-price-axis-width"));
    const reference_range = JSON.parse(await feature_page.getAttribute("html", "data-visible-logical-range"));
    const reference_price_extent = await feature_page.evaluate(() => [
      window.__reference.series.coordinateToPrice(0),
      window.__reference.series.coordinateToPrice(window.__reference.chart.panes()[0].getHeight() - 1),
    ]);
    const reference_pane_height = await feature_page.evaluate(() => window.__reference.chart.panes()[0].getHeight());
    // Compact axes are intentionally narrower than the reference's strips; the shared
    // content contract is equal ranges and price extents (asserted below).
    expect(axis_width).toBeLessThan(reference_axis_width);
    // Same right-anchored last bar on both sides; the wider Aeris pane shows more bars to
    // the left (pinning a range here would refit bar spacing and break pixel alignment, so
    // the ranges agree on the anchor and order on the edge instead). Edge prices agree
    // relatively: the extra visible bars can nudge autoscale extrema, so this proves mapping
    // sanity (1e-3) rather than bit-exact autoscale inputs.
    expect(Aeris_range.to).toBe(reference_range.to);
    expect(Aeris_range.from).toBeLessThan(reference_range.from);
    for (const [got, want] of [
      [Aeris_price_extent[0], reference_price_extent[0]],
      [Aeris_price_extent[1], reference_price_extent[1]],
    ]) {
      expect(Math.abs(got - want) / Math.abs(want)).toBeLessThan(1e-3);
    }
    const reference = PNG.sync.read(await feature_page.screenshot({ animations: "disabled", fullPage: false }));
    const { report, visuals } = regional_fidelity_report(
      Aeris,
      reference,
      fixture.pixel_ratio,
      { price_axis_width: axis_width, time_axis_height: fixture.time_axis_height },
      {
        price_axis_width: reference_axis_width,
        time_axis_height: fixture.css_height - reference_pane_height,
      },
    );
    captures[feature] = { Aeris, reference };
    console.log(`${feature} ranges: Aeris ${JSON.stringify(Aeris_range)} reference ${JSON.stringify(reference_range)}, price extents Aeris ${JSON.stringify(Aeris_price_extent)} reference ${JSON.stringify(reference_price_extent)}`);
    feature_reports[feature] = report;
    console.log(`${feature}: ${JSON.stringify(report)}`);
    await test_info.attach(`${feature}-Aeris.png`, { body: PNG.sync.write(Aeris), contentType: "image/png" });
    await test_info.attach(`${feature}-reference.png`, { body: PNG.sync.write(reference), contentType: "image/png" });
    await test_info.attach(`${feature}-diff.png`, { body: PNG.sync.write(visuals.full), contentType: "image/png" });
    await context.close();
  }
  const footprints = {};
  for (const feature of ["markers", "volume"]) {
    footprints[feature] = {
      Aeris: changed_footprint(captures.base.Aeris, captures[feature].Aeris),
      reference: changed_footprint(captures.base.reference, captures[feature].reference),
    };
  }
  console.log(`feature footprints: ${JSON.stringify(footprints)}`);
  await test_info.attach("aeris_charts-reference-features.json", {
    body: Buffer.from(JSON.stringify({ ref_version: "5.2.0", features: feature_reports, footprints }, null, 2)),
    contentType: "application/json",
  });
  for (const entry of reference_features.cases) {
    const report = feature_reports[entry.name];
    for (const [region, ceiling] of Object.entries(entry.maximum)) {
      expect(report[region].perceptual_percent / 100, `${entry.name}/${region} exceeded its reference ceiling`).toBeLessThanOrEqual(ceiling);
    }
  }
});


test("screenshot add_top_layer keeps shared axes optional on both backends", async ({ page }) => {
  const captures = {};
  for (const backend of ["canvas2d", "auto"]) {
    await page.goto(`/?runtimeTest=presentedFrame&backend=${backend}&forceFallbackAdapter=1`);
    await wait_for_chart(page);
    captures[backend] = {
      with_top: await page.evaluate(() => window.__chart.take_screenshot(true).toDataURL("image/png")),
    };
    // Capture the live surface after the ordinary full snapshot has refreshed the retained frame,
    // then isolate the pane-only snapshot's clear/copy/restore cycle.
    const presented_before = await page.screenshot({ animations: "disabled", fullPage: false });
    captures[backend].pane_only = await page.evaluate(() =>
      window.__chart.take_screenshot(false).toDataURL("image/png"),
    );
    const presented_after = await page.screenshot({ animations: "disabled", fullPage: false });
    expect(
      presented_after.equals(presented_before),
      `take_screenshot(false) must restore the live ${backend} surface`,
    ).toBe(true);
  }

  // The Canvas2D snapshot path is deterministic regardless of the live backend.
  expect(captures.auto.with_top).toBe(captures.canvas2d.with_top);
  expect(captures.auto.pane_only).toBe(captures.canvas2d.pane_only);

  const with_top = PNG.sync.read(Buffer.from(captures.auto.with_top.split(",")[1], "base64"));
  const pane_only = PNG.sync.read(Buffer.from(captures.auto.pane_only.split(",")[1], "base64"));
  const pane_width = Math.round((fixture.css_width - fixture.price_axis_width) * fixture.pixel_ratio);
  const pane_height = Math.round((fixture.css_height - fixture.time_axis_height) * fixture.pixel_ratio);
  expect(rgba_diff(
    crop_png(with_top, pane_width, 0, with_top.width - pane_width, pane_height).data,
    crop_png(pane_only, pane_width, 0, pane_only.width - pane_width, pane_height).data,
    0,
  ).different_pixels, "price-axis chrome must be omitted with add_top_layer=false").toBeGreaterThan(0);
  expect(rgba_diff(
    crop_png(with_top, 0, pane_height, pane_width, with_top.height - pane_height).data,
    crop_png(pane_only, 0, pane_height, pane_width, pane_only.height - pane_height).data,
    0,
  ).different_pixels, "time-axis chrome must be omitted with add_top_layer=false").toBeGreaterThan(0);
});

// Exercise the ordinary demo and its default price tick through real pointer events. In this
// view a price tick is smaller than a device pixel; rounding motion first would skip ticks.
async function drag_position(page, backend, kind, theme) {
  await page.goto(`/?backend=${backend}&theme=${theme}&forceFallbackAdapter=1`);
  await page.waitForFunction(() => window.__main && window.__chart.time_scale().get_visible_logical_range());
  await page.evaluate(() => document.fonts.ready);
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const placement = await page.evaluate(() => {
    const chart = window.__chart;
    const series = window.__main;
    const range = chart.time_scale().get_visible_logical_range();
    const logical = Math.floor(range.from + (range.to - range.from) * 0.4);
    const bar = series.data_by_index(logical);
    const box = document.getElementById("chart_container").getBoundingClientRect();
    return {
      x: box.left + chart.time_scale().logical_to_coordinate(logical),
      y: box.top + series.price_to_coordinate(bar.close),
      tick: chart.trading().state().instrument.tick_size ?? series.options().price_format.min_move,
    };
  });
  expect(placement.tick).toBe(0.01);
  await page.click(`#drawings_group [data-tool="${kind}"]`);
  await page.mouse.click(placement.x, placement.y);
  const initial = await page.evaluate(() => window.__chart.drawings().at(-1).points());
  for (const point of initial) {
    expect(point.price / placement.tick).toBeCloseTo(Math.round(point.price / placement.tick), 8);
  }
  const controls = await page.evaluate(() => {
    const [entry, target, stop] = window.__chart.drawings().at(-1).points();
    const box = document.getElementById("chart_container").getBoundingClientRect();
    const scale = window.__chart.time_scale();
    // The public coordinate query takes integer bar indices; interpolate a drawing's
    // fractional logical coordinate using those adjacent bar centers.
    const x = (p) => {
      const logical = Math.floor(p.logical);
      const left = scale.logical_to_coordinate(logical);
      return box.left + left + (scale.logical_to_coordinate(logical + 1) - left) * (p.logical - logical);
    };
    const y = (p) => box.top + window.__main.price_to_coordinate(p.price);
    return [[x(entry), y(target)], [x(target), y(entry)], [x(entry), y(stop)]];
  });
  await page.mouse.move(2, 2);
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const screenshot = PNG.sync.read(await page.screenshot({ animations: "disabled" }));
  const dpr = await page.evaluate(() => window.devicePixelRatio);
  const squares = controls.map(([x, y]) => {
    const cx = Math.round(x * dpr);
    const cy = Math.round(y * dpr);
    const radius = Math.ceil(6 * dpr);
    const pixels = [];
    for (let py = cy - radius; py <= cy + radius; py += 1) {
      for (let px = cx - radius; px <= cx + radius; px += 1) {
        const offset = (py * screenshot.width + px) * 4;
        pixels.push(...screenshot.data.subarray(offset, offset + 4));
      }
    }
    const center = (cy * screenshot.width + cx) * 4;
    const fill = theme === "light" ? [255, 255, 255] : [0, 0, 0];
    expect([...screenshot.data.subarray(center, center + 3)], "square anchor has an opaque theme fill").toEqual(fill);
    let border_pixels = 0;
    for (let offset = 0; offset < pixels.length; offset += 4) {
      if (pixels[offset] < 20 && Math.abs(pixels[offset + 1] - 145) < 20 && pixels[offset + 2] > 235) border_pixels += 1;
    }
    expect(border_pixels, "square anchor has a visible thin primary border").toBeGreaterThan(12);
    return pixels;
  });
  const trail = [];
  for (const part of ["body", "target", "entry", "stop"]) {
    const grab = await page.evaluate(({ part, tick }) => {
      const [entry, target, stop] = window.__chart.drawings().at(-1).points();
      const point = part === "target" ? target : part === "stop" ? stop : entry;
      const box = document.getElementById("chart_container").getBoundingClientRect();
      const y = window.__main.price_to_coordinate(point.price);
      const scale = window.__chart.time_scale();
      const logical = Math.floor(entry.logical);
      const left = scale.logical_to_coordinate(logical);
      const bar_px = scale.logical_to_coordinate(logical + 1) - left;
      return {
        x: box.left + left + bar_px * (entry.logical - logical) + (part === "body" ? 30 : 0),
        y: box.top + y,
        tick_px: window.__main.price_to_coordinate(point.price + tick) - y,
        bar_px,
        start: [entry, target, stop],
      };
    }, { part, tick: placement.tick });
    expect(Math.abs(grab.tick_px) * dpr).toBeLessThan(1);
    await page.mouse.move(grab.x, grab.y);
    await page.mouse.down();
    // A press moves nothing until it travels the shared 5 px drag threshold; past it the handle
    // follows the pointer exactly, back to sub-pixel offsets from the press too.
    await page.mouse.move(grab.x, grab.y + 12);
    for (const [fraction, ticks] of [[0.3, 0], [0.7, 1], [1.3, 1], [1.7, 2]]) {
      await page.mouse.move(grab.x, grab.y + grab.tick_px * fraction);
      const points = await page.evaluate(() => window.__chart.drawings().at(-1).points());
      for (let index = 0; index < points.length; index += 1) {
        const changes = part === "body" || index === (part === "target" ? 1 : part === "stop" ? 2 : 0);
        expect(points[index].price, `${part} at ${fraction} tick`).toBeCloseTo(grab.start[index].price + (changes ? ticks * placement.tick : 0), 8);
      }
      trail.push(points);
    }
    await page.mouse.up();
  }
  await page.evaluate(() => window.__chart.subscribe_crosshair_move((event) => {
    window.__position_cursor_logical = event.logical;
  }));
  for (const part of ["body", "entry", "extent"]) {
    const grab = await page.evaluate((part) => {
      const start = window.__chart.drawings().at(-1).points();
      const box = document.getElementById("chart_container").getBoundingClientRect();
      const scale = window.__chart.time_scale();
      const point = start[part === "extent" ? 1 : 0];
      const logical = Math.floor(point.logical);
      const left = scale.logical_to_coordinate(logical);
      const spacing = scale.logical_to_coordinate(logical + 1) - left;
      return {
        x: box.left + left + spacing * (point.logical - logical) + (part === "body" ? 30 : 0),
        y: box.top + window.__main.price_to_coordinate(start[0].price),
        spacing, start,
      };
    }, part);
    await page.mouse.move(grab.x, grab.y);
    const cursor_start = await page.evaluate(() => window.__position_cursor_logical);
    expect(Number.isInteger(cursor_start)).toBe(true);
    await page.mouse.down();
    // Cross the shared 5 px drag threshold first, as above; the pull keeps the press's time.
    await page.mouse.move(grab.x, grab.y + 12);
    for (const fraction of [0.2, 0.3, 0.7, 1.3, 1.7, -0.2, -0.3, -0.7, -1.3, -1.7]) {
      await page.mouse.move(grab.x + grab.spacing * fraction, grab.y);
      const { points, cursor } = await page.evaluate(() => ({
        points: window.__chart.drawings().at(-1).points(),
        cursor: window.__position_cursor_logical,
      }));
      for (let index = 0; index < points.length; index += 1) {
        const expected = part === "body" ? grab.start[index].logical + cursor - cursor_start
          : ((part === "entry" && index !== 1) || (part === "extent" && index === 1)) ? cursor
          : grab.start[index].logical;
        expect(points[index].logical, `${part} follows cursor at ${fraction} bars`).toBeCloseTo(expected, 6);
        expect(points[index].price).toBeCloseTo(grab.start[index].price, 8);
      }
      trail.push(points);
    }
    await page.mouse.up();
  }
  return { initial, trail, squares, backend: await page.evaluate(() => window.__chart.backend()) };
}

for (const kind of ["long_position", "short_position"]) {
  for (const theme of ["light", "dark"]) {
    test(`${kind} uses crosshair time steps, exact price ticks and filled rounded anchors in ${theme}`, async ({ page }) => {
      const gpu = await drag_position(page, "auto", kind, theme);
      const canvas = await drag_position(page, "canvas2d", kind, theme);
      expect(gpu.backend).toBe("webgpu");
      expect(canvas.backend).toBe("canvas2d");
      expect(gpu.initial).toEqual(canvas.initial);
      expect(gpu.trail).toEqual(canvas.trail);
      for (let square = 0; square < gpu.squares.length; square += 1) {
        let different = 0;
        for (let offset = 0; offset < gpu.squares[square].length; offset += 4) {
          const delta = Math.max(...[0, 1, 2, 3].map((channel) => Math.abs(gpu.squares[square][offset + channel] - canvas.squares[square][offset + channel])));
          if (delta > 96) different += 1;
        }
        expect(different, "anchor fill and border match across WebGPU and Canvas2D").toBe(0);
      }
    });
  }
}

for (const kind of ["long_position", "short_position"]) {
  test(`${kind} stats render amounts, quantity and borderless labels on both backends`, async ({ page }) => {
    await page.addInitScript(() => {
      const original = CanvasRenderingContext2D.prototype.fillText;
      window.__position_stats_text = [];
      CanvasRenderingContext2D.prototype.fillText = function (text, ...args) {
        if (/^(Target:|Stop:|Open P&L:|Closed P&L:|Risk\/reward ratio:)/.test(String(text))) {
          window.__position_stats_text.push(String(text));
        }
        return original.call(this, text, ...args);
      };
    });
    for (const backend of ["auto", "canvas2d"]) {
      await page.goto(`/?backend=${backend}&theme=dark&forceFallbackAdapter=1`);
      await wait_for_chart(page);
      await page.evaluate(() => window.__chart.trading().apply_snapshot({
        instrument: { tick_size: 0.25, point_value: 20, quantity_precision: 3 },
        positions: [], orders: [], executions: [],
      }));
      const placement = await page.evaluate(() => {
        const range = window.__chart.time_scale().get_visible_logical_range();
        const logical = Math.floor(range.from + (range.to - range.from) * 0.4);
        const bar = window.__main.data_by_index(logical);
        const box = document.getElementById("chart_container").getBoundingClientRect();
        return { x: box.left + window.__chart.time_scale().logical_to_coordinate(logical), y: box.top + window.__main.price_to_coordinate(bar.close) };
      });
      await page.click(`#drawings_group [data-tool="${kind}"]`);
      await page.mouse.click(placement.x, placement.y);
      await page.mouse.move(2, 2);
      await settle_page(page);
      const stats = await page.evaluate(() => {
        const [entry, target, stop] = window.__chart.drawings().at(-1).points();
        const risk = Math.abs(entry.price - stop.price);
        const reward = Math.abs(target.price - entry.price);
        return { risk, reward, quantity: 250 / risk / 20, text: window.__position_stats_text };
      });
      expect(stats.text.some((text) => text.startsWith("Target:") && text.endsWith(", Amount: 1500"))).toBe(true);
      expect(stats.text.some((text) => text.startsWith("Stop:") && text.endsWith(", Amount: 750"))).toBe(true);
      const quantity = String(Number(stats.quantity.toFixed(3)));
      expect(stats.text.some((text) => text.includes(`, Qty: ${quantity}`))).toBe(true);
      expect(stats.text).toContain("Risk/reward ratio: 2");
      // A contrast outline would read white on these surfaces; the solid label carries none.
      for (const color of ["#0000ff", "#000000"]) {
        const label = await page.evaluate((color) => {
          window.__chart.apply_options({ layout: { background: { type: "solid", color } } });
          const [entry, target] = window.__chart.drawings().at(-1).points();
          const scale = window.__chart.time_scale();
          const box = document.getElementById("chart_container").getBoundingClientRect();
          const x = (scale.logical_to_coordinate(entry.logical) + scale.logical_to_coordinate(target.logical)) / 2;
          const target_y = window.__main.price_to_coordinate(target.price);
          const entry_y = window.__main.price_to_coordinate(entry.price);
          const y = target_y + (target_y < entry_y ? -14 : 14);
          const size = Math.max(window.__chart.options().layout.fontSize, 11);
          return { x: box.left + x, top: box.top + y - (size * 1.25 + 6) / 2, dpr: window.devicePixelRatio };
        }, color);
        await settle_page(page);
        const png = PNG.sync.read(await page.screenshot({ animations: "disabled", path: color === "#000000" ? `test-results/position-stats-${kind}-${backend}.png` : undefined }));
        let best = 0;
        for (let dy = -2; dy <= 2; dy += 1) {
          const y = Math.round(label.top * label.dpr) + dy;
          let pixels = 0;
          for (let dx = -15; dx <= 15; dx += 1) {
            const x = Math.round(label.x * label.dpr) + dx;
            const offset = (y * png.width + x) * 4;
            if ([0, 1, 2].every((channel) => png.data[offset + channel] > 239)) pixels += 1;
          }
          best = Math.max(best, pixels);
        }
        expect(best, `${backend} label has no white outline on ${color}`).toBeLessThan(3);
      }
      await page.locator("#position_account_size").fill("2000");
      await page.locator("#position_account_size").press("Tab");
      await page.locator("#position_risk_percent").fill("2");
      await page.locator("#position_risk_percent").press("Tab");
      await settle_page(page);
      const updated = await page.evaluate(() => ({ options: window.__chart.drawings().at(-1).options(), text: window.__position_stats_text }));
      expect(updated.options.position_account_size).toBe(2000);
      expect(updated.options.position_risk_percent).toBe(2);
      expect(updated.text.some((text) => text.startsWith("Target:") && text.endsWith(", Amount: 2080"))).toBe(true);
      expect(updated.text.some((text) => text.startsWith("Stop:") && text.endsWith(", Amount: 1960"))).toBe(true);
    }
  });
}

// Measuring tools emit crisp device-pixel rules and arrow shafts over a translucent area fill;
// the arrowheads, label glyphs and the fill's own edges are antialiased. The interior of each
// measured area (two pixels in from every edge, clear of the arrow tips) must therefore be
// pixel-identical between WebGPU and Canvas2D, for rising and falling pulls in both themes.
async function render_measures(page, backend, theme) {
  await page.goto(`/?backend=${backend}&theme=${theme}&forceFallbackAdapter=1`);
  await page.waitForFunction(() => window.__main && window.__chart.time_scale().get_visible_logical_range());
  const areas = await page.evaluate(() => {
    const chart = window.__chart;
    const series = window.__main;
    const range = chart.time_scale().get_visible_logical_range();
    const logical = (fraction) => Math.round(range.from + (range.to - range.from) * fraction);
    const close = (index) => series.data_by_index(index).close;
    const specs = [
      ["price_range", 0.1, 0.22, 0.97, 1.04],
      ["date_range", 0.48, 0.3, 1.02, 0.98],
      ["date_and_price_range", 0.55, 0.75, 1.03, 0.95],
    ];
    const box = document.getElementById("chart_container").getBoundingClientRect();
    const left = box.left + chart.wasm.pane_left();
    const css = (l, price) => [left + chart.time_scale().logical_to_coordinate(l), box.top + series.price_to_coordinate(price)];
    const result = [];
    for (const [kind, from, to, start_scale, end_scale] of specs) {
      const base = close(logical(from));
      const points = [
        { logical: logical(from), price: Math.round(base * start_scale * 100) / 100 },
        { logical: logical(to), price: Math.round(base * end_scale * 100) / 100 },
      ];
      chart.add_drawing(kind, points);
      result.push(points.map((point) => css(point.logical, point.price)));
    }
    // The transient Shift-click measure, pulled downward.
    const a = logical(0.78);
    const b = logical(0.92);
    const start = [chart.time_scale().logical_to_coordinate(a), series.price_to_coordinate(close(a) * 1.02)];
    const end = [chart.time_scale().logical_to_coordinate(b), series.price_to_coordinate(close(a) * 0.97)];
    chart.wasm.measure_pointer_down(start[0], start[1], true, false);
    chart.wasm.measure_pointer_move(end[0], end[1], false);
    chart.wasm.measure_pointer_up(end[0], end[1], false);
    const measured = JSON.parse(chart.wasm.measure_points_json());
    result.push(measured.map((point) => css(point.logical, point.price)));
    return result;
  });
  await page.mouse.move(2, 2);
  await settle_page(page);
  const png = PNG.sync.read(await page.screenshot({ animations: "disabled" }));
  const dpr = await page.evaluate(() => window.devicePixelRatio);
  return { png, dpr, areas, backend: await page.evaluate(() => window.__chart.backend()) };
}

for (const theme of ["light", "dark"]) {
  test(`measure areas are pixel-identical across WebGPU and Canvas2D in ${theme}`, async ({ page }) => {
    const gpu = await render_measures(page, "auto", theme);
    const canvas = await render_measures(page, "canvas2d", theme);
    expect(gpu.backend).toBe("webgpu");
    expect(canvas.backend).toBe("canvas2d");
    expect(gpu.areas).toEqual(canvas.areas);
    for (const [[sx, sy], [ex, ey]] of gpu.areas) {
      const left = Math.round(Math.min(sx, ex) * gpu.dpr) + 2;
      const right = Math.round(Math.max(sx, ex) * gpu.dpr) - 2;
      const top = Math.round(Math.min(sy, ey) * gpu.dpr) + 2;
      const bottom = Math.round(Math.max(sy, ey) * gpu.dpr) - 2;
      // Antialiased arrowheads sit at the arrow tips on the end edges.
      const tips = [[(sx + ex) / 2, ey], [ex, (sy + ey) / 2]].map(([x, y]) => [x * gpu.dpr, y * gpu.dpr]);
      let compared = 0;
      let different = 0;
      for (let y = top; y <= bottom; y += 1) {
        for (let x = left; x <= right; x += 1) {
          if (tips.some(([tx, ty]) => Math.abs(x - tx) <= 10 * gpu.dpr && Math.abs(y - ty) <= 10 * gpu.dpr)) continue;
          const offset = (y * gpu.png.width + x) * 4;
          const delta = Math.max(...[0, 1, 2].map((channel) => Math.abs(gpu.png.data[offset + channel] - canvas.png.data[offset + channel])));
          compared += 1;
          if (delta > 3) different += 1;
        }
      }
      expect(compared, "measure area was sampled").toBeGreaterThan(400);
      expect(different, `measure area ${left},${top}–${right},${bottom} matches across backends`).toBe(0);
    }
  });
}
