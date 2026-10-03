import { test, expect } from "@playwright/test";

// KLineChart's 27 indicator templates through the public package API (Canvas2D): every template
// binds with the outputs the engine schema promises, values start after the declared warm-up and
// stay finite, several match independent JS formulas, bindings report and restore their
// parameters, and invalid definitions are rejected without touching the chart.

const BAR_COUNT = 240;
const FIRST_TIME = 1_700_000_000;
const TOLERANCE = 1e-9;

// Deterministic OHLCV with integer volumes (so volume sums are exact) and a few unchanged closes.
const bars = [];
for (let index = 0; index < BAR_COUNT; index += 1) {
  const open = 100 + Math.sin(index * 0.21) * 6 + Math.cos(index * 0.047) * 3 + index * 0.01;
  let close = open + Math.sin(index * 0.9 + 1) * 1.3 + ((index % 5) - 2) * 0.15;
  if (index > 0 && index % 40 === 39) close = bars[index - 1].close;
  bars.push({
    time: FIRST_TIME + index * 86_400,
    open,
    high: Math.max(open, close) + 0.4 + (index % 3) * 0.2,
    low: Math.min(open, close) - 0.4 - (index % 4) * 0.15,
    close,
    volume: 1000 + ((index * 37) % 11) * 113 + (index % 7) * 29,
  });
}

// KLineChart's 27 templates at KLineChart's default parameters: the output keys in output order,
// the outputs drawn as bars or dots, whether a volume series is required, and whether the template
// draws over the candles.
const TEMPLATES = [
  { definition: { indicator: "ma", periods: [5, 10, 30, 60] }, keys: ["ma1", "ma2", "ma3", "ma4"], price: true },
  { definition: { indicator: "ema", periods: [6, 12, 20] }, keys: ["ema1", "ema2", "ema3"], price: true },
  { definition: { indicator: "sma", period: 12, weight: 2 }, keys: ["sma"], price: true },
  { definition: { indicator: "boll", period: 20, multiplier: 2 }, keys: ["up", "mid", "dn"], price: true },
  { definition: { indicator: "sar", start: 2, step: 2, max: 20 }, keys: ["sar"], dots: [0], price: true },
  { definition: { indicator: "bbi", periods: [3, 6, 12, 24] }, keys: ["bbi"], price: true },
  { definition: { indicator: "avp" }, keys: ["avp"], volume: true, price: true },
  { definition: { indicator: "vol", periods: [5, 10, 20] }, keys: ["volume", "ma1", "ma2", "ma3"], bars: [0], volume: true },
  { definition: { indicator: "macd", short: 12, long: 26, signal: 9 }, keys: ["dif", "dea", "macd"], bars: [2] },
  { definition: { indicator: "kdj", period: 9, k_smoothing: 3, d_smoothing: 3 }, keys: ["k", "d", "j"] },
  { definition: { indicator: "rsi", periods: [6, 12, 24] }, keys: ["rsi1", "rsi2", "rsi3"] },
  { definition: { indicator: "bias", periods: [6, 12, 24] }, keys: ["bias1", "bias2", "bias3"] },
  { definition: { indicator: "brar", period: 26 }, keys: ["br", "ar"] },
  { definition: { indicator: "cci", period: 20 }, keys: ["cci"] },
  { definition: { indicator: "dmi", period: 14, adxr_period: 6 }, keys: ["pdi", "mdi", "adx", "adxr"] },
  { definition: { indicator: "cr", period: 26, ma_periods: [10, 20, 40, 60] }, keys: ["cr", "ma1", "ma2", "ma3", "ma4"] },
  { definition: { indicator: "psy", period: 12, ma_period: 6 }, keys: ["psy", "maPsy"] },
  { definition: { indicator: "dma", short: 10, long: 50, signal: 10 }, keys: ["dma", "ama"] },
  { definition: { indicator: "trix", period: 12, ma_period: 9 }, keys: ["trix", "maTrix"] },
  { definition: { indicator: "obv", ma_period: 30 }, keys: ["obv", "maObv"], volume: true },
  { definition: { indicator: "vr", period: 26, ma_period: 6 }, keys: ["vr", "maVr"], volume: true },
  { definition: { indicator: "wr", periods: [6, 10, 14] }, keys: ["wr1", "wr2", "wr3"] },
  { definition: { indicator: "mtm", period: 12, ma_period: 6 }, keys: ["mtm", "maMtm"] },
  { definition: { indicator: "emv", period: 14 }, keys: ["emv", "maEmv"], volume: true },
  { definition: { indicator: "roc", period: 12, ma_period: 6 }, keys: ["roc", "maRoc"] },
  { definition: { indicator: "pvt" }, keys: ["pvt"], volume: true },
  { definition: { indicator: "ao", short: 5, long: 34 }, keys: ["ao"], bars: [0] },
];

// The same templates with parameters that differ from every default and, for lists, in length.
const CUSTOM_DEFINITIONS = [
  { indicator: "ma", periods: [3, 7] },
  { indicator: "ema", periods: [4, 9, 16, 25, 36] },
  { indicator: "sma", period: 7, weight: 1.5 },
  { indicator: "boll", period: 10, multiplier: 1.5 },
  { indicator: "sar", start: 1, step: 1, max: 10 },
  { indicator: "bbi", periods: [2, 4, 6, 8] },
  { indicator: "vol", periods: [3] },
  { indicator: "macd", short: 5, long: 13, signal: 4 },
  { indicator: "kdj", period: 14, k_smoothing: 5, d_smoothing: 4 },
  { indicator: "rsi", periods: [5] },
  { indicator: "bias", periods: [4, 8] },
  { indicator: "brar", period: 10 },
  { indicator: "cci", period: 14 },
  { indicator: "dmi", period: 10, adxr_period: 4 },
  { indicator: "cr", period: 10, ma_periods: [3, 4, 5, 6] },
  { indicator: "psy", period: 8, ma_period: 3 },
  { indicator: "dma", short: 6, long: 20, signal: 5 },
  { indicator: "trix", period: 8, ma_period: 4 },
  { indicator: "obv", ma_period: 10 },
  { indicator: "vr", period: 12, ma_period: 4 },
  { indicator: "wr", periods: [5] },
  { indicator: "mtm", period: 6, ma_period: 3 },
  { indicator: "emv", period: 7 },
  { indicator: "roc", period: 6, ma_period: 3 },
  { indicator: "ao", short: 3, long: 10 },
];

const page_errors = new WeakMap();

test.beforeEach(async ({ page }) => {
  const errors = [];
  page_errors.set(page, errors);
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  // A fresh Canvas2D chart on the fixture: candles, a hidden volume series, and a hidden turnover
  // series (traded value per bar, the source series of AVP).
  await page.addScriptTag({
    type: "module",
    content: `
      import { create_chart } from "/dist/aeris_charts_financial.js";
      window.__build_klinechart_chart = async (bars) => {
        const host = document.createElement("div");
        host.style.cssText = "position:absolute;left:-10000px;width:800px;height:500px";
        document.body.append(host);
        const chart = await create_chart(host, { backend: "canvas2d", autoSize: false });
        const main = chart.add_series("candlestick");
        main.set_data(bars.map(({ time, open, high, low, close }) => ({ time, open, high, low, close })));
        const volume = chart.add_series("histogram", { visible: false });
        volume.set_data(bars.map(({ time, volume }) => ({ time, value: volume })));
        const turnover = chart.add_series("line", { visible: false });
        turnover.set_data(bars.map(({ time, volume, high, low, close }) => ({ time, value: volume * (high + low + close) / 3 })));
        return { chart, main, volume, turnover, host };
      };
    `,
  });
  await page.waitForFunction(() => window.__build_klinechart_chart !== undefined);
});

test.afterEach(async ({ page }) => {
  expect(page_errors.get(page)).toEqual([]);
});

const flatten_definition = ({ indicator, ...parameters }) => Object.values(parameters).flat();
const finite_rows = (data) => data.filter((point) => "value" in point);

test("all 27 templates bind through add_klinechart_indicator with the outputs their engine schema reports", async ({ page }) => {
  expect(TEMPLATES).toHaveLength(27);
  const records = await page.evaluate(async ({ bars, templates }) => {
    const { chart, main, volume, turnover, host } = await window.__build_klinechart_chart(bars);
    const records = [];
    const settle = () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    for (const template of templates) {
      const { definition } = template;
      const source = definition.indicator === "avp" ? turnover : main;
      const schema = chart.indicator_schema(`klinechart_${definition.indicator}`);
      const outputs = chart.add_klinechart_indicator(source, definition, template.volume ? volume : undefined);
      await settle();
      records.push({
        name: definition.indicator,
        schema_outputs: schema.outputs.map((output) => output.name),
        schema_defaults: schema.parameters
          .filter((parameter) => parameter.name !== "source" && parameter.parameter_type !== "series")
          .map((parameter) => parameter.default),
        schema_series: schema.parameters.filter((parameter) => parameter.parameter_type === "series").map((parameter) => parameter.name),
        source_id: source.id,
        volume_id: volume.id,
        outputs: outputs.map((output) => {
          const info = output.indicator_info();
          return {
            id: output.id,
            type: output.series_type(),
            pane: output.pane_index(),
            kind: info.kind,
            binding_id: info.binding_id,
            output_name: info.output_name,
            output_index: info.output_index,
            output_count: info.output_count,
            klinechart: info.parameters.klinechart,
            source: info.source.id,
            source_input: info.source_input,
            volume_source: info.volume_source?.id ?? null,
            amount_source: info.amount_source?.id ?? null,
            warmup_bars: info.warmup_bars,
            point_markers: info.style.point_markers,
            data: output.data(),
          };
        }),
      });
      // Removing one output removes the whole binding.
      chart.remove_series(outputs[0]);
    }
    const remaining = chart.panes().flatMap((pane) => pane.get_series().map((series) => series.id));
    const pane_count = chart.panes().length;
    chart.remove();
    host.remove();
    return { records, remaining, pane_count };
  }, { bars, templates: TEMPLATES });

  // Every binding was removed; only the three helper series remain, and no extra pane survives.
  expect(records.remaining.slice().sort()).toEqual([0, 1, 2]);
  expect(records.pane_count).toBe(1);
  expect(records.records.map((record) => record.name)).toEqual(TEMPLATES.map((template) => template.definition.indicator));

  for (const [index, record] of records.records.entries()) {
    const template = TEMPLATES[index];
    const name = template.definition.indicator;
    const info = (message) => `${name}: ${message}`;

    // The engine schema is the owner of the template list: names, defaults, and volume needs.
    expect(record.schema_outputs, info("schema outputs")).toEqual(template.keys);
    expect(record.schema_defaults, info("schema defaults are the definition's parameters")).toEqual(flatten_definition(template.definition));
    expect(record.schema_series, info("schema volume requirement")).toEqual(template.volume ? ["volume_source"] : []);

    // One series per output, in output order, all sharing one binding.
    expect(record.outputs.map((output) => output.output_name), info("output names")).toEqual(template.keys);
    expect(record.outputs.map((output) => output.output_index), info("output indexes")).toEqual(template.keys.map((_, i) => i));
    expect(new Set(record.outputs.map((output) => output.id)).size, info("distinct series")).toBe(template.keys.length);
    expect(new Set(record.outputs.map((output) => output.binding_id)).size, info("one binding")).toBe(1);
    for (const [slot, output] of record.outputs.entries()) {
      const label = info(`output ${slot}`);
      // indicator_info reports a kind inside the declared type and the exact definition back.
      expect(output.kind, label).toBe(`klinechart_${name}`);
      expect(output.klinechart, label).toEqual(template.definition);
      expect(output.output_count, label).toBe(template.keys.length);
      expect(output.source, label).toBe(record.source_id);
      expect(output.source_input, label).toBe("close");
      expect(output.volume_source, label).toBe(template.volume ? record.volume_id : null);
      expect(output.amount_source, label).toBeNull();
      // Bars are histograms, everything else lines; SAR is drawn as dots.
      expect(output.type, label).toBe((template.bars ?? []).includes(slot) ? "histogram" : "line");
      expect(output.point_markers, label).toBe((template.dots ?? []).includes(slot));
      // Price templates draw on the candles' pane, every other template in a pane below it.
      if (template.price) expect(output.pane, label).toBe(0);
      else expect(output.pane, label).toBeGreaterThan(0);

      // Values start at the declared warm-up row (AVP, whose warm-up is volume-dependent, starts at
      // row 0 here) and stay finite through the last bar.
      const values = finite_rows(output.data);
      expect(output.data.length, label).toBe(values.length);
      expect(values.length, label).toBe(BAR_COUNT - output.warmup_bars);
      expect(values.length, label).toBeGreaterThan(0);
      expect(values.every((point) => Number.isFinite(point.value)), label).toBe(true);
      expect(values.at(-1).time, label).toBe(bars.at(-1).time);
    }
    // Outputs of one binding share a pane.
    expect(new Set(record.outputs.map((output) => output.pane)).size, info("one pane")).toBe(1);
  }
});

test("custom parameters round-trip through indicator_info and size the outputs", async ({ page }) => {
  const records = await page.evaluate(async ({ bars, definitions }) => {
    const { chart, main, volume, turnover, host } = await window.__build_klinechart_chart(bars);
    const needs_volume = new Set(["vol", "obv", "vr", "emv", "pvt", "avp"]);
    const records = definitions.map((definition) => {
      const outputs = chart.add_klinechart_indicator(main, definition, needs_volume.has(definition.indicator) ? volume : undefined);
      const info = outputs[0].indicator_info();
      return {
        count: outputs.length,
        output_count: info.output_count,
        klinechart: info.parameters.klinechart,
        kind: info.kind,
        period: info.period,
        deviation: info.deviation,
        rows: outputs.map((output) => output.data().length),
      };
    });
    chart.remove();
    host.remove();
    return records;
  }, { bars, definitions: CUSTOM_DEFINITIONS });

  for (const [index, record] of records.entries()) {
    const definition = CUSTOM_DEFINITIONS[index];
    expect(record.klinechart, definition.indicator).toEqual(definition);
    expect(record.kind).toBe(`klinechart_${definition.indicator}`);
    expect(record.deviation, definition.indicator).toBeNull();
    // `period` is the first period (a whole-number parameter); SAR has only real-valued factors.
    const first_period = definition.indicator === "sar" ? 0 : flatten_definition(definition)[0];
    expect(record.period, definition.indicator).toBe(first_period);
    expect(record.count, definition.indicator).toBe(record.output_count);
  }
  // Lists size the outputs: one per period, plus VOL's bars.
  const counts = Object.fromEntries(records.map((record, index) => [CUSTOM_DEFINITIONS[index].indicator, record.count]));
  expect(counts).toMatchObject({ ma: 2, ema: 5, vol: 2, rsi: 1, bias: 2, wr: 1, cr: 5, bbi: 1 });
  // A shorter window starts earlier.
  expect(records[0].rows[0]).toBe(BAR_COUNT - 2);
  expect(records[0].rows[1]).toBe(BAR_COUNT - 6);
});

// ---- independent references, written from the formulas rather than from the engine -------------

const closes = bars.map((bar) => bar.close);
const unset = () => Array(BAR_COUNT).fill(NaN);

function rolling_mean(values, period) {
  const out = unset();
  for (let row = period - 1; row < values.length; row += 1) {
    let sum = 0;
    for (let i = row - period + 1; i <= row; i += 1) sum += values[i];
    out[row] = sum / period;
  }
  return out;
}

/** EMA seeded with the mean of the first `period` values, then (2x + (n - 1) ema') / (n + 1). */
function seeded_ema(values, period, from = 0) {
  const out = unset();
  let ema = 0;
  for (let row = from + period - 1; row < values.length; row += 1) {
    if (row === from + period - 1) {
      let sum = 0;
      for (let i = from; i <= row; i += 1) sum += values[i];
      ema = sum / period;
    } else {
      ema = (2 * values[row] + (period - 1) * ema) / (period + 1);
    }
    out[row] = ema;
  }
  return out;
}

/** Wilder RSI: the first average is the mean of the first `period` changes, so the first value is row `period`. */
function rsi(values, period) {
  const out = unset();
  let gains = 0;
  let losses = 0;
  let average_gain = 0;
  let average_loss = 0;
  for (let row = 1; row < values.length; row += 1) {
    const change = values[row] - values[row - 1];
    const gain = Math.max(change, 0);
    const loss = Math.max(-change, 0);
    if (row < period) {
      gains += gain;
      losses += loss;
      continue;
    }
    if (row === period) {
      average_gain = (gains + gain) / period;
      average_loss = (losses + loss) / period;
    } else {
      average_gain = (average_gain * (period - 1) + gain) / period;
      average_loss = (average_loss * (period - 1) + loss) / period;
    }
    out[row] = average_loss === 0 ? 100 : average_gain === 0 ? 0 : 100 - 100 / (1 + average_gain / average_loss);
  }
  return out;
}

function macd(values, short, long, signal) {
  const fast = seeded_ema(values, short);
  const slow = seeded_ema(values, long);
  const first = Math.max(short, long) - 1;
  const dif = unset();
  for (let row = first; row < values.length; row += 1) dif[row] = fast[row] - slow[row];
  // DEA is the EMA of DIF, seeded with the mean of the first `signal` DIF values.
  const dea = seeded_ema(dif.map((value) => (Number.isNaN(value) ? 0 : value)), signal, first);
  const histogram = unset();
  for (let row = first + signal - 1; row < values.length; row += 1) histogram[row] = (dif[row] - dea[row]) * 2;
  return { dif, dea: dea.map((value, row) => (row < first + signal - 1 ? NaN : value)), histogram };
}

function boll(values, period, multiplier) {
  const mid = rolling_mean(values, period);
  const up = unset();
  const dn = unset();
  for (let row = period - 1; row < values.length; row += 1) {
    let squares = 0;
    for (let i = row - period + 1; i <= row; i += 1) squares += (values[i] - mid[row]) ** 2;
    const deviation = Math.sqrt(squares / period);
    up[row] = mid[row] + multiplier * deviation;
    dn[row] = mid[row] - multiplier * deviation;
  }
  return { up, mid, dn };
}

function obv(values, volumes) {
  const out = [];
  let running = 0;
  for (let row = 0; row < values.length; row += 1) {
    const previous = values[Math.max(row - 1, 0)];
    if (values[row] > previous) running += volumes[row];
    else if (values[row] < previous) running -= volumes[row];
    out.push(running);
  }
  return out;
}

function average_price(volumes, turnovers) {
  let traded = 0;
  let total = 0;
  return volumes.map((volume, row) => {
    traded += turnovers[row];
    total += volume;
    return total === 0 ? NaN : traded / total;
  });
}

function mismatches(label, actual, expected) {
  const by_time = new Map(finite_rows(actual).map((point) => [point.time, point.value]));
  const found = [];
  expected.forEach((value, row) => {
    const time = bars[row].time;
    if (Number.isNaN(value)) {
      if (by_time.has(time)) found.push(`${label} row ${row}: expected unset, got ${by_time.get(time)}`);
    } else if (!by_time.has(time) || !(Math.abs(by_time.get(time) - value) <= TOLERANCE)) {
      found.push(`${label} row ${row}: expected ${value}, got ${by_time.get(time)}`);
    }
  });
  return found;
}

test("MA, EMA, BOLL, RSI, MACD, VOL, OBV and AVP match independent JS references to 1e-9", async ({ page }) => {
  const outputs = await page.evaluate(async ({ bars }) => {
    const { chart, main, volume, turnover, host } = await window.__build_klinechart_chart(bars);
    const data = (series) => series.map((output) => output.data());
    const result = {
      ma: data(chart.add_klinechart_indicator(main, { indicator: "ma", periods: [5, 10, 30, 60] })),
      ema: data(chart.add_klinechart_indicator(main, { indicator: "ema", periods: [6, 12, 20] })),
      boll: data(chart.add_klinechart_indicator(main, { indicator: "boll", period: 20, multiplier: 2 })),
      rsi: data(chart.add_klinechart_indicator(main, { indicator: "rsi", periods: [6, 12, 24] })),
      macd: data(chart.add_klinechart_indicator(main, { indicator: "macd", short: 12, long: 26, signal: 9 })),
      macd_custom: data(chart.add_klinechart_indicator(main, { indicator: "macd", short: 5, long: 13, signal: 4 })),
      vol: data(chart.add_klinechart_indicator(main, { indicator: "vol", periods: [5, 10, 20] }, volume)),
      obv: data(chart.add_klinechart_indicator(main, { indicator: "obv", ma_period: 30 }, volume)),
      avp: data(chart.add_klinechart_indicator(turnover, { indicator: "avp" }, volume)),
    };
    chart.remove();
    host.remove();
    return result;
  }, { bars });

  const volumes = bars.map((bar) => bar.volume);
  const turnovers = bars.map((bar) => bar.volume * (bar.high + bar.low + bar.close) / 3);
  const found = [];
  [5, 10, 30, 60].forEach((period, i) => found.push(...mismatches(`MA${period}`, outputs.ma[i], rolling_mean(closes, period))));
  [6, 12, 20].forEach((period, i) => found.push(...mismatches(`EMA${period}`, outputs.ema[i], seeded_ema(closes, period))));
  const bands = boll(closes, 20, 2);
  found.push(...mismatches("BOLL up", outputs.boll[0], bands.up), ...mismatches("BOLL mid", outputs.boll[1], bands.mid), ...mismatches("BOLL dn", outputs.boll[2], bands.dn));
  [6, 12, 24].forEach((period, i) => found.push(...mismatches(`RSI${period}`, outputs.rsi[i], rsi(closes, period))));
  for (const [label, series, parameters] of [["MACD 12/26/9", outputs.macd, [12, 26, 9]], ["MACD 5/13/4", outputs.macd_custom, [5, 13, 4]]]) {
    const reference = macd(closes, ...parameters);
    found.push(...mismatches(`${label} DIF`, series[0], reference.dif), ...mismatches(`${label} DEA`, series[1], reference.dea), ...mismatches(`${label} histogram`, series[2], reference.histogram));
  }
  found.push(...mismatches("VOL bars", outputs.vol[0], volumes));
  [5, 10, 20].forEach((period, i) => found.push(...mismatches(`VOL MA${period}`, outputs.vol[i + 1], rolling_mean(volumes, period))));
  const running = obv(closes, volumes);
  found.push(...mismatches("OBV", outputs.obv[0], running), ...mismatches("MAOBV", outputs.obv[1], rolling_mean(running, 30)));
  found.push(...mismatches("AVP", outputs.avp[0], average_price(volumes, turnovers)));
  expect(found).toEqual([]);

  // The references are not vacuous: values exist and differ between outputs.
  expect(finite_rows(outputs.rsi[0]).length).toBe(BAR_COUNT - 6);
  expect(finite_rows(outputs.macd[2]).length).toBe(BAR_COUNT - 33);
  expect(finite_rows(outputs.ma[0]).at(-1).value).not.toBe(finite_rows(outputs.ma[3]).at(-1).value);
});

test("a scalar source series binds any template and is read as open = high = low = close = its value", async ({ page }) => {
  const outputs = await page.evaluate(async ({ bars }) => {
    const { chart, turnover, host } = await window.__build_klinechart_chart(bars);
    // `turnover` is a line series: macd binds over it (no OHLC needed) and reads its values as the close.
    const result = chart.add_klinechart_indicator(turnover, { indicator: "macd", short: 12, long: 26, signal: 9 }).map((output) => output.data());
    chart.remove();
    host.remove();
    return result;
  }, { bars });
  const values = bars.map((bar) => bar.volume * (bar.high + bar.low + bar.close) / 3);
  const reference = macd(values, 12, 26, 9);
  expect([
    ...mismatches("scalar MACD DIF", outputs[0], reference.dif),
    ...mismatches("scalar MACD DEA", outputs[1], reference.dea),
    ...mismatches("scalar MACD histogram", outputs[2], reference.histogram),
  ]).toEqual([]);
  expect(finite_rows(outputs[0]).length).toBe(BAR_COUNT - 25);
});

test("add_klinechart_indicator applies series options to every output", async ({ page }) => {
  const result = await page.evaluate(async ({ bars }) => {
    const { chart, main, volume, host } = await window.__build_klinechart_chart(bars);
    const outputs = chart.add_klinechart_indicator(main, { indicator: "ma", periods: [5, 10] }, undefined, { line_width: 3, visible: false });
    // A stray `kind` field of the definition cannot retarget the study.
    const stray = chart.add_klinechart_indicator(main, { indicator: "ma", periods: [5], kind: "sma" });
    // volume_source comes before options, as in add_obv: both reach the binding.
    const with_volume = chart.add_klinechart_indicator(main, { indicator: "obv", ma_period: 30 }, volume, { line_width: 2 });
    const record = {
      widths: outputs.map((output) => output.options().line_width),
      visible: outputs.map((output) => output.options().visible),
      stray_kind: stray[0].indicator_info().kind,
      volume_widths: with_volume.map((output) => output.options().line_width),
      volume_ids: with_volume.map((output) => output.indicator_info().volume_source.id),
      expected_volume_id: volume.id,
    };
    chart.remove();
    host.remove();
    return record;
  }, { bars });
  const { expected_volume_id, ...reported } = result;
  expect(reported).toEqual({
    widths: [3, 3],
    visible: [false, false],
    stray_kind: "klinechart_ma",
    volume_widths: [2, 2],
    volume_ids: [expected_volume_id, expected_volume_id],
  });
});

test("export_state and import_state restore KLineChart bindings with their volume, turnover, and styles", async ({ page }) => {
  const result = await page.evaluate(async ({ bars }) => {
    const build = window.__build_klinechart_chart;
    const first = await build(bars);
    const definitions = [
      { definition: { indicator: "macd", short: 5, long: 13, signal: 4 }, source: "main" },
      { definition: { indicator: "ma", periods: [3, 7] }, source: "main" },
      { definition: { indicator: "boll", period: 10, multiplier: 1.5 }, source: "main" },
      { definition: { indicator: "sar", start: 1, step: 1, max: 10 }, source: "main" },
      { definition: { indicator: "vol", periods: [3, 6] }, source: "main", volume: true },
      { definition: { indicator: "obv", ma_period: 10 }, source: "main", volume: true },
      { definition: { indicator: "avp" }, source: "turnover", volume: true },
    ];
    for (const { definition, source, volume } of definitions) {
      first.chart.add_klinechart_indicator(first[source], definition, volume ? first.volume : undefined);
    }
    const snapshot = (chart) => chart.panes()
      .flatMap((pane) => pane.get_series())
      .filter((series) => series.indicator_info() !== null)
      .map((series) => {
        const info = series.indicator_info();
        return {
          kind: info.kind,
          klinechart: info.parameters.klinechart,
          output_index: info.output_index,
          output_count: info.output_count,
          source: info.source.id,
          volume_source: info.volume_source?.id ?? null,
          line_color: info.style.line_color,
          visible: info.style.visible,
          type: series.series_type(),
          pane: series.pane_index(),
          data: series.data(),
        };
      });
    // Restyle one output so style persistence is observable.
    const restyled = first.chart.panes().flatMap((pane) => pane.get_series())
      .find((series) => series.indicator_info()?.kind === "klinechart_ma" && series.indicator_info().output_index === 1);
    restyled.set_indicator_output_style({ line_color: "#123456", visible: false });

    const state = first.chart.export_state();
    const before = snapshot(first.chart);
    const second = await build(bars);
    const restored = second.chart.import_state(state);
    const after = snapshot(second.chart);
    const canonical = second.chart.export_state();
    for (const built of [first, second]) {
      built.chart.remove();
      built.host.remove();
    }
    return { state, restored, before, after, canonical };
  }, { bars });

  expect(result.restored.schema_version).toBe(3);
  expect(result.state.indicators.map((study) => study.kind)).toEqual([
    { kind: "klinechart", indicator: "macd", short: 5, long: 13, signal: 4 },
    { kind: "klinechart", indicator: "ma", periods: [3, 7] },
    { kind: "klinechart", indicator: "boll", period: 10, multiplier: 1.5 },
    { kind: "klinechart", indicator: "sar", start: 1, step: 1, max: 10 },
    { kind: "klinechart", indicator: "vol", periods: [3, 6] },
    { kind: "klinechart", indicator: "obv", ma_period: 10 },
    { kind: "klinechart", indicator: "avp" },
  ]);
  // Volume and turnover sources persist as references to the host series.
  expect(result.state.indicators.map((study) => study.volume_source?.id ?? null)).toEqual([null, null, null, null, 1, 1, 1]);
  expect(result.state.indicators[6].source).toEqual({ kind: "series", id: 2 });

  // Outputs: macd 3, ma 2, boll 3, sar 1, vol 3, obv 2, avp 1.
  expect(result.before).toHaveLength(15);
  expect(result.after).toEqual(result.before);
  expect(result.after.find((output) => output.kind === "klinechart_ma" && output.output_index === 1)).toMatchObject({ line_color: "#123456", visible: false });
  expect(result.canonical).toEqual(result.state);
});

test("invalid KLineChart definitions are rejected with invalid_options and leave the chart unchanged", async ({ page }) => {
  const result = await page.evaluate(async ({ bars }) => {
    const { chart, main, volume, turnover, host } = await window.__build_klinechart_chart(bars);
    const other_candles = chart.add_series("candlestick");
    other_candles.set_data(bars.map(({ time, open, high, low, close }) => ({ time, open, high, low, close })));
    const macd = { indicator: "macd", short: 12, long: 26, signal: 9 };
    const cases = {
      "unknown template": () => chart.add_klinechart_indicator(main, { indicator: "nope" }),
      "upper-case template": () => chart.add_klinechart_indicator(main, { ...macd, indicator: "MACD" }),
      "inherited property name": () => chart.add_klinechart_indicator(main, { indicator: "constructor" }),
      "no template": () => chart.add_klinechart_indicator(main, {}),
      "null definition": () => chart.add_klinechart_indicator(main, null),
      "zero period": () => chart.add_klinechart_indicator(main, { ...macd, short: 0 }),
      "negative period": () => chart.add_klinechart_indicator(main, { ...macd, long: -26 }),
      "fractional period": () => chart.add_klinechart_indicator(main, { ...macd, signal: 9.5 }),
      "period above the limit": () => chart.add_klinechart_indicator(main, { indicator: "cci", period: 1_000_001 }),
      "string period": () => chart.add_klinechart_indicator(main, { indicator: "cci", period: "20" }),
      "NaN weight": () => chart.add_klinechart_indicator(main, { indicator: "sma", period: 12, weight: Number.NaN }),
      "zero weight": () => chart.add_klinechart_indicator(main, { indicator: "sma", period: 12, weight: 0 }),
      "negative multiplier": () => chart.add_klinechart_indicator(main, { indicator: "boll", period: 20, multiplier: -1 }),
      "zero SAR factor": () => chart.add_klinechart_indicator(main, { indicator: "sar", start: 0, step: 2, max: 20 }),
      "missing parameter": () => chart.add_klinechart_indicator(main, { indicator: "macd", short: 12, long: 26 }),
      "no parameters": () => chart.add_klinechart_indicator(main, { indicator: "kdj" }),
      "too many periods": () => chart.add_klinechart_indicator(main, { indicator: "ma", periods: [1, 2, 3, 4, 5, 6] }),
      "too many VOL periods": () => chart.add_klinechart_indicator(main, { indicator: "vol", periods: [1, 2, 3, 4, 5] }, volume),
      "empty period list": () => chart.add_klinechart_indicator(main, { indicator: "rsi", periods: [] }),
      "short BBI list": () => chart.add_klinechart_indicator(main, { indicator: "bbi", periods: [3, 6, 12] }),
      "short CR list": () => chart.add_klinechart_indicator(main, { indicator: "cr", period: 26, ma_periods: [10, 20] }),
      "volume template without volume": () => chart.add_klinechart_indicator(main, { indicator: "obv", ma_period: 30 }),
      "PVT without volume": () => chart.add_klinechart_indicator(main, { indicator: "pvt" }),
      "AVP without volume": () => chart.add_klinechart_indicator(turnover, { indicator: "avp" }),
      "volume given to a price template": () => chart.add_klinechart_indicator(main, { indicator: "ma", periods: [5] }, volume),
      "volume equal to the source": () => chart.add_klinechart_indicator(volume, { indicator: "obv", ma_period: 30 }, volume),
      "OHLC volume series": () => chart.add_klinechart_indicator(main, { indicator: "obv", ma_period: 30 }, other_candles),
      "AVP over an OHLC source": () => chart.add_klinechart_indicator(main, { indicator: "avp" }, volume),
    };
    const state = () => ({
      series: chart.panes().flatMap((pane) => pane.get_series().map((series) => series.id)),
      panes: chart.panes().length,
      studies: (chart.export_state().indicators ?? []).length,
    });
    const before = state();
    const errors = {};
    const messages = {};
    for (const [label, run] of Object.entries(cases)) {
      try {
        run();
        errors[label] = "accepted";
      } catch (error) {
        errors[label] = `${error.name}:${error.code}`;
        messages[label] = error.message;
      }
    }
    let unknown_schema = null;
    try { chart.indicator_schema("klinechart_nope"); } catch (error) { unknown_schema = error.code; }
    const after = state();
    chart.remove();
    host.remove();
    return { errors, messages, before, after, labels: Object.keys(cases), unknown_schema };
  }, { bars });

  expect(result.errors).toEqual(Object.fromEntries(result.labels.map((label) => [label, "AerisChartsError:invalid_options"])));
  // The engine owns the template list; the error still names the template that was passed.
  expect(result.messages["unknown template"]).toContain('"nope"');
  expect(result.messages["upper-case template"]).toContain('"MACD"');
  expect(result.messages["inherited property name"]).toContain('"constructor"');
  expect(result.unknown_schema).toBe("invalid_options");
  expect(result.after).toEqual(result.before);
  expect(result.before.studies).toBe(0);
});
