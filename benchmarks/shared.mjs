export const generator_version = 1;

function next_random(state) {
  let value = state.value >>> 0;
  value ^= value << 13;
  value ^= value >>> 17;
  value ^= value << 5;
  state.value = value >>> 0;
  return state.value / 0x1_0000_0000;
}

export function generate_ohlcv(points, seed = 0x02f6e2b1, configuration = {}) {
  if (!Number.isInteger(points) || points < 0) throw new TypeError("points must be a non-negative integer");
  const start_time = configuration.start_time ?? 1_577_836_800;
  const interval_seconds = configuration.interval_seconds ?? 60;
  const start_price = configuration.start_price ?? 100;
  const volatility = configuration.volatility ?? 0.8;
  const state = { value: (seed >>> 0) || 1 };
  const times = new Float64Array(points);
  const open = new Float64Array(points);
  const high = new Float64Array(points);
  const low = new Float64Array(points);
  const close = new Float64Array(points);
  const volume = new Float64Array(points);
  let price = start_price;
  for (let index = 0; index < points; index += 1) {
    const noise = (next_random(state) - 0.5) * volatility;
    const drift = Math.sin(index * 0.017) * volatility;
    const next = Math.max(0.01, price + drift + noise);
    const wick = 0.1 + next_random(state) * volatility;
    times[index] = start_time + index * interval_seconds;
    open[index] = price;
    high[index] = Math.max(price, next) + wick;
    low[index] = Math.max(0, Math.min(price, next) - wick);
    close[index] = next;
    volume[index] = 100 + Math.floor(next_random(state) * 9_900);
    price = next;
  }
  return { times, open, high, low, close, volume };
}

export function percentile(sorted_samples, probability) {
  if (sorted_samples.length === 0) return null;
  if (!(probability >= 0 && probability <= 1)) throw new RangeError("probability must be between 0 and 1");
  const index = Math.max(0, Math.ceil(probability * sorted_samples.length) - 1);
  return sorted_samples[index];
}

export function summarize(samples) {
  if (!Array.isArray(samples) || samples.length === 0) throw new TypeError("samples must be a non-empty array");
  if (samples.some((value) => !Number.isFinite(value))) throw new TypeError("samples must contain only finite numbers");
  const sorted = [...samples].sort((left, right) => left - right);
  const mean = sorted.reduce((sum, value) => sum + value, 0) / sorted.length;
  const variance = sorted.reduce((sum, value) => sum + (value - mean) ** 2, 0) / sorted.length;
  return {
    count: sorted.length,
    min: sorted[0],
    max: sorted.at(-1),
    mean,
    p50: percentile(sorted, 0.50),
    p90: percentile(sorted, 0.90),
    p95: percentile(sorted, 0.95),
    p99: percentile(sorted, 0.99),
    standard_deviation: Math.sqrt(variance),
  };
}

export function metric(samples, unit, direction, visibility, methodology, availability = "measured") {
  if (availability !== "measured") {
    return { availability, unit, direction, visibility, methodology, samples: [], summary: null };
  }
  return { availability, unit, direction, visibility, methodology, samples, summary: summarize(samples) };
}

export function dataset_metadata(points, seed, configuration = {}) {
  return {
    generator_version,
    seed: seed >>> 0,
    data_type: "deterministic_ohlcv",
    points,
    series_count: configuration.series_count ?? 1,
    pane_count: configuration.pane_count ?? 1,
    configuration: {
      start_time: configuration.start_time ?? 1_577_836_800,
      interval_seconds: configuration.interval_seconds ?? 60,
      start_price: configuration.start_price ?? 100,
      volatility: configuration.volatility ?? 0.8,
      ...(configuration.studies === undefined ? {} : { studies: configuration.studies }),
    },
  };
}
