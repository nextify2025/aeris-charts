import { expect } from "@playwright/test";
import { test, wait_for_chart } from "./page-ready.mjs";

async function install_reset_fixture(page) {
  return page.evaluate(async () => {
    const chart = window.__chart;
    const rows = window.__data.slice(0, 4);
    const points = (base) => rows.map((row, index) => ({ time: row.time, value: base + index * 10 }));
    const right = chart.add_price_scale({
      id: "reset-right",
      side: "right",
      order: 0,
      minimum_width: 72,
    });
    const left = chart.add_price_scale({
      id: "reset-left",
      side: "left",
      order: 0,
      minimum_width: 74,
    });
    const right_series = chart.add_series("line", { price_scale_id: "reset-right" });
    const left_series = chart.add_series("line", { price_scale_id: "reset-left" });
    right_series.set_data(points(1_000));
    left_series.set_data(points(10_000));
    chart.time_scale().fit_content();
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    right.set_visible_range({ from: 995, to: 1_015 });
    left.set_visible_range({ from: 9_995, to: 10_015 });
    await new Promise((resolve) => requestAnimationFrame(resolve));
    return {
      right_x: chart.wasm.pane_left() + chart.wasm.time_scale_width() + right.width() / 2,
      left_x: chart.wasm.pane_left() - left.width() / 2,
      y: chart.wasm.pane_height(0) / 2,
    };
  });
}

async function auto_scale_state(page) {
  return page.evaluate(() => ({
    right: window.__chart.price_scale("reset-right").options().auto_scale,
    left: window.__chart.price_scale("reset-left").options().auto_scale,
  }));
}

async function install_series_drag_fixture(page, manual = true) {
  return page.evaluate(async ({ manual }) => {
    const chart = window.__chart;
    const rows = window.__data.slice(0, 5);
    const right = chart.add_price_scale({
      id: "drag-right",
      side: "right",
      order: 0,
      minimum_width: 72,
      scale_margins: { top: 0.55, bottom: 0.05 },
    });
    const left = chart.add_price_scale({
      id: "drag-left",
      side: "left",
      order: 0,
      minimum_width: 72,
      scale_margins: { top: 0.05, bottom: 0.55 },
    });
    const lower = chart.add_series("candlestick", { price_scale_id: "drag-right" });
    const upper = chart.add_series("candlestick", { price_scale_id: "drag-left" });
    lower.set_data(rows.map((row, index) => ({
      time: row.time,
      open: 100 + index,
      high: 105 + index,
      low: 95 + index,
      close: 102 + index,
    })));
    upper.set_data(rows.map((row, index) => ({
      time: row.time,
      open: 1_200 + index * 5,
      high: 1_210 + index * 5,
      low: 1_190 + index * 5,
      close: 1_205 + index * 5,
    })));
    chart.time_scale().fit_content();
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    if (manual) {
      right.set_visible_range({ from: 80, to: 180 });
      left.set_visible_range({ from: 900, to: 1_260 });
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    }
    const row = rows.at(-1);
    const x = chart.time_scale().time_to_coordinate(row.time);
    return {
      x: chart.wasm.pane_left() + x,
      lower_y: lower.price_to_coordinate(106),
      upper_y: upper.price_to_coordinate(1_225),
      pane: 0,
    };
  }, { manual });
}

async function drag_scale_ranges(page) {
  return page.evaluate(() => ({
    right: window.__chart.price_scale("drag-right").get_visible_range(),
    left: window.__chart.price_scale("drag-left").get_visible_range(),
    right_auto: window.__chart.price_scale("drag-right").options().auto_scale,
    left_auto: window.__chart.price_scale("drag-left").options().auto_scale,
  }));
}

for (const backend of ["canvas2d", "webgpu"]) {
  test(`${backend}: named scales own placement, formatting, and comparison normalization`, async ({ page }) => {
    await page.goto(`/?backend=${backend}&forceFallbackAdapter=1`);
    await wait_for_chart(page);

    const result = await page.evaluate(async () => {
      const chart = window.__chart;
      const rows = window.__data.slice(0, 3);
      const points = (values) => rows.map((row, index) => ({ time: row.time, value: values[index] }));

      const inner = chart.add_price_scale({ id: "comparison-inner", side: "right", order: 0, minimum_width: 70 });
      const outer = chart.add_price_scale({ id: "comparison-outer", side: "right", minimum_width: 76 });
      const left = chart.add_price_scale({ id: "comparison-left", side: "left", order: 0, minimum_width: 72 });
      const percentage = chart.add_price_scale({ id: "shared-percentage", side: "left", order: 0, mode: 2 });

      const inner_series = chart.add_series("line", { price_scale_id: "comparison-inner" });
      const outer_series = chart.add_series("area", { price_scale_id: "comparison-outer" });
      const left_series = chart.add_series("histogram", { price_scale_id: "comparison-left" });
      const pct_a = chart.add_series("line", { price_scale_id: "shared-percentage" });
      const pct_b = chart.add_series("line", { price_scale_id: "shared-percentage" });
      inner_series.set_data(points([1_000, 1_010, 1_020]));
      outer_series.set_data(points([10_000, 10_100, 10_200]));
      left_series.set_data(points([10, 11, 12]));
      pct_a.set_data(points([100, 105, 110]));
      pct_b.set_data(points([200, 210, 220]));
      chart.time_scale().fit_content();
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));

      const scales = chart.price_scales();
      const info = Object.fromEntries(scales.map((scale) => [scale.id, scale]));
      const pane_width = chart.wasm.time_scale_width();
      const inner_target = chart.wasm.price_scale_target_by_id(0, "comparison-inner");
      const outer_target = chart.wasm.price_scale_target_by_id(0, "comparison-outer");
      const right_target = chart.wasm.price_scale_target_by_id(0, "right");
      const inner_width = inner.width();
      const right_width = chart.price_scale("right").width();
      const outer_width = outer.width();
      const hit_inner = chart.wasm.price_axis_target_at(0, pane_width + inner_width / 2);
      const hit_outer = chart.wasm.price_axis_target_at(
        0,
        pane_width + inner_width + right_width + outer_width / 2,
      );
      const pct_y_a = pct_a.price_to_coordinate(110);
      const pct_y_b = pct_b.price_to_coordinate(220);

      inner.apply_options({ visible: false });
      await new Promise((resolve) => requestAnimationFrame(resolve));

      return {
        backend: chart.backend(),
        ids: [inner_series.price_scale_id(), outer_series.price_scale_id(), left_series.price_scale_id()],
        right_order: [
          [info["comparison-inner"].id, info["comparison-inner"].order],
          [info.right.id, info.right.order],
          [info["comparison-outer"].id, info["comparison-outer"].order],
        ],
        left_side: info["comparison-left"].side,
        widths: [inner_width, outer_width, left.width()],
        targets: { inner_target, outer_target, right_target, hit_inner, hit_outer },
        percentage_delta: Math.abs(pct_y_a - pct_y_b),
        hidden_width: inner.width(),
        hidden_state: inner.options().visible,
      };
    });

    expect(result.backend).toBe(backend);
    expect(result.ids).toEqual(["comparison-inner", "comparison-outer", "comparison-left"]);
    expect(result.right_order).toEqual([
      ["comparison-inner", 0],
      ["right", 1],
      ["comparison-outer", 2],
    ]);
    expect(result.left_side).toBe("left");
    expect(result.widths.every((width) => width >= 70)).toBe(true);
    expect(result.targets.hit_inner).toBe(result.targets.inner_target);
    expect(result.targets.hit_outer).toBe(result.targets.outer_target);
    expect(result.targets.inner_target).not.toBe(result.targets.right_target);
    expect(result.percentage_delta).toBeLessThan(1e-7);
    expect(result.hidden_width).toBe(0);
    expect(result.hidden_state).toBe(false);
  });

  test(`${backend}: grabbing autoscaled comparison series preserves both locks`, async ({ page }) => {
    await page.goto(`/?backend=${backend}&forceFallbackAdapter=1`);
    await wait_for_chart(page);
    const fixture = await install_series_drag_fixture(page, false);
    const overlay = page.locator("#chart_container canvas:last-of-type");
    const box = await overlay.boundingBox();
    const before = await drag_scale_ranges(page);
    expect(before).toMatchObject({ right_auto: true, left_auto: true });

    await page.mouse.move(box.x + fixture.x, box.y + fixture.lower_y);
    await page.mouse.down();
    await page.mouse.move(box.x + fixture.x, box.y + fixture.lower_y + 32, { steps: 4 });
    await page.mouse.up();
    await wait_for_chart(page);

    const after = await drag_scale_ranges(page);
    expect(after).toEqual(before);
  });

  test(`${backend}: grabbing either comparison series pans only its own unlocked scale`, async ({ page }) => {
    await page.goto(`/?backend=${backend}&forceFallbackAdapter=1`);
    await wait_for_chart(page);
    const fixture = await install_series_drag_fixture(page);
    const overlay = page.locator("#chart_container canvas:last-of-type");
    const box = await overlay.boundingBox();
    const before = await drag_scale_ranges(page);
    expect(before).toMatchObject({ right_auto: false, left_auto: false });

    await page.mouse.move(box.x + fixture.x, box.y + fixture.lower_y);
    await page.mouse.down();
    await page.mouse.move(box.x + fixture.x, box.y + fixture.lower_y + 32, { steps: 4 });
    await page.mouse.up();
    await wait_for_chart(page);
    const after_lower = await drag_scale_ranges(page);
    expect(after_lower.right).not.toEqual(before.right);
    expect(after_lower.left).toEqual(before.left);
    expect(after_lower).toMatchObject({ right_auto: false, left_auto: false });

    await page.mouse.move(box.x + fixture.x, box.y + fixture.upper_y);
    await page.mouse.down();
    await page.mouse.move(box.x + fixture.x, box.y + fixture.upper_y - 28, { steps: 4 });
    await page.mouse.up();
    await wait_for_chart(page);
    const after_upper = await drag_scale_ranges(page);
    expect(after_upper.right).toEqual(after_lower.right);
    expect(after_upper.left).not.toEqual(after_lower.left);
    expect(after_upper).toMatchObject({ right_auto: false, left_auto: false });
  });

  test(`${backend}: one comparison axis double-click resets only its exact price scale`, async ({ page }) => {
    await page.goto(`/?backend=${backend}&forceFallbackAdapter=1`);
    await wait_for_chart(page);
    const fixture = await install_reset_fixture(page);
    const overlay = page.locator("#chart_container canvas:last-of-type");
    const box = await overlay.boundingBox();

    expect(await auto_scale_state(page)).toEqual({ right: false, left: false });
    await page.mouse.dblclick(box.x + fixture.right_x, box.y + fixture.y);
    await wait_for_chart(page);
    expect(await auto_scale_state(page)).toEqual({ right: true, left: false });
  });
}

test("named comparison reset supports touch, configuration gates, and explicit global reset", async ({ page }) => {
  await page.goto("/?backend=canvas2d&forceFallbackAdapter=1");
  await wait_for_chart(page);
  const fixture = await install_reset_fixture(page);

  await page.evaluate(({ right_x, y }) => {
    const chart = window.__chart;
    const overlay = chart.chart_element().querySelector("canvas:last-of-type");
    const rect = overlay.getBoundingClientRect();
    const clientX = rect.left + right_x;
    const clientY = rect.top + y;
    const touch = () => new Touch({
      identifier: 91, target: overlay, clientX, clientY, pageX: clientX, pageY: clientY,
      screenX: clientX, screenY: clientY, radiusX: 1, radiusY: 1, rotationAngle: 0, force: 0.5,
    });
    const send = (type, touches, changedTouches) => overlay.dispatchEvent(new TouchEvent(type, {
      touches, targetTouches: touches, changedTouches, bubbles: true, cancelable: true,
    }));
    const first = touch();
    send("touchstart", [first], [first]);
    send("touchend", [], [first]);
    const second = touch();
    send("touchstart", [second], [second]);
    send("touchend", [], [second]);
  }, fixture);
  await wait_for_chart(page);
  expect(await auto_scale_state(page)).toEqual({ right: true, left: false });

  await page.evaluate(() => {
    const chart = window.__chart;
    chart.price_scale("reset-right").set_visible_range({ from: 995, to: 1_015 });
    chart.price_scale("reset-left").set_visible_range({ from: 9_995, to: 10_015 });
    chart.apply_options({ handle_scale: { axis_double_click_reset: { price: false } } });
  });
  const overlay = page.locator("#chart_container canvas:last-of-type");
  const box = await overlay.boundingBox();
  await page.mouse.dblclick(box.x + fixture.right_x, box.y + fixture.y);
  await wait_for_chart(page);
  expect(await auto_scale_state(page)).toEqual({ right: false, left: false });

  await page.evaluate(() => window.__chart.reset_view());
  await wait_for_chart(page);
  expect(await auto_scale_state(page)).toEqual({ right: true, left: true });
});

test("touch dragging a comparison candle pans only that candle's unlocked scale", async ({ page }) => {
  await page.goto("/?backend=canvas2d&forceFallbackAdapter=1");
  await wait_for_chart(page);
  const fixture = await install_series_drag_fixture(page);
  const before = await drag_scale_ranges(page);
  expect(before).toMatchObject({ right_auto: false, left_auto: false });

  await page.evaluate(({ x, lower_y }) => {
    const overlay = window.__chart.chart_element().querySelector("canvas:last-of-type");
    const rect = overlay.getBoundingClientRect();
    const touch = (y) => new Touch({
      identifier: 117, target: overlay, clientX: rect.left + x, clientY: rect.top + y,
      pageX: rect.left + x, pageY: rect.top + y, screenX: rect.left + x, screenY: rect.top + y,
      radiusX: 1, radiusY: 1, rotationAngle: 0, force: 0.5,
    });
    const send = (type, touches, changedTouches) => overlay.dispatchEvent(new TouchEvent(type, {
      touches, targetTouches: touches, changedTouches, bubbles: true, cancelable: true,
    }));
    const down = touch(lower_y);
    send("touchstart", [down], [down]);
    const crossing = touch(lower_y + 18);
    send("touchmove", [crossing], [crossing]);
    const moved = touch(lower_y + 36);
    send("touchmove", [moved], [moved]);
    send("touchend", [], [moved]);
  }, fixture);
  await wait_for_chart(page);

  const after = await drag_scale_ranges(page);
  expect(after.right).not.toEqual(before.right);
  expect(after.left).toEqual(before.left);
  expect(after).toMatchObject({ right_auto: false, left_auto: false });
});

test("named scale descriptors support atomic pane moves and host-owned reconstruction", async ({ page }) => {
  await page.goto("/?backend=canvas2d&forceFallbackAdapter=1");
  await wait_for_chart(page);

  const result = await page.evaluate(async () => {
    const chart = window.__chart;
    const rows = window.__data.slice(0, 3);
    const series = chart.add_series("line", { price_scale_id: "right" });
    series.set_data(rows.map((row, index) => ({ time: row.time, value: 500 + index * 10 })));
    const handle = chart.add_price_scale({
      id: "host-owned",
      side: "right",
      order: 0,
      minimum_width: 88,
      invert_scale: true,
    });
    series.move_to_price_scale("host-owned");
    const descriptor = chart.price_scales().find((scale) => scale.id === "host-owned");
    const options = handle.options();

    const pane = chart.add_pane(true);
    let failed_move_code = null;
    try {
      series.move_to_pane(pane.pane_index());
    } catch (error) {
      failed_move_code = error.code;
    }
    const after_failed_move = { pane: series.pane_index(), scale: series.price_scale_id() };

    chart.add_price_scale({ id: descriptor.id, side: descriptor.side, order: descriptor.order, ...options }, pane.pane_index());
    series.move_to_pane(pane.pane_index());
    const after_successful_move = { pane: series.pane_index(), scale: series.price_scale_id() };

    series.move_to_pane(0);
    series.move_to_price_scale("right");
    chart.remove_price_scale("host-owned", 0);
    let stale_code = null;
    try {
      handle.options();
    } catch (error) {
      stale_code = error.code;
    }
    const restored = chart.add_price_scale({
      id: descriptor.id,
      side: descriptor.side,
      order: descriptor.order,
      ...options,
    });
    series.move_to_price_scale("host-owned");
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));

    return {
      descriptor,
      failed_move_code,
      after_failed_move,
      after_successful_move,
      stale_code,
      restored_options: restored.options(),
      final_binding: { pane: series.pane_index(), scale: series.price_scale_id() },
    };
  });

  expect(result.descriptor).toMatchObject({
    id: "host-owned",
    side: "right",
    order: 0,
    visible: true,
    built_in: false,
    pane_index: 0,
  });
  expect(result.failed_move_code).toBe("invalid_options");
  expect(result.after_failed_move).toEqual({ pane: 0, scale: "host-owned" });
  expect(result.after_successful_move).toEqual({ pane: 1, scale: "host-owned" });
  expect(result.stale_code).toBe("stale_handle");
  expect(result.restored_options).toMatchObject({ minimum_width: 88, invert_scale: true, visible: true });
  expect(result.final_binding).toEqual({ pane: 0, scale: "host-owned" });
});
