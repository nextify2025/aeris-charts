//! Custom-series item alignment (plugin platform Phase C-c), pure and host-testable.
//!
//! A custom series keeps two aligned views of its rows: the engine's data-layer rows carry
//! times only (whitespace-style), and the host's item list carries the raw plugin items
//! (`{time, ...}` JS objects). The engine's sort/dedupe/update rules (data_validation.rs
//! `sanitize_rows`, data_layer.rs `update_styled`) must hold identically across both views so
//! the row ↔ item mapping never drifts; this module is that mapping's single source of truth.

use aeris_charts_core::model::data_validation::{
    ValidationError, ValidationReport, validate_timestamp,
};

/// Sanitize `(time, item)` pairs into ascending, unique rows, carrying each item through its
/// row's fate — the item-level mirror of `sanitize_rows` (data_validation.rs):
/// 1. any invalid time rejects the complete batch;
/// 2. unordered input stably sorts by time (`reordered` flagged);
/// 3. duplicate timestamps collapse last-wins (the last occurrence in the *source* input
///    survives, matching a streaming `update` overwriting a bar).
///
/// Items carry no engine values, so there is nothing else to validate: whitespace-vs-data is
/// plugin-defined (the pane view's `is_whitespace`) and never gates ingestion.
pub fn sanitize_items<T>(
    times: &[f64],
    items: Vec<T>,
) -> Result<(Vec<i64>, Vec<T>, ValidationReport), ValidationError> {
    debug_assert_eq!(times.len(), items.len());
    let mut report = ValidationReport::default();
    let mut rows: Vec<(i64, usize, T)> = Vec::with_capacity(times.len());
    for (index, (&time, item)) in times.iter().zip(items).enumerate() {
        let time = validate_timestamp(time)
            .map_err(|error| ValidationError::InvalidTimestamp { index, error })?;
        rows.push((time, index, item));
    }
    report.reordered = rows.windows(2).any(|w| w[0].0 > w[1].0);
    if report.reordered {
        // Stable by time so that, within a duplicate group, source order is preserved and the
        // last source occurrence is the one kept below (mirrors `sanitize_rows`).
        rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    }
    let mut out_times: Vec<i64> = Vec::with_capacity(rows.len());
    let mut out_items: Vec<T> = Vec::with_capacity(rows.len());
    for (time, _, item) in rows {
        if out_times.last() == Some(&time) {
            report.dropped_duplicate += 1;
            let last = out_items.len() - 1;
            out_items[last] = item;
        } else {
            out_times.push(time);
            out_items.push(item);
        }
    }
    report.accepted = out_times.len();
    Ok((out_times, out_items, report))
}

/// Streaming update mirroring the data layer's raw-row `update_styled`: append a new time or
/// replace the item at an existing one (a mid-history change splices in place, like the data
/// layer's rebuild case). Times stay ascending-unique by construction.
pub fn upsert_item<T>(times: &mut Vec<i64>, items: &mut Vec<T>, time: i64, item: T) {
    debug_assert_eq!(times.len(), items.len());
    match times.binary_search(&time) {
        Ok(position) => items[position] = item,
        Err(position) => {
            times.insert(position, time);
            items.insert(position, item);
        }
    }
}

/// Remove the last `count` rows (reference `ISeriesApi.pop`, clamped to the row count), truncating
/// times and items together. Returns the new row count.
pub fn pop_items<T>(times: &mut Vec<i64>, items: &mut Vec<T>, count: usize) -> usize {
    debug_assert_eq!(times.len(), items.len());
    let keep = times.len().saturating_sub(count);
    times.truncate(keep);
    items.truncate(keep);
    keep
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sanitize(times: &[f64]) -> (Vec<i64>, Vec<String>, ValidationReport) {
        let items: Vec<String> = (0..times.len()).map(|i| format!("item{i}")).collect();
        sanitize_items(times, items).unwrap()
    }

    #[test]
    fn clean_input_passes_through_untouched() {
        let (times, items, report) = sanitize(&[1.0, 2.0, 3.0]);
        assert_eq!(times, [1, 2, 3]);
        assert_eq!(items, ["item0", "item1", "item2"]);
        assert!(report.is_clean());
        assert_eq!(report.accepted, 3);
    }

    #[test]
    fn invalid_time_rejects_the_complete_custom_batch() {
        let items = vec!["first", "invalid"];
        let error = sanitize_items(&[1.0, f64::NAN], items).unwrap_err();
        assert!(matches!(
            error,
            ValidationError::InvalidTimestamp { index: 1, .. }
        ));
    }

    #[test]
    fn unordered_input_sorts_stably_and_carries_items() {
        let (times, items, report) = sanitize(&[3.0, 1.0, 2.0]);
        assert_eq!(times, [1, 2, 3]);
        assert_eq!(items, ["item1", "item2", "item0"]);
        assert!(report.reordered);
        assert_eq!(report.dropped_duplicate, 0);
    }

    #[test]
    fn duplicates_collapse_last_wins_in_order() {
        let (times, items, report) = sanitize(&[1.0, 2.0, 2.0, 3.0]);
        assert_eq!(times, [1, 2, 3]);
        assert_eq!(items, ["item0", "item2", "item3"]);
        assert_eq!(report.dropped_duplicate, 1);
        assert!(!report.reordered);
    }

    #[test]
    fn duplicates_collapse_last_wins_after_a_sort() {
        // Out of order AND duplicated: the source index breaks the tie so the later input
        // wins — times 2(a=item0), 1(=item1), 2(b=item2) -> sorted stable 1, 2a, 2b -> keep 2b.
        let (times, items, report) = sanitize(&[2.0, 1.0, 2.0]);
        assert_eq!(times, [1, 2]);
        assert_eq!(items, ["item1", "item2"]);
        assert!(report.reordered);
        assert_eq!(report.dropped_duplicate, 1);
    }

    #[test]
    fn fractional_times_are_rejected() {
        let error = sanitize_items(&[1.9], vec!["item"]).unwrap_err();
        assert!(matches!(
            error,
            ValidationError::InvalidTimestamp { index: 0, .. }
        ));
    }

    #[test]
    fn empty_input_is_clean_empty_output() {
        let (times, items, report) = sanitize(&[]);
        assert!(times.is_empty() && items.is_empty());
        assert!(report.is_clean());
    }

    #[test]
    fn upsert_appends_replaces_and_splices_in_sorted_order() {
        let mut times = vec![1, 2, 4];
        let mut items = vec!["a", "b", "d"];
        // Append a new max time.
        upsert_item(&mut times, &mut items, 5, "e");
        assert_eq!(times, [1, 2, 4, 5]);
        assert_eq!(items, ["a", "b", "d", "e"]);
        // Replace the last row.
        upsert_item(&mut times, &mut items, 5, "e2");
        assert_eq!(times, [1, 2, 4, 5]);
        assert_eq!(items, ["a", "b", "d", "e2"]);
        // Mid-history insert (the data layer's rebuild case).
        upsert_item(&mut times, &mut items, 3, "c");
        assert_eq!(times, [1, 2, 3, 4, 5]);
        assert_eq!(items, ["a", "b", "c", "d", "e2"]);
        // Mid-history replace.
        upsert_item(&mut times, &mut items, 2, "b2");
        assert_eq!(times, [1, 2, 3, 4, 5]);
        assert_eq!(items, ["a", "b2", "c", "d", "e2"]);
    }

    #[test]
    fn pop_truncates_both_views_and_clamps() {
        let mut times = vec![1, 2, 3];
        let mut items = vec!["a", "b", "c"];
        assert_eq!(pop_items(&mut times, &mut items, 0), 3);
        assert_eq!(pop_items(&mut times, &mut items, 2), 1);
        assert_eq!(times, [1]);
        assert_eq!(items, ["a"]);
        assert_eq!(pop_items(&mut times, &mut items, 10), 0);
        assert!(times.is_empty() && items.is_empty());
    }
}
