import { test, expect } from "@playwright/test";
import { PNG } from "pngjs";

async function open_trading_demo(page, backend = "canvas2d") {
  await page.goto(`/?feature=trading&backend=${backend}`);
  await page.waitForFunction(() => window.__demo_catalogs?.lab.active_ids().includes("trading-bracket"));
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  await page.evaluate(() => {
    // The close control is the trailing cell of the marker's one container. Sweep the marker span
    // for it instead of hardcoding the cell widths, and answer with the cell's center.
    window.__close_x = (id, y) => {
      const trading = window.__chart.trading();
      const width = Math.round(window.__chart.time_scale().width());
      let first = null;
      let last = null;
      for (let x = width; x > width - 320; x -= 1) {
        const hit = trading.hit_at(x, y);
        if (hit?.id === id && hit.kind === "cancel_button") {
          if (last === null) last = x;
          first = x;
        } else if (last !== null) {
          break;
        }
      }
      return first === null ? null : (first + last) / 2;
    };
  });
}

function count_near(image, expected, tolerance = 10) {
  let count = 0;
  for (let offset = 0; offset < image.data.length; offset += 4) {
    if (
      Math.abs(image.data[offset] - expected[0]) <= tolerance
      && Math.abs(image.data[offset + 1] - expected[1]) <= tolerance
      && Math.abs(image.data[offset + 2] - expected[2]) <= tolerance
      && image.data[offset + 3] > 200
    ) count += 1;
  }
  return count;
}

test("first-party trading snapshot preserves identity and broker relationships", async ({ page }) => {
  await open_trading_demo(page);
  const state = await page.evaluate(() => window.__chart.trading().state());
  expect(state.positions).toEqual([expect.objectContaining({
    id: "demo-position",
    side: "long",
    quantity: 12,
  })]);
  expect(state.orders).toHaveLength(3);
  expect(state.orders.find((order) => order.id === "demo-target")).toMatchObject({
    role: "take_profit",
    position_id: "demo-position",
    bracket_id: "demo-bracket",
    oco_group_id: "demo-oco",
  });
  expect(state.orders.find((order) => order.id === "demo-stop")).toMatchObject({
    role: "stop_loss",
    oco_group_id: "demo-oco",
  });
  expect(state.orders.find((order) => order.id === "demo-partial")).toMatchObject({
    status: "partially_filled",
    quantity: 12,
    filled_quantity: 5,
  });
  expect(state.executions).toEqual([expect.objectContaining({
    id: "demo-fill",
    kind: "partial_fill",
    order_id: "demo-partial",
  })]);

  const invalid = await page.evaluate(() => {
    const trading = window.__chart.trading();
    const before = trading.state();
    try {
      trading.apply_snapshot({
        positions: [
          { id: "duplicate", side: "long", average_price: 100, quantity: 1 },
          { id: "duplicate", side: "short", average_price: 101, quantity: 1 },
        ],
      });
      return { threw: false };
    } catch (error) {
      return { threw: true, code: error.code, unchanged: JSON.stringify(trading.state()) === JSON.stringify(before) };
    }
  });
  expect(invalid).toEqual({ threw: true, code: "invalid_data", unchanged: true });
});

test("selected position drawing places one host-sized bracket request", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  expect(await page.evaluate(() => window.__demo_catalogs.lab.active_ids())).not.toContain("trading-bracket");
  const placement = await page.evaluate(() => {
    const range = window.__chart.time_scale().get_visible_logical_range();
    const logical = Math.floor(range.from + (range.to - range.from) * 0.55);
    const bar = window.__main.data_by_index(logical);
    const rect = document.getElementById("chart_container").getBoundingClientRect();
    return {
      x: rect.left + window.__chart.time_scale().logical_to_coordinate(logical),
      y: rect.top + window.__main.price_to_coordinate((bar.high + bar.low) / 2),
    };
  });
  await page.click('#drawings_group [data-tool="long_position"]');
  await page.mouse.click(placement.x, placement.y);

  const setup = await page.evaluate(() => {
    const drawing = window.__chart.drawings().at(-1);
    window.__placed_bracket_intents = [];
    window.__chart.trading().subscribe_intents((intent) => {
      if (intent.action === "place_bracket_order") window.__placed_bracket_intents.push(intent);
    });
    const points = drawing?.points() ?? [];
    return {
      drawing_id: drawing?.id,
      drawing_kind: drawing?.kind(),
      drawing_count: window.__chart.drawings().length,
      entry_price: points[0]?.price,
      take_profit_price: points[1]?.price,
      stop_loss_price: points[2]?.price,
    };
  });
  expect(setup.drawing_count).toBe(1);
  expect(setup.drawing_kind).toBe("long_position");
  await expect(page.locator("#place_position_order")).toBeEnabled();
  await expect(page.locator("#place_position_order")).toHaveText("place long order");
  await page.fill("#position_order_quantity", "7");
  await page.click("#place_position_order");

  const placed = await page.evaluate(() => ({
    intents: window.__placed_bracket_intents,
    trading_state: window.__chart.trading().state(),
    drawing_count: window.__chart.drawings().length,
  }));
  expect(placed.intents).toHaveLength(1);
  expect(placed.intents[0]).toEqual(expect.objectContaining({
    action: "place_bracket_order",
    drawing_id: setup.drawing_id,
    pane_index: 0,
    price_scale: "right",
    side: "buy",
    kind: "limit",
    role: "working",
    quantity: 7,
  }));
  expect(placed.intents[0].price).toBeCloseTo(setup.entry_price, 10);
  expect(placed.intents[0].take_profit_price).toBeCloseTo(setup.take_profit_price, 10);
  expect(placed.intents[0].stop_loss_price).toBeCloseTo(setup.stop_loss_price, 10);
  expect(await page.evaluate(() => window.__demo_catalogs.lab.active_ids())).not.toContain("trading-bracket");
  expect(placed.trading_state.positions).toEqual([]);
  expect(placed.trading_state.executions).toEqual([]);
  expect(placed.trading_state.orders).toHaveLength(3);
  expect(placed.trading_state.orders).toEqual(expect.arrayContaining([
    expect.objectContaining({ id: "demo-entry-1", role: "working", side: "buy", quantity: 7 }),
    expect.objectContaining({ id: "demo-target-1", role: "take_profit", side: "sell", quantity: 7 }),
    expect.objectContaining({ id: "demo-stop-1", role: "stop_loss", side: "sell", quantity: 7 }),
  ]));
  expect(placed.drawing_count, "broker acknowledgement does not remove the planning tool").toBe(1);
});

test("quantity cells fit their formatted text and move the close hit with them", async ({ page }) => {
  await open_trading_demo(page);
  const widths = await page.evaluate(() => {
    const trading = window.__chart.trading();
    const snapshot = (quantity) => trading.apply_snapshot({
      instrument: { quantity_precision: 0 },
      positions: [{ id: "sized-position", side: "long", average_price: 100, quantity }],
    });
    const y = window.__main.price_to_coordinate(100);
    snapshot(12);
    const short = window.__close_x("sized-position", y);
    snapshot(123456);
    const long = window.__close_x("sized-position", y);
    return {
      short,
      long,
      long_hit: trading.hit_at(long, y),
      old_hit: trading.hit_at(short, y),
    };
  });
  expect(widths.long).toBeGreaterThan(widths.short + 20);
  expect(widths.long_hit).toMatchObject({ id: "sized-position", kind: "cancel_button" });
  expect(widths.old_hit).toMatchObject({ id: "sized-position", kind: "position_line" });
});

test("trading lines use dedicated hits and render semantic colors through the shared frame", async ({ page }) => {
  await open_trading_demo(page);
  const probe = await page.evaluate(() => {
    const trading = window.__chart.trading();
    const target = trading.state().orders.find((order) => order.id === "demo-target");
    const position = trading.state().positions[0];
    const position_y = window.__main.price_to_coordinate(position.average_price);
    const width = window.__chart.time_scale().width();
    const first_line_hit = (id, y) => {
      for (let x = 0; x <= width; x += 1) {
        const hit = trading.hit_at(x, y);
        if (hit?.id === id && hit.kind.endsWith("_line")) return x;
      }
      return null;
    };
    const exact_axis_controls = [
      { id: position.id, price: position.average_price },
      ...trading.state().orders.map((order) => ({ id: order.id, price: order.price })),
    ].map(({ id, price }) => {
      const y = window.__main.price_to_coordinate(price);
      return { id, y, hit: trading.hit_at(window.__close_x(id, y), y) };
    });
    return {
      left_line: trading.hit_at(40, window.__main.price_to_coordinate(target.price)),
      chart_width: width,
      order: trading.hit_at(width - 200, window.__main.price_to_coordinate(target.price)),
      position: trading.hit_at(width - 200, position_y),
      order_start: first_line_hit(target.id, window.__main.price_to_coordinate(target.price)),
      position_start: first_line_hit(position.id, position_y),
      close: trading.hit_at(window.__close_x(position.id, position_y), position_y),
      exact_axis_controls,
    };
  });
  // A confirmed protection order is draggable from its complete visible rule, not only the marker.
  expect(probe.left_line).toMatchObject({ id: "demo-target", kind: "order_line" });
  expect(probe.order).toMatchObject({
    object_type: "order",
    id: "demo-target",
    kind: "order_line",
  });
  expect(probe.position).toMatchObject({ object_type: "position", kind: "position_line" });
  expect(probe.position_start, "order and position rules share one interactive start").toBe(probe.order_start);
  expect(probe.order_start, "the complete visible rule is interactive, not only its marker").toBeLessThan(40);
  expect(probe.close).toMatchObject({ object_type: "position", kind: "cancel_button" });
  for (const control of probe.exact_axis_controls) {
    expect(control.hit, `${control.id} close control must remain on its exact price coordinate`).toMatchObject({
      id: control.id,
      kind: "cancel_button",
    });
  }

  const url = await page.evaluate(() => window.__chart.take_screenshot().toDataURL("image/png"));
  const image = PNG.sync.read(Buffer.from(url.split(",")[1], "base64"));
  expect(count_near(image, [245, 158, 10]), "stop-loss order line/label pixels").toBeGreaterThan(100);
  expect(count_near(image, [8, 153, 129]), "long position and buy order pixels").toBeGreaterThan(100);
});

test("trading-line hover and drag apply the engine cursor", async ({ page }) => {
  await open_trading_demo(page);
  const y = await page.evaluate(() => {
    const order = window.__chart.trading().state().orders.find((entry) => entry.id === "demo-target");
    return window.__main.price_to_coordinate(order.price);
  });
  const overlay = page.locator("#chart_container canvas:last-of-type");
  const box = await overlay.boundingBox();
  await page.mouse.move(box.x + 40, box.y + y);
  expect(await overlay.evaluate((element) => element.style.cursor)).toBe("grab");
  await page.mouse.down();
  await page.mouse.move(box.x + 40, box.y + y - 20);
  expect(await overlay.evaluate((element) => element.style.cursor)).toBe("grabbing");
  await page.mouse.up();
});

test("trading state is chart-local and clear removes all live objects", async ({ page }) => {
  await open_trading_demo(page);
  await page.evaluate(() => window.__demo_catalogs.lab.clear());
  expect(await page.evaluate(() => window.__chart.trading().state())).toEqual({
    instrument: {},
    positions: [],
    orders: [],
    executions: [],
    round_trips: [],
  });
});

test("pointer drag has trading priority and emits one broker-neutral modify intent", async ({ page }) => {
  await open_trading_demo(page);
  const probe = await page.evaluate(() => {
    window.__trading_intents = [];
    window.__chart.trading().subscribe_intents((intent) => window.__trading_intents.push(intent));
    const order = window.__chart.trading().state().orders.find((item) => item.id === "demo-target");
    window.__trading_blocker = window.__chart.add_drawing(
      "horizontal_line",
      [{ logical: 620, price: order.price }],
      { color: "#a459d1" },
    );
    const overlay = document.querySelector("#chart_container canvas:last-of-type").getBoundingClientRect();
    const before_range = window.__chart.time_scale().get_visible_logical_range();
    return {
      from: { x: overlay.left + window.__chart.time_scale().width() - 200, y: overlay.top + window.__main.price_to_coordinate(order.price) },
      to: { x: overlay.left + window.__chart.time_scale().width() - 200, y: overlay.top + window.__main.price_to_coordinate(order.price + 1.25) },
      before_range,
      confirmed_price: order.price,
      drawing_points: window.__trading_blocker.points(),
    };
  });
  await page.mouse.move(probe.from.x, probe.from.y);
  await page.mouse.down();
  await page.mouse.move(probe.to.x, probe.to.y, { steps: 6 });
  await page.mouse.up();

  const result = await page.evaluate(() => ({
    intents: window.__trading_intents,
    preview: window.__chart.trading().preview(),
    confirmed: window.__chart.trading().state().orders.find((item) => item.id === "demo-target"),
    range: window.__chart.time_scale().get_visible_logical_range(),
    drawing_points: window.__trading_blocker.points(),
  }));
  expect(result.intents).toHaveLength(1);
  expect(result.intents[0]).toMatchObject({
    action: "modify_order",
    order_id: "demo-target",
    role: "take_profit",
    base_revision: 0,
  });
  // Release applies the move; nothing lingers in a preview waiting on the host.
  expect(result.preview).toBeNull();
  expect(result.confirmed.price).not.toBe(probe.confirmed_price);
  expect(result.confirmed.price).toBe(result.intents[0].price);
  // The drag stayed inside trading: it neither panned the chart nor grabbed the drawing beneath it.
  expect(result.range).toEqual(probe.before_range);
  expect(result.drawing_points).toEqual(probe.drawing_points);

  // Rejecting puts the price back where it was.
  expect(await page.evaluate(() => {
    const sequence = window.__trading_intents[0].sequence;
    return window.__chart.trading().resolve_intent(sequence, false);
  })).toBe(true);
  expect(await page.evaluate(() => ({
    preview: window.__chart.trading().preview(),
    price: window.__chart.trading().state().orders.find((item) => item.id === "demo-target").price,
  }))).toEqual({ preview: null, price: probe.confirmed_price });
});

test("unlinked protection orders remain draggable and preserve stop-limit modify fields", async ({ page }) => {
  await open_trading_demo(page);
  const probe = await page.evaluate(() => {
    const trading = window.__chart.trading();
    trading.apply_snapshot({
      instrument: { tick_size: 0.25, price_precision: 2 },
      orders: [{
        id: "orphan-stop",
        side: "sell",
        kind: "stop_limit",
        role: "stop_loss",
        status: "working",
        price: 100,
        stop_price: 99.5,
        quantity: 2,
        revision: 7,
      }],
    });
    window.__orphan_intents = [];
    trading.subscribe_intents((intent) => window.__orphan_intents.push(intent));
    const overlay = document.querySelector("#chart_container canvas:last-of-type").getBoundingClientRect();
    const width = window.__chart.time_scale().width();
    const chip_x = width - 200;
    const y = window.__main.price_to_coordinate(100);
    return {
      from: { x: overlay.left + chip_x, y: overlay.top + y },
      to: { x: overlay.left + chip_x, y: overlay.top + window.__main.price_to_coordinate(98) },
      hit: trading.hit_at(chip_x, y),
    };
  });
  expect(probe.hit).toMatchObject({ id: "orphan-stop", kind: "order_line" });
  await page.mouse.move(probe.from.x, probe.from.y);
  expect(await page.locator("#chart_container canvas:last-of-type").evaluate((canvas) => canvas.style.cursor)).toBe("grab");
  await page.mouse.down();
  await page.mouse.move(probe.to.x, probe.to.y, { steps: 6 });
  await page.mouse.up();
  expect(await page.evaluate(() => window.__orphan_intents)).toEqual([
    expect.objectContaining({
      action: "modify_order",
      order_id: "orphan-stop",
      kind: "stop_limit",
      stop_price: 99.5,
      price: 98,
      base_revision: 7,
    }),
  ]);
});

test("a control's action tooltip waits out the hover dwell instead of appearing on contact", async ({ page }) => {
  await open_trading_demo(page);
  const probe = await page.evaluate(() => {
    // Hide the crosshair so the band diff measures the tooltip alone, not the pointer's own chrome.
    window.__chart.apply_options({ crosshair: { mode: 2 } });
    const order = window.__chart.trading().state().orders.find((item) => item.id === "demo-target");
    const overlay = document.querySelector("#chart_container canvas:last-of-type").getBoundingClientRect();
    const y = window.__main.price_to_coordinate(order.price);
    return {
      x: overlay.left + window.__close_x(order.id, y),
      y: overlay.top + y,
      line_y: y,
      pane_width: window.__chart.time_scale().width(),
      dpr: window.devicePixelRatio,
    };
  });
  const shot = async () => {
    const url = await page.evaluate(() => window.__chart.take_screenshot().toDataURL("image/png"));
    return PNG.sync.read(Buffer.from(url.split(",")[1], "base64"));
  };
  // The tooltip is drawn in the band just above the hovered control. Diffing only that band keeps
  // the once-a-second countdown chip in the axis strip out of the measurement.
  const band = {
    top: Math.round((probe.line_y - 62) * probe.dpr),
    bottom: Math.round((probe.line_y - 12) * probe.dpr),
    right: Math.round((probe.pane_width - 40) * probe.dpr),
  };
  const changed_in_band = (a, b) => {
    let count = 0;
    for (let y = Math.max(band.top, 0); y < band.bottom; y += 1) {
      for (let x = 0; x < Math.min(band.right, a.width); x += 1) {
        const offset = (y * a.width + x) * 4;
        if (a.data[offset] !== b.data[offset]
          || a.data[offset + 1] !== b.data[offset + 1]
          || a.data[offset + 2] !== b.data[offset + 2]) count += 1;
      }
    }
    return count;
  };

  const resting = await shot();
  await page.mouse.move(probe.x, probe.y);
  await page.waitForTimeout(120);
  const early = await shot();
  expect(changed_in_band(resting, early), "nothing appears above the control within the dwell").toBe(0);

  await page.waitForTimeout(700);
  const late = await shot();
  expect(changed_in_band(early, late), "the tooltip appears once the dwell elapses").toBeGreaterThan(300);

  // Moving off the control retracts it and restarts the dwell.
  await page.mouse.move(probe.x - 220, probe.y);
  await page.waitForTimeout(120);
  expect(changed_in_band(resting, await shot()), "the tooltip retracts with the hover").toBe(0);
});

test("cancel control removes the order, emits the intent, and a rejection restores it", async ({ page }) => {
  await open_trading_demo(page);
  const probe = await page.evaluate(() => {
    window.__cancel_intents = [];
    window.__chart.trading().subscribe_intents((intent) => window.__cancel_intents.push(intent));
    const order = window.__chart.trading().state().orders.find((item) => item.id === "demo-stop");
    const overlay = document.querySelector("#chart_container canvas:last-of-type").getBoundingClientRect();
    const y = window.__main.price_to_coordinate(order.price);
    return {
      x: overlay.left + window.__close_x(order.id, y),
      y: overlay.top + y,
    };
  });
  await page.mouse.click(probe.x, probe.y);
  expect(await page.evaluate(() => window.__cancel_intents)).toEqual([
    expect.objectContaining({ action: "cancel_order", order_id: "demo-stop" }),
  ]);
  // Closing means gone: the order leaves the chart with the intent, not after it.
  expect(await page.evaluate(() => window.__chart.trading().state().orders.some((order) => order.id === "demo-stop"))).toBe(false);
  expect(await page.evaluate(() => window.__chart.trading().preview())).toBeNull();

  expect(await page.evaluate(() => window.__chart.trading().resolve_intent(window.__cancel_intents[0].sequence, false))).toBe(true);
  expect(await page.evaluate(() => window.__chart.trading().state().orders.find((order) => order.id === "demo-stop"))).toMatchObject({
    id: "demo-stop",
    status: "working",
  });
});

test("releasing over a trading close control requires the press to start there", async ({ page }) => {
  await open_trading_demo(page);
  const probe = await page.evaluate(() => {
    const chart = window.__chart;
    const trading = chart.trading();
    const order = trading.state().orders.find((entry) => entry.id === "demo-stop");
    const y = window.__main.price_to_coordinate(order.price);
    const close = window.__close_x(order.id, y);
    let marker = null;
    for (let x = close - 2; x > close - 150; x -= 1) {
      const hit = trading.hit_at(x, y);
      if (hit?.id === order.id && hit.kind !== "cancel_button") {
        marker = x;
        break;
      }
    }
    if (marker === null) throw new Error("no adjacent trading marker segment");
    window.__press_origin_intents = [];
    trading.subscribe_intents((intent) => window.__press_origin_intents.push(intent));
    const bounds = chart.chart_element().querySelector("canvas:last-of-type").getBoundingClientRect();
    return { start: { x: bounds.left + marker, y: bounds.top + y },
      close: { x: bounds.left + close, y: bounds.top + y } };
  });
  await page.mouse.move(probe.start.x, probe.start.y);
  await page.mouse.down();
  await page.mouse.move(probe.close.x, probe.close.y);
  await page.mouse.up();
  expect(await page.evaluate(() => window.__press_origin_intents.some((intent) =>
    intent.action === "cancel_order" && intent.order_id === "demo-stop"))).toBe(false);
  expect(await page.evaluate(() => window.__chart.trading().state().orders.some((order) =>
    order.id === "demo-stop"))).toBe(true);
});

test("bracket connector disappears as soon as the host acknowledges the drag", async ({ page }) => {
  await open_trading_demo(page, "canvas2d");
  const probe = await page.evaluate(() => {
    window.__connector_intents = [];
    const trading = window.__chart.trading();
    trading.subscribe_intents((intent) => window.__connector_intents.push(intent));
    const target = trading.state().orders.find((order) => order.id === "demo-target");
    const overlay = document.querySelector("#chart_container canvas:last-of-type").getBoundingClientRect();
    return {
      overlay: { left: overlay.left, top: overlay.top, width: overlay.width, height: overlay.height },
      pane_width: window.__chart.time_scale().width(),
      connector_color: getComputedStyle(document.documentElement).getPropertyValue("--primary").trim(),
      from_y: window.__main.price_to_coordinate(target.price),
      to_y: window.__main.price_to_coordinate(target.price + 0.5),
    };
  });
  await page.mouse.move(probe.overlay.left + probe.pane_width - 200, probe.overlay.top + probe.from_y);
  await page.mouse.down();
  await page.mouse.move(probe.overlay.left + 30, probe.overlay.top + probe.to_y, { steps: 5 });
  await page.mouse.up();
  const states = await page.evaluate(() => {
    const trading = window.__chart.trading();
    const intent = window.__connector_intents[0];
    const pending_canvas = window.__chart.take_screenshot();
    trading.resolve_intent(intent.sequence, true);
    const target = trading.state().orders.find((order) => order.id === "demo-target");
    trading.update_order({ ...target, price: intent.price, revision: target.revision + 1 });
    const stop = trading.state().orders.find((order) => order.id === "demo-stop");
    const acknowledged_canvas = window.__chart.take_screenshot();
    return {
      pending_url: pending_canvas.toDataURL("image/png"),
      acknowledged_url: acknowledged_canvas.toDataURL("image/png"),
      width: acknowledged_canvas.width,
      height: acknowledged_canvas.height,
      top: window.__main.price_to_coordinate(intent.price),
      bottom: window.__main.price_to_coordinate(stop.price),
      order_ids: trading.state().orders.map((order) => order.id),
    };
  });
  expect(states.order_ids).toEqual(["demo-target", "demo-stop", "demo-partial"]);

  const connector_pixels = (url) => {
    const image = PNG.sync.read(Buffer.from(url.split(",")[1], "base64"));
    const connector_rgb = probe.connector_color.match(/[\da-f]{2}/gi).map((value) => Number.parseInt(value, 16));
    const scale_x = states.width / probe.overlay.width;
    const scale_y = states.height / probe.overlay.height;
    const x = Math.round((probe.pane_width - 8) * scale_x);
    const y0 = Math.round(Math.min(states.top, states.bottom) * scale_y);
    const y1 = Math.round(Math.max(states.top, states.bottom) * scale_y);
    let count = 0;
    for (let y = y0; y <= y1; y += 1) {
      for (let dx = -2; dx <= 2; dx += 1) {
        const offset = (y * image.width + x + dx) * 4;
        if (
          Math.abs(image.data[offset] - connector_rgb[0]) <= 12
          && Math.abs(image.data[offset + 1] - connector_rgb[1]) <= 12
          && Math.abs(image.data[offset + 2] - connector_rgb[2]) <= 12
          && image.data[offset + 3] > 200
        ) count += 1;
      }
    }
    return count;
  };
  expect(connector_pixels(states.pending_url)).toBeGreaterThan(connector_pixels(states.acknowledged_url) + 20);
});

test("dedicated entry TP and SL buttons create fixed-role protection", async ({ page }) => {
  await open_trading_demo(page);
  const probe = await page.evaluate(() => {
    const trading = window.__chart.trading();
    trading.apply_snapshot({
      instrument: { tick_size: 0.25, price_precision: 2 },
      orders: [
        { id: "buy-limit", side: "buy", kind: "limit", status: "working", price: 100, quantity: 1 },
        { id: "sell-market", side: "sell", kind: "market", status: "filled", price: 98, quantity: 1, filled_quantity: 1 },
      ],
    });
    window.__manual_intents = [];
    trading.subscribe_intents((intent) => window.__manual_intents.push(intent));
    const overlay = document.querySelector("#chart_container canvas:last-of-type").getBoundingClientRect();
    const width = window.__chart.time_scale().width();
    const button_x = (kind, y) => {
      for (let x = 0; x <= width; x += 0.5) {
        if (trading.hit_at(x, y)?.kind === kind) return x;
      }
      throw new Error(`missing ${kind}`);
    };
    const buy_y = window.__main.price_to_coordinate(100);
    const sell_y = window.__main.price_to_coordinate(98);
    return {
      overlay: { left: overlay.left, top: overlay.top },
      width,
      buy_y,
      target_y: window.__main.price_to_coordinate(101.25),
      sell_y,
      sell_target_y: window.__main.price_to_coordinate(99.25),
      buy_tp_x: button_x("take_profit_button", buy_y),
      sell_sl_x: button_x("stop_loss_button", sell_y),
      hits: {
        empty_left: trading.hit_at(40, window.__main.price_to_coordinate(100)),
        marker: trading.hit_at(width - 200, window.__main.price_to_coordinate(100)),
        cancel: trading.hit_at(
          window.__close_x("buy-limit", window.__main.price_to_coordinate(100)),
          window.__main.price_to_coordinate(100),
        ),
      },
    };
  });
  expect(probe.hits.empty_left).toMatchObject({ id: "buy-limit", kind: "order_line" });
  expect(probe.hits.marker).toMatchObject({ id: "buy-limit", kind: "order_line" });
  expect(probe.hits.cancel).toMatchObject({ id: "buy-limit", kind: "cancel_button" });

  // A buy entry dragged upward creates a take profit and leaves the entry untouched.
  await page.mouse.move(probe.overlay.left + probe.buy_tp_x, probe.overlay.top + probe.buy_y);
  await page.mouse.down();
  await page.mouse.move(probe.overlay.left + 30, probe.overlay.top + probe.target_y, { steps: 5 });
  await page.mouse.up();
  expect(await page.evaluate(() => ({
    preview: window.__chart.trading().preview(),
    intents: window.__manual_intents,
    order: window.__chart.trading().state().orders.find((order) => order.id === "buy-limit"),
  }))).toMatchObject({
    preview: null,
    intents: [expect.objectContaining({
      action: "create_take_profit",
      order_id: "buy-limit",
      side: "sell",
      kind: "limit",
      role: "take_profit",
      price: 101.25,
    })],
    order: { price: 100 },
  });
  await page.evaluate(() => window.__chart.trading().resolve_intent(window.__manual_intents[0].sequence, false));
  expect(await page.evaluate(() => window.__chart.trading().state().orders.find((order) => order.id === "buy-limit").price)).toBe(100);

  // A filled sell-side market entry dragged upward creates a stop loss. Market entries use the
  // same interaction even though their remaining quantity is zero.
  await page.mouse.move(probe.overlay.left + probe.sell_sl_x, probe.overlay.top + probe.sell_y);
  await page.mouse.down();
  await page.mouse.move(probe.overlay.left + 30, probe.overlay.top + probe.sell_target_y, { steps: 5 });
  await page.mouse.up();
  expect(await page.evaluate(() => ({
    preview: window.__chart.trading().preview(),
    intent: window.__manual_intents.at(-1),
    intent_count: window.__manual_intents.length,
  }))).toMatchObject({
    preview: null,
    intent: {
      action: "create_stop_loss",
      order_id: "sell-market",
      side: "buy",
      kind: "stop",
      role: "stop_loss",
      quantity: 1,
      price: 99.25,
    },
    intent_count: 2,
  });
});

test("existing TP and SL adjustments release into intents with no confirmation surface", async ({ page }) => {
  await open_trading_demo(page);
  const probe = await page.evaluate(() => {
    const trading = window.__chart.trading();
    window.__protection_intents = [];
    trading.subscribe_intents((intent) => window.__protection_intents.push(intent));
    const overlay = document.querySelector("#chart_container canvas:last-of-type").getBoundingClientRect();
    const target = trading.state().orders.find((order) => order.id === "demo-target");
    const stop = trading.state().orders.find((order) => order.id === "demo-stop");
    const position = trading.state().positions.find((item) => item.id === "demo-position");
    return {
      overlay: { left: overlay.left, top: overlay.top },
      width: window.__chart.time_scale().width(),
      target_y: window.__main.price_to_coordinate(target.price),
      target_next_y: window.__main.price_to_coordinate(target.price + 0.75),
      stop_y: window.__main.price_to_coordinate(stop.price),
      // Cross the long entry deliberately: a confirmed SL keeps its role after placement.
      stop_next_y: window.__main.price_to_coordinate(position.average_price + 0.75),
    };
  });

  for (const [id, role, from_y, to_y, expected_count] of [
    ["demo-target", "take_profit", probe.target_y, probe.target_next_y, 1],
    ["demo-stop", "stop_loss", probe.stop_y, probe.stop_next_y, 2],
  ]) {
    await page.mouse.move(probe.overlay.left + probe.width - 200, probe.overlay.top + from_y);
    await page.mouse.down();
    await page.mouse.move(probe.overlay.left + 30, probe.overlay.top + to_y, { steps: 5 });
    await page.mouse.up();
    expect(await page.evaluate(() => ({
      preview: window.__chart.trading().preview(),
      intents: window.__protection_intents,
    }))).toMatchObject({
      preview: null,
      intents: expect.arrayContaining([
        expect.objectContaining({ action: "modify_order", order_id: id, role }),
      ]),
    });
    expect(await page.evaluate(() => window.__protection_intents.length)).toBe(expected_count);
    await page.evaluate((sequence) => window.__chart.trading().resolve_intent(sequence, false), (await page.evaluate(() => window.__protection_intents.at(-1).sequence)));
  }

  // Nothing on the chart offers a confirm or discard control any more.
  expect(await page.evaluate(() => "set_confirmation_mode" in window.__chart.trading())).toBe(false);
});

for (const backend of ["canvas2d", "webgpu"]) {
  test(`${backend} compact position marker uses dedicated protection buttons and an attached close chip`, async ({ page }) => {
    await open_trading_demo(page, backend);
    const probe = await page.evaluate(() => {
      const trading = window.__chart.trading();
      trading.apply_snapshot({
        instrument: { tick_size: 0.25, price_precision: 2, quantity_precision: 0 },
        positions: [{ id: "position-only", side: "long", average_price: 100, quantity: 2 }],
      });
      window.__creation_intents = [];
      trading.subscribe_intents((intent) => window.__creation_intents.push(intent));
      const overlay = document.querySelector("#chart_container canvas:last-of-type").getBoundingClientRect();
      const width = window.__chart.time_scale().width();
      const left_x = width / 2;
      const entry_y = window.__main.price_to_coordinate(100);
      let take_profit_x = null;
      for (let x = 0; x <= width; x += 0.5) {
        if (trading.hit_at(x, entry_y)?.kind === "take_profit_button") {
          take_profit_x = x;
          break;
        }
      }
      return {
        overlay: { left: overlay.left, top: overlay.top },
        width,
        left_x,
        entry_y,
        take_profit_x,
        left_line: trading.hit_at(left_x, entry_y),
        marker: trading.hit_at(width - 200, entry_y),
        close_x: window.__close_x("position-only", entry_y),
        close: trading.hit_at(window.__close_x("position-only", entry_y), entry_y),
      };
    });
    expect(probe.left_line).toMatchObject({ id: "position-only", kind: "position_line" });
    expect(probe.marker).toMatchObject({ id: "position-only", kind: "position_line" });
    expect(probe.close).toMatchObject({ id: "position-only", kind: "cancel_button" });

    expect(probe.take_profit_x).not.toBeNull();
    await page.mouse.move(probe.overlay.left + probe.take_profit_x, probe.overlay.top + probe.entry_y);
    expect(await page.locator("#chart_container canvas:last-of-type").evaluate((canvas) => canvas.style.cursor)).toBe("pointer");
    await page.mouse.down();
    await page.mouse.move(
      probe.overlay.left + probe.take_profit_x,
      probe.overlay.top + probe.entry_y - 40,
      { steps: 6 },
    );
    await page.mouse.up();
    // Dragging a long position above its average price requests an attached take profit.
    const drag = await page.evaluate(() => ({
      intents: window.__creation_intents,
      preview: window.__chart.trading().preview(),
    }));
    expect(drag.preview).toBeNull();
    expect(drag.intents).toEqual([
      expect.objectContaining({
        action: "create_take_profit",
        position_id: "position-only",
        role: "take_profit",
        side: "sell",
      }),
    ]);
    await page.evaluate((sequence) => window.__chart.trading().resolve_intent(sequence, false), drag.intents[0].sequence);

    await page.mouse.click(probe.overlay.left + probe.close_x, probe.overlay.top + probe.entry_y);
    expect(await page.evaluate(() => window.__creation_intents.slice(1))).toEqual([
      expect.objectContaining({
        action: "close_position",
        position_id: "position-only",
      }),
    ]);
  });
}

test("execution marks sit outside their bar, stack same-bar fills, and read as clickable", async ({ page }) => {
  await page.goto("/?feature=executions&backend=canvas2d");
  await page.waitForFunction(() => window.__demo_catalogs?.lab.active_ids().includes("execution-marks"));
  // The scenario frames its fills after the first painted frames.
  await page.waitForFunction(() => window.__chart.timeScale().getVisibleLogicalRange().from > window.__data.length - 50);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const bars = window.__data;
    const probe = (back, id) => {
      const logical = bars.length - back;
      const bar = bars[logical];
      const x = chart.timeScale().logical_to_coordinate(logical);
      const high = window.__main.price_to_coordinate(bar.high);
      const low = window.__main.price_to_coordinate(bar.low);
      const ys = [];
      for (let y = 0; y < 2000; y += 1) {
        const hit = chart.trading_hit_at(x, y);
        if (hit?.object_type === "execution" && hit.id.startsWith(id)) ys.push(y);
      }
      return { high, low, top: Math.min(...ys), bottom: Math.max(...ys), id: chart.trading_hit_at(x, ys[0])?.id,
        cursor: chart.trading_cursor_at(x, ys[0]),
        inside: chart.trading_hit_at(x, (high + low) / 2) };
    };
    return { buy: probe(23, "multi-buy"), sell: probe(17, "multi-sell"), single: probe(34, "single-buy") };
  });
  // Buys answer only below the bar's low, sells only above its high; the candle body is free.
  expect(result.buy.top).toBeGreaterThan(result.buy.low);
  expect(result.sell.bottom).toBeLessThan(result.sell.high);
  expect(result.buy.inside).toBeNull();
  // A stacked mark answers with the bar's latest fill and is taller than a single arrow.
  expect(result.buy.id).toBe("multi-buy-2");
  expect(result.sell.id).toBe("multi-sell-3");
  expect(result.buy.bottom - result.buy.top).toBeGreaterThan(result.single.bottom - result.single.top);
  expect(result.buy.cursor).toBe("pointer");
});
