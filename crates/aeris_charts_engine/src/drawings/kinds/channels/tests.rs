//! Channels-family engine tests: catalog defaults, armed placement, shared-part frames (lines,
//! crossing-split fills, extensions, regression statistics against an independent reference),
//! hit testing (indexed and brute force), drags, nudges, magnet, time identity, streaming data,
//! schema and kind options, patches with history, persistence, clipboard, and sync.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{LineStyle, Prim, TextAlign};
use aeris_charts_render::shape::point_in_polygon;

use super::super::super::{DrawingPlacement, DrawingTextHAlign, DrawingTextVAlign};
use super::{FAMILY, regression_stats};
use crate::{
    ChartEngine, DrawingDragPart, DrawingId, DrawingKind, DrawingMagnetMode, DrawingModifiers,
    DrawingPoint, DrawingPriceScale, IndicatorInputSource,
};

const CHANNEL_KINDS: [DrawingKind; 4] = [
    DrawingKind::ParallelChannel,
    DrawingKind::RegressionTrend,
    DrawingKind::FlatTopChannel,
    DrawingKind::DisjointChannel,
];
const INK: &str = "#123456";
const HOUR: f64 = 3_600.0;

fn chart_with(times: &[f64], dpr: f64) -> ChartEngine {
    let mut chart = ChartEngine::new(800.0, 500.0, dpr);
    let close = (0..times.len())
        .map(|index| 100.0 + (index % 7) as f64)
        .collect::<Vec<_>>();
    chart
        .set_series_data(0, times, &close, &close, &close, &close)
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.build_frame();
    chart
}

fn hourly(count: usize) -> Vec<f64> {
    (0..count).map(|index| index as f64 * HOUR).collect()
}

fn chart() -> ChartEngine {
    chart_with(&hourly(40), 1.0)
}

/// Distinct OHLC columns on a noisy uptrend, so every regression source differs.
fn ohlc(count: usize) -> [Vec<f64>; 4] {
    let close = (0..count)
        .map(|index| 100.0 + 0.25 * index as f64 + 1.5 * (0.7 * index as f64).sin())
        .collect::<Vec<_>>();
    let open = close
        .iter()
        .enumerate()
        .map(|(index, value)| value - 0.5 + 0.1 * (index % 4) as f64)
        .collect();
    let high = close
        .iter()
        .enumerate()
        .map(|(index, value)| value + 1.0 + 0.1 * (index % 3) as f64)
        .collect();
    let low = close.iter().map(|value| value - 1.2).collect();
    [open, high, low, close]
}

fn trending_chart() -> ChartEngine {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let [open, high, low, close] = ohlc(40);
    chart
        .set_series_data(0, &hourly(40), &open, &high, &low, &close)
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.build_frame();
    chart
}

fn p(logical: f64, price: f64) -> DrawingPoint {
    DrawingPoint { logical, price }
}

fn add(
    chart: &mut ChartEngine,
    kind: DrawingKind,
    points: Vec<DrawingPoint>,
    options: &str,
) -> DrawingId {
    chart.add_drawing(kind, 0, points, Some(options)).unwrap()
}

fn to_px(chart: &ChartEngine, point: DrawingPoint) -> (f64, f64) {
    chart
        .drawing_to_px_for(0, DrawingPriceScale::Right, point)
        .unwrap()
}

fn from_px(chart: &ChartEngine, x: f64, y: f64) -> DrawingPoint {
    chart
        .drawing_from_px_for(0, DrawingPriceScale::Right, x, y)
        .unwrap()
}

/// Channel anchors built in media px: a rising base and a third anchor `offset` px below the
/// base at logical 15.
fn channel_points(chart: &ChartEngine, offset: f64) -> Vec<DrawingPoint> {
    let a = p(10.0, 101.0);
    let b = p(20.0, 104.0);
    let (ax, ay) = to_px(chart, a);
    let (bx, by) = to_px(chart, b);
    let cx = to_px(chart, p(15.0, 0.0)).0;
    let base_y = ay + (by - ay) * (cx - ax) / (bx - ax);
    vec![a, b, from_px(chart, cx, base_y + offset)]
}

/// Round anchors (exact in JSON) for each tool: a regression range, or a channel whose third
/// anchor sits one price unit below the base.
fn default_points(kind: DrawingKind) -> Vec<DrawingPoint> {
    if kind == DrawingKind::RegressionTrend {
        vec![p(8.0, 101.0), p(28.0, 104.0)]
    } else {
        vec![p(10.0, 101.0), p(20.0, 104.0), p(15.0, 101.5)]
    }
}

fn anchor(chart: &ChartEngine, id: DrawingId, index: usize) -> (f64, f64) {
    chart.drawing_point_to_coordinate(id, index).unwrap()
}

fn ink() -> Color {
    Color::parse_css(INK).unwrap()
}

fn wash() -> Color {
    Color::rgba(0x12, 0x34, 0x56, 51)
}

/// One drawing-colored polyline of the first pane: points, width, and style.
type InkLine = (Vec<(f64, f64)>, f32, LineStyle);

fn color_polylines(chart: &mut ChartEngine, wanted: Color) -> Vec<InkLine> {
    let frame = chart.build_frame();
    let pane = &frame.panes[0];
    pane.main
        .iter()
        .filter_map(|prim| match prim {
            Prim::Polyline {
                first_point,
                point_count,
                width,
                style,
                color,
                ..
            } if *color == wanted => Some((
                pane.points[*first_point as usize..(*first_point + *point_count) as usize]
                    .iter()
                    .map(|point| (f64::from(point[0]), f64::from(point[1])))
                    .collect(),
                *width,
                *style,
            )),
            _ => None,
        })
        .collect()
}

fn ink_polylines(chart: &mut ChartEngine) -> Vec<InkLine> {
    color_polylines(chart, ink())
}

/// One painted line: a solid stroke's polyline, or a dashed stroke's solid dash runs (the frame
/// splits dashes so every executor paints the same gaps), in order along the line.
#[derive(Clone, Debug, PartialEq)]
struct Stroke {
    runs: Vec<Vec<(f64, f64)>>,
    width: f32,
}

impl Stroke {
    fn start(&self) -> (f64, f64) {
        self.runs[0][0]
    }

    fn end(&self) -> (f64, f64) {
        *self.runs.last().unwrap().last().unwrap()
    }

    fn dashed(&self) -> bool {
        self.runs.len() > 1
    }

    /// The first two points, for the slope of a straight stroke.
    fn line(&self) -> [(f64, f64); 2] {
        [self.start(), self.end()]
    }

    /// Starts at `start` and runs toward `end`, stopping within one dash period of it (a dashed
    /// line may end in a gap); every run lies on the segment.
    fn spans(&self, start: (f64, f64), end: (f64, f64), tolerance: f64) -> bool {
        let period = if self.dashed() {
            12.0 * f64::from(self.width)
        } else {
            0.0
        };
        let length = (end.0 - start.0).hypot(end.1 - start.1);
        let along = |point: (f64, f64)| {
            ((point.0 - start.0) * (end.0 - start.0) + (point.1 - start.1) * (end.1 - start.1))
                / length
        };
        close(self.start(), start, tolerance)
            && self.runs.iter().flatten().all(|&point| {
                collinear(start, end, point)
                    && along(point) >= -tolerance
                    && along(point) <= length + tolerance
            })
            && along(self.end()) >= length - period - tolerance
    }
}

/// Group consecutive polylines into strokes: a straight run continuing the previous one along
/// its line, in the same direction, is another dash of the same stroke.
fn strokes(lines: &[InkLine]) -> Vec<Stroke> {
    let mut strokes: Vec<Stroke> = Vec::new();
    for (points, width, style) in lines {
        assert_eq!(
            *style,
            LineStyle::Solid,
            "family strokes reach the frame solid"
        );
        if let Some(stroke) = strokes.last_mut() {
            let first = &stroke.runs[0];
            let (a, b) = (first[0], *first.last().unwrap());
            let previous_end = stroke.end();
            let continues = stroke.width == *width
                && first.len() == 2
                && points.len() == 2
                && collinear(a, b, points[0])
                && collinear(a, b, points[1])
                && (points[0].0 - previous_end.0) * (b.0 - a.0)
                    + (points[0].1 - previous_end.1) * (b.1 - a.1)
                    > 0.0;
            if continues {
                stroke.runs.push(points.clone());
                continue;
            }
        }
        strokes.push(Stroke {
            runs: vec![points.clone()],
            width: *width,
        });
    }
    strokes
}

/// Boundary (solid, the drawing's stroke) and middle (dashed) strokes.
fn split_lines(lines: &[InkLine]) -> (Vec<Stroke>, Vec<Stroke>) {
    strokes(lines)
        .into_iter()
        .partition(|stroke| !stroke.dashed())
}

/// Every filled region of the first pane in `color` as a polygon outline (upper chain, then the
/// lower chain reversed).
fn fills(chart: &mut ChartEngine, color: Color) -> Vec<Vec<(f64, f64)>> {
    let frame = chart.build_frame();
    let pane = &frame.panes[0];
    pane.main
        .iter()
        .filter_map(|prim| match prim {
            Prim::BandFill {
                upper_first,
                lower_first,
                point_count,
                fill,
                ..
            } if *fill == color => {
                let chain = |first: u32| {
                    pane.points[first as usize..(first + point_count) as usize]
                        .iter()
                        .map(|point| (f64::from(point[0]), f64::from(point[1])))
                        .collect::<Vec<_>>()
                };
                let mut outline = chain(*upper_first);
                outline.extend(chain(*lower_first).into_iter().rev());
                Some(outline)
            }
            _ => None,
        })
        .collect()
}

fn close(a: (f64, f64), b: (f64, f64), tolerance: f64) -> bool {
    (a.0 - b.0).abs() <= tolerance && (a.1 - b.1).abs() <= tolerance
}

fn collinear(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> bool {
    let cross = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    cross.abs() <= 1e-3 * (b.0 - a.0).hypot(b.1 - a.1) * (c.0 - a.0).hypot(c.1 - a.1).max(1.0)
}

fn slope(line: &[(f64, f64)]) -> f64 {
    (line[1].1 - line[0].1) / (line[1].0 - line[0].0)
}

fn hit_id(chart: &ChartEngine, (x, y): (f64, f64)) -> Option<DrawingId> {
    chart.hit_test_drawing(x, y).map(|hit| hit.id)
}

/// The fork's pre-merge channel defaults, which documents it wrote omitted, come back through
/// `apply_legacy_fork_defaults` (upstream renders these channels now; the flat-bottom channel is
/// the other half of the fork's `flat_top_bottom`).
#[test]
fn catalog_defaults_follow_each_tool() {
    for kind in CHANNEL_KINDS
        .into_iter()
        .chain([DrawingKind::FlatBottomChannel])
    {
        let spec = kind.spec();
        assert!(spec.family.is_none(), "{kind:?} is upstream-rendered");
        let mut drawing = crate::Drawing::new(1, kind, 0, Vec::new());
        super::super::apply_legacy_fork_defaults(&mut drawing);
        assert!(drawing.fill_enabled, "{kind:?} fills by default");
        assert!(drawing.tool_options.is_empty());
        assert!(!drawing.extend_left && !drawing.extend_right);
        let regression = kind == DrawingKind::RegressionTrend;
        assert_eq!(drawing.width, if regression { 1.0 } else { 2.0 });
        // The fork's segment-run channels aligned their label top-right.
        assert_eq!(
            (drawing.text_h_align, drawing.text_v_align),
            if regression {
                (DrawingTextHAlign::Center, DrawingTextVAlign::Middle)
            } else {
                (DrawingTextHAlign::Right, DrawingTextVAlign::Top)
            }
        );
    }
    // The price channel keeps its own family defaults, untouched by the legacy table.
    let fresh = crate::Drawing::new(1, DrawingKind::PriceChannel, 0, Vec::new());
    assert!(std::ptr::eq(
        DrawingKind::PriceChannel.spec().family.unwrap(),
        &FAMILY
    ));
    let mut legacy = fresh.clone();
    super::super::apply_legacy_fork_defaults(&mut legacy);
    assert_eq!(legacy, fresh);
    assert!(!fresh.fill_enabled && fresh.extend_left && fresh.extend_right);
}

/// Twenty-two trend lines past the anchors under test, so candidate queries take the culled path.
fn crowd(chart: &mut ChartEngine) {
    for index in 0..22 {
        add(
            chart,
            DrawingKind::TrendLine,
            vec![p(30.0 + index as f64 * 0.1, 100.0), p(31.0, 100.5)],
            "{}",
        );
    }
}

fn viewport_candidate(chart: &ChartEngine, id: DrawingId) -> bool {
    let candidates = chart.take_drawing_candidates(0, None);
    let found = candidates.contains(&id);
    chart.recycle_drawing_candidates(candidates);
    found
}

#[test]
fn a_second_line_inside_the_pane_is_never_culled_with_its_anchors_above_it() {
    let mut chart = chart();
    crowd(&mut chart);
    let top = chart.panes[0].top;
    // A steep base above the pane and a third anchor far right, still above the pane: the
    // parallel line through it crosses the visible pane.
    let (ax, bx, cx) = (
        to_px(&chart, p(8.0, 0.0)).0,
        to_px(&chart, p(12.0, 0.0)).0,
        to_px(&chart, p(24.0, 0.0)).0,
    );
    let (ay, by) = (top - 40.0, top - 140.0);
    let base_at_c = ay + (by - ay) * (cx - ax) / (bx - ax);
    let offset = 300.0;
    let points = vec![
        from_px(&chart, ax, ay),
        from_px(&chart, bx, by),
        from_px(&chart, cx, base_at_c + offset),
    ];
    assert!(base_at_c + offset < top, "every anchor sits above the pane");
    let id = add(
        &mut chart,
        DrawingKind::ParallelChannel,
        points,
        r##"{"color":"#123456"}"##,
    );
    chart.build_frame();
    assert!(viewport_candidate(&chart, id));
    let x = (ax + bx) / 2.0;
    let y = (ay + by) / 2.0 + offset;
    assert!(y > top && y < top + chart.panes[0].height);
    assert_eq!(hit_id(&chart, (x, y)), Some(id));
}

/// Two-pass least squares of `(x, y)` pairs: slope, mean point, residual deviation (n − 1), and
/// Pearson's R.
fn reference_fit(pairs: &[(f64, f64)]) -> (f64, (f64, f64), f64, f64) {
    let n = pairs.len() as f64;
    let mx = pairs.iter().map(|pair| pair.0).sum::<f64>() / n;
    let my = pairs.iter().map(|pair| pair.1).sum::<f64>() / n;
    let sxx = pairs.iter().map(|pair| (pair.0 - mx).powi(2)).sum::<f64>();
    let syy = pairs.iter().map(|pair| (pair.1 - my).powi(2)).sum::<f64>();
    let sxy = pairs
        .iter()
        .map(|pair| (pair.0 - mx) * (pair.1 - my))
        .sum::<f64>();
    let slope = sxy / sxx;
    let residual = pairs
        .iter()
        .map(|pair| (pair.1 - (my + slope * (pair.0 - mx))).powi(2))
        .sum::<f64>();
    (
        slope,
        (mx, my),
        (residual / (n - 1.0)).sqrt(),
        sxy / (sxx * syy).sqrt(),
    )
}

#[test]
fn regression_statistics_match_an_independent_reference_for_every_source() {
    let chart = trending_chart();
    let [open, high, low, close_values] = ohlc(40);
    // Anchors off bar centers and in reverse order: bars 7..=30, the bars whose positions lie
    // between the anchors (upstream's `ceil`/`floor` window).
    let drawing = crate::Drawing::new(
        1,
        DrawingKind::RegressionTrend,
        0,
        vec![p(30.6, 90.0), p(6.4, 120.0)],
    );
    for source in IndicatorInputSource::ALL {
        let value = |index: usize| {
            let (o, h, l, c) = (open[index], high[index], low[index], close_values[index]);
            match source {
                IndicatorInputSource::Open => o,
                IndicatorInputSource::High => h,
                IndicatorInputSource::Low => l,
                IndicatorInputSource::Close => c,
                IndicatorInputSource::Hl2 => (h + l) / 2.0,
                IndicatorInputSource::Hlc3 => (h + l + c) / 3.0,
                IndicatorInputSource::Ohlc4 => (o + h + l + c) / 4.0,
                IndicatorInputSource::Hlcc4 => (h + l + 2.0 * c) / 4.0,
            }
        };
        let pairs = (7..=30)
            .map(|index| (index as f64, value(index)))
            .collect::<Vec<_>>();
        let (slope, (mx, my), deviation, pearson) = reference_fit(&pairs);
        let stats = regression_stats(&chart, &drawing, source).unwrap();
        assert_eq!(stats.count, 24, "{source:?}");
        assert!((stats.slope - slope).abs() < 1e-10, "{source:?}");
        assert!((stats.price_at(mx) - my).abs() < 1e-9, "{source:?}");
        assert!((stats.deviation - deviation).abs() < 1e-9, "{source:?}");
        assert!(
            (stats.pearson.unwrap() - pearson).abs() < 1e-10,
            "{source:?}"
        );
    }
    // A range reaching beyond the data uses the bars that exist.
    let beyond = crate::Drawing::new(
        1,
        DrawingKind::RegressionTrend,
        0,
        vec![p(-12.0, 0.0), p(80.0, 0.0)],
    );
    let stats = regression_stats(&chart, &beyond, IndicatorInputSource::Close).unwrap();
    assert_eq!(stats.count, 40);
    // One bar: a flat line with no spread and no correlation.
    let single = crate::Drawing::new(
        1,
        DrawingKind::RegressionTrend,
        0,
        vec![p(5.2, 0.0), p(4.8, 0.0)],
    );
    let stats = regression_stats(&chart, &single, IndicatorInputSource::Close).unwrap();
    assert_eq!(
        (stats.count, stats.slope, stats.deviation, stats.spread),
        (1, 0.0, 0.0, 0.0)
    );
    assert_eq!(stats.pearson, None);
    assert!((stats.price_at(9.0) - close_values[5]).abs() < 1e-12);
    // Anchors between two bar positions select no bar.
    let between = crate::Drawing::new(
        1,
        DrawingKind::RegressionTrend,
        0,
        vec![p(5.2, 0.0), p(5.8, 0.0)],
    );
    assert_eq!(
        regression_stats(&chart, &between, IndicatorInputSource::Close),
        None
    );
    // No bars in range.
    let future = crate::Drawing::new(
        1,
        DrawingKind::RegressionTrend,
        0,
        vec![p(60.0, 0.0), p(70.0, 0.0)],
    );
    assert_eq!(
        regression_stats(&chart, &future, IndicatorInputSource::Close),
        None
    );
}

#[test]
fn regression_trends_follow_streaming_updates_of_their_source() {
    let mut chart = trending_chart();
    let channel = channel_points(&chart, 60.0);
    add(&mut chart, DrawingKind::ParallelChannel, channel, "{}");
    chart.build_frame();
    let [open, high, low, close_values] = ohlc(40);
    let last = 39;
    let time = 39.0 * HOUR;
    assert!(chart.update_series_bar(
        0,
        time,
        [open[last], high[last], low[last], close_values[last] + 0.5]
    ));
    chart.build_frame();
    assert_eq!(
        chart.frame_build_stats().drawing_rebuilds,
        0,
        "drawings that ignore data keep the retained layer"
    );

    let id = add(
        &mut chart,
        DrawingKind::RegressionTrend,
        vec![p(20.0, 0.0), p(39.0, 0.0)],
        r##"{"color":"#123456"}"##,
    );
    let before = ink_polylines(&mut chart);
    assert!(chart.update_series_bar(
        0,
        time,
        [
            open[last],
            high[last] + 9.0,
            low[last],
            close_values[last] + 8.0
        ]
    ));
    let after = ink_polylines(&mut chart);
    assert_eq!(chart.frame_build_stats().drawing_rebuilds, 1);
    assert_ne!(before, after, "the fit follows the updated bar");
    let stats = regression_stats(
        &chart,
        chart.drawing(id).unwrap(),
        IndicatorInputSource::Close,
    )
    .unwrap();
    // Upstream's regression line is the fit's center: its first two derived points.
    let fitted = chart.regression_points(chart.drawing(id).unwrap()).unwrap();
    assert_eq!(fitted[0], p(20.0, stats.price_at(20.0)));
    assert_eq!(fitted[1], p(39.0, stats.price_at(39.0)));
    let (start, end) = (to_px(&chart, fitted[0]), to_px(&chart, fitted[1]));
    assert!(
        strokes(&after)
            .iter()
            .any(|stroke| stroke.spans(start, end, 1e-3)),
        "the painted center line follows the updated fit"
    );
}

#[test]
fn regression_trends_ignore_ticks_of_series_they_do_not_measure() {
    let mut chart = trending_chart();
    let pane = chart.add_pane(true).unwrap();
    let other = chart.add_series(crate::SeriesKind::Line);
    let values = (0..40)
        .map(|index| 10.0 + (index % 5) as f64)
        .collect::<Vec<_>>();
    chart
        .set_series_data(other, &hourly(40), &values, &values, &values, &values)
        .unwrap();
    chart.set_series_pane(other, pane, 1.0);
    add(
        &mut chart,
        DrawingKind::RegressionTrend,
        vec![p(20.0, 0.0), p(39.0, 0.0)],
        r##"{"color":"#123456"}"##,
    );
    chart.build_frame();
    chart.build_frame();
    // A value-only tick inside the other series' range (its scale keeps its range).
    let time = 39.0 * HOUR;
    assert!(chart.update_series_bar(other, time, [12.0, 12.0, 12.0, 12.0]));
    chart.build_frame();
    assert_eq!(
        chart.frame_build_stats().drawing_rebuilds,
        0,
        "a tick of a series no drawing reads keeps every retained drawings layer"
    );
    // A tick of the measured source still refits.
    let [open, high, low, close] = ohlc(40);
    assert!(chart.update_series_bar(0, time, [open[39], high[39], low[39], close[39] + 0.5]));
    chart.build_frame();
    assert!(chart.frame_build_stats().drawing_rebuilds >= 1);
}

#[test]
fn chart_magnet_snaps_channel_placement_to_bar_values() {
    let mut chart = chart();
    chart.set_drawing_magnet_mode(DrawingMagnetMode::Strong);
    assert!(chart.set_drawing_tool(Some(DrawingKind::FlatTopChannel), None, None));
    let mut created = None;
    for (logical, price) in [(8.0, 101.3), (16.0, 102.4), (22.0, 104.2)] {
        let x = chart.logical_to_coordinate(logical).unwrap() + 3.0;
        let y = chart.series_price_to_coordinate(0, price).unwrap();
        created = chart
            .drawing_tool_activate(x, y, DrawingModifiers::default())
            .created;
    }
    let points = chart.drawing(created.unwrap()).unwrap().points.clone();
    for (point, logical) in points.iter().zip([8.0, 16.0, 22.0]) {
        assert_eq!(point.logical, logical);
        assert_eq!(point.price, 100.0 + (logical as usize % 7) as f64);
    }
}

#[test]
fn templates_replace_channel_options_including_the_defaults_they_leave_unset() {
    let mut chart = trending_chart();
    let points = default_points(DrawingKind::RegressionTrend);
    let custom = add(
        &mut chart,
        DrawingKind::RegressionTrend,
        points.clone(),
        r#"{"tool_options":{"channel":{"upper_deviation":3,"source":"hl2"}},"width":3}"#,
    );
    let plain = add(
        &mut chart,
        DrawingKind::RegressionTrend,
        points.clone(),
        "{}",
    );
    let partial = add(
        &mut chart,
        DrawingKind::RegressionTrend,
        points,
        r#"{"tool_options":{"channel":{"show_pearsons":false}}}"#,
    );
    let customized = chart.drawing(custom).unwrap().clone();
    let kind_options = |chart: &ChartEngine, id| {
        serde_json::from_str::<serde_json::Value>(&chart.drawing_kind_options_json(id).unwrap())
            .unwrap()
    };
    // A template of a default regression restores every default, family options included.
    let template = chart.drawing_template_json(plain, "Plain").unwrap();
    assert!(chart.apply_drawing_template_json(custom, &template));
    let applied = chart.drawing(custom).unwrap();
    assert!(
        applied.tool_options.is_empty(),
        "{:?}",
        applied.tool_options
    );
    assert_eq!(applied.width, 1.0);
    assert_eq!(kind_options(&chart, custom), kind_options(&chart, plain));
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(custom).unwrap(), &customized);
    // A partly set block replaces the fields rather than merging with them.
    let template = chart.drawing_template_json(partial, "Quiet").unwrap();
    assert!(chart.apply_drawing_template_json(custom, &template));
    assert_eq!(
        chart.drawing(custom).unwrap().tool_options,
        chart.drawing(partial).unwrap().tool_options
    );
    // A host template without `tool_options` leaves them alone, and a template of another
    // channel kind is rejected.
    assert!(chart.apply_drawing_template_json(
        partial,
        r#"{"name":"Wide","kind":"regression_trend","options":{"width":4}}"#
    ));
    let kept = chart.drawing(partial).unwrap();
    assert_eq!((kept.width, kept.tool_options.is_empty()), (4.0, false));
    let channel = add(
        &mut chart,
        DrawingKind::ParallelChannel,
        default_points(DrawingKind::ParallelChannel),
        "{}",
    );
    let template = chart.drawing_template_json(channel, "Channel").unwrap();
    assert!(!chart.apply_drawing_template_json(custom, &template));
}

#[test]
fn clipboard_and_sync_payloads_carry_channel_tool_options() {
    let mut source = trending_chart();
    let points = default_points(DrawingKind::RegressionTrend);
    let id = add(
        &mut source,
        DrawingKind::RegressionTrend,
        points,
        r#"{"tool_options":{"channel":{"upper_deviation":2.5,"source":"hlc3"}},"extend_left":true}"#,
    );
    let copied = source.copy_drawings_json(&[id]).unwrap();
    let mut target = trending_chart();
    let pasted = target.paste_drawings_json(&copied, 0, 0.0, 0.0).unwrap();
    let drawing = target.drawing(pasted[0]).unwrap();
    assert_eq!(drawing.kind, DrawingKind::RegressionTrend);
    assert!(drawing.extend_left && drawing.fill_enabled);
    assert_eq!(
        drawing.tool_options,
        source.drawing(id).unwrap().tool_options
    );

    let payload = source.drawing_sync_payload_json("cell-a").unwrap();
    let mut mirror = trending_chart();
    assert!(mirror.apply_drawing_sync_payload_json(&payload));
    assert_eq!(
        mirror.drawings()[0].tool_options,
        source.drawing(id).unwrap().tool_options
    );
    assert_eq!(
        mirror.drawings()[0].points,
        source.drawing(id).unwrap().points
    );
}

#[test]
fn regressions_fit_the_series_of_their_own_pane() {
    let mut chart = chart();
    let pane = chart.add_pane(true).unwrap();
    let series = chart.add_series(crate::SeriesKind::Line);
    let values = (0..40)
        .map(|index| 10.0 + index as f64 * 0.1 + (index % 3) as f64 * 0.05)
        .collect::<Vec<_>>();
    chart
        .set_series_data(series, &hourly(40), &values, &values, &values, &values)
        .unwrap();
    chart.set_series_pane(series, pane, 1.0);
    chart.build_frame();
    let id = chart
        .add_drawing(
            DrawingKind::RegressionTrend,
            pane,
            vec![p(10.0, 10.5), p(30.0, 11.5)],
            Some(r##"{"color":"#123456"}"##),
        )
        .unwrap();
    let stats = regression_stats(
        &chart,
        chart.drawing(id).unwrap(),
        IndicatorInputSource::Close,
    )
    .unwrap();
    let pairs = (10..=30)
        .map(|index| (index as f64, values[index]))
        .collect::<Vec<_>>();
    assert!((stats.slope - reference_fit(&pairs).0).abs() < 1e-12);
    let frame = chart.build_frame();
    let ink_in = |pane_index: usize| {
        frame.panes[pane_index]
            .main
            .iter()
            .filter(|prim| matches!(prim, Prim::Polyline { color, .. } if *color == ink()))
            .count()
    };
    assert_eq!(ink_in(0), 0);
    assert!(ink_in(pane) > 0);
    let middle = chart
        .drawing_to_px_for(
            pane,
            DrawingPriceScale::Right,
            p(20.0, stats.price_at(20.0)),
        )
        .unwrap();
    assert!(middle.1 > chart.panes[pane].top);
    assert_eq!(hit_id(&chart, middle), Some(id));
}

#[test]
fn regressions_fit_only_the_bars_replay_shows() {
    let mut chart = trending_chart();
    let drawing = crate::Drawing::new(
        1,
        DrawingKind::RegressionTrend,
        0,
        vec![p(5.0, 0.0), p(35.0, 0.0)],
    );
    let full = regression_stats(&chart, &drawing, IndicatorInputSource::Close).unwrap();
    assert_eq!(full.count, 31);
    chart
        .set_replay_clock_micros(Some((20.0 * HOUR) as i64 * 1_000_000))
        .unwrap();
    let replayed = regression_stats(&chart, &drawing, IndicatorInputSource::Close).unwrap();
    assert_eq!(replayed.count, 16, "bars 5..=20 only");
    let close_values = ohlc(40)[3].clone();
    let pairs = (5..=20)
        .map(|index| (index as f64, close_values[index]))
        .collect::<Vec<_>>();
    assert!((replayed.slope - reference_fit(&pairs).0).abs() < 1e-10);
}

#[test]
fn regression_trends_without_a_fit_keep_their_dashed_anchor_segment() {
    let mut chart = chart();
    // Show the empty future after the last bar.
    chart.set_visible_logical_range(20.0, 70.0);
    chart.build_frame();
    let id = add(
        &mut chart,
        DrawingKind::RegressionTrend,
        vec![p(52.0, 101.0), p(60.0, 103.0)],
        r##"{"color":"#123456"}"##,
    );
    assert_eq!(chart.regression_points(chart.drawing(id).unwrap()), None);
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let lines = strokes(&ink_polylines(&mut chart));
    assert_eq!(lines.len(), 1);
    assert!(lines[0].dashed() && lines[0].spans(a, b, 1e-3));
    assert_eq!(
        hit_id(&chart, ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)),
        Some(id)
    );
    // One close in the window is no fit either (upstream's rule): the same placeholder, not a
    // flat center line with zero-width bands.
    assert!(chart.drawing_set_points(
        id,
        &serde_json::to_string(&[p(39.0, 101.0), p(45.0, 103.0)]).unwrap()
    ));
    assert_eq!(chart.regression_points(chart.drawing(id).unwrap()), None);
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let lines = strokes(&ink_polylines(&mut chart));
    assert_eq!(lines.len(), 1);
    assert!(lines[0].dashed() && lines[0].spans(a, b, 1e-3));
    // Two closes fit: the solid center and band lines replace the placeholder.
    assert!(chart.drawing_set_points(
        id,
        &serde_json::to_string(&[p(38.0, 101.0), p(45.0, 103.0)]).unwrap()
    ));
    assert!(
        chart
            .regression_points(chart.drawing(id).unwrap())
            .is_some()
    );
    let lines = strokes(&ink_polylines(&mut chart));
    assert!(!lines.is_empty() && lines.iter().all(|stroke| !stroke.dashed()));
}

#[test]
fn hidden_channels_neither_paint_nor_hit_and_locked_ones_stay_put() {
    let mut chart = chart();
    let points = channel_points(&chart, 90.0);
    let id = add(
        &mut chart,
        DrawingKind::ParallelChannel,
        points.clone(),
        r##"{"color":"#123456"}"##,
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let slope = (b.1 - a.1) / (b.0 - a.0);
    let handle = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0 + 90.0);
    let on_second = (handle.0 + 12.0, handle.1 + 12.0 * slope);
    assert_eq!(hit_id(&chart, on_second), Some(id));
    assert!(chart.set_drawing_visibility(id, false));
    assert!(ink_polylines(&mut chart).is_empty());
    assert!(fills(&mut chart, wash()).is_empty());
    assert_eq!(hit_id(&chart, on_second), None);
    assert!(chart.set_drawing_visibility(id, true));
    assert!(chart.set_drawing_locked(id, true));
    chart.set_selected_drawing(Some(id));
    for grab in [handle, on_second] {
        chart.drawing_drag_start_at(grab.0, grab.1);
        chart.drawing_drag_to(grab.0 + 10.0, grab.1 + 30.0, DrawingModifiers::default());
        chart.drawing_drag_end();
        assert_eq!(chart.drawing(id).unwrap().points, points);
    }
    assert!(!chart.nudge_selected_drawing(0.0, 5.0, Some(2)));
    // Z-order: a channel raised above another wins the shared line.
    let copy = add(&mut chart, DrawingKind::ParallelChannel, points, "{}");
    assert_eq!(hit_id(&chart, on_second), Some(copy));
    assert!(chart.move_drawing_z_order(id, 1));
    assert_eq!(hit_id(&chart, on_second), Some(id));
}

#[test]
fn regression_fits_are_memoized_per_drawing_and_bounded() {
    let mut chart = trending_chart();
    let range = |id: DrawingId, first: f64, second: f64| {
        crate::Drawing::new(
            id,
            DrawingKind::RegressionTrend,
            0,
            vec![p(first, 0.0), p(second, 0.0)],
        )
    };
    let memo = |chart: &ChartEngine| chart.drawing_settings.regression_memo.borrow().len();
    let passes = |chart: &ChartEngine| chart.drawing_settings.regression_memo.borrow().passes;
    let drawing = range(1, 4.0, 30.0);
    let first = regression_stats(&chart, &drawing, IndicatorInputSource::Close).unwrap();
    assert_eq!((memo(&chart), passes(&chart)), (1, 1));
    // Panning and pointer hit tests reuse the fit.
    assert_eq!(
        regression_stats(&chart, &drawing, IndicatorInputSource::Close),
        Some(first)
    );
    assert_eq!((memo(&chart), passes(&chart)), (1, 1));
    // Another source or range of the same drawing replaces its one entry.
    regression_stats(&chart, &drawing, IndicatorInputSource::High).unwrap();
    assert_eq!((memo(&chart), passes(&chart)), (1, 2));
    // Fits of drawings the chart does not hold stay bounded by the slack.
    for id in 10..60 {
        regression_stats(&chart, &range(id, 2.0, 20.0), IndicatorInputSource::Close);
        assert!(memo(&chart) <= chart.drawings().len() + crate::drawings::DRAWING_MEMO_SLACK);
    }
    // A data change re-fits even when the row count is unchanged.
    let [open, high, low, mut close_values] = ohlc(40);
    close_values[20] += 25.0;
    chart
        .set_series_data(0, &hourly(40), &open, &high, &low, &close_values)
        .unwrap();
    let refit = regression_stats(&chart, &drawing, IndicatorInputSource::Close).unwrap();
    assert!(refit.mean_price > first.mean_price);
    let pairs = (4..=30)
        .map(|index| (index as f64, close_values[index]))
        .collect::<Vec<_>>();
    assert!((refit.slope - reference_fit(&pairs).0).abs() < 1e-10);
}

#[test]
fn many_regressions_fit_once_across_pans_hover_and_live_ticks() {
    let mut chart = trending_chart();
    // More regressions over distinct ranges than a fixed-slot cache would hold.
    let count = 3 * crate::drawings::DRAWING_MEMO_SLACK;
    for index in 0..count {
        let first = (index % 8) as f64;
        let second = 20.0 + (index / 8) as f64 + 0.25 * (index % 8) as f64;
        add(
            &mut chart,
            DrawingKind::RegressionTrend,
            vec![p(first, 0.0), p(second, 0.0)],
            "{}",
        );
    }
    let passes = |chart: &ChartEngine| chart.drawing_settings.regression_memo.borrow().passes;
    chart.build_frame();
    let fitted = passes(&chart);
    assert!(fitted >= count, "{fitted}");
    for shift in [0.5, -0.5, 1.5] {
        chart.set_visible_logical_range(-2.0 + shift, 42.0 + shift);
        chart.build_frame();
        for gy in 0..12 {
            for gx in 0..20 {
                chart.hit_test_drawing(f64::from(gx) * 40.0 + 7.0, f64::from(gy) * 40.0 + 9.0);
            }
        }
    }
    assert_eq!(passes(&chart), fitted, "pans and hover reuse every fit");
    // A streaming update of the latest bar extends each fit by the changed row: no fit repeats
    // its pass over the range.
    let [open, high, low, close_values] = ohlc(40);
    assert!(chart.update_series_bar(
        0,
        39.0 * HOUR,
        [open[39], high[39] + 2.0, low[39], close_values[39] + 2.0]
    ));
    chart.build_frame();
    assert_eq!(passes(&chart), fitted);
    assert!(chart.drawing_settings.regression_memo.borrow().len() <= count + 1);
}

/// The fit a full pass gives for `drawing`'s range (a drawing id the memo has not seen).
fn fresh_fit(chart: &ChartEngine, drawing: &crate::Drawing) -> Option<super::RegressionStats> {
    let mut probe = drawing.clone();
    probe.id = 1_000_000 + drawing.id;
    regression_stats(chart, &probe, IndicatorInputSource::Close)
}

#[test]
fn live_bars_extend_regression_fits_without_a_full_pass() {
    let mut chart = trending_chart();
    // Covering the latest bar, reaching into the future, and ending before the latest bar.
    for (first, second) in [(5.0, 39.0), (5.0, 60.0), (5.0, 20.0)] {
        add(
            &mut chart,
            DrawingKind::RegressionTrend,
            vec![p(first, 0.0), p(second, 0.0)],
            "{}",
        );
    }
    let passes = |chart: &ChartEngine| chart.drawing_settings.regression_memo.borrow().passes;
    let fits = |chart: &mut ChartEngine| {
        chart.build_frame();
        chart
            .drawings()
            .iter()
            .map(|drawing| regression_stats(chart, drawing, IndicatorInputSource::Close))
            .collect::<Vec<_>>()
    };
    let assert_fresh = |chart: &ChartEngine, fits: &[Option<super::RegressionStats>]| {
        for (drawing, fit) in chart.drawings().iter().zip(fits) {
            let fresh = fresh_fit(chart, drawing);
            assert_eq!(
                fit.map(|stats| format!("{stats:?}")),
                fresh.map(|stats| format!("{stats:?}")),
                "bitwise equal to a full pass"
            );
        }
    };
    fits(&mut chart);

    // Replacing the latest bar, twice, then appending bars (the future range takes them in).
    let [open, high, low, close] = ohlc(40);
    for bump in [1.5, -0.75] {
        let before = passes(&chart);
        assert!(chart.update_series_bar(
            0,
            39.0 * HOUR,
            [open[39], high[39] + bump, low[39], close[39] + bump]
        ));
        let live = fits(&mut chart);
        assert_eq!(
            passes(&chart),
            before,
            "a tip replacement runs no full pass"
        );
        assert_fresh(&chart, &live);
    }
    for row in 40..44 {
        let before = passes(&chart);
        let value = 110.0 + row as f64 * 0.3;
        assert!(chart.update_series_bar(
            0,
            row as f64 * HOUR,
            [value, value + 1.0, value - 1.0, value + 0.2]
        ));
        let live = fits(&mut chart);
        assert_eq!(passes(&chart), before, "an append runs no full pass");
        assert_fresh(&chart, &live);
    }
    assert_eq!(
        regression_stats(&chart, &chart.drawings()[1], IndicatorInputSource::Close)
            .unwrap()
            .count,
        39,
        "the future range took the appended bars in"
    );

    // A correction inside every range refits each one in full.
    let before = passes(&chart);
    assert!(chart.update_series_bar(0, 10.0 * HOUR, [100.0, 130.0, 99.0, 128.0]));
    let live = fits(&mut chart);
    assert_eq!(passes(&chart), before + 3);
    assert_fresh(&chart, &live);

    // A time point inserted inside the axis by another series moves the fits' positions.
    let before = passes(&chart);
    let other = chart.add_series(crate::SeriesKind::Line);
    chart
        .set_series_data(
            other,
            &[12.5 * HOUR],
            &[100.0],
            &[100.0],
            &[100.0],
            &[100.0],
        )
        .unwrap();
    let live = fits(&mut chart);
    assert_eq!(passes(&chart), before + 3);
    assert_fresh(&chart, &live);
    let inserted = passes(&chart);

    // Retention trimming the head moves every position too.
    assert!(chart.set_series_max_points(0, Some(30)));
    let live = fits(&mut chart);
    assert!(passes(&chart) > inserted);
    assert_fresh(&chart, &live);
}

/// Centers of the selection handles painted in the first pane (outer disc of each pair).
fn painted_handles(chart: &mut ChartEngine) -> Vec<(f64, f64)> {
    let frame = chart.build_frame();
    let circles = frame.panes[0]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::Circle { cx, cy, .. } => Some((f64::from(*cx), f64::from(*cy))),
            _ => None,
        })
        .collect::<Vec<_>>();
    circles.into_iter().step_by(2).collect()
}

#[test]
fn channel_handles_sit_on_the_painted_lines_and_drive_their_anchors() {
    let mut chart = chart();
    // The third anchor lies right of the base line's bars: its handle is the second line's
    // midpoint, not the off-line click. A shallow base keeps every second line in the pane.
    let (a, b) = (p(10.0, 101.0), p(20.0, 102.0));
    let (ax, ay) = to_px(&chart, a);
    let (bx, by) = to_px(&chart, b);
    let cx = to_px(&chart, p(28.0, 0.0)).0;
    let c = from_px(&chart, cx, ay + (by - ay) * (cx - ax) / (bx - ax) + 70.0);
    // The price channel is the family's channel since the upstream catalog took over the
    // parallel, flat, and disjoint channels.
    {
        let kind = DrawingKind::PriceChannel;
        let id = add(&mut chart, kind, vec![a, b, c], "{}");
        chart.set_selected_drawing(Some(id));
        let third = anchor(&chart, id, 2);
        let middle_x = (ax + bx) / 2.0;
        let middle_y = (ay + by) / 2.0 + 70.0;
        let expected = [(ax, ay), (bx, by), (middle_x, middle_y)];
        let painted = painted_handles(&mut chart);
        assert_eq!(painted.len(), 3, "{kind:?}");
        for (painted, expected) in painted.iter().zip(expected) {
            assert!(close(*painted, expected, 1e-3), "{kind:?} {painted:?}");
        }
        assert_eq!(
            chart
                .hit_test_drawing(middle_x, middle_y)
                .map(|hit| hit.part),
            Some(DrawingDragPart::Anchor(2)),
            "{kind:?}"
        );
        assert_ne!(
            chart.hit_test_drawing(third.0, third.1).map(|hit| hit.part),
            Some(DrawingDragPart::Anchor(2)),
            "{kind:?}: nothing to grab at the off-line click"
        );

        // Dragging the handle down moves the second line with the pointer and nothing else.
        assert!(chart.drawing_drag_start_at(middle_x, middle_y));
        chart.drawing_drag_to(middle_x, middle_y + 25.0, DrawingModifiers::default());
        chart.drawing_drag_end();
        let points = chart.drawing(id).unwrap().points.clone();
        assert_eq!(points[..2], [a, b]);
        let handle = painted_handles(&mut chart)[2];
        assert!(
            close(handle, (middle_x, middle_y + 25.0), 1e-3),
            "{kind:?} {handle:?}"
        );
        // The keyboard reaches the same handle from its painted position.
        assert!(chart.nudge_selected_drawing(0.0, -5.0, Some(2)));
        assert!(close(
            painted_handles(&mut chart)[2],
            (middle_x, middle_y + 20.0),
            1e-3
        ));
        assert!(chart.undo_drawing() && chart.undo_drawing());
        assert_eq!(chart.drawing(id).unwrap().points, [a, b, c]);
        chart.remove_drawing(id);
    }
}

#[test]
fn dashed_and_dotted_channel_lines_reach_the_frame_as_solid_dash_runs() {
    // Executors without a dash concept (the WebGPU tessellator) paint exactly these runs, so the
    // gaps match Canvas2D's dashes by construction. The width is explicit (new price channels
    // default to 1 px) so the three 2 px boundaries stand apart from the 1 px middle line.
    for (style, on, off) in [("dashed", 12.0, 12.0), ("dotted", 2.0, 8.0)] {
        let mut chart = chart();
        let points = channel_points(&chart, 90.0);
        let id = add(
            &mut chart,
            DrawingKind::PriceChannel,
            points,
            &format!(
                r##"{{"color":"#123456","width":2,"style":"{style}","extend_left":false,"extend_right":false}}"##
            ),
        );
        let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
        let lines = ink_polylines(&mut chart);
        let all = strokes(&lines);
        let boundaries = all
            .iter()
            .filter(|stroke| stroke.width == 2.0)
            .collect::<Vec<_>>();
        assert_eq!(boundaries.len(), 3, "{style}");
        let base = boundaries[0];
        assert!(base.dashed() && base.spans(a, b, 1e-3), "{style}");
        let length = |from: (f64, f64), to: (f64, f64)| (to.0 - from.0).hypot(to.1 - from.1);
        for pair in base.runs.windows(2) {
            assert!(
                (length(pair[0][0], pair[0][1]) - on).abs() < 1e-3,
                "{style}"
            );
            assert!(
                (length(pair[0][1], pair[1][0]) - off).abs() < 1e-3,
                "{style}"
            );
        }
        // The body stays continuous for the pointer: a gap between dashes still hits.
        let gap = base.runs[0][1];
        let direction = ((b.0 - a.0) / length(a, b), (b.1 - a.1) / length(a, b));
        let inside_gap = (
            gap.0 + direction.0 * off / 2.0,
            gap.1 + direction.1 * off / 2.0,
        );
        assert_eq!(hit_id(&chart, inside_gap), Some(id), "{style}");
    }
}

#[test]
fn three_click_placement_previews_the_base_line_then_the_channel() {
    let mut chart = chart();
    let [a, b, c] =
        [p(10.0, 101.0), p(20.0, 104.0), p(15.0, 102.0)].map(|point| to_px(&chart, point));
    {
        let kind = DrawingKind::PriceChannel;
        assert!(chart.set_drawing_tool(
            Some(kind),
            Some(r##"{"color":"#123456","fill_enabled":true,"extend_left":false,"extend_right":false}"##),
            None
        ));
        chart.drawing_tool_activate(a.0, a.1, DrawingModifiers::default());
        // Placing the second anchor: the base line follows the pointer, with the first anchor's
        // handle and no fill yet.
        chart.drawing_tool_pointer_move(b.0, b.1, DrawingModifiers::default(), false);
        let lines = strokes(&ink_polylines(&mut chart));
        assert_eq!(lines.len(), 1, "{kind:?}");
        assert!(lines[0].spans(a, b, 1e-3), "{kind:?}");
        assert!(fills(&mut chart, wash()).is_empty());
        let handles = painted_handles(&mut chart);
        assert!(handles.len() == 1 && close(handles[0], a, 1e-3), "{kind:?}");
        // Placing the third: the whole channel previews through the pointer.
        chart.drawing_tool_activate(b.0, b.1, DrawingModifiers::default());
        chart.drawing_tool_pointer_move(c.0, c.1 + 60.0, DrawingModifiers::default(), false);
        let (boundaries, _) = split_lines(&ink_polylines(&mut chart));
        assert_eq!(boundaries.len(), 3, "{kind:?}");
        assert!(!fills(&mut chart, wash()).is_empty(), "{kind:?}");
        let handles = painted_handles(&mut chart);
        assert_eq!(handles.len(), 2, "{kind:?}");
        assert!(close(handles[0], a, 1e-3) && close(handles[1], b, 1e-3));
        let created = chart
            .drawing_tool_activate(c.0, c.1 + 60.0, DrawingModifiers::default())
            .created
            .unwrap();
        chart.remove_drawing(created);
    }
}

#[test]
fn dashed_lines_reaching_far_off_screen_split_only_their_visible_length() {
    let mut chart = chart();
    // The base starts 50,000 bars left of the pane: its dashed middle line is millions of px long.
    let (a, b) = (p(-50_000.0, 101.0), p(20.0, 101.5));
    let (ax, ay) = to_px(&chart, a);
    let (bx, by) = to_px(&chart, b);
    let c = from_px(&chart, bx, by + 60.0);
    add(
        &mut chart,
        DrawingKind::PriceChannel,
        vec![a, b, c],
        r##"{"color":"#123456","extend_left":false,"extend_right":false,"tool_options":{"channel":{"middle_line":true}}}"##,
    );
    let (_, middles) = split_lines(&ink_polylines(&mut chart));
    let middle = &middles[0];
    let visible = chart.pane_w.hypot(chart.panes[0].height);
    assert!(
        middle.runs.len() as f64 <= visible / 12.0 + 3.0,
        "{} runs",
        middle.runs.len()
    );
    // Every dash sits where the unclipped line's pattern puts it: whole 12 px periods from the
    // middle line's true start.
    let start = (ax, ay + 30.0);
    let end = (bx, by + 30.0);
    assert!(middle.spans(middle.start(), end, 1e-2));
    for run in &middle.runs {
        let from_start = (run[0].0 - start.0).hypot(run[0].1 - start.1) / 12.0;
        assert!(
            (from_start - from_start.round()).abs() < 1e-3,
            "{from_start}"
        );
    }
}

#[test]
fn regressions_on_an_as_of_source_fit_its_own_bars_once_each() {
    const DAY: f64 = 86_400.0;
    let days = |list: &[f64]| list.iter().map(|day| day * DAY).collect::<Vec<_>>();
    let mut chart = ChartEngine::new(800.0, 400.0, 1.0);
    let hk = [100.0, 102.0, 101.0, 105.0, 104.0];
    chart
        .set_series_data(0, &days(&[1.0, 2.0, 3.0, 5.0, 6.0]), &hk, &hk, &hk, &hk)
        .unwrap();
    let us = chart.add_series(crate::SeriesKind::Line);
    let us_close = [4000.0, 4040.0, 4100.0, 4120.0, 4200.0];
    chart
        .set_series_data(
            us,
            &days(&[1.0, 2.0, 4.0, 5.0, 7.0]),
            &us_close,
            &us_close,
            &us_close,
            &us_close,
        )
        .unwrap();
    chart.set_series_pane(us, 1, 1.0);
    chart
        .set_series_time_alignment(
            us,
            crate::TimeAlignment::AsOf {
                max_staleness: None,
            },
        )
        .unwrap();
    let drawing = crate::Drawing::new(
        1,
        DrawingKind::RegressionTrend,
        1,
        vec![p(0.0, 0.0), p(4.0, 0.0)],
    );
    assert_eq!(chart.drawing_source_series(&drawing), Some(us));
    let stats = regression_stats(&chart, &drawing, IndicatorInputSource::Close).unwrap();
    // The overlay's own bars d1, d2, d4, d5 (d4 sits at the d5 point, 3; d7 waits past d6).
    assert_eq!(stats.count, 4);
    let (slope, (mean_x, mean_y), deviation, pearson) =
        reference_fit(&[(0.0, 4000.0), (1.0, 4040.0), (3.0, 4100.0), (3.0, 4120.0)]);
    assert!((stats.slope - slope).abs() < 1e-9);
    assert!((stats.mean_logical - mean_x).abs() < 1e-9);
    assert!((stats.mean_price - mean_y).abs() < 1e-9);
    assert!((stats.deviation - deviation).abs() < 1e-9);
    assert!((stats.pearson.unwrap() - pearson).abs() < 1e-9);
}

/// KLineChart's `priceChannelLine`: the base line is the channel's centre, with its parallel
/// through the third anchor and the mirror of that parallel on the other side.
#[test]
fn price_channels_are_symmetric_about_their_base_line_and_bare_by_default() {
    let kind = DrawingKind::PriceChannel;
    let spec = kind.spec();
    assert_eq!((spec.wire_id, spec.name), (244, "price_channel"));
    assert_eq!(DrawingKind::from_u8(244), Some(kind));
    assert_eq!(DrawingKind::from_name("price_channel"), Some(kind));
    assert!(std::ptr::eq(spec.family.unwrap(), &FAMILY));
    assert_eq!(spec.placement, DrawingPlacement::ClickAnchors { count: 3 });
    let drawing = crate::Drawing::new(1, kind, 0, Vec::new());
    // Three bare lines across the pane: no fill, no middle line, extended both ways.
    assert!(!drawing.fill_enabled);
    assert!(drawing.extend_left && drawing.extend_right);

    let mut chart = chart();
    let points = channel_points(&chart, 80.0);
    let id = add(&mut chart, kind, points, r##"{"color":"#123456"}"##);
    let (a, b, c) = (
        anchor(&chart, id, 0),
        anchor(&chart, id, 1),
        anchor(&chart, id, 2),
    );
    let (boundaries, middles) = split_lines(&ink_polylines(&mut chart));
    assert_eq!(
        (boundaries.len(), middles.len()),
        (3, 0),
        "base, parallel and mirror"
    );
    let lines = boundaries.iter().map(Stroke::line).collect::<Vec<_>>();
    // The first line is the base; the second passes through the third anchor, parallel to it;
    // the third is the same distance on the other side.
    assert!(collinear(lines[0][0], lines[0][1], a) && collinear(lines[0][0], lines[0][1], b));
    assert!(collinear(lines[1][0], lines[1][1], c));
    let base_slope = slope(&lines[0]);
    assert!((slope(&lines[1]) - base_slope).abs() < 1e-6);
    assert!((slope(&lines[2]) - base_slope).abs() < 1e-6);
    let at = |line: &[(f64, f64); 2], x: f64| line[0].1 + base_slope * (x - line[0].0);
    let x = 400.0;
    let (middle, parallel, mirror) = (at(&lines[0], x), at(&lines[1], x), at(&lines[2], x));
    assert!(
        ((parallel - middle) + (mirror - middle)).abs() < 1e-3,
        "the parallel and the mirror sit on opposite sides at the same distance"
    );
    assert!((parallel - middle).abs() > 10.0);
    // All three reach the pane edges at both ends (extended both ways by default).
    let (top, bottom) = (
        chart.panes[0].top,
        chart.panes[0].top + chart.panes[0].height,
    );
    let on_edge = |point: (f64, f64)| {
        point.0.abs() < 0.5
            || (point.0 - chart.pane_w).abs() < 0.5
            || (point.1 - top).abs() < 0.5
            || (point.1 - bottom).abs() < 0.5
    };
    for line in &lines {
        assert!(on_edge(line[0]) && on_edge(line[1]), "{line:?}");
    }
    // The mirror is a body target too, far from the anchors.
    let y = at(&lines[2], x);
    assert_eq!(hit_id(&chart, (x, y)), Some(id));
}

#[test]
fn vertical_price_channels_step_horizontally_and_fill_the_whole_band_when_enabled() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::PriceChannel,
        vec![p(12.0, 101.0), p(12.0, 106.0), p(10.0, 103.0)],
        r##"{"color":"#123456"}"##,
    );
    let (a, c) = (anchor(&chart, id, 0), anchor(&chart, id, 2));
    let (boundaries, _) = split_lines(&ink_polylines(&mut chart));
    assert_eq!(boundaries.len(), 3);
    let xs = boundaries
        .iter()
        .map(|line| {
            let [start, end] = line.line();
            assert!((start.0 - end.0).abs() < 1e-6, "vertical: {line:?}");
            start.0
        })
        .collect::<Vec<_>>();
    assert!((xs[0] - a.0).abs() < 1e-6 && (xs[1] - c.0).abs() < 1e-6);
    assert!(
        (xs[2] - (2.0 * a.0 - c.0)).abs() < 1e-6,
        "mirrored about the base"
    );

    // Two anchors preview the base line alone.
    let mut chart = super::tests::chart();
    assert!(chart.set_drawing_tool(
        Some(DrawingKind::PriceChannel),
        Some(r##"{"color":"#123456"}"##),
        None
    ));
    chart.drawing_tool_activate(200.0, 260.0, DrawingModifiers::default());
    chart.drawing_tool_activate(420.0, 150.0, DrawingModifiers::default());
    chart.drawing_tool_pointer_move(300.0, 330.0, DrawingModifiers::default(), false);
    let (preview, _) = split_lines(&ink_polylines(&mut chart));
    assert_eq!(
        preview.len(),
        3,
        "the pending channel previews all three lines"
    );

    // Enabling the fill shades the whole band, mirror to parallel.
    let mut chart = super::tests::chart();
    let points = channel_points(&chart, 80.0);
    let id = add(
        &mut chart,
        DrawingKind::PriceChannel,
        points,
        r##"{"color":"#123456","fill_enabled":true}"##,
    );
    let c = anchor(&chart, id, 2);
    let (boundaries, _) = split_lines(&ink_polylines(&mut chart));
    let mirror = boundaries[2].line();
    let regions = fills(&mut chart, wash());
    assert_eq!(regions.len(), 1, "one convex region");
    assert!(
        point_in_polygon((c.0, c.1 - 10.0), &regions[0]),
        "covers the base-to-parallel half"
    );
    let probe_x = 400.0;
    // The band lies between the mirror (above the base) and the parallel (below it), so a point
    // just under the mirror line is inside it.
    let probe_y = mirror[0].1 + slope(&mirror) * (probe_x - mirror[0].0) + 10.0;
    assert!(
        point_in_polygon((probe_x, probe_y), &regions[0]),
        "and the mirror half too"
    );
}

#[test]
fn channel_extensions_run_both_lines_and_the_fill_to_the_pane_edges() {
    let cases = [
        (
            DrawingKind::ParallelChannel,
            vec![p(10.0, 101.0), p(20.0, 104.0), p(15.0, 101.5)],
        ),
        (
            DrawingKind::FlatTopChannel,
            vec![p(10.0, 101.0), p(20.0, 103.0), p(15.0, 105.0)],
        ),
        (
            DrawingKind::FlatBottomChannel,
            vec![p(10.0, 103.0), p(20.0, 105.0), p(15.0, 100.5)],
        ),
        (
            DrawingKind::DisjointChannel,
            vec![
                p(10.0, 101.0),
                p(20.0, 104.0),
                p(10.0, 100.0),
                p(20.0, 101.0),
            ],
        ),
    ];
    for (kind, points) in cases {
        let mut chart = chart();
        let id = add(&mut chart, kind, points, r##"{"color":"#123456"}"##);
        let width = chart.pane_w;
        let reach = |chart: &mut ChartEngine| {
            ink_polylines(chart)
                .iter()
                .flat_map(|(points, ..)| points.iter().map(|point| point.0))
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| {
                    (lo.min(x), hi.max(x))
                })
        };
        let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
        let (left, right) = reach(&mut chart);
        assert!(
            left >= a.0 - 1.0 && right <= b.0 + 1.0,
            "{kind:?} stays between its anchors"
        );
        // Beyond the first anchor on the first line: nothing until the line extends there.
        let slope = (b.1 - a.1) / (b.0 - a.0);
        let beyond = (a.0 - 60.0, a.1 - 60.0 * slope);
        assert_eq!(hit_id(&chart, beyond), None, "{kind:?}");
        assert!(chart.drawing_apply_options(id, r#"{"extend_left":true,"extend_right":true}"#));
        let (left, right) = reach(&mut chart);
        assert!(
            left <= 1.0 && right >= width - 1.0,
            "{kind:?}: {left}..{right}"
        );
        assert_eq!(
            hit_id(&chart, beyond),
            Some(id),
            "{kind:?} hits along the extension"
        );
        // The fill follows the lines to the edges.
        assert!(chart.drawing_apply_options(id, r#"{"fill_enabled":true}"#));
        let frame = chart.build_frame();
        let filled_left = frame.panes[0].main.iter().any(|prim| match prim {
            Prim::BandFill { upper_first, .. } => {
                f64::from(frame.panes[0].points[*upper_first as usize][0]) <= 1.0
            }
            _ => false,
        });
        assert!(filled_left, "{kind:?} fills to the left edge");
    }
}

/// Text runs of the first pane: text, position, and alignment.
fn texts(chart: &mut ChartEngine) -> Vec<(String, (f64, f64), TextAlign)> {
    let frame = chart.build_frame();
    frame.panes[0]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::Text {
                text, x, y, align, ..
            } => Some((text.clone(), (f64::from(*x), f64::from(*y)), *align)),
            _ => None,
        })
        .collect()
}

/// A regression's 14% default zone wash over the test ink.
fn zone() -> Color {
    Color::rgba(0x12, 0x34, 0x56, 35)
}

/// Population residual deviation of `stats` (the unit of a regression's band).
fn population(stats: super::RegressionStats) -> f64 {
    let count = stats.count as f64;
    stats.deviation * ((count - 1.0) / count).sqrt()
}

/// Media px of the point `deviations` population deviations off `stats`' fit at `logical`.
fn on_fit(
    chart: &ChartEngine,
    stats: super::RegressionStats,
    logical: f64,
    deviations: f64,
) -> (f64, f64) {
    to_px(
        chart,
        p(
            logical,
            stats.price_at(logical) + deviations * population(stats),
        ),
    )
}

fn fit(chart: &ChartEngine, id: DrawingId, source: IndicatorInputSource) -> super::RegressionStats {
    regression_stats(chart, chart.drawing(id).unwrap(), source).unwrap()
}

/// The property schema lists the `tool_options.channel` rows each upstream channel kind reads:
/// the middle line on every channel, and the regression trend's band, source, and Pearson's R
/// rows on it alone, all at upstream's defaults.
#[test]
fn upstream_channel_schemas_list_the_channel_rows_they_read() {
    use crate::{DrawingPropertyType, drawing_property_schema};
    let rows = |kind: DrawingKind| {
        drawing_property_schema(kind)
            .properties
            .into_iter()
            .filter(|row| row.name.starts_with("tool_options.channel."))
            .map(|row| {
                let name = row.name["tool_options.channel.".len()..].to_string();
                (name, row)
            })
            .collect::<Vec<_>>()
    };
    let middle = [
        (
            "middle_line",
            DrawingPropertyType::Boolean,
            serde_json::json!(false),
        ),
        (
            "middle_color",
            DrawingPropertyType::Color,
            serde_json::json!(""),
        ),
    ];
    for kind in [
        DrawingKind::ParallelChannel,
        DrawingKind::FlatTopChannel,
        DrawingKind::FlatBottomChannel,
        DrawingKind::DisjointChannel,
    ] {
        let rows = rows(kind);
        assert_eq!(rows.len(), middle.len(), "{kind:?}: {rows:?}");
        for ((name, row), (expected, property_type, default)) in rows.iter().zip(&middle) {
            assert_eq!(
                (name.as_str(), row.property_type, &row.default),
                (*expected, *property_type, default),
                "{kind:?}"
            );
        }
    }
    let rows = rows(DrawingKind::RegressionTrend);
    let names = rows
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "middle_line",
            "middle_color",
            "upper_deviation",
            "lower_deviation",
            "use_upper_deviation",
            "use_lower_deviation",
            "source",
            "show_pearsons",
        ]
    );
    let row = |name: &str| &rows.iter().find(|(row, _)| row == name).unwrap().1;
    for (name, (expected, property_type, default)) in names.iter().zip(&middle) {
        assert_eq!(name, expected);
        assert_eq!(
            (row(name).property_type, &row(name).default),
            (*property_type, default)
        );
    }
    for name in ["upper_deviation", "lower_deviation"] {
        let row = row(name);
        assert_eq!(row.property_type, DrawingPropertyType::Number);
        assert_eq!(row.default, serde_json::Value::Null);
        assert_eq!((row.min, row.max), (Some(-100.0), Some(100.0)));
    }
    for (name, default) in [
        ("use_upper_deviation", true),
        ("use_lower_deviation", true),
        ("show_pearsons", false),
    ] {
        assert_eq!(row(name).property_type, DrawingPropertyType::Boolean);
        assert_eq!(row(name).default, serde_json::json!(default), "{name}");
    }
    let source = row("source");
    assert_eq!(source.property_type, DrawingPropertyType::Enum);
    assert_eq!(source.default, serde_json::json!("close"));
    for value in [
        "open", "high", "low", "close", "hl2", "hlc3", "ohlc4", "hlcc4",
    ] {
        assert!(
            source.enum_values.iter().any(|item| item == value),
            "{value}"
        );
    }
}

/// Upstream's parallel, flat, and disjoint channels paint the dashed middle line of their stored
/// `middle_line` (off by default, upstream's look) between their two lines, under them, in
/// `middle_color`, and run it to the pane edges with their extensions.
#[test]
fn upstream_channels_paint_a_dashed_middle_line_when_enabled() {
    let mut chart = chart();
    let cases = [
        (
            DrawingKind::ParallelChannel,
            vec![p(10.0, 101.0), p(20.0, 104.0), p(15.0, 101.5)],
        ),
        (
            DrawingKind::FlatTopChannel,
            vec![p(10.0, 101.0), p(20.0, 103.0), p(15.0, 105.0)],
        ),
        (
            DrawingKind::FlatBottomChannel,
            vec![p(10.0, 103.0), p(20.0, 105.0), p(15.0, 100.5)],
        ),
        (
            DrawingKind::DisjointChannel,
            vec![
                p(10.0, 101.0),
                p(20.0, 104.0),
                p(10.0, 100.0),
                p(20.0, 99.0),
            ],
        ),
    ];
    for (kind, points) in cases {
        let id = add(
            &mut chart,
            kind,
            points,
            r##"{"color":"#123456","width":2,"fill_enabled":false}"##,
        );
        let (boundaries, middles) = split_lines(&ink_polylines(&mut chart));
        assert_eq!((boundaries.len(), middles.len()), (2, 0), "{kind:?}");
        let (first, second) = (boundaries[0].line(), boundaries[1].line());
        let halfway = [0, 1].map(|end| {
            (
                (first[end].0 + second[end].0) / 2.0,
                (first[end].1 + second[end].1) / 2.0,
            )
        });
        let probe = (
            (halfway[0].0 + halfway[1].0) / 2.0,
            (halfway[0].1 + halfway[1].1) / 2.0,
        );
        assert_eq!(hit_id(&chart, probe), None, "{kind:?}");

        assert!(
            chart.drawing_apply_options(id, r#"{"tool_options":{"channel":{"middle_line":true}}}"#)
        );
        let lines = ink_polylines(&mut chart);
        let (boundaries, middles) = split_lines(&lines);
        assert_eq!((boundaries.len(), middles.len()), (2, 1), "{kind:?}");
        assert_eq!(boundaries[0].line(), first, "{kind:?}: the lines stay");
        assert_eq!(middles[0].width, 1.0);
        assert!(
            middles[0].spans(halfway[0], halfway[1], 1e-3),
            "{kind:?} {:?}",
            middles[0]
        );
        // Under the lines: fill, middle, first, second.
        assert_eq!(lines[0].1, 1.0, "{kind:?} paints its middle first");
        // A body target while unselected, like the lines.
        assert_eq!(
            chart
                .hit_test_drawing(probe.0, probe.1)
                .map(|hit| (hit.id, hit.part)),
            Some((id, DrawingDragPart::Body)),
            "{kind:?}"
        );
        assert_eq!(
            chart.hit_test_drawing(probe.0, probe.1),
            chart.hit_test_drawing_bruteforce(probe.0, probe.1)
        );

        // `middle_color` recolours the middle line alone.
        assert!(chart.drawing_apply_options(
            id,
            r##"{"tool_options":{"channel":{"middle_color":"#e91e63"}}}"##
        ));
        let pink = strokes(&color_polylines(
            &mut chart,
            Color::parse_css("#e91e63").unwrap(),
        ));
        assert!(pink.len() == 1 && pink[0].dashed(), "{kind:?}");
        assert_eq!(strokes(&ink_polylines(&mut chart)).len(), 2);

        // The extensions run it with the lines to where they leave the pane (its dashes are
        // clipped to the pane, so a steep line stops at the top or bottom edge).
        assert!(chart.drawing_apply_options(id, r#"{"extend_left":true,"extend_right":true}"#));
        let reach = color_polylines(&mut chart, Color::parse_css("#e91e63").unwrap())
            .iter()
            .flat_map(|(points, ..)| points.iter().map(|point| point.0))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| {
                (lo.min(x), hi.max(x))
            });
        assert!(
            reach.0 < halfway[0].0 - 50.0 && reach.1 > halfway[1].0 + 50.0,
            "{kind:?}: {reach:?} beyond {halfway:?}"
        );
        assert!(chart.remove_drawing(id));
    }
}

/// A disjoint channel fills with one band between its lines, its ends paired by side (owner
/// decision C3): a second line that runs opposite to the first still fills the whole quad (the
/// former two triangles left its right-hand wedge unpainted and hit an hourglass), and lines that
/// cross fill two lobes meeting at the crossing. The selected fill hits exactly where it paints.
#[test]
fn disjoint_channels_fill_and_hit_the_region_between_their_lines() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::DisjointChannel,
        vec![
            p(10.0, 101.0),
            p(20.0, 101.0),
            p(20.0, 104.0),
            p(10.0, 104.0),
        ],
        r##"{"color":"#123456"}"##,
    );
    let (a, b, c) = (
        anchor(&chart, id, 0),
        anchor(&chart, id, 1),
        anchor(&chart, id, 2),
    );
    let middle_y = (a.1 + c.1) / 2.0;
    let quarters = [
        (a.0 + 10.0, middle_y),
        (b.0 - 10.0, middle_y),
        ((a.0 + b.0) / 2.0, c.1 + 10.0),
        ((a.0 + b.0) / 2.0, a.1 - 10.0),
    ];
    let regions = fills(&mut chart, wash());
    assert_eq!(regions.len(), 1);
    for probe in quarters {
        assert!(point_in_polygon(probe, &regions[0]), "{probe:?}");
        assert_eq!(hit_id(&chart, probe), None, "unselected: the pane pans");
    }
    chart.set_selected_drawing(Some(id));
    for probe in quarters {
        assert_eq!(
            chart
                .hit_test_drawing(probe.0, probe.1)
                .map(|hit| (hit.id, hit.part)),
            Some((id, DrawingDragPart::Body)),
            "{probe:?}"
        );
    }

    // Crossing lines: two lobes that meet where the lines cross; nothing beyond the crossing
    // between the lines' outer ends.
    assert!(
        chart
            .set_drawing_anchors(
                id,
                &[
                    p(10.0, 101.0).into(),
                    p(20.0, 105.0).into(),
                    p(10.0, 104.0).into(),
                    p(20.0, 102.0).into(),
                ],
            )
            .is_ok()
    );
    let px = [0, 1, 2, 3].map(|index| anchor(&chart, id, index));
    let as_f32 = |point: (f64, f64)| [point.0 as f32, point.1 as f32];
    let crossing = aeris_charts_render::line::band_crossing(
        as_f32(px[0]),
        as_f32(px[1]),
        as_f32(px[2]),
        as_f32(px[3]),
    )
    .expect("the lines cross");
    let crossing = (f64::from(crossing[0]), f64::from(crossing[1]));
    let regions = fills(&mut chart, wash());
    assert_eq!(regions.len(), 1);
    let between = |x: f64| {
        let along = |(start, end): ((f64, f64), (f64, f64))| {
            start.1 + (end.1 - start.1) * (x - start.0) / (end.0 - start.0)
        };
        (x, (along((px[0], px[1])) + along((px[2], px[3]))) / 2.0)
    };
    for lobe in [between(px[0].0 + 10.0), between(px[1].0 - 10.0)] {
        assert!(point_in_polygon(lobe, &regions[0]), "{lobe:?}");
        assert_eq!(hit_id(&chart, lobe), Some(id), "{lobe:?}");
    }
    for outside in [
        (crossing.0, crossing.1 - 20.0),
        (crossing.0, crossing.1 + 20.0),
    ] {
        assert!(!point_in_polygon(outside, &regions[0]), "{outside:?}");
        assert_eq!(hit_id(&chart, outside), None, "{outside:?}");
    }
}

/// The triangles the triangle executors (WebGPU, GPUI, native) paint for every `BandFill` of the
/// first pane in `color`: two per band segment, split at a crossing.
fn band_triangles(chart: &mut ChartEngine, color: Color) -> Vec<[(f64, f64); 3]> {
    let frame = chart.build_frame();
    let pane = &frame.panes[0];
    let mut triangles = Vec::new();
    for prim in &pane.main {
        let Prim::BandFill {
            upper_first,
            lower_first,
            point_count,
            fill,
            ..
        } = prim
        else {
            continue;
        };
        if *fill != color {
            continue;
        }
        let at = |first: u32, index: u32| pane.points[(first + index) as usize];
        for index in 1..*point_count {
            let corners = aeris_charts_render::line::band_segment_triangles(
                at(*upper_first, index - 1),
                at(*upper_first, index),
                at(*lower_first, index - 1),
                at(*lower_first, index),
            )
            .map(|point| (f64::from(point[0]), f64::from(point[1])));
            triangles.push([corners[0], corners[1], corners[2]]);
            triangles.push([corners[3], corners[4], corners[5]]);
        }
    }
    triangles
}

/// A disjoint's four free ends can make a concave quad, which the triangle executors' fixed fan
/// over a 2-point band would overpaint (the notch at the reflex end, and twice where its two
/// triangles overlap). Its fill lowers through the exact ribbon instead, so every executor paints,
/// and the selected fill hits, the quad only.
#[test]
fn concave_disjoint_channels_fill_and_hit_only_their_quad() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::DisjointChannel,
        vec![
            p(10.0, 100.0),
            p(15.0, 101.0),
            p(10.0, 104.0),
            p(20.0, 100.0),
        ],
        r##"{"color":"#123456"}"##,
    );
    chart.set_selected_drawing(Some(id));
    let px = [0, 1, 2, 3].map(|index| anchor(&chart, id, index));
    let quad = [px[0], px[2], px[3], px[1]];
    // Under the reflex end (the second anchor), between the lines' ends at bar 15.
    let notch = to_px(&chart, p(15.0, 100.5));
    let inside = [to_px(&chart, p(15.0, 101.5)), to_px(&chart, p(12.0, 102.0))];
    assert!(!point_in_polygon(notch, &quad));
    let triangles = band_triangles(&mut chart, wash());
    let painted = |probe: (f64, f64)| {
        triangles
            .iter()
            .filter(|triangle| point_in_polygon(probe, &triangle[..]))
            .count()
    };
    assert_eq!(painted(notch), 0, "the notch stays empty");
    assert_eq!(hit_id(&chart, notch), None, "the notch does not hit");
    for probe in inside {
        assert!(point_in_polygon(probe, &quad), "{probe:?}");
        assert_eq!(painted(probe), 1, "painted once: {probe:?}");
        assert_eq!(hit_id(&chart, probe), Some(id), "{probe:?}");
    }
    // Canvas2D fills the same region as one path.
    let regions = fills(&mut chart, wash());
    assert_eq!(regions.len(), 1);
    assert!(!point_in_polygon(notch, &regions[0]));
    for probe in inside {
        assert!(point_in_polygon(probe, &regions[0]), "{probe:?}");
    }
}

/// The fork's signed per-side deviations are overrides of `regression_deviations` (owner decision
/// C5): each moves its own line only, the band keeps the flat field, and two sides on the same
/// side of the centre fill from the centre to the farther one.
#[test]
fn regression_side_overrides_offset_each_line() {
    let mut chart = trending_chart();
    let id = add(
        &mut chart,
        DrawingKind::RegressionTrend,
        vec![p(6.0, 101.0), p(31.0, 104.0)],
        r##"{"color":"#123456","width":2,"tool_options":{"channel":{"upper_deviation":3,"lower_deviation":-1}}}"##,
    );
    let stats = fit(&chart, id, IndicatorInputSource::Close);
    let lines = |chart: &mut ChartEngine| {
        strokes(&ink_polylines(chart))
            .iter()
            .map(Stroke::line)
            .collect::<Vec<_>>()
    };
    let ends = |chart: &ChartEngine, deviations: f64| {
        [6.0, 31.0].map(|logical| on_fit(chart, stats, logical, deviations))
    };
    let painted = lines(&mut chart);
    assert_eq!(painted.len(), 3);
    // Upstream's order: lower, upper, centre.
    for (line, deviations) in painted.iter().zip([-1.0, 3.0, 0.0]) {
        let expected = ends(&chart, deviations);
        assert!(
            close(line[0], expected[0], 1e-3) && close(line[1], expected[1], 1e-3),
            "{deviations}: {line:?} {expected:?}"
        );
    }
    let block = chart
        .drawing(id)
        .unwrap()
        .tool_options
        .channel
        .clone()
        .unwrap();
    assert_eq!(
        (block.upper_deviation, block.lower_deviation),
        (Some(3.0), Some(-1.0))
    );
    // A side patch moves its own line and leaves the flat band alone.
    assert!(
        chart.drawing_apply_options(id, r#"{"tool_options":{"channel":{"upper_deviation":1}}}"#)
    );
    let painted = lines(&mut chart);
    assert!(close(painted[0][0], ends(&chart, -1.0)[0], 1e-3));
    assert!(close(painted[1][0], ends(&chart, 1.0)[0], 1e-3));
    assert_eq!(chart.drawing(id).unwrap().regression_deviations, 2.0);
    // Changing the flat band keeps the override (owner decision C4): only the other side moves.
    assert!(chart.drawing_apply_options(id, r#"{"regression_deviations":1.5}"#));
    let painted = lines(&mut chart);
    assert!(close(painted[0][0], ends(&chart, -1.0)[0], 1e-3));
    assert!(close(painted[1][0], ends(&chart, 1.0)[0], 1e-3));
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"channel":{"lower_deviation":null}}}"#
    ));
    assert!(close(lines(&mut chart)[0][0], ends(&chart, -1.5)[0], 1e-3));

    // Both sides above the centre: the zone runs from the centre to the farther side.
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"channel":{"upper_deviation":2,"lower_deviation":0.5}}}"#
    ));
    let regions = fills(&mut chart, zone());
    assert_eq!(regions.len(), 1);
    for (deviations, inside) in [(0.25, true), (1.0, true), (1.75, true), (-0.25, false)] {
        assert_eq!(
            point_in_polygon(on_fit(&chart, stats, 18.0, deviations), &regions[0]),
            inside,
            "{deviations}"
        );
    }
}

/// `use_upper_deviation` / `use_lower_deviation` switch a side's line and its zone off: the
/// band fills from the centre to the other side, and with both off only the centre remains.
#[test]
fn regression_side_switches_drop_a_line_and_its_zone() {
    let mut chart = trending_chart();
    let id = add(
        &mut chart,
        DrawingKind::RegressionTrend,
        vec![p(6.0, 101.0), p(31.0, 104.0)],
        r##"{"color":"#123456","width":2,"tool_options":{"channel":{"use_lower_deviation":false}}}"##,
    );
    let stats = fit(&chart, id, IndicatorInputSource::Close);
    let painted = strokes(&ink_polylines(&mut chart));
    assert_eq!(painted.len(), 2, "upper and centre");
    assert!(close(
        painted[0].start(),
        on_fit(&chart, stats, 6.0, 2.0),
        1e-3
    ));
    assert!(close(
        painted[1].start(),
        on_fit(&chart, stats, 6.0, 0.0),
        1e-3
    ));
    let regions = fills(&mut chart, zone());
    assert_eq!(regions.len(), 1);
    // The zone's lower chain (the outline's last two points, reversed) is the centre line.
    assert!(close(regions[0][3], on_fit(&chart, stats, 6.0, 0.0), 1e-3));
    assert!(close(regions[0][2], on_fit(&chart, stats, 31.0, 0.0), 1e-3));
    let gone = on_fit(&chart, stats, 18.0, -2.0);
    assert_eq!(hit_id(&chart, gone), None, "no lower line to hit");
    chart.set_selected_drawing(Some(id));
    assert_eq!(hit_id(&chart, on_fit(&chart, stats, 18.0, -1.0)), None);
    chart.set_selected_drawing(None);

    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"channel":{"use_upper_deviation":false}}}"#
    ));
    assert!(fills(&mut chart, zone()).is_empty());
    let painted = strokes(&ink_polylines(&mut chart));
    assert_eq!(painted.len(), 1, "the centre alone");
    assert!(close(
        painted[0].start(),
        on_fit(&chart, stats, 6.0, 0.0),
        1e-3
    ));
}

/// `source` picks the bar value the fit reads; a change refits once (the memo is keyed by it).
#[test]
fn regression_fits_the_selected_source_value() {
    let mut chart = trending_chart();
    let id = add(
        &mut chart,
        DrawingKind::RegressionTrend,
        vec![p(6.0, 101.0), p(31.0, 104.0)],
        r##"{"color":"#123456","width":2}"##,
    );
    let passes = |chart: &ChartEngine| chart.drawing_settings.regression_memo.borrow().passes;
    let centre_start = |chart: &mut ChartEngine| strokes(&ink_polylines(chart))[2].start();
    let close_fit = fit(&chart, id, IndicatorInputSource::Close);
    assert!(close(
        centre_start(&mut chart),
        on_fit(&chart, close_fit, 6.0, 0.0),
        1e-3
    ));
    let mut seen = Vec::new();
    for (source, value) in [
        (IndicatorInputSource::High, "\"high\""),
        (IndicatorInputSource::Hl2, "\"hl2\""),
        (IndicatorInputSource::Close, "null"),
    ] {
        let before = passes(&chart);
        assert!(chart.drawing_apply_options(
            id,
            &format!(r#"{{"tool_options":{{"channel":{{"source":{value}}}}}}}"#)
        ));
        let start = centre_start(&mut chart);
        assert_eq!(passes(&chart), before + 1, "{source:?} refits once");
        let expected = on_fit(&chart, fit(&chart, id, source), 6.0, 0.0);
        assert!(close(start, expected, 1e-3), "{source:?}");
        seen.push(start);
    }
    assert!(seen[0] != seen[1] && seen[1] != seen[2]);
    assert!(close(seen[2], on_fit(&chart, close_fit, 6.0, 0.0), 1e-3));
}

/// `show_pearsons` paints Pearson's R below the regression's start (off by default, upstream's
/// look): left-aligned into the channel, ending at the start when the end lies left of it, not
/// a hit target, nothing without a correlation, and culled with its reach.
#[test]
fn regressions_paint_pearsons_r_below_their_start_when_enabled() {
    let mut chart = trending_chart();
    let id = add(
        &mut chart,
        DrawingKind::RegressionTrend,
        vec![p(6.0, 101.0), p(31.0, 104.0)],
        r##"{"color":"#123456"}"##,
    );
    assert!(texts(&mut chart).is_empty(), "no label by default");
    assert!(
        chart.drawing_apply_options(id, r#"{"tool_options":{"channel":{"show_pearsons":true}}}"#)
    );
    let stats = fit(&chart, id, IndicatorInputSource::Close);
    let pearson = format!("{:.4}", stats.pearson.unwrap());
    let runs = texts(&mut chart);
    assert_eq!(runs.len(), 1, "{runs:?}");
    let (text, at, align) = runs[0].clone();
    assert_eq!(text, pearson);
    assert_eq!(align, TextAlign::Left);
    let start = on_fit(&chart, stats, 6.0, 0.0);
    let lowest = on_fit(&chart, stats, 6.0, -2.0);
    assert!((at.0 - start.0).abs() < 1e-3, "{at:?} {start:?}");
    assert!(
        at.1 > lowest.1 + 4.0,
        "below the lowest line: {at:?} {lowest:?}"
    );
    assert_eq!(
        hit_id(&chart, (at.0 + 12.0, at.1)),
        None,
        "not a hit target"
    );

    // Drawn right to left: right-aligned at the start, reading into the channel.
    assert!(
        chart
            .set_drawing_anchors(id, &[p(31.0, 104.0).into(), p(6.0, 101.0).into()])
            .is_ok()
    );
    let (_, at, _) = texts(&mut chart)[0].clone();
    let start = on_fit(&chart, stats, 31.0, 0.0);
    assert!(at.0 < start.0 - 10.0, "ends at the start: {at:?} {start:?}");

    // No correlation on constant data: no label.
    let mut flat = chart_with(&hourly(40), 1.0);
    let values = vec![100.0; 40];
    flat.set_series_data(0, &hourly(40), &values, &values, &values, &values)
        .unwrap();
    add(
        &mut flat,
        DrawingKind::RegressionTrend,
        vec![p(6.0, 101.0), p(31.0, 104.0)],
        r#"{"tool_options":{"channel":{"show_pearsons":true}}}"#,
    );
    assert!(texts(&mut flat).is_empty());

    // Two bars left of the pane: only the label reaches in, and the drawing stays a candidate.
    let mut culled = trending_chart();
    crowd(&mut culled);
    culled.set_visible_logical_range(10.0, 210.0);
    let narrow = add(
        &mut culled,
        DrawingKind::RegressionTrend,
        vec![p(2.0, 101.0), p(3.0, 101.0)],
        r#"{"tool_options":{"channel":{"show_pearsons":true}}}"#,
    );
    let end_x = anchor(&culled, narrow, 1).0;
    assert!(end_x < -25.0, "{end_x}");
    let runs = texts(&mut culled);
    assert_eq!(runs.len(), 1, "the label paints: {runs:?}");
    let size = culled.drawing_text_size(culled.drawing(narrow).unwrap());
    let family = culled.options.get().layout.font_family.clone();
    let label_end = runs[0].1.0 + culled.measure_text_run(&runs[0].0, size, &family, 400, false);
    assert!(
        label_end > 0.0,
        "the label reaches into the pane: {label_end}"
    );
    assert!(viewport_candidate(&culled, narrow));
    assert!(culled.drawing_apply_options(
        narrow,
        r#"{"tool_options":{"channel":{"show_pearsons":false}}}"#
    ));
    assert!(!viewport_candidate(&culled, narrow));
}

/// `middle_line` draws a regression's centre as the 1 px dashed middle line in `middle_color`
/// (the fork's look, which fork documents carry); without it the centre is upstream's solid line.
#[test]
fn regression_middle_line_dashes_the_centre_in_its_color() {
    let mut chart = trending_chart();
    let id = add(
        &mut chart,
        DrawingKind::RegressionTrend,
        vec![p(6.0, 101.0), p(31.0, 104.0)],
        r##"{"color":"#123456","width":2}"##,
    );
    let stats = fit(&chart, id, IndicatorInputSource::Close);
    let (solid, dashed) = split_lines(&ink_polylines(&mut chart));
    assert_eq!((solid.len(), dashed.len()), (3, 0));
    assert!(
        chart.drawing_apply_options(id, r#"{"tool_options":{"channel":{"middle_line":true}}}"#)
    );
    let (solid, dashed) = split_lines(&ink_polylines(&mut chart));
    assert_eq!((solid.len(), dashed.len()), (2, 1));
    assert_eq!(dashed[0].width, 1.0);
    assert!(dashed[0].spans(
        on_fit(&chart, stats, 6.0, 0.0),
        on_fit(&chart, stats, 31.0, 0.0),
        1e-3
    ));
    assert_eq!(
        hit_id(&chart, on_fit(&chart, stats, 18.0, 0.0)),
        Some(id),
        "the centre stays a body target"
    );
    assert!(chart.drawing_apply_options(
        id,
        r##"{"tool_options":{"channel":{"middle_color":"#e91e63"}}}"##
    ));
    let pink = strokes(&color_polylines(
        &mut chart,
        Color::parse_css("#e91e63").unwrap(),
    ));
    assert!(pink.len() == 1 && pink[0].dashed());
    assert_eq!(strokes(&ink_polylines(&mut chart)).len(), 2);
}

/// `extend_left`/`extend_right` run a regression's lines and zone to the pane edges, like a
/// channel's; Pearson's R stays at the fitted start.
#[test]
fn regression_extensions_run_the_band_to_the_pane_edges() {
    let mut chart = trending_chart();
    let id = add(
        &mut chart,
        DrawingKind::RegressionTrend,
        vec![p(10.0, 101.0), p(20.0, 104.0)],
        r##"{"color":"#123456","tool_options":{"channel":{"show_pearsons":true}}}"##,
    );
    let stats = fit(&chart, id, IndicatorInputSource::Close);
    let reach = |chart: &mut ChartEngine| {
        ink_polylines(chart)
            .iter()
            .flat_map(|(points, ..)| points.iter().map(|point| point.0))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| {
                (lo.min(x), hi.max(x))
            })
    };
    let (start, end) = (
        on_fit(&chart, stats, 10.0, 0.0),
        on_fit(&chart, stats, 20.0, 0.0),
    );
    let (left, right) = reach(&mut chart);
    assert!((left - start.0).abs() < 1e-3 && (right - end.0).abs() < 1e-3);
    let beyond = on_fit(&chart, stats, 4.0, 2.0);
    assert_eq!(hit_id(&chart, beyond), None);
    let label = texts(&mut chart)[0].1;
    assert!(chart.drawing_apply_options(id, r#"{"extend_left":true,"extend_right":true}"#));
    let (left, right) = reach(&mut chart);
    let width = chart.pane_w;
    assert!(left <= 1.0 && right >= width - 1.0, "{left}..{right}");
    assert_eq!(hit_id(&chart, beyond), Some(id), "hits along the extension");
    let regions = fills(&mut chart, zone());
    assert!(regions[0].iter().any(|point| point.0 <= 1.0));
    assert!(regions[0].iter().any(|point| point.0 >= width - 1.0));
    assert_eq!(
        texts(&mut chart)[0].1,
        label,
        "Pearson's R stays at the start"
    );
}

/// A selected regression's zone is a drag surface (unselected, the pane pans through it), and a
/// body drag moves it along time only (owner decision C1).
#[test]
fn selected_regression_zones_are_drag_surfaces() {
    let mut chart = trending_chart();
    let id = add(
        &mut chart,
        DrawingKind::RegressionTrend,
        vec![p(6.0, 101.0), p(31.0, 104.0)],
        r##"{"color":"#123456"}"##,
    );
    let stats = fit(&chart, id, IndicatorInputSource::Close);
    let inside = on_fit(&chart, stats, 18.0, 1.0);
    assert_eq!(hit_id(&chart, inside), None);
    chart.set_selected_drawing(Some(id));
    assert_eq!(
        chart
            .hit_test_drawing(inside.0, inside.1)
            .map(|hit| (hit.id, hit.part)),
        Some((id, DrawingDragPart::Body))
    );
    assert_eq!(
        chart.hit_test_drawing(inside.0, inside.1),
        chart.hit_test_drawing_bruteforce(inside.0, inside.1)
    );
    // Outside the band: nothing.
    assert_eq!(hit_id(&chart, on_fit(&chart, stats, 18.0, 3.0)), None);
    let before = chart.drawing(id).unwrap().points.clone();
    let bar = chart.logical_to_coordinate(1.0).unwrap() - chart.logical_to_coordinate(0.0).unwrap();
    assert!(chart.drawing_drag_start_at(inside.0, inside.1));
    chart.drawing_drag_to(
        inside.0 + 2.0 * bar,
        inside.1 + 60.0,
        DrawingModifiers::default(),
    );
    chart.drawing_drag_end();
    for (old, new) in before.iter().zip(&chart.drawing(id).unwrap().points) {
        assert!((new.logical - old.logical - 2.0).abs() < 1e-6);
        assert_eq!(new.price, old.price);
    }
    // Without a fill there is nothing to grab between the lines.
    assert!(chart.drawing_apply_options(id, r#"{"fill_enabled":false}"#));
    let moved = fit(&chart, id, IndicatorInputSource::Close);
    assert_eq!(hit_id(&chart, on_fit(&chart, moved, 20.0, 1.0)), None);
}

/// A regression's handles sit on the fitted line's ends, not at its clicks (owner decision C1):
/// painted, hit, and dragged there, they follow the refit; without a fit they stay on the
/// anchors of the dashed placeholder.
#[test]
fn regression_handles_sit_on_the_fitted_line_ends() {
    let mut chart = trending_chart();
    // Anchor prices far from the data.
    let id = add(
        &mut chart,
        DrawingKind::RegressionTrend,
        vec![p(6.0, 60.0), p(31.0, 140.0)],
        "{}",
    );
    chart.set_selected_drawing(Some(id));
    let stats = fit(&chart, id, IndicatorInputSource::Close);
    let ends = [6.0, 31.0].map(|logical| on_fit(&chart, stats, logical, 0.0));
    let painted = painted_handles(&mut chart);
    assert_eq!(painted.len(), 2);
    for (painted, end) in painted.iter().zip(ends) {
        assert!(close(*painted, end, 1e-3), "{painted:?} {end:?}");
    }
    assert_eq!(chart.drawing_handle_count(id), Some(2));
    assert_eq!(
        chart
            .hit_test_drawing(ends[1].0, ends[1].1)
            .map(|hit| hit.part),
        Some(DrawingDragPart::Anchor(1))
    );
    let click = anchor(&chart, id, 1);
    assert_eq!(chart.hit_test_drawing(click.0, click.1), None);

    // Dragging the end handle three bars left refits the shorter range; the handle stays on
    // the new line and the anchor keeps its price.
    let bar = chart.logical_to_coordinate(1.0).unwrap() - chart.logical_to_coordinate(0.0).unwrap();
    assert!(chart.drawing_drag_start_at(ends[1].0, ends[1].1));
    chart.drawing_drag_to(
        ends[1].0 - 3.0 * bar,
        ends[1].1 + 40.0,
        DrawingModifiers::default(),
    );
    chart.drawing_drag_end();
    let end = chart.drawing(id).unwrap().points[1];
    assert!((end.logical - 28.0).abs() < 1e-6);
    assert_eq!(end.price, 140.0);
    let refit = fit(&chart, id, IndicatorInputSource::Close);
    assert!(close(
        painted_handles(&mut chart)[1],
        on_fit(&chart, refit, end.logical, 0.0),
        1e-3
    ));
    // The keyboard reaches the handle from where it is painted.
    assert!(chart.nudge_selected_drawing(bar, 0.0, Some(1)));
    assert!((chart.drawing(id).unwrap().points[1].logical - 29.0).abs() < 1e-6);

    // Without a fit (beyond the data), the handles stay on the placeholder's anchors.
    assert!(
        chart
            .set_drawing_anchors(id, &[p(50.0, 100.0).into(), p(60.0, 101.0).into()])
            .is_ok()
    );
    let placeholder = [anchor(&chart, id, 0), anchor(&chart, id, 1)];
    let painted = painted_handles(&mut chart);
    assert!(close(painted[0], placeholder[0], 1e-3) && close(painted[1], placeholder[1], 1e-3));
}

/// Placing a regression previews the fit, and the placed anchor's handle sits on the previewed
/// line, as it will once committed.
#[test]
fn regression_placement_previews_its_handle_on_the_fitted_line() {
    let mut chart = trending_chart();
    assert!(chart.set_drawing_tool(
        Some(DrawingKind::RegressionTrend),
        Some(r##"{"color":"#123456"}"##),
        None
    ));
    // Clicks away from the fit: the prices only pick bars.
    let start = to_px(&chart, p(6.0, 109.0));
    let end = to_px(&chart, p(31.0, 100.0));
    chart.drawing_tool_activate(start.0, start.1, DrawingModifiers::default());
    chart.drawing_tool_pointer_move(end.0, end.1, DrawingModifiers::default(), false);
    let preview = crate::Drawing::new(
        0,
        DrawingKind::RegressionTrend,
        0,
        vec![p(6.0, 109.0), p(31.0, 100.0)],
    );
    let stats = regression_stats(&chart, &preview, IndicatorInputSource::Close).unwrap();
    let fitted_start = on_fit(&chart, stats, 6.0, 0.0);
    let handles = painted_handles(&mut chart);
    assert_eq!(handles.len(), 1);
    assert!(close(handles[0], fitted_start, 1e-3), "{handles:?}");
    let id = chart
        .drawing_tool_activate(end.0, end.1, DrawingModifiers::default())
        .created
        .unwrap();
    assert_eq!(painted_handles(&mut chart)[0], handles[0]);
    assert_eq!(chart.selected_drawing(), Some(id));
}

/// A regression's anchors choose bars only (owner decision C1): body and handle drags, magnet
/// snaps, and keyboard nudges move it along time, a vertical nudge records nothing, and undo
/// restores each step.
#[test]
fn regression_trends_move_along_time_only() {
    let mut chart = trending_chart();
    let id = add(
        &mut chart,
        DrawingKind::RegressionTrend,
        vec![p(6.0, 101.0), p(31.0, 104.0)],
        "{}",
    );
    let before = chart.drawing(id).unwrap().points.clone();
    let stats = fit(&chart, id, IndicatorInputSource::Close);
    let grab = on_fit(&chart, stats, 18.0, 0.0);
    let bar = chart.logical_to_coordinate(1.0).unwrap() - chart.logical_to_coordinate(0.0).unwrap();
    assert!(chart.drawing_drag_start_at(grab.0, grab.1));
    chart.drawing_drag_to(
        grab.0 + 3.0 * bar,
        grab.1 - 80.0,
        DrawingModifiers::default(),
    );
    chart.drawing_drag_end();
    let after = chart.drawing(id).unwrap().points.clone();
    for (old, new) in before.iter().zip(&after) {
        assert!((new.logical - old.logical - 3.0).abs() < 1e-6);
        assert_eq!(new.price, old.price, "the price never follows the pointer");
    }
    // An anchor drag from the handle on the fitted end snaps to a bar with the magnet and
    // stays horizontal.
    chart.set_selected_drawing(Some(id));
    let moved = fit(&chart, id, IndicatorInputSource::Close);
    let (bx, by) = on_fit(&chart, moved, after[1].logical, 0.0);
    assert!(chart.drawing_drag_start_at(bx, by));
    chart.drawing_drag_to(
        bx - 2.4 * bar,
        by + 50.0,
        DrawingModifiers {
            magnet: true,
            straighten: false,
        },
    );
    chart.drawing_drag_end();
    let end = chart.drawing(id).unwrap().points[1];
    assert_eq!(end.logical, 32.0);
    assert_eq!(end.price, after[1].price);
    // A vertical keyboard nudge cannot move it: no change and no history entry.
    let revision = chart.drawing_revision();
    assert!(!chart.nudge_selected_drawing(0.0, -5.0, None));
    assert!(!chart.nudge_selected_drawing(0.0, 5.0, Some(0)));
    assert_eq!(chart.drawing_revision(), revision);
    assert!(chart.undo_drawing() && chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);
}

/// The fork's documents keep its channel look through their stored (or fork-default) options:
/// the parallel channel's middle line, the regression's dashed centre, asymmetric and one-sided
/// bands, Pearson's R, and the fitted source; a `flat_top_bottom` whose level crosses its base
/// paints both lobes.
#[test]
fn fork_documents_paint_their_channel_presentation() {
    let at = |logical: f64, price: f64| serde_json::json!({"logical": logical, "price": price, "time": logical * HOUR});
    let document = serde_json::json!({
        "schema": "aeris_charts-state",
        "schema_version": 1,
        "panes": [{"id": "pane-1"}],
        "drawings": [
            {"id": 1, "kind": "parallel_channel", "pane_id": "pane-1",
             "style": {"color": INK},
             "anchors": [at(10.0, 101.0), at(20.0, 104.0), at(15.0, 101.5)]},
            {"id": 2, "kind": "regression_trend", "pane_id": "pane-1",
             "style": {"color": INK, "tool_options": {"channel": {
                 "upper_deviation": 3.0, "use_lower_deviation": false}}},
             "anchors": [at(6.0, 101.0), at(31.0, 104.0)]},
            {"id": 3, "kind": "flat_top_bottom", "pane_id": "pane-1",
             "style": {"color": INK},
             "anchors": [at(10.0, 101.0), at(20.0, 105.0), at(15.0, 103.0)]},
            {"id": 4, "kind": "regression_trend", "pane_id": "pane-1",
             "style": {"color": INK, "tool_options": {"channel": {"source": "hlc3"}}},
             "anchors": [at(6.0, 101.0), at(31.0, 104.0)]}
        ],
    })
    .to_string();
    let mut chart = trending_chart();
    chart.import_state_json(&document).unwrap();
    let only = |chart: &mut ChartEngine, id: DrawingId| {
        for other in [1, 2, 3, 4] {
            let visible = format!(r#"{{"visible":{}}}"#, other == id);
            assert!(chart.drawing_apply_options(other, &visible));
        }
    };
    // The parallel channel's dashed middle line, on by the fork's default.
    only(&mut chart, 1);
    let (boundaries, middles) = split_lines(&ink_polylines(&mut chart));
    assert_eq!((boundaries.len(), middles.len()), (2, 1));
    // The regression: one-sided at +3 (sample σ became population σ), dashed centre, R.
    only(&mut chart, 2);
    let stats = fit(&chart, 2, IndicatorInputSource::Close);
    let (solid, dashed) = split_lines(&ink_polylines(&mut chart));
    assert_eq!((solid.len(), dashed.len()), (1, 1));
    assert!(close(
        solid[0].start(),
        on_fit(&chart, stats, 6.0, 3.0),
        1e-3
    ));
    assert!(dashed[0].spans(
        on_fit(&chart, stats, 6.0, 0.0),
        on_fit(&chart, stats, 31.0, 0.0),
        1e-3
    ));
    let pearson = format!("{:.4}", stats.pearson.unwrap());
    assert!(texts(&mut chart).iter().any(|(text, ..)| *text == pearson));
    // The crossing flat level: a disjoint channel whose band fill splits into two lobes.
    only(&mut chart, 3);
    assert_eq!(chart.drawing(3).unwrap().kind, DrawingKind::DisjointChannel);
    let regions = fills(&mut chart, wash());
    assert_eq!(regions.len(), 1);
    let [a, b, level_a, level_b] = [0, 1, 2, 3].map(|index| anchor(&chart, 3, index));
    let between = |x: f64| {
        let along = |(start, end): ((f64, f64), (f64, f64))| {
            start.1 + (end.1 - start.1) * (x - start.0) / (end.0 - start.0)
        };
        (x, (along((a, b)) + along((level_a, level_b))) / 2.0)
    };
    for lobe in [between(a.0 + 10.0), between(b.0 - 10.0)] {
        assert!(point_in_polygon(lobe, &regions[0]), "{lobe:?}");
    }
    // A stored `source` keeps its fit: the dashed centre runs along the HLC3 fit.
    only(&mut chart, 4);
    let hlc3 = fit(&chart, 4, IndicatorInputSource::Hlc3);
    let (_, dashed) = split_lines(&ink_polylines(&mut chart));
    assert_eq!(dashed.len(), 1);
    assert!(dashed[0].spans(
        on_fit(&chart, hlc3, 6.0, 0.0),
        on_fit(&chart, hlc3, 31.0, 0.0),
        1e-3
    ));
    assert!(!close(
        dashed[0].start(),
        on_fit(
            &chart,
            fit(&chart, 4, IndicatorInputSource::Close),
            6.0,
            0.0
        ),
        1e-3
    ));

    // An export restores to the same picture.
    let frame = |chart: &mut ChartEngine| {
        let frame = chart.build_frame();
        (frame.panes[0].main.clone(), frame.panes[0].points.clone())
    };
    for id in [1, 2, 3, 4] {
        assert!(chart.drawing_apply_options(id, r#"{"visible":true}"#));
    }
    let mut again = trending_chart();
    again
        .import_state_json(&chart.export_state_json().unwrap())
        .unwrap();
    assert_eq!(frame(&mut again), frame(&mut chart));
}
