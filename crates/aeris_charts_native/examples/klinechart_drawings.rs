//! Render the drawing tools ported from KLineChart's overlays, one chart per tool, for checking
//! them by eye.
//!
//! ```text
//! cargo run -p aeris_charts_native --example klinechart_drawings -- <output dir>
//! ```
//!
//! Writes `drawing_<name>.png` for each tool and `drawings_sheet.png` with all of them.

use aeris_charts_engine::{ChartEngine, ChartTheme, DrawingKind, DrawingPoint};
use aeris_charts_native::render_engine;
use aeris_charts_render::canvas2d::Canvas2d;
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::TextAlign;
use tiny_skia::{Pixmap, PixmapPaint, Transform};

const WIDTH: f64 = 720.0;
const HEIGHT: f64 = 400.0;
const PRICE_AXIS_WIDTH: f64 = 60.0;
const BARS: usize = 120;

fn point(logical: f64, price: f64) -> DrawingPoint {
    DrawingPoint { logical, price }
}

/// Each tool with anchors placed on the fixture's swings, and its options.
fn tools() -> Vec<(DrawingKind, Vec<DrawingPoint>, &'static str)> {
    vec![
        (
            DrawingKind::StraightLine,
            vec![point(10.0, 121.0), point(45.0, 137.0)],
            r##"{"color":"#1677FF"}"##,
        ),
        (
            DrawingKind::RayLine,
            vec![point(50.0, 142.0), point(80.0, 132.0)],
            r##"{"color":"#E11D74"}"##,
        ),
        (
            DrawingKind::HorizontalSegment,
            vec![point(30.0, 140.0), point(75.0, 140.0)],
            r##"{"color":"#FF9600"}"##,
        ),
        (
            DrawingKind::VerticalRay,
            vec![point(52.0, 146.0), point(52.0, 150.0)],
            r##"{"color":"#935EBD"}"##,
        ),
        (
            DrawingKind::VerticalSegment,
            vec![point(88.0, 126.0), point(88.0, 138.0)],
            r##"{"color":"#01C5C4"}"##,
        ),
        (
            DrawingKind::PriceLine,
            vec![point(70.0, 134.0)],
            r##"{"color":"#1677FF"}"##,
        ),
        (
            DrawingKind::ParallelLine,
            vec![point(5.0, 119.0), point(45.0, 136.0), point(5.0, 126.0)],
            r##"{"color":"#935EBD"}"##,
        ),
        (
            DrawingKind::PriceChannel,
            vec![point(55.0, 145.0), point(95.0, 128.0), point(55.0, 139.0)],
            r##"{"color":"#E11D74"}"##,
        ),
        (
            DrawingKind::FibonacciLine,
            vec![point(52.0, 146.0), point(92.0, 126.0)],
            r##"{"color":"#FF9600"}"##,
        ),
        (
            DrawingKind::SimpleAnnotation,
            vec![point(92.0, 124.0)],
            r##"{"color":"#1677FF","text":"Swing low"}"##,
        ),
        (
            DrawingKind::SimpleTag,
            vec![point(60.0, 130.0)],
            r##"{"color":"#01C5C4","text":"Support"}"##,
        ),
    ]
}

/// A rise, a fall, and a recovery, so every tool has swings to sit on.
fn chart() -> ChartEngine {
    let mut seed = 7u64;
    let mut random = || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let (mut times, mut open, mut high, mut low, mut close) =
        (vec![], vec![], vec![], vec![], vec![]);
    let mut price = 120.0;
    for bar in 0..BARS {
        let drift = match bar {
            0..=50 => 0.45,
            51..=92 => -0.45,
            _ => 0.3,
        };
        let o = price + (random() - 0.5);
        let c = o + drift + (random() - 0.5) * 2.5;
        times.push(1_735_689_600.0 + bar as f64 * 86_400.0);
        open.push(o);
        high.push(o.max(c) + random() * 1.2);
        low.push(o.min(c) - random() * 1.2);
        close.push(c);
        price = c;
    }
    let mut engine = ChartEngine::new(WIDTH, HEIGHT, 1.0);
    engine.set_theme(ChartTheme::Light);
    engine
        .set_series_data(0, &times, &open, &high, &low, &close)
        .expect("candles");
    engine.pane_w = WIDTH - PRICE_AXIS_WIDTH;
    engine.axis_w = PRICE_AXIS_WIDTH;
    engine.pane_h = HEIGHT - engine.time_axis_height();
    engine.layout_panes(engine.pane_h);
    engine.time_scale.set_width(engine.pane_w);
    engine.fit_content();
    engine
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "klinechart_drawings".to_owned());
    std::fs::create_dir_all(&dir).expect("output directory");
    let tools = tools();
    let columns = 3u32;
    let (thumb_w, thumb_h) = (480u32, 267u32);
    let rows = (tools.len() as u32).div_ceil(columns);
    let mut sheet = Pixmap::new(thumb_w * columns, thumb_h * rows).expect("sheet");
    sheet.fill(tiny_skia::Color::WHITE);
    for (index, (kind, points, options)) in tools.into_iter().enumerate() {
        let mut engine = chart();
        let id = engine
            .add_drawing(kind, 0, points, Some(options))
            .expect("valid drawing");
        // Selected, so the anchors show.
        engine.set_selected_drawing(Some(id));
        let mut canvas = render_engine(&mut engine);
        canvas.fill_text(
            kind.name(),
            10.0,
            14.0,
            "600 13px sans-serif",
            Color::rgb(0x33, 0x33, 0x33),
            TextAlign::Left,
        );
        let path = format!("{dir}/drawing_{}.png", kind.name());
        canvas.save_png(&path).expect("save png");
        println!("wrote {path}");

        let pixmap = canvas.pixmap();
        let (column, row) = (index as u32 % columns, index as u32 / columns);
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
    let path = format!("{dir}/drawings_sheet.png");
    sheet.save_png(&path).expect("save sheet");
    println!("wrote {path}");
}
