//! Multi-series data layer. Port of the merged-time-point concept from `src/model/data-layer.ts`,
//! restructured for our SoA storage.
//!
//! The horizontal axis is indexed by position in the **union** of every series' timestamps
//! (the "merged time points"). Each series maps its own data onto those indices; a series with
//! no point at a given index is simply absent there (whitespace). This is what lets a price
//! series and a volume series — or a candlestick and a moving-average overlay — share one time
//! scale even when their sample sets differ.
//!
//! A series opted into [`TimeAlignment::AsOf`] (an overlay from another market calendar) never
//! joins that union. Each merged point up to the union's last real bar shows its last row at or
//! before the point's time instead, through a sparse plot-row -> canonical-row map; values are
//! not copied.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::OnceLock;

use crate::helpers::algorithms::lower_bound;
use crate::model::lod::LodPyramid;
use crate::model::plot_list::{PlotList, PlotListView, PlotValueIndex, PlotValues};
use crate::TimePointIndex;

/// Opaque chart-local series identity. It is deliberately not a storage position: removed
/// identities are never reused, while their storage slots are.
pub type SeriesId = u32;

/// Recoverable result of resolving a consumer-provided series identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeriesIdError {
    Unknown(SeriesId),
    Stale(SeriesId),
}

/// Per-data-item color channels (reference data-item colors, model/series-bar-colorer.ts). `Body` is
/// the candle/bar body color, the line/area stroke (reference `lineColor` on area items) and point
/// marker, and the histogram column color; `Wick`/`Border` are the candlestick parts
/// (`wickColor`/`borderColor`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointColorChannel {
    Body = 0,
    Wick = 1,
    Border = 2,
}

/// Number of [`PointColorChannel`] slots carried per row.
pub const POINT_COLOR_CHANNELS: usize = 3;

/// A `0` entry in a present channel means "no override at this row" (a fully transparent color
/// is not renderable, so 0 is reserved as the absent marker).
pub const POINT_COLOR_ABSENT: u32 = 0;

/// Read-only per-row color columns resolved once from an opaque series identity. Frame builders
/// keep this view across their row loop so identity lookup never becomes per-point work. Rows are
/// plot rows: an as-of aligned series reads the canonical row each plot row shows.
#[derive(Clone, Copy)]
pub struct PointColors<'a> {
    channels: [&'a [u32]; POINT_COLOR_CHANNELS],
    rows: Option<&'a [u32]>,
}

impl PointColors<'_> {
    pub fn color(self, channel: PointColorChannel, row: usize) -> Option<u32> {
        let channel = self.channels[channel as usize];
        if channel.is_empty() {
            return None;
        }
        let row = match self.rows {
            Some(rows) => *rows.get(row)? as usize,
            None => row,
        };
        channel
            .get(row)
            .copied()
            .filter(|&color| color != POINT_COLOR_ABSENT)
    }
}

/// How an owned series' rows land on the shared time axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimeAlignment {
    /// The series' timestamps join the merged time points (the default, reference behavior).
    #[default]
    Union,
    /// The series never adds time points. Each merged point, up to the last point where a union
    /// series holds a real bar, shows the series' last row at or before the point's time (an
    /// as-of join): rows between two points collapse into the later one, and a point with no
    /// newer row repeats the previous one. With `max_staleness` (seconds), a point whose as-of row
    /// is older than that stays empty instead; `None` repeats without limit.
    AsOf { max_staleness: Option<i64> },
}

impl TimeAlignment {
    pub fn is_as_of(self) -> bool {
        matches!(self, Self::AsOf { .. })
    }

    fn max_staleness(self) -> Option<i64> {
        match self {
            Self::AsOf { max_staleness } => max_staleness,
            Self::Union => None,
        }
    }
}

enum SeriesValues {
    Single(Vec<f64>),
    Ohlc([Vec<f64>; 4]),
}

impl SeriesValues {
    fn len(&self) -> usize {
        match self {
            Self::Single(values) => values.len(),
            Self::Ohlc(values) => values[0].len(),
        }
    }

    fn view(&self) -> PlotValues<'_> {
        match self {
            Self::Single(values) => PlotValues::Single(values),
            Self::Ohlc(values) => {
                PlotValues::Ohlc([&values[0], &values[1], &values[2], &values[3]])
            }
        }
    }

    fn columns(&self) -> [&[f64]; 4] {
        match self {
            Self::Single(values) => [values, values, values, values],
            Self::Ohlc(values) => [&values[0], &values[1], &values[2], &values[3]],
        }
    }

    fn set_row(&mut self, row: usize, values: [f64; 4]) {
        if values.iter().all(|&value| value == values[0]) {
            match self {
                Self::Single(column) => column[row] = values[0],
                Self::Ohlc(columns) => {
                    for (column, value) in columns.iter_mut().zip(values) {
                        column[row] = value;
                    }
                }
            }
        } else {
            self.ensure_ohlc();
            let Self::Ohlc(columns) = self else {
                unreachable!()
            };
            for (column, value) in columns.iter_mut().zip(values) {
                column[row] = value;
            }
        }
    }

    fn push(&mut self, values: [f64; 4]) {
        if values.iter().all(|&value| value == values[0]) {
            match self {
                Self::Single(column) => column.push(values[0]),
                Self::Ohlc(columns) => {
                    for (column, value) in columns.iter_mut().zip(values) {
                        column.push(value);
                    }
                }
            }
        } else {
            self.ensure_ohlc();
            let Self::Ohlc(columns) = self else {
                unreachable!()
            };
            for (column, value) in columns.iter_mut().zip(values) {
                column.push(value);
            }
        }
    }

    fn insert(&mut self, row: usize, values: [f64; 4]) {
        if values.iter().all(|&value| value == values[0]) {
            match self {
                Self::Single(column) => column.insert(row, values[0]),
                Self::Ohlc(columns) => {
                    for (column, value) in columns.iter_mut().zip(values) {
                        column.insert(row, value);
                    }
                }
            }
        } else {
            self.ensure_ohlc();
            let Self::Ohlc(columns) = self else {
                unreachable!()
            };
            for (column, value) in columns.iter_mut().zip(values) {
                column.insert(row, value);
            }
        }
    }

    fn truncate(&mut self, len: usize) {
        match self {
            Self::Single(values) => values.truncate(len),
            Self::Ohlc(values) => values.iter_mut().for_each(|column| column.truncate(len)),
        }
    }

    fn drain_front(&mut self, count: usize) {
        match self {
            Self::Single(values) => {
                values.drain(..count);
            }
            Self::Ohlc(values) => {
                for column in values {
                    column.drain(..count);
                }
            }
        }
    }

    fn ensure_ohlc(&mut self) {
        if let Self::Single(values) = self {
            let values = std::mem::take(values);
            *self = Self::Ohlc([values.clone(), values.clone(), values.clone(), values]);
        }
    }

    fn logical_bytes(&self) -> usize {
        self.len()
            * std::mem::size_of::<f64>()
            * if matches!(self, Self::Single(_)) {
                1
            } else {
                4
            }
    }

    fn capacity_bytes(&self) -> usize {
        match self {
            Self::Single(values) => values.capacity() * std::mem::size_of::<f64>(),
            Self::Ohlc(values) => values
                .iter()
                .map(|column| column.capacity() * std::mem::size_of::<f64>())
                .sum(),
        }
    }
}

struct RawSeries {
    times: Vec<i64>,
    /// Indicator outputs share a contiguous source-time range by identity instead of retaining
    /// another timestamp column. Ordinary host series keep owned times.
    time_alias: Option<TimeAlias>,
    values: SeriesValues,
    /// Per-row color overrides indexed by `PointColorChannel`. Each channel is either empty
    /// (absent for the whole series) or aligned 1:1 with `times`; kept in lockstep with the
    /// value columns across set_data/update so plot rows (which mirror raw rows) stay aligned.
    point_colors: [Vec<u32>; POINT_COLOR_CHANNELS],
    /// Custom series (plugin platform Phase C-c): their values live host-side, so the rows
    /// here carry times only (whitespace-style) — yet they still mark real bars for the
    /// time-scale base index (the reference's custom plot rows carry values, so they count in
    /// `_getBaseIndex`).
    rows_count_as_data: bool,
    /// Rebuilt against merged indices; keys are positions in `merged_times`.
    plot: PlotList,
    /// Compact endpoint and extrema source-row identities used only for dense viewport queries.
    /// Rows are plot rows: canonical rows, or merged points for an as-of mapped plot.
    lod: LodPyramid,
    last_lod_update_nodes: usize,
    /// Changes on every accepted mutation of this series' canonical rows. Derived-data
    /// runtimes use it to reject incremental continuation from stale source state.
    generation: u64,
    /// How an owned series lands on the merged time points. Aliased outputs follow their source.
    alignment: TimeAlignment,
}

#[derive(Clone, Copy)]
struct TimeAlias {
    source: SeriesId,
    offset: usize,
    len: usize,
}

impl RawSeries {
    fn empty() -> Self {
        Self {
            times: Vec::new(),
            time_alias: None,
            values: SeriesValues::Single(Vec::new()),
            point_colors: [vec![], vec![], vec![]],
            rows_count_as_data: false,
            plot: PlotList::new(),
            lod: LodPyramid::default(),
            last_lod_update_nodes: 0,
            generation: 0,
            alignment: TimeAlignment::Union,
        }
    }

    /// Canonical-row LOD maintenance for a mutation. An as-of series summarizes plot rows
    /// instead, so its row-map maintenance repairs the LOD once the map is current.
    fn rebuild_lod(&mut self) {
        if self.alignment.is_as_of() {
            return;
        }
        self.lod
            .rebuild(self.values.view().with_rows(self.plot.source_rows()));
        self.last_lod_update_nodes = self.lod.node_count();
    }

    fn rebuild_lod_range(&mut self, affected: Range<usize>) {
        if self.alignment.is_as_of() {
            return;
        }
        self.last_lod_update_nodes = self
            .lod
            .rebuild_range(
                self.values.view().with_rows(self.plot.source_rows()),
                affected,
            )
            .nodes_updated;
    }

    /// Repair the LOD over plot rows `affected` after the row map, or the values those rows
    /// show, changed. The node holding the last surviving row is always refreshed, so a
    /// shortened plot cannot keep a summary naming a dropped row.
    fn repair_plot_lod(&mut self, affected: Range<usize>) {
        let values = self.values.view().with_rows(self.plot.source_rows());
        let len = values.len();
        let start = affected.start.min(len.saturating_sub(1));
        let end = affected.end.min(len).max(start + 1);
        self.last_lod_update_nodes = self.lod.rebuild_range(values, start..end).nodes_updated;
    }

    fn rebuild_plot_lod(&mut self) {
        self.lod
            .rebuild(self.values.view().with_rows(self.plot.source_rows()));
        self.last_lod_update_nodes = self.lod.node_count();
    }

    /// Whether this series' own timestamps join the merged union (owned and not as-of).
    fn joins_union(&self) -> bool {
        self.time_alias.is_none() && !self.alignment.is_as_of()
    }
}

/// Most union series the timestamp merge handles as sorted runs; a wider layer sorts the
/// concatenation instead.
const MAX_MERGED_RUNS: usize = 16;

/// Write the sorted, duplicate-free union of ascending `runs` to `out`, consuming the slices.
fn merge_sorted_runs(runs: &mut [&[i64]], out: &mut Vec<i64>) {
    if let [only] = runs {
        out.extend_from_slice(only);
        out.dedup();
        return;
    }
    while let Some(&next) = runs.iter().filter_map(|run| run.first()).min() {
        if out.last() != Some(&next) {
            out.push(next);
        }
        for run in runs.iter_mut() {
            if run.first() == Some(&next) {
                *run = &run[1..];
            }
        }
    }
}

#[derive(Default)]
pub struct DataLayer {
    series: Vec<RawSeries>,
    live_slots: HashMap<SeriesId, usize>,
    free_slots: Vec<usize>,
    next_series_id: SeriesId,
    merged_times: Vec<i64>,
    merged_times_scratch: Vec<i64>,
    /// Changes only when the merged timestamp sequence changes. Value-only current-bar updates
    /// leave it untouched, so time-derived consumers can distinguish them without rescanning.
    time_points_generation: u64,
    /// Changes whenever an existing time point can move or leave: anything but appending points
    /// after the last one. While it holds, every point below the old length keeps its index.
    time_index_generation: u64,
    /// Old union for the current owner transaction, captured only when a rebuild actually changes
    /// logical indices and consumed when the owner synchronizes the final time scale.
    merged_time_rebase_source: Option<Vec<i64>>,
    capture_merged_time_rebase: bool,
    /// Inclusive whole-second visibility boundary supplied by the chart replay clock. Canonical
    /// rows remain retained; merged indices and public/derived views expose only this prefix.
    time_cutoff: Option<i64>,
    /// Lifetime count of whole-layer union merges plus reindex passes. Work telemetry for tests
    /// that prove a historical value correction stays local.
    index_rebuilds: u64,
    /// Live owned series with [`TimeAlignment::AsOf`]; zero keeps every union path as before.
    as_of_series: usize,
    /// Merged points `[0, as_of_through)` carry as-of rows: through the last point where a union
    /// series holds a real bar, so an overlay never runs into future session slots.
    as_of_through: usize,
    /// Series whose as-of plot rows changed outside their own mutation (a new union point, a
    /// moved data extent, or a union reindex), drained by the owner to invalidate their retained
    /// frames.
    realigned: Vec<SeriesId>,
    /// Lifetime count of plot rows (re)written by as-of maintenance (work telemetry for tests
    /// that prove live tips stay proportional to the change).
    as_of_rows_rewritten: u64,
}

/// Piecewise-linear old-to-new logical-index mapping through timestamps present in both unions.
/// Exact common timestamps remain exact, while anchors outside the common extent extrapolate with
/// slope one. The data layer creates this only for a transaction that rebuilt the merged union.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergedTimeMapping {
    common_indices: Vec<(usize, usize)>,
    /// Old logical indices of the first and last common timestamp, retained even when the
    /// breakpoints collapse to identity. `None` means the unions share no timestamp.
    common_old_extent: Option<(usize, usize)>,
    /// Whether at least two common timestamps exist and every one keeps the same index offset
    /// (a same-resolution prepend, trim, or window shift rather than an interval change).
    translation: bool,
    /// The old merged union, moved out of the data layer so time-identity consumers can derive
    /// anchor times on the axis the logical indices referred to. Empty for mappings built
    /// directly from slices.
    old_times: Vec<i64>,
}

impl MergedTimeMapping {
    fn between(old: &[i64], new: &[i64]) -> Self {
        let mut common_indices = Vec::new();
        let mut first_common: Option<(usize, usize)> = None;
        let mut last_common: Option<(usize, usize)> = None;
        let mut offsets_equal = true;
        let (mut old_index, mut new_index) = (0, 0);
        while old_index < old.len() && new_index < new.len() {
            match old[old_index].cmp(&new[new_index]) {
                std::cmp::Ordering::Less => old_index += 1,
                std::cmp::Ordering::Greater => new_index += 1,
                std::cmp::Ordering::Equal => {
                    let current = (old_index, new_index);
                    let first = *first_common.get_or_insert(current);
                    offsets_equal &=
                        new_index as i128 - old_index as i128 == first.1 as i128 - first.0 as i128;
                    last_common = Some(current);
                    if common_indices.len() >= 2 {
                        let a: (usize, usize) = common_indices[common_indices.len() - 2];
                        let b: (usize, usize) = common_indices[common_indices.len() - 1];
                        let ab_old = (b.0 - a.0) as u128;
                        let ab_new = (b.1 - a.1) as u128;
                        let bc_old = (current.0 - b.0) as u128;
                        let bc_new = (current.1 - b.1) as u128;
                        if ab_old * bc_new == ab_new * bc_old {
                            *common_indices.last_mut().expect("two breakpoints exist") = current;
                        } else {
                            common_indices.push(current);
                        }
                    } else {
                        common_indices.push(current);
                    }
                    old_index += 1;
                    new_index += 1;
                }
            }
        }
        if common_indices.is_empty()
            || common_indices
                .iter()
                .all(|&(old_index, new_index)| old_index == new_index)
        {
            common_indices.clear();
        }
        let common_old_extent = first_common.zip(last_common).map(|(a, b)| (a.0, b.0));
        Self {
            common_indices,
            common_old_extent,
            translation: offsets_equal && first_common != last_common,
            old_times: Vec::new(),
        }
    }

    /// Old logical indices of the first and last timestamp present in both unions, or `None`
    /// when the unions share no timestamp (an interval switch or unrelated reload).
    pub fn common_old_extent(&self) -> Option<(usize, usize)> {
        self.common_old_extent
    }

    /// Whether the mapping is a pure index translation over at least two common timestamps.
    pub fn is_translation(&self) -> bool {
        self.translation
    }

    /// The old merged union the mapped logical indices referred to (empty when unknown).
    pub fn old_times(&self) -> &[i64] {
        &self.old_times
    }

    /// Whether any timestamp survived the transaction, i.e. whether [`Self::map_logical`] is
    /// anchored on real common points rather than the identity fallback for disjoint unions.
    pub fn has_common_time(&self) -> bool {
        self.common_old_extent.is_some()
    }

    pub fn map_logical(&self, logical: f64) -> f64 {
        if !logical.is_finite() || self.common_indices.is_empty() {
            return logical;
        }
        let upper = self
            .common_indices
            .partition_point(|&(old_index, _)| (old_index as f64) < logical);
        if let Some(&(old_index, new_index)) = self.common_indices.get(upper) {
            if old_index as f64 == logical {
                return new_index as f64;
            }
        }
        if upper == 0 {
            let (old_index, new_index) = self.common_indices[0];
            return new_index as f64 + logical - old_index as f64;
        }
        if upper == self.common_indices.len() {
            let (old_index, new_index) = self.common_indices[upper - 1];
            return new_index as f64 + logical - old_index as f64;
        }
        let (old_left, new_left) = self.common_indices[upper - 1];
        let (old_right, new_right) = self.common_indices[upper];
        let fraction = (logical - old_left as f64) / (old_right - old_left) as f64;
        new_left as f64 + fraction * (new_right - new_left) as f64
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DataLayerMemoryUsage {
    pub rows: usize,
    pub owned_time_bytes: usize,
    pub canonical_value_bytes: usize,
    pub plot_index_bytes: usize,
    pub merged_time_bytes: usize,
    pub point_color_bytes: usize,
    pub autoscale_cache_bytes: usize,
    pub lod_bytes: usize,
    pub scratch_capacity_bytes: usize,
    pub allocated_capacity_bytes: usize,
    pub aligned_series: usize,
    pub dense_index_series: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SeriesMemoryUsage {
    pub rows: usize,
    pub owned_time_bytes: usize,
    pub canonical_value_bytes: usize,
    pub plot_index_bytes: usize,
    pub point_color_bytes: usize,
    pub lod_bytes: usize,
    pub aligned_time_view: bool,
    pub dense_index_view: bool,
}

impl DataLayerMemoryUsage {
    pub fn logical_payload_bytes(self) -> usize {
        self.owned_time_bytes
            + self.canonical_value_bytes
            + self.plot_index_bytes
            + self.merged_time_bytes
            + self.point_color_bytes
            + self.autoscale_cache_bytes
            + self.lod_bytes
    }
}

impl DataLayer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn time_cutoff(&self) -> Option<i64> {
        self.time_cutoff
    }

    /// Set the inclusive replay visibility boundary without deleting canonical rows.
    pub fn set_time_cutoff(&mut self, cutoff: Option<i64>) -> bool {
        if self.time_cutoff == cutoff {
            return false;
        }
        self.time_cutoff = cutoff;
        self.rebuild_merged();
        self.reindex_all();
        true
    }

    fn visible_len(&self, times: &[i64]) -> usize {
        self.time_cutoff.map_or(times.len(), |cutoff| {
            times.partition_point(|&time| time <= cutoff)
        })
    }

    /// Start an owner-level market-data transaction. This is allocation-free until a merged-union
    /// rebuild changes the sequence; tail appends and value-only replacements therefore stay free.
    pub fn begin_merged_time_transaction(&mut self) {
        self.capture_merged_time_rebase = true;
    }

    /// Finish the owner transaction against its final merged union.
    pub fn take_merged_time_mapping(&mut self) -> Option<MergedTimeMapping> {
        self.capture_merged_time_rebase = false;
        let old = self.merged_time_rebase_source.take()?;
        let mut mapping = MergedTimeMapping::between(&old, &self.merged_times);
        mapping.old_times = old;
        Some(mapping)
    }

    /// Choose how an owned series lands on the merged time points. Aliased indicator outputs
    /// follow their source and are refused, as are unknown ids and a negative staleness bound.
    /// A change rebuilds the union and every mapping once. Returns whether the policy changed.
    pub fn set_time_alignment(&mut self, id: SeriesId, alignment: TimeAlignment) -> bool {
        let Some(slot) = self.series_slot(id) else {
            return false;
        };
        if alignment.max_staleness().is_some_and(|max| max < 0) {
            return false;
        }
        let series = &mut self.series[slot];
        if series.time_alias.is_some() || series.alignment == alignment {
            return false;
        }
        let was_as_of = series.alignment.is_as_of();
        series.alignment = alignment;
        match (was_as_of, alignment.is_as_of()) {
            (false, true) => self.as_of_series += 1,
            (true, false) => self.as_of_series -= 1,
            _ => {}
        }
        self.rebuild_merged();
        self.reindex_all();
        if !alignment.is_as_of() {
            // Back on canonical rows: its summaries name canonical rows again.
            self.series[slot].rebuild_lod();
        }
        true
    }

    /// The alignment a series follows: its own, or its source's for an aliased output. `None`
    /// for an unknown or stale id.
    pub fn time_alignment(&self, id: SeriesId) -> Option<TimeAlignment> {
        let slot = self.series_slot(id)?;
        let owner = self
            .resolved_alias_range(slot)
            .map_or(slot, |(source, _, _)| source);
        Some(self.series[owner].alignment)
    }

    /// Series whose as-of plot rows changed as a side effect of another series' mutation (a new
    /// union point, a moved data extent, or any union reindex) since the previous call. The owner
    /// invalidates their retained presentation; each id appears once.
    pub fn take_realigned(&mut self) -> Vec<SeriesId> {
        std::mem::take(&mut self.realigned)
    }

    /// Lifetime count of plot rows (re)written by as-of maintenance (work telemetry).
    #[doc(hidden)]
    pub fn as_of_rows_rewritten(&self) -> u64 {
        self.as_of_rows_rewritten
    }

    /// Merged index of the last point where a host-owned union series holds a real row
    /// (whitespace excluded, time-only custom rows included), within the replay cutoff. As-of
    /// rows never extend past it, so an overlay stops where the chart's own bars stop instead of
    /// running into host-installed future session slots. `O(series × log rows)`.
    fn union_data_extent(&self) -> Option<usize> {
        let mut extent: Option<usize> = None;
        for &slot in self.live_slots.values() {
            let series = &self.series[slot];
            if series.time_alias.is_some()
                || series.alignment.is_as_of()
                || series.plot.source_rows().is_some()
            {
                continue;
            }
            let plot = PlotListView::with_lod(&series.plot, series.values.view(), &series.lod);
            let row = if series.rows_count_as_data {
                plot.size().checked_sub(1)
            } else {
                plot.last_non_whitespace_row_before(plot.size())
            };
            if let Some(index) = row.and_then(|row| plot.index_at(row)) {
                extent = extent.max(Some(index as usize));
            }
        }
        extent
    }

    fn as_of_slots(&self) -> Vec<usize> {
        if self.as_of_series == 0 {
            return Vec::new();
        }
        self.live_slots
            .values()
            .copied()
            .filter(|&slot| {
                self.series[slot].time_alias.is_none() && self.series[slot].alignment.is_as_of()
            })
            .collect()
    }

    fn note_realigned(&mut self, slot: usize) {
        let Some(id) = self
            .live_slots
            .iter()
            .find_map(|(&id, &candidate)| (candidate == slot).then_some(id))
        else {
            return;
        };
        if !self.realigned.contains(&id) {
            self.realigned.push(id);
        }
    }

    /// Rebuild every as-of mapping against the current union (outputs are re-aliased by the
    /// caller). A union reindex can move an overlay's points without changing the time points (a
    /// popped or replaced bar inside pre-installed session slots), so each one is reported.
    fn rebuild_as_of_all(&mut self) {
        self.as_of_through = if self.as_of_series == 0 {
            0
        } else {
            self.union_data_extent().map_or(0, |index| index + 1)
        };
        for slot in self.as_of_slots() {
            self.rebuild_as_of_rows(slot);
            self.note_realigned(slot);
        }
    }

    /// Rebuild one owned as-of series' mapping and plot LOD (outputs are re-aliased by callers).
    fn rebuild_as_of_rows(&mut self, slot: usize) {
        let cutoff = self.time_cutoff;
        let through = self.as_of_through;
        let series = &mut self.series[slot];
        let len = cutoff.map_or(series.times.len(), |cutoff| {
            series.times.partition_point(|&time| time <= cutoff)
        });
        series.plot.rebuild_as_of(
            &self.merged_times,
            through,
            &series.times[..len],
            series.alignment.max_staleness(),
        );
        series.rebuild_plot_lod();
        self.as_of_rows_rewritten = self
            .as_of_rows_rewritten
            .wrapping_add(series.plot.size() as u64);
    }

    /// Re-derive one owned as-of series' rows for the merged points `[from, as_of_through)`,
    /// then carry the change to the outputs aliasing it. Returns whether any plot row changed.
    fn resync_as_of_tail(&mut self, slot: usize, from: usize) -> bool {
        let cutoff = self.time_cutoff;
        let through = self.as_of_through;
        let series = &mut self.series[slot];
        let len = cutoff.map_or(series.times.len(), |cutoff| {
            series.times.partition_point(|&time| time <= cutoff)
        });
        let old_size = series.plot.size();
        let keep = series.plot.resync_as_of_tail(
            &self.merged_times,
            from,
            through,
            &series.times[..len],
            series.alignment.max_staleness(),
        );
        let size = series.plot.size();
        if keep == old_size && keep == size {
            return false;
        }
        series.repair_plot_lod(keep..size);
        self.as_of_rows_rewritten = self.as_of_rows_rewritten.wrapping_add((size - keep) as u64);
        self.realias_outputs(slot, keep);
        true
    }

    /// After a union change, carry the as-of rows up to the union's new data extent. Every
    /// point before the previous extent keeps its time, so only the tail is rewritten.
    fn sync_as_of_tail(&mut self) {
        if self.as_of_series == 0 {
            return;
        }
        let through = self.union_data_extent().map_or(0, |index| index + 1);
        if through == self.as_of_through {
            return;
        }
        let from = self.as_of_through.min(through);
        self.as_of_through = through;
        for slot in self.as_of_slots() {
            if self.resync_as_of_tail(slot, from) {
                self.note_realigned(slot);
            }
        }
    }

    /// Re-alias the outputs whose ultimate source is `source_slot`. The source's first `keep`
    /// plot rows are unchanged since those outputs were last aliased (0 = copy everything).
    fn realias_outputs(&mut self, source_slot: usize, keep: usize) {
        let aliases = self
            .live_slots
            .values()
            .filter_map(|&alias| {
                self.resolved_alias_range(alias)
                    .filter(|&(source, _, _)| source == source_slot)
                    .map(|range| (alias, range))
            })
            .collect::<Vec<_>>();
        for (alias, (source, offset, len)) in aliases {
            let first_changed = self.copy_plot_range(alias, source, offset, len, keep);
            let target = &mut self.series[alias];
            if keep == 0 {
                target.rebuild_plot_lod();
            } else {
                let size = target.plot.size();
                target.repair_plot_lod(first_changed..size);
            }
            self.note_realigned(alias);
        }
    }

    pub fn add_series(&mut self) -> SeriesId {
        let id = self.next_series_id;
        self.next_series_id = self
            .next_series_id
            .checked_add(1)
            .expect("series identity space exhausted");
        let slot = if let Some(slot) = self.free_slots.pop() {
            self.series[slot] = RawSeries::empty();
            slot
        } else {
            self.series.push(RawSeries::empty());
            self.series.len() - 1
        };
        self.live_slots.insert(id, slot);
        id
    }

    pub fn series_count(&self) -> usize {
        self.live_slots.len()
    }

    /// Allocated storage slots, exposed for bounded-storage diagnostics.
    pub fn slot_count(&self) -> usize {
        self.series.len()
    }

    pub fn memory_usage(&self) -> DataLayerMemoryUsage {
        let mut usage = DataLayerMemoryUsage {
            merged_time_bytes: self.merged_times.len() * std::mem::size_of::<i64>(),
            scratch_capacity_bytes: self.merged_times_scratch.capacity()
                * std::mem::size_of::<i64>()
                + self
                    .merged_time_rebase_source
                    .as_ref()
                    .map_or(0, |times| times.capacity() * std::mem::size_of::<i64>()),
            ..DataLayerMemoryUsage::default()
        };
        for &slot in self.live_slots.values() {
            let series = &self.series[slot];
            usage.rows += series.values.len();
            usage.owned_time_bytes += series.times.len() * std::mem::size_of::<i64>();
            usage.canonical_value_bytes += series.values.logical_bytes();
            usage.plot_index_bytes += series.plot.index_bytes();
            usage.point_color_bytes += series
                .point_colors
                .iter()
                .map(|colors| colors.len() * std::mem::size_of::<u32>())
                .sum::<usize>();
            usage.autoscale_cache_bytes += series.plot.cache_payload_bytes();
            usage.lod_bytes += series.lod.logical_bytes();
            usage.allocated_capacity_bytes += series.times.capacity() * std::mem::size_of::<i64>()
                + series.values.capacity_bytes()
                + series.plot.index_capacity_bytes()
                + series.lod.capacity_bytes()
                + series
                    .point_colors
                    .iter()
                    .map(|colors| colors.capacity() * std::mem::size_of::<u32>())
                    .sum::<usize>();
            usage.aligned_series += usize::from(series.time_alias.is_some());
            usage.dense_index_series += usize::from(series.plot.is_dense());
        }
        usage.allocated_capacity_bytes += self.merged_times.capacity() * std::mem::size_of::<i64>()
            + usage.scratch_capacity_bytes;
        usage
    }

    /// Canonical rows a series stores, including rows past the replay time cutoff that its
    /// queries do not expose yet.
    pub fn series_rows(&self, id: SeriesId) -> Option<usize> {
        Some(self.series.get(self.series_slot(id)?)?.values.len())
    }

    pub fn series_memory_usage(&self, id: SeriesId) -> Option<SeriesMemoryUsage> {
        let slot = self.series_slot(id)?;
        let series = &self.series[slot];
        Some(SeriesMemoryUsage {
            rows: series.values.len(),
            owned_time_bytes: series.times.len() * std::mem::size_of::<i64>(),
            canonical_value_bytes: series.values.logical_bytes(),
            plot_index_bytes: series.plot.index_bytes(),
            point_color_bytes: series
                .point_colors
                .iter()
                .map(|colors| colors.len() * std::mem::size_of::<u32>())
                .sum(),
            lod_bytes: series.lod.logical_bytes(),
            aligned_time_view: series.time_alias.is_some(),
            dense_index_view: series.plot.is_dense(),
        })
    }

    /// Current storage position for a live opaque id.
    pub fn series_slot(&self, id: SeriesId) -> Option<usize> {
        self.live_slots.get(&id).copied()
    }

    pub fn validate_series_id(&self, id: SeriesId) -> Result<(), SeriesIdError> {
        if self.live_slots.contains_key(&id) {
            Ok(())
        } else if id < self.next_series_id {
            Err(SeriesIdError::Stale(id))
        } else {
            Err(SeriesIdError::Unknown(id))
        }
    }

    fn materialize_time_alias(&mut self, id: SeriesId) -> Option<usize> {
        let slot = self.series_slot(id)?;
        if self.series[slot].time_alias.is_some() {
            let times = self.series_times_by_slot(slot)?.to_vec();
            // An output of an as-of source keeps following that calendar once it owns its
            // times; its copied row map already describes those rows.
            let alignment = self.time_alignment(id).unwrap_or_default();
            self.series[slot].times = times;
            self.series[slot].time_alias = None;
            self.series[slot].alignment = alignment;
            self.as_of_series += usize::from(alignment.is_as_of());
        }
        Some(slot)
    }

    /// Release a live series and make its storage reusable. Returns false for unknown/stale ids.
    pub fn remove_series(&mut self, id: SeriesId) -> bool {
        let dependents = self
            .live_slots
            .iter()
            .filter_map(|(&candidate, &slot)| {
                (candidate != id
                    && self.series[slot]
                        .time_alias
                        .is_some_and(|alias| alias.source == id))
                .then_some(candidate)
            })
            .collect::<Vec<_>>();
        for dependent in dependents {
            self.materialize_time_alias(dependent);
        }
        let Some(slot) = self.live_slots.remove(&id) else {
            return false;
        };
        if self.series[slot].time_alias.is_none() && self.series[slot].alignment.is_as_of() {
            self.as_of_series -= 1;
        }
        self.realigned.retain(|&realigned| realigned != id);
        self.series[slot] = RawSeries::empty();
        self.free_slots.push(slot);
        self.rebuild_merged();
        self.reindex_all();
        true
    }

    /// Union of all series' timestamps, sorted ascending (the time-scale points).
    pub fn merged_times(&self) -> &[i64] {
        &self.merged_times
    }

    /// Monotonic identity for the complete merged timestamp sequence.
    pub fn time_points_generation(&self) -> u64 {
        self.time_points_generation
    }

    /// Identity of the merged points' positions: unchanged while the union only grows at its end,
    /// so a consumer keyed on it may keep per-index work for the points it already read.
    pub fn time_index_generation(&self) -> u64 {
        self.time_index_generation
    }

    /// Plot data for a live series. Unknown/stale ids safely read as an empty plot; callers that
    /// need to distinguish that case use [`Self::try_plot`].
    pub fn plot(&self, id: SeriesId) -> PlotListView<'_> {
        self.try_plot(id).unwrap_or_else(|| {
            static EMPTY: OnceLock<PlotList> = OnceLock::new();
            PlotListView::new(EMPTY.get_or_init(PlotList::new), PlotValues::Single(&[]))
        })
    }

    /// Plot data for a live series, or `None` for an unknown/stale id.
    pub fn try_plot(&self, id: SeriesId) -> Option<PlotListView<'_>> {
        let slot = self.series_slot(id)?;
        let series = &self.series[slot];
        Some(PlotListView::with_lod(
            &series.plot,
            series.values.view(),
            &series.lod,
        ))
    }

    pub fn min_max_on_range_cached(
        &mut self,
        id: SeriesId,
        start: TimePointIndex,
        end: TimePointIndex,
        plots: &[PlotValueIndex],
    ) -> Option<crate::model::plot_list::MinMax> {
        let slot = self.series_slot(id)?;
        let series = &mut self.series[slot];
        series
            .plot
            .min_max_on_range_cached(series.values.view(), start, end, plots)
    }

    /// Mark a series whose time-only rows still count as data rows for [`base_index`] (custom
    /// series, Phase C-c). Set at series creation / kind conversion by the engine, which owns
    /// the kind knowledge; an unknown id is ignored.
    pub fn set_rows_count_as_data(&mut self, id: SeriesId, flag: bool) {
        if let Some(s) = self
            .series_slot(id)
            .and_then(|slot| self.series.get_mut(slot))
        {
            s.rows_count_as_data = flag;
            self.sync_as_of_tail();
        }
    }

    /// Raw series columns for platform-independent derived-data producers.
    pub fn series_data(&self, id: SeriesId) -> Option<(&[i64], [&[f64]; 4])> {
        let slot = self.series_slot(id)?;
        let s = self.series.get(slot)?;
        let times = self.series_times_by_slot(slot)?;
        let len = self.visible_len(times).min(s.values.len());
        let columns = s.values.columns();
        Some((
            &times[..len],
            [
                &columns[0][..len],
                &columns[1][..len],
                &columns[2][..len],
                &columns[3][..len],
            ],
        ))
    }

    fn series_times_by_slot(&self, mut slot: usize) -> Option<&[i64]> {
        let mut offset = 0usize;
        let mut len = self.series.get(slot)?.values.len();
        for _ in 0..self.series.len().max(1) {
            let series = self.series.get(slot)?;
            let Some(alias) = series.time_alias else {
                let end = offset.saturating_add(len).min(series.times.len());
                return series.times.get(offset.min(end)..end);
            };
            offset = offset.checked_add(alias.offset)?;
            len = len.min(alias.len);
            slot = self.series_slot(alias.source)?;
        }
        debug_assert!(false, "series time alias cycle");
        None
    }

    pub fn series_generation(&self, id: SeriesId) -> Option<u64> {
        Some(self.series.get(self.series_slot(id)?)?.generation)
    }

    /// Lifetime count of timestamp-union merges and plot reindex passes (work telemetry).
    #[doc(hidden)]
    pub fn index_rebuilds(&self) -> u64 {
        self.index_rebuilds
    }

    #[doc(hidden)]
    pub fn last_lod_update_nodes(&self, id: SeriesId) -> Option<usize> {
        Some(
            self.series
                .get(self.series_slot(id)?)?
                .last_lod_update_nodes,
        )
    }

    /// Merged index of the last point that has data (the time-scale base index), or None.
    /// Whitespace rows (reference `{time}`-only items) occupy time points but carry no data, so —
    /// like the reference's `_getBaseIndex` (data-layer.ts:495-510), which reads the whitespace-filtered
    /// series rows — the base index is the last point holding a real bar in any series. When
    /// every series' rows are whitespace the index is 0 (the reference's initialized `baseIndex`).
    /// As-of aligned series (and their outputs) own no points and never reach past the union's
    /// last real bar, so they are not consulted.
    pub fn base_index(&self) -> Option<TimePointIndex> {
        if self.merged_times.is_empty() {
            return None;
        }
        let mut last_data_time: Option<i64> = None;
        for &slot in self.live_slots.values() {
            let s = &self.series[slot];
            if s.alignment.is_as_of() || s.plot.source_rows().is_some() {
                continue;
            }
            let Some(times) = self.series_times_by_slot(slot) else {
                continue;
            };
            let rows = s.values.len().min(times.len());
            // The LOD pyramid finds the last non-whitespace row in a logarithmic number of
            // summaries, so trailing whitespace (pre-installed session slots) costs nothing here.
            let row = if s.rows_count_as_data {
                rows.checked_sub(1)
            } else {
                s.lod.view(s.values.view()).last_row_before(rows)
            };
            if let Some(row) = row {
                last_data_time = Some(match last_data_time {
                    Some(t) => t.max(times[row]),
                    None => times[row],
                });
            }
        }
        match last_data_time {
            // The time came from a series, so it is in the merged union by construction; fall
            // back to the insertion point instead of panicking if that invariant ever breaks.
            Some(t) => Some(
                self.merged_times
                    .binary_search(&t)
                    .unwrap_or_else(|pos| pos.min(self.merged_times.len() - 1))
                    as TimePointIndex,
            ),
            None => Some(0),
        }
    }

    /// Full (re)assignment of a series' data. `times` must be ascending. Rebuilds the merged
    /// time points and re-maps every series onto the new index space. Resets the series'
    /// per-point colors (the host re-installs them against the new rows afterwards).
    pub fn set_data(
        &mut self,
        id: SeriesId,
        times: Vec<i64>,
        open: Vec<f64>,
        high: Vec<f64>,
        low: Vec<f64>,
        close: Vec<f64>,
    ) -> bool {
        debug_assert!(
            times.windows(2).all(|w| w[0] < w[1]),
            "series times must be ascending unique"
        );
        let Some(slot) = self.series_slot(id) else {
            return false;
        };
        let s = &mut self.series[slot];
        s.times = times;
        s.time_alias = None;
        s.values = SeriesValues::Ohlc([open, high, low, close]);
        s.rebuild_lod();
        s.point_colors = [vec![], vec![], vec![]];
        s.generation = s.generation.wrapping_add(1);
        self.rebuild_merged();
        self.reindex_all();
        true
    }

    /// Full assignment for a scalar series. The one value column is canonical and exposed
    /// through all four reference-compatible plot slots without four stored copies.
    pub fn set_single_data(&mut self, id: SeriesId, times: Vec<i64>, values: Vec<f64>) -> bool {
        debug_assert_eq!(times.len(), values.len());
        debug_assert!(times.windows(2).all(|window| window[0] < window[1]));
        let Some(slot) = self.series_slot(id) else {
            return false;
        };
        let series = &mut self.series[slot];
        series.times = times;
        series.time_alias = None;
        series.values = SeriesValues::Single(values);
        series.rebuild_lod();
        series.point_colors = [vec![], vec![], vec![]];
        series.generation = series.generation.wrapping_add(1);
        self.rebuild_merged();
        self.reindex_all();
        true
    }

    /// Install a scalar series whose timestamps are a contiguous suffix/range of another live
    /// series. Only the source identity and row range are retained; values remain canonically
    /// owned by this series and no timestamp/index columns are duplicated.
    pub fn set_single_data_aligned(
        &mut self,
        id: SeriesId,
        source: SeriesId,
        source_from: usize,
        values: Vec<f64>,
    ) -> bool {
        let Some(target_slot) = self.series_slot(id) else {
            return false;
        };
        let Some(source_slot) = self.series_slot(source) else {
            return false;
        };
        if target_slot == source_slot {
            return false;
        }
        let Some(source_times) = self.series_times_by_slot(source_slot) else {
            return false;
        };
        let source_len = self.visible_len(source_times);
        if source_from + values.len() != source_len {
            return false;
        }
        if self.series[target_slot].time_alias.is_none()
            && self.series[target_slot].alignment.is_as_of()
        {
            // An alias follows its source's alignment; a materialized as-of output gives up its own.
            self.as_of_series -= 1;
            self.series[target_slot].alignment = TimeAlignment::Union;
        }
        {
            let target = &mut self.series[target_slot];
            target.times = Vec::new();
            target.time_alias = Some(TimeAlias {
                source,
                offset: source_from,
                len: values.len(),
            });
            target.values = SeriesValues::Single(values);
            target.point_colors = [vec![], vec![], vec![]];
            target.generation = target.generation.wrapping_add(1);
        }
        let aliasable = self.aliasable_rows(source_slot);
        self.copy_plot_range(
            target_slot,
            source_slot,
            source_from.min(aliasable),
            aliasable.saturating_sub(source_from),
            0,
        );
        // Summaries name plot rows, which an as-of source only fixes once the alias is copied.
        self.series[target_slot].rebuild_plot_lod();
        true
    }

    /// Replace an aligned scalar output suffix after an incremental source repair. The unchanged
    /// prefix and its colors stay in place; current replacement and append remain tail-local.
    pub fn update_single_aligned(
        &mut self,
        id: SeriesId,
        source: SeriesId,
        source_from: usize,
        values: &[f64],
    ) -> Option<usize> {
        let target_slot = self.series_slot(id)?;
        let source_slot = self.series_slot(source)?;
        let alias = self.series[target_slot].time_alias?;
        if alias.source != source {
            return None;
        }
        // Before an indicator reaches warm-up it has no output rows, so there is nothing to stay
        // aligned with: the output starts where this update's first value lands. That keeps an
        // empty alias at the current source suffix, so successive appends do not manufacture a
        // gap, and a batch that rewrites earlier source rows and completes the warm-up in one step
        // starts the output at its first value instead of reading as a gap.
        let alias_offset = if self.series[target_slot].values.len() == 0 {
            source_from
        } else {
            alias.offset
        };
        if source_from < alias_offset {
            return None;
        }
        let source_len = self.visible_len(self.series_times_by_slot(source_slot)?);
        if source_from + values.len() != source_len {
            return None;
        }
        let output_row = source_from - alias_offset;
        let source_plot_len = self.aliasable_rows(source_slot);
        let plot_offset = alias_offset.min(source_plot_len);
        let output_len = source_len
            .saturating_sub(alias_offset)
            .min(source_plot_len - plot_offset);
        let changed_from = output_row.min(output_len);
        if output_row > self.series[target_slot].values.len() {
            return None;
        }
        let as_of = self.series[source_slot].plot.source_rows().is_some();
        {
            let target = &mut self.series[target_slot];
            let mut column =
                match std::mem::replace(&mut target.values, SeriesValues::Single(Vec::new())) {
                    SeriesValues::Single(values) => values,
                    SeriesValues::Ohlc(mut columns) => std::mem::take(&mut columns[3]),
                };
            column.truncate(output_row);
            column.extend_from_slice(values);
            column.truncate(output_len);
            target.values = SeriesValues::Single(column);
            target.time_alias = Some(TimeAlias {
                source,
                offset: alias_offset,
                len: output_len,
            });
            for colors in &mut target.point_colors {
                if !colors.is_empty() {
                    colors.truncate(output_row);
                    colors.resize(output_len, POINT_COLOR_ABSENT);
                }
            }
            target.generation = target.generation.wrapping_add(1);
            if !as_of {
                target.rebuild_lod_range(changed_from..output_len);
            }
        }
        if !as_of {
            self.copy_plot_range(target_slot, source_slot, plot_offset, output_len, 0);
            return Some(changed_from);
        }
        // An as-of source's mapping changes reach its outputs as they happen, so only the rows
        // showing recomputed values (and a moved end) are rewritten here.
        let unchanged = if alias_offset == alias.offset {
            self.series[source_slot]
                .plot
                .rows_before_source(alias_offset + changed_from)
        } else {
            0
        };
        let first_changed =
            self.copy_plot_range(target_slot, source_slot, plot_offset, output_len, unchanged);
        let target = &mut self.series[target_slot];
        let size = target.plot.size();
        target.repair_plot_lod(first_changed..size);
        Some(changed_from)
    }

    /// Replace rows `[source_from, source_from + values.len())` of an aligned scalar output whose
    /// later rows the owner knows are unchanged: a study stops at its union source's last real
    /// row, so the trailing whitespace rows of pre-installed session slots keep their empty
    /// values instead of being rewritten on every tick. The output then covers the source's rows,
    /// growing by whitespace rows or shrinking with it, so the work is proportional to the written
    /// rows plus that growth, not the trailing rows. `None` (nothing changed) for an unknown or
    /// unaliased output, a write before the output's first row, past its end, or past the source,
    /// and for an as-of source, whose outputs rewrite through [`Self::update_single_aligned`].
    pub fn update_single_aligned_within(
        &mut self,
        id: SeriesId,
        source: SeriesId,
        source_from: usize,
        values: &[f64],
    ) -> Option<usize> {
        let target_slot = self.series_slot(id)?;
        let source_slot = self.series_slot(source)?;
        let alias = self.series[target_slot].time_alias?;
        if alias.source != source || self.series[source_slot].plot.source_rows().is_some() {
            return None;
        }
        let alias_offset = if self.series[target_slot].values.len() == 0 {
            source_from
        } else {
            alias.offset
        };
        let source_len = self.visible_len(self.series_times_by_slot(source_slot)?);
        if source_from < alias_offset || source_from + values.len() > source_len {
            return None;
        }
        let output_row = source_from - alias_offset;
        let source_plot_len = self.aliasable_rows(source_slot);
        let plot_offset = alias_offset.min(source_plot_len);
        let output_len = source_len
            .saturating_sub(alias_offset)
            .min(source_plot_len - plot_offset);
        let old_len = self.series[target_slot].values.len();
        if output_row > old_len {
            return None;
        }
        let written = (output_row + values.len()).min(output_len);
        let target = &mut self.series[target_slot];
        let mut column =
            match std::mem::replace(&mut target.values, SeriesValues::Single(Vec::new())) {
                SeriesValues::Single(values) => values,
                SeriesValues::Ohlc(mut columns) => std::mem::take(&mut columns[3]),
            };
        let overlap = column.len().min(written).saturating_sub(output_row);
        column[output_row..output_row + overlap].copy_from_slice(&values[..overlap]);
        column.extend_from_slice(&values[overlap..written - output_row]);
        column.resize(output_len, f64::NAN);
        target.values = SeriesValues::Single(column);
        target.time_alias = Some(TimeAlias {
            source,
            offset: alias_offset,
            len: output_len,
        });
        for colors in &mut target.point_colors {
            if !colors.is_empty() {
                colors.resize(output_len, POINT_COLOR_ABSENT);
                colors[output_row.min(output_len)..written].fill(POINT_COLOR_ABSENT);
            }
        }
        target.generation = target.generation.wrapping_add(1);
        let changed_from = output_row.min(output_len);
        if output_len < old_len {
            target.rebuild_lod();
        } else {
            // The written rows, plus any whitespace rows the output grew by.
            let end = if output_len > old_len {
                output_len
            } else {
                written
            };
            target.rebuild_lod_range(changed_from..end);
        }
        self.copy_plot_range(target_slot, source_slot, plot_offset, output_len, 0);
        Some(changed_from)
    }

    /// Canonical rows of `slot` its plot can alias: the plotted rows of a union-timed series, or
    /// every visible canonical row of an as-of mapped one (whose plot rows are merged points).
    fn aliasable_rows(&self, slot: usize) -> usize {
        let series = &self.series[slot];
        if series.plot.source_rows().is_none() {
            return series.plot.size();
        }
        self.series_times_by_slot(slot)
            .map_or(0, |times| self.visible_len(times))
            .min(series.values.len())
    }

    /// Returns the first target plot row that may differ (see [`PlotList::copy_range_from`]).
    fn copy_plot_range(
        &mut self,
        target_slot: usize,
        source_slot: usize,
        offset: usize,
        len: usize,
        unchanged: usize,
    ) -> usize {
        debug_assert_ne!(target_slot, source_slot);
        if target_slot < source_slot {
            let (left, right) = self.series.split_at_mut(source_slot);
            left[target_slot]
                .plot
                .copy_range_from(&right[0].plot, offset, len, unchanged)
        } else {
            let (left, right) = self.series.split_at_mut(target_slot);
            right[0]
                .plot
                .copy_range_from(&left[source_slot].plot, offset, len, unchanged)
        }
    }

    /// Install the series' per-row color channels (reference data-item colors). Each channel is
    /// `None`/empty for absent, or must match the series' row count exactly; a length mismatch
    /// rejects the whole call (false, no partial state). Within a channel, a `0` entry means
    /// "no override at this row" ([`POINT_COLOR_ABSENT`]).
    pub fn set_point_colors(
        &mut self,
        id: SeriesId,
        channels: [Option<Vec<u32>>; POINT_COLOR_CHANNELS],
    ) -> bool {
        let Some(slot) = self.series_slot(id) else {
            return false;
        };
        let rows = self.series[slot].values.len();
        if channels
            .iter()
            .flatten()
            .any(|channel| !channel.is_empty() && channel.len() != rows)
        {
            return false;
        }
        let s = &mut self.series[slot];
        for (slot, channel) in s.point_colors.iter_mut().zip(channels) {
            *slot = channel.unwrap_or_default();
        }
        true
    }

    /// Set one installed row's color override, creating the aligned channel lazily.
    pub fn set_point_color(
        &mut self,
        id: SeriesId,
        channel: PointColorChannel,
        row: usize,
        color: u32,
    ) -> bool {
        let Some(slot) = self.series_slot(id) else {
            return false;
        };
        let rows = self.series[slot].values.len();
        if row >= rows {
            return false;
        }
        let colors = &mut self.series[slot].point_colors[channel as usize];
        if colors.is_empty() {
            colors.resize(rows, POINT_COLOR_ABSENT);
        }
        colors[row] = color;
        true
    }

    /// The per-point color override at canonical `row` for `channel`, or `None` when the channel
    /// is absent or the row carries [`POINT_COLOR_ABSENT`]. Plot rows mirror canonical rows
    /// except for as-of aligned series; plot-row readers use [`Self::point_colors`].
    pub fn point_color(&self, id: SeriesId, channel: PointColorChannel, row: usize) -> Option<u32> {
        let s = self.series.get(self.series_slot(id)?)?;
        PointColors {
            channels: [&s.point_colors[0], &s.point_colors[1], &s.point_colors[2]],
            rows: None,
        }
        .color(channel, row)
    }

    /// Resolve every point-color channel once for a row-processing hot path, indexed by plot
    /// row (an as-of aligned series reads the canonical row each plot row shows).
    pub fn point_colors(&self, id: SeriesId) -> Option<PointColors<'_>> {
        let s = self.series.get(self.series_slot(id)?)?;
        Some(PointColors {
            channels: [&s.point_colors[0], &s.point_colors[1], &s.point_colors[2]],
            rows: s.plot.source_rows(),
        })
    }

    /// Whether the series has any per-point color channel installed.
    pub fn has_point_colors(&self, id: SeriesId) -> bool {
        self.series_slot(id)
            .and_then(|slot| self.series.get(slot))
            .is_some_and(|s| s.point_colors.iter().any(|c| !c.is_empty()))
    }

    /// Streaming update of a series' last point: replaces the last bar or appends a new one.
    /// The fast path (append at a new global max time, or replace an existing point) avoids a
    /// full rebuild.
    pub fn update(&mut self, id: SeriesId, time: i64, values: [f64; 4]) -> bool {
        let applied = self.update_styled_impl(id, time, values, [None; POINT_COLOR_CHANNELS], true);
        self.sync_as_of_tail();
        applied
    }

    /// [`update`] plus the target bar's per-point colors (reference `series.update` with data-item
    /// colors; `None` = no custom color for that channel). The color channels stay aligned
    /// with the rows in every path: appended rows push, a replaced bar takes the new channels (a
    /// plain `update` clears that bar's overrides, matching the reference's whole-bar
    /// replacement), and a mid-history insert splices. The first explicit color on a series
    /// without that channel creates it, so the bar is colored exactly as a full install would.
    pub fn update_styled(
        &mut self,
        id: SeriesId,
        time: i64,
        values: [f64; 4],
        colors: [Option<u32>; POINT_COLOR_CHANNELS],
    ) -> bool {
        let applied = self.update_styled_impl(id, time, values, colors, true);
        self.sync_as_of_tail();
        applied
    }

    fn update_styled_impl(
        &mut self,
        id: SeriesId,
        time: i64,
        values: [f64; 4],
        colors: [Option<u32>; POINT_COLOR_CHANNELS],
        update_lod: bool,
    ) -> bool {
        let Some(slot) = self.materialize_time_alias(id) else {
            return false;
        };
        if self.time_cutoff.is_some_and(|cutoff| time > cutoff) {
            let s = &mut self.series[slot];
            let row = lower_bound(&s.times, |&candidate| candidate < time);
            if s.times.get(row) == Some(&time) {
                s.values.set_row(row, values);
                for (channel, color) in s.point_colors.iter_mut().zip(colors) {
                    if !channel.is_empty() {
                        channel[row] = color.unwrap_or(POINT_COLOR_ABSENT);
                    }
                }
                if update_lod {
                    s.rebuild_lod_range(row..row + 1);
                }
            } else {
                s.times.insert(row, time);
                s.values.insert(row, values);
                for (channel, color) in s.point_colors.iter_mut().zip(colors) {
                    if !channel.is_empty() {
                        channel.insert(row, color.unwrap_or(POINT_COLOR_ABSENT));
                    }
                }
                if update_lod {
                    s.rebuild_lod();
                }
            }
            s.generation = s.generation.wrapping_add(1);
            return true;
        }
        if self.series[slot].alignment.is_as_of() {
            self.update_as_of_row(slot, time, values, colors);
            return true;
        }
        let last_merged = self.merged_times.last().copied();

        // Case 1: brand-new global max time — appended at the end, no indices shift. Rows retained
        // past a replay clock still follow it, so such a time is a history insert instead.
        let series_last = self.series[slot].times.last().copied();
        if last_merged.is_none_or(|last| time > last) && series_last.is_none_or(|last| time > last)
        {
            let new_index = self.merged_times.len() as TimePointIndex;
            self.merged_times.push(time);
            self.time_points_generation = self.time_points_generation.wrapping_add(1);
            let s = &mut self.series[slot];
            let affected = s.values.len();
            push_raw(s, time, values, colors);
            if update_lod {
                s.rebuild_lod_range(affected..affected + 1);
            }
            s.plot.upsert_last(new_index);
            s.generation = s.generation.wrapping_add(1);
            return true;
        }

        // Case 2: an existing merged time, at or after this series' own last point — a
        // replace-last or append-at-series-end that maps to a non-decreasing plot index.
        let existing = self.merged_times.binary_search(&time).ok();
        if let (Some(pos), true) = (existing, series_last.is_none_or(|lt| time >= lt)) {
            let s = &mut self.series[slot];
            let affected = if series_last == Some(time) {
                let row = s.times.len() - 1;
                s.values.set_row(row, values);
                set_row_colors(s, row, colors);
                row
            } else {
                let row = s.values.len();
                push_raw(s, time, values, colors);
                row
            };
            if update_lod {
                s.rebuild_lod_range(affected..affected + 1);
            }
            s.plot.upsert_last(pos as TimePointIndex);
            s.generation = s.generation.wrapping_add(1);
            return true;
        }

        // Case 3a: an in-place correction of an existing historical row (a late N-1/N-2 fix).
        // No timestamp changes, so the union and every plot mapping stay valid; repair only the
        // row's LOD path and its autoscale chunk.
        let s = &mut self.series[slot];
        let insert = lower_bound(&s.times, |&t| t < time);
        if s.times.get(insert) == Some(&time) {
            s.values.set_row(insert, values);
            set_row_colors(s, insert, colors);
            if update_lod {
                s.rebuild_lod_range(insert..insert + 1);
            }
            s.plot.invalidate_row(insert);
            s.generation = s.generation.wrapping_add(1);
            return true;
        }

        // Case 3b: a new row inside this series' history. Its rows shift, so it is reindexed; the
        // union is merged only when the time is genuinely new to the chart.
        let rows = s.values.len();
        for (channel, color) in s.point_colors.iter_mut().zip(colors) {
            if let Some(channel) = color_channel(channel, rows, color) {
                channel.insert(insert, color.unwrap_or(POINT_COLOR_ABSENT));
            }
        }
        s.times.insert(insert, time);
        s.values.insert(insert, values);
        if update_lod {
            s.rebuild_lod();
        }
        s.generation = s.generation.wrapping_add(1);
        if existing.is_some() {
            self.reindex_series(slot);
        } else {
            self.insert_merged_time(time);
            self.reindex_all();
        }
        true
    }

    /// Streaming write to an as-of series (visible time). It never touches the union: a value
    /// change repairs only the plot rows showing that row, a new last row rewrites only the
    /// points at or after its time, and a new historical row re-derives this series' mapping.
    fn update_as_of_row(
        &mut self,
        slot: usize,
        time: i64,
        values: [f64; 4],
        colors: [Option<u32>; POINT_COLOR_CHANNELS],
    ) {
        let s = &mut self.series[slot];
        let row = lower_bound(&s.times, |&candidate| candidate < time);
        s.generation = s.generation.wrapping_add(1);
        if s.times.get(row) == Some(&time) {
            s.values.set_row(row, values);
            set_row_colors(s, row, colors);
            let shown = s.plot.invalidate_row(row);
            if !shown.is_empty() {
                self.as_of_rows_rewritten =
                    self.as_of_rows_rewritten.wrapping_add(shown.len() as u64);
                s.repair_plot_lod(shown);
            }
            return;
        }
        if row == s.times.len() {
            push_raw(s, time, values, colors);
            let from =
                self.merged_times[..self.as_of_through].partition_point(|&point| point < time);
            if from < self.as_of_through {
                self.resync_as_of_tail(slot, from);
            }
            return;
        }
        let rows = s.values.len();
        for (channel, color) in s.point_colors.iter_mut().zip(colors) {
            if let Some(channel) = color_channel(channel, rows, color) {
                channel.insert(row, color.unwrap_or(POINT_COLOR_ABSENT));
            }
        }
        s.times.insert(row, time);
        s.values.insert(row, values);
        self.index_rebuilds = self.index_rebuilds.wrapping_add(1);
        self.rebuild_as_of_rows(slot);
        self.realias_outputs(slot, 0);
    }

    /// Insert one timestamp that no series held before into the union. Equivalent to
    /// [`Self::rebuild_merged`] for a single new time without re-sorting every series' times.
    fn insert_merged_time(&mut self, time: i64) {
        let Err(position) = self.merged_times.binary_search(&time) else {
            return;
        };
        self.index_rebuilds = self.index_rebuilds.wrapping_add(1);
        if self.capture_merged_time_rebase && self.merged_time_rebase_source.is_none() {
            self.merged_time_rebase_source = Some(self.merged_times.clone());
        }
        if position < self.merged_times.len() {
            self.time_index_generation = self.time_index_generation.wrapping_add(1);
        }
        self.merged_times.insert(position, time);
        self.time_points_generation = self.time_points_generation.wrapping_add(1);
    }

    /// Reindex one owned series (and outputs aliasing its time range) against an unchanged union.
    fn reindex_series(&mut self, slot: usize) {
        self.index_rebuilds = self.index_rebuilds.wrapping_add(1);
        // Rows past a replay clock hold no point yet.
        let len = self.visible_len(&self.series[slot].times);
        let s = &mut self.series[slot];
        s.plot.rebuild_from(&self.merged_times, &s.times[..len]);
        let aliases = self
            .live_slots
            .values()
            .filter_map(|&alias| {
                self.resolved_alias_range(alias)
                    .filter(|&(source, _, _)| source == slot)
                    .map(|range| (alias, range))
            })
            .collect::<Vec<_>>();
        for (alias, (source, offset, len)) in aliases {
            self.copy_plot_range(alias, source, offset, len, 0);
        }
    }

    /// Apply an ascending batch whose rows before this series' last bar all correct bars that
    /// already exist: those rows change in place (no timestamp or plot mapping moves) and the rest
    /// append or replace the tail through the streaming path. `None` when a row would insert a new
    /// historical time, which needs the merging path. Batch rows replace whole bars, so their
    /// colors clear.
    fn update_rows_without_history_insert(
        &mut self,
        id: SeriesId,
        slot: usize,
        times: &[i64],
        values: [&[f64]; 4],
    ) -> Option<()> {
        let series = &self.series[slot];
        let historical = series
            .times
            .last()
            .map_or(0, |&last| times.partition_point(|&time| time < last));
        let rows = times[..historical]
            .iter()
            .map(|time| series.times.binary_search(time).ok())
            .collect::<Option<Vec<_>>>()?;
        let s = &mut self.series[slot];
        for (index, &row) in rows.iter().enumerate() {
            s.values.set_row(
                row,
                [
                    values[0][index],
                    values[1][index],
                    values[2][index],
                    values[3][index],
                ],
            );
            for channel in &mut s.point_colors {
                if !channel.is_empty() {
                    channel[row] = POINT_COLOR_ABSENT;
                }
            }
            s.plot.invalidate_row(row);
        }
        if let (Some(&first), Some(&last)) = (rows.first(), rows.last()) {
            if s.alignment.is_as_of() {
                // The corrected rows repair the plot rows showing them (none if collapsed).
                let shown = s.plot.rows_before_source(first)..s.plot.rows_before_source(last + 1);
                if !shown.is_empty() {
                    s.repair_plot_lod(shown);
                }
            } else {
                s.rebuild_lod_range(first..last + 1);
            }
        }
        s.generation = s.generation.wrapping_add(historical as u64);

        let tail_from = times
            .get(historical)
            .map_or(0, |&first| lower_bound(&s.times, |&time| time < first));
        for row in historical..times.len() {
            self.update_styled_impl(
                id,
                times[row],
                [
                    values[0][row],
                    values[1][row],
                    values[2][row],
                    values[3][row],
                ],
                [None; POINT_COLOR_CHANNELS],
                false,
            );
        }
        if historical < times.len() {
            let series = &mut self.series[slot];
            series.rebuild_lod_range(tail_from..series.values.len());
        }
        Some(())
    }

    /// Apply an ascending, unique batch as one logical data-layer mutation. Tail rows keep the
    /// O(1)-per-row append/replace path and rows correcting existing history change in place;
    /// only a batch inserting new historical times is merged in O(n + k), rebuilding the shared
    /// index once instead of once per row. Returns the first affected row in the resulting
    /// source series.
    pub fn update_many(
        &mut self,
        id: SeriesId,
        times: &[i64],
        values: [&[f64]; 4],
    ) -> Option<usize> {
        let affected = self.update_many_impl(id, times, values);
        self.sync_as_of_tail();
        affected
    }

    fn update_many_impl(
        &mut self,
        id: SeriesId,
        times: &[i64],
        values: [&[f64]; 4],
    ) -> Option<usize> {
        if self
            .series_slot(id)
            .is_some_and(|slot| matches!(self.series[slot].values, SeriesValues::Single(_)))
            && values[0] == values[1]
            && values[0] == values[2]
            && values[0] == values[3]
        {
            return self.update_many_single_impl(id, times, values[0]);
        }
        let slot = self.materialize_time_alias(id)?;
        if times.is_empty() {
            return Some(self.series[slot].times.len());
        }
        debug_assert!(times.windows(2).all(|window| window[0] < window[1]));
        debug_assert!(values.iter().all(|column| column.len() == times.len()));

        let affected = lower_bound(&self.series[slot].times, |&time| time < times[0]);
        if self
            .update_rows_without_history_insert(id, slot, times, values)
            .is_some()
        {
            return Some(affected);
        }
        let new_time_points = self.new_time_points(slot, times);
        let old = &self.series[slot];
        let old_values = old.values.columns();
        let capacity = old.times.len() + times.len();
        let mut merged_times = Vec::with_capacity(capacity);
        let mut merged_values: [Vec<f64>; 4] =
            std::array::from_fn(|_| Vec::with_capacity(capacity));
        let mut merged_colors: [Vec<u32>; POINT_COLOR_CHANNELS] = std::array::from_fn(|channel| {
            if old.point_colors[channel].is_empty() {
                Vec::new()
            } else {
                Vec::with_capacity(capacity)
            }
        });
        let mut old_row = 0usize;
        let mut new_row = 0usize;
        while old_row < old.times.len() || new_row < times.len() {
            let take_new = old_row == old.times.len()
                || (new_row < times.len() && times[new_row] <= old.times[old_row]);
            if take_new {
                let replaces = old_row < old.times.len() && times[new_row] == old.times[old_row];
                merged_times.push(times[new_row]);
                for (column, source) in merged_values.iter_mut().zip(values) {
                    column.push(source[new_row]);
                }
                for colors in &mut merged_colors {
                    if !colors.is_empty() || colors.capacity() > 0 {
                        colors.push(POINT_COLOR_ABSENT);
                    }
                }
                new_row += 1;
                old_row += usize::from(replaces);
            } else {
                merged_times.push(old.times[old_row]);
                for (column, source) in merged_values.iter_mut().zip(old_values) {
                    column.push(source[old_row]);
                }
                for (column, source) in merged_colors.iter_mut().zip(&old.point_colors) {
                    if !source.is_empty() {
                        column.push(source[old_row]);
                    }
                }
                old_row += 1;
            }
        }

        let series = &mut self.series[slot];
        series.times = merged_times;
        series.values = SeriesValues::Ohlc(merged_values);
        // Batches that only correct existing history (plus any tail) returned through the in-place
        // path above, so every merge here inserts rows and shifts the hierarchy.
        series.rebuild_lod();
        series.point_colors = merged_colors;
        series.generation = series.generation.wrapping_add(times.len() as u64);
        self.rebuild_merged();
        self.time_points_generation = self
            .time_points_generation
            .wrapping_add(new_time_points.saturating_sub(1) as u64);
        self.reindex_all();
        Some(affected)
    }

    /// Timestamps of a merging batch that are new to the union (none for an as-of series, which
    /// never adds points).
    fn new_time_points(&self, slot: usize, times: &[i64]) -> usize {
        if self.series[slot].alignment.is_as_of() {
            return 0;
        }
        times
            .iter()
            .filter(|time| self.merged_times.binary_search(time).is_err())
            .count()
    }

    /// Scalar-series variant of [`Self::update_many`]. It preserves one canonical value column
    /// through historical merges instead of constructing four identical temporary columns.
    pub fn update_many_single(
        &mut self,
        id: SeriesId,
        times: &[i64],
        values: &[f64],
    ) -> Option<usize> {
        let affected = self.update_many_single_impl(id, times, values);
        self.sync_as_of_tail();
        affected
    }

    fn update_many_single_impl(
        &mut self,
        id: SeriesId,
        times: &[i64],
        values: &[f64],
    ) -> Option<usize> {
        let slot = self.materialize_time_alias(id)?;
        if times.is_empty() {
            return Some(self.series[slot].times.len());
        }
        debug_assert_eq!(times.len(), values.len());
        debug_assert!(times.windows(2).all(|window| window[0] < window[1]));

        let affected = lower_bound(&self.series[slot].times, |&time| time < times[0]);
        if self
            .update_rows_without_history_insert(id, slot, times, [values, values, values, values])
            .is_some()
        {
            return Some(affected);
        }
        let new_time_points = self.new_time_points(slot, times);
        let old = &self.series[slot];
        let old_values = old.values.columns()[3];
        let capacity = old.times.len() + times.len();
        let mut merged_times = Vec::with_capacity(capacity);
        let mut merged_values = Vec::with_capacity(capacity);
        let mut merged_colors: [Vec<u32>; POINT_COLOR_CHANNELS] = std::array::from_fn(|channel| {
            if old.point_colors[channel].is_empty() {
                Vec::new()
            } else {
                Vec::with_capacity(capacity)
            }
        });
        let mut old_row = 0usize;
        let mut new_row = 0usize;
        while old_row < old.times.len() || new_row < times.len() {
            let take_new = old_row == old.times.len()
                || (new_row < times.len() && times[new_row] <= old.times[old_row]);
            if take_new {
                let replaces = old_row < old.times.len() && times[new_row] == old.times[old_row];
                merged_times.push(times[new_row]);
                merged_values.push(values[new_row]);
                for colors in &mut merged_colors {
                    if !colors.is_empty() || colors.capacity() > 0 {
                        colors.push(POINT_COLOR_ABSENT);
                    }
                }
                new_row += 1;
                old_row += usize::from(replaces);
            } else {
                merged_times.push(old.times[old_row]);
                merged_values.push(old_values[old_row]);
                for (column, source) in merged_colors.iter_mut().zip(&old.point_colors) {
                    if !source.is_empty() {
                        column.push(source[old_row]);
                    }
                }
                old_row += 1;
            }
        }

        let series = &mut self.series[slot];
        series.times = merged_times;
        series.values = SeriesValues::Single(merged_values);
        // Batches that only correct existing history (plus any tail) returned through the in-place
        // path above, so every merge here inserts rows and shifts the hierarchy.
        series.rebuild_lod();
        series.point_colors = merged_colors;
        series.generation = series.generation.wrapping_add(times.len() as u64);
        self.rebuild_merged();
        self.time_points_generation = self
            .time_points_generation
            .wrapping_add(new_time_points.saturating_sub(1) as u64);
        self.reindex_all();
        Some(affected)
    }

    /// Remove the last `count` rows of a series (reference `popSeriesData`, data-layer.ts:338-383):
    /// `count` 0 is a no-op, larger counts clamp to the row count. Per-point color channels
    /// truncate in lockstep with their rows, and the merged time points are rebuilt so times
    /// no series occupies anymore leave the shared axis. Returns the new row count.
    pub fn pop(&mut self, id: SeriesId, count: usize) -> Option<usize> {
        let slot = self.materialize_time_alias(id)?;
        let keep = self.series[slot].times.len().saturating_sub(count);
        let s = &mut self.series[slot];
        if keep == s.times.len() {
            return Some(keep);
        }
        s.times.truncate(keep);
        s.values.truncate(keep);
        s.rebuild_lod();
        for channel in &mut s.point_colors {
            if !channel.is_empty() {
                channel.truncate(keep);
            }
        }
        s.generation = s.generation.wrapping_add(1);
        self.rebuild_merged();
        self.reindex_all();
        Some(keep)
    }

    /// Drop the **oldest** rows of a series until at most `keep` remain (the eviction half of the
    /// `max_points` retention policy; [`pop`] is its newest-first counterpart). Per-point color
    /// channels shift in lockstep with their rows, and the merged time points are rebuilt so times
    /// no series occupies anymore leave the shared axis. Returns the new row count.
    ///
    /// Cost is `O(total rows)` — the row shift plus a merged rebuild and reindex — so callers must
    /// not run this once per appended point. The engine trims with hysteresis for exactly that
    /// reason (see `ChartEngine::enforce_series_cap`). A retention that trims several series at
    /// once calls [`trim_fronts`](Self::trim_fronts) instead, which rebuilds the shared axis once.
    pub fn trim_front(&mut self, id: SeriesId, keep: usize) -> Option<usize> {
        let (rows, dropped) = self.trim_series_rows_front(id, keep)?;
        if dropped {
            self.rebuild_merged();
            self.reindex_all();
        }
        Some(rows)
    }

    /// Apply [`trim_front`](Self::trim_front) to several series as one data-layer transaction: each
    /// `(id, keep)` entry drops its series' oldest rows in order (an unknown id is skipped, a
    /// repeated id applies again to the already trimmed rows), then the merged time points and
    /// every plot index are rebuilt **once** if any row left. The result equals the sequence of
    /// individual trims, without an `O(total rows)` union merge and reindex per series.
    pub fn trim_fronts(&mut self, trims: &[(SeriesId, usize)]) {
        let mut dropped = false;
        for &(id, keep) in trims {
            dropped |= self
                .trim_series_rows_front(id, keep)
                .is_some_and(|(_, dropped)| dropped);
        }
        if dropped {
            self.rebuild_merged();
            self.reindex_all();
        }
    }

    /// The per-series half of a front trim: drop the oldest rows, their colors and summaries.
    /// Leaves the merged axis and plot indices stale, which the caller must rebuild when this
    /// reports rows dropped. `None` for an unknown id; otherwise the remaining row count and whether
    /// any row left. An aliased series owns its times first, even when nothing is dropped.
    fn trim_series_rows_front(&mut self, id: SeriesId, keep: usize) -> Option<(usize, bool)> {
        let slot = self.materialize_time_alias(id)?;
        let len = self.series[slot].times.len();
        if len <= keep {
            return Some((len, false));
        }
        let drop = len - keep;
        let s = &mut self.series[slot];
        s.times.drain(..drop);
        s.values.drain_front(drop);
        s.rebuild_lod();
        for channel in &mut s.point_colors {
            if !channel.is_empty() {
                channel.drain(..drop);
            }
        }
        s.generation = s.generation.wrapping_add(1);
        Some((keep, true))
    }

    fn rebuild_merged(&mut self) {
        self.index_rebuilds = self.index_rebuilds.wrapping_add(1);
        // Each union series' visible times are ascending and duplicate-free, so the union is a
        // merge of sorted runs. The runs sit in a stack array: no allocation, no sort.
        let mut runs: [&[i64]; MAX_MERGED_RUNS] = [&[]; MAX_MERGED_RUNS];
        let mut run_count = 0;
        let mut total = 0;
        for &slot in self.live_slots.values() {
            let series = &self.series[slot];
            if series.joins_union() {
                let times = &series.times;
                let len = self.time_cutoff.map_or(times.len(), |cutoff| {
                    times.partition_point(|&time| time <= cutoff)
                });
                if let Some(run) = runs.get_mut(run_count) {
                    *run = &times[..len];
                }
                run_count += 1;
                total += len;
            }
        }
        let all = &mut self.merged_times_scratch;
        all.clear();
        if run_count <= MAX_MERGED_RUNS {
            let runs = &mut runs[..run_count];
            all.reserve(runs.iter().map(|run| run.len()).max().unwrap_or(0));
            merge_sorted_runs(runs, all);
        } else {
            all.reserve(total);
            for &slot in self.live_slots.values() {
                let series = &self.series[slot];
                if series.joins_union() {
                    let times = &series.times;
                    let len = self.time_cutoff.map_or(times.len(), |cutoff| {
                        times.partition_point(|&time| time <= cutoff)
                    });
                    all.extend_from_slice(&times[..len]);
                }
            }
            all.sort_unstable();
            all.dedup();
        }
        if *all != self.merged_times {
            if self.capture_merged_time_rebase && self.merged_time_rebase_source.is_none() {
                self.merged_time_rebase_source = Some(self.merged_times.clone());
            }
            if !all.starts_with(&self.merged_times) {
                self.time_index_generation = self.time_index_generation.wrapping_add(1);
            }
            std::mem::swap(&mut self.merged_times, all);
            self.time_points_generation = self.time_points_generation.wrapping_add(1);
        }
    }

    fn reindex_all(&mut self) {
        self.index_rebuilds = self.index_rebuilds.wrapping_add(1);
        // Reindex owned timelines first. Aliased outputs copy a range from their ultimate owned
        // source, so they retain no mapping allocation when that source is dense.
        let merged = &self.merged_times;
        for &slot in self.live_slots.values() {
            let s = &mut self.series[slot];
            if s.time_alias.is_none() && !s.alignment.is_as_of() {
                let len = self.time_cutoff.map_or(s.times.len(), |cutoff| {
                    s.times.partition_point(|&time| time <= cutoff)
                });
                s.plot.rebuild_from(merged, &s.times[..len]);
            }
        }
        // As-of series map onto the finished union (their extent reads the union plots).
        self.rebuild_as_of_all();
        let aliases = self
            .live_slots
            .values()
            .filter_map(|&slot| self.resolved_alias_range(slot).map(|range| (slot, range)))
            .collect::<Vec<_>>();
        for (slot, (source_slot, offset, len)) in aliases {
            let was_as_of = self.series[slot].plot.source_rows().is_some();
            self.copy_plot_range(slot, source_slot, offset, len, 0);
            // Summaries of an as-of alias name plot rows, which this copy may have moved.
            if was_as_of || self.series[slot].plot.source_rows().is_some() {
                self.series[slot].rebuild_plot_lod();
                self.note_realigned(slot);
            }
        }
    }

    fn resolved_alias_range(&self, mut slot: usize) -> Option<(usize, usize, usize)> {
        let mut offset = 0usize;
        let len = self.series.get(slot)?.values.len();
        let mut aliased = false;
        for _ in 0..self.series.len().max(1) {
            let Some(alias) = self.series.get(slot)?.time_alias else {
                let source_size = self.aliasable_rows(slot);
                let offset = offset.min(source_size);
                let available = source_size - offset;
                return aliased.then_some((slot, offset, len.min(available)));
            };
            aliased = true;
            offset = offset.checked_add(alias.offset)?;
            slot = self.series_slot(alias.source)?;
        }
        debug_assert!(false, "series time alias cycle");
        None
    }
}

/// The row-aligned color channel that stores `color`. An absent channel is created (every existing
/// row without an override) the first time a streamed row carries an explicit color, so a styled
/// update colors its bar like the same item in a full data install; `None` keeps it absent.
fn color_channel(channel: &mut Vec<u32>, rows: usize, color: Option<u32>) -> Option<&mut Vec<u32>> {
    if channel.is_empty() {
        color?;
        channel.resize(rows, POINT_COLOR_ABSENT);
    }
    Some(channel)
}

/// Replace one existing row's color overrides (a whole-bar update clears absent channels' rows).
fn set_row_colors(s: &mut RawSeries, row: usize, colors: [Option<u32>; POINT_COLOR_CHANNELS]) {
    let rows = s.values.len();
    for (channel, color) in s.point_colors.iter_mut().zip(colors) {
        if let Some(channel) = color_channel(channel, rows, color) {
            channel[row] = color.unwrap_or(POINT_COLOR_ABSENT);
        }
    }
}

fn push_raw(
    s: &mut RawSeries,
    time: i64,
    values: [f64; 4],
    colors: [Option<u32>; POINT_COLOR_CHANNELS],
) {
    let rows = s.values.len();
    for (channel, color) in s.point_colors.iter_mut().zip(colors) {
        if let Some(channel) = color_channel(channel, rows, color) {
            channel.push(color.unwrap_or(POINT_COLOR_ABSENT));
        }
    }
    s.times.push(time);
    s.values.push(values);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::plot_list::{MismatchDirection, PlotValueIndex};

    /// Value of a series at a merged *index* (maps index -> sparse row).
    fn value_at_index(
        dl: &DataLayer,
        id: SeriesId,
        index: TimePointIndex,
        plot: PlotValueIndex,
    ) -> f64 {
        let row = dl
            .plot(id)
            .search(index, MismatchDirection::None)
            .expect("index present");
        dl.plot(id).value_at(row, plot)
    }

    /// Sets a single-value series (all OHLC = the value) at the given times.
    fn set(dl: &mut DataLayer, id: SeriesId, times: &[i64], vals: &[f64]) {
        let col = |f: fn(f64) -> f64| vals.iter().map(|&v| f(v)).collect::<Vec<f64>>();
        dl.set_data(
            id,
            times.to_vec(),
            col(|v| v),
            col(|v| v),
            col(|v| v),
            col(|v| v),
        );
    }

    fn indices(dl: &DataLayer, id: SeriesId) -> Vec<TimePointIndex> {
        dl.plot(id).indices().collect()
    }

    #[test]
    fn replay_cutoff_masks_without_discarding_canonical_rows() {
        let mut dl = DataLayer::new();
        let first = dl.add_series();
        let second = dl.add_series();
        set(&mut dl, first, &[1, 2, 3, 4], &[10.0, 20.0, 30.0, 40.0]);
        set(&mut dl, second, &[2, 4, 6], &[2.0, 4.0, 6.0]);

        assert!(dl.set_time_cutoff(Some(3)));
        assert_eq!(dl.merged_times(), &[1, 2, 3]);
        assert_eq!(dl.series_data(first).unwrap().0, &[1, 2, 3]);
        assert_eq!(dl.series_data(second).unwrap().0, &[2]);
        assert_eq!(indices(&dl, first), vec![0, 1, 2]);
        assert_eq!(indices(&dl, second), vec![1]);

        assert!(dl.update(first, 5, [50.0; 4]));
        assert_eq!(dl.merged_times(), &[1, 2, 3]);
        assert_eq!(dl.series_data(first).unwrap().0, &[1, 2, 3]);

        assert!(dl.set_time_cutoff(Some(5)));
        assert_eq!(dl.merged_times(), &[1, 2, 3, 4, 5]);
        assert_eq!(dl.series_data(first).unwrap().0, &[1, 2, 3, 4, 5]);
        assert_eq!(
            dl.series_data(first).unwrap().1[3],
            &[10.0, 20.0, 30.0, 40.0, 50.0]
        );
        assert!(!dl.set_time_cutoff(Some(5)));
        assert!(dl.set_time_cutoff(None));
        assert_eq!(dl.merged_times(), &[1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn replay_writes_before_rows_retained_past_the_clock_keep_rows_and_points_ordered() {
        let mut dl = DataLayer::new();
        let first = dl.add_series();
        let second = dl.add_series();
        set(&mut dl, first, &[1, 2, 3, 5], &[10.0, 20.0, 30.0, 50.0]);
        set(&mut dl, second, &[2, 5], &[2.0, 5.0]);
        assert!(dl.set_time_cutoff(Some(4)));
        assert_eq!(dl.merged_times(), &[1, 2, 3]);

        // A row at a time the chart already shows, before a retained future row: only visible
        // rows are indexed.
        assert!(dl.update(second, 3, [3.0; 4]));
        assert_eq!(dl.series_data(second).unwrap().0, &[2, 3]);
        assert_eq!(indices(&dl, second), vec![1, 2]);

        // A new visible time past every point but before a retained future row inserts into
        // history instead of appending after that row.
        assert!(dl.update(first, 4, [40.0; 4]));
        assert_eq!(dl.merged_times(), &[1, 2, 3, 4]);
        assert_eq!(
            dl.series[dl.series_slot(first).unwrap()].times,
            &[1, 2, 3, 4, 5]
        );
        assert_eq!(indices(&dl, first), vec![0, 1, 2, 3]);
        assert!(dl.set_time_cutoff(None));
        assert_eq!(dl.merged_times(), &[1, 2, 3, 4, 5]);
        assert_eq!(
            dl.series_data(first).unwrap().1[3],
            &[10.0, 20.0, 30.0, 40.0, 50.0]
        );
        assert_eq!(indices(&dl, second), vec![1, 2, 4]);
    }

    fn assert_lod_matches_fresh(dl: &DataLayer, id: SeriesId) {
        let slot = dl.series_slot(id).unwrap();
        let series = &dl.series[slot];
        let values = series.values.view();
        let mut fresh = LodPyramid::default();
        fresh.rebuild(values);
        let len = series.values.len();
        for range in [0..len, len / 7..len * 6 / 7, len.saturating_sub(97)..len] {
            let actual = series
                .lod
                .view(values)
                .rows_on_range(range.clone(), usize::MAX)
                .0
                .iter()
                .collect::<Vec<_>>();
            let expected = fresh
                .view(values)
                .rows_on_range(range, usize::MAX)
                .0
                .iter()
                .collect::<Vec<_>>();
            assert_eq!(actual, expected);
        }
        assert_eq!(series.lod.logical_bytes(), fresh.logical_bytes());
    }

    #[test]
    fn lod_tracks_batches_corrections_insertions_truncation_and_removal() {
        let mut dl = DataLayer::new();
        let id = dl.add_series();
        let rows = 1_024usize;
        let times = (0..rows).map(|row| row as i64 * 2).collect::<Vec<_>>();
        let close = (0..rows)
            .map(|row| 100.0 + (row as f64 * 0.07).sin())
            .collect::<Vec<_>>();
        let open = close.iter().map(|value| value - 0.1).collect();
        let high = close.iter().map(|value| value + 0.5).collect();
        let low = close.iter().map(|value| value - 0.5).collect();
        assert!(dl.set_data(id, times, open, high, low, close));
        assert_lod_matches_fresh(&dl, id);

        let last_time = (rows as i64 - 1) * 2;
        assert!(dl.update(id, last_time, [101.0, 200.0, -100.0, 102.0]));
        assert!(dl.last_lod_update_nodes(id).unwrap() <= 4);
        assert_lod_matches_fresh(&dl, id);

        assert!(dl.update(id, last_time + 2, [102.0, 103.0, 101.0, 102.5]));
        assert!(dl.last_lod_update_nodes(id).unwrap() <= 4);
        assert_lod_matches_fresh(&dl, id);

        let batch_times = (1..=100)
            .map(|row| last_time + 2 + row * 2)
            .collect::<Vec<_>>();
        let batch_close = (0..100)
            .map(|row| 103.0 + row as f64 * 0.01)
            .collect::<Vec<_>>();
        let batch_open = batch_close.clone();
        let batch_high = batch_close
            .iter()
            .map(|value| value + 0.2)
            .collect::<Vec<_>>();
        let batch_low = batch_close
            .iter()
            .map(|value| value - 0.2)
            .collect::<Vec<_>>();
        assert_eq!(
            dl.update_many(
                id,
                &batch_times,
                [&batch_open, &batch_high, &batch_low, &batch_close],
            ),
            Some(rows + 1)
        );
        assert!(dl.last_lod_update_nodes(id).unwrap() < batch_times.len());
        assert_lod_matches_fresh(&dl, id);

        assert!(dl.update(id, 400, [100.0, 500.0, -500.0, 101.0]));
        assert!(dl.last_lod_update_nodes(id).unwrap() <= 4);
        assert_lod_matches_fresh(&dl, id);

        assert!(dl.update(id, 401, [100.0, 101.0, 99.0, 100.5]));
        assert_lod_matches_fresh(&dl, id);

        assert_eq!(dl.pop(id, 37), Some(rows + 102 - 37));
        assert_lod_matches_fresh(&dl, id);
        assert_eq!(dl.trim_front(id, 500), Some(500));
        assert_lod_matches_fresh(&dl, id);

        let replacement_times = (0..300).map(|row| row as i64 * 3).collect::<Vec<_>>();
        let replacement = (0..300).map(|row| row as f64).collect::<Vec<_>>();
        assert!(dl.set_single_data(id, replacement_times, replacement));
        assert_lod_matches_fresh(&dl, id);
        assert!(dl.memory_usage().lod_bytes > 0);
        assert!(dl.remove_series(id));
        assert_eq!(dl.memory_usage().lod_bytes, 0);
    }

    #[test]
    fn memory_attribution_has_one_ohlc_owner_and_dense_indices() {
        let mut dl = DataLayer::new();
        let source = dl.add_series();
        dl.set_data(
            source,
            vec![1, 2, 3, 4],
            vec![10.0; 4],
            vec![11.0; 4],
            vec![9.0; 4],
            vec![10.5; 4],
        );

        let source_usage = dl.series_memory_usage(source).unwrap();
        assert_eq!(
            source_usage.owned_time_bytes,
            4 * std::mem::size_of::<i64>()
        );
        assert_eq!(
            source_usage.canonical_value_bytes,
            4 * 4 * std::mem::size_of::<f64>()
        );
        assert_eq!(source_usage.plot_index_bytes, 0);
        assert!(source_usage.dense_index_view);
        assert_eq!(
            dl.memory_usage().merged_time_bytes,
            4 * std::mem::size_of::<i64>()
        );
    }

    #[test]
    fn aligned_scalar_output_owns_only_values() {
        let mut dl = DataLayer::new();
        let source = dl.add_series();
        let output = dl.add_series();
        set(&mut dl, source, &[1, 2, 3, 4], &[10.0, 20.0, 30.0, 40.0]);
        assert!(dl.set_single_data_aligned(output, source, 1, vec![2.0, 3.0, 4.0]));

        let usage = dl.series_memory_usage(output).unwrap();
        assert_eq!(usage.rows, 3);
        assert_eq!(usage.owned_time_bytes, 0);
        assert_eq!(usage.canonical_value_bytes, 3 * std::mem::size_of::<f64>());
        assert_eq!(usage.plot_index_bytes, 0);
        assert!(usage.aligned_time_view);
        assert!(usage.dense_index_view);
        assert_eq!(dl.series_data(output).unwrap().0, &[2, 3, 4]);
    }

    /// An indicator before warm-up holds an empty alias at the source suffix. A batch that
    /// rewrites an earlier source row and completes the warm-up in one step starts the output at
    /// its first value; it is not a gap after the empty alias.
    #[test]
    fn empty_aligned_output_starts_at_the_first_value_of_its_update() {
        let mut dl = DataLayer::new();
        let source = dl.add_series();
        let output = dl.add_series();
        set(&mut dl, source, &[1, 2], &[10.0, 20.0]);
        assert!(dl.set_single_data_aligned(output, source, 2, Vec::new()));
        set(
            &mut dl,
            source,
            &[1, 2, 3, 4, 5],
            &[10.0, 21.0, 30.0, 40.0, 50.0],
        );
        assert_eq!(
            dl.update_single_aligned(output, source, 3, &[25.0, 35.0]),
            Some(0)
        );
        let (times, values) = dl.series_data(output).unwrap();
        assert_eq!(times, &[4, 5]);
        assert_eq!(values[3], &[25.0, 35.0]);
        // Still before warm-up, an empty update keeps the empty alias at the new suffix.
        let empty = dl.add_series();
        assert!(dl.set_single_data_aligned(empty, source, 5, Vec::new()));
        assert_eq!(dl.update_single_aligned(empty, source, 5, &[]), Some(0));
        assert!(dl.series_data(empty).unwrap().0.is_empty());
    }

    #[test]
    fn independently_timed_series_retain_sparse_mapping() {
        let mut dl = DataLayer::new();
        let odd = dl.add_series();
        let even = dl.add_series();
        set(&mut dl, odd, &[1, 3, 5], &[1.0, 3.0, 5.0]);
        set(&mut dl, even, &[2, 4], &[2.0, 4.0]);

        let usage = dl.series_memory_usage(odd).unwrap();
        assert_eq!(
            usage.plot_index_bytes,
            3 * std::mem::size_of::<TimePointIndex>()
        );
        assert!(!usage.dense_index_view);
        assert_eq!(indices(&dl, odd), [0, 2, 4]);
        assert_eq!(indices(&dl, even), [1, 3]);
    }

    #[test]
    fn large_small_large_replacement_reuses_merge_high_water() {
        let mut dl = DataLayer::new();
        let source = dl.add_series();
        let install = |dl: &mut DataLayer, rows: usize| {
            let times = (0..rows).map(|row| row as i64).collect::<Vec<_>>();
            let values = (0..rows).map(|row| row as f64).collect::<Vec<_>>();
            dl.set_data(
                source,
                times,
                values.clone(),
                values.clone(),
                values.clone(),
                values,
            );
        };

        install(&mut dl, 10_000);
        let first = dl.memory_usage().allocated_capacity_bytes;
        install(&mut dl, 2);
        assert_eq!(
            dl.memory_usage().canonical_value_bytes,
            2 * 4 * std::mem::size_of::<f64>()
        );
        install(&mut dl, 10_000);
        let second = dl.memory_usage().allocated_capacity_bytes;

        assert!(second <= first + 64, "{first} -> {second}");
    }

    #[test]
    fn merged_union_and_per_series_indices() {
        let mut dl = DataLayer::new();
        let a = dl.add_series();
        let b = dl.add_series();
        // A at times 1,2,3 ; B at 2,4 -> merged 1,2,3,4
        set(&mut dl, a, &[1, 2, 3], &[10.0, 20.0, 30.0]);
        set(&mut dl, b, &[2, 4], &[5.0, 7.0]);

        assert_eq!(dl.merged_times(), &[1, 2, 3, 4]);
        assert_eq!(dl.base_index(), Some(3));

        // A occupies merged indices 0,1,2 ; B occupies 1,3 (whitespace at 0,2)
        assert_eq!(indices(&dl, a), [0, 1, 2]);
        assert_eq!(indices(&dl, b), [1, 3]);
        assert!(dl.plot(b).contains(1));
        assert!(!dl.plot(b).contains(0));
        assert!(!dl.plot(b).contains(2));
    }

    #[test]
    fn adding_a_series_reindexes_existing() {
        let mut dl = DataLayer::new();
        let a = dl.add_series();
        set(&mut dl, a, &[10, 20, 30], &[1.0, 2.0, 3.0]);
        assert_eq!(indices(&dl, a), [0, 1, 2]);

        // new series introduces earlier + interleaved times -> A's indices shift
        let b = dl.add_series();
        set(&mut dl, b, &[5, 15, 25], &[9.0, 9.0, 9.0]);
        assert_eq!(dl.merged_times(), &[5, 10, 15, 20, 25, 30]);
        assert_eq!(indices(&dl, a), [1, 3, 5]);
        assert_eq!(indices(&dl, b), [0, 2, 4]);
    }

    #[test]
    fn update_appends_new_max_without_rebuild() {
        let mut dl = DataLayer::new();
        let a = dl.add_series();
        set(&mut dl, a, &[1, 2, 3], &[10.0, 20.0, 30.0]);
        dl.update(a, 4, [40.0, 41.0, 39.0, 40.0]);
        assert_eq!(dl.merged_times(), &[1, 2, 3, 4]);
        assert_eq!(indices(&dl, a), [0, 1, 2, 3]);
        assert_eq!(value_at_index(&dl, a, 3, PlotValueIndex::Close), 40.0);
    }

    #[test]
    fn update_replaces_last_point() {
        let mut dl = DataLayer::new();
        let a = dl.add_series();
        set(&mut dl, a, &[1, 2, 3], &[10.0, 20.0, 30.0]);
        dl.update(a, 3, [33.0, 35.0, 31.0, 34.0]);
        assert_eq!(dl.merged_times(), &[1, 2, 3]); // no new point
        assert_eq!(value_at_index(&dl, a, 2, PlotValueIndex::High), 35.0);
    }

    #[test]
    fn time_generation_tracks_sequence_changes_not_value_updates() {
        let mut dl = DataLayer::new();
        let id = dl.add_series();
        set(&mut dl, id, &[0, 60, 120, 86_400], &[1.0; 4]);
        let initial = dl.time_points_generation();

        set(&mut dl, id, &[0, 3_600, 7_200, 86_400], &[2.0; 4]);
        assert_ne!(dl.time_points_generation(), initial);
        let interior = dl.time_points_generation();

        set(&mut dl, id, &[0, 3_600, 7_200, 86_400], &[3.0; 4]);
        assert_eq!(dl.time_points_generation(), interior);

        set(&mut dl, id, &[-60, 3_600, 7_200, 86_400], &[3.0; 4]);
        let first = dl.time_points_generation();
        assert_ne!(first, interior);

        set(&mut dl, id, &[-60, 3_600, 7_200, 172_800], &[3.0; 4]);
        let last = dl.time_points_generation();
        assert_ne!(last, first);

        dl.update(id, 259_200, [4.0; 4]);
        let appended = dl.time_points_generation();
        assert_ne!(appended, last);

        dl.update(id, 259_200, [5.0; 4]);
        assert_eq!(dl.time_points_generation(), appended);
    }

    #[test]
    fn time_index_generation_holds_while_the_union_only_grows_at_its_end() {
        let mut dl = DataLayer::new();
        let a = dl.add_series();
        let b = dl.add_series();
        set(&mut dl, a, &[10, 20, 30], &[1.0; 3]);
        let base = dl.time_index_generation();
        // Appends past the last point, by an update or by another series' install, keep indices.
        dl.update(a, 40, [2.0; 4]);
        set(&mut dl, b, &[30, 40, 50], &[1.0; 3]);
        assert_eq!(dl.time_index_generation(), base);
        // A point inside the union moves every later index.
        dl.update(b, 25, [1.0; 4]);
        let inserted = dl.time_index_generation();
        assert_ne!(inserted, base);
        // So does a point leaving the front.
        set(&mut dl, a, &[20, 30, 40], &[1.0; 3]);
        assert_ne!(dl.time_index_generation(), inserted);
        let trimmed = dl.time_index_generation();
        // A replay cutoff that reveals points at the end keeps indices; one that hides them does not.
        dl.set_time_cutoff(Some(40));
        let hidden = dl.time_index_generation();
        assert_ne!(hidden, trimmed);
        dl.set_time_cutoff(None);
        assert_eq!(dl.time_index_generation(), hidden);
    }

    #[test]
    fn merged_time_mapping_interpolates_common_timestamps_and_extrapolates_with_unit_slope() {
        let mut dl = DataLayer::new();
        let id = dl.add_series();
        set(&mut dl, id, &[10, 20, 30], &[1.0; 3]);

        dl.begin_merged_time_transaction();
        assert!(dl.update(id, 15, [2.0; 4]));
        let mapping = dl.take_merged_time_mapping().unwrap();

        assert_eq!(mapping.map_logical(0.0), 0.0);
        assert_eq!(mapping.map_logical(0.5), 1.0);
        assert_eq!(mapping.map_logical(1.0), 2.0);
        assert_eq!(mapping.map_logical(2.0), 3.0);
        assert_eq!(mapping.map_logical(-1.25), -1.25);
        assert_eq!(mapping.map_logical(3.5), 4.5);
    }

    #[test]
    fn merged_time_mapping_is_identity_without_common_timestamps() {
        let mut dl = DataLayer::new();
        let id = dl.add_series();
        set(&mut dl, id, &[10, 20, 30], &[1.0; 3]);

        dl.begin_merged_time_transaction();
        set(&mut dl, id, &[40, 50, 60], &[2.0; 3]);
        let mapping = dl.take_merged_time_mapping().unwrap();
        assert_eq!(mapping.map_logical(-1.5), -1.5);
        assert_eq!(mapping.map_logical(1.25), 1.25);
        assert_eq!(mapping.map_logical(4.0), 4.0);
    }

    #[test]
    fn merged_time_mapping_uses_the_transaction_final_union() {
        let mut dl = DataLayer::new();
        let primary = dl.add_series();
        let divergent = dl.add_series();
        set(&mut dl, primary, &[10, 20, 30, 40], &[1.0; 4]);
        set(&mut dl, divergent, &[20], &[2.0]);

        dl.begin_merged_time_transaction();
        set(&mut dl, primary, &[15, 30, 40], &[3.0; 3]);
        set(&mut dl, divergent, &[20, 35], &[4.0; 2]);
        let mapping = dl.take_merged_time_mapping().unwrap();

        assert_eq!(dl.merged_times(), &[15, 20, 30, 35, 40]);
        assert_eq!(mapping.map_logical(1.0), 1.0); // shared timestamp 20
        assert_eq!(mapping.map_logical(2.0), 2.0); // shared timestamp 30
        assert_eq!(mapping.map_logical(3.0), 4.0); // shared timestamp 40
        assert_eq!(mapping.map_logical(2.5), 3.0);
    }

    #[test]
    fn merged_time_mapping_retains_only_slope_change_breakpoints() {
        let old: Vec<i64> = (0..10_000).map(|index| i64::from(index) * 2).collect();
        let mut new = old.clone();
        new.insert(5_000, 9_999);

        let mapping = MergedTimeMapping::between(&old, &new);

        assert_eq!(mapping.common_indices.len(), 4);
        assert_eq!(mapping.map_logical(4_999.5), 5_000.0);
        assert_eq!(mapping.map_logical(9_999.0), 10_000.0);
    }

    #[test]
    fn merged_time_mapping_reports_extent_translation_and_old_union() {
        let mut dl = DataLayer::new();
        let id = dl.add_series();
        set(&mut dl, id, &[30, 40, 50], &[1.0; 3]);

        // Prepend: a pure translation over the shared extent, with the old union retained.
        dl.begin_merged_time_transaction();
        set(&mut dl, id, &[10, 20, 30, 40, 50], &[1.0; 5]);
        let mapping = dl.take_merged_time_mapping().unwrap();
        assert!(mapping.is_translation());
        assert_eq!(mapping.common_old_extent(), Some((0, 2)));
        assert_eq!(mapping.old_times(), &[30, 40, 50]);

        // Identity-collapsed breakpoints still report their shared extent and translation.
        dl.begin_merged_time_transaction();
        set(&mut dl, id, &[10, 20, 30, 40, 50, 60], &[1.0; 6]);
        let mapping = dl.take_merged_time_mapping().unwrap();
        assert!(mapping.is_translation());
        assert_eq!(mapping.common_old_extent(), Some((0, 4)));
        assert_eq!(mapping.map_logical(7.0), 7.0);

        // A coarser interval shares only aligned stamps with a different index density.
        dl.begin_merged_time_transaction();
        set(&mut dl, id, &[0, 30, 60], &[1.0; 3]);
        let mapping = dl.take_merged_time_mapping().unwrap();
        assert!(!mapping.is_translation());
        assert_eq!(mapping.common_old_extent(), Some((2, 5)));

        // Unrelated stamps share nothing.
        dl.begin_merged_time_transaction();
        set(&mut dl, id, &[1, 2, 3], &[1.0; 3]);
        let mapping = dl.take_merged_time_mapping().unwrap();
        assert!(!mapping.is_translation());
        assert_eq!(mapping.common_old_extent(), None);
        assert_eq!(mapping.old_times(), &[0, 30, 60]);
    }

    #[test]
    fn replacements_and_tail_appends_do_not_create_a_mapping() {
        let mut dl = DataLayer::new();
        let id = dl.add_series();
        set(&mut dl, id, &[10, 20, 30], &[1.0; 3]);

        dl.begin_merged_time_transaction();
        assert!(dl.update(id, 30, [2.0; 4]));
        assert!(dl.take_merged_time_mapping().is_none());

        dl.begin_merged_time_transaction();
        assert!(dl.update(id, 40, [3.0; 4]));
        assert!(dl.take_merged_time_mapping().is_none());
    }

    #[test]
    fn batch_update_merges_history_once_and_keeps_distinct_times() {
        let mut dl = DataLayer::new();
        let id = dl.add_series();
        set(&mut dl, id, &[1, 3, 5], &[10.0, 30.0, 50.0]);
        let before = dl.series_generation(id).unwrap();

        let times = [2, 3, 4];
        let values = [20.0, 33.0, 40.0];
        assert_eq!(
            dl.update_many(id, &times, [&values, &values, &values, &values]),
            Some(1)
        );

        let (actual_times, columns) = dl.series_data(id).unwrap();
        assert_eq!(actual_times, &[1, 2, 3, 4, 5]);
        assert_eq!(columns[3], &[10.0, 20.0, 33.0, 40.0, 50.0]);
        assert_eq!(dl.merged_times(), actual_times);
        assert_eq!(dl.series_generation(id), Some(before + 3));
    }

    #[test]
    fn batch_tail_replace_and_append_match_single_updates() {
        let mut batch = DataLayer::new();
        let batch_id = batch.add_series();
        set(&mut batch, batch_id, &[1, 2], &[10.0, 20.0]);

        let mut singles = DataLayer::new();
        let singles_id = singles.add_series();
        set(&mut singles, singles_id, &[1, 2], &[10.0, 20.0]);

        let times = [2, 3, 4];
        let values = [22.0, 30.0, 40.0];
        batch.update_many(batch_id, &times, [&values, &values, &values, &values]);
        for (&time, &value) in times.iter().zip(&values) {
            singles.update(singles_id, time, [value; 4]);
        }

        assert_eq!(batch.merged_times(), singles.merged_times());
        assert_eq!(
            batch.series_data(batch_id).unwrap().0,
            singles.series_data(singles_id).unwrap().0
        );
        assert_eq!(
            batch.series_data(batch_id).unwrap().1,
            singles.series_data(singles_id).unwrap().1
        );
    }

    /// Late corrections of bars N-1/N-2 are the common real-time case. They must repair only the
    /// corrected row (LOD path and autoscale chunk) instead of merging the timestamp union and
    /// reindexing every series, which is O(total rows).
    #[test]
    fn historical_correction_of_an_existing_time_is_local_work() {
        let mut dl = DataLayer::new();
        let a = dl.add_series();
        let b = dl.add_series();
        let rows = 20_000i64;
        let times = (0..rows).map(|row| row * 60).collect::<Vec<_>>();
        let values = (0..rows).map(|row| row as f64).collect::<Vec<_>>();
        set(&mut dl, a, &times, &values);
        let sparse = times.iter().copied().step_by(3).collect::<Vec<_>>();
        set(&mut dl, b, &sparse, &vec![1.0; sparse.len()]);
        let last = (rows - 1) as TimePointIndex;
        // Warm the autoscale chunk that holds N-2 so a stale cache would be observable.
        let before = dl
            .min_max_on_range_cached(
                a,
                last - 5,
                last,
                &[PlotValueIndex::High, PlotValueIndex::Low],
            )
            .unwrap();
        assert_eq!(
            (before.min, before.max),
            ((rows - 6) as f64, (rows - 1) as f64)
        );
        let generation = dl.time_points_generation();
        let passes = dl.index_rebuilds();
        let series_generation = dl.series_generation(a).unwrap();

        dl.begin_merged_time_transaction();
        assert!(dl.update(a, times[times.len() - 2], [5.0, 1.0e6, -1.0e6, 7.0]));
        assert!(dl.update(a, times[times.len() - 3], [8.0, 9.0, 4.0, 6.0]));

        assert_eq!(dl.index_rebuilds(), passes, "no union merge or reindex");
        assert_eq!(dl.time_points_generation(), generation);
        assert!(dl.take_merged_time_mapping().is_none());
        assert_eq!(dl.series_generation(a), Some(series_generation + 2));
        assert!(dl.last_lod_update_nodes(a).unwrap() <= 8);
        let after = dl
            .min_max_on_range_cached(
                a,
                last - 5,
                last,
                &[PlotValueIndex::High, PlotValueIndex::Low],
            )
            .unwrap();
        assert_eq!((after.min, after.max), (-1.0e6, 1.0e6));
        assert_eq!(value_at_index(&dl, a, last - 1, PlotValueIndex::Close), 7.0);
        assert_eq!(value_at_index(&dl, a, last - 2, PlotValueIndex::Open), 8.0);
        assert_eq!(dl.merged_times(), times.as_slice());
        assert_lod_matches_fresh(&dl, a);
    }

    /// A time the union already holds (owned by another series) inserts a new row for this series
    /// without re-sorting the union; only a genuinely new time merges the union.
    #[test]
    fn historical_insert_at_an_existing_union_time_skips_the_union_merge() {
        let mut dl = DataLayer::new();
        let a = dl.add_series();
        let b = dl.add_series();
        set(&mut dl, a, &[1, 2, 3, 4, 5], &[1.0, 2.0, 3.0, 4.0, 5.0]);
        set(&mut dl, b, &[1, 5], &[10.0, 50.0]);
        let generation = dl.time_points_generation();
        let passes = dl.index_rebuilds();
        dl.begin_merged_time_transaction();

        assert!(dl.update(b, 3, [30.0; 4]));
        assert_eq!(
            dl.index_rebuilds(),
            passes + 1,
            "one reindex pass, no union merge"
        );
        assert_eq!(dl.time_points_generation(), generation);
        assert!(dl.take_merged_time_mapping().is_none());
        assert_eq!(indices(&dl, b), [0, 2, 4]);
        assert_eq!(value_at_index(&dl, b, 2, PlotValueIndex::Close), 30.0);
        assert_eq!(dl.merged_times(), &[1, 2, 3, 4, 5]);

        // A genuinely new historical time still merges the union and rebases logical indices.
        dl.begin_merged_time_transaction();
        assert!(dl.update(b, 0, [0.5; 4]));
        assert_ne!(dl.time_points_generation(), generation);
        assert_eq!(dl.merged_times(), &[0, 1, 2, 3, 4, 5]);
        assert!(dl.take_merged_time_mapping().is_some());
        assert_eq!(indices(&dl, a), [1, 2, 3, 4, 5]);
        assert_eq!(indices(&dl, b), [0, 1, 3, 5]);
    }

    /// A typed batch that only corrects existing historical rows applies in place, like the single
    /// path, instead of copying every column and reindexing the layer.
    #[test]
    fn batch_corrections_of_existing_times_apply_in_place() {
        let mut dl = DataLayer::new();
        let id = dl.add_series();
        let times = (0..5_000i64).collect::<Vec<_>>();
        let values = (0..5_000).map(|row| row as f64).collect::<Vec<_>>();
        set(&mut dl, id, &times, &values);
        assert!(dl.set_point_colors(id, [Some(vec![7; 5_000]), None, None]));
        let passes = dl.index_rebuilds();
        let generation = dl.time_points_generation();
        let before = dl.series_generation(id).unwrap();

        let open = [1.0, 2.0];
        let high = [10.0, 20.0];
        let low = [0.5, 1.5];
        let close = [5.0, 6.0];
        assert_eq!(
            dl.update_many(id, &[4_990, 4_995], [&open, &high, &low, &close]),
            Some(4_990)
        );
        assert_eq!(dl.index_rebuilds(), passes);
        assert_eq!(dl.time_points_generation(), generation);
        assert_eq!(dl.series_generation(id), Some(before + 2));
        let (_, columns) = dl.series_data(id).unwrap();
        assert_eq!(columns[1][4_990], 10.0);
        assert_eq!(columns[3][4_995], 6.0);
        assert_eq!(columns[3][4_994], 4_994.0);
        assert_eq!(dl.point_color(id, PointColorChannel::Body, 4_990), None);
        assert_eq!(dl.point_color(id, PointColorChannel::Body, 4_991), Some(7));
        assert_lod_matches_fresh(&dl, id);

        // The scalar batch path keeps one canonical column through the same in-place correction.
        let scalar = dl.add_series();
        set(&mut dl, scalar, &[1, 2, 3], &[1.0, 2.0, 3.0]);
        let passes = dl.index_rebuilds();
        let replacement = [20.0];
        assert_eq!(dl.update_many_single(scalar, &[2], &replacement), Some(1));
        assert_eq!(dl.index_rebuilds(), passes);
        assert_eq!(dl.series_data(scalar).unwrap().1[3], &[1.0, 20.0, 3.0]);
    }

    /// The common feed batch corrects a late bar and streams the current bar in one call. The
    /// correction applies in place and the tail rows append through the streaming path, so the
    /// batch never merges the union or reindexes the layer, and the result equals the same rows
    /// applied one by one. A new historical time still takes the merging path.
    #[test]
    fn batch_correction_plus_tail_append_stays_in_place() {
        let rows = 5_000i64;
        let times = (0..rows).collect::<Vec<_>>();
        let values = (0..rows).map(|row| row as f64).collect::<Vec<_>>();
        let mut dl = DataLayer::new();
        let id = dl.add_series();
        set(&mut dl, id, &times, &values);
        let passes = dl.index_rebuilds();
        let generation = dl.time_points_generation();

        let batch = [rows - 3, rows - 1, rows, rows + 1];
        let open = [1.0, 2.0, 3.0, 4.0];
        let high = [10.0, 20.0, 30.0, 40.0];
        let low = [0.5, 1.5, 2.5, 3.5];
        let close = [5.0, 6.0, 7.0, 8.0];
        dl.begin_merged_time_transaction();
        assert_eq!(
            dl.update_many(id, &batch, [&open, &high, &low, &close]),
            Some((rows - 3) as usize)
        );
        assert_eq!(dl.index_rebuilds(), passes, "no union merge or reindex");
        assert_eq!(dl.time_points_generation(), generation + 2);
        assert!(dl.take_merged_time_mapping().is_none());
        let (stored_times, columns) = dl.series_data(id).unwrap();
        assert_eq!(stored_times.len(), (rows + 2) as usize);
        assert_eq!(columns[1][(rows - 3) as usize], 10.0);
        assert_eq!(columns[3][(rows - 2) as usize], (rows - 2) as f64);
        assert_eq!(columns[3][(rows - 1) as usize], 6.0);
        assert_eq!(columns[0][(rows + 1) as usize], 4.0);
        assert_lod_matches_fresh(&dl, id);
        let mut single = DataLayer::new();
        let one_by_one = single.add_series();
        set(&mut single, one_by_one, &times, &values);
        for (index, &time) in batch.iter().enumerate() {
            assert!(single.update(
                one_by_one,
                time,
                [open[index], high[index], low[index], close[index]]
            ));
        }
        assert_eq!(single.series_data(one_by_one), dl.series_data(id));
        assert_eq!(indices(&single, one_by_one), indices(&dl, id));

        // A new historical time in the same shape of batch merges once, as before.
        let passes = dl.index_rebuilds();
        let inserted = [-1, rows + 1];
        assert_eq!(
            dl.update_many(
                id,
                &inserted,
                [&[9.0, 9.0], &[9.0, 9.0], &[9.0, 9.0], &[9.0, 9.0]]
            ),
            Some(0)
        );
        assert!(dl.index_rebuilds() > passes);
        assert_eq!(dl.series_data(id).unwrap().0[0], -1);
        assert_lod_matches_fresh(&dl, id);
    }

    /// Reference `series.update` colors the updated item even when no earlier item had a color:
    /// the first explicit color creates the row-aligned channel in every streaming path.
    #[test]
    fn first_styled_update_creates_its_color_channel() {
        let mut dl = DataLayer::new();
        let id = dl.add_series();
        assert!(dl.update_styled(id, 10, [1.0; 4], [Some(7), None, None]));
        assert_eq!(dl.point_color(id, PointColorChannel::Body, 0), Some(7));
        assert!(dl.point_colors(id).unwrap().channels[1].is_empty());

        let plain = dl.add_series();
        set(&mut dl, plain, &[10, 20, 30, 40], &[1.0, 2.0, 3.0, 4.0]);
        assert!(!dl.has_point_colors(plain));
        // Replace-last, append, historical correction, and historical insert each color only their
        // own bar and keep the channel aligned with the rows.
        assert!(dl.update_styled(plain, 40, [5.0; 4], [None, Some(1), None]));
        assert!(dl.update_styled(plain, 50, [6.0; 4], [Some(2), None, None]));
        assert!(dl.update_styled(plain, 20, [7.0; 4], [None, None, Some(3)]));
        assert!(dl.update_styled(plain, 25, [8.0; 4], [Some(4), None, None]));
        let colors = |channel| {
            (0..6)
                .map(|row| dl.point_color(plain, channel, row))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            colors(PointColorChannel::Body),
            [None, None, Some(4), None, None, Some(2)]
        );
        assert_eq!(
            colors(PointColorChannel::Wick),
            [None, None, None, None, Some(1), None]
        );
        assert_eq!(
            colors(PointColorChannel::Border),
            [None, Some(3), None, None, None, None]
        );
        for channel in dl.point_colors(plain).unwrap().channels {
            assert_eq!(channel.len(), 6);
        }
        // An unstyled update neither creates a channel nor disturbs existing ones.
        let untouched = dl.add_series();
        set(&mut dl, untouched, &[10, 20], &[1.0, 2.0]);
        assert!(dl.update(untouched, 30, [3.0; 4]));
        assert!(!dl.has_point_colors(untouched));
    }

    #[test]
    fn update_into_other_series_gap_uses_existing_index() {
        let mut dl = DataLayer::new();
        let a = dl.add_series();
        let b = dl.add_series();
        set(&mut dl, a, &[1, 2, 3, 4], &[1.0, 2.0, 3.0, 4.0]);
        set(&mut dl, b, &[1, 4], &[9.0, 9.0]); // whitespace at 2,3
                                               // B gets a point at time 3 (an existing merged time, index 2)
        dl.update(b, 3, [7.0, 7.0, 7.0, 7.0]);
        assert_eq!(dl.merged_times(), &[1, 2, 3, 4]);
        assert!(dl.plot(b).contains(2)); // time 3 -> merged index 2
        assert_eq!(value_at_index(&dl, b, 2, PlotValueIndex::Close), 7.0);
    }

    #[test]
    fn empty_layer() {
        let mut dl = DataLayer::new();
        let a = dl.add_series();
        assert_eq!(dl.merged_times(), &[] as &[i64]);
        assert_eq!(dl.base_index(), None);
        assert!(dl.plot(a).is_empty());
    }

    #[test]
    fn point_colors_validate_lengths_and_clear() {
        let mut dl = DataLayer::new();
        let a = dl.add_series();
        set(&mut dl, a, &[1, 2, 3], &[10.0, 20.0, 30.0]);
        assert!(!dl.has_point_colors(a));

        // A channel longer than the row count rejects the whole call (no partial state).
        assert!(!dl.set_point_colors(a, [Some(vec![7, 8]), None, None]));
        assert!(!dl.has_point_colors(a));

        assert!(dl.set_point_colors(a, [Some(vec![11, 0, 33]), Some(vec![1, 2, 3]), None]));
        assert!(dl.has_point_colors(a));
        assert_eq!(dl.point_color(a, PointColorChannel::Body, 0), Some(11));
        assert_eq!(dl.point_color(a, PointColorChannel::Body, 1), None); // 0 = absent
        assert_eq!(dl.point_color(a, PointColorChannel::Body, 2), Some(33));
        assert_eq!(dl.point_color(a, PointColorChannel::Wick, 1), Some(2));
        assert_eq!(dl.point_color(a, PointColorChannel::Border, 1), None);

        // None/empty channels clear.
        assert!(dl.set_point_colors(a, [None, Some(vec![]), None]));
        assert!(!dl.has_point_colors(a));
    }

    #[test]
    fn set_data_resets_point_colors() {
        let mut dl = DataLayer::new();
        let a = dl.add_series();
        set(&mut dl, a, &[1, 2], &[10.0, 20.0]);
        assert!(dl.set_point_colors(a, [Some(vec![5, 6]), None, None]));
        set(&mut dl, a, &[1, 2, 3], &[10.0, 20.0, 30.0]);
        assert!(!dl.has_point_colors(a));
        assert_eq!(dl.point_color(a, PointColorChannel::Body, 0), None);
    }

    #[test]
    fn update_keeps_point_colors_aligned() {
        let mut dl = DataLayer::new();
        let a = dl.add_series();
        set(&mut dl, a, &[1, 2, 3], &[10.0, 20.0, 30.0]);
        assert!(dl.set_point_colors(a, [Some(vec![11, 22, 33]), None, None]));

        // Append with a styled update: the new row carries its override.
        dl.update_styled(a, 4, [40.0, 41.0, 39.0, 40.0], [Some(44), None, None]);
        assert_eq!(dl.point_color(a, PointColorChannel::Body, 3), Some(44));

        // Append with a plain update: the new row has no override, the channel stays aligned.
        dl.update(a, 5, [50.0, 51.0, 49.0, 50.0]);
        assert_eq!(dl.point_color(a, PointColorChannel::Body, 4), None);
        assert_eq!(dl.point_color(a, PointColorChannel::Body, 3), Some(44));

        // Replace-last with a plain update clears that bar's override (reference whole-bar
        // replacement); a styled replace sets it.
        dl.update(a, 5, [55.0, 56.0, 54.0, 55.0]);
        assert_eq!(dl.point_color(a, PointColorChannel::Body, 4), None);
        dl.update_styled(a, 5, [55.0, 56.0, 54.0, 55.0], [Some(55), None, None]);
        assert_eq!(dl.point_color(a, PointColorChannel::Body, 4), Some(55));
    }

    #[test]
    fn pop_truncates_rows_colors_and_merged_times() {
        let mut dl = DataLayer::new();
        let a = dl.add_series();
        let b = dl.add_series();
        set(&mut dl, a, &[1, 2, 3, 4], &[10.0, 20.0, 30.0, 40.0]);
        set(&mut dl, b, &[3, 4], &[90.0, 95.0]);
        assert!(dl.set_point_colors(a, [Some(vec![11, 22, 33, 44]), None, None]));

        // Clamp to the row count; colors shift along with their rows (reference popSeriesData).
        assert_eq!(dl.pop(a, 10), Some(0));
        assert!(indices(&dl, a).is_empty());
        // A's times left the merged axis; B's remain.
        assert_eq!(dl.merged_times(), &[3, 4]);
        assert!(!dl.has_point_colors(a));

        set(&mut dl, a, &[1, 2, 3, 4], &[10.0, 20.0, 30.0, 40.0]);
        assert!(dl.set_point_colors(a, [Some(vec![11, 22, 33, 44]), None, None]));
        assert_eq!(dl.pop(a, 0), Some(4)); // count 0 is a no-op
        assert_eq!(dl.pop(a, 2), Some(2));
        assert_eq!(indices(&dl, a), [0, 1]);
        assert_eq!(dl.point_color(a, PointColorChannel::Body, 0), Some(11));
        assert_eq!(dl.point_color(a, PointColorChannel::Body, 1), Some(22));
        assert_eq!(dl.point_color(a, PointColorChannel::Body, 2), None);
        // shared times 3,4 survive through B
        assert_eq!(dl.merged_times(), &[1, 2, 3, 4]);
        assert_eq!(dl.pop(a, 5), Some(0));
        assert_eq!(dl.merged_times(), &[3, 4]);
    }

    #[test]
    fn trim_front_evicts_oldest_rows_colors_and_merged_times() {
        let mut dl = DataLayer::new();
        let a = dl.add_series();
        let b = dl.add_series();
        set(&mut dl, a, &[1, 2, 3, 4], &[10.0, 20.0, 30.0, 40.0]);
        set(&mut dl, b, &[3, 4], &[90.0, 95.0]);
        assert!(dl.set_point_colors(a, [Some(vec![11, 22, 33, 44]), None, None]));

        // Keeping at least the row count is a no-op.
        assert_eq!(dl.trim_front(a, 4), Some(4));
        assert_eq!(dl.trim_front(a, 9), Some(4));
        assert_eq!(dl.merged_times(), &[1, 2, 3, 4]);

        // Oldest-first: rows 1,2 leave; the surviving colors are those of rows 3,4.
        assert_eq!(dl.trim_front(a, 2), Some(2));
        assert_eq!(indices(&dl, a), [0, 1]);
        assert_eq!(dl.point_color(a, PointColorChannel::Body, 0), Some(33));
        assert_eq!(dl.point_color(a, PointColorChannel::Body, 1), Some(44));
        // Times 1,2 belonged to A alone, so they leave the shared axis; 3,4 survive through both.
        assert_eq!(dl.merged_times(), &[3, 4]);
        assert_eq!(dl.series_data(a).unwrap().1[3], &[30.0, 40.0]);
        // B is untouched by A's eviction.
        assert_eq!(dl.series_data(b).unwrap().0, &[3, 4]);

        assert_eq!(dl.trim_front(a, 0), Some(0));
        assert!(!dl.has_point_colors(a));
        assert_eq!(dl.merged_times(), &[3, 4]);
    }

    /// Everything a front trim can change in one series, to compare two layers trim for trim.
    #[derive(Debug, PartialEq)]
    struct SeriesObservation {
        times: Vec<i64>,
        columns: Vec<Vec<u64>>,
        indices: Vec<TimePointIndex>,
        colors: [Vec<u32>; POINT_COLOR_CHANNELS],
        generation: Option<u64>,
        aliased: bool,
        alignment: Option<TimeAlignment>,
    }

    fn observe(
        dl: &DataLayer,
        ids: &[SeriesId],
    ) -> (Vec<i64>, Option<TimePointIndex>, Vec<SeriesObservation>) {
        let series = ids
            .iter()
            .map(|&id| {
                let (times, columns) = dl.series_data(id).unwrap();
                let slot = dl.series_slot(id).unwrap();
                SeriesObservation {
                    times: times.to_vec(),
                    columns: columns
                        .iter()
                        .map(|column| column.iter().map(|value| value.to_bits()).collect())
                        .collect(),
                    indices: indices(dl, id),
                    colors: dl.series[slot].point_colors.clone(),
                    generation: dl.series_generation(id),
                    aliased: dl.series[slot].time_alias.is_some(),
                    alignment: dl.time_alignment(id),
                }
            })
            .collect();
        (dl.merged_times().to_vec(), dl.base_index(), series)
    }

    /// Three union series (`a` with point colors, `b` offset in time, `c` sparse), an as-of
    /// overlay and a `2 * close` output aliasing `a`.
    fn trim_layer() -> (DataLayer, [SeriesId; 5]) {
        let mut dl = DataLayer::new();
        let ids = [(); 5].map(|()| dl.add_series());
        let [a, b, c, overlay, output] = ids;
        let values = |rows: usize| (0..rows).map(|row| row as f64 + 1.0).collect::<Vec<_>>();
        set(&mut dl, a, &(1..=12).collect::<Vec<_>>(), &values(12));
        set(&mut dl, b, &(5..=16).collect::<Vec<_>>(), &values(12));
        set(&mut dl, c, &[3, 6, 9, 12, 15, 18], &values(6));
        set(&mut dl, overlay, &[2, 7, 11, 14, 20], &values(5));
        assert!(dl.set_time_alignment(overlay, AS_OF));
        let colors = |base: u32| Some((0..12).map(|row| base + row).collect::<Vec<u32>>());
        assert!(dl.set_point_colors(a, [colors(100), None, colors(300)]));
        let doubled = (2..12).map(|row| f64::from(row) * 2.0).collect();
        assert!(dl.set_single_data_aligned(output, a, 2, doubled));
        (dl, ids)
    }

    /// Run `trims` as one `trim_fronts` on one layer and as sequential `trim_front` calls on an
    /// identical one; both must end in the same state. Returns each arm's union and reindex passes.
    fn assert_trim_fronts_equals_sequential(
        prepare: impl Fn(&mut DataLayer, [SeriesId; 5]),
        trims: impl Fn([SeriesId; 5]) -> Vec<(SeriesId, usize)>,
    ) -> (u64, u64) {
        let (mut batched, ids) = trim_layer();
        let (mut sequential, _) = trim_layer();
        for dl in [&mut batched, &mut sequential] {
            prepare(dl, ids);
            dl.begin_merged_time_transaction();
        }
        let trims = trims(ids);
        let points = |dl: &DataLayer| (dl.time_points_generation(), dl.time_index_generation());
        let (batched_points, sequential_points) = (points(&batched), points(&sequential));
        let batched_passes = batched.index_rebuilds();
        batched.trim_fronts(&trims);
        let batched_passes = batched.index_rebuilds() - batched_passes;
        let sequential_passes = sequential.index_rebuilds();
        for &(id, keep) in &trims {
            sequential.trim_front(id, keep);
        }
        let sequential_passes = sequential.index_rebuilds() - sequential_passes;

        assert_eq!(observe(&batched, &ids), observe(&sequential, &ids));
        // A batch bumps the union counters at most once where sequential trims bump them per
        // step, so they only agree on whether the union moved at all.
        assert_eq!(
            points(&batched) != batched_points,
            points(&sequential) != sequential_points
        );
        assert_eq!(
            batched.take_merged_time_mapping(),
            sequential.take_merged_time_mapping()
        );
        for id in &ids[..3] {
            assert_lod_matches_fresh(&batched, *id);
        }
        assert_plot_lod_matches_fresh(&batched, ids[3]);
        (batched_passes, sequential_passes)
    }

    #[test]
    fn trim_fronts_matches_sequential_trims_with_one_union_rebuild() {
        // Three series lose rows: one union merge and one reindex instead of three of each.
        let passes = assert_trim_fronts_equals_sequential(
            |_, _| {},
            |[a, b, c, ..]| vec![(a, 8), (b, 9), (c, 4)],
        );
        assert_eq!(passes, (2, 6));

        // Under a replay clock a row count keeps rows past the clock, exactly as before.
        let passes = assert_trim_fronts_equals_sequential(
            |dl, _| assert!(dl.set_time_cutoff(Some(9))),
            |[a, b, c, ..]| vec![(a, 8), (b, 9), (c, 4)],
        );
        assert_eq!(passes, (2, 6));

        // Trimming the aliased source and the output together.
        let passes = assert_trim_fronts_equals_sequential(
            |_, _| {},
            |[a, _, _, _, output]| vec![(a, 6), (output, 3)],
        );
        assert_eq!(passes, (2, 4));
    }

    #[test]
    fn trim_fronts_skips_unknown_ids_keeps_no_ops_free_and_applies_duplicates_in_order() {
        // Nothing to drop and an unknown id: no union work at all.
        let passes = assert_trim_fronts_equals_sequential(
            |_, _| {},
            |[a, b, ..]| vec![(a, 12), (b, 99), (9_999, 0)],
        );
        assert_eq!(passes, (0, 0));

        // The second entry for a series sees the rows the first one left.
        let passes = assert_trim_fronts_equals_sequential(
            |_, _| {},
            |[a, b, ..]| vec![(a, 8), (a, 5), (b, 7)],
        );
        assert_eq!(passes, (2, 6));

        // An aliased output owns its times first, even when it drops nothing.
        let (mut dl, [a, .., output]) = trim_layer();
        let rebuilds = dl.index_rebuilds();
        dl.trim_fronts(&[(output, 10)]);
        assert_eq!(dl.index_rebuilds(), rebuilds);
        assert!(dl.series[dl.series_slot(output).unwrap()]
            .time_alias
            .is_none());
        assert_eq!(
            dl.series_data(output).unwrap().0,
            &dl.series_data(a).unwrap().0[2..]
        );
        let passes =
            assert_trim_fronts_equals_sequential(|_, _| {}, |[.., output]| vec![(output, 10)]);
        assert_eq!(passes, (0, 0));
        let (mut dl, _) = trim_layer();
        assert_eq!(dl.trim_front(9_999, 0), None);
    }

    /// The sorted union the way the merge replaced: concatenate, sort, deduplicate.
    fn sorted_union(runs: &[Vec<i64>]) -> Vec<i64> {
        let mut all = runs.concat();
        all.sort_unstable();
        all.dedup();
        all
    }

    #[test]
    fn merge_sorted_runs_equals_sorting_the_concatenation() {
        let mut rng = Rng(0x2545_F491_4F6C_DD1D);
        let random_run = |rng: &mut Rng, len: u64, span: u64| {
            let mut run = (0..len).map(|_| rng.below(span) as i64).collect::<Vec<_>>();
            run.sort_unstable();
            run.dedup();
            run
        };
        let check = |runs: Vec<Vec<i64>>| {
            let mut slices = runs.iter().map(Vec::as_slice).collect::<Vec<_>>();
            let mut merged = vec![-1];
            merged.clear();
            merge_sorted_runs(&mut slices, &mut merged);
            assert_eq!(merged, sorted_union(&runs), "{runs:?}");
        };
        check(Vec::new());
        check(vec![Vec::new(), Vec::new()]);
        check(vec![vec![3, 5, 9]]);
        check(vec![Vec::new(), vec![1, 2], Vec::new(), vec![2, 3]]);
        // A run that repeats a time still yields each time once, as the sort did.
        check(vec![vec![1, 1, 2], vec![2, 2, 3]]);
        let identical = random_run(&mut rng, 200, 1_000);
        check(vec![identical.clone(); 4]);
        check(vec![identical; MAX_MERGED_RUNS]);
        for _ in 0..200 {
            let runs = (0..1 + rng.below(MAX_MERGED_RUNS as u64))
                .map(|_| {
                    let (len, span) = (rng.below(60), 1 + rng.below(150));
                    random_run(&mut rng, len, span)
                })
                .collect::<Vec<_>>();
            check(runs);
        }
    }

    #[test]
    fn merged_union_matches_a_sorted_concatenation_below_and_above_the_merge_width() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for series_count in [
            1,
            2,
            MAX_MERGED_RUNS,
            MAX_MERGED_RUNS + 1,
            2 * MAX_MERGED_RUNS,
        ] {
            let mut dl = DataLayer::new();
            let mut runs = Vec::new();
            for _ in 0..series_count {
                let id = dl.add_series();
                let mut times = (0..40).map(|_| rng.below(120) as i64).collect::<Vec<_>>();
                times.sort_unstable();
                times.dedup();
                let values = vec![1.0; times.len()];
                set(&mut dl, id, &times, &values);
                runs.push(times);
            }
            assert_eq!(
                dl.merged_times(),
                sorted_union(&runs),
                "{series_count} series"
            );
            // The replay cutoff masks every run the same way before the merge.
            assert!(dl.set_time_cutoff(Some(60)));
            let visible = runs
                .iter()
                .map(|run| run.iter().copied().filter(|&time| time <= 60).collect())
                .collect::<Vec<_>>();
            assert_eq!(
                dl.merged_times(),
                sorted_union(&visible),
                "{series_count} series, cutoff"
            );
        }
    }

    #[test]
    fn rows_count_as_data_series_anchor_the_base_index() {
        let mut dl = DataLayer::new();
        let a = dl.add_series();
        let nan = f64::NAN;
        // A custom series (Phase C-c): time-only (whitespace-style) rows whose values live
        // host-side. Like the reference's custom plot rows (which carry values), they count as data for
        // the base index once flagged.
        dl.set_data(
            a,
            vec![1, 2, 3],
            vec![nan, nan, nan],
            vec![nan, nan, nan],
            vec![nan, nan, nan],
            vec![nan, nan, nan],
        );
        // Unflagged, an all-whitespace series anchors on 0 (the reference's initialized baseIndex).
        assert_eq!(dl.base_index(), Some(0));
        dl.set_rows_count_as_data(a, true);
        assert_eq!(dl.base_index(), Some(2));
        // A second ordinary series' real bars still win by time, and clearing the flag
        // restores whitespace semantics.
        let b = dl.add_series();
        set(&mut dl, b, &[1], &[7.0]);
        assert_eq!(dl.base_index(), Some(2));
        dl.set_rows_count_as_data(a, false);
        assert_eq!(dl.base_index(), Some(0));
    }

    #[test]
    fn whitespace_rows_stay_in_place_and_off_the_base_index() {
        let mut dl = DataLayer::new();
        let a = dl.add_series();
        let nan = f64::NAN;
        // bars at 1,2,4 and explicit whitespace rows at 3,5 (reference `{time}`-only items)
        dl.set_data(
            a,
            vec![1, 2, 3, 4, 5],
            vec![10.0, 20.0, nan, 40.0, nan],
            vec![10.0, 20.0, nan, 40.0, nan],
            vec![10.0, 20.0, nan, 40.0, nan],
            vec![10.0, 20.0, nan, 40.0, nan],
        );
        // the whitespace times occupy merged slots (reference keeps the time-scale points)
        assert_eq!(dl.merged_times(), &[1, 2, 3, 4, 5]);
        assert_eq!(indices(&dl, a), [0, 1, 2, 3, 4]);
        assert!(dl.plot(a).is_whitespace_row(2));
        assert!(dl.plot(a).is_whitespace_row(4));
        // base index = the last point with real data (reference _getBaseIndex), not the trailing ws
        assert_eq!(dl.base_index(), Some(3));
    }

    #[test]
    fn whitespace_update_replaces_and_appends() {
        let mut dl = DataLayer::new();
        let a = dl.add_series();
        let nan = f64::NAN;
        set(&mut dl, a, &[1, 2, 3], &[10.0, 20.0, 30.0]);
        // reference `series.update` with a `{time}`-only item replaces the last bar with whitespace.
        dl.update(a, 3, [nan; 4]);
        assert!(dl.plot(a).is_whitespace_row(2));
        assert_eq!(dl.base_index(), Some(1));
        // a whitespace append creates a time point but does not move the base index
        dl.update(a, 4, [nan; 4]);
        assert_eq!(dl.merged_times(), &[1, 2, 3, 4]);
        assert_eq!(dl.base_index(), Some(1));
        // ...and a real bar at that whitespace time moves it again (the gated reference shift case)
        dl.update(a, 4, [40.0, 41.0, 39.0, 40.0]);
        assert!(!dl.plot(a).is_whitespace_row(3));
        assert_eq!(dl.base_index(), Some(3));
    }

    #[test]
    fn mid_history_insert_splices_point_colors() {
        let mut dl = DataLayer::new();
        let a = dl.add_series();
        set(&mut dl, a, &[1, 2, 4], &[10.0, 20.0, 40.0]);
        assert!(dl.set_point_colors(a, [Some(vec![11, 22, 44]), None, None]));

        // Insert a new bar at time 3 (mid-history rebuild): colors shift with the rows.
        dl.update_styled(a, 3, [30.0, 31.0, 29.0, 30.0], [Some(33), None, None]);
        assert_eq!(
            (0..4)
                .map(|row| dl.point_color(a, PointColorChannel::Body, row))
                .collect::<Vec<_>>(),
            vec![Some(11), Some(22), Some(33), Some(44)]
        );

        // Overwrite a mid-history bar with a plain update: only its override clears.
        dl.update(a, 2, [20.0, 21.0, 19.0, 20.0]);
        assert_eq!(dl.point_color(a, PointColorChannel::Body, 1), None);
        assert_eq!(dl.point_color(a, PointColorChannel::Body, 2), Some(33));
    }

    const DAY: i64 = 86_400;

    fn days(list: &[i64]) -> Vec<i64> {
        list.iter().map(|day| day * DAY).collect()
    }

    /// `(logical index, close)` for every plot row of a series.
    fn plot_points(dl: &DataLayer, id: SeriesId) -> Vec<(TimePointIndex, f64)> {
        let plot = dl.plot(id);
        (0..plot.size())
            .map(|row| {
                (
                    plot.index_at(row).unwrap(),
                    plot.value_at(row, PlotValueIndex::Close),
                )
            })
            .collect()
    }

    /// Reference as-of join over the whole union prefix `[0, through)`.
    fn naive_as_of(
        merged: &[i64],
        through: usize,
        times: &[i64],
        values: &[f64],
        max_staleness: Option<i64>,
    ) -> Vec<(TimePointIndex, f64)> {
        merged[..through]
            .iter()
            .enumerate()
            .filter_map(|(index, &point)| {
                let row = times.iter().rposition(|&time| time <= point)?;
                if max_staleness.is_some_and(|max| point - times[row] > max) {
                    return None;
                }
                Some((index as TimePointIndex, values[row]))
            })
            .collect()
    }

    fn assert_plot_lod_matches_fresh(dl: &DataLayer, id: SeriesId) {
        let slot = dl.series_slot(id).unwrap();
        let series = &dl.series[slot];
        let values = series.values.view().with_rows(series.plot.source_rows());
        let mut fresh = LodPyramid::default();
        fresh.rebuild(values);
        let len = values.len();
        for range in [0..len, len / 3..len, len.saturating_sub(40)..len] {
            let actual = series
                .lod
                .view(values)
                .rows_on_range(range.clone(), usize::MAX)
                .0
                .iter()
                .collect::<Vec<_>>();
            let expected = fresh
                .view(values)
                .rows_on_range(range, usize::MAX)
                .0
                .iter()
                .collect::<Vec<_>>();
            assert_eq!(actual, expected);
        }
        let plot = dl.plot(id);
        assert_eq!(
            plot.last_non_whitespace_row_before(len),
            (0..len).rev().find(|&row| !plot.is_whitespace_row(row))
        );
    }

    const AS_OF: TimeAlignment = TimeAlignment::AsOf {
        max_staleness: None,
    };

    #[test]
    fn as_of_overlay_adds_no_time_points_and_shows_the_last_row_at_or_before_each_point() {
        let mut dl = DataLayer::new();
        let hk = dl.add_series();
        let us = dl.add_series();
        // HK trades d1 d2 d3 d5 d6 (d4 is an HK holiday); US trades d1 d2 d4 d5 d7 (d3 a US
        // holiday, d7 after the last HK day).
        set(
            &mut dl,
            hk,
            &days(&[1, 2, 3, 5, 6]),
            &[1.0, 2.0, 3.0, 5.0, 6.0],
        );
        set(
            &mut dl,
            us,
            &days(&[1, 2, 4, 5, 7]),
            &[10.0, 20.0, 40.0, 50.0, 70.0],
        );
        // The union inserts d4 and d7 into HK and leaves gaps in both series.
        assert_eq!(dl.merged_times().len(), 7);

        assert!(dl.set_time_alignment(us, AS_OF));
        assert!(!dl.set_time_alignment(us, AS_OF));
        assert_eq!(dl.time_alignment(us), Some(AS_OF));
        assert_eq!(dl.merged_times(), days(&[1, 2, 3, 5, 6]).as_slice());
        assert_eq!(indices(&dl, hk), vec![0, 1, 2, 3, 4]);
        assert!(dl.plot(hk).search(3, MismatchDirection::None).is_some());
        // d3 repeats the US d2 close, d4 collapses into d5, d6 repeats d5, and d7 has no point.
        assert_eq!(
            plot_points(&dl, us),
            vec![(0, 10.0), (1, 20.0), (2, 20.0), (3, 50.0), (4, 50.0)]
        );
        let plot = dl.plot(us);
        assert_eq!(plot.source_row(2), 1);
        assert_eq!(plot.row_for_source(1), Some(1));
        assert_eq!(plot.row_for_source(2), None, "collapsed US d4 is not shown");
        assert_eq!(
            plot.row_for_source(4),
            None,
            "US d7 is past the last HK bar"
        );
        // The canonical rows, their times and the base index stay the series' own.
        assert_eq!(
            dl.series_data(us).unwrap().0,
            days(&[1, 2, 4, 5, 7]).as_slice()
        );
        assert_eq!(dl.base_index(), Some(4));
        let min_max = dl
            .min_max_on_range_cached(us, 0, 4, &[PlotValueIndex::Close])
            .unwrap();
        assert_eq!((min_max.min, min_max.max), (10.0, 50.0));
        assert_plot_lod_matches_fresh(&dl, us);

        // Back to the union restores the reference behavior exactly.
        assert!(dl.set_time_alignment(us, TimeAlignment::Union));
        assert_eq!(dl.merged_times().len(), 7);
        assert_eq!(indices(&dl, us), vec![0, 1, 3, 4, 6]);
        assert!(!dl.plot(us).is_as_of());
    }

    #[test]
    fn as_of_tips_on_either_side_rewrite_only_the_affected_points() {
        let mut dl = DataLayer::new();
        let main = dl.add_series();
        let overlay = dl.add_series();
        let main_times = (0..500).map(|minute| minute * 60).collect::<Vec<_>>();
        set(&mut dl, main, &main_times, &vec![1.0; 500]);
        // The overlay prints every 7 minutes, offset by 30 s.
        let overlay_times = (0..71).map(|k| k * 420 + 30).collect::<Vec<_>>();
        let overlay_values = (0..71).map(|k| k as f64).collect::<Vec<_>>();
        set(&mut dl, overlay, &overlay_times, &overlay_values);
        assert!(dl.set_time_alignment(overlay, AS_OF));
        assert_eq!(dl.merged_times(), main_times.as_slice());

        let check = |dl: &DataLayer, times: &[i64], values: &[f64]| {
            let merged = dl.merged_times();
            assert_eq!(
                plot_points(dl, overlay),
                naive_as_of(merged, merged.len(), times, values, None)
            );
            assert_plot_lod_matches_fresh(dl, overlay);
        };
        check(&dl, &overlay_times, &overlay_values);

        // A value tick on the overlay's last row repaints only the points showing it.
        let mut times = overlay_times.clone();
        let mut values = overlay_values.clone();
        let before = dl.as_of_rows_rewritten();
        assert!(dl.update(overlay, *times.last().unwrap(), [99.0; 4]));
        *values.last_mut().unwrap() = 99.0;
        // Its last row (29 430 s) is shown by the nine minutes 491..=499.
        assert_eq!(dl.as_of_rows_rewritten() - before, 9);
        check(&dl, &times, &values);

        // A new overlay row past the last main point shows nothing until the main catches up.
        let rebuilds = dl.index_rebuilds();
        let before = dl.as_of_rows_rewritten();
        assert!(dl.update(overlay, 500 * 60 + 10, [100.0; 4]));
        times.push(500 * 60 + 10);
        values.push(100.0);
        assert_eq!(dl.as_of_rows_rewritten(), before);
        assert_eq!(dl.merged_times(), main_times.as_slice());
        check(&dl, &times, &values);

        // A new main bar extends the overlay by exactly that point.
        let before = dl.as_of_rows_rewritten();
        assert!(dl.update(main, 500 * 60, [2.0; 4]));
        assert!(dl.update(main, 501 * 60, [2.0; 4]));
        assert_eq!(dl.as_of_rows_rewritten() - before, 2);
        assert_eq!(dl.take_realigned(), vec![overlay]);
        assert!(dl.take_realigned().is_empty());
        check(&dl, &times, &values);
        assert_eq!(
            dl.index_rebuilds(),
            rebuilds,
            "tips never rebuild the index"
        );

        // A historical overlay row re-derives that series only.
        assert!(dl.update(overlay, 45, [-1.0; 4]));
        times.insert(1, 45);
        values.insert(1, -1.0);
        check(&dl, &times, &values);
        assert_eq!(dl.merged_times().len(), 502);
    }

    #[test]
    fn as_of_stops_at_the_last_real_bar_and_follows_it_into_session_slots() {
        let mut dl = DataLayer::new();
        let main = dl.add_series();
        let index = dl.add_series();
        let nan = f64::NAN;
        // A session of six slots; only the first two have traded.
        set(
            &mut dl,
            main,
            &[60, 120, 180, 240, 300, 360],
            &[1.0, 2.0, nan, nan, nan, nan],
        );
        set(&mut dl, index, &[0, 150], &[100.0, 101.0]);
        assert!(dl.set_time_alignment(index, AS_OF));
        assert_eq!(plot_points(&dl, index), vec![(0, 100.0), (1, 100.0)]);

        // The third minute trades: the index follows it, but not into the empty slots.
        assert!(dl.update(main, 180, [3.0; 4]));
        assert_eq!(
            plot_points(&dl, index),
            vec![(0, 100.0), (1, 100.0), (2, 101.0)]
        );
        assert_eq!(dl.take_realigned(), vec![index]);
        // Replacing that bar with whitespace pulls the extent back.
        assert!(dl.update(main, 180, [nan; 4]));
        assert_eq!(plot_points(&dl, index), vec![(0, 100.0), (1, 100.0)]);
        assert_plot_lod_matches_fresh(&dl, index);
    }

    #[test]
    fn as_of_max_staleness_blanks_points_whose_row_is_too_old() {
        let mut dl = DataLayer::new();
        let hk = dl.add_series();
        let us = dl.add_series();
        set(&mut dl, hk, &days(&[1, 2, 3, 5, 6]), &[1.0; 5]);
        set(&mut dl, us, &days(&[1, 2, 4, 5]), &[10.0, 20.0, 40.0, 50.0]);
        let exact = TimeAlignment::AsOf {
            max_staleness: Some(0),
        };
        assert!(dl.set_time_alignment(us, exact));
        assert_eq!(plot_points(&dl, us), vec![(0, 10.0), (1, 20.0), (3, 50.0)]);
        assert!(!dl.plot(us).is_empty());
        let one_day = TimeAlignment::AsOf {
            max_staleness: Some(DAY),
        };
        assert!(dl.set_time_alignment(us, one_day));
        assert_eq!(
            plot_points(&dl, us),
            vec![(0, 10.0), (1, 20.0), (2, 20.0), (3, 50.0), (4, 50.0)]
        );
        assert!(!dl.set_time_alignment(
            us,
            TimeAlignment::AsOf {
                max_staleness: Some(-1)
            }
        ));
        // A live main bar two days later is past the bound.
        assert!(dl.update(hk, 8 * DAY, [1.0; 4]));
        assert_eq!(plot_points(&dl, us).len(), 5);
        assert!(dl.update(us, 7 * DAY, [70.0; 4]));
        assert_eq!(plot_points(&dl, us).last(), Some(&(5, 70.0)));
        assert_plot_lod_matches_fresh(&dl, us);
    }

    #[test]
    fn outputs_aliasing_an_as_of_source_follow_its_points_incrementally() {
        let mut dl = DataLayer::new();
        let main = dl.add_series();
        let overlay = dl.add_series();
        let output = dl.add_series();
        let main_times = (0..300).map(|minute| minute * 60).collect::<Vec<_>>();
        set(&mut dl, main, &main_times, &vec![1.0; 300]);
        let overlay_times = (0..100).map(|k| k * 180 + 20).collect::<Vec<_>>();
        set(&mut dl, overlay, &overlay_times, &vec![5.0; 100]);
        assert!(dl.set_time_alignment(overlay, AS_OF));
        // An indicator output from overlay row 10 on.
        let output_values = (10..100).map(|row| row as f64).collect::<Vec<_>>();
        assert!(dl.set_single_data_aligned(output, overlay, 10, output_values.clone()));
        assert_eq!(dl.time_alignment(output), Some(AS_OF));

        let expected = |dl: &DataLayer, values: &[f64]| {
            let times = &dl.series_data(overlay).unwrap().0[10..];
            let merged = dl.merged_times();
            naive_as_of(merged, merged.len(), times, values, None)
        };
        assert_eq!(plot_points(&dl, output), expected(&dl, &output_values));
        assert_plot_lod_matches_fresh(&dl, output);

        // New main points reach the output through its source without a full copy.
        let mut values = output_values;
        for minute in 300..310 {
            assert!(dl.update(main, minute * 60, [1.0; 4]));
            assert_eq!(plot_points(&dl, output), expected(&dl, &values));
        }
        // The source appends a row; the study recomputes its tail incrementally.
        assert!(dl.update(overlay, 100 * 180 + 20, [5.0; 4]));
        values.push(100.0);
        let last = *values.last().unwrap();
        assert_eq!(
            dl.update_single_aligned(output, overlay, 99, &[99.0, last]),
            Some(89)
        );
        assert_eq!(plot_points(&dl, output), expected(&dl, &values));
        assert_plot_lod_matches_fresh(&dl, output);
        // A value-only recompute of the last row repaints the points showing it.
        *values.last_mut().unwrap() = 123.0;
        assert_eq!(
            dl.update_single_aligned(output, overlay, 100, &[123.0]),
            Some(90)
        );
        assert_eq!(plot_points(&dl, output), expected(&dl, &values));
        let min_max = dl
            .min_max_on_range_cached(output, 0, 400, &[PlotValueIndex::Close])
            .unwrap();
        assert_eq!(min_max.max, 123.0);
        assert_plot_lod_matches_fresh(&dl, output);

        // Switching the source back to the union re-aliases the output onto its own times.
        assert!(dl.set_time_alignment(overlay, TimeAlignment::Union));
        assert!(!dl.plot(output).is_as_of());
        assert_eq!(dl.plot(output).size(), values.len());
        assert_plot_lod_matches_fresh(&dl, output);
    }

    #[test]
    fn as_of_point_colors_and_batches_read_the_row_each_point_shows() {
        let mut dl = DataLayer::new();
        let main = dl.add_series();
        let overlay = dl.add_series();
        set(&mut dl, main, &[10, 20, 30, 40], &[1.0; 4]);
        set(&mut dl, overlay, &[5, 25], &[50.0, 250.0]);
        assert!(dl.set_point_colors(overlay, [Some(vec![7, 9]), None, None]));
        assert!(dl.set_time_alignment(overlay, AS_OF));
        let colors = dl.point_colors(overlay).unwrap();
        assert_eq!(
            (0..4)
                .map(|row| colors.color(PointColorChannel::Body, row))
                .collect::<Vec<_>>(),
            vec![Some(7), Some(7), Some(9), Some(9)]
        );
        // Canonical lookups keep canonical rows.
        assert_eq!(dl.point_color(overlay, PointColorChannel::Body, 1), Some(9));

        // An in-place batch correction plus a tail append.
        assert_eq!(
            dl.update_many_single(overlay, &[5, 35], &[55.0, 350.0]),
            Some(0)
        );
        assert_eq!(
            plot_points(&dl, overlay),
            vec![(0, 55.0), (1, 55.0), (2, 250.0), (3, 350.0)]
        );
        // A merging batch that inserts history.
        assert_eq!(
            dl.update_many_single(overlay, &[15, 36], &[150.0, 360.0]),
            Some(1)
        );
        assert_eq!(dl.merged_times(), &[10, 20, 30, 40]);
        assert_eq!(
            plot_points(&dl, overlay),
            vec![(0, 55.0), (1, 150.0), (2, 250.0), (3, 360.0)]
        );
        assert_plot_lod_matches_fresh(&dl, overlay);
    }

    #[test]
    fn as_of_respects_the_replay_cutoff_and_retention() {
        let mut dl = DataLayer::new();
        let main = dl.add_series();
        let overlay = dl.add_series();
        set(&mut dl, main, &[10, 20, 30, 40], &[1.0; 4]);
        set(&mut dl, overlay, &[15, 25, 35], &[1.5, 2.5, 3.5]);
        assert!(dl.set_time_alignment(overlay, AS_OF));
        assert!(dl.set_time_cutoff(Some(30)));
        assert_eq!(plot_points(&dl, overlay), vec![(1, 1.5), (2, 2.5)]);
        // A tip beyond the clock is retained but not shown.
        assert!(dl.update(overlay, 45, [4.5; 4]));
        assert_eq!(plot_points(&dl, overlay), vec![(1, 1.5), (2, 2.5)]);
        assert!(dl.set_time_cutoff(None));
        assert_eq!(
            plot_points(&dl, overlay),
            vec![(1, 1.5), (2, 2.5), (3, 3.5)]
        );
        assert_eq!(dl.trim_front(overlay, 2), Some(2));
        assert_eq!(plot_points(&dl, overlay), vec![(3, 3.5)]);
        assert!(dl.remove_series(overlay));
        assert_eq!(dl.memory_usage().rows, 4);
    }

    /// Deterministic xorshift for the randomized as-of differential test.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: u64) -> u64 {
            if n == 0 {
                0
            } else {
                self.next() % n
            }
        }

        fn chance(&mut self, percent: u64) -> bool {
            self.below(100) < percent
        }
    }

    fn same_value(left: f64, right: f64) -> bool {
        left.to_bits() == right.to_bits() || (left.is_nan() && right.is_nan())
    }

    /// `(logical index, canonical row)` of every plot row.
    fn plot_rows(dl: &DataLayer, id: SeriesId) -> Vec<(TimePointIndex, usize)> {
        let plot = dl.plot(id);
        (0..plot.size())
            .map(|row| (plot.index_at(row).unwrap(), plot.source_row(row)))
            .collect()
    }

    /// The as-of join recomputed from scratch from the visible canonical columns.
    fn reference_as_of_rows(
        dl: &DataLayer,
        union: &[SeriesId],
        overlay: SeriesId,
    ) -> Vec<(TimePointIndex, usize)> {
        let merged = dl.merged_times();
        let mut expected_union = union
            .iter()
            .flat_map(|&id| dl.series_data(id).unwrap().0.to_vec())
            .collect::<Vec<_>>();
        expected_union.sort_unstable();
        expected_union.dedup();
        assert_eq!(
            merged,
            expected_union.as_slice(),
            "as-of rows joined the union"
        );
        let through = union
            .iter()
            .filter_map(|&id| {
                let (times, columns) = dl.series_data(id).unwrap();
                (0..times.len())
                    .rev()
                    .find(|&row| columns.iter().any(|column| !column[row].is_nan()))
                    .map(|row| merged.binary_search(&times[row]).unwrap() + 1)
            })
            .max()
            .unwrap_or(0);
        let TimeAlignment::AsOf { max_staleness } = dl.time_alignment(overlay).unwrap() else {
            panic!("not as-of");
        };
        let (times, _) = dl.series_data(overlay).unwrap();
        merged[..through]
            .iter()
            .enumerate()
            .filter_map(|(index, &point)| {
                let row = times
                    .partition_point(|&time| time <= point)
                    .checked_sub(1)?;
                if max_staleness.is_some_and(|max| point - times[row] > max) {
                    return None;
                }
                Some((index as TimePointIndex, row))
            })
            .collect()
    }

    fn naive_min_max(values: impl Iterator<Item = f64>) -> Option<(f64, f64)> {
        values
            .filter(|value| !value.is_nan())
            .fold(None, |result, value| match result {
                None => Some((value, value)),
                Some((min, max)) => Some((value.min(min), value.max(max))),
            })
    }

    /// Every reader of an as-of plot (and of an output aliasing it) against the reference.
    fn check_as_of_readers(
        dl: &mut DataLayer,
        union: &[SeriesId],
        overlay: SeriesId,
        output: SeriesId,
        rng: &mut Rng,
        context: &str,
    ) {
        let overlay_as_of = dl.time_alignment(overlay).unwrap().is_as_of();
        let expected = if overlay_as_of {
            reference_as_of_rows(dl, union, overlay)
        } else {
            let merged = dl.merged_times();
            let (times, _) = dl.series_data(overlay).unwrap();
            times
                .iter()
                .enumerate()
                .map(|(row, time)| {
                    (
                        merged.binary_search(time).expect("union row has a point") as i64,
                        row,
                    )
                })
                .collect()
        };
        assert_eq!(dl.plot(overlay).is_as_of(), overlay_as_of, "{context}");
        // Only owned series carry an alignment; the owner count backs the union fast path.
        assert!(dl.live_slots.values().all(|&slot| {
            dl.series[slot].time_alias.is_none()
                || dl.series[slot].alignment == TimeAlignment::Union
        }));
        assert_eq!(
            dl.as_of_series,
            dl.live_slots
                .values()
                .filter(|&&slot| dl.series[slot].alignment.is_as_of())
                .count(),
            "{context}"
        );
        assert_eq!(plot_rows(dl, overlay), expected, "{context}: overlay rows");
        {
            let plot = dl.plot(overlay);
            let (_, columns) = dl.series_data(overlay).unwrap();
            for (row, &(_, source)) in expected.iter().enumerate() {
                for value in [
                    PlotValueIndex::Open,
                    PlotValueIndex::High,
                    PlotValueIndex::Low,
                    PlotValueIndex::Close,
                ] {
                    assert!(
                        same_value(plot.value_at(row, value), columns[value as usize][source]),
                        "{context}: overlay value"
                    );
                }
            }
            let len = plot.size();
            for end in [len, len / 2, len.saturating_sub(1)] {
                assert_eq!(
                    plot.last_non_whitespace_row_before(end),
                    (0..end).rev().find(|&row| !plot.is_whitespace_row(row)),
                    "{context}: predecessor"
                );
            }
        }
        if overlay_as_of {
            assert_plot_lod_matches_fresh(dl, overlay);
        }

        // The output shows `2 * close` of the overlay rows it aliases, at the same points.
        let slot = dl.series_slot(output).unwrap();
        let alias = dl.series[slot].time_alias.expect("output stays aliased");
        let out_len = dl.series[slot].values.len();
        let expected_output = expected
            .iter()
            .filter(|&&(_, source)| source >= alias.offset && source < alias.offset + out_len)
            .map(|&(index, source)| (index, source - alias.offset))
            .collect::<Vec<_>>();
        assert_eq!(
            dl.time_alignment(output),
            dl.time_alignment(overlay),
            "{context}"
        );
        assert_eq!(
            plot_rows(dl, output),
            expected_output,
            "{context}: output rows"
        );
        {
            let plot = dl.plot(output);
            let (_, columns) = dl.series_data(overlay).unwrap();
            for (row, &(_, source)) in expected_output.iter().enumerate() {
                assert!(
                    same_value(
                        plot.value_at(row, PlotValueIndex::Close),
                        columns[3][source + alias.offset] * 2.0
                    ),
                    "{context}: output value"
                );
            }
        }
        if overlay_as_of {
            assert_plot_lod_matches_fresh(dl, output);
        }

        // Autoscale chunks (cached across operations) against a fresh fold.
        let points = dl.merged_times().len() as u64 + 2;
        for _ in 0..3 {
            let from = rng.below(points) as i64 - 1;
            let to = from + rng.below(points) as i64;
            for (id, rows, offset, scale) in [
                (overlay, &expected, 0, 1.0),
                (output, &expected_output, alias.offset, 2.0),
            ] {
                let (_, columns) = dl.series_data(overlay).unwrap();
                let naive = naive_min_max(
                    rows.iter()
                        .filter(|(index, _)| (from..=to).contains(index))
                        .flat_map(|&(_, source)| {
                            let source = source + offset;
                            if id == overlay {
                                [columns[1][source], columns[2][source]]
                            } else {
                                [columns[3][source] * scale; 2]
                            }
                        }),
                );
                let actual = dl
                    .min_max_on_range_cached(
                        id,
                        from,
                        to,
                        &[PlotValueIndex::High, PlotValueIndex::Low],
                    )
                    .map(|min_max| (min_max.min, min_max.max));
                assert_eq!(
                    actual, naive,
                    "{context}: min/max of {id} over {from}..={to}"
                );
            }
        }
    }

    fn random_row(rng: &mut Rng, whitespace_percent: u64) -> [f64; 4] {
        if rng.chance(whitespace_percent) {
            return [f64::NAN; 4];
        }
        let low = rng.below(1_000) as f64 / 8.0;
        let high = low + rng.below(80) as f64 / 8.0;
        let open = low + rng.below(8) as f64 / 8.0;
        [open, high, low, low.max(high - 1.0)]
    }

    fn random_time(rng: &mut Rng, dl: &DataLayer, id: SeriesId) -> i64 {
        let times = &dl.series[dl.series_slot(id).unwrap()].times;
        let last = times.last().copied().unwrap_or(0);
        match rng.below(6) {
            0 | 1 => last + 1 + rng.below(9) as i64,
            2 => last,
            3 if !times.is_empty() => times[rng.below(times.len() as u64) as usize],
            _ => rng.below(last.max(1) as u64 + 5) as i64,
        }
    }

    fn random_batch(rng: &mut Rng, dl: &DataLayer, id: SeriesId) -> Vec<i64> {
        let mut times = (0..1 + rng.below(6))
            .map(|_| random_time(rng, dl, id))
            .collect::<Vec<_>>();
        times.sort_unstable();
        times.dedup();
        times
    }

    #[test]
    fn as_of_incremental_maintenance_matches_a_fresh_join_under_random_mutations() {
        for seed in 1..=80u64 {
            let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed.wrapping_mul(0x2545_F491_4F6C_DD1D));
            let mut dl = DataLayer::new();
            let main = dl.add_series();
            let volume = dl.add_series();
            let overlay = dl.add_series();
            let output = dl.add_series();
            let union = [main, volume];
            // Every fourth seed is large enough for two summary levels and many autoscale chunks.
            let scale: u64 = if seed % 4 == 0 { 6 } else { 1 };
            // Main: a session of 2-second slots, traded for the first part.
            let slots = (0..60 * scale as i64)
                .map(|slot| slot * 2)
                .collect::<Vec<_>>();
            let traded = rng.below(60 * scale);
            let rows = (0..60 * scale)
                .map(|slot| random_row(&mut rng, if slot < traded { 5 } else { 100 }))
                .collect::<Vec<_>>();
            let column = |value: usize| rows.iter().map(|row| row[value]).collect::<Vec<_>>();
            dl.set_data(main, slots, column(0), column(1), column(2), column(3));
            let volume_times = (0..15 * scale as i64)
                .map(|row| row * 3 + 1)
                .collect::<Vec<_>>();
            let volume_values = (0..15 * scale)
                .map(|_| random_row(&mut rng, 30)[3])
                .collect::<Vec<_>>();
            dl.set_single_data(volume, volume_times, volume_values);
            let overlay_times = (0..25 * scale as i64)
                .map(|row| row * 5 + rng.below(5) as i64)
                .collect::<Vec<_>>();
            let overlay_rows = (0..25 * scale)
                .map(|_| random_row(&mut rng, 10))
                .collect::<Vec<_>>();
            let column = |value: usize| overlay_rows.iter().map(|row| row[value]).collect();
            dl.set_data(
                overlay,
                overlay_times,
                column(0),
                column(1),
                column(2),
                column(3),
            );
            let alignments = [
                TimeAlignment::AsOf {
                    max_staleness: None,
                },
                TimeAlignment::AsOf {
                    max_staleness: Some(4),
                },
                TimeAlignment::AsOf {
                    max_staleness: Some(0),
                },
                TimeAlignment::Union,
            ];
            dl.set_time_alignment(overlay, alignments[rng.below(3) as usize]);

            // An indicator-like output: `2 * close` from a warm-up row on, recomputed the way the
            // engine does after every source change (full after trims and replacements).
            let install_output = |dl: &mut DataLayer| {
                let (_, columns) = dl.series_data(overlay).unwrap();
                let from = columns[3].len().min(3);
                let values = columns[3][from..].iter().map(|close| close * 2.0).collect();
                assert!(dl.set_single_data_aligned(output, overlay, from, values));
            };
            install_output(&mut dl);
            dl.take_realigned();

            for step in 0..120 {
                let before_rows = plot_rows(&dl, overlay);
                let before_generation = dl.time_points_generation();
                let before_overlay = dl.series_data(overlay).unwrap().0.to_vec();
                let before_close = dl.series_data(overlay).unwrap().1[3].to_vec();
                let op = rng.below(14);
                let target = [main, volume, overlay][rng.below(3) as usize];
                let mut full = false;
                let description = match op {
                    0..=4 => {
                        let time = random_time(&mut rng, &dl, target);
                        let row = random_row(&mut rng, if target == main { 20 } else { 10 });
                        dl.update(target, time, row);
                        format!("update {target} at {time}")
                    }
                    5 | 6 => {
                        let times = random_batch(&mut rng, &dl, target);
                        let rows = times
                            .iter()
                            .map(|_| random_row(&mut rng, 10))
                            .collect::<Vec<_>>();
                        let column =
                            |value: usize| rows.iter().map(|row| row[value]).collect::<Vec<_>>();
                        if target == volume {
                            dl.update_many_single(volume, &times, &column(3));
                        } else {
                            dl.update_many(
                                target,
                                &times,
                                [&column(0), &column(1), &column(2), &column(3)],
                            );
                        }
                        format!("batch {target} at {times:?}")
                    }
                    7 => {
                        dl.pop(target, 1 + rng.below(3) as usize);
                        full = true;
                        format!("pop {target}")
                    }
                    8 => {
                        let len = dl.series_data(target).unwrap().0.len();
                        let keep = len.saturating_sub(1 + rng.below(3) as usize);
                        if rng.chance(40) {
                            // One retention over two distinct series, in one transaction.
                            let others = [main, volume, overlay]
                                .into_iter()
                                .filter(|&id| id != target)
                                .collect::<Vec<_>>();
                            let other = others[rng.below(2) as usize];
                            let other_len = dl.series_data(other).unwrap().0.len();
                            let other_keep = other_len.saturating_sub(1 + rng.below(3) as usize);
                            dl.trim_fronts(&[(target, keep), (other, other_keep)]);
                            full = true;
                            format!("trim batch {target} and {other}")
                        } else {
                            dl.trim_front(target, keep);
                            full = true;
                            format!("trim {target}")
                        }
                    }
                    9 => {
                        let cutoff = (!rng.chance(40)).then(|| rng.below(160) as i64);
                        dl.set_time_cutoff(cutoff);
                        full = true;
                        format!("cutoff {cutoff:?}")
                    }
                    10 => {
                        let alignment = alignments[rng.below(4) as usize];
                        dl.set_time_alignment(overlay, alignment);
                        format!("align {alignment:?}")
                    }
                    11 => {
                        let len = 1 + rng.below(30 * scale) as i64;
                        let start = rng.below(40) as i64;
                        let times = (0..len).map(|row| start + row * 4).collect::<Vec<_>>();
                        let rows = times
                            .iter()
                            .map(|_| random_row(&mut rng, 15))
                            .collect::<Vec<_>>();
                        let column =
                            |value: usize| rows.iter().map(|row| row[value]).collect::<Vec<_>>();
                        if target == volume || (target == overlay && rng.chance(50)) {
                            dl.set_single_data(target, times, column(3));
                        } else {
                            dl.set_data(target, times, column(0), column(1), column(2), column(3));
                        }
                        full = true;
                        format!("set {target}")
                    }
                    12 => {
                        // A direct write materializes the output; the recompute re-aliases it.
                        dl.pop(output, 0);
                        full = true;
                        "materialize output".to_string()
                    }
                    _ => {
                        let len = dl.series_data(target).unwrap().0.len();
                        if len > 0 {
                            let time = dl.series_data(target).unwrap().0[len - 1];
                            dl.update(target, time, [f64::NAN; 4]);
                        }
                        format!("whitespace tip {target}")
                    }
                };
                let context = format!("seed {seed} step {step}: {description}");

                // Recompute the output like the engine: full after a replacement, a trim, or a
                // moved clock; otherwise from the first overlay row that changed.
                let (times, columns) = dl.series_data(overlay).unwrap();
                let changed = before_overlay
                    .iter()
                    .zip(times)
                    .zip(before_close.iter().zip(columns[3]))
                    .position(|((old_time, new_time), (old, new))| {
                        old_time != new_time || !same_value(*old, *new)
                    })
                    .unwrap_or(before_overlay.len().min(times.len()));
                let front_moved = before_overlay.first() != times.first();
                if full || front_moved || rng.chance(15) {
                    install_output(&mut dl);
                } else {
                    let alias = dl.series[dl.series_slot(output).unwrap()]
                        .time_alias
                        .unwrap();
                    let out_len = dl.series[dl.series_slot(output).unwrap()].values.len();
                    let from = changed.max(alias.offset).min(alias.offset + out_len);
                    let from = from.min(times.len());
                    let values = columns[3][from..]
                        .iter()
                        .map(|close| close * 2.0)
                        .collect::<Vec<_>>();
                    assert!(
                        dl.update_single_aligned(output, overlay, from, &values)
                            .is_some(),
                        "{context}: incremental output"
                    );
                }

                // A mapping moved by another series without a time-point change must be reported,
                // or the engine would keep presenting the overlay's stale retained layer.
                let realigned = dl.take_realigned();
                if target != overlay
                    && op != 10
                    && dl.time_points_generation() == before_generation
                    && plot_rows(&dl, overlay) != before_rows
                {
                    assert!(
                        realigned.contains(&overlay),
                        "{context}: overlay not realigned"
                    );
                }
                check_as_of_readers(&mut dl, &union, overlay, output, &mut rng, &context);
            }
        }
    }

    #[test]
    fn a_materialized_as_of_output_realiased_onto_its_source_follows_the_source_again() {
        let mut dl = DataLayer::new();
        let main = dl.add_series();
        let overlay = dl.add_series();
        let output = dl.add_series();
        let main_times = (0..200).collect::<Vec<_>>();
        set(&mut dl, main, &main_times, &vec![1.0; 200]);
        let overlay_times = (0..100).map(|row| row * 2 + 1).collect::<Vec<_>>();
        let overlay_values = (0..100).map(f64::from).collect::<Vec<_>>();
        set(&mut dl, overlay, &overlay_times, &overlay_values);
        assert!(dl.set_time_alignment(overlay, AS_OF));
        assert!(dl.set_single_data_aligned(output, overlay, 0, overlay_values.clone()));
        // A direct write materializes the output as its own as-of series.
        assert!(dl.update(output, 199, [7.0; 4]));
        assert_eq!(dl.time_alignment(output), Some(AS_OF));
        // Re-aliasing hands it back to its source's alignment, including after a switch.
        assert!(dl.set_single_data_aligned(output, overlay, 0, overlay_values.clone()));
        assert!(dl.set_time_alignment(overlay, TimeAlignment::Union));
        assert_eq!(dl.time_alignment(output), Some(TimeAlignment::Union));
        // A recompute from row 50, inside a complete summary node.
        let mut values = overlay_values;
        values[50] = -5.0;
        assert_eq!(
            dl.update_single_aligned(output, overlay, 50, &values[50..]),
            Some(50)
        );
        assert_lod_matches_fresh(&dl, output);
        assert_eq!(
            dl.plot(output)
                .lod()
                .unwrap()
                .rows_on_range(0..100, 0)
                .0
                .iter()
                .map(|row| dl.plot(output).value_at(row, PlotValueIndex::Low))
                .fold(f64::INFINITY, f64::min),
            -5.0
        );
        // The count of as-of owners returns to zero once nothing is as-of.
        assert!(dl.remove_series(output));
        assert_eq!(dl.as_of_series, 0);
    }
}
