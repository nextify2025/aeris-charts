//! Deterministic pixel-parity fixtures, shared by the capture harness and the parity tests.
//!
//! These exist so the *same* `Prim` list can be rendered by official GPUI (through
//! [`crate::backend`]) and by `aeris_charts_native`'s tiny-skia rasterizer, and the two rasterizations
//! compared pixel for pixel.
//!
//! Every fixture is built parametrically from a device-pixel-ratio so the *same* logical scene can
//! be emitted at any DPR: the engine bakes the DPR into `Prim` coordinates, so a fixture must too.
//! No RNG, no clock — byte-identical across runs and machines.
//!
//! Fixtures are split by *cause* rather than by feature, because the point is to attribute any
//! residual: geometry that both backends fill as axis-aligned rectangles is separable from
//! antialiased triangle edges, which is separable from gradients, which is separable from glyphs.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{
    Gradient, IRect, LineStyle, LineType, Prim, RasterImage, TextAlign,
};
use std::sync::Arc;

/// One fixture: a prim layer, its point pool, and the device-pixel size it covers.
pub struct Fixture {
    pub name: &'static str,
    /// What a residual here would be attributable to.
    pub attribution: &'static str,
    pub prims: Vec<Prim>,
    pub points: Vec<[f32; 2]>,
    pub width: u32,
    pub height: u32,
    /// The opaque background both rasterizers start from, so "unpainted" is well defined.
    pub background: Color,
}

/// Logical size every fixture covers, before the DPR is applied.
pub const LOGICAL_W: f32 = 480.0;
pub const LOGICAL_H: f32 = 300.0;

const BG: Color = Color::rgb(0xff, 0xff, 0xff);
const GRID: Color = Color::rgb(0xd6, 0xdc, 0xde);
const UP: Color = Color::rgb(0x26, 0xa6, 0x9a);
const DOWN: Color = Color::rgb(0xef, 0x53, 0x50);
const LINE: Color = Color::rgb(0x21, 0x96, 0xf3);
const INK: Color = Color::rgb(0x13, 0x17, 0x22);

fn dims(dpr: f32) -> (u32, u32) {
    (
        (LOGICAL_W * dpr).round().max(1.0) as u32,
        (LOGICAL_H * dpr).round().max(1.0) as u32,
    )
}

/// Round to a device-pixel integer, the way the engine's crisp-rect math does.
fn d(v: f32, dpr: f32) -> i32 {
    (v * dpr).round() as i32
}

/// **Crisp rects only.** Every prim here is filled by both backends as a solid axis-aligned
/// rectangle with no antialiasing, so a residual would mean a genuine coordinate, colour, or
/// coverage disagreement — not a rasterizer style difference. This is the fixture that can
/// legitimately be held to zero differing pixels.
pub fn crisp_rects(dpr: f32) -> Fixture {
    let (w, h) = dims(dpr);
    let mut prims = Vec::new();

    // Opaque background as a plain rect (not a gradient — gradients are a separate fixture).
    prims.push(Prim::Rect {
        rect: IRect {
            x: 0,
            y: 0,
            w: w as i32,
            h: h as i32,
        },
        color: BG,
    });
    // Grid: horizontal and vertical, odd and even widths, solid / dotted / dashed.
    for i in 1..6 {
        prims.push(Prim::HLine {
            y: d(i as f32 * 50.0, dpr),
            x0: 0,
            x1: w as i32,
            width: if i == 3 { 2 } else { 1 },
            style: match i {
                2 => LineStyle::Dotted,
                4 => LineStyle::Dashed,
                _ => LineStyle::Solid,
            },
            color: GRID,
        });
    }
    for i in 1..9 {
        prims.push(Prim::VLine {
            x: d(i as f32 * 50.0, dpr),
            y0: 0,
            y1: h as i32,
            width: if i == 5 { 3 } else { 1 },
            style: if i == 7 {
                LineStyle::Dashed
            } else {
                LineStyle::Solid
            },
            color: GRID,
        });
    }
    // Candles: a wick VLine plus a body Rect each, alternating colour.
    let bodies = [
        (40.0f32, 120.0f32, 60.0f32, true),
        (90.0, 150.0, 40.0, false),
        (140.0, 100.0, 70.0, true),
        (190.0, 130.0, 50.0, false),
        (240.0, 90.0, 55.0, true),
        (290.0, 140.0, 45.0, false),
    ];
    for (cx, top, body_h, is_up) in bodies {
        let color = if is_up { UP } else { DOWN };
        prims.push(Prim::VLine {
            x: d(cx, dpr),
            y0: d(top - 25.0, dpr),
            y1: d(top + body_h + 25.0, dpr),
            width: 1,
            style: LineStyle::Solid,
            color,
        });
        prims.push(Prim::Rect {
            rect: IRect {
                x: d(cx - 8.0, dpr),
                y: d(top, dpr),
                w: d(16.0, dpr),
                h: d(body_h, dpr),
            },
            color,
        });
    }
    // A hollow frame (RectFrame), border scaled with the DPR.
    prims.push(Prim::RectFrame {
        rect: IRect {
            x: d(350.0, dpr),
            y: d(30.0, dpr),
            w: d(100.0, dpr),
            h: d(60.0, dpr),
        },
        border: d(2.0, dpr).max(1),
        color: INK,
    });

    Fixture {
        name: "crisp_rects",
        attribution: "coordinates, colour, and solid-rect coverage",
        prims,
        points: Vec::new(),
        width: w,
        height: h,
        background: BG,
    }
}

/// **Antialiased triangle geometry.** A polyline stroke, a batch of one-bar segments, a band fill, a
/// disc, a triangle, and a rounded rect. Both GPUI and WebGPU consume Aeris's triangles; Canvas2D and tiny-skia describe
/// the same shapes analytically and antialias them. A residual here is expected and is attributable
/// to edge antialiasing, not to geometry.
pub fn tessellated(dpr: f32) -> Fixture {
    let (w, h) = dims(dpr);
    let mut prims = Vec::new();
    let mut points: Vec<[f32; 2]> = Vec::new();

    prims.push(Prim::Rect {
        rect: IRect {
            x: 0,
            y: 0,
            w: w as i32,
            h: h as i32,
        },
        color: BG,
    });

    // A polyline across the whole fixture.
    let first = points.len() as u32;
    let n = 24;
    for i in 0..n {
        let t = i as f32 / (n - 1) as f32;
        points.push([
            (20.0 + t * 440.0) * dpr,
            (150.0 - (t * 6.0).sin() * 70.0) * dpr,
        ]);
    }
    prims.push(Prim::Polyline {
        first_point: first,
        point_count: n,
        width: 2.0 * dpr,
        style: LineStyle::Solid,
        line_type: LineType::Simple,
        color: LINE,
    });

    // A batch of touching one-bar segments at stepping heights, like a period-reset study drawn
    // on daily bars. The pairs share boundary columns, where a single-path stroke (Canvas2D)
    // unions coverage and a per-pair mesh (GPUI, WebGPU) composites twice.
    let first = points.len() as u32;
    let bars = 10u32;
    for bar in 0..bars {
        let x = 20.0 + bar as f32 * 44.0;
        let y = 36.0 + (bar % 4) as f32 * 7.0;
        points.push([x * dpr, y * dpr]);
        points.push([(x + 44.0) * dpr, y * dpr]);
    }
    prims.push(Prim::Segments {
        first_point: first,
        segment_count: bars,
        width: 3.0 * dpr,
        color: DOWN,
    });

    // A solid band fill between two edges over a shared x sequence.
    let upper_first = points.len() as u32;
    for i in 0..12 {
        let t = i as f32 / 11.0;
        points.push([(20.0 + t * 440.0) * dpr, (230.0 - t * 20.0) * dpr]);
    }
    let lower_first = points.len() as u32;
    for i in 0..12 {
        let t = i as f32 / 11.0;
        points.push([(20.0 + t * 440.0) * dpr, (270.0 - t * 10.0) * dpr]);
    }
    prims.push(Prim::BandFill {
        line_type: LineType::Simple,
        upper_first,
        lower_first,
        point_count: 12,
        fill: Color::rgb(0x8e, 0x9b, 0xb0),
    });

    prims.push(Prim::Circle {
        cx: 400.0 * dpr,
        cy: 200.0 * dpr,
        radius: 18.0 * dpr,
        fill: UP,
        stroke_width: 0.0,
        stroke: UP,
    });
    prims.push(Prim::Triangle {
        a: [60.0 * dpr, 280.0 * dpr],
        b: [100.0 * dpr, 280.0 * dpr],
        c: [80.0 * dpr, 245.0 * dpr],
        color: DOWN,
    });
    prims.push(Prim::RoundRect {
        x: 150.0 * dpr,
        y: 250.0 * dpr,
        w: 90.0 * dpr,
        h: 34.0 * dpr,
        radii: [4.0 * dpr; 4],
        fill: Color::rgb(0xe8, 0xec, 0xf2),
        border_width: 0.0,
        border_color: INK,
    });
    prims.push(Prim::RoundRect {
        x: 255.0 * dpr,
        y: 250.0 * dpr,
        w: 90.0 * dpr,
        h: 34.0 * dpr,
        radii: [6.0 * dpr; 4],
        fill: Color::rgb(0xe8, 0xec, 0xf2),
        border_width: 2.0 * dpr,
        border_color: INK,
    });

    Fixture {
        name: "tessellated",
        attribution: "triangle-edge antialiasing",
        prims,
        points,
        width: w,
        height: h,
        background: BG,
    }
}

/// **Curved brush strokes.** Sparse and dense point streams at thin through heavy brush widths.
/// Every stroke keeps `LineType::Curved`, so GPUI and the reference rasterizer consume the shared
/// curved-line expansion rather than a backend-specific smoothing path.
pub fn curved_brushes(dpr: f32) -> Fixture {
    let (w, h) = dims(dpr);
    let mut prims = vec![Prim::Rect {
        rect: IRect {
            x: 0,
            y: 0,
            w: w as i32,
            h: h as i32,
        },
        color: BG,
    }];
    let mut points = Vec::new();

    for (y_offset, width, color) in [(0.0, 1.0, LINE), (72.0, 6.0, DOWN)] {
        let first_point = points.len() as u32;
        for [x, y] in [
            [20.0, 42.0],
            [92.0, 18.0],
            [176.0, 62.0],
            [270.0, 24.0],
            [360.0, 66.0],
            [460.0, 36.0],
        ] {
            points.push([x * dpr, (y + y_offset) * dpr]);
        }
        prims.push(Prim::Polyline {
            first_point,
            point_count: 6,
            width: width * dpr,
            style: LineStyle::Solid,
            line_type: LineType::Curved,
            color,
        });
    }

    for (y_offset, width, color) in [(198.0, 2.0, UP), (260.0, 4.0, INK)] {
        let first_point = points.len() as u32;
        let point_count = 96;
        for i in 0..point_count {
            let t = i as f32 / (point_count - 1) as f32;
            points.push([
                (20.0 + t * 440.0) * dpr,
                (y_offset + (t * 19.0).sin() * 20.0 + (t * 7.0).cos() * 8.0) * dpr,
            ]);
        }
        prims.push(Prim::Polyline {
            first_point,
            point_count,
            width: width * dpr,
            style: LineStyle::Solid,
            line_type: LineType::Curved,
            color,
        });
    }

    Fixture {
        name: "curved_brushes",
        attribution: "shared curved-line expansion across brush density and width",
        prims,
        points,
        width: w,
        height: h,
        background: BG,
    }
}

/// **Gradients.** A `Background` vertical ramp and an `AreaFill`. A residual here is attributable to
/// gradient interpolation (GPUI interpolates in its own colour space; Canvas2D and tiny-skia
/// interpolate premultiplied sRGB).
pub fn gradients(dpr: f32) -> Fixture {
    let (w, h) = dims(dpr);
    let mut prims = Vec::new();
    let mut points: Vec<[f32; 2]> = Vec::new();

    prims.push(Prim::Background {
        rect: [0.0, 0.0, w as f32, h as f32],
        gradient: Gradient {
            top: Color::rgb(0xff, 0xff, 0xff),
            bottom: Color::rgb(0xd8, 0xe4, 0xff),
        },
    });

    let first = points.len() as u32;
    let n = 20;
    for i in 0..n {
        let t = i as f32 / (n - 1) as f32;
        points.push([
            (20.0 + t * 440.0) * dpr,
            (120.0 - (t * 5.0).cos() * 50.0) * dpr,
        ]);
    }
    prims.push(Prim::AreaFill {
        first_point: first,
        point_count: n,
        base_y: 260.0 * dpr,
        line_type: LineType::Simple,
        gradient: Gradient {
            top: Color::rgba(0x2e, 0xdc, 0x87, 0xcc),
            bottom: Color::rgba(0x28, 0xdd, 0x64, 0x18),
        },
    });

    Fixture {
        name: "gradients",
        attribution: "gradient interpolation",
        prims,
        points,
        width: w,
        height: h,
        background: BG,
    }
}

/// **Text.** Axis-style labels in each alignment and weight. A residual here is attributable to
/// glyph rasterization: GPUI uses DirectWrite, `aeris_charts_native` uses the host system UI sans
/// through `ab_glyph`, and the browser uses its own engine. These are different rasterizers by construction.
pub fn text(dpr: f32) -> Fixture {
    let (w, h) = dims(dpr);
    let mut prims = vec![Prim::Rect {
        rect: IRect {
            x: 0,
            y: 0,
            w: w as i32,
            h: h as i32,
        },
        color: BG,
    }];
    let labels: [(f32, f32, &str, TextAlign, u16); 5] = [
        (470.0, 40.0, "1234.50", TextAlign::Right, 400),
        (240.0, 90.0, "09:30", TextAlign::Center, 400),
        (10.0, 140.0, "Aeris", TextAlign::Left, 700),
        (10.0, 190.0, "-12.75%", TextAlign::Left, 400),
        (240.0, 240.0, "1.2345e-8", TextAlign::Center, 400),
    ];
    for (x, y, s, align, weight) in labels {
        prims.push(Prim::Text {
            x: x * dpr,
            y: y * dpr,
            text: s.to_string(),
            color: INK,
            size: 12.0 * dpr,
            family: "sans-serif".into(),
            align,
            weight,
            italic: false,
        });
    }
    Fixture {
        name: "text",
        attribution: "glyph rasterization",
        prims,
        points: Vec::new(),
        width: w,
        height: h,
        background: BG,
    }
}

/// **Translucent axis-aligned rects.** Isolates *alpha compositing* from antialiasing: every prim is
/// a pixel-aligned rect, so no edge is partially covered, but each is drawn with alpha < 255 over an
/// opaque base and over its neighbours.
///
/// This exists to answer a specific question rather than to cover a feature: is a residual on
/// blended geometry caused by the blend arithmetic (GPU float versus tiny-skia's premultiplied `u8`)
/// or by edge coverage? If this fixture is exact, alpha compositing is not a source of irreducible
/// difference and only antialiasing is.
pub fn translucent_rects(dpr: f32) -> Fixture {
    let (w, h) = dims(dpr);
    let mut prims = vec![Prim::Rect {
        rect: IRect {
            x: 0,
            y: 0,
            w: w as i32,
            h: h as i32,
        },
        color: BG,
    }];
    // A ladder of alphas, plus deliberately overlapping rects so the blend is applied twice.
    for (i, alpha) in [0x20u8, 0x40, 0x60, 0x80, 0xa0, 0xc0, 0xe0]
        .iter()
        .enumerate()
    {
        let x = 20.0 + i as f32 * 62.0;
        prims.push(Prim::Rect {
            rect: IRect {
                x: d(x, dpr),
                y: d(30.0, dpr),
                w: d(50.0, dpr),
                h: d(90.0, dpr),
            },
            color: Color::rgba(UP.r(), UP.g(), UP.b(), *alpha),
        });
        // Overlaps the one above, so the same colour composites onto itself.
        prims.push(Prim::Rect {
            rect: IRect {
                x: d(x + 25.0, dpr),
                y: d(80.0, dpr),
                w: d(50.0, dpr),
                h: d(90.0, dpr),
            },
            color: Color::rgba(DOWN.r(), DOWN.g(), DOWN.b(), *alpha),
        });
    }
    // Translucent lines, still pixel-aligned.
    for i in 1..5 {
        prims.push(Prim::HLine {
            y: d(200.0 + i as f32 * 20.0, dpr),
            x0: 0,
            x1: w as i32,
            width: if i % 2 == 0 { 2 } else { 1 },
            style: LineStyle::Solid,
            color: Color::rgba(0x21, 0x96, 0xf3, 0x70),
        });
    }
    Fixture {
        name: "translucent",
        attribution: "alpha compositing, no antialiasing",
        prims,
        points: Vec::new(),
        width: w,
        height: h,
        background: BG,
    }
}

/// **Opaque antialiased geometry.** The mirror of [`translucent_rects`]: isolates *antialiasing* from
/// alpha compositing. Every prim is fully opaque, but all are non-axis-aligned shapes whose edges a
/// coverage-based rasterizer antialiases. GPUI strokes/discs/rings carry their own 1 px Loop-Blinn
/// coverage transition, so both sides antialias; the residual difference is the two rasterizers'
/// edge-coverage ramps. Filled triangles carry a GPUI fringe for the 1x MSAA path.
pub fn opaque_aa(dpr: f32) -> Fixture {
    let (w, h) = dims(dpr);
    let mut prims = vec![Prim::Rect {
        rect: IRect {
            x: 0,
            y: 0,
            w: w as i32,
            h: h as i32,
        },
        color: BG,
    }];
    let mut points: Vec<[f32; 2]> = Vec::new();

    // Opaque diagonal polyline.
    let first = points.len() as u32;
    let n = 16;
    for i in 0..n {
        let t = i as f32 / (n - 1) as f32;
        points.push([(20.0 + t * 440.0) * dpr, (60.0 + t * 80.0) * dpr]);
    }
    prims.push(Prim::Polyline {
        first_point: first,
        point_count: n,
        width: 3.0 * dpr,
        style: LineStyle::Solid,
        line_type: LineType::Simple,
        color: LINE,
    });
    // Opaque discs and triangles.
    for i in 0..4 {
        prims.push(Prim::Circle {
            cx: (70.0 + i as f32 * 110.0) * dpr,
            cy: 210.0 * dpr,
            radius: 22.0 * dpr,
            fill: if i % 2 == 0 { UP } else { DOWN },
            stroke_width: 0.0,
            stroke: UP,
        });
        prims.push(Prim::Triangle {
            a: [(40.0 + i as f32 * 110.0) * dpr, 285.0 * dpr],
            b: [(90.0 + i as f32 * 110.0) * dpr, 285.0 * dpr],
            c: [(65.0 + i as f32 * 110.0) * dpr, 250.0 * dpr],
            color: INK,
        });
    }
    Fixture {
        name: "opaque_aa",
        attribution: "antialiasing, no alpha compositing",
        prims,
        points,
        width: w,
        height: h,
        background: BG,
    }
}

/// The original SVG image through the shared axis converter.
pub fn crosshair_action(dpr: f32) -> Fixture {
    use aeris_charts_engine::{AxisFrame, AxisIcon, ChartEngine};
    let (width, height) = dims(dpr);
    let engine = ChartEngine::new(LOGICAL_W as f64, LOGICAL_H as f64, dpr as f64);
    let axis = AxisFrame {
        crosshair_action_icon: Some(AxisIcon {
            x: 40.0 - 19.0 * 0.9 / 2.0,
            y: 40.0 - 19.0 * 0.9 / 2.0,
            side: 19.0 * 0.9,
            image: aeris_charts_render::crosshair_icon::crosshair_icon(
                (19.0 * 0.9 * dpr).round() as u32
            ),
        }),
        ..AxisFrame::default()
    };
    let mut prims = Vec::new();
    engine.build_axis_primitives_into(&axis, &mut prims);
    // The icon paints last above axis chrome.
    let mut icon = prims.split_off(prims.len() - 1);
    icon.insert(
        0,
        Prim::Rect {
            rect: IRect {
                x: 0,
                y: 0,
                w: width as i32,
                h: height as i32,
            },
            color: Color::rgb(0x13, 0x13, 0x13),
        },
    );
    Fixture {
        name: "crosshair_action",
        attribution: "original SVG stroke weight and pixel alignment",
        prims: icon,
        points: Vec::new(),
        width,
        height,
        background: Color::rgb(0x13, 0x13, 0x13),
    }
}

/// Asymmetric red/blue image pixels reveal an RGBA/BGRA executor mismatch.
pub fn colored_image(dpr: f32) -> Fixture {
    let (width, height) = dims(dpr);
    let image = RasterImage {
        key: 0x00c0_10a1,
        width: 2,
        height: 2,
        pixels: Arc::from([
            255, 0, 0, 255, 0, 0, 255, 255, 0, 128, 128, 255, 255, 128, 0, 255,
        ]),
    };
    Fixture {
        name: "colored_image",
        attribution: "image RGBA-to-BGRA channel conversion",
        prims: vec![Prim::Image {
            image,
            rect: [40.0 * dpr, 40.0 * dpr, 2.0, 2.0],
            opacity: 1.0,
        }],
        points: Vec::new(),
        width,
        height,
        background: BG,
    }
}

/// Upscaled image checks GPUI's sprite filter and opacity against native bilinear rendering.
pub fn scaled_colored_image(dpr: f32) -> Fixture {
    let (width, height) = dims(dpr);
    Fixture {
        name: "scaled_colored_image",
        attribution: "bilinear image sampling and opacity",
        prims: vec![Prim::Image {
            image: RasterImage {
                key: 0x00c0_10a2,
                width: 2,
                height: 2,
                pixels: Arc::from([
                    255, 0, 0, 255, 0, 0, 255, 255, 0, 255, 0, 255, 255, 255, 255, 255,
                ]),
            },
            rect: [40.25 * dpr, 40.75 * dpr, 20.0 * dpr, 20.0 * dpr],
            opacity: 0.72,
        }],
        points: Vec::new(),
        width,
        height,
        background: BG,
    }
}

/// Engine-produced bid/ask heatmap pixels, drawn at source resolution to isolate channel order.
pub fn depth_heatmap_colors(dpr: f32) -> Fixture {
    use aeris_charts_engine::{
        ChartEngine, DepthHeatmapOptions, DepthLevel, DepthOptions, DepthSnapshot,
    };
    let (width, height) = dims(dpr);
    let mut engine = ChartEngine::new(LOGICAL_W as f64, LOGICAL_H as f64, dpr as f64);
    let times = [0.0, 1.0, 2.0];
    let prices = [100.0; 3];
    engine
        .set_series_data(0, &times, &prices, &prices, &prices, &prices)
        .expect("valid heatmap time axis");
    let stream = engine
        .add_depth_stream("parity-depth", DepthOptions::default())
        .expect("valid depth stream");
    engine
        .set_depth_snapshot(
            stream,
            DepthSnapshot {
                timestamp_micros: 0,
                sequence: 1,
                bids: vec![DepthLevel {
                    price: 99.0,
                    size: 500.0,
                    order_count: Some(1),
                }],
                asks: vec![DepthLevel {
                    price: 101.0,
                    size: 750.0,
                    order_count: Some(1),
                }],
            },
        )
        .expect("valid depth snapshot");
    engine
        .add_depth_heatmap(
            stream,
            DepthHeatmapOptions {
                price_min: 99.0,
                price_max: 101.0,
                ..DepthHeatmapOptions::default()
            },
        )
        .expect("valid depth heatmap");
    let content_height = (f64::from(LOGICAL_H) - engine.time_axis_height()).max(1.0);
    engine.layout_panes(content_height);
    engine.time_scale.set_width(f64::from(LOGICAL_W));
    engine.fit_content();
    let frame = engine.build_frame();
    let image = frame
        .panes
        .iter()
        .flat_map(|pane| pane.under.iter())
        .find_map(|prim| match prim {
            Prim::Image { image, .. } => Some(image.clone()),
            _ => None,
        })
        .expect("engine must emit a depth heatmap image");
    Fixture {
        name: "depth_heatmap_colors",
        attribution: "engine bid/ask heatmap color channels",
        prims: vec![Prim::Image {
            rect: [
                60.0 * dpr,
                60.0 * dpr,
                image.width as f32,
                image.height as f32,
            ],
            image,
            opacity: 1.0,
        }],
        points: Vec::new(),
        width,
        height,
        background: BG,
    }
}

/// Every fixture, in attribution order.
///
/// `translucent` and `opaque_aa` sit between the exact fixture and the mixed ones deliberately: they
/// separate the two independent causes of irreducible cross-rasterizer difference, so the report can
/// say which one is responsible rather than guessing.
pub fn all(dpr: f32) -> Vec<Fixture> {
    vec![
        crisp_rects(dpr),
        translucent_rects(dpr),
        translucent_joins(dpr),
        opaque_aa(dpr),
        tessellated(dpr),
        curved_brushes(dpr),
        gradients(dpr),
        text(dpr),
        crosshair_action(dpr),
        colored_image(dpr),
        scaled_colored_image(dpr),
        depth_heatmap_colors(dpr),
    ]
}

/// A half-alpha stroke with alternating sharp turns, exercising GPUI's shared join tessellation.
pub fn translucent_joins(dpr: f32) -> Fixture {
    let (width, height) = dims(dpr);
    let points = [
        (80.0, 220.0),
        (160.0, 80.0),
        (240.0, 220.0),
        (320.0, 80.0),
        (400.0, 220.0),
    ]
    .map(|(x, y)| [x * dpr, y * dpr]);
    Fixture {
        name: "translucent_joins",
        attribution: "half-alpha stroke coverage at sharp joins",
        prims: vec![Prim::Polyline {
            first_point: 0,
            point_count: points.len() as u32,
            width: 8.0 * dpr,
            style: LineStyle::Solid,
            line_type: LineType::Simple,
            color: Color::rgba(0, 0, 0, 128),
        }],
        points: points.to_vec(),
        width,
        height,
        background: BG,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fixture_is_non_empty_and_sized() {
        for dpr in [1.0f32, 1.25, 1.5, 2.0, 2.5] {
            for f in all(dpr) {
                assert!(!f.prims.is_empty(), "{} @ {dpr} is empty", f.name);
                assert!(f.width > 0 && f.height > 0);
                assert_eq!(f.width, (LOGICAL_W * dpr).round() as u32);
            }
        }
    }

    #[test]
    fn fixtures_are_deterministic() {
        for dpr in [1.0f32, 1.5] {
            let a = all(dpr);
            let b = all(dpr);
            for (x, y) in a.iter().zip(&b) {
                assert_eq!(x.prims, y.prims, "{} is not reproducible", x.name);
                assert_eq!(x.points, y.points);
            }
        }
    }

    #[test]
    fn curved_brush_fixture_covers_density_width_and_dpr_matrix() {
        for dpr in [1.0f32, 1.25, 1.5, 2.0, 2.5] {
            let fixture = curved_brushes(dpr);
            let strokes: Vec<_> = fixture
                .prims
                .iter()
                .filter_map(|prim| match prim {
                    Prim::Polyline {
                        point_count,
                        width,
                        line_type,
                        ..
                    } => Some((*point_count, *width, *line_type)),
                    _ => None,
                })
                .collect();
            assert_eq!(strokes.len(), 4);
            assert!(strokes.iter().all(|stroke| stroke.2 == LineType::Curved));
            assert_eq!(
                strokes.iter().map(|stroke| stroke.0).collect::<Vec<_>>(),
                [6, 6, 96, 96]
            );
            for (actual, logical) in strokes
                .iter()
                .map(|stroke| stroke.1)
                .zip([1.0f32, 6.0, 2.0, 4.0])
            {
                assert!((actual - logical * dpr).abs() <= f32::EPSILON);
            }
        }
    }

    #[test]
    fn point_pool_ranges_are_in_bounds() {
        for dpr in [1.0f32, 1.5, 2.5] {
            for f in all(dpr) {
                for prim in &f.prims {
                    let (first, count) = match prim {
                        Prim::Polyline {
                            first_point,
                            point_count,
                            ..
                        }
                        | Prim::AreaFill {
                            first_point,
                            point_count,
                            ..
                        } => (*first_point, *point_count),
                        Prim::BandFill {
                            lower_first,
                            point_count,
                            ..
                        } => (*lower_first, *point_count),
                        _ => continue,
                    };
                    assert!(
                        (first + count) as usize <= f.points.len(),
                        "{}: range {first}+{count} exceeds pool {}",
                        f.name,
                        f.points.len()
                    );
                }
            }
        }
    }

    /// The crisp-rect fixture must contain no antialiased prim, or its zero-diff claim is void.
    #[test]
    fn the_crisp_fixture_contains_only_crisp_prims() {
        for prim in &crisp_rects(1.5).prims {
            assert!(
                matches!(
                    prim,
                    Prim::Rect { .. }
                        | Prim::RectFrame { .. }
                        | Prim::HLine { .. }
                        | Prim::VLine { .. }
                ),
                "{prim:?} is not a crisp-rect prim"
            );
        }
    }

    /// `translucent` must contain only pixel-aligned rect-family prims, or it would not isolate
    /// alpha compositing from antialiasing.
    #[test]
    fn the_translucent_fixture_has_no_antialiased_prim() {
        for prim in &translucent_rects(1.5).prims {
            assert!(
                matches!(
                    prim,
                    Prim::Rect { .. }
                        | Prim::RectFrame { .. }
                        | Prim::HLine { .. }
                        | Prim::VLine { .. }
                ),
                "{prim:?} is not axis-aligned"
            );
        }
    }

    /// `translucent` must actually contain translucent prims (otherwise it proves nothing).
    #[test]
    fn the_translucent_fixture_is_actually_translucent() {
        let translucent = translucent_rects(1.5)
            .prims
            .iter()
            .filter(|p| match p {
                Prim::Rect { color, .. } | Prim::HLine { color, .. } => color.a() < 0xff,
                _ => false,
            })
            .count();
        assert!(translucent >= 15, "only {translucent} translucent prims");
    }

    /// `opaque_aa` must be fully opaque everywhere, or it would not isolate antialiasing.
    #[test]
    fn the_opaque_aa_fixture_is_fully_opaque() {
        for prim in &opaque_aa(1.5).prims {
            let alpha = match prim {
                Prim::Rect { color, .. }
                | Prim::Polyline { color, .. }
                | Prim::Triangle { color, .. } => color.a(),
                Prim::Circle { fill, .. } => fill.a(),
                other => panic!("unexpected prim in opaque_aa: {other:?}"),
            };
            assert_eq!(alpha, 0xff, "{prim:?} is not opaque");
        }
    }
}
