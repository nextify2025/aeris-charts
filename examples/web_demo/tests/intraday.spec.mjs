import { test, expect } from "@playwright/test";

// Intraday time-sharing (分时) recipe through the public API and the rendered frame
// (examples/web_demo/intraday.html): whole-session whitespace slots, a price line that stops at
// the last trade, 均价 = sum(amount) / sum(volume), a percentage axis centred on the previous close,
// exchange-time anchor ticks, a view that input cannot move, previous-close volume colors, and
// the five-day variant's day-open marks and per-day line runs (no connector across a trading-day
// boundary for the average or the price). The browser runs in New York so every exchange-time
// label must come from the chart's Asia/Shanghai zone, not the host's.

test.use({ timezoneId: "America/New_York" });

const RED = [247, 82, 95];
const GREEN = [8, 153, 129];

async function open_intraday(page, query = "") {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  // Record every axis string the Canvas2D executor paints.
  await page.addInitScript(() => {
    window.__texts = [];
    const original = CanvasRenderingContext2D.prototype.fillText;
    CanvasRenderingContext2D.prototype.fillText = function (text, ...args) {
      window.__texts.push(String(text));
      return original.call(this, text, ...args);
    };
  });
  await page.goto(`/intraday.html?backend=canvas2d&feed=off${query}`);
  await page.waitForFunction(() => window.__intraday !== undefined);
  await settle(page);
  return errors;
}

async function settle(page) {
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

/** Fresh axis text: clear the log, force a repaint, and return what was drawn. */
async function axis_texts(page) {
  await page.evaluate(() => {
    window.__texts.length = 0;
    window.__intraday.chart.apply_options({});
  });
  await settle(page);
  return page.evaluate(() => [...window.__texts]);
}

test("the whole session is in view and the price stops at the last traded minute", async ({ page }) => {
  const errors = await open_intraday(page);
  const result = await page.evaluate(() => {
    const { chart, price, slots, traded } = window.__intraday;
    const scale = chart.time_scale();
    const rows = price.data();
    const valued = rows.filter((row) => row.value !== undefined);
    const last_traded = slots[traded() - 1];
    const pane = chart.panes()[0].get_geometry();
    const shot = chart.take_screenshot(true, false);
    const ratio = shot.width / chart.chart_element().clientWidth;
    const pixels = shot.getContext("2d").getImageData(0, 0, shot.width, shot.height).data;
    const x_last = pane.left + scale.time_to_coordinate(last_traded);
    // Strongly colored price-line pixels left and right of the last traded minute.
    const line_pixels = (x_from, x_to) => {
      let count = 0;
      for (let y = Math.ceil(pane.top * ratio); y < Math.floor((pane.top + pane.height) * ratio); y += 1) {
        for (let x = Math.ceil(x_from * ratio); x < Math.floor(x_to * ratio); x += 1) {
          const i = (y * shot.width + x) * 4;
          const [r, g, b] = [pixels[i], pixels[i + 1], pixels[i + 2]];
          const near = (color) => Math.abs(r - color[0]) + Math.abs(g - color[1]) + Math.abs(b - color[2]) < 40;
          if (near([247, 82, 95]) || near([8, 153, 129])) count += 1;
        }
      }
      return count;
    };
    return {
      range: scale.get_visible_logical_range(),
      slot_count: slots.length,
      first_slot_time: scale.coordinate_to_time(scale.logical_to_coordinate(0)),
      last_slot_time: scale.coordinate_to_time(scale.logical_to_coordinate(slots.length - 1)),
      slots_first: slots[0],
      slots_last: slots.at(-1),
      rows: rows.length,
      valued: valued.length,
      traded: traded(),
      last_valued_time: valued.at(-1).time,
      last_traded,
      after_last: rows[traded()],
      left_pixels: line_pixels(pane.left + 2, x_last - 2),
      right_pixels: line_pixels(x_last + 6, pane.left + pane.width - 1),
    };
  });
  // 241 slots (09:30 plus 09:31..11:30 and 13:01..15:00), all of them in view from the open,
  // padded by half a bar so the edge columns stay whole.
  expect(result.slot_count).toBe(241);
  expect(result.range).toEqual({ from: -0.5, to: 240.5 });
  expect(result.first_slot_time).toBe(result.slots_first);
  expect(result.last_slot_time).toBe(result.slots_last);
  // Future minutes are whitespace: the series ends at the last trade.
  expect(result.rows).toBe(241);
  expect(result.valued).toBe(result.traded);
  expect(result.last_valued_time).toBe(result.last_traded);
  expect(result.after_last.value).toBeUndefined();
  expect(result.left_pixels).toBeGreaterThan(50);
  expect(result.right_pixels).toBe(0);
  expect(errors).toEqual([]);
});

test("the average price is turnover over volume at every sampled minute, live included", async ({ page }) => {
  await open_intraday(page);
  const result = await page.evaluate(() => {
    const { average, minutes, advance, traded } = window.__intraday;
    const check = (indices) => {
      const by_time = new Map(average.data().map((row) => [row.time, row.value]));
      return indices.map((index) => {
        let amount = 0;
        let volume = 0;
        for (let i = 0; i <= index; i += 1) {
          amount += minutes[i].amount;
          volume += minutes[i].volume;
        }
        return { index, chart: by_time.get(minutes[index].time), expected: amount / volume };
      });
    };
    const before = check([0, 1, 30, 60, 119, 120, 121, 149]);
    advance(5);
    const after = check([150, 154]);
    return { before, after, traded: traded() };
  });
  expect(result.traded).toBe(155);
  for (const { index, chart, expected } of [...result.before, ...result.after]) {
    expect(chart, `minute ${index}`).toBeCloseTo(expected, 9);
  }
});

test("the percentage axis is centred on the previous close and anchors read exchange time", async ({ page }) => {
  await open_intraday(page);
  const texts = await axis_texts(page);
  const result = await page.evaluate(() => {
    const { chart, price, reference_close } = window.__intraday;
    const pane = chart.panes()[0].get_geometry();
    return {
      reference_close,
      percent: chart.price_scale("right").get_visible_range(),
      price: chart.price_scale("left").get_visible_range(),
      percent_options: chart.price_scale("right").options(),
      reference_y: price.price_to_coordinate(reference_close),
      pane_height: pane.height,
    };
  });
  expect(result.percent_options.mode).toBe(2);
  expect(result.percent_options.base_value).toBe(result.reference_close);
  // Symmetric around 0% and around the previous close, which sits mid-pane.
  expect(result.percent.from).toBeCloseTo(-result.percent.to, 9);
  expect(result.percent.to).toBeGreaterThan(0);
  expect((result.price.from + result.price.to) / 2).toBeCloseTo(result.reference_close, 9);
  expect(Math.abs(result.reference_y - result.pane_height / 2)).toBeLessThan(1);
  // Explicit anchors replace the automatic hour ticks, in Shanghai time on a New York browser.
  const anchors = ["09:30", "10:30", "11:30/13:00", "14:00", "15:00"];
  for (const anchor of anchors) expect(texts).toContain(anchor);
  expect(texts).not.toContain("10:00");
  expect(texts).not.toContain("21:30");
  const junction = await page.evaluate(() => {
    const { chart, slots } = window.__intraday;
    const scale = chart.time_scale();
    const local = (time) => new Intl.DateTimeFormat("en-GB", {
      timeZone: "Asia/Shanghai", hour: "2-digit", minute: "2-digit", hourCycle: "h23",
    }).format(new Date(time * 1000));
    const morning_close = slots.findIndex((time) => local(time) === "11:30");
    return {
      morning_close,
      next: local(slots[morning_close + 1]),
      spacing: scale.logical_to_coordinate(morning_close + 1) - scale.logical_to_coordinate(morning_close),
      bar: scale.logical_to_coordinate(1) - scale.logical_to_coordinate(0),
    };
  });
  // The lunch break takes no width: the 11:30 slot and the afternoon's first slot are neighbours.
  expect(junction.morning_close).toBe(120);
  expect(junction.next).toBe("13:01");
  expect(junction.spacing).toBeCloseTo(junction.bar, 9);

  // Clearing the marks through the package restores the automatic tick selection.
  await page.evaluate(() => window.__intraday.chart.time_scale().apply_options({ tick_marks: null }));
  const automatic = await axis_texts(page);
  expect(automatic).toContain("10:00");
  expect(automatic).not.toContain("11:30/13:00");
  expect(await page.evaluate(() => window.__intraday.chart.time_scale().options().tick_marks)).toBeNull();
});

test("before the open every slot is whitespace, and the first trade is drawn at once", async ({ page }) => {
  const errors = await open_intraday(page, "&traded=0");
  const texts = await axis_texts(page);
  const before = await page.evaluate(() => {
    const { chart, price, volume } = window.__intraday;
    return {
      range: chart.time_scale().get_visible_logical_range(),
      rows: price.data().length,
      valued: price.data().filter((row) => row.value !== undefined).length,
      volumes: volume.data().filter((row) => row.value !== undefined).length,
      clock: document.getElementById("last_time").textContent,
    };
  });
  // The session is reserved and its anchors drawn before anything trades.
  expect(before).toEqual({ range: { from: -0.5, to: 240.5 }, rows: 241, valued: 0, volumes: 0, clock: "待开盘 Pre-open" });
  for (const anchor of ["09:30", "10:30", "11:30/13:00", "14:00", "15:00"]) expect(texts).toContain(anchor);

  /** Price-pane and volume-pane pixels around the first slot, at the current size. */
  const first_slot = () => page.evaluate(() => {
    const { chart, slots } = window.__intraday;
    const shot = chart.take_screenshot(true, false);
    const ratio = shot.width / chart.chart_element().clientWidth;
    const pixels = shot.getContext("2d").getImageData(0, 0, shot.width, shot.height).data;
    const [price_pane, volume_pane] = chart.panes().map((pane) => pane.get_geometry());
    const x = price_pane.left + chart.time_scale().time_to_coordinate(slots[0]);
    const spacing = chart.time_scale().logical_to_coordinate(1) - chart.time_scale().logical_to_coordinate(0);
    const colors = (pane, y_from, y_to) => {
      const found = new Set();
      for (let y = Math.ceil(y_from * ratio); y < Math.floor(y_to * ratio); y += 1) {
        for (let dx = Math.floor((x - spacing) * ratio); dx <= Math.ceil((x + spacing) * ratio); dx += 1) {
          if (dx < Math.ceil(pane.left * ratio) || dx >= Math.floor((pane.left + pane.width) * ratio)) continue;
          const i = (y * shot.width + dx) * 4;
          const near = (color) => Math.abs(pixels[i] - color[0]) + Math.abs(pixels[i + 1] - color[1]) + Math.abs(pixels[i + 2] - color[2]) < 40;
          if (near([247, 82, 95])) found.add("red");
          if (near([8, 153, 129])) found.add("green");
        }
      }
      return [...found];
    };
    return {
      x_inside: x > price_pane.left && x < price_pane.left + price_pane.width,
      price: colors(price_pane, price_pane.top, price_pane.top + price_pane.height),
      // The column grows from zero volume, so the pane's middle row crosses it.
      volume: colors(volume_pane, volume_pane.top + volume_pane.height * 0.45, volume_pane.top + volume_pane.height * 0.55),
    };
  });
  expect((await first_slot()).price).toEqual([]);

  // The opening trade fills the 09:30 slot: one traded row still draws a bar-wide price segment
  // and a volume column, both in the color of its side of the previous close.
  const opened = await page.evaluate(() => {
    const { price, step, reference_close } = window.__intraday;
    step();
    const valued = price.data().filter((row) => row.value !== undefined);
    return { valued: valued.length, side: valued[0].value >= reference_close ? "red" : "green" };
  });
  await settle(page);
  expect(opened.valued).toBe(1);
  const wide = await first_slot();
  expect(wide.x_inside).toBe(true);
  expect(wide.price).toEqual([opened.side]);
  expect(wide.volume).toEqual([opened.side]);

  // A phone-width chart keeps the opening column and segment inside the pane.
  await page.setViewportSize({ width: 390, height: 760 });
  await settle(page);
  await settle(page);
  const narrow = await first_slot();
  expect(narrow.x_inside).toBe(true);
  expect(narrow.price).toEqual([opened.side]);
  expect(narrow.volume).toEqual([opened.side]);
  expect(await page.evaluate(() => window.__intraday.chart.time_scale().get_visible_logical_range())).toEqual({ from: -0.5, to: 240.5 });
  expect(errors).toEqual([]);
});

test("the session view stays fixed after resize, wheel, drag, and keyboard input", async ({ page }) => {
  await open_intraday(page);
  const full = { from: -0.5, to: 240.5 };
  const range = () => page.evaluate(() => window.__intraday.chart.time_scale().get_visible_logical_range());
  expect(await range()).toEqual(full);

  await page.setViewportSize({ width: 760, height: 560 });
  await settle(page);
  await settle(page);
  expect(await range()).toEqual(full);

  const box = await page.locator("#chart").boundingBox();
  const center = { x: box.x + box.width / 2, y: box.y + box.height * 0.35 };
  await page.mouse.move(center.x, center.y);
  await page.mouse.wheel(0, -600);
  await page.mouse.wheel(240, 0);
  await settle(page);
  expect(await range()).toEqual(full);

  await page.mouse.move(center.x, center.y);
  await page.mouse.down();
  await page.mouse.move(center.x - 220, center.y, { steps: 8 });
  await page.mouse.up();
  await settle(page);
  expect(await range()).toEqual(full);

  await page.evaluate(() => window.__intraday.chart.accessibility().focus(0));
  for (const key of ["+", "-", "ArrowLeft", "ArrowRight", "Home", "End"]) await page.keyboard.press(key);
  await page.evaluate(() => window.__intraday.chart.accessibility().focus_target("time-axis"));
  for (const key of ["Home", "ArrowLeft", "ArrowRight"]) await page.keyboard.press(key);
  await settle(page);
  expect(await range()).toEqual(full);

  // Live minutes fill future slots without moving the view.
  await page.evaluate(() => window.__intraday.advance(20));
  await settle(page);
  expect(await range()).toEqual(full);
});

test("volume columns follow the previous-close rule in red-up/green-down colors", async ({ page }) => {
  await open_intraday(page);
  const sample = async () => page.evaluate(() => {
    const { chart, volume, minutes, slots, traded, reference_close } = window.__intraday;
    const scale = chart.time_scale();
    const pane = chart.panes()[1].get_geometry();
    const shot = chart.take_screenshot(true, false);
    const ratio = shot.width / chart.chart_element().clientWidth;
    const context = shot.getContext("2d");
    const out = [];
    for (let index = 0; index < traded(); index += 7) {
      const x = pane.left + scale.time_to_coordinate(slots[index]);
      // Series coordinates are chart-content y, so the lower pane needs no offset.
      const y = volume.price_to_coordinate(minutes[index].volume / 2);
      const [r, g, b] = context.getImageData(Math.round(x * ratio), Math.round(y * ratio), 1, 1).data;
      const previous = index === 0 ? reference_close : minutes[index - 1].price;
      out.push({ index, rgb: [r, g, b], up: minutes[index].price >= previous });
    }
    return out;
  });
  const classify = ([r, g, b]) => {
    const distance = (color) => Math.abs(r - color[0]) + Math.abs(g - color[1]) + Math.abs(b - color[2]);
    return distance(RED) < 30 ? "up" : distance(GREEN) < 30 ? "down" : `other ${r},${g},${b}`;
  };
  const check = (samples) => {
    expect(samples.some((sample) => sample.up)).toBe(true);
    expect(samples.some((sample) => !sample.up)).toBe(true);
    for (const { index, rgb, up } of samples) expect(classify(rgb), `minute ${index}`).toBe(up ? "up" : "down");
  };
  check(await sample());
  // Minutes delivered by the feed color the same way.
  await page.evaluate(() => window.__intraday.advance(40));
  await settle(page);
  check(await sample());
});

test("sequenced live updates fill future minutes and reject stale deliveries", async ({ page }) => {
  await open_intraday(page);
  const result = await page.evaluate(() => {
    const { price, step, traded, forming, minutes } = window.__intraday;
    const before = traded();
    step();
    const opened = { traded: traded(), forming: forming(), value: price.data()[before].value };
    step();
    const closed = { forming: forming(), value: price.data()[before].value };
    // A late replay with an old sequence is rejected without touching the bar.
    price.update({ time: minutes[before].time, value: 1 }, { sequence: 1 });
    return {
      before,
      opened,
      closed,
      final: minutes[before].price,
      stale: price.last_ingestion_diagnostics()?.code,
      unchanged: price.data()[before].value,
    };
  });
  expect(result.opened.traded).toBe(result.before + 1);
  expect(result.opened.forming).toBe(true);
  expect(result.closed.forming).toBe(false);
  expect(result.closed.value).toBe(result.final);
  expect(result.stale).toBe("stale_sequence");
  expect(result.unchanged).toBe(result.final);
});

test("the five-day variant labels each day's open and resets the average every session", async ({ page }) => {
  const errors = await open_intraday(page, "&days=5");
  const texts = await axis_texts(page);
  const result = await page.evaluate(() => {
    const { chart, average, minutes, days, slots } = window.__intraday;
    const by_time = new Map(average.data().map((row) => [row.time, row.value]));
    return {
      range: chart.time_scale().get_visible_logical_range(),
      slots: slots.length,
      days: days.map((day) => day.date),
      // The first minute of each day averages that minute alone.
      openings: days.map((day) => {
        const minute = minutes.find((row) => row.time === day.first);
        return { chart: by_time.get(day.first), expected: minute.amount / minute.volume };
      }),
      tick_marks: chart.time_scale().options().tick_marks,
    };
  });
  expect(result.slots).toBe(5 * 241);
  expect(result.range).toEqual({ from: -0.5, to: 5 * 241 - 0.5 });
  expect(result.days).toEqual(["2026-09-21", "2026-09-22", "2026-09-23", "2026-09-24", "2026-09-25"]);
  for (const label of ["09-21", "09-22", "09-23", "09-24", "09-25"]) expect(texts).toContain(label);
  expect(result.tick_marks.map((mark) => mark.label)).toEqual(["09-21", "09-22", "09-23", "09-24", "09-25"]);
  for (const { chart, expected } of result.openings) expect(chart).toBeCloseTo(expected, 9);
  expect(errors).toEqual([]);
});

/** Record every path the Canvas2D executor strokes (subpaths in bitmap px) while recording is on. */
async function record_canvas_strokes(page) {
  await page.addInitScript(() => {
    window.__strokes = null;
    const proto = CanvasRenderingContext2D.prototype;
    const { beginPath, moveTo, lineTo, stroke } = proto;
    let path = [];
    proto.beginPath = function (...args) {
      path = [];
      return beginPath.apply(this, args);
    };
    proto.moveTo = function (x, y) {
      path.push([[x, y]]);
      return moveTo.call(this, x, y);
    };
    proto.lineTo = function (x, y) {
      if (path.length === 0) path.push([]);
      path.at(-1).push([x, y]);
      return lineTo.call(this, x, y);
    };
    proto.stroke = function (...args) {
      if (window.__strokes !== null) {
        window.__strokes.push({ color: this.strokeStyle, subpaths: path.map((subpath) => subpath.slice()) });
      }
      return stroke.apply(this, args);
    };
  });
}

/**
 * Stroked runs of the price pane in `colors`, each mapped to logical slots through the frame's
 * device-pixel transform and to the trading day it lies in (`inside` is false when a run joins
 * two days). A lone row's one-bar segment spans half a slot to each side of its row.
 */
async function day_runs(page, colors) {
  await page.evaluate(() => {
    window.__strokes = [];
    window.__intraday.chart.apply_options({});
  });
  await settle(page);
  const { strokes, layout } = await page.evaluate(() => {
    const { chart, days, slots } = window.__intraday;
    const pane = chart.panes()[0].get_geometry();
    const scale = chart.time_scale();
    const recorded = window.__strokes;
    window.__strokes = null;
    return {
      strokes: recorded,
      layout: {
        dpr: window.devicePixelRatio,
        left: pane.left,
        width: pane.width,
        slot_zero: scale.logical_to_coordinate(0),
        spacing: scale.logical_to_coordinate(1) - scale.logical_to_coordinate(0),
        per_day: slots.length / days.length,
      },
    };
  });
  // The engine's pane bitmap ratio and pane offset (frame/mod.rs).
  const hpr = Math.max(1, Math.round(layout.width * layout.dpr)) / Math.max(1, layout.width);
  const left = Math.round(layout.left * layout.dpr);
  const logical = (x) => ((x - left) / hpr - layout.slot_zero) / layout.spacing;
  return strokes
    .filter((stroke) => colors.includes(stroke.color))
    .flatMap((stroke) => stroke.subpaths)
    .filter((path) => path.length >= 2)
    .map((path) => {
      const logicals = path.map(([x]) => logical(x));
      const middle = (logicals[0] + logicals.at(-1)) / 2;
      const day = Math.floor((middle + 0.5) / layout.per_day);
      const start = day * layout.per_day - 0.5;
      const inside = logicals.every((value) => value >= start - 0.05 && value <= start + layout.per_day + 0.05);
      return { day, inside, logicals };
    });
}

test("the five-day lines restart every trading day with no connector, live minutes included", async ({ page }) => {
  await record_canvas_strokes(page);
  const errors = await open_intraday(page, "&days=5&traded=0");
  const AVERAGE = "#f59e0a";
  const PRICE = ["#f7525f", "#089981"];
  const days = (runs) => [...new Set(runs.map((run) => run.day))].sort();

  // Four traded days before the fifth opens: the average draws one run per day, and the price
  // (red above / green below the previous close) never joins two days either.
  const average = await day_runs(page, [AVERAGE]);
  expect(average.map((run) => run.day)).toEqual([0, 1, 2, 3]);
  for (const run of average) {
    expect(run.inside, `average run of day ${run.day}`).toBe(true);
    // Every vertex sits on a slot: the transform above is the frame's own.
    for (const value of run.logicals) expect(Math.abs(value - Math.round(value))).toBeLessThan(0.05);
  }
  const price = await day_runs(page, PRICE);
  expect(days(price)).toEqual([0, 1, 2, 3]);
  for (const run of price) expect(run.inside, `price run of day ${run.day}`).toBe(true);

  // The fifth day's opening trade draws its own one-bar segments instead of a diagonal from
  // yesterday's last average and price.
  const first = await page.evaluate(() => {
    window.__intraday.step();
    return window.__intraday.slots.length - window.__intraday.slots.length / 5;
  });
  await settle(page);
  const opened = await day_runs(page, [AVERAGE]);
  expect(opened.map((run) => run.day)).toEqual([0, 1, 2, 3, 4]);
  for (const run of opened) expect(run.inside, `average run of day ${run.day}`).toBe(true);
  expect(opened[4].logicals.map((value) => Number(value.toFixed(2)))).toEqual([first - 0.5, first + 0.5]);
  const opened_price = await day_runs(page, PRICE);
  expect(days(opened_price)).toEqual([0, 1, 2, 3, 4]);
  for (const run of opened_price) expect(run.inside, `price run of day ${run.day}`).toBe(true);

  // Later minutes extend the fifth day's runs without reaching back into the fourth.
  await page.evaluate(() => window.__intraday.advance(30));
  await settle(page);
  const live = await day_runs(page, [AVERAGE]);
  expect(live.map((run) => run.day)).toEqual([0, 1, 2, 3, 4]);
  expect(live[4].logicals[0]).toBeCloseTo(first, 1);
  for (const run of live) expect(run.inside, `average run of day ${run.day}`).toBe(true);
  for (const run of await day_runs(page, PRICE)) expect(run.inside, `price run of day ${run.day}`).toBe(true);

  // Control: without the option the price line joins the days again, and the check sees it.
  await page.evaluate(() => {
    for (const series of [window.__intraday.price, window.__intraday.percent]) {
      series.apply_options({ break_on_trading_day: false });
    }
  });
  expect(await page.evaluate(() => window.__intraday.price.options().break_on_trading_day)).toBe(false);
  const joined = await day_runs(page, PRICE);
  expect(joined.some((run) => !run.inside)).toBe(true);
  expect(errors).toEqual([]);
});

test("the one-day variant keeps a single continuous price and average line", async ({ page }) => {
  await record_canvas_strokes(page);
  const errors = await open_intraday(page);
  const average = await day_runs(page, ["#f59e0a"]);
  expect(average).toHaveLength(1);
  expect(average[0].logicals.length).toBeGreaterThan(100);
  expect(errors).toEqual([]);
});

test("session slots, explicit marks, and KDJ seeds validate through the package", async ({ page }) => {
  await open_intraday(page);
  const result = await page.evaluate(async () => {
    const { session_slot_times, create_chart } = await import("/dist/aeris_charts_financial.js");
    const code = (run) => {
      try {
        run();
        return null;
      } catch (error) {
        return error.code ?? String(error);
      }
    };
    const shanghai = session_slot_times({
      date: { year: 2026, month: 9, day: 25 },
      windows: [["09:30", "11:30"], ["13:00", "15:00"]],
      interval_seconds: 60,
      time_zone: "Asia/Shanghai",
    });
    // New York regular hours on both sides of the March 2024 DST change.
    const new_york = ["2024-03-08", "2024-03-11"].map((date) => session_slot_times({
      date,
      windows: [["09:30", "16:00"]],
      interval_seconds: 60,
      time_zone: "America/New_York",
    }));
    const { chart } = window.__intraday;
    const scale = chart.time_scale();
    const before = scale.options().tick_marks;
    const rejected = code(() => scale.apply_options({
      tick_marks: [{ time: 2 }, { time: 1 }],
      time_visible: false,
    }));
    const host = document.createElement("div");
    host.style.cssText = "position:fixed;left:0;top:0;width:600px;height:400px";
    document.body.append(host);
    const other = await create_chart(host, { autoSize: false, backend: "canvas2d" });
    const bars = other.add_series("candlestick");
    bars.set_data(Array.from({ length: 40 }, (_, i) => ({
      time: 1_700_000_000 + i * 86_400, open: 10 + Math.sin(i), high: 11 + Math.sin(i), low: 9 + Math.sin(i), close: 10 + Math.cos(i),
    })));
    const [textbook] = other.add_kdj(bars);
    const [first_value] = other.add_kdj(bars, 9, 3, 3, undefined, { convention: "china" });
    const kdj = {
      textbook: textbook.indicator_info().parameters.kdj_seed,
      first_value: first_value.indicator_info().parameters.kdj_seed,
      bad: code(() => other.add_kdj(bars, 9, 3, 3, undefined, { seed: "zero" })),
    };
    other.remove();
    host.remove();
    return {
      shanghai: [shanghai.length, shanghai[0] % 86_400],
      new_york: new_york.map((slots) => [slots.length, slots[0] % 86_400]),
      bad_date: code(() => session_slot_times({ date: "2026-02-30", windows: [["09:30", "11:30"]], interval_seconds: 60 })),
      bad_window: code(() => session_slot_times({ date: "2026-09-25", windows: [["13:00", "15:00"], ["09:30", "11:30"]], interval_seconds: 60 })),
      rejected,
      time_visible: scale.options().time_visible,
      unchanged: JSON.stringify(scale.options().tick_marks) === JSON.stringify(before),
      kdj,
    };
  });
  expect(result.shanghai).toEqual([240, 3_600 + 1_800]);
  expect(result.new_york).toEqual([[390, 14 * 3_600 + 1_800], [390, 13 * 3_600 + 1_800]]);
  expect(result.bad_date).toBe("invalid_options");
  expect(result.bad_window).toBe("invalid_options");
  // A rejected mark list changes nothing, including the other keys of the same call.
  expect(result.rejected).toBe("invalid_options");
  expect(result.time_visible).toBe(true);
  expect(result.unchanged).toBe(true);
  expect(result.kdj).toEqual({ textbook: "fifty", first_value: "first_value", bad: "invalid_options" });
});

test("an autoscale provider that calls the chart fails safely on every render path", async ({ page }) => {
  const errors = await open_intraday(page);
  const warnings = [];
  page.on("console", (message) => {
    if (message.type() === "warning") warnings.push(message.text());
  });
  const result = await page.evaluate(async () => {
    const { create_chart } = await import("/dist/aeris_charts_financial.js");
    const frames = () => new Promise((resolve) => {
      requestAnimationFrame(() => requestAnimationFrame(resolve));
    });
    const host = document.createElement("div");
    host.style.cssText = "position:fixed;left:0;top:0;width:640px;height:360px";
    document.body.append(host);
    // Auto-size: resizes render from the engine's own ResizeObserver callback.
    const chart = await create_chart(host, { autoSize: true, backend: "canvas2d" });
    const series = chart.add_series("line");
    const t0 = 1_700_000_000;
    series.set_data(Array.from({ length: 60 }, (_, i) => ({ time: t0 + i * 60, value: 100 + Math.sin(i / 5) })));
    const seen = [];
    series.apply_options({
      autoscale_info_provider: (base) => {
        try {
          chart.time_scale().get_visible_logical_range();
          seen.push("re-entered");
        } catch (error) {
          seen.push(error.code ?? String(error));
        }
        return base();
      },
    });
    chart.time_scale().fit_content();
    await frames();
    host.style.width = "520px";
    host.style.height = "300px";
    await frames();
    await frames();
    // A provider that lets the error escape is ignored for that pass.
    series.apply_options({
      autoscale_info_provider: () => {
        chart.price_scale("right").get_visible_range();
        return { price_range: { min_value: -1000, max_value: 1000 } };
      },
    });
    chart.time_scale().fit_content();
    await frames();
    const escaped_range = chart.price_scale("right").get_visible_range();
    // The chart stays fully usable afterwards.
    series.apply_options({ autoscale_info_provider: null });
    series.update({ time: t0 + 60 * 60, value: 105 });
    await frames();
    const after = {
      rows: series.data().length,
      range: chart.time_scale().get_visible_logical_range() !== null,
      width: chart.time_scale().width(),
    };
    chart.remove();
    host.remove();
    return { seen, escaped_range, after };
  });
  expect(result.seen.length).toBeGreaterThan(1);
  expect(new Set(result.seen)).toEqual(new Set(["unsupported_operation"]));
  expect(result.escaped_range.to).toBeLessThan(1000);
  expect(result.after.rows).toBe(61);
  expect(result.after.range).toBe(true);
  expect(result.after.width).toBeGreaterThan(300);
  expect(warnings.some((text) => text.includes("autoscale_info_provider"))).toBe(true);
  expect(errors).toEqual([]);
});

// Open-stamped minute candles keep their open identity and print their close time when the chart
// option `bar_time_label` asks for it: the crosshair reads 09:31 on the first candle (opened 09:30)
// and 15:00 on the last (opened 14:59), on both backends. Every time the API reports stays the open.
for (const backend of ["canvas2d", "webgpu"]) {
  test(`minute candles with bar_open slots print their close time (${backend})`, async ({ page }) => {
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.addInitScript(() => {
      window.__texts = [];
      const original = CanvasRenderingContext2D.prototype.fillText;
      CanvasRenderingContext2D.prototype.fillText = function (text, ...args) {
        window.__texts.push(String(text));
        return original.call(this, text, ...args);
      };
    });
    await page.goto(`/?runtimeTest=presentedFrame&backend=${backend}&forceFallbackAdapter=1`);
    await page.waitForFunction((expected) => window.__chart?.backend?.() === expected, backend);
    const windows = [["09:30", "11:30"], ["13:00", "15:00"]];
    const setup = await page.evaluate(async (windows) => {
      const { session_slot_times } = await import("/dist/aeris_charts_financial.js");
      const chart = window.__chart;
      chart.remove_series(window.__main);
      const slots = session_slot_times({
        date: "2026-09-25", windows, interval_seconds: 60, time_zone: "Asia/Shanghai", convention: "bar_open",
      });
      const candles = chart.add_series("candlestick");
      candles.set_data(slots.map((time, index) => ({
        time, open: 10 + index / 1000, high: 10.05 + index / 1000, low: 9.95 + index / 1000, close: 10.01 + index / 1000,
      })));
      chart.apply_options({
        timeScale: {
          timeZone: "Asia/Shanghai", timeVisible: true, lockVisibleLogicalRange: true,
          barTimeLabel: { anchor: "close", interval_seconds: 60, windows },
        },
      });
      chart.time_scale().set_visible_logical_range({ from: -0.5, to: slots.length - 0.5 });
      window.__candles = candles;
      window.__slots = slots;
      return { count: slots.length, first: slots[0], last: slots.at(-1), options: chart.time_scale().options() };
    }, windows);
    expect(setup.count).toBe(240);
    expect(setup.options.bar_time_label).toEqual({ anchor: "close", interval_seconds: 60, windows });
    await settle(page);

    // Crosshair label of the first and last slot: the text the executor paints.
    const label_at = async (index) => {
      await page.evaluate((slot) => {
        window.__texts.length = 0;
        window.__chart.set_crosshair_position(10, slot, window.__candles);
      }, setup[index === 0 ? "first" : "last"]);
      await settle(page);
      return page.evaluate(() => window.__texts.find((text) => /^\d\d \w{3} '\d\d\s+\d\d:\d\d$/.test(text)) ?? null);
    };
    expect(await label_at(0)).toMatch(/09:31$/);
    expect(await label_at(239)).toMatch(/15:00$/);

    // Identity: the rows, the visible range, and the coordinate mapping are still open-stamped.
    const identity = await page.evaluate(() => {
      const scale = window.__chart.time_scale();
      const rows = window.__candles.data();
      return {
        first: rows[0].time,
        last: rows.at(-1).time,
        at_first: scale.coordinate_to_time(scale.logical_to_coordinate(0)),
        at_last: scale.coordinate_to_time(scale.logical_to_coordinate(239)),
        label_only_instant: scale.time_to_coordinate(window.__slots[119] + 60) ?? null,
      };
    });
    expect(identity.first).toBe(setup.first);
    expect(identity.last).toBe(setup.last);
    expect(identity.at_first).toBe(setup.first);
    expect(identity.at_last).toBe(setup.last);
    // 11:30 is the close of the 11:29 bar, not a bar: it has no slot to point at.
    expect(identity.label_only_instant).toBeNull();

    // Back to the open text.
    await page.evaluate(() => window.__chart.apply_options({ timeScale: { barTimeLabel: "open" } }));
    await settle(page);
    expect(await label_at(0)).toMatch(/09:30$/);
    expect(errors).toEqual([]);
  });
}

// The close-time label windows and the session start must fit each other. A `session_start` the
// installed windows cannot be placed on is rejected with `invalid_options` through both option
// entry points and changes nothing; one call that moves the start and replaces the label is judged
// against the label it installs.
test("a session_start that the close-label windows do not fit is rejected atomically", async ({ page }) => {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d&forceFallbackAdapter=1");
  await page.waitForFunction(() => window.__chart?.backend?.() === "canvas2d");
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const scale = chart.time_scale();
    const windows = [["09:00", "10:00"], ["20:00", "21:00"]];
    const label = { anchor: "close", interval_seconds: 2_700, windows };
    chart.apply_options({ timeScale: { timeZone: "UTC", sessionStart: 0, barTimeLabel: label } });
    const attempt = (run) => {
      try {
        run();
        return null;
      } catch (error) {
        return { code: error.code, message: String(error.message) };
      }
    };
    const snapshot = () => {
      const options = scale.options();
      return { session_start: options.session_start, label: options.bar_time_label };
    };
    const before = snapshot();
    // Windows [09:00-10:00, 20:00-21:00] are unordered when the trading day starts at 12:00.
    const rejected = [
      attempt(() => scale.apply_options({ session_start: 43_200 })),
      attempt(() => chart.apply_options({ timeScale: { sessionStart: 43_200 } })),
    ];
    const unchanged = snapshot();
    // One call that also replaces the windows is judged against the label it installs.
    const moved_windows = [["20:00", "21:00"], ["09:00", "10:00"]];
    const both = attempt(() => scale.apply_options({
      session_start: 43_200,
      bar_time_label: { anchor: "close", interval_seconds: 2_700, windows: moved_windows },
    }));
    const moved = snapshot();
    // Clearing the label first lets the start move on its own.
    const cleared = attempt(() => scale.apply_options({ bar_time_label: "open", session_start: 0 }));
    return { before, rejected, unchanged, both, moved, cleared, after: snapshot(), label, moved_windows };
  });
  for (const rejection of result.rejected) {
    expect(rejection.code).toBe("invalid_options");
    expect(rejection.message).toContain("barTimeLabel");
  }
  expect(result.before).toEqual({ session_start: 0, label: result.label });
  expect(result.unchanged).toEqual(result.before);
  expect(result.both).toBeNull();
  expect(result.moved.session_start).toBe(43_200);
  expect(result.moved.label.windows).toEqual(result.moved_windows);
  expect(result.cleared).toBeNull();
  expect(result.after).toEqual({ session_start: 0, label: "open" });
  expect(errors).toEqual([]);
});
