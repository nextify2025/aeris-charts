import { test, expect } from "@playwright/test";

async function open_chart(page) {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() === "canvas2d");
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
}

test("a captured pane pan uses the shared grabbing cursor", async ({ page }) => {
  await open_chart(page);
  const overlay = page.locator("#chart_container canvas:last-of-type");
  const box = await overlay.boundingBox();
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + 8, y);
  await page.mouse.move(x + 24, y);
  expect(await overlay.evaluate((element) => element.style.cursor)).toBe("grabbing");
  await page.mouse.up();
});

test("browser second press at the same pane point can become a pan", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const overlay = chart.chart_element().querySelector("canvas:last-of-type");
    const rect = overlay.getBoundingClientRect();
    const x = rect.left + chart.wasm.pane_left() + chart.wasm.time_scale_width() / 2;
    const y = rect.top + chart.wasm.pane_height(0) / 2;
    const send = (type, clientX, buttons) => overlay.dispatchEvent(new PointerEvent(type, {
      pointerId: 81, pointerType: "mouse", button: 0, buttons, clientX, clientY: y,
      bubbles: true, cancelable: true,
    }));
    send("pointerdown", x, 1);
    send("pointerup", x, 0);
    const before = chart.wasm.scroll_position();
    send("pointerdown", x, 1);
    send("pointermove", x + 25, 1);
    send("pointermove", x + 60, 1);
    const moved = chart.wasm.scroll_position();
    send("pointerup", x + 60, 0);
    let doubleClicks = 0;
    chart.subscribe_dbl_click(() => { doubleClicks += 1; });
    send("pointerdown", x + 120, 1);
    send("pointerup", x + 120, 0);
    send("pointerdown", x + 120, 1);
    send("pointerup", x + 120, 0);
    return { before, moved, doubleClicks };
  });
  expect(result.moved).not.toBeCloseTo(result.before, 8);
  expect(result.doubleClicks).toBe(1);
});

test("an armed drawing tool keeps the engine crosshair cursor over series", async ({ page }) => {
  await open_chart(page);
  const overlay = page.locator("#chart_container canvas:last-of-type");
  const box = await overlay.boundingBox();
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.evaluate(() => window.__chart.set_drawing_tool("trend_line"));
  await page.mouse.move(x + 1, y);
  expect(await overlay.evaluate((element) => element.style.cursor)).toBe("crosshair");
});

test("auto wheel pins the right edge, Ctrl zooms at the cursor, and horizontal deltas pan", async ({ page }) => {
  await open_chart(page);
  const box = await page.locator("#chart_container canvas:last-of-type").boundingBox();
  const geometry = await page.evaluate(() => ({
    paneLeft: window.__chart.wasm.pane_left(),
    paneWidth: window.__chart.wasm.time_scale_width(),
  }));
  const paneX = geometry.paneWidth / 2;
  const state = () => page.evaluate((x) => ({
    spacing: window.__chart.wasm.bar_spacing(),
    offset: window.__chart.wasm.scroll_position(),
    logical: window.__chart.time_scale().coordinate_to_logical(x),
    rightBarStays: window.__chart.time_scale().options().right_bar_stays_on_scroll,
  }), paneX);
  await page.mouse.move(box.x + geometry.paneLeft + paneX, box.y + box.height / 2);

  // Measured TradingView behavior: a plain wheel keeps the right offset (the latest bars stay
  // put) while the bar under the cursor moves.
  const beforeZoom = await state();
  expect(beforeZoom.rightBarStays).toBe(true);
  await page.mouse.wheel(0, -24);
  const afterZoom = await state();
  expect(afterZoom.spacing).toBeGreaterThan(beforeZoom.spacing);
  expect(afterZoom.offset).toBeCloseTo(beforeZoom.offset, 8);
  expect(afterZoom.logical).not.toBeCloseTo(beforeZoom.logical, 5);

  const beforeFocused = await state();
  await page.keyboard.down("Control");
  await page.mouse.wheel(0, -24);
  await page.keyboard.up("Control");
  const afterFocused = await state();
  expect(afterFocused.spacing).toBeGreaterThan(beforeFocused.spacing);
  expect(afterFocused.logical).toBeCloseTo(beforeFocused.logical, 5);
  expect(afterFocused.offset).not.toBeCloseTo(beforeFocused.offset, 8);

  // Shift is not a zoom-anchor modifier: it zooms like a plain wheel.
  const beforeShiftPan = await state();
  await page.keyboard.down("Shift");
  await page.mouse.wheel(0, -24);
  await page.keyboard.up("Shift");
  const afterShiftPan = await state();
  expect(afterShiftPan.spacing).toBeGreaterThan(beforeShiftPan.spacing);
  expect(afterShiftPan.offset).toBeCloseTo(beforeShiftPan.offset, 8);

  const beforePan = await state();
  await page.mouse.wheel(24, 0);
  const afterPan = await state();
  expect(afterPan.spacing).toBeCloseTo(beforePan.spacing, 8);
  expect(afterPan.offset).not.toBeCloseTo(beforePan.offset, 8);
});

test("auto wheel can zoom the price axis when that option is enabled", async ({ page }) => {
  await open_chart(page);
  const before = await page.evaluate(() => {
    const chart = window.__chart;
    chart.price_scale("right").set_visible_range({ from: 80, to: 120 });
    chart.apply_options({ price_axis_wheel_zoom: true });
    return {
      range: chart.price_scale("right").get_visible_range(),
      spacing: chart.wasm.bar_spacing(),
      x: chart.wasm.pane_left() + chart.wasm.time_scale_width() + 12,
      y: chart.wasm.pane_height(0) / 2,
    };
  });
  await page.evaluate(({ x, y }) => {
    const overlay = document.querySelector("#chart_container canvas:last-of-type");
    const rect = overlay.getBoundingClientRect();
    overlay.dispatchEvent(new WheelEvent("wheel", {
      deltaY: -100, clientX: rect.left + x, clientY: rect.top + y,
      bubbles: true, cancelable: true,
    }));
  }, before);
  const after = await page.evaluate(() => ({
    range: window.__chart.price_scale("right").get_visible_range(),
    spacing: window.__chart.wasm.bar_spacing(),
  }));
  expect(after.range).not.toEqual(before.range);
  expect(after.spacing).toBeCloseTo(before.spacing, 8);
});

test("wheel refreshes the engine hover and cursor at its event position", async ({ page }) => {
  await open_chart(page);
  const overlay = page.locator("#chart_container canvas:last-of-type");
  const geometry = await page.evaluate(() => ({
    left: window.__chart.wasm.pane_left(),
    width: window.__chart.wasm.time_scale_width(),
    height: window.__chart.wasm.pane_height(0),
  }));
  const box = await overlay.boundingBox();
  await page.mouse.move(box.x + geometry.left + geometry.width / 2, box.y + geometry.height / 2);
  await page.evaluate(({ x, y }) => {
    const canvas = document.querySelector("#chart_container canvas:last-of-type");
    const rect = canvas.getBoundingClientRect();
    canvas.dispatchEvent(new WheelEvent("wheel", {
      deltaY: -24, clientX: rect.left + x, clientY: rect.top + y,
      bubbles: true, cancelable: true,
    }));
  }, { x: geometry.left + geometry.width + 12, y: geometry.height / 2 });
  expect(await overlay.evaluate((element) => element.style.cursor)).toBe("ns-resize");
});

test("price-axis wheel zoom applies only inside the plot height", async ({ page }) => {
  await open_chart(page);
  const before = await page.evaluate(() => {
    const chart = window.__chart;
    chart.price_scale("right").set_visible_range({ from: 80, to: 120 });
    chart.apply_options({ price_axis_wheel_zoom: true });
    return {
      range: chart.price_scale("right").get_visible_range(),
      spacing: chart.wasm.bar_spacing(),
      x: chart.wasm.pane_left() + chart.wasm.time_scale_width() + 12,
      y: chart.wasm.pane_height(0) + chart.wasm.time_scale_height() / 2,
    };
  });
  await page.evaluate(({ x, y }) => {
    const canvas = document.querySelector("#chart_container canvas:last-of-type");
    const rect = canvas.getBoundingClientRect();
    canvas.dispatchEvent(new WheelEvent("wheel", {
      deltaY: -100, clientX: rect.left + x, clientY: rect.top + y,
      bubbles: true, cancelable: true,
    }));
  }, before);
  const after = await page.evaluate(() => ({
    range: window.__chart.price_scale("right").get_visible_range(),
    spacing: window.__chart.wasm.bar_spacing(),
  }));
  expect(after.range).toEqual(before.range);
  expect(after.spacing).not.toBeCloseTo(before.spacing, 8);
});

test("wheel input cancels a held keyboard pan", async ({ page }) => {
  await open_chart(page);
  await page.evaluate(() => {
    const overlay = document.querySelector("#chart_container canvas:last-of-type");
    overlay.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true, cancelable: true }));
  });
  await page.waitForTimeout(100);
  await page.evaluate(() => {
    const overlay = document.querySelector("#chart_container canvas:last-of-type");
    overlay.dispatchEvent(new WheelEvent("wheel", {
      deltaX: 0,
      deltaY: -24,
      clientX: overlay.getBoundingClientRect().left + 200,
      clientY: overlay.getBoundingClientRect().top + 100,
      bubbles: true,
      cancelable: true,
    }));
  });
  const afterWheel = await page.evaluate(() => window.__chart.wasm.scroll_position());
  await page.waitForTimeout(150);
  const afterWait = await page.evaluate(() => window.__chart.wasm.scroll_position());
  expect(afterWait).toBeCloseTo(afterWheel, 8);
  await page.evaluate(() => window.dispatchEvent(new KeyboardEvent("keyup", { key: "ArrowRight" })));
});

test("End returns to the latest bar through the shared key controller", async ({ page }) => {
  await open_chart(page);
  const overlay = page.locator("#chart_container canvas:last-of-type");
  await overlay.focus();
  await page.evaluate(() => window.__chart.wasm.scroll_to_position(-30));
  const before = await page.evaluate(() => window.__chart.wasm.scroll_position());
  expect(before).toBeLessThan(-20);
  await overlay.press("End");
  const after = await page.evaluate(() => window.__chart.wasm.scroll_position());
  expect(after).toBeGreaterThan(before + 20);
});

test("Page keys, zoom keys, and Home follow the engine navigation bindings", async ({ page }) => {
  await open_chart(page);
  const overlay = page.locator("#chart_container canvas:last-of-type");
  await overlay.focus();
  await page.evaluate(() => window.__chart.wasm.scroll_to_position(-30));
  const start = await page.evaluate(() => window.__chart.wasm.scroll_position());
  await overlay.press("PageUp");
  const older = await page.evaluate(() => window.__chart.wasm.scroll_position());
  expect(older).toBeLessThan(start);
  await overlay.press("PageDown");
  const newer = await page.evaluate(() => window.__chart.wasm.scroll_position());
  expect(newer).toBeGreaterThan(older);

  const spacing = await page.evaluate(() => window.__chart.wasm.bar_spacing());
  await overlay.press("+");
  const zoomed = await page.evaluate(() => window.__chart.wasm.bar_spacing());
  expect(zoomed).toBeGreaterThan(spacing);
  await overlay.press("-");
  expect(await page.evaluate(() => window.__chart.wasm.bar_spacing())).toBeLessThan(zoomed);

  await page.evaluate(() => {
    window.__chart.wasm.set_bar_spacing(20);
    window.__chart.price_scale("right").set_visible_range({ from: 80, to: 120 });
  });
  await overlay.press("Home");
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  expect(await page.evaluate(() => window.__chart.wasm.bar_spacing())).toBe(6);
  expect(await page.evaluate(() => window.__chart.price_scale("right").get_visible_range())).not.toEqual({ from: 80, to: 120 });
});

test("Delete removes a selected host series through the controller event queue", async ({ page }) => {
  await open_chart(page);
  const overlay = page.locator("#chart_container canvas:last-of-type");
  await overlay.focus();
  const id = await page.evaluate(() => {
    const id = window.__main.id;
    window.__chart.wasm.set_selected_series(id);
    return id;
  });
  expect(await page.evaluate((id) => window.__chart.series_by_id.has(id), id)).toBe(true);
  await overlay.press("Delete");
  expect(await page.evaluate((id) => window.__chart.series_by_id.has(id), id)).toBe(false);
});

test("Escape reports that the crosshair left the chart", async ({ page }) => {
  await open_chart(page);
  const overlay = page.locator("#chart_container canvas:last-of-type");
  await overlay.focus();
  await page.evaluate(() => {
    window.__escapeCrosshairPoints = [];
    window.__chart.subscribe_crosshair_move((event) => window.__escapeCrosshairPoints.push(event.point));
  });
  const box = await overlay.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await overlay.press("Escape");
  const points = await page.evaluate(() => window.__escapeCrosshairPoints);
  expect(points.length).toBeGreaterThan(1);
  expect(points.at(-1)).toBeNull();
});

test("pointer interaction does not move focus into the accessibility application", async ({ page }) => {
  await open_chart(page);
  const overlay = page.locator("#chart_container canvas:last-of-type");
  const box = await overlay.boundingBox();
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
  const state = await page.evaluate(() => {
    const layer = window.__chart.chart_element().querySelector(".aeris_charts-a11y-layer");
    return {
      accessibilityFocused: layer.contains(document.activeElement),
      outline: getComputedStyle(layer).outlineStyle,
    };
  });
  expect(state.accessibilityFocused).toBe(false);
  expect(state.outline).toBe("none");
});

test("Touch Events pinch around the starting centroid and preserve primary-touch continuation", async ({ page, browserName }) => {
  test.skip(browserName !== "chromium", "Firefox lacks Touch constructors and WebKit forbids synthetic construction");
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const overlay = chart.chart_element().querySelector("canvas:last-of-type");
    const rect = overlay.getBoundingClientRect();
    const paneLeft = chart.wasm.pane_left();
    const x = rect.left + paneLeft + chart.wasm.time_scale_width() / 2;
    const y = rect.top + chart.wasm.pane_height(0) / 2;
    const touch = (identifier, clientX, clientY) => new Touch({
      identifier, target: overlay, clientX, clientY, pageX: clientX, pageY: clientY,
      screenX: clientX, screenY: clientY, radiusX: 1, radiusY: 1, rotationAngle: 0, force: 0.5,
    });
    const send = (type, touches, changedTouches) => {
      overlay.dispatchEvent(new TouchEvent(type, {
        touches, targetTouches: touches, changedTouches, bubbles: true, cancelable: true,
      }));
    };
    const first = touch(11, x - 50, y);
    const second = touch(12, x + 50, y);
    const before = {
      spacing: chart.wasm.bar_spacing(),
      offset: chart.wasm.scroll_position(),
      logical: chart.time_scale().coordinate_to_logical(chart.wasm.time_scale_width() / 2),
    };
    send("touchstart", [first], [first]);
    send("touchstart", [first, second], [second]);
    const spread = touch(12, x + 100, y + 20);
    send("touchmove", [first, spread], [spread]);
    const pinched = {
      spacing: chart.wasm.bar_spacing(),
      offset: chart.wasm.scroll_position(),
      logical: chart.time_scale().coordinate_to_logical(chart.wasm.time_scale_width() / 2),
    };
    send("touchend", [first], [spread]);
    const rebased = chart.wasm.scroll_position();
    const crossing = touch(11, x - 10, y);
    send("touchmove", [crossing], [crossing]);
    const atCrossing = chart.wasm.scroll_position();
    const continuedTouch = touch(11, x + 10, y);
    send("touchmove", [continuedTouch], [continuedTouch]);
    const continued = chart.wasm.scroll_position();
    send("touchend", [], [continuedTouch]);
    return { before, pinched, rebased, atCrossing, continued, touchAction: overlay.style.touchAction };
  });
  expect(result.touchAction).toBe("auto");
  expect(result.pinched.spacing).not.toBeCloseTo(result.before.spacing, 6);
  expect(result.pinched.logical).toBeCloseTo(result.before.logical, 5);
  expect(result.atCrossing).toBeCloseTo(result.rebased, 8);
  expect(result.continued).not.toBeCloseTo(result.rebased, 6);
  expect(Math.abs(result.continued - result.rebased)).toBeCloseTo(20 / result.pinched.spacing, 5);
});

test("touch cancellation ends the canonical gesture and ignores later samples", async ({ page, browserName }) => {
  test.skip(browserName !== "chromium", "Firefox lacks Touch constructors and WebKit forbids synthetic construction");
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const overlay = chart.chart_element().querySelector("canvas:last-of-type");
    const rect = overlay.getBoundingClientRect();
    const x = rect.left + chart.wasm.pane_left() + chart.wasm.time_scale_width() / 2;
    const y = rect.top + chart.wasm.pane_height(0) / 2;
    const touch = (clientX) => new Touch({
      identifier: 31, target: overlay, clientX, clientY: y, pageX: clientX, pageY: y,
      screenX: clientX, screenY: y, radiusX: 1, radiusY: 1, rotationAngle: 0, force: 0.5,
    });
    const send = (type, touches, changedTouches) => overlay.dispatchEvent(new TouchEvent(type, {
      touches, targetTouches: touches, changedTouches, bubbles: true, cancelable: true,
    }));
    const down = touch(x);
    send("touchstart", [down], [down]);
    const crossing = touch(x - 40);
    send("touchmove", [crossing], [crossing]);
    const moved = touch(x - 80);
    send("touchmove", [moved], [moved]);
    const atLoss = chart.wasm.scroll_position();
    send("touchcancel", [], [moved]);
    const afterCancel = touch(x - 140);
    send("touchmove", [afterCancel], [afterCancel]);
    const after = chart.wasm.scroll_position();
    return { atLoss, after };
  });
  expect(result.after).toBeCloseTo(result.atLoss, 8);
});

test("Touch Events use the controller long-press deadline and next-tap exit", async ({ page, browserName }) => {
  test.skip(browserName !== "chromium", "Firefox lacks Touch constructors and WebKit forbids synthetic construction");
  await open_chart(page);
  const result = await page.evaluate(async () => {
    const chart = window.__chart;
    const overlay = chart.chart_element().querySelector("canvas:last-of-type");
    const rect = overlay.getBoundingClientRect();
    const x = rect.left + chart.wasm.pane_left() + chart.wasm.time_scale_width() / 2;
    const y = rect.top + chart.wasm.pane_height(0) / 2;
    const touch = (identifier, clientX) => new Touch({
      identifier, target: overlay, clientX, clientY: y, pageX: clientX, pageY: y,
      screenX: clientX, screenY: y, radiusX: 1, radiusY: 1, rotationAngle: 0, force: 0.5,
    });
    const send = (type, touches, changedTouches) => overlay.dispatchEvent(new TouchEvent(type, {
      touches, targetTouches: touches, changedTouches, bubbles: true, cancelable: true,
    }));
    const crosshair = () => {
      const point = new Float64Array(2);
      chart.wasm.controller_crosshair_into(point);
      return Array.from(point);
    };
    const start = touch(41, x);
    send("touchstart", [start], [start]);
    await new Promise((resolve) => setTimeout(resolve, 300));
    const scroll = chart.wasm.scroll_position();
    const moved = touch(41, x + 30);
    send("touchmove", [moved], [moved]);
    const tracked = crosshair();
    const afterMove = chart.wasm.scroll_position();
    send("touchend", [], [moved]);
    const afterEnd = crosshair();
    const next = touch(42, x + 50);
    send("touchstart", [next], [next]);
    send("touchend", [], [next]);
    return { tracked, afterEnd, afterTap: crosshair(), scroll, afterMove,
      localX: x + 30 - rect.left - chart.wasm.pane_left(), localY: y - rect.top };
  });
  expect(result.tracked[0]).toBeCloseTo(result.localX, 5);
  expect(result.tracked[1]).toBeCloseTo(result.localY, 5);
  expect(result.afterMove).toBeCloseTo(result.scroll, 8);
  expect(result.afterEnd[0]).toBeCloseTo(result.localX, 5);
  expect(Number.isNaN(result.afterTap[0])).toBe(true);
});

test("vertical page-scroll arbitration releases the controller touch press", async ({ page, browserName }) => {
  test.skip(browserName !== "chromium", "Firefox lacks Touch constructors and WebKit forbids synthetic construction");
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    chart.apply_options({ handle_scroll: { vert_touch_drag: false, horz_touch_drag: true } });
    const overlay = chart.chart_element().querySelector("canvas:last-of-type");
    const rect = overlay.getBoundingClientRect();
    const x = rect.left + chart.wasm.pane_left() + chart.wasm.time_scale_width() / 2;
    const y = rect.top + chart.wasm.pane_height(0) / 2;
    const touch = (clientY) => new Touch({
      identifier: 51, target: overlay, clientX: x, clientY, pageX: x, pageY: clientY,
      screenX: x, screenY: clientY, radiusX: 1, radiusY: 1, rotationAngle: 0, force: 0.5,
    });
    const send = (type, touches, changedTouches) => {
      const event = new TouchEvent(type, {
        touches, targetTouches: touches, changedTouches, bubbles: true, cancelable: true,
      });
      overlay.dispatchEvent(event);
      return event.defaultPrevented;
    };
    const start = touch(y);
    send("touchstart", [start], [start]);
    const candidateBefore = chart.wasm.controller_touch_page_scroll_candidate(51);
    const moved = touch(y + 35);
    const prevented = send("touchmove", [moved], [moved]);
    const candidateAfter = chart.wasm.controller_touch_page_scroll_candidate(51);
    send("touchend", [], [moved]);
    return { candidateBefore, candidateAfter, prevented };
  });
  expect(result.candidateBefore).toBe(true);
  expect(result.candidateAfter).toBe(false);
  expect(result.prevented).toBe(false);
});

test("accessibility is default, singleton, bounded, silent for streaming, and keyboard drawing edits roll back", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    const chart = window.__chart;
    const host = chart.chart_element();
    const first = chart.accessibility();
    const compatible = api.enable_accessibility(chart, { chart_title: "Unified chart" });
    const drawing = chart.add_drawing("trend_line", [
      { logical: 20, price: 100 },
      { logical: 30, price: 105 },
    ]);
    chart.wasm.set_selected_drawing(drawing.id);
    first.focus_target(`drawing:${drawing.id}`);
    const layer = document.activeElement;
    const before = drawing.points();
    for (const key of ["Enter", "ArrowRight", "Escape"]) {
      layer.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
    }
    const after = drawing.points();
    const last = window.__main.data().at(-1);
    window.__main.update({ ...last, close: last.close + 0.25 });
    await new Promise((resolve) => setTimeout(resolve, 220));
    await new Promise((resolve) => requestAnimationFrame(resolve));
    const state = {
      singleton: first === compatible,
      hostRole: host.getAttribute("role"),
      applications: host.querySelectorAll('[role="application"]').length,
      canvasHidden: [...host.querySelectorAll("canvas")].every((canvas) => canvas.getAttribute("aria-hidden") === "true"),
      live: host.querySelector(".aeris_charts-a11y-shared-status-region")?.textContent ?? "",
      before,
      after,
    };
    chart.apply_options({ accessibility: false });
    state.disabledApplications = host.querySelectorAll('[role="application"]').length;
    chart.apply_options({ accessibility: true });
    state.reenabledApplications = host.querySelectorAll('[role="application"]').length;
    return state;
  });
  expect(result).toMatchObject({
    singleton: true,
    hostRole: "group",
    applications: 1,
    canvasHidden: true,
    live: "",
    disabledApplications: 0,
    reenabledApplications: 1,
  });
  expect(result.after).toEqual(result.before);
});

test("a keyboard nudge that moves nothing is not undone by Escape", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const a11y = chart.accessibility();
    const recolored = chart.add_drawing("trend_line", [
      { logical: 20, price: 100 },
      { logical: 30, price: 105 },
    ]);
    recolored.apply_options({ color: "#00ff00" });
    const line = chart.add_drawing("vertical_line", [{ logical: 25, price: 100 }]);
    chart.wasm.set_selected_drawing(line.id);
    a11y.focus_target(`drawing:${line.id}`);
    const layer = document.activeElement;
    const key = (name) => layer.dispatchEvent(new KeyboardEvent("keydown", { key: name, bubbles: true, cancelable: true }));
    key("Enter");
    // A vertical line only moves in time: ArrowUp is consumed but changes nothing.
    const consumed = !key("ArrowUp");
    key("Escape");
    return {
      consumed,
      kinds: chart.drawings().map((drawing) => drawing.kind()),
      color: recolored.options().color,
    };
  });
  expect(result.consumed).toBe(true);
  expect(result.kinds).toEqual(["trend_line", "vertical_line"]);
  expect(result.color).toBe("#00ff00");
});

test("semantic axis and trading targets route keyboard actions through canonical engine paths", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const a11y = chart.accessibility();
    const trading = chart.trading();
    const intents = [];
    trading.subscribe_intents((intent) => intents.push(intent));
    trading.apply_snapshot({
      instrument: { tick_size: 0.25, price_precision: 2 },
      orders: [{
        id: "keyboard-order", pane_index: 0, price_scale: "right", side: "buy",
        kind: "limit", role: "working", status: "working", price: 100,
        quantity: 2, filled_quantity: 0, revision: 3,
      }],
    });
    a11y.refresh();
    a11y.focus_target("order:keyboard-order");
    for (const key of ["Enter", "ArrowUp", "Enter"]) {
      document.activeElement.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
    }
    const price = chart.price_scale("right", 0);
    price.set_visible_range({ from: 90, to: 110 });
    a11y.focus_target("price-axis");
    document.activeElement.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true, cancelable: true }));
    const adjusted = price.get_visible_range();
    document.activeElement.dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true, cancelable: true }));
    return {
      intent: intents[0],
      adjusted,
      autoScale: price.options().auto_scale,
      focused: document.activeElement.getAttribute("aria-label"),
    };
  });
  expect(result.intent).toMatchObject({
    action: "create_take_profit",
    order_id: "keyboard-order",
    side: "sell",
    kind: "limit",
    role: "take_profit",
    price: 100.25,
  });
  expect(result.adjusted.to - result.adjusted.from).toBeCloseTo(19, 8);
  expect(result.autoScale).toBe(true);
  expect(result.focused).toContain("price axis");
});

test("DPR-only transitions resize auto-sized bitmap surfaces", async ({ page, browserName }) => {
  test.skip(browserName !== "chromium", "devicePixelRatio override is only deterministic in Chromium");
  await open_chart(page);
  await page.evaluate(() => window.__chart.apply_options({ autoSize: true }));
  const before = await page.evaluate(() => {
    const canvas = window.__chart.chart_element().querySelector("canvas");
    return { bitmap: canvas.width, css: canvas.getBoundingClientRect().width };
  });
  await page.evaluate(() => {
    Object.defineProperty(window, "devicePixelRatio", { configurable: true, value: 2 });
    window.dispatchEvent(new Event("orientationchange"));
  });
  await page.waitForFunction((oldWidth) => document.querySelector("#chart_container canvas").width !== oldWidth, before.bitmap);
  const after = await page.evaluate(() => {
    const canvas = window.__chart.chart_element().querySelector("canvas");
    return { bitmap: canvas.width, css: canvas.getBoundingClientRect().width };
  });
  expect(after.css).toBeCloseTo(before.css, 1);
  expect(after.bitmap).toBeGreaterThan(before.bitmap);
});
