//! Single-fixture companion to `pixel_parity` (which captures all fixtures headlessly on Linux
//! under X11 and on Windows through DWM): paints ONE fixture through the real GPUI pipeline and
//! holds it on screen so a person, or an external Wayland capture (`grim`), can look at exactly
//! what GPUI rasterized.
//!
//! ```text
//! AERIS_CHARTS_FIXTURE=curved_brushes cargo run -p aeris_charts_render_gpui \
//!     --features gpui-backend --example gpui_fixture_view
//! ```

use aeris_charts_render_gpui::{fixtures, AerisViewport, GpuiChartRenderer};
use gpui::{
    canvas, div, prelude::*, px, size, App, Bounds, Context, Entity, Render, Window, WindowBounds,
    WindowOptions,
};
use gpui_platform::application;

struct View {
    renderer: GpuiChartRenderer,
    fixture: fixtures::Fixture,
}

impl Render for View {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();
        let entity: Entity<View> = cx.entity();
        div().size_full().child(
            canvas(
                move |bounds: Bounds<gpui::Pixels>, _window, _cx| bounds,
                move |bounds: Bounds<gpui::Pixels>, _prep, window, cx| {
                    entity.update(cx, |v: &mut View, cx| {
                        let sf = window.scale_factor();
                        let prims = v.fixture.prims.clone();
                        let points = v.fixture.points.clone();
                        let viewport = AerisViewport::from_bounds(
                            bounds.origin.x.into(),
                            bounds.origin.y.into(),
                            bounds.size.width.into(),
                            bounds.size.height.into(),
                        );
                        v.renderer
                            .paint_prims(&prims, &points, viewport, sf, window, cx);
                    });
                },
            )
            .size_full(),
        )
    }
}

fn main() {
    let name = std::env::var("AERIS_CHARTS_FIXTURE").unwrap_or_else(|_| "curved_brushes".into());
    application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(
            None,
            size(px(fixtures::LOGICAL_W), px(fixtures::LOGICAL_H)),
            cx,
        );
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                ..Default::default()
            },
            |window, cx| {
                let scale = window.scale_factor();
                println!("fixture-view: window scale factor {scale}");
                let fixture = fixtures::all(scale)
                    .into_iter()
                    .find(|f| f.name == name)
                    .unwrap_or_else(|| panic!("unknown fixture {name}"));
                cx.new(|_| View {
                    renderer: GpuiChartRenderer::new(),
                    fixture,
                })
            },
        )
        .expect("the fixture window opens");
        cx.activate(true);
    });
}
