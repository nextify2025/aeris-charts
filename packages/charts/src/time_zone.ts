// Exchange time-zone resolution and exchange-time text for package-owned surfaces.
//
// The engine is platform-free: it accepts only an explicit UTC-offset schedule. This module turns
// an IANA name into that schedule with `Intl.DateTimeFormat` (available on the main thread and in
// workers) and formats exchange-local wall-clock seconds for DOM surfaces (tooltip,
// accessibility). The browser's own time zone is never consulted.

import { AerisChartsError } from "./errors.js";
import type { business_day, time_label_context, time_zone, utc_offset_transition } from "./types.js";

/**
 * IANA zones are resolved over 1970-01-01..2100-01-01. Intraday market data predates 1970 only
 * in archives (daily history should use calendar dates, which never shift), and nothing after
 * 2100 is market data; the span needs at most ~2 transitions per year (≈262), far below the
 * engine's 1024-entry bound. Outside the span the nearest resolved offset applies.
 */
const RESOLVE_FROM_SECONDS = 0;
const RESOLVE_TO_SECONDS = 4_102_444_800;
/**
 * Offsets are sampled weekly and each change is bisected to the exact second. Real rule changes
 * keep an offset for far longer than a week, so no transition pair is skipped.
 */
const SAMPLE_SECONDS = 7 * 86_400;
/** Bounded cache of resolved zones (a page rarely uses more than a handful). */
const MAX_CACHED_ZONES = 32;
const resolved_zones = new Map<string, readonly utc_offset_transition[]>();

/** `en-US` h23 output, e.g. `3/10/2024, 07:00:00`. */
const EN_US_DATE_TIME = /^(\d+)\/(\d+)\/(\d+),? (\d+):(\d+):(\d+)$/;
const QUARTER_HOUR_SECONDS = 900;

function offset_seconds_at(formatter: Intl.DateTimeFormat, utc_seconds: number): number {
  const date = new Date(utc_seconds * 1_000);
  // `format` is several times cheaper than `formatToParts`; the parts path covers engines whose
  // en-US output differs from the expected shape.
  const match = EN_US_DATE_TIME.exec(formatter.format(date));
  let fields: number[];
  if (match !== null) {
    fields = [Number(match[3]), Number(match[1]), Number(match[2]), Number(match[4]), Number(match[5]), Number(match[6])];
  } else {
    fields = [1970, 1, 1, 0, 0, 0];
    for (const part of formatter.formatToParts(date)) {
      const index = ["year", "month", "day", "hour", "minute", "second"].indexOf(part.type);
      if (index >= 0) fields[index] = Number(part.value);
    }
  }
  const [year, month, day, hour, minute, second] = fields as [number, number, number, number, number, number];
  return Date.UTC(year, month - 1, day, hour % 24, minute, second) / 1_000 - utc_seconds;
}

/**
 * First second in `(from, to]` whose offset differs from `offset` (the offset at `to` differs).
 * Post-1970 rule changes happen on quarter-hour boundaries, so those are searched first and the
 * per-second search runs only when a change is not boundary-aligned.
 */
function first_changed_second(
  formatter: Intl.DateTimeFormat,
  from: number,
  to: number,
  offset: number,
): number {
  let low = from;
  let high = to;
  const first_boundary = Math.floor(from / QUARTER_HOUR_SECONDS) + 1;
  const last_boundary = Math.floor(to / QUARTER_HOUR_SECONDS);
  if (first_boundary <= last_boundary) {
    if (offset_seconds_at(formatter, last_boundary * QUARTER_HOUR_SECONDS) !== offset) {
      let unchanged = first_boundary - 1;
      let changed = last_boundary;
      while (changed - unchanged > 1) {
        const middle = Math.floor((unchanged + changed) / 2);
        if (offset_seconds_at(formatter, middle * QUARTER_HOUR_SECONDS) === offset) unchanged = middle;
        else changed = middle;
      }
      high = changed * QUARTER_HOUR_SECONDS;
      low = Math.max(from, (changed - 1) * QUARTER_HOUR_SECONDS);
      if (offset_seconds_at(formatter, high - 1) === offset) return high;
    } else {
      low = last_boundary * QUARTER_HOUR_SECONDS;
    }
  }
  while (high - low > 1) {
    const middle = Math.floor((low + high) / 2);
    if (offset_seconds_at(formatter, middle) === offset) low = middle;
    else high = middle;
  }
  return high;
}

function resolve_iana_zone(name: string): readonly utc_offset_transition[] {
  const cached = resolved_zones.get(name);
  if (cached !== undefined) return cached;
  let formatter: Intl.DateTimeFormat;
  try {
    formatter = new Intl.DateTimeFormat("en-US", {
      timeZone: name,
      hourCycle: "h23",
      year: "numeric",
      month: "numeric",
      day: "numeric",
      hour: "numeric",
      minute: "numeric",
      second: "numeric",
    });
  } catch {
    throw new AerisChartsError("invalid_options", `unknown time zone ${JSON.stringify(name)}`);
  }
  let from = RESOLVE_FROM_SECONDS;
  let offset = offset_seconds_at(formatter, from);
  const transitions: utc_offset_transition[] = [{ from_utc_seconds: from, offset_seconds: offset }];
  while (from < RESOLVE_TO_SECONDS) {
    const to = Math.min(from + SAMPLE_SECONDS, RESOLVE_TO_SECONDS);
    if (offset_seconds_at(formatter, to) === offset) {
      from = to;
      continue;
    }
    const changed = first_changed_second(formatter, from, to, offset);
    offset = offset_seconds_at(formatter, changed);
    transitions.push({ from_utc_seconds: changed, offset_seconds: offset });
    from = changed;
  }
  const frozen = Object.freeze(transitions.map((transition) => Object.freeze(transition)));
  if (resolved_zones.size >= MAX_CACHED_ZONES) {
    const oldest = resolved_zones.keys().next().value;
    if (oldest !== undefined) resolved_zones.delete(oldest);
  }
  resolved_zones.set(name, frozen);
  return frozen;
}

/**
 * Resolve a public {@link time_zone} into the engine's explicit schedule. `"UTC"` is the empty
 * schedule; explicit schedules are passed through after a shape check (the engine validates
 * ordering and bounds and rejects the patch atomically).
 */
export function resolve_time_zone(zone: time_zone): readonly utc_offset_transition[] {
  if (typeof zone === "string") {
    if (zone === "UTC" || zone === "Etc/UTC") return [];
    return resolve_iana_zone(zone);
  }
  if (!Array.isArray(zone)) {
    throw new AerisChartsError("invalid_options", "time_zone must be an IANA name or an offset schedule");
  }
  return zone.map((transition, index) => {
    const from = (transition as utc_offset_transition | null)?.from_utc_seconds;
    const offset = (transition as utc_offset_transition | null)?.offset_seconds;
    if (!Number.isInteger(from) || !Number.isInteger(offset)) {
      throw new AerisChartsError(
        "invalid_options",
        `time_zone transition ${index} needs integer from_utc_seconds and offset_seconds`,
      );
    }
    return { from_utc_seconds: from as number, offset_seconds: offset as number };
  });
}

/** Explicit time-axis marks with their times already converted to UTC seconds. */
export type engine_time_tick_marks = readonly { time: number; label?: string }[] | null;

/**
 * Engine JSON for `timeScale.timeZone` / `timeScale.sessionStart` / `timeScale.tickMarks`, which the
 * engine validates together. `undefined` keys are omitted.
 */
export function exchange_time_json(
  zone: time_zone | undefined,
  session_start: number | undefined,
  tick_marks?: engine_time_tick_marks,
): string {
  const patch: Record<string, unknown> = {};
  if (tick_marks !== undefined) patch.tickMarks = tick_marks;
  if (zone !== undefined) {
    const transitions = resolve_time_zone(zone);
    patch.timeZone = transitions.length === 0 ? "UTC" : transitions;
  }
  if (session_start !== undefined) {
    if (!Number.isInteger(session_start)) {
      throw new AerisChartsError("invalid_options", "session_start must be a whole number of seconds");
    }
    patch.sessionStart = session_start;
  }
  return JSON.stringify(patch);
}

/**
 * Split `timeScale.timeZone` / `timeScale.sessionStart` / `timeScale.tickMarks` out of an engine
 * options patch: IANA names and mark times must be resolved here before the engine sees them, and
 * the three keys apply as one validated step. The caller's object is never mutated.
 */
export function split_exchange_time_options(options: Record<string, unknown>): {
  engine: Record<string, unknown>;
  zone: time_zone | undefined;
  session_start: number | undefined;
  tick_marks: unknown;
} {
  const time_scale = options.timeScale;
  if (time_scale === null || typeof time_scale !== "object") {
    return { engine: options, zone: undefined, session_start: undefined, tick_marks: undefined };
  }
  const { timeZone, sessionStart, tickMarks, ...rest } = time_scale as Record<string, unknown>;
  if (timeZone === undefined && sessionStart === undefined && tickMarks === undefined) {
    return { engine: options, zone: undefined, session_start: undefined, tick_marks: undefined };
  }
  const engine = { ...options };
  if (Object.keys(rest).length > 0) engine.timeScale = rest;
  else delete engine.timeScale;
  return {
    engine,
    zone: timeZone as time_zone | undefined,
    session_start: sessionStart as number | undefined,
    tick_marks: tickMarks,
  };
}

/** Calendar date of UTC-midnight seconds. */
export function business_day_of(seconds: number): business_day {
  const date = new Date(seconds * 1_000);
  return { year: date.getUTCFullYear(), month: date.getUTCMonth() + 1, day: date.getUTCDate() };
}

export function time_label_context_for(seconds: number, calendar_dates: boolean): time_label_context {
  return { business_day: calendar_dates ? business_day_of(seconds) : null };
}

const MAX_CACHED_FORMATTERS = 8;
const formatters = new Map<string, Intl.DateTimeFormat>();

function exchange_formatter(locale: string | undefined, with_time: boolean, seconds: boolean): Intl.DateTimeFormat {
  const key = `${locale ?? ""}|${with_time ? 1 : 0}|${seconds ? 1 : 0}`;
  let formatter = formatters.get(key);
  if (formatter === undefined) {
    formatter = new Intl.DateTimeFormat(locale, {
      timeZone: "UTC",
      year: "numeric",
      month: "short",
      day: "numeric",
      ...(with_time ? { hour: "2-digit", minute: "2-digit", hourCycle: "h23" } : {}),
      ...(with_time && seconds ? { second: "2-digit" } : {}),
    });
    if (formatters.size >= MAX_CACHED_FORMATTERS) {
      const oldest = formatters.keys().next().value;
      if (oldest !== undefined) formatters.delete(oldest);
    }
    formatters.set(key, formatter);
  }
  return formatter;
}

/**
 * Locale text for exchange-local wall-clock seconds (already shifted by the chart's schedule).
 * The formatter runs in UTC so the chart's exchange time, not the browser's, is shown.
 */
export function format_exchange_seconds(
  local_seconds: number,
  locale: string | undefined,
  with_time: boolean,
  seconds: boolean,
): string {
  return exchange_formatter(locale, with_time, seconds).format(new Date(local_seconds * 1_000));
}
