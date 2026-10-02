//! B8 Channels family (wire ids 48..=63): parallel channel, regression trend, flat top/bottom,
//! and disjoint channel.
//!
//! The three click-placed channels share one construction. The first two anchors define the
//! base line; the second boundary line spans the same bars (so the channel's sides are vertical)
//! and lies on the line through the third anchor, whichever bar that anchor sits on:
//!
//! - parallel channel: the base line translated vertically on screen, so it stays parallel on
//!   every price-scale mode;
//! - flat top/bottom: a horizontal line at the third anchor's price;
//! - disjoint channel: the base line's slope mirrored, so the two lines converge or diverge.
//!
//! `extend_left` extends both lines (and the fill) beyond the first anchor, `extend_right` beyond
//! the second, each to the pane edge. The fill between the lines (common `fill_enabled`, on by
//! default; `fill_color`, default the stroke color at 20% alpha) splits where the lines cross so
//! every piece stays convex, and is clipped to the pane. The optional dashed middle line
//! (`tool_options.channel.middle_line`, on by default for the parallel channel) runs halfway
//! between the boundaries. With the base line vertical (both anchors on one bar) the second line
//! is the base moved sideways through the third anchor and nothing is filled.
//!
//! The regression trend fits a least-squares line to the source series' bars between its two
//! anchors (bar positions rounded), and draws it with lines `upper_deviation` and
//! `lower_deviation` residual standard deviations (sample, `n − 1`) away, the zones between them
//! filled, and Pearson's R (signed correlation of bar position and source value) below the start.
//! The anchors fix only the bar range: the body and handles move horizontally, and the lines'
//! prices come from the data, which the frame follows through the family's
//! `reads_series_data` hook. A fit is one allocation-free pass over the source's canonical rows
//! in the range, memoized by everything it reads, so frames and pointer hit tests repeat it only
//! after the range, the source, or the axis positions change; a live replacement of the latest
//! bar or appended bars extend it by the changed rows.
//!
//! Every boundary and middle line is a body target; a fill is a drag surface only while the
//! drawing is selected, like the rectangle's. Channel lines carry no end caps. Handles sit on the
//! painted lines: the base line's ends, the second line's midpoint for the third anchor (whose
//! bar is otherwise free along that line), and the regression line's ends.

use std::collections::HashMap;

use aeris_charts_core::model::plot_list::PlotValueIndex;
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::LineStyle;
use aeris_charts_render::shape::{self, Point, Rect};

use super::super::handles::DrawingHandle;
use super::super::parts::{DrawingParts, PartContext, PartLabel, PartStroke};
use super::super::tools::{
    DrawingAnchorLink, DrawingHandleMode, DrawingLogicalExtent, DrawingMovementAxis,
    DrawingPlacement, DrawingPriceExtent, DrawingStraightenMode, DrawingTextLayout,
    DrawingToolSpec,
};
use super::super::{
    Drawing, DrawingPoint, DrawingSourceRows, DrawingTextHAlign, DrawingTextVAlign,
};
use super::DrawingFamily;
use crate::{
    ChartEngine, DrawingDragPart, DrawingId, DrawingKind, DrawingKindOptions,
    DrawingPropertyDescriptor, DrawingPropertyType, IndicatorInputSource, SeriesId,
};

/// Channels-family options (`tool_options.channel`). Every field is optional: an absent field
/// takes the tool's own default, so deep-merged patches, templates, and `null` resets never
/// depend on which channel tool a block came from. The deviation, source, and Pearson fields
/// apply to the regression trend only.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ChannelToolOptions {
    /// Paint the dashed middle line (the regression line on a regression trend). Default: on
    /// for the parallel channel and the regression trend, off for the others.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub middle_line: Option<bool>,
    /// Middle-line CSS color; absent or `""` follows the stroke color.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub middle_color: Option<String>,
    /// Upper line offset in residual standard deviations (default 2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upper_deviation: Option<f64>,
    /// Lower line offset in residual standard deviations (default -2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lower_deviation: Option<f64>,
    /// Paint the upper deviation line and its zone (default true).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_upper_deviation: Option<bool>,
    /// Paint the lower deviation line and its zone (default true).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_lower_deviation: Option<bool>,
    /// Source value of each bar (default close).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<IndicatorInputSource>,
    /// Paint Pearson's R below the regression's start (default true).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_pearsons: Option<bool>,
}

/// Largest accepted deviation multiplier magnitude.
pub(crate) const MAX_DEVIATION: f64 = 100.0;
/// Longest accepted middle-line color string, in bytes.
const MAX_COLOR_BYTES: usize = 256;

impl ChannelToolOptions {
    pub fn validate(&self) -> bool {
        let deviation = |value: Option<f64>| value.is_none_or(|value| value.abs() <= MAX_DEVIATION);
        deviation(self.upper_deviation)
            && deviation(self.lower_deviation)
            && self
                .middle_color
                .as_ref()
                .is_none_or(|color| color.len() <= MAX_COLOR_BYTES)
    }

    fn resolve(&self, kind: DrawingKind) -> Resolved<'_> {
        Resolved {
            middle_line: self.middle_line.unwrap_or(matches!(
                kind,
                DrawingKind::ParallelChannel | DrawingKind::RegressionTrend
            )),
            middle_color: self
                .middle_color
                .as_deref()
                .filter(|color| !color.is_empty()),
            upper_deviation: self.upper_deviation.unwrap_or(2.0),
            lower_deviation: self.lower_deviation.unwrap_or(-2.0),
            use_upper_deviation: self.use_upper_deviation.unwrap_or(true),
            use_lower_deviation: self.use_lower_deviation.unwrap_or(true),
            source: self.source.unwrap_or_default(),
            show_pearsons: self.show_pearsons.unwrap_or(true),
        }
    }
}

/// A drawing's channel options with every default applied.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Resolved<'a> {
    middle_line: bool,
    middle_color: Option<&'a str>,
    upper_deviation: f64,
    lower_deviation: f64,
    use_upper_deviation: bool,
    use_lower_deviation: bool,
    source: IndicatorInputSource,
    show_pearsons: bool,
}

fn options(drawing: &Drawing) -> Resolved<'_> {
    const DEFAULTS: ChannelToolOptions = ChannelToolOptions {
        middle_line: None,
        middle_color: None,
        upper_deviation: None,
        lower_deviation: None,
        use_upper_deviation: None,
        use_lower_deviation: None,
        source: None,
        show_pearsons: None,
    };
    match &drawing.tool_options.channel {
        Some(block) => block.resolve(drawing.kind),
        None => DEFAULTS.resolve(drawing.kind),
    }
}

/// Middle-line width in CSS px (dashed).
const MIDDLE_WIDTH: f64 = 1.0;
/// Default fill alpha over the stroke color (20%, the rectangle's wash).
const FILL_ALPHA: u8 = 51;
/// Gap between the lowest regression line and Pearson's R, in CSS px.
const PEARSON_GAP: f64 = 4.0;
/// Widest Pearson's R text, measured for the culling pad.
const PEARSON_SAMPLE: &str = "-0.0000";

/// Shared three-anchor channel behavior; every spec below overrides its identity.
const CHANNEL_TOOL: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::ParallelChannel,
    wire_id: 48,
    name: "parallel_channel",
    placement: DrawingPlacement::ClickAnchors { count: 3 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::Segment45,
    logical_extent: DrawingLogicalExtent::Finite,
    // The second line spans the base line's bars through the third anchor, so its ends can leave
    // the anchors' price box; only the bar range bounds the channel.
    price_extent: DrawingPriceExtent::Full,
    bounds_padding_ratio: 0.0,
    default_width: 2.0,
    requests_text_editor: false,
    family: Some(&FAMILY),
    text_layout: DrawingTextLayout::Segment,
    axis_price_label: false,
    grid_snap: false,
    anchor_link: DrawingAnchorLink::None,
    axis_tag_text: false,
};

pub(crate) const PARALLEL_CHANNEL: DrawingToolSpec = CHANNEL_TOOL;

pub(crate) const REGRESSION_TREND: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::RegressionTrend,
    wire_id: 49,
    name: "regression_trend",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    // The anchors choose bars; prices come from the data.
    movement_axis: DrawingMovementAxis::HorizontalOnly,
    straighten: DrawingStraightenMode::None,
    default_width: 1.0,
    text_layout: DrawingTextLayout::Box,
    ..CHANNEL_TOOL
};

pub(crate) const FLAT_TOP_BOTTOM: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::FlatTopBottom,
    wire_id: 50,
    name: "flat_top_bottom",
    // Both lines' ends sit inside the anchors' box.
    price_extent: DrawingPriceExtent::Finite,
    ..CHANNEL_TOOL
};

pub(crate) const DISJOINT_CHANNEL: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::DisjointChannel,
    wire_id: 51,
    name: "disjoint_channel",
    ..CHANNEL_TOOL
};

// The base line is the channel's centre: its parallel through the third anchor and the mirror of
// that parallel on the other side bound it.
pub(crate) const PRICE_CHANNEL: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::PriceChannel,
    wire_id: 52,
    name: "price_channel",
    ..CHANNEL_TOOL
};

pub(crate) static FAMILY: DrawingFamily = {
    let mut family = DrawingFamily::new(build_parts, kind_options);
    family.apply_defaults = apply_defaults;
    family.decoration_extent = decoration_extent;
    family.extend_schema = extend_schema;
    family.reads_series_data = |drawing| drawing.kind == DrawingKind::RegressionTrend;
    family.handles = |engine, drawing, px, handles| move_handles(engine, drawing, px, handles);
    // Placing the second anchor previews the base line alone.
    family.partial_preview = true;
    family
};

fn apply_defaults(drawing: &mut Drawing) {
    // KLineChart's price channel is three bare lines across the pane.
    let price_channel = drawing.kind == DrawingKind::PriceChannel;
    drawing.fill_enabled = !price_channel;
    drawing.extend_left = price_channel;
    drawing.extend_right = price_channel;
}

fn build_parts(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    if ctx.drawing.kind == DrawingKind::RegressionTrend {
        regression_parts(ctx, parts);
    } else {
        channel_parts(ctx, parts);
    }
}

type Line = (Point, Point);

fn channel_parts(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let (Some(&a), Some(&b)) = (ctx.px.first(), ctx.px.get(1)) else {
        return;
    };
    let base = (a, b);
    let Some(&c) = ctx.px.get(2) else {
        stroke_line(ctx, parts, base, PartStroke::default(), true);
        return;
    };
    let other = second_line(drawing.kind, a, b, c);
    let resolved = options(drawing);
    // A price channel is symmetric about its base line: the third line mirrors `other`.
    let mirror = (drawing.kind == DrawingKind::PriceChannel).then(|| {
        let reflect = |p: Point, q: Point| (2.0 * p.0 - q.0, 2.0 * p.1 - q.1);
        (reflect(a, other.0), reflect(b, other.1))
    });
    if drawing.fill_enabled && (b.0 - a.0).abs() > f64::EPSILON {
        let span = extended_span(ctx.pane, base, drawing);
        fill_between(ctx, parts, mirror.unwrap_or(base), other, span);
    }
    if resolved.middle_line {
        let middle = (shape::midpoint(a, other.0), shape::midpoint(b, other.1));
        stroke_line(ctx, parts, middle, middle_stroke(resolved), false);
    }
    stroke_line(ctx, parts, base, PartStroke::default(), true);
    stroke_line(ctx, parts, other, PartStroke::default(), false);
    if let Some(mirror) = mirror {
        stroke_line(ctx, parts, mirror, PartStroke::default(), false);
    }
}

/// The channel's second boundary through `c`, spanning the base line's x range.
fn second_line(kind: DrawingKind, a: Point, b: Point, c: Point) -> Line {
    let dx = b.0 - a.0;
    if dx.abs() <= f64::EPSILON {
        return ((c.0, a.1), (c.0, b.1));
    }
    let slope = (b.1 - a.1) / dx;
    match kind {
        DrawingKind::FlatTopBottom => ((a.0, c.1), (b.0, c.1)),
        DrawingKind::DisjointChannel => (
            (a.0, c.1 - slope * (a.0 - c.0)),
            (b.0, c.1 - slope * (b.0 - c.0)),
        ),
        _ => {
            let offset = c.1 - (a.1 + slope * (c.0 - a.0));
            ((a.0, a.1 + offset), (b.0, b.1 + offset))
        }
    }
}

/// Handles on the painted lines: a channel's third handle at its second line's midpoint (the
/// third anchor's bar is free along that line), and a regression's two on the regression line's
/// ends. Each still drives its own anchor by pointer deltas.
fn move_handles(
    engine: &ChartEngine,
    drawing: &Drawing,
    px: &[Point],
    handles: &mut [DrawingHandle],
) {
    let mut moved = |index: usize, point: Point| {
        if let Some(handle) = handles
            .iter_mut()
            .find(|handle| handle.part == DrawingDragPart::Anchor(index))
        {
            handle.point = point;
        }
    };
    if drawing.kind == DrawingKind::RegressionTrend {
        let Some(stats) = regression_stats(engine, drawing, options(drawing).source) else {
            return;
        };
        for (index, anchor) in drawing.points.iter().enumerate() {
            let on_line = DrawingPoint {
                logical: anchor.logical,
                price: stats.price_at(anchor.logical),
            };
            if let Some(point) =
                engine.drawing_to_px_for(drawing.pane_index, drawing.price_scale, on_line)
            {
                moved(index, point);
            }
        }
    } else if let [a, b, c] = *px {
        let (start, end) = second_line(drawing.kind, a, b, c);
        moved(2, shape::midpoint(start, end));
    }
}

fn middle_stroke(resolved: Resolved<'_>) -> PartStroke {
    PartStroke {
        color: resolved.middle_color.and_then(Color::parse_css),
        ..PartStroke::decoration(MIDDLE_WIDTH, LineStyle::Dashed)
    }
}

/// Stroke `line` with the drawing's extensions, clipped to the pane in its own direction.
fn stroke_line(
    ctx: &PartContext<'_>,
    parts: &mut DrawingParts,
    (a, b): Line,
    stroke: PartStroke,
    label_gap: bool,
) {
    let drawing = ctx.drawing;
    let (start, end) =
        shape::extend_segment(a, b, ctx.pane, drawing.extend_left, drawing.extend_right);
    parts.stroke(&[start, end], stroke, label_gap);
}

/// The base line's parameter range (0 at its start, 1 at its end) that the fill covers: the
/// anchors' span, widened to the pane's left and right edges on the extended sides.
fn extended_span(pane: Rect, (a, b): Line, drawing: &Drawing) -> (f64, f64) {
    let dx = b.0 - a.0;
    let (left, right) = ((pane.left - a.0) / dx, (pane.right - a.0) / dx);
    let (low, high) = (left.min(right), left.max(right));
    (
        if drawing.extend_left {
            low.min(0.0)
        } else {
            0.0
        },
        if drawing.extend_right {
            high.max(1.0)
        } else {
            1.0
        },
    )
}

/// Fill between two lines that share their start and end x (vertical sides) over `span` of
/// their common parameter. The vertical gap between them is linear, so a sign change marks the
/// one crossing, where the region splits into two convex pieces; each is clipped to the pane.
fn fill_between(
    ctx: &PartContext<'_>,
    parts: &mut DrawingParts,
    first: Line,
    second: Line,
    (start, end): (f64, f64),
) {
    let at = |(a, b): Line, t: f64| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    let gap = |t: f64| at(second, t).1 - at(first, t).1;
    let (g0, g1) = (gap(start), gap(end));
    let mut cuts = [start, end, end];
    let mut count = 2;
    if g0 * g1 < 0.0 {
        cuts[1] = start + (end - start) * g0 / (g0 - g1);
        count = 3;
    }
    let color = ctx.drawing.fill_or_wash(FILL_ALPHA);
    let hit = ctx.fills_hit();
    let mut clipped = Vec::with_capacity(8);
    for pair in cuts[..count].windows(2) {
        let (t0, t1) = (pair[0], pair[1]);
        let quad = [at(first, t0), at(first, t1), at(second, t1), at(second, t0)];
        shape::clip_polygon_to_rect(&quad, ctx.pane, &mut clipped);
        parts.fill_convex(&clipped, Some(color), hit);
    }
}

/// Least-squares fit of a source series over a bar range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RegressionStats {
    /// Bars with a finite source value in the range.
    pub(crate) count: usize,
    pub(crate) mean_logical: f64,
    pub(crate) mean_price: f64,
    /// Price change per bar.
    pub(crate) slope: f64,
    /// Sample standard deviation of the residuals (`n − 1` denominator; 0 for one bar).
    pub(crate) deviation: f64,
    /// Pearson correlation of bar position and source value; `None` when either is constant.
    pub(crate) pearson: Option<f64>,
}

impl RegressionStats {
    pub(crate) fn price_at(&self, logical: f64) -> f64 {
        self.mean_price + self.slope * (logical - self.mean_logical)
    }
}

fn source_value(rows: DrawingSourceRows<'_>, row: usize, source: IndicatorInputSource) -> f64 {
    let value = |key| rows.value(row, key);
    let (open, high, low, close) = (
        PlotValueIndex::Open,
        PlotValueIndex::High,
        PlotValueIndex::Low,
        PlotValueIndex::Close,
    );
    match source {
        IndicatorInputSource::Open => value(open),
        IndicatorInputSource::High => value(high),
        IndicatorInputSource::Low => value(low),
        IndicatorInputSource::Close => value(close),
        IndicatorInputSource::Hl2 => (value(high) + value(low)) * 0.5,
        IndicatorInputSource::Hlc3 => (value(high) + value(low) + value(close)) / 3.0,
        IndicatorInputSource::Ohlc4 => {
            (value(open) + value(high) + value(low) + value(close)) * 0.25
        }
        IndicatorInputSource::Hlcc4 => (value(high) + value(low) + 2.0 * value(close)) * 0.25,
    }
}

/// The inputs whose change forces a full pass: the source series, the positions of the merged
/// points (an as-of source also reads every point, since a new one can place a waiting row), the
/// replay boundary, the bar range, and the source value. The source's own data is tracked
/// separately, so a change the engine reports as a tail change extends the fit instead.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RegressionKey {
    series: SeriesId,
    time_index: u64,
    as_of_time_points: Option<u64>,
    replay: Option<i64>,
    low: i64,
    high: i64,
    source: IndicatorInputSource,
}

/// Shifted sums of one fit, accumulated row by row. Rows are always added in order, so sums that
/// continue from a retained prefix are bitwise identical to one pass over the whole range.
#[derive(Clone, Copy, Debug, Default)]
struct RegressionSums {
    count: usize,
    /// The first finite value, by which every value is shifted to keep the sums well conditioned.
    shift: Option<f64>,
    sx: f64,
    sy: f64,
    sxx: f64,
    syy: f64,
    sxy: f64,
}

impl RegressionSums {
    fn add(
        &mut self,
        rows: DrawingSourceRows<'_>,
        row: usize,
        low: f64,
        input: IndicatorInputSource,
    ) {
        let value = source_value(rows, row, input);
        let Some(index) = rows.index_of(row).filter(|_| value.is_finite()) else {
            return;
        };
        let x = index as f64 - low;
        let y = value - *self.shift.get_or_insert(value);
        self.count += 1;
        self.sx += x;
        self.sy += y;
        self.sxx += x * x;
        self.syy += y * y;
        self.sxy += x * y;
    }

    fn stats(&self, low: f64) -> Option<RegressionStats> {
        let shift = self.shift?;
        let n = self.count as f64;
        let (mean_x, mean_y) = (self.sx / n, self.sy / n);
        let m2x = (self.sxx - self.sx * mean_x).max(0.0);
        let m2y = (self.syy - self.sy * mean_y).max(0.0);
        let cxy = self.sxy - self.sx * mean_y;
        let slope = if m2x > 0.0 { cxy / m2x } else { 0.0 };
        let residual = (m2y - slope * cxy).max(0.0);
        let deviation = if self.count > 1 {
            (residual / (n - 1.0)).sqrt()
        } else {
            0.0
        };
        let pearson = (m2x > 0.0 && m2y > 0.0).then(|| (cxy / (m2x * m2y).sqrt()).clamp(-1.0, 1.0));
        Some(RegressionStats {
            count: self.count,
            mean_logical: low + mean_x,
            mean_price: shift + mean_y,
            slope,
            deviation,
            pearson,
        })
    }
}

/// One drawing's latest fit: its key, the source data generation it read, how far the engine's
/// change reports have followed that data since (`tracked`, and the first row any of them
/// touched), and the fit's rows `start..=last` with the sums of `start..last` (every row but the
/// last, which the latest bar replaces in place).
#[derive(Clone, Copy, Debug)]
struct RegressionFit {
    key: RegressionKey,
    generation: u64,
    tracked: u64,
    unchanged_below: usize,
    start: usize,
    last: Option<usize>,
    closed: RegressionSums,
    stats: Option<RegressionStats>,
}

/// The latest regression fit of each drawing, keyed by everything it read, so frame rebuilds and
/// pointer hit tests reuse one pass over a range until its data, range, or source changes. One
/// entry per drawing id, however many regressions a chart holds (a shared fixed-slot cache would
/// refit every regression on every frame once they outnumber its slots). A data change the engine
/// reports from a row at or after a fit's last row (a live replacement of the latest bar, or bars
/// appended) extends the fit by the changed rows instead of repeating the pass over the range.
/// Fits of removed drawings are dropped once the entries exceed the chart's drawings by
/// [`super::super::DRAWING_MEMO_SLACK`], so the memory stays bounded by the live drawings.
#[derive(Debug, Default)]
pub(crate) struct RegressionMemo {
    fits: HashMap<DrawingId, RegressionFit>,
    /// Full passes over a range (memo misses), for tests of the reuse contract.
    #[cfg(test)]
    pub(crate) passes: usize,
}

impl RegressionMemo {
    /// The engine reports a data change of `series` from its generation `previous` to
    /// `generation`, touching rows from `from` on. A fit that followed every earlier change keeps
    /// following; one that missed a change (reported or not) is refit in full when next read.
    pub(crate) fn note_change(
        &mut self,
        series: SeriesId,
        previous: u64,
        generation: u64,
        from: usize,
    ) {
        for fit in self.fits.values_mut() {
            if fit.key.series == series && fit.tracked == previous {
                fit.tracked = generation;
                fit.unchanged_below = fit.unchanged_below.min(from);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.fits.len()
    }
}

/// Regression statistics of `drawing`'s source over the bars between its two anchors (rounded
/// positions, inclusive), memoized per input. `None` without a source or without a finite source
/// value in the range. The source's canonical rows are read once each (see
/// [`ChartEngine::drawing_source_window`]).
pub(crate) fn regression_stats(
    engine: &ChartEngine,
    drawing: &Drawing,
    source: IndicatorInputSource,
) -> Option<RegressionStats> {
    let (first, second) = (drawing.points.first()?, drawing.points.get(1)?);
    let low = first.logical.min(second.logical).round();
    let high = first.logical.max(second.logical).round();
    if !low.is_finite() || !high.is_finite() {
        return None;
    }
    let rows = engine.drawing_source_window(drawing)?;
    let key = RegressionKey {
        series: rows.series,
        time_index: engine.data.time_index_generation(),
        as_of_time_points: rows
            .is_as_of()
            .then(|| engine.data.time_points_generation()),
        replay: engine.replay_clock_micros,
        low: low as i64,
        high: high as i64,
        source,
    };
    let generation = engine.data.series_generation(rows.series)?;
    let mut memo = engine.drawing_settings.regression_memo.borrow_mut();
    if let Some(fit) = memo.fits.get_mut(&drawing.id).filter(|fit| fit.key == key) {
        if fit.generation == generation {
            return fit.stats;
        }
        if let Some(stats) = extend_fit(fit, rows, generation, low, high, source) {
            return stats;
        }
    }
    let window = rows.rows_between(low as i64, high as i64);
    let mut closed = RegressionSums::default();
    let last = window
        .end
        .checked_sub(1)
        .filter(|&last| last >= window.start);
    for row in window.start..last.unwrap_or(window.start) {
        closed.add(rows, row, low, source);
    }
    let mut total = closed;
    if let Some(last) = last {
        total.add(rows, last, low, source);
    }
    let stats = total.stats(low);
    #[cfg(test)]
    {
        memo.passes += 1;
    }
    let fit = RegressionFit {
        key,
        generation,
        tracked: generation,
        unchanged_below: usize::MAX,
        start: window.start,
        last,
        closed,
        stats,
    };
    super::super::insert_drawing_memo(&mut memo.fits, &engine.drawings, drawing.id, fit);
    stats
}

/// Extend `fit` over a data change that left every row before its last untouched: the old last
/// row, any appended rows, and the new last row are the only rows read, so a live bar costs the
/// changed rows rather than the range. `None` when the change reached earlier rows, was not
/// followed, or the source is as-of (whose rows' positions follow every axis point); the caller
/// then refits in full.
fn extend_fit(
    fit: &mut RegressionFit,
    rows: DrawingSourceRows<'_>,
    generation: u64,
    low: f64,
    high: f64,
    source: IndicatorInputSource,
) -> Option<Option<RegressionStats>> {
    let last = fit.last?;
    if rows.is_as_of() || fit.tracked != generation || fit.unchanged_below < last {
        return None;
    }
    let window = rows.rows_between(low as i64, high as i64);
    if window.start != fit.start || window.end <= last {
        return None;
    }
    let new_last = window.end - 1;
    let mut closed = fit.closed;
    for row in last..new_last {
        closed.add(rows, row, low, source);
    }
    let mut total = closed;
    total.add(rows, new_last, low, source);
    let stats = total.stats(low);
    *fit = RegressionFit {
        generation,
        tracked: generation,
        unchanged_below: usize::MAX,
        last: Some(new_last),
        closed,
        stats,
        ..*fit
    };
    Some(stats)
}

fn regression_parts(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let (Some(&a), Some(&b)) = (ctx.px.first(), ctx.px.get(1)) else {
        return;
    };
    let resolved = options(drawing);
    let Some(stats) = regression_stats(ctx.engine, drawing, resolved.source) else {
        // No source bars in range yet: the anchor segment keeps the drawing visible and
        // selectable until data arrives.
        parts.stroke(&[a, b], middle_stroke(resolved), false);
        return;
    };
    let (first, second) = (drawing.points[0].logical, drawing.points[1].logical);
    let line_at = |deviations: f64| -> Option<Line> {
        let offset = deviations * stats.deviation;
        let point = |logical: f64| {
            ctx.point_px(DrawingPoint {
                logical,
                price: stats.price_at(logical) + offset,
            })
        };
        Some((point(first)?, point(second)?))
    };
    let Some(center) = line_at(0.0) else {
        return;
    };
    let upper = resolved
        .use_upper_deviation
        .then(|| line_at(resolved.upper_deviation))
        .flatten();
    let lower = resolved
        .use_lower_deviation
        .then(|| line_at(resolved.lower_deviation))
        .flatten();
    if drawing.fill_enabled && (b.0 - a.0).abs() > f64::EPSILON {
        let span = extended_span(ctx.pane, center, drawing);
        for band in [upper, lower].into_iter().flatten() {
            fill_between(ctx, parts, center, band, span);
        }
    }
    if resolved.middle_line {
        stroke_line(ctx, parts, center, middle_stroke(resolved), false);
    }
    for band in [upper, lower].into_iter().flatten() {
        stroke_line(ctx, parts, band, PartStroke::default(), false);
    }
    if let Some(pearson) = stats.pearson.filter(|_| resolved.show_pearsons) {
        // Below the lowest line at the regression's start, reading into the channel.
        let start = center.0;
        let bottom = [Some(center), upper, lower]
            .into_iter()
            .flatten()
            .map(|line| line.0 .1)
            .fold(start.1, f64::max);
        parts.label(PartLabel {
            anchor: (start.0, bottom + PEARSON_GAP * ctx.scale),
            h_align: if b.0 >= a.0 {
                DrawingTextHAlign::Left
            } else {
                DrawingTextHAlign::Right
            },
            v_align: DrawingTextVAlign::Top,
            lines: vec![format!("{pearson:.4}")],
            size: ctx.engine.drawing_text_size(drawing) * ctx.scale,
            weight: drawing.text_weight.unwrap_or(400),
            italic: drawing.text_italic,
            color: None,
            background: None,
            border: None,
            padding: (0.0, 0.0),
            hit: false,
        });
    }
}

/// Pearson's R reaches right of the regression's start by its width; everything else stays in
/// the bar range (vertical culling is off for the family's data-driven and third-anchor lines).
fn decoration_extent(engine: &ChartEngine, drawing: &Drawing) -> f64 {
    if drawing.kind != DrawingKind::RegressionTrend || !options(drawing).show_pearsons {
        return 0.0;
    }
    let size = engine.drawing_text_size(drawing);
    let family = &engine.options.get().layout.font_family;
    let width = engine.measure_text_run(
        PEARSON_SAMPLE,
        size,
        family,
        drawing.text_weight.unwrap_or(400),
        drawing.text_italic,
    );
    PEARSON_GAP + width + size
}

fn descriptor(
    name: &str,
    property_type: DrawingPropertyType,
    default: serde_json::Value,
) -> DrawingPropertyDescriptor {
    crate::drawing_contract::descriptor(
        format!("tool_options.channel.{name}"),
        property_type,
        default,
    )
}

fn extend_schema(template: &Drawing, properties: &mut Vec<DrawingPropertyDescriptor>) {
    let resolved = options(template);
    properties.push(descriptor(
        "middle_line",
        DrawingPropertyType::Boolean,
        serde_json::json!(resolved.middle_line),
    ));
    properties.push(descriptor(
        "middle_color",
        DrawingPropertyType::Color,
        serde_json::json!(resolved.middle_color.unwrap_or_default()),
    ));
    if template.kind != DrawingKind::RegressionTrend {
        return;
    }
    for (name, value) in [
        ("upper_deviation", resolved.upper_deviation),
        ("lower_deviation", resolved.lower_deviation),
    ] {
        properties.push(DrawingPropertyDescriptor {
            min: Some(-MAX_DEVIATION),
            max: Some(MAX_DEVIATION),
            ..descriptor(name, DrawingPropertyType::Number, serde_json::json!(value))
        });
    }
    for (name, value) in [
        ("use_upper_deviation", resolved.use_upper_deviation),
        ("use_lower_deviation", resolved.use_lower_deviation),
    ] {
        properties.push(descriptor(
            name,
            DrawingPropertyType::Boolean,
            serde_json::json!(value),
        ));
    }
    properties.push(DrawingPropertyDescriptor {
        enum_values: IndicatorInputSource::ALL
            .iter()
            .filter_map(|source| serde_json::to_value(source).ok())
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect(),
        ..descriptor(
            "source",
            DrawingPropertyType::Enum,
            serde_json::json!(resolved.source),
        )
    });
    properties.push(descriptor(
        "show_pearsons",
        DrawingPropertyType::Boolean,
        serde_json::json!(resolved.show_pearsons),
    ));
}

fn kind_options(drawing: &Drawing) -> DrawingKindOptions {
    let resolved = options(drawing);
    let middle_color = resolved.middle_color.map(str::to_string);
    if drawing.kind == DrawingKind::RegressionTrend {
        DrawingKindOptions::RegressionTrend {
            middle_line: resolved.middle_line,
            middle_color,
            upper_deviation: resolved.upper_deviation,
            lower_deviation: resolved.lower_deviation,
            use_upper_deviation: resolved.use_upper_deviation,
            use_lower_deviation: resolved.use_lower_deviation,
            source: resolved.source,
            show_pearsons: resolved.show_pearsons,
        }
    } else {
        DrawingKindOptions::Channel {
            middle_line: resolved.middle_line,
            middle_color,
        }
    }
}

#[cfg(test)]
mod tests;
