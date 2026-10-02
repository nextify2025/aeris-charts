//! Golden-image regression test (roadmap Phase D1).
//!
//! Re-renders the reference scene and diffs it against the committed golden PNG. This protects
//! the render path (executor + rasterizer) against regressions: any change that alters the output
//! fails here until the golden is deliberately regenerated (`cargo run -p aeris_charts_native --example
//! scene -- crates/aeris_charts_native/tests/goldens/scene.png`).
//!
//! The golden is currently our own deterministic render of geometry (no text). When a
//! headless-Chromium comparison pipeline exists, independently captured public-library output can
//! be evaluated as additional goldens with the same diff.

use aeris_charts_engine::{ChartEngine, SeriesKind};
use aeris_charts_native::{
    diff_pixmaps,
    engine_scene::{demo_engine, parity_engine},
    load_png, render_engine, render_prims,
    scene::demo_scene,
    TinySkiaCanvas,
};
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{LineStyle, LineType, Prim};

const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/goldens/scene.png");

#[test]
fn scene_matches_golden() {
    let s = demo_scene();
    let canvas = render_prims(s.width, s.height, s.background, &s.prims, &s.points);
    let golden = load_png(GOLDEN).expect("committed golden PNG should load");

    // Same machine + deterministic CPU rasterizer => exact. Allow a hair of per-channel tolerance
    // and a tiny differing-pixel budget so a tiny-skia patch bump doesn't spuriously fail CI.
    let stats = diff_pixmaps(canvas.pixmap(), &golden, 2).expect("golden and render are same size");
    assert!(
        stats.fraction() < 0.001,
        "render drifted from golden: {} / {} px differ (max channel delta {}). \
         If intentional, regenerate the golden.",
        stats.differing_pixels,
        stats.total_pixels,
        stats.max_channel_delta,
    );
}

#[test]
fn diff_detects_a_changed_scene() {
    // Sanity: a modified scene must diff against the golden (guards against a no-op comparison).
    let s = demo_scene();
    let mut prims = s.prims.clone();
    prims.truncate(prims.len().saturating_sub(3)); // drop the marker + price line + polyline
    let changed = render_prims(s.width, s.height, s.background, &prims, &s.points);
    let golden = load_png(GOLDEN).unwrap();
    let stats = diff_pixmaps(changed.pixmap(), &golden, 2).unwrap();
    assert!(
        stats.differing_pixels > 0,
        "a changed scene should differ from the golden"
    );
}

#[test]
fn real_engine_frame_paints_chart_geometry() {
    let mut chart = demo_engine();
    let canvas = render_engine(&mut chart);
    assert_eq!(
        (canvas.pixmap().width(), canvas.pixmap().height()),
        (480, 300)
    );
    let surface = canvas.pixel_rgba(0, 0);
    let painted = canvas
        .pixmap()
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[0..3] != surface[0..3])
        .count();
    assert!(
        painted > 100,
        "expected candle/grid geometry, got {painted} non-surface pixels"
    );
}

#[test]
fn shared_browser_native_fixture_has_expected_pane_bitmap() {
    let mut chart = parity_engine();
    let canvas = render_engine(&mut chart);
    // Fixture geometry is the compact default: (1280 - 46) x (720 - 22) CSS at 1.5 DPR.
    assert_eq!(
        (canvas.pixmap().width(), canvas.pixmap().height()),
        (1851, 1047)
    );
}

/// `bars` one-bar pairs that touch at mid-pixel boundaries and step by about a pixel, so neighbours
/// share anti-aliased pixels the way a slowly moving study line does; plus the bar pitch.
fn touching_pairs(bars: u32) -> (Vec<[f32; 2]>, f32) {
    let pitch = 13.37;
    let mut points = Vec::new();
    for bar in 0..bars {
        let x = 6.0 + bar as f32 * pitch;
        let y = 20.0 + (bar % 4) as f32 * 1.3;
        points.push([x, y]);
        points.push([x + pitch, y]);
    }
    (points, pitch)
}

/// Each pair of a `Segments` batch written out as a separate solid two-point polyline, with pool
/// indices shifted by `base`.
fn separate_polylines(first_point: u32, segment_count: u32, width: f32, color: Color) -> Vec<Prim> {
    (0..segment_count)
        .map(|pair| Prim::Polyline {
            first_point: first_point + 2 * pair,
            point_count: 2,
            width,
            style: LineStyle::Solid,
            line_type: LineType::Simple,
            color,
        })
        .collect()
}

#[test]
fn segments_render_like_two_point_polylines() {
    const BARS: u32 = 12;
    let (points, pitch) = touching_pairs(BARS);
    let color = Color::rgb(0x20, 0x60, 0xc0);
    let white = Color::rgb(0xff, 0xff, 0xff);
    let batched = render_prims(
        180,
        60,
        white,
        &[Prim::Segments {
            first_point: 0,
            segment_count: BARS,
            width: 3.0,
            color,
        }],
        &points,
    );
    let separate = render_prims(
        180,
        60,
        white,
        &separate_polylines(0, BARS, 3.0, color),
        &points,
    );
    // One stroked path unions the coverage of the pairs, while separate strokes composite twice
    // in the column two pairs share. So the batch may differ only there, and only by being
    // darker (never lighter) on this light background.
    let seams: Vec<f32> = (1..BARS).map(|bar| 6.0 + bar as f32 * pitch).collect();
    let (mut differing, mut max_delta) = (0u32, 0u8);
    for y in 0..60 {
        for x in 0..180 {
            let (one, many) = (batched.pixel_rgba(x, y), separate.pixel_rgba(x, y));
            if one == many {
                continue;
            }
            differing += 1;
            assert!(
                seams
                    .iter()
                    .any(|seam| (x as f32 + 0.5 - seam).abs() <= 1.0),
                "pixel ({x}, {y}) differs away from a shared boundary: {one:?} vs {many:?}"
            );
            for channel in 0..3 {
                assert!(
                    one[channel] <= many[channel],
                    "({x}, {y}) {one:?} vs {many:?}"
                );
                max_delta = max_delta.max(many[channel] - one[channel]);
            }
        }
    }
    // Measured on this fixture: 17 px, max channel delta 55. The worst case is a column split half
    // and half between two pairs, where compositing twice leaves 0.75 of the coverage one path
    // gives, so a channel moves by at most a quarter of its range.
    assert!(
        differing > 0,
        "the fixture must exercise the shared-column case"
    );
    assert!(
        differing <= 8 * seams.len() as u32,
        "{differing} seam pixels differ"
    );
    assert!(max_delta <= 64, "max channel delta {max_delta}");
}

/// A daily chart with a line series in pane 0 and session VWAP in pane 1. Every bar is its own
/// period, so pane 1's study is one `Segments` batch, and its pool indices start at zero in pane
/// 1's own pool while pane 0's line already holds points in the image export's concatenated pool.
fn two_pane_daily_study() -> ChartEngine {
    let mut chart = ChartEngine::new(480.0, 300.0, 1.0);
    let times: Vec<f64> = (0..40)
        .map(|day| (1_704_067_200 + day * 86_400) as f64)
        .collect();
    let close: Vec<f64> = (0..40)
        .map(|day| 100.0 + ((day * 7) % 11) as f64 * 0.8)
        .collect();
    chart
        .set_series_data(0, &times, &close, &close, &close, &close)
        .unwrap();
    chart.series[0].kind = SeriesKind::Line;
    let vwap = chart.add_vwap(0, None).unwrap();
    assert!(chart.series_apply_options_json(vwap, r##"{"color":"#e0401c","line_width":2}"##));
    let pane = chart.add_pane(true).unwrap();
    chart.set_series_pane(vwap, pane, 1.0);
    chart.time_scale.set_width(480.0);
    chart.fit_content();
    chart
}

/// What `render_engine` rasterizes, with every `Segments` batch written out as separate solid
/// two-point polylines and the pool indices rebased by hand.
fn render_engine_with_polylines(chart: &mut ChartEngine) -> TinySkiaCanvas {
    let frame = chart.build_frame();
    let mut prims = Vec::new();
    let mut points = Vec::new();
    for pane in frame.panes {
        let base = points.len() as u32;
        points.extend(pane.points);
        prims.extend(pane.under);
        for prim in pane.main.into_iter().chain(pane.top_prims) {
            match prim {
                Prim::Polyline {
                    first_point,
                    point_count,
                    width,
                    style,
                    line_type,
                    color,
                } => prims.push(Prim::Polyline {
                    first_point: first_point + base,
                    point_count,
                    width,
                    style,
                    line_type,
                    color,
                }),
                Prim::Segments {
                    first_point,
                    segment_count,
                    width,
                    color,
                } => prims.extend(separate_polylines(
                    first_point + base,
                    segment_count,
                    width,
                    color,
                )),
                other => {
                    assert!(
                        !matches!(other, Prim::AreaFill { .. } | Prim::BandFill { .. }),
                        "the fixture draws no pool-indexed fill"
                    );
                    prims.push(other);
                }
            }
        }
    }
    let options = chart.options.get();
    let surface = aeris_charts_core::style::DEFAULT_SURFACE_RGB;
    let background = Color::parse_css(&options.layout.background.color)
        .unwrap_or(Color::rgb(surface.0, surface.1, surface.2));
    render_prims(
        (frame.width * frame.pixel_ratio).round() as u32,
        (frame.height * frame.pixel_ratio).round() as u32,
        background,
        &prims,
        &points,
    )
}

#[test]
fn image_export_rebases_segments_in_every_pane() {
    let mut chart = two_pane_daily_study();
    let frame = chart.build_frame();
    assert_eq!(frame.panes.len(), 2);
    assert!(
        !frame.panes[0].points.is_empty(),
        "pane 0's line precedes pane 1's points in the exported pool"
    );
    assert!(
        frame.panes[1]
            .main
            .iter()
            .any(|prim| matches!(prim, Prim::Segments { .. })),
        "pane 1 holds the study batch"
    );
    let exported = render_engine(&mut chart);
    let reference = render_engine_with_polylines(&mut chart);
    let stats = diff_pixmaps(exported.pixmap(), reference.pixmap(), 0).expect("same size");
    // Neighbouring pairs step by far more than the line width, so they share no anti-aliased
    // pixel and the batch must match the separate strokes exactly. Reading pane 0's points
    // instead of pane 1's moved every pair into the wrong pane (3,736 px off before the rebase).
    assert_eq!(stats.differing_pixels, 0, "{stats:?}");
}
