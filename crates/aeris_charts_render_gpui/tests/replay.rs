//! Cache and memory stability under a sustained update replay.
//!
//! The gate is "no unbounded cache, atlas, geometry, or allocation growth". The way that fails in
//! practice is not a leak but a *plateau that never arrives*: a scratch buffer that regrows, a plan
//! that appends instead of clearing, or a cache keyed on something that changes every frame. Each
//! test here drives many frames through a mutating engine and asserts the retained footprint stops
//! growing.
//!
//! These run in the normal `cargo test` pass and need no GPUI feature, window, or GPU.

use aeris_charts_engine::{ChartEngine, SeriesKind};
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{Prim, TextAlign};
use aeris_charts_render_gpui::{GpuiChartRenderer, PreparedAerisFrame, TextKey, TextMetrics};

const CSS_W: f64 = 1200.0;
const CSS_H: f64 = 700.0;
const DPR: f64 = 1.5;

fn engine_with(bars: usize) -> ChartEngine {
    let mut engine = ChartEngine::new(CSS_W, CSS_H, DPR);
    let times: Vec<f64> = (0..bars)
        .map(|i| 1_600_000_000.0 + i as f64 * 60.0)
        .collect();
    let close: Vec<f64> = (0..bars)
        .map(|i| 100.0 + (i as f64 * 0.05).sin() * 12.0)
        .collect();
    let open: Vec<f64> = close
        .iter()
        .enumerate()
        .map(|(i, c)| if i == 0 { *c } else { close[i - 1] })
        .collect();
    let high: Vec<f64> = open
        .iter()
        .zip(&close)
        .map(|(o, c)| o.max(*c) + 1.0)
        .collect();
    let low: Vec<f64> = open
        .iter()
        .zip(&close)
        .map(|(o, c)| o.min(*c) - 1.0)
        .collect();
    engine
        .set_series_data(0, &times, &open, &high, &low, &close)
        .expect("series loads");
    engine.series[0].kind = SeriesKind::Candlestick;
    let line = engine.add_series(SeriesKind::Line);
    engine
        .set_series_data(line, &times, &close, &close, &close, &close)
        .expect("line loads");

    engine.css_width = CSS_W;
    engine.css_height = CSS_H;
    engine.dpr = DPR;
    let content_h = (CSS_H - engine.time_axis_height()).max(1.0);
    engine.layout_panes(content_h);
    engine.time_scale.set_width(CSS_W);
    engine.fit_content();
    engine
}

/// Retained bytes attributable to the adapter's reusable buffers.
fn footprint(r: &GpuiChartRenderer) -> usize {
    r.scratch_bytes()
        + r.plan().ops.capacity() * std::mem::size_of::<aeris_charts_render_gpui::SceneOp>()
        + r.plan().vertices.capacity() * std::mem::size_of::<aeris_charts_render_gpui::MeshVertex>()
}

#[test]
fn a_live_append_replay_reaches_a_stable_footprint() {
    let mut engine = engine_with(600);
    let mut renderer = GpuiChartRenderer::new();

    // Warm up: the first frames legitimately grow the plan and scratch to the frame's size.
    for _ in 0..32 {
        let frame = engine.build_frame();
        renderer
            .plan_frame(&PreparedAerisFrame::new(&frame), DPR as f32)
            .expect("frame plans");
    }
    let settled = footprint(&renderer);
    assert!(settled > 0, "the adapter should retain reusable buffers");

    // Sustained replay with a live append every frame — the pan/scroll/append steady state.
    let mut ops_seen = Vec::new();
    for i in 0..600 {
        let t = 1_600_000_000.0 + (600 + i) as f64 * 60.0;
        let c = 100.0 + (i as f64 * 0.05).sin() * 12.0;
        engine.update_series_bar(0, t, [c, c + 1.0, c - 1.0, c]);
        engine.fit_content();
        let frame = engine.build_frame();
        let m = renderer
            .plan_frame(&PreparedAerisFrame::new(&frame), DPR as f32)
            .expect("frame plans");
        ops_seen.push(m.ops);
        assert_eq!(m.dropped_prims, 0, "frame {i} dropped a prim");
    }

    let after = footprint(&renderer);
    assert!(
        after <= settled * 2,
        "footprint grew from {settled} to {after} bytes over a 600-frame append replay"
    );

    // The op count must stay in a bounded band: the engine conflates to the visible pixel width,
    // so appending data does not grow the emitted scene.
    let min = *ops_seen.iter().min().unwrap();
    let max = *ops_seen.iter().max().unwrap();
    assert!(
        max <= min * 2,
        "emitted op count drifted from {min} to {max} across the replay"
    );
}

#[test]
fn replanning_the_same_frame_never_grows_anything() {
    let mut engine = engine_with(400);
    let frame = engine.build_frame();
    let prepared = PreparedAerisFrame::new(&frame);
    let mut renderer = GpuiChartRenderer::new();

    for _ in 0..8 {
        renderer.plan_frame(&prepared, DPR as f32).unwrap();
    }
    let settled = footprint(&renderer);
    let first = renderer.plan_frame(&prepared, DPR as f32).unwrap();

    for _ in 0..500 {
        let m = renderer.plan_frame(&prepared, DPR as f32).unwrap();
        assert_eq!(m.ops, first.ops);
        assert_eq!(m.prims, first.prims);
        assert_eq!(m.mesh_vertices, first.mesh_vertices);
    }
    assert_eq!(
        footprint(&renderer),
        settled,
        "an unchanged frame must not move the footprint at all"
    );
}

#[test]
fn a_resize_and_dpr_sweep_settles_rather_than_ratcheting() {
    // Every distinct size and DPR produces a differently sized frame; the buffers should grow to
    // the largest and then stay there, not accumulate per configuration.
    let mut engine = engine_with(500);
    let mut renderer = GpuiChartRenderer::new();
    let configs = [
        (800.0f64, 500.0f64, 1.0f64),
        (1200.0, 700.0, 1.5),
        (1600.0, 900.0, 2.0),
        (1001.0, 601.0, 1.25),
        (1600.0, 900.0, 2.5),
    ];

    // First pass: grow to the largest configuration.
    for _ in 0..3 {
        for (w, h, dpr) in configs {
            engine.css_width = w;
            engine.css_height = h;
            engine.dpr = dpr;
            let content_h = (h - engine.time_axis_height()).max(1.0);
            engine.layout_panes(content_h);
            engine.time_scale.set_width(w);
            engine.fit_content();
            let frame = engine.build_frame();
            renderer
                .plan_frame(&PreparedAerisFrame::new(&frame), dpr as f32)
                .expect("frame plans at every size and dpr");
        }
    }
    let settled = footprint(&renderer);

    // Second pass over the same configurations must not grow it further.
    for _ in 0..6 {
        for (w, h, dpr) in configs {
            engine.css_width = w;
            engine.css_height = h;
            engine.dpr = dpr;
            let content_h = (h - engine.time_axis_height()).max(1.0);
            engine.layout_panes(content_h);
            engine.time_scale.set_width(w);
            engine.fit_content();
            let frame = engine.build_frame();
            renderer
                .plan_frame(&PreparedAerisFrame::new(&frame), dpr as f32)
                .unwrap();
        }
    }
    assert_eq!(
        footprint(&renderer),
        settled,
        "revisiting known sizes must not grow the buffers"
    );
}

#[test]
fn crosshair_movement_does_not_grow_the_footprint() {
    let mut engine = engine_with(500);
    let mut renderer = GpuiChartRenderer::new();
    for _ in 0..16 {
        engine.crosshair = Some((600.0, 350.0));
        let frame = engine.build_frame();
        renderer
            .plan_frame(&PreparedAerisFrame::new(&frame), DPR as f32)
            .unwrap();
    }
    let settled = footprint(&renderer);

    for i in 0..400 {
        engine.crosshair = Some((10.0 + (i % 1000) as f64, 40.0 + (i % 600) as f64));
        let frame = engine.build_frame();
        renderer
            .plan_frame(&PreparedAerisFrame::new(&frame), DPR as f32)
            .unwrap();
    }
    assert!(
        footprint(&renderer) <= settled * 2,
        "crosshair movement grew the footprint from {settled} to {}",
        footprint(&renderer)
    );
}

#[test]
fn the_text_cache_is_bounded_by_distinct_runs_not_by_frames() {
    // A static axis label repeated every frame must hit; a scrolling one must not accumulate past
    // the cache capacity.
    let mut renderer = GpuiChartRenderer::new();
    let axis: Vec<Prim> = (0..24)
        .map(|i| Prim::Text {
            x: 1180.0,
            y: 20.0 + i as f32 * 24.0,
            text: format!("{:.2}", 100.0 + i as f32),
            color: Color::rgb(0x13, 0x17, 0x22),
            size: 12.0,
            family: "sans-serif".into(),
            align: TextAlign::Right,
            weight: 400,
            italic: false,
        })
        .collect();

    let mut engine = engine_with(300);
    let frame = engine.build_frame();
    let prepared = PreparedAerisFrame::new(&frame).with_axis(&axis, &[]);

    for _ in 0..200 {
        let m = renderer.plan_frame(&prepared, DPR as f32).unwrap();
        assert_eq!(m.text_runs, 24, "every label should lower to a text run");
    }
    // `plan_frame` does not shape text (that needs GPUI), so the cache stays empty here — the
    // measurement path is exercised directly below to prove the bound.
    assert_eq!(renderer.text_cache().len(), 0);
}

#[test]
fn the_text_cache_plateaus_at_its_capacity_under_scrolling_labels() {
    use aeris_charts_render_gpui::TextCache;
    let mut cache = TextCache::with_capacity(64);
    // A scrolling label changes subpixel phase every frame, so every frame is a miss. The cache
    // must plateau at its capacity rather than growing without bound.
    for i in 0..5_000 {
        let run = aeris_charts_render_gpui::TextRun {
            x: i as f32 * 0.37,
            y: 10.0,
            text: "42.50".into(),
            color: Color::rgb(0, 0, 0),
            size: 12.0,
            family: "sans-serif".into(),
            align: TextAlign::Left,
            weight: 400,
            italic: false,
            angle: 0.0,
        };
        cache.measure_with(TextKey::for_run(&run), || TextMetrics {
            width: 30.0,
            ascent: 10.0,
            descent: -3.0,
        });
        assert!(
            cache.len() <= 64,
            "cache exceeded its capacity at iteration {i}: {}",
            cache.len()
        );
    }
    assert_eq!(
        cache.len(),
        64,
        "a saturated cache sits exactly at capacity"
    );
    assert!(cache.heap_bytes() > 0 && cache.heap_bytes() < 64 * 1024);
}

#[test]
fn invalidating_caches_releases_everything_and_allows_regrowth() {
    let mut renderer = GpuiChartRenderer::new();
    let mut engine = engine_with(300);
    let frame = engine.build_frame();
    let prepared = PreparedAerisFrame::new(&frame);
    for _ in 0..8 {
        renderer.plan_frame(&prepared, DPR as f32).unwrap();
    }
    let before = renderer.text_cache().generation();
    renderer.invalidate_caches();
    assert_eq!(renderer.text_cache().generation(), before + 1);
    assert!(renderer.text_cache().is_empty());
    // Planning still works after an invalidation.
    renderer.plan_frame(&prepared, DPR as f32).unwrap();
}
