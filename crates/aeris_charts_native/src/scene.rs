//! A fixed, deterministic chart-like scene used by the example renderer and the golden test.
//! Kept in the library so the golden harness and the `scene` example render byte-identical output.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{
    Gradient, IRect, LineStyle, LineType, Prim, RasterImage, TextAlign,
};
use std::sync::Arc;

/// A self-contained prim layer plus its point pool and target size.
pub struct Scene {
    pub width: u32,
    pub height: u32,
    pub background: Color,
    pub prims: Vec<Prim>,
    pub points: Vec<[f32; 2]>,
}

/// Build the reference scene: background gradient, grid, six candlesticks, an area+line series,
/// a dashed price line, a circle marker, and the remaining frame-contract families. Uses only
/// deterministic math (no RNG / time) so geometry is reproducible for golden comparison.
pub fn demo_scene() -> Scene {
    let (w, h) = (480u32, 300u32);
    let grid = Color::rgb(0xe1, 0xe3, 0xea);
    let up = Color::rgb(0x26, 0xa6, 0x9a);
    let down = Color::rgb(0xef, 0x53, 0x50);
    let line_c = Color::rgb(0x21, 0x96, 0xf3);

    let mut prims: Vec<Prim> = Vec::new();
    prims.push(Prim::Background {
        rect: [0.0, 0.0, w as f32, h as f32],
        gradient: Gradient {
            top: Color::rgb(0xff, 0xff, 0xff),
            bottom: Color::rgb(0xf0, 0xf4, 0xff),
        },
    });
    for i in 1..6 {
        prims.push(Prim::HLine {
            y: i * 50,
            x0: 0,
            x1: w as i32,
            width: 1,
            style: LineStyle::Solid,
            color: grid,
        });
    }
    for i in 1..9 {
        prims.push(Prim::VLine {
            x: i * 50,
            y0: 0,
            y1: h as i32,
            width: 1,
            style: LineStyle::Solid,
            color: grid,
        });
    }
    let bodies = [
        (40, 120, 60, true),
        (90, 150, 40, false),
        (140, 100, 70, true),
        (190, 130, 50, false),
        (240, 90, 55, true),
        (290, 140, 45, false),
    ];
    for (cx, top, body_h, is_up) in bodies {
        let color = if is_up { up } else { down };
        prims.push(Prim::VLine {
            x: cx,
            y0: top - 25,
            y1: top + body_h + 25,
            width: 1,
            style: LineStyle::Solid,
            color,
        });
        prims.push(Prim::Rect {
            rect: IRect {
                x: cx - 8,
                y: top,
                w: 16,
                h: body_h,
            },
            color,
        });
    }
    let mut points: Vec<[f32; 2]> = (0..8)
        .map(|i| {
            [
                350.0 + i as f32 * 16.0,
                180.0 - (i as f32 * 1.3).sin() * 30.0,
            ]
        })
        .collect();
    let count = points.len() as u32;
    prims.push(Prim::AreaFill {
        first_point: 0,
        point_count: count,
        base_y: 260.0,
        line_type: LineType::Simple,
        gradient: Gradient {
            top: Color::rgba(0x21, 0x96, 0xf3, 0x80),
            bottom: Color::rgba(0x21, 0x96, 0xf3, 0x08),
        },
    });
    prims.push(Prim::Polyline {
        first_point: 0,
        point_count: count,
        width: 2.0,
        style: LineStyle::Solid,
        line_type: LineType::Simple,
        color: line_c,
    });
    prims.push(Prim::HLine {
        y: 210,
        x0: 0,
        x1: w as i32,
        width: 1,
        style: LineStyle::Dashed,
        color: down,
    });
    prims.push(Prim::Circle {
        cx: 240.0,
        cy: 90.0,
        radius: 6.0,
        fill: line_c,
        stroke_width: 2.0,
        stroke: Color::rgb(0xff, 0xff, 0xff),
    });

    prims.push(Prim::RoundRect {
        x: 335.0,
        y: 14.0,
        w: 105.0,
        h: 43.0,
        radii: [8.0; 4],
        fill: Color::rgba(0xe8, 0xec, 0xf2, 0xc0),
        border_width: 2.0,
        border_color: Color::rgb(0x13, 0x17, 0x22),
    });
    prims.push(Prim::Image {
        image: RasterImage {
            key: 0x5343_454e,
            width: 2,
            height: 2,
            pixels: Arc::from([
                255, 0, 0, 255, 0, 0, 255, 255, 0, 180, 100, 255, 255, 210, 0, 255,
            ]),
        },
        rect: [371.0, 19.0, 32.0, 32.0],
        opacity: 0.8,
    });
    prims.push(Prim::Triangle {
        a: [335.0, 80.0],
        b: [385.0, 80.0],
        c: [360.0, 119.0],
        color: down,
    });
    let upper_first = points.len() as u32;
    points.extend([
        [330.0, 222.0],
        [370.0, 216.0],
        [410.0, 233.0],
        [455.0, 220.0],
    ]);
    let lower_first = points.len() as u32;
    points.extend([
        [330.0, 258.0],
        [370.0, 252.0],
        [410.0, 265.0],
        [455.0, 254.0],
    ]);
    prims.push(Prim::BandFill {
        upper_first,
        lower_first,
        point_count: 4,
        line_type: LineType::Simple,
        fill: Color::rgba(0x21, 0x96, 0xf3, 0x80),
    });
    let dashed_first = points.len() as u32;
    points.extend([[20.0, 20.0], [105.0, 30.0], [195.0, 17.0], [285.0, 27.0]]);
    prims.push(Prim::Polyline {
        first_point: dashed_first,
        point_count: 4,
        width: 2.0,
        style: LineStyle::Dashed,
        line_type: LineType::Simple,
        color: line_c,
    });
    let stepped_first = points.len() as u32;
    points.extend([[20.0, 62.0], [105.0, 52.0], [195.0, 65.0], [285.0, 56.0]]);
    prims.push(Prim::Polyline {
        first_point: stepped_first,
        point_count: 4,
        width: 2.0,
        style: LineStyle::Dotted,
        line_type: LineType::WithSteps,
        color: down,
    });
    prims.push(Prim::Text {
        x: 14.0,
        y: 276.0,
        text: "Aeris".into(),
        color: Color::rgb(0x13, 0x17, 0x22),
        size: 18.0,
        family: "sans-serif".into(),
        align: TextAlign::Left,
        weight: 400,
        italic: false,
    });

    Scene {
        width: w,
        height: h,
        background: Color::rgb(0xff, 0xff, 0xff),
        prims,
        points,
    }
}
