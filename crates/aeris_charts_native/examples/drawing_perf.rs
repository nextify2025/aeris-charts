use std::hint::black_box;
use std::time::Instant;

use aeris_charts_engine::{ChartEngine, ChartFrame, DrawingKind, DrawingModifiers, DrawingPoint};

const RUNS: usize = 120;
const WIDTH: f64 = 1280.0;
const HEIGHT: f64 = 720.0;

fn percentile(mut samples: Vec<f64>) -> [f64; 3] {
    samples.sort_by(f64::total_cmp);
    let at = |percent: usize| samples[(samples.len() - 1) * percent / 100];
    [at(50), at(95), at(99)]
}

fn measure(mut operation: impl FnMut()) -> [f64; 3] {
    for _ in 0..10 {
        operation();
    }
    percentile(
        (0..RUNS)
            .map(|_| {
                let started = Instant::now();
                operation();
                started.elapsed().as_secs_f64() * 1_000_000.0
            })
            .collect(),
    )
}

fn chart(bars: usize) -> ChartEngine {
    let times = (0..bars).map(|index| index as f64).collect::<Vec<_>>();
    let close = (0..bars)
        .map(|index| 100.0 + (index as f64 * 0.013).sin() * 4.0)
        .collect::<Vec<_>>();
    let open = close.clone();
    let high = close.iter().map(|value| value + 1.0).collect::<Vec<_>>();
    let low = close.iter().map(|value| value - 1.0).collect::<Vec<_>>();
    let mut chart = ChartEngine::new(WIDTH, HEIGHT, 1.0);
    chart
        .set_series_data(0, &times, &open, &high, &low, &close)
        .expect("fixture");
    chart.time_scale.set_width(WIDTH);
    chart.set_visible_logical_range(49_950.0, 50_050.0);
    let mut frame = ChartFrame::default();
    chart.build_frame_into(&mut frame);
    chart
}

fn points(kind: DrawingKind, index: usize, mostly_offscreen: bool) -> Vec<DrawingPoint> {
    let logical = if mostly_offscreen && index < 10 {
        50_000.0
    } else if mostly_offscreen {
        index as f64 * 100.0
    } else {
        49_960.0 + (index % 80) as f64
    };
    let price = if mostly_offscreen && index >= 10 {
        200.0 + index as f64
    } else {
        98.0 + (index % 40) as f64 * 0.1
    };
    // One anchor per defining point, spread along a rising diagonal.
    (0..kind.anchor_count())
        .map(|step| DrawingPoint {
            logical: logical + 8.0 * step as f64,
            price: price + 2.0 * step as f64,
        })
        .collect()
}

fn install_drawings(chart: &mut ChartEngine, count: usize, mix: &str, mostly_offscreen: bool) {
    let kinds = [
        DrawingKind::TrendLine,
        DrawingKind::Rectangle,
        DrawingKind::Brush,
        DrawingKind::Path,
        DrawingKind::Text,
        DrawingKind::HorizontalLine,
        DrawingKind::HorizontalRay,
        DrawingKind::VerticalLine,
    ];
    // Every B8 family tool (wire ids 32 and up, reserved per family), read from the engine
    // catalog so family work never edits this harness.
    let families = (32..=u8::MAX)
        .filter_map(DrawingKind::from_u8)
        .collect::<Vec<_>>();
    for index in 0..count {
        let kind = match mix {
            "families" => families[index % families.len()],
            "trend" => DrawingKind::TrendLine,
            "rectangle" => DrawingKind::Rectangle,
            "brush" => DrawingKind::Brush,
            "text" => DrawingKind::Text,
            _ => kinds[index % kinds.len()],
        };
        chart
            .add_drawing(
                kind,
                0,
                points(kind, index, mostly_offscreen),
                (kind == DrawingKind::Text).then_some(r#"{"text":"density label"}"#),
            )
            .expect("drawing");
    }
}

fn row(count: usize, mix: &str, mostly_offscreen: bool) {
    let mut chart = chart(100_000);
    install_drawings(&mut chart, count, mix, mostly_offscreen);
    let mut frame = ChartFrame::default();
    let initial_started = Instant::now();
    chart.build_frame_into(&mut frame);
    let initial_us = initial_started.elapsed().as_secs_f64() * 1_000_000.0;
    let primitive_count = frame
        .panes
        .iter()
        .map(|pane| pane.main.len())
        .sum::<usize>();

    let frame_samples = measure(|| {
        chart.set_visible_logical_range(49_950.0, 50_050.0);
        chart.build_frame_into(&mut frame);
        black_box(&frame);
    });
    let hit_samples = measure(|| {
        black_box(chart.hit_test_drawing(17.0, 17.0));
    });
    let selection_samples = measure(|| {
        black_box(chart.select_drawing_at(17.0, 17.0));
    });
    let mut pan_toggle = false;
    let pan_samples = measure(|| {
        pan_toggle = !pan_toggle;
        let shift = if pan_toggle { 0.5 } else { -0.5 };
        chart.set_visible_logical_range(49_950.0 + shift, 50_050.0 + shift);
        chart.build_frame_into(&mut frame);
    });
    let mut zoom_toggle = false;
    let zoom_samples = measure(|| {
        zoom_toggle = !zoom_toggle;
        let edge = if zoom_toggle { 0.5 } else { 1.0 };
        chart.set_visible_logical_range(49_950.0 - edge, 50_050.0 + edge);
        chart.build_frame_into(&mut frame);
    });

    let drag_samples = if count == 0 {
        [0.0; 3]
    } else {
        let id = chart.drawings()[0].id;
        chart.set_selected_drawing(Some(id));
        let (x, y) = chart
            .drawing_point_to_coordinate(id, 0)
            .expect("visible anchor");
        assert!(chart.drawing_drag_start_at(x, y));
        let mut step = 0.0;
        let samples = measure(|| {
            step += 0.01;
            chart.drawing_drag_to(x + step, y, DrawingModifiers::default());
            chart.build_frame_into(&mut frame);
        });
        chart.drawing_drag_end();
        samples
    };

    chart.reset_drawing_work_stats();
    chart.set_visible_logical_range(49_950.0, 50_050.0);
    chart.build_frame_into(&mut frame);
    let frame_work = chart.drawing_work_stats();
    chart.reset_drawing_work_stats();
    black_box(chart.hit_test_drawing(17.0, 17.0));
    let hit_work = chart.drawing_work_stats();
    println!(
        "count={count} mix={mix} offscreen={mostly_offscreen} initial_us={initial_us:.3} primitives={primitive_count} frame_us={frame_samples:?} hit_us={hit_samples:?} selection_us={selection_samples:?} drag_us={drag_samples:?} pan_us={pan_samples:?} zoom_us={zoom_samples:?} frame_candidates={} visible={} hit_candidates={} precise={} geometry_rebuilds={} bounds_rebuilds={}",
        frame_work.candidates,
        frame_work.visible,
        hit_work.candidates,
        hit_work.precise_hit_tests,
        frame_work.geometry_rebuilds,
        frame_work.bounds_rebuilds,
    );
}

fn brush_path_row(point_count: usize) {
    let mut chart = chart(100_000);
    let points = (0..point_count)
        .map(|index| DrawingPoint {
            logical: 49_960.0 + index as f64 * 80.0 / point_count as f64,
            price: 100.0 + (index as f64 * 0.03).sin() * 2.0,
        })
        .collect();
    let id = chart
        .add_drawing(DrawingKind::Brush, 0, points, None)
        .expect("brush");
    let mut frame = ChartFrame::default();
    let frame_us = measure(|| {
        chart.set_visible_logical_range(49_950.0, 50_050.0);
        chart.build_frame_into(&mut frame);
    });
    let hit_us = measure(|| {
        black_box(chart.hit_test_drawing(WIDTH / 2.0, HEIGHT / 2.0));
    });
    let (x, y) = chart.drawing_point_to_coordinate(id, 0).unwrap();
    assert!(chart.drawing_drag_start_at(x, y));
    let move_us = measure(|| chart.drawing_drag_to(x + 1.0, y, DrawingModifiers::default()));
    chart.drawing_drag_end();
    println!(
        "brush_points={point_count} frame_us={frame_us:?} hit_us={hit_us:?} move_us={move_us:?} runtime_bytes={}",
        chart.memory_usage().drawing_runtime_capacity_bytes,
    );
}

fn combined_foundation_row() {
    let mut chart = chart(1_000_000);
    install_drawings(&mut chart, 1_000, "mixed", true);
    let mut frame = ChartFrame::default();
    chart.build_frame_into(&mut frame);
    let mut pan_toggle = false;
    let pan_us = measure(|| {
        pan_toggle = !pan_toggle;
        let shift = if pan_toggle { 1.0 } else { -1.0 };
        chart.set_visible_logical_range(49_950.0 + shift, 50_050.0 + shift);
        chart.build_frame_into(&mut frame);
    });
    let mut zoom_toggle = false;
    let zoom_us = measure(|| {
        zoom_toggle = !zoom_toggle;
        let edge = if zoom_toggle { 1.0 } else { 2.0 };
        chart.set_visible_logical_range(49_950.0 - edge, 50_050.0 + edge);
        chart.build_frame_into(&mut frame);
    });
    let mut crosshair_x = 500.0;
    let crosshair_us = measure(|| {
        crosshair_x = if crosshair_x == 500.0 { 501.0 } else { 500.0 };
        chart.set_crosshair_at(crosshair_x, 300.0);
        chart.build_frame_into(&mut frame);
    });
    let mut current_close = 101.0;
    let current_us = measure(|| {
        current_close = if current_close == 101.0 { 101.5 } else { 101.0 };
        chart.update_series_bar(
            0,
            999_999.0,
            [
                current_close,
                current_close + 1.0,
                current_close - 1.0,
                current_close,
            ],
        );
        chart.build_frame_into(&mut frame);
    });
    chart.reset_drawing_work_stats();
    chart.set_visible_logical_range(49_950.0, 50_050.0);
    chart.build_frame_into(&mut frame);
    let work = chart.drawing_work_stats();
    println!(
        "combined_bars=1000000 drawings=1000 pan_us={pan_us:?} zoom_us={zoom_us:?} crosshair_us={crosshair_us:?} current_us={current_us:?} candidates={} visible={} geometry_rebuilds={} runtime_bytes={}",
        work.candidates,
        work.visible,
        work.geometry_rebuilds,
        chart.memory_usage().drawing_runtime_capacity_bytes,
    );
}

fn main() {
    if std::env::var_os("AERIS_CHARTS_DRAWING_QUICK").is_some() {
        row(1_000, "mixed", true);
        row(1_000, "families", false);
        row(1_000, "families", true);
        combined_foundation_row();
        return;
    }
    for count in [0, 10, 50, 100, 250, 500, 1_000] {
        for mix in ["trend", "rectangle", "brush", "text", "families", "mixed"] {
            row(count, mix, false);
        }
    }
    row(1_000, "mixed", true);
    row(1_000, "families", true);
    for points in [100, 1_000, 10_000] {
        brush_path_row(points);
    }
    combined_foundation_row();
}
