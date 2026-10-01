//! Projection & Annotations engine tests: catalog defaults, armed placement, shared-part frames
//! and hit testing (indexed and brute force), forecast outcomes, bars-pattern capture and mapping,
//! range stats, the projection sector, pane-anchored text, drags, keyboard nudges, magnet, time
//! identity, schema and kind options, patches with history, persistence, clipboard, and sync.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{LineStyle, Prim, TextAlign};

use super::super::super::{DrawingPlacement, DrawingTextHAlign, DrawingTextVAlign};
use super::{BarsPatternMode, DrawingIcon, MAX_BARS_PATTERN_BARS};
use crate::{
    ChartEngine, DrawingAnchor, DrawingDragPart, DrawingId, DrawingKind, DrawingLabelMetric,
    DrawingLineCap, DrawingMagnetMode, DrawingModifiers, DrawingPoint, DrawingPriceSegment,
};

const KINDS: [DrawingKind; 19] = [
    DrawingKind::Forecast,
    DrawingKind::BarsPattern,
    DrawingKind::PriceRange,
    DrawingKind::DateRange,
    DrawingKind::DateAndPriceRange,
    DrawingKind::Projection,
    DrawingKind::AnchoredText,
    DrawingKind::Note,
    DrawingKind::PriceNote,
    DrawingKind::Callout,
    DrawingKind::Comment,
    DrawingKind::PriceLabel,
    DrawingKind::Signpost,
    DrawingKind::FlagMark,
    DrawingKind::ArrowMarkUp,
    DrawingKind::ArrowMarkDown,
    DrawingKind::ArrowMarkLeft,
    DrawingKind::ArrowMarkRight,
    DrawingKind::Icon,
];
const INK: &str = "#123456";
const HOUR: f64 = 3_600.0;

fn value_at(index: usize) -> f64 {
    100.0 + (index % 7) as f64
}

fn chart_with(times: &[f64], dpr: f64) -> ChartEngine {
    let mut chart = ChartEngine::new(800.0, 500.0, dpr);
    let close = (0..times.len()).map(value_at).collect::<Vec<_>>();
    let open = close.iter().map(|value| value - 0.25).collect::<Vec<_>>();
    let high = close.iter().map(|value| value + 0.5).collect::<Vec<_>>();
    let low = close.iter().map(|value| value - 0.5).collect::<Vec<_>>();
    chart
        .set_series_data(0, times, &open, &high, &low, &close)
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

fn p(logical: f64, price: f64) -> DrawingPoint {
    DrawingPoint { logical, price }
}

fn add(
    chart: &mut ChartEngine,
    kind: DrawingKind,
    points: Vec<DrawingPoint>,
    options: &str,
) -> DrawingId {
    chart
        .add_drawing(kind, 0, points, Some(options))
        .unwrap_or_else(|| panic!("{kind:?} added"))
}

/// Anchors for each kind: pane fractions for anchored text, data points otherwise.
fn points_for(kind: DrawingKind) -> Vec<DrawingPoint> {
    match kind.anchor_count() {
        1 if kind == DrawingKind::AnchoredText => vec![p(0.3, 0.2)],
        1 => vec![p(15.0, 103.0)],
        2 => vec![p(10.0, 101.0), p(20.0, 105.0)],
        _ => vec![p(10.0, 101.0), p(22.0, 104.0), p(20.0, 106.0)],
    }
}

fn anchor(chart: &ChartEngine, id: DrawingId, index: usize) -> (f64, f64) {
    chart.drawing_point_to_coordinate(id, index).unwrap()
}

fn ink() -> Color {
    Color::parse_css(INK).unwrap()
}

fn texts(chart: &mut ChartEngine) -> Vec<(String, f32, f32)> {
    let frame = chart.build_frame();
    frame.panes[0]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::Text { text, x, y, .. } => Some((text.clone(), *x, *y)),
            _ => None,
        })
        .collect()
}

fn texts_of(chart: &mut ChartEngine) -> Vec<String> {
    texts(chart).into_iter().map(|(text, ..)| text).collect()
}

/// Every filled region (`BandFill`) of the first pane as its outline path: upper chain forward,
/// lower chain backward, with its fill color.
fn fills(chart: &mut ChartEngine) -> Vec<(Vec<(f64, f64)>, Color)> {
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
            } => {
                let at = |index: u32| {
                    let point = pane.points[index as usize];
                    (f64::from(point[0]), f64::from(point[1]))
                };
                let mut outline = (0..*point_count)
                    .map(|index| at(upper_first + index))
                    .collect::<Vec<_>>();
                outline.extend((0..*point_count).rev().map(|index| at(lower_first + index)));
                Some((outline, *fill))
            }
            _ => None,
        })
        .collect()
}

fn ink_polylines(chart: &mut ChartEngine) -> Vec<(Vec<(f64, f64)>, LineStyle)> {
    let frame = chart.build_frame();
    let pane = &frame.panes[0];
    pane.main
        .iter()
        .filter_map(|prim| match prim {
            Prim::Polyline {
                first_point,
                point_count,
                style,
                color,
                ..
            } if *color == ink() => Some((
                pane.points[*first_point as usize..(*first_point + *point_count) as usize]
                    .iter()
                    .map(|point| (f64::from(point[0]), f64::from(point[1])))
                    .collect(),
                *style,
            )),
            _ => None,
        })
        .collect()
}

fn ink_vlines(chart: &mut ChartEngine) -> Vec<(i32, i32, i32)> {
    let frame = chart.build_frame();
    frame.panes[0]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::VLine {
                x, y0, y1, color, ..
            } if *color == ink() => Some((*x, *y0, *y1)),
            _ => None,
        })
        .collect()
}

fn close(a: (f64, f64), b: (f64, f64), tolerance: f64) -> bool {
    (a.0 - b.0).abs() <= tolerance && (a.1 - b.1).abs() <= tolerance
}

fn hit(chart: &ChartEngine, x: f64, y: f64) -> Option<DrawingId> {
    chart.hit_test_drawing(x, y).map(|hit| hit.id)
}

#[test]
fn catalog_defaults_follow_each_tool() {
    for kind in KINDS {
        let spec = kind.spec();
        assert!(spec.family.is_some(), "{kind:?} is a family tool");
        assert!((128..=146).contains(&spec.wire_id));
        assert!(!spec.axis_price_label);
        // Placement opens the editor for the tools that start from a default text the user
        // replaces or extends; price texts start empty beside their price and arrows carry none.
        assert_eq!(
            spec.requests_text_editor,
            matches!(
                kind,
                DrawingKind::AnchoredText
                    | DrawingKind::Note
                    | DrawingKind::Callout
                    | DrawingKind::Comment
                    | DrawingKind::Signpost
            ),
            "{kind:?}"
        );
        let expected = match kind {
            DrawingKind::Projection => 3,
            DrawingKind::Forecast
            | DrawingKind::BarsPattern
            | DrawingKind::PriceRange
            | DrawingKind::DateRange
            | DrawingKind::DateAndPriceRange
            | DrawingKind::PriceNote
            | DrawingKind::Callout => 2,
            _ => 1,
        };
        assert_eq!(
            spec.placement,
            DrawingPlacement::ClickAnchors { count: expected },
            "{kind:?}"
        );
        assert_eq!(kind.pane_anchored(), kind == DrawingKind::AnchoredText);
        let drawing = crate::Drawing::new(1, kind, 0, Vec::new());
        let ranged = matches!(
            kind,
            DrawingKind::PriceRange | DrawingKind::DateRange | DrawingKind::DateAndPriceRange
        );
        assert_eq!(
            drawing.fill_enabled,
            ranged || kind == DrawingKind::Projection
        );
        assert_eq!(
            drawing.stroke_end,
            if ranged {
                DrawingLineCap::Arrow
            } else {
                DrawingLineCap::None
            }
        );
        let metrics = drawing
            .labels
            .iter()
            .map(|label| label.metric)
            .collect::<Vec<_>>();
        use DrawingLabelMetric::*;
        assert_eq!(
            metrics,
            match kind {
                DrawingKind::PriceRange => vec![PriceChange, PercentChange, Ticks],
                DrawingKind::DateRange => vec![BarCount, Duration],
                DrawingKind::DateAndPriceRange => {
                    vec![PriceChange, PercentChange, Ticks, BarCount, Duration]
                }
                _ => Vec::new(),
            }
        );
        assert_eq!(
            drawing.text,
            match kind {
                DrawingKind::AnchoredText => "Text",
                DrawingKind::Note => "Note",
                DrawingKind::Callout => "Callout",
                DrawingKind::Comment => "Comment",
                DrawingKind::Signpost => "Signpost",
                _ => "",
            }
        );
        let color = match kind {
            DrawingKind::ArrowMarkUp => aeris_charts_core::style::MARKET_UP_CSS,
            DrawingKind::ArrowMarkDown => aeris_charts_core::style::MARKET_DOWN_CSS,
            _ => crate::DRAWING_DEFAULT_COLOR,
        };
        assert_eq!(drawing.color, color, "{kind:?}");
        assert!(drawing.tool_options.is_empty());
    }
    let text = crate::Drawing::new(1, DrawingKind::AnchoredText, 0, Vec::new());
    assert_eq!(
        (text.text_h_align, text.text_v_align),
        (DrawingTextHAlign::Left, DrawingTextVAlign::Top)
    );
}

#[test]
fn neutral_hooks_stay_neutral_for_other_families() {
    fn no_parts(_: &super::PartContext<'_>, _: &mut super::DrawingParts) {}
    let family = super::DrawingFamily::new(no_parts, |_| crate::DrawingKindOptions::Generic);
    let chart = chart();
    let mut drawing = crate::Drawing::new(1, DrawingKind::Ray, 0, vec![p(1.0, 1.0); 2]);
    let untouched = drawing.clone();
    (family.on_create)(&chart, &mut drawing, true);
    assert_eq!(drawing, untouched);
    assert!(!family.owns_text);
    assert!(!(family.reveals_on_focus)(&drawing));
    assert!(KINDS.iter().all(|&kind| !(family.pane_anchored)(kind)));
    assert!(!DrawingKind::Ray.pane_anchored() && !DrawingKind::Text.pane_anchored());
}

#[test]
fn armed_tools_place_every_kind() {
    let mut chart = chart();
    for kind in KINDS {
        assert!(chart.set_drawing_tool(Some(kind), Some(r##"{"color":"#123456"}"##), None));
        let clicks = [(200.0, 260.0), (420.0, 150.0), (380.0, 120.0)];
        let mut created = None;
        for &(x, y) in &clicks[..kind.anchor_count()] {
            created = chart
                .drawing_tool_activate(x, y, DrawingModifiers::default())
                .created;
        }
        let id = created.unwrap_or_else(|| panic!("{kind:?} committed"));
        let drawing = chart.drawing(id).unwrap();
        assert_eq!(drawing.kind, kind);
        assert_eq!(drawing.points.len(), kind.anchor_count());
        assert_eq!(drawing.color, INK);
        assert_eq!(chart.active_drawing_tool(), None, "one-shot tools disarm");
        assert_eq!(chart.selected_drawing(), Some(id));
    }
    let frame = chart.build_frame();
    assert!(frame.panes[0]
        .points
        .iter()
        .all(|point| point[0].is_finite() && point[1].is_finite()));
}

#[test]
fn every_tool_paints_and_hits_its_own_geometry() {
    for kind in KINDS {
        let mut chart = chart();
        let clean = chart.build_frame().panes[0].main.len();
        let id = add(
            &mut chart,
            kind,
            points_for(kind),
            r##"{"color":"#123456"}"##,
        );
        let frame = chart.build_frame();
        assert!(
            frame.panes[0].main.len() > clean,
            "{kind:?} paints on its pane"
        );
        // Every tool is a body target somewhere on a coarse grid.
        let found = (0..80).any(|gx| {
            (0..50)
                .any(|gy| hit(&chart, gx as f64 * 10.0 + 2.0, gy as f64 * 10.0 + 3.0) == Some(id))
        });
        assert!(found, "{kind:?} hits");
    }
}

#[test]
fn forecasts_evaluate_success_failure_and_pending_from_the_source_series() {
    let mut chart = chart();
    // Bars after 10 reach 106.5 at bar 13 (close 106 + 0.5 high).
    let id = add(
        &mut chart,
        DrawingKind::Forecast,
        vec![p(10.0, 101.0), p(20.0, 106.0)],
        r##"{"color":"#123456"}"##,
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let runs = texts(&mut chart);
    let lines = runs
        .iter()
        .map(|(text, ..)| text.as_str())
        .collect::<Vec<_>>();
    assert!(lines.contains(&"101.00"), "source price: {lines:?}");
    assert!(lines.contains(&"+5.00 (+4.95%)"), "{lines:?}");
    assert!(lines.contains(&"Success"), "{lines:?}");
    // The source box sits before the source, the target box beyond the target.
    let (_, source_x, _) = runs.iter().find(|(text, ..)| text == "101.00").unwrap();
    assert!(f64::from(*source_x) < a.0 - 8.0);
    let (_, target_x, _) = runs
        .iter()
        .find(|(text, ..)| text == "+5.00 (+4.95%)")
        .unwrap();
    assert!(f64::from(*target_x) > b.0 + 8.0);
    let success = Color::parse_css(aeris_charts_core::style::MARKET_UP_CSS).unwrap();
    let frame = chart.build_frame();
    assert!(frame.panes[0].main.iter().any(|prim| matches!(
        prim,
        Prim::Rect { color, .. }
            if (color.r(), color.g(), color.b(), color.a()) == (success.r(), success.g(), success.b(), 224)
    )));
    // A target no bar reaches fails once the data passes it.
    assert!(chart
        .set_drawing_anchors(id, &[p(10.0, 101.0).into(), p(20.0, 107.0).into()])
        .is_ok());
    assert!(texts_of(&mut chart).contains(&"Failure".to_string()));
    // A target beyond the data is pending: no outcome line.
    assert!(chart
        .set_drawing_anchors(id, &[p(10.0, 101.0).into(), p(45.0, 107.0).into()])
        .is_ok());
    let runs = texts_of(&mut chart);
    assert!(!runs
        .iter()
        .any(|text| text == "Failure" || text == "Success"));
    // A target on the latest bar stays pending while that bar may still form; the next bar
    // decides the failure. (No bar reaches 107, and its outcome box stays on the pane: boxes
    // off the pane are not painted.)
    assert!(chart
        .set_drawing_anchors(id, &[p(30.0, 101.0).into(), p(39.0, 107.0).into()])
        .is_ok());
    assert!(!texts_of(&mut chart).contains(&"Failure".to_string()));
    assert!(chart.update_series_bar(0, 40.0 * HOUR, [100.0, 100.5, 99.5, 100.0]));
    assert!(texts_of(&mut chart).contains(&"Failure".to_string()));
    // Falling targets test the lows; the line hits and the target box selects the drawing.
    assert!(chart
        .set_drawing_anchors(id, &[p(13.0, 106.0).into(), p(20.0, 100.0).into()])
        .is_ok());
    assert!(texts_of(&mut chart).contains(&"Success".to_string()));
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    assert_eq!(hit(&chart, (a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0), Some(id));
    assert_eq!(hit(&chart, b.0 + 20.0, b.1), Some(id));
}

#[test]
fn bars_patterns_copy_their_source_and_pin_it_to_the_anchors() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::BarsPattern,
        vec![p(20.4, 90.0), p(10.2, 80.0)],
        r##"{"color":"#123456"}"##,
    );
    let drawing = chart.drawing(id).unwrap().clone();
    let block = drawing.tool_options.projection_annotation.clone().unwrap();
    assert_eq!(block.bars.len(), 11, "bars 10..=20");
    for (offset, bar) in block.bars.iter().enumerate() {
        let close = value_at(10 + offset);
        assert_eq!(*bar, [close - 0.25, close + 0.5, close - 0.5, close]);
    }
    // Anchors now span the copy's box, oldest first: the first bar at the highest high (bar 13's
    // 106.5), the last bar at the lowest low (bar 14's 99.5).
    let (high, low) = (106.5, 99.5);
    assert_eq!(drawing.points, vec![p(10.0, high), p(20.0, low)]);
    // Sticks paint exactly over the source bars' high–low ranges.
    let sticks = ink_vlines(&mut chart);
    assert_eq!(sticks.len(), 11);
    for (offset, &(x, y0, y1)) in sticks.iter().enumerate() {
        let index = 10 + offset;
        let bar_x = chart.logical_to_coordinate(index as f64).unwrap();
        let high = chart
            .series_price_to_coordinate(0, value_at(index) + 0.5)
            .unwrap();
        let low = chart
            .series_price_to_coordinate(0, value_at(index) - 0.5)
            .unwrap();
        assert_eq!(x, bar_x.round() as i32);
        assert!((f64::from(y0) - high).abs() <= 1.0 && (f64::from(y1) - low).abs() <= 1.0);
    }
    // Moving the pattern keeps its copy: a body drag translates every stick.
    let middle = sticks[5];
    let grab = (f64::from(middle.0), f64::from(middle.1 + middle.2) / 2.0);
    assert_eq!(hit(&chart, grab.0, grab.1), Some(id));
    assert!(chart.drawing_drag_start_at(grab.0, grab.1));
    chart.drawing_drag_to(grab.0 + 100.0, grab.1 - 30.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    let moved = ink_vlines(&mut chart);
    assert_eq!(moved.len(), 11);
    for (before, after) in sticks.iter().zip(&moved) {
        assert!((after.0 - before.0 - 100).abs() <= 1);
        assert!((after.1 - before.1 + 30).abs() <= 1);
    }
    assert_eq!(
        chart.drawing(id).unwrap().tool_options,
        drawing.tool_options,
        "the copy never recaptures"
    );
    assert!(chart.undo_drawing());

    // The second anchor stretches the copy in time and scales its box: twice the price span
    // doubles every stick.
    assert!(chart
        .set_drawing_anchors(
            id,
            &[
                p(10.0, high).into(),
                p(30.0, high - 2.0 * (high - low)).into()
            ]
        )
        .is_ok());
    let stretched = ink_vlines(&mut chart);
    let last_x = chart.logical_to_coordinate(30.0).unwrap();
    assert_eq!(stretched.last().unwrap().0, last_x.round() as i32);
    let unit = chart.series_price_to_coordinate(0, 100.0).unwrap()
        - chart.series_price_to_coordinate(0, 101.0).unwrap();
    // The doubled box reaches below the pane, where crisp sticks are clamped to its edge (or
    // dropped); every stick inside the pane doubles.
    let bottom = (chart.panes[0].top + chart.panes[0].height).ceil() as i32;
    assert!(stretched
        .iter()
        .all(|&(_, y0, y1)| y0 < bottom && y1 <= bottom));
    let inside = stretched
        .iter()
        .filter(|&&(_, _, y1)| y1 < bottom)
        .collect::<Vec<_>>();
    assert!(inside.len() >= 6, "{stretched:?}");
    for &&(_, y0, y1) in &inside {
        assert!(
            (f64::from(y1 - y0) - 2.0 * unit).abs() <= 1.5,
            "a 1-point stick doubles"
        );
    }

    // Line modes paint one polyline through the chosen field; mirroring reverses it in time.
    assert!(chart
        .set_drawing_anchors(id, &[p(10.0, high).into(), p(20.0, low).into()])
        .is_ok());
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"projection_annotation":{"bars_mode":"line_close"}}}"#
    ));
    let lines = ink_polylines(&mut chart);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].0.len(), 11);
    for (offset, point) in lines[0].0.iter().enumerate() {
        let y = chart
            .series_price_to_coordinate(0, value_at(10 + offset))
            .unwrap();
        assert!((point.1 - y).abs() < 1e-3);
    }
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"projection_annotation":{"mirrored":true}}}"#
    ));
    let mirrored = ink_polylines(&mut chart).remove(0).0;
    // Mirrored: the same box, the closes reversed in time.
    for (index, point) in mirrored.iter().enumerate() {
        assert!((point.0 - lines[0].0[index].0).abs() < 1e-3);
        assert!((point.1 - lines[0].0[10 - index].1).abs() < 1e-3);
    }
    // Flipped: upside down within the anchors' box.
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"projection_annotation":{"mirrored":false,"flipped":true}}}"#
    ));
    let (top, bottom) = (anchor(&chart, id, 0).1, anchor(&chart, id, 1).1);
    let flipped = ink_polylines(&mut chart).remove(0).0;
    for (straight, flipped) in lines[0].0.iter().zip(&flipped) {
        assert!((flipped.1 - (top + bottom - straight.1)).abs() < 1e-3);
    }
}

#[test]
fn bars_patterns_bound_long_ranges_preview_their_capture_and_survive_missing_data() {
    let mut long = chart_with(&hourly(400), 1.0);
    let id = add(
        &mut long,
        DrawingKind::BarsPattern,
        vec![p(0.0, 0.0), p(399.0, 0.0)],
        "{}",
    );
    let block = long
        .drawing(id)
        .unwrap()
        .tool_options
        .projection_annotation
        .clone()
        .unwrap();
    assert_eq!(block.bars.len(), MAX_BARS_PATTERN_BARS);
    // Each bucket merges its bars: extremes survive aggregation.
    assert!(block
        .bars
        .iter()
        .all(|bar| bar[1] >= bar[0] && bar[2] <= bar[3]));
    let highest = block.bars.iter().map(|bar| bar[1]).fold(f64::MIN, f64::max);
    assert_eq!(highest, 106.5);
    // The LOD summaries reproduce a full scan of every bucket: first open, extrema, last close.
    for (bucket, bar) in block.bars.iter().enumerate() {
        let rows = bucket * 400 / MAX_BARS_PATTERN_BARS..(bucket + 1) * 400 / MAX_BARS_PATTERN_BARS;
        let expected = [
            value_at(rows.start) - 0.25,
            rows.clone()
                .map(|row| value_at(row) + 0.5)
                .fold(f64::MIN, f64::max),
            rows.clone()
                .map(|row| value_at(row) - 0.5)
                .fold(f64::MAX, f64::min),
            value_at(rows.end - 1),
        ];
        assert_eq!(*bar, expected, "bucket {bucket}");
    }
    assert!(block.validate());
    let serialized = serde_json::to_vec(&long.drawing(id).unwrap().tool_options).unwrap();
    assert!(serialized.len() < crate::drawing_contract::MAX_DRAWING_TOOL_OPTIONS_BYTES);

    // While placing, the pending pattern previews the bars the commit will copy, and the commit
    // copies its own range even when the tool template carries a stale copy.
    let mut chart = chart();
    let stale = r##"{"color":"#123456","tool_options":{"projection_annotation":{"bars":[[1,2,0.5,1.5],[1,2,0.5,1.5]]}}}"##;
    assert!(chart.set_drawing_tool(Some(DrawingKind::BarsPattern), Some(stale), None));
    let start = chart
        .drawing_to_px_for(0, crate::DrawingPriceScale::Right, p(10.0, 103.0))
        .unwrap();
    let end = chart
        .drawing_to_px_for(0, crate::DrawingPriceScale::Right, p(14.0, 101.0))
        .unwrap();
    chart.drawing_tool_activate(start.0, start.1, DrawingModifiers::default());
    chart.drawing_tool_pointer_move(end.0, end.1, DrawingModifiers::default(), false);
    assert_eq!(
        ink_vlines(&mut chart).len(),
        5,
        "the preview shows the copy in place"
    );
    let placed = chart
        .drawing_tool_activate(end.0, end.1, DrawingModifiers::default())
        .created
        .unwrap();
    let copy = chart
        .drawing(placed)
        .unwrap()
        .tool_options
        .projection_annotation
        .clone()
        .unwrap()
        .bars;
    assert_eq!(copy.len(), 5);
    assert_eq!(copy[0][3], value_at(10));
    // A template applied later restyles another pattern but keeps that pattern's own copy.
    let template = chart.drawing_template_json(placed, "ghost").unwrap();
    let other = add(
        &mut chart,
        DrawingKind::BarsPattern,
        vec![p(20.0, 0.0), p(30.0, 0.0)],
        "{}",
    );
    let own = chart.drawing(other).unwrap().tool_options.clone();
    assert!(chart.apply_drawing_template_json(other, &template));
    assert_eq!(chart.drawing(other).unwrap().tool_options, own);

    // Created before any data, a pattern copies nothing and paints a dashed box.
    let mut empty = ChartEngine::new(800.0, 500.0, 1.0);
    let id = empty
        .add_drawing(
            DrawingKind::BarsPattern,
            0,
            vec![p(1.0, 1.0), p(5.0, 2.0)],
            None,
        )
        .unwrap();
    assert!(empty.drawing(id).unwrap().tool_options.is_empty());
    empty.build_frame();
    let values = (0..40).map(value_at).collect::<Vec<_>>();
    empty
        .set_series_data(0, &hourly(40), &values, &values, &values, &values)
        .unwrap();
    empty.time_scale.set_width(800.0);
    empty.fit_content();
    assert!(empty.drawing_apply_options(id, r##"{"color":"#123456"}"##));
    // Frames clip family strokes to the pane, so the box moves into view at the data's prices
    // (anchors set later never capture).
    assert!(empty
        .set_drawing_anchors(id, &[p(1.0, 101.0).into(), p(5.0, 103.0).into()])
        .is_ok());
    assert!(empty.drawing(id).unwrap().tool_options.is_empty());
    // The dashed outline reaches executors as solid dash runs, each on the anchors' box.
    let dashes = ink_polylines(&mut empty);
    let (a, b) = (anchor(&empty, id, 0), anchor(&empty, id, 1));
    let (left, right) = (a.0.min(b.0), a.0.max(b.0));
    let (top, bottom) = (a.1.min(b.1), a.1.max(b.1));
    let on_box = |&(x, y): &(f64, f64)| {
        let near = |value: f64, edge: f64| (value - edge).abs() <= 0.5;
        (near(x, left) || near(x, right)) && (top - 0.5..=bottom + 0.5).contains(&y)
            || (near(y, top) || near(y, bottom)) && (left - 0.5..=right + 0.5).contains(&x)
    };
    assert!(dashes.len() > 4, "{dashes:?}");
    for (points, style) in &dashes {
        assert_eq!(*style, LineStyle::Solid);
        assert!(points.iter().all(on_box), "{points:?}");
    }
}

#[test]
fn ranges_measure_with_fills_arrows_and_engine_stats() {
    let mut chart = chart();
    let price = add(
        &mut chart,
        DrawingKind::PriceRange,
        vec![p(10.0, 100.0), p(20.0, 105.0)],
        r##"{"color":"#123456"}"##,
    );
    let (a, b) = (anchor(&chart, price, 0), anchor(&chart, price, 1));
    let runs = texts(&mut chart);
    let (stats, _, stats_y) = runs
        .iter()
        .find(|(text, ..)| text.starts_with("+5.00  +5.00%"))
        .unwrap_or_else(|| panic!("price stats in {runs:?}"));
    assert!(stats.ends_with("ticks"), "{stats}");
    assert!(
        f64::from(*stats_y) < b.1,
        "above a rising range's upper edge"
    );
    let frame = chart.build_frame();
    let pane_prims = &frame.panes[0].main;
    let edges = pane_prims
        .iter()
        .filter(|prim| matches!(prim, Prim::HLine { color, .. } if *color == ink()))
        .count();
    assert_eq!(edges, 2, "both price edges");
    let fill = Color::rgba(0x12, 0x34, 0x56, 51);
    let regions = fills(&mut chart);
    assert!(regions.iter().any(|(_, color)| *color == fill), "20% fill");
    let arrowhead = regions
        .iter()
        .find(|(outline, color)| *color == ink() && outline.len() >= 6)
        .expect("arrowhead");
    let middle_x = (a.0 + b.0) / 2.0;
    assert!(
        close(arrowhead.0[0], (middle_x, b.1), 0.01),
        "the arrow points at the second price"
    );
    // The fill is a body target; turning it off leaves only the lines.
    let inside = (middle_x + 20.0, (a.1 + b.1) / 2.0);
    assert_eq!(hit(&chart, inside.0, inside.1), Some(price));
    assert!(chart.drawing_apply_options(price, r#"{"fill_enabled":false}"#));
    assert_eq!(hit(&chart, inside.0, inside.1), None);
    chart.remove_drawing(price);

    let date = add(
        &mut chart,
        DrawingKind::DateRange,
        vec![p(10.0, 100.0), p(20.0, 105.0)],
        r##"{"color":"#123456"}"##,
    );
    let (a, b) = (anchor(&chart, date, 0), anchor(&chart, date, 1));
    let runs = texts(&mut chart);
    let (_, _, y) = runs
        .iter()
        .find(|(text, ..)| text == "10 bars  10h")
        .unwrap_or_else(|| panic!("date stats in {runs:?}"));
    assert!(f64::from(*y) > a.1.max(b.1), "below the range");
    let frame = chart.build_frame();
    let verticals = frame.panes[0]
        .main
        .iter()
        .filter(|prim| matches!(prim, Prim::VLine { color, .. } if *color == ink()))
        .count();
    assert_eq!(verticals, 2);
    chart.remove_drawing(date);

    let both = add(
        &mut chart,
        DrawingKind::DateAndPriceRange,
        vec![p(10.0, 105.0), p(20.0, 100.0)],
        r##"{"color":"#123456"}"##,
    );
    let runs = texts_of(&mut chart);
    // The price formatter writes a typographic minus; percentages use ASCII.
    assert!(
        runs.iter()
            .any(|text| text.starts_with("\u{2212}5.00  -4.76%")),
        "{runs:?}"
    );
    assert!(runs.contains(&"10 bars  10h".to_string()), "{runs:?}");
    let arrowheads = fills(&mut chart)
        .into_iter()
        .filter(|(outline, color)| *color == ink() && outline.len() >= 6)
        .count();
    assert_eq!(arrowheads, 2, "a price arrow and a time arrow");
    // Hiding the stats removes the box.
    assert!(chart.drawing_apply_options(both, r#"{"labels":[]}"#));
    assert!(!texts_of(&mut chart)
        .iter()
        .any(|text| text.contains("bars")));
}

#[test]
fn projections_fill_the_sector_between_their_rays() {
    let mut chart = chart();
    let apex = p(10.0, 102.0);
    let (ox, oy) = chart
        .drawing_to_px_for(0, crate::DrawingPriceScale::Right, apex)
        .unwrap();
    let radius_point = chart
        .drawing_from_px_for(0, crate::DrawingPriceScale::Right, ox + 100.0, oy)
        .unwrap();
    let price_point = chart
        .drawing_from_px_for(0, crate::DrawingPriceScale::Right, ox + 60.0, oy - 60.0)
        .unwrap();
    let id = add(
        &mut chart,
        DrawingKind::Projection,
        vec![apex, radius_point, price_point],
        r##"{"color":"#123456"}"##,
    );
    let outline = ink_polylines(&mut chart).remove(0).0;
    assert!(close(outline[0], (ox, oy), 1e-3) && close(outline[outline.len() - 1], (ox, oy), 1e-3));
    for point in &outline[1..outline.len() - 1] {
        assert!(
            ((point.0 - ox).hypot(point.1 - oy) - 100.0).abs() < 1e-3,
            "arc radius"
        );
    }
    let last_arc = outline[outline.len() - 2];
    assert!(
        ((last_arc.1 - oy) / (last_arc.0 - ox) + 1.0).abs() < 1e-3,
        "ends on the 45° ray"
    );
    // Inside the 45° sector hits; outside the arc and below the first ray miss.
    assert_eq!(hit(&chart, ox + 70.0, oy - 20.0), Some(id));
    assert_eq!(hit(&chart, ox + 130.0, oy - 60.0), None);
    assert_eq!(hit(&chart, ox + 50.0, oy + 30.0), None);
    // Visible labels measure from the apex to the price point.
    assert!(chart.drawing_apply_options(
        id,
        r#"{"labels":[{"metric":"bar_count","visible":true,"position":"on"}]}"#
    ));
    assert!(texts_of(&mut chart)
        .iter()
        .any(|text| text.ends_with("bars")));
}

#[test]
fn projections_cull_and_hit_test_by_their_sector_box() {
    let mut chart = chart();
    // Enough drawings that candidate queries take the culled path.
    for index in 0..22 {
        add(
            &mut chart,
            DrawingKind::TrendLine,
            vec![p(30.0 + index as f64 * 0.1, 100.0), p(31.0, 100.5)],
            "{}",
        );
    }
    let candidate = |chart: &ChartEngine, id: DrawingId| {
        let candidates = chart.take_drawing_candidates(0, None);
        let found = candidates.contains(&id);
        chart.recycle_drawing_candidates(candidates);
        found
    };
    // Thousands of px left of the pane, far beyond its radius.
    let far = add(
        &mut chart,
        DrawingKind::Projection,
        vec![p(-300.0, 102.0), p(-295.0, 102.0), p(-297.0, 104.0)],
        "{}",
    );
    chart.build_frame();
    assert!(!candidate(&chart, far));
    chart.reset_drawing_work_stats();
    assert_eq!(hit(&chart, 50.0, 50.0), None);
    assert_eq!(chart.drawing_work_stats().precise_hit_tests, 0);

    // Its apex left of the pane, the sector still reaches into it: it paints and hits.
    let apex = (-60.0, chart.panes[0].top + 200.0);
    let points = [
        apex,
        (apex.0 + 200.0, apex.1),
        (apex.0 + 150.0, apex.1 - 150.0),
    ]
    .map(|(x, y)| {
        chart
            .drawing_from_px_for(0, crate::DrawingPriceScale::Right, x, y)
            .unwrap()
    });
    let reaching = add(
        &mut chart,
        DrawingKind::Projection,
        points.to_vec(),
        r##"{"color":"#123456"}"##,
    );
    chart.build_frame();
    assert!(candidate(&chart, reaching));
    assert_eq!(hit(&chart, apex.0 + 150.0, apex.1 - 40.0), Some(reaching));
    assert!(!ink_polylines(&mut chart).is_empty());
}

#[test]
fn anchored_text_is_pinned_to_its_pane_position() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::AnchoredText,
        vec![p(0.25, 0.1)],
        r#"{"text":"Plan"}"#,
    );
    let pane = (chart.pane_w, chart.panes[0].top, chart.panes[0].height);
    let expected = (0.25 * pane.0, pane.1 + 0.1 * pane.2);
    assert!(close(anchor(&chart, id, 0), expected, 1e-9));
    let at = |chart: &mut ChartEngine| {
        texts(chart)
            .into_iter()
            .find(|(text, ..)| text == "Plan")
            .map(|(_, x, y)| (f64::from(x), f64::from(y)))
            .unwrap()
    };
    let before = at(&mut chart);
    assert!(
        before.0 >= expected.0 && before.1 > expected.1,
        "left/top on the anchor"
    );
    // Scrolling, zooming, and an interval switch leave it in place.
    chart.set_visible_logical_range(-20.0, 10.0);
    assert!(close(at(&mut chart), before, 1e-3));
    let half_hourly = (0..80)
        .map(|index| index as f64 * HOUR / 2.0)
        .collect::<Vec<_>>();
    let values = vec![100.0; half_hourly.len()];
    chart
        .set_series_data(0, &half_hourly, &values, &values, &values, &values)
        .unwrap();
    let anchors = chart.drawing_anchors(id).unwrap();
    assert_eq!(anchors[0].logical, Some(0.25));
    assert_eq!(anchors[0].price, 0.1);
    assert_eq!(anchors[0].time, None, "pane fractions carry no time");
    // A body drag moves it by pane fractions; magnets never snap it.
    chart.set_drawing_magnet_mode(DrawingMagnetMode::Strong);
    chart.build_frame();
    let text = at(&mut chart);
    assert_eq!(hit(&chart, text.0 + 3.0, text.1), Some(id));
    assert!(chart.drawing_drag_start_at(text.0 + 3.0, text.1));
    chart.drawing_drag_to(text.0 + 83.0, text.1 + 50.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    let point = chart.drawing(id).unwrap().points[0];
    assert!((point.logical - (0.25 + 80.0 / pane.0)).abs() < 1e-9);
    assert!((point.price - (0.1 + 50.0 / pane.2)).abs() < 1e-9);
    // Dragged past the pane it clamps inside.
    assert!(chart.drawing_drag_start_at(text.0 + 83.0, text.1 + 50.0));
    chart.drawing_drag_to(
        text.0 + 5_000.0,
        text.1 - 5_000.0,
        DrawingModifiers::default(),
    );
    chart.drawing_drag_end();
    assert_eq!(chart.drawing(id).unwrap().points[0], p(1.0, 0.0));
    assert!(chart.undo_drawing() && chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points[0], p(0.25, 0.1));
    // Armed placement stores the click's pane fraction, unsnapped.
    assert!(chart.set_drawing_tool(Some(DrawingKind::AnchoredText), None, None));
    let placed = chart
        .drawing_tool_activate(
            200.0,
            pane.1 + 100.0,
            DrawingModifiers {
                magnet: true,
                straighten: false,
            },
        )
        .created
        .unwrap();
    assert_eq!(
        chart.drawing(placed).unwrap().points[0],
        p(200.0 / pane.0, 100.0 / pane.2)
    );
    // Price-basis rescales and paste offsets leave pane fractions alone.
    chart
        .rescale_drawing_prices(
            &[DrawingPriceSegment {
                from_time: None,
                to_time: None,
                factor: 2.0,
            }],
            None,
        )
        .unwrap();
    assert_eq!(chart.drawing(id).unwrap().points[0], p(0.25, 0.1));
    let copied = chart.copy_drawings_json(&[id]).unwrap();
    let pasted = chart.paste_drawings_json(&copied, 0, 5.0, 1.0).unwrap();
    assert_eq!(chart.drawing(pasted[0]).unwrap().points[0], p(0.25, 0.1));
    // Time-only anchors are rejected; persistence writes no anchor time.
    assert!(chart
        .add_drawing_anchors(
            DrawingKind::AnchoredText,
            0,
            &[DrawingAnchor {
                logical: None,
                price: 0.5,
                time: Some(HOUR)
            }],
            None
        )
        .is_err());
    let state: serde_json::Value =
        serde_json::from_str(&chart.export_state_json().unwrap()).unwrap();
    let saved = state["drawings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|drawing| drawing["kind"] == "anchored_text")
        .unwrap();
    assert!(saved["anchors"][0].get("time").is_none(), "{saved}");
}

#[test]
fn annotations_place_their_boxes_and_markers_around_the_anchor() {
    let mut chart = chart();
    let point = p(15.0, 103.0);
    let options = r##"{"color":"#123456"}"##;
    let comment = add(&mut chart, DrawingKind::Comment, vec![point], options);
    let (x, y) = anchor(&chart, comment, 0);
    let runs = texts(&mut chart);
    let (_, tx, ty) = runs.iter().find(|(text, ..)| text == "Comment").unwrap();
    assert!(
        f64::from(*tx) > x && f64::from(*ty) < y - 10.0,
        "above right of the tail"
    );
    // The tail tip is the anchor and hits; white text contrasts with the dark box.
    let tail = fills(&mut chart)
        .into_iter()
        .find(|(_, color)| *color == ink())
        .unwrap();
    assert!(close(tail.0[0], (x, y), 1e-3));
    assert_eq!(hit(&chart, x + 1.0, y - 3.0), Some(comment));
    let frame = chart.build_frame();
    assert!(frame.panes[0].main.iter().any(|prim| matches!(
        prim,
        Prim::Text { text, color, .. } if text == "Comment" && *color == Color::rgb(255, 255, 255)
    )));
    chart.remove_drawing(comment);

    let label = add(&mut chart, DrawingKind::PriceLabel, vec![point], options);
    assert!(texts_of(&mut chart).contains(&"103.00".to_string()));
    assert!(chart.drawing_apply_options(label, r##"{"text":"entry","text_color":"#ff0000"}"##));
    let frame = chart.build_frame();
    assert!(frame.panes[0].main.iter().any(|prim| matches!(
        prim,
        Prim::Text { text, color, .. } if text == "entry" && *color == Color::rgb(255, 0, 0)
    )));
    chart.remove_drawing(label);

    let price_note = add(
        &mut chart,
        DrawingKind::PriceNote,
        vec![point, p(20.0, 105.0)],
        options,
    );
    let (a, b) = (anchor(&chart, price_note, 0), anchor(&chart, price_note, 1));
    let runs = texts(&mut chart);
    let (_, tx, ty) = runs.iter().find(|(text, ..)| text == "103.00").unwrap();
    assert!(
        f64::from(*tx) > b.0 && (f64::from(*ty) - b.1).abs() < 1.0,
        "boxed at the second anchor"
    );
    assert_eq!(
        hit(&chart, (a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0),
        Some(price_note)
    );
    chart.remove_drawing(price_note);

    let callout = add(
        &mut chart,
        DrawingKind::Callout,
        vec![point, p(25.0, 106.0)],
        options,
    );
    let (tip, center) = (anchor(&chart, callout, 0), anchor(&chart, callout, 1));
    let runs = texts(&mut chart);
    let (_, tx, ty) = runs.iter().find(|(text, ..)| text == "Callout").unwrap();
    assert!(
        f64::from(*tx) < center.0 && (f64::from(*ty) - center.1).abs() < 1.0,
        "centered on the second anchor"
    );
    let pointer = (
        tip.0 + (center.0 - tip.0) * 0.2,
        tip.1 + (center.1 - tip.1) * 0.2,
    );
    assert_eq!(
        hit(&chart, pointer.0, pointer.1),
        Some(callout),
        "the pointer is a body target"
    );
    chart.remove_drawing(callout);

    // The note shows only its pin until it is hovered or selected (the reference platform's
    // note), unless it always shows its text.
    let note = add(&mut chart, DrawingKind::Note, vec![point], options);
    let (x, y) = anchor(&chart, note, 0);
    assert_eq!(hit(&chart, x, y - 17.0), Some(note), "the pin head");
    assert!(!texts_of(&mut chart).contains(&"Note".to_string()));
    assert_eq!(
        hit(&chart, x + 30.0, y - 17.0),
        None,
        "a hidden box is no target"
    );
    chart.set_hovered_drawing(Some(note));
    assert!(texts_of(&mut chart).contains(&"Note".to_string()));
    assert_eq!(
        hit(&chart, x + 30.0, y - 17.0),
        Some(note),
        "the revealed box"
    );
    chart.set_hovered_drawing(None);
    assert!(!texts_of(&mut chart).contains(&"Note".to_string()));
    chart.set_selected_drawing(Some(note));
    assert!(texts_of(&mut chart).contains(&"Note".to_string()));
    chart.set_selected_drawing(None);
    assert!(!texts_of(&mut chart).contains(&"Note".to_string()));
    assert!(chart.drawing_apply_options(
        note,
        r#"{"tool_options":{"projection_annotation":{"always_show_text":true}}}"#
    ));
    assert!(texts_of(&mut chart).contains(&"Note".to_string()));
    let mut restored = chart_with(&hourly(40), 1.0);
    restored
        .import_state_json(&chart.export_state_json().unwrap())
        .unwrap();
    assert!(restored.drawings()[0]
        .tool_options
        .projection_annotation
        .as_ref()
        .is_some_and(|block| block.always_show_text));
    chart.remove_drawing(note);

    let signpost = add(&mut chart, DrawingKind::Signpost, vec![point], options);
    let (x, y) = anchor(&chart, signpost, 0);
    let runs = texts(&mut chart);
    let (_, _, ty) = runs.iter().find(|(text, ..)| text == "Signpost").unwrap();
    assert!(f64::from(*ty) < y - 40.0, "the plate sits on the pole");
    assert_eq!(hit(&chart, x, y - 20.0), Some(signpost), "the pole");
    chart.remove_drawing(signpost);

    let flag = add(&mut chart, DrawingKind::FlagMark, vec![point], options);
    let (x, y) = anchor(&chart, flag, 0);
    assert_eq!(hit(&chart, x + 8.0, y - 18.0), Some(flag), "the flag");
    assert_eq!(hit(&chart, x + 8.0, y - 3.0), None, "beside the pole");
}

#[test]
fn arrow_marks_point_their_tip_at_the_anchor_with_text_past_the_tail() {
    let mut chart = chart();
    for (kind, direction) in [
        (DrawingKind::ArrowMarkUp, (0.0, -1.0)),
        (DrawingKind::ArrowMarkDown, (0.0, 1.0)),
        (DrawingKind::ArrowMarkLeft, (-1.0, 0.0)),
        (DrawingKind::ArrowMarkRight, (1.0, 0.0)),
    ] {
        let id = add(
            &mut chart,
            kind,
            vec![p(15.0, 103.0)],
            r##"{"color":"#123456","text":"buy"}"##,
        );
        let (x, y) = anchor(&chart, id, 0);
        let arrow = fills(&mut chart)
            .into_iter()
            .find(|(_, color)| *color == ink())
            .unwrap()
            .0;
        assert!(close(arrow[0], (x, y), 1e-3), "{kind:?} tip on the anchor");
        // The shaft runs back from the tip; the head and shaft both hit.
        let back = |distance: f64| (x - direction.0 * distance, y - direction.1 * distance);
        for distance in [3.0, 16.0] {
            let (hx, hy) = back(distance);
            assert_eq!(hit(&chart, hx, hy), Some(id), "{kind:?} at {distance}");
        }
        let side = back(18.0);
        let (sx, sy) = (side.0 + direction.1 * 7.0, side.1 - direction.0 * 7.0);
        assert_eq!(hit(&chart, sx, sy), None, "{kind:?} beside the shaft");
        let runs = texts(&mut chart);
        let (_, tx, ty) = runs.iter().find(|(text, ..)| text == "buy").unwrap();
        let along = (f64::from(*tx) - x) * -direction.0 + (f64::from(*ty) - y) * -direction.1;
        assert!(along > 22.0, "{kind:?} text past the tail");
        chart.remove_drawing(id);
    }
}

#[test]
fn icons_stamp_every_built_in_shape_with_exact_hit_regions() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::Icon,
        vec![p(15.0, 103.0)],
        r##"{"color":"#123456"}"##,
    );
    let (x, y) = anchor(&chart, id, 0);
    // The default star: its fan fills the star and its notches miss.
    assert_eq!(hit(&chart, x, y), Some(id));
    assert_eq!(hit(&chart, x, y - 11.0), Some(id), "the top spike");
    // The lower spikes end at (±7.05, 9.71) around an inner vertex at (0, 4.8).
    assert_eq!(
        hit(&chart, x, y + 9.0),
        None,
        "between the two lower spikes"
    );
    assert_eq!(
        hit(&chart, x + 6.5, y + 9.0),
        Some(id),
        "inside a lower spike"
    );
    for icon in [
        DrawingIcon::Heart,
        DrawingIcon::Check,
        DrawingIcon::Cross,
        DrawingIcon::Circle,
        DrawingIcon::Square,
        DrawingIcon::Diamond,
        DrawingIcon::TriangleUp,
        DrawingIcon::TriangleDown,
    ] {
        let name = serde_json::to_value(icon).unwrap();
        assert!(chart.drawing_apply_options(
            id,
            &format!(
                r#"{{"tool_options":{{"projection_annotation":{{"icon":{name},"icon_size":40}}}}}}"#
            )
        ));
        let frame = chart.build_frame();
        let painted = frame.panes[0].main.iter().any(|prim| match prim {
            Prim::BandFill { fill, .. } => *fill == ink(),
            Prim::Circle { fill, .. } => *fill == ink(),
            Prim::Polyline { color, .. } => *color == ink(),
            _ => false,
        });
        assert!(painted, "{icon:?} paints");
        let probe = match icon {
            // The check's corner at (-0.1, 0.28) of 40 px.
            DrawingIcon::Check => (x - 4.0, y + 11.2),
            DrawingIcon::Cross => (x + 6.0, y + 6.0),
            DrawingIcon::TriangleUp => (x, y + 10.0),
            DrawingIcon::TriangleDown => (x, y - 10.0),
            _ => (x, y),
        };
        assert_eq!(
            hit(&chart, probe.0, probe.1),
            Some(id),
            "{icon:?} hits inside"
        );
        assert_eq!(
            hit(&chart, x + 26.0, y + 26.0),
            None,
            "{icon:?} misses outside"
        );
    }
    // Sizes are bounded.
    for invalid in [
        r#"{"tool_options":{"projection_annotation":{"icon_size":4}}}"#,
        r#"{"tool_options":{"projection_annotation":{"icon_size":500}}}"#,
        r#"{"tool_options":{"projection_annotation":{"icon":"rocket"}}}"#,
    ] {
        assert!(!chart.drawing_apply_options(id, invalid), "{invalid}");
    }
}

#[test]
fn indexed_hit_testing_matches_brute_force_for_every_tool() {
    let mut chart = chart();
    for copy in 0..3 {
        let shift = copy as f64 * 0.9;
        for kind in KINDS {
            let points = points_for(kind)
                .into_iter()
                .map(|point| {
                    if kind == DrawingKind::AnchoredText {
                        p(point.logical + shift * 0.1, point.price + shift * 0.1)
                    } else {
                        p(point.logical + shift, point.price + shift * 0.3)
                    }
                })
                .collect();
            add(&mut chart, kind, points, "{}");
        }
    }
    assert!(
        chart.drawings().len() > 20,
        "exercises the culled candidate path"
    );
    chart.build_frame();
    let mut hits = 0;
    for gy in 0..48 {
        for gx in 0..78 {
            let (x, y) = (f64::from(gx) * 10.0 + 3.0, f64::from(gy) * 10.0 + 4.0);
            let indexed = chart.hit_test_drawing(x, y);
            assert_eq!(
                indexed,
                chart.hit_test_drawing_bruteforce(x, y),
                "({x}, {y})"
            );
            hits += usize::from(indexed.is_some());
        }
    }
    assert!(hits > 100, "the grid meets the drawings ({hits} hits)");
}

#[test]
fn drags_nudges_and_magnet_edit_family_tools_as_single_history_entries() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::Callout,
        vec![p(10.0, 101.0), p(20.0, 104.0)],
        "{}",
    );
    let before = chart.drawing(id).unwrap().points.clone();
    chart.set_selected_drawing(Some(id));
    assert_eq!(chart.drawing_handle_count(id), Some(2));
    let (tx, ty) = anchor(&chart, id, 0);
    assert!(chart.drawing_drag_start_at(tx, ty));
    chart.drawing_drag_to(tx - 30.0, ty + 12.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    assert_eq!(
        chart.drawing(id).unwrap().points[1],
        before[1],
        "only the tip moves"
    );
    assert!(close(anchor(&chart, id, 0), (tx - 30.0, ty + 12.0), 1e-6));
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);

    // Keyboard: one handle per anchor; nudging the whole drawing is one undo step.
    let (x0, y0) = anchor(&chart, id, 1);
    assert!(chart.nudge_selected_drawing(0.0, -10.0, None));
    let (x1, y1) = anchor(&chart, id, 1);
    assert!((x1 - x0).abs() < 1e-6 && (y1 - y0 + 10.0).abs() < 1e-6);
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);
    let note = add(&mut chart, DrawingKind::Note, vec![p(12.0, 102.0)], "{}");
    assert_eq!(chart.drawing_handle_count(note), Some(1));

    // The chart magnet snaps a placed marker to the bar's rendered values.
    chart.set_drawing_magnet_mode(DrawingMagnetMode::Strong);
    assert!(chart.set_drawing_tool(Some(DrawingKind::FlagMark), None, None));
    let x = chart.logical_to_coordinate(12.0).unwrap() + 3.0;
    let y = chart
        .series_price_to_coordinate(0, value_at(12) + 0.4)
        .unwrap();
    let flag = chart
        .drawing_tool_activate(x, y, DrawingModifiers::default())
        .created
        .unwrap();
    let point = chart.drawing(flag).unwrap().points[0];
    assert_eq!(point.logical, 12.0);
    assert!(
        (point.price - (value_at(12) + 0.5)).abs() < 1e-9,
        "snapped to the high"
    );

    // Locked drawings select but never drag.
    assert!(chart.drawing_apply_options(note, r#"{"locked":true}"#));
    let (nx, ny) = anchor(&chart, note, 0);
    assert!(!chart.drawing_drag_start_at(nx, ny - 17.0));
    assert_eq!(chart.selected_drawing(), Some(note));
}

#[test]
fn anchors_resolve_by_time_across_an_interval_switch() {
    let mut chart = chart();
    let id = chart
        .add_drawing_anchors(
            DrawingKind::PriceNote,
            0,
            &[
                DrawingAnchor {
                    logical: None,
                    price: 101.0,
                    time: Some(10.0 * HOUR),
                },
                DrawingAnchor {
                    logical: None,
                    price: 103.0,
                    time: Some(20.5 * HOUR),
                },
            ],
            None,
        )
        .unwrap();
    let half_hourly = (0..80)
        .map(|index| index as f64 * HOUR / 2.0)
        .collect::<Vec<_>>();
    let values = vec![100.0; half_hourly.len()];
    chart
        .set_series_data(0, &half_hourly, &values, &values, &values, &values)
        .unwrap();
    let anchors = chart.drawing_anchors(id).unwrap();
    assert_eq!(anchors[0].logical, Some(20.0));
    assert_eq!(anchors[1].logical, Some(41.0));
    assert_eq!(anchors[1].time, Some(20.5 * HOUR));
}

#[test]
fn schema_kind_options_and_option_patches_are_typed_and_atomic() {
    let schema = crate::drawing_property_schema(DrawingKind::BarsPattern);
    let find = |schema: &crate::DrawingPropertySchema, name: &str| {
        schema
            .properties
            .iter()
            .find(|property| property.name == name)
            .unwrap_or_else(|| panic!("{name} descriptor"))
            .clone()
    };
    let mode = find(&schema, "tool_options.projection_annotation.bars_mode");
    assert_eq!(mode.default, serde_json::json!("hl_bars"));
    assert_eq!(
        mode.enum_values,
        [
            "hl_bars",
            "oc_bars",
            "line_open",
            "line_high",
            "line_low",
            "line_close"
        ]
    );
    assert_eq!(
        find(&schema, "tool_options.projection_annotation.mirrored").default,
        false
    );
    let icon_schema = crate::drawing_property_schema(DrawingKind::Icon);
    let size = find(&icon_schema, "tool_options.projection_annotation.icon_size");
    assert_eq!((size.min, size.max), (Some(8.0), Some(128.0)));
    assert_eq!(size.default, serde_json::json!(24.0));
    assert_eq!(
        find(&icon_schema, "tool_options.projection_annotation.icon")
            .enum_values
            .len(),
        9
    );
    let note = crate::drawing_property_schema(DrawingKind::Note);
    assert_eq!(
        find(&note, "tool_options.projection_annotation.always_show_text").default,
        false
    );
    let comment = crate::drawing_property_schema(DrawingKind::Comment);
    assert_eq!(find(&comment, "text").default, "Comment");
    assert!(!comment
        .properties
        .iter()
        .any(|property| property.name.starts_with("tool_options")));
    let range = crate::drawing_property_schema(DrawingKind::PriceRange);
    assert_eq!(find(&range, "fill_enabled").default, true);
    assert_eq!(find(&range, "stroke_end").default, "arrow");
    let arrow = crate::drawing_property_schema(DrawingKind::ArrowMarkDown);
    assert_eq!(
        find(&arrow, "color").default,
        aeris_charts_core::style::MARKET_DOWN_CSS
    );

    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::BarsPattern,
        vec![p(10.0, 1.0), p(14.0, 1.0)],
        "{}",
    );
    let kind_options = |chart: &ChartEngine| {
        serde_json::from_str::<serde_json::Value>(&chart.drawing_kind_options_json(id).unwrap())
            .unwrap()
    };
    assert_eq!(
        kind_options(&chart),
        serde_json::json!({
            "kind": "projection_annotation",
            "bars_mode": "hl_bars",
            "mirrored": false,
            "flipped": false,
            "pattern_bars": 5,
            "icon": "star",
            "icon_size": 24.0,
            "always_show_text": false
        })
    );
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"projection_annotation":{"bars_mode":"oc_bars"}},"width":3}"#
    ));
    assert_eq!(kind_options(&chart)["bars_mode"], "oc_bars");
    assert_eq!(
        kind_options(&chart)["pattern_bars"],
        5,
        "absent keys keep the copy"
    );
    let before = chart.drawing(id).unwrap().clone();
    let too_many = serde_json::json!({
        "tool_options": {"projection_annotation": {"bars": vec![[1.0, 2.0, 0.5, 1.5]; MAX_BARS_PATTERN_BARS + 1]}}
    });
    for invalid in [
        r#"{"tool_options":{"projection_annotation":{"bars_mode":"candles"}},"width":9}"#
            .to_string(),
        r#"{"tool_options":{"projection_annotation":7}}"#.to_string(),
        too_many.to_string(),
    ] {
        assert!(!chart.drawing_apply_options(id, &invalid), "{invalid}");
        assert_eq!(chart.drawing(id).unwrap(), &before);
    }
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"projection_annotation":null}}"#));
    assert!(chart.drawing(id).unwrap().tool_options.is_empty());
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().tool_options, before.tool_options);
    assert_eq!(
        chart
            .drawing(id)
            .unwrap()
            .tool_options
            .projection_annotation
            .as_ref()
            .unwrap()
            .bars_mode,
        BarsPatternMode::OcBars
    );
}

#[test]
fn persistence_round_trips_every_tool_and_omits_kind_defaults() {
    let mut chart = chart();
    for kind in KINDS {
        if kind != DrawingKind::BarsPattern {
            add(&mut chart, kind, points_for(kind), "{}");
        }
    }
    let exported: serde_json::Value =
        serde_json::from_str(&chart.export_state_json().unwrap()).unwrap();
    for drawing in exported["drawings"].as_array().unwrap() {
        let style = &drawing["style"];
        for field in [
            "fill_enabled",
            "stroke_end",
            "labels",
            "tool_options",
            "text",
        ] {
            assert!(
                style.get(field).is_none(),
                "{} writes its default {field}",
                drawing["kind"]
            );
        }
    }
    let customized = [
        (
            DrawingKind::BarsPattern,
            r#"{"tool_options":{"projection_annotation":{"bars_mode":"line_high","mirrored":true}}}"#,
        ),
        (
            DrawingKind::PriceRange,
            r#"{"fill_enabled":false,"stroke_end":"none","labels":[]}"#,
        ),
        (DrawingKind::Comment, r#"{"text":""}"#),
        (
            DrawingKind::Callout,
            r##"{"text":"two\nlines","text_h_align":"left","box_border_color":"#ffffff"}"##,
        ),
        (
            DrawingKind::Icon,
            r#"{"tool_options":{"projection_annotation":{"icon":"heart","icon_size":48}}}"#,
        ),
        (
            DrawingKind::AnchoredText,
            r##"{"box_color":"#202020","text":"pinned"}"##,
        ),
        (
            DrawingKind::ArrowMarkUp,
            r##"{"color":"#ff00ff","text":"long"}"##,
        ),
    ];
    for (kind, options) in customized {
        add(&mut chart, kind, points_for(kind), options);
    }
    let document = chart.export_state_json().unwrap();
    let mut restored = chart_with(&hourly(40), 1.0);
    restored.import_state_json(&document).unwrap();
    assert_eq!(restored.export_state_json().unwrap(), document);
    for (restored, original) in restored.drawings().iter().zip(chart.drawings()) {
        let mut original = original.clone();
        original.pending_times.clear();
        assert_eq!(restored.kind, original.kind);
        assert_eq!(restored.points, original.points, "{:?}", original.kind);
        assert_eq!(restored.text, original.text, "{:?}", original.kind);
        assert_eq!(restored.fill_enabled, original.fill_enabled);
        assert_eq!(restored.stroke_end, original.stroke_end);
        assert_eq!(restored.labels, original.labels);
        assert_eq!(restored.color, original.color);
        assert_eq!(
            restored.tool_options, original.tool_options,
            "{:?}",
            original.kind
        );
    }
    let cleared = restored
        .drawings()
        .iter()
        .rfind(|drawing| drawing.kind == DrawingKind::Comment)
        .unwrap();
    assert_eq!(cleared.text, "", "a cleared default text stays cleared");
    let pattern = restored
        .drawings()
        .iter()
        .find(|drawing| drawing.kind == DrawingKind::BarsPattern)
        .unwrap();
    assert_eq!(
        pattern
            .tool_options
            .projection_annotation
            .as_ref()
            .unwrap()
            .bars
            .len(),
        11
    );
}

#[test]
fn clipboard_and_sync_carry_copied_bars_and_family_options() {
    let mut source = chart();
    let pattern = add(
        &mut source,
        DrawingKind::BarsPattern,
        vec![p(10.0, 1.0), p(20.0, 1.0)],
        "{}",
    );
    let icon = add(
        &mut source,
        DrawingKind::Icon,
        vec![p(12.0, 102.0)],
        r#"{"tool_options":{"projection_annotation":{"icon":"check"}}}"#,
    );
    let copied = source.copy_drawings_json(&[pattern, icon]).unwrap();
    // A target chart with different data keeps the copied bars instead of recapturing.
    let mut target = chart_with(&hourly(60), 1.0);
    let values = vec![500.0; 60];
    target
        .set_series_data(0, &hourly(60), &values, &values, &values, &values)
        .unwrap();
    let pasted = target.paste_drawings_json(&copied, 0, 2.0, 0.0).unwrap();
    for (id, original) in pasted.iter().zip([pattern, icon]) {
        assert_eq!(
            target.drawing(*id).unwrap().tool_options,
            source.drawing(original).unwrap().tool_options
        );
    }
    assert_eq!(
        target.drawing(pasted[0]).unwrap().points[0].logical,
        12.0,
        "offset applied"
    );

    let payload = source.drawing_sync_payload_json("cell-a").unwrap();
    let mut mirror = chart();
    assert!(mirror.apply_drawing_sync_payload_json(&payload));
    for (mirrored, original) in mirror.drawings().iter().zip(source.drawings()) {
        assert_eq!(mirrored.kind, original.kind);
        assert_eq!(mirrored.tool_options, original.tool_options);
        assert_eq!(mirrored.points, original.points);
    }
}

#[test]
fn frames_scale_family_geometry_with_the_device_pixel_ratio() {
    for dpr in [1.0, 1.5, 2.0] {
        let mut chart = chart_with(&hourly(40), dpr);
        let id = add(
            &mut chart,
            DrawingKind::ArrowMarkUp,
            vec![p(15.0, 103.0)],
            r##"{"color":"#123456"}"##,
        );
        let (x, y) = anchor(&chart, id, 0);
        let hpr = (chart.pane_w * dpr).round() / chart.pane_w;
        let vpr = (chart.pane_h * dpr).round() / chart.pane_h;
        let arrow = fills(&mut chart)
            .into_iter()
            .find(|(_, color)| *color == ink())
            .unwrap()
            .0;
        assert!(
            close(arrow[0], (x * hpr, y * vpr), 1e-3),
            "tip at dpr {dpr}"
        );
        let tail = arrow.iter().map(|point| point.1).fold(f64::MIN, f64::max);
        assert!(
            (tail - (y * vpr + 22.0 * vpr)).abs() < 1e-3,
            "length scales at dpr {dpr}"
        );
        let comment = add(&mut chart, DrawingKind::Comment, vec![p(25.0, 103.0)], "{}");
        let frame = chart.build_frame();
        let size = frame.panes[0]
            .main
            .iter()
            .find_map(|prim| match prim {
                Prim::Text { text, size, .. } if text == "Comment" => Some(f64::from(*size)),
                _ => None,
            })
            .unwrap();
        let expected = chart.options.get().layout.font_size * vpr;
        assert!((size - expected).abs() < 1e-3, "glyphs scale at dpr {dpr}");
        chart.remove_drawing(comment);
    }
}

#[test]
fn family_tools_tolerate_charts_without_data_and_degenerate_anchors() {
    let mut empty = ChartEngine::new(800.0, 500.0, 1.0);
    for kind in KINDS {
        assert!(
            empty.add_drawing(kind, 0, points_for(kind), None).is_some(),
            "{kind:?}"
        );
    }
    empty.build_frame();
    assert_eq!(empty.hit_test_drawing(100.0, 100.0), None);

    let mut chart = chart();
    assert!(chart
        .add_drawing(
            DrawingKind::Forecast,
            0,
            vec![p(f64::NAN, 1.0), p(2.0, 3.0)],
            None
        )
        .is_none());
    for kind in KINDS {
        let point = if kind == DrawingKind::AnchoredText {
            p(0.5, 0.5)
        } else {
            p(12.0, 102.0)
        };
        assert!(chart
            .add_drawing(kind, 0, vec![point; kind.anchor_count()], None)
            .is_some());
    }
    let frame = chart.build_frame();
    assert!(frame.panes[0]
        .points
        .iter()
        .all(|point| point[0].is_finite() && point[1].is_finite()));
    // Culling keeps anchors scrolled far away out of the candidates.
    chart.set_visible_logical_range(200.0, 240.0);
    let candidates = chart.take_drawing_candidates(0, None);
    let kinds = candidates
        .iter()
        .filter_map(|id| chart.drawing(*id).map(|drawing| drawing.kind))
        .collect::<Vec<_>>();
    chart.recycle_drawing_candidates(candidates);
    assert!(!kinds.contains(&DrawingKind::Comment) && !kinds.contains(&DrawingKind::PriceRange));
    assert!(
        kinds.contains(&DrawingKind::AnchoredText),
        "pane-anchored text never culls"
    );
}

#[test]
fn stats_labels_and_hits_follow_the_drawing_onto_a_lower_pane() {
    let mut chart = chart();
    let pane = chart.add_pane(true).unwrap();
    let series = chart.add_series(crate::SeriesKind::Line);
    let values = (0..40)
        .map(|index| 10.0 + index as f64 * 0.1)
        .collect::<Vec<_>>();
    chart
        .set_series_data(series, &hourly(40), &values, &values, &values, &values)
        .unwrap();
    chart.set_series_pane(series, pane, 1.0);
    chart.build_frame();
    let id = chart
        .add_drawing(
            DrawingKind::DateRange,
            pane,
            vec![p(10.0, 10.5), p(20.0, 11.5)],
            None,
        )
        .unwrap();
    let frame = chart.build_frame();
    assert!(frame.panes[pane]
        .main
        .iter()
        .any(|prim| matches!(prim, Prim::Text { text, .. } if text == "10 bars  10h")));
    assert!(!frame.panes[0]
        .main
        .iter()
        .any(|prim| matches!(prim, Prim::Text { text, .. } if text.contains("bars"))));
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    assert_eq!(
        hit(&chart, (a.0 + b.0) / 2.0 + 7.0, (a.1 + b.1) / 2.0 + 5.0),
        Some(id)
    );
    let part = chart.hit_test_drawing(a.0, a.1).map(|hit| hit.part);
    assert_eq!(part, Some(DrawingDragPart::Body));
}

/// The first body target of drawing `id` on a coarse grid.
fn body_point(chart: &ChartEngine, id: DrawingId) -> (f64, f64) {
    (0..80)
        .flat_map(|gx| (0..50).map(move |gy| (gx as f64 * 10.0 + 2.0, gy as f64 * 10.0 + 3.0)))
        .find(|&(x, y)| {
            chart
                .hit_test_drawing(x, y)
                .is_some_and(|hit| hit.id == id && hit.part == DrawingDragPart::Body)
        })
        .unwrap_or_else(|| panic!("{:?} has a body target", chart.drawing(id).unwrap().kind))
}

fn anchors_px(chart: &ChartEngine, id: DrawingId) -> Vec<(f64, f64)> {
    (0..chart.drawing(id).unwrap().points.len())
        .map(|index| anchor(chart, id, index))
        .collect()
}

#[test]
fn every_tool_drags_and_nudges_each_handle_and_its_body_as_single_history_entries() {
    for kind in KINDS {
        let mut chart = chart();
        let id = add(&mut chart, kind, points_for(kind), "{}");
        chart.set_selected_drawing(Some(id));
        chart.build_frame();
        let count = kind.anchor_count();
        assert_eq!(chart.drawing_handle_count(id), Some(count), "{kind:?}");
        let start = anchors_px(&chart, id);
        for handle in 0..count {
            // Pointer drag of the handle: only that anchor follows the pointer.
            let (x, y) = start[handle];
            let hit = chart.hit_test_drawing(x, y).unwrap();
            assert_eq!(
                (hit.id, hit.part),
                (id, DrawingDragPart::Anchor(handle)),
                "{kind:?}"
            );
            assert!(chart.drawing_drag_start_at(x, y));
            chart.drawing_drag_to(x + 13.0, y - 9.0, DrawingModifiers::default());
            chart.drawing_drag_end();
            let moved = anchors_px(&chart, id);
            for (index, (&now, &before)) in moved.iter().zip(&start).enumerate() {
                let expected = if index == handle {
                    (before.0 + 13.0, before.1 - 9.0)
                } else {
                    before
                };
                assert!(
                    close(now, expected, 1e-6),
                    "{kind:?} handle {handle} anchor {index}"
                );
            }
            assert!(chart.undo_drawing(), "{kind:?}");
            // Keyboard nudge of the same handle.
            assert!(chart.nudge_selected_drawing(0.0, -10.0, Some(handle)));
            let nudged = anchor(&chart, id, handle);
            assert!(
                close(nudged, (start[handle].0, start[handle].1 - 10.0), 1e-6),
                "{kind:?} nudged handle {handle}"
            );
            assert!(chart.undo_drawing());
            assert!(close(anchor(&chart, id, handle), start[handle], 1e-6));
        }
        // Body drag and body nudge translate every anchor rigidly.
        let (x, y) = body_point(&chart, id);
        assert!(chart.drawing_drag_start_at(x, y), "{kind:?}");
        chart.drawing_drag_to(x + 17.0, y + 11.0, DrawingModifiers::default());
        chart.drawing_drag_end();
        for (now, before) in anchors_px(&chart, id).into_iter().zip(&start) {
            assert!(
                close(now, (before.0 + 17.0, before.1 + 11.0), 1e-6),
                "{kind:?} body drag"
            );
        }
        assert!(chart.undo_drawing());
        assert!(chart.nudge_selected_drawing(5.0, 0.0, None));
        for (now, before) in anchors_px(&chart, id).into_iter().zip(&start) {
            assert!(
                close(now, (before.0 + 5.0, before.1), 1e-6),
                "{kind:?} body nudge"
            );
        }
        assert!(chart.undo_drawing());
        for (now, before) in anchors_px(&chart, id).into_iter().zip(&start) {
            assert!(close(now, *before, 1e-6), "{kind:?} undone");
        }
    }
}

#[test]
fn every_tool_honors_visibility_lock_and_z_order() {
    for kind in KINDS {
        let mut chart = chart();
        let clean = chart.build_frame().panes[0].main.len();
        let lower = add(&mut chart, kind, points_for(kind), "{}");
        let upper = add(&mut chart, kind, points_for(kind), "{}");
        chart.set_selected_drawing(None);
        chart.build_frame();
        let (x, y) = body_point(&chart, upper);
        assert_eq!(hit(&chart, x, y), Some(upper), "{kind:?} topmost first");
        // Z-order: bringing the lower copy forward makes it the target.
        assert!(chart.move_drawing_z_order(lower, 1));
        assert_eq!(hit(&chart, x, y), Some(lower), "{kind:?} reordered");
        assert!(chart.undo_drawing());
        assert_eq!(hit(&chart, x, y), Some(upper));
        // Hidden drawings neither paint nor hit, and stay in the object list.
        assert!(chart.set_drawing_visibility(lower, false));
        assert!(chart.set_drawing_visibility(upper, false));
        chart.set_selected_drawing(None);
        assert_eq!(chart.build_frame().panes[0].main.len(), clean, "{kind:?}");
        assert_eq!(hit(&chart, x, y), None, "{kind:?} hidden");
        assert_eq!(chart.drawings().len(), 2);
        assert!(chart.set_drawing_visibility(upper, true));
        assert_eq!(hit(&chart, x, y), Some(upper));
        // Locked drawings select but never open a drag, and ignore nudges.
        assert!(chart.set_drawing_locked(upper, true));
        let before = chart.drawing(upper).unwrap().points.clone();
        assert!(!chart.drawing_drag_start_at(x, y), "{kind:?} locked");
        assert_eq!(chart.selected_drawing(), Some(upper));
        assert!(!chart.nudge_selected_drawing(4.0, 4.0, None));
        assert_eq!(chart.drawing(upper).unwrap().points, before);
    }
}

#[test]
fn every_time_anchored_tool_keeps_its_anchor_times_across_an_interval_switch() {
    for kind in KINDS {
        let mut chart = chart();
        let anchors = points_for(kind)
            .into_iter()
            .map(|point| DrawingAnchor {
                logical: None,
                price: point.price,
                time: (kind != DrawingKind::AnchoredText).then_some(point.logical * HOUR),
            })
            .map(|anchor| {
                if kind == DrawingKind::AnchoredText {
                    DrawingAnchor {
                        logical: Some(0.3),
                        ..anchor
                    }
                } else {
                    anchor
                }
            })
            .collect::<Vec<_>>();
        let id = chart
            .add_drawing_anchors(kind, 0, &anchors, None)
            .unwrap_or_else(|error| panic!("{kind:?}: {error:?}"));
        let before = chart.drawing_anchors(id).unwrap();
        let half_hourly = (0..80)
            .map(|index| index as f64 * HOUR / 2.0)
            .collect::<Vec<_>>();
        let values = (0..80).map(value_at).collect::<Vec<_>>();
        chart
            .set_series_data(0, &half_hourly, &values, &values, &values, &values)
            .unwrap();
        let after = chart.drawing_anchors(id).unwrap();
        for (now, then) in after.iter().zip(&before) {
            if kind == DrawingKind::AnchoredText {
                assert_eq!(now, then, "pane fractions stay put");
                assert_eq!(now.time, None);
            } else {
                let time = then.time.unwrap();
                assert_eq!(now.time, Some(time), "{kind:?}");
                assert_eq!(now.logical, Some(time / HOUR * 2.0), "{kind:?}");
                assert_eq!(now.price, then.price);
            }
        }
    }
}

#[test]
fn pane_fractions_clamp_into_the_pane_and_out_of_pane_documents_fail() {
    let mut chart = chart();
    for (outside, clamped) in [
        (p(1.5, 0.2), p(1.0, 0.2)),
        (p(0.2, -0.1), p(0.2, 0.0)),
        (p(-0.01, 7.0), p(0.0, 1.0)),
    ] {
        let id = add(&mut chart, DrawingKind::AnchoredText, vec![outside], "{}");
        assert_eq!(chart.drawing(id).unwrap().points, vec![clamped]);
    }
    let id = add(
        &mut chart,
        DrawingKind::AnchoredText,
        vec![p(0.3, 0.4)],
        "{}",
    );
    assert!(chart.set_drawing_anchors(id, &[p(2.0, 0.4).into()]).is_ok());
    assert_eq!(chart.drawing(id).unwrap().points, vec![p(1.0, 0.4)]);

    // A document whose pane fraction leaves the pane fails without mutating the chart.
    let document = chart.export_state_json().unwrap();
    let mut state: serde_json::Value = serde_json::from_str(&document).unwrap();
    state["drawings"][0]["anchors"][0]["logical"] = serde_json::json!(3.0);
    let mut restored = chart_with(&hourly(40), 1.0);
    assert!(restored.import_state_json(&state.to_string()).is_err());
    assert!(restored.drawings().is_empty());
}

#[test]
fn pane_anchored_text_ignores_non_time_bar_identities() {
    use crate::{
        AggressorSide, FootprintAggregationOptions, FootprintBarAggregation,
        FootprintSeriesOptions, FootprintTrade,
    };
    let trade = |timestamp_micros, price| FootprintTrade {
        timestamp_micros,
        price,
        volume: 1.0,
        aggressor: AggressorSide::Buy,
        bid: None,
        ask: None,
        sequence: None,
        trade_id: None,
        conditions: 0,
        session_id: Some(1),
    };
    let options = || FootprintSeriesOptions {
        aggregation: FootprintAggregationOptions {
            tick_size: 1.0,
            ticks_per_row: 1,
            bars: FootprintBarAggregation::Trades { trades_per_bar: 1 },
            ..FootprintAggregationOptions::default()
        },
        ..FootprintSeriesOptions::default()
    };
    let trades = vec![trade(2_000_001, 100.0), trade(3_000_001, 101.0)];
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let footprint = chart.add_footprint_series(options()).unwrap();
    chart
        .set_footprint_trades(footprint, trades.clone())
        .unwrap();
    let text = add(
        &mut chart,
        DrawingKind::AnchoredText,
        vec![p(0.2, 0.25)],
        "{}",
    );
    let line = add(
        &mut chart,
        DrawingKind::PriceNote,
        vec![p(0.0, 100.0), p(1.0, 101.0)],
        "{}",
    );
    let document = chart.export_state_json().unwrap();
    let mut state: serde_json::Value = serde_json::from_str(&document).unwrap();
    let drawings = state["drawings"].as_array_mut().unwrap();
    assert!(
        drawings[1].get("anchor_times_micros").is_some(),
        "{document}"
    );
    assert!(
        drawings[0].get("anchor_times_micros").is_none(),
        "pane fractions carry no bar identity"
    );
    // A stray bar identity in a document never moves a pane-anchored drawing.
    drawings[0]["anchor_times_micros"] =
        serde_json::json!([drawings[1]["anchor_times_micros"][1].clone()]);
    let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
    restored.import_state_json(&state.to_string()).unwrap();
    let footprint = restored.add_footprint_series(options()).unwrap();
    restored.set_footprint_trades(footprint, trades).unwrap();
    assert_eq!(restored.drawing(text).unwrap().points, vec![p(0.2, 0.25)]);
    assert_eq!(
        restored.drawing(line).unwrap().points,
        chart.drawing(line).unwrap().points
    );
}

#[test]
fn derived_points_share_the_anchor_space_when_bitmap_ratios_differ() {
    // 801 × 1.5 rounds to 1202 bitmap px, so x scales by 1202/801 while y scales by 1.5. Frame
    // construction debug-asserts that `PartContext::point_px` maps every anchor onto its px,
    // including the pane fractions of anchored text.
    let mut chart = ChartEngine::new(801.0, 500.0, 1.5);
    let times = hourly(40);
    let closes = (0..times.len()).map(value_at).collect::<Vec<_>>();
    chart
        .set_series_data(0, &times, &closes, &closes, &closes, &closes)
        .unwrap();
    chart.time_scale.set_width(801.0);
    chart.fit_content();
    chart.build_frame();
    let hpr = (chart.pane_w * 1.5).round() / chart.pane_w;
    let vpr = (chart.pane_h * 1.5).round() / chart.pane_h;
    assert!(
        (hpr - vpr).abs() > 1e-4,
        "the ratios differ: {hpr} vs {vpr}"
    );
    let pattern = add(
        &mut chart,
        DrawingKind::BarsPattern,
        vec![p(10.0, 0.0), p(20.0, 0.0)],
        r##"{"color":"#123456","tool_options":{"projection_annotation":{"bars_mode":"line_close"}}}"##,
    );
    add(
        &mut chart,
        DrawingKind::AnchoredText,
        vec![p(0.4, 0.3)],
        r##"{"color":"#123456"}"##,
    );
    let (a, b) = (anchor(&chart, pattern, 0), anchor(&chart, pattern, 1));
    let line = ink_polylines(&mut chart).remove(0).0;
    assert_eq!(line.len(), 11);
    // The ghost's ends sit on the anchors' times, and every copied close lands on its source
    // through the same ratios.
    assert!((line[0].0 - a.0 * hpr).abs() < 1e-3 && (line[10].0 - b.0 * hpr).abs() < 1e-3);
    for index in [0, 5, 10] {
        let x = chart.logical_to_coordinate(10.0 + index as f64).unwrap();
        let y = chart
            .series_price_to_coordinate(0, value_at(10 + index))
            .unwrap();
        assert!(
            close(line[index], (x * hpr, y * vpr), 1e-3),
            "close {index}"
        );
    }
}

// --- review regressions ---------------------------------------------------------------------------

/// Candles on `times` whose rows past `real` are whitespace (host-installed future session slots).
fn chart_with_future_slots(times: &[f64], real: usize) -> ChartEngine {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let at = |offset: f64| {
        (0..times.len())
            .map(|index| {
                if index < real {
                    value_at(index) + offset
                } else {
                    f64::NAN
                }
            })
            .collect::<Vec<_>>()
    };
    chart
        .set_series_data(0, times, &at(-0.25), &at(0.5), &at(-0.5), &at(0.0))
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.build_frame();
    chart
}

#[test]
fn forecasts_stay_pending_over_future_whitespace_session_slots() {
    // Real bars 0..=20, then whitespace slots 21..=39: the session has not traded past bar 20.
    // No bar reaches the 107 target, whose outcome box stays on the pane.
    let mut chart = chart_with_future_slots(&hourly(40), 21);
    let id = add(
        &mut chart,
        DrawingKind::Forecast,
        vec![p(10.0, 101.0), p(20.0, 107.0)],
        r##"{"color":"#123456"}"##,
    );
    let outcome = |chart: &mut ChartEngine| {
        texts_of(chart)
            .into_iter()
            .find(|text| text == "Success" || text == "Failure")
    };
    assert_eq!(outcome(&mut chart), None, "empty slots are not bars");
    // A target inside the empty slots stays pending as well.
    assert!(chart
        .set_drawing_anchors(id, &[p(10.0, 101.0).into(), p(30.0, 107.0).into()])
        .is_ok());
    assert_eq!(outcome(&mut chart), None);
    // The first traded bar after the target decides the failure.
    assert!(chart
        .set_drawing_anchors(id, &[p(10.0, 101.0).into(), p(20.0, 107.0).into()])
        .is_ok());
    assert!(chart.update_series_bar(0, 21.0 * HOUR, [100.0, 100.5, 99.5, 100.0]));
    assert_eq!(outcome(&mut chart).as_deref(), Some("Failure"));
}

#[test]
fn forecasts_follow_live_updates_that_leave_every_scale_unchanged() {
    // Bars 36..=39 close at 101..=104; the 105 target sits on the latest bar, still pending.
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::Forecast,
        vec![p(35.0, 100.0), p(39.0, 105.0)],
        r##"{"color":"#123456"}"##,
    );
    let outcome = |chart: &mut ChartEngine| {
        texts_of(chart)
            .into_iter()
            .find(|text| text == "Success" || text == "Failure")
    };
    assert_eq!(outcome(&mut chart), None);
    // A tick on that bar reaches the target without moving the time or price scales (the
    // chart's range stays 99.5..=106.5): the retained drawings layer still rebuilds.
    let revisions = chart.panes[0].scale_revisions();
    assert!(chart.update_series_bar(0, 39.0 * HOUR, [103.75, 105.25, 103.5, 105.0]));
    assert_eq!(outcome(&mut chart).as_deref(), Some("Success"));
    assert_eq!(chart.panes[0].scale_revisions(), revisions);
    assert!(chart.drawing(id).is_some());
}

#[test]
fn group_moves_leave_pane_anchored_text_pinned_and_documents_restorable() {
    let mut chart = chart();
    let text = add(
        &mut chart,
        DrawingKind::AnchoredText,
        vec![p(0.9, 0.9)],
        r#"{"group_id":"g"}"#,
    );
    let note = add(
        &mut chart,
        DrawingKind::Note,
        vec![p(15.0, 103.0)],
        r#"{"group_id":"g"}"#,
    );
    assert_eq!(chart.move_drawing_group("g", 5.0, 2.0), 1);
    assert_eq!(chart.drawing(text).unwrap().points, vec![p(0.9, 0.9)]);
    assert_eq!(chart.drawing(note).unwrap().points, vec![p(20.0, 105.0)]);
    let document = chart.export_state_json().unwrap();
    let mut restored = chart_with(&hourly(40), 1.0);
    restored.import_state_json(&document).unwrap();
    assert_eq!(restored.drawing(text).unwrap().points, vec![p(0.9, 0.9)]);
}

#[test]
fn style_templates_keep_each_annotations_own_text() {
    for kind in [
        DrawingKind::Note,
        DrawingKind::Callout,
        DrawingKind::Comment,
        DrawingKind::Signpost,
        DrawingKind::AnchoredText,
    ] {
        let mut chart = chart();
        let anchors = |at: f64| match kind.anchor_count() {
            1 if kind == DrawingKind::AnchoredText => vec![p(0.2 + at / 100.0, 0.5)],
            1 => vec![p(at, 103.0)],
            _ => vec![p(at, 103.0), p(at + 4.0, 105.0)],
        };
        let source = add(
            &mut chart,
            kind,
            anchors(10.0),
            r##"{"text":"Source words","text_color":"#123456","text_size":19}"##,
        );
        let target = add(&mut chart, kind, anchors(20.0), r#"{"text":"My own note"}"#);
        let template = chart.drawing_template_json(source, "loud").unwrap();
        assert!(
            chart.apply_drawing_template_json(target, &template),
            "{kind:?}"
        );
        let restyled = chart.drawing(target).unwrap();
        assert_eq!(restyled.text, "My own note", "{kind:?}");
        assert_eq!(restyled.text_color.as_deref(), Some("#123456"), "{kind:?}");
        assert_eq!(restyled.text_size, Some(19.0), "{kind:?}");
    }
}

#[test]
fn templates_carry_style_but_never_a_patterns_copied_bars() {
    let mut chart = chart();
    let source = add(
        &mut chart,
        DrawingKind::BarsPattern,
        vec![p(10.0, 0.0), p(14.0, 0.0)],
        r##"{"color":"#123456","tool_options":{"projection_annotation":{"bars_mode":"oc_bars","flipped":true}}}"##,
    );
    let template = chart.drawing_template_json(source, "ghost").unwrap();
    assert!(!template.contains("\"bars\""), "{template}");
    // Applying the template restyles another pattern and keeps that pattern's own copy.
    let other = add(
        &mut chart,
        DrawingKind::BarsPattern,
        vec![p(20.0, 0.0), p(30.0, 0.0)],
        "{}",
    );
    let own = chart.drawing(other).unwrap().tool_options.clone();
    assert_eq!(own.projection_annotation.as_ref().unwrap().bars.len(), 11);
    assert!(chart.apply_drawing_template_json(other, &template));
    let applied = chart
        .drawing(other)
        .unwrap()
        .tool_options
        .projection_annotation
        .clone()
        .unwrap();
    assert_eq!(applied.bars, own.projection_annotation.unwrap().bars);
    assert_eq!(applied.bars_mode, BarsPatternMode::OcBars);
    assert!(applied.flipped);
    assert_eq!(chart.drawing(other).unwrap().color, INK);
    // Creating from the template's options copies the new pattern's own range.
    let template: crate::DrawingTemplate = serde_json::from_str(&template).unwrap();
    let created = add(
        &mut chart,
        DrawingKind::BarsPattern,
        vec![p(25.0, 0.0), p(27.0, 0.0)],
        &template.options.to_string(),
    );
    let copy = chart
        .drawing(created)
        .unwrap()
        .tool_options
        .projection_annotation
        .clone()
        .unwrap();
    assert_eq!(copy.bars.len(), 3);
    assert_eq!(copy.bars[0][3], value_at(25));
    assert_eq!(copy.bars_mode, BarsPatternMode::OcBars);
}

#[test]
fn three_anchor_placement_shows_the_placed_apex_and_a_provisional_line() {
    let mut chart = chart();
    assert!(chart.set_drawing_tool(
        Some(DrawingKind::Projection),
        Some(r##"{"color":"#123456"}"##),
        None
    ));
    chart.drawing_tool_activate(200.0, 260.0, DrawingModifiers::default());
    chart.drawing_tool_pointer_move(420.0, 150.0, DrawingModifiers::default(), false);
    let lines = ink_polylines(&mut chart);
    assert!(
        lines.iter().any(|(line, _)| line.len() == 2
            && close(line[0], (200.0, 260.0), 1e-3)
            && close(line[1], (420.0, 150.0), 1e-3)),
        "a provisional line runs from the apex to the pointer: {lines:?}"
    );
    let frame = chart.build_frame();
    let handles = frame.panes[0]
        .main
        .iter()
        .filter(|prim| {
            matches!(prim, Prim::Circle { cx, cy, .. }
                if close((f64::from(*cx), f64::from(*cy)), (200.0, 260.0), 1e-3))
        })
        .count();
    assert_eq!(handles, 2, "the placed apex shows its handle disc pair");
    // The second click previews the whole sector; the third commits it.
    chart.drawing_tool_activate(420.0, 150.0, DrawingModifiers::default());
    chart.drawing_tool_pointer_move(380.0, 120.0, DrawingModifiers::default(), false);
    assert!(!fills(&mut chart).is_empty(), "the sector previews");
    let id = chart
        .drawing_tool_activate(380.0, 120.0, DrawingModifiers::default())
        .created
        .unwrap();
    assert_eq!(chart.drawing(id).unwrap().points.len(), 3);
}

#[test]
fn bars_patterns_fit_their_copy_into_the_anchor_box_without_blowing_up() {
    // A swing whose first and last closes nearly coincide (100.00 → 110 → 100.01).
    let closes = [100.0, 104.0, 108.0, 110.0, 106.0, 102.0, 100.01];
    let times = hourly(closes.len() + 10);
    let close_at = |index: usize| closes.get(index).copied().unwrap_or(100.0);
    let column = |offset: f64| {
        (0..times.len())
            .map(|index| close_at(index) + offset)
            .collect::<Vec<_>>()
    };
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &times,
            &column(-0.25),
            &column(0.5),
            &column(-0.5),
            &column(0.0),
        )
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.build_frame();
    let id = add(
        &mut chart,
        DrawingKind::BarsPattern,
        vec![p(0.0, 0.0), p(6.0, 0.0)],
        r##"{"color":"#123456"}"##,
    );
    // The anchors span the copy's box: its first bar at the highest high, its last bar at the
    // lowest low, so the ghost overlays its source exactly.
    assert_eq!(
        chart.drawing(id).unwrap().points,
        vec![p(0.0, 110.5), p(6.0, 99.5)]
    );
    let y = |chart: &ChartEngine, price: f64| chart.series_price_to_coordinate(0, price).unwrap();
    for (index, &(_, top, bottom)) in ink_vlines(&mut chart).iter().enumerate() {
        assert!((f64::from(top) - y(&chart, close_at(index) + 0.5)).abs() <= 1.0);
        assert!((f64::from(bottom) - y(&chart, close_at(index) - 0.5)).abs() <= 1.0);
    }
    // Lowering the second anchor by half a point stretches the ghost by 0.5 / 11: it stays inside
    // the box between the anchors instead of scaling by the tiny close-to-close span.
    assert!(chart
        .set_drawing_anchors(id, &[p(0.0, 110.5).into(), p(6.0, 99.0).into()])
        .is_ok());
    let sticks = ink_vlines(&mut chart);
    let (top, bottom) = (y(&chart, 110.5), y(&chart, 99.0));
    assert!(sticks
        .iter()
        .all(|&(_, y0, y1)| f64::from(y0) >= top - 1.0 && f64::from(y1) <= bottom + 1.0));
    assert!(
        (f64::from(sticks[3].1) - top).abs() <= 1.0,
        "the high pins to the top"
    );
    assert!(
        (f64::from(sticks[6].2) - bottom).abs() <= 1.0,
        "the low pins to the bottom"
    );
    // Flipped turns the copy upside down within the same box.
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"projection_annotation":{"flipped":true}}}"#
    ));
    let flipped = ink_vlines(&mut chart);
    assert!(
        (f64::from(flipped[3].2) - bottom).abs() <= 1.0,
        "the high flips to the bottom"
    );
    assert!(
        (f64::from(flipped[6].1) - top).abs() <= 1.0,
        "the low flips to the top"
    );
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"projection_annotation":{"flipped":false}}}"#
    ));
    // A price-basis rescale scales the ghost exactly with its anchors.
    let ghost_prices = |chart: &ChartEngine| {
        let drawing = chart.drawing(id).unwrap();
        let block = drawing.tool_options.projection_annotation.clone().unwrap();
        let ghost = super::Ghost::new(&block, drawing.points[0], drawing.points[1]);
        block
            .bars
            .iter()
            .flatten()
            .map(|&value| ghost.price(value))
            .collect::<Vec<_>>()
    };
    let before = ghost_prices(&chart);
    assert!(chart
        .rescale_drawing_prices(
            &[DrawingPriceSegment {
                from_time: None,
                to_time: None,
                factor: 0.5,
            }],
            None,
        )
        .is_ok());
    assert_eq!(
        chart.drawing(id).unwrap().points,
        vec![p(0.0, 55.25), p(6.0, 49.5)]
    );
    for (after, before) in ghost_prices(&chart).into_iter().zip(before) {
        assert!((after - before * 0.5).abs() < 1e-9, "{after} vs {before}");
    }
    // The ghost stays inside its anchors' price box, so it culls like any finite drawing.
    assert_eq!(
        DrawingKind::BarsPattern.spec().price_extent,
        super::DrawingPriceExtent::Finite
    );
}

#[test]
fn every_tool_stays_finite_and_indexed_on_log_and_percentage_scales_at_extreme_zoom() {
    use crate::PriceScaleMode;
    for mode in [PriceScaleMode::Logarithmic, PriceScaleMode::Percentage] {
        let mut chart = chart();
        chart.set_price_scale_mode(0, false, mode);
        for kind in KINDS {
            // Inside the data, and entirely in the future area beyond the last bar.
            for shift in [0.0, 60.0] {
                let points = points_for(kind)
                    .into_iter()
                    .map(|point| {
                        if kind == DrawingKind::AnchoredText {
                            point
                        } else {
                            p(point.logical + shift, point.price)
                        }
                    })
                    .collect();
                add(&mut chart, kind, points, r##"{"color":"#123456"}"##);
            }
        }
        for (from, to) in [
            (-5.0, 45.0),
            (14.9, 15.1),
            (-50_000.0, 50_000.0),
            (50.0, 90.0),
        ] {
            chart.set_visible_logical_range(from, to);
            let frame = chart.build_frame();
            assert!(
                frame.panes[0]
                    .points
                    .iter()
                    .all(|point| point[0].is_finite() && point[1].is_finite()),
                "{mode:?} {from}..{to}"
            );
            for gy in 0..24 {
                for gx in 0..39 {
                    let (x, y) = (f64::from(gx) * 20.0 + 7.0, f64::from(gy) * 20.0 + 9.0);
                    assert_eq!(
                        chart.hit_test_drawing(x, y),
                        chart.hit_test_drawing_bruteforce(x, y),
                        "{mode:?} {from}..{to} ({x}, {y})"
                    );
                }
            }
        }
    }
}

/// The painted `Prim::Text` runs of the first pane as `(text, x, y, color)`.
fn text_runs(chart: &mut ChartEngine) -> Vec<(String, f64, f64, Color)> {
    let frame = chart.build_frame();
    frame.panes[0]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::Text {
                text, x, y, color, ..
            } => Some((text.clone(), f64::from(*x), f64::from(*y), *color)),
            _ => None,
        })
        .collect()
}

fn sync_revision(chart: &ChartEngine) -> u64 {
    let payload: serde_json::Value =
        serde_json::from_str(&chart.drawing_sync_payload_json("cell").unwrap()).unwrap();
    payload["revision"].as_u64().unwrap()
}

#[test]
fn text_boxes_are_editable_in_place_as_multiline_boxes() {
    let mut chart = chart();
    for kind in KINDS {
        let id = add(&mut chart, kind, points_for(kind), "{}");
        let expected = matches!(
            kind,
            DrawingKind::AnchoredText
                | DrawingKind::Note
                | DrawingKind::PriceNote
                | DrawingKind::Callout
                | DrawingKind::Comment
                | DrawingKind::PriceLabel
                | DrawingKind::Signpost
                | DrawingKind::ArrowMarkUp
                | DrawingKind::ArrowMarkDown
                | DrawingKind::ArrowMarkLeft
                | DrawingKind::ArrowMarkRight
        );
        assert_eq!(chart.drawing_text_editable(id), expected, "{kind:?}");
        assert_eq!(
            chart.drawing_text_edit_layout(id).is_some(),
            expected,
            "{kind:?}"
        );
        assert_eq!(
            chart.begin_drawing_text_edit(id, false),
            expected,
            "{kind:?}"
        );
        assert_eq!(chart.commit_drawing_text_edit(), expected, "{kind:?}");
    }
    // The text tool and trend labels edit through the same session, as one-line runs.
    let text = add(&mut chart, DrawingKind::Text, vec![p(12.0, 102.0)], "{}");
    let trend = add(
        &mut chart,
        DrawingKind::TrendLine,
        vec![p(10.0, 101.0), p(20.0, 105.0)],
        "{}",
    );
    let rectangle = add(
        &mut chart,
        DrawingKind::Rectangle,
        vec![p(10.0, 101.0), p(20.0, 105.0)],
        r#"{"text":"box"}"#,
    );
    for id in [text, trend, rectangle] {
        assert!(chart.drawing_text_editable(id));
        assert!(!chart.drawing_text_edit_layout(id).unwrap().multiline);
    }
    // Locked, hidden, and interval-hidden drawings never open an editor.
    let comment = add(&mut chart, DrawingKind::Comment, vec![p(15.0, 103.0)], "{}");
    for patch in [
        r#"{"locked":true}"#,
        r#"{"visible":false}"#,
        r#"{"interval_visibility":{"enabled":true,"intervals":[]}}"#,
    ] {
        let before = chart.drawing(comment).unwrap().clone();
        assert!(chart.drawing_apply_options(comment, patch), "{patch}");
        assert!(!chart.drawing_text_editable(comment), "{patch}");
        assert!(!chart.begin_drawing_text_edit(comment, false), "{patch}");
        assert!(chart.undo_drawing());
        assert_eq!(chart.drawing(comment).unwrap(), &before);
    }
    assert!(chart.drawing_text_editable(comment));
}

/// Valid anchors for any catalog tool inside the fixture chart's data: pane fractions for a
/// pane-anchored tool, otherwise the tool's minimum anchor count on a gentle zigzag.
fn catalog_points(kind: DrawingKind) -> Vec<DrawingPoint> {
    if kind.pane_anchored() {
        return vec![p(0.3, 0.2)];
    }
    let count = kind.anchor_count();
    (0..count)
        .map(|index| {
            let step = index as f64;
            p(
                8.0 + step * 12.0 / count.max(2) as f64,
                if index % 2 == 0 {
                    101.5 + step * 0.2
                } else {
                    105.0 - step * 0.2
                },
            )
        })
        .collect()
}

/// The tools whose drawing paints no text of its own, by design: their `text` is accepted but
/// never painted, so there is nothing to edit in place.
const TEXTLESS: [DrawingKind; 8] = [
    DrawingKind::Forecast,
    DrawingKind::BarsPattern,
    DrawingKind::PriceRange,
    DrawingKind::DateRange,
    DrawingKind::DateAndPriceRange,
    DrawingKind::Projection,
    DrawingKind::FlagMark,
    DrawingKind::Icon,
];

#[test]
fn every_tool_is_text_editable_exactly_when_it_paints_its_text() {
    let mut chart = chart();
    let mut editable_tools = 0;
    for spec in crate::drawings::DRAWING_TOOL_SPECS {
        let kind = spec.kind;
        let id = add(&mut chart, kind, catalog_points(kind), r#"{"text":"t"}"#);
        let expected = !TEXTLESS.contains(&kind);
        editable_tools += usize::from(expected);
        assert_eq!(chart.drawing_text_editable(id), expected, "{kind:?}");
        let layout = chart.drawing_text_edit_layout(id);
        assert_eq!(layout.is_some(), expected, "{kind:?}");
        if let Some(layout) = layout {
            // Family boxes take several lines; every other drawing edits one rotated or level
            // run, which the engine keeps on one line.
            assert_eq!(
                layout.multiline,
                spec.family.is_some_and(|family| family.owns_text),
                "{kind:?}"
            );
            assert!(layout.x.is_finite() && layout.y.is_finite() && layout.angle.is_finite());
            assert!(layout.size > 0.0 && layout.line_height > layout.size);
        }
        assert_eq!(
            chart.begin_drawing_text_edit(id, false),
            expected,
            "{kind:?}"
        );
        assert_eq!(chart.editing_drawing().is_some(), expected, "{kind:?}");
        assert_eq!(chart.commit_drawing_text_edit(), expected, "{kind:?}");
        // Locked, hidden, and interval-hidden drawings never open an editor.
        for patch in [
            r#"{"locked":true}"#,
            r#"{"visible":false}"#,
            r#"{"interval_visibility":{"enabled":true,"intervals":[]}}"#,
        ] {
            assert!(chart.drawing_apply_options(id, patch), "{kind:?} {patch}");
            assert!(!chart.drawing_text_editable(id), "{kind:?} {patch}");
            assert!(
                !chart.begin_drawing_text_edit(id, false),
                "{kind:?} {patch}"
            );
            assert!(chart.undo_drawing());
        }
        assert_eq!(chart.drawing_text_editable(id), expected, "{kind:?}");
    }
    assert_eq!(crate::drawings::DRAWING_TOOL_SPECS.len(), 84);
    assert_eq!(editable_tools, 76);
}

#[test]
fn the_edit_layout_is_the_painted_text_box() {
    let mut chart = chart();
    let size =
        chart.drawing_text_size(&crate::Drawing::new(1, DrawingKind::Comment, 0, Vec::new()));
    // The comment's box holds only its text; the price label's starts with its price.
    let comment = add(
        &mut chart,
        DrawingKind::Comment,
        vec![p(15.0, 103.0)],
        r#"{"text":"first\nsecond"}"#,
    );
    let layout = chart.drawing_text_edit_layout(comment).unwrap();
    let runs = text_runs(&mut chart);
    let (_, x, y, color) = runs.iter().find(|(text, ..)| text == "first").unwrap();
    assert!((layout.x - x).abs() < 1e-3 && (layout.y - y).abs() < 1e-3);
    let (_, _, y2, _) = runs.iter().find(|(text, ..)| text == "second").unwrap();
    assert!((layout.y + layout.line_height - y2).abs() < 1e-3);
    assert_eq!(layout.line_height, size * 1.25);
    assert_eq!(
        (layout.size, layout.weight, layout.italic),
        (size, 400, false)
    );
    assert_eq!(layout.color, color.to_css());
    let [left, top, right, bottom] = layout.rect;
    assert!(left < layout.x && right > layout.x && top < layout.y && bottom > *y2);
    assert_eq!(
        hit(&chart, (left + right) / 2.0, (top + bottom) / 2.0),
        Some(comment)
    );
    chart.remove_drawing(comment);

    let label = add(
        &mut chart,
        DrawingKind::PriceLabel,
        vec![p(15.0, 103.0)],
        r#"{"text":"note","text_size":20,"text_weight":700,"text_italic":true}"#,
    );
    let layout = chart.drawing_text_edit_layout(label).unwrap();
    let runs = text_runs(&mut chart);
    let (_, price_x, price_y, _) = runs.iter().find(|(text, ..)| text == "103.00").unwrap();
    let (_, x, y, _) = runs.iter().find(|(text, ..)| text == "note").unwrap();
    assert!((layout.x - price_x).abs() < 1e-3 && (layout.x - x).abs() < 1e-3);
    assert!((layout.y - (price_y + layout.line_height)).abs() < 1e-3);
    assert!(
        (layout.y - y).abs() < 1e-3,
        "the text follows the price line"
    );
    assert_eq!(
        (layout.size, layout.weight, layout.italic),
        (20.0, 700, true)
    );
}

/// One painted generic text run: `(x, y, clockwise angle, align, size, weight, italic, color)`.
type PaintedRun = (f64, f64, f64, TextAlign, f64, u16, bool, Color);

/// The painted run of `text` in the first pane, whether it lowers to `Text` or `RotatedText`.
fn painted_run(chart: &mut ChartEngine, wanted: &str) -> PaintedRun {
    let frame = chart.build_frame();
    frame.panes[0]
        .main
        .iter()
        .find_map(|prim| match prim {
            Prim::Text {
                text,
                x,
                y,
                color,
                size,
                align,
                weight,
                italic,
                ..
            } if text == wanted => Some((
                f64::from(*x),
                f64::from(*y),
                0.0,
                *align,
                f64::from(*size),
                *weight,
                *italic,
                *color,
            )),
            Prim::RotatedText {
                text,
                x,
                y,
                color,
                size,
                align,
                weight,
                italic,
                angle,
                ..
            } if text == wanted => Some((
                f64::from(*x),
                f64::from(*y),
                f64::from(*angle),
                *align,
                f64::from(*size),
                *weight,
                *italic,
                *color,
            )),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{wanted:?} is painted"))
}

/// Where a run painted at anchor `(x, y)` with `align` and clockwise `angle` starts (its left
/// edge, vertically centered), for the advance `width`.
fn run_start(run: &PaintedRun, width: f64) -> (f64, f64) {
    let (x, y, angle, align, ..) = *run;
    let left = match align {
        TextAlign::Left => 0.0,
        TextAlign::Center => -width / 2.0,
        TextAlign::Right => -width,
    };
    (x + angle.cos() * left, y + angle.sin() * left)
}

#[test]
fn the_run_edit_layout_is_where_the_frame_paints_the_generic_label() {
    // The frame places runs in bitmap px, the layout in media px: they agree at any pixel ratio.
    for dpr in [1.0, 2.0] {
        let mut chart = chart_with(&hourly(40), dpr);
        let cases = [
            // A ray's label follows its stroke: rotated, top-right slot, stroke-colored.
            (
                DrawingKind::Ray,
                r##"{"text":"ray text","color":"#123456","text_size":18,"text_weight":700,"text_italic":true}"##,
                "ray text",
            ),
            // A rectangle's label is level and centered in its box; a circle's in its shape box.
            (DrawingKind::Rectangle, r#"{"text":"box text"}"#, "box text"),
            (
                DrawingKind::Circle,
                r#"{"text":"circle text"}"#,
                "circle text",
            ),
        ];
        for (kind, options, wanted) in cases {
            let id = add(&mut chart, kind, points_for(kind), options);
            let layout = chart.drawing_text_edit_layout(id).unwrap();
            let mut painted = painted_run(&mut chart, wanted);
            let (.., weight, italic, color) = painted;
            // Bitmap px to media px.
            painted.0 /= dpr;
            painted.1 /= dpr;
            painted.4 /= dpr;
            let size = painted.4;
            let width = wanted.chars().count() as f64 * size * 0.6;
            let (x, y) = run_start(&painted, width);
            let label = format!("{kind:?} at {dpr}x");
            assert!((layout.x - x).abs() < 1e-3, "{label} x {} vs {x}", layout.x);
            assert!((layout.y - y).abs() < 1e-3, "{label} y {} vs {y}", layout.y);
            assert!((layout.angle - painted.2).abs() < 1e-6, "{label}");
            assert!((layout.size - size).abs() < 1e-9, "{label}");
            assert_eq!((layout.weight, layout.italic), (weight, italic), "{label}");
            assert_eq!(layout.color, color.to_css(), "{label}");
            assert_eq!(
                layout.color,
                chart
                    .drawing_label_color(chart.drawing(id).unwrap())
                    .to_css()
            );
            assert!(!layout.multiline);
            assert!((layout.line_height - size * 1.2).abs() < 1e-9);
            // The engine's caret transform and the layout agree on the run.
            let (tx, ty, angle) = chart.drawing_text_transform(id).unwrap();
            assert!((tx - painted.0).abs() < 1e-3 && (ty - painted.1).abs() < 1e-3);
            assert!((angle - painted.2).abs() < 1e-6);
            // The rect bounds the padded run box, whose center is the label's center.
            let [left, top, right, bottom] = layout.rect;
            let center = (x + angle.cos() * width / 2.0, y + angle.sin() * width / 2.0);
            assert!(
                ((left + right) / 2.0 - center.0).abs() < 1e-3
                    && ((top + bottom) / 2.0 - center.1).abs() < 1e-3,
                "{label}"
            );
            assert!(right - left >= width && bottom - top >= size, "{label}");
            if kind == DrawingKind::Ray {
                assert_eq!(
                    layout.color,
                    ink().to_css(),
                    "a segment label follows the stroke"
                );
                assert!(angle.abs() > 0.1, "the ray label is rotated");
            }
            // The wasm host reads the layout as JSON: the new fields ride along.
            let json = serde_json::to_value(&layout).unwrap();
            assert_eq!(json["angle"], layout.angle, "{label}");
            assert_eq!(json["multiline"], false, "{label}");
            chart.remove_drawing(id);
        }
    }
}

#[test]
fn an_empty_run_label_keeps_a_one_em_caret_slot_while_edited() {
    let mut chart = chart();
    let ray = add(
        &mut chart,
        DrawingKind::Ray,
        points_for(DrawingKind::Ray),
        r#"{"text_h_align":"right","text_v_align":"top"}"#,
    );
    let anchor = chart.drawing_text_transform(ray).unwrap();
    let layout = chart.drawing_text_edit_layout(ray).unwrap();
    let em = layout.size;
    // A right-aligned empty run opens one em to the left of its aligned anchor, along the stroke.
    assert!((layout.x - (anchor.0 - anchor.2.cos() * em)).abs() < 1e-3);
    assert!((layout.y - (anchor.1 - anchor.2.sin() * em)).abs() < 1e-3);
    assert!(chart.begin_drawing_text_edit(ray, false));
    assert!(chart.set_drawing_text_edit("wider than one em", usize::MAX));
    let typed = chart.drawing_text_edit_layout(ray).unwrap();
    let width = "wider than one em".chars().count() as f64 * em * 0.6;
    assert!((typed.x - (anchor.0 - anchor.2.cos() * width)).abs() < 1e-3);
}

#[test]
fn an_edit_session_is_one_undo_step_and_one_sync_revision() {
    let mut chart = chart();
    let comment = add(&mut chart, DrawingKind::Comment, vec![p(15.0, 103.0)], "{}");
    let created = chart.drawing(comment).unwrap().clone();
    let revision = sync_revision(&chart);

    // Live text repaints and relays out, but records nothing until the session ends.
    assert!(!chart.set_drawing_text_edit("orphan", 0), "no session open");
    assert!(chart.begin_drawing_text_edit(comment, false));
    assert_eq!(chart.editing_drawing(), Some(comment));
    let one_line = chart.drawing_text_edit_layout(comment).unwrap();
    for text in ["A", "AB", "AB\nC"] {
        assert!(chart.set_drawing_text_edit(text, usize::MAX));
    }
    assert!(texts_of(&mut chart).contains(&"C".to_string()));
    let two_lines = chart.drawing_text_edit_layout(comment).unwrap();
    assert!(
        (one_line.y - two_lines.y - two_lines.line_height).abs() < 1e-6,
        "the bottom-aligned bubble grows upward, so the first line moves up"
    );
    assert_eq!(sync_revision(&chart), revision);
    assert!(chart.commit_drawing_text_edit());
    assert_eq!(chart.editing_drawing(), None);
    assert!(!chart.commit_drawing_text_edit(), "already closed");
    assert_eq!(chart.drawing(comment).unwrap().text, "AB\nC");
    assert_eq!(sync_revision(&chart), revision + 1);

    // One undo step reverts the whole edit; redo replays it.
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(comment).unwrap(), &created);
    assert!(chart.redo_drawing());
    assert_eq!(chart.drawing(comment).unwrap().text, "AB\nC");

    // Cancel restores the text it began from without a history entry.
    let committed = chart.drawing(comment).unwrap().clone();
    assert!(chart.begin_drawing_text_edit(comment, false));
    assert!(chart.set_drawing_text_edit("discarded", usize::MAX));
    assert!(chart.cancel_drawing_text_edit());
    assert_eq!(chart.drawing(comment).unwrap(), &committed);
    assert!(chart.undo_drawing());
    assert_eq!(
        chart.drawing(comment).unwrap(),
        &created,
        "undo reverts the committed edit, not the cancelled one"
    );
    assert!(chart.redo_drawing());

    // An unchanged session records nothing; undo during a session commits it first.
    assert!(chart.begin_drawing_text_edit(comment, false));
    assert!(chart.commit_drawing_text_edit());
    assert!(chart.begin_drawing_text_edit(comment, false));
    assert!(chart.set_drawing_text_edit("typed", usize::MAX));
    assert!(chart.undo_drawing());
    assert_eq!(chart.editing_drawing(), None);
    assert_eq!(chart.drawing(comment).unwrap().text, "AB\nC");
    assert!(chart.redo_drawing());
    assert_eq!(chart.drawing(comment).unwrap().text, "typed");

    // The committed text persists and syncs; the session itself never does.
    let document = chart.export_state_json().unwrap();
    let mut restored = chart_with(&hourly(40), 1.0);
    restored.import_state_json(&document).unwrap();
    assert_eq!(restored.drawings()[0].text, "typed");
    let mut mirror = chart_with(&hourly(40), 1.0);
    assert!(
        mirror.apply_drawing_sync_payload_json(&chart.drawing_sync_payload_json("cell-a").unwrap())
    );
    assert_eq!(mirror.drawings()[0].text, "typed");
    let mirrored = mirror.drawings()[0].id;
    assert!(mirror.begin_drawing_text_edit(mirrored, false));
    assert!(chart.drawing_apply_options(comment, r#"{"text":"synced"}"#));
    assert!(
        mirror.apply_drawing_sync_payload_json(&chart.drawing_sync_payload_json("cell-b").unwrap())
    );
    assert_eq!(
        mirror.editing_drawing(),
        None,
        "a sync payload ends the session"
    );
    assert!(chart.begin_drawing_text_edit(comment, false));
    assert!(chart.remove_drawing(comment));
    assert_eq!(chart.editing_drawing(), None, "removal ends the session");
}

#[test]
fn an_emptied_text_box_keeps_its_caret_line_while_edited() {
    let mut chart = chart();
    let comment = add(&mut chart, DrawingKind::Comment, vec![p(15.0, 103.0)], "{}");
    let boxes = |chart: &mut ChartEngine| {
        let frame = chart.build_frame();
        frame.panes[0]
            .main
            .iter()
            .filter(|prim| matches!(prim, Prim::Rect { .. }))
            .count()
    };
    let resting = boxes(&mut chart);
    assert!(chart.begin_drawing_text_edit(comment, false));
    assert!(chart.set_drawing_text_edit("", usize::MAX));
    assert_eq!(boxes(&mut chart), resting, "the caret line keeps the box");
    let layout = chart.drawing_text_edit_layout(comment).unwrap();
    let [left, _, right, _] = layout.rect;
    assert!(right - left > 0.0);
    assert!(chart.commit_drawing_text_edit());
    assert_eq!(
        boxes(&mut chart),
        resting - 1,
        "an empty comment paints no box"
    );
    assert_eq!(
        chart
            .drawing_text_edit_layout(comment)
            .map(|edit| edit.rect),
        Some(layout.rect),
        "the editor reopens on the same caret line"
    );
    assert!(chart.drawing_text_editable(comment));

    // A price note keeps its price line and adds the caret line below it.
    let price_note = add(
        &mut chart,
        DrawingKind::PriceNote,
        vec![p(10.0, 101.0), p(20.0, 105.0)],
        "{}",
    );
    assert!(chart.begin_drawing_text_edit(price_note, false));
    let layout = chart.drawing_text_edit_layout(price_note).unwrap();
    let runs = text_runs(&mut chart);
    let (_, _, price_y, _) = runs.iter().find(|(text, ..)| text == "101.00").unwrap();
    assert!((layout.y - price_y - layout.line_height).abs() < 1e-3);
    assert!(chart.commit_drawing_text_edit());

    // A note hidden until focus reveals its text box while it is edited.
    let note = add(&mut chart, DrawingKind::Note, vec![p(25.0, 104.0)], "{}");
    assert!(!texts_of(&mut chart).contains(&"Note".to_string()));
    assert!(chart.begin_drawing_text_edit(note, false));
    assert!(texts_of(&mut chart).contains(&"Note".to_string()));
    assert!(chart.cancel_drawing_text_edit());
    assert!(!texts_of(&mut chart).contains(&"Note".to_string()));
}

/// Union axis d1 d2 d3 d5 d6 d8 (the primary on pane 0) and an as-of candle overlay alone on pane
/// 1 trading d1 d2 d4 d5 d7 d8, whose d4 bar (high 120) collapses into the d5 point.
fn as_of_overlay() -> ChartEngine {
    const DAY: f64 = 86_400.0;
    let days = |list: &[f64]| list.iter().map(|day| day * DAY).collect::<Vec<_>>();
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let flat = [50.0; 6];
    chart
        .set_series_data(
            0,
            &days(&[1.0, 2.0, 3.0, 5.0, 6.0, 8.0]),
            &flat,
            &flat,
            &flat,
            &flat,
        )
        .unwrap();
    let overlay = chart.add_series(crate::SeriesKind::Candlestick);
    chart
        .set_series_data(
            overlay,
            &days(&[1.0, 2.0, 4.0, 5.0, 7.0, 8.0]),
            &[100.0, 100.0, 101.0, 105.0, 105.0, 105.0],
            &[101.0, 102.0, 120.0, 106.0, 106.0, 106.0],
            &[99.0, 99.0, 100.0, 104.0, 104.0, 104.0],
            &[100.0, 101.0, 105.0, 105.0, 105.0, 105.0],
        )
        .unwrap();
    chart.set_series_pane(overlay, 1, 1.0);
    chart
        .set_series_time_alignment(
            overlay,
            crate::TimeAlignment::AsOf {
                max_staleness: None,
            },
        )
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.build_frame();
    chart
}

#[test]
fn forecasts_and_bars_patterns_read_an_as_of_sources_own_bars() {
    let mut chart = as_of_overlay();
    let status = |chart: &ChartEngine, points: Vec<DrawingPoint>| {
        super::forecast_status(
            chart,
            &crate::Drawing::new(900, DrawingKind::Forecast, 1, points),
        )
    };
    // The d4 bar collapsed into the d5 point still reaches the target inside (d1, d5].
    assert_eq!(
        status(&chart, vec![p(0.0, 100.0), p(3.0, 106.5)]),
        Some(true)
    );
    // The only point in (d2, d3] repeats the source bar d2, which is not a later bar; later bars
    // exist, so the forecast failed.
    assert_eq!(
        status(&chart, vec![p(1.0, 101.0), p(2.0, 102.0)]),
        Some(false)
    );
    // Through the frame: the outcome box reads the same.
    let id = chart
        .add_drawing(
            DrawingKind::Forecast,
            1,
            vec![p(0.0, 100.0), p(3.0, 106.5)],
            None,
        )
        .unwrap();
    let frame = chart.build_frame();
    assert!(frame.panes[1]
        .main
        .iter()
        .any(|prim| matches!(prim, Prim::Text { text, .. } if text == "Success")));
    chart.remove_drawing(id);

    // A pattern over logical 0..=4 copies the overlay's own d1, d2, d4, d5, each once.
    let pattern = chart
        .add_drawing(
            DrawingKind::BarsPattern,
            1,
            vec![p(0.0, 90.0), p(4.0, 80.0)],
            None,
        )
        .unwrap();
    let copy = chart
        .drawing(pattern)
        .unwrap()
        .tool_options
        .projection_annotation
        .clone()
        .unwrap()
        .bars;
    assert_eq!(
        copy.iter().map(|bar| bar[1]).collect::<Vec<_>>(),
        [101.0, 102.0, 120.0, 106.0]
    );
    // Pinned on the copy's box: the first bar at logical 0, the last (d5) at its point, 3.
    let points = &chart.drawing(pattern).unwrap().points;
    assert_eq!((points[0].logical, points[1].logical), (0.0, 3.0));
}
