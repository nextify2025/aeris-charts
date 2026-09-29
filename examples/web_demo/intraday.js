// Intraday time-sharing (分时) chart: one A-share trading day, or five with day separators,
// composed from public Aeris APIs only. Market data is generated deterministically (no network).
//
// Recipe (docs/Public_api.md, "Intraday (分时) charts"):
//   1. `session_slot_times` reserves every minute of the session as a whitespace slot.
//   2. A baseline series relative to the previous close (red above, green below) carries price;
//      the left axis is autoscaled symmetrically around the previous close and a mirror series
//      puts the same prices on a right percentage axis based on the previous close.
//   3. The average price (均价) is VWAP with a turnover (amount) source, reset per session; its
//      line restarts at each reset. The five-day price lines also break at every trading day.
//   4. The volume histogram colors each minute against the previous minute's close.
//   5. Explicit time-axis marks 09:30/10:30/11:30|13:00/14:00/15:00 (day opens for five days).
//   6. The whole session is held in view: `lock_visible_logical_range` plus disabled scroll/scale.
//   7. A simulated live feed fills future minutes with sequenced update/merge calls.

import { create_chart, init_wasm, session_slot_times } from "./dist/aeris_charts_financial.js";

// Session slots come from the engine, so instantiate it before building the data.
await init_wasm();

const query = new URLSearchParams(location.search);
const DAYS = query.get("days") === "5" ? 5 : 1;
const TIME_ZONE = "Asia/Shanghai";
const SESSION = [["09:30", "11:30"], ["13:00", "15:00"]];
// Trading dates are host-owned calendar data; this demo uses one fixed Monday-Friday week.
const DATES = ["2026-09-21", "2026-09-22", "2026-09-23", "2026-09-24", "2026-09-25"].slice(-DAYS);
const FIRST_PREV_CLOSE = 10;
// The first 150 points of the last day have traded (through 13:29); the rest are future slots.
// `?traded=0` opens before the first trade, with every slot of the day still whitespace.
const requested_traded = Number.parseInt(query.get("traded") ?? "", 10);
const TRADED = Number.isInteger(requested_traded) ? Math.max(0, Math.min(241, requested_traded)) : 150;
const FEED_MS = 350;
// Canonical Aeris market palette with the A-share convention: red up, green down.
const RED = "#f7525f";
const GREEN = "#089981";
const AVERAGE = "#f59e0a";

const theme = query.get("theme") ?? (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
document.documentElement.dataset.theme = theme;
for (const link of document.querySelectorAll("[data-days]")) {
  link.setAttribute("aria-current", String(Number(link.dataset.days) === DAYS));
}

// ---------------------------------------------------------------------------------------------
// Deterministic synthetic market data
// ---------------------------------------------------------------------------------------------

function random(seed) {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const cents = (value) => Math.round(value * 100) / 100;

/** 241 minutes of one day: price, share volume, and turnover (amount) per slot. */
function generate_day(slots, prev_close, seed) {
  const next = random(seed);
  const minutes = [];
  let price = cents(prev_close * (1 + (next() - 0.45) * 0.012));
  for (let index = 0; index < slots.length; index += 1) {
    const previous = price;
    if (index > 0) price = cents(Math.max(0.01, price * (1 + (next() - 0.5) * 0.0035)));
    // U-shaped intraday volume profile: busy open and close, quiet midday.
    const edge = Math.min(index, slots.length - 1 - index) / (slots.length / 2);
    const lots = Math.round((index === 0 ? 900 : 120) * (1.6 - edge) * (0.4 + next()));
    const volume = Math.max(1, lots) * 100;
    // Turnover at the minute's traded prices, so the average stays inside the price range.
    const amount = cents(volume * (previous + price) / 2);
    minutes.push({ time: slots[index], price, volume, amount });
  }
  return minutes;
}

const days = [];
let prev_close = FIRST_PREV_CLOSE;
for (const [day_index, date] of DATES.entries()) {
  const slots = session_slot_times({
    date,
    windows: SESSION,
    interval_seconds: 60,
    time_zone: TIME_ZONE,
    convention: "bar_close_with_open",
  });
  const minutes = generate_day(slots, prev_close, 20260921 + day_index);
  days.push({ date, slots, minutes, prev_close });
  prev_close = minutes[minutes.length - 1].price;
}
const today = days[days.length - 1];
// The chart is relative to the close before its first day.
const reference_close = days[0].prev_close;
const slots = days.flatMap((day) => day.slots);
const minutes = days.flatMap((day) => day.minutes);
const live_from = slots.length - today.slots.length;
let traded = live_from + TRADED;

// ---------------------------------------------------------------------------------------------
// Chart
// ---------------------------------------------------------------------------------------------

const container = document.getElementById("chart");
const chart = await create_chart(container, {
  autoSize: true,
  theme,
  backend: query.get("backend") === "canvas2d" ? "canvas2d" : "auto",
  // The whole session stays in view: no pan/zoom gestures (the keyboard follows these switches).
  handle_scroll: false,
  handle_scale: false,
  leftPriceScale: { visible: true },
  rightPriceScale: { visible: true },
  grid: { vertLines: { visible: true }, horzLines: { visible: true } },
});

const margins = { top: 0.08, bottom: 0.08 };
// Several days: each day's price line starts fresh instead of joining the previous day's close.
const break_on_trading_day = DAYS > 1;
// Price: a baseline series against the previous close (red above, green below), on the left.
const price = chart.add_series("baseline", {
  price_scale_id: "left",
  baseline_value: reference_close,
  break_on_trading_day,
  line_width: 1.5,
  top_line_color: RED,
  top_fill_color1: "rgba(247, 82, 95, 0.18)",
  top_fill_color2: "rgba(247, 82, 95, 0.02)",
  bottom_line_color: GREEN,
  bottom_fill_color1: "rgba(8, 153, 129, 0.02)",
  bottom_fill_color2: "rgba(8, 153, 129, 0.18)",
  price_line_visible: false,
  countdown_visible: false,
  crosshair_marker_visible: true,
  price_format: { type: "price", min_move: 0.01 },
});
// The same prices on the right axis as the change from the previous close.
const percent = chart.add_series("line", {
  price_scale_id: "right",
  color: "#787b86",
  line_visible: false,
  break_on_trading_day,
  price_line_visible: false,
  countdown_visible: false,
  crosshair_marker_visible: false,
});
// Volume (shares) and turnover (yuan) are independent series; turnover only feeds the average.
const volume = chart.add_series("histogram", {
  pane: 1,
  pane_stretch: 0.35,
  price_format: { type: "volume" },
  histogram_updown: true,
  histogram_updown_rule: "previous_close",
  up_color: RED,
  down_color: GREEN,
  last_value_visible: false,
  price_line_visible: false,
  countdown_visible: false,
});
const amount = chart.add_series("line", { pane: 1, visible: false, countdown_visible: false });

chart.price_scale("left").apply_options({ autoscale_center: reference_close, scale_margins: margins });
chart.price_scale("right").apply_options({
  mode: 2,
  base_value: reference_close,
  autoscale_center: reference_close,
  scale_margins: margins,
});

const row = (minute, index, value) => (index < traded ? { time: minute.time, value } : { time: minute.time });
price.set_data(minutes.map((minute, index) => row(minute, index, minute.price)));
percent.set_data(minutes.map((minute, index) => row(minute, index, minute.price)));
volume.set_data(minutes.map((minute, index) => row(minute, index, minute.volume)));
amount.set_data(minutes.map((minute, index) => row(minute, index, minute.amount)));

// 均价: sum(amount) / sum(volume), reset each trading session; each reset starts a new line.
const average = chart.add_vwap(price, volume, {
  price_scale_id: "left",
  color: AVERAGE,
  line_width: 1.5,
  title: "均价",
  countdown_visible: false,
  price_line_visible: false,
}, { amount_source: amount });

// The previous close (昨收), labelled with its price on the left axis.
price.create_price_line({
  price: reference_close,
  color: "#9598a1",
  line_width: 1,
  line_style: "dashed",
});

// Exchange-time axis with explicit anchors: the session's hours, or each day's open.
const at = (day, hh_mm) => day.slots.find((slot) => {
  const local = new Date((slot + 8 * 3600) * 1000);
  return `${String(local.getUTCHours()).padStart(2, "0")}:${String(local.getUTCMinutes()).padStart(2, "0")}` === hh_mm;
});
const tick_marks = DAYS === 1
  ? [
      { time: at(today, "09:30") },
      { time: at(today, "10:30") },
      { time: at(today, "11:30"), label: "11:30/13:00" },
      { time: at(today, "14:00") },
      { time: at(today, "15:00") },
    ]
  : days.map((day) => ({ time: day.slots[0], label: day.date.slice(5) }));
chart.time_scale().apply_options({
  time_zone: TIME_ZONE,
  time_visible: true,
  tick_marks,
  // Five days of minutes need finer spacing than the reference minimum on narrow screens.
  min_bar_spacing: 0.1,
  lock_visible_logical_range: true,
});
// Every slot in view, padded by half a bar: slot centres sit half a bar in from the pane edges, so
// the opening column stays inside the pane on narrow screens instead of straddling its edge.
chart.time_scale().set_visible_logical_range({ from: -0.5, to: slots.length - 0.5 });

// ---------------------------------------------------------------------------------------------
// Header summary and simulated live feed
// ---------------------------------------------------------------------------------------------

const status = {
  price: document.getElementById("last_price"),
  change: document.getElementById("last_change"),
  average: document.getElementById("last_average"),
  volume: document.getElementById("last_volume"),
  clock: document.getElementById("last_time"),
  feed: document.getElementById("feed_toggle"),
};
const time_text = new Intl.DateTimeFormat("zh-CN", {
  timeZone: TIME_ZONE,
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  hourCycle: "h23",
});
const volume_text = new Intl.NumberFormat("zh-CN", { notation: "compact", maximumFractionDigits: 2 });

function refresh_summary() {
  if (traded === live_from) {
    // Pre-open: the day's slots are reserved but nothing has traded yet.
    for (const field of [status.price, status.change, status.average]) {
      field.textContent = "—";
      delete field.dataset.direction;
    }
    status.volume.textContent = "0";
    status.clock.textContent = "待开盘 Pre-open";
    return;
  }
  // Everything shown is read back from the chart, including the forming minute.
  const last = price.data()[traded - 1];
  const change = last.value - today.prev_close;
  const direction = change > 0 ? "up" : change < 0 ? "down" : "flat";
  const session_volume = volume.data().slice(live_from, traded).reduce((sum, row) => sum + row.value, 0);
  const average_value = average.data().findLast((row) => row.time === last.time)?.value;
  status.price.textContent = last.value.toFixed(2);
  status.price.dataset.direction = direction;
  status.change.dataset.direction = direction;
  status.change.textContent = `${change >= 0 ? "+" : "−"}${Math.abs(change).toFixed(2)}  ${change >= 0 ? "+" : "−"}${Math.abs(change / today.prev_close * 100).toFixed(2)}%`;
  status.average.textContent = average_value === undefined ? "—" : average_value.toFixed(3);
  status.volume.textContent = volume_text.format(session_volume);
  status.clock.textContent = traded >= slots.length ? "收盘 Closed" : time_text.format(new Date(last.time * 1000));
}

// Sequence numbers make every delivery idempotent: a replayed or late tick is rejected.
let sequence = 0;
let forming = false;
/** Advance the feed by one tick: open the next minute, then complete it with a merge. */
function step() {
  if (traded >= slots.length && !forming) return false;
  const minute = forming ? minutes[traded - 1] : minutes[traded];
  sequence += 1;
  if (!forming) {
    // The minute's first trade fills its whitespace slot with a partial size; the day's first
    // trade opens between the previous close and the minute's close.
    const previous = traded > 0 ? minutes[traded - 1].price : today.prev_close;
    const first = { time: minute.time, value: cents((previous + minute.price) / 2) };
    const opening_volume = Math.max(100, Math.round(minute.volume / 200) * 100);
    price.update(first, { sequence });
    percent.update(first, { sequence });
    volume.update({ time: minute.time, value: opening_volume }, { sequence });
    amount.update({ time: minute.time, value: cents(opening_volume * first.value) }, { sequence });
    forming = true;
    traded += 1;
  } else {
    // Later ticks of the forming minute merge the latest price and cumulative size.
    price.merge({ time: minute.time, value: minute.price }, { sequence });
    percent.merge({ time: minute.time, value: minute.price }, { sequence });
    volume.merge({ time: minute.time, value: minute.volume }, { sequence });
    amount.merge({ time: minute.time, value: minute.amount }, { sequence });
    forming = false;
  }
  refresh_summary();
  return true;
}

/** Complete `count` minutes synchronously (test and fast-forward hook). */
function advance(count = 1) {
  if (forming) step();
  for (let done = 0; done < count && traded < slots.length; done += 1) {
    step();
    step();
  }
}

let timer = null;
function set_feed(running) {
  if (timer !== null) clearInterval(timer);
  timer = running ? setInterval(() => { if (!step()) set_feed(false); }, FEED_MS) : null;
  status.feed.textContent = timer === null ? "▶ 行情" : "⏸ 行情";
  status.feed.setAttribute("aria-pressed", String(timer !== null));
}
status.feed.addEventListener("click", () => set_feed(timer === null));
refresh_summary();
set_feed(query.get("feed") !== "off" && traded < slots.length);

window.__intraday = {
  chart,
  price,
  percent,
  average,
  volume,
  amount,
  slots,
  minutes,
  days: days.map(({ date, slots, prev_close }) => ({ date, first: slots[0], last: slots.at(-1), prev_close })),
  reference_close,
  tick_marks,
  traded: () => traded,
  forming: () => forming,
  step,
  advance,
  set_feed,
};
