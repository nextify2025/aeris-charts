import { test, expect } from "@playwright/test";
import { PNG } from "pngjs";

for (const backend of ["canvas2d", "webgpu"]) {
  test(`volume profile computes, renders and updates through ${backend}`, async ({ page }) => {
    await page.goto(`/?backend=${backend}`);
    await page.waitForFunction((expected) => window.__chart?.backend?.() === expected, backend);
    const initial = await page.evaluate(() => {
      const chart = window.__chart;
      window.__main.apply_options({ visible: false });
      const prices = chart.add_series("candlestick");
      const times = window.__data.slice(-3).map((bar) => bar.time);
      prices.set_data(times.map((time, index) => ({
        time, open: index === 2 ? 103 : 101, high: 104, low: 100, close: 102,
      })));
      const volume = chart.add_series("histogram", { visible: false });
      volume.set_data([{ time: times[0], value: 40 }, { time: times[2], value: 80 }]);
      chart.time_scale().set_visible_range({ from: times[0], to: times[2] });
      const profile = chart.add_volume_profile(prices, volume, {
        rows: 4,
        up_color: "#089981",
        down_color: "#f7525f",
        value_area_up_color: "#089981",
        value_area_down_color: "#f7525f",
        poc_color: "#f5a623",
      });
      window.__profile_test = { chart, prices, volume, profile, times };
      return profile.snapshot();
    });
    expect(initial.total_volume).toBe(120);
    expect(initial.bar_count).toBe(2);
    expect(initial.rows.map((row) => row.volume)).toEqual([30, 30, 30, 30]);
    expect(initial.rows.map((row) => row.up_volume)).toEqual([10, 10, 10, 10]);
    expect(initial.rows.map((row) => row.down_volume)).toEqual([20, 20, 20, 20]);
    expect(initial.poc).toBe(100.5);
    expect(initial.value_area_low).toBe(100);
    expect(initial.value_area_high).toBe(103);
    await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    const pixels = PNG.sync.read(await page.locator("#chart_wrap").screenshot());
    let green = 0;
    let red = 0;
    for (let offset = 0; offset < pixels.data.length; offset += 4) {
      if (Math.abs(pixels.data[offset] - 8) <= 2 && Math.abs(pixels.data[offset + 1] - 153) <= 2 && Math.abs(pixels.data[offset + 2] - 129) <= 2) green++;
      if (Math.abs(pixels.data[offset] - 247) <= 2 && Math.abs(pixels.data[offset + 1] - 82) <= 2 && Math.abs(pixels.data[offset + 2] - 95) <= 2) red++;
    }
    expect(green).toBeGreaterThan(100);
    expect(red).toBeGreaterThan(100);
    const updates = await page.evaluate(() => {
      const { chart, volume, profile, times } = window.__profile_test;
      const cached = profile.snapshot().calculation_revision;
      profile.apply_options({ width_percent: 30 });
      const styled = profile.snapshot().calculation_revision;
      volume.update({ time: times[2], value: 160 });
      const updated = profile.snapshot();
      chart.time_scale().set_visible_range({ from: times[0], to: times[1] });
      const ranged = profile.snapshot();
      let invalid;
      try { profile.apply_options({ rows: 513 }); } catch (error) { invalid = error.code; }
      const rows = profile.options().rows;
      chart.remove_series(volume);
      let stale;
      try { profile.snapshot(); } catch (error) { stale = error.code; }
      profile.remove();
      profile.remove();
      return { cached, styled, updated, ranged, invalid, rows, stale };
    });
    expect(updates.styled).toBe(updates.cached);
    expect(updates.updated.total_volume).toBe(200);
    expect(updates.ranged.total_volume).toBe(40);
    expect(updates.invalid).toBe("invalid_options");
    expect(updates.rows).toBe(4);
    expect(updates.stale).toBe("stale_handle");
  });
}

function count_anchor_border(png) {
  // Semantic primary #0091ff, the selection anchor border.
  let n = 0;
  for (let o = 0; o < png.data.length; o += 4) {
    if (Math.abs(png.data[o]) <= 30 && Math.abs(png.data[o + 1] - 145) <= 30 && Math.abs(png.data[o + 2] - 255) <= 30) n += 1;
  }
  return n;
}

async function chart_capture(page) {
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const data_url = await page.evaluate(() => window.__chart.take_screenshot().toDataURL("image/png"));
  return PNG.sync.read(Buffer.from(data_url.split(",")[1], "base64"));
}

test("volume profile is hovered, selected, deselected and deleted like other indicators", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() === "canvas2d");
  const spot = await page.evaluate(() => {
    const chart = window.__chart;
    window.__main.apply_options({ visible: false });
    // The profile follows its price series' visibility, so the prices stay visible.
    const prices = chart.add_series("candlestick");
    const times = window.__data.slice(-3).map((bar) => bar.time);
    prices.set_data(times.map((time) => ({ time, open: 101, high: 104, low: 100, close: 102 })));
    const volume = chart.add_series("histogram", { visible: false });
    volume.set_data([{ time: times[0], value: 40 }, { time: times[2], value: 80 }]);
    chart.time_scale().set_visible_range({ from: times[0], to: times[2] });
    const profile = chart.add_volume_profile(prices, volume, { rows: 4 });
    window.__profile_select = { profile };
    const poc = profile.snapshot().poc;
    const bounds = document.getElementById("chart_container").getBoundingClientRect();
    return {
      x: bounds.left + chart.wasm.pane_left() + chart.time_scale().width() - 60,
      y: bounds.top + prices.price_to_coordinate(poc),
    };
  });
  const cursor = () => page.evaluate(() => {
    const canvases = document.querySelectorAll("#chart_container canvas");
    return canvases[canvases.length - 1].style.cursor;
  });
  const selected = () => page.evaluate(() => window.__profile_select.profile.selected());

  const before = count_anchor_border(await chart_capture(page));
  await page.mouse.move(spot.x, spot.y);
  await expect.poll(cursor).toBe("pointer");
  await page.mouse.click(spot.x, spot.y);
  expect(await selected()).toBe(true);
  expect(count_anchor_border(await chart_capture(page)), "selection anchors paint").toBeGreaterThan(before + 20);

  await page.keyboard.press("Escape");
  expect(await selected()).toBe(false);
  expect(count_anchor_border(await chart_capture(page))).toBeLessThanOrEqual(before);

  // Outlast the engine's 500 ms double-click window so the next press is a fresh click.
  await page.waitForTimeout(550);
  await page.mouse.click(spot.x, spot.y);
  expect(await selected()).toBe(true);
  await page.keyboard.press("Delete");
  const stale = await page.evaluate(() => {
    try { window.__profile_select.profile.snapshot(); } catch (error) { return error.code; }
    return null;
  });
  expect(stale).toBe("stale_handle");
});

test("demo volume profile uses the built-in calculation", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() === "canvas2d");
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const toggle = page.locator("#volume_profile_toggle");
  await toggle.check();
  await expect(toggle).toBeChecked();
  await toggle.uncheck();
  await expect(toggle).not.toBeChecked();
  expect(errors).toEqual([]);
});
