import { test, expect } from "@playwright/test";

async function open_chart(page) {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() === "canvas2d");
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
}

test("auto wheel retains reference-informed cursor anchoring, modifier neutrality, and independent horizontal pan", async ({ page }) => {
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

  const beforeZoom = await state();
  expect(beforeZoom.rightBarStays).toBe(false);
  await page.mouse.wheel(0, -24);
  const afterZoom = await state();
  expect(afterZoom.spacing).toBeGreaterThan(beforeZoom.spacing);
  expect(afterZoom.logical).toBeCloseTo(beforeZoom.logical, 5);
  expect(afterZoom.offset).not.toBeCloseTo(beforeZoom.offset, 8);

  const beforeFocused = await state();
  await page.keyboard.down("Control");
  await page.mouse.wheel(0, -24);
  await page.keyboard.up("Control");
  const afterFocused = await state();
  expect(afterFocused.spacing).toBeGreaterThan(beforeFocused.spacing);
  expect(afterFocused.logical).toBeCloseTo(beforeFocused.logical, 5);
  expect(afterFocused.offset).not.toBeCloseTo(beforeFocused.offset, 8);

  const beforeShiftPan = await state();
  await page.keyboard.down("Shift");
  await page.mouse.wheel(0, -24);
  await page.keyboard.up("Shift");
  const afterShiftPan = await state();
  expect(afterShiftPan.spacing).toBeGreaterThan(beforeShiftPan.spacing);
  expect(afterShiftPan.logical).toBeCloseTo(beforeShiftPan.logical, 5);

  const beforePan = await state();
  await page.mouse.wheel(24, 0);
  const afterPan = await state();
  expect(afterPan.spacing).toBeCloseTo(beforePan.spacing, 8);
  expect(afterPan.offset).not.toBeCloseTo(beforePan.offset, 8);
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
