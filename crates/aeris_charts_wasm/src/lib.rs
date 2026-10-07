//! aeris_charts_wasm — browser host shell.
//!
//! Phase 0: a real chart pipeline end to end — `aeris_charts_core` scales -> candle geometry ->
//! WebGPU quads. The JS side (packages/charts, examples/web_demo) owns DOM events and calls
//! the exported gesture methods; rendering happens on demand via `render()`.

#[cfg(any(target_arch = "wasm32", test))]
mod backend_policy;
#[cfg(any(target_arch = "wasm32", test))]
mod color_policy;
// Pure JSON→Prim decoder for pane-primitive command buffers (plugin platform Phase C-a);
// compiled for the host target too so its tests run outside the browser.
#[cfg(any(target_arch = "wasm32", test))]
mod prim_decode;
// Pure custom-series item alignment (plugin platform Phase C-c); compiled for the host target
// too so its tests run outside the browser.
#[cfg(any(target_arch = "wasm32", test))]
mod custom_align;
// Pure text-run cache + placement math for the WebGPU `Prim::Text` path; compiled for the host
// target too so the LRU/placement tests run outside the browser.
#[cfg(any(target_arch = "wasm32", test))]
mod text_cache;
// Rolling last-frame telemetry behind `chart_api.frame_stats()`; compiled for the host target
// too so the record/encoding tests run outside the browser.
#[cfg(any(target_arch = "wasm32", test))]
mod telemetry;
// Pure ring-buffer layout/drain math for `series_api.set_ring_source`; compiled for the host
// target too, because wrap, overrun and cursor-overflow are the cases worth testing off-browser.
#[cfg(any(target_arch = "wasm32", test))]
mod ring_source;
// Session slot generation boundary (JSON request → engine); host-testable.
#[cfg(any(target_arch = "wasm32", test))]
mod session_slots;
// Redundant stroke-setter elimination for the browser Canvas2D target; compiled for the host
// target too so its state rules are tested outside the browser.
#[cfg(any(target_arch = "wasm32", test))]
mod stroke_state;

#[cfg(target_arch = "wasm32")]
mod canvas2d_target;
#[cfg(target_arch = "wasm32")]
mod chart;
#[cfg(target_arch = "wasm32")]
mod workspace;

#[cfg(target_arch = "wasm32")]
pub use chart::{AerisChart, create_chart};
#[cfg(target_arch = "wasm32")]
pub use workspace::AerisWorkspace;

#[cfg(target_arch = "wasm32")]
pub use smoke::render_prim_smoke_2d;

/// A tiny exported entry point that renders a handful of prims through the web-sys [`Canvas2d`]
/// target — the browser-side proof that the Prim-IR executor drives a real 2D canvas. Exercised
/// from the demo/tests via `getImageData`; not part of the public chart API.
#[cfg(target_arch = "wasm32")]
mod smoke {
    use crate::canvas2d_target::WasmCanvas2d;
    use aeris_charts_render::canvas2d::{Viewport, execute};
    use aeris_charts_render::color::Color;
    use aeris_charts_render::draw_list::{Gradient, IRect, LineStyle, LineType, Prim};
    use wasm_bindgen::prelude::*;
    use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

    #[wasm_bindgen]
    pub fn render_prim_smoke_2d(canvas: &HtmlCanvasElement) -> Result<(), JsValue> {
        let ctx = canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("no 2d context"))?
            .dyn_into::<CanvasRenderingContext2d>()?;
        let (w, h) = (canvas.width() as f32, canvas.height() as f32);

        let points = [[20.0f32, 80.0], [50.0, 30.0], [80.0, 60.0], [110.0, 20.0]];
        let prims = [
            Prim::Background {
                rect: [0.0, 0.0, w, h],
                gradient: Gradient {
                    top: Color::rgb(0xff, 0xff, 0xff),
                    bottom: Color::rgb(0xe8, 0xf0, 0xff),
                },
            },
            Prim::Rect {
                rect: IRect {
                    x: 10,
                    y: 10,
                    w: 40,
                    h: 40,
                },
                color: Color::rgb(0xff, 0x00, 0x00),
            },
            Prim::Circle {
                cx: 150.0,
                cy: 40.0,
                radius: 15.0,
                fill: Color::rgb(0x00, 0x00, 0xff),
                stroke_width: 0.0,
                stroke: Color::rgb(0, 0, 0),
            },
            Prim::Polyline {
                first_point: 0,
                point_count: 4,
                width: 3.0,
                style: LineStyle::Solid,
                line_type: LineType::Simple,
                color: Color::rgb(0x00, 0x99, 0x00),
            },
        ];
        let mut target = WasmCanvas2d::new(&ctx);
        execute(
            &prims,
            &points,
            &mut target,
            Viewport {
                width: w,
                height: h,
            },
        );
        Ok(())
    }
}
