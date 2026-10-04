//! Engine-owned Heikin Ashi projection for candlestick presentation.
//!
//! The source series remains the canonical raw OHLC owner. This cache is only the derived
//! presentation projection used by frame/scaling paths; trading and crosshair queries continue to
//! read the source columns.

#[derive(Default)]
pub(crate) struct HeikinAshiCache {
    generation: Option<u64>,
    rows: Vec<[f64; 4]>,
}

impl HeikinAshiCache {
    pub(crate) fn row(
        &mut self,
        generation: u64,
        columns: [&[f64]; 4],
        row: usize,
    ) -> Option<[f64; 4]> {
        if self.generation != Some(generation) {
            self.rebuild(generation, columns);
        }
        self.rows.get(row).copied()
    }

    fn rebuild(&mut self, generation: u64, columns: [&[f64]; 4]) {
        let rows = columns.iter().map(|column| column.len()).min().unwrap_or(0);
        self.rows.clear();
        self.rows.reserve(rows);
        let mut previous = None;
        #[allow(clippy::needless_range_loop)]
        for row in 0..rows {
            let raw = [
                columns[0][row],
                columns[1][row],
                columns[2][row],
                columns[3][row],
            ];
            let projected = Self::project(previous, raw);
            if !projected[3].is_nan() {
                previous = Some(projected);
            }
            self.rows.push(projected);
        }
        self.generation = Some(generation);
    }

    /// One Heikin Ashi row from a raw `[open, high, low, close]` and the previous non-whitespace
    /// Heikin Ashi row (`None` for the first bar). Whitespace or non-finite raw rows project to an
    /// all-NaN row and leave the previous row for the next bar, so whitespace never resets the
    /// chain. Shared by the cached rebuild and the display path for an eased live bar, so the two
    /// cannot drift apart.
    pub(crate) fn project(previous: Option<[f64; 4]>, raw: [f64; 4]) -> [f64; 4] {
        if raw.iter().any(|value| !value.is_finite()) {
            return [f64::NAN; 4];
        }
        let close = (raw[0] + raw[1] + raw[2] + raw[3]) / 4.0;
        let open = match previous {
            Some(previous) => (previous[0] + previous[3]) / 2.0,
            None => (raw[0] + raw[3]) / 2.0,
        };
        let high = raw[1].max(open).max(close);
        let low = raw[2].min(open).min(close);
        [open, high, low, close]
    }

    #[cfg(test)]
    pub(crate) fn rows(&self) -> &[[f64; 4]] {
        &self.rows
    }
}

#[cfg(test)]
mod tests {
    use super::HeikinAshiCache;

    #[test]
    fn projection_keeps_raw_rows_out_of_the_derived_owner() {
        let columns = [
            &[10.0, 12.0][..],
            &[14.0, 16.0][..],
            &[8.0, 10.0][..],
            &[12.0, 14.0][..],
        ];
        let mut cache = HeikinAshiCache::default();
        assert_eq!(cache.row(1, columns, 0), Some([11.0, 14.0, 8.0, 11.0]));
        assert_eq!(cache.row(1, columns, 1), Some([11.0, 16.0, 10.0, 13.0]));
        assert_eq!(
            cache.rows(),
            &[[11.0, 14.0, 8.0, 11.0], [11.0, 16.0, 10.0, 13.0]]
        );
    }

    #[test]
    fn the_display_projection_matches_the_cached_rebuild_row_for_row() {
        let columns = [
            &[10.0, f64::NAN, 14.0][..],
            &[14.0, f64::NAN, 18.0][..],
            &[8.0, f64::NAN, 12.0][..],
            &[12.0, f64::NAN, 16.0][..],
        ];
        let mut cache = HeikinAshiCache::default();
        cache.row(1, columns, 2);
        let first = HeikinAshiCache::project(None, [10.0, 14.0, 8.0, 12.0]);
        assert_eq!(first, cache.rows()[0]);
        assert!(HeikinAshiCache::project(Some(first), [f64::NAN; 4])
            .iter()
            .all(|value| value.is_nan()));
        assert_eq!(
            HeikinAshiCache::project(Some(first), [14.0, 18.0, 12.0, 16.0]),
            cache.rows()[2]
        );
    }

    #[test]
    fn whitespace_does_not_reset_the_previous_heikin_ashi_open() {
        let columns = [
            &[10.0, f64::NAN, 14.0][..],
            &[14.0, f64::NAN, 18.0][..],
            &[8.0, f64::NAN, 12.0][..],
            &[12.0, f64::NAN, 16.0][..],
        ];
        let mut cache = HeikinAshiCache::default();
        cache.row(1, columns, 2);
        assert!(cache.rows()[1].iter().all(|value| value.is_nan()));
        assert_eq!(cache.rows()[2], [11.0, 18.0, 11.0, 15.0]);
    }
}
