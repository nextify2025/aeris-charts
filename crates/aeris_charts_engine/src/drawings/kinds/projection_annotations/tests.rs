//! Projection & Annotations engine tests: catalog defaults, armed placement, shared-part frames
//! and hit testing (indexed and brute force), forecast outcomes, bars-pattern capture and mapping,
//! range stats, the projection sector, pane-anchored text, drags, keyboard nudges, magnet, time
//! identity, schema and kind options, patches with history, persistence, clipboard, and sync.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{LineStyle, Prim, TextAlign};

use super::super::super::{DrawingPlacement, DrawingTextHAlign, DrawingTextVAlign};
use crate::{
    ChartEngine, DrawingDragPart, DrawingId, DrawingKind, DrawingLabelMetric, DrawingLineCap,
    DrawingModifiers, DrawingPoint,
};

const KINDS: [DrawingKind; 19] = [
    DrawingKind::Forecast,
    DrawingKind::BarsPattern,
    DrawingKind::PriceRange,
    DrawingKind::DateRange,
    DrawingKind::DatePriceRange,
    DrawingKind::Projection,
    DrawingKind::AnchoredText,
    DrawingKind::Note,
    DrawingKind::PriceNote,
    DrawingKind::Callout,
    DrawingKind::Comment,
    DrawingKind::PriceLabel,
    DrawingKind::Signpost,
    DrawingKind::FlagMark,
    DrawingKind::ArrowMarkerUp,
    DrawingKind::ArrowMarkerDown,
    DrawingKind::ArrowMarkerLeft,
    DrawingKind::ArrowMarkerRight,
    DrawingKind::IconStamp,
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

/// The measuring ranges keep their own family defaults and identities (13..=15); the fork's
/// pre-merge defaults of the upstream-rendered tools, which documents it wrote omitted, come back
/// through `apply_legacy_fork_defaults`.
#[test]
fn catalog_defaults_follow_each_tool() {
    for kind in KINDS {
        let spec = kind.spec();
        let ranged = matches!(
            kind,
            DrawingKind::PriceRange | DrawingKind::DateRange | DrawingKind::DatePriceRange
        );
        assert_eq!(spec.family.is_some(), ranged, "{kind:?} renderer owner");
        if ranged {
            assert!((13..=15).contains(&spec.wire_id));
            assert!(!spec.axis_price_label);
            assert!(!spec.requests_text_editor);
            assert_eq!(spec.placement, DrawingPlacement::ClickAnchors { count: 2 });
        }
        // Only the measuring ranges snap their anchors to whole bars and price ticks.
        assert_eq!(spec.grid_snap, ranged, "{kind:?}");
        let mut drawing = crate::Drawing::new(1, kind, 0, Vec::new());
        crate::drawings::kinds::apply_legacy_fork_defaults(&mut drawing);
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
                DrawingKind::DatePriceRange => {
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
            DrawingKind::ArrowMarkerUp => aeris_charts_core::style::MARKET_UP_CSS,
            DrawingKind::ArrowMarkerDown => aeris_charts_core::style::MARKET_DOWN_CSS,
            _ => crate::DRAWING_DEFAULT_COLOR,
        };
        assert_eq!(drawing.color, color, "{kind:?}");
        assert!(drawing.tool_options.is_empty());
        assert_eq!(drawing.box_color, None, "{kind:?}");
    }
    let mut text = crate::Drawing::new(1, DrawingKind::AnchoredText, 0, Vec::new());
    crate::drawings::kinds::apply_legacy_fork_defaults(&mut text);
    assert_eq!(
        (text.text_h_align, text.text_v_align),
        (DrawingTextHAlign::Left, DrawingTextVAlign::Top)
    );
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
        // Upstream's text annotations paint only their text, which starts empty.
        let id = add(
            &mut chart,
            kind,
            points_for(kind),
            r##"{"color":"#123456","text":"t"}"##,
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

/// The forecast's evaluated outcome (the fork's `forecast_status`, re-applied on upstream's
/// forecast through `forecast_result`).
fn forecast_outcome(chart: &ChartEngine, id: DrawingId) -> Option<bool> {
    chart.forecast_result(chart.drawing(id).unwrap())
}

/// Upstream's forecast label: the move and the evaluated outcome.
fn forecast_label(chart: &mut ChartEngine) -> Option<String> {
    texts_of(chart)
        .into_iter()
        .find(|text| text.contains(" · "))
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
    assert_eq!(forecast_outcome(&chart, id), Some(true));
    assert_eq!(
        forecast_label(&mut chart).as_deref(),
        Some("+5.0% · target reached")
    );
    // A target no bar reaches fails once the data passes it.
    assert!(chart
        .set_drawing_anchors(id, &[p(10.0, 101.0).into(), p(20.0, 107.0).into()])
        .is_ok());
    assert_eq!(forecast_outcome(&chart, id), Some(false));
    assert!(forecast_label(&mut chart).unwrap().ends_with("expired"));
    // A target beyond the data is pending.
    assert!(chart
        .set_drawing_anchors(id, &[p(10.0, 101.0).into(), p(45.0, 107.0).into()])
        .is_ok());
    assert_eq!(forecast_outcome(&chart, id), None);
    assert!(forecast_label(&mut chart).unwrap().ends_with("pending"));
    // A target on the latest bar stays pending while that bar may still form; the next bar
    // decides the failure.
    assert!(chart
        .set_drawing_anchors(id, &[p(30.0, 101.0).into(), p(39.0, 107.0).into()])
        .is_ok());
    assert_eq!(forecast_outcome(&chart, id), None);
    assert!(chart.update_series_bar(0, 40.0 * HOUR, [100.0, 100.5, 99.5, 100.0]));
    assert_eq!(forecast_outcome(&chart, id), Some(false));
    assert!(forecast_label(&mut chart).unwrap().ends_with("expired"));
    // Falling targets test the lows; the line hits.
    assert!(chart
        .set_drawing_anchors(id, &[p(13.0, 106.0).into(), p(20.0, 100.0).into()])
        .is_ok());
    assert_eq!(forecast_outcome(&chart, id), Some(true));
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    assert_eq!(hit(&chart, (a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0), Some(id));
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
    // The shaft is one crisp device-pixel column through the area's middle and the arrow's apex
    // sits on that pixel's center at the second price's pixel row.
    assert!(
        close(
            arrowhead.0[0],
            (middle_x.round() + 0.5, b.1.round() + 0.5),
            1e-9
        ),
        "the arrow points at the second price: {:?}",
        arrowhead.0[0]
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
        DrawingKind::DatePriceRange,
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
fn range_arrows_are_crisp_device_pixel_shafts_ended_by_the_drawing_caps() {
    for dpr in [1.0, 1.25, 2.0] {
        let mut chart = chart_with(&hourly(40), dpr);
        add(
            &mut chart,
            DrawingKind::DatePriceRange,
            vec![p(10.0, 105.0), p(20.0, 100.0)],
            r##"{"color":"#123456"}"##,
        );
        // No antialiased stroke: both shafts are crisp full-pixel lines of the stroke width.
        assert!(ink_polylines(&mut chart).is_empty(), "dpr {dpr}");
        let width = dpr.round().max(1.0) as i32;
        let frame = chart.build_frame();
        let main = &frame.panes[0].main;
        let verticals = main
            .iter()
            .filter_map(|prim| match prim {
                Prim::VLine {
                    x, width: w, color, ..
                } if *color == ink() => Some((*x, *w)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let horizontals = main
            .iter()
            .filter_map(|prim| match prim {
                Prim::HLine {
                    y, width: w, color, ..
                } if *color == ink() => Some((*y, *w)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(verticals.len(), 1, "dpr {dpr}: the price arrow's shaft");
        assert_eq!(horizontals.len(), 1, "dpr {dpr}: the time arrow's shaft");
        assert_eq!((verticals[0].1, horizontals[0].1), (width, width));
        // Each cap's apex sits on its shaft's pixel center.
        let (x, w) = verticals[0];
        let shaft_x = f64::from(x - w / 2) + f64::from(w) / 2.0;
        let (y, w) = horizontals[0];
        let shaft_y = f64::from(y - w / 2) + f64::from(w) / 2.0;
        let apexes = fills(&mut chart)
            .into_iter()
            .filter(|(_, color)| *color == ink())
            .map(|(outline, _)| outline[0])
            .collect::<Vec<_>>();
        assert_eq!(apexes.len(), 2, "dpr {dpr}");
        assert!(
            apexes.iter().any(|apex| (apex.0 - shaft_x).abs() < 1e-9),
            "dpr {dpr}: the price arrow's apex is on its column {apexes:?} {shaft_x}"
        );
        assert!(
            apexes.iter().any(|apex| (apex.1 - shaft_y).abs() < 1e-9),
            "dpr {dpr}: the time arrow's apex is on its row {apexes:?} {shaft_y}"
        );
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
                .map(|point| p(point.logical + shift, point.price + shift * 0.3))
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
fn schema_kind_options_and_option_patches_are_typed_and_atomic() {
    let find = |schema: &crate::DrawingPropertySchema, name: &str| {
        schema
            .properties
            .iter()
            .find(|property| property.name == name)
            .unwrap_or_else(|| panic!("{name} descriptor"))
            .clone()
    };
    let range = crate::drawing_property_schema(DrawingKind::PriceRange);
    assert_eq!(find(&range, "fill_enabled").default, true);
    assert_eq!(find(&range, "stroke_end").default, "arrow");
    // Upstream-rendered tools list no fork option rows: their options are the flat fields.
    for kind in [
        DrawingKind::BarsPattern,
        DrawingKind::IconStamp,
        DrawingKind::Note,
        DrawingKind::Comment,
    ] {
        assert!(
            !crate::drawing_property_schema(kind)
                .properties
                .iter()
                .any(|property| property.name.starts_with("tool_options")),
            "{kind:?}"
        );
    }
    let mode = find(
        &crate::drawing_property_schema(DrawingKind::BarsPattern),
        "bars_pattern_mode",
    );
    assert!(mode.enum_values.iter().any(|value| value == "oc_bars"));
    // The ranges' kind options are the family's typed projection.
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::DateRange,
        points_for(DrawingKind::DateRange),
        "{}",
    );
    let kind_options: serde_json::Value =
        serde_json::from_str(&chart.drawing_kind_options_json(id).unwrap()).unwrap();
    assert_eq!(kind_options["kind"], "projection_annotation");
    // An invalid block rejects the whole patch.
    let before = chart.drawing(id).unwrap().clone();
    assert!(!chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"projection_annotation":{"icon_size":500}},"width":4}"#
    ));
    assert_eq!(chart.drawing(id).unwrap(), &before);
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

/// A grid-snapped tool's anchor after a move: on the bar slot and price tick nearest the raw
/// point, never further than half a slot or half a tick (0.01 on this scale) from it.
fn assert_snapped_to_the_grid(
    chart: &ChartEngine,
    now: DrawingPoint,
    raw_px: (f64, f64),
    context: &str,
) {
    let raw = chart
        .drawing_from_px_for(0, crate::DrawingPriceScale::Right, raw_px.0, raw_px.1)
        .unwrap();
    assert_eq!(now.logical, now.logical.round(), "{context}: whole bar");
    assert!(
        (now.logical - raw.logical).abs() <= 0.5 + 1e-9,
        "{context}: nearest slot ({} vs {})",
        now.logical,
        raw.logical
    );
    assert!(
        (now.price * 100.0 - (now.price * 100.0).round()).abs() < 1e-6,
        "{context}: price tick"
    );
    assert!(
        (now.price - raw.price).abs() <= 0.005 + 1e-9,
        "{context}: nearest tick ({} vs {})",
        now.price,
        raw.price
    );
}

#[test]
fn every_tool_drags_and_nudges_each_handle_and_its_body_as_single_history_entries() {
    // Upstream's text annotations edit like the text tool, without anchor handles.
    for kind in KINDS
        .into_iter()
        .filter(|kind| kind.spec().handles == crate::drawings::DrawingHandleMode::Anchors)
    {
        let mut chart = chart();
        let id = add(&mut chart, kind, points_for(kind), "{}");
        chart.set_selected_drawing(Some(id));
        chart.build_frame();
        let count = kind.anchor_count();
        assert_eq!(chart.drawing_handle_count(id), Some(count), "{kind:?}");
        let start = anchors_px(&chart, id);
        let start_points = chart.drawing(id).unwrap().points.clone();
        // Grid-snapped tools (the measuring ranges) land on whole bars and price ticks instead
        // of following the pointer pixel for pixel.
        let grid = kind.spec().grid_snap;
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
            if grid {
                let points = chart.drawing(id).unwrap().points.clone();
                for (index, (now, before)) in points.iter().zip(&start_points).enumerate() {
                    if index == handle {
                        assert_snapped_to_the_grid(
                            &chart,
                            *now,
                            (start[handle].0 + 13.0, start[handle].1 - 9.0),
                            &format!("{kind:?} handle {handle} drag"),
                        );
                    } else {
                        assert_eq!(now, before, "{kind:?} handle {handle} anchor {index}");
                    }
                }
            } else {
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
            }
            assert!(chart.undo_drawing(), "{kind:?}");
            // Keyboard nudge of the same handle.
            assert!(chart.nudge_selected_drawing(0.0, -10.0, Some(handle)));
            if grid {
                let points = chart.drawing(id).unwrap().points.clone();
                assert_snapped_to_the_grid(
                    &chart,
                    points[handle],
                    (start[handle].0, start[handle].1 - 10.0),
                    &format!("{kind:?} nudged handle {handle}"),
                );
                assert_eq!(points[handle].logical, start_points[handle].logical);
            } else {
                let nudged = anchor(&chart, id, handle);
                assert!(
                    close(nudged, (start[handle].0, start[handle].1 - 10.0), 1e-6),
                    "{kind:?} nudged handle {handle}"
                );
            }
            assert!(chart.undo_drawing());
            assert!(close(anchor(&chart, id, handle), start[handle], 1e-6));
        }
        // Body drag and body nudge translate every anchor rigidly (grid-snapped tools: by one
        // shared whole-bar step, each anchor's price on its own tick).
        let (x, y) = body_point(&chart, id);
        assert!(chart.drawing_drag_start_at(x, y), "{kind:?}");
        chart.drawing_drag_to(x + 17.0, y + 11.0, DrawingModifiers::default());
        chart.drawing_drag_end();
        if grid {
            let points = chart.drawing(id).unwrap().points.clone();
            let steps = points[0].logical - start_points[0].logical;
            assert_eq!(steps, steps.round(), "{kind:?} body drag whole bars");
            for (index, (now, before)) in points.iter().zip(&start_points).enumerate() {
                assert_eq!(now.logical - before.logical, steps, "{kind:?} body drag");
                let raw = chart
                    .drawing_from_px_for(
                        0,
                        crate::DrawingPriceScale::Right,
                        start[index].0 + 17.0,
                        start[index].1 + 11.0,
                    )
                    .unwrap();
                assert!(
                    (now.price - raw.price).abs() <= 0.005 + 1e-9
                        && (now.price * 100.0 - (now.price * 100.0).round()).abs() < 1e-6,
                    "{kind:?} body drag price tick"
                );
            }
        } else {
            for (now, before) in anchors_px(&chart, id).into_iter().zip(&start) {
                assert!(
                    close(now, (before.0 + 17.0, before.1 + 11.0), 1e-6),
                    "{kind:?} body drag"
                );
            }
        }
        assert!(chart.undo_drawing());
        assert!(chart.nudge_selected_drawing(5.0, 0.0, None));
        if grid {
            // A key step below one bar still moves the whole body by exactly one bar.
            let points = chart.drawing(id).unwrap().points.clone();
            for (now, before) in points.iter().zip(&start_points) {
                assert_eq!(now.logical, before.logical + 1.0, "{kind:?} body nudge");
                assert!(
                    (now.price - before.price).abs() < 1e-9,
                    "{kind:?} body nudge"
                );
            }
        } else {
            for (now, before) in anchors_px(&chart, id).into_iter().zip(&start) {
                assert!(
                    close(now, (before.0 + 5.0, before.1), 1e-6),
                    "{kind:?} body nudge"
                );
            }
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
        chart.build_frame();
        forecast_outcome(chart, id)
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
    assert_eq!(outcome(&mut chart), Some(false));
    assert!(forecast_label(&mut chart).unwrap().ends_with("expired"));
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
    // The painted label is the outcome the frame read.
    let outcome = |chart: &mut ChartEngine| forecast_label(chart).unwrap();
    assert!(outcome(&mut chart).ends_with("pending"));
    // A tick on that bar reaches the target without moving the time or price scales (the
    // chart's range stays 99.5..=106.5): the retained drawings layer still rebuilds.
    let revisions = chart.panes[0].scale_revisions();
    assert!(chart.update_series_bar(0, 39.0 * HOUR, [103.75, 105.25, 103.5, 105.0]));
    assert!(outcome(&mut chart).ends_with("target reached"));
    assert_eq!(forecast_outcome(&chart, id), Some(true));
    assert_eq!(chart.panes[0].scale_revisions(), revisions);
    assert!(chart.drawing(id).is_some());
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
        vec![p(10.0, 0.0), p(14.0, 0.0), p(16.0, 100.0)],
        r##"{"color":"#123456","bars_pattern_mode":"oc_bars","bars_pattern_mirror_y":true}"##,
    );
    let template = chart.drawing_template_json(source, "ghost").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&template).unwrap();
    assert!(
        parsed["options"].get("bars_pattern").is_none() && !template.contains("\"bars\""),
        "{template}"
    );
    // Applying the template restyles another pattern and keeps that pattern's own copy.
    let other = add(
        &mut chart,
        DrawingKind::BarsPattern,
        vec![p(20.0, 0.0), p(30.0, 0.0), p(32.0, 100.0)],
        "{}",
    );
    let own = chart.drawing(other).unwrap().bars_pattern.clone();
    assert_eq!(own.len(), 11);
    assert!(chart.apply_drawing_template_json(other, &template));
    let applied = chart.drawing(other).unwrap();
    assert_eq!(applied.bars_pattern, own);
    assert_eq!(applied.bars_pattern_mode, "oc_bars");
    assert!(applied.bars_pattern_mirror_y);
    assert_eq!(applied.color, INK);
    // A fork-shaped template's legacy block still restyles through the flat fields.
    let fork_template = r#"{"name":"fork","kind":"bars_pattern","options":{"tool_options":{"projection_annotation":{"bars_mode":"hl_bars","flipped":false}}}}"#;
    assert!(chart.apply_drawing_template_json(other, fork_template));
    let restyled = chart.drawing(other).unwrap();
    assert_eq!(restyled.bars_pattern_mode, "bars");
    assert!(!restyled.bars_pattern_mirror_y);
    assert_eq!(restyled.bars_pattern, own);
    // Creating from the template's options copies the new pattern's own range.
    let template: crate::DrawingTemplate = serde_json::from_str(&template).unwrap();
    let created = add(
        &mut chart,
        DrawingKind::BarsPattern,
        vec![p(25.0, 0.0), p(27.0, 0.0), p(29.0, 100.0)],
        &template.options.to_string(),
    );
    let copy = chart.drawing(created).unwrap();
    assert_eq!(copy.bars_pattern.len(), 3);
    assert_eq!(copy.bars_pattern[0].close, value_at(25));
    assert_eq!(copy.bars_pattern_mode, "oc_bars");
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
    // Every tool that paints text opens on its empty text: upstream-rendered tools as one-line
    // runs, the simple annotation's family box over several lines.
    for kind in KINDS.into_iter().chain([DrawingKind::SimpleAnnotation]) {
        let id = add(&mut chart, kind, points_for(kind), "{}");
        let expected = !TEXTLESS.contains(&kind);
        assert_eq!(chart.drawing_text_editable(id), expected, "{kind:?}");
        assert_eq!(
            chart
                .drawing_text_edit_layout(id)
                .map(|layout| layout.multiline),
            expected.then_some(kind == DrawingKind::SimpleAnnotation),
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

/// Valid anchors for any catalog tool inside the fixture chart's data: the tool's minimum anchor
/// count on a gentle zigzag.
fn catalog_points(kind: DrawingKind) -> Vec<DrawingPoint> {
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
/// never painted, so there is nothing to edit in place. Every upstream-rendered tool paints its
/// text as a generic run, so only the measuring ranges and the simple tag (whose text is its
/// price-axis tag) are left.
const TEXTLESS: [DrawingKind; 4] = [
    DrawingKind::PriceRange,
    DrawingKind::DateRange,
    DrawingKind::DatePriceRange,
    DrawingKind::SimpleTag,
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
    assert_eq!(crate::drawings::DRAWING_TOOL_SPECS.len(), 92);
    assert_eq!(editable_tools, 88);
}

#[test]
fn the_edit_layout_is_the_painted_text_box() {
    let mut chart = chart();
    let size = chart.drawing_text_size(&crate::Drawing::new(
        1,
        DrawingKind::SimpleAnnotation,
        0,
        Vec::new(),
    ));
    // The family box (the simple annotation's) holds only its text, one line per line.
    let comment = add(
        &mut chart,
        DrawingKind::SimpleAnnotation,
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
    // The simple annotation's family box edits several lines (upstream's comment edits one run).
    let comment = add(
        &mut chart,
        DrawingKind::SimpleAnnotation,
        vec![p(15.0, 103.0)],
        "{}",
    );
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
    // The family box (upstream removes an emptied comment, see `drawing_text_edit`).
    let comment = add(
        &mut chart,
        DrawingKind::SimpleAnnotation,
        vec![p(15.0, 103.0)],
        r#"{"text":"Note"}"#,
    );
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
        "an empty annotation paints no box"
    );
    assert_eq!(
        chart
            .drawing_text_edit_layout(comment)
            .map(|edit| edit.rect),
        Some(layout.rect),
        "the editor reopens on the same caret line"
    );
    assert!(chart.drawing_text_editable(comment));
}

/// The overlay's bars between logical 0 and 4 as (axis offset, high): d1, d2, and d4 collapsed
/// with d5 onto d5's point (merged into one bar: the 120 high of d4, d5's close).
const BARS_PATTERN_AS_OF_COPY: [(u16, f64); 3] = [(0, 101.0), (1, 102.0), (3, 120.0)];

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
    // Through the frame: the outcome label reads the same.
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
        .any(|prim| matches!(prim, Prim::Text { text, .. } if text.ends_with("target reached"))));
    chart.remove_drawing(id);

    // A pattern over logical 0..=4 copies the overlay's own bars at their axis offsets (upstream's
    // frozen snapshot, read through the shared source window).
    let pattern = chart
        .add_drawing(
            DrawingKind::BarsPattern,
            1,
            vec![p(0.0, 90.0), p(4.0, 80.0), p(6.0, 85.0)],
            None,
        )
        .unwrap();
    let copy = &chart.drawing(pattern).unwrap().bars_pattern;
    assert_eq!(
        copy.iter()
            .map(|bar| (bar.offset, bar.high))
            .collect::<Vec<_>>(),
        BARS_PATTERN_AS_OF_COPY
    );
    assert_eq!(
        chart.drawing(pattern).unwrap().bars_pattern[2].close,
        105.0,
        "the later collapsed row closes the slot"
    );
}

/// Texts of the price-axis tags (labels with a background) in the axis frame.
fn axis_tags(chart: &mut ChartEngine) -> Vec<String> {
    chart.axis_w = 80.0;
    chart.build_frame();
    chart
        .build_axis_frame(
            80.0,
            |text, _bold| text.len() as f64 * 7.0,
            |text, _bold| text.len() as f64 * 6.0,
        )
        .labels
        .into_iter()
        .filter(|label| label.background.is_some())
        .map(|label| label.text)
        .collect()
}

#[test]
fn simple_tags_tag_the_price_axis_with_their_text_or_price_and_paint_no_chart_text() {
    let kind = DrawingKind::SimpleTag;
    let spec = kind.spec();
    assert_eq!((spec.wire_id, spec.name), (245, "simple_tag"));
    assert_eq!(DrawingKind::from_u8(245), Some(kind));
    assert_eq!(DrawingKind::from_name("simple_tag"), Some(kind));
    assert_eq!(spec.placement, DrawingPlacement::ClickAnchors { count: 1 });
    assert!(spec.axis_price_label && spec.axis_tag_text);
    assert_eq!(
        crate::Drawing::new(1, kind, 0, Vec::new()).style,
        LineStyle::Dashed
    );

    let mut chart = chart();
    let id = add(
        &mut chart,
        kind,
        vec![p(4.0, 103.0)],
        r##"{"color":"#123456"}"##,
    );
    let a = anchor(&chart, id, 0);
    let pane_w = chart.pane_w;
    // A dashed crisp line across the whole pane at the anchor's price.
    let frame = chart.build_frame();
    let hlines = frame.panes[0]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::HLine {
                y,
                x0,
                x1,
                style,
                color,
                ..
            } if *color == ink() => Some((*y, *x0, *x1, *style)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(hlines.len(), 1, "{hlines:?}");
    let (y, x0, x1, style) = hlines[0];
    assert!((f64::from(y) - a.1).abs() <= 1.0, "{hlines:?}");
    assert_eq!((x0, x1), (0, pane_w.round() as i32), "across the pane");
    assert_eq!(style, LineStyle::Dashed);
    // Without text the axis tag is the price.
    assert!(axis_tags(&mut chart).contains(&"103.00".to_string()));
    // With text the tag shows the text instead, and nothing is painted on the chart: there is no
    // text to edit in place.
    assert!(chart.drawing_apply_options(id, r#"{"text":"Support"}"#));
    let tags = axis_tags(&mut chart);
    assert!(tags.contains(&"Support".to_string()), "{tags:?}");
    assert!(!tags.contains(&"103.00".to_string()), "{tags:?}");
    assert!(!texts_of(&mut chart).contains(&"Support".to_string()));
    assert!(!chart.drawing_text_editable(id));
    // Hit across the pane, far from the anchor.
    assert_eq!(hit(&chart, pane_w - 20.0, a.1), Some(id));
}

#[test]
fn simple_annotations_stand_a_dashed_stem_with_a_head_under_a_boxed_text() {
    let kind = DrawingKind::SimpleAnnotation;
    let spec = kind.spec();
    assert_eq!((spec.wire_id, spec.name), (246, "simple_annotation"));
    assert_eq!(DrawingKind::from_u8(246), Some(kind));
    assert_eq!(DrawingKind::from_name("simple_annotation"), Some(kind));
    assert!(spec.requests_text_editor, "placing it opens the editor");
    let defaults = crate::Drawing::new(1, kind, 0, Vec::new());
    assert_eq!(defaults.style, LineStyle::Dashed);
    assert!(defaults.text.is_empty(), "the user supplies the text");

    let mut chart = chart();
    let id = add(
        &mut chart,
        kind,
        vec![p(6.0, 102.0)],
        r##"{"color":"#123456","text":"Earnings"}"##,
    );
    let a = anchor(&chart, id, 0);
    // KLineChart's geometry at device scale 1: the stem runs 6 px above the anchor to 56 px above
    // it, the head's apex sits on the stem's top, 5 px tall and 8 px wide, and the box sits above.
    let stems = ink_vlines(&mut chart);
    assert_eq!(stems.len(), 1, "{stems:?}");
    let (x, y0, y1) = stems[0];
    assert!((f64::from(x) - a.0).abs() <= 1.0);
    assert!(
        (f64::from(y1) - (a.1 - 6.0)).abs() <= 1.0 && (f64::from(y0) - (a.1 - 56.0)).abs() <= 1.0,
        "{stems:?}"
    );
    let heads = fills(&mut chart)
        .into_iter()
        .filter(|(_, color)| *color == ink())
        .collect::<Vec<_>>();
    assert_eq!(heads.len(), 1, "{heads:?}");
    let head = &heads[0].0;
    let top = head
        .iter()
        .map(|point| point.1)
        .fold(f64::INFINITY, f64::min);
    let bottom = head
        .iter()
        .map(|point| point.1)
        .fold(f64::NEG_INFINITY, f64::max);
    let (left, right) = (
        head.iter()
            .map(|point| point.0)
            .fold(f64::INFINITY, f64::min),
        head.iter()
            .map(|point| point.0)
            .fold(f64::NEG_INFINITY, f64::max),
    );
    assert!(
        (bottom - (a.1 - 56.0)).abs() <= 1.0 && (top - (a.1 - 61.0)).abs() <= 1.0,
        "{head:?}"
    );
    assert!(((right - left) - 8.0).abs() <= 1.0, "{head:?}");
    // The text is above the head.
    let runs = texts(&mut chart);
    let (_, _, text_y) = runs
        .iter()
        .find(|(text, ..)| text == "Earnings")
        .expect("the text");
    assert!(f64::from(*text_y) < a.1 - 61.0, "{runs:?}");
    // Without text only the stem and head remain.
    assert!(chart.drawing_apply_options(id, r#"{"text":""}"#));
    assert!(!texts_of(&mut chart).contains(&"Earnings".to_string()));
    assert_eq!(ink_vlines(&mut chart).len(), 1);
    // The stem is a body target.
    assert_eq!(hit(&chart, a.0, a.1 - 30.0), Some(id));
    assert_eq!(hit(&chart, a.0 + 60.0, a.1 - 30.0), None);
}
