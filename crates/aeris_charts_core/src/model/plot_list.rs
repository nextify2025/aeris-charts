//! Series index/query storage with a chunked min/max cache. Canonical values live in the
//! data layer; a [`PlotListView`] joins those values to this compact logical-index mapping for
//! read-only query and rendering phases.
//!
//! Rows are keyed by *time-point index* (position in the merged time scale), which may be
//! sparse when a series has whitespace. All searches are binary over the sorted index column.
//!
//! A plot row is the canonical row of the same position, except for as-of time-aligned series
//! (and outputs aliasing them): their plot rows are merged time points, and a non-decreasing
//! source-row map names the canonical row each one shows.

use std::collections::HashMap;

use crate::helpers::algorithms::{lower_bound, upper_bound};
use crate::model::lod::{LodPyramid, LodPyramidView};
use crate::TimePointIndex;

/// `CHUNK_SIZE` in reference.
const CHUNK_SIZE: i64 = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlotValueIndex {
    Open = 0,
    High = 1,
    Low = 2,
    Close = 3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinMax {
    pub min: f64,
    pub max: f64,
}

fn merge_min_max(first: Option<MinMax>, second: Option<MinMax>) -> Option<MinMax> {
    match (first, second) {
        (None, s) => s,
        (f, None) => f,
        (Some(f), Some(s)) => Some(MinMax {
            min: f.min.min(s.min),
            max: f.max.max(s.max),
        }),
    }
}

/// Search direction when no row exists at the exact index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MismatchDirection {
    NearestLeft,
    None,
    NearestRight,
}

#[derive(Default)]
enum PlotIndices {
    #[default]
    Empty,
    /// A common aligned series needs no per-row mapping allocation.
    Dense {
        start: TimePointIndex,
        len: usize,
    },
    Sparse(Vec<TimePointIndex>),
}

#[derive(Default)]
pub struct PlotList {
    indices: PlotIndices,
    /// As-of time alignment: the canonical row each plot row shows (non-decreasing, one entry
    /// per plot row). `None` is the identity used by every union-timed series.
    source_rows: Option<Vec<u32>>,
    /// (plot, chunk_index) -> chunk min/max. Cleared on set_data.
    min_max_cache: HashMap<(usize, i64), Option<MinMax>>,
}

/// Borrowed canonical value columns. A scalar series aliases one column into the four
/// reference-compatible plot slots without storing four copies.
#[derive(Clone, Copy)]
pub enum PlotValues<'a> {
    Single(&'a [f64]),
    Ohlc([&'a [f64]; 4]),
    /// Canonical columns read through an as-of time-alignment row map: plot row `r` shows
    /// canonical row `rows[r]`, so one source row may back several consecutive plot rows.
    AsOf {
        columns: [&'a [f64]; 4],
        rows: &'a [u32],
    },
}

impl<'a> PlotValues<'a> {
    /// The canonical column. For [`PlotValues::AsOf`] it is indexed by source row
    /// ([`Self::source_row`]), not by plot row; [`Self::value_at`] resolves both.
    pub fn column(self, plot: PlotValueIndex) -> &'a [f64] {
        self.columns()[plot as usize]
    }

    /// Canonical columns in [`PlotValueIndex`] order.
    pub fn columns(self) -> [&'a [f64]; 4] {
        match self {
            Self::Single(values) => [values, values, values, values],
            Self::Ohlc(columns) | Self::AsOf { columns, .. } => columns,
        }
    }

    /// Number of plot rows these values back.
    pub fn len(self) -> usize {
        match self {
            Self::Single(values) => values.len(),
            Self::Ohlc(columns) => columns[3].len(),
            Self::AsOf { rows, .. } => rows.len(),
        }
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// Canonical row shown by plot row `row` (the identity unless as-of aligned).
    pub fn source_row(self, row: usize) -> usize {
        match self {
            Self::AsOf { rows, .. } => rows[row] as usize,
            Self::Single(_) | Self::Ohlc(_) => row,
        }
    }

    pub fn value_at(self, row: usize, plot: PlotValueIndex) -> f64 {
        match self {
            Self::Single(values) => values[row],
            Self::Ohlc(columns) => columns[plot as usize][row],
            Self::AsOf { columns, rows } => columns[plot as usize][rows[row] as usize],
        }
    }

    pub(crate) fn is_whitespace_row(self, row: usize) -> bool {
        match self {
            Self::Single(values) => values[row].is_nan(),
            Self::Ohlc(values) => values.iter().all(|column| column[row].is_nan()),
            Self::AsOf { columns, rows } => {
                let row = rows[row] as usize;
                columns.iter().all(|column| column[row].is_nan())
            }
        }
    }

    /// Canonical values viewed through an optional as-of row map.
    pub(crate) fn with_rows(self, rows: Option<&'a [u32]>) -> Self {
        match rows {
            Some(rows) => Self::AsOf {
                columns: self.columns(),
                rows,
            },
            None => self,
        }
    }
}

/// A short-lived view joining logical indices to the canonical series values. It contains only
/// borrows, so constructing it is allocation-free and cannot outlive a data-layer mutation.
#[derive(Clone, Copy)]
pub struct PlotListView<'a> {
    list: &'a PlotList,
    values: PlotValues<'a>,
    lod: Option<&'a LodPyramid>,
}

impl<'a> PlotListView<'a> {
    /// Join canonical `values` to `list`; an as-of list reads them through its row map.
    pub fn new(list: &'a PlotList, values: PlotValues<'a>) -> Self {
        Self {
            list,
            values: values.with_rows(list.source_rows()),
            lod: None,
        }
    }

    pub(crate) fn with_lod(
        list: &'a PlotList,
        values: PlotValues<'a>,
        lod: &'a LodPyramid,
    ) -> Self {
        Self {
            list,
            values: values.with_rows(list.source_rows()),
            lod: Some(lod),
        }
    }

    /// Canonical row shown by plot row `row`: the identity except for as-of aligned series,
    /// whose consecutive plot rows may repeat one source row.
    pub fn source_row(self, row: usize) -> usize {
        self.values.source_row(row)
    }

    /// First plot row showing canonical row `source_row`, if that row is shown at all (an as-of
    /// row collapsed into a later row's time point, or past the aligned extent, is not).
    pub fn row_for_source(self, source_row: usize) -> Option<usize> {
        match self.list.source_rows() {
            None => (source_row < self.size()).then_some(source_row),
            Some(rows) => {
                let row = rows.partition_point(|&row| (row as usize) < source_row);
                (rows.get(row).is_some_and(|&row| row as usize == source_row)).then_some(row)
            }
        }
    }

    /// Whether plot rows map through an as-of time-alignment row map.
    pub fn is_as_of(self) -> bool {
        self.list.source_rows().is_some()
    }

    /// Canonical rows spanned by the plot rows `rows`, each once: the identity unless as-of,
    /// where repeated points collapse and rows collapsed between two points are included.
    pub fn source_range(self, rows: std::ops::Range<usize>) -> std::ops::Range<usize> {
        match self.list.source_rows() {
            None => rows,
            Some(_) if rows.is_empty() => 0..0,
            Some(map) => map[rows.start] as usize..map[rows.end - 1] as usize + 1,
        }
    }

    #[doc(hidden)]
    pub fn lod(self) -> Option<LodPyramidView<'a>> {
        self.lod.map(|lod| lod.view(self.values))
    }

    pub fn size(self) -> usize {
        self.list.size()
    }

    pub fn is_empty(self) -> bool {
        self.list.is_empty()
    }

    pub fn first_index(self) -> Option<TimePointIndex> {
        self.list.first_index()
    }

    pub fn last_index(self) -> Option<TimePointIndex> {
        self.list.last_index()
    }

    pub fn index_at(self, row: usize) -> Option<TimePointIndex> {
        self.list.index_at(row)
    }

    pub fn indices(self) -> PlotIndexIter<'a> {
        self.list.indices()
    }

    /// The canonical column, indexed by [`Self::source_row`] (plot rows index it directly unless
    /// the series is as-of aligned); prefer [`Self::value_at`] for plot rows.
    pub fn column(self, plot: PlotValueIndex) -> &'a [f64] {
        self.values.column(plot)
    }

    pub fn contains(self, index: TimePointIndex) -> bool {
        self.list.contains(index)
    }

    pub fn search(self, index: TimePointIndex, direction: MismatchDirection) -> Option<usize> {
        self.list.search(index, direction)
    }

    pub fn value_at(self, row: usize, plot: PlotValueIndex) -> f64 {
        self.values.value_at(row, plot)
    }

    pub fn is_whitespace_row(self, row: usize) -> bool {
        self.values.is_whitespace_row(row)
    }

    pub fn last_non_whitespace_row(self, index: TimePointIndex) -> Option<usize> {
        let row = self.search(index, MismatchDirection::NearestLeft)?;
        self.last_non_whitespace_row_before(row + 1)
    }

    /// Last non-whitespace source row strictly before `end_row`.
    pub fn last_non_whitespace_row_before(self, end_row: usize) -> Option<usize> {
        if let Some(lod) = self.lod() {
            return lod.last_row_before(end_row);
        }
        (0..end_row.min(self.size()))
            .rev()
            .find(|&row| !self.is_whitespace_row(row))
    }

    pub fn first_non_whitespace_row(self, index: TimePointIndex) -> Option<usize> {
        let mut row = self.search(index, MismatchDirection::NearestRight)?;
        loop {
            if !self.is_whitespace_row(row) {
                return Some(row);
            }
            row += 1;
            if row >= self.size() {
                return None;
            }
        }
    }

    pub fn visible_rows(self, from: TimePointIndex, to: TimePointIndex) -> std::ops::Range<usize> {
        self.list.visible_rows(from, to)
    }
}

pub enum PlotIndexIter<'a> {
    Empty,
    Dense(std::ops::Range<TimePointIndex>),
    Sparse(std::slice::Iter<'a, TimePointIndex>),
}

impl Iterator for PlotIndexIter<'_> {
    type Item = TimePointIndex;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Empty => None,
            Self::Dense(indices) => indices.next(),
            Self::Sparse(indices) => indices.next().copied(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = match self {
            Self::Empty => 0,
            Self::Dense(indices) => indices.end.saturating_sub(indices.start) as usize,
            Self::Sparse(indices) => indices.len(),
        };
        (len, Some(len))
    }
}

impl ExactSizeIterator for PlotIndexIter<'_> {}

impl PlotList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_indices(&mut self, indices: Vec<TimePointIndex>) {
        debug_assert!(
            indices.windows(2).all(|w| w[0] < w[1]),
            "indices must be sorted unique"
        );
        self.indices = dense_or_sparse(indices);
        self.source_rows = None;
        self.min_max_cache.clear();
    }

    /// The as-of row map (plot row -> canonical row), or `None` for the identity.
    pub(crate) fn source_rows(&self) -> Option<&[u32]> {
        self.source_rows.as_deref()
    }

    /// Canonical row shown by plot row `row`.
    pub fn source_row(&self, row: usize) -> usize {
        self.source_rows
            .as_ref()
            .map_or(row, |rows| rows[row] as usize)
    }

    /// Number of plot rows showing canonical rows before `row`.
    pub(crate) fn rows_before_source(&self, row: usize) -> usize {
        match &self.source_rows {
            None => row.min(self.size()),
            Some(rows) => rows.partition_point(|&source| (source as usize) < row),
        }
    }

    /// Plot rows showing canonical row `row` (empty when it is not shown).
    pub(crate) fn plot_rows_for_source(&self, row: usize) -> std::ops::Range<usize> {
        match &self.source_rows {
            None if row < self.size() => row..row + 1,
            None => row..row,
            Some(rows) => {
                rows.partition_point(|&source| (source as usize) < row)
                    ..rows.partition_point(|&source| source as usize <= row)
            }
        }
    }

    /// Rebuild an as-of mapping over the merged points `[0, through)`: each point shows the last
    /// canonical row (`times` ascending) at or before its time, provided that row is at most
    /// `max_staleness` seconds older than the point. Points before the first row, or past the
    /// staleness bound, stay empty. `O(through + rows)`.
    pub(crate) fn rebuild_as_of(
        &mut self,
        merged_times: &[i64],
        through: usize,
        times: &[i64],
        max_staleness: Option<i64>,
    ) {
        // Start from `Empty` so a contiguous mapping lands in the allocation-free dense form.
        self.indices = PlotIndices::Empty;
        self.source_rows.get_or_insert_with(Vec::new).clear();
        self.extend_as_of(merged_times, 0, through, times, max_staleness);
        self.min_max_cache.clear();
    }

    /// Re-derive the as-of rows of the merged points `[from, through)` after a change that
    /// leaves every earlier point's source row intact (a new tail point, a moved data extent, or
    /// a new canonical row at or after `merged_times[from]`). Returns the first plot row that
    /// may differ; the work is proportional to the rewritten tail.
    pub(crate) fn resync_as_of_tail(
        &mut self,
        merged_times: &[i64],
        from: usize,
        through: usize,
        times: &[i64],
        max_staleness: Option<i64>,
    ) -> usize {
        debug_assert!(self.source_rows.is_some(), "not an as-of mapping");
        let keep = self.lowerbound(from as TimePointIndex);
        let old = (self.index_at(keep), self.last_index());
        self.truncate_plot_rows(keep);
        self.extend_as_of(merged_times, from, through, times, max_staleness);
        self.invalidate_tail_chunks(keep, old);
        keep
    }

    fn extend_as_of(
        &mut self,
        merged_times: &[i64],
        from: usize,
        through: usize,
        times: &[i64],
        max_staleness: Option<i64>,
    ) {
        let through = through.min(merged_times.len());
        if from >= through {
            return;
        }
        let mut row = times.partition_point(|&time| time < merged_times[from]);
        for (index, &point) in merged_times.iter().enumerate().take(through).skip(from) {
            while times.get(row).is_some_and(|&time| time <= point) {
                row += 1;
            }
            let Some(source) = row.checked_sub(1) else {
                continue;
            };
            if max_staleness.is_some_and(|max| point.saturating_sub(times[source]) > max) {
                continue;
            }
            self.push_plot_row(index as TimePointIndex, source);
        }
    }

    fn push_plot_row(&mut self, index: TimePointIndex, source: usize) {
        match &mut self.indices {
            PlotIndices::Empty => {
                self.indices = PlotIndices::Dense {
                    start: index,
                    len: 1,
                }
            }
            PlotIndices::Dense { start, len } if *start + *len as i64 == index => *len += 1,
            PlotIndices::Dense { start, len } => {
                let mut indices = (*start..*start + *len as i64).collect::<Vec<_>>();
                indices.push(index);
                self.indices = PlotIndices::Sparse(indices);
            }
            PlotIndices::Sparse(indices) => indices.push(index),
        }
        debug_assert!(u32::try_from(source).is_ok(), "source row exceeds u32");
        self.source_rows
            .get_or_insert_with(Vec::new)
            .push(source as u32);
    }

    fn truncate_plot_rows(&mut self, len: usize) {
        match &mut self.indices {
            PlotIndices::Empty => {}
            PlotIndices::Dense { len: current, .. } => {
                if len == 0 {
                    self.indices = PlotIndices::Empty;
                } else {
                    *current = (*current).min(len);
                }
            }
            PlotIndices::Sparse(indices) => indices.truncate(len),
        }
        if let Some(rows) = &mut self.source_rows {
            rows.truncate(len);
        }
    }

    /// Drop cached autoscale chunks of a rewritten tail: from the first rewritten row's old or
    /// new index through the old or new last index.
    fn invalidate_tail_chunks(
        &mut self,
        keep: usize,
        (old_first, old_last): (Option<TimePointIndex>, Option<TimePointIndex>),
    ) {
        if keep == 0 {
            self.min_max_cache.clear();
            return;
        }
        let first = [old_first, self.index_at(keep)].into_iter().flatten().min();
        let last = [old_last, self.last_index()].into_iter().flatten().max();
        if let (Some(first), Some(last)) = (first, last) {
            self.invalidate_index_chunks(first, last);
        }
    }

    fn invalidate_index_chunks(&mut self, first: TimePointIndex, last: TimePointIndex) {
        for chunk in first.div_euclid(CHUNK_SIZE)..=last.div_euclid(CHUNK_SIZE) {
            for plot in 0..4 {
                self.min_max_cache.remove(&(plot, chunk));
            }
        }
    }

    /// Reindex canonical columns while retaining the plot's high-water allocation. Full data
    /// installs and retention trims call this repeatedly; replacing the vectors would fragment
    /// the WASM allocator even though the retained row count is bounded.
    pub(crate) fn rebuild_from(&mut self, merged_times: &[i64], times: &[i64]) {
        self.source_rows = None;
        if times.is_empty() {
            self.indices = PlotIndices::Empty;
            self.min_max_cache.clear();
            return;
        }
        let dense_start = times
            .first()
            .and_then(|time| merged_times.binary_search(time).ok())
            .filter(|&start| merged_times.get(start..start + times.len()) == Some(times));
        if let Some(start) = dense_start {
            self.indices = PlotIndices::Dense {
                start: start as TimePointIndex,
                len: times.len(),
            };
        } else {
            let mut indices = match std::mem::take(&mut self.indices) {
                PlotIndices::Sparse(indices) => indices,
                _ => Vec::new(),
            };
            indices.clear();
            if indices.capacity() < times.len() {
                indices.reserve(times.len());
            }
            for time in times {
                let index = merged_times.binary_search(time).unwrap_or_else(|position| {
                    debug_assert!(false, "series time {time} missing from merged time points");
                    position.min(merged_times.len().saturating_sub(1))
                });
                indices.push(index as TimePointIndex);
            }
            self.indices = PlotIndices::Sparse(indices);
        }
        self.min_max_cache.clear();
    }

    /// Alias the plot rows of `source` that show its canonical rows `[offset, offset + len)`.
    /// For a union source those are the same rows; an as-of source contributes the plot rows
    /// showing them, with its row map shifted onto this series' own rows.
    ///
    /// `unchanged` is how many of the source's leading plot rows the caller knows kept their
    /// mapping since this list was last copied from it with the same `offset` (0 = unknown). An
    /// as-of copy rewrites only the rows after them, so a live tail stays proportional to the
    /// change. Returns the first plot row of this list that may differ.
    pub(crate) fn copy_range_from(
        &mut self,
        source: &Self,
        offset: usize,
        len: usize,
        unchanged: usize,
    ) -> usize {
        let Some(rows) = source.source_rows.as_deref() else {
            debug_assert!(offset + len <= source.size());
            self.source_rows = None;
            self.indices = match &source.indices {
                PlotIndices::Empty => PlotIndices::Empty,
                PlotIndices::Dense { start, .. } => {
                    if len == 0 {
                        PlotIndices::Empty
                    } else {
                        PlotIndices::Dense {
                            start: *start + offset as i64,
                            len,
                        }
                    }
                }
                PlotIndices::Sparse(indices) => {
                    dense_or_sparse(indices[offset..offset + len].to_vec())
                }
            };
            self.min_max_cache.clear();
            return 0;
        };
        let first = rows.partition_point(|&row| (row as usize) < offset);
        let end = rows.partition_point(|&row| (row as usize) < offset.saturating_add(len));
        let count = end - first;
        let keep = if self.source_rows.is_some() {
            unchanged.saturating_sub(first).min(self.size()).min(count)
        } else {
            0
        };
        debug_assert!(
            (0..keep).all(|row| self.index_at(row) == source.index_at(first + row)
                && self.source_row(row) + offset == rows[first + row] as usize),
            "as-of alias prefix changed without a full copy"
        );
        let old = (self.index_at(keep), self.last_index());
        self.indices = match &source.indices {
            PlotIndices::Empty => PlotIndices::Empty,
            _ if count == 0 => PlotIndices::Empty,
            PlotIndices::Dense { start, .. } => PlotIndices::Dense {
                start: *start + first as i64,
                len: count,
            },
            PlotIndices::Sparse(indices) => {
                let mut target = match std::mem::take(&mut self.indices) {
                    PlotIndices::Sparse(target) => target,
                    PlotIndices::Dense { start, len } if keep > 0 => {
                        (start..start + len as i64).collect()
                    }
                    _ => Vec::new(),
                };
                target.truncate(keep);
                target.extend_from_slice(&indices[first + keep..end]);
                PlotIndices::Sparse(target)
            }
        };
        let mut map = self.source_rows.take().unwrap_or_default();
        map.truncate(keep);
        map.extend(
            rows[first + keep..end]
                .iter()
                .map(|&row| row - offset as u32),
        );
        self.source_rows = Some(map);
        self.invalidate_tail_chunks(keep, old);
        keep
    }

    /// Drop the cached autoscale chunks showing canonical `row` after an in-place value
    /// correction. The row-to-index mapping is unchanged, so no other chunk or mapping state is
    /// touched. Returns the plot rows showing it.
    pub(crate) fn invalidate_row(&mut self, row: usize) -> std::ops::Range<usize> {
        let rows = self.plot_rows_for_source(row);
        if let (Some(first), Some(last)) = (
            self.index_at(rows.start),
            rows.end.checked_sub(1).and_then(|row| self.index_at(row)),
        ) {
            if !rows.is_empty() {
                self.invalidate_index_chunks(first, last);
            }
        }
        rows
    }

    /// Streaming append/replace of the last row (the `update()` hot path).
    pub fn upsert_last(&mut self, index: TimePointIndex) {
        debug_assert!(self.source_rows.is_none(), "as-of rows resync their tail");
        match self.last_index() {
            Some(last) if index == last => {
                // invalidate the chunk containing this row
                let chunk = index.div_euclid(CHUNK_SIZE);
                for plot in 0..4 {
                    self.min_max_cache.remove(&(plot, chunk));
                }
            }
            Some(last) if index > last => match &mut self.indices {
                PlotIndices::Dense { len, .. } if index == last + 1 => *len += 1,
                PlotIndices::Dense { start, len } => {
                    let mut indices = (*start..*start + *len as i64).collect::<Vec<_>>();
                    indices.push(index);
                    self.indices = PlotIndices::Sparse(indices);
                }
                PlotIndices::Sparse(indices) => indices.push(index),
                PlotIndices::Empty => unreachable!(),
            },
            None => {
                self.indices = PlotIndices::Dense {
                    start: index,
                    len: 1,
                };
            }
            _ => {
                // Out-of-order updates are routed to the rebuild path by `DataLayer::update`;
                // reaching this arm is an internal ordering bug. Ignore rather than poison the
                // whole chart (a panic aborts the wasm instance).
                debug_assert!(false, "cannot update older data: index {index} < last");
            }
        }
    }

    pub fn size(&self) -> usize {
        match &self.indices {
            PlotIndices::Empty => 0,
            PlotIndices::Dense { len, .. } => *len,
            PlotIndices::Sparse(indices) => indices.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.size() == 0
    }

    pub fn first_index(&self) -> Option<TimePointIndex> {
        self.index_at(0)
    }

    pub fn last_index(&self) -> Option<TimePointIndex> {
        self.size()
            .checked_sub(1)
            .and_then(|row| self.index_at(row))
    }

    pub fn index_at(&self, row: usize) -> Option<TimePointIndex> {
        match &self.indices {
            PlotIndices::Empty => None,
            PlotIndices::Dense { start, len } => (row < *len).then(|| *start + row as i64),
            PlotIndices::Sparse(indices) => indices.get(row).copied(),
        }
    }

    pub fn indices(&self) -> PlotIndexIter<'_> {
        match &self.indices {
            PlotIndices::Empty => PlotIndexIter::Empty,
            PlotIndices::Dense { start, len } => PlotIndexIter::Dense(*start..*start + *len as i64),
            PlotIndices::Sparse(indices) => PlotIndexIter::Sparse(indices.iter()),
        }
    }

    pub(crate) fn index_bytes(&self) -> usize {
        let rows = self
            .source_rows
            .as_ref()
            .map_or(0, |rows| rows.len() * std::mem::size_of::<u32>());
        rows + match &self.indices {
            PlotIndices::Sparse(indices) => indices.len() * std::mem::size_of::<TimePointIndex>(),
            PlotIndices::Empty | PlotIndices::Dense { .. } => 0,
        }
    }

    pub(crate) fn index_capacity_bytes(&self) -> usize {
        let rows = self
            .source_rows
            .as_ref()
            .map_or(0, |rows| rows.capacity() * std::mem::size_of::<u32>());
        rows + match &self.indices {
            PlotIndices::Sparse(indices) => {
                indices.capacity() * std::mem::size_of::<TimePointIndex>()
            }
            PlotIndices::Empty | PlotIndices::Dense { .. } => 0,
        }
    }

    pub(crate) fn cache_payload_bytes(&self) -> usize {
        self.min_max_cache.len()
            * (std::mem::size_of::<(usize, i64)>() + std::mem::size_of::<Option<MinMax>>())
    }

    pub(crate) fn is_dense(&self) -> bool {
        !matches!(self.indices, PlotIndices::Sparse(_))
    }

    pub fn contains(&self, index: TimePointIndex) -> bool {
        self.bsearch(index).is_some()
    }

    /// Row offset of `index`, honoring the mismatch direction. Port of `_search`.
    pub fn search(&self, index: TimePointIndex, direction: MismatchDirection) -> Option<usize> {
        let exact = self.bsearch(index);
        if exact.is_none() && direction != MismatchDirection::None {
            return match direction {
                MismatchDirection::NearestLeft => self.search_nearest_left(index),
                MismatchDirection::NearestRight => self.search_nearest_right(index),
                MismatchDirection::None => unreachable!(),
            };
        }
        exact
    }

    /// Row offsets `[start, end)` whose merged index lies in the inclusive range `[from, to]`.
    /// Used to slice a (possibly sparse) series to the visible window for rendering.
    pub fn visible_rows(&self, from: TimePointIndex, to: TimePointIndex) -> std::ops::Range<usize> {
        self.lowerbound(from)..self.upperbound(to)
    }

    /// Min/max over the time-point index range `[start, end]` (inclusive, like the strict
    /// visible range), merged over `plots`. Port of `minMaxOnRangeCached`.
    pub fn min_max_on_range_cached(
        &mut self,
        values: PlotValues<'_>,
        start: TimePointIndex,
        end: TimePointIndex,
        plots: &[PlotValueIndex],
    ) -> Option<MinMax> {
        if self.is_empty() {
            return None;
        }

        // Canonical values read through an as-of map; moved out so the cache can be mutated.
        let rows = self.source_rows.take();
        let values = values.with_rows(rows.as_deref());
        let mut result: Option<MinMax> = None;
        for &plot in plots {
            let plot_min_max = self.min_max_on_range_cached_impl(values, start, end, plot);
            result = merge_min_max(result, plot_min_max);
        }
        self.source_rows = rows;
        result
    }

    fn bsearch(&self, index: TimePointIndex) -> Option<usize> {
        let start = self.lowerbound(index);
        if start != self.size() && self.index_at(start).is_some_and(|value| index >= value) {
            return Some(start);
        }
        None
    }

    fn search_nearest_left(&self, index: TimePointIndex) -> Option<usize> {
        let pos = self.lowerbound(index).saturating_sub(1);
        (pos != self.size() && self.index_at(pos).is_some_and(|value| value < index)).then_some(pos)
    }

    fn search_nearest_right(&self, index: TimePointIndex) -> Option<usize> {
        let pos = self.upperbound(index);
        (pos != self.size() && self.index_at(pos).is_some_and(|value| index < value)).then_some(pos)
    }

    fn lowerbound(&self, index: TimePointIndex) -> usize {
        match &self.indices {
            PlotIndices::Empty => 0,
            PlotIndices::Dense { start, len } => {
                index.saturating_sub(*start).clamp(0, *len as i64) as usize
            }
            PlotIndices::Sparse(indices) => lower_bound(indices, |&i| i < index),
        }
    }

    fn upperbound(&self, index: TimePointIndex) -> usize {
        match &self.indices {
            PlotIndices::Empty => 0,
            PlotIndices::Dense { start, len } => {
                (index - *start + 1).clamp(0, *len as i64) as usize
            }
            PlotIndices::Sparse(indices) => upper_bound(indices, |&i| i > index),
        }
    }

    /// Brute min/max over row offsets `[start_row, end_row)`, skipping NaN.
    /// (the reference's for-loop is a no-op when start >= end; Rust slicing would panic, so guard.)
    fn plot_min_max(
        &self,
        values: PlotValues<'_>,
        start_row: usize,
        end_row: usize,
        plot: usize,
    ) -> Option<MinMax> {
        if start_row >= end_row {
            return None;
        }
        let col = values.column(match plot {
            0 => PlotValueIndex::Open,
            1 => PlotValueIndex::High,
            2 => PlotValueIndex::Low,
            _ => PlotValueIndex::Close,
        });
        let fold = |result: Option<MinMax>, v: f64| {
            if v.is_nan() {
                return result;
            }
            Some(match result {
                None => MinMax { min: v, max: v },
                Some(mm) => MinMax {
                    min: mm.min.min(v),
                    max: mm.max.max(v),
                },
            })
        };
        match values {
            PlotValues::AsOf { rows, .. } => rows[start_row..end_row]
                .iter()
                .fold(None, |result, &row| fold(result, col[row as usize])),
            PlotValues::Single(_) | PlotValues::Ohlc(_) => col[start_row..end_row]
                .iter()
                .fold(None, |result, &v| fold(result, v)),
        }
    }

    fn min_max_on_range_cached_impl(
        &mut self,
        values: PlotValues<'_>,
        start: TimePointIndex,
        end: TimePointIndex,
        plot: PlotValueIndex,
    ) -> Option<MinMax> {
        if self.is_empty() {
            return None;
        }
        let plot = plot as usize;

        let (Some(first_index), Some(last_index)) = (self.first_index(), self.last_index()) else {
            return None;
        };

        let s = start.max(first_index);
        let e = end.min(last_index);

        // chunk boundaries in time-point index space
        let cached_low = ((s as f64) / CHUNK_SIZE as f64).ceil() as i64 * CHUNK_SIZE;
        let cached_high =
            cached_low.max(((e as f64) / CHUNK_SIZE as f64).floor() as i64 * CHUNK_SIZE);

        let mut result: Option<MinMax> = None;

        // head: [s, min(e, cached_low, end)) — non-inclusive end via upperbound
        {
            let start_row = self.lowerbound(s);
            let end_row = self.upperbound(e.min(cached_low).min(end));
            result = merge_min_max(result, self.plot_min_max(values, start_row, end_row, plot));
        }

        // cached chunks
        let mut c = (cached_low + 1).max(s);
        while c < cached_high {
            let chunk_index = c.div_euclid(CHUNK_SIZE);

            let chunk_min_max = match self.min_max_cache.get(&(plot, chunk_index)) {
                Some(&mm) => mm,
                None => {
                    let chunk_start = self.lowerbound(chunk_index * CHUNK_SIZE);
                    let chunk_end = self.upperbound((chunk_index + 1) * CHUNK_SIZE - 1);
                    let mm = self.plot_min_max(values, chunk_start, chunk_end, plot);
                    self.min_max_cache.insert((plot, chunk_index), mm);
                    mm
                }
            };

            result = merge_min_max(result, chunk_min_max);
            c += CHUNK_SIZE;
        }

        // tail: [cached_high, e]
        {
            let start_row = self.lowerbound(cached_high);
            let end_row = self.upperbound(e);
            result = merge_min_max(result, self.plot_min_max(values, start_row, end_row, plot));
        }

        result
    }
}

fn dense_or_sparse(indices: Vec<TimePointIndex>) -> PlotIndices {
    let Some(&start) = indices.first() else {
        return PlotIndices::Empty;
    };
    if indices
        .iter()
        .enumerate()
        .all(|(row, &index)| index == start + row as i64)
    {
        PlotIndices::Dense {
            start,
            len: indices.len(),
        }
    } else {
        PlotIndices::Sparse(indices)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestPlot {
        list: PlotList,
        values: [Vec<f64>; 4],
    }

    impl TestPlot {
        fn new(indices: Vec<i64>, values: [Vec<f64>; 4]) -> Self {
            let mut list = PlotList::new();
            list.set_indices(indices);
            Self { list, values }
        }

        fn view(&self) -> PlotListView<'_> {
            PlotListView::new(
                &self.list,
                PlotValues::Ohlc([
                    &self.values[0],
                    &self.values[1],
                    &self.values[2],
                    &self.values[3],
                ]),
            )
        }

        fn min_max(&mut self, start: i64, end: i64, plots: &[PlotValueIndex]) -> Option<MinMax> {
            self.list.min_max_on_range_cached(
                PlotValues::Ohlc([
                    &self.values[0],
                    &self.values[1],
                    &self.values[2],
                    &self.values[3],
                ]),
                start,
                end,
                plots,
            )
        }
    }

    fn make_list(n: i64) -> TestPlot {
        TestPlot::new(
            (0..n).collect(),
            [
                (0..n).map(|i| i as f64 + 5.0).collect(),
                (0..n).map(|i| i as f64 + 10.0).collect(),
                (0..n).map(|i| i as f64).collect(),
                (0..n).map(|i| i as f64 + 7.0).collect(),
            ],
        )
    }

    #[test]
    fn search_modes() {
        let plot = TestPlot::new(vec![2, 5, 9], std::array::from_fn(|_| vec![1.0, 2.0, 3.0]));
        let pl = plot.view();
        assert_eq!(pl.search(5, MismatchDirection::None), Some(1));
        assert_eq!(pl.search(4, MismatchDirection::None), None);
        assert_eq!(pl.search(4, MismatchDirection::NearestLeft), Some(0));
        assert_eq!(pl.search(4, MismatchDirection::NearestRight), Some(1));
        assert_eq!(pl.search(1, MismatchDirection::NearestLeft), None);
        assert_eq!(pl.search(10, MismatchDirection::NearestRight), None);
    }

    #[test]
    fn min_max_matches_brute_force_across_chunks() {
        let mut pl = make_list(200);
        for (start, end) in [
            (0i64, 199i64),
            (5, 25),
            (29, 31),
            (30, 89),
            (61, 150),
            (100, 100),
        ] {
            let cached = pl
                .min_max(start, end, &[PlotValueIndex::Low, PlotValueIndex::High])
                .unwrap();
            // brute force: low = i, high = i + 10
            assert_eq!(cached.min, start as f64, "range {start}..{end}");
            assert_eq!(cached.max, end as f64 + 10.0, "range {start}..{end}");
        }
    }

    #[test]
    fn min_max_cache_is_consistent_on_repeat() {
        let mut pl = make_list(500);
        let a = pl.min_max(50, 450, &[PlotValueIndex::Low]).unwrap();
        let b = pl.min_max(50, 450, &[PlotValueIndex::Low]).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.min, 50.0);
        assert_eq!(a.max, 450.0);
    }

    #[test]
    fn min_max_clamps_to_data_bounds() {
        let mut pl = make_list(10);
        let mm = pl.min_max(-100, 100, &[PlotValueIndex::Close]).unwrap();
        assert_eq!(mm.min, 7.0);
        assert_eq!(mm.max, 16.0);
    }

    #[test]
    fn nan_values_are_skipped() {
        let mut pl = TestPlot::new(
            vec![0, 1, 2],
            std::array::from_fn(|_| vec![1.0, f64::NAN, 3.0]),
        );
        let mm = pl.min_max(0, 2, &[PlotValueIndex::Close]).unwrap();
        assert_eq!(mm.min, 1.0);
        assert_eq!(mm.max, 3.0);
    }

    #[test]
    fn upsert_last_invalidates_chunk_cache() {
        let mut pl = make_list(100);
        // warm the cache
        let before = pl.min_max(0, 99, &[PlotValueIndex::High]).unwrap();
        assert_eq!(before.max, 109.0);
        // replace last bar with a spike
        pl.values[1][99] = 999.0;
        pl.list.upsert_last(99);
        let after = pl.min_max(0, 99, &[PlotValueIndex::High]).unwrap();
        assert_eq!(after.max, 999.0);
        // append a new bar
        for (column, value) in pl.values.iter_mut().zip([1.0, 2000.0, 0.5, 1.5]) {
            column.push(value);
        }
        pl.list.upsert_last(100);
        let appended = pl.min_max(0, 100, &[PlotValueIndex::High]).unwrap();
        assert_eq!(appended.max, 2000.0);
    }

    #[test]
    fn sparse_indices_whitespace() {
        let mut pl = TestPlot::new(
            // data at indices 0, 10, 20 (whitespace between)
            vec![0, 10, 20],
            [
                vec![1.0, 5.0, 3.0],
                vec![2.0, 6.0, 4.0],
                vec![0.5, 4.0, 2.0],
                vec![1.5, 5.5, 3.5],
            ],
        );
        let mm = pl.min_max(5, 15, &[PlotValueIndex::High]).unwrap();
        assert_eq!(mm.max, 6.0); // only index 10 in range
        assert_eq!(
            pl.view().search(15, MismatchDirection::NearestLeft),
            Some(1)
        );
    }

    #[test]
    fn whitespace_rows_are_skipped_by_non_whitespace_searches() {
        // bars at 0, 3; explicit whitespace rows at 1, 2 (all-NaN, reference `{time}`-only items)
        let nan = f64::NAN;
        let mut pl = TestPlot::new(
            vec![0, 1, 2, 3],
            std::array::from_fn(|_| vec![1.0, nan, nan, 4.0]),
        );
        let view = pl.view();
        assert!(view.is_whitespace_row(1));
        assert!(view.is_whitespace_row(2));
        assert!(!view.is_whitespace_row(0));
        assert!(!view.is_whitespace_row(3));

        // last-value tracking scans left past whitespace (series.ts lastValueData)
        assert_eq!(view.last_non_whitespace_row(3), Some(3));
        assert_eq!(view.last_non_whitespace_row(2), Some(0));
        assert_eq!(view.last_non_whitespace_row(10), Some(3));
        // first-value selection scans right past whitespace
        assert_eq!(view.first_non_whitespace_row(0), Some(0));
        assert_eq!(view.first_non_whitespace_row(1), Some(3));
        assert_eq!(view.first_non_whitespace_row(-5), Some(0));
        // min/max already ignores the NaN rows
        let mm = pl.min_max(0, 3, &[PlotValueIndex::Close]).unwrap();
        assert_eq!(mm.min, 1.0);
        assert_eq!(mm.max, 4.0);
    }

    #[test]
    fn all_whitespace_list_has_no_non_whitespace_rows() {
        let nan = f64::NAN;
        let mut pl = TestPlot::new(vec![0, 1], std::array::from_fn(|_| vec![nan, nan]));
        let view = pl.view();
        assert_eq!(view.last_non_whitespace_row(1), None);
        assert_eq!(view.first_non_whitespace_row(0), None);
        assert_eq!(pl.min_max(0, 1, &[PlotValueIndex::Close]), None);
    }
}
