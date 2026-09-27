//! Time identity for drawing anchors on ordinary (non-sequence) time axes.
//!
//! Logical index `i` sits at `times[i]` (the merged union in UTC seconds). A fractional position
//! interpolates linearly between the neighbouring bar times, and a position beyond the data
//! extrapolates with the prevailing bar interval at that edge. [`logical_at_time`] is the exact
//! inverse, so an anchor's time survives an interval switch, a reload, or a restore into a
//! different history window. All math stays in `f64`; work is bounded by one binary search plus
//! a fixed-size interval sample per call. Results outside the persisted value range
//! ([`MAX_SAFE_VALUE`]) are reported as unplaceable, so a derived time or logical position can
//! always be exported and imported again.

use aeris_charts_core::model::data_layer::MergedTimeMapping;
use aeris_charts_core::model::data_validation::MAX_SAFE_VALUE;

fn bounded(value: f64) -> Option<f64> {
    (value.is_finite() && value.abs() <= MAX_SAFE_VALUE).then_some(value)
}

/// Bar spacings sampled at one edge to find the prevailing interval.
const PREVAILING_INTERVAL_SAMPLE: usize = 16;

/// The most frequent positive bar spacing among the last (`at_end`) or first
/// [`PREVAILING_INTERVAL_SAMPLE`] spacings. Ties prefer the smaller spacing, so session and
/// weekend gaps never become the extrapolation step. `None` for fewer than two bars.
pub(crate) fn prevailing_interval(times: &[i64], at_end: bool) -> Option<f64> {
    if times.len() < 2 {
        return None;
    }
    let count = (times.len() - 1).min(PREVAILING_INTERVAL_SAMPLE);
    let mut spacings = [0i64; PREVAILING_INTERVAL_SAMPLE];
    for (k, spacing) in spacings.iter_mut().take(count).enumerate() {
        let (a, b) = if at_end {
            (times[times.len() - 2 - k], times[times.len() - 1 - k])
        } else {
            (times[k], times[k + 1])
        };
        *spacing = b.saturating_sub(a);
    }
    let spacings = &mut spacings[..count];
    spacings.sort_unstable();
    let (mut best, mut best_run, mut start) = (spacings[0], 0usize, 0usize);
    while start < count {
        let mut end = start;
        while end < count && spacings[end] == spacings[start] {
            end += 1;
        }
        if end - start > best_run {
            best_run = end - start;
            best = spacings[start];
        }
        start = end;
    }
    (best > 0).then_some(best as f64)
}

/// The anchor time (UTC seconds) of a logical position on `times`.
pub(crate) fn time_at_logical(times: &[i64], logical: f64) -> Option<f64> {
    if !logical.is_finite() || times.is_empty() {
        return None;
    }
    let last = (times.len() - 1) as f64;
    if logical < 0.0 {
        return bounded(times[0] as f64 + logical * prevailing_interval(times, false)?);
    }
    if logical > last {
        return bounded(
            times[times.len() - 1] as f64 + (logical - last) * prevailing_interval(times, true)?,
        );
    }
    let index = logical.floor();
    let i = index as usize;
    let fraction = logical - index;
    if fraction == 0.0 {
        return Some(times[i] as f64);
    }
    let (a, b) = (times[i] as f64, times[i + 1] as f64);
    Some(a + fraction * (b - a))
}

/// The logical position of an anchor time on `times` (the inverse of [`time_at_logical`]).
pub(crate) fn logical_at_time(times: &[i64], time: f64) -> Option<f64> {
    if !time.is_finite() || times.is_empty() {
        return None;
    }
    let first = times[0] as f64;
    let last_time = times[times.len() - 1] as f64;
    if time < first {
        return bounded((time - first) / prevailing_interval(times, false)?);
    }
    if time > last_time {
        return bounded(
            (times.len() - 1) as f64 + (time - last_time) / prevailing_interval(times, true)?,
        );
    }
    let i = times.partition_point(|&t| (t as f64) <= time) - 1;
    let a = times[i] as f64;
    if a == time || i + 1 == times.len() {
        return Some(i as f64);
    }
    let b = times[i + 1] as f64;
    Some(i as f64 + (time - a) / (b - a))
}

/// Whether a same-resolution translation moved the shared bars to higher indices: history was
/// prepended, or the window moved to earlier times.
pub(crate) fn translation_prepends(mapping: &MergedTimeMapping) -> bool {
    mapping.is_translation()
        && mapping
            .common_old_extent()
            .is_some_and(|(first, _)| mapping.map_logical(first as f64) > first as f64)
}

/// Rebase one anchor across a merged-union change. Anchors inside the common-timestamp extent
/// keep the exact merged mapping. A same-resolution translation keeps bar-count extrapolation
/// outside it (a retention trim or a later window never makes live drawings jump across session
/// gaps), except left of the shared extent when history was prepended: those anchors sat at an
/// extrapolated time (often one an interval switch resolved there), so they resolve that time on
/// the new bars instead of drifting by the gaps the new history contains. Otherwise (an interval
/// switch or a reload with no shared stamps) the anchor's time on the old axis is resolved on the
/// new axis; without a derivable time the existing mapping applies unchanged.
pub(crate) fn rebase_logical(mapping: &MergedTimeMapping, new_times: &[i64], logical: f64) -> f64 {
    if !logical.is_finite() {
        return logical;
    }
    if let Some((first, last)) = mapping.common_old_extent() {
        let (first, last) = (first as f64, last as f64);
        let by_bar_count =
            mapping.is_translation() && !(logical < first && translation_prepends(mapping));
        if by_bar_count || (first <= logical && logical <= last) {
            return mapping.map_logical(logical);
        }
    }
    time_at_logical(mapping.old_times(), logical)
        .and_then(|time| logical_at_time(new_times, time))
        .unwrap_or_else(|| mapping.map_logical(logical))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3_600;
    const DAY: i64 = 86_400;

    #[test]
    fn prevailing_interval_ignores_session_and_weekend_gaps() {
        // Mon..Fri, Mon..Fri daily bars: the 3-day weekend spacing never wins.
        let days = [0, 1, 2, 3, 4, 7, 8, 9, 10, 11].map(|day| day * DAY);
        assert_eq!(prevailing_interval(&days, true), Some(DAY as f64));
        assert_eq!(prevailing_interval(&days, false), Some(DAY as f64));
        // A-share hourly stamps: 10:30, 11:30, 14:00, 15:00, then the next day.
        let hourly = [10.5, 11.5, 14.0, 15.0, 34.5, 35.5, 38.0, 39.0]
            .map(|hour: f64| (hour * HOUR as f64) as i64);
        assert_eq!(prevailing_interval(&hourly, true), Some(HOUR as f64));
        assert_eq!(prevailing_interval(&[5], true), None);
    }

    #[test]
    fn time_and_logical_are_inverse_inside_and_beyond_the_data() {
        let times = (0..10).map(|i| i * HOUR).collect::<Vec<_>>();
        for logical in [-3.25, -1.0, 0.0, 0.5, 4.0, 4.617, 9.0, 12.5] {
            let time = time_at_logical(&times, logical).unwrap();
            let back = logical_at_time(&times, time).unwrap();
            assert!(
                (back - logical).abs() < 1e-9,
                "{logical} -> {time} -> {back}"
            );
        }
        assert_eq!(time_at_logical(&times, 2.0), Some(2.0 * HOUR as f64));
        assert_eq!(time_at_logical(&times, 11.0), Some(11.0 * HOUR as f64));
        assert_eq!(logical_at_time(&times, -2.0 * HOUR as f64), Some(-2.0));
        assert_eq!(time_at_logical(&[], 0.0), None);
        assert_eq!(time_at_logical(&[7], 1.0), None);
        assert_eq!(time_at_logical(&[7], 0.0), Some(7.0));
    }

    #[test]
    fn interval_switch_resolves_by_time_not_by_bar_count() {
        // Old: 1m bars 10:00..10:59; new: hourly bars 08:00..13:00 sharing only 10:00.
        let minutes = (0..60).map(|m| 10 * HOUR + m * 60).collect::<Vec<_>>();
        let hours = (8..=13).map(|h| h * HOUR).collect::<Vec<_>>();
        let mut dl = aeris_charts_core::model::data_layer::DataLayer::new();
        let id = dl.add_series();
        let ones = |n: usize| vec![1.0; n];
        assert!(dl.set_data(id, minutes, ones(60), ones(60), ones(60), ones(60)));
        dl.begin_merged_time_transaction();
        assert!(dl.set_data(id, hours.clone(), ones(6), ones(6), ones(6), ones(6)));
        let mapping = dl.take_merged_time_mapping().unwrap();
        // 10:37 lands 37/60 of the way from the 10:00 bar toward the 11:00 bar.
        let rebased = rebase_logical(&mapping, &hours, 37.0);
        assert!((rebased - (2.0 + 37.0 / 60.0)).abs() < 1e-12);
        // A future anchor 10 minutes past 10:59 extrapolates in time, not in bars.
        let future = rebase_logical(&mapping, &hours, 69.0);
        assert!((future - (2.0 + 69.0 / 60.0)).abs() < 1e-12);
    }
}
