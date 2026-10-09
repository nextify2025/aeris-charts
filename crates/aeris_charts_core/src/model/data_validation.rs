//! Boundary data validation & sanitization (roadmap Phase A3).
//!
//! Real market feeds are messy: out-of-order rows, duplicate timestamps, NaN/Infinity, absurd
//! magnitudes, mismatched array lengths. The rest of the engine ([`super::data_layer::DataLayer`]
//! and [`super::plot_list::PlotList`]) assumes **ascending, unique, finite** input — its ordering
//! guard is a `debug_assert!` that is compiled out of the release wasm build, so bad data there
//! would silently corrupt indices or panic in `reindex_all`.
//!
//! This module is the single choke point that makes that assumption safe to hold. It independently
//! implements stricter production behavior: we *repair* what we can and *report* what we changed,
//! so a production embedder gets a
//! rendered chart plus a diagnostic instead of a thrown error or a dead canvas.
//!
//! Repair policy, in order:
//! 1. **Length mismatch** between the time and value columns is unrecoverable → [`Err`].
//! 2. **Invalid timestamps** reject the complete batch. Times must be finite, integral UTC
//!    seconds in the inclusive years 0000..9999 range.
//! 3. **Non-finite / out-of-safe-range values** (NaN, ±Inf, or |v| beyond [`MAX_SAFE_VALUE`])
//!    are dropped and counted — **except** a row whose four values are all
//!    NaN, which is kept as an explicit **whitespace** row (the reference's `{time}`-only item,
//!    data-consumer.ts `isWhitespaceData`): a real bar never has all four NaN, and for
//!    single-value series a NaN value is whitespace. Whitespace rows occupy their time point
//!    but draw nothing; genuinely malformed rows (a partial NaN set, ±Inf, out-of-range) are
//!    still dropped.
//! 4. **Unordered** rows are stably sorted by time (`reordered` flagged).
//! 5. **Duplicate** timestamps collapse **last-wins** (the last occurrence in the *source*
//!    input for that timestamp survives — matching a streaming `update()` overwriting a bar).

/// the reference's safe magnitude bound (`data-validators.ts`): `Number.MAX_SAFE_INTEGER / 100`.
pub const MAX_SAFE_VALUE: f64 = 9_007_199_254_740_991.0 / 100.0;
/// Symmetric lower bound.
pub const MIN_SAFE_VALUE: f64 = -MAX_SAFE_VALUE;

/// Earliest supported whole UTC second: 0000-01-01T00:00:00Z.
pub const MIN_TIMESTAMP: i64 = -62_167_219_200;
/// Latest supported whole UTC second: 9999-12-31T23:59:59Z.
pub const MAX_TIMESTAMP: i64 = 253_402_300_799;

/// Compact reason an input cannot be used as a canonical timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimestampErrorCategory {
    NonFinite,
    Fractional,
    OutOfRange,
}

/// A likely unit used by a timestamp that is outside the supported seconds range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimestampUnit {
    Milliseconds,
    Microseconds,
    Nanoseconds,
}

impl TimestampUnit {
    fn name(self) -> &'static str {
        match self {
            Self::Milliseconds => "milliseconds",
            Self::Microseconds => "microseconds",
            Self::Nanoseconds => "nanoseconds",
        }
    }
}

/// Structured timestamp failure with an optional wrong-unit hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimestampError {
    pub category: TimestampErrorCategory,
    pub likely_unit: Option<TimestampUnit>,
}

impl core::fmt::Display for TimestampError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "expected a finite whole number of UTC seconds in the inclusive range {MIN_TIMESTAMP}..{MAX_TIMESTAMP}"
        )?;
        match self.category {
            TimestampErrorCategory::NonFinite => write!(f, "; received a non-finite value"),
            TimestampErrorCategory::Fractional => write!(f, "; received fractional seconds"),
            TimestampErrorCategory::OutOfRange => {
                write!(f, "; received a value outside the supported range")?;
                if let Some(unit) = self.likely_unit {
                    write!(
                        f,
                        "; the value appears to be {}, convert it to UTC seconds before ingestion (timestamps are not auto-converted)",
                        unit.name()
                    )?;
                }
                Ok(())
            }
        }
    }
}

/// Validate one numeric timestamp without truncating or converting it.
pub fn validate_timestamp(time: f64) -> Result<i64, TimestampError> {
    if !time.is_finite() {
        return Err(TimestampError {
            category: TimestampErrorCategory::NonFinite,
            likely_unit: None,
        });
    }
    if time.fract() != 0.0 {
        return Err(TimestampError {
            category: TimestampErrorCategory::Fractional,
            likely_unit: None,
        });
    }
    if !(MIN_TIMESTAMP as f64..=MAX_TIMESTAMP as f64).contains(&time) {
        let likely_unit = [
            (1_000.0, 1_000_000_000_000.0, TimestampUnit::Milliseconds),
            (
                1_000_000.0,
                1_000_000_000_000_000.0,
                TimestampUnit::Microseconds,
            ),
            (
                1_000_000_000.0,
                1_000_000_000_000_000_000.0,
                TimestampUnit::Nanoseconds,
            ),
        ]
        .into_iter()
        .find_map(|(scale, minimum_magnitude, unit)| {
            let seconds = time / scale;
            (time.abs() >= minimum_magnitude
                && (MIN_TIMESTAMP as f64..=MAX_TIMESTAMP as f64).contains(&seconds))
            .then_some(unit)
        });
        return Err(TimestampError {
            category: TimestampErrorCategory::OutOfRange,
            likely_unit,
        });
    }
    Ok(time as i64)
}

/// What the sanitizer had to change to make the data ingestible.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ValidationReport {
    /// Rows dropped for a non-finite or out-of-range value. Invalid times reject the transaction.
    pub dropped_invalid: usize,
    /// Invalid rows dropped specifically because a value was NaN/infinite (excluding an all-NaN
    /// whitespace row).
    pub dropped_non_finite: usize,
    /// Invalid rows dropped because a finite value exceeded the supported safe range.
    pub dropped_out_of_range: usize,
    /// Rows discarded because a later row shared their timestamp (last-wins).
    pub dropped_duplicate: usize,
    /// The input was not already ascending and had to be sorted.
    pub reordered: bool,
    /// Rows that made it into the sanitized output.
    pub accepted: usize,
    /// Accepted finite rows whose OHLC relationships are impossible. Values are preserved; the
    /// host chooses whether to warn or reject.
    pub semantic_anomalies: usize,
}

impl ValidationReport {
    /// True when the input was already clean (nothing dropped or reordered).
    pub fn is_clean(&self) -> bool {
        self.dropped_invalid == 0
            && self.dropped_duplicate == 0
            && !self.reordered
            && self.semantic_anomalies == 0
    }
}

/// Structural problems the sanitizer cannot repair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// The caller supplied an identity that this chart has never issued.
    UnknownSeries(u32),
    /// The caller supplied an identity whose series has already been removed.
    StaleSeries(u32),
    /// The caller tried to replace an engine-owned advanced series with generic OHLC rows.
    UnsupportedSeriesData(u32),
    /// A timestamp failed the shared whole-UTC-seconds contract.
    InvalidTimestamp { index: usize, error: TimestampError },
    /// The time column and the value columns have differing lengths.
    LengthMismatch {
        times: usize,
        open: usize,
        high: usize,
        low: usize,
        close: usize,
    },
    /// A per-point color channel's length differs from the time column.
    ColorLengthMismatch {
        times: usize,
        channel: &'static str,
        colors: usize,
    },
}

impl core::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ValidationError::UnknownSeries(id) => write!(f, "unknown series id {id}"),
            ValidationError::StaleSeries(id) => write!(f, "stale series id {id}"),
            ValidationError::UnsupportedSeriesData(id) => write!(
                f,
                "series {id} owns its source data; use its series-specific ingestion API"
            ),
            ValidationError::InvalidTimestamp { index, error } => {
                write!(f, "invalid timestamp at row {index}: {error}")
            }
            ValidationError::LengthMismatch {
                times,
                open,
                high,
                low,
                close,
            } => write!(
                f,
                "time/OHLC arrays must have equal length (times={times}, open={open}, high={high}, low={low}, close={close})"
            ),
            ValidationError::ColorLengthMismatch {
                times,
                channel,
                colors,
            } => write!(
                f,
                "point-color channel must match the row count (channel={channel}, times={times}, colors={colors})"
            ),
        }
    }
}

/// Ascending, unique, finite OHLC rows ready for [`super::data_layer::DataLayer::set_data`].
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SanitizedOhlc {
    pub times: Vec<i64>,
    pub open: Vec<f64>,
    pub high: Vec<f64>,
    pub low: Vec<f64>,
    pub close: Vec<f64>,
    pub report: ValidationReport,
}

fn safe(v: f64) -> bool {
    v.is_finite() && (MIN_SAFE_VALUE..=MAX_SAFE_VALUE).contains(&v)
}

/// Whether the four values form an explicit whitespace row (reference `{time}`-only item): all
/// four are NaN. A real bar never has all four NaN; single-value series alias one value
/// into all four slots, so a NaN value is whitespace there as well.
pub fn is_whitespace_values(values: [f64; 4]) -> bool {
    values.iter().all(|v| v.is_nan())
}

/// Whether finite `[open, high, low, close]` values violate the OHLC envelope. The sanitizer
/// reports but never repairs or drops these rows.
pub fn is_semantic_ohlc_anomaly([open, high, low, close]: [f64; 4]) -> bool {
    high < low || high < open || high < close || low > open || low > close
}

/// Sanitize parallel time/OHLC columns into ascending, unique, finite rows.
///
/// `times` are whole UTC seconds as `f64` at the JS boundary. Any invalid timestamp rejects the
/// complete batch. Single-value series pass the same value in all four columns, so this covers
/// line/area/histogram too.
pub fn sanitize_ohlc(
    times: &[f64],
    open: &[f64],
    high: &[f64],
    low: &[f64],
    close: &[f64],
) -> Result<SanitizedOhlc, ValidationError> {
    sanitize_rows(times, open, high, low, close, |_| ()).map(|(out, _)| out)
}

/// [`sanitize_ohlc`] output plus the per-row color channels carried through the same repair.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SanitizedOhlcStyled {
    pub data: SanitizedOhlc,
    /// The three reference data-item color channels (body/wick/border) after the repair pipeline;
    /// each is empty (channel absent) or aligned with `data`'s rows.
    pub colors: [Vec<u32>; 3],
}

/// [`sanitize_ohlc`] carrying per-row data-item color channels (reference series-bar-colorer.ts).
/// Every present channel must match the time column's length (a mismatch is unrecoverable,
/// like the OHLC columns); within a channel, `0` means "no override at this row". The repair
/// policy treats the channels as part of their row: invalid rows drop them, the stable sort
/// moves them, and the last-wins dedupe keeps the winning row's channels.
pub fn sanitize_ohlc_styled(
    times: &[f64],
    open: &[f64],
    high: &[f64],
    low: &[f64],
    close: &[f64],
    colors: [Option<Vec<u32>>; 3],
) -> Result<SanitizedOhlcStyled, ValidationError> {
    const CHANNEL_NAMES: [&str; 3] = ["body", "wick", "border"];
    for (name, channel) in CHANNEL_NAMES.into_iter().zip(&colors) {
        if let Some(channel) = channel
            && channel.len() != times.len()
        {
            return Err(ValidationError::ColorLengthMismatch {
                times: times.len(),
                channel: name,
                colors: channel.len(),
            });
        }
    }
    let present = [
        colors[0].is_some(),
        colors[1].is_some(),
        colors[2].is_some(),
    ];
    let (data, payloads) = sanitize_rows(times, open, high, low, close, |row| {
        [
            colors[0].as_ref().map_or(0, |c| c[row]),
            colors[1].as_ref().map_or(0, |c| c[row]),
            colors[2].as_ref().map_or(0, |c| c[row]),
        ]
    })?;
    let mut channels: [Vec<u32>; 3] = [vec![], vec![], vec![]];
    for i in 0..3 {
        if present[i] {
            channels[i] = payloads.iter().map(|p| p[i]).collect();
        }
    }
    Ok(SanitizedOhlcStyled {
        data,
        colors: channels,
    })
}

/// Shared repair pipeline for [`sanitize_ohlc`] and [`sanitize_ohlc_styled`]: applies the
/// drop-invalid → stable-sort → last-wins-dedupe policy, carrying a per-row payload through
/// the same fate (the payload follows the winning row).
fn sanitize_rows<P: Clone>(
    times: &[f64],
    open: &[f64],
    high: &[f64],
    low: &[f64],
    close: &[f64],
    payload_of: impl Fn(usize) -> P,
) -> Result<(SanitizedOhlc, Vec<P>), ValidationError> {
    let n = times.len();
    if open.len() != n || high.len() != n || low.len() != n || close.len() != n {
        return Err(ValidationError::LengthMismatch {
            times: n,
            open: open.len(),
            high: high.len(),
            low: low.len(),
            close: close.len(),
        });
    }

    let valid_times = times
        .iter()
        .copied()
        .enumerate()
        .map(|(index, time)| {
            validate_timestamp(time)
                .map_err(|error| ValidationError::InvalidTimestamp { index, error })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut report = ValidationReport::default();

    // 1. Keep only finite, in-range value rows; remember source order for stable sort + last-wins.
    //    All-NaN rows survive as explicit whitespace (reference `{time}`-only items).
    let mut rows: Vec<(i64, [f64; 4], usize, P)> = Vec::with_capacity(n);
    for i in 0..n {
        let v = [open[i], high[i], low[i], close[i]];
        let whitespace = is_whitespace_values(v);
        let non_finite = !whitespace && v.iter().any(|value| !value.is_finite());
        let out_of_range = !whitespace
            && !non_finite
            && v.iter()
                .any(|value| !(MIN_SAFE_VALUE..=MAX_SAFE_VALUE).contains(value));
        if non_finite || out_of_range {
            report.dropped_invalid += 1;
            report.dropped_non_finite += usize::from(non_finite);
            report.dropped_out_of_range += usize::from(out_of_range);
            continue;
        }
        rows.push((valid_times[i], v, i, payload_of(i)));
    }

    // 2. Detect out-of-order before sorting (so `reordered` reflects the caller's input).
    report.reordered = rows.windows(2).any(|w| w[0].0 > w[1].0);
    if report.reordered {
        // Stable by time so that, within a duplicate group, source order is preserved and the
        // last source occurrence is the one we keep below.
        rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.2.cmp(&b.2)));
    }

    // 3. Collapse duplicate timestamps, last-wins. `rows` is time-ascending; equal times are in
    //    ascending source-index order, so the last of each run is the latest-provided bar.
    let mut out = SanitizedOhlc::default();
    out.times.reserve(rows.len());
    let mut payloads: Vec<P> = Vec::with_capacity(rows.len());
    for (t, v, _, payload) in rows {
        let semantic_anomaly = !is_whitespace_values(v) && is_semantic_ohlc_anomaly(v);
        if out.times.last() == Some(&t) {
            report.dropped_duplicate += 1;
            let last = out.times.len() - 1;
            let previous = [
                out.open[last],
                out.high[last],
                out.low[last],
                out.close[last],
            ];
            if !is_whitespace_values(previous) && is_semantic_ohlc_anomaly(previous) {
                report.semantic_anomalies -= 1;
            }
            out.open[last] = v[0];
            out.high[last] = v[1];
            out.low[last] = v[2];
            out.close[last] = v[3];
            payloads[last] = payload;
        } else {
            out.times.push(t);
            out.open.push(v[0]);
            out.high.push(v[1]);
            out.low.push(v[2]);
            out.close.push(v[3]);
            payloads.push(payload);
        }
        report.semantic_anomalies += usize::from(semantic_anomaly);
    }

    report.accepted = out.times.len();
    out.report = report;
    Ok((out, payloads))
}

/// Owned-input variant used by typed-array hosts. Clean integer-timestamp feeds take ownership of
/// their columns without the intermediate row matrix; other feeds fall back to the full validator
/// and value-repair sanitizer. This keeps the common ingestion path to one JS→WASM copy.
pub fn sanitize_ohlc_owned(
    times: Vec<f64>,
    open: Vec<f64>,
    high: Vec<f64>,
    low: Vec<f64>,
    close: Vec<f64>,
) -> Result<SanitizedOhlc, ValidationError> {
    let n = times.len();
    if open.len() != n || high.len() != n || low.len() != n || close.len() != n {
        return Err(ValidationError::LengthMismatch {
            times: n,
            open: open.len(),
            high: high.len(),
            low: low.len(),
            close: close.len(),
        });
    }
    let mut semantic_anomalies = 0;
    let mut clean = true;
    for i in 0..n {
        let time = times[i];
        let values = [open[i], high[i], low[i], close[i]];
        clean &= validate_timestamp(time).is_ok()
            && (i == 0 || times[i - 1] < time)
            && (is_whitespace_values(values) || values.iter().copied().all(safe));
        if !is_whitespace_values(values)
            && values.iter().copied().all(safe)
            && is_semantic_ohlc_anomaly(values)
        {
            semantic_anomalies += 1;
        }
    }
    if clean {
        let accepted = times.len();
        return Ok(SanitizedOhlc {
            times: times.into_iter().map(|t| t as i64).collect(),
            open,
            high,
            low,
            close,
            report: ValidationReport {
                accepted,
                semantic_anomalies,
                ..ValidationReport::default()
            },
        });
    }
    sanitize_ohlc(&times, &open, &high, &low, &close)
}

/// Sanitize a single streaming point. Returns `None` (with no effect on the chart) when the point
/// is non-finite or out of range, so a bad tick is dropped instead of corrupting the series.
/// An all-NaN value set is a valid whitespace update (reference `series.update` with a `{time}`-only
/// item replaces the bar with whitespace); a partial NaN set or ±Inf is a bad tick.
pub fn sanitize_point(time: f64, values: [f64; 4]) -> Option<(i64, [f64; 4])> {
    let time = validate_timestamp(time).ok()?;
    if !(is_whitespace_values(values) || values.iter().copied().all(safe)) {
        return None;
    }
    Some((time, values))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ohlc(times: &[f64], v: &[f64]) -> Result<SanitizedOhlc, ValidationError> {
        // single-value convenience: all four columns equal
        sanitize_ohlc(times, v, v, v, v)
    }

    #[test]
    fn clean_data_passes_through_untouched() {
        let s = ohlc(&[1.0, 2.0, 3.0], &[10.0, 20.0, 30.0]).unwrap();
        assert_eq!(s.times, [1, 2, 3]);
        assert_eq!(s.close, [10.0, 20.0, 30.0]);
        assert!(s.report.is_clean());
        assert_eq!(s.report.accepted, 3);
    }

    #[test]
    fn length_mismatch_is_an_error() {
        let err = sanitize_ohlc(&[1.0, 2.0], &[1.0], &[1.0], &[1.0], &[1.0]).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::LengthMismatch {
                times: 2,
                open: 1,
                ..
            }
        ));
    }

    #[test]
    fn drops_non_finite_and_out_of_range() {
        let times = [1.0, 2.0, 3.0, 4.0, 5.0];
        let vals = [10.0, f64::NAN, f64::INFINITY, MAX_SAFE_VALUE * 2.0, 50.0];
        let s = ohlc(&times, &vals).unwrap();
        // single-value columns: row 2 is all-NaN → explicit whitespace (kept); rows 3,4 drop
        assert_eq!(s.times, [1, 2, 5]);
        assert_eq!(s.close[0], 10.0);
        assert!(s.close[1].is_nan());
        assert_eq!(s.close[2], 50.0);
        assert_eq!(s.report.dropped_invalid, 2);
        assert_eq!(s.report.dropped_non_finite, 1);
        assert_eq!(s.report.dropped_out_of_range, 1);
    }

    #[test]
    fn all_nan_rows_are_kept_as_whitespace() {
        // reference `{time}`-only items: an all-NaN row is explicit whitespace, not invalid data.
        let nan = f64::NAN;
        let s = sanitize_ohlc(
            &[1.0, 2.0, 3.0],
            &[10.0, nan, 30.0],
            &[10.0, nan, 30.0],
            &[10.0, nan, 30.0],
            &[10.0, nan, 30.0],
        )
        .unwrap();
        assert_eq!(s.times, [1, 2, 3]);
        assert!(s.close[1].is_nan());
        assert!(s.open[1].is_nan() && s.high[1].is_nan() && s.low[1].is_nan());
        assert!(s.report.is_clean());
        assert_eq!(s.report.accepted, 3);

        // A partial NaN set is genuinely malformed and still drops, as do ±Inf rows.
        let s = sanitize_ohlc(
            &[1.0, 2.0, 3.0, 4.0],
            &[10.0, nan, 30.0, 40.0],
            &[10.0, 1.0, 30.0, 40.0],
            &[10.0, 1.0, 30.0, 40.0],
            &[10.0, 1.0, 30.0, 40.0],
        )
        .unwrap();
        assert_eq!(s.times, [1, 3, 4]);
        assert_eq!(s.report.dropped_invalid, 1);
        let s = sanitize_ohlc(
            &[1.0, 2.0],
            &[10.0, f64::INFINITY],
            &[10.0, f64::INFINITY],
            &[10.0, f64::INFINITY],
            &[10.0, f64::INFINITY],
        )
        .unwrap();
        assert_eq!(s.times, [1]);
        assert_eq!(s.report.dropped_invalid, 1);
    }

    #[test]
    fn whitespace_rows_sort_dedupe_and_carry_colors_like_real_rows() {
        let nan = f64::NAN;
        let s = sanitize_ohlc_styled(
            &[2.0, 1.0, 3.0],
            &[20.0, 10.0, nan],
            &[20.0, 10.0, nan],
            &[20.0, 10.0, nan],
            &[20.0, 10.0, nan],
            [Some(vec![22, 11, 33]), None, None],
        )
        .unwrap();
        assert_eq!(s.data.times, [1, 2, 3]);
        assert!(s.data.close[2].is_nan());
        assert_eq!(s.colors[0], [11, 22, 33]); // the whitespace row keeps its channel slot
    }

    #[test]
    fn sanitize_point_accepts_whitespace_ticks() {
        let nan = f64::NAN;
        let ws = sanitize_point(2.0, [nan, nan, nan, nan]).unwrap();
        assert_eq!(ws.0, 2);
        assert!(ws.1.iter().all(|v| v.is_nan()));
        // partial NaN / Inf ticks are still dropped
        assert!(sanitize_point(2.0, [1.0, nan, 1.0, 1.0]).is_none());
        assert!(sanitize_point(2.0, [f64::INFINITY; 4]).is_none());
    }

    #[test]
    fn invalid_timestamp_rejects_the_complete_batch() {
        let err = ohlc(&[1.0, f64::NAN, 3.0], &[10.0, 20.0, 30.0]).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::InvalidTimestamp {
                index: 1,
                error: TimestampError {
                    category: TimestampErrorCategory::NonFinite,
                    ..
                }
            }
        ));
    }

    #[test]
    fn sorts_unordered_input() {
        let s = ohlc(&[3.0, 1.0, 2.0], &[30.0, 10.0, 20.0]).unwrap();
        assert_eq!(s.times, [1, 2, 3]);
        assert_eq!(s.close, [10.0, 20.0, 30.0]);
        assert!(s.report.reordered);
        assert_eq!(s.report.dropped_duplicate, 0);
    }

    #[test]
    fn dedupes_last_wins_in_order() {
        // duplicate time 2 appears twice; the later value (25) must win
        let s = ohlc(&[1.0, 2.0, 2.0, 3.0], &[10.0, 20.0, 25.0, 30.0]).unwrap();
        assert_eq!(s.times, [1, 2, 3]);
        assert_eq!(s.close, [10.0, 25.0, 30.0]);
        assert_eq!(s.report.dropped_duplicate, 1);
        assert!(!s.report.reordered);
    }

    #[test]
    fn dedupes_last_wins_after_sort() {
        // out of order AND duplicated: source indices break the tie so the later input wins
        // times: 2(a=20) , 1(=10) , 2(b=99) -> sorted stable: 1, 2a, 2b -> keep 2b
        let s = ohlc(&[2.0, 1.0, 2.0], &[20.0, 10.0, 99.0]).unwrap();
        assert_eq!(s.times, [1, 2]);
        assert_eq!(s.close, [10.0, 99.0]);
        assert!(s.report.reordered);
        assert_eq!(s.report.dropped_duplicate, 1);
    }

    #[test]
    fn empty_input_is_clean_empty_output() {
        let s = ohlc(&[], &[]).unwrap();
        assert!(s.times.is_empty());
        assert!(s.report.is_clean());
        assert_eq!(s.report.accepted, 0);
    }

    #[test]
    fn full_ohlc_columns_are_kept_independent() {
        let s = sanitize_ohlc(
            &[1.0, 2.0],
            &[1.0, 2.0],
            &[5.0, 6.0],
            &[0.5, 1.5],
            &[3.0, 4.0],
        )
        .unwrap();
        assert_eq!(s.open, [1.0, 2.0]);
        assert_eq!(s.high, [5.0, 6.0]);
        assert_eq!(s.low, [0.5, 1.5]);
        assert_eq!(s.close, [3.0, 4.0]);
    }

    #[test]
    fn impossible_ohlc_is_preserved_and_reported() {
        let s = sanitize_ohlc(
            &[1.0, 2.0, 3.0],
            &[10.0, 10.0, 10.0],
            &[9.0, 12.0, 12.0],
            &[8.0, 11.0, 8.0],
            &[8.5, 11.5, 13.0],
        )
        .unwrap();
        assert_eq!(s.report.accepted, 3);
        assert_eq!(s.report.semantic_anomalies, 3);
        assert_eq!(s.open, [10.0, 10.0, 10.0]);
        assert!(!s.report.is_clean());

        let valid =
            sanitize_ohlc_owned(vec![1.0], vec![10.0], vec![12.0], vec![8.0], vec![11.0]).unwrap();
        assert_eq!(valid.report.semantic_anomalies, 0);
        assert!(valid.report.is_clean());
    }

    #[test]
    fn deduplication_reports_only_the_winning_rows_semantics() {
        let s = sanitize_ohlc(
            &[1.0, 1.0],
            &[10.0, 10.0],
            &[9.0, 12.0],
            &[8.0, 8.0],
            &[11.0, 11.0],
        )
        .unwrap();
        assert_eq!(s.report.dropped_duplicate, 1);
        assert_eq!(s.report.semantic_anomalies, 0);
        assert_eq!(s.high, [12.0]);
    }

    #[test]
    fn timestamp_validator_accepts_exact_boundaries_and_preserves_valid_input() {
        assert_eq!(validate_timestamp(MIN_TIMESTAMP as f64), Ok(MIN_TIMESTAMP));
        assert_eq!(validate_timestamp(MAX_TIMESTAMP as f64), Ok(MAX_TIMESTAMP));
        assert_eq!(validate_timestamp(1_725_000_000.0), Ok(1_725_000_000));
    }

    #[test]
    fn timestamp_validator_rejects_non_finite_fractional_and_out_of_range_values() {
        assert_eq!(
            validate_timestamp(f64::INFINITY).unwrap_err().category,
            TimestampErrorCategory::NonFinite
        );
        assert_eq!(
            validate_timestamp(1.5).unwrap_err().category,
            TimestampErrorCategory::Fractional
        );
        assert_eq!(
            validate_timestamp(MIN_TIMESTAMP as f64 - 1.0)
                .unwrap_err()
                .category,
            TimestampErrorCategory::OutOfRange
        );
        assert_eq!(
            validate_timestamp(MAX_TIMESTAMP as f64 + 1.0)
                .unwrap_err()
                .category,
            TimestampErrorCategory::OutOfRange
        );
    }

    #[test]
    fn timestamp_validator_reports_likely_wrong_units_without_converting() {
        let seconds = 1_725_000_000.0;
        for (value, unit) in [
            (seconds * 1_000.0, TimestampUnit::Milliseconds),
            (seconds * 1_000_000.0, TimestampUnit::Microseconds),
            (seconds * 1_000_000_000.0, TimestampUnit::Nanoseconds),
        ] {
            let error = validate_timestamp(value).unwrap_err();
            assert_eq!(error.category, TimestampErrorCategory::OutOfRange);
            assert_eq!(error.likely_unit, Some(unit));
            assert!(error.to_string().contains(unit.name()));
        }

        assert_eq!(
            validate_timestamp(MAX_TIMESTAMP as f64 + 1.0)
                .unwrap_err()
                .likely_unit,
            None
        );
    }

    #[test]
    fn borrowed_owned_and_styled_batches_share_atomic_timestamp_validation() {
        let times = [MIN_TIMESTAMP as f64, MAX_TIMESTAMP as f64 + 1.0];
        let values = [10.0, 20.0];
        let assert_second_row = |error| {
            assert!(matches!(
                error,
                ValidationError::InvalidTimestamp { index: 1, .. }
            ));
        };

        assert_second_row(ohlc(&times, &values).unwrap_err());
        assert_second_row(
            sanitize_ohlc_owned(
                times.to_vec(),
                values.to_vec(),
                values.to_vec(),
                values.to_vec(),
                values.to_vec(),
            )
            .unwrap_err(),
        );
        assert_second_row(
            sanitize_ohlc_styled(
                &times,
                &values,
                &values,
                &values,
                &values,
                [Some(vec![1, 2]), None, None],
            )
            .unwrap_err(),
        );
    }

    #[test]
    fn sanitize_point_rejects_bad_ticks() {
        assert!(sanitize_point(f64::NAN, [1.0, 1.0, 1.0, 1.0]).is_none());
        assert!(sanitize_point(1.0, [1.0, f64::INFINITY, 1.0, 1.0]).is_none());
        assert!(sanitize_point(1.5, [1.0, 2.0, 0.5, 1.5]).is_none());
        assert!(sanitize_point(MAX_TIMESTAMP as f64 + 1.0, [1.0; 4]).is_none());
    }

    #[test]
    fn styled_dedupe_last_wins_keeps_the_winning_color() {
        // duplicate time 2 appears twice; the later row (value 25, color 99) must win.
        let s = sanitize_ohlc_styled(
            &[1.0, 2.0, 2.0, 3.0],
            &[10.0, 20.0, 25.0, 30.0],
            &[10.0, 20.0, 25.0, 30.0],
            &[10.0, 20.0, 25.0, 30.0],
            &[10.0, 20.0, 25.0, 30.0],
            [Some(vec![10, 20, 99, 30]), None, None],
        )
        .unwrap();
        assert_eq!(s.data.times, [1, 2, 3]);
        assert_eq!(s.data.close, [10.0, 25.0, 30.0]);
        assert_eq!(s.colors[0], [10, 99, 30]);
        assert!(s.colors[1].is_empty() && s.colors[2].is_empty());
        assert_eq!(s.data.report.dropped_duplicate, 1);
    }

    #[test]
    fn styled_sort_carries_colors_with_their_rows() {
        // out of order AND duplicated: stable sort keeps the last source occurrence of
        // time 2 (value 99, color 77).
        let s = sanitize_ohlc_styled(
            &[2.0, 1.0, 2.0],
            &[20.0, 10.0, 99.0],
            &[20.0, 10.0, 99.0],
            &[20.0, 10.0, 99.0],
            &[20.0, 10.0, 99.0],
            [Some(vec![55, 11, 77]), Some(vec![5, 1, 7]), None],
        )
        .unwrap();
        assert_eq!(s.data.times, [1, 2]);
        assert_eq!(s.data.close, [10.0, 99.0]);
        assert_eq!(s.colors[0], [11, 77]);
        assert_eq!(s.colors[1], [1, 7]);
        assert!(s.data.report.reordered);
    }

    #[test]
    fn styled_drops_colors_of_invalid_rows_and_checks_lengths() {
        let s = sanitize_ohlc_styled(
            &[1.0, 2.0, 3.0],
            &[10.0, f64::NAN, 30.0],
            &[10.0, 1.0, 30.0],
            &[10.0, 1.0, 30.0],
            &[10.0, 1.0, 30.0],
            [Some(vec![10, 20, 30]), None, None],
        )
        .unwrap();
        assert_eq!(s.data.times, [1, 3]);
        assert_eq!(s.colors[0], [10, 30]);

        let err = sanitize_ohlc_styled(
            &[1.0, 2.0],
            &[1.0, 2.0],
            &[1.0, 2.0],
            &[1.0, 2.0],
            &[1.0, 2.0],
            [Some(vec![1]), None, None],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ValidationError::ColorLengthMismatch {
                times: 2,
                channel: "body",
                colors: 1
            }
        ));
    }

    #[test]
    fn owned_clean_input_avoids_repair_path() {
        let s = sanitize_ohlc_owned(
            vec![1.0, 2.0],
            vec![1.0, 2.0],
            vec![2.0, 3.0],
            vec![0.0, 1.0],
            vec![1.5, 2.5],
        )
        .unwrap();
        assert!(s.report.is_clean());
        assert_eq!(s.times, [1, 2]);
    }
}
