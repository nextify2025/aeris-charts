import { expect } from "@playwright/test";
import { test, wait_for_chart } from "./page-ready.mjs";
import { PNG } from "pngjs";

// The demo's RSI(14) toggle exercises the separate-pane API end-to-end: move_to_pane stacks a
// dedicated pane with its own price scale and a draggable separator, cross-pane hover/click
// affordances work there, and removing the series prunes the empty pane.

const PURPLE = [171, 71, 188]; // #ab47bc — the demo's RSI stroke
const BLUE = [0, 145, 255]; // semantic primary #0091ff — selection anchor border

async function capture(page) {
  const data_url = await page.evaluate(() => window.__chart.take_screenshot().toDataURL("image/png"));
  return PNG.sync.read(Buffer.from(data_url.split(",")[1], "base64"));
}

function count_color_below(png, target, y_min, tol = 30) {
  let n = 0;
  for (let y = Math.ceil(y_min); y < png.height; y++) {
    for (let x = 0; x < png.width; x++) {
      const o = (y * png.width + x) * 4;
      if (
        Math.abs(png.data[o] - target[0]) <= tol &&
        Math.abs(png.data[o + 1] - target[1]) <= tol &&
        Math.abs(png.data[o + 2] - target[2]) <= tol
      ) n += 1;
    }
  }
  return n;
}

async function overlay_cursor(page) {
  return page.evaluate(() => {
    const canvases = document.querySelectorAll("#chart_container canvas");
    return canvases[canvases.length - 1].style.cursor;
  });
}

test("RSI toggle stacks a separate pane with its own scale; unchecking prunes it", async ({ page }) => {
  await page.goto("/?backend=canvas2d&forceFallbackAdapter=1");
  await wait_for_chart(page);
  expect(await page.evaluate(() => window.__chart.panes().length)).toBe(1);

  await page.check("#rsi_toggle");
  await wait_for_chart(page);
  expect(await page.evaluate(() => window.__chart.panes().length)).toBe(2);
  const sep = await page.evaluate(() => Array.from(window.__chart.wasm.pane_separator_ys()));
  expect(sep.length).toBe(1);

  // The oscillator's own scale maps mid-range RSI into pane 1 (below the separator)…
  const rsi_y = await page.evaluate(() => window.__rsi.price_to_coordinate(50));
  expect(rsi_y).toBeGreaterThan(sep[0]);

  // Public coordinates are chart-content space, never pane-local: the RSI series converts inside
  // pane 1, the chart-level inverse selects pane 1's default scale for that y (it used to
  // extrapolate the main series), and the chart-level pair agrees with the main series on pane 0.
  const coordinates = await page.evaluate(() => {
    const chart = window.__chart;
    const rsi_geometry = chart.panes()[1].get_geometry();
    const y = window.__rsi.price_to_coordinate(50);
    const main_price = window.__main.coordinate_to_price(rsi_geometry.top / 2);
    return {
      top: rsi_geometry.top,
      bottom: rsi_geometry.top + rsi_geometry.height,
      y,
      price_at_y: chart.coordinate_to_price(y),
      main_price,
      main_round_trip: chart.coordinate_to_price(window.__main.price_to_coordinate(main_price)),
      chart_y: chart.price_to_coordinate(main_price),
      main_y: window.__main.price_to_coordinate(main_price),
    };
  });
  expect(coordinates.y).toBeGreaterThanOrEqual(coordinates.top);
  expect(coordinates.y).toBeLessThanOrEqual(coordinates.bottom);
  expect(Math.abs(coordinates.price_at_y - 50)).toBeLessThan(1e-6);
  expect(Math.abs(coordinates.main_round_trip - coordinates.main_price)).toBeLessThan(1e-6);
  expect(coordinates.chart_y).toBeCloseTo(coordinates.main_y, 6);
  // …and the purple stroke actually paints there (with the 70/30 guide lines).
  const shot = await capture(page);
  expect(count_color_below(shot, PURPLE, sep[0] + 2)).toBeGreaterThan(50);

  // Cross-pane affordances: hovering the RSI line shows the pointer cursor…
  // (page.mouse works in page coords — offset the chart-relative point by the container rect).
  const spot = await page.evaluate(() => {
    const range = window.__chart.time_scale().get_visible_logical_range();
    const index = Math.floor((range.from + range.to) / 2);
    const point = window.__rsi.data_by_index(index);
    const rect = document.querySelector("#chart_container").getBoundingClientRect();
    return {
      x: rect.left + window.__chart.time_scale().logical_to_coordinate(index),
      y: rect.top + window.__rsi.price_to_coordinate(point.value),
    };
  });
  await page.mouse.move(spot.x, spot.y);
  expect(await overlay_cursor(page)).toBe("pointer");
  // …and clicking selects it: accent-blue anchors paint in pane 1.
  await page.mouse.click(spot.x, spot.y);
  const selected = await capture(page);
  expect(count_color_below(selected, BLUE, sep[0] + 2)).toBeGreaterThan(20);

  // Uncheck: the series is removed and its empty pane pruned.
  await page.uncheck("#rsi_toggle");
  await wait_for_chart(page);
  expect(await page.evaluate(() => window.__chart.panes().length)).toBe(1);
});

test("pane divider follows the axis border color (theme-aware) until pinned", async ({ page }) => {
  await page.goto("/?backend=canvas2d&forceFallbackAdapter=1");
  await wait_for_chart(page);
  await page.check("#rsi_toggle");
  await wait_for_chart(page);

  // The divider row near the separator coordinate whose pixels (at several x) match a color.
  // Read fresh each sample: a theme switch can shift the demo page's own chrome (the chart
  // container gains/loses a few px), which moves the separator.
  const divider_color = async () => {
    const sep_y = await page.evaluate(() => Array.from(window.__chart.wasm.pane_separator_ys())[0]);
    const shot = await capture(page);
    const scale = shot.width / (await page.evaluate(() => document.querySelector("#chart_container").getBoundingClientRect().width));
    const y0 = Math.round(sep_y * scale);
    const counts = new Map();
    for (let y = y0 - 2; y <= y0 + 2; y++) {
      for (const x of [10, 100, 200, 400, 600].map((v) => Math.round(v * scale))) {
        const o = (y * shot.width + x) * 4;
        const key = `${shot.data[o]},${shot.data[o + 1]},${shot.data[o + 2]}`;
        counts.set(key, (counts.get(key) ?? 0) + 1);
      }
    }
    return counts;
  };
  const hex = (s) => [1, 3, 5].map((i) => Number.parseInt(s.slice(i, i + 2), 16));
  const border_color = () => page.evaluate(() => window.__chart.options().rightPriceScale.borderColor);

  // Default (light): the divider paints in the axis border color.
  let expected = hex(await border_color()).join(",");
  expect((await divider_color()).get(expected) ?? 0, `divider must use border color ${expected}`).toBeGreaterThanOrEqual(3);

  // Dark theme: the axis border changes and the divider tracks it (#333333).
  await page.selectOption("#theme_select", "dark");
  await wait_for_chart(page);
  expected = hex(await border_color()).join(",");
  expect(expected).toBe("51,51,51");
  expect((await divider_color()).get(expected) ?? 0, `dark divider must use border color ${expected}`).toBeGreaterThanOrEqual(3);

  // An explicit separator color pins it through theme switches.
  await page.locator("#pane_separator_color").evaluate((el) => {
    el.value = "#ff8800";
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await wait_for_chart(page);
  await page.selectOption("#theme_select", "light");
  await wait_for_chart(page);
  expect((await divider_color()).get("255,136,0") ?? 0, "pinned divider color survives theme switch").toBeGreaterThanOrEqual(3);
});

test("crosshair hides on separator hover and during the resize drag, then resumes", async ({ page }) => {
  await page.goto("/?backend=canvas2d&forceFallbackAdapter=1");
  await wait_for_chart(page);
  await page.check("#rsi_toggle");
  await wait_for_chart(page);

  const geom = await page.evaluate(() => {
    const rect = document.querySelector("#chart_container").getBoundingClientRect();
    const sep = Array.from(window.__chart.wasm.pane_separator_ys())[0];
    return { left: rect.left, top: rect.top, sep };
  });
  const legend = () => page.evaluate(() => document.getElementById("legend").textContent);
  const pane_x = geom.left + 400;
  const pane_y = geom.top + geom.sep - 60; // inside pane 0, above the separator
  const sep_page_y = geom.top + geom.sep;

  // Hover inside the pane: the crosshair feeds the OHLC legend.
  await page.mouse.move(pane_x, pane_y);
  expect(await legend(), "crosshair feeds the legend over the pane").toContain("H");

  // Hover the divider itself: chrome behavior — the crosshair hides (legend resets).
  await page.mouse.move(pane_x, sep_page_y);
  expect(await legend(), "separator hover hides the crosshair").toBe("O — H — L — C —");

  // Drag the divider down: the crosshair stays hidden through the resize (no frozen frame).
  await page.mouse.move(pane_x, sep_page_y);
  await page.mouse.down();
  await page.mouse.move(pane_x, sep_page_y + 40, { steps: 5 });
  expect(await legend(), "crosshair hidden during the separator drag").toBe("O — H — L — C —");
  const moved_sep = await page.evaluate(() => Array.from(window.__chart.wasm.pane_separator_ys())[0]);
  expect(moved_sep).toBeGreaterThan(geom.sep); // the drag actually resized the pane
  await page.mouse.up();

  // Back inside the pane the crosshair resumes tracking.
  await page.mouse.move(pane_x, geom.top + moved_sep - 60);
  expect(await legend(), "crosshair resumes over the pane").toContain("H");
});

test("pane divider returns to its grab point after hitting the height clamp", async ({ page }) => {
  await page.goto("/?backend=canvas2d&forceFallbackAdapter=1");
  await wait_for_chart(page);
  await page.check("#rsi_toggle");
  await wait_for_chart(page);
  const geometry = await page.evaluate(() => {
    const rect = document.querySelector("#chart_container").getBoundingClientRect();
    return { x: rect.left + 400, top: rect.top,
      separator: Array.from(window.__chart.wasm.pane_separator_ys())[0] };
  });
  const separator = () => page.evaluate(() => Array.from(window.__chart.wasm.pane_separator_ys())[0]);
  await page.mouse.move(geometry.x, geometry.top + geometry.separator);
  await page.mouse.down();
  await page.mouse.move(geometry.x, geometry.top + geometry.separator + 400);
  const clamped = await separator();
  await page.mouse.move(geometry.x, geometry.top + geometry.separator + 60);
  const reversed = await separator();
  await page.mouse.up();
  expect(clamped).toBeGreaterThan(geometry.separator + 60);
  expect(reversed).toBeCloseTo(geometry.separator + 60, 0);
});

test("touch divider keeps its original grab point after the height clamp", async ({ page }) => {
  await page.goto("/?backend=canvas2d&forceFallbackAdapter=1");
  await wait_for_chart(page);
  await page.check("#rsi_toggle");
  await wait_for_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const overlay = chart.chart_element().querySelector("canvas:last-of-type");
    const rect = overlay.getBoundingClientRect();
    const initial = Array.from(chart.wasm.pane_separator_ys())[0];
    const x = rect.left + 400;
    const touch = (y) => new Touch({
      identifier: 37, target: overlay, clientX: x, clientY: rect.top + y,
      pageX: x, pageY: rect.top + y, screenX: x, screenY: rect.top + y,
      radiusX: 1, radiusY: 1, rotationAngle: 0, force: 0.5,
    });
    const send = (type, touches, changedTouches) => overlay.dispatchEvent(new TouchEvent(type, {
      touches, targetTouches: touches, changedTouches, bubbles: true, cancelable: true,
    }));
    const start = touch(initial + 10);
    send("touchstart", [start], [start]);
    const far = touch(initial + 400);
    send("touchmove", [far], [far]);
    const clamped = Array.from(chart.wasm.pane_separator_ys())[0];
    const back = touch(initial + 60);
    send("touchmove", [back], [back]);
    const reversed = Array.from(chart.wasm.pane_separator_ys())[0];
    send("touchend", [], [back]);
    return { initial, clamped, reversed };
  });
  expect(result.clamped).toBeGreaterThan(result.initial + 50);
  expect(result.reversed).toBeCloseTo(result.initial + 50, 0);
});

for (const backend of ["canvas2d", "webgpu"]) {
  test(`price-axis glyphs never paint across a pane separator (${backend})`, async ({ page }) => {
    await page.goto(`/?backend=${backend}&forceFallbackAdapter=1`);
    await wait_for_chart(page);
    await page.check("#rsi_toggle");
    await wait_for_chart(page);
    await page.mouse.move(1, 1);
    await page.evaluate(() => window.__chart.apply_options({ layout: { background: { type: "solid", color: "#131313" }, textColor: "#ffffff" } }));
    for (const offset of [0, 1, 2, 10, 25, 49]) {
      const geometry = await page.evaluate((offset) => {
        window.__main.price_scale().set_visible_range({ from: 80 + offset, to: 220 + offset });
        window.__rsi.price_scale().set_visible_range({ from: -50 + offset, to: 200 + offset });
        return { separator: Array.from(window.__chart.wasm.pane_separator_ys())[0], width: document.querySelector("#chart_container").getBoundingClientRect().width, plot: window.__chart.time_scale().width() };
      }, offset);
      const png = await capture(page);
      const dpr = png.width / geometry.width;
      let ink = 0;
      for (let y = Math.floor((geometry.separator - 1) * dpr); y <= Math.ceil((geometry.separator + 2) * dpr); y++) {
        for (let x = Math.ceil((geometry.plot + 4) * dpr); x < png.width - 2; x++) {
          const i = (y * png.width + x) * 4;
          if (Math.min(png.data[i], png.data[i + 1], png.data[i + 2]) > 180) ink++;
        }
      }
      expect(ink, `axis text crossed divider with range offset ${offset}`).toBe(0);
    }
  });
}
