//! Series hit testing (reference v5.2 `hitTestPane`/`hitTestSeriesRange`/`hitTestLineSeries` ports).
//!
//! Geometry is computed in media (CSS) px — x from the pane's left edge, y from the chart's
//! top — the same space the frame builders' coordinate converters produce, so a hit exactly
//! matches what was painted. Item selection reuses the frame builders' conflation helpers
//! (`visible_ohlc`/`visible_line_rows`/`visible_histogram_rows`), keeping the tested geometry
//! identical to the drawn geometry at any bar spacing.
//!
//! Per-kind rules (ports, with the reference default `hitTestTolerance` of 3 px):
//! - candlestick/bar: `hitTestSeriesRange` over the bar's full high–low span (reference
//!   bars-pane-view-base.ts uses `highY..lowY` — no body/wick distinction), inside the bar's
//!   horizontal slot (midpoints between adjacent bars, `barSpacing/2` at the edges).
//! - histogram: the same range test over `value..base`.
//! - line/area/baseline: `hitTestLineSeries` — distance to the stroke polyline (or the
//!   step/curve expansion) within `lineWidth/2 + tolerance` (width 1 when `lineVisible` is
//!   off), point-marker discs within `radius + tolerance`, and the reference's single-visible-point
//!   horizontal segment. The area/baseline fills carry no hit test, exactly like reference
//!   (no renderer implements `hitTest` in v5.2).
//!
//! Cross-series arbitration follows behavior learned from public hit-test semantics: point-style hits beat strokes/ranges,
//! otherwise the smaller distance wins, and equal-distance non-point ties keep the paint
//! order (the caller walks topmost-first, so ties resolve to the topmost series).

use aeris_charts_core::model::data_layer::SeriesId;
use aeris_charts_core::model::plot_list::PlotValueIndex;
use aeris_charts_render::draw_list::LineType;

use crate::feature_series::FeatureSeriesKind;
use crate::frame::{pane_scale, series_scale_target};
use crate::{
    ChartEngine, MAX_SELECTION_ANCHORS, MAX_SELECTION_MEMBERS, SELECTION_ANCHOR_SPACING_CSS,
    SelectionAnchorMemberSnapshot, SelectionAnchorSnapshot, SeriesKind,
};

/// reference `SeriesOptionsCommon.hitTestTolerance` default (series-options-defaults.ts:15).
const HIT_TEST_TOLERANCE: f64 = 3.0;

/// reference `HitTestPriority` (model/internal-hit-test.ts): the class a series hit belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeriesHitKind {
    /// Range-style hit (bar/candle/histogram interval).
    Range,
    /// Stroke-style hit (line segment).
    Line,
    /// Point-style hit (point marker or a single visible point).
    Point,
}

/// One series' hit-test outcome (reference `InternalHitTestCandidate` plus the series identity).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeriesHit {
    pub series: SeriesId,
    /// Geometric distance from the cursor to the series geometry, in CSS px.
    pub distance: f64,
    pub kind: SeriesHitKind,
}

impl SeriesHit {
    /// Port of reference `isBetterHit`: a point hit beats any non-point hit; otherwise the smaller
    /// distance wins and equal-distance non-point ties lose (preserving the caller's
    /// topmost-first walk order).
    pub fn is_better_than(&self, current: &SeriesHit) -> bool {
        if self.kind == SeriesHitKind::Point && current.kind != SeriesHitKind::Point {
            return true;
        }
        if current.kind == SeriesHitKind::Point && self.kind != SeriesHitKind::Point {
            return false;
        }
        self.distance < current.distance
    }
}

/// reference `distanceToSegment` (renderers/hit-test-common.ts).
fn distance_to_segment(x: f64, y: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let dx = x2 - x1;
    let dy = y2 - y1;
    if dx == 0.0 && dy == 0.0 {
        return (x - x1).hypot(y - y1);
    }
    let projection = ((x - x1) * dx + (y - y1) * dy) / (dx * dx + dy * dy);
    let clamped = projection.clamp(0.0, 1.0);
    (x - (x1 + dx * clamped)).hypot(y - (y1 + dy * clamped))
}

/// reference `hitTestSeriesRange` (renderers/range-hit-test.ts): horizontal slots bounded by the
/// midpoints between adjacent items (`barSpacing/2` at the data edges), vertical range from
/// the item's `rangeProvider`; inside the range the distance is 0, within tolerance outside
/// it the distance to the nearer edge. `items` is `(x, geometry_time, range_start_y,
/// range_end_y)` per visible item, x ascending.
fn hit_test_series_range(
    items: &[(f64, i64, f64, f64)],
    x: f64,
    y: f64,
    bar_spacing: f64,
    tolerance: f64,
) -> Option<f64> {
    if items.is_empty() {
        return None;
    }
    let horizontal_radius = bar_spacing / 2.0 + tolerance;
    // lowerBoundByX/upperBoundByX over the x-ascending items.
    let candidate_from = items.partition_point(|item| item.0 < x - horizontal_radius);
    let candidate_to = items.partition_point(|item| item.0 <= x + horizontal_radius);
    if candidate_from >= candidate_to {
        return None;
    }
    let mut min_distance = f64::INFINITY;
    for index in candidate_from..candidate_to {
        let item = items[index];
        // NOTE: `then` (lazy), not `then_some` — the argument is eagerly evaluated there and
        // `items[index ± 1]` would panic at the edges of the visible set.
        let previous = (index > 0).then(|| items[index - 1]);
        let next = (index + 1 < items.len()).then(|| items[index + 1]);
        // slotStart/slotEnd: adjacent items (consecutive geometry keys) share their midpoint.
        let slot_start = match previous {
            Some(p) if p.1 == item.1 - 1 => (p.0 + item.0) / 2.0,
            _ => item.0 - bar_spacing / 2.0,
        } - tolerance;
        let slot_end = match next {
            Some(n) if n.1 == item.1 + 1 => (item.0 + n.0) / 2.0,
            _ => item.0 + bar_spacing / 2.0,
        } + tolerance;
        if x < slot_start || x > slot_end {
            continue;
        }
        let actual_top = item.2.min(item.3);
        let actual_bottom = item.2.max(item.3);
        if y >= actual_top && y <= actual_bottom {
            min_distance = min_distance.min(0.0);
            continue;
        }
        if y >= actual_top - tolerance && y <= actual_bottom + tolerance {
            min_distance = min_distance.min((y - actual_top).abs().min((actual_bottom - y).abs()));
        }
    }
    min_distance.is_finite().then_some(min_distance)
}

/// reference `getControlPoints` (renderers/walk-line.ts), tension 6.
fn control_points(points: &[(f64, f64)], from: usize, to: usize) -> [(f64, f64); 2] {
    const CURVE_TENSION: f64 = 6.0;
    let before_from = points[from.saturating_sub(1)];
    let after_to = points[(to + 1).min(points.len() - 1)];
    [
        (
            points[from].0 + (points[to].0 - before_from.0) / CURVE_TENSION,
            points[from].1 + (points[to].1 - before_from.1) / CURVE_TENSION,
        ),
        (
            points[to].0 - (after_to.0 - points[from].0) / CURVE_TENSION,
            points[to].1 - (after_to.1 - points[from].1) / CURVE_TENSION,
        ),
    ]
}

/// reference `distanceToBezierCurve` (renderers/line-hit-test.ts): 12-step polyline approximation.
fn distance_to_bezier_curve(x: f64, y: f64, points: [(f64, f64); 4]) -> f64 {
    const STEPS: usize = 12;
    let cubic = |p0: f64, p1: f64, p2: f64, p3: f64, t: f64| {
        let u = 1.0 - t;
        u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3
    };
    let mut min_distance = f64::INFINITY;
    let mut previous = points[0];
    for step in 1..=STEPS {
        let t = step as f64 / STEPS as f64;
        let current = (
            cubic(points[0].0, points[1].0, points[2].0, points[3].0, t),
            cubic(points[0].1, points[1].1, points[2].1, points[3].1, t),
        );
        min_distance = min_distance.min(distance_to_segment(
            x, y, previous.0, previous.1, current.0, current.1,
        ));
        previous = current;
    }
    min_distance
}

/// reference `hitTestLineSeries` (renderers/line-hit-test.ts): point markers and the
/// single-visible-point segment report `Point`, stroke segments report `Line`. Aeris's
/// `visible_line_rows` selection already produces the drawn item set, so the items here are
/// exactly the visible points (reference instead indexes into its full item list by the extended
/// visible range — same set at the pane edges Aeris draws). Shared with the brush drawing tool's
/// body test (drawings.rs), whose smooth stroke is the same curved geometry.
#[allow(clippy::too_many_arguments)] // mirrors the reference renderer-data signature
pub(crate) fn hit_test_line_series(
    points: &[(f64, f64)],
    x: f64,
    y: f64,
    line_type: LineType,
    line_width: f64,
    point_markers_radius: Option<f64>,
    bar_spacing: f64,
    tolerance: f64,
) -> Option<(f64, SeriesHitKind)> {
    if points.is_empty() {
        return None;
    }
    let radius = (line_width / 2.0).max(point_markers_radius.unwrap_or(0.0)) + tolerance;
    let mut point_min_distance = f64::INFINITY;

    if let Some(markers_radius) = point_markers_radius {
        let point_radius = markers_radius + tolerance;
        for &(px, py) in points {
            // isWithinHorizontalSweep
            if x < px - point_radius || x > px + point_radius {
                continue;
            }
            let distance = (x - px).hypot(y - py);
            if distance <= point_radius {
                point_min_distance = point_min_distance.min(distance);
            }
        }
    }

    if points.len() < 2 {
        let (px, py) = points[0];
        let single_half_width = (bar_spacing / 2.0).max(radius);
        let distance =
            distance_to_segment(x, y, px - single_half_width, py, px + single_half_width, py);
        if distance <= radius {
            point_min_distance = point_min_distance.min(distance);
        }
        return point_min_distance
            .is_finite()
            .then_some((point_min_distance, SeriesHitKind::Point));
    }

    let mut line_min_distance = f64::INFINITY;
    for index in 1..points.len() {
        let first = points[index - 1];
        let second = points[index];
        // isWithinHorizontalSweep against the segment's horizontal bounds (with the control
        // points included for curves, as in reference).
        let (left, right) = match line_type {
            LineType::Curved => {
                let [cp1, cp2] = control_points(points, index - 1, index);
                (
                    first.0.min(second.0).min(cp1.0).min(cp2.0),
                    first.0.max(second.0).max(cp1.0).max(cp2.0),
                )
            }
            _ => (first.0.min(second.0), first.0.max(second.0)),
        };
        if x < left - radius || x > right + radius {
            continue;
        }
        let distance = match line_type {
            LineType::WithSteps => distance_to_segment(x, y, first.0, first.1, second.0, first.1)
                .min(distance_to_segment(
                    x, y, second.0, first.1, second.0, second.1,
                )),
            LineType::Curved => {
                let [cp1, cp2] = control_points(points, index - 1, index);
                distance_to_bezier_curve(x, y, [first, cp1, cp2, second])
            }
            _ => distance_to_segment(x, y, first.0, first.1, second.0, second.1),
        };
        if distance <= radius {
            line_min_distance = line_min_distance.min(distance);
        }
    }

    if point_min_distance.is_finite() {
        return Some((point_min_distance, SeriesHitKind::Point));
    }
    line_min_distance
        .is_finite()
        .then_some((line_min_distance, SeriesHitKind::Line))
}

impl ChartEngine {
    /// The stacked pane containing chart-top-relative media y `y_css`, or `None` when it
    /// falls between panes, on the time-axis strip, or off the chart. Hit testing is
    /// restricted to the pane under the cursor (reference hit-tests only the hovered pane widget).
    pub fn pane_at_y(&self, y_css: f64) -> Option<usize> {
        if !y_css.is_finite() {
            return None;
        }
        self.panes
            .iter()
            .position(|pane| y_css >= pane.top && y_css <= pane.top + pane.height)
    }

    /// Hit-test one series at pane-relative media px `(x_css, y_css)` (x from the pane's left
    /// edge, y from the chart's top), independent of any other series. `None` for a hidden,
    /// removed, pane-less, or unscaled series, or a miss. Per-kind geometry and tolerances
    /// are the reference ports documented at the module level.
    pub fn hit_test_one_series(&self, id: SeriesId, x_css: f64, y_css: f64) -> Option<SeriesHit> {
        self.reset_lod_work();
        if !x_css.is_finite() || !y_css.is_finite() {
            return None;
        }
        let series = self.series_entry(id)?;
        // reference gates a series' pane-view hit test on visibility (series-pane-view-base.ts).
        if !series.visible || series.pane_index >= self.panes.len() {
            return None;
        }
        let (from, to) = self.visible_range_for_frame()?;
        let base_value = self.series_base_value(id, from)?;
        let scale = pane_scale(&self.panes[series.pane_index], series_scale_target(series));
        if scale.is_empty() {
            return None;
        }
        // The drawn geometry is what is hittable: an eased last bar tests its display values.
        let plot = self.display_plot(id);
        let bar_spacing = self.time_scale.bar_spacing();
        // The frame build's horizontal ratio (frame/mod.rs): conflation buckets in physical
        // pixels, so the tested items must be selected with the same ratio they were drawn.
        let hpr = (self.pane_w * self.dpr.max(0.01)).round().max(1.0) / self.pane_w.max(1.0);
        let result = match series.kind {
            SeriesKind::Candlestick | SeriesKind::Bar | SeriesKind::Footprint => {
                let footprint_aggregation = series
                    .footprint
                    .as_ref()
                    .and_then(|state| self.trade_stream(state.trade_stream_id))
                    .map(|stream| stream.options());
                let mut work = crate::frame::conflation::DensityWork::default();
                let visible = crate::frame::conflation::visible_ohlc_with_work(
                    plot,
                    from,
                    to,
                    bar_spacing,
                    hpr,
                    |index| self.time_scale.index_to_coordinate(index) * hpr,
                    &mut work,
                );
                self.record_lod_work(
                    work.selected_level,
                    work.summary_nodes,
                    work.raw_rows,
                    work.candidates,
                );
                let items = visible
                    .into_iter()
                    .map(|bar| {
                        let (low, high) =
                            footprint_aggregation.map_or((bar.low, bar.high), |aggregation| {
                                crate::footprint::footprint_row_price_bounds(
                                    &aggregation,
                                    bar.low,
                                    bar.high,
                                )
                            });
                        (
                            bar.x_px / hpr,
                            bar.geometry_time,
                            scale.price_to_coordinate(high, base_value),
                            scale.price_to_coordinate(low, base_value),
                        )
                    })
                    .collect::<Vec<_>>();
                hit_test_series_range(&items, x_css, y_css, bar_spacing, HIT_TEST_TOLERANCE)
                    .map(|distance| (distance, SeriesHitKind::Range))
            }
            SeriesKind::Histogram => {
                let base_y = scale.price_to_coordinate(series.base, base_value);
                let close = |row: usize| plot.value_at(row, PlotValueIndex::Close);
                let mut work = crate::frame::conflation::DensityWork::default();
                let visible = crate::frame::conflation::visible_histogram_rows_with_work(
                    plot,
                    from,
                    to,
                    bar_spacing,
                    hpr,
                    |index| self.time_scale.index_to_coordinate(index) * hpr,
                    &mut work,
                );
                self.record_lod_work(
                    work.selected_level,
                    work.summary_nodes,
                    work.raw_rows,
                    work.candidates,
                );
                let items = visible
                    .into_iter()
                    .map(|item| {
                        (
                            item.x_px / hpr,
                            item.geometry_time,
                            scale.price_to_coordinate(close(item.source_row), base_value),
                            base_y,
                        )
                    })
                    .collect::<Vec<_>>();
                hit_test_series_range(&items, x_css, y_css, bar_spacing, HIT_TEST_TOLERANCE)
                    .map(|distance| (distance, SeriesHitKind::Range))
            }
            SeriesKind::Line | SeriesKind::Area | SeriesKind::Baseline => {
                let close = |row: usize| plot.value_at(row, PlotValueIndex::Close);
                let mut work = crate::frame::conflation::DensityWork::default();
                let visible = crate::frame::conflation::visible_line_rows_with_work(
                    plot,
                    from,
                    to,
                    bar_spacing,
                    hpr,
                    |index| self.time_scale.index_to_coordinate(index) * hpr,
                    &mut work,
                );
                self.record_lod_work(
                    work.selected_level,
                    work.summary_nodes,
                    work.raw_rows,
                    work.candidates,
                );
                let points = visible
                    .iter()
                    .map(|&row| {
                        (
                            self.time_scale.index_to_coordinate(
                                plot.index_at(row).expect("hit-test row index"),
                            ),
                            scale.price_to_coordinate(close(row), base_value),
                        )
                    })
                    .collect::<Vec<_>>();
                // The frame's period breaks: each run is tested alone, so the connector the
                // frame omits between runs cannot be hit (a lone row keeps its one-bar segment).
                let runs = self.line_run_ranges(id, plot, &visible, from, to);
                // reference line-hit-test-pane-view-base.ts: width 1 when the stroke is hidden;
                // point markers join with their resolved radius (default lineWidth/2 + 2).
                let line_width = if series.line_visible {
                    series.line_width.unwrap_or(crate::frame::LINE_WIDTH)
                } else {
                    1.0
                };
                let markers_radius = series.point_markers.then(|| {
                    series
                        .point_markers_radius
                        .unwrap_or(line_width / 2.0 + 2.0)
                });
                runs.into_iter()
                    .filter_map(|run| {
                        hit_test_line_series(
                            &points[run],
                            x_css,
                            y_css,
                            series.line_type,
                            line_width,
                            markers_radius,
                            bar_spacing,
                            HIT_TEST_TOLERANCE,
                        )
                    })
                    .reduce(|best, hit| {
                        let better = SeriesHit {
                            series: id,
                            distance: hit.0,
                            kind: hit.1,
                        }
                        .is_better_than(&SeriesHit {
                            series: id,
                            distance: best.0,
                            kind: best.1,
                        });
                        if better { hit } else { best }
                    })
            }
            SeriesKind::Feature => {
                let feature = series.feature.as_ref()?;
                match feature.kind {
                    FeatureSeriesKind::BackgroundShade => None,
                    _ => {
                        let items = plot
                            .visible_rows(from, to)
                            .filter_map(|row| {
                                let projection =
                                    feature.rows.get(row)?.value.as_ref()?.projection();
                                let logical = plot.index_at(row)?;
                                let high = scale.price_to_coordinate(projection[1], base_value);
                                let low = scale.price_to_coordinate(projection[2], base_value);
                                (high.is_finite() && low.is_finite()).then_some((
                                    self.time_scale.index_to_coordinate(logical),
                                    logical,
                                    high,
                                    low,
                                ))
                            })
                            .collect::<Vec<_>>();
                        hit_test_series_range(&items, x_css, y_css, bar_spacing, HIT_TEST_TOLERANCE)
                            .map(|distance| (distance, SeriesHitKind::Range))
                    }
                }
            }
            // A custom series' geometry is plugin-defined; the engine has no built-in hit for
            // it (the reference's renderer-level `hitTest` is out of scope of the host contract).
            SeriesKind::Custom => None,
        };
        result.map(|(distance, kind)| SeriesHit {
            series: id,
            distance,
            kind,
        })
    }

    /// The series under pane-relative media px `(x_css, y_css)` (reference
    /// `MouseEventParams.hoveredSeries`), or `None` off the panes/data. Walks the stable
    /// paint order topmost-first (reference deliberately does NOT use the temporary
    /// hovered-on-top render order for arbitration, pane-hit-test.ts) and arbitrates
    /// competing hits with [`SeriesHit::is_better_than`].
    pub fn hit_test_series(&self, x_css: f64, y_css: f64) -> Option<SeriesId> {
        let pane = self.pane_at_y(y_css)?;
        let mut best: Option<SeriesHit> = None;
        for &id in self.series_order.iter().rev() {
            if self
                .series_entry(id)
                .is_none_or(|series| series.pane_index != pane)
            {
                continue;
            }
            let Some(hit) = self.hit_test_one_series(id, x_css, y_css) else {
                continue;
            };
            if best.is_none_or(|current| hit.is_better_than(&current)) {
                best = Some(hit);
            }
        }
        best.map(|hit| hit.series)
    }

    /// The hovered series driving the `hoveredSeriesOnTop` render z-bump (reference
    /// `ChartModel.hoveredSource`). Hosts refresh it from their hover pipeline; a removed
    /// id never sticks.
    pub fn set_hovered_series(&mut self, id: Option<SeriesId>) {
        let next = id.filter(|&sid| self.series.iter().any(|s| s.id == sid && !s.removed));
        if self.hovered_series != next {
            self.hovered_series = next;
            // Hover-on-top changes only retained-layer assembly order; per-series geometry stays
            // valid and is reassembled in the new order by the next frame build.
            self.invalidate_frame_assembly();
        }
    }

    pub fn hovered_series(&self) -> Option<SeriesId> {
        self.hovered_series
    }

    /// Select one series, expanding engine-owned indicator and external-study outputs into one
    /// interaction group.
    /// The primary id remains the hit-tested series for compatibility with commands that need a
    /// single target.
    pub fn set_selected_series(&mut self, id: Option<SeriesId>) {
        let next = id.filter(|&sid| self.series.iter().any(|s| s.id == sid && !s.removed));
        let Some(series) = next else {
            if self.selection.take().is_some() {
                self.invalidate_frame_overlay();
            }
            return;
        };
        self.clear_volume_profile_selection();
        let members = if let Some(binding) = self.indicator_binding_id(series) {
            self.indicator_group_outputs(binding)
        } else if let Some(study) = self.external_study_for_series(series) {
            self.external_study_series(study)
        } else {
            vec![series]
        };
        let _ = self.set_selected_series_group(series, &members);
    }

    /// Select a host-defined group of related output series while retaining `primary` as the
    /// command target. Invalid, duplicate, empty, or oversized groups are rejected atomically.
    /// This remains the integration boundary for product-owned multi-output series that are not
    /// registered as engine indicators or external studies.
    pub fn set_selected_series_group(&mut self, primary: SeriesId, members: &[SeriesId]) -> bool {
        if members.is_empty()
            || members.len() > MAX_SELECTION_MEMBERS
            || !members.contains(&primary)
        {
            return false;
        }
        let mut unique = std::collections::HashSet::with_capacity(members.len());
        if members.iter().any(|series| {
            !unique.insert(*series)
                || !self
                    .series
                    .iter()
                    .any(|entry| entry.id == *series && !entry.removed)
        }) {
            return false;
        }
        if self.selected_series() == Some(primary)
            && self.selection.as_ref().is_some_and(|selection| {
                selection.members.len() == members.len()
                    && selection
                        .members
                        .iter()
                        .map(|member| member.series)
                        .eq(members.iter().copied())
            })
        {
            return true;
        }
        self.selection = Some(SelectionAnchorSnapshot {
            series: primary,
            members: members
                .iter()
                .copied()
                .map(|series| SelectionAnchorMemberSnapshot {
                    series,
                    times: self.sample_selection_anchor_times(series),
                })
                .collect(),
        });
        self.invalidate_frame_overlay();
        true
    }

    pub fn selected_series(&self) -> Option<SeriesId> {
        self.selection.as_ref().map(|selection| selection.series)
    }

    /// Series participating in the current selection, in the host-provided output order.
    pub fn selected_series_members(&self) -> impl Iterator<Item = SeriesId> + '_ {
        self.selection
            .iter()
            .flat_map(|selection| selection.members.iter().map(|member| member.series))
    }

    pub(crate) fn series_is_selected(&self, series: SeriesId) -> bool {
        self.selection.as_ref().is_some_and(|selection| {
            selection
                .members
                .iter()
                .any(|member| member.series == series)
        })
    }

    /// Canonical timestamp identities retained for the current series/indicator selection.
    /// Internal hosts expose this only as a deterministic interaction-test hook.
    #[doc(hidden)]
    pub fn selection_anchor_identities(&self) -> &[i64] {
        self.selection
            .as_ref()
            .and_then(|selection| {
                selection
                    .members
                    .iter()
                    .find(|member| member.series == selection.series)
            })
            .map_or(&[], |member| member.times.as_slice())
    }

    fn sample_selection_anchor_times(&self, series: SeriesId) -> Vec<i64> {
        let Some(entry) = self.series_entry(series) else {
            return Vec::new();
        };
        if entry.kind == SeriesKind::Custom {
            return Vec::new();
        }
        let plot = self.data.plot(series);
        let Some((times, _)) = self.data.series_data(series) else {
            return Vec::new();
        };
        let Some(first_index) = plot.first_index() else {
            return Vec::new();
        };
        let Some(last_index) = plot.last_index() else {
            return Vec::new();
        };
        let Some(first_row) = plot.first_non_whitespace_row(first_index) else {
            return Vec::new();
        };
        let Some(last_row) = plot.last_non_whitespace_row(last_index) else {
            return Vec::new();
        };
        // Anchors are canonical row times; an as-of plot row names the row it repeats.
        if first_row == last_row {
            return times
                .get(plot.source_row(first_row))
                .copied()
                .into_iter()
                .collect();
        }

        let first_x = plot
            .index_at(first_row)
            .map(|index| self.time_scale.index_to_coordinate(index));
        let last_x = plot
            .index_at(last_row)
            .map(|index| self.time_scale.index_to_coordinate(index));
        let extent = first_x
            .zip(last_x)
            .map_or(0.0, |(first, last)| (last - first).abs());
        let count = ((extent / SELECTION_ANCHOR_SPACING_CSS).round() as usize)
            .clamp(2, MAX_SELECTION_ANCHORS)
            .min(last_row - first_row + 1);

        let mut rows = Vec::with_capacity(count);
        for slot in 0..count {
            let target = first_row + slot * (last_row - first_row) / (count - 1);
            let row = if slot == 0 {
                first_row
            } else if slot + 1 == count {
                last_row
            } else {
                let index = plot.index_at(target).unwrap_or(i64::MAX);
                let left = plot
                    .last_non_whitespace_row(index)
                    .filter(|row| *row >= first_row);
                let right = plot
                    .first_non_whitespace_row(index)
                    .filter(|row| *row <= last_row);
                match (left, right) {
                    (Some(left), Some(right)) => {
                        if target - left <= right - target {
                            left
                        } else {
                            right
                        }
                    }
                    (Some(left), None) => left,
                    (None, Some(right)) => right,
                    (None, None) => continue,
                }
            };
            rows.push(row);
        }
        rows.sort_unstable();
        rows.dedup();
        let mut anchors = rows
            .into_iter()
            .filter_map(|row| times.get(plot.source_row(row)).copied())
            .collect::<Vec<_>>();
        anchors.dedup();
        anchors
    }

    pub(crate) fn prune_selection_anchor_snapshot(&mut self) {
        let Some(selection) = self.selection.as_mut() else {
            return;
        };
        if self.data.series_data(selection.series).is_none() {
            self.selection = None;
            return;
        }
        let data = &self.data;
        selection.members.retain_mut(|member| {
            let Some((times, _)) = data.series_data(member.series) else {
                return false;
            };
            member
                .times
                .retain(|time| times.binary_search(time).is_ok());
            true
        });
    }

    pub(crate) fn restart_selection_anchor_snapshot_after_replacement(
        &mut self,
        replaced: SeriesId,
    ) {
        let Some(selection) = self.selection.as_ref() else {
            return;
        };
        if selection
            .members
            .iter()
            .any(|member| member.series == replaced)
            || self.indicator_changes.iter().any(|&(series, _)| {
                selection
                    .members
                    .iter()
                    .any(|member| member.series == series)
            })
        {
            let selected = selection.series;
            let members = selection
                .members
                .iter()
                .map(|member| member.series)
                .collect::<Vec<_>>();
            self.selection = Some(SelectionAnchorSnapshot {
                series: selected,
                members: members
                    .into_iter()
                    .map(|series| SelectionAnchorMemberSnapshot {
                        series,
                        times: self.sample_selection_anchor_times(series),
                    })
                    .collect(),
            });
            self.invalidate_frame_overlay();
        }
    }
}

#[cfg(test)]
mod tests;
