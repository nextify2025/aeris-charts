//! Reference pixels for the browser's 2x2-to-20x20 image parity case.

use std::sync::Arc;

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{Prim, RasterImage};

fn main() {
    let output = std::env::args().nth(1).expect("expected output PNG path");
    let image = RasterImage {
        key: 1,
        width: 2,
        height: 2,
        pixels: Arc::from([
            255, 0, 0, 255, 0, 0, 255, 255, 0, 255, 0, 255, 255, 255, 255, 255,
        ]),
    };
    aeris_charts_native::render_prims(
        64,
        64,
        Color::rgb(255, 255, 255),
        &[Prim::Image {
            image,
            rect: [22.0, 22.0, 20.0, 20.0],
            opacity: 0.72,
        }],
        &[],
    )
    .save_png(&output)
    .expect("native image fixture saved");
}
