//! Compact hierarchical row summaries for viewport-density rendering.
//!
//! Nodes retain only source-row identities. Canonical values stay in `DataLayer`; render and
//! query paths dereference the selected endpoint, OHLC-extrema, and close-extrema rows through
//! `PlotListView`.

use std::ops::Range;

use super::plot_list::{PlotValueIndex, PlotValues};

pub const LOD_FANOUT: usize = 16;
const NO_ROW: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LodSummary {
    first: u32,
    low: u32,
    high: u32,
    close_min: u32,
    close_max: u32,
    last: u32,
}

impl LodSummary {
    const EMPTY: Self = Self {
        first: NO_ROW,
        low: NO_ROW,
        high: NO_ROW,
        close_min: NO_ROW,
        close_max: NO_ROW,
        last: NO_ROW,
    };

    fn row(row: usize, values: PlotValues<'_>) -> Self {
        let Ok(row) = u32::try_from(row) else {
            return Self::EMPTY;
        };
        // Summaries name plot rows; an as-of plot row reads the canonical row it shows.
        let row_usize = values.source_row(row as usize);
        let columns = values.columns();
        if columns.iter().all(|column| column[row_usize].is_nan()) {
            return Self::EMPTY;
        }
        let finite_row = |column: PlotValueIndex| {
            if columns[column as usize][row_usize].is_finite() {
                row
            } else {
                NO_ROW
            }
        };
        Self {
            first: row,
            low: finite_row(PlotValueIndex::Low),
            high: finite_row(PlotValueIndex::High),
            close_min: finite_row(PlotValueIndex::Close),
            close_max: finite_row(PlotValueIndex::Close),
            last: row,
        }
    }

    fn merge(self, other: Self, values: PlotValues<'_>) -> Self {
        if self.first == NO_ROW {
            return other;
        }
        if other.first == NO_ROW {
            return self;
        }
        let value = |plot: PlotValueIndex, row: u32| values.value_at(row as usize, plot);
        let low_row = match (self.low, other.low) {
            (NO_ROW, row) | (row, NO_ROW) => row,
            (left, right) => {
                if value(PlotValueIndex::Low, right) < value(PlotValueIndex::Low, left) {
                    right
                } else {
                    left
                }
            }
        };
        let high_row = match (self.high, other.high) {
            (NO_ROW, row) | (row, NO_ROW) => row,
            (left, right) => {
                if value(PlotValueIndex::High, right) > value(PlotValueIndex::High, left) {
                    right
                } else {
                    left
                }
            }
        };
        let close_min = match (self.close_min, other.close_min) {
            (NO_ROW, row) | (row, NO_ROW) => row,
            (left, right) => {
                if value(PlotValueIndex::Close, right) < value(PlotValueIndex::Close, left) {
                    right
                } else {
                    left
                }
            }
        };
        let close_max = match (self.close_max, other.close_max) {
            (NO_ROW, row) | (row, NO_ROW) => row,
            (left, right) => {
                if value(PlotValueIndex::Close, right) > value(PlotValueIndex::Close, left) {
                    right
                } else {
                    left
                }
            }
        };
        Self {
            first: self.first,
            low: low_row,
            high: high_row,
            close_min,
            close_max,
            last: other.last,
        }
    }

    fn rows(self) -> LodRows {
        let mut rows = [
            self.first,
            self.low,
            self.high,
            self.close_min,
            self.close_max,
            self.last,
        ];
        rows.sort_unstable();
        let mut output = LodRows::default();
        for row in rows.into_iter().filter(|&row| row != NO_ROW) {
            if output.len == 0 || output.rows[output.len - 1] != row as usize {
                output.rows[output.len] = row as usize;
                output.len += 1;
            }
        }
        output
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LodRows {
    rows: [usize; 6],
    len: usize,
}

impl LodRows {
    pub fn iter(self) -> impl Iterator<Item = usize> {
        self.rows.into_iter().take(self.len)
    }

    pub fn is_empty(self) -> bool {
        self.len == 0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LodQueryStats {
    pub selected_level: usize,
    pub summary_nodes: usize,
    pub raw_rows: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LodUpdateStats {
    pub nodes_updated: usize,
}

#[derive(Default)]
pub(crate) struct LodPyramid {
    /// Level zero aggregates `LOD_FANOUT` raw rows; every following level aggregates that many
    /// nodes from the preceding level.
    levels: Vec<Vec<LodSummary>>,
}

#[derive(Clone, Copy)]
pub struct LodPyramidView<'a> {
    pyramid: &'a LodPyramid,
    values: PlotValues<'a>,
}

impl LodPyramid {
    pub(crate) fn view<'a>(&'a self, values: PlotValues<'a>) -> LodPyramidView<'a> {
        LodPyramidView {
            pyramid: self,
            values,
        }
    }

    pub(crate) fn rebuild(&mut self, values: PlotValues<'_>) {
        let len = values.len();
        if len > NO_ROW as usize {
            self.levels.clear();
            return;
        }
        self.sync_layout(len);
        if len < LOD_FANOUT {
            return;
        }
        self.rebuild_range(values, 0..len);
    }

    pub(crate) fn rebuild_range(
        &mut self,
        values: PlotValues<'_>,
        affected: Range<usize>,
    ) -> LodUpdateStats {
        let len = values.len();
        if len > NO_ROW as usize {
            self.levels.clear();
            return LodUpdateStats::default();
        }
        self.sync_layout(len);
        if self.levels.is_empty() || affected.start >= affected.end || affected.start >= len {
            return LodUpdateStats::default();
        }

        let mut start = affected.start / LOD_FANOUT;
        let mut end = affected.end.min(len).div_ceil(LOD_FANOUT);
        let mut nodes_updated = 0;
        for node in start..end {
            let rows = node * LOD_FANOUT..((node + 1) * LOD_FANOUT).min(len);
            self.levels[0][node] = rows.fold(LodSummary::EMPTY, |summary, row| {
                summary.merge(LodSummary::row(row, values), values)
            });
            nodes_updated += 1;
        }

        for level in 1..self.levels.len() {
            start /= LOD_FANOUT;
            end = end.div_ceil(LOD_FANOUT);
            let (lower, current) = self.levels.split_at_mut(level);
            let children = &lower[level - 1];
            for node in start..end.min(current[0].len()) {
                let child_start = node * LOD_FANOUT;
                let child_end = ((node + 1) * LOD_FANOUT).min(children.len());
                current[0][node] = children[child_start..child_end]
                    .iter()
                    .copied()
                    .fold(LodSummary::EMPTY, |summary, child| {
                        summary.merge(child, values)
                    });
                nodes_updated += 1;
            }
        }
        LodUpdateStats { nodes_updated }
    }

    pub(crate) fn logical_bytes(&self) -> usize {
        self.levels
            .iter()
            .map(|level| level.len() * std::mem::size_of::<LodSummary>())
            .sum()
    }

    pub(crate) fn node_count(&self) -> usize {
        self.levels.iter().map(Vec::len).sum()
    }

    pub(crate) fn capacity_bytes(&self) -> usize {
        self.levels.capacity() * std::mem::size_of::<Vec<LodSummary>>()
            + self
                .levels
                .iter()
                .map(|level| level.capacity() * std::mem::size_of::<LodSummary>())
                .sum::<usize>()
    }

    fn sync_layout(&mut self, len: usize) {
        let mut counts = Vec::new();
        let mut count = len;
        while count >= LOD_FANOUT {
            count = count.div_ceil(LOD_FANOUT);
            counts.push(count);
        }
        self.levels.truncate(counts.len());
        for (level, count) in counts.into_iter().enumerate() {
            if level == self.levels.len() {
                self.levels.push(Vec::new());
            }
            self.levels[level].resize(count, LodSummary::EMPTY);
        }
    }
}

impl LodPyramidView<'_> {
    /// Last non-whitespace source row strictly before `end`. The prefix is decomposed into at
    /// most `LOD_FANOUT - 1` raw rows or summary nodes per hierarchy level, so trailing
    /// whitespace cannot turn a latest/predecessor query into a history-length scan.
    pub fn last_row_before(self, end: usize) -> Option<usize> {
        let len = self.values.len();
        let mut position = end.min(len);
        while position > 0 {
            let mut selected = None;
            let mut group = LOD_FANOUT;
            for level in 0..self.pyramid.levels.len() {
                if group > position || !position.is_multiple_of(group) {
                    break;
                }
                selected = Some((level, group));
                let Some(next) = group.checked_mul(LOD_FANOUT) else {
                    break;
                };
                group = next;
            }
            if let Some((level, group)) = selected {
                let node = position / group - 1;
                let row = self.pyramid.levels[level][node].last;
                if row != NO_ROW {
                    return Some(row as usize);
                }
                position -= group;
            } else {
                position -= 1;
                if !self.values.is_whitespace_row(position) {
                    return Some(position);
                }
            }
        }
        None
    }

    /// Deepest summary level whose group fits within the average physical-pixel density. Level
    /// zero means the canonical raw path; level one aggregates `LOD_FANOUT` rows.
    pub fn selected_level(self, rows_per_pixel: f64) -> usize {
        let mut group = LOD_FANOUT;
        let mut selected = 0;
        for level in 0..self.pyramid.levels.len() {
            if group as f64 > rows_per_pixel {
                break;
            }
            selected = level + 1;
            let Some(next) = group.checked_mul(LOD_FANOUT) else {
                break;
            };
            group = next;
        }
        selected
    }

    /// Exact endpoint, OHLC-extrema, and close-extrema representatives for a raw-row range.
    /// Interior aligned groups use the selected hierarchy level; partial boundaries descend to
    /// lower levels or raw rows.
    pub fn rows_on_range(
        self,
        range: Range<usize>,
        selected_level: usize,
    ) -> (LodRows, LodQueryStats) {
        let len = self.values.len();
        let mut position = range.start.min(len);
        let end = range.end.min(len);
        let selected_level = selected_level.min(self.pyramid.levels.len());
        let mut summary = LodSummary::EMPTY;
        let mut stats = LodQueryStats {
            selected_level,
            ..LodQueryStats::default()
        };
        while position < end {
            let mut selected = None;
            let mut group = LOD_FANOUT;
            for level in 0..selected_level {
                if position.is_multiple_of(group) && position + group <= end {
                    selected = Some((level, group));
                }
                let Some(next) = group.checked_mul(LOD_FANOUT) else {
                    break;
                };
                group = next;
            }
            if let Some((level, group)) = selected {
                summary = summary.merge(self.pyramid.levels[level][position / group], self.values);
                stats.summary_nodes += 1;
                position += group;
            } else {
                summary = summary.merge(LodSummary::row(position, self.values), self.values);
                stats.raw_rows += 1;
                position += 1;
            }
        }
        (summary.rows(), stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(close: &[f64]) -> PlotValues<'_> {
        PlotValues::Single(close)
    }

    #[test]
    fn summaries_preserve_first_extrema_last_and_order() {
        let mut data = vec![10.0; 300];
        data[17] = -50.0;
        data[201] = 90.0;
        let mut pyramid = LodPyramid::default();
        pyramid.rebuild(values(&data));
        let (rows, stats) = pyramid.view(values(&data)).rows_on_range(3..277, 2);
        assert_eq!(rows.iter().collect::<Vec<_>>(), vec![3, 17, 201, 276]);
        assert!(stats.summary_nodes > 0);
        assert!(stats.raw_rows < 2 * LOD_FANOUT);
    }

    #[test]
    fn predecessor_lookup_skips_pathological_whitespace_with_the_pyramid() {
        let mut data = vec![f64::NAN; LOD_FANOUT.pow(5) + 7];
        data[3] = 12.0;
        data[LOD_FANOUT.pow(3) + 2] = 34.0;
        let mut pyramid = LodPyramid::default();
        pyramid.rebuild(values(&data));
        let view = pyramid.view(values(&data));

        assert_eq!(
            view.last_row_before(data.len()),
            Some(LOD_FANOUT.pow(3) + 2)
        );
        assert_eq!(view.last_row_before(LOD_FANOUT.pow(3) + 2), Some(3));
        assert_eq!(view.last_row_before(3), None);
    }

    #[test]
    fn ohlc_summaries_keep_endpoints_and_distinct_price_extrema_in_time_order() {
        let mut open = vec![10.0; 300];
        let mut high = vec![11.0; 300];
        let mut low = vec![9.0; 300];
        let mut close = vec![10.0; 300];
        open[0] = 123.0;
        low[5] = -500.0;
        close[20] = 400.0;
        high[40] = 500.0;
        close[70] = -400.0;
        close[299] = 321.0;
        let values = PlotValues::Ohlc([&open, &high, &low, &close]);
        let mut pyramid = LodPyramid::default();
        pyramid.rebuild(values);
        assert_eq!(
            pyramid
                .view(values)
                .rows_on_range(0..300, 2)
                .0
                .iter()
                .collect::<Vec<_>>(),
            vec![0, 5, 20, 40, 70, 299]
        );
    }

    #[test]
    fn whitespace_does_not_invent_a_representative() {
        let data = vec![f64::NAN; 256];
        let mut pyramid = LodPyramid::default();
        pyramid.rebuild(values(&data));
        assert!(
            pyramid
                .view(values(&data))
                .rows_on_range(0..data.len(), 1)
                .0
                .is_empty()
        );
    }

    #[test]
    fn point_update_changes_only_one_node_per_level() {
        let mut data = vec![10.0; 1_000_000];
        let mut pyramid = LodPyramid::default();
        pyramid.rebuild(values(&data));
        data[999_999] = 100.0;
        let stats = pyramid.rebuild_range(values(&data), 999_999..1_000_000);
        assert_eq!(stats.nodes_updated, pyramid.levels.len());
        assert_eq!(
            pyramid
                .view(values(&data))
                .rows_on_range(0..data.len(), pyramid.levels.len())
                .0
                .iter()
                .collect::<Vec<_>>(),
            vec![0, 999_999]
        );
    }

    #[test]
    fn level_selection_changes_only_at_fanout_boundaries() {
        let data = vec![10.0; LOD_FANOUT * LOD_FANOUT * 2];
        let mut pyramid = LodPyramid::default();
        pyramid.rebuild(values(&data));
        let view = pyramid.view(values(&data));
        assert_eq!(view.selected_level(LOD_FANOUT as f64 - 0.001), 0);
        assert_eq!(view.selected_level(LOD_FANOUT as f64), 1);
        assert_eq!(
            view.selected_level((LOD_FANOUT * LOD_FANOUT) as f64 - 0.001),
            1
        );
        assert_eq!(view.selected_level((LOD_FANOUT * LOD_FANOUT) as f64), 2);
    }
}
