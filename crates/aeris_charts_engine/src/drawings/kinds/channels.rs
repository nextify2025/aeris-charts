//! B8 Channels family, own-line tool (wire id 244): the KLineChart price channel, plus the
//! channel and regression presentation layered on upstream's channel and regression arms.
//!
//! The price channel's first two anchors define its centre line; the line through the third
//! anchor parallel to it (translated vertically on screen, so it stays parallel on every
//! price-scale mode) and that line's mirror on the other side bound it. `extend_left` extends
//! every line (and the fill) beyond the first anchor, `extend_right` beyond the second, each to
//! the pane edge; both are on by default, like KLineChart's bare lines across the pane. The fill
//! between the outer lines (common `fill_enabled`, off by default; `fill_color`, default the
//! stroke color at 20% alpha) is clipped to the pane. The optional dashed middle line
//! (`tool_options.channel.middle_line`, off by default) runs halfway between the centre and the
//! third anchor's line. With the base line vertical (both anchors on one bar) the second line is
//! the base moved sideways through the third anchor and nothing is filled.
//!
//! Every line is a body target; a fill is a drag surface only while the drawing is selected,
//! like the rectangle's. The third anchor's handle sits on its line's midpoint.
//!
//! Upstream's parallel, flat, and disjoint channels (resolved by `geometry.rs`) read the same
//! `middle_line`/`middle_color` ([`channel_middle`]): a 1 CSS px dashed line between the two
//! lines' endpoints paired by direction ([`paired_second`]; for a disjoint whose ends sit on
//! different bars, the line through the midpoints of its paired ends). Their one band fill runs
//! between the paired lines, so a disjoint whose second line runs opposite to its first fills its
//! whole quad, and a crossing fills two lobes that meet at the crossing; a concave disjoint quad
//! fills through its exact ribbon ([`channel_fill_ribbon`]).
//!
//! Upstream's regression trend fits its line through [`regression_stats`]: a least-squares fit
//! of the source series' bars between its two anchors (upstream's window, `ceil` of the earlier
//! anchor through `floor` of the later one), one
//! allocation-free pass over the source's canonical rows in the range, memoized by everything it
//! reads, so frames and pointer hit tests repeat it only after the range, the source, or the axis
//! positions change; a live replacement of the latest bar or appended bars extend it by the
//! changed rows. Its options resolve in [`regression_band`]: the bar value it fits
//! (`source`), each side's offset (an `upper_deviation`/`lower_deviation` override, else
//! `±regression_deviations`, in population residual deviations) and switch, the dashed centre
//! line in `middle_color` (`middle_line`), and Pearson's R below its start (`show_pearsons`,
//! [`regression_parts`]). The anchors choose bars only: a regression moves along time, and its
//! handles sit on the fitted line's ends (upstream's projection, `geometry::anchor_handle_points`).

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
use super::super::{Drawing, DrawingSourceRows, DrawingTextHAlign, DrawingTextVAlign};
use super::DrawingFamily;
use crate::{
    ChartEngine, DrawingDragPart, DrawingId, DrawingKind, DrawingKindOptions,
    DrawingPropertyDescriptor, DrawingPropertyType, IndicatorInputSource, SeriesId,
};

/// Channels-family options (`tool_options.channel`). Every field is optional: an absent field
/// takes the tool's own default, so deep-merged patches, templates, and `null` resets never
/// depend on which channel tool a block came from. Every channel kind reads `middle_line` and
/// `middle_color`. On upstream's regression trend the deviation fields and their switches are
/// per-side overrides of the flat `regression_deviations` (an absent side follows it; see
/// `drawing_contract::take_legacy_flat_options`), and it also reads the source and the Pearson
/// field (`regression_band`). Every default is upstream's look; documents the fork wrote carry
/// the fork's on-by-default middle line and Pearson's R explicitly
/// (`super::legacy_fork_tool_options`).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ChannelToolOptions {
    /// Paint the dashed middle line (on a regression trend, its centre line dashed in
    /// `middle_color` instead of upstream's solid stroke). Default off; the fork's parallel
    /// channels and regression trends carry it on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub middle_line: Option<bool>,
    /// Middle-line CSS color; absent or `""` follows the stroke color.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub middle_color: Option<String>,
    /// Upper line offset in residual standard deviations; absent follows `regression_deviations`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upper_deviation: Option<f64>,
    /// Lower line offset in residual standard deviations; absent follows `-regression_deviations`.
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
    /// Paint Pearson's R below the regression's start. Default off; the fork's regression trends
    /// carry it on.
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

    fn resolve(&self) -> Resolved<'_> {
        Resolved {
            middle_line: self.middle_line.unwrap_or(false),
            middle_color: self
                .middle_color
                .as_deref()
                .filter(|color| !color.is_empty()),
        }
    }
}

/// A drawing's channel options with every default applied.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Resolved<'a> {
    middle_line: bool,
    middle_color: Option<&'a str>,
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
        Some(block) => block.resolve(),
        None => DEFAULTS.resolve(),
    }
}

/// Middle-line width in CSS px (dashed).
pub(crate) const MIDDLE_WIDTH: f64 = 1.0;
/// Gap between the lowest regression line's start and the top of Pearson's R, in CSS px.
const PEARSON_GAP: f64 = 4.0;
/// The widest Pearson's R text, measured for the culling pad.
const PEARSON_SAMPLE: &str = "-0.0000";
/// Default fill alpha over the stroke color (20%, the rectangle's wash).
const FILL_ALPHA: u8 = 51;

// The base line is the channel's centre: its parallel through the third anchor and the mirror of
// that parallel on the other side bound it. The lines span the base line's bars through the third
// anchor, so their ends can leave the anchors' price box; only the bar range bounds the channel.
pub(crate) const PRICE_CHANNEL: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::PriceChannel,
    wire_id: 244,
    name: "price_channel",
    placement: DrawingPlacement::ClickAnchors { count: 3 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::Segment45,
    logical_extent: DrawingLogicalExtent::Finite,
    price_extent: DrawingPriceExtent::Full,
    bounds_padding_ratio: 0.0,
    default_width: 1.0,
    requests_text_editor: false,
    family: Some(&FAMILY),
    text_layout: DrawingTextLayout::Segment,
    axis_price_label: false,
    anchor_link: DrawingAnchorLink::None,
    axis_tag_text: false,
};

pub(crate) static FAMILY: DrawingFamily = {
    let mut family = DrawingFamily::new(build_parts, kind_options);
    family.apply_defaults = apply_defaults;
    family.extend_schema = extend_schema;
    family.handles = |_, _, px, handles| move_handles(px, handles);
    // Placing the second anchor previews the base line alone.
    family.partial_preview = true;
    family
};

fn apply_defaults(drawing: &mut Drawing) {
    // KLineChart's price channel is three bare lines across the pane.
    drawing.fill_enabled = false;
    drawing.extend_left = true;
    drawing.extend_right = true;
}

/// The fork's pre-merge defaults of the upstream channel tools it rendered (see
/// [`super::apply_legacy_fork_defaults`]): every one filled between its lines.
pub(super) fn legacy_defaults(drawing: &mut Drawing) {
    if matches!(
        drawing.kind,
        DrawingKind::ParallelChannel
            | DrawingKind::RegressionTrend
            | DrawingKind::FlatTopChannel
            | DrawingKind::FlatBottomChannel
            | DrawingKind::DisjointChannel
    ) {
        drawing.fill_enabled = true;
    }
}

/// The fork's unstored `tool_options` defaults of the upstream channel tools it rendered (see
/// [`super::legacy_fork_tool_options`]): the parallel channel's dashed middle line, and the
/// regression trend's dashed centre line and Pearson's R. The fork skipped these unset options
/// when writing, so its documents carry none of them.
pub(super) fn legacy_tool_options(kind: DrawingKind) -> Option<(&'static str, serde_json::Value)> {
    match kind {
        DrawingKind::ParallelChannel => Some(("channel", serde_json::json!({"middle_line": true}))),
        DrawingKind::RegressionTrend => Some((
            "channel",
            serde_json::json!({"middle_line": true, "show_pearsons": true}),
        )),
        _ => None,
    }
}

fn build_parts(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    channel_parts(ctx, parts);
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
    let other = second_line(a, b, c);
    let resolved = options(drawing);
    // The channel is symmetric about its base line: the third line mirrors `other`.
    let reflect = |p: Point, q: Point| (2.0 * p.0 - q.0, 2.0 * p.1 - q.1);
    let mirror = (reflect(a, other.0), reflect(b, other.1));
    if drawing.fill_enabled && (b.0 - a.0).abs() > f64::EPSILON {
        let span = extended_span(ctx.pane, base, drawing);
        fill_between(ctx, parts, mirror, other, span);
    }
    if resolved.middle_line {
        let middle = (shape::midpoint(a, other.0), shape::midpoint(b, other.1));
        stroke_line(ctx, parts, middle, middle_stroke(resolved), false);
    }
    stroke_line(ctx, parts, base, PartStroke::default(), true);
    stroke_line(ctx, parts, other, PartStroke::default(), false);
    stroke_line(ctx, parts, mirror, PartStroke::default(), false);
}

/// The channel's second boundary through `c`: the base line translated vertically on screen,
/// spanning the base line's x range (moved sideways through `c` when the base is vertical).
fn second_line(a: Point, b: Point, c: Point) -> Line {
    let dx = b.0 - a.0;
    if dx.abs() <= f64::EPSILON {
        return ((c.0, a.1), (c.0, b.1));
    }
    let slope = (b.1 - a.1) / dx;
    let offset = c.1 - (a.1 + slope * (c.0 - a.0));
    ((a.0, a.1 + offset), (b.0, b.1 + offset))
}

/// The third handle sits at the second line's midpoint (the third anchor's bar is otherwise free
/// along that line); it still drives its own anchor by pointer deltas.
fn move_handles(px: &[Point], handles: &mut [DrawingHandle]) {
    let [a, b, c] = *px else {
        return;
    };
    let (start, end) = second_line(a, b, c);
    if let Some(handle) = handles
        .iter_mut()
        .find(|handle| handle.part == DrawingDragPart::Anchor(2))
    {
        handle.point = shape::midpoint(start, end);
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

/// The middle-line color override of `drawing` while its `middle_line` is on: `Some(None)`
/// follows the stroke color, `Some(Some(color))` is a parsed `middle_color`; `None` while off.
pub(crate) fn middle_line(drawing: &Drawing) -> Option<Option<Color>> {
    let resolved = options(drawing);
    resolved
        .middle_line
        .then(|| resolved.middle_color.and_then(Color::parse_css))
}

/// `second` with its ends ordered like `first`'s (reversed when the two lines run opposite ways
/// in x), so a channel's ends pair up by side: the band fill's paired chains, its selected-fill
/// hit, and the middle line. Lines that already run the same way (every parallel and flat
/// channel) come back unchanged.
pub(crate) fn paired_second(first: [Point; 2], second: [Point; 2]) -> [Point; 2] {
    if (first[1].0 - first[0].0) * (second[1].0 - second[0].0) < 0.0 {
        [second[1], second[0]]
    } else {
        second
    }
}

/// The exact ribbon of an upstream channel's band fill between `first` and `paired`
/// ([`paired_second`]) when the 2-point band between them would not paint exactly the quad
/// `(first[0], paired[0], paired[1], first[1])`: its upper then lower chains of equal length.
/// `None` keeps the 2-point band, which every executor fills exactly while the quad is convex
/// (every parallel and flat channel) or its lines properly cross (two lobes meeting at the
/// crossing, split the way the triangle executors split it). A disjoint's four free ends can also
/// make a concave quad, or one whose paired ends' sides cross, where the triangle executors'
/// fixed fan would paint outside the quad; it becomes [`shape::nonzero_ribbon`]'s tessellation,
/// which every executor and [`shape::point_in_ribbon`] cover exactly (empty without area).
pub(crate) fn channel_fill_ribbon(first: [Point; 2], paired: [Point; 2]) -> Option<Vec<Point>> {
    let quad = [first[0], paired[0], paired[1], first[1]];
    let turn = |index: usize| {
        let (a, b, c) = (quad[index], quad[(index + 1) % 4], quad[(index + 2) % 4]);
        (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0)
    };
    let turns = [turn(0), turn(1), turn(2), turn(3)];
    let convex = turns.iter().all(|turn| *turn >= 0.0) || turns.iter().all(|turn| *turn <= 0.0);
    // The executors receive f32 points, so the crossing test runs on them as theirs does.
    let encode = |(x, y): Point| [x as f32, y as f32];
    if convex
        || aeris_charts_render::line::band_crossing(
            encode(first[0]),
            encode(first[1]),
            encode(paired[0]),
            encode(paired[1]),
        )
        .is_some()
    {
        return None;
    }
    let mut chains = Vec::new();
    let count = shape::nonzero_ribbon(&quad, &mut chains);
    chains.truncate(count * 2);
    Some(chains)
}

/// An upstream channel's dashed middle line between its resolved (extended) lines `first` and
/// `paired` ([`paired_second`]), with its color override ([`middle_line`]); `None` while
/// `middle_line` is off. It joins the midpoints of the paired ends: halfway between the lines
/// wherever they share their ends' x positions (every parallel and flat channel, and a disjoint
/// whose second line spans the first's bars).
pub(crate) fn channel_middle(
    drawing: &Drawing,
    first: [Point; 2],
    paired: [Point; 2],
) -> Option<([Point; 2], Option<Color>)> {
    let color = middle_line(drawing)?;
    Some((
        [
            shape::midpoint(first[0], paired[0]),
            shape::midpoint(first[1], paired[1]),
        ],
        color,
    ))
}

/// A regression trend's band and presentation, resolved from its `tool_options.channel` block
/// against the flat `regression_deviations`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RegressionBand {
    /// The upper line's signed offset in population residual deviations
    /// (`upper_deviation`, else `regression_deviations`); `None` while `use_upper_deviation` is
    /// off.
    pub(crate) upper: Option<f64>,
    /// The lower line's signed offset (`lower_deviation`, else `-regression_deviations`); `None`
    /// while `use_lower_deviation` is off.
    pub(crate) lower: Option<f64>,
    /// The bar value the fit reads (default close).
    pub(crate) source: IndicatorInputSource,
    /// Paint Pearson's R below the start (default off).
    pub(crate) show_pearsons: bool,
}

/// The band of regression trend `drawing` ([`RegressionBand`]). Without a block it is upstream's
/// symmetric band of closes.
pub(crate) fn regression_band(drawing: &Drawing) -> RegressionBand {
    let symmetric = drawing.regression_deviations;
    let block = drawing.tool_options.channel.as_ref();
    let side = |offset: Option<f64>, switch: Option<bool>, default: f64| {
        switch.unwrap_or(true).then(|| offset.unwrap_or(default))
    };
    RegressionBand {
        upper: side(
            block.and_then(|block| block.upper_deviation),
            block.and_then(|block| block.use_upper_deviation),
            symmetric,
        ),
        lower: side(
            block.and_then(|block| block.lower_deviation),
            block.and_then(|block| block.use_lower_deviation),
            -symmetric,
        ),
        source: block
            .and_then(|block| block.source)
            .unwrap_or(IndicatorInputSource::Close),
        show_pearsons: block.and_then(|block| block.show_pearsons).unwrap_or(false),
    }
}

/// The region a regression trend fills between its resolved lines, as the paired chains of one
/// band fill: from the upper line to the lower (the zones from the centre to each side; a side
/// that is off lies on the centre line), or, when both sides sit on the same side of the centre,
/// from the centre to the farther one. `None` with both sides off. The frame paints it and a
/// selected regression's hit test reads it, so the two agree.
pub(crate) fn regression_zone(
    band: RegressionBand,
    center: [Point; 2],
    upper: [Point; 2],
    lower: [Point; 2],
) -> Option<([Point; 2], [Point; 2])> {
    match (band.upper, band.lower) {
        (None, None) => None,
        (Some(up), Some(down)) if up * down > 0.0 => Some(if up.abs() >= down.abs() {
            (upper, center)
        } else {
            (center, lower)
        }),
        _ => Some((upper, lower)),
    }
}

/// The parts a regression trend layers on upstream's regression arm (`ctx.px`: its anchors,
/// then the fitted centre, upper, and lower lines' unextended ends): Pearson's R, left-aligned
/// at the start (right-aligned when the end lies left of it), its top [`PEARSON_GAP`] below the
/// lowest drawn line's start, in the drawing's label size and color. Not a hit target.
pub(crate) fn regression_parts(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let band = regression_band(drawing);
    let [a, b, center, _, upper, _, lower, _] = ctx.px else {
        return;
    };
    if !band.show_pearsons {
        return;
    }
    let Some(pearson) =
        regression_stats(ctx.engine, drawing, band.source).and_then(|stats| stats.pearson)
    else {
        return;
    };
    // A side that is off lies on the centre line, so the lowest start is the lowest drawn one.
    let bottom = center.1.max(upper.1).max(lower.1);
    parts.label(PartLabel {
        anchor: (center.0, bottom + PEARSON_GAP * ctx.scale),
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

/// The CSS-px reach of Pearson's R beyond a regression trend's anchors (see
/// [`super::upstream_decoration_extent`]); 0 for every other kind and while it is off.
pub(crate) fn upstream_decoration_extent(engine: &ChartEngine, drawing: &Drawing) -> f64 {
    if drawing.kind != DrawingKind::RegressionTrend || !regression_band(drawing).show_pearsons {
        return 0.0;
    }
    let size = engine.drawing_text_size(drawing);
    let width = engine.measure_text_run(
        PEARSON_SAMPLE,
        size,
        &engine.options.get().layout.font_family,
        drawing.text_weight.unwrap_or(400),
        drawing.text_italic,
    );
    PEARSON_GAP + width + size
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
    /// Sum of squared bar-position deviations from their mean: 0 when every value sits on one
    /// bar (one bar, or as-of rows collapsed onto one axis point), so no line is defined.
    pub(crate) spread: f64,
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
            spread: m2x,
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

/// Regression statistics of `drawing`'s source over the bars between its two anchors (`ceil` of
/// the earlier anchor through `floor` of the later one, inclusive: the bars whose positions lie
/// between the anchors, upstream's window), memoized per input. `None` without a source or
/// without a finite source value in the range. The source's canonical rows are read once each (see
/// [`ChartEngine::drawing_source_window`]).
pub(crate) fn regression_stats(
    engine: &ChartEngine,
    drawing: &Drawing,
    source: IndicatorInputSource,
) -> Option<RegressionStats> {
    let (first, second) = (drawing.points.first()?, drawing.points.get(1)?);
    let low = first.logical.min(second.logical).ceil();
    let high = first.logical.max(second.logical).floor();
    if !low.is_finite() || !high.is_finite() || high < low {
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
}

/// The `tool_options.channel` descriptors upstream's channel kinds read (see
/// [`super::extend_upstream_schema`]): the middle line on every channel, and on the regression
/// trend its side overrides (`null` follows `regression_deviations`), side switches, source, and
/// Pearson's R. Defaults follow `template` (a kind's `Drawing::new`, which has no block).
pub(crate) fn extend_upstream_schema(
    kind: DrawingKind,
    template: &Drawing,
    properties: &mut Vec<DrawingPropertyDescriptor>,
) {
    if !matches!(
        kind,
        DrawingKind::ParallelChannel
            | DrawingKind::FlatTopChannel
            | DrawingKind::FlatBottomChannel
            | DrawingKind::DisjointChannel
            | DrawingKind::RegressionTrend
    ) {
        return;
    }
    extend_schema(template, properties);
    if kind != DrawingKind::RegressionTrend {
        return;
    }
    let block = template.tool_options.channel.clone().unwrap_or_default();
    let band = regression_band(template);
    for (name, value) in [
        ("upper_deviation", block.upper_deviation),
        ("lower_deviation", block.lower_deviation),
    ] {
        properties.push(DrawingPropertyDescriptor {
            min: Some(-MAX_DEVIATION),
            max: Some(MAX_DEVIATION),
            ..descriptor(name, DrawingPropertyType::Number, serde_json::json!(value))
        });
    }
    for (name, value) in [
        ("use_upper_deviation", band.upper.is_some()),
        ("use_lower_deviation", band.lower.is_some()),
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
            serde_json::json!(band.source),
        )
    });
    properties.push(descriptor(
        "show_pearsons",
        DrawingPropertyType::Boolean,
        serde_json::json!(band.show_pearsons),
    ));
}

fn kind_options(drawing: &Drawing) -> DrawingKindOptions {
    let resolved = options(drawing);
    DrawingKindOptions::Channel {
        middle_line: resolved.middle_line,
        middle_color: resolved.middle_color.map(str::to_string),
    }
}

#[cfg(test)]
mod tests;
