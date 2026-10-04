import { test, expect } from "@playwright/test";

// Host interaction switches gate real input in the browser recognizer (`gestures.ts` reading
// `chart.gesture_config()`): each `handle_scroll` / `handle_scale` family and the pane-resize
// switch blocks exactly the real drag or wheel it names (a gated axis or separator press never
// falls back to a pan), an unhandled wheel is left to the page (no preventDefault, the document
// scrolls), and the overlay cursor advertises only the drags that are enabled. Real right,
// middle, and double clicks and a real pointer leave keep their reference semantics: context
// without selection, no middle-button pan, a time-axis reset behind its own switch, and a
// cleared crosshair.

const OVERLAY = "#chart_container canvas:last-of-type";

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
});

async function settle_frames(page) {
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

async function goto_fixture(page) {
  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d&forceFallbackAdapter=1");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  await settle_frames(page);
}

/** Client-space pane geometry: pane x = 0 sits at `left`, chart y = 0 at `top`. */
async function geometry(page) {
  return page.evaluate((selector) => {
    const rect = document.querySelector(selector).getBoundingClientRect();
    const wasm = window.__chart.wasm;
    return {
      left: rect.left + wasm.pane_left(),
      top: rect.top,
      width: wasm.time_scale_width(),
      pane_height: wasm.pane_height(0),
      time_axis_y: rect.top + rect.height - wasm.time_scale_height() / 2,
    };
  }, OVERLAY);
}

const view = (page) => page.evaluate(() => {
  const scale = window.__chart.price_scale("right");
  return {
    range: window.__chart.time_scale().get_visible_logical_range(),
    spacing: window.__chart.wasm.bar_spacing(),
    offset: window.__chart.wasm.scroll_position(),
    price: scale.get_visible_range(),
    auto_scale: scale.options().auto_scale,
  };
});

const overlay_cursor = (page) => page.evaluate((selector) => document.querySelector(selector).style.cursor, OVERLAY);

async function drag(page, from, to, steps = 8, button = "left") {
  await page.mouse.move(from.x, from.y);
  await page.mouse.down({ button });
  await page.mouse.move(to.x, to.y, { steps });
  await page.mouse.up({ button });
}

/** A pane point above every visible candle's high: a miss for drawings and series. */
async function empty_spot(page) {
  const g = await geometry(page);
  const spot = await page.evaluate(() => {
    const range = window.__chart.time_scale().get_visible_logical_range();
    let max_high = -Infinity;
    for (let i = Math.ceil(range.from); i <= Math.floor(range.to); i++) {
      const bar = window.__main.data_by_index(i);
      if (bar) max_high = Math.max(max_high, bar.high);
    }
    const x = window.__chart.time_scale().logical_to_coordinate(Math.floor((range.from + range.to) / 2));
    return { x, y: window.__main.price_to_coordinate(max_high) - 20 };
  });
  return { x: Math.round(g.left + spot.x), y: Math.round(g.top + spot.y) };
}

/** A visible bar's body center: an engine series hit. */
async function series_spot(page) {
  const g = await geometry(page);
  const spot = await page.evaluate(() => {
    const range = window.__chart.time_scale().get_visible_logical_range();
    const index = Math.floor((range.from + range.to) / 2);
    const bar = window.__main.data_by_index(index);
    return {
      x: window.__chart.time_scale().logical_to_coordinate(index),
      y: window.__main.price_to_coordinate((bar.high + bar.low) / 2),
    };
  });
  return { x: Math.round(g.left + spot.x), y: Math.round(g.top + spot.y) };
}

async function add_second_pane(page) {
  await page.evaluate(() => {
    const series = window.__chart.add_series("line", {});
    series.set_data(window.__data.map((bar) => ({ time: bar.time, value: bar.close })));
    series.move_to_pane(1);
  });
  await settle_frames(page);
  const g = await geometry(page);
  const y = await page.evaluate(() => Array.from(window.__chart.wasm.pane_separator_ys())[0]);
  return { x: Math.round(g.left + g.width * 0.5), y: Math.round(g.top + y) };
}

const separator_y = (page) => page.evaluate(() => Array.from(window.__chart.wasm.pane_separator_ys())[0]);

/** Let the document scroll below a fixed-size chart and record each wheel's default handling. */
async function make_page_scrollable(page) {
  await page.evaluate(() => {
    document.documentElement.style.overflow = "auto";
    document.documentElement.style.height = "auto";
    document.body.style.overflow = "visible";
    document.body.style.height = "auto";
    document.getElementById("wrap").style.height = `${window.innerHeight}px`;
    const spacer = document.createElement("div");
    spacer.style.height = "2000px";
    document.body.append(spacer);
    window.__wheels = [];
    window.addEventListener("wheel", (event) => window.__wheels.push(event.defaultPrevented), { passive: true });
  });
  await settle_frames(page);
}

async function wheel_once(page, delta_x, delta_y) {
  const before = await page.evaluate(() => window.__wheels.length);
  await page.mouse.wheel(delta_x, delta_y);
  await page.waitForFunction((count) => window.__wheels.length > count, before);
  await settle_frames(page);
  return page.evaluate(() => window.__wheels.at(-1));
}

test("pressed_mouse_move off blocks a real pane drag until it is switched back on", async ({ page }) => {
  await goto_fixture(page);
  const g = await geometry(page);
  const from = { x: g.left + g.width * 0.6, y: g.top + g.pane_height * 0.3 };
  const to = { x: from.x - 150, y: from.y + 20 };

  await page.evaluate(() => window.__chart.apply_options({ handle_scroll: { pressed_mouse_move: false } }));
  const locked = await view(page);
  await drag(page, from, to);
  expect(await view(page)).toEqual(locked);

  await page.evaluate(() => window.__chart.apply_options({ handle_scroll: { pressed_mouse_move: true } }));
  await drag(page, from, to);
  expect((await view(page)).offset).not.toBeCloseTo(locked.offset, 3);
});

test("an unhandled wheel is left to the page while a handled one is consumed", async ({ page }) => {
  await goto_fixture(page);
  await make_page_scrollable(page);
  const spot = await empty_spot(page);
  await page.mouse.move(spot.x, spot.y);

  // Defaults: a vertical wheel zooms time, a horizontal wheel pans, and the page stays put.
  let before = await view(page);
  expect(await wheel_once(page, 0, 100), "zoom consumes the wheel").toBe(true);
  let after = await view(page);
  expect(after.spacing).not.toBeCloseTo(before.spacing, 6);
  before = after;
  expect(await wheel_once(page, 100, 0), "pan consumes the wheel").toBe(true);
  after = await view(page);
  expect(after.offset).not.toBeCloseTo(before.offset, 6);
  expect(await page.evaluate(() => window.scrollY)).toBe(0);

  await page.evaluate(() => window.__chart.apply_options({
    handle_scroll: { mouse_wheel: false },
    handle_scale: { mouse_wheel: false },
  }));
  before = await view(page);
  expect(await wheel_once(page, 100, 0), "a gated horizontal wheel keeps its default").toBe(false);
  expect(await wheel_once(page, 0, 100), "a gated vertical wheel keeps its default").toBe(false);
  expect(await view(page)).toEqual(before);
  await page.waitForFunction(() => window.scrollY > 0);
});

test("price-axis drag gate shows the default cursor and leaves the view alone", async ({ page }) => {
  await goto_fixture(page);
  const g = await geometry(page);
  const from = { x: g.left + g.width + 10, y: g.top + g.pane_height * 0.4 };
  const to = { x: from.x, y: from.y + 80 };

  await page.evaluate(() => window.__chart.apply_options({ handle_scale: { axis_pressed_mouse_move: { price: false } } }));
  await page.mouse.move(from.x, from.y);
  expect(await overlay_cursor(page)).toBe("default");
  const locked = await view(page);
  // The gated drag also runs sideways into the pane: an axis press never pans.
  await drag(page, from, { x: from.x - 150, y: to.y });
  expect(await view(page)).toEqual(locked);

  await page.evaluate(() => window.__chart.apply_options({ handle_scale: { axis_pressed_mouse_move: { price: true } } }));
  await page.mouse.move(from.x, from.y + 1);
  expect(await overlay_cursor(page)).toBe("ns-resize");
  await drag(page, from, to);
  const scaled = await view(page);
  expect(scaled.auto_scale).toBe(false);
  expect(scaled.price.to - scaled.price.from).not.toBeCloseTo(locked.price.to - locked.price.from, 3);
});

test("time-axis drag gate shows the default cursor and leaves the view alone", async ({ page }) => {
  await goto_fixture(page);
  const g = await geometry(page);
  const from = { x: g.left + g.width * 0.6, y: g.time_axis_y };
  const to = { x: from.x + 150, y: from.y };

  await page.evaluate(() => window.__chart.apply_options({ handle_scale: { axis_pressed_mouse_move: { time: false } } }));
  await page.mouse.move(from.x, from.y);
  expect(await overlay_cursor(page)).toBe("default");
  const locked = await view(page);
  await drag(page, from, to);
  expect(await view(page)).toEqual(locked);

  await page.evaluate(() => window.__chart.apply_options({ handle_scale: { axis_pressed_mouse_move: { time: true } } }));
  await page.mouse.move(from.x + 1, from.y);
  expect(await overlay_cursor(page)).toBe("ew-resize");
  await drag(page, from, to);
  expect((await view(page)).spacing).not.toBeCloseTo(locked.spacing, 6);
});

test("pane resize gate drops the row-resize cursor and pins the separator and the view", async ({ page }) => {
  await goto_fixture(page);
  const separator = await add_second_pane(page);
  const empty = await empty_spot(page);
  const to = { x: separator.x, y: separator.y + 60 };

  await page.evaluate(() => window.__chart.apply_options({ layout: { panes: { enableResize: false } } }));
  await page.mouse.move(empty.x, empty.y);
  await page.mouse.move(separator.x, separator.y);
  expect(await overlay_cursor(page)).toBe("crosshair");
  const pinned = await separator_y(page);
  const locked = await view(page);
  await drag(page, separator, { x: to.x - 150, y: to.y });
  expect(await separator_y(page)).toBe(pinned);
  expect(await view(page)).toEqual(locked);

  await page.evaluate(() => window.__chart.apply_options({ layout: { panes: { enableResize: true } } }));
  await page.mouse.move(separator.x + 1, separator.y);
  expect(await overlay_cursor(page)).toBe("row-resize");
  await drag(page, separator, to);
  expect(await separator_y(page)).toBeGreaterThan(pinned + 30);
});

test("a press on a price axis that cannot scale shows the default cursor and never pans", async ({ page }) => {
  await goto_fixture(page);
  const g = await geometry(page);
  const from = { x: g.left + g.width + 10, y: g.top + g.pane_height * 0.4 };

  // Percentage mode: the reference `scaleTo` is a no-op, so the axis offers no drag at all.
  await page.evaluate(() => window.__chart.price_scale("right").apply_options({ mode: 2 }));
  await settle_frames(page);
  await page.mouse.move(from.x, from.y);
  expect(await overlay_cursor(page)).toBe("default");
  const locked = await view(page);
  await drag(page, from, { x: from.x - 150, y: from.y + 80 });
  expect(await view(page)).toEqual(locked);
});

test("a fully locked view ignores real drags, wheels, and axis double-clicks", async ({ page }) => {
  await goto_fixture(page);
  const g = await geometry(page);
  // Move the view off its reset state first so a stray axis reset would show.
  await page.evaluate(() => window.__chart.price_scale("right").set_visible_range({ from: 95, to: 115 }));
  await page.evaluate(() => window.__chart.apply_options({ handle_scroll: false, handle_scale: false }));
  await settle_frames(page);
  const locked = await view(page);
  const pane = { x: g.left + g.width * 0.6, y: g.top + g.pane_height * 0.3 };
  const price_axis = { x: g.left + g.width + 10, y: g.top + g.pane_height * 0.4 };
  const time_axis = { x: g.left + g.width * 0.6, y: g.time_axis_y };

  await drag(page, pane, { x: pane.x - 150, y: pane.y + 40 });
  await page.mouse.move(pane.x, pane.y);
  await page.mouse.wheel(0, -120);
  await page.mouse.wheel(120, 0);
  await drag(page, price_axis, { x: price_axis.x, y: price_axis.y + 80 });
  await drag(page, time_axis, { x: time_axis.x + 150, y: time_axis.y });
  await page.mouse.dblclick(price_axis.x, price_axis.y);
  await page.mouse.dblclick(time_axis.x, time_axis.y);
  await settle_frames(page);
  expect(await view(page)).toEqual(locked);

  await page.mouse.move(price_axis.x, price_axis.y + 1);
  expect(await overlay_cursor(page)).toBe("default");
  await page.mouse.move(time_axis.x + 1, time_axis.y);
  expect(await overlay_cursor(page)).toBe("default");
});

test("the overlay cursor follows the region under a real hover", async ({ page }) => {
  await goto_fixture(page);
  const separator = await add_second_pane(page);
  const g = await geometry(page);
  const empty = await empty_spot(page);

  await page.mouse.move(empty.x, empty.y);
  expect(await overlay_cursor(page)).toBe("crosshair");
  await page.mouse.move(g.left + g.width + 10, g.top + g.pane_height * 0.4);
  expect(await overlay_cursor(page)).toBe("ns-resize");
  await page.mouse.move(g.left + g.width * 0.6, g.time_axis_y);
  expect(await overlay_cursor(page)).toBe("ew-resize");
  await page.mouse.move(separator.x, separator.y);
  expect(await overlay_cursor(page)).toBe("row-resize");
  await page.mouse.move(empty.x, empty.y);
  expect(await overlay_cursor(page)).toBe("crosshair");
});

test("the crosshair hides over both axis strips and resumes over the pane", async ({ page }) => {
  await goto_fixture(page);
  await page.evaluate(() => {
    window.__crosshair = [];
    window.__chart.subscribe_crosshair_move((params) => window.__crosshair.push(params.point));
  });
  const g = await geometry(page);
  const series = await series_spot(page);
  const last_point = () => page.evaluate(() => window.__crosshair.at(-1) ?? null);

  await page.mouse.move(series.x, series.y);
  expect(await last_point()).toEqual({ x: series.x - g.left, y: series.y - g.top });
  expect(await page.evaluate(() => window.__chart.hover_series_id())).toBe(await page.evaluate(() => window.__main.id));

  await page.mouse.move(g.left + g.width + 10, series.y);
  expect(await last_point(), "price axis hides the crosshair").toBeNull();
  expect(await page.evaluate(() => window.__chart.hover_series_id())).toBeNull();
  const events_on_price_axis = await page.evaluate(() => window.__crosshair.length);
  await page.mouse.move(series.x, g.time_axis_y);
  expect(await page.evaluate(() => window.__crosshair.length), "no crosshair over the time axis").toBe(events_on_price_axis);

  await page.mouse.move(series.x, series.y);
  expect(await last_point()).toEqual({ x: series.x - g.left, y: series.y - g.top });
});

test("a real time-axis double-click resets the time scale unless its switch is off", async ({ page }) => {
  await goto_fixture(page);
  const g = await geometry(page);
  const reset = await page.evaluate(() => {
    window.__chart.time_scale().reset_time_scale();
    return { spacing: window.__chart.wasm.bar_spacing(), offset: window.__chart.wasm.scroll_position() };
  });
  await settle_frames(page);
  const axis = { x: g.left + g.width * 0.6, y: g.time_axis_y };
  const zoom_by_axis_drag = async () => {
    await drag(page, axis, { x: axis.x - 150, y: axis.y });
    const zoomed = await view(page);
    expect(zoomed.spacing).not.toBeCloseTo(reset.spacing, 3);
    return zoomed;
  };

  await zoom_by_axis_drag();
  await page.mouse.dblclick(axis.x, axis.y);
  const restored = await view(page);
  expect(restored.spacing).toBeCloseTo(reset.spacing, 9);
  expect(restored.offset).toBeCloseTo(reset.offset, 9);

  await page.evaluate(() => window.__chart.apply_options({ handle_scale: { axis_double_click_reset: { time: false } } }));
  const zoomed = await zoom_by_axis_drag();
  await page.mouse.dblclick(axis.x, axis.y);
  expect(await view(page)).toEqual(zoomed);
});

test("a real right-click emits one chart context and changes no selection or view", async ({ page }) => {
  await goto_fixture(page);
  const g = await geometry(page);
  await page.evaluate(() => {
    const range = window.__chart.time_scale().get_visible_logical_range();
    const price = window.__chart.price_scale("right").get_visible_range();
    const level = price.from + (price.to - price.from) * 0.85;
    window.__drawing = window.__chart.add_drawing("trend_line", [
      { logical: Math.floor(range.from + (range.to - range.from) * 0.2), price: level },
      { logical: Math.floor(range.from + (range.to - range.from) * 0.4), price: level },
    ]);
    window.__contexts = [];
    window.__clicks = 0;
    window.__context_defaults = [];
    window.__chart.subscribe_click(() => { window.__clicks += 1; });
    window.__context_handler = (context) => window.__contexts.push({
      point: context.point,
      pane_index: context.pane_index,
    });
    window.__chart.subscribe_chart_context(window.__context_handler);
    window.addEventListener("contextmenu", (event) => window.__context_defaults.push(event.defaultPrevented));
  });
  await settle_frames(page);
  const body = await page.evaluate(() => {
    const [a, b] = window.__drawing.points();
    const scale = window.__chart.time_scale();
    return {
      x: (scale.logical_to_coordinate(a.logical) + scale.logical_to_coordinate(b.logical)) / 2,
      y: window.__main.price_to_coordinate(a.price),
    };
  });
  await page.mouse.click(g.left + body.x, g.top + body.y);
  const selected = await page.evaluate(() => window.__chart.selected_drawing()?.id ?? null);
  expect(selected).toBe(await page.evaluate(() => window.__drawing.id));
  const clicks = await page.evaluate(() => window.__clicks);
  const before = await view(page);

  const empty = await empty_spot(page);
  await page.mouse.click(empty.x, empty.y, { button: "right" });
  let state = await page.evaluate(() => ({
    contexts: window.__contexts,
    clicks: window.__clicks,
    defaults: window.__context_defaults,
    selected: window.__chart.selected_drawing()?.id ?? null,
    drawings: window.__chart.drawings().length,
  }));
  expect(state.contexts).toEqual([{ point: { x: empty.x - g.left, y: empty.y - g.top }, pane_index: 0 }]);
  expect(state.clicks).toBe(clicks);
  expect(state.defaults, "a handled context menu suppresses the native one").toEqual([true]);
  expect(state.selected).toBe(selected);
  expect(state.drawings).toBe(1);

  // A right-button drag is no pan either.
  await drag(page, empty, { x: empty.x - 150, y: empty.y }, 8, "right");
  expect(await view(page)).toEqual(before);
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id ?? null)).toBe(selected);
  expect(await page.evaluate(() => window.__contexts.length)).toBe(2);

  // Without a context subscriber the browser keeps its own menu.
  await page.evaluate(() => {
    window.__context_defaults.length = 0;
    window.__chart.unsubscribe_chart_context(window.__context_handler);
  });
  await page.mouse.click(empty.x, empty.y, { button: "right" });
  state = await page.evaluate(() => ({ contexts: window.__contexts.length, defaults: window.__context_defaults }));
  expect(state).toEqual({ contexts: 2, defaults: [false] });
});

test("a middle-button drag neither pans nor moves the crosshair", async ({ page }) => {
  await goto_fixture(page);
  await page.evaluate(() => {
    window.__crosshair = [];
    window.__clicks = 0;
    window.__chart.subscribe_crosshair_move((params) => window.__crosshair.push(params.point));
    window.__chart.subscribe_click(() => { window.__clicks += 1; });
    window.__middle_down_prevented = null;
    window.addEventListener("mousedown", (event) => {
      if (event.button === 1) window.__middle_down_prevented = event.defaultPrevented;
    });
  });
  const g = await geometry(page);
  const from = await empty_spot(page);
  await page.mouse.move(from.x, from.y);
  const before = await view(page);
  const events = await page.evaluate(() => window.__crosshair.length);

  await page.mouse.down({ button: "middle" });
  await page.mouse.move(from.x - 150, from.y + 30, { steps: 8 });
  await page.mouse.up({ button: "middle" });
  expect(await view(page)).toEqual(before);
  expect(await page.evaluate(() => window.__crosshair.length), "middle-drag moves feed no crosshair").toBe(events);
  expect(await page.evaluate(() => window.__crosshair.at(-1))).toEqual({ x: from.x - g.left, y: from.y - g.top });
  expect(await page.evaluate(() => window.__clicks)).toBe(0);
  // Chrome's middle-click autoscroll is suppressed, as in the reference (Chrome only).
  expect(await page.evaluate(() => window.__middle_down_prevented)).toBe(
    await page.evaluate(() => window.chrome !== undefined),
  );
});

test("a real pointer leave clears the crosshair and the hovered series", async ({ page }) => {
  await goto_fixture(page);
  // Leave room around the chart so the pointer can exit it inside the viewport.
  await page.evaluate(() => window.__chart.resize(1000, 560));
  await settle_frames(page);
  await page.evaluate(() => {
    window.__crosshair = [];
    window.__chart.subscribe_crosshair_move((params) => window.__crosshair.push(params.point));
  });
  const g = await geometry(page);
  const series = await series_spot(page);
  await page.mouse.move(series.x, series.y);
  expect(await page.evaluate(() => window.__chart.hover_series_id())).toBe(await page.evaluate(() => window.__main.id));
  expect(await page.evaluate(() => window.__crosshair.at(-1))).toEqual({ x: series.x - g.left, y: series.y - g.top });

  await page.mouse.move(g.left + 1100, g.top + 640);
  expect(await page.evaluate(() => window.__crosshair.at(-1))).toBeNull();
  expect(await page.evaluate(() => window.__chart.hover_series_id())).toBeNull();
});
