//! Patterns, Elliott waves, and cycles engine tests: catalog defaults, armed and progressive
//! placement, shared-part frames (ratios, fills, necklines, apexes, degree labels, repeats) and
//! hit testing (indexed and brute force, on log and percentage scales and lower panes), culling
//! that never drops a painted part, drags, keyboard nudges, magnet, time identity, schema, kind
//! options, templates, patches with history, persistence (old documents included), clipboard,
//! sync, and bounded repeats at extreme zoom.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{LineStyle, Prim};

use super::super::super::parts::{DrawingPart, DrawingParts, PartContext};
use super::super::super::{DrawingHandleMode, DrawingPlacement, DrawingTextLayout};
use super::{ElliottWaveDegree, WaveMark, MAX_CURVE_POINTS};
use crate::{
    ChartEngine, DrawingAnchor, DrawingDragPart, DrawingId, DrawingKind, DrawingMagnetMode,
    DrawingModifiers, DrawingPoint, DrawingPriceScale,
};

const KINDS: [DrawingKind; 14] = [
    DrawingKind::XabcdPattern,
    DrawingKind::CypherPattern,
    DrawingKind::AbcdPattern,
    DrawingKind::HeadAndShoulders,
    DrawingKind::TrianglePattern,
    DrawingKind::ThreeDrivesPattern,
    DrawingKind::ElliottImpulseWave,
    DrawingKind::ElliottCorrectionWave,
    DrawingKind::ElliottTriangleWave,
    DrawingKind::ElliottDoubleCombo,
    DrawingKind::ElliottTripleCombo,
    DrawingKind::CyclicLines,
    DrawingKind::TimeCycles,
    DrawingKind::SineLine,
];
const INK: &str = "#123456";
const HOUR: f64 = 3_600.0;

fn chart_with(times: &[f64], dpr: f64) -> ChartEngine {
    let mut chart = ChartEngine::new(800.0, 500.0, dpr);
    let values = (0..times.len())
        .map(|index| 100.0 + (index % 7) as f64)
        .collect::<Vec<_>>();
    chart
        .set_series_data(0, times, &values, &values, &values, &values)
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
    chart.add_drawing(kind, 0, points, Some(options)).unwrap()
}

/// A zigzag alternating between highs and lows, one anchor per defining point.
fn zigzag_points(kind: DrawingKind) -> Vec<DrawingPoint> {
    (0..kind.anchor_count())
        .map(|index| {
            let high = index % 2 == 1;
            p(
                6.0 + index as f64 * 3.0,
                if high { 104.5 } else { 101.0 } + index as f64 * 0.1,
            )
        })
        .collect()
}

fn anchor(chart: &ChartEngine, id: DrawingId, index: usize) -> (f64, f64) {
    chart.drawing_point_to_coordinate(id, index).unwrap()
}

fn ink() -> Color {
    Color::parse_css(INK).unwrap()
}

fn ink_fill() -> Color {
    Color::rgba(0x12, 0x34, 0x56, super::FILL_ALPHA)
}

/// One drawing-colored polyline of the first pane: points, width, and style.
type InkLine = (Vec<(f64, f64)>, f32, LineStyle);

fn ink_polylines(chart: &mut ChartEngine) -> Vec<InkLine> {
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
            } if *color == ink() => Some((
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

fn ink_fills(chart: &mut ChartEngine) -> usize {
    let frame = chart.build_frame();
    frame.panes[0]
        .main
        .iter()
        .filter(|prim| matches!(prim, Prim::BandFill { fill, .. } if *fill == ink_fill()))
        .count()
}

fn ink_vlines(chart: &mut ChartEngine) -> Vec<i32> {
    let frame = chart.build_frame();
    frame.panes[0]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::VLine { x, color, .. } if *color == ink() => Some(*x),
            _ => None,
        })
        .collect()
}

/// Every text run of the first pane: text, left x, and first-line center y.
fn texts(chart: &mut ChartEngine) -> Vec<(String, f64, f64)> {
    let frame = chart.build_frame();
    frame.panes[0]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::Text { text, x, y, .. } => Some((text.clone(), f64::from(*x), f64::from(*y))),
            _ => None,
        })
        .collect()
}

fn texts_of(chart: &mut ChartEngine) -> Vec<String> {
    texts(chart).into_iter().map(|(text, ..)| text).collect()
}

fn text_y(chart: &mut ChartEngine, text: &str) -> f64 {
    texts(chart)
        .into_iter()
        .find(|(run, ..)| run == text)
        .unwrap_or_else(|| panic!("{text} is painted"))
        .2
}

/// Total length of the thin (1 CSS px at DPR 1) solid runs lying on segment `a → b`: a dashed
/// connector reaches the frame pre-split into its dashes.
fn dashes_on(lines: &[InkLine], a: (f64, f64), b: (f64, f64)) -> f64 {
    lines
        .iter()
        .filter(|(points, width, style)| {
            *width == 1.0
                && *style == LineStyle::Solid
                && points.len() == 2
                && points.iter().all(|&point| {
                    aeris_charts_render::shape::distance_to_segment(point, a, b) < 0.01
                })
        })
        .map(|(points, ..)| (points[1].0 - points[0].0).hypot(points[1].1 - points[0].1))
        .sum()
}

/// Whether `a → b` is painted as a dashed connector: about half its length in dashes.
fn dashed_between(lines: &[InkLine], a: (f64, f64), b: (f64, f64)) -> bool {
    let coverage = dashes_on(lines, a, b) / (b.0 - a.0).hypot(b.1 - a.1);
    (0.35..=0.65).contains(&coverage)
}

fn close(a: (f64, f64), b: (f64, f64), tolerance: f64) -> bool {
    (a.0 - b.0).abs() <= tolerance && (a.1 - b.1).abs() <= tolerance
}

fn px(chart: &ChartEngine, logical: f64, price: f64) -> (f64, f64) {
    chart
        .drawing_to_px_for(0, DrawingPriceScale::Right, p(logical, price))
        .unwrap()
}

#[test]
fn catalog_defaults_follow_each_tool() {
    for kind in KINDS {
        let drawing = crate::Drawing::new(1, kind, 0, Vec::new());
        let spec = kind.spec();
        assert!(spec.family.is_some(), "{kind:?} is a family tool");
        assert!(spec.family.unwrap().partial_preview);
        assert_eq!(spec.handles, DrawingHandleMode::Anchors);
        assert_eq!(spec.text_layout, DrawingTextLayout::Box);
        assert!(!spec.axis_price_label);
        let count = match kind {
            DrawingKind::XabcdPattern | DrawingKind::CypherPattern => 5,
            DrawingKind::AbcdPattern
            | DrawingKind::TrianglePattern
            | DrawingKind::ElliottCorrectionWave
            | DrawingKind::ElliottDoubleCombo => 4,
            DrawingKind::HeadAndShoulders | DrawingKind::ThreeDrivesPattern => 7,
            DrawingKind::ElliottImpulseWave
            | DrawingKind::ElliottTriangleWave
            | DrawingKind::ElliottTripleCombo => 6,
            _ => 2,
        };
        assert_eq!(spec.placement, DrawingPlacement::ClickAnchors { count });
        let (color, fill, width) = match kind {
            DrawingKind::XabcdPattern | DrawingKind::CypherPattern => ("#2962FF", true, 2.0),
            DrawingKind::AbcdPattern => ("#089981", false, 2.0),
            DrawingKind::HeadAndShoulders => ("#089981", true, 2.0),
            DrawingKind::TrianglePattern => ("#673AB7", true, 2.0),
            DrawingKind::ThreeDrivesPattern => ("#673AB7", false, 2.0),
            DrawingKind::ElliottImpulseWave | DrawingKind::ElliottCorrectionWave => {
                ("#3D85C6", false, 2.0)
            }
            DrawingKind::ElliottTriangleWave => ("#FF9800", false, 2.0),
            DrawingKind::ElliottDoubleCombo | DrawingKind::ElliottTripleCombo => {
                ("#6AA84F", false, 2.0)
            }
            DrawingKind::CyclicLines => ("#80CCDB", false, 1.0),
            DrawingKind::TimeCycles => ("#159980", true, 2.0),
            _ => ("#159980", false, 2.0),
        };
        assert_eq!(
            (drawing.color.as_str(), drawing.fill_enabled, drawing.width),
            (color, fill, width),
            "{kind:?}"
        );
        assert!(!drawing.extend_left && !drawing.extend_right);
        assert!(drawing.labels.is_empty() && drawing.tool_options.is_empty());
    }
}

#[test]
fn elliott_degrees_label_with_their_notation() {
    let impulse = |degree: ElliottWaveDegree| {
        (1..=5)
            .map(|number| degree.label(WaveMark::Number(number)))
            .collect::<Vec<_>>()
    };
    let owned = |labels: [&str; 5], ring: bool| {
        labels
            .into_iter()
            .map(|label| (label.to_string(), ring))
            .collect::<Vec<_>>()
    };
    use ElliottWaveDegree::*;
    assert_eq!(
        impulse(Supermillennium),
        owned(["{I}", "{II}", "{III}", "{IV}", "{V}"], false)
    );
    assert_eq!(
        impulse(Millennium),
        owned(["[I]", "[II]", "[III]", "[IV]", "[V]"], false)
    );
    assert_eq!(
        impulse(Submillennium),
        owned(["<I>", "<II>", "<III>", "<IV>", "<V>"], false)
    );
    assert_eq!(
        impulse(GrandSupercycle),
        owned(["I", "II", "III", "IV", "V"], true)
    );
    assert_eq!(
        impulse(Supercycle),
        owned(["(I)", "(II)", "(III)", "(IV)", "(V)"], false)
    );
    assert_eq!(impulse(Cycle), owned(["I", "II", "III", "IV", "V"], false));
    assert_eq!(impulse(Primary), owned(["1", "2", "3", "4", "5"], true));
    assert_eq!(
        impulse(Intermediate),
        owned(["(1)", "(2)", "(3)", "(4)", "(5)"], false)
    );
    assert_eq!(impulse(Minor), owned(["1", "2", "3", "4", "5"], false));
    assert_eq!(impulse(Minute), owned(["i", "ii", "iii", "iv", "v"], true));
    assert_eq!(
        impulse(Minuette),
        owned(["(i)", "(ii)", "(iii)", "(iv)", "(v)"], false)
    );
    assert_eq!(
        impulse(Subminuette),
        owned(["i", "ii", "iii", "iv", "v"], false)
    );
    let letter = |degree: ElliottWaveDegree, letter| degree.label(WaveMark::Letter(letter));
    assert_eq!(letter(Primary, 'a'), ("A".to_string(), true));
    assert_eq!(letter(Intermediate, 'W'), ("(W)".to_string(), false));
    assert_eq!(letter(Minor, 'z'), ("Z".to_string(), false));
    assert_eq!(letter(Cycle, 'B'), ("b".to_string(), false));
    assert_eq!(letter(Minuette, 'X'), ("(x)".to_string(), false));
    assert_eq!(letter(Supermillennium, 'C'), ("{c}".to_string(), false));
    // Every degree labels distinctly.
    let impulses = ElliottWaveDegree::ALL.map(impulse);
    for (index, labels) in impulses.iter().enumerate() {
        assert!(impulses[index + 1..].iter().all(|other| other != labels));
    }
}

#[test]
fn armed_tools_place_every_family_kind() {
    let mut chart = chart();
    for kind in KINDS {
        assert!(chart.set_drawing_tool(Some(kind), Some(r##"{"color":"#123456"}"##), None));
        let mut created = None;
        for index in 0..kind.anchor_count() {
            let (x, y) = (
                120.0 + index as f64 * 70.0,
                300.0 - (index % 2) as f64 * 120.0,
            );
            let update = chart.drawing_tool_activate(x, y, DrawingModifiers::default());
            assert!(update.consumed);
            if index + 1 < kind.anchor_count() {
                assert_eq!(update.created, None, "{kind:?} waits for every anchor");
            }
            created = update.created;
        }
        let id = created.unwrap_or_else(|| panic!("{kind:?} committed"));
        let drawing = chart.drawing(id).unwrap();
        assert_eq!(drawing.kind, kind);
        assert_eq!(drawing.points.len(), kind.anchor_count());
        assert_eq!(drawing.color, INK);
        assert_eq!(chart.active_drawing_tool(), None, "one-shot tools disarm");
        assert_eq!(chart.selected_drawing(), Some(id));
        assert_eq!(chart.drawing_handle_count(id), Some(kind.anchor_count()));
    }
}

#[test]
fn placement_previews_the_legs_and_labels_placed_so_far() {
    let mut chart = chart();
    assert!(chart.set_drawing_tool(
        Some(DrawingKind::XabcdPattern),
        Some(r##"{"color":"#123456"}"##),
        None
    ));
    let (x, a) = (px(&chart, 6.0, 101.0), px(&chart, 10.0, 105.0));
    let b = px(&chart, 14.0, 102.0);
    chart.drawing_tool_activate(x.0, x.1, DrawingModifiers::default());
    chart.drawing_tool_pointer_move(a.0, a.1, DrawingModifiers::default(), false);
    let legs = ink_polylines(&mut chart);
    assert_eq!(legs.len(), 1, "X to the pointer after one click");
    assert!(close(legs[0].0[0], x, 0.01) && close(legs[0].0[1], a, 0.01));
    chart.drawing_tool_activate(a.0, a.1, DrawingModifiers::default());
    chart.drawing_tool_pointer_move(b.0, b.1, DrawingModifiers::default(), false);
    let zigzag = ink_polylines(&mut chart)
        .into_iter()
        .find(|(points, _, style)| points.len() == 3 && *style == LineStyle::Solid)
        .expect("X-A-B preview");
    assert!(close(zigzag.0[2], b, 0.01));
    let runs = texts_of(&mut chart);
    // AB/XA = 3/4.
    for label in ["X", "A", "B", "0.750"] {
        assert!(runs.iter().any(|run| run == label), "{label} in {runs:?}");
    }
    assert_eq!(ink_fills(&mut chart), 1, "the XAB triangle shades");
    assert!(
        chart.drawings().is_empty(),
        "nothing commits before five anchors"
    );

    // Every kind previews every prefix without panicking or emitting non-finite geometry.
    for kind in KINDS {
        assert!(chart.set_drawing_tool(Some(kind), None, None));
        for index in 0..kind.anchor_count().saturating_sub(1) {
            let (x, y) = (
                150.0 + index as f64 * 60.0,
                280.0 - (index % 2) as f64 * 100.0,
            );
            chart.drawing_tool_activate(x, y, DrawingModifiers::default());
            chart.drawing_tool_pointer_move(x + 30.0, y - 40.0, DrawingModifiers::default(), false);
            let frame = chart.build_frame();
            assert!(frame.panes[0]
                .points
                .iter()
                .all(|point| point[0].is_finite() && point[1].is_finite()));
        }
        chart.set_drawing_tool(None, None, None);
    }
}

#[test]
fn xabcd_draws_ratio_connectors_shaded_triangles_and_point_labels() {
    let mut chart = chart();
    let points = vec![
        p(6.0, 100.0),
        p(10.0, 105.0),
        p(14.0, 101.91),
        p(18.0, 104.0),
        p(22.0, 100.5),
    ];
    let id = add(
        &mut chart,
        DrawingKind::XabcdPattern,
        points,
        r##"{"color":"#123456"}"##,
    );
    let runs = texts_of(&mut chart);
    for ratio in ["0.618", "0.676", "1.675", "0.900"] {
        assert!(runs.iter().any(|run| run == ratio), "{ratio} in {runs:?}");
    }
    let lines = ink_polylines(&mut chart);
    assert_eq!(lines[0].0.len(), 5, "the zigzag passes every anchor");
    assert_eq!(lines[0].2, LineStyle::Solid);
    // Connectors XB, AC, BD, and XD, each pre-split into solid dashes.
    let [x, a, b, c, d] = [0, 1, 2, 3, 4].map(|index| anchor(&chart, id, index));
    for (from, to) in [(x, b), (a, c), (b, d), (x, d)] {
        assert!(dashed_between(&lines, from, to), "{from:?} → {to:?}");
    }
    assert!(lines.iter().all(|(_, _, style)| *style == LineStyle::Solid));
    assert_eq!(ink_fills(&mut chart), 2);
    // X and B are lows (labels below), A and C highs (labels above).
    assert!(text_y(&mut chart, "X") > x.1);
    assert!(text_y(&mut chart, "A") < a.1);
    assert!(text_y(&mut chart, "B") > b.1);
    // A point label is a body target clear of every stroke.
    let label_y = text_y(&mut chart, "A");
    assert_eq!(
        chart.hit_test_drawing(a.0, label_y).map(|hit| hit.id),
        Some(id)
    );

    // Ratios hide with their connectors; fill switches off with the common property.
    assert!(chart.drawing_apply_options(
        id,
        r#"{"tool_options":{"pattern":{"show_ratios":false}},"fill_enabled":false}"#
    ));
    assert!(!texts_of(&mut chart).iter().any(|run| run == "0.618"));
    assert!(!ink_polylines(&mut chart)
        .iter()
        .any(|(_, width, _)| *width == 1.0));
    assert_eq!(ink_fills(&mut chart), 0);
}

#[test]
fn ratio_labels_paint_above_every_connector() {
    for kind in [
        DrawingKind::XabcdPattern,
        DrawingKind::CypherPattern,
        DrawingKind::AbcdPattern,
        DrawingKind::ThreeDrivesPattern,
    ] {
        let mut chart = chart();
        add(
            &mut chart,
            kind,
            zigzag_points(kind),
            r##"{"color":"#123456"}"##,
        );
        let frame = chart.build_frame();
        let main = &frame.panes[0].main;
        let last_connector = main.iter().rposition(|prim| {
            matches!(prim, Prim::Polyline { width, color, .. } if *width == 1.0 && *color == ink())
        });
        let first_label = main
            .iter()
            .position(|prim| matches!(prim, Prim::Rect { color, .. } if *color == ink()));
        assert!(
            last_connector.is_some() && last_connector < first_label,
            "{kind:?}: a connector dash paints over a ratio label"
        );
    }
}

#[test]
fn dashed_and_dotted_styles_reach_executors_as_solid_runs() {
    // The WebGPU tessellator has no dash concept, so the frame splits every family stroke's
    // dash pattern itself and executors receive only solid polylines.
    let mut chart = chart();
    for (kind, style) in [
        (DrawingKind::AbcdPattern, "dashed"),
        (DrawingKind::ElliottImpulseWave, "dotted"),
        (DrawingKind::SineLine, "dashed"),
        (DrawingKind::TimeCycles, "dotted"),
    ] {
        chart.clear_drawings();
        let options = serde_json::json!({ "color": INK, "style": style }).to_string();
        let id = add(&mut chart, kind, zigzag_points(kind), &options);
        let frame = chart.build_frame();
        assert!(frame.panes[0].main.iter().all(|prim| !matches!(
            prim,
            Prim::Polyline { style, .. } if *style != LineStyle::Solid
        )));
        let runs = ink_polylines(&mut chart);
        assert!(
            runs.len() > kind.anchor_count(),
            "{kind:?} is split into runs"
        );
        if kind == DrawingKind::AbcdPattern {
            let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
            let dashes = runs
                .iter()
                .filter(|(points, ..)| {
                    points.iter().all(|&point| {
                        aeris_charts_render::shape::distance_to_segment(point, a, b) < 0.01
                    })
                })
                .map(|(points, ..)| (points[1].0 - points[0].0).hypot(points[1].1 - points[0].1))
                .sum::<f64>();
            let coverage = dashes / (b.0 - a.0).hypot(b.1 - a.1);
            assert!((0.35..=0.65).contains(&coverage), "{coverage}");
        }
    }
}

#[test]
fn cypher_abcd_and_three_drives_measure_their_conventional_ratios() {
    let mut chart = chart();
    add(
        &mut chart,
        DrawingKind::CypherPattern,
        vec![
            p(6.0, 100.0),
            p(9.0, 105.0),
            p(12.0, 102.0),
            p(15.0, 106.0),
            p(18.0, 101.284),
        ],
        r##"{"color":"#123456"}"##,
    );
    let runs = texts_of(&mut chart);
    for ratio in ["0.600", "1.200", "0.786"] {
        assert!(runs.iter().any(|run| run == ratio), "{ratio} in {runs:?}");
    }
    chart.clear_drawings();

    add(
        &mut chart,
        DrawingKind::AbcdPattern,
        vec![p(6.0, 105.0), p(10.0, 101.0), p(14.0, 103.5), p(18.0, 99.5)],
        r##"{"color":"#123456"}"##,
    );
    let runs = texts_of(&mut chart);
    // BC/AB = 2.5/4, CD/BC = 4/2.5.
    for label in ["0.625", "1.600", "A", "B", "C", "D"] {
        assert!(runs.iter().any(|run| run == label), "{label} in {runs:?}");
    }
    assert_eq!(ink_fills(&mut chart), 0, "ABCD has no regions");
    chart.clear_drawings();

    let drives = add(
        &mut chart,
        DrawingKind::ThreeDrivesPattern,
        vec![
            p(4.0, 100.0),
            p(8.0, 102.0),
            p(11.0, 101.0),
            p(15.0, 103.5),
            p(18.0, 102.5),
            p(22.0, 105.0),
            p(26.0, 103.0),
        ],
        r##"{"color":"#123456"}"##,
    );
    let runs = texts_of(&mut chart);
    // 1/2, 2.5/1, 1/2.5, 2.5/1 and the drive numbers.
    for label in ["0.500", "2.500", "0.400", "1", "2", "3"] {
        assert!(runs.iter().any(|run| run == label), "{label} in {runs:?}");
    }
    let lines = ink_polylines(&mut chart);
    for index in 1..=4 {
        let (from, to) = (
            anchor(&chart, drives, index - 1),
            anchor(&chart, drives, index + 1),
        );
        assert!(dashed_between(&lines, from, to), "connector {index}");
    }
}

#[test]
fn head_and_shoulders_draws_the_neckline_between_the_outer_legs() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::HeadAndShoulders,
        vec![
            p(4.0, 101.0),
            p(8.0, 104.0),
            p(12.0, 102.0),
            p(16.0, 106.0),
            p(20.0, 102.0),
            p(24.0, 104.0),
            p(28.0, 101.0),
        ],
        r##"{"color":"#123456"}"##,
    );
    let lines = ink_polylines(&mut chart);
    let neckline = lines
        .iter()
        .find(|(points, ..)| points.len() == 2)
        .expect("neckline")
        .0
        .clone();
    let start = px(&chart, 4.0 + 4.0 / 3.0, 102.0);
    let end = px(&chart, 24.0 + 8.0 / 3.0, 102.0);
    assert!(close(neckline[0], start, 0.01), "{neckline:?} vs {start:?}");
    assert!(close(neckline[1], end, 0.01));
    assert_eq!(ink_fills(&mut chart), 3, "both shoulders and the head");
    let head = anchor(&chart, id, 3);
    assert_eq!(
        texts_of(&mut chart),
        ["Left Shoulder", "Head", "Right Shoulder"]
    );
    assert!(
        text_y(&mut chart, "Head") < head.1,
        "the head's label sits above it"
    );
    assert!(text_y(&mut chart, "Left Shoulder") < anchor(&chart, id, 1).1);
    // Inverse: the same anchors mirrored put the labels below.
    let mirrored = chart
        .drawing(id)
        .unwrap()
        .points
        .iter()
        .map(|point| DrawingAnchor::from(p(point.logical, 206.0 - point.price)))
        .collect::<Vec<_>>();
    assert!(chart.set_drawing_anchors(id, &mirrored).is_ok());
    let head = anchor(&chart, id, 3);
    assert!(text_y(&mut chart, "Head") > head.1);
}

#[test]
fn triangle_pattern_extends_converging_sides_to_their_apex() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::TrianglePattern,
        vec![p(5.0, 106.0), p(8.0, 100.0), p(15.0, 104.0), p(18.0, 102.0)],
        r##"{"color":"#123456"}"##,
    );
    let apex = px(&chart, 21.5, 102.7);
    let lines = ink_polylines(&mut chart);
    let a = anchor(&chart, id, 0);
    let b = anchor(&chart, id, 1);
    assert!(lines
        .iter()
        .any(|(line, ..)| close(line[0], a, 0.01) && close(line[1], apex, 0.01)));
    assert!(lines
        .iter()
        .any(|(line, ..)| close(line[0], b, 0.01) && close(line[1], apex, 0.01)));
    assert_eq!(ink_fills(&mut chart), 1);
    // The side beyond the anchors is a body target.
    let beyond = ((a.0 + 9.0 * apex.0) / 10.0, (a.1 + 9.0 * apex.1) / 10.0);
    assert!(beyond.0 > anchor(&chart, id, 3).0);
    assert_eq!(
        chart.hit_test_drawing(beyond.0, beyond.1).map(|hit| hit.id),
        Some(id)
    );

    // Diverging sides stay between their anchors and shade the convex quad (every anchor on the
    // pane: frames clip family strokes to it).
    let diverging =
        [p(5.0, 106.0), p(8.0, 100.0), p(15.0, 107.0), p(18.0, 99.5)].map(DrawingAnchor::from);
    assert!(chart.set_drawing_anchors(id, &diverging).is_ok());
    let (c, d) = (anchor(&chart, id, 2), anchor(&chart, id, 3));
    let lines = ink_polylines(&mut chart);
    assert!(lines
        .iter()
        .any(|(line, ..)| line.len() == 2 && close(line[0], a, 0.01) && close(line[1], c, 0.01)));
    assert!(lines
        .iter()
        .any(|(line, ..)| line.len() == 2 && close(line[0], b, 0.01) && close(line[1], d, 0.01)));
    assert_eq!(ink_fills(&mut chart), 1);
}

#[test]
fn elliott_waves_label_each_wave_in_the_selected_degree() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::ElliottImpulseWave,
        vec![
            p(5.0, 100.0),
            p(9.0, 103.0),
            p(12.0, 101.5),
            p(18.0, 106.0),
            p(21.0, 104.0),
            p(26.0, 105.5),
        ],
        r##"{"color":"#123456"}"##,
    );
    let runs = texts_of(&mut chart);
    assert_eq!(runs, ["(1)", "(2)", "(3)", "(4)", "(5)"]);
    for (index, label) in ["(1)", "(2)", "(3)", "(4)", "(5)"].into_iter().enumerate() {
        let y = text_y(&mut chart, label);
        let anchor_y = anchor(&chart, id, index + 1).1;
        if index % 2 == 0 {
            assert!(y < anchor_y, "{label} above its high");
        } else {
            assert!(y > anchor_y, "{label} below its low");
        }
    }
    let lines = ink_polylines(&mut chart);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].0.len(), 6);

    // Ringed degrees add a ring around each label; bare ones do not.
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"pattern":{"degree":"primary"}}}"#));
    assert_eq!(texts_of(&mut chart), ["1", "2", "3", "4", "5"]);
    let rings = ink_polylines(&mut chart)
        .into_iter()
        .filter(|(points, width, _)| *width == 1.0 && points.len() > 8)
        .collect::<Vec<_>>();
    assert_eq!(rings.len(), 5);
    let (_, x, y) = texts(&mut chart).remove(0);
    let ring = &rings[0].0;
    let center = (
        ring.iter().map(|point| point.0).sum::<f64>() / ring.len() as f64,
        ring.iter().map(|point| point.1).sum::<f64>() / ring.len() as f64,
    );
    assert!(
        (center.1 - y).abs() < 1.0 && center.0 > x,
        "the ring wraps the label"
    );
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"pattern":{"degree":"minor"}}}"#));
    assert_eq!(ink_polylines(&mut chart).len(), 1);

    // Without the wave only the labels paint, and they remain body targets.
    assert!(chart.drawing_apply_options(id, r#"{"tool_options":{"pattern":{"show_wave":false}}}"#));
    assert!(ink_polylines(&mut chart).is_empty());
    let (_, x, y) = texts(&mut chart).remove(0);
    assert_eq!(
        chart.hit_test_drawing(x + 2.0, y).map(|hit| hit.id),
        Some(id)
    );

    // The other waves use their letters.
    let letters = [
        (
            DrawingKind::ElliottCorrectionWave,
            vec!["(A)", "(B)", "(C)"],
        ),
        (
            DrawingKind::ElliottTriangleWave,
            vec!["(A)", "(B)", "(C)", "(D)", "(E)"],
        ),
        (DrawingKind::ElliottDoubleCombo, vec!["(W)", "(X)", "(Y)"]),
        (
            DrawingKind::ElliottTripleCombo,
            vec!["(W)", "(X)", "(Y)", "(X)", "(Z)"],
        ),
    ];
    for (kind, expected) in letters {
        chart.clear_drawings();
        add(&mut chart, kind, zigzag_points(kind), "{}");
        assert_eq!(texts_of(&mut chart), expected, "{kind:?}");
    }
}

#[test]
fn cyclic_lines_repeat_the_interval_to_the_right_edge() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::CyclicLines,
        vec![p(10.0, 102.0), p(14.0, 102.0)],
        r##"{"color":"#123456"}"##,
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let spacing = b.0 - a.0;
    let lines = ink_vlines(&mut chart);
    let expected = ((chart.pane_w - a.0) / spacing).floor() as usize + 1;
    assert_eq!(lines.len(), expected);
    assert!(lines.iter().all(|&x| f64::from(x) >= a.0.round() - 0.5));
    assert_eq!(lines[0], a.0.round() as i32);
    assert_eq!(lines[1], b.0.round() as i32);
    assert!(dashed_between(&ink_polylines(&mut chart), a, b));
    // A repeat far from both anchors is a body target; the span before the first anchor is not.
    let repeat_x = a.0 + 4.0 * spacing;
    assert_eq!(
        chart.hit_test_drawing(repeat_x, 60.0).map(|hit| hit.id),
        Some(id)
    );
    assert_eq!(chart.hit_test_drawing(a.0 - spacing, 60.0), None);
}

#[test]
fn time_cycles_repeat_arches_both_ways_and_shade_them() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::TimeCycles,
        vec![p(10.0, 101.0), p(14.0, 104.0)],
        r##"{"color":"#123456"}"##,
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let (width, height) = (b.0 - a.0, a.1 - b.1);
    let arches = ink_polylines(&mut chart);
    let expected = (chart.pane_w / width).ceil() as usize;
    assert!(
        arches.len() >= expected && arches.len() <= expected + 2,
        "{} arches for a {width} px cycle",
        arches.len()
    );
    assert_eq!(ink_fills(&mut chart), arches.len());
    // The arch before the first anchor mirrors the defining one.
    let before = arches
        .iter()
        .find(|(arch, ..)| close(arch[0], (a.0 - width, a.1), 0.01))
        .expect("an arch before the anchors");
    assert!(close(*before.0.last().unwrap(), a, 0.01));
    let top = before
        .0
        .iter()
        .map(|point| point.1)
        .fold(f64::INFINITY, f64::min);
    assert!(
        (a.1 - top - height).abs() < 0.3,
        "as tall as the anchors' price difference"
    );

    // The arch far to the right hits; its unselected shading pans the chart, selected it drags.
    let far = (a.0 + 4.5 * width, a.1 - height);
    assert_eq!(
        chart.hit_test_drawing(far.0, far.1).map(|hit| hit.id),
        Some(id)
    );
    let inside = (a.0 + 4.5 * width, a.1 - height / 3.0);
    assert_eq!(chart.hit_test_drawing(inside.0, inside.1), None);
    chart.set_selected_drawing(Some(id));
    assert_eq!(
        chart
            .hit_test_drawing(inside.0, inside.1)
            .map(|hit| hit.part),
        Some(DrawingDragPart::Body)
    );
}

#[test]
fn sine_lines_pass_both_anchors_and_span_the_pane() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::SineLine,
        vec![p(10.0, 105.0), p(14.0, 101.0)],
        r##"{"color":"#123456"}"##,
    );
    let (a, b) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let wave = ink_polylines(&mut chart).remove(0).0;
    assert!(wave.iter().any(|&point| close(point, a, 1e-3)));
    assert!(wave.iter().any(|&point| close(point, b, 1e-3)));
    assert!(wave[0].0 <= 0.0 && wave.last().unwrap().0 >= chart.pane_w);
    assert!(wave
        .iter()
        .all(|point| point.1 >= a.1 - 0.01 && point.1 <= b.1 + 0.01));
    // The next peak sits one period after the first anchor.
    let period = 2.0 * (b.0 - a.0);
    assert!(wave
        .iter()
        .any(|&point| close(point, (a.0 + period, a.1), 1e-3)));
    // Midway between the anchors the wave crosses the middle price.
    let middle = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    assert_eq!(
        chart.hit_test_drawing(middle.0, middle.1).map(|hit| hit.id),
        Some(id)
    );
    assert_eq!(chart.hit_test_drawing(middle.0, a.1), None);
}

#[test]
fn dense_repeats_collapse_and_curves_stay_within_their_budget() {
    let mut chart = chart();
    // A hundredth of a bar is far below the repeat spacing.
    add(
        &mut chart,
        DrawingKind::CyclicLines,
        vec![p(10.0, 102.0), p(10.01, 102.0)],
        r##"{"color":"#123456"}"##,
    );
    assert_eq!(ink_vlines(&mut chart).len(), 2);
    chart.clear_drawings();
    add(
        &mut chart,
        DrawingKind::TimeCycles,
        vec![p(10.0, 101.0), p(10.01, 104.0)],
        r##"{"color":"#123456"}"##,
    );
    assert_eq!(ink_polylines(&mut chart).len(), 1);
    chart.clear_drawings();
    add(
        &mut chart,
        DrawingKind::SineLine,
        vec![p(10.0, 101.0), p(10.01, 104.0)],
        r##"{"color":"#123456"}"##,
    );
    let wave = ink_polylines(&mut chart).remove(0).0;
    assert!(wave.len() <= 257, "one half period: {}", wave.len());
    chart.clear_drawings();

    // Just above the spacing, with tall arches, the tessellation stays bounded.
    let (x0, _) = px(&chart, 10.0, 100.0);
    let narrow = chart
        .drawing_from_px_for(0, DrawingPriceScale::Right, x0 + 3.5, 0.0)
        .unwrap();
    add(
        &mut chart,
        DrawingKind::TimeCycles,
        vec![p(10.0, 99.0), p(narrow.logical, 110.0)],
        r##"{"color":"#123456"}"##,
    );
    let arches = ink_polylines(&mut chart);
    assert!(arches.len() > 200);
    let points = arches.iter().map(|(arch, ..)| arch.len()).sum::<usize>();
    assert!(
        points <= MAX_CURVE_POINTS + 2 * arches.len(),
        "{points} points"
    );
}

#[test]
fn cycles_paint_and_hit_with_their_anchors_scrolled_away() {
    let mut chart = chart();
    crowd(&mut chart);
    let cyclic = add(
        &mut chart,
        DrawingKind::CyclicLines,
        vec![p(-40.0, 102.0), p(-36.0, 102.0)],
        r##"{"color":"#123456"}"##,
    );
    let sine = add(
        &mut chart,
        DrawingKind::SineLine,
        vec![p(-40.0, 104.0), p(-36.0, 103.0)],
        "{}",
    );
    let arches = add(
        &mut chart,
        DrawingKind::TimeCycles,
        vec![p(-40.0, 100.0), p(-36.0, 100.5)],
        "{}",
    );
    chart.build_frame();
    for id in [cyclic, sine, arches] {
        assert!(viewport_candidate(&chart, id), "{id} reaches the pane");
    }
    let x = chart.logical_to_coordinate(20.0).unwrap();
    assert_eq!(
        chart.hit_test_drawing(x, 40.0).map(|hit| hit.id),
        Some(cyclic)
    );
    assert!(!ink_vlines(&mut chart).is_empty());
}

#[test]
fn indexed_hit_testing_matches_brute_force() {
    let mut chart = chart();
    for copy in 0..2 {
        let shift = copy as f64 * 0.7;
        for kind in KINDS {
            let points = zigzag_points(kind)
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
    for gy in 0..24 {
        for gx in 0..39 {
            let (x, y) = (f64::from(gx) * 20.0 + 3.0, f64::from(gy) * 20.0 + 4.0);
            let indexed = chart.hit_test_drawing(x, y);
            assert_eq!(
                indexed,
                chart.hit_test_drawing_bruteforce(x, y),
                "({x}, {y})"
            );
            hits += usize::from(indexed.is_some());
        }
    }
    assert!(hits > 50, "the grid meets the drawings ({hits} hits)");
}

#[test]
fn drags_nudges_and_undo_edit_patterns_as_single_history_entries() {
    let mut chart = chart();
    let id = add(
        &mut chart,
        DrawingKind::XabcdPattern,
        zigzag_points(DrawingKind::XabcdPattern),
        "{}",
    );
    let before = chart.drawing(id).unwrap().points.clone();
    chart.set_selected_drawing(Some(id));
    let (bx, by) = anchor(&chart, id, 2);
    assert!(chart.drawing_drag_start_at(bx, by));
    chart.drawing_drag_to(bx + 20.0, by + 10.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    let after = chart.drawing(id).unwrap().points.clone();
    for index in [0, 1, 3, 4] {
        assert_eq!(after[index], before[index], "only anchor 2 moves");
    }
    assert!(close(anchor(&chart, id, 2), (bx + 20.0, by + 10.0), 1e-6));
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);

    // Body drag from the first leg moves every anchor rigidly.
    let (x0, x1) = (anchor(&chart, id, 0), anchor(&chart, id, 1));
    let grab = ((x0.0 + x1.0) / 2.0, (x0.1 + x1.1) / 2.0);
    chart.set_selected_drawing(None);
    assert!(chart.drawing_drag_start_at(grab.0, grab.1));
    chart.drawing_drag_to(grab.0 + 30.0, grab.1 - 12.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    for (index, &point) in before.iter().enumerate() {
        let moved = anchor(&chart, id, index);
        let original = chart
            .drawing_to_px_for(0, DrawingPriceScale::Right, point)
            .unwrap();
        assert!(close(moved, (original.0 + 30.0, original.1 - 12.0), 1e-6));
    }
    assert!(chart.undo_drawing());

    // Keyboard: every anchor is a handle; a nudge moves one handle.
    assert_eq!(chart.drawing_handle_count(id), Some(5));
    chart.set_selected_drawing(Some(id));
    let (x3, y3) = anchor(&chart, id, 3);
    assert!(chart.nudge_selected_drawing(10.0, 0.0, Some(3)));
    let (nx, ny) = anchor(&chart, id, 3);
    assert!((nx - x3 - 10.0).abs() < 1e-6 && (ny - y3).abs() < 1e-6);
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);

    // Locked drawings stay selectable but do not drag; hidden ones neither paint nor hit.
    assert!(chart.set_drawing_locked(id, true));
    let (lx, ly) = anchor(&chart, id, 1);
    if chart.drawing_drag_start_at(lx, ly) {
        chart.drawing_drag_to(lx + 25.0, ly, DrawingModifiers::default());
        chart.drawing_drag_end();
    }
    assert_eq!(chart.drawing(id).unwrap().points, before);
    assert!(chart.set_drawing_visibility(id, false));
    chart.set_selected_drawing(None);
    assert!(texts_of(&mut chart).is_empty());
    assert_eq!(chart.hit_test_drawing(grab.0, grab.1), None);
}

#[test]
fn z_order_decides_which_overlapping_pattern_hits() {
    let mut chart = chart();
    let points = zigzag_points(DrawingKind::AbcdPattern);
    let below = add(&mut chart, DrawingKind::AbcdPattern, points.clone(), "{}");
    let above = add(&mut chart, DrawingKind::ElliottCorrectionWave, points, "{}");
    let (a, b) = (anchor(&chart, below, 0), anchor(&chart, below, 1));
    let leg = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    assert_eq!(
        chart.hit_test_drawing(leg.0, leg.1).map(|hit| hit.id),
        Some(above)
    );
    assert!(chart.move_drawing_z_order(above, -1));
    assert_eq!(
        chart.hit_test_drawing(leg.0, leg.1).map(|hit| hit.id),
        Some(below)
    );
    assert!(chart.undo_drawing());
    assert_eq!(
        chart.hit_test_drawing(leg.0, leg.1).map(|hit| hit.id),
        Some(above)
    );
}

#[test]
fn the_chart_magnet_snaps_every_placed_anchor() {
    let mut chart = chart();
    chart.set_drawing_magnet_mode(DrawingMagnetMode::Strong);
    assert!(chart.set_drawing_tool(Some(DrawingKind::ElliottCorrectionWave), None, None));
    let mut created = None;
    for (index, logical) in [8.0, 12.0, 16.0, 20.0].into_iter().enumerate() {
        let x = chart.logical_to_coordinate(logical).unwrap() + 3.0;
        let y = chart
            .series_price_to_coordinate(0, 102.4 + index as f64 * 0.3)
            .unwrap();
        created = chart
            .drawing_tool_activate(x, y, DrawingModifiers::default())
            .created;
    }
    let drawing = chart.drawing(created.unwrap()).unwrap();
    for (point, logical) in drawing.points.iter().zip([8.0, 12.0, 16.0, 20.0]) {
        assert_eq!(point.logical, logical);
        assert_eq!(point.price, 100.0 + (logical as usize % 7) as f64);
    }
}

#[test]
fn anchors_resolve_by_time_across_an_interval_switch() {
    let mut chart = chart();
    let anchors =
        [(10.0, 101.0), (14.5, 104.0), (18.0, 102.0), (22.0, 105.0)].map(|(hours, price)| {
            DrawingAnchor {
                logical: None,
                price,
                time: Some(hours * HOUR),
            }
        });
    let id = chart
        .add_drawing_anchors(DrawingKind::AbcdPattern, 0, &anchors, None)
        .unwrap();
    // A cycle anchored in the future area beyond the last bar keeps its times too.
    let future = add(
        &mut chart,
        DrawingKind::CyclicLines,
        vec![p(45.0, 101.0), p(50.0, 101.0)],
        "{}",
    );
    let half_hourly = (0..80)
        .map(|index| index as f64 * HOUR / 2.0)
        .collect::<Vec<_>>();
    let values = vec![100.0; half_hourly.len()];
    chart
        .set_series_data(0, &half_hourly, &values, &values, &values, &values)
        .unwrap();
    let resolved = chart.drawing_anchors(id).unwrap();
    let logicals = resolved
        .iter()
        .map(|anchor| anchor.logical)
        .collect::<Vec<_>>();
    assert_eq!(logicals, [Some(20.0), Some(29.0), Some(36.0), Some(44.0)]);
    assert_eq!(resolved[1].time, Some(14.5 * HOUR));
    let future = chart.drawing_anchors(future).unwrap();
    assert_eq!(
        future
            .iter()
            .map(|anchor| anchor.logical)
            .collect::<Vec<_>>(),
        [Some(90.0), Some(100.0)]
    );
    assert_eq!(future[1].time, Some(50.0 * HOUR));
}

#[test]
fn schema_kind_options_and_tool_option_patches_are_typed_and_atomic() {
    let default_of = |kind: DrawingKind, name: &str| {
        crate::drawing_property_schema(kind)
            .properties
            .into_iter()
            .find(|property| property.name == name)
            .map(|property| (property.default, property.enum_values))
    };
    assert_eq!(
        default_of(DrawingKind::XabcdPattern, "color").unwrap().0,
        serde_json::json!("#2962FF")
    );
    assert_eq!(
        default_of(DrawingKind::XabcdPattern, "fill_enabled")
            .unwrap()
            .0,
        serde_json::json!(true)
    );
    assert_eq!(
        default_of(
            DrawingKind::XabcdPattern,
            "tool_options.pattern.show_ratios"
        )
        .unwrap()
        .0,
        serde_json::json!(true)
    );
    let (degree, degrees) = default_of(
        DrawingKind::ElliottImpulseWave,
        "tool_options.pattern.degree",
    )
    .unwrap();
    assert_eq!(degree, serde_json::json!("intermediate"));
    assert_eq!(degrees.len(), 12);
    assert_eq!(degrees[0], "supermillennium");
    assert_eq!(degrees[11], "subminuette");
    assert_eq!(
        default_of(
            DrawingKind::ElliottTripleCombo,
            "tool_options.pattern.show_wave"
        )
        .unwrap()
        .0,
        serde_json::json!(true)
    );
    for kind in [
        DrawingKind::HeadAndShoulders,
        DrawingKind::TrianglePattern,
        DrawingKind::CyclicLines,
        DrawingKind::TimeCycles,
        DrawingKind::SineLine,
    ] {
        assert!(
            !crate::drawing_property_schema(kind)
                .properties
                .iter()
                .any(|property| property.name.starts_with("tool_options")),
            "{kind:?} has no family options"
        );
    }
    assert!(default_of(
        DrawingKind::ElliottImpulseWave,
        "tool_options.pattern.show_ratios"
    )
    .is_none());

    let mut chart = chart();
    let kind_options = |chart: &ChartEngine, id| {
        serde_json::from_str::<serde_json::Value>(&chart.drawing_kind_options_json(id).unwrap())
            .unwrap()
    };
    let xabcd = add(
        &mut chart,
        DrawingKind::XabcdPattern,
        zigzag_points(DrawingKind::XabcdPattern),
        "{}",
    );
    let wave = add(
        &mut chart,
        DrawingKind::ElliottImpulseWave,
        zigzag_points(DrawingKind::ElliottImpulseWave),
        "{}",
    );
    let sine = add(
        &mut chart,
        DrawingKind::SineLine,
        zigzag_points(DrawingKind::SineLine),
        "{}",
    );
    assert_eq!(
        kind_options(&chart, xabcd),
        serde_json::json!({"kind": "pattern", "show_ratios": true})
    );
    assert_eq!(
        kind_options(&chart, wave),
        serde_json::json!({"kind": "elliott_wave", "degree": "intermediate", "show_wave": true})
    );
    assert_eq!(
        kind_options(&chart, sine),
        serde_json::json!({"kind": "generic"})
    );

    assert!(chart.drawing_apply_options(
        wave,
        r#"{"tool_options":{"pattern":{"degree":"minute"}},"width":3}"#
    ));
    assert_eq!(
        kind_options(&chart, wave),
        serde_json::json!({"kind": "elliott_wave", "degree": "minute", "show_wave": true})
    );
    let before = chart.drawing(wave).unwrap().clone();
    for invalid in [
        r#"{"tool_options":{"pattern":{"degree":"tiny"}},"width":9}"#,
        r#"{"tool_options":{"pattern":{"show_wave":"yes"}},"width":9}"#,
        r#"{"tool_options":{"pattern":4}}"#,
    ] {
        assert!(!chart.drawing_apply_options(wave, invalid), "{invalid}");
        assert_eq!(chart.drawing(wave).unwrap(), &before);
    }
    assert!(chart.drawing_apply_options(wave, r#"{"tool_options":{}}"#));
    assert_eq!(chart.drawing(wave).unwrap(), &before);
    assert!(chart.drawing_apply_options(wave, r#"{"tool_options":{"pattern":null}}"#));
    assert!(chart.drawing(wave).unwrap().tool_options.is_empty());
    assert!(chart.undo_drawing());
    assert_eq!(
        chart.drawing(wave).unwrap().tool_options,
        before.tool_options
    );
    let options: serde_json::Value =
        serde_json::from_str(&chart.drawing_options_json(wave).unwrap()).unwrap();
    assert_eq!(
        options["tool_options"],
        serde_json::json!({"pattern": {"show_ratios": true, "degree": "minute", "show_wave": true}})
    );
}

#[test]
fn persistence_round_trips_family_tools_and_omits_kind_defaults() {
    let mut chart = chart();
    for kind in KINDS {
        add(&mut chart, kind, zigzag_points(kind), "{}");
    }
    let exported: serde_json::Value =
        serde_json::from_str(&chart.export_state_json().unwrap()).unwrap();
    for drawing in exported["drawings"].as_array().unwrap() {
        let style = &drawing["style"];
        for field in ["fill_enabled", "tool_options", "labels", "extend_right"] {
            assert!(
                style.get(field).is_none(),
                "{} writes its default {field}",
                drawing["kind"]
            );
        }
    }
    let customized = [
        (
            DrawingKind::XabcdPattern,
            r#"{"fill_enabled":false,"tool_options":{"pattern":{"show_ratios":false}}}"#,
        ),
        (
            DrawingKind::ElliottTriangleWave,
            r#"{"tool_options":{"pattern":{"degree":"grand_supercycle","show_wave":false}}}"#,
        ),
        (
            DrawingKind::AbcdPattern,
            r#"{"fill_enabled":true,"style":"dashed"}"#,
        ),
        (DrawingKind::TimeCycles, r##"{"fill_color":"#ff000033"}"##),
        (DrawingKind::CyclicLines, r#"{"width":3,"text":"cycle"}"#),
    ];
    for (kind, options) in customized {
        add(&mut chart, kind, zigzag_points(kind), options);
    }
    let document = chart.export_state_json().unwrap();
    let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
    restored.import_state_json(&document).unwrap();
    assert_eq!(restored.export_state_json().unwrap(), document);
    for (restored, original) in restored.drawings().iter().zip(chart.drawings()) {
        assert_eq!(restored.kind, original.kind);
        assert_eq!(restored.points, original.points);
        assert_eq!(restored.color, original.color);
        assert_eq!(restored.fill_enabled, original.fill_enabled);
        assert_eq!(restored.fill_color, original.fill_color);
        assert_eq!(restored.style, original.style);
        assert_eq!(restored.tool_options, original.tool_options);
    }

    // A document written without style fields (an older or hand-written layout) restores each
    // tool with its own kind defaults.
    let anchors = |count: usize| {
        (0..count)
            .map(|index| {
                serde_json::json!({
                    "logical": 6.0 + index as f64 * 2.0,
                    "price": if index % 2 == 1 { 104.0 } else { 101.0 },
                })
            })
            .collect::<Vec<_>>()
    };
    let document = serde_json::json!({
        "schema": "aeris_charts-state",
        "schema_version": 1,
        "panes": [{"id": "pane-1", "stretch_factor": 1.0, "preserve_empty": false}],
        "drawings": [
            {"id": 1, "kind": "xabcd_pattern", "pane_id": "pane-1", "anchors": anchors(5)},
            {"id": 2, "kind": "elliott_triple_combo", "pane_id": "pane-1", "anchors": anchors(6)},
            {"id": 3, "kind": "time_cycles", "pane_id": "pane-1", "anchors": anchors(2)},
        ],
    });
    let mut restored = chart_with(&hourly(40), 1.0);
    restored.import_state_json(&document.to_string()).unwrap();
    let defaults = restored
        .drawings()
        .iter()
        .map(|drawing| (drawing.color.as_str(), drawing.fill_enabled, drawing.width))
        .collect::<Vec<_>>();
    assert_eq!(
        defaults,
        [
            ("#2962FF", true, 2.0),
            ("#6AA84F", false, 2.0),
            ("#159980", true, 2.0)
        ]
    );
    assert!(restored
        .drawings()
        .iter()
        .all(|drawing| drawing.tool_options.is_empty()));
    let waves = texts_of(&mut restored)
        .into_iter()
        .filter(|text| text.starts_with('('))
        .collect::<Vec<_>>();
    assert_eq!(waves, ["(W)", "(X)", "(Y)", "(X)", "(Z)"]);
}

#[test]
fn templates_and_armed_tools_carry_family_options() {
    let mut chart = chart();
    let points = zigzag_points(DrawingKind::ElliottImpulseWave);
    let source = add(
        &mut chart,
        DrawingKind::ElliottImpulseWave,
        points.clone(),
        r##"{"color":"#123456","tool_options":{"pattern":{"degree":"minute","show_wave":false}}}"##,
    );
    let template = chart.drawing_template_json(source, "minute waves").unwrap();
    let target = add(&mut chart, DrawingKind::ElliottImpulseWave, points, "{}");
    assert!(chart.apply_drawing_template_json(target, &template));
    assert_eq!(
        chart.drawing(target).unwrap().tool_options,
        chart.drawing(source).unwrap().tool_options
    );
    assert_eq!(chart.drawing(target).unwrap().color, INK);
    // A template applies only to its own kind.
    let pattern = add(
        &mut chart,
        DrawingKind::XabcdPattern,
        zigzag_points(DrawingKind::XabcdPattern),
        "{}",
    );
    let before = chart.drawing(pattern).unwrap().clone();
    assert!(!chart.apply_drawing_template_json(pattern, &template));
    assert_eq!(chart.drawing(pattern).unwrap(), &before);
    chart.clear_drawings();

    // Arming a tool with family options places drawings that carry them.
    assert!(chart.set_drawing_tool(
        Some(DrawingKind::ElliottCorrectionWave),
        Some(r##"{"color":"#123456","tool_options":{"pattern":{"degree":"primary"}}}"##),
        None
    ));
    let mut created = None;
    for index in 0..4 {
        let (x, y) = (
            150.0 + index as f64 * 80.0,
            300.0 - (index % 2) as f64 * 120.0,
        );
        created = chart
            .drawing_tool_activate(x, y, DrawingModifiers::default())
            .created;
    }
    let drawing = chart.drawing(created.unwrap()).unwrap();
    assert_eq!(
        drawing.tool_options.pattern.map(|options| options.degree),
        Some(ElliottWaveDegree::Primary)
    );
    assert_eq!(texts_of(&mut chart), ["A", "B", "C"]);
    let rings = ink_polylines(&mut chart)
        .into_iter()
        .filter(|(points, width, _)| *width == 1.0 && points.len() > 8)
        .count();
    assert_eq!(rings, 3, "primary degree rings every label");
}

#[test]
fn clipboard_and_sync_payloads_carry_family_options() {
    let mut source = chart();
    let id = add(
        &mut source,
        DrawingKind::ElliottDoubleCombo,
        zigzag_points(DrawingKind::ElliottDoubleCombo),
        r#"{"tool_options":{"pattern":{"degree":"cycle"}},"fill_enabled":true}"#,
    );
    let copied = source.copy_drawings_json(&[id]).unwrap();
    let mut target = chart();
    let pasted = target.paste_drawings_json(&copied, 0, 0.0, 0.0).unwrap();
    let drawing = target.drawing(pasted[0]).unwrap();
    assert_eq!(drawing.kind, DrawingKind::ElliottDoubleCombo);
    assert!(drawing.fill_enabled);
    assert_eq!(
        drawing.tool_options,
        source.drawing(id).unwrap().tool_options
    );
    assert_eq!(texts_of(&mut target), ["w", "x", "y"]);

    let payload = source.drawing_sync_payload_json("cell-a").unwrap();
    let mut mirror = chart();
    assert!(mirror.apply_drawing_sync_payload_json(&payload));
    assert_eq!(
        mirror.drawings()[0].tool_options,
        source.drawing(id).unwrap().tool_options
    );
}

#[test]
fn frames_scale_family_geometry_with_the_device_pixel_ratio() {
    // 1.5 makes the horizontal and vertical bitmap ratios differ (odd pane sizes round apart).
    for dpr in [1.0, 1.5, 2.0] {
        let mut chart = chart_with(&hourly(40), dpr);
        let id = add(
            &mut chart,
            DrawingKind::AbcdPattern,
            vec![p(6.0, 105.0), p(10.0, 101.0), p(14.0, 103.5), p(18.0, 99.5)],
            r##"{"color":"#123456"}"##,
        );
        let hpr = (chart.pane_w * dpr).round() / chart.pane_w;
        let vpr = (chart.pane_h * dpr).round() / chart.pane_h;
        let zigzag = ink_polylines(&mut chart).remove(0);
        assert!((f64::from(zigzag.1) - 2.0 * vpr).abs() < 1e-5);
        for (index, point) in zigzag.0.iter().enumerate() {
            let a = anchor(&chart, id, index);
            assert!(close(*point, (a.0 * hpr, a.1 * vpr), 1e-3), "dpr {dpr}");
        }
        let b = anchor(&chart, id, 1);
        let y = text_y(&mut chart, "B");
        assert!(
            y > b.1 * vpr + 6.0 * vpr,
            "the low's label sits below it at dpr {dpr}"
        );

        let sine = add(
            &mut chart,
            DrawingKind::SineLine,
            vec![p(20.0, 105.0), p(24.0, 101.0)],
            r##"{"color":"#123456"}"##,
        );
        let peak = anchor(&chart, sine, 0);
        let wave = ink_polylines(&mut chart)
            .into_iter()
            .find(|(points, ..)| points.len() > 4)
            .unwrap()
            .0;
        assert!(wave
            .iter()
            .any(|&point| close(point, (peak.0 * hpr, peak.1 * vpr), 1e-3)));
    }
}

#[test]
fn family_tools_paint_and_hit_on_a_lower_pane() {
    let mut chart = chart();
    let pane = chart.add_pane(true).unwrap();
    let series = chart.add_series(crate::SeriesKind::Line);
    let values = (0..40)
        .map(|index| 10.0 + (index % 5) as f64 * 0.5)
        .collect::<Vec<_>>();
    chart
        .set_series_data(series, &hourly(40), &values, &values, &values, &values)
        .unwrap();
    chart.set_series_pane(series, pane, 1.0);
    chart.build_frame();
    let (top, bottom) = (
        chart.panes[pane].top,
        chart.panes[pane].top + chart.panes[pane].height,
    );
    let points = vec![
        p(6.0, 10.2),
        p(10.0, 11.8),
        p(14.0, 10.6),
        p(18.0, 11.5),
        p(22.0, 10.4),
    ];
    let pattern = chart
        .add_drawing(
            DrawingKind::XabcdPattern,
            pane,
            points,
            Some(r##"{"color":"#123456"}"##),
        )
        .unwrap();
    let cycles = chart
        .add_drawing(
            DrawingKind::CyclicLines,
            pane,
            vec![p(9.0, 11.0), p(13.0, 11.0)],
            Some(r##"{"color":"#123456"}"##),
        )
        .unwrap();
    let frame = chart.build_frame();
    // Point labels and repeats paint on the drawing's own pane, and the repeats span only it.
    let label_y = frame.panes[pane]
        .main
        .iter()
        .find_map(|prim| match prim {
            Prim::Text { text, y, .. } if text == "A" => Some(f64::from(*y)),
            _ => None,
        })
        .expect("the A label on the lower pane");
    assert!(label_y > top && label_y < bottom);
    assert!(!frame.panes[0]
        .main
        .iter()
        .any(|prim| matches!(prim, Prim::Text { text, .. } if text == "A")));
    let repeats = frame.panes[pane]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::VLine { y0, y1, color, .. } if *color == ink() => Some((*y0, *y1)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(repeats.len() > 5);
    assert!(repeats
        .iter()
        .all(|&(y0, y1)| y0 == top.round() as i32 && y1 == bottom.round() as i32));
    // Legs, labels, and far repeats hit on that pane.
    let (a, b) = (anchor(&chart, pattern, 0), anchor(&chart, pattern, 1));
    assert!(a.1 > top && b.1 > top);
    let leg = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    assert_eq!(
        chart.hit_test_drawing(leg.0, leg.1).map(|hit| hit.id),
        Some(pattern)
    );
    let x = chart.logical_to_coordinate(33.0).unwrap();
    assert_eq!(
        chart
            .hit_test_drawing(x, (top + bottom) / 2.0)
            .map(|hit| hit.id),
        Some(cycles)
    );
    assert_eq!(chart.hit_test_drawing(x, top / 2.0), None);
}

#[test]
fn family_tools_tolerate_charts_without_data_and_degenerate_anchors() {
    let mut empty = ChartEngine::new(800.0, 500.0, 1.0);
    for kind in KINDS {
        assert!(empty
            .add_drawing(kind, 0, zigzag_points(kind), None)
            .is_some());
    }
    empty.build_frame();
    assert_eq!(empty.hit_test_drawing(100.0, 100.0), None);

    let mut chart = chart();
    assert!(chart
        .add_drawing(
            DrawingKind::SineLine,
            0,
            vec![p(f64::NAN, 1.0), p(2.0, 3.0)],
            None
        )
        .is_none());
    assert!(
        chart
            .add_drawing(
                DrawingKind::XabcdPattern,
                0,
                zigzag_points(DrawingKind::AbcdPattern),
                None
            )
            .is_none(),
        "a pattern needs its exact anchor count"
    );
    for kind in KINDS {
        let points = vec![p(12.0, 102.0); kind.anchor_count()];
        assert!(chart.add_drawing(kind, 0, points, None).is_some());
    }
    let frame = chart.build_frame();
    assert!(frame.panes[0]
        .points
        .iter()
        .all(|point| point[0].is_finite() && point[1].is_finite()));
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
fn labels_and_triangle_apexes_keep_culled_drawings_visible() {
    let mut chart = chart();
    crowd(&mut chart);
    // The head's label reaches above the anchors' box.
    let head = add(
        &mut chart,
        DrawingKind::HeadAndShoulders,
        vec![
            p(4.0, 101.0),
            p(8.0, 104.0),
            p(12.0, 102.0),
            p(16.0, 106.0),
            p(20.0, 102.0),
            p(24.0, 104.0),
            p(28.0, 101.0),
        ],
        "{}",
    );
    chart.build_frame();
    let label_y = text_y(&mut chart, "Head");
    let head_x = anchor(&chart, head, 3).0;
    assert_eq!(
        chart.hit_test_drawing(head_x, label_y).map(|hit| hit.id),
        Some(head)
    );
    // A triangle whose anchors sit left of the pane still paints its apex region inside it.
    let triangle = add(
        &mut chart,
        DrawingKind::TrianglePattern,
        vec![
            p(-14.0, 106.0),
            p(-11.0, 100.0),
            p(-4.0, 104.0),
            p(-1.0, 102.0),
        ],
        "{}",
    );
    chart.build_frame();
    assert!(viewport_candidate(&chart, triangle));
    let apex = px(&chart, 2.5, 102.7);
    assert!(apex.0 > 0.0);
    let a = anchor(&chart, triangle, 0);
    let near_apex = ((a.0 + 9.0 * apex.0) / 10.0, (a.1 + 9.0 * apex.1) / 10.0);
    assert!(near_apex.0 > 0.0);
    assert_eq!(
        chart
            .hit_test_drawing(near_apex.0, near_apex.1)
            .map(|hit| hit.id),
        Some(triangle)
    );
}

#[test]
fn wide_ratio_labels_keep_their_drawing_visible_past_the_anchors() {
    let mut chart = chart();
    crowd(&mut chart);
    // A near-flat AB leg makes BC/AB a ten-digit ratio. Its A-C connector is vertical on the
    // anchors' left edge, so half of that label reaches left of every anchor.
    let id = add(
        &mut chart,
        DrawingKind::AbcdPattern,
        vec![
            p(20.0, 100.0),
            p(24.0, 100.0 + 1e-9),
            p(20.0, 104.0),
            p(24.0, 101.0),
        ],
        r##"{"color":"#123456"}"##,
    );
    // The anchors sit two bars past the right edge; only the label reaches into the pane.
    chart.set_visible_logical_range(-21.5, 17.5);
    chart.build_frame();
    let a = anchor(&chart, id, 0);
    assert!(a.0 > chart.pane_w + 30.0, "{a:?}");
    let (text, left, y) = texts(&mut chart)
        .into_iter()
        .find(|(text, ..)| text.parse::<f64>().is_ok_and(|value| value > 1e9))
        .expect("the ten-digit ratio is painted");
    assert!(left < chart.pane_w - 5.0, "{text} starts inside the pane");
    assert!(viewport_candidate(&chart, id));
    assert_eq!(
        chart
            .hit_test_drawing(chart.pane_w - 3.0, y)
            .map(|hit| hit.id),
        Some(id),
        "the visible part of the label is a body target"
    );
}

/// The drawing's parts in media px (the hit-test space).
fn media_parts(chart: &ChartEngine, id: DrawingId) -> DrawingParts {
    let drawing = chart.drawing(id).unwrap();
    let px = chart.drawing_px(drawing).unwrap();
    let mut parts = DrawingParts::default();
    (super::FAMILY.build_parts)(
        &PartContext::media(chart, drawing, &px).unwrap(),
        &mut parts,
    );
    parts
}

#[test]
fn decoration_extent_bounds_every_label_beyond_the_anchors() {
    // Culling pads the anchors' box by the family's decoration extent, so every label box and
    // ring must stay inside that padded box, at any text size, degree, and ratio width.
    let mut chart = chart();
    let bounded = [
        DrawingKind::XabcdPattern,
        DrawingKind::CypherPattern,
        DrawingKind::AbcdPattern,
        DrawingKind::HeadAndShoulders,
        DrawingKind::ThreeDrivesPattern,
        DrawingKind::ElliottImpulseWave,
        DrawingKind::ElliottCorrectionWave,
        DrawingKind::ElliottTriangleWave,
        DrawingKind::ElliottDoubleCombo,
        DrawingKind::ElliottTripleCombo,
    ];
    let family = chart.options.get().layout.font_family.clone();
    for options in [
        "{}",
        r#"{"text_size":28,"tool_options":{"pattern":{"degree":"primary"}}}"#,
        r#"{"text_size":40,"text_weight":700,"tool_options":{"pattern":{"degree":"supermillennium"}}}"#,
        r#"{"text_size":9,"tool_options":{"pattern":{"degree":"minute"}}}"#,
    ] {
        for kind in bounded {
            for flat in [false, true] {
                let mut points = zigzag_points(kind);
                if flat {
                    // A near-flat second leg prints ten-digit ratios.
                    points[2].price = points[1].price + 1e-9;
                }
                let id = add(&mut chart, kind, points, options);
                let drawing = chart.drawing(id).unwrap();
                let extent = (super::FAMILY.decoration_extent)(&chart, drawing);
                let px = chart.drawing_px(drawing).unwrap();
                let (left, top, right, bottom) = px.iter().fold(
                    (
                        f64::INFINITY,
                        f64::INFINITY,
                        f64::NEG_INFINITY,
                        f64::NEG_INFINITY,
                    ),
                    |(l, t, r, b), &(x, y)| (l.min(x), t.min(y), r.max(x), b.max(y)),
                );
                let inside = |(x, y): (f64, f64)| {
                    x >= left - extent - 1e-6
                        && x <= right + extent + 1e-6
                        && y >= top - extent - 1e-6
                        && y <= bottom + extent + 1e-6
                };
                let parts = media_parts(&chart, id);
                for label in &parts.labels {
                    let rect = label
                        .layout(|line| {
                            chart.measure_text_run(
                                line,
                                label.size,
                                &family,
                                label.weight,
                                label.italic,
                            )
                        })
                        .rect;
                    assert!(
                        inside((rect.left, rect.top)) && inside((rect.right, rect.bottom)),
                        "{kind:?} {options} flat {flat}: {:?} {rect:?} beyond {extent} px",
                        label.lines
                    );
                }
                assert!(
                    parts.points.iter().all(|&point| inside(point)),
                    "{kind:?} {options}: a ring or stroke point beyond {extent} px"
                );
                chart.remove_drawing(id);
            }
        }
    }
}

/// Whether segment `a → b` meets the rect `(left, top, right, bottom)` (Liang–Barsky).
fn segment_meets(a: (f64, f64), b: (f64, f64), rect: (f64, f64, f64, f64)) -> bool {
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    for (p, q) in [
        (-dx, a.0 - rect.0),
        (dx, rect.2 - a.0),
        (-dy, a.1 - rect.1),
        (dy, rect.3 - a.1),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return false;
            }
        } else if p < 0.0 {
            t0 = t0.max(q / p);
        } else {
            t1 = t1.min(q / p);
        }
    }
    t0 <= t1
}

/// Whether any part the family resolves for `id` meets the media-px `rect`.
fn paints_inside(chart: &ChartEngine, id: DrawingId, rect: (f64, f64, f64, f64)) -> bool {
    let parts = media_parts(chart, id);
    let family = chart.options.get().layout.font_family.clone();
    let boxes_meet = |(left, top, right, bottom): (f64, f64, f64, f64)| {
        left <= rect.2 && right >= rect.0 && top <= rect.3 && bottom >= rect.1
    };
    parts.items.iter().any(|part| match *part {
        DrawingPart::Stroke { start, end, .. } | DrawingPart::Tube { start, end, .. } => parts
            .points[start..end]
            .windows(2)
            .any(|pair| segment_meets(pair[0], pair[1], rect)),
        DrawingPart::HLine { y, x0, x1, .. } => segment_meets((x0, y), (x1, y), rect),
        DrawingPart::VLine { x, y0, y1, .. } => segment_meets((x, y0), (x, y1), rect),
        DrawingPart::Fill {
            upper,
            lower,
            count,
            ..
        } => boxes_meet(
            parts.points[upper..upper + count]
                .iter()
                .chain(&parts.points[lower..lower + count])
                .fold(
                    (
                        f64::INFINITY,
                        f64::INFINITY,
                        f64::NEG_INFINITY,
                        f64::NEG_INFINITY,
                    ),
                    |(l, t, r, b), &(x, y)| (l.min(x), t.min(y), r.max(x), b.max(y)),
                ),
        ),
        DrawingPart::Disc { center, radius, .. } => boxes_meet((
            center.0 - radius,
            center.1 - radius,
            center.0 + radius,
            center.1 + radius,
        )),
        DrawingPart::Label { index } => {
            let label = &parts.labels[index];
            let bounds = label
                .layout(|line| {
                    chart.measure_text_run(line, label.size, &family, label.weight, label.italic)
                })
                .rect;
            boxes_meet((bounds.left, bounds.top, bounds.right, bounds.bottom))
        }
    })
}

/// Every drawing in `ids` whose parts meet the pane must survive culling.
fn assert_painted_drawings_are_candidates(chart: &mut ChartEngine, ids: &[DrawingId], note: &str) {
    chart.build_frame();
    let (top, height) = (chart.panes[0].top, chart.panes[0].height);
    let pane = (0.5, top + 0.5, chart.pane_w - 0.5, top + height - 0.5);
    for &id in ids {
        if paints_inside(chart, id, pane) {
            assert!(
                viewport_candidate(chart, id),
                "{note}: {:?} paints inside the pane but is culled",
                chart.drawing(id).unwrap().kind
            );
        }
    }
}

#[test]
fn culling_keeps_every_drawing_whose_parts_reach_the_pane() {
    let mut chart = chart();
    crowd(&mut chart);
    let mut ids = KINDS
        .into_iter()
        .map(|kind| add(&mut chart, kind, zigzag_points(kind), "{}"))
        .collect::<Vec<_>>();
    for options in [
        r#"{"text_size":30,"tool_options":{"pattern":{"degree":"grand_supercycle"}}}"#,
        r#"{"text_size":30,"tool_options":{"pattern":{"degree":"supermillennium"}}}"#,
    ] {
        ids.push(add(
            &mut chart,
            DrawingKind::ElliottImpulseWave,
            zigzag_points(DrawingKind::ElliottImpulseWave),
            options,
        ));
    }
    ids.push(add(
        &mut chart,
        DrawingKind::HeadAndShoulders,
        zigzag_points(DrawingKind::HeadAndShoulders),
        r#"{"text_size":40}"#,
    ));
    // Scroll every drawing across both pane edges.
    for position in -80..=80 {
        chart.scroll_to_position(f64::from(position));
        assert_painted_drawings_are_candidates(&mut chart, &ids, &format!("position {position}"));
    }
    chart.clear_drawings();
    crowd(&mut chart);

    // Move a squashed copy of every tool past the pane's top and bottom edges, where only the
    // labels of its highs or lows still reach in.
    chart.fit_content();
    chart.build_frame();
    let price_at = |chart: &ChartEngine, y: f64| {
        chart
            .drawing_from_px_for(0, DrawingPriceScale::Right, 10.0, y)
            .unwrap()
            .price
    };
    let (bottom, top) = (price_at(&chart, 500.0), price_at(&chart, 0.0));
    let per_px = (top - bottom) / 500.0;
    for shift in (-120..=120).step_by(2) {
        let base = if shift < 0 { bottom } else { top };
        let ids = KINDS
            .into_iter()
            .map(|kind| {
                let points = zigzag_points(kind)
                    .into_iter()
                    .map(|point| {
                        let offset = (point.price - 101.0) * 5.0 + f64::from(shift);
                        p(point.logical, base + offset * per_px)
                    })
                    .collect();
                add(
                    &mut chart,
                    kind,
                    points,
                    r#"{"text_size":28,"tool_options":{"pattern":{"degree":"primary"}}}"#,
                )
            })
            .collect::<Vec<_>>();
        assert_painted_drawings_are_candidates(&mut chart, &ids, &format!("shift {shift}"));
        for id in ids {
            chart.remove_drawing(id);
        }
    }
}

#[test]
fn log_and_percentage_scales_keep_indexed_hits_equal_to_brute_force() {
    use crate::PriceScaleMode;
    for mode in [
        PriceScaleMode::Logarithmic,
        PriceScaleMode::Percentage,
        PriceScaleMode::IndexedTo100,
    ] {
        let mut chart = chart();
        for kind in KINDS {
            add(&mut chart, kind, zigzag_points(kind), "{}");
            // Non-positive prices clamp on a log scale instead of producing non-finite geometry.
            let points = (0..kind.anchor_count())
                .map(|index| p(20.0 + index as f64 * 2.0, -(index as f64)))
                .collect();
            add(&mut chart, kind, points, "{}");
        }
        chart.set_price_scale_mode(0, false, mode);
        let frame = chart.build_frame();
        assert!(frame.panes[0]
            .points
            .iter()
            .all(|point| point[0].is_finite() && point[1].is_finite()));
        for gy in 0..16 {
            for gx in 0..26 {
                let (x, y) = (f64::from(gx) * 30.0 + 3.0, f64::from(gy) * 30.0 + 4.0);
                assert_eq!(
                    chart.hit_test_drawing(x, y),
                    chart.hit_test_drawing_bruteforce(x, y),
                    "{mode:?} ({x}, {y})"
                );
            }
        }
    }
}

#[test]
fn extreme_zoom_keeps_frame_geometry_finite_and_bounded() {
    for spacing in [0.5, 50.0, 5_000.0, 500_000.0] {
        let mut chart = chart();
        chart.set_bar_spacing(spacing);
        for kind in KINDS {
            add(&mut chart, kind, zigzag_points(kind), "{}");
        }
        // Arches and a wave a billion price units tall.
        add(
            &mut chart,
            DrawingKind::TimeCycles,
            vec![p(10.0, 101.0), p(11.0, 1.0e9)],
            "{}",
        );
        add(
            &mut chart,
            DrawingKind::SineLine,
            vec![p(10.0, 101.0), p(10.5, 1.0e9)],
            "{}",
        );
        let frame = chart.build_frame();
        let points = &frame.panes[0].points;
        assert!(points
            .iter()
            .all(|point| point[0].is_finite() && point[1].is_finite()));
        assert!(
            points.len() < 8 * MAX_CURVE_POINTS,
            "{} points at bar spacing {spacing}",
            points.len()
        );
    }
}
