//! Viewport-bounded data conflation: when bar spacing drops below one device pixel, rows that
//! share an x pixel are reduced to endpoint/extrema representatives (per series kind).

use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DensityWork {
    pub(crate) selected_level: usize,
    pub(crate) summary_nodes: usize,
    pub(crate) raw_rows: usize,
    pub(crate) candidates: usize,
}

fn density_rows(
    plot: PlotListView<'_>,
    from: i64,
    to: i64,
    bar_spacing: f64,
    hpr: f64,
    x_at: &impl Fn(i64) -> f64,
    use_lod: bool,
) -> (Vec<usize>, DensityWork) {
    let spacing = bar_spacing * hpr;
    let raw = || {
        let range = plot.visible_rows(from, to);
        let raw_rows = range.len();
        let rows = range
            .filter(|&row| !plot.is_whitespace_row(row))
            .collect::<Vec<_>>();
        let candidates = rows.len();
        (
            rows,
            DensityWork {
                raw_rows,
                candidates,
                ..DensityWork::default()
            },
        )
    };
    if !use_lod || spacing >= 1.0 || !spacing.is_finite() || spacing <= 0.0 {
        return raw();
    }
    let Some(lod) = plot.lod() else {
        return raw();
    };
    let level = lod.selected_level(1.0 / spacing);
    if level == 0 {
        return raw();
    }

    let x_zero = x_at(0);
    let first_bucket = x_at(from).floor() as i64;
    let last_bucket = x_at(to).floor() as i64;
    let mut output = Vec::with_capacity((last_bucket - first_bucket + 1).max(0) as usize * 6);
    let mut work = DensityWork {
        selected_level: level,
        ..DensityWork::default()
    };
    for bucket in first_bucket..=last_bucket {
        let mut logical_start = (((bucket as f64 - x_zero) / spacing).ceil() as i64).max(from);
        while logical_start <= to && (x_at(logical_start).floor() as i64) < bucket {
            logical_start += 1;
        }
        while logical_start > from && (x_at(logical_start - 1).floor() as i64) >= bucket {
            logical_start -= 1;
        }
        let mut logical_end =
            ((((bucket + 1) as f64 - x_zero) / spacing).ceil() as i64 - 1).min(to);
        while logical_end >= logical_start && (x_at(logical_end).floor() as i64) > bucket {
            logical_end -= 1;
        }
        while logical_end < to && (x_at(logical_end + 1).floor() as i64) <= bucket {
            logical_end += 1;
        }
        if logical_start > logical_end {
            continue;
        }
        let range = plot.visible_rows(logical_start, logical_end);
        let (rows, stats) = lod.rows_on_range(range, level);
        work.summary_nodes += stats.summary_nodes;
        work.raw_rows += stats.raw_rows;
        output.extend(rows.iter());
    }
    work.candidates = output.len();
    (output, work)
}

/// Pick a bounded set of rows when several source points occupy the same physical x pixel.
///
/// The normal-spacing path remains unchanged. Once the source spacing drops below one physical
/// pixel, each bucket keeps its first/last rows plus the close extrema, preserving the visible
/// envelope and the line's endpoints while avoiding an O(number-of-source-points) draw list.
pub(crate) fn visible_line_rows_with_work(
    plot: PlotListView<'_>,
    from: i64,
    to: i64,
    bar_spacing: f64,
    hpr: f64,
    x_at: impl Fn(i64) -> f64,
    work: &mut DensityWork,
) -> Vec<usize> {
    visible_line_rows_policy(plot, from, to, bar_spacing, hpr, x_at, work, true)
}

#[allow(clippy::too_many_arguments)] // production arguments plus the test-only raw/LOD policy seam
fn visible_line_rows_policy(
    plot: PlotListView<'_>,
    from: i64,
    to: i64,
    bar_spacing: f64,
    hpr: f64,
    x_at: impl Fn(i64) -> f64,
    work: &mut DensityWork,
    use_lod: bool,
) -> Vec<usize> {
    // Whitespace rows (reference `{time}`-only items) draw nothing: dropping them here leaves the
    // surrounding real bars adjacent in the result, so the line connects across the gap
    // exactly like the reference's whitespace-free plot list.
    let (visible, measured) = density_rows(plot, from, to, bar_spacing, hpr, &x_at, use_lod);
    *work = measured;
    let rows = if bar_spacing * hpr >= 1.0 {
        visible
    } else {
        conflate_line_rows(plot, visible, &x_at)
    };
    with_edge_neighbours(plot, from, to, rows)
}

/// Add the nearest real row beyond each visible edge (reference `visibleTimedValues(..., extendedRange
/// = true)`). A path must keep running to the pane edge while its next point is off-screen; without
/// the neighbours the edge segment vanishes and pops back when that point scrolls into view. The
/// pane clip trims the overhang.
fn with_edge_neighbours(
    plot: PlotListView<'_>,
    from: i64,
    to: i64,
    mut rows: Vec<usize>,
) -> Vec<usize> {
    let range = plot.visible_rows(from, to);
    let before = plot.last_non_whitespace_row_before(range.start);
    let after = plot
        .index_at(range.end)
        .and_then(|index| plot.first_non_whitespace_row(index));
    if let Some(row) = before.filter(|row| rows.first() != Some(row)) {
        rows.insert(0, row);
    }
    if let Some(row) = after.filter(|row| rows.last() != Some(row)) {
        rows.push(row);
    }
    rows
}

/// Split `0..len` into consecutive line runs at `breaks` (ascending positions in `1..len`, each
/// the first element of a new run). No breaks yields the single run `0..len`.
pub(crate) fn line_runs(
    len: usize,
    breaks: &[usize],
) -> impl Iterator<Item = std::ops::Range<usize>> + '_ {
    let starts = std::iter::once(0).chain(breaks.iter().copied());
    let ends = breaks.iter().copied().chain(std::iter::once(len));
    starts
        .zip(ends)
        .map(|(start, end)| start..end)
        .filter(|run| !run.is_empty())
}

/// Sub-pixel conflation: keep first, low, high, and last per device-pixel column.
fn conflate_line_rows(
    plot: PlotListView<'_>,
    visible: Vec<usize>,
    x_at: &impl Fn(i64) -> f64,
) -> Vec<usize> {
    let close = |row: usize| plot.value_at(row, PlotValueIndex::Close);
    let mut out = Vec::new();
    let mut bucket_rows = Vec::new();
    let mut bucket: Option<i64> = None;

    let flush = |bucket_rows: &mut Vec<usize>, out: &mut Vec<usize>| {
        let (Some(&first), Some(&last)) = (bucket_rows.first(), bucket_rows.last()) else {
            return;
        };
        let mut low = first;
        let mut high = first;
        for &row in bucket_rows.iter().skip(1) {
            if close(row).is_finite() && (!close(low).is_finite() || close(row) < close(low)) {
                low = row;
            }
            if close(row).is_finite() && (!close(high).is_finite() || close(row) > close(high)) {
                high = row;
            }
        }
        let mut selected = [first, low, high, last];
        selected.sort_unstable();
        for row in selected {
            if out.last().copied() != Some(row) {
                out.push(row);
            }
        }
        bucket_rows.clear();
    };

    for row in visible {
        let current_bucket = x_at(plot.index_at(row).expect("visible row index")).floor() as i64;
        if bucket.is_some_and(|previous| previous != current_bucket) {
            flush(&mut bucket_rows, &mut out);
        }
        bucket = Some(current_bucket);
        bucket_rows.push(row);
    }
    flush(&mut bucket_rows, &mut out);
    out
}

#[cfg(test)]
pub(crate) fn visible_line_rows_raw_reference(
    plot: PlotListView<'_>,
    from: i64,
    to: i64,
    bar_spacing: f64,
    hpr: f64,
    x_at: impl Fn(i64) -> f64,
) -> Vec<usize> {
    visible_line_rows_policy(
        plot,
        from,
        to,
        bar_spacing,
        hpr,
        x_at,
        &mut DensityWork::default(),
        false,
    )
}

pub(crate) fn visible_line_rows(
    plot: PlotListView<'_>,
    from: i64,
    to: i64,
    bar_spacing: f64,
    hpr: f64,
    x_at: impl Fn(i64) -> f64,
) -> Vec<usize> {
    visible_line_rows_with_work(
        plot,
        from,
        to,
        bar_spacing,
        hpr,
        x_at,
        &mut DensityWork::default(),
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct VisibleOhlc {
    /// Physical-pixel x coordinate. Aggregated buckets are pinned to their integer pixel so
    /// adjacent buckets cannot round back onto the same column in the geometry builders.
    pub(crate) x_px: f64,
    pub(crate) open: f64,
    pub(crate) high: f64,
    pub(crate) low: f64,
    pub(crate) close: f64,
    /// Source row supplying the bar's identity fields (open/high/low/close + per-point colors):
    /// the row itself at normal spacing, the bucket's last row (which owns the close) when
    /// compressed.
    pub(crate) source_row: usize,
    /// Geometry-local adjacency key (the reference's conflated-item `time` in the range hit test): the
    /// actual time-point index at normal spacing, the physical pixel bucket when compressed.
    pub(crate) geometry_time: i64,
}

/// Aggregate source OHLC rows that share a physical x pixel.
///
/// Each compressed bucket is itself a valid OHLC bar: first open, maximum high, minimum low, and
/// last close. At normal spacing this is an identity transform, apart from copying the visible
/// values into the small frame-local item list required by the render geometry builders.
pub(crate) fn visible_ohlc_with_work(
    plot: PlotListView<'_>,
    from: i64,
    to: i64,
    bar_spacing: f64,
    hpr: f64,
    x_at: impl Fn(i64) -> f64,
    work: &mut DensityWork,
) -> Vec<VisibleOhlc> {
    visible_ohlc_with_values(plot, from, to, bar_spacing, hpr, x_at, work, true, |row| {
        Some([
            plot.value_at(row, PlotValueIndex::Open),
            plot.value_at(row, PlotValueIndex::High),
            plot.value_at(row, PlotValueIndex::Low),
            plot.value_at(row, PlotValueIndex::Close),
        ])
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn visible_ohlc_with_values(
    plot: PlotListView<'_>,
    from: i64,
    to: i64,
    bar_spacing: f64,
    hpr: f64,
    x_at: impl Fn(i64) -> f64,
    work: &mut DensityWork,
    use_lod: bool,
    value_at: impl Fn(usize) -> Option<[f64; 4]>,
) -> Vec<VisibleOhlc> {
    let (visible, measured) = density_rows(plot, from, to, bar_spacing, hpr, &x_at, use_lod);
    *work = measured;
    if bar_spacing * hpr >= 1.0 {
        return visible
            .into_iter()
            .filter_map(|row| {
                let values = value_at(row)?;
                Some(VisibleOhlc {
                    x_px: x_at(plot.index_at(row).expect("visible row index")),
                    open: values[0],
                    high: values[1],
                    low: values[2],
                    close: values[3],
                    source_row: row,
                    geometry_time: plot.index_at(row).expect("visible row index"),
                })
            })
            .collect();
    }
    let mut out = Vec::new();
    let mut current_bucket: Option<i64> = None;
    let mut current: Option<VisibleOhlc> = None;
    for row in visible {
        let Some(values) = value_at(row) else {
            continue;
        };
        let bucket = x_at(plot.index_at(row).expect("visible row index")).floor() as i64;
        if current_bucket.is_some_and(|previous| previous != bucket)
            && let Some(item) = current.take()
        {
            out.push(item);
        }
        match current.as_mut() {
            Some(item) => {
                item.high = item.high.max(values[1]);
                item.low = item.low.min(values[2]);
                item.close = values[3];
                item.source_row = row;
            }
            None => {
                current = Some(VisibleOhlc {
                    x_px: bucket as f64,
                    open: values[0],
                    high: values[1],
                    low: values[2],
                    close: values[3],
                    source_row: row,
                    geometry_time: bucket,
                });
            }
        }
        current_bucket = Some(bucket);
    }
    if let Some(item) = current {
        out.push(item);
    }
    out
}

#[allow(clippy::too_many_arguments)] // production arguments plus the test-only raw/LOD policy seam
#[cfg(test)]
fn visible_ohlc_policy(
    plot: PlotListView<'_>,
    from: i64,
    to: i64,
    bar_spacing: f64,
    hpr: f64,
    x_at: impl Fn(i64) -> f64,
    work: &mut DensityWork,
    use_lod: bool,
) -> Vec<VisibleOhlc> {
    visible_ohlc_with_values(
        plot,
        from,
        to,
        bar_spacing,
        hpr,
        x_at,
        work,
        use_lod,
        |row| {
            Some([
                plot.value_at(row, PlotValueIndex::Open),
                plot.value_at(row, PlotValueIndex::High),
                plot.value_at(row, PlotValueIndex::Low),
                plot.value_at(row, PlotValueIndex::Close),
            ])
        },
    )
}

#[cfg(test)]
pub(crate) fn visible_ohlc_raw_reference(
    plot: PlotListView<'_>,
    from: i64,
    to: i64,
    bar_spacing: f64,
    hpr: f64,
    x_at: impl Fn(i64) -> f64,
) -> Vec<VisibleOhlc> {
    visible_ohlc_policy(
        plot,
        from,
        to,
        bar_spacing,
        hpr,
        x_at,
        &mut DensityWork::default(),
        false,
    )
}

#[cfg(test)]
pub(crate) fn visible_ohlc(
    plot: PlotListView<'_>,
    from: i64,
    to: i64,
    bar_spacing: f64,
    hpr: f64,
    x_at: impl Fn(i64) -> f64,
) -> Vec<VisibleOhlc> {
    visible_ohlc_with_work(
        plot,
        from,
        to,
        bar_spacing,
        hpr,
        x_at,
        &mut DensityWork::default(),
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct VisibleHistogramRow {
    pub(crate) x_px: f64,
    pub(crate) source_row: usize,
    /// Geometry-local adjacency key. It is the actual time-point index at normal spacing and the
    /// physical pixel bucket when compressed.
    pub(crate) geometry_time: i64,
}

/// Select one conservative histogram sample per physical pixel, retaining the value with the
/// greatest magnitude so a volume/value spike cannot disappear merely because the scale is
/// compressed. The selected source row also carries its source up/down color classification.
pub(crate) fn visible_histogram_rows_with_work(
    plot: PlotListView<'_>,
    from: i64,
    to: i64,
    bar_spacing: f64,
    hpr: f64,
    x_at: impl Fn(i64) -> f64,
    work: &mut DensityWork,
) -> Vec<VisibleHistogramRow> {
    visible_histogram_rows_policy(plot, from, to, bar_spacing, hpr, x_at, work, true)
}

#[allow(clippy::too_many_arguments)] // production arguments plus the test-only raw/LOD policy seam
fn visible_histogram_rows_policy(
    plot: PlotListView<'_>,
    from: i64,
    to: i64,
    bar_spacing: f64,
    hpr: f64,
    x_at: impl Fn(i64) -> f64,
    work: &mut DensityWork,
    use_lod: bool,
) -> Vec<VisibleHistogramRow> {
    let close = |row: usize| plot.value_at(row, PlotValueIndex::Close);
    // Whitespace rows draw nothing (the reference's plot list omits them).
    let (visible, measured) = density_rows(plot, from, to, bar_spacing, hpr, &x_at, use_lod);
    *work = measured;
    if bar_spacing * hpr >= 1.0 {
        return visible
            .into_iter()
            .map(|source_row| VisibleHistogramRow {
                x_px: x_at(plot.index_at(source_row).expect("visible row index")),
                source_row,
                geometry_time: plot.index_at(source_row).expect("visible row index"),
            })
            .collect();
    }

    let mut out: Vec<VisibleHistogramRow> = Vec::new();
    for source_row in visible {
        let bucket = x_at(plot.index_at(source_row).expect("visible row index")).floor() as i64;
        match out.last_mut() {
            Some(item) if item.geometry_time == bucket => {
                if close(source_row).abs() > close(item.source_row).abs() {
                    item.source_row = source_row;
                }
            }
            _ => out.push(VisibleHistogramRow {
                x_px: bucket as f64,
                source_row,
                geometry_time: bucket,
            }),
        }
    }
    out
}

#[cfg(test)]
pub(crate) fn visible_histogram_rows_raw_reference(
    plot: PlotListView<'_>,
    from: i64,
    to: i64,
    bar_spacing: f64,
    hpr: f64,
    x_at: impl Fn(i64) -> f64,
) -> Vec<VisibleHistogramRow> {
    visible_histogram_rows_policy(
        plot,
        from,
        to,
        bar_spacing,
        hpr,
        x_at,
        &mut DensityWork::default(),
        false,
    )
}

#[cfg(test)]
pub(crate) fn visible_histogram_rows(
    plot: PlotListView<'_>,
    from: i64,
    to: i64,
    bar_spacing: f64,
    hpr: f64,
    x_at: impl Fn(i64) -> f64,
) -> Vec<VisibleHistogramRow> {
    visible_histogram_rows_with_work(
        plot,
        from,
        to,
        bar_spacing,
        hpr,
        x_at,
        &mut DensityWork::default(),
    )
}
