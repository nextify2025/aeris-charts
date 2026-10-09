//! Finite capture of the shared D1 engine pane through official GPUI.
//!
//! This is the GPUI half of `examples/web_demo/tests/gpui-webgpu-matrix.spec.mjs`.
//! It builds the same checked-in D1 fixture used by the browser, paints only the engine-owned pane
//! frame, captures the presented client area, writes metadata, and exits. The capture reads the
//! window through DWM on Windows (`tools/capture_window.ps1`) and from the X server on Linux
//! (`support/x11_capture.rs`, run under `xvfb-run` with `GPUI_X11_SCALE_FACTOR=1.5` and a virtual
//! screen larger than the 1851x1047 pane, for example `-screen 0 2560x1600x24`).
//!
//! Environment:
//! - `AERIS_CHARTS_GPUI_CAPTURE_OUT` — required output PNG path.
//! - `AERIS_CHARTS_GPUI_CAPTURE_METADATA` — optional metadata JSON path (defaults beside the PNG).

use std::path::{Path, PathBuf};
#[cfg(not(target_os = "linux"))]
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
#[cfg(not(target_os = "linux"))]
use std::time::{Duration, Instant};

use aeris_charts_engine::{
    AggressorSide, ChartEngine, ChartFrame, FootprintAggregationOptions, FootprintBarAggregation,
    FootprintImbalanceOptions, FootprintSeriesOptions, FootprintTrade,
};
use aeris_charts_render_gpui::{AerisViewport, GpuiChartRenderer, Paint, PreparedAerisFrame};
use gpui::{
    App, Bounds, Context, Entity, Render, Window, WindowBounds, WindowOptions, canvas, div,
    prelude::*, px, size,
};
use gpui_platform::application;

#[cfg(target_os = "linux")]
#[path = "support/x11_capture.rs"]
mod x11_capture;

const WINDOW_TITLE_PREFIX: &str = "aeris_charts-gpui-webgpu-pane-capture";
const WARMUP_FRAMES: u64 = 12;
#[cfg(not(target_os = "linux"))]
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
enum Phase {
    Warmup(u64),
    Capturing(Receiver<Result<String, String>>),
}

struct Capture {
    renderer: GpuiChartRenderer,
    frame: ChartFrame,
    output: PathBuf,
    metadata: PathBuf,
    phase: Phase,
    done: bool,
    scale_factor: f32,
    background: gpui::Hsla,
    frame_background: Paint,
    case_name: String,
    theme: String,
    spacing: Option<f64>,
    feature: String,
    window_title: String,
}

impl Capture {
    fn new(scale_factor: f32, output: PathBuf, metadata: PathBuf, window_title: String) -> Self {
        let fixture = aeris_charts_native::engine_scene::parity_fixture();
        let feature = std::env::var("AERIS_CHARTS_GPUI_FEATURE").unwrap_or_else(|_| "base".into());
        assert!(
            matches!(
                feature.as_str(),
                "base" | "markers" | "trading" | "footprint"
            ),
            "AERIS_CHARTS_GPUI_FEATURE must be base, markers, trading, or footprint"
        );
        if feature != "footprint" {
            assert!(
                (scale_factor as f64 - fixture.pixel_ratio).abs() <= 1e-4,
                "the shared D1 pixel gate requires physical GPUI DPR {}, but this monitor reports {}; run it on a Windows display configured to {}%, or on Linux/X11 with GPUI_X11_SCALE_FACTOR={}",
                fixture.pixel_ratio,
                scale_factor,
                (fixture.pixel_ratio * 100.0).round(),
                fixture.pixel_ratio
            );
        }

        let theme = std::env::var("AERIS_CHARTS_GPUI_THEME").unwrap_or_else(|_| "light".into());
        assert!(
            matches!(theme.as_str(), "light" | "dark"),
            "AERIS_CHARTS_GPUI_THEME must be light or dark"
        );
        let spacing = std::env::var("AERIS_CHARTS_GPUI_BAR_SPACING")
            .ok()
            .map(|value| {
                value
                    .parse::<f64>()
                    .expect("AERIS_CHARTS_GPUI_BAR_SPACING must be numeric")
            });
        let mut engine = aeris_charts_native::engine_scene::parity_engine();
        if feature == "footprint" {
            install_footprint_fixture(&mut engine, fixture.end_time);
            engine.dpr = scale_factor as f64;
        }
        if theme == "dark" {
            let surface = aeris_charts_core::style::DARK_SURFACE_CSS;
            let text = aeris_charts_core::style::DARK_AXIS_TEXT_CSS;
            let border = aeris_charts_core::style::DARK_BORDER_CSS;
            let options = format!(
                r#"{{"layout":{{"background":{{"type":"solid","color":"{surface}"}},"textColor":"{text}"}},"grid":{{"vertLines":{{"color":"{border}"}},"horzLines":{{"color":"{border}"}}}}}}"#
            );
            engine
                .apply_options(&options)
                .expect("the built-in dark fixture options are valid");
        }
        if let Some(spacing) = spacing {
            engine.apply_bar_spacing_option(spacing);
            engine.apply_right_offset_option(0.0);
        }
        if feature == "markers" {
            use aeris_charts_engine::{Marker, marker_pos, marker_shape};
            use aeris_charts_render::color::Color;

            let start = fixture.end_time - (fixture.bar_count.saturating_sub(1) as i64) * 3_600;
            let marker = |index: usize, position, shape, color: &str, text: &str| Marker {
                time: start + index as i64 * 3_600,
                position,
                shape,
                color: Color::parse_css(color).expect("built-in marker color is valid"),
                text: text.into(),
                id: format!("capture-{index}"),
                size: 1.0,
                price: None,
            };
            engine.set_series_markers(
                0,
                vec![
                    marker(
                        840,
                        marker_pos::ABOVE,
                        marker_shape::ARROW_DOWN,
                        "#f7525f",
                        "SELL",
                    ),
                    marker(
                        880,
                        marker_pos::BELOW,
                        marker_shape::ARROW_UP,
                        "#089981",
                        "BUY",
                    ),
                    marker(
                        920,
                        marker_pos::IN_BAR,
                        marker_shape::CIRCLE,
                        "#7e57c2",
                        "MID",
                    ),
                    marker(
                        960,
                        marker_pos::ABOVE,
                        marker_shape::SQUARE,
                        "#2962ff",
                        "NOTE",
                    ),
                ],
            );
            engine.set_series_markers_auto_scale(0, true);
        }
        if feature == "trading" {
            aeris_charts_native::engine_scene::install_trading_fixture(&mut engine);
        }
        let background_color = aeris_charts_render::color::Color::parse_css(
            &engine.options.get().layout.background.color,
        )
        .expect("fixture background color is valid");
        let background = aeris_charts_render_gpui::backend::to_hsla(background_color);
        let frame_background = Paint::Solid(background_color);
        let frame = engine.build_frame();
        let expected_width = fixture.css_width - fixture.price_axis_width;
        let expected_height = fixture.css_height - fixture.time_axis_height;
        assert!(
            (frame.width - expected_width).abs() <= f64::EPSILON
                && (frame.height - expected_height).abs() <= f64::EPSILON,
            "shared fixture pane changed: frame={}x{}, expected={}x{}",
            frame.width,
            frame.height,
            expected_width,
            expected_height
        );
        let spacing_label = spacing.map_or_else(
            || (feature == "footprint").then_some("72".to_owned()),
            |value| Some(value.to_string().replace('.', "_")),
        );
        let spacing_label = spacing_label.as_deref().unwrap_or("fit");
        // The names the browser matrix spec selects and asserts, such as `dpr-1_5-spacing-fit-light-base`.
        let case_name = format!(
            "dpr-{}-spacing-{}-{}-{}",
            scale_factor.to_string().replace('.', "_"),
            spacing_label,
            theme,
            feature
        );

        Self {
            renderer: GpuiChartRenderer::new(),
            frame,
            output,
            metadata,
            phase: Phase::Warmup(0),
            done: false,
            scale_factor,
            background,
            frame_background,
            case_name,
            theme,
            spacing,
            feature,
            window_title,
        }
    }

    fn spawn_capture(
        &self,
        #[cfg(target_os = "linux")] window: &Window,
    ) -> Receiver<Result<String, String>> {
        let (tx, rx) = mpsc::channel();
        #[cfg(target_os = "linux")]
        {
            // The window id is only reachable from the UI thread; the X11 read itself is not.
            let window_id = x11_capture::window_id(window);
            let expected = (
                (self.frame.width * self.frame.pixel_ratio).round() as u32,
                (self.frame.height * self.frame.pixel_ratio).round() as u32,
            );
            let output = self.output.clone();
            let window_title = self.window_title.clone();
            std::thread::spawn(move || {
                let result = window_id
                    .and_then(|id| x11_capture::capture(id, expected, &output))
                    .map_err(|error| format!("capture of window {window_title:?} failed: {error}"));
                let _ = tx.send(result);
            });
        }
        #[cfg(not(target_os = "linux"))]
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/capture_window.ps1");
        #[cfg(not(target_os = "linux"))]
        let output = self.output.clone();
        #[cfg(not(target_os = "linux"))]
        let window_title = self.window_title.clone();
        #[cfg(not(target_os = "linux"))]
        std::thread::spawn(move || {
            let result = Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                    script.to_str().unwrap_or_default(),
                    "-Title",
                    &window_title,
                    "-Out",
                    output.to_str().unwrap_or_default(),
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|error| format!("failed to start capture helper: {error}"))
                .and_then(|mut child| {
                    let started = Instant::now();
                    loop {
                        match child.try_wait() {
                            Ok(Some(_)) => break,
                            Ok(None) if started.elapsed() < CAPTURE_TIMEOUT => {
                                std::thread::sleep(Duration::from_millis(20));
                            }
                            Ok(None) => {
                                let _ = child.kill();
                                let _ = child.wait();
                                return Err(format!(
                                    "capture helper exceeded the {} second deadline",
                                    CAPTURE_TIMEOUT.as_secs()
                                ));
                            }
                            Err(error) => {
                                let _ = child.kill();
                                let _ = child.wait();
                                return Err(format!("could not poll capture helper: {error}"));
                            }
                        }
                    }
                    child
                        .wait_with_output()
                        .map_err(|error| format!("could not collect capture helper output: {error}"))
                })
                .and_then(|result| {
                    let stdout = String::from_utf8_lossy(&result.stdout).trim().to_owned();
                    let stderr = String::from_utf8_lossy(&result.stderr).trim().to_owned();
                    if result.status.success() && stdout.starts_with("OK ") {
                        Ok(stdout)
                    } else {
                        Err(format!(
                            "capture helper failed (status={}): stdout={stdout:?}, stderr={stderr:?}",
                            result.status
                        ))
                    }
                });
            let _ = tx.send(result);
        });
        rx
    }

    fn finish(&self, capture_note: &str) -> Result<(), String> {
        let image = aeris_charts_native::load_png(self.output.to_str().unwrap_or_default())
            .map_err(|error| format!("could not decode GPUI capture: {error:?}"))?;
        let expected_width = (self.frame.width * self.frame.pixel_ratio).round() as u32;
        let expected_height = (self.frame.height * self.frame.pixel_ratio).round() as u32;
        if (image.width(), image.height()) != (expected_width, expected_height) {
            return Err(format!(
                "GPUI capture is {}x{}, expected {}x{} device pixels",
                image.width(),
                image.height(),
                expected_width,
                expected_height
            ));
        }

        let spacing_json = self.spacing.map_or_else(
            || (self.feature == "footprint").then_some("72".to_owned()),
            |value| Some(value.to_string()),
        );
        let metadata = format!(
            concat!(
                "{{\n",
                "  \"schema\": 1,\n",
                "  \"fixture\": \"{}\",\n",
                "  \"case\": \"{}\",\n",
                "  \"scope\": \"pane\",\n",
                "  \"theme\": \"{}\",\n",
                "  \"bar_spacing\": {},\n",
                "  \"feature\": \"{}\",\n",
                "  \"logical_width\": {},\n",
                "  \"logical_height\": {},\n",
                "  \"scale_factor\": {},\n",
                "  \"pixel_width\": {},\n",
                "  \"pixel_height\": {},\n",
                "  \"capture\": \"{}\"\n",
                "}}\n"
            ),
            if self.feature == "footprint" {
                "footprint-12bar-dense"
            } else {
                "candles-1000-default-light"
            },
            self.case_name,
            self.theme,
            spacing_json.as_deref().unwrap_or("null"),
            self.feature,
            self.frame.width,
            self.frame.height,
            self.scale_factor,
            image.width(),
            image.height(),
            capture_note.replace('"', "'")
        );
        std::fs::write(&self.metadata, metadata)
            .map_err(|error| format!("could not write capture metadata: {error}"))?;
        println!(
            "GPUI_PANE_CAPTURE_OK output={} metadata={} size={}x{} dpr={}",
            self.output.display(),
            self.metadata.display(),
            image.width(),
            image.height(),
            self.scale_factor
        );
        Ok(())
    }
}

fn install_footprint_fixture(engine: &mut ChartEngine, end_time: i64) {
    engine
        .options
        .apply_str(
            r##"{
                "layout":{"background":{"type":"solid","color":"#ffffff"},"textColor":"#191919"},
                "grid":{"vertLines":{"color":"#d6dcde","visible":true},"horzLines":{"color":"#d6dcde","visible":true}},
                "leftPriceScale":{"borderColor":"#2b2b43","textColor":"#191919"},
                "rightPriceScale":{"borderColor":"#2b2b43","textColor":"#191919"},
                "timeScale":{"borderColor":"#2b2b43"}
            }"##,
        )
        .expect("light footprint capture options are valid");
    let series_id = engine
        .add_footprint_series(FootprintSeriesOptions {
            aggregation: FootprintAggregationOptions {
                tick_size: 0.25,
                ticks_per_row: 1,
                bars: FootprintBarAggregation::Time {
                    interval_micros: 3_600_000_000,
                    anchor_micros: 0,
                },
                imbalance: FootprintImbalanceOptions {
                    ratio: 3.0,
                    minimum_volume: 20.0,
                    consecutive_levels: 3,
                },
            },
            visual: Default::default(),
        })
        .expect("footprint capture options are valid");
    let first = end_time - 11 * 3_600;
    let mut trades = Vec::with_capacity(12 * 11 * 2);
    let mut trade_id = 1_u64;
    for bar in 0..12 {
        let center = 400 + (bar as i64 % 3) - 1;
        for level in 0..11 {
            let price = (center + level as i64 - 5) as f64 * 0.25;
            let timestamp = (first + bar as i64 * 3_600) * 1_000_000 + level as i64 * 10_000;
            let volume = 10.0 + (level % 7) as f64 * 3.0;
            for (offset, aggressor) in [(0_i64, AggressorSide::Sell), (1, AggressorSide::Buy)] {
                trades.push(FootprintTrade {
                    timestamp_micros: timestamp + offset,
                    price,
                    volume,
                    aggressor,
                    bid: None,
                    ask: None,
                    sequence: Some((level * 2 + offset as usize) as u64),
                    trade_id: Some(trade_id),
                    conditions: 0,
                    session_id: Some(1),
                });
                trade_id += 1;
            }
        }
    }
    engine
        .set_footprint_trades(series_id, trades)
        .expect("footprint capture tape is valid");
    engine.set_series_visible(0, false);
    engine.set_bar_spacing(72.0);
    engine.scroll_to_real_time();
}

impl Render for Capture {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.done {
            cx.quit();
            return div();
        }
        window.request_animation_frame();
        let entity: Entity<Capture> = cx.entity();

        let background = self.background;
        let frame_background = self.frame_background;
        div().bg(background).size_full().child(
            canvas(
                move |bounds: Bounds<gpui::Pixels>, _window, _cx| bounds,
                move |bounds: Bounds<gpui::Pixels>, _prepared, window, cx| {
                    entity.update(cx, |capture: &mut Capture, cx| {
                        let viewport = AerisViewport::from_bounds(
                            bounds.origin.x.into(),
                            bounds.origin.y.into(),
                            bounds.size.width.into(),
                            bounds.size.height.into(),
                        );
                        capture
                            .renderer
                            .paint_frame(
                                &PreparedAerisFrame {
                                    frame: &capture.frame,
                                    axis_prims: &[],
                                    axis_points: &[],
                                    background: frame_background,
                                },
                                viewport,
                                window.scale_factor(),
                                window,
                                cx,
                            )
                            .expect("shared pane frame must paint at the physical window DPR");

                        match &capture.phase {
                            Phase::Warmup(frames) if *frames >= WARMUP_FRAMES => {
                                capture.phase = Phase::Capturing(capture.spawn_capture(
                                    #[cfg(target_os = "linux")]
                                    window,
                                ));
                            }
                            Phase::Warmup(frames) => capture.phase = Phase::Warmup(frames + 1),
                            Phase::Capturing(receiver) => {
                                if let Ok(result) = receiver.try_recv() {
                                    match result.and_then(|note| {
                                        capture.finish(&note)?;
                                        Ok(note)
                                    }) {
                                        Ok(_) => capture.done = true,
                                        Err(error) => panic!("GPUI pane capture failed: {error}"),
                                    }
                                }
                            }
                        }
                    });
                },
            )
            .size_full(),
        )
    }
}

fn default_metadata_path(output: &Path) -> PathBuf {
    output.with_extension("json")
}

fn main() {
    let output = PathBuf::from(
        std::env::var_os("AERIS_CHARTS_GPUI_CAPTURE_OUT")
            .expect("AERIS_CHARTS_GPUI_CAPTURE_OUT must name the output PNG"),
    );
    let metadata = std::env::var_os("AERIS_CHARTS_GPUI_CAPTURE_METADATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| default_metadata_path(&output));
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).expect("capture output directory is creatable");
    }
    if let Some(parent) = metadata.parent() {
        std::fs::create_dir_all(parent).expect("metadata output directory is creatable");
    }

    let fixture = aeris_charts_native::engine_scene::parity_fixture();
    let pane_width = (fixture.css_width - fixture.price_axis_width) as f32;
    let pane_height = (fixture.css_height - fixture.time_axis_height) as f32;
    let window_title = format!("{WINDOW_TITLE_PREFIX}-{}", std::process::id());
    application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(pane_width), px(pane_height)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some(window_title.clone().into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                let scale_factor = window.scale_factor();
                cx.new(|_| {
                    Capture::new(
                        scale_factor,
                        output.clone(),
                        metadata.clone(),
                        window_title.clone(),
                    )
                })
            },
        )
        .expect("the finite GPUI capture window opens");
        cx.activate(true);
    });
}
