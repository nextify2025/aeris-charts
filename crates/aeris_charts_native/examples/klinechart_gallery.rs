//! Render every KLineChart indicator through the native backend, for checking them by eye.
//!
//! ```text
//! cargo run -p aeris_charts_native --example klinechart_gallery -- <output dir>
//! ```
//!
//! Writes one PNG per indicator (candles plus the indicator, with a KLineChart-style legend of
//! the last values), `overview.png` with KLineChart's default layout (MA over the candles, VOL and
//! MACD below), `contact_sheet.png` with all 27 indicators side by side, and `market.json` with
//! the bars, so the same chart can be rendered in KLineChart for comparison.

use aeris_charts_core::model::data_layer::SeriesId;
use aeris_charts_engine::klinechart::{Indicator, Placement, NAMES};
use aeris_charts_engine::{ChartEngine, ChartTheme, SeriesKind};
use aeris_charts_native::{render_engine, TinySkiaCanvas};
use aeris_charts_render::canvas2d::Canvas2d;
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::TextAlign;
use tiny_skia::{Pixmap, PixmapPaint, Transform};

const WIDTH: f64 = 960.0;
const HEIGHT: f64 = 540.0;
const PRICE_AXIS_WIDTH: f64 = 64.0;
const BARS: usize = 180;
const DAY: f64 = 86_400.0;

struct Market {
    times: Vec<f64>,
    open: Vec<f64>,
    high: Vec<f64>,
    low: Vec<f64>,
    close: Vec<f64>,
    volume: Vec<f64>,
    turnover: Vec<f64>,
}

/// A seeded daily random walk with a trend change, so trend and oscillator studies both move.
fn market() -> Market {
    let mut seed = 20_260_926u64;
    let mut random = || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut market = Market {
        times: vec![],
        open: vec![],
        high: vec![],
        low: vec![],
        close: vec![],
        volume: vec![],
        turnover: vec![],
    };
    let mut price = 120.0;
    for day in 0..BARS {
        let drift = if day < 70 {
            0.35
        } else if day < 125 {
            -0.45
        } else {
            0.2
        };
        let open = price + (random() - 0.5) * 1.2;
        let close = (open + drift + (random() - 0.5) * 4.0).max(5.0);
        let high = open.max(close) + random() * 2.0;
        let low = open.min(close) - random() * 2.0;
        let volume = (1.0e6 + random() * 2.5e6) * (1.0 + (close - open).abs() / 3.0);
        market.times.push(1_735_689_600.0 + day as f64 * DAY);
        market.open.push(open);
        market.high.push(high);
        market.low.push(low);
        market.close.push(close);
        market.volume.push(volume.round());
        market
            .turnover
            .push(volume.round() * (high + low + close) / 3.0);
        price = close;
    }
    market
}

struct Chart {
    engine: ChartEngine,
    /// Each bound indicator with its output series.
    bindings: Vec<(Indicator, Vec<SeriesId>)>,
}

fn chart(market: &Market, indicators: &[Indicator], height: f64) -> Chart {
    let mut engine = ChartEngine::new(WIDTH, height, 1.0);
    engine.set_theme(ChartTheme::Light);
    engine
        .set_series_data(
            0,
            &market.times,
            &market.open,
            &market.high,
            &market.low,
            &market.close,
        )
        .expect("candles");
    // Volume and turnover are inputs only.
    let volume = engine.add_series(SeriesKind::Histogram);
    let v = &market.volume;
    engine
        .set_series_data(volume, &market.times, v, v, v, v)
        .expect("volume");
    engine.set_series_visible(volume, false);
    let turnover = engine.add_series(SeriesKind::Line);
    let t = &market.turnover;
    engine
        .set_series_data(turnover, &market.times, t, t, t, t)
        .expect("turnover");
    engine.set_series_visible(turnover, false);

    let mut bindings = Vec::new();
    for indicator in indicators {
        let source = if matches!(indicator, Indicator::Avp) {
            turnover
        } else {
            0
        };
        let outputs = engine.add_klinechart_indicator(
            source,
            indicator.clone(),
            indicator.needs_volume().then_some(volume),
        );
        assert_eq!(
            outputs.len(),
            indicator.output_count(),
            "{}",
            indicator.title()
        );
        bindings.push((indicator.clone(), outputs));
    }

    engine.pane_w = WIDTH - PRICE_AXIS_WIDTH;
    engine.axis_w = PRICE_AXIS_WIDTH;
    engine.pane_h = height - engine.time_axis_height();
    engine.layout_panes(engine.pane_h);
    engine.time_scale.set_width(engine.pane_w);
    engine.fit_content();
    Chart { engine, bindings }
}

/// KLineChart's tooltip line: `MACD(12,26,9)  DIF: 1.2345  DEA: ...`, from the last row.
fn legend(engine: &ChartEngine, indicator: &Indicator, outputs: &[SeriesId]) -> String {
    let mut text = indicator.title();
    for (title, &id) in indicator.output_titles().iter().zip(outputs) {
        let value = engine
            .series_data(id)
            .last()
            .and_then(|point| engine.series_format_price(id, point.close))
            .unwrap_or_else(|| "n/a".to_owned());
        text.push_str(&format!("  {title}: {value}"));
    }
    text
}

fn render(chart: &mut Chart) -> TinySkiaCanvas {
    let mut canvas = render_engine(&mut chart.engine);
    // Label each pane at its top-left corner, the way KLineChart's tooltip sits.
    for (indicator, outputs) in &chart.bindings {
        let (pane, _) = chart
            .engine
            .series_price_scale(outputs[0])
            .expect("output pane");
        let top = chart.engine.panes[pane].top as f32;
        let offset = if indicator.placement() == Placement::Price {
            // Stack price-pane legends below one another.
            chart
                .bindings
                .iter()
                .take_while(|(other, _)| other != indicator)
                .filter(|(other, _)| other.placement() == Placement::Price)
                .count() as f32
                * 16.0
        } else {
            0.0
        };
        canvas.fill_text(
            &legend(&chart.engine, indicator, outputs),
            8.0,
            top + 12.0 + offset,
            "12px sans-serif",
            Color::rgb(0x33, 0x33, 0x33),
            TextAlign::Left,
        );
    }
    canvas
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "klinechart_gallery".to_owned());
    std::fs::create_dir_all(&dir).expect("output directory");
    let market = market();
    // The same bars for rendering the overview in KLineChart itself.
    let bars = (0..BARS)
        .map(|row| {
            serde_json::json!({
                "timestamp": market.times[row] as i64 * 1000,
                "open": market.open[row],
                "high": market.high[row],
                "low": market.low[row],
                "close": market.close[row],
                "volume": market.volume[row],
                "turnover": market.turnover[row],
            })
        })
        .collect::<Vec<_>>();
    let path = format!("{dir}/market.json");
    std::fs::write(&path, serde_json::to_string(&bars).expect("json")).expect("write market");
    println!("wrote {path}");

    let overview = ["MA", "VOL", "MACD"].map(|name| Indicator::from_name(name).unwrap());
    let mut overview = chart(&market, &overview, 720.0);
    let path = format!("{dir}/overview.png");
    render(&mut overview).save_png(&path).expect("save png");
    println!("wrote {path}");

    let columns = 4;
    let (thumb_w, thumb_h) = (480u32, 270u32);
    let rows = NAMES.len().div_ceil(columns) as u32;
    let mut sheet = Pixmap::new(thumb_w * columns as u32, thumb_h * rows).expect("sheet");
    sheet.fill(tiny_skia::Color::WHITE);
    for (index, name) in NAMES.iter().enumerate() {
        let indicator = Indicator::from_name(name).unwrap();
        let mut single = chart(&market, std::slice::from_ref(&indicator), HEIGHT);
        let canvas = render(&mut single);
        let path = format!("{dir}/{:02}_{}.png", index + 1, name.to_ascii_lowercase());
        canvas.save_png(&path).expect("save png");
        println!("wrote {path}");

        let pixmap = canvas.pixmap();
        let (column, row) = ((index % columns) as u32, (index / columns) as u32);
        sheet.draw_pixmap(
            0,
            0,
            pixmap.as_ref(),
            &PixmapPaint::default(),
            Transform::from_row(
                thumb_w as f32 / pixmap.width() as f32,
                0.0,
                0.0,
                thumb_h as f32 / pixmap.height() as f32,
                (column * thumb_w) as f32,
                (row * thumb_h) as f32,
            ),
            None,
        );
    }
    let path = format!("{dir}/contact_sheet.png");
    sheet.save_png(&path).expect("save contact sheet");
    println!("wrote {path}");
}
