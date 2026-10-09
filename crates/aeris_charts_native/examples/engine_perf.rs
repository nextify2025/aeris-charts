use std::time::Instant;

use aeris_charts_engine::{ChartEngine, SeriesKind};

fn benchmark_frame(chart: &mut ChartEngine, bars: usize, kind: SeriesKind, label: &str) {
    chart.series[0].kind = kind;
    let mut frame = aeris_charts_engine::ChartFrame::default();
    let start = Instant::now();
    chart.build_frame_into(&mut frame);
    let elapsed = start.elapsed();
    let points: usize = frame.panes.iter().map(|pane| pane.points.len()).sum();
    let primitives: usize = frame
        .panes
        .iter()
        .map(|pane| pane.under.len() + pane.main.len())
        .sum();
    println!(
        "{label} conflation: {bars} installed source points -> {points} points / {primitives} primitives in {elapsed:?}"
    );
}

fn main() {
    let bars: usize = std::env::var("AERIS_CHARTS_BARS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(50_000);
    let mut times = Vec::with_capacity(bars);
    let mut open = Vec::with_capacity(bars);
    let mut high = Vec::with_capacity(bars);
    let mut low = Vec::with_capacity(bars);
    let mut close = Vec::with_capacity(bars);
    let mut price = 100.0;
    for i in 0..bars {
        let wobble = ((i as f64) * 0.017).sin() * 0.8;
        let next = price + wobble;
        times.push(i as f64);
        open.push(price);
        high.push(price.max(next) + 0.4);
        low.push(price.min(next) - 0.4);
        close.push(next);
        price = next;
    }

    let mut chart = ChartEngine::new(1600.0, 800.0, 1.0);
    let load_start = Instant::now();
    chart
        .set_series_data(0, &times, &open, &high, &low, &close)
        .expect("valid fixture");
    println!("{bars} bar install: {:?}", load_start.elapsed());
    chart.time_scale.set_width(1600.0);
    chart.fit_content();
    let _sma = chart.add_sma(0, 20).expect("valid indicator");
    let update_start = Instant::now();
    for i in 0..1_000 {
        let value = price + (i as f64 * 0.01).sin();
        chart.update_series_bar(
            0,
            (bars + i) as f64,
            [value, value + 0.4, value - 0.4, value],
        );
    }
    let update_elapsed = update_start.elapsed();
    println!(
        "1,000 streaming updates with SMA: {:?} ({:.3} µs/update)",
        update_elapsed,
        update_elapsed.as_secs_f64() * 1_000_000.0 / 1_000.0
    );
    let mut frame = aeris_charts_engine::ChartFrame::default();
    let start = Instant::now();
    for _ in 0..10 {
        chart.build_frame_into(&mut frame);
    }
    let elapsed = start.elapsed();
    println!(
        "{bars} bars: 10 retained frame builds in {:?} ({:.2} ms/frame)",
        elapsed,
        elapsed.as_secs_f64() * 100.0
    );

    // Isolate every sub-pixel series path on the same headless model and visible range. This keeps
    // backend overhead out of the measurement and makes frame output growth directly observable.
    let mut conflation_chart = ChartEngine::new(1600.0, 800.0, 1.0);
    conflation_chart
        .set_series_data(0, &times, &open, &high, &low, &close)
        .expect("valid conflation fixture");
    conflation_chart.time_scale.set_width(1600.0);
    conflation_chart.fit_content();
    benchmark_frame(&mut conflation_chart, bars, SeriesKind::Line, "line");
    benchmark_frame(
        &mut conflation_chart,
        bars,
        SeriesKind::Candlestick,
        "candlestick",
    );
    benchmark_frame(&mut conflation_chart, bars, SeriesKind::Bar, "bar");
    benchmark_frame(
        &mut conflation_chart,
        bars,
        SeriesKind::Histogram,
        "histogram",
    );
}
