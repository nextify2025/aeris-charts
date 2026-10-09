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
        assert_eq!(spec.price_tick_snap, ranged, "{kind:?}");
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
    assert!(
        frame.panes[0]
            .points
            .iter()
            .all(|point| point[0].is_finite() && point[1].is_finite())
    );
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
    assert!(
        chart
            .set_drawing_anchors(id, &[p(10.0, 101.0).into(), p(20.0, 107.0).into()])
            .is_ok()
    );
    assert_eq!(forecast_outcome(&chart, id), Some(false));
    assert!(forecast_label(&mut chart).unwrap().ends_with("expired"));
    // A target beyond the data is pending.
    assert!(
        chart
            .set_drawing_anchors(id, &[p(10.0, 101.0).into(), p(45.0, 107.0).into()])
            .is_ok()
    );
    assert_eq!(forecast_outcome(&chart, id), None);
    assert!(forecast_label(&mut chart).unwrap().ends_with("pending"));
    // A target on the latest bar stays pending while that bar may still form; the next bar
    // decides the failure.
    assert!(
        chart
            .set_drawing_anchors(id, &[p(30.0, 101.0).into(), p(39.0, 107.0).into()])
            .is_ok()
    );
    assert_eq!(forecast_outcome(&chart, id), None);
    assert!(chart.update_series_bar(0, 40.0 * HOUR, [100.0, 100.5, 99.5, 100.0]));
    assert_eq!(forecast_outcome(&chart, id), Some(false));
    assert!(forecast_label(&mut chart).unwrap().ends_with("expired"));
    // Falling targets test the lows; the line hits.
    assert!(
        chart
            .set_drawing_anchors(id, &[p(13.0, 106.0).into(), p(20.0, 100.0).into()])
            .is_ok()
    );
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
    assert!(
        !texts_of(&mut chart)
            .iter()
            .any(|text| text.contains("bars"))
    );
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
    assert!(
        frame.panes[pane]
            .main
            .iter()
            .any(|prim| matches!(prim, Prim::Text { text, .. } if text == "10 bars  10h"))
    );
    assert!(
        !frame.panes[0]
            .main
            .iter()
            .any(|prim| matches!(prim, Prim::Text { text, .. } if text.contains("bars")))
    );
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

/// `now` sits on the bar slot nearest the raw pointer position `raw_px`: a whole bar, at the raw
/// price, or on the nearest price tick for the tick-snapped tools (the measuring ranges).
fn assert_on_the_slot(
    chart: &ChartEngine,
    now: DrawingPoint,
    raw_px: (f64, f64),
    tick: bool,
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
    if !tick {
        assert!(
            (now.price - raw.price).abs() < 1e-9,
            "{context}: raw price ({} vs {})",
            now.price,
            raw.price
        );
        return;
    }
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
        // Every tool but the screen-pinned anchored text lands on whole bars; the measuring
        // ranges also land on price ticks, the rest keep the raw price.
        let bars = kind != DrawingKind::AnchoredText;
        let tick = kind.spec().price_tick_snap;
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
            if bars {
                let points = chart.drawing(id).unwrap().points.clone();
                for (index, (now, before)) in points.iter().zip(&start_points).enumerate() {
                    if index == handle {
                        assert_on_the_slot(
                            &chart,
                            *now,
                            (start[handle].0 + 13.0, start[handle].1 - 9.0),
                            tick,
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
            // Keyboard nudge of the same handle: vertical, so the anchor keeps its bar.
            assert!(chart.nudge_selected_drawing(0.0, -10.0, Some(handle)));
            if tick {
                let points = chart.drawing(id).unwrap().points.clone();
                assert_on_the_slot(
                    &chart,
                    points[handle],
                    (start[handle].0, start[handle].1 - 10.0),
                    tick,
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
        // Body drag and body nudge translate every anchor rigidly (by one shared whole-bar
        // step; the measuring ranges put each anchor's price on its own tick).
        let (x, y) = body_point(&chart, id);
        assert!(chart.drawing_drag_start_at(x, y), "{kind:?}");
        chart.drawing_drag_to(x + 17.0, y + 11.0, DrawingModifiers::default());
        chart.drawing_drag_end();
        if bars {
            let points = chart.drawing(id).unwrap().points.clone();
            let steps = points[0].logical - start_points[0].logical;
            assert_eq!(steps, steps.round(), "{kind:?} body drag whole bars");
            // The body follows the slot changes under the grabbed point.
            let slot = |x: f64| chart.coordinate_to_logical(x).unwrap();
            assert_eq!(steps, slot(x + 17.0) - slot(x), "{kind:?} body drag steps");
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
                if tick {
                    assert!(
                        (now.price - raw.price).abs() <= 0.005 + 1e-9
                            && (now.price * 100.0 - (now.price * 100.0).round()).abs() < 1e-6,
                        "{kind:?} body drag price tick"
                    );
                } else {
                    assert!(
                        (now.price - raw.price).abs() < 1e-9,
                        "{kind:?} body drag raw price"
                    );
                }
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
        if bars {
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
    assert!(
        chart
            .set_drawing_anchors(id, &[p(10.0, 101.0).into(), p(30.0, 107.0).into()])
            .is_ok()
    );
    assert_eq!(outcome(&mut chart), None);
    // The first traded bar after the target decides the failure.
    assert!(
        chart
            .set_drawing_anchors(id, &[p(10.0, 101.0).into(), p(20.0, 107.0).into()])
            .is_ok()
    );
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

/// The bars pattern's source window (its first two anchors) shows as a dashed outline while the
/// pattern is placed, from its first anchor to the pointer on, and while it is selected; an
/// unselected pattern paints only its copied bars.
#[test]
fn a_bars_patterns_source_window_outlines_in_its_preview_and_selection_only() {
    // The dashed ink rules of the first pane as one (left, top, right, bottom) box, if any.
    let outline = |chart: &mut ChartEngine| {
        let ink = Color::parse_css(INK).unwrap();
        let frame = chart.build_frame();
        let (mut rows, mut columns) = (Vec::new(), Vec::new());
        for prim in &frame.panes[0].main {
            match *prim {
                Prim::HLine {
                    y,
                    x0,
                    x1,
                    style: LineStyle::Dashed,
                    color,
                    ..
                } if color == ink => rows.push((y, x0, x1)),
                Prim::VLine {
                    x,
                    y0,
                    y1,
                    style: LineStyle::Dashed,
                    color,
                    ..
                } if color == ink => columns.push((x, y0, y1)),
                _ => {}
            }
        }
        if rows.is_empty() && columns.is_empty() {
            return None;
        }
        assert_eq!((rows.len(), columns.len()), (2, 2), "{rows:?} {columns:?}");
        let (left, right) = (columns[0].0, columns[1].0);
        let (top, bottom) = (rows[0].0, rows[1].0);
        assert!(rows.iter().all(|&(_, x0, x1)| (x0, x1) == (left, right)));
        assert!(columns.iter().all(|&(_, y0, y1)| (y0, y1) == (top, bottom)));
        Some((left, top, right, bottom))
    };
    let window = |a: (f64, f64), b: (f64, f64)| {
        Some((
            a.0.min(b.0).round() as i32,
            a.1.min(b.1).round() as i32,
            a.0.max(b.0).round() as i32,
            a.1.max(b.1).round() as i32,
        ))
    };
    let mut chart = chart();
    let slot = |chart: &ChartEngine, (x, y): (f64, f64)| {
        let logical = chart.coordinate_to_logical(x).unwrap();
        (chart.logical_to_coordinate(logical).unwrap(), y)
    };
    let modifiers = DrawingModifiers::default();
    assert!(chart.set_drawing_tool(
        Some(DrawingKind::BarsPattern),
        Some(r##"{"color":"#123456"}"##),
        None
    ));
    let (a, b, c) = ((200.0, 300.0), (330.0, 260.0), (520.0, 200.0));
    assert_eq!(outline(&mut chart), None, "nothing placed");
    chart.drawing_tool_activate(a.0, a.1, modifiers);
    chart.drawing_tool_pointer_move(b.0, b.1, modifiers, false);
    assert_eq!(
        outline(&mut chart),
        window(slot(&chart, a), slot(&chart, b)),
        "the window follows the pointer"
    );
    chart.drawing_tool_activate(b.0, b.1, modifiers);
    let id = chart
        .drawing_tool_activate(c.0, c.1, modifiers)
        .created
        .expect("committed");
    assert_eq!(chart.selected_drawing(), Some(id));
    assert!(!chart.drawing(id).unwrap().bars_pattern.is_empty());
    let source = window(anchor(&chart, id, 0), anchor(&chart, id, 1));
    assert_eq!(outline(&mut chart), source, "selected");
    chart.set_selected_drawing(None);
    assert_eq!(outline(&mut chart), None, "unselected");
    chart.set_selected_drawing(Some(id));
    assert_eq!(outline(&mut chart), source);
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
    // runs, the text annotations' blocks and the simple annotation's family box over several
    // lines.
    for kind in KINDS.into_iter().chain([DrawingKind::SimpleAnnotation]) {
        let id = add(&mut chart, kind, points_for(kind), "{}");
        let expected = !TEXTLESS.contains(&kind);
        assert_eq!(chart.drawing_text_editable(id), expected, "{kind:?}");
        assert_eq!(
            chart
                .drawing_text_edit_layout(id)
                .map(|layout| layout.multiline),
            expected.then_some(kind == DrawingKind::SimpleAnnotation || kind.is_text_annotation()),
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
            // Family boxes and the text annotations' blocks take several lines; every other
            // drawing edits one rotated or level run, which the engine keeps on one line.
            assert_eq!(
                layout.multiline,
                spec.family.is_some_and(|family| family.owns_text) || kind.is_text_annotation(),
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
    assert!(
        frame.panes[1].main.iter().any(
            |prim| matches!(prim, Prim::Text { text, .. } if text.ends_with("target reached"))
        )
    );
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
// --- R9: the fork form (owner decisions A1-A10) and the multi-line text owner ----------------

/// The fork-form marker (`tool_options.projection_annotation` present, written `{}`).
const FORK: &str = r#""tool_options":{"projection_annotation":{}}"#;

/// `options` (a JSON object body without braces) with the fork-form marker.
fn fork(options: &str) -> String {
    if options.is_empty() {
        format!("{{{FORK}}}")
    } else {
        format!("{{{options},{FORK}}}")
    }
}

/// One `BandFill`: its upper chain, its lower chain, and its fill.
type BandFill = (Vec<(f64, f64)>, Vec<(f64, f64)>, Color);

/// Every `BandFill` of the first pane.
fn band_fills(chart: &mut ChartEngine) -> Vec<BandFill> {
    let frame = chart.build_frame();
    let pane = &frame.panes[0];
    let at = |index: u32| {
        let point = pane.points[index as usize];
        (f64::from(point[0]), f64::from(point[1]))
    };
    pane.main
        .iter()
        .filter_map(|prim| match prim {
            Prim::BandFill {
                upper_first,
                lower_first,
                point_count,
                fill,
                ..
            } => Some((
                (0..*point_count).map(|i| at(upper_first + i)).collect(),
                (0..*point_count).map(|i| at(lower_first + i)).collect(),
                *fill,
            )),
            _ => None,
        })
        .collect()
}

/// The first pane's `Rect` prims as `(left, top, right, bottom, color)`.
fn rects(chart: &mut ChartEngine) -> Vec<(f64, f64, f64, f64, Color)> {
    let frame = chart.build_frame();
    frame.panes[0]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::Rect { rect, color } => Some((
                f64::from(rect.x),
                f64::from(rect.y),
                f64::from(rect.x + rect.w),
                f64::from(rect.y + rect.h),
                *color,
            )),
            _ => None,
        })
        .collect()
}

fn with_alpha(color: Color, alpha: u8) -> Color {
    Color::rgba(color.r(), color.g(), color.b(), alpha)
}

fn run_of<'a>(
    runs: &'a [(String, f64, f64, Color)],
    wanted: &str,
) -> &'a (String, f64, f64, Color) {
    runs.iter()
        .find(|(text, ..)| text == wanted)
        .unwrap_or_else(|| panic!("{wanted:?} is painted: {runs:?}"))
}

#[test]
fn fork_form_projection_fills_the_sector_and_upstream_keeps_its_triangle() {
    let mut chart = chart();
    let options = fork(&format!(r##""color":"{INK}""##));
    let id = add(
        &mut chart,
        DrawingKind::Projection,
        vec![p(10.0, 101.0), p(20.0, 105.0)],
        &options,
    );
    let (pivot, target) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let radius = (target.0 - pivot.0).hypot(target.1 - pivot.1);
    // The fan from the pivot at the fill alpha, under the outline pivot → arc → pivot.
    let fills = band_fills(&mut chart);
    assert_eq!(fills.len(), 1, "one sector fill");
    let (arc, hub, fill) = &fills[0];
    assert_eq!(*fill, with_alpha(ink(), 51));
    assert!(hub.iter().all(|&point| close(point, pivot, 0.01)));
    assert!(
        arc.iter()
            .all(|&(x, y)| ((x - pivot.0).hypot(y - pivot.1) - radius).abs() < 0.5)
    );
    // From the horizontal ray on the target's side to the target.
    assert!(close(arc[0], (pivot.0 + radius, pivot.1), 0.5));
    assert!(close(*arc.last().unwrap(), target, 0.5));
    let outline = ink_polylines(&mut chart);
    assert_eq!(outline.len(), 1);
    let outline = &outline[0].0;
    assert!(close(outline[0], pivot, 0.01) && close(*outline.last().unwrap(), pivot, 0.01));
    assert!(
        outline[1..outline.len() - 1]
            .iter()
            .all(|&(x, y)| ((x - pivot.0).hypot(y - pivot.1) - radius).abs() < 0.5)
    );
    // The arc hits; the interior only while selected; past the arc never.
    let bisector = (target.1 - pivot.1).atan2(target.0 - pivot.0) / 2.0;
    let along = |distance: f64| {
        (
            pivot.0 + distance * bisector.cos(),
            pivot.1 + distance * bisector.sin(),
        )
    };
    let (on_arc, inside, outside) = (along(radius), along(radius * 0.5), along(radius * 1.2));
    assert_eq!(hit(&chart, on_arc.0, on_arc.1), Some(id));
    assert_eq!(hit(&chart, inside.0, inside.1), None);
    chart.set_selected_drawing(Some(id));
    assert_eq!(hit(&chart, inside.0, inside.1), Some(id));
    assert_eq!(hit(&chart, outside.0, outside.1), None);
    // Without the marker the projection is upstream's triangle, prims unchanged.
    let upstream = add(
        &mut chart,
        DrawingKind::Projection,
        vec![p(10.0, 101.0), p(20.0, 105.0)],
        &format!(r##"{{"color":"{INK}"}}"##),
    );
    chart.remove_drawing(id);
    let frame = chart.build_frame();
    assert!(frame.panes[0].main.iter().any(
        |prim| matches!(prim, Prim::Triangle { color, .. } if *color == with_alpha(ink(), 51))
    ));
    assert!(band_fills(&mut chart).is_empty());
    assert_eq!(hit(&chart, on_arc.0, on_arc.1), None, "{upstream}");
}

#[test]
fn a_fork_form_projection_whose_arc_reaches_into_the_pane_is_a_candidate() {
    let mut chart = chart();
    // Both anchors left of the pane; only the arc's bulge past the target's time reaches in.
    let id = add(
        &mut chart,
        DrawingKind::Projection,
        vec![p(-10.0, 101.0), p(-4.0, 104.5)],
        &fork(""),
    );
    let (pivot, target) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let radius = (target.0 - pivot.0).hypot(target.1 - pivot.1);
    // The target lies well left of the pane (its time bounds alone would cull the drawing).
    assert!(
        target.0 < -60.0 && pivot.0 + radius > 20.0,
        "{pivot:?} {target:?}"
    );
    let sweep = (target.1 - pivot.1).atan2(target.0 - pivot.0);
    let angle = sweep * 0.05;
    let on_arc = (
        pivot.0 + radius * angle.cos(),
        pivot.1 + radius * angle.sin(),
    );
    assert!(on_arc.0 > 0.0);
    // The arc hits unselected; the interior in the pane only once selected.
    assert_eq!(hit(&chart, on_arc.0, on_arc.1), Some(id));
    assert_eq!(
        chart
            .hit_test_drawing_bruteforce(on_arc.0, on_arc.1)
            .map(|hit| hit.id),
        Some(id)
    );
    let inside = (on_arc.0 - 8.0, on_arc.1 - 2.0);
    assert_eq!(hit(&chart, inside.0, inside.1), None);
    chart.set_selected_drawing(Some(id));
    assert_eq!(hit(&chart, inside.0, inside.1), Some(id));
    assert!(chart.drawing_viewport_candidate_reference(chart.drawing(id).unwrap()));
}

#[test]
fn a_fork_form_projection_paints_its_stats_box_beside_the_target() {
    let mut chart = chart();
    let labels = r#""labels":[{"metric":"price_change","visible":true,"position":"on"}]"#;
    let id = add(
        &mut chart,
        DrawingKind::Projection,
        vec![p(10.0, 101.0), p(20.0, 105.0)],
        &fork(labels),
    );
    let target = anchor(&chart, id, 1);
    let runs = text_runs(&mut chart);
    assert_eq!(runs.len(), 1, "one stats line, no generic label: {runs:?}");
    let (_, x, y, _) = &runs[0];
    assert!(
        *x >= target.0 + 8.0 && (y - target.1).abs() < 1.0,
        "{runs:?}"
    );
    // A stats box on the stroke color at the stats alpha, and a body target.
    let stroke = Color::parse_css(crate::DRAWING_DEFAULT_COLOR).unwrap();
    assert!(
        rects(&mut chart)
            .iter()
            .any(|rect| rect.4 == with_alpha(stroke, crate::drawings::parts::STATS_ALPHA))
    );
    assert_eq!(hit(&chart, x + 2.0, *y), Some(id));
    // Upstream form keeps the generic label pass.
    chart.remove_drawing(id);
    add(
        &mut chart,
        DrawingKind::Projection,
        vec![p(10.0, 101.0), p(20.0, 105.0)],
        &format!("{{{labels}}}"),
    );
    let upstream = text_runs(&mut chart);
    assert_eq!(upstream.len(), 1);
    assert_ne!((upstream[0].1, upstream[0].2), (*x, *y));
}

#[test]
fn fork_form_notes_stand_a_pin_and_reveal_their_box_on_focus() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::Note,
        vec![p(15.0, 103.0)],
        &fork(&format!(r##""text":"Hello","color":"{INK}""##)),
    );
    let tip = anchor(&chart, id, 0);
    // The pin in the stroke color, its tip on the anchor, with a contrasting dot in its head.
    let fills = band_fills(&mut chart);
    assert_eq!(fills.len(), 1);
    assert!(fills[0].2 == ink());
    let frame = chart.build_frame();
    let dot = frame.panes[0].main.iter().find_map(|prim| match prim {
        Prim::Circle {
            cx,
            cy,
            radius,
            fill,
            ..
        } => Some((f64::from(*cx), f64::from(*cy), f64::from(*radius), *fill)),
        _ => None,
    });
    let (cx, cy, radius, fill) = dot.expect("the head's dot");
    assert!(close((cx, cy), (tip.0, tip.1 - 17.0), 1e-3) && (radius - 2.5).abs() < 1e-6);
    assert_eq!(fill, Color::rgb(255, 255, 255));
    // Unfocused, the box is hidden and no target.
    assert!(!texts_of(&mut chart).contains(&"Hello".to_string()));
    assert_eq!(hit(&chart, tip.0, tip.1 - 17.0), Some(id));
    assert_eq!(hit(&chart, tip.0, tip.1 - 8.0), Some(id));
    assert_eq!(hit(&chart, tip.0 + 30.0, tip.1 - 17.0), None);
    // Hover and selection show it beside the head; the box is then a target.
    chart.build_frame();
    chart.set_hovered_drawing(Some(id));
    let runs = text_runs(&mut chart);
    assert_eq!(
        chart.frame_build_stats().drawing_rebuilds,
        1,
        "gaining focus rebuilds the drawing layer"
    );
    let (_, x, y, color) = run_of(&runs, "Hello");
    assert!((x - (tip.0 + 7.0 + 4.0 + 8.0)).abs() < 1e-3 && (y - (tip.1 - 17.0)).abs() < 1e-3);
    assert_eq!(*color, Color::rgb(255, 255, 255));
    assert_eq!(hit(&chart, tip.0 + 30.0, tip.1 - 17.0), Some(id));
    chart.set_hovered_drawing(None);
    assert!(!texts_of(&mut chart).contains(&"Hello".to_string()));
    chart.set_selected_drawing(Some(id));
    assert!(texts_of(&mut chart).contains(&"Hello".to_string()));
    // The selected note's focus frame encloses its box.
    let (left, top, right, bottom, _) = rects(&mut chart)
        .into_iter()
        .find(|rect| rect.4 == ink())
        .expect("the note's box");
    let frame = chart.build_frame();
    assert!(frame.panes[0].main.iter().any(|prim| matches!(
        prim,
        Prim::RectFrame { rect, .. }
            if f64::from(rect.x) < left
                && f64::from(rect.y) < top
                && f64::from(rect.x + rect.w) > right
                && f64::from(rect.y + rect.h) > bottom
    )));
    let layout = chart.drawing_text_edit_layout(id).unwrap();
    assert!(layout.multiline);
    assert!((layout.x - x).abs() < 1e-3 && (layout.y - y).abs() < 1e-3);
    chart.set_selected_drawing(None);
    // `always_show_text` shows it unfocused, and round-trips.
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"projection_annotation":{"always_show_text":true}}}"#
    ));
    assert!(texts_of(&mut chart).contains(&"Hello".to_string()));
    let exported = chart.export_state_json().unwrap();
    let mut again = chart_with(&hourly(40), 1.0);
    again.import_state_json(&exported).unwrap();
    assert!(
        again
            .drawing(id)
            .unwrap()
            .tool_options
            .projection_annotation
            .as_ref()
            .unwrap()
            .always_show_text
    );
    // Upstream's note keeps its tinted box and rebuilds nothing on hover.
    let upstream = add(
        &mut chart,
        DrawingKind::Note,
        vec![p(25.0, 103.0)],
        r#"{"text":"Up"}"#,
    );
    chart.build_frame();
    chart.set_hovered_drawing(Some(upstream));
    chart.build_frame();
    assert_eq!(chart.frame_build_stats().drawing_rebuilds, 0);
    assert!(
        rects(&mut chart)
            .iter()
            .any(|rect| rect.4 == Color::parse_css("#facc1533").unwrap())
    );
}

#[test]
fn fork_form_comments_and_price_labels_are_speech_bubbles_at_their_anchor() {
    let mut chart = chart();
    let comment = add(
        &mut chart,
        DrawingKind::Comment,
        vec![p(15.0, 103.0)],
        &fork(&format!(r##""text":"Hi","color":"{INK}""##)),
    );
    let tip = anchor(&chart, comment, 0);
    // The tail in the stroke color, its tip on the anchor; the bubble above and to the right.
    let fills = band_fills(&mut chart);
    assert_eq!(fills.len(), 1);
    let (upper, lower, color) = &fills[0];
    assert_eq!(*color, ink());
    assert!(
        upper
            .iter()
            .chain(lower)
            .any(|&point| close(point, tip, 1e-3))
    );
    let runs = text_runs(&mut chart);
    let (_, x, y, color) = run_of(&runs, "Hi");
    assert!(*x >= tip.0 + 8.0 - 1e-3 && *y < tip.1 - 10.0, "{runs:?}");
    assert_eq!(*color, Color::rgb(255, 255, 255));
    assert!(rects(&mut chart).iter().any(|rect| rect.4 == ink()));
    assert_eq!(hit(&chart, tip.0 + 1.0, tip.1 - 3.0), Some(comment));
    assert_eq!(hit(&chart, x + 2.0, *y), Some(comment));
    // A comment of two lines paints two.
    assert!(chart.drawing_apply_options(comment, r#"{"text":"a\nb"}"#));
    let texts = texts_of(&mut chart);
    assert!(texts.contains(&"a".to_string()) && texts.contains(&"b".to_string()));
    chart.remove_drawing(comment);

    // A price label's bubble starts with its price, at the anchor rather than the axis edge.
    let label = add(
        &mut chart,
        DrawingKind::PriceLabel,
        vec![p(15.0, 103.0)],
        &fork(&format!(r##""color":"{INK}""##)),
    );
    let tip = anchor(&chart, label, 0);
    let fills = band_fills(&mut chart);
    assert!(
        fills.len() == 1
            && fills[0]
                .0
                .iter()
                .chain(&fills[0].1)
                .any(|&point| close(point, tip, 1e-3)),
        "the tail's tip is the anchor: {fills:?}"
    );
    let price = chart.format_drawing_price(chart.drawing(label).unwrap(), 103.0);
    let runs = text_runs(&mut chart);
    let (_, x, y, _) = run_of(&runs, &price);
    assert!(
        (x - (tip.0 + 8.0)).abs() < 1e-3 && *y < tip.1 - 10.0,
        "{runs:?}"
    );
    assert!(*x < chart.pane_w - 100.0);
    assert!(chart.drawing_apply_options(label, r##"{"text":"entry","text_color":"#ff0000"}"##));
    let runs = text_runs(&mut chart);
    let (_, entry_x, entry_y, color) = run_of(&runs, "entry");
    assert_eq!(*color, Color::rgb(255, 0, 0));
    // The bubble grows upward: the price moves up a line, the text takes the bottom line.
    let (_, x, y, _) = run_of(&runs, &price);
    assert!((entry_x - x).abs() < 1e-3 && entry_y > y, "{runs:?}");
    // The editor edits from the line after the price.
    let layout = chart.drawing_text_edit_layout(label).unwrap();
    assert!(layout.multiline && (layout.y - entry_y).abs() < 1e-3);
    // Upstream's price label keeps its own bubble: a tail whose tip is the anchor and the price
    // left-aligned above-right of it, never the fork's speech bubble.
    chart.remove_drawing(label);
    let upstream = add(
        &mut chart,
        DrawingKind::PriceLabel,
        vec![p(15.0, 103.0)],
        "{}",
    );
    let tip = anchor(&chart, upstream, 0);
    let frame = chart.build_frame();
    let main = &frame.panes[0].main;
    assert!(main.iter().any(|prim| matches!(
        prim,
        Prim::Triangle { a, .. } if close((f64::from(a[0]), f64::from(a[1])), tip, 1e-3)
    )));
    assert!(main.iter().any(|prim| matches!(
        prim,
        Prim::Text { text, x, y, align: TextAlign::Left, .. }
            if *text == price && f64::from(*x) > tip.0 && f64::from(*y) < tip.1
    )));
    assert!(band_fills(&mut chart).is_empty(), "{upstream}");
}

#[test]
fn a_fork_form_price_note_boxes_its_price_and_text_in_its_text_slot() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::PriceNote,
        vec![p(15.0, 103.0)],
        &fork(&format!(r##""text":"memo","color":"{INK}""##)),
    );
    let line_y = anchor(&chart, id, 0).1;
    let price = chart.format_drawing_price(chart.drawing(id).unwrap(), 103.0);
    let runs = text_runs(&mut chart);
    let (_, px, py, color) = run_of(&runs, &price);
    let (_, mx, my, _) = run_of(&runs, "memo");
    // Upstream's Right/Top slot: above the line at its right end, the price first.
    assert!(*py < line_y && *my > *py && (mx - px).abs() < 1e-3);
    assert!(*px > chart.pane_w / 2.0);
    assert_eq!(*color, Color::rgb(255, 255, 255));
    let boxes = rects(&mut chart);
    let (left, top, right, bottom, _) = boxes.iter().find(|rect| rect.4 == ink()).unwrap();
    assert!((right - (chart.pane_w - 4.0)).abs() <= 1.0 && *bottom <= line_y - 3.0);
    assert!(*left < *px && *top < *py);
    // The box is a body target; the editor edits from the text line.
    assert_eq!(hit(&chart, mx + 2.0, *my), Some(id));
    let layout = chart.drawing_text_edit_layout(id).unwrap();
    assert!(layout.multiline && (layout.y - my).abs() < 1e-3);
    // Upstream's price note paints only its text run.
    chart.remove_drawing(id);
    add(
        &mut chart,
        DrawingKind::PriceNote,
        vec![p(15.0, 103.0)],
        r#"{"text":"memo"}"#,
    );
    assert_eq!(texts_of(&mut chart), ["memo"]);
}

#[test]
fn coincident_signposts_stand_a_pole_whose_top_handle_drags_from_where_it_is_painted() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::Signpost,
        vec![p(15.0, 103.0), p(15.0, 103.0)],
        "{}",
    );
    let foot = anchor(&chart, id, 0);
    // The pole is 40 CSS px tall and hits along its length.
    assert_eq!(hit(&chart, foot.0, foot.1 - 20.0), Some(id));
    assert_eq!(hit(&chart, foot.0, foot.1 - 60.0), None);
    // Its top handle sits on the pole's top.
    chart.set_selected_drawing(Some(id));
    let top = (foot.0, foot.1 - 40.0);
    let grabbed = chart.hit_test_drawing(top.0, top.1).unwrap();
    assert_eq!((grabbed.id, grabbed.part), (id, DrawingDragPart::Handle(0)));
    // Dragging it 10 px up puts the top anchor under the pointer, not 40 px below it.
    assert!(chart.drawing_drag_start_at(top.0, top.1));
    chart.drawing_drag_to(top.0, top.1 - 10.0, DrawingModifiers::default());
    assert!(close(anchor(&chart, id, 1), (top.0, top.1 - 10.0), 1e-3));
    assert!(close(anchor(&chart, id, 0), foot, 1e-9));
    // Escape restores the coincident anchors.
    chart.drawing_drag_cancel();
    let points = &chart.drawing(id).unwrap().points;
    assert_eq!(points[0], points[1]);
    // Distinct anchors keep upstream's handles and stem.
    assert!(
        chart
            .set_drawing_anchors(id, &[p(15.0, 103.0).into(), p(15.0, 104.0).into()])
            .is_ok()
    );
    let top = anchor(&chart, id, 1);
    let grabbed = chart.hit_test_drawing(top.0, top.1).unwrap();
    assert_eq!(grabbed.part, DrawingDragPart::Anchor(1));
}

#[test]
fn dragging_a_coincident_signposts_foot_moves_its_pole_with_it() {
    // The fork's one-anchor signpost moved whole: its foot handle keeps the anchors coincident,
    // so the pole stands 40 CSS px tall on the new foot instead of shrinking to a stem.
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::Signpost,
        vec![p(15.0, 103.0), p(15.0, 103.0)],
        "{}",
    );
    chart.set_selected_drawing(Some(id));
    let foot = anchor(&chart, id, 0);
    let grabbed = chart.hit_test_drawing(foot.0, foot.1).unwrap();
    assert_eq!((grabbed.id, grabbed.part), (id, DrawingDragPart::Anchor(0)));
    assert!(chart.drawing_drag_start_at(foot.0, foot.1));
    chart.drawing_drag_to(foot.0, foot.1 + 5.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    let points = &chart.drawing(id).unwrap().points;
    assert_eq!(points[0], points[1]);
    let moved = anchor(&chart, id, 0);
    assert!(close(moved, (foot.0, foot.1 + 5.0), 1e-3), "{moved:?}");
    // The top handle stands on the moved foot's pole.
    let grabbed = chart.hit_test_drawing(moved.0, moved.1 - 40.0).unwrap();
    assert_eq!((grabbed.id, grabbed.part), (id, DrawingDragPart::Handle(0)));
    // A keyboard nudge of the foot keeps them coincident too.
    assert!(chart.nudge_selected_drawing(0.0, -10.0, Some(0)));
    let points = &chart.drawing(id).unwrap().points;
    assert_eq!(points[0], points[1]);
}

#[test]
fn a_foot_drag_sample_onto_a_distinct_signposts_top_does_not_capture_it() {
    // Only a signpost that was coincident when the drag began keeps its top on its foot: a
    // distinct-anchor signpost whose foot passes exactly over its top (time-snapped slot, same
    // price) keeps its top where it started once the foot moves on.
    let mut chart = chart();
    let foot_point = p(15.0, 103.0);
    let probe = crate::Drawing::new(0, DrawingKind::Signpost, 0, Vec::new());
    let foot = chart.drawing_point_px(&probe, foot_point).unwrap();
    // The exact price the drag converts the pointer at `foot.1 - 40` into.
    let top_y = foot.1 + ((foot.1 - 40.0) - foot.1);
    let top_point = chart
        .drawing_from_px_for(0, crate::DrawingPriceScale::Right, foot.0, top_y)
        .unwrap();
    let id = add(
        &mut chart,
        DrawingKind::Signpost,
        vec![foot_point, top_point],
        "{}",
    );
    chart.set_selected_drawing(Some(id));
    let grabbed = chart.hit_test_drawing(foot.0, foot.1).unwrap();
    assert_eq!((grabbed.id, grabbed.part), (id, DrawingDragPart::Anchor(0)));
    assert!(chart.drawing_drag_start_at(foot.0, foot.1));
    chart.drawing_drag_to(foot.0, foot.1 - 40.0, DrawingModifiers::default());
    let points = &chart.drawing(id).unwrap().points;
    assert_eq!(points[0], points[1], "the sample lands exactly on the top");
    chart.drawing_drag_to(foot.0, foot.1 + 20.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    let points = &chart.drawing(id).unwrap().points;
    assert_eq!(points[1], top_point);
    assert_ne!(points[0], points[1]);
}

#[test]
fn a_coincident_signposts_label_sits_on_its_pole_at_every_pixel_ratio() {
    // A coincident signpost's label follows its 40 CSS px pole exactly as a signpost placed with
    // a 40 CSS px stem does, at every device pixel ratio: the painted run (bitmap px) and the
    // editor's run (media px) differ by upstream's marker offset only, never by the pole.
    for dpr in [1.0, 2.0] {
        let mut chart = chart_with(&hourly(40), dpr);
        let mut offsets = Vec::new();
        for (text, coincident) in [("Pole", true), ("Stem", false)] {
            let foot = p(15.0, 103.0);
            let top = if coincident {
                foot
            } else {
                let (x, y) = chart
                    .drawing_point_px(
                        &crate::Drawing::new(0, DrawingKind::Signpost, 0, Vec::new()),
                        foot,
                    )
                    .unwrap();
                chart
                    .drawing_from_px_for(0, crate::DrawingPriceScale::Right, x, y - 40.0)
                    .unwrap()
            };
            let id = add(
                &mut chart,
                DrawingKind::Signpost,
                vec![foot, top],
                &format!(r#"{{"text":"{text}"}}"#),
            );
            let (x, y, ..) = painted_run(&mut chart, text);
            let (mx, my) = chart.drawing_text_coordinate(id).unwrap();
            offsets.push((x / dpr - mx, y / dpr - my, my));
        }
        let (pole, stem) = (offsets[0], offsets[1]);
        assert!(
            (pole.0 - stem.0).abs() < 1e-3 && (pole.1 - stem.1).abs() < 1e-3,
            "dpr {dpr}: {pole:?} vs {stem:?}"
        );
        // Both labels sit at the same height on their marker.
        assert!(
            (pole.2 - stem.2).abs() < 1e-3,
            "dpr {dpr}: {pole:?} vs {stem:?}"
        );
    }
}

#[test]
fn fork_form_signposts_and_arrow_marks_box_their_text_and_keep_one_editor_owner() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::Signpost,
        vec![p(15.0, 103.0), p(15.0, 103.0)],
        &fork(&format!(r##""text":"Sign","color":"{INK}""##)),
    );
    let foot = anchor(&chart, id, 0);
    let top = (foot.0, foot.1 - 40.0);
    let runs = text_runs(&mut chart);
    let (_, x, y, color) = run_of(&runs, "Sign");
    // The plate on the pennant, centered on the pole, on the stroke color.
    assert!(*y < top.1 - 14.0, "{runs:?}");
    assert_eq!(*color, Color::rgb(255, 255, 255));
    let plate = rects(&mut chart)
        .into_iter()
        .find(|rect| rect.4 == ink())
        .unwrap();
    assert!(((plate.0 + plate.2) / 2.0 - top.0).abs() <= 1.0 && plate.3 <= top.1 - 13.0);
    assert_eq!(hit(&chart, x + 2.0, *y), Some(id));
    let layout = chart.drawing_text_edit_layout(id).unwrap();
    assert!(layout.multiline);
    assert!((layout.x - x).abs() < 1e-3 && (layout.y - y).abs() < 1e-3);
    // An emptied fork-form signpost or arrow mark opens one multi-line editor, which keeps its
    // owner as text is typed (static ownership).
    for kind in [DrawingKind::Signpost, DrawingKind::ArrowMarkerUp] {
        let empty = add(&mut chart, kind, points_for(kind), &fork(""));
        let points = if kind == DrawingKind::Signpost {
            vec![p(25.0, 103.0), p(25.0, 103.0)]
        } else {
            vec![p(25.0, 103.0)]
        };
        assert!(
            chart
                .set_drawing_anchors(
                    empty,
                    &points.into_iter().map(Into::into).collect::<Vec<_>>()
                )
                .is_ok()
        );
        assert!(
            chart.drawing_text_edit_layout(empty).unwrap().multiline,
            "{kind:?}"
        );
        assert!(chart.begin_drawing_text_edit(empty, false), "{kind:?}");
        assert!(chart.drawing_text_edit_insert("x"));
        assert!(
            chart.drawing_text_edit_layout(empty).unwrap().multiline,
            "{kind:?}"
        );
        assert!(chart.drawing_text_edit_insert("\n"));
        assert_eq!(chart.drawing(empty).unwrap().text, "x\n");
        assert!(chart.commit_drawing_text_edit());
        assert_eq!(chart.drawing(empty).unwrap().text, "x", "{kind:?} kept");
        chart.remove_drawing(empty);
    }
    chart.remove_drawing(id);
    // An arrow mark's text sits past its tail, aligned by direction, in the arrow's color.
    let r = 7.0;
    for (kind, direction) in [
        (DrawingKind::ArrowMarkerUp, (0.0, -1.0)),
        (DrawingKind::ArrowMarkerDown, (0.0, 1.0)),
        (DrawingKind::ArrowMarkerLeft, (-1.0, 0.0)),
        (DrawingKind::ArrowMarkerRight, (1.0, 0.0)),
    ] {
        let id = add(
            &mut chart,
            kind,
            vec![p(15.0, 103.0)],
            &fork(&format!(r##""text":"Go","color":"{INK}""##)),
        );
        let tip = anchor(&chart, id, 0);
        let runs = text_runs(&mut chart);
        let (_, x, y, color) = run_of(&runs, "Go");
        assert_eq!(*color, ink(), "{kind:?}");
        let width = chart.measure_text_run(
            "Go",
            chart.drawing_text_size(chart.drawing(id).unwrap()),
            &chart.options.get().layout.font_family,
            400,
            false,
        );
        // The run's nearest point to the tip lies past the glyph's tail.
        let (dx, dy) = (x - tip.0, y - tip.1);
        let back = -(dx * direction.0 + dy * direction.1);
        let near = match kind {
            DrawingKind::ArrowMarkerRight => back - width,
            _ => back,
        };
        assert!(near > 2.0 * r - 1.0, "{kind:?}: {near}");
        assert_eq!(hit(&chart, x + 1.0, *y), Some(id), "{kind:?}");
        chart.remove_drawing(id);
        // Upstream form centers the run on the marker box.
        let upstream = add(&mut chart, kind, vec![p(15.0, 103.0)], r#"{"text":"Go"}"#);
        let (ux, uy, ..) = painted_run(&mut chart, "Go");
        assert!(
            (ux - tip.0).abs() <= r + 1.0 && (uy - tip.1).abs() <= 2.0 * r + 1.0,
            "{kind:?}"
        );
        chart.remove_drawing(upstream);
    }
}

#[test]
fn fork_form_annotations_start_from_their_starter_text_and_upstream_form_starts_empty() {
    for (kind, text) in [
        (DrawingKind::Note, "Note"),
        (DrawingKind::Comment, "Comment"),
        (DrawingKind::Callout, "Callout"),
        (DrawingKind::Signpost, "Signpost"),
        (DrawingKind::AnchoredText, "Text"),
    ] {
        for (armed, applied, expected) in [
            (fork(""), None, text),
            ("{}".to_string(), Some(fork("")), text),
            ("{}".to_string(), None, ""),
        ] {
            let mut chart = chart();
            assert!(chart.set_drawing_tool(Some(kind), Some(&armed), None));
            if let Some(patch) = &applied {
                assert!(chart.drawing_tool_apply_options(patch));
            }
            let clicks = [(200.0, 260.0), (420.0, 150.0)];
            let mut update = Default::default();
            for &(x, y) in &clicks[..kind.anchor_count()] {
                update = chart.drawing_tool_activate(x, y, DrawingModifiers::default());
            }
            let crate::DrawingCreationUpdate {
                created: Some(id),
                request_text_edit,
                ..
            } = update
            else {
                panic!("{kind:?} placed");
            };
            assert_eq!(
                chart.drawing(id).unwrap().text,
                expected,
                "{kind:?} {armed}"
            );
            // Placement opens the editor on it; a signpost only in fork form.
            assert_eq!(
                request_text_edit,
                kind != DrawingKind::Signpost || !expected.is_empty(),
                "{kind:?} {armed}"
            );
            assert_eq!(chart.drawing_requests_text_edit(id), request_text_edit);
        }
    }
}

#[test]
fn fork_form_forecasts_box_the_source_and_target_on_market_colors() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::Forecast,
        vec![p(10.0, 101.0), p(20.0, 106.0)],
        &fork(&format!(r##""color":"{INK}""##)),
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let drawing = chart.drawing(id).unwrap().clone();
    let source = chart.format_drawing_price(&drawing, 101.0);
    let change = format!("+{} (+4.95%)", chart.format_drawing_price(&drawing, 5.0));
    let runs = text_runs(&mut chart);
    // The source price left of the source, the change, time, and outcome right of the target.
    let (_, sx, sy, _) = run_of(&runs, &source);
    assert!(*sx < a.0 - 8.0 && (sy - a.1).abs() < 1.0, "{runs:?}");
    let (_, cx, _, _) = run_of(&runs, &change);
    assert!(*cx > b.0 + 8.0);
    run_of(&runs, "Success");
    let time = chart
        .format_crosshair_ts(chart.drawing_anchor_time_of(&drawing, 1).unwrap().round() as i64);
    run_of(&runs, &time);
    assert!(forecast_label(&mut chart).is_none(), "no upstream label");
    let up = Color::parse_css(aeris_charts_core::style::MARKET_UP_CSS).unwrap();
    assert!(
        rects(&mut chart)
            .iter()
            .any(|rect| rect.4 == with_alpha(up, crate::drawings::parts::STATS_ALPHA))
    );
    let frame = chart.build_frame();
    assert!(frame.panes[0].main.iter().any(|prim| matches!(
        prim,
        Prim::Circle { cx, cy, .. } if close((f64::from(*cx), f64::from(*cy)), a, 1e-3)
    )));
    // Both boxes are targets; upstream's label is not.
    assert_eq!(hit(&chart, b.0 + 20.0, b.1), Some(id));
    assert_eq!(hit(&chart, sx + 2.0, *sy), Some(id));
    // Failure on the market's down color; nothing while pending.
    assert!(
        chart
            .set_drawing_anchors(id, &[p(10.0, 101.0).into(), p(20.0, 107.0).into()])
            .is_ok()
    );
    let down = Color::parse_css(aeris_charts_core::style::MARKET_DOWN_CSS).unwrap();
    assert!(texts_of(&mut chart).contains(&"Failure".to_string()));
    assert!(
        rects(&mut chart)
            .iter()
            .any(|rect| rect.4 == with_alpha(down, crate::drawings::parts::STATS_ALPHA))
    );
    assert!(
        chart
            .set_drawing_anchors(id, &[p(10.0, 101.0).into(), p(45.0, 107.0).into()])
            .is_ok()
    );
    let texts = texts_of(&mut chart);
    assert!(
        !texts
            .iter()
            .any(|text| text == "Success" || text == "Failure")
    );
    // A target on the latest bar stays pending until the next streamed bar decides it, and the
    // retained frame repaints the box.
    assert!(
        chart
            .set_drawing_anchors(id, &[p(30.0, 101.0).into(), p(39.0, 107.0).into()])
            .is_ok()
    );
    assert!(!texts_of(&mut chart).contains(&"Failure".to_string()));
    assert!(chart.update_series_bar(0, 40.0 * HOUR, [100.0, 100.5, 99.5, 100.0]));
    assert!(texts_of(&mut chart).contains(&"Failure".to_string()));
    // Upstream form: its label, and no box target.
    chart.remove_drawing(id);
    let upstream = add(
        &mut chart,
        DrawingKind::Forecast,
        vec![p(10.0, 101.0), p(20.0, 106.0)],
        "{}",
    );
    assert_eq!(
        forecast_label(&mut chart).as_deref(),
        Some("+5.0% · target reached")
    );
    assert_eq!(hit(&chart, b.0 + 20.0, b.1), None, "{upstream}");
}

#[test]
fn indexed_hit_testing_matches_brute_force_for_every_fork_form_tool() {
    let mut chart = chart();
    let options = fork(
        r#""text":"one\ntwo","labels":[{"metric":"price_change","visible":true,"position":"on"}]"#,
    );
    for copy in 0..3 {
        let shift = copy as f64 * 0.9;
        for kind in KINDS {
            let mut points: Vec<DrawingPoint> = points_for(kind)
                .into_iter()
                .map(|point| p(point.logical + shift, point.price + shift * 0.3))
                .collect();
            if kind == DrawingKind::Signpost && copy == 0 {
                points[1] = points[0];
            }
            add(&mut chart, kind, points, &options);
            // Upstream's text annotations of several lines too.
            if kind.is_text_annotation() {
                add(
                    &mut chart,
                    kind,
                    points_for(kind),
                    r#"{"text":"one\ntwo\nthree"}"#,
                );
            }
        }
    }
    chart.set_selected_drawing(chart.drawings().first().map(|drawing| drawing.id));
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
            assert_eq!(
                chart.drawing_text_hit_at(x, y),
                chart.drawing_text_hit_at_bruteforce(x, y),
                "({x}, {y})"
            );
            hits += usize::from(indexed.is_some());
        }
    }
    assert!(hits > 100, "the grid meets the drawings ({hits} hits)");
}

#[test]
fn text_annotations_stack_multi_line_text_in_one_box() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::Note,
        vec![p(15.0, 103.0)],
        r#"{"text":"first\nsecond"}"#,
    );
    let size = chart.drawing_text_size(chart.drawing(id).unwrap());
    let runs = text_runs(&mut chart);
    let (_, x1, y1, _) = run_of(&runs, "first");
    let (_, x2, y2, _) = run_of(&runs, "second");
    assert_eq!(runs.len(), 2, "one run per line: {runs:?}");
    assert!((x1 - x2).abs() < 1e-3 && ((y2 - y1) - size * 1.2).abs() < 1e-3);
    // One box covers both lines, centered on the anchor (Middle).
    let anchor_y = anchor(&chart, id, 0).1;
    assert!(((y1 + y2) / 2.0 - anchor_y).abs() < 1e-3);
    let boxes = rects(&mut chart);
    let tint = Color::parse_css("#facc1533").unwrap();
    let boxes = boxes
        .iter()
        .filter(|rect| rect.4 == tint)
        .collect::<Vec<_>>();
    assert_eq!(boxes.len(), 1);
    assert!(boxes[0].1 < y1 - size * 0.6 && boxes[0].3 > y2 + size * 0.6);
    // The second line is a target; the editor is multi-line on the painted lines.
    assert_eq!(hit(&chart, x2 + 2.0, *y2), Some(id));
    let layout = chart.drawing_text_edit_layout(id).unwrap();
    assert!(layout.multiline);
    assert!((layout.x - x1).abs() < 1e-3 && (layout.y - y1).abs() < 1e-3);
    assert!((layout.line_height - size * 1.2).abs() < 1e-9);
    // The native caret stands on its own line.
    assert!(chart.begin_drawing_text_edit(id, true));
    let frame = chart.build_frame();
    let ink_color = chart.drawing_label_color(chart.drawing(id).unwrap());
    let caret = frame.panes[0]
        .main
        .iter()
        .find_map(|prim| match prim {
            Prim::Rect { rect, color } if *color == ink_color && rect.w == 1 => Some(*rect),
            _ => None,
        })
        .expect("a caret bar");
    let center = f64::from(caret.y) + f64::from(caret.h) / 2.0;
    assert!((center - y2).abs() <= 1.0, "{center} vs {y2}");
    assert!(chart.cancel_drawing_text_edit());
    // A single line paints upstream's one centered run.
    assert!(chart.drawing_apply_options(id, r#"{"text":"single"}"#));
    let frame = chart.build_frame();
    assert!(frame.panes[0].main.iter().any(|prim| matches!(
        prim,
        Prim::Text { text, align: TextAlign::Center, .. } if text == "single"
    )));
    // Culling covers every line: a five-line note above its anchor, which sits below the pane,
    // still paints its upper lines.
    let bottom = chart.panes[0].top + chart.panes[0].height;
    let below = chart
        .drawing_from_px_for(0, crate::DrawingPriceScale::Right, 300.0, bottom + 40.0)
        .unwrap();
    let tall = add(
        &mut chart,
        DrawingKind::Note,
        vec![below],
        r#"{"text":"1\n2\n3\n4\n5","text_v_align":"top"}"#,
    );
    let runs = text_runs(&mut chart);
    assert!(run_of(&runs, "1").2 < bottom);
    assert!(chart.drawing_viewport_candidate_reference(chart.drawing(tall).unwrap()));
}

/// `count` hourly bars whose highs and lows vary bar to bar, and their OHLC columns.
fn wide_chart(count: usize) -> (ChartEngine, [Vec<f64>; 4]) {
    let times = hourly(count);
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let close = (0..count)
        .map(|index| 100.0 + ((index * 37) % 101) as f64 / 10.0)
        .collect::<Vec<_>>();
    let open = close.iter().map(|value| value - 0.25).collect::<Vec<_>>();
    let high = (0..count)
        .map(|index| close[index] + 0.5 + ((index * 13) % 7) as f64 / 10.0)
        .collect::<Vec<_>>();
    let low = (0..count)
        .map(|index| close[index] - 0.5 - ((index * 11) % 5) as f64 / 10.0)
        .collect::<Vec<_>>();
    chart
        .set_series_data(0, &times, &open, &high, &low, &close)
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.build_frame();
    (chart, [open, high, low, close])
}

#[test]
fn wide_bars_pattern_sources_aggregate_into_bucket_index_offsets() {
    let (mut chart, [open, high, low, close]) = wide_chart(2000);
    let id = chart
        .add_drawing(
            DrawingKind::BarsPattern,
            0,
            vec![p(0.0, 100.0), p(1999.0, 101.0), p(100.0, 105.0)],
            None,
        )
        .expect("a 2000-bar source is captured");
    let bars = chart.drawing(id).unwrap().bars_pattern.clone();
    // 2000 slots in buckets of 4: offsets are bucket indexes, within the stored bound, and each
    // bucket merges its bars (first open, highest high, lowest low, last close).
    assert_eq!(bars.len(), 500);
    for (index, bar) in bars.iter().enumerate() {
        assert_eq!(usize::from(bar.offset), index);
        let rows = index * 4..index * 4 + 4;
        assert_eq!(bar.open, open[rows.start], "bucket {index}");
        assert_eq!(bar.close, close[rows.end - 1], "bucket {index}");
        let highest = high[rows.clone()].iter().copied().fold(f64::MIN, f64::max);
        let lowest = low[rows].iter().copied().fold(f64::MAX, f64::min);
        assert_eq!((bar.high, bar.low), (highest, lowest), "bucket {index}");
    }
    // A range within the bound copies each bar at its own offset, as before.
    let narrow = chart
        .add_drawing(
            DrawingKind::BarsPattern,
            0,
            vec![p(10.0, 100.0), p(521.0, 101.0), p(100.0, 105.0)],
            None,
        )
        .unwrap();
    let copy = &chart.drawing(narrow).unwrap().bars_pattern;
    assert_eq!(copy.len(), 512);
    assert_eq!((copy[0].close, copy[511].offset), (close[10], 511));
    // A source edit widening past the bound commits: 1491 slots in buckets of 3.
    assert!(
        chart
            .set_drawing_anchors(
                narrow,
                &[
                    p(10.0, 100.0).into(),
                    p(1500.0, 101.0).into(),
                    p(100.0, 105.0).into()
                ]
            )
            .is_ok()
    );
    let widened = &chart.drawing(narrow).unwrap().bars_pattern;
    assert_eq!(widened.len(), 497);
    assert_eq!(widened[1].open, open[13]);
    // The aggregated copy persists.
    let exported = chart.export_state_json().unwrap();
    let (mut again, _) = wide_chart(2000);
    again.import_state_json(&exported).unwrap();
    let restored = &again.drawing(id).unwrap().bars_pattern;
    assert_eq!(restored.len(), bars.len());
    assert!(restored.iter().zip(&bars).all(|(a, b)| a.offset == b.offset
        && (a.high - b.high).abs() < 1e-9
        && (a.low - b.low).abs() < 1e-9));
}

#[test]
fn wide_as_of_bars_patterns_merge_their_own_rows_into_bucket_index_offsets() {
    // A 1600-point axis and an as-of overlay of its own: hours with no row (gaps) and extra
    // half-hour rows that collapse onto the next point.
    let count = 1600;
    let axis = hourly(count);
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let flat = vec![50.0; count];
    chart
        .set_series_data(0, &axis, &flat, &flat, &flat, &flat)
        .unwrap();
    let mut times = Vec::new();
    for (index, &time) in axis.iter().enumerate() {
        if index % 5 == 2 {
            times.push(time - HOUR / 2.0);
        }
        if index % 9 != 4 {
            times.push(time);
        }
    }
    let value =
        |row: usize, step: usize, spread: f64| 200.0 + ((row * step) % 101) as f64 / 10.0 + spread;
    let close = (0..times.len())
        .map(|row| value(row, 37, 0.0))
        .collect::<Vec<_>>();
    let open = (0..times.len())
        .map(|row| value(row, 23, 0.0))
        .collect::<Vec<_>>();
    let high = (0..times.len())
        .map(|row| close[row].max(open[row]) + 0.5 + ((row * 13) % 7) as f64 / 10.0)
        .collect::<Vec<_>>();
    let low = (0..times.len())
        .map(|row| close[row].min(open[row]) - 0.5 - ((row * 11) % 5) as f64 / 10.0)
        .collect::<Vec<_>>();
    let overlay = chart.add_series(crate::SeriesKind::Candlestick);
    chart
        .set_series_data(overlay, &times, &open, &high, &low, &close)
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
    // Logical 10..=1590 is 1581 slots: buckets of 4 slots.
    let (first, last, stride) = (10_usize, 1590_usize, 4_usize);
    let id = chart
        .add_drawing(
            DrawingKind::BarsPattern,
            1,
            vec![
                p(first as f64, 200.0),
                p(last as f64, 201.0),
                p(1600.0, 205.0),
            ],
            None,
        )
        .expect("a wide as-of range is captured");
    // Brute force: each row sits at the first axis point at or after its time; rows merge per
    // bucket in row order.
    let mut expected: Vec<(u16, f64, f64, f64, f64)> = Vec::new();
    for (row, &time) in times.iter().enumerate() {
        let index = axis.partition_point(|&point| point < time);
        if index < first || index > last {
            continue;
        }
        let offset = ((index - first) / stride) as u16;
        match expected.last_mut() {
            Some(bar) if bar.0 == offset => {
                bar.2 = bar.2.max(high[row]);
                bar.3 = bar.3.min(low[row]);
                bar.4 = close[row];
            }
            _ => expected.push((offset, open[row], high[row], low[row], close[row])),
        }
    }
    let copy = chart
        .drawing(id)
        .unwrap()
        .bars_pattern
        .iter()
        .map(|bar| (bar.offset, bar.open, bar.high, bar.low, bar.close))
        .collect::<Vec<_>>();
    assert_eq!(copy.len(), (last - first + 1).div_ceil(stride));
    assert!(
        copy.iter()
            .all(|bar| usize::from(bar.0) < crate::drawings::MAX_BARS_PATTERN_BARS)
    );
    assert_eq!(copy, expected);
}

#[test]
fn wide_union_bars_patterns_read_a_logarithmic_number_of_summaries_per_bucket() {
    let count = 100_000;
    let (chart, _) = wide_chart(count);
    let drawing = crate::Drawing::new(
        900,
        DrawingKind::BarsPattern,
        0,
        vec![p(0.0, 100.0), p((count - 1) as f64, 101.0), p(10.0, 105.0)],
    );
    let rows = chart.drawing_source_window(&drawing).unwrap();
    let stride = count.div_ceil(crate::drawings::MAX_BARS_PATTERN_BARS);
    let (bars, read) = chart
        .capture_bars_pattern_buckets(rows, 0, (count - 1) as i64, stride as i64)
        .unwrap();
    assert_eq!(bars.len(), count.div_ceil(stride));
    let bound = bars.len() * 4 * (count as f64).log2().ceil() as usize;
    assert!(read <= bound, "{read} rows and summaries for {count} bars");
}
