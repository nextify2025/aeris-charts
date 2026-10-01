//! Official-GPUI integration demo and finite probe for Aeris's complete engine frame.
//!
//! With `AERIS_CHARTS_PROBE_FRAMES` unset this is a real native application shell: grouped controls,
//! independently seeded `Workspace` split cells, drawing creation, indicators, exact package
//! themes, OHLC/click status, and host-side visual approximations of the web plugin fixtures. Those fixture
//! toggles insert engine `Prim`s or use native engine APIs; they are explicitly not a JavaScript
//! object bridge. With `AERIS_CHARTS_PROBE_FRAMES` set a single chart paints N frames and exits
//! silently (a smoke run of layout, native text measurement, and chrome). The demo data is static.
//!
//! ```text
//! cargo run -p aeris_charts_render_gpui --features gpui-backend --example gpui_probe
//! ```
//!
//! Environment knobs:
//! - `AERIS_CHARTS_PROBE_BARS` — synthetic bars to load (default 500).
//! - `AERIS_CHARTS_PROBE_FRAMES` — quit after N painted frames. Unset runs interactively until the
//!   window closes. The demo never prints frame data.
//! - `AERIS_CHARTS_PROBE_FEATURE=footprint` — finite probes start in the deterministic detailed-LOD
//!   footprint fixture instead of the default candlestick fixture.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use aeris_charts_core::model::data_layer::SeriesId;
use aeris_charts_engine::{
    crosshair_mode_from_u8, marker_pos, marker_shape, AggressorSide, BrushRange, BrushStyle,
    ChartEngine, ChartFrame, DeltaTooltipActiveRange, DeltaTooltipOptions, DrawingId, DrawingKind,
    DrawingModifiers, DrawingPoint, DrawingTextEditKey, FootprintAggregationOptions,
    FootprintBarAggregation, FootprintImbalanceOptions, FootprintSeriesOptions, FootprintTrade,
    GestureResolver, GestureUpdateKind, InputDevice, InputModifiers, InputTarget, Marker,
    NativePrimitiveId, PointerSample, PriceLineExtent, PriceScaleTarget,
    PrimitiveAutoscaleContribution, SeriesKind, SplitDirection, TradeStudyOptions, WheelBehavior,
    WheelDeltaMode, WheelIntent, WheelSample, Workspace, WorkspaceLayout,
};
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{IRect, LineStyle, Prim, TextAlign};
use aeris_charts_render_gpui::{
    backend::{measure_text, text_cap_centerer, text_measurer},
    AerisViewport, GpuiChartRenderer, GpuiFrameMetrics, PreparedAerisFrame,
};
use gpui::{
    canvas, div, prelude::*, px, relative, rgb, size, AnyElement, App, Bounds, Context,
    CursorStyle, Entity, FocusHandle, Focusable, KeyDownEvent, KeyUpEvent, ModifiersChangedEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PinchEvent, Render, ScrollDelta,
    ScrollHandle, ScrollWheelEvent, Subscription, Window, WindowBounds, WindowOptions,
};
use gpui_platform::application;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DemoTheme {
    Light,
    Dark,
}

impl DemoTheme {
    fn surface(self) -> &'static str {
        match self {
            Self::Light => aeris_charts_core::style::LIGHT_SURFACE_CSS,
            Self::Dark => aeris_charts_core::style::DARK_SURFACE_CSS,
        }
    }

    fn crosshair(self) -> &'static str {
        match self {
            Self::Light => aeris_charts_core::style::LIGHT_CROSSHAIR_CSS,
            Self::Dark => aeris_charts_core::style::DARK_CROSSHAIR_CSS,
        }
    }

    fn primary(self) -> &'static str {
        match self {
            Self::Light => aeris_charts_core::style::LIGHT_PRIMARY_CSS,
            Self::Dark => aeris_charts_core::style::DARK_PRIMARY_CSS,
        }
    }

    fn primary_foreground(self) -> &'static str {
        match self {
            Self::Light => aeris_charts_core::style::LIGHT_PRIMARY_FOREGROUND_CSS,
            Self::Dark => aeris_charts_core::style::DARK_PRIMARY_FOREGROUND_CSS,
        }
    }

    fn primary_hover(self) -> &'static str {
        match self {
            Self::Light => aeris_charts_core::style::LIGHT_PRIMARY_HOVER_CSS,
            Self::Dark => aeris_charts_core::style::DARK_PRIMARY_HOVER_CSS,
        }
    }

    fn muted(self) -> &'static str {
        match self {
            Self::Light => aeris_charts_core::style::LIGHT_MUTED_CSS,
            Self::Dark => aeris_charts_core::style::DARK_MUTED_CSS,
        }
    }

    fn muted_foreground(self) -> &'static str {
        match self {
            Self::Light => aeris_charts_core::style::LIGHT_MUTED_FOREGROUND_CSS,
            Self::Dark => aeris_charts_core::style::DARK_MUTED_FOREGROUND_CSS,
        }
    }

    fn accent(self) -> &'static str {
        match self {
            Self::Light => aeris_charts_core::style::LIGHT_ACCENT_CSS,
            Self::Dark => aeris_charts_core::style::DARK_ACCENT_CSS,
        }
    }

    fn ring(self) -> &'static str {
        match self {
            Self::Light => aeris_charts_core::style::LIGHT_RING_CSS,
            Self::Dark => aeris_charts_core::style::DARK_RING_CSS,
        }
    }

    fn patch(self) -> String {
        let surface = self.surface();
        let border = theme_border(self);
        let text = theme_text(self);
        let crosshair = self.crosshair();
        format!(
            r#"{{"layout":{{"background":{{"type":"solid","color":"{surface}"}},"textColor":"{text}","panes":{{"separatorColor":"{border}"}}}},"leftPriceScale":{{"borderColor":"{border}"}},"rightPriceScale":{{"borderColor":"{border}"}},"timeScale":{{"borderColor":"{border}"}},"grid":{{"vertLines":{{"color":"{border}"}},"horzLines":{{"color":"{border}"}}}},"crosshair":{{"vertLine":{{"color":"{crosshair}","labelBackgroundColor":"{crosshair}"}},"horzLine":{{"color":"{crosshair}","labelBackgroundColor":"{crosshair}"}}}}}}"#
        )
    }
}

fn apply_package_theme(engine: &mut ChartEngine, theme: DemoTheme) {
    let patch = theme.patch();
    engine
        .options
        .apply_str(&patch)
        .expect("built-in GPUI package theme is valid JSON");
}

#[derive(Clone, Debug, Default)]
struct StylePins {
    grid_color: Option<String>,
    axis_border_color: Option<String>,
    text_color: Option<String>,
    separator_color: Option<String>,
}

impl StylePins {
    fn apply(&self, engine: &mut ChartEngine) {
        if let Some(color) = &self.grid_color {
            engine
                .options
                .apply_str(&format!(
                    r#"{{"grid":{{"vertLines":{{"color":"{color}"}},"horzLines":{{"color":"{color}"}}}}}}"#
                ))
                .expect("pinned grid color is valid");
        }
        if let Some(color) = &self.axis_border_color {
            engine
                .options
                .apply_str(&format!(
                    r#"{{"leftPriceScale":{{"borderColor":"{color}"}},"rightPriceScale":{{"borderColor":"{color}"}},"timeScale":{{"borderColor":"{color}"}}}}"#
                ))
                .expect("pinned axis color is valid");
        }
        if let Some(color) = &self.text_color {
            engine
                .options
                .apply_str(&format!(r#"{{"layout":{{"textColor":"{color}"}}}}"#))
                .expect("pinned text color is valid");
        }
        if let Some(color) = &self.separator_color {
            engine
                .options
                .apply_str(&format!(
                    r#"{{"layout":{{"panes":{{"separatorColor":"{color}"}}}}}}"#
                ))
                .expect("pinned separator color is valid");
        }
    }
}

fn theme_border(theme: DemoTheme) -> &'static str {
    match theme {
        DemoTheme::Light => aeris_charts_core::style::LIGHT_BORDER_CSS,
        DemoTheme::Dark => aeris_charts_core::style::DARK_BORDER_CSS,
    }
}

fn theme_text(theme: DemoTheme) -> &'static str {
    match theme {
        DemoTheme::Light => aeris_charts_core::style::LIGHT_AXIS_TEXT_CSS,
        DemoTheme::Dark => aeris_charts_core::style::DARK_AXIS_TEXT_CSS,
    }
}

fn shell_rgb(css: &str, fallback: u32) -> u32 {
    let hex = css.strip_prefix('#').unwrap_or(css);
    if hex.len() != 6 {
        return fallback;
    }
    u32::from_str_radix(hex, 16).unwrap_or(fallback)
}

/// Stable, testable manifest for the native toolbar. Controls are intentionally compact cycle
/// buttons rather than HTML inputs; every item maps to an engine or host-side native action.
const TOOLBAR_FEATURE_MANIFEST: &[&str] = &[
    "series:candlestick,bar,line,area,brushable-area,footprint,histogram,baseline",
    "style:candle-body,wick-colors,border-colors,wick-visible,border-visible,reset-parts,line-color,line-width,area-fill",
    "overlay:sma20,volume,volume-profile,rsi14,cvd",
    "workspace:split-horizontal,split-vertical,shortcuts,maximize,restore,close,cap,usage,active,resize",
    "drawing:trend,h-line,h-ray,v-line,rect,text,path,brush,clear,color,style,width,label,text-color,size,weight,italic",
    "crosshair:mode,color,width,style,label-background,labels",
    "chart:theme,grid,grid-color,grid-style,font-family,font-size",
    "series-chrome:price-line,extent,style,last-value,title-visible,title-text,countdown,bid-ask",
    "axes:border-visible,border-color,text-color,separator",
    "watermark:visible,text,color,size",
    "interaction:axis-scaling,mouse-kinetic,reset-view",
    "native-visual-approximations:day-bands,position-band,autoscale-band,markers,vertical-line",
];

/// The CVD study's canonical trade stream and its line series.
#[derive(Clone, Copy)]
struct CvdDemoState {
    stream_id: u64,
    series_id: SeriesId,
}

/// Column-major OHLC, in the shape `ChartEngine::set_series_data` takes.
#[derive(Clone)]
struct Bars {
    times: Vec<f64>,
    open: Vec<f64>,
    high: Vec<f64>,
    low: Vec<f64>,
    close: Vec<f64>,
}

/// Deterministic synthetic OHLC: no RNG, no clock, so successive runs are comparable.
fn synthetic_bars(count: usize) -> Bars {
    let mut times = Vec::with_capacity(count);
    let mut open = Vec::with_capacity(count);
    let mut high = Vec::with_capacity(count);
    let mut low = Vec::with_capacity(count);
    let mut close = Vec::with_capacity(count);
    let mut price = 100.0f64;
    for i in 0..count {
        let t = i as f64;
        let c = 100.0 + (t * 0.11).sin() * 6.0 + (t * 0.031).cos() * 14.0;
        let o = price;
        price = c;
        times.push(1_600_000_000.0 + t * 60.0);
        open.push(o);
        high.push(o.max(c) + 1.5 + (t * 0.7).sin().abs());
        low.push(o.min(c) - 1.5 - (t * 0.9).cos().abs());
        close.push(c);
    }
    Bars {
        times,
        open,
        high,
        low,
        close,
    }
}

/// Match the Web demo's root-cell fixture: 1,000 deterministic hourly bars by default.
fn interactive_root_bars(count: usize) -> Bars {
    let end_time = 1_600_000_000.0 + count.saturating_sub(1) as f64 * 3_600.0;
    web_demo_bars(count, 42, 100.0, 2.4, 1.2, end_time)
}

/// Match the Web demo's split-cell contract: every new cell gets 300 deterministic hourly bars
/// for a distinct synthetic asset, aligned to the root asset's final timestamp.
fn split_asset_bars(sequence: usize, end_time: f64) -> Bars {
    web_demo_bars(
        300,
        42_u32.wrapping_add((sequence as u32).wrapping_mul(977)),
        40.0 + sequence as f64 * 25.0,
        2.2,
        0.9,
        end_time,
    )
}

fn web_demo_bars(
    count: usize,
    mut seed: u32,
    start_price: f64,
    close_span: f64,
    wick_span: f64,
    end_time: f64,
) -> Bars {
    let mut times = Vec::with_capacity(count);
    let mut open = Vec::with_capacity(count);
    let mut high = Vec::with_capacity(count);
    let mut low = Vec::with_capacity(count);
    let mut close = Vec::with_capacity(count);
    let mut random = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        f64::from(seed) / f64::from(u32::MAX)
    };
    let mut price = start_price;
    let start = end_time - count.saturating_sub(1) as f64 * 3_600.0;
    for i in 0..count {
        let o = price;
        let c = (o + (random() - 0.5) * close_span).max(1.0);
        times.push(start + i as f64 * 3_600.0);
        open.push(o);
        high.push(o.max(c) + random() * wick_span);
        low.push(o.min(c) - random() * wick_span);
        close.push(c);
        price = c;
    }
    Bars {
        times,
        open,
        high,
        low,
        close,
    }
}

/// Explicit synthetic tick tape used by both demos' Footprint showcase. OHLC never supplies bid,
/// ask, or aggressor truth: the demo generates those trade fields directly, then the production
/// footprint aggregator derives every displayed bar and price level from this tape.
fn footprint_demo_trades(bars: &Bars) -> Vec<FootprintTrade> {
    const TICK_SIZE: f64 = 0.25;
    const SHAPE: [u32; 11] = [3, 6, 11, 17, 23, 28, 23, 17, 11, 6, 3];
    const OFFSETS: [i64; 11] = [-5, -4, -3, -2, -1, 0, 1, 2, 3, 4, 5];
    let first = bars.times.len().saturating_sub(12);
    let mut trades = Vec::with_capacity((bars.times.len() - first) * SHAPE.len() * 2);
    let mut trade_id = 1_u64;
    for (sample_index, bar_index) in (first..bars.times.len()).enumerate() {
        let start_seconds = bars.times[bar_index] as i64 / 3_600 * 3_600;
        let center_level =
            (bars.close[bar_index] / TICK_SIZE).round() as i64 + sample_index as i64 % 3 - 1;
        let ask_dominant = sample_index % 2 == 0;
        let mut events = Vec::with_capacity(SHAPE.len() * 2);
        for (offset, peak) in OFFSETS.into_iter().zip(SHAPE) {
            let smaller = (peak as f64 * 0.2).round().max(2.0);
            let bid_volume = if ask_dominant {
                smaller
            } else {
                f64::from(peak)
            };
            let ask_volume = if ask_dominant {
                f64::from(peak)
            } else {
                smaller
            };
            events.push((center_level + offset, bid_volume, AggressorSide::Sell));
            events.push((center_level + offset, ask_volume, AggressorSide::Buy));
        }
        if !ask_dominant {
            events.reverse();
        }
        for (event_index, (level, volume, aggressor)) in events.into_iter().enumerate() {
            trades.push(FootprintTrade {
                timestamp_micros: start_seconds * 1_000_000 + event_index as i64 * 10_000 + 1,
                price: level as f64 * TICK_SIZE,
                volume,
                aggressor,
                bid: None,
                ask: None,
                sequence: Some(event_index as u64),
                trade_id: Some(trade_id),
                conditions: 0,
                session_id: Some(1),
            });
            trade_id += 1;
        }
    }
    trades
}

/// One minute bar's synthetic prints for the CVD study: an open → extreme → extreme → close
/// path whose aggressor mix follows the bar direction, so cumulative delta tracks the candles.
fn cvd_demo_bar_trades(bars: &Bars, index: usize, trade_id: &mut u64) -> Vec<FootprintTrade> {
    let (open, high, low, close) = (
        bars.open[index],
        bars.high[index],
        bars.low[index],
        bars.close[index],
    );
    let path = if close >= open {
        [open, low, high, close]
    } else {
        [open, high, low, close]
    };
    let start_micros = bars.times[index] as i64 * 1_000_000;
    let buy_share = (0.5 + (close - open) / (high - low).max(1e-9) * 0.4).clamp(0.1, 0.9);
    let mut trades = Vec::with_capacity(path.len() * 2);
    for (step, price) in path.into_iter().enumerate() {
        let volume = 40.0 + ((index * 7 + step * 13) % 23) as f64;
        for (offset, (side, share)) in [
            (AggressorSide::Buy, buy_share),
            (AggressorSide::Sell, 1.0 - buy_share),
        ]
        .into_iter()
        .enumerate()
        {
            trades.push(FootprintTrade {
                timestamp_micros: start_micros + (step * 2 + offset) as i64 * 1_000_000 + 1,
                // Real prints trade on the stream's 0.01 tick grid.
                price: (price * 100.0).round() / 100.0,
                volume: (volume * share).round().max(1.0),
                aggressor: side,
                bid: None,
                ask: None,
                sequence: None,
                trade_id: Some(*trade_id),
                conditions: 0,
                session_id: Some(1),
            });
            *trade_id += 1;
        }
    }
    trades
}

const CLICK_SLOP_MANHATTAN: f64 = 5.0;
const PANE_SEPARATOR_HIT: f64 = 4.0;
const WHEEL_LINE_HEIGHT: f32 = 32.0;

/// The browser host's resolved desktop defaults. Keeping them explicit prevents a demo-only
/// behavior from silently diverging from `packages/charts/src/impl.ts`.
#[derive(Clone, Copy, Debug)]
struct GestureConfig {
    pan: bool,
    wheel_scroll: bool,
    wheel_zoom: bool,
    wheel_behavior: WheelBehavior,
    axis_dblclick_reset_time: bool,
    axis_dblclick_reset_price: bool,
    axis_scale_price: bool,
    axis_scale_time: bool,
    kinetic_mouse: bool,
    panes_resize: bool,
}

impl Default for GestureConfig {
    fn default() -> Self {
        Self {
            pan: true,
            wheel_scroll: true,
            wheel_zoom: true,
            wheel_behavior: WheelBehavior::Auto,
            axis_dblclick_reset_time: true,
            axis_dblclick_reset_price: true,
            axis_scale_price: true,
            axis_scale_time: true,
            kinetic_mouse: false,
            panes_resize: true,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum DragMode {
    Pan {
        price_pan: Option<(usize, PriceScaleTarget)>,
    },
    TimeAxis,
    PriceAxis {
        pane: usize,
        target: PriceScaleTarget,
    },
    PaneSeparator {
        index: usize,
        last_y: f64,
    },
    Drawing,
    DrawingCreation,
    DeltaTooltip,
    CrosshairAction,
}

#[derive(Clone, Copy, Debug)]
struct BrushableAreaState {
    tooltip_id: NativePrimitiveId,
    styled_range: Option<DeltaTooltipActiveRange>,
}

#[derive(Clone, Copy, Debug)]
struct FootprintDemoState {
    series_id: SeriesId,
    previous_bar_spacing: f64,
    previous_right_offset: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DrawingTemplate {
    color: String,
    style: &'static str,
    width: u8,
    /// Content for the standalone Text tool only. Trend-line labels are typed in place, so the
    /// template never writes a label onto a trend line or over a selected drawing's text.
    text: String,
    /// `None` keeps the engine default: a trend label follows its line color and standalone
    /// text follows the chart foreground. Set only once the user explicitly picks a color.
    text_color: Option<String>,
    text_size: u8,
    text_weight: u16,
    text_italic: bool,
    text_h_align: &'static str,
    text_v_align: &'static str,
}

impl Default for DrawingTemplate {
    /// The browser demo's toolbar defaults, so both hosts create identical drawings.
    fn default() -> Self {
        Self {
            color: "#168ef7".into(),
            style: "solid",
            width: 2,
            text: String::new(),
            text_color: None,
            text_size: 14,
            text_weight: 400,
            text_italic: false,
            text_h_align: "right",
            text_v_align: "top",
        }
    }
}

impl DrawingTemplate {
    /// Style fields every tool shares; label content and ink stay opt-in.
    fn style_fields(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut fields = serde_json::Map::new();
        fields.insert("color".into(), self.color.clone().into());
        fields.insert("style".into(), self.style.into());
        fields.insert("width".into(), self.width.into());
        fields.insert("text_size".into(), self.text_size.into());
        fields.insert("text_weight".into(), self.text_weight.into());
        fields.insert("text_italic".into(), self.text_italic.into());
        fields.insert("text_h_align".into(), self.text_h_align.into());
        fields.insert("text_v_align".into(), self.text_v_align.into());
        if let Some(color) = &self.text_color {
            fields.insert("text_color".into(), color.clone().into());
        }
        fields
    }

    /// Options for arming `kind`. Only the Text tool receives template content.
    fn json(&self, kind: DrawingKind) -> String {
        let mut fields = self.style_fields();
        if kind == DrawingKind::Text {
            fields.insert("text".into(), self.text.clone().into());
        }
        serde_json::Value::Object(fields).to_string()
    }

    /// Changed style fields only, merged into an in-flight creation and the selected drawing.
    /// Label content is never part of a live patch, so it cannot wipe a drawing's real text.
    fn patch_from(&self, previous: &Self) -> String {
        let current = self.style_fields();
        let before = previous.style_fields();
        let mut patch: serde_json::Map<String, serde_json::Value> = current
            .into_iter()
            .filter(|(key, value)| before.get(key) != Some(value))
            .collect();
        if self.text_color.is_none() && previous.text_color.is_some() {
            // Back to the inherited default: an empty color clears the explicit override.
            patch.insert("text_color".into(), "".into());
        }
        serde_json::Value::Object(patch).to_string()
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct NativeFixtures {
    day_bands: bool,
    position_band: bool,
    autoscale_band: bool,
    markers: bool,
    plugin_watermark: bool,
    vertical_line: bool,
}

/// The probe's chart state: an engine, the adapter, and the last built frame.
struct Probe {
    engine: ChartEngine,
    renderer: GpuiChartRenderer,
    frame: ChartFrame,
    /// Real engine-produced watermark/axis/crosshair top layer.
    axis: Vec<Prim>,
    focus_handle: Option<FocusHandle>,
    /// Size and DPR the frame was last built for, so resize and scale changes are detected.
    built_for: (f32, f32, f32),
    dirty: bool,
    plan_dirty: bool,
    fitted: bool,
    fit_on_first_frame: bool,
    /// Newest captured drawing sample since the last painted frame. Wayland may deliver motion at
    /// HID cadence; the platform host coalesces that stream to presentation cadence before
    /// forwarding it to the engine-owned drawing controller.
    pending_creation_point: Option<(f64, f64, DrawingModifiers)>,
    /// A placement committed on pointer-down. Suppresses the release-side click selection without
    /// the host knowing which concrete drawing kind uses press placement.
    creation_press_committed: bool,
    viewport_offset: (f32, f32),
    gesture_config: GestureConfig,
    input: GestureResolver,
    input_target: InputTarget,
    cursor_style: CursorStyle,
    press_start: Option<(f64, f64)>,
    press_moved: bool,
    /// The drawing selected when the current press began: a selected text drawing opens its
    /// editor on the next click (the browser host's two-step select-then-type model).
    text_press_selected: Option<DrawingId>,
    /// A press on the label being edited: the engine session keeps it, so its release must not
    /// fall through to click selection (which would reopen and reset the session).
    press_in_text_editor: bool,
    drag: Option<DragMode>,
    drag_started: bool,
    kinetic_active: bool,
    keyboard_pan_direction: i8,
    keyboard_pan_delta: f64,
    brushable_area: Option<BrushableAreaState>,
    footprint: Option<FootprintDemoState>,
    source_bars: Bars,
    drawing_template: DrawingTemplate,
    style_pins: StylePins,
    fixtures: NativeFixtures,
    sma_id: Option<SeriesId>,
    volume_id: Option<SeriesId>,
    volume_profile: Option<(u32, SeriesId)>,
    rsi_id: Option<SeriesId>,
    cvd: Option<CvdDemoState>,
    legend: String,
    click_status: String,
    bars: usize,
    painted: u64,
    frame_budget: Option<u64>,
    last: GpuiFrameMetrics,
    /// Distinct scale factors and sizes observed, to prove the propagation actually happened.
    seen_scales: Vec<f32>,
    seen_sizes: Vec<(f32, f32)>,
    started: Instant,
}

impl Probe {
    fn new(bars: usize, frame_budget: Option<u64>) -> Self {
        let mut engine = ChartEngine::new(1024.0, 640.0, 1.0);
        apply_package_theme(&mut engine, DemoTheme::Light);
        let b = synthetic_bars(bars);
        engine
            .set_series_data(0, &b.times, &b.open, &b.high, &b.low, &b.close)
            .expect("synthetic series is well formed");
        engine.series[0].kind = SeriesKind::Candlestick;
        engine.create_price_line(
            0,
            108.0,
            Color::rgb(0x29, 0xb6, 0xf6),
            2,
            LineStyle::Dashed,
            "LI",
        );
        let last_logical = bars.saturating_sub(1).max(1) as f64;
        engine
            .add_drawing(
                DrawingKind::TrendLine,
                0,
                vec![
                    DrawingPoint {
                        logical: last_logical * 0.28,
                        price: 96.0,
                    },
                    DrawingPoint {
                        logical: last_logical * 0.72,
                        price: 112.0,
                    },
                ],
                Some(r##"{"color":"#ffb300","line_width":2}"##),
            )
            .expect("the deterministic trend-line fixture is valid");
        Self {
            engine,
            renderer: GpuiChartRenderer::new(),
            frame: ChartFrame::default(),
            axis: Vec::new(),
            focus_handle: None,
            built_for: (0.0, 0.0, 0.0),
            dirty: true,
            plan_dirty: true,
            fitted: false,
            fit_on_first_frame: true,
            pending_creation_point: None,
            creation_press_committed: false,
            viewport_offset: (0.0, 0.0),
            gesture_config: GestureConfig::default(),
            input: GestureResolver::default(),
            input_target: InputTarget::Pane,
            cursor_style: CursorStyle::Crosshair,
            press_start: None,
            press_moved: false,
            text_press_selected: None,
            press_in_text_editor: false,
            drag: None,
            drag_started: false,
            kinetic_active: false,
            keyboard_pan_direction: 0,
            keyboard_pan_delta: 0.0,
            brushable_area: None,
            footprint: None,
            source_bars: b,
            drawing_template: DrawingTemplate::default(),
            style_pins: StylePins::default(),
            fixtures: NativeFixtures::default(),
            sma_id: None,
            volume_id: None,
            volume_profile: None,
            rsi_id: None,
            cvd: None,
            legend: "O —  H —  L —  C —".to_string(),
            click_status: "ready".to_string(),
            bars,
            painted: 0,
            frame_budget,
            last: GpuiFrameMetrics::default(),
            seen_scales: Vec::new(),
            seen_sizes: Vec::new(),
            started: Instant::now(),
        }
    }

    fn new_interactive(bars: usize) -> Self {
        let mut probe = Self::new(bars, None);
        // The interactive demo is a view of canonical engine defaults, not a second style owner.
        // Reset the deterministic finite-probe theme and change only the one intentional demo
        // presentation choice: hide both grid families while preserving their native dashed style.
        probe.engine.options = Default::default();
        probe
            .engine
            .options
            .apply_str(r#"{"grid":{"vertLines":{"visible":false},"horzLines":{"visible":false}}}"#)
            .expect("interactive demo grid visibility patch is valid");
        probe.engine.series[0].price_lines.clear();
        probe.engine.clear_drawings();
        probe.fit_on_first_frame = false;
        probe.replace_source_bars(interactive_root_bars(bars));
        probe
    }

    fn replace_source_bars(&mut self, bars: Bars) {
        self.engine
            .set_series_data(
                0,
                &bars.times,
                &bars.open,
                &bars.high,
                &bars.low,
                &bars.close,
            )
            .expect("split-cell synthetic series is well formed");
        self.bars = bars.times.len();
        self.source_bars = bars;
        self.fitted = false;
        self.dirty = true;
    }

    fn apply_theme(&mut self, theme: DemoTheme) {
        apply_package_theme(&mut self.engine, theme);
        self.style_pins.apply(&mut self.engine);
        self.renderer.invalidate_caches();
        self.dirty = true;
    }

    fn arm_drawing(&mut self, kind: DrawingKind) {
        self.pending_creation_point = None;
        self.creation_press_committed = false;
        let next = (self.engine.active_drawing_tool() != Some(kind)).then_some(kind);
        let template = next.map(|kind| self.drawing_template.json(kind));
        let armed = self
            .engine
            .set_drawing_tool(next, template.as_deref(), None);
        debug_assert!(armed, "native drawing template is always valid JSON");
        self.click_status = self.engine.active_drawing_tool().map_or_else(
            || "drawing tool disarmed".to_string(),
            |tool| format!("{} armed", tool.name()),
        );
        self.dirty = true;
    }

    /// A committed placement. Text-capable tools that request typing open the engine session.
    fn drawing_created(&mut self, id: DrawingId) {
        self.click_status = format!("created drawing #{id}");
        if self.engine.drawing_requests_text_edit(id) {
            self.engine.begin_drawing_text_edit(id, true);
        }
        self.dirty = true;
    }

    /// Whether a pane point lies on the label the open text session is editing.
    fn on_text_editor(&self, pane_x: f64, y: f64) -> bool {
        let Some((editing, _, _)) = self.engine.drawing_text_edit() else {
            return false;
        };
        self.engine.drawing_text_hit_at(pane_x, y) == Some(editing)
            || self.engine.hit_test_drawing(pane_x, y).is_some_and(|hit| {
                hit.id == editing
                    && self
                        .engine
                        .drawing(editing)
                        .is_some_and(|drawing| drawing.kind == DrawingKind::Text)
            })
    }

    fn place_drawing_anchor(&mut self, x: f64, y: f64, modifiers: DrawingModifiers) -> i64 {
        if self.engine.active_drawing_tool().is_none() {
            return 0;
        }
        let update = self.engine.drawing_tool_activate(x, y, modifiers);
        if let Some(id) = update.created {
            self.drawing_created(id);
            return i64::from(id);
        }
        if update.consumed {
            -1
        } else {
            0
        }
    }

    fn finish_drawing_creation(&mut self) -> bool {
        let update = self.engine.drawing_tool_finish();
        let Some(id) = update.created else {
            return false;
        };
        self.drawing_created(id);
        self.dirty = true;
        true
    }

    fn pop_drawing_anchor(&mut self) -> bool {
        let changed = self.engine.drawing_tool_pop_anchor();
        self.dirty |= changed;
        changed
    }

    fn update_drawing_template(&mut self, mutate: impl FnOnce(&mut DrawingTemplate)) {
        let previous = self.drawing_template.clone();
        mutate(&mut self.drawing_template);
        let patch = self.drawing_template.patch_from(&previous);
        if patch == "{}" {
            return;
        }
        // Merge only changed fields into both an in-progress creation and a selected drawing.
        // This preserves placed anchors and unrelated selected-drawing options.
        if self.engine.active_drawing_tool().is_some() {
            self.engine.drawing_tool_apply_options(&patch);
        }
        if let Some(id) = self.engine.selected_drawing() {
            self.engine.drawing_apply_options(id, &patch);
        }
        self.dirty = true;
    }

    fn toggle_grid_color_pin(&mut self, theme: DemoTheme) {
        self.style_pins.grid_color = self
            .style_pins
            .grid_color
            .is_none()
            .then(|| "#2962ff".to_string());
        let color = self
            .style_pins
            .grid_color
            .as_deref()
            .unwrap_or_else(|| theme_border(theme));
        self.engine
            .options
            .apply_str(&format!(
                r#"{{"grid":{{"vertLines":{{"color":"{color}"}},"horzLines":{{"color":"{color}"}}}}}}"#
            ))
            .expect("grid color toggle is valid");
        self.dirty = true;
    }

    fn toggle_axis_border_pin(&mut self, theme: DemoTheme) {
        self.style_pins.axis_border_color = self
            .style_pins
            .axis_border_color
            .is_none()
            .then(|| "#2962ff".to_string());
        let color = self
            .style_pins
            .axis_border_color
            .as_deref()
            .unwrap_or_else(|| theme_border(theme));
        self.engine
            .options
            .apply_str(&format!(
                r#"{{"leftPriceScale":{{"borderColor":"{color}"}},"rightPriceScale":{{"borderColor":"{color}"}},"timeScale":{{"borderColor":"{color}"}}}}"#
            ))
            .expect("axis color toggle is valid");
        self.dirty = true;
    }

    fn toggle_text_color_pin(&mut self, theme: DemoTheme) {
        self.style_pins.text_color = self
            .style_pins
            .text_color
            .is_none()
            .then(|| "#ab47bc".to_string());
        let color = self
            .style_pins
            .text_color
            .as_deref()
            .unwrap_or_else(|| theme_text(theme));
        self.engine
            .options
            .apply_str(&format!(r#"{{"layout":{{"textColor":"{color}"}}}}"#))
            .expect("text color toggle is valid");
        self.dirty = true;
    }

    fn toggle_separator_pin(&mut self, theme: DemoTheme) {
        self.style_pins.separator_color = self
            .style_pins
            .separator_color
            .is_none()
            .then(|| "#ff9800".to_string());
        let color = self
            .style_pins
            .separator_color
            .as_deref()
            .unwrap_or_else(|| theme_border(theme));
        self.engine
            .options
            .apply_str(&format!(
                r#"{{"layout":{{"panes":{{"separatorColor":"{color}"}}}}}}"#
            ))
            .expect("separator color toggle is valid");
        self.dirty = true;
    }

    fn set_series_kind(&mut self, kind: SeriesKind) {
        self.disable_footprint();
        self.disable_brushable_area();
        self.engine.convert_series_kind(0, kind);
        self.click_status = format!("series: {kind:?}");
        self.dirty = true;
    }

    fn enable_brushable_area(&mut self) {
        self.disable_footprint();
        self.disable_brushable_area();
        self.engine.convert_series_kind(0, SeriesKind::Area);
        let tooltip_id = self
            .engine
            .add_delta_tooltip(0, DeltaTooltipOptions::default())
            .expect("the built-in Area series accepts the delta-tooltip interaction");
        self.brushable_area = Some(BrushableAreaState {
            tooltip_id,
            styled_range: None,
        });
        self.click_status = "series: brushable area · drag to compare".into();
        self.dirty = true;
    }

    fn enable_footprint(&mut self) {
        self.disable_footprint();
        self.disable_brushable_area();
        let previous_bar_spacing = self.engine.bar_spacing();
        let previous_right_offset = self.engine.right_offset();
        let options = FootprintSeriesOptions {
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
            visual: aeris_charts_engine::FootprintVisualOptions {
                font_size: 10.0,
                show_bar_summary: true,
                ..Default::default()
            },
        };
        let series_id = self
            .engine
            .add_footprint_series(options)
            .expect("the built-in GPUI footprint options are valid");
        self.engine
            .set_footprint_trades(series_id, footprint_demo_trades(&self.source_bars))
            .expect("the deterministic GPUI footprint tape is valid");
        if let Some(series) = self
            .engine
            .series
            .iter_mut()
            .find(|series| series.id == series_id && !series.removed)
        {
            series.price_line_visible = true;
            series.last_value_visible = true;
            series.countdown_visible = true;
            series.title = "ORDER FLOW".into();
        }
        self.engine.set_series_visible(0, false);
        self.engine.set_bar_spacing(72.0);
        self.engine.scroll_to_real_time();
        self.footprint = Some(FootprintDemoState {
            series_id,
            previous_bar_spacing,
            previous_right_offset,
        });
        self.click_status = "series: footprint · native tick tape".into();
        self.dirty = true;
    }

    fn disable_footprint(&mut self) {
        let Some(state) = self.footprint.take() else {
            return;
        };
        self.engine.remove_series(state.series_id);
        self.engine.set_series_visible(0, true);
        self.engine.set_bar_spacing(state.previous_bar_spacing);
        self.engine.set_right_offset(state.previous_right_offset);
        self.dirty = true;
    }

    fn displayed_series_index(&self) -> usize {
        let displayed_id = self.footprint.map_or(0, |state| state.series_id);
        self.engine
            .series
            .iter()
            .position(|series| series.id == displayed_id && !series.removed)
            .unwrap_or(0)
    }

    fn disable_brushable_area(&mut self) {
        let Some(state) = self.brushable_area.take() else {
            return;
        };
        self.engine.remove_native_primitive(state.tooltip_id);
        self.engine.clear_area_brush_state(0);
        if matches!(self.drag, Some(DragMode::DeltaTooltip)) {
            self.drag = None;
        }
        self.dirty = true;
    }

    fn clear_brushable_area_selection(&mut self) -> bool {
        let Some(state) = self.brushable_area else {
            return false;
        };
        self.engine.clear_delta_tooltip(state.tooltip_id);
        self.sync_brushable_area();
        true
    }

    fn sync_brushable_area(&mut self) {
        let Some(mut state) = self.brushable_area else {
            return;
        };
        let range = self.engine.delta_tooltip_active_range(state.tooltip_id);
        if state.styled_range == range {
            return;
        }
        if let Some(range) = range {
            let faded = BrushStyle {
                line_color: Color::rgba(40, 98, 255, 51),
                top_color: Color::rgba(40, 98, 255, 13),
                bottom_color: Color::rgba(40, 98, 255, 0),
                line_width: 2.0,
            };
            let selected = if range.positive {
                BrushStyle {
                    line_color: Color::rgb(4, 153, 129),
                    top_color: Color::rgba(4, 153, 129, 102),
                    bottom_color: Color::rgba(4, 153, 129, 0),
                    line_width: 3.0,
                }
            } else {
                BrushStyle {
                    line_color: Color::rgb(239, 83, 80),
                    top_color: Color::rgba(239, 83, 80, 102),
                    bottom_color: Color::rgba(239, 83, 80, 0),
                    line_width: 3.0,
                }
            };
            let applied = self.engine.set_area_brush_state(
                0,
                faded,
                vec![BrushRange {
                    from: range.from as f64,
                    to: range.to as f64,
                    style: selected,
                }],
            );
            debug_assert!(applied, "brush state belongs to the live Area series");
        } else {
            self.engine.clear_area_brush_state(0);
        }
        state.styled_range = range;
        self.brushable_area = Some(state);
        self.dirty = true;
    }

    fn toggle_sma(&mut self) {
        if let Some(id) = self.sma_id.take() {
            self.engine.remove_series(id);
        } else {
            self.sma_id = self.engine.add_sma(0, 20);
            if let Some(id) = self.sma_id {
                if let Some(series) = self
                    .engine
                    .series
                    .iter_mut()
                    .find(|series| series.id == id && !series.removed)
                {
                    series.line_color = Some("#ff9800".into());
                    series.line_width = Some(2.0);
                }
            }
        }
        self.dirty = true;
    }

    fn toggle_volume_profile(&mut self) {
        if let Some((indicator, volume)) = self.volume_profile.take() {
            self.engine.remove_native_primitive(indicator);
            self.engine.remove_series(volume);
        } else {
            self.set_series_kind(SeriesKind::Candlestick);
            let volume = self.engine.add_series(SeriesKind::Histogram);
            self.engine
                .series
                .iter_mut()
                .find(|series| series.id == volume)
                .unwrap()
                .visible = false;
            let values: Vec<f64> = self
                .source_bars
                .close
                .iter()
                .zip(&self.source_bars.open)
                .map(|(close, open)| (800.0 + (close - open).abs() * 4000.0).round())
                .collect();
            self.engine
                .set_series_data(
                    volume,
                    &self.source_bars.times,
                    &values,
                    &values,
                    &values,
                    &values,
                )
                .unwrap();
            let indicator = self
                .engine
                .add_volume_profile_indicator(0, volume, Default::default())
                .unwrap();
            self.volume_profile = Some((indicator, volume));
        }
        self.dirty = true;
    }

    fn toggle_volume(&mut self) {
        if let Some(id) = self.volume_id.take() {
            self.engine.remove_series(id);
        } else {
            let id = self.engine.add_series(SeriesKind::Histogram);
            let volume = self
                .source_bars
                .high
                .iter()
                .zip(&self.source_bars.low)
                .enumerate()
                .map(|(i, (h, l))| (h - l) * 25_000.0 + i as f64 * 31.0)
                .collect::<Vec<_>>();
            self.engine
                .set_series_data(
                    id,
                    &self.source_bars.times,
                    &volume,
                    &volume,
                    &volume,
                    &volume,
                )
                .expect("native volume fixture is aligned");
            self.engine
                .set_series_price_scale(id, PriceScaleTarget::Overlay);
            let s = self
                .engine
                .series
                .iter_mut()
                .find(|series| series.id == id && !series.removed)
                .expect("new volume series is live");
            s.histogram_updown = true;
            s.price_line_visible = false;
            s.title = "Volume".into();
            self.volume_id = Some(id);
        }
        self.dirty = true;
    }

    fn toggle_rsi(&mut self) {
        if let Some(id) = self.rsi_id.take() {
            self.engine.remove_series(id);
        } else {
            self.rsi_id = self.engine.add_rsi(0, 14);
            if let Some(id) = self.rsi_id {
                if let Some(series) = self
                    .engine
                    .series
                    .iter_mut()
                    .find(|series| series.id == id && !series.removed)
                {
                    series.line_color = Some("#ab47bc".into());
                    series.line_width = Some(2.0);
                }
            }
        }
        self.dirty = true;
    }

    /// CVD derives from trades, not OHLC: build a one-minute trade stream from the demo candles
    /// and let the engine's cumulative-delta study own the line in its own pane.
    fn toggle_cvd(&mut self) {
        if let Some(cvd) = self.cvd.take() {
            self.engine.remove_series(cvd.series_id);
            let _ = self.engine.remove_trade_stream(cvd.stream_id);
            self.dirty = true;
            return;
        }
        // Anchor the one-minute buckets on the candle opens so every CVD point shares its
        // candle's timestamp instead of interleaving new time points.
        let anchor_micros = self
            .source_bars
            .times
            .first()
            .map_or(0, |&time| (time as i64).rem_euclid(60) * 1_000_000);
        let Ok(stream_id) = self.engine.add_trade_stream(
            "GPUI:CVD",
            FootprintAggregationOptions {
                tick_size: 0.01,
                ticks_per_row: 1,
                bars: FootprintBarAggregation::Time {
                    interval_micros: 60_000_000,
                    anchor_micros,
                },
                imbalance: FootprintImbalanceOptions::default(),
            },
        ) else {
            self.click_status = "CVD: trade stream unavailable".into();
            return;
        };
        let mut next_trade_id = 1;
        let trades = (0..self.source_bars.times.len())
            .flat_map(|index| cvd_demo_bar_trades(&self.source_bars, index, &mut next_trade_id))
            .collect::<Vec<_>>();
        let pane = self.engine.panes.len();
        let installed = self
            .engine
            .set_trade_stream_trades(stream_id, trades)
            .and_then(|_| {
                self.engine
                    .add_cvd_series(stream_id, pane, TradeStudyOptions::default())
            });
        match installed {
            Ok(series_id) => {
                if let Some(series) = self
                    .engine
                    .series
                    .iter_mut()
                    .find(|series| series.id == series_id && !series.removed)
                {
                    series.line_color = Some("#26a69a".into());
                    series.line_width = Some(2.0);
                }
                self.cvd = Some(CvdDemoState {
                    stream_id,
                    series_id,
                });
            }
            Err(error) => {
                let _ = self.engine.remove_trade_stream(stream_id);
                self.click_status = format!("CVD: {error}");
            }
        }
        self.dirty = true;
    }

    fn toggle_markers(&mut self) {
        self.fixtures.markers = !self.fixtures.markers;
        let markers = if self.fixtures.markers && !self.source_bars.times.is_empty() {
            let n = self.source_bars.times.len();
            vec![
                Marker {
                    time: self.source_bars.times[n / 3] as i64,
                    position: marker_pos::ABOVE,
                    shape: marker_shape::ARROW_DOWN,
                    color: Color::rgb(0xef, 0x53, 0x50),
                    text: "Native A".into(),
                    id: "native-a".into(),
                    size: 1.0,
                    price: None,
                },
                Marker {
                    time: self.source_bars.times[n * 2 / 3] as i64,
                    position: marker_pos::BELOW,
                    shape: marker_shape::ARROW_UP,
                    color: Color::rgb(0x26, 0xa6, 0x9a),
                    text: "Native B".into(),
                    id: "native-b".into(),
                    size: 1.0,
                    price: None,
                },
            ]
        } else {
            Vec::new()
        };
        self.engine.set_series_markers(0, markers);
        self.dirty = true;
    }

    fn update_legend(&mut self, pane_x: f64) {
        if self.source_bars.close.is_empty() {
            return;
        }
        let logical = (self.engine.time_scale.coordinate_to_float_index(pane_x) + 0.5).round();
        let i = logical.clamp(0.0, (self.source_bars.close.len() - 1) as f64) as usize;
        self.legend = format!(
            "O {:.2}  H {:.2}  L {:.2}  C {:.2}",
            self.source_bars.open[i],
            self.source_bars.high[i],
            self.source_bars.low[i],
            self.source_bars.close[i]
        );
    }

    fn inject_native_equivalents(&mut self) {
        let Some(scissor) = self.frame.panes.first().map(|pane| pane.scissor) else {
            return;
        };
        let [sx, sy, sw, sh] = scissor;
        let (sx, sy, sw, sh) = (sx as i32, sy as i32, sw as i32, sh as i32);
        let dpr = self.engine.dpr;
        let device_x = |logical: f64| {
            self.engine
                .logical_to_coordinate(logical)
                .map(|x| ((self.engine.pane_left + x) * dpr).round() as i32)
        };
        let device_y = |price: f64| {
            self.engine
                .series_price_to_coordinate(0, price)
                .map(|y| (y * dpr).round() as i32)
        };
        let mut under = Vec::new();
        let mut top = Vec::new();
        let count = self.source_bars.close.len();

        if self.fixtures.day_bands {
            // Native equivalent: deterministic session blocks anchored to logical ranges, so they
            // pan/zoom with the data rather than decorating the viewport.
            for start in (0..count).step_by(50).step_by(2) {
                if let (Some(x0), Some(x1)) = (
                    device_x(start as f64 - 0.5),
                    device_x((start + 50).min(count) as f64 - 0.5),
                ) {
                    under.push(Prim::Rect {
                        rect: IRect {
                            x: x0,
                            y: sy,
                            w: (x1 - x0).max(1),
                            h: sh,
                        },
                        color: Color::rgba(0x29, 0x62, 0xff, 20),
                    });
                }
            }
        }
        if self.fixtures.position_band && count > 0 {
            let entry = self.source_bars.close[count / 2];
            if let (Some(y0), Some(y1)) = (device_y(entry * 1.02), device_y(entry * 0.98)) {
                let (top_y, bottom_y) = (y0.min(y1), y0.max(y1));
                top.push(Prim::Rect {
                    rect: IRect {
                        x: sx,
                        y: top_y,
                        w: sw,
                        h: (bottom_y - top_y).max(1),
                    },
                    color: Color::rgba(0xff, 0x98, 0x00, 38),
                });
                for y in [top_y, bottom_y] {
                    top.push(Prim::HLine {
                        y,
                        x0: sx,
                        x1: sx + sw,
                        width: 2,
                        style: LineStyle::Dashed,
                        color: Color::rgb(0xff, 0x98, 0x00),
                    });
                }
            }
        }
        if self.fixtures.autoscale_band && count > 0 {
            let low = self
                .source_bars
                .low
                .iter()
                .copied()
                .fold(f64::INFINITY, f64::min)
                - 10.0;
            let high = self
                .source_bars
                .high
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max)
                + 10.0;
            for y in [device_y(low), device_y(high)].into_iter().flatten() {
                top.push(Prim::HLine {
                    y,
                    x0: sx,
                    x1: sx + sw,
                    width: 2,
                    style: LineStyle::Dashed,
                    color: Color::rgb(0x9c, 0x27, 0xb0),
                });
            }
        }
        if self.fixtures.vertical_line && count > 0 {
            if let Some(x) = device_x((count / 2) as f64) {
                top.push(Prim::VLine {
                    x,
                    y0: sy,
                    y1: sy + sh,
                    width: 3,
                    style: LineStyle::Solid,
                    color: Color::rgb(0xe9, 0x1e, 0x63),
                });
            }
        }
        if self.fixtures.plugin_watermark {
            top.push(Prim::Text {
                x: (sx + sw / 2) as f32,
                y: (sy + sh / 2) as f32,
                text: "Aeris · native plugin equivalent".into(),
                color: Color::rgba(0x29, 0x62, 0xff, 70),
                size: 28.0 * dpr as f32,
                family: "monospace".into(),
                align: TextAlign::Center,
                weight: 700,
                italic: false,
            });
        }
        if let Some(pane) = self.frame.panes.first_mut() {
            pane.under.extend(under);
            pane.top_prims.extend(top);
        }
    }

    /// Rebuild the complete frame for `(width, height)` logical px at `scale_factor` using
    /// host-native text width callbacks at the resolved axis (`measure`) and countdown
    /// (`countdown_measure`) sizes, each with matching weight.
    fn rebuild_with_measure<F, G>(
        &mut self,
        width: f32,
        height: f32,
        scale_factor: f32,
        measure: F,
        countdown_measure: G,
    ) where
        F: Fn(&str, bool) -> f64,
        G: Fn(&str, bool) -> f64,
    {
        if !self.seen_scales.contains(&scale_factor) {
            self.seen_scales.push(scale_factor);
        }
        if !self.seen_sizes.contains(&(width, height)) {
            self.seen_sizes.push((width, height));
        }
        let key = (width, height, scale_factor);
        let resized = self.built_for != key;
        if !resized && !self.dirty && !self.frame.panes.is_empty() {
            return;
        }
        if self.built_for.2 != 0.0 && self.built_for.2 != scale_factor {
            self.renderer.invalidate_caches();
        }
        self.built_for = key;
        self.dirty = false;
        self.engine.css_width = f64::from(width);
        self.engine.css_height = f64::from(height);
        self.engine.dpr = f64::from(scale_factor);

        self.engine.clear_autoscale_contributions();
        if self.frame_budget.is_none() && self.engine.series[0].countdown_visible {
            if let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) {
                self.engine.set_now_seconds(now.as_secs_f64());
            }
        }
        if self.fixtures.autoscale_band && !self.source_bars.low.is_empty() {
            let min = self
                .source_bars
                .low
                .iter()
                .copied()
                .fold(f64::INFINITY, f64::min)
                - 10.0;
            let max = self
                .source_bars
                .high
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max)
                + 10.0;
            self.engine
                .add_autoscale_contribution(PrimitiveAutoscaleContribution {
                    series: 0,
                    pane: 0,
                    target: PriceScaleTarget::Right,
                    min,
                    max,
                });
        }
        self.engine.recompute_layout_with_measure(
            resized,
            |text, bold| measure(text, bold),
            |text, bold| countdown_measure(text, bold),
        );
        if !self.fitted {
            if self.fit_on_first_frame {
                self.engine.fit_content();
            }
            self.fitted = true;
            if self.fit_on_first_frame {
                self.engine.recompute_layout_with_measure(
                    true,
                    |text, bold| measure(text, bold),
                    |text, bold| countdown_measure(text, bold),
                );
            }
        }
        let max_label_width = (self.engine.axis_font_size() + 4.0) * 5.0 / 8.0
            * f64::from(self.engine.tick_mark_max_character_length.max(1));
        let axis_frame = self.engine.build_axis_frame(
            max_label_width,
            |text, bold| measure(text, bold),
            |text, bold| countdown_measure(text, bold),
        );
        self.engine.build_frame_into(&mut self.frame);
        self.inject_native_equivalents();
        // GPUI's Prim text executor already converts a vertical center into a baseline from native
        // ascent/descent. The browser needs a Canvas ink-box correction; GPUI correctly supplies 0.
        self.engine
            .build_axis_primitives_into(&axis_frame, &mut self.axis, |_| 0.0);
        self.plan_dirty = true;

        let content_h = self.engine.pane_h;
        let pane = self
            .frame
            .panes
            .first()
            .expect("the probe engine always has a primary pane");
        let expected_x_px = (self.engine.pane_left * self.engine.dpr).round() as u32;
        let expected_w_px = (self.engine.pane_w * self.engine.dpr).round() as u32;
        // The primary pane shares the content height with any indicator panes below it, so its
        // scissor follows the engine's own vertical pixel ratio for that pane's height.
        let vpr = (content_h * self.engine.dpr).round().max(1.0) / content_h.max(1.0);
        let expected_h_px = (self.engine.panes[0].height * vpr).round() as u32;
        assert!(
            (self.engine.pane_left + self.engine.pane_w + self.engine.axis_w - f64::from(width))
                .abs()
                < 0.01
                && (self.frame.width - self.engine.pane_left - self.engine.pane_w).abs() < 0.01
                && (self.frame.height - content_h).abs() < 0.01,
            "probe layout must follow logical canvas bounds: canvas={width}x{height}, pane_left={}, pane={}x{}, axes={}+{}",
            self.engine.pane_left,
            self.engine.pane_w,
            self.engine.pane_h,
            self.engine.left_axis_w,
            self.engine.axis_w
        );
        assert_eq!(
            [pane.scissor[0], pane.scissor[2], pane.scissor[3]],
            [expected_x_px, expected_w_px, expected_h_px],
            "probe pane scissor must follow the physical negotiated pane extent"
        );
    }

    /// Forward the newest coalesced captured-drawing sample (at most one per painted frame).
    /// Only a semantic change dirties the frame.
    fn flush_pending_drawing_creation(&mut self) {
        if let Some((x, y, modifiers)) = self.pending_creation_point.take() {
            if self
                .engine
                .drawing_tool_pointer_move(x, y, modifiers, true)
                .changed
            {
                self.dirty = true;
            }
        }
    }

    /// GPUI prepaint entry: use the exact native shaper that the paint backend uses.
    fn rebuild(&mut self, width: f32, height: f32, scale_factor: f32, window: &Window) {
        self.flush_pending_drawing_creation();
        if self.built_for == (width, height, scale_factor)
            && !self.dirty
            && !self.frame.panes.is_empty()
        {
            return;
        }
        let layout = self.engine.options.get().layout.clone();
        // Live native measurement for every engine request (labels, caret prefixes, the trend
        // prompt, device-scaled frame runs), matching the browser host's canvas measurer.
        self.engine
            .set_text_measure(Some(Box::new(text_measurer(window))));
        self.engine
            .set_text_cap_center(Some(Box::new(text_cap_centerer(window))));
        let axis_size = self.engine.axis_font_size();
        let countdown_size = self.engine.countdown_font_size();
        self.rebuild_with_measure(
            width,
            height,
            scale_factor,
            |text, bold| {
                f64::from(
                    measure_text(
                        window,
                        text,
                        &layout.font_family,
                        axis_size as f32,
                        if bold { 700 } else { 400 },
                        false,
                    )
                    .width,
                )
            },
            |text, bold| {
                f64::from(
                    measure_text(
                        window,
                        text,
                        &layout.font_family,
                        countdown_size as f32,
                        if bold { 700 } else { 400 },
                        false,
                    )
                    .width,
                )
            },
        );
    }

    fn record_frame_metrics(&mut self, metrics: GpuiFrameMetrics) {
        self.last = metrics;
        self.painted += 1;
    }

    fn now_ms(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1_000.0
    }

    fn needs_animation_frame(&self) -> bool {
        self.frame_budget.is_some()
            || self.kinetic_active
            || self.engine.keyboard_scroll_active()
            || self.engine.scroll_animation_active()
    }

    fn local_position(&self, position: gpui::Point<gpui::Pixels>) -> (f64, f64, f64) {
        let window_x: f32 = position.x.into();
        let window_y: f32 = position.y.into();
        let chart_x = f64::from(window_x - self.viewport_offset.0);
        let y = f64::from(window_y - self.viewport_offset.1);
        (chart_x, chart_x - self.engine.pane_left, y)
    }

    fn separator_at(&self, y: f64) -> Option<usize> {
        self.engine
            .panes
            .iter()
            .skip(1)
            .position(|pane| (y - pane.top).abs() <= PANE_SEPARATOR_HIT)
    }

    fn update_cursor(&mut self, chart_x: f64, y: f64) {
        let pane_x = chart_x - self.engine.pane_left;
        let active_separator = matches!(self.drag, Some(DragMode::PaneSeparator { .. }));
        let separator = self
            .gesture_config
            .panes_resize
            .then(|| self.separator_at(y))
            .flatten();
        let separator_hover = (!active_separator).then_some(separator).flatten();
        if self.engine.separator_hover != separator_hover {
            self.engine.set_separator_hover(separator_hover);
            self.dirty = true;
        }
        let drawing_cursor = (self.engine.active_drawing_tool().is_none()
            && pane_x >= 0.0
            && pane_x <= self.engine.pane_w
            && y >= 0.0
            && y <= self.engine.pane_h)
            .then(|| self.engine.drawing_hover_at(pane_x, y))
            .flatten()
            .map(|(_, cursor)| match cursor {
                "text" => CursorStyle::IBeam,
                "pointer" => CursorStyle::PointingHand,
                "move" => CursorStyle::ClosedHand,
                "ns-resize" => CursorStyle::ResizeUpDown,
                "ew-resize" => CursorStyle::ResizeLeftRight,
                "nwse-resize" => CursorStyle::ResizeUpLeftDownRight,
                "nesw-resize" => CursorStyle::ResizeUpRightDownLeft,
                _ => CursorStyle::Crosshair,
            });
        let cursor = if self.engine.alert_create_hit_at(pane_x, y) {
            CursorStyle::PointingHand
        } else if active_separator || separator.is_some() {
            CursorStyle::ResizeRow
        } else if y > self.engine.pane_h {
            if self.gesture_config.axis_scale_time {
                CursorStyle::ResizeLeftRight
            } else {
                CursorStyle::Arrow
            }
        } else if chart_x < self.engine.pane_left
            || chart_x > self.engine.pane_left + self.engine.pane_w
        {
            let pane = self.engine.pane_index_at_y(y);
            let target = self.engine.price_axis_target_at(pane, pane_x);
            if self.gesture_config.axis_scale_price
                && target.is_some_and(|target| self.engine.price_axis_scalable(pane, target))
            {
                CursorStyle::ResizeUpDown
            } else {
                CursorStyle::Arrow
            }
        } else if let Some(cursor) = drawing_cursor {
            cursor
        } else if self.engine.hovered_series().is_some() {
            CursorStyle::PointingHand
        } else {
            CursorStyle::Crosshair
        };
        if self.cursor_style != cursor {
            self.cursor_style = cursor;
            self.dirty = true;
        }
    }

    fn cancel_kinetic_scroll(&mut self) {
        self.engine.kinetic_stop();
        if self.kinetic_active {
            self.engine.time_scale_end_scroll();
        }
        self.kinetic_active = false;
    }

    fn begin_keyboard_pan_at(&mut self, direction: i8, step: f64, is_held: bool, now_ms: f64) {
        debug_assert!(direction == -1 || direction == 1);
        let delta = f64::from(direction) * step;
        self.cancel_kinetic_scroll();
        self.engine.cancel_scroll_animation();
        // GPUI reports OS key repeats through `is_held`. The engine owns the repeat cadence, so
        // an unchanged repeat must not reset velocity; only a new direction or modifier speed
        // retunes the live session.
        if !is_held || self.keyboard_pan_direction != direction || self.keyboard_pan_delta != delta
        {
            self.engine.start_keyboard_scroll(delta, now_ms);
            self.keyboard_pan_direction = direction;
            self.keyboard_pan_delta = delta;
        }
    }

    fn cancel_keyboard_pan(&mut self) {
        self.engine.cancel_keyboard_scroll();
        self.keyboard_pan_direction = 0;
        self.keyboard_pan_delta = 0.0;
    }

    fn begin_mouse_pan(&mut self, pane_x: f64) {
        // Close any stale snapshot first. In particular, clicking during a previous coast must not
        // let the next `scroll_to` reuse that coast's source start point.
        self.engine.time_scale_end_scroll();
        self.engine.time_scale_start_scroll(pane_x);
        self.engine.kinetic_begin_sampling(
            self.gesture_config.kinetic_mouse,
            self.engine.scroll_position(),
            self.now_ms(),
        );
    }

    fn end_mouse_pan(&mut self) {
        self.kinetic_active = self.gesture_config.kinetic_mouse
            && self
                .engine
                .kinetic_release(self.engine.scroll_position(), self.now_ms());
        if !self.kinetic_active {
            self.engine.kinetic_stop();
            self.engine.time_scale_end_scroll();
        }
    }

    fn mark_press_moved(&mut self, pane_x: f64, y: f64) {
        if let Some((start_x, start_y)) = self.press_start {
            self.press_moved |=
                (pane_x - start_x).abs() + (y - start_y).abs() >= CLICK_SLOP_MANHATTAN;
        }
    }

    fn mouse_sample(
        &self,
        pane_x: f64,
        y: f64,
        target: InputTarget,
        modifiers: gpui::Modifiers,
    ) -> PointerSample {
        PointerSample {
            id: 1,
            device: InputDevice::Mouse,
            target,
            modifiers: InputModifiers {
                shift: modifiers.shift,
                control: modifiers.control,
                alt: modifiers.alt,
                meta: modifiers.platform,
            },
            x: pane_x,
            y,
            timestamp_ms: self.now_ms(),
            pressure: 0.5,
            tilt_x: 0.0,
            tilt_y: 0.0,
        }
    }

    fn input_target_at(&self, chart_x: f64, pane_x: f64, y: f64) -> InputTarget {
        if self.engine.alert_create_hit_at(pane_x, y) {
            InputTarget::Alert
        } else if self.gesture_config.panes_resize && self.separator_at(y).is_some() {
            InputTarget::Separator
        } else if y > self.engine.pane_h {
            InputTarget::TimeAxis
        } else if chart_x < self.engine.pane_left
            || chart_x > self.engine.pane_left + self.engine.pane_w
        {
            InputTarget::PriceAxis
        } else if self.engine.active_drawing_tool().is_some()
            || self.engine.hit_test_drawing(pane_x, y).is_some()
        {
            InputTarget::Drawing
        } else {
            InputTarget::Pane
        }
    }

    /// Browser-host parity (`clear_hover`): release every hover promotion together so the
    /// series bump, drawing promotion, and text hover ring can never desync across hosts.
    fn clear_hover(&mut self) {
        self.engine.set_hovered_series(None);
        self.engine.set_hovered_text(None);
        self.engine.set_hovered_drawing(None);
    }

    fn update_crosshair(&mut self, pane_x: f64, y: f64) {
        if pane_x >= 0.0 && pane_x <= self.engine.pane_w && y >= 0.0 && y <= self.engine.pane_h {
            self.engine.crosshair = Some((pane_x, y));
            // Browser-host parity (wasm `hover_at`): a drawing hit wins over series hits so
            // overlaps stay selectable, clears the series bump, and drives generic hover
            // promotion plus the text-only hover ring. Hit testing stays on stable order so
            // promotion cannot oscillate hover.
            // Engine-owned arbitration: a trend label or its `+ Add text` prompt wins before
            // the drawing body, so moving onto the label keeps the prompt clickable.
            if self.engine.update_drawing_hover(pane_x, y).is_some() {
                self.engine.set_hovered_series(None);
            } else {
                let hovered = self.engine.hit_test_series(pane_x, y);
                self.engine.set_hovered_series(hovered);
            }
        } else {
            self.engine.crosshair = None;
            self.clear_hover();
        }
        self.dirty = true;
    }

    fn update_pointer_feedback(&mut self, chart_x: f64, pane_x: f64, y: f64) {
        let over_separator = self.gesture_config.panes_resize && self.separator_at(y).is_some();
        if matches!(self.drag, Some(DragMode::PaneSeparator { .. })) || over_separator {
            self.engine.crosshair = None;
            self.clear_hover();
            self.dirty = true;
        } else {
            // Match the browser host: refresh the hit-test first, then derive the cursor from the
            // same move so a candle/series immediately exposes its click affordance.
            self.update_crosshair(pane_x, y);
        }
        self.update_cursor(chart_x, y);
    }

    fn update_crosshair_modifier(&mut self, control: bool, platform: bool) {
        let enabled = (control || platform)
            && (self.engine.active_drawing_tool().is_some() || self.engine.drawing_drag_active());
        if self.engine.crosshair_ohlc_magnet != enabled {
            self.engine.crosshair_ohlc_magnet = enabled;
            self.dirty = true;
        }
    }

    fn clear_pointer_state(&mut self) {
        self.input.cancel();
        match self.drag.take() {
            Some(DragMode::Pan { price_pan }) => {
                if let Some((pane, target)) = price_pan {
                    self.engine.price_axis_end_scroll(pane, target);
                }
                if self.drag_started {
                    self.engine.time_scale_end_scroll();
                }
            }
            Some(DragMode::TimeAxis) => self.engine.time_axis_end_scale(),
            Some(DragMode::PriceAxis { pane, target }) => {
                self.engine.price_axis_end_scale(pane, target);
            }
            Some(DragMode::Drawing) => self.engine.drawing_drag_end(),
            Some(DragMode::DrawingCreation) => {
                self.engine.cancel_drawing_creation();
                self.pending_creation_point = None;
            }
            Some(
                DragMode::PaneSeparator { .. } | DragMode::DeltaTooltip | DragMode::CrosshairAction,
            )
            | None => {}
        }
        self.engine.delta_tooltip_leave();
        self.sync_brushable_area();
        self.cancel_kinetic_scroll();
        self.drag_started = false;
        self.press_start = None;
        self.press_moved = false;
        self.engine.crosshair_ohlc_magnet = false;
        self.engine.crosshair = None;
        self.clear_hover();
        self.engine.set_separator_hover(None);
        self.legend = "O —  H —  L —  C —".to_string();
        self.cursor_style = CursorStyle::Crosshair;
        self.dirty = true;
    }

    fn on_hover(&mut self, hovered: &bool, _window: &mut Window, cx: &mut Context<Self>) {
        if !*hovered {
            self.clear_pointer_state();
            cx.notify();
        }
    }

    fn on_modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.update_crosshair_modifier(event.control, event.platform);
        cx.notify();
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(focus) = &self.focus_handle {
            window.focus(focus, cx);
        }
        self.cancel_kinetic_scroll();
        self.cancel_keyboard_pan();
        // Recover defensively from a stale scroll snapshot left by an interrupted host gesture.
        self.engine.time_scale_end_scroll();
        self.engine.cancel_scroll_animation();
        let (chart_x, pane_x, y) = self.local_position(event.position);
        // Typing mode behaves like the browser's focused editor: a press on the edited label
        // stays in the session, any other press commits it (blur) and proceeds normally.
        self.press_in_text_editor = self.on_text_editor(pane_x, y);
        if self.press_in_text_editor {
            // Like the browser editor: a click inside the label places the caret there.
            if self.engine.drawing_text_edit_caret_at(pane_x, y) {
                self.dirty = true;
            }
            cx.notify();
            return;
        }
        if self.engine.commit_drawing_text_edit() {
            self.dirty = true;
        }
        self.text_press_selected = self.engine.selected_drawing();
        self.input_target = self.input_target_at(chart_x, pane_x, y);
        let sample = self.mouse_sample(pane_x, y, self.input_target, event.modifiers);
        self.input.pointer_down(sample);
        self.update_crosshair_modifier(event.modifiers.control, event.modifiers.platform);
        let pane = self.engine.pane_index_at_y(y);
        self.update_cursor(chart_x, y);
        self.press_start = Some((pane_x, y));
        self.press_moved = false;

        if self.input_target == InputTarget::Alert {
            self.drag = Some(DragMode::CrosshairAction);
            cx.notify();
            return;
        }

        if event.click_count >= 2 && self.engine.drawing_tool_sequence_active() {
            self.engine.drawing_tool_activate(
                pane_x,
                y,
                DrawingModifiers {
                    magnet: event.modifiers.control || event.modifiers.platform,
                    straighten: event.modifiers.shift,
                },
            );
            self.finish_drawing_creation();
            self.press_moved = true;
            self.dirty = true;
            cx.notify();
            return;
        }

        if self.engine.active_drawing_tool().is_some() {
            let update = self.engine.drawing_tool_pointer_down(
                pane_x,
                y,
                DrawingModifiers {
                    magnet: event.modifiers.control || event.modifiers.platform,
                    straighten: event.modifiers.shift,
                },
            );
            self.creation_press_committed = update.created.is_some();
            if let Some(id) = update.created {
                self.drawing_created(id);
            }
            if update.pointer_capture {
                self.drag = Some(DragMode::DrawingCreation);
            }
            self.update_crosshair(pane_x, y);
            self.dirty |= update.changed;
            cx.notify();
            return;
        }

        if event.click_count >= 2 {
            if chart_x >= self.engine.pane_left
                && chart_x <= self.engine.pane_left + self.engine.pane_w
                && (0.0..=self.engine.pane_h).contains(&y)
                && self.clear_brushable_area_selection()
            {
                self.click_status = "brush selection cleared".into();
            } else if y > self.engine.pane_h && self.gesture_config.axis_dblclick_reset_time {
                self.engine.reset_time_scale();
            } else if self.gesture_config.axis_dblclick_reset_price {
                if let Some(target) = self.engine.price_axis_target_at(pane, pane_x) {
                    self.engine.reset_price_scale(pane, target);
                }
            }
            self.press_moved = true;
            self.dirty = true;
            cx.notify();
            return;
        }

        self.drag = if let Some(index) = self
            .gesture_config
            .panes_resize
            .then(|| self.separator_at(y))
            .flatten()
        {
            self.engine.set_separator_hover(None);
            Some(DragMode::PaneSeparator { index, last_y: y })
        } else if y > self.engine.pane_h {
            if self.gesture_config.axis_scale_time {
                self.engine.time_axis_start_scale(pane_x);
                Some(DragMode::TimeAxis)
            } else {
                None
            }
        } else if chart_x < self.engine.pane_left
            || chart_x > self.engine.pane_left + self.engine.pane_w
        {
            if let Some(target) = self.engine.price_axis_target_at(pane, pane_x) {
                if self.gesture_config.axis_scale_price
                    && self.engine.price_axis_scalable(pane, target)
                {
                    self.engine.price_axis_start_scale(pane, target, y);
                    Some(DragMode::PriceAxis { pane, target })
                } else {
                    None
                }
            } else {
                None
            }
        } else if self.engine.active_drawing_tool().is_some() {
            None
        } else if self.engine.drawing_drag_start_at(pane_x, y) {
            Some(DragMode::Drawing)
        } else if self
            .engine
            .delta_tooltip_mouse_down_with_shift(pane_x, event.modifiers.shift)
        {
            self.sync_brushable_area();
            Some(DragMode::DeltaTooltip)
        } else if self.gesture_config.pan {
            let price_pan = self
                .engine
                .price_pan_target_at(pane, pane_x, y)
                .map(|target| (pane, target));
            Some(DragMode::Pan { price_pan })
        } else {
            None
        };
        if matches!(self.drag, Some(DragMode::PaneSeparator { .. })) {
            self.engine.crosshair = None;
            self.clear_hover();
            self.dirty = true;
        } else {
            self.update_crosshair(pane_x, y);
        }
        cx.notify();
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (chart_x, pane_x, y) = self.local_position(event.position);
        let sample = self.mouse_sample(pane_x, y, self.input_target, event.modifiers);
        let update = self.input.pointer_move(sample);
        if self.engine.delta_tooltip_mouse_move(pane_x) {
            self.sync_brushable_area();
            self.dirty = true;
        }
        self.update_crosshair_modifier(event.modifiers.control, event.modifiers.platform);
        if event.dragging() {
            self.mark_press_moved(pane_x, y);
        }
        match self.drag {
            Some(DragMode::Pan { price_pan })
                if event.dragging() && update.kind == GestureUpdateKind::DragStarted =>
            {
                // Open at the 5 px crossing sample; movement starts on the following sample.
                self.begin_mouse_pan(pane_x);
                if let Some((pane, target)) = price_pan {
                    self.engine.price_axis_start_scroll(pane, target, y);
                }
                self.drag = Some(DragMode::Pan { price_pan });
                self.drag_started = true;
            }
            Some(DragMode::Pan { price_pan })
                if event.dragging()
                    && self.drag_started
                    && update.kind == GestureUpdateKind::DragMoved =>
            {
                self.engine.time_scale_scroll_to(pane_x);
                self.engine
                    .kinetic_add_sample(self.engine.scroll_position(), self.now_ms());
                if let Some((pane, target)) = price_pan {
                    self.engine.price_axis_scroll_to(pane, target, y);
                }
            }
            Some(DragMode::TimeAxis)
                if event.dragging()
                    && matches!(
                        update.kind,
                        GestureUpdateKind::DragStarted | GestureUpdateKind::DragMoved
                    ) =>
            {
                self.engine.time_axis_scale_to(pane_x);
                self.drag_started = true;
            }
            Some(DragMode::PriceAxis { pane, target })
                if event.dragging()
                    && matches!(
                        update.kind,
                        GestureUpdateKind::DragStarted | GestureUpdateKind::DragMoved
                    ) =>
            {
                self.engine.price_axis_scale_to(pane, target, y);
                self.drag_started = true;
            }
            Some(DragMode::PaneSeparator { index, last_y })
                if event.dragging()
                    && matches!(
                        update.kind,
                        GestureUpdateKind::DragStarted | GestureUpdateKind::DragMoved
                    ) =>
            {
                self.engine.drag_pane_separator(index, y - last_y);
                self.drag = Some(DragMode::PaneSeparator { index, last_y: y });
                self.drag_started = true;
                self.dirty = true;
            }
            Some(DragMode::Drawing) if event.dragging() => {
                self.engine.drawing_drag_to(
                    pane_x,
                    y,
                    DrawingModifiers {
                        magnet: event.modifiers.control || event.modifiers.platform,
                        straighten: event.modifiers.shift,
                    },
                );
            }
            Some(DragMode::DrawingCreation) if event.dragging() => {
                // Keep only the newest captured sample; `rebuild` forwards it once per painted
                // frame so native high-Hz pointer delivery does not perturb canonical tool math.
                self.pending_creation_point = Some((
                    pane_x,
                    y,
                    DrawingModifiers {
                        magnet: event.modifiers.control || event.modifiers.platform,
                        straighten: event.modifiers.shift,
                    },
                ));
            }
            Some(DragMode::DeltaTooltip) => {}
            Some(DragMode::CrosshairAction) => {}
            _ => {
                if self.engine.active_drawing_tool().is_some() {
                    let update = self.engine.drawing_tool_pointer_move(
                        pane_x,
                        y,
                        DrawingModifiers {
                            magnet: event.modifiers.control || event.modifiers.platform,
                            straighten: event.modifiers.shift,
                        },
                        event.dragging(),
                    );
                    self.dirty |= update.changed;
                }
            }
        }
        if chart_x >= self.engine.pane_left
            && chart_x <= self.engine.pane_left + self.engine.pane_w
            && (0.0..=self.engine.pane_h).contains(&y)
        {
            self.update_legend(pane_x);
        } else {
            self.legend = "O —  H —  L —  C —".to_string();
        }
        self.update_pointer_feedback(chart_x, pane_x, y);
        cx.notify();
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.press_in_text_editor) {
            return;
        }
        let (chart_x, pane_x, y) = self.local_position(event.position);
        let sample = self.mouse_sample(pane_x, y, self.input_target, event.modifiers);
        self.input.pointer_up(sample);
        if self.engine.delta_tooltip_mouse_up() {
            self.sync_brushable_area();
            self.dirty = true;
        }
        self.update_crosshair_modifier(event.modifiers.control, event.modifiers.platform);
        self.mark_press_moved(pane_x, y);
        let moved = self.press_moved;
        let committed_on_press = std::mem::take(&mut self.creation_press_committed);
        self.press_start = None;
        self.press_moved = false;
        let select_click = match self.drag.take() {
            Some(DragMode::CrosshairAction) => {
                if !moved && self.engine.activate_alert_create_at(pane_x, y) {
                    for request in self.engine.take_alert_create_requests() {
                        self.click_status = format!(
                            "action requested: pane {} price {}",
                            request.pane_index, request.price
                        );
                    }
                }
                false
            }
            Some(DragMode::Pan { price_pan }) => {
                if let Some((pane, target)) = price_pan {
                    self.engine.price_axis_end_scroll(pane, target);
                }
                if self.drag_started {
                    self.end_mouse_pan();
                }
                !moved
            }
            Some(DragMode::TimeAxis) => {
                self.engine.time_axis_end_scale();
                false
            }
            Some(DragMode::PriceAxis { pane, target }) => {
                self.engine.price_axis_end_scale(pane, target);
                false
            }
            Some(DragMode::PaneSeparator { .. }) => false,
            Some(DragMode::Drawing) => {
                self.engine.drawing_drag_end();
                false
            }
            Some(DragMode::DrawingCreation) => {
                let modifiers = DrawingModifiers {
                    magnet: event.modifiers.control || event.modifiers.platform,
                    straighten: event.modifiers.shift,
                };
                // Flush the newest coalesced move first. The controller itself then owns the exact
                // release endpoint, so browser and native hosts cannot disagree on the terminal
                // freehand point because of platform event cadence.
                self.flush_pending_drawing_creation();
                let update = self.engine.drawing_tool_pointer_up(pane_x, y, modifiers);
                if let Some(id) = update.created {
                    self.drawing_created(id);
                }
                false
            }
            Some(DragMode::DeltaTooltip) => false,
            None if committed_on_press => false,
            None if self.engine.active_drawing_tool().is_some() => {
                if !moved {
                    self.place_drawing_anchor(
                        pane_x,
                        y,
                        DrawingModifiers {
                            magnet: event.modifiers.control || event.modifiers.platform,
                            straighten: event.modifiers.shift,
                        },
                    );
                }
                false
            }
            None => !moved,
        };
        self.drag_started = false;
        if select_click {
            let selected = self.engine.hit_test_series(pane_x, y);
            self.engine.set_selected_series(selected);
            // Browser-host parity: a trend label (or its `+ Add text` prompt) opens typing on the
            // first click; a text drawing opens it when empty or already selected at press.
            if let Some(id) = self.engine.drawing_text_hit_at(pane_x, y) {
                self.engine.set_selected_drawing(Some(id));
                self.engine.begin_drawing_text_edit(id, true);
            } else if self.engine.select_drawing_at(pane_x, y) {
                if let Some(drawing) = self
                    .engine
                    .selected_drawing()
                    .and_then(|id| self.engine.drawing(id))
                    .filter(|drawing| drawing.kind == DrawingKind::Text)
                {
                    let id = drawing.id;
                    if drawing.text.trim().is_empty() || self.text_press_selected == Some(id) {
                        self.engine.begin_drawing_text_edit(id, true);
                    }
                }
            }
            self.update_legend(pane_x);
            self.click_status = format!("click x={pane_x:.1} y={y:.1}");
        }
        self.update_pointer_feedback(chart_x, pane_x, y);
        cx.notify();
    }

    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (chart_x, pane_x, y) = self.local_position(event.position);
        self.update_crosshair_modifier(event.modifiers.control, event.modifiers.platform);
        // GPUI's Windows backend reports wheel-up as a positive line delta. The browser turns
        // wheel-up's negative DOM deltaY into the same positive normalized value. Convert lines
        // with the browser's 32px adjustment and do not reverse either axis a second time.
        let delta = event.delta.pixel_delta(px(WHEEL_LINE_HEIGHT));
        let dx: f32 = delta.x.into();
        let dy: f32 = delta.y.into();
        let normalized_x = f64::from(dx) / 100.0;
        let normalized_y = f64::from(dy) / 100.0;
        let delta_mode = if matches!(event.delta, ScrollDelta::Pixels(_)) {
            WheelDeltaMode::Pixel
        } else {
            WheelDeltaMode::Line
        };
        let intent = WheelSample {
            x: pane_x,
            y,
            delta_x: normalized_x,
            delta_y: normalized_y,
            delta_mode,
            modifiers: InputModifiers {
                shift: event.modifiers.shift,
                control: event.modifiers.control,
                alt: event.modifiers.alt,
                meta: event.modifiers.platform,
            },
            timestamp_ms: self.now_ms(),
        }
        .intent(self.gesture_config.wheel_behavior);
        if matches!(intent, WheelIntent::Zoom | WheelIntent::PanAndZoom)
            && normalized_y != 0.0
            && self.gesture_config.wheel_zoom
        {
            let zoom = aeris_charts_engine::wheel_zoom_scale(normalized_y);
            let pane = self.engine.pane_index_at_y(y);
            let price_target = self.engine.price_axis_target_at(pane, pane_x);
            if self.gesture_config.wheel_behavior == WheelBehavior::Zoom {
                if let Some(price_target) = price_target {
                    self.engine
                        .price_axis_wheel_zoom(pane, price_target, y, zoom);
                } else {
                    self.engine.time_scale_zoom(pane_x, zoom);
                }
            } else {
                // Auto mode implements the reference-informed behavior: every surface targets the time scale and
                // modifiers do not change routing. Focused Ctrl zoom remains an explicit-mode
                // Aeris extension.
                if self.gesture_config.wheel_behavior == WheelBehavior::Zoom
                    && event.modifiers.control
                {
                    self.engine.time_scale_zoom_focused(pane_x, zoom);
                } else {
                    self.engine.time_scale_zoom(pane_x, zoom);
                }
            }
        }
        let pan_delta = if normalized_x.abs() >= normalized_y.abs() {
            normalized_x
        } else {
            -normalized_y
        };
        if matches!(intent, WheelIntent::Pan | WheelIntent::PanAndZoom)
            && pan_delta != 0.0
            && self.gesture_config.wheel_scroll
        {
            self.engine.time_scale_start_scroll(0.0);
            self.engine
                .time_scale_scroll_to(aeris_charts_engine::WHEEL_SCROLL_PX_PER_DELTA * pan_delta);
            self.engine.time_scale_end_scroll();
        }
        self.update_pointer_feedback(chart_x, pane_x, y);
        cx.stop_propagation();
        cx.notify();
    }

    fn on_pinch(&mut self, event: &PinchEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let (chart_x, pane_x, y) = self.local_position(event.position);
        let intent = WheelSample {
            x: pane_x,
            y,
            delta_y: f64::from(event.delta),
            modifiers: InputModifiers {
                control: true,
                ..InputModifiers::default()
            },
            timestamp_ms: self.now_ms(),
            ..WheelSample::default()
        }
        .intent(self.gesture_config.wheel_behavior);
        if intent == WheelIntent::Zoom && self.gesture_config.wheel_zoom {
            self.engine.time_scale_zoom(
                pane_x,
                aeris_charts_engine::pinch_zoom_scale(f64::from(event.delta)),
            );
            self.update_pointer_feedback(chart_x, pane_x, y);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.engine.drawing_text_edit().is_some() {
            let modifiers = event.keystroke.modifiers;
            let command = (modifiers.control || modifiers.platform) && !modifiers.alt;
            match event.keystroke.key.as_str() {
                // Clipboard shortcuts, like the browser editor: paste flattens to one line.
                "v" if command => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        self.engine.drawing_text_edit_insert(&text);
                    }
                }
                "c" | "x" if command => {
                    if let Some(selected) = self.engine.drawing_text_edit_selection() {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                            selected.to_string(),
                        ));
                        if event.keystroke.key == "x" {
                            self.engine
                                .drawing_text_edit_key(DrawingTextEditKey::Backspace, false);
                        }
                    }
                }
                _ => self.on_text_edit_key(event),
            }
            self.dirty = true;
            cx.stop_propagation();
            cx.notify();
            return;
        }
        let step = if event.keystroke.modifiers.control || event.keystroke.modifiers.shift {
            10.0
        } else {
            1.0
        };
        let center = self.engine.pane_w / 2.0;
        let handled = match event.keystroke.key.as_str() {
            "left" => {
                self.begin_keyboard_pan_at(-1, step, event.is_held, self.now_ms());
                true
            }
            "right" => {
                self.begin_keyboard_pan_at(1, step, event.is_held, self.now_ms());
                true
            }
            "+" | "=" if self.gesture_config.wheel_zoom => {
                self.engine.time_scale_zoom(center, 0.5);
                true
            }
            "-" | "_" if self.gesture_config.wheel_zoom => {
                self.engine.time_scale_zoom(center, -0.5);
                true
            }
            "home" => {
                self.engine.fit_content();
                true
            }
            "enter" => self.finish_drawing_creation(),
            "backspace"
                if self.engine.drawing_tool_sequence_active()
                    && self.engine.drawing_create_active() =>
            {
                self.pop_drawing_anchor()
            }
            "delete" | "backspace" => self.engine.remove_selected_drawing(),
            "escape" => {
                self.clear_brushable_area_selection();
                self.engine.cancel_drawing_tool();
                self.pending_creation_point = None;
                self.creation_press_committed = false;
                self.engine.set_selected_drawing(None);
                self.engine.crosshair = None;
                self.clear_hover();
                true
            }
            _ => false,
        };
        if handled {
            // Browser keyboard gestures stop any wheel/mouse coast and close its saved scroll
            // snapshot. Left/Right keeps its separate engine-owned velocity session until key-up.
            self.cancel_kinetic_scroll();
            self.dirty = true;
            cx.stop_propagation();
            cx.notify();
        }
    }

    /// Typing mode owns the keyboard: editing keys and committed characters go to the engine
    /// session; chart and workspace shortcuts stay inert until the session ends.
    fn on_text_edit_key(&mut self, event: &KeyDownEvent) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        // Word motion follows the platform convention: Ctrl on Windows/Linux, Option on macOS;
        // Cmd+arrows jump to the line ends on macOS.
        let word = if cfg!(target_os = "macos") {
            modifiers.alt
        } else {
            modifiers.control
        };
        let line = cfg!(target_os = "macos") && modifiers.platform;
        let key = match keystroke.key.as_str() {
            "enter" => {
                self.engine.commit_drawing_text_edit();
                return;
            }
            "escape" => {
                self.engine.cancel_drawing_text_edit();
                return;
            }
            "a" if (modifiers.control || modifiers.platform) && !modifiers.alt => {
                self.engine.drawing_text_edit_select_all();
                return;
            }
            "backspace" if word => Some(DrawingTextEditKey::DeleteWordBackward),
            "backspace" => Some(DrawingTextEditKey::Backspace),
            "delete" if word => Some(DrawingTextEditKey::DeleteWordForward),
            "delete" => Some(DrawingTextEditKey::Delete),
            "left" if line => Some(DrawingTextEditKey::Home),
            "right" if line => Some(DrawingTextEditKey::End),
            "left" if word => Some(DrawingTextEditKey::WordLeft),
            "right" if word => Some(DrawingTextEditKey::WordRight),
            "left" => Some(DrawingTextEditKey::Left),
            "right" => Some(DrawingTextEditKey::Right),
            "home" | "up" => Some(DrawingTextEditKey::Home),
            "end" | "down" => Some(DrawingTextEditKey::End),
            _ => None,
        };
        if let Some(key) = key {
            self.engine.drawing_text_edit_key(key, modifiers.shift);
        } else if let Some(text) = keystroke.key_char.as_deref().filter(|_| {
            // AltGr characters arrive with Ctrl+Alt on Windows; GPUI marks them as text.
            event.prefer_character_input
                || (!keystroke.modifiers.control && !keystroke.modifiers.platform)
        }) {
            self.engine.drawing_text_edit_insert(text);
        }
    }

    fn on_key_up(&mut self, event: &KeyUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let direction = match event.keystroke.key.as_str() {
            "left" => -1,
            "right" => 1,
            _ => return,
        };
        if self.keyboard_pan_direction != direction {
            return;
        }
        self.cancel_keyboard_pan();
        self.dirty = true;
        cx.stop_propagation();
        cx.notify();
    }

    fn tick_animations(&mut self) {
        let now = self.now_ms();
        if self.kinetic_active {
            if self.engine.kinetic_finished(now) {
                self.engine.time_scale_end_scroll();
                self.engine.kinetic_stop();
                self.kinetic_active = false;
            } else if let Some(position) = self.engine.kinetic_position(now) {
                self.engine.scroll_to_position(position);
                self.dirty = true;
            }
        }
        if self.engine.keyboard_scroll_active() {
            self.engine.keyboard_scroll_tick(now);
            self.dirty = true;
        }
        if self.engine.scroll_animation_active() {
            self.engine.scroll_animation_tick(now);
            self.dirty = true;
        }
    }
}

/// Build and paint one frame. Split out so both closures can hold disjoint borrows of `Probe`.
fn paint_probe(probe: &mut Probe, bounds: Bounds<gpui::Pixels>, window: &mut Window, cx: &mut App) {
    if probe
        .frame_budget
        .is_some_and(|budget| probe.painted >= budget)
    {
        return;
    }
    let viewport = AerisViewport::from_bounds(
        bounds.origin.x.into(),
        bounds.origin.y.into(),
        bounds.size.width.into(),
        bounds.size.height.into(),
    );
    let scale_factor = window.scale_factor();
    let plan_dirty = probe.plan_dirty;
    let mut cached_metrics = probe.last;
    cached_metrics.plan_nanos = 0;

    // Disjoint field borrows: the adapter reads `frame` while mutating `renderer`.
    let Probe {
        engine,
        renderer,
        frame,
        axis,
        ..
    } = probe;
    let prepared = PreparedAerisFrame::from_engine(frame, engine).with_axis(axis, &[]);
    let result = if plan_dirty {
        renderer.paint_frame(&prepared, viewport, scale_factor, window, cx)
    } else {
        renderer.paint_planned_frame(
            &prepared,
            viewport,
            scale_factor,
            window,
            cx,
            cached_metrics,
        )
    };
    match result {
        Ok(metrics) => {
            probe.plan_dirty = false;
            probe.record_frame_metrics(metrics);
        }
        Err(e) => eprintln!("aeris_charts probe: frame skipped: {e}"),
    }
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity: Entity<Probe> = cx.entity();
        let prepaint_entity = entity.clone();

        // Finite probes deliberately sample consecutive frames. Interactive charts request a
        // follow-up only while an engine-owned animation is active; ordinary input/resize/data
        // mutations already notify GPUI and an idle chart must stay idle.
        let done = self
            .frame_budget
            .is_some_and(|budget| self.painted >= budget);
        if done {
            cx.quit();
        } else if self.needs_animation_frame() {
            window.request_animation_frame();
        }

        // A solid Aeris layout background emits no `Prim`: each host clears its own surface from
        // the layout options (as WebGPU does). The probe is a GPUI host, so it must do the same;
        // otherwise GPUI's default transparent/black client clear shows through.
        let fallback = aeris_charts_core::style::DEFAULT_SURFACE_RGB;
        let background = Color::parse_css(&self.engine.options.get().layout.background.color)
            .unwrap_or(Color::rgb(fallback.0, fallback.1, fallback.2));
        let background = aeris_charts_render_gpui::backend::to_hsla(background);
        let focus = self
            .focus_handle
            .as_ref()
            .expect("the window host installs a focus handle")
            .clone();
        div()
            .relative()
            .bg(background)
            .size_full()
            .cursor(self.cursor_style)
            .id("aeris_charts-chart-root")
            .track_focus(&focus)
            .key_context("AerisGpuiChart")
            .on_hover(cx.listener(Self::on_hover))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .on_pinch(cx.listener(Self::on_pinch))
            .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .child(
                canvas(
                    move |bounds: Bounds<gpui::Pixels>, window, cx| {
                        let w: f32 = bounds.size.width.into();
                        let h: f32 = bounds.size.height.into();
                        let offset_x: f32 = bounds.origin.x.into();
                        let offset_y: f32 = bounds.origin.y.into();
                        let scale_factor = window.scale_factor();
                        prepaint_entity.update(cx, |probe: &mut Probe, _| {
                            if probe
                                .frame_budget
                                .is_some_and(|budget| probe.painted >= budget)
                            {
                                return;
                            }
                            probe.viewport_offset = (offset_x, offset_y);
                            probe.tick_animations();
                            probe.rebuild(w, h, scale_factor, window);
                        });
                        bounds
                    },
                    move |_bounds: Bounds<gpui::Pixels>, prepainted, window, cx| {
                        entity.update(cx, |probe: &mut Probe, cx| {
                            paint_probe(probe, prepainted, window, cx);
                        });
                    },
                )
                .size_full(),
            )
    }
}

impl Focusable for Probe {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle
            .as_ref()
            .expect("the window host installs a focus handle")
            .clone()
    }
}

fn layout_first(node: &WorkspaceLayout) -> u64 {
    match node {
        WorkspaceLayout::Cell { id } => *id,
        WorkspaceLayout::Split { a, .. } => layout_first(a),
    }
}

fn layout_last(node: &WorkspaceLayout) -> u64 {
    match node {
        WorkspaceLayout::Cell { id } => *id,
        WorkspaceLayout::Split { b, .. } => layout_last(b),
    }
}

const WORKSPACE_DIVIDER_LAYOUT_PX: f32 = 1.0;
const WORKSPACE_DIVIDER_HIT_PX: f32 = 5.0;

fn layout_extent_in_direction<F>(
    node: &WorkspaceLayout,
    direction: SplitDirection,
    leaf_extent: &F,
) -> f32
where
    F: Fn(u64, SplitDirection) -> f32,
{
    match node {
        WorkspaceLayout::Cell { id } => leaf_extent(*id, direction).max(0.0),
        WorkspaceLayout::Split {
            direction: split_direction,
            a,
            b,
            ..
        } => {
            let a_extent = layout_extent_in_direction(a, direction, leaf_extent);
            let b_extent = layout_extent_in_direction(b, direction, leaf_extent);
            if *split_direction == direction {
                a_extent + WORKSPACE_DIVIDER_LAYOUT_PX + b_extent
            } else {
                a_extent.max(b_extent)
            }
        }
    }
}

fn owning_split_total_extent<F>(
    node: &WorkspaceLayout,
    left: u64,
    right: u64,
    direction: SplitDirection,
    leaf_extent: &F,
) -> Option<f32>
where
    F: Fn(u64, SplitDirection) -> f32,
{
    match node {
        WorkspaceLayout::Cell { .. } => None,
        WorkspaceLayout::Split {
            direction: split_direction,
            a,
            b,
            ..
        } => {
            if *split_direction == direction && layout_last(a) == left && layout_first(b) == right {
                return Some(
                    layout_extent_in_direction(a, direction, leaf_extent)
                        + WORKSPACE_DIVIDER_LAYOUT_PX
                        + layout_extent_in_direction(b, direction, leaf_extent),
                );
            }
            owning_split_total_extent(a, left, right, direction, leaf_extent)
                .or_else(|| owning_split_total_extent(b, left, right, direction, leaf_extent))
        }
    }
}

fn split_flex_ratios(ratio: f64) -> (f32, f32) {
    let first = ratio.clamp(0.0, 1.0) as f32;
    (first, 1.0 - first)
}

#[derive(Clone, Copy, Debug)]
enum DemoAction {
    Series(SeriesKind),
    BrushableArea,
    Footprint,
    CandleBodyColor,
    CandleWickColor,
    CandleBorderColor,
    CandleWicksVisible,
    CandleBordersVisible,
    CandlePartsReset,
    LineColor,
    LineWidth,
    AreaColor,
    Sma,
    Volume,
    VolumeProfile,
    Rsi,
    Cvd,
    Split(SplitDirection),
    Close,
    Cap,
    Drawing(DrawingKind),
    ClearDrawings,
    DrawingColor,
    DrawingStyle,
    DrawingWidth,
    DrawingText,
    DrawingTextColor,
    DrawingTextSize,
    DrawingTextWeight,
    DrawingItalic,
    CrosshairMode,
    CrosshairColor,
    CrosshairWidth,
    CrosshairStyle,
    CrosshairLabelBackground,
    CrosshairLabels,
    Theme,
    Grid,
    GridColor,
    GridStyle,
    Font,
    FontSize,
    PriceLine,
    PriceLineExtent,
    PriceLineStyle,
    LastValue,
    TitleVisible,
    TitleText,
    Countdown,
    BidAsk,
    AxisBorders,
    AxisBorderColor,
    AxisText,
    Separator,
    Watermark,
    WatermarkText,
    WatermarkColor,
    WatermarkSize,
    AxisScaling,
    Kinetic,
    Reset,
    Fixture(usize),
    Controls,
}

struct DemoCell {
    id: u64,
    chart: Entity<Probe>,
}

#[derive(Clone, Copy)]
struct WorkspaceDrag {
    left: u64,
    right: u64,
    direction: SplitDirection,
    start: f32,
    start_ratio: f64,
    current_ratio: f64,
    extent: f32,
}

struct InteractiveDemo {
    workspace: Workspace,
    cells: Vec<DemoCell>,
    active: u64,
    maximized: Option<u64>,
    /// The release of an Alt+click maximize press. The layout changes under the pointer, so the
    /// mouse-up must not reach whichever chart is now there (no stray click or drawing anchor).
    swallow_mouse_up: bool,
    theme: DemoTheme,
    max_index: usize,
    max_charts: Option<usize>,
    split_asset_seq: usize,
    focus_initialized: bool,
    _root_observer: Subscription,
    workspace_drag: Option<WorkspaceDrag>,
    status: String,
    inspector_scroll: ScrollHandle,
    inspector_open: bool,
}

impl InteractiveDemo {
    fn new(bars: usize, cx: &mut Context<Self>) -> Self {
        let chart = cx.new(|cx| {
            let mut probe = Probe::new_interactive(bars);
            probe.focus_handle = Some(cx.focus_handle());
            probe
        });
        let root_observer = cx.observe(&chart, |_, _, cx| cx.notify());
        Self {
            workspace: Workspace::new(),
            cells: vec![DemoCell { id: 1, chart }],
            active: 1,
            maximized: None,
            swallow_mouse_up: false,
            theme: DemoTheme::Dark,
            max_index: 0,
            max_charts: None,
            split_asset_seq: 0,
            focus_initialized: false,
            _root_observer: root_observer,
            workspace_drag: None,
            inspector_scroll: ScrollHandle::new(),
            inspector_open: true,
            status: format!(
                "interactive native GPUI demo · active cell 1 · {} feature groups",
                TOOLBAR_FEATURE_MANIFEST.len()
            ),
        }
    }

    fn active_chart(&self) -> Option<Entity<Probe>> {
        self.cells
            .iter()
            .find(|cell| cell.id == self.active)
            .map(|cell| cell.chart.clone())
    }

    fn root_chart(&self) -> Option<Entity<Probe>> {
        self.cells
            .iter()
            .find(|cell| cell.id == 1)
            .map(|cell| cell.chart.clone())
    }

    fn update_chart(
        chart: Option<Entity<Probe>>,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Probe),
    ) {
        if let Some(chart) = chart {
            chart.update(cx, |probe, child_cx| {
                f(probe);
                probe.dirty = true;
                child_cx.notify();
            });
        }
    }

    fn update_active(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Probe)) {
        Self::update_chart(self.active_chart(), cx, f);
    }

    fn update_root(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Probe)) {
        Self::update_chart(self.root_chart(), cx, f);
    }

    fn activate(&mut self, id: u64, cx: &mut Context<Self>) {
        self.active = id;
        self.status = format!("active cell {id}");
        cx.notify();
    }

    fn split(&mut self, direction: SplitDirection, activate_new: bool, cx: &mut Context<Self>) {
        if self
            .max_charts
            .is_some_and(|max| self.workspace.chart_count() >= max)
        {
            self.status = "split rejected: host chart limit reached".into();
            return;
        }
        let end_time = self
            .root_chart()
            .and_then(|chart| chart.read(cx).source_bars.times.last().copied())
            .unwrap_or(1_600_000_000.0);
        match self.workspace.split(self.active, direction) {
            Ok(id) => {
                self.maximized = None;
                self.split_asset_seq += 1;
                let split_bars = split_asset_bars(self.split_asset_seq, end_time);
                let sequence = self.split_asset_seq;
                let chart = cx.new(|cx| {
                    let mut probe = Probe::new_interactive(300);
                    probe.replace_source_bars(split_bars);
                    probe.focus_handle = Some(cx.focus_handle());
                    probe
                });
                self.cells.push(DemoCell { id, chart });
                if activate_new {
                    self.active = id;
                }
                self.status = format!("created independent ASSET {sequence} in cell {id}");
            }
            Err(error) => self.status = format!("split rejected: {error:?}"),
        }
    }

    fn close_active(&mut self) {
        if self.active == 1 {
            self.status = "primary cell 1 is protected".into();
            return;
        }
        if self.workspace.remove(self.active).is_ok() {
            let removed = self.active;
            self.cells.retain(|cell| cell.id != removed);
            self.maximized = None;
            self.active = self.workspace.cell_ids()[0];
            self.status = format!("closed cell {removed}; active {}", self.active);
        } else {
            self.status = "the final chart cannot be closed".into();
        }
    }

    fn toggle_maximize(&mut self, id: u64) {
        self.active = id;
        self.maximized = (self.maximized != Some(id)).then_some(id);
        self.status = self.maximized.map_or_else(
            || format!("restored split layout; active cell {id}"),
            |_| format!("maximized cell {id}; Alt+click to restore"),
        );
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        if event.keystroke.key == "tab"
            && !modifiers.control
            && !modifiers.platform
            && !modifiers.alt
        {
            if modifiers.shift {
                window.focus_prev(cx);
            } else {
                window.focus_next(cx);
            }
            cx.stop_propagation();
            return;
        }
        if !(modifiers.control || modifiers.platform) || modifiers.shift || modifiers.alt {
            return;
        }
        let direction = match event.keystroke.key.as_str() {
            "h" => Some(SplitDirection::Horizontal),
            "v" => Some(SplitDirection::Vertical),
            _ => None,
        };
        if let Some(direction) = direction {
            self.split(direction, false, cx);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn drag_workspace_divider(&mut self, position: gpui::Point<gpui::Pixels>) {
        let Some(mut drag) = self.workspace_drag else {
            return;
        };
        let current: f32 = match drag.direction {
            SplitDirection::Horizontal => position.x.into(),
            SplitDirection::Vertical => position.y.into(),
        };
        let ratio = (drag.start_ratio + f64::from((current - drag.start) / drag.extent.max(1.0)))
            .clamp(0.05, 0.95);
        if (ratio - drag.current_ratio).abs() > f64::EPSILON {
            drag.current_ratio = ratio;
            self.workspace_drag = Some(drag);
            self.status = format!("resizing divider {}/{}", drag.left, drag.right);
        }
    }

    fn finish_workspace_drag(&mut self) {
        let Some(drag) = self.workspace_drag.take() else {
            return;
        };
        let delta = drag.current_ratio - drag.start_ratio;
        if delta.abs() > f64::EPSILON
            && self
                .workspace
                .resize_between(drag.left, drag.right, delta)
                .is_ok()
        {
            self.status = format!("resized divider {}/{}", drag.left, drag.right);
        }
    }

    fn apply_action(&mut self, action: DemoAction, cx: &mut Context<Self>) {
        match action {
            DemoAction::Controls => self.inspector_open = !self.inspector_open,
            DemoAction::Series(kind) => self.update_active(cx, |p| p.set_series_kind(kind)),
            DemoAction::BrushableArea => self.update_active(cx, Probe::enable_brushable_area),
            DemoAction::Footprint => self.update_active(cx, Probe::enable_footprint),
            DemoAction::CandleBodyColor => self.update_active(cx, |p| {
                let s = &mut p.engine.series[0];
                let alternate = s.up_color.as_deref() == Some(aeris_charts_core::style::MARKET_UP_CSS);
                s.up_color = Some(
                    if alternate {
                        "#2962ff"
                    } else {
                        aeris_charts_core::style::MARKET_UP_CSS
                    }
                    .into(),
                );
                s.down_color = Some(
                    if alternate {
                        "#ff9800"
                    } else {
                        aeris_charts_core::style::MARKET_DOWN_CSS
                    }
                    .into(),
                );
            }),
            DemoAction::CandleWickColor => self.update_active(cx, |p| {
                let s = &mut p.engine.series[0];
                if s.wick_up_color.is_some() {
                    s.wick_up_color = None;
                    s.wick_down_color = None;
                } else {
                    s.wick_up_color = Some("#2962ff".into());
                    s.wick_down_color = Some("#ff9800".into());
                }
            }),
            DemoAction::CandleBorderColor => self.update_active(cx, |p| {
                let s = &mut p.engine.series[0];
                if s.border_up_color.is_some() {
                    s.border_up_color = None;
                    s.border_down_color = None;
                } else {
                    s.border_up_color = Some("#2962ff".into());
                    s.border_down_color = Some("#ff9800".into());
                }
            }),
            DemoAction::CandleWicksVisible => self.update_active(cx, |p| {
                let s = &mut p.engine.series[0];
                s.wick_visible = Some(!s.wick_visible.unwrap_or(true));
            }),
            DemoAction::CandleBordersVisible => self.update_active(cx, |p| {
                let s = &mut p.engine.series[0];
                s.border_visible = Some(!s.border_visible.unwrap_or(true));
            }),
            DemoAction::CandlePartsReset => self.update_active(cx, |p| {
                let s = &mut p.engine.series[0];
                s.wick_up_color = None;
                s.wick_down_color = None;
                s.border_up_color = None;
                s.border_down_color = None;
            }),
            DemoAction::LineColor => self.update_active(cx, |p| {
                let s = &mut p.engine.series[0];
                s.line_color = Some(
                    if s.line_color.as_deref() == Some("#2196f3") {
                        "#ab47bc"
                    } else {
                        "#2196f3"
                    }
                    .into(),
                );
            }),
            DemoAction::LineWidth => self.update_active(cx, |p| {
                let s = &mut p.engine.series[0];
                s.line_width = Some(if s.line_width.unwrap_or(3.0) >= 8.0 {
                    1.0
                } else {
                    s.line_width.unwrap_or(3.0) + 1.0
                });
            }),
            DemoAction::AreaColor => self.update_active(cx, |p| {
                let s = &mut p.engine.series[0];
                let alternate = s.area_top_color.as_deref()
                    == Some(aeris_charts_core::style::MARKET_UP_CSS);
                let top = if alternate {
                    "#2962ff"
                } else {
                    aeris_charts_core::style::MARKET_UP_CSS
                };
                s.area_top_color = Some(top.into());
                s.area_bottom_color = Some(format!("{top}00"));
            }),
            DemoAction::Sma => self.update_root(cx, Probe::toggle_sma),
            DemoAction::Volume => self.update_root(cx, Probe::toggle_volume),
            DemoAction::VolumeProfile => self.update_root(cx, Probe::toggle_volume_profile),
            DemoAction::Rsi => self.update_root(cx, Probe::toggle_rsi),
            DemoAction::Cvd => self.update_root(cx, Probe::toggle_cvd),
            DemoAction::Split(direction) => self.split(direction, true, cx),
            DemoAction::Close => self.close_active(),
            DemoAction::Cap => {
                self.max_index = (self.max_index + 1) % 4;
                let cap = [None, Some(2), Some(3), Some(4)][self.max_index];
                self.max_charts = cap;
                self.status = format!("max charts: {}", cap.map_or("∞".into(), |v| v.to_string()));
            }
            DemoAction::Drawing(kind) => self.update_root(cx, |p| p.arm_drawing(kind)),
            DemoAction::ClearDrawings => self.update_root(cx, |p| p.engine.clear_drawings()),
            DemoAction::DrawingColor => self.update_root(cx, |p| {
                p.update_drawing_template(|t| {
                    t.color = if t.color == "#168ef7" { "#ff9800" } else { "#168ef7" }.into();
                });
            }),
            DemoAction::DrawingStyle => self.update_root(cx, |p| {
                p.update_drawing_template(|t| {
                    t.style = match t.style { "solid" => "dotted", "dotted" => "dashed", _ => "solid" };
                });
            }),
            DemoAction::DrawingWidth => self.update_root(cx, |p| {
                p.update_drawing_template(|t| t.width = if t.width >= 4 { 1 } else { t.width + 1 });
            }),
            DemoAction::DrawingText => self.update_root(cx, |p| {
                // Content for the next standalone Text drawing only (trend labels are typed).
                p.update_drawing_template(|t| t.text = if t.text.is_empty() { "Note".into() } else { String::new() });
            }),
            DemoAction::DrawingTextColor => self.update_root(cx, |p| {
                // Explicit ink, then back to the inherited default (line color / foreground).
                p.update_drawing_template(|t| t.text_color = match t.text_color { None => Some("#ab47bc".into()), Some(_) => None });
            }),
            DemoAction::DrawingTextSize => self.update_root(cx, |p| {
                p.update_drawing_template(|t| t.text_size = if t.text_size >= 20 { 12 } else { t.text_size + 2 });
            }),
            DemoAction::DrawingTextWeight => self.update_root(cx, |p| {
                p.update_drawing_template(|t| t.text_weight = if t.text_weight >= 700 { 400 } else { t.text_weight + 100 });
            }),
            DemoAction::DrawingItalic => self.update_root(cx, |p| {
                p.update_drawing_template(|t| t.text_italic = !t.text_italic);
            }),
            DemoAction::CrosshairMode => self.update_root(cx, |p| {
                let next = match p.engine.options.get().crosshair.mode {
                    0 => 1,
                    1 => 3,
                    3 => 2,
                    _ => 0,
                };
                p.engine.options.apply_str(&format!(r#"{{"crosshair":{{"mode":{next}}}}}"#)).unwrap();
                p.engine.crosshair_mode = crosshair_mode_from_u8(next);
            }),
            DemoAction::CrosshairColor => self.update_root(cx, |p| {
                let current = &p.engine.options.get().crosshair.vert_line.color;
                let color = if current == aeris_charts_core::style::DEFAULT_CROSSHAIR_CSS {
                    "#2962ff"
                } else {
                    aeris_charts_core::style::DEFAULT_CROSSHAIR_CSS
                };
                p.engine.options.apply_str(&format!(r#"{{"crosshair":{{"vertLine":{{"color":"{color}"}},"horzLine":{{"color":"{color}"}}}}}}"#)).unwrap();
            }),
            DemoAction::CrosshairWidth => self.update_root(cx, |p| {
                let current = p.engine.options.get().crosshair.vert_line.width;
                let width = if current >= 4.0 { 1.0 } else { current + 1.0 };
                p.engine.options.apply_str(&format!(r#"{{"crosshair":{{"vertLine":{{"width":{width}}},"horzLine":{{"width":{width}}}}}}}"#)).unwrap();
            }),
            DemoAction::CrosshairStyle => self.update_root(cx, |p| {
                let style = (p.engine.options.get().crosshair.vert_line.style + 1) % 3;
                p.engine.options.apply_str(&format!(r#"{{"crosshair":{{"vertLine":{{"style":{style}}},"horzLine":{{"style":{style}}}}}}}"#)).unwrap();
            }),
            DemoAction::CrosshairLabelBackground => self.update_root(cx, |p| {
                let current = &p.engine.options.get().crosshair.vert_line.label_background_color;
                let color = if current == aeris_charts_core::style::DEFAULT_CROSSHAIR_CSS {
                    "#2962ff"
                } else {
                    aeris_charts_core::style::DEFAULT_CROSSHAIR_CSS
                };
                p.engine.options.apply_str(&format!(r#"{{"crosshair":{{"vertLine":{{"labelBackgroundColor":"{color}"}},"horzLine":{{"labelBackgroundColor":"{color}"}}}}}}"#)).unwrap();
            }),
            DemoAction::CrosshairLabels => self.update_root(cx, |p| {
                let visible = !p.engine.options.get().crosshair.vert_line.label_visible;
                p.engine.options.apply_str(&format!(r#"{{"crosshair":{{"vertLine":{{"labelVisible":{visible}}},"horzLine":{{"labelVisible":{visible}}}}}}}"#)).unwrap();
            }),
            DemoAction::Theme => {
                self.theme = if self.theme == DemoTheme::Light { DemoTheme::Dark } else { DemoTheme::Light };
                for cell in &self.cells {
                    let theme = self.theme;
                    cell.chart.update(cx, |p, child| { p.apply_theme(theme); child.notify(); });
                }
            }
            DemoAction::Grid => self.update_root(cx, |p| {
                let visible = !p.engine.options.get().grid.vert_lines.visible;
                p.engine.options.apply_str(&format!(r#"{{"grid":{{"vertLines":{{"visible":{visible}}},"horzLines":{{"visible":{visible}}}}}}}"#)).unwrap();
            }),
            DemoAction::GridColor => {
                let theme = self.theme;
                self.update_root(cx, move |p| p.toggle_grid_color_pin(theme));
            }
            DemoAction::GridStyle => self.update_root(cx, |p| {
                let style = (p.engine.options.get().grid.vert_lines.style + 1) % 3;
                p.engine.options.apply_str(&format!(r#"{{"grid":{{"vertLines":{{"style":{style}}},"horzLines":{{"style":{style}}}}}}}"#)).unwrap();
            }),
            DemoAction::Font => self.update_root(cx, |p| {
                let current = &p.engine.options.get().layout.font_family;
                let family = if current.contains("mono") {
                    "Georgia, serif"
                } else if current.contains("Georgia") {
                    "-apple-system, BlinkMacSystemFont, 'Trebuchet MS', Roboto, Ubuntu, sans-serif"
                } else {
                    "monospace"
                };
                p.engine.options.apply_str(&format!(r#"{{"layout":{{"fontFamily":"{family}"}}}}"#)).unwrap();
                p.renderer.invalidate_caches();
            }),
            DemoAction::FontSize => self.update_root(cx, |p| {
                let current = p.engine.options.get().layout.font_size;
                let size = if current >= 20.0 { 8.0 } else { current + 1.0 };
                p.engine.options.apply_str(&format!(r#"{{"layout":{{"fontSize":{size}}}}}"#)).unwrap();
                p.renderer.invalidate_caches();
            }),
            DemoAction::PriceLine => self.update_active(cx, |p| {
                let index = p.displayed_series_index();
                p.engine.series[index].price_line_visible =
                    !p.engine.series[index].price_line_visible;
            }),
            DemoAction::PriceLineExtent => self.update_active(cx, |p| {
                let index = p.displayed_series_index();
                p.engine.series[index].price_line_extent = match p.engine.series[index].price_line_extent {
                    PriceLineExtent::Partial => PriceLineExtent::Full,
                    PriceLineExtent::Full => PriceLineExtent::Partial,
                };
            }),
            DemoAction::PriceLineStyle => self.update_active(cx, |p| {
                let index = p.displayed_series_index();
                p.engine.series[index].price_line_style =
                    (p.engine.series[index].price_line_style + 1) % 3;
            }),
            DemoAction::LastValue => self.update_active(cx, |p| {
                let index = p.displayed_series_index();
                p.engine.series[index].last_value_visible =
                    !p.engine.series[index].last_value_visible;
            }),
            DemoAction::TitleVisible => self.update_active(cx, |p| {
                let index = p.displayed_series_index();
                p.engine.series[index].title_visible = !p.engine.series[index].title_visible;
            }),
            DemoAction::TitleText => self.update_active(cx, |p| {
                let index = p.displayed_series_index();
                p.engine.series[index].title = if p.engine.series[index].title == "Aeris"
                    || p.engine.series[index].title == "ORDER FLOW"
                {
                    "ASSET".into()
                } else if p.footprint.is_some() {
                    "ORDER FLOW".into()
                } else {
                    "Aeris".into()
                };
            }),
            DemoAction::Countdown => self.update_active(cx, |p| {
                let index = p.displayed_series_index();
                p.engine.series[index].countdown_visible =
                    !p.engine.series[index].countdown_visible;
            }),
            DemoAction::BidAsk => self.update_active(cx, |p| {
                let index = p.displayed_series_index();
                let series_id = p.engine.series[index].id;
                let on = !p.engine.series[index].bid_ask_visible;
                p.engine.series[index].bid_ask_visible = on;
                p.engine.set_bid_ask(
                    series_id,
                    on.then_some(107.95),
                    on.then_some(108.05),
                );
            }),
            DemoAction::AxisBorders => self.update_root(cx, |p| { let on = !p.engine.options.get().time_scale.border_visible; p.engine.options.apply_str(&format!(r#"{{"leftPriceScale":{{"borderVisible":{on}}},"rightPriceScale":{{"borderVisible":{on}}},"timeScale":{{"borderVisible":{on}}}}}"#)).unwrap(); }),
            DemoAction::AxisBorderColor => {
                let theme = self.theme;
                self.update_root(cx, move |p| p.toggle_axis_border_pin(theme));
            }
            DemoAction::AxisText => {
                let theme = self.theme;
                self.update_root(cx, move |p| p.toggle_text_color_pin(theme));
            }
            DemoAction::Separator => {
                let theme = self.theme;
                self.update_root(cx, move |p| p.toggle_separator_pin(theme));
            }
            DemoAction::Watermark => self.update_root(cx, |p| {
                let watermark = &p.engine.options.get().watermark;
                let on = !watermark.visible;
                let text = if watermark.text.is_empty() { "Aeris" } else { &watermark.text };
                let color = if watermark.color == "rgba(0, 0, 0, 0)" { "#b0b8c480" } else { &watermark.color };
                p.engine.options.apply_str(&format!(r#"{{"watermark":{{"visible":{on},"text":"{text}","color":"{color}"}}}}"#)).unwrap();
            }),
            DemoAction::WatermarkText => self.update_root(cx, |p| { let current = &p.engine.options.get().watermark.text; let text = if current == "Aeris" { "aeris-charts" } else { "Aeris" }; p.engine.options.apply_str(&format!(r#"{{"watermark":{{"text":"{text}"}}}}"#)).unwrap(); }),
            DemoAction::WatermarkColor => self.update_root(cx, |p| { let current = &p.engine.options.get().watermark.color; let color = if current == "#b0b8c480" { "#2962ff80" } else { "#b0b8c480" }; p.engine.options.apply_str(&format!(r#"{{"watermark":{{"color":"{color}"}}}}"#)).unwrap(); }),
            DemoAction::WatermarkSize => self.update_root(cx, |p| { let current = p.engine.options.get().watermark.font_size; let size = if current >= 160.0 { 16.0 } else { current + 4.0 }; p.engine.options.apply_str(&format!(r#"{{"watermark":{{"fontSize":{size}}}}}"#)).unwrap(); }),
            DemoAction::AxisScaling => self.update_root(cx, |p| { p.gesture_config.axis_scale_price = !p.gesture_config.axis_scale_price; p.gesture_config.axis_scale_time = p.gesture_config.axis_scale_price; }),
            DemoAction::Kinetic => self.update_root(cx, |p| p.gesture_config.kinetic_mouse = !p.gesture_config.kinetic_mouse),
            DemoAction::Reset => self.update_root(cx, |p| { p.engine.reset_time_scale(); p.engine.fit_content(); for pane in 0..p.engine.panes.len() { p.engine.set_price_scale_auto_scale_for(pane, PriceScaleTarget::Left, true); p.engine.set_price_scale_auto_scale_for(pane, PriceScaleTarget::Right, true); } }),
            DemoAction::Fixture(index) => self.update_root(cx, move |p| match index {
                0 => p.fixtures.day_bands = !p.fixtures.day_bands,
                1 => p.fixtures.position_band = !p.fixtures.position_band,
                2 => p.fixtures.autoscale_band = !p.fixtures.autoscale_band,
                3 => p.toggle_markers(),
                4 => p.fixtures.plugin_watermark = !p.fixtures.plugin_watermark,
                _ => p.fixtures.vertical_line = !p.fixtures.vertical_line,
            }),
        }
        let chart_count = self.workspace.chart_count();
        if !matches!(
            action,
            DemoAction::Split(_) | DemoAction::Close | DemoAction::Cap
        ) {
            self.status = format!(
                "active {} · {} charts · {} splits",
                self.active,
                chart_count,
                chart_count.saturating_sub(1)
            );
        }
        cx.notify();
    }

    fn action_selected(&self, action: DemoAction, cx: &Context<Self>) -> bool {
        let active = self.active_chart();
        let root = self.root_chart();
        match action {
            DemoAction::Controls => self.inspector_open,
            DemoAction::Series(kind) => active.as_ref().is_some_and(|chart| {
                let probe = chart.read(cx);
                probe
                    .engine
                    .series
                    .first()
                    .is_some_and(|series| series.kind == kind)
                    && probe.footprint.is_none()
                    && (kind != SeriesKind::Area || probe.brushable_area.is_none())
            }),
            DemoAction::BrushableArea => active
                .as_ref()
                .is_some_and(|chart| chart.read(cx).brushable_area.is_some()),
            DemoAction::Footprint => active
                .as_ref()
                .is_some_and(|chart| chart.read(cx).footprint.is_some()),
            DemoAction::CandleWicksVisible => active
                .as_ref()
                .is_some_and(|chart| chart.read(cx).engine.series[0].wick_visible.unwrap_or(true)),
            DemoAction::CandleBordersVisible => active.as_ref().is_some_and(|chart| {
                chart.read(cx).engine.series[0]
                    .border_visible
                    .unwrap_or(true)
            }),
            DemoAction::Sma => root
                .as_ref()
                .is_some_and(|chart| chart.read(cx).sma_id.is_some()),
            DemoAction::VolumeProfile => root
                .as_ref()
                .is_some_and(|chart| chart.read(cx).volume_profile.is_some()),
            DemoAction::Volume => root
                .as_ref()
                .is_some_and(|chart| chart.read(cx).volume_id.is_some()),
            DemoAction::Rsi => root
                .as_ref()
                .is_some_and(|chart| chart.read(cx).rsi_id.is_some()),
            DemoAction::Cvd => root
                .as_ref()
                .is_some_and(|chart| chart.read(cx).cvd.is_some()),
            DemoAction::Cap => self.max_index != 0,
            DemoAction::Drawing(kind) => root
                .as_ref()
                .is_some_and(|chart| chart.read(cx).engine.active_drawing_tool() == Some(kind)),
            DemoAction::DrawingItalic => root
                .as_ref()
                .is_some_and(|chart| chart.read(cx).drawing_template.text_italic),
            DemoAction::CrosshairLabels => root.as_ref().is_some_and(|chart| {
                chart
                    .read(cx)
                    .engine
                    .options
                    .get()
                    .crosshair
                    .vert_line
                    .label_visible
            }),
            DemoAction::Theme => self.theme == DemoTheme::Dark,
            DemoAction::Grid => root
                .as_ref()
                .is_some_and(|chart| chart.read(cx).engine.options.get().grid.vert_lines.visible),
            DemoAction::GridColor => root
                .as_ref()
                .is_some_and(|chart| chart.read(cx).style_pins.grid_color.is_some()),
            DemoAction::PriceLine => active.as_ref().is_some_and(|chart| {
                let probe = chart.read(cx);
                probe.engine.series[probe.displayed_series_index()].price_line_visible
            }),
            DemoAction::PriceLineExtent => active.as_ref().is_some_and(|chart| {
                let probe = chart.read(cx);
                probe.engine.series[probe.displayed_series_index()].price_line_extent
                    == PriceLineExtent::Partial
            }),
            DemoAction::LastValue => active.as_ref().is_some_and(|chart| {
                let probe = chart.read(cx);
                probe.engine.series[probe.displayed_series_index()].last_value_visible
            }),
            DemoAction::TitleVisible => active.as_ref().is_some_and(|chart| {
                let probe = chart.read(cx);
                probe.engine.series[probe.displayed_series_index()].title_visible
            }),
            DemoAction::Countdown => active.as_ref().is_some_and(|chart| {
                let probe = chart.read(cx);
                probe.engine.series[probe.displayed_series_index()].countdown_visible
            }),
            DemoAction::BidAsk => active.as_ref().is_some_and(|chart| {
                let probe = chart.read(cx);
                probe.engine.series[probe.displayed_series_index()].bid_ask_visible
            }),
            DemoAction::AxisBorders => root.as_ref().is_some_and(|chart| {
                chart
                    .read(cx)
                    .engine
                    .options
                    .get()
                    .time_scale
                    .border_visible
            }),
            DemoAction::AxisBorderColor => root
                .as_ref()
                .is_some_and(|chart| chart.read(cx).style_pins.axis_border_color.is_some()),
            DemoAction::AxisText => root
                .as_ref()
                .is_some_and(|chart| chart.read(cx).style_pins.text_color.is_some()),
            DemoAction::Separator => root
                .as_ref()
                .is_some_and(|chart| chart.read(cx).style_pins.separator_color.is_some()),
            DemoAction::Watermark => root
                .as_ref()
                .is_some_and(|chart| chart.read(cx).engine.options.get().watermark.visible),
            DemoAction::AxisScaling => root
                .as_ref()
                .is_some_and(|chart| chart.read(cx).gesture_config.axis_scale_price),
            DemoAction::Kinetic => root
                .as_ref()
                .is_some_and(|chart| chart.read(cx).gesture_config.kinetic_mouse),
            DemoAction::Fixture(index) => root.as_ref().is_some_and(|chart| {
                let fixtures = chart.read(cx).fixtures;
                match index {
                    0 => fixtures.day_bands,
                    1 => fixtures.position_band,
                    2 => fixtures.autoscale_band,
                    3 => fixtures.markers,
                    4 => fixtures.plugin_watermark,
                    _ => fixtures.vertical_line,
                }
            }),
            _ => false,
        }
    }

    fn action_enabled(&self, action: DemoAction, cx: &Context<Self>) -> bool {
        let kind = self.active_chart().and_then(|chart| {
            let probe = chart.read(cx);
            if probe.footprint.is_some() {
                Some(SeriesKind::Footprint)
            } else {
                probe.engine.series.first().map(|series| series.kind)
            }
        });
        match action {
            DemoAction::CandleBodyColor
            | DemoAction::CandleWickColor
            | DemoAction::CandleBorderColor
            | DemoAction::CandleWicksVisible
            | DemoAction::CandleBordersVisible
            | DemoAction::CandlePartsReset => {
                matches!(kind, Some(SeriesKind::Candlestick | SeriesKind::Bar))
            }
            DemoAction::LineColor | DemoAction::LineWidth => matches!(
                kind,
                Some(SeriesKind::Line | SeriesKind::Area | SeriesKind::Baseline)
            ),
            DemoAction::AreaColor => kind == Some(SeriesKind::Area),
            _ => true,
        }
    }

    fn button(
        &self,
        label: &'static str,
        action: DemoAction,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity();
        let selected = self.action_selected(action, cx);
        let enabled = self.action_enabled(action, cx);
        let primary = shell_rgb(self.theme.primary(), 0x168ef7);
        let primary_foreground = shell_rgb(self.theme.primary_foreground(), 0xffffff);
        let primary_hover = shell_rgb(self.theme.primary_hover(), primary);
        let muted = shell_rgb(self.theme.muted(), 0x181818);
        let foreground = shell_rgb(theme_text(self.theme), 0xf0f0f0);
        let accent = shell_rgb(self.theme.accent(), 0x252525);
        let ring = shell_rgb(self.theme.ring(), 0x353535);
        let background = if selected { primary } else { muted };
        let foreground = if selected {
            primary_foreground
        } else {
            foreground
        };
        let border = if selected {
            primary
        } else {
            shell_rgb(theme_border(self.theme), 0x252525)
        };
        let control = div()
            .id(label)
            .flex()
            .items_center()
            .justify_center()
            .min_w(px(40.0))
            .min_h(px(40.0))
            .px_3()
            .py_2()
            .text_size(px(12.0))
            .rounded_md()
            .border_1()
            .border_color(rgb(border))
            .bg(rgb(background))
            .text_color(rgb(foreground))
            .child(label);
        if !enabled {
            return control.opacity(0.45).into_any_element();
        }
        control
            .cursor(CursorStyle::PointingHand)
            .hover(move |style| style.bg(rgb(if selected { primary_hover } else { accent })))
            .active(move |style| {
                style
                    .bg(rgb(primary_hover))
                    .text_color(rgb(primary_foreground))
            })
            .tab_index(0)
            .focus(move |style| style.border_color(rgb(ring)))
            .on_click(move |_, _, app| {
                entity.update(app, |demo, cx| demo.apply_action(action, cx));
            })
            .into_any_element()
    }

    fn group(&self, caption: &'static str, controls: Vec<AnyElement>) -> AnyElement {
        div()
            .id(caption)
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w_full()
            .gap_3()
            .py_4()
            .border_b_1()
            .border_color(rgb(shell_rgb(theme_border(self.theme), 0xe5e5e5)))
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(caption),
            )
            .child(div().flex().flex_wrap().gap_2().children(controls))
            .into_any_element()
    }

    fn render_node(
        &self,
        node: &WorkspaceLayout,
        divider_color: u32,
        divider_line_width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match node {
            WorkspaceLayout::Cell { id } => {
                let chart = self
                    .cells
                    .iter()
                    .find(|cell| cell.id == *id)
                    .expect("workspace snapshots reference a live GPUI cell")
                    .chart
                    .clone();
                let maximize_entity = cx.entity();
                let release_entity = cx.entity();
                let entity = cx.entity();
                let id = *id;
                div()
                    .relative()
                    .size_full()
                    // Alt+click toggles this cell's maximize in a multi-chart layout. Capture
                    // phase plus stop_propagation: the chart never sees the press, so the
                    // shortcut cannot also pan, select, or place a drawing anchor.
                    .capture_any_mouse_down(move |event, _, app| {
                        let toggled = maximize_entity.update(app, |demo, cx| {
                            // A new press always starts clean, even if the previous release
                            // landed outside every chart cell.
                            demo.swallow_mouse_up = false;
                            if event.button != MouseButton::Left
                                || !event.modifiers.alt
                                || (demo.cells.len() < 2 && demo.maximized.is_none())
                            {
                                return false;
                            }
                            demo.toggle_maximize(id);
                            demo.swallow_mouse_up = true;
                            cx.notify();
                            true
                        });
                        if toggled {
                            app.stop_propagation();
                        }
                    })
                    .capture_any_mouse_up(move |_, _, app| {
                        let swallow = release_entity
                            .update(app, |demo, _| std::mem::take(&mut demo.swallow_mouse_up));
                        if swallow {
                            app.stop_propagation();
                        }
                    })
                    .on_mouse_down(MouseButton::Left, move |_, _, app| {
                        entity.update(app, |demo, cx| {
                            demo.activate(id, cx);
                            cx.notify();
                        });
                    })
                    .child(chart)
                    .into_any_element()
            }
            WorkspaceLayout::Split {
                direction,
                ratio,
                a,
                b,
            } => {
                let first = self.render_node(a, divider_color, divider_line_width, cx);
                let second = self.render_node(b, divider_color, divider_line_width, cx);
                let direction = *direction;
                let left = layout_last(a);
                let right = layout_first(b);
                let start_ratio = *ratio;
                let down_entity = cx.entity();
                let drag_handle = div()
                    .absolute()
                    .cursor(match direction {
                        SplitDirection::Horizontal => CursorStyle::ResizeLeftRight,
                        SplitDirection::Vertical => CursorStyle::ResizeRow,
                    })
                    .on_mouse_down(MouseButton::Left, move |event, window, app| {
                        let start: f32 = match direction {
                            SplitDirection::Horizontal => event.position.x.into(),
                            SplitDirection::Vertical => event.position.y.into(),
                        };
                        let viewport = window.viewport_size();
                        let fallback_extent = match direction {
                            SplitDirection::Horizontal => f32::from(viewport.width),
                            SplitDirection::Vertical => f32::from(viewport.height),
                        };
                        down_entity.update(app, |demo, cx| {
                            let layout = demo.workspace.layout();
                            let measured_extent = owning_split_total_extent(
                                &layout,
                                left,
                                right,
                                direction,
                                &|id, axis| {
                                    demo.cells
                                        .iter()
                                        .find(|cell| cell.id == id)
                                        .map(|cell| {
                                            let built_for = cell.chart.read(cx).built_for;
                                            match axis {
                                                SplitDirection::Horizontal => built_for.0,
                                                SplitDirection::Vertical => built_for.1,
                                            }
                                        })
                                        .unwrap_or(0.0)
                                },
                            )
                            .filter(|extent| *extent > 0.0)
                            .unwrap_or_else(|| fallback_extent.max(1.0));
                            demo.workspace_drag = Some(WorkspaceDrag {
                                left,
                                right,
                                direction,
                                start,
                                start_ratio,
                                current_ratio: start_ratio,
                                extent: measured_extent,
                            });
                            cx.notify();
                        });
                    });
                let hit_offset = (WORKSPACE_DIVIDER_LAYOUT_PX - WORKSPACE_DIVIDER_HIT_PX) / 2.0;
                let divider_line = canvas(
                    move |bounds: Bounds<gpui::Pixels>, _, _| bounds,
                    move |_, mut bounds: Bounds<gpui::Pixels>, window, _| {
                        let dpr = window.scale_factor().max(f32::EPSILON);
                        let device_width = (divider_line_width * dpr).round().max(1.0);
                        let logical_width = device_width / dpr;
                        match direction {
                            SplitDirection::Horizontal => {
                                let edge: f32 = bounds.origin.x.into();
                                let extent: f32 = bounds.size.width.into();
                                let aligned_device =
                                    ((edge + extent / 2.0) * dpr - device_width / 2.0 + 0.5)
                                        .floor();
                                bounds.origin.x = px(aligned_device / dpr);
                                bounds.size.width = px(logical_width);
                            }
                            SplitDirection::Vertical => {
                                let edge: f32 = bounds.origin.y.into();
                                let extent: f32 = bounds.size.height.into();
                                let aligned_device =
                                    ((edge + extent / 2.0) * dpr - device_width / 2.0 + 0.5)
                                        .floor();
                                bounds.origin.y = px(aligned_device / dpr);
                                bounds.size.height = px(logical_width);
                            }
                        }
                        window.paint_quad(gpui::fill(bounds, rgb(divider_color)));
                    },
                )
                .absolute()
                .size_full();
                let divider = match direction {
                    SplitDirection::Horizontal => div()
                        .relative()
                        .h_full()
                        .w(px(WORKSPACE_DIVIDER_LAYOUT_PX))
                        .flex_shrink_0()
                        .child(divider_line)
                        .child(
                            drag_handle
                                .left(px(hit_offset))
                                .h_full()
                                .w(px(WORKSPACE_DIVIDER_HIT_PX)),
                        ),
                    SplitDirection::Vertical => div()
                        .relative()
                        .w_full()
                        .h(px(WORKSPACE_DIVIDER_LAYOUT_PX))
                        .flex_shrink_0()
                        .child(divider_line)
                        .child(
                            drag_handle
                                .top(px(hit_offset))
                                .w_full()
                                .h(px(WORKSPACE_DIVIDER_HIT_PX)),
                        ),
                };
                let effective_ratio = self
                    .workspace_drag
                    .filter(|drag| {
                        drag.left == left && drag.right == right && drag.direction == direction
                    })
                    .map_or(*ratio, |drag| drag.current_ratio);
                let (first_ratio, second_ratio) = split_flex_ratios(effective_ratio);
                let base = div().flex().size_full();
                match direction {
                    SplitDirection::Horizontal => base
                        .flex_row()
                        .child(
                            div()
                                .h_full()
                                .flex_basis(relative(first_ratio))
                                .flex_shrink(1.0)
                                .child(first),
                        )
                        .child(divider)
                        .child(
                            div()
                                .h_full()
                                .flex_basis(relative(second_ratio))
                                .flex_shrink(1.0)
                                .child(second),
                        )
                        .into_any_element(),
                    SplitDirection::Vertical => base
                        .flex_col()
                        .child(
                            div()
                                .w_full()
                                .flex_basis(relative(first_ratio))
                                .flex_shrink(1.0)
                                .child(first),
                        )
                        .child(divider)
                        .child(
                            div()
                                .w_full()
                                .flex_basis(relative(second_ratio))
                                .flex_shrink(1.0)
                                .child(second),
                        )
                        .into_any_element(),
                }
            }
        }
    }
}

impl Render for InteractiveDemo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focus_initialized {
            if let Some(focus) = self
                .root_chart()
                .and_then(|chart| chart.read(cx).focus_handle.clone())
            {
                window.focus(&focus, cx);
                self.focus_initialized = true;
            }
        }
        let chart_count = self.workspace.chart_count();
        let mut b = |label, action| self.button(label, action, cx);
        let toolbar = vec![
            self.group(
                "Series",
                vec![
                    b("candles", DemoAction::Series(SeriesKind::Candlestick)),
                    b("bars", DemoAction::Series(SeriesKind::Bar)),
                    b("line", DemoAction::Series(SeriesKind::Line)),
                    b("area", DemoAction::Series(SeriesKind::Area)),
                    b("brushable area", DemoAction::BrushableArea),
                    b("footprint", DemoAction::Footprint),
                    b("histogram", DemoAction::Series(SeriesKind::Histogram)),
                    b("baseline", DemoAction::Series(SeriesKind::Baseline)),
                ],
            ),
            self.group(
                "Candle style",
                vec![
                    b("body colors", DemoAction::CandleBodyColor),
                    b("wick colors", DemoAction::CandleWickColor),
                    b("border colors", DemoAction::CandleBorderColor),
                    b("wicks", DemoAction::CandleWicksVisible),
                    b("borders", DemoAction::CandleBordersVisible),
                    b("reset parts", DemoAction::CandlePartsReset),
                ],
            ),
            self.group(
                "Line / area style",
                vec![
                    b("line color", DemoAction::LineColor),
                    b("line width", DemoAction::LineWidth),
                    b("area fill", DemoAction::AreaColor),
                ],
            ),
            self.group(
                "Indicators",
                vec![
                    b("SMA(20)", DemoAction::Sma),
                    b("volume overlay", DemoAction::Volume),
                    b("volume profile", DemoAction::VolumeProfile),
                    b("RSI(14) pane", DemoAction::Rsi),
                    b("CVD pane", DemoAction::Cvd),
                ],
            ),
            self.group(
                "Multi-chart",
                vec![
                    b("split ↔", DemoAction::Split(SplitDirection::Horizontal)),
                    b("split ↕", DemoAction::Split(SplitDirection::Vertical)),
                    b("close", DemoAction::Close),
                    b("max ∞/2/3/4", DemoAction::Cap),
                    div()
                        .text_xs()
                        .text_color(rgb(0x787b86))
                        .child(format!(
                            "{} chart{} · {} splits",
                            chart_count,
                            if chart_count == 1 { "" } else { "s" },
                            chart_count.saturating_sub(1)
                        ))
                        .into_any_element(),
                ],
            ),
            self.group(
                "Drawing tools",
                vec![
                    b("trend", DemoAction::Drawing(DrawingKind::TrendLine)),
                    b("h-line", DemoAction::Drawing(DrawingKind::HorizontalLine)),
                    b("h-ray", DemoAction::Drawing(DrawingKind::HorizontalRay)),
                    b("v-line", DemoAction::Drawing(DrawingKind::VerticalLine)),
                    b("rect", DemoAction::Drawing(DrawingKind::Rectangle)),
                    b("text", DemoAction::Drawing(DrawingKind::Text)),
                    b("path", DemoAction::Drawing(DrawingKind::Path)),
                    b("brush", DemoAction::Drawing(DrawingKind::Brush)),
                    b("clear", DemoAction::ClearDrawings),
                ],
            ),
            self.group(
                "Drawing style",
                vec![
                    b("color", DemoAction::DrawingColor),
                    b("style", DemoAction::DrawingStyle),
                    b("width", DemoAction::DrawingWidth),
                    b("label", DemoAction::DrawingText),
                    b("text color", DemoAction::DrawingTextColor),
                    b("size", DemoAction::DrawingTextSize),
                    b("weight", DemoAction::DrawingTextWeight),
                    b("italic", DemoAction::DrawingItalic),
                ],
            ),
            self.group(
                "Crosshair",
                vec![
                    b("mode", DemoAction::CrosshairMode),
                    b("color", DemoAction::CrosshairColor),
                    b("width", DemoAction::CrosshairWidth),
                    b("style", DemoAction::CrosshairStyle),
                    b("label bg", DemoAction::CrosshairLabelBackground),
                    b("labels", DemoAction::CrosshairLabels),
                ],
            ),
            self.group(
                "Chart",
                vec![
                    b("light/dark", DemoAction::Theme),
                    b("grid on/off", DemoAction::Grid),
                    b("grid color follow/custom", DemoAction::GridColor),
                    b("grid style", DemoAction::GridStyle),
                    b("font family", DemoAction::Font),
                    b("font size", DemoAction::FontSize),
                ],
            ),
            self.group(
                "Series chrome",
                vec![
                    b("price line", DemoAction::PriceLine),
                    b("partial/full", DemoAction::PriceLineExtent),
                    b("line style", DemoAction::PriceLineStyle),
                    b("last value", DemoAction::LastValue),
                    b("title chip", DemoAction::TitleVisible),
                    b("title text", DemoAction::TitleText),
                    b("countdown", DemoAction::Countdown),
                    b("bid/ask", DemoAction::BidAsk),
                ],
            ),
            self.group(
                "Axes",
                vec![
                    b("borders", DemoAction::AxisBorders),
                    b("border color", DemoAction::AxisBorderColor),
                    b("text color", DemoAction::AxisText),
                    b("separator", DemoAction::Separator),
                ],
            ),
            self.group(
                "Watermark",
                vec![
                    b("show", DemoAction::Watermark),
                    b("text", DemoAction::WatermarkText),
                    b("color", DemoAction::WatermarkColor),
                    b("size", DemoAction::WatermarkSize),
                ],
            ),
            self.group(
                "Interaction",
                vec![
                    b("axis scaling", DemoAction::AxisScaling),
                    b("mouse kinetic", DemoAction::Kinetic),
                    b("reset view", DemoAction::Reset),
                ],
            ),
            self.group(
                "Native overlay approximations",
                vec![
                    b("day bands", DemoAction::Fixture(0)),
                    b("position band", DemoAction::Fixture(1)),
                    b("autoscale band", DemoAction::Fixture(2)),
                    b("markers", DemoAction::Fixture(3)),
                    b("canvas v-line fixture", DemoAction::Fixture(5)),
                ],
            ),
        ];
        let (divider_color, legend) = self.root_chart().map_or_else(
            || {
                (
                    shell_rgb(theme_border(self.theme), 0xe5e5e5),
                    "O —  H —  L —  C —".to_string(),
                )
            },
            |chart| {
                let probe = chart.read(cx);
                (
                    shell_rgb(
                        &probe.engine.options.get().time_scale.border_color,
                        shell_rgb(theme_border(self.theme), 0xe5e5e5),
                    ),
                    format!("{}  ·  {}", probe.legend, probe.click_status),
                )
            },
        );
        let dpr = window.scale_factor();
        let divider_line_width =
            aeris_charts_core::style::border_width_device_px(f64::from(dpr)) as f32 / dpr;
        let chart = if let Some(id) = self.maximized {
            self.render_node(
                &WorkspaceLayout::Cell { id },
                divider_color,
                divider_line_width,
                cx,
            )
        } else {
            let layout = self.workspace.layout();
            self.render_node(&layout, divider_color, divider_line_width, cx)
        };
        let move_entity = cx.entity();
        let up_entity = move_entity.clone();
        let up_out_entity = move_entity.clone();
        let entity_id = cx.entity_id();
        div()
            .id("interactive-demo-root")
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(shell_rgb(self.theme.surface(), 0x141414)))
            .text_color(rgb(shell_rgb(theme_text(self.theme), 0xf0f0f0)))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_move(move |event, _, app| {
                if event.dragging() {
                    move_entity.update(app, |demo, cx| {
                        if demo.workspace_drag.is_some() {
                            demo.drag_workspace_divider(event.position);
                            cx.notify();
                        }
                    });
                }
            })
            .on_mouse_up(MouseButton::Left, move |_, _, app| {
                up_entity.update(app, |demo, cx| {
                    if demo.workspace_drag.is_some() {
                        demo.finish_workspace_drag();
                        cx.notify();
                    }
                });
            })
            .on_mouse_up_out(MouseButton::Left, move |_, _, app| {
                up_out_entity.update(app, |demo, cx| {
                    if demo.workspace_drag.is_some() {
                        demo.finish_workspace_drag();
                        cx.notify();
                    }
                });
            })
            .child(
                div()
                    .id("demo-header")
                    .flex()
                    .items_center()
                    .justify_between()
                    .flex_shrink_0()
                    .h(px(64.0))
                    .px_4()
                    .border_b_1()
                    .border_color(rgb(shell_rgb(theme_border(self.theme), 0xe5e5e5)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(15.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("Aeris Charts"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(shell_rgb(
                                        self.theme.muted_foreground(),
                                        0xb7b7b7,
                                    )))
                                    .child("Market lab · Native"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(self.button("Reset view", DemoAction::Reset, cx))
                            .child(self.button("Light / dark", DemoAction::Theme, cx))
                            .child(self.button("Controls", DemoAction::Controls, cx)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(chart)
                            .child(
                                div()
                                    .absolute()
                                    .top_3()
                                    .left_3()
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .bg(rgb(shell_rgb(self.theme.surface(), 0x141414)))
                                    .text_size(px(12.0))
                                    .child(legend),
                            ),
                    )
                    .when(self.inspector_open, |body| {
                        body.child(
                            div()
                                .id("demo-inspector")
                                .flex()
                                .flex_col()
                                .flex_shrink_0()
                                .w(px(340.0))
                                .h_full()
                                .border_l_1()
                                .border_color(rgb(shell_rgb(theme_border(self.theme), 0xe5e5e5)))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_2()
                                        .p_4()
                                        .flex_shrink_0()
                                        .border_b_1()
                                        .border_color(rgb(shell_rgb(
                                            theme_border(self.theme),
                                            0xe5e5e5,
                                        )))
                                        .child(
                                            div()
                                                .text_size(px(16.0))
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .child("Chart controls"),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(12.0))
                                                .text_color(rgb(shell_rgb(
                                                    self.theme.muted_foreground(),
                                                    0xb7b7b7,
                                                )))
                                                .child("Style your chart or explore a section."),
                                        )
                                        .child(
                                            div().flex().flex_wrap().gap_1().children(
                                                [
                                                    ("Series", 0),
                                                    ("Analysis", 3),
                                                    ("Workspace", 4),
                                                    ("Drawings", 5),
                                                    ("Appearance", 7),
                                                    ("Examples", 13),
                                                ]
                                                .into_iter()
                                                .map(|(label, index)| {
                                                    let scroll = self.inspector_scroll.clone();
                                                    let primary =
                                                        shell_rgb(self.theme.primary(), 0x168ef7);
                                                    let primary_foreground = shell_rgb(
                                                        self.theme.primary_foreground(),
                                                        0xffffff,
                                                    );
                                                    let ring =
                                                        shell_rgb(self.theme.ring(), 0x353535);
                                                    div()
                                                        .id(("section", index))
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .min_h(px(40.0))
                                                        .px_2()
                                                        .rounded_md()
                                                        .text_size(px(12.0))
                                                        .cursor(CursorStyle::PointingHand)
                                                        .tab_index(0)
                                                        .border_1()
                                                        .border_color(rgb(shell_rgb(
                                                            theme_border(self.theme),
                                                            0xe5e5e5,
                                                        )))
                                                        .hover(move |style| {
                                                            style
                                                                .bg(rgb(primary))
                                                                .text_color(rgb(primary_foreground))
                                                        })
                                                        .focus(move |style| {
                                                            style.border_color(rgb(ring))
                                                        })
                                                        .on_click(move |_, _, cx| {
                                                            scroll.scroll_to_top_of_item(index);
                                                            cx.notify(entity_id);
                                                        })
                                                        .child(label)
                                                }),
                                            ),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("interactive-toolbar")
                                        .tab_group()
                                        .tab_stop(false)
                                        .flex()
                                        .flex_col()
                                        .flex_1()
                                        .min_h_0()
                                        .overflow_y_scroll()
                                        .track_scroll(&self.inspector_scroll)
                                        .px_4()
                                        .children(toolbar),
                                ),
                        )
                    }),
            )
            .child(
                div()
                    .id("demo-status")
                    .flex()
                    .items_center()
                    .h(px(32.0))
                    .flex_shrink_0()
                    .px_4()
                    .border_t_1()
                    .border_color(rgb(shell_rgb(theme_border(self.theme), 0xe5e5e5)))
                    .text_size(px(11.0))
                    .text_color(rgb(shell_rgb(self.theme.muted_foreground(), 0xb7b7b7)))
                    .child(format!(
                        "{} · active {} · cap {}{}",
                        self.status,
                        self.active,
                        ["∞", "2", "3", "4"][self.max_index],
                        self.maximized
                            .map(|id| format!(" · cell {id} maximized"))
                            .unwrap_or_default(),
                    )),
            )
    }
}

enum AppRoot {
    Interactive(Entity<InteractiveDemo>),
    Finite(Entity<Probe>),
}

impl Render for AppRoot {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        match self {
            Self::Interactive(view) => view.clone().into_any_element(),
            Self::Finite(view) => view.clone().into_any_element(),
        }
    }
}

fn main() {
    let budget = std::env::var("AERIS_CHARTS_PROBE_FRAMES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());
    let bars = std::env::var("AERIS_CHARTS_PROBE_BARS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(if budget.is_none() {
            1_000usize
        } else {
            500usize
        });

    application().run(move |cx: &mut App| {
        let interactive = budget.is_none();
        let bounds = Bounds::centered(
            None,
            if interactive {
                size(px(1280.0), px(820.0))
            } else {
                size(px(1024.0), px(640.0))
            },
            cx,
        );
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |window, cx| {
                window.set_window_title("Aeris Charts — Market lab");
                let root = if interactive {
                    AppRoot::Interactive(cx.new(|cx| InteractiveDemo::new(bars, cx)))
                } else {
                    AppRoot::Finite(cx.new(|cx| {
                        let mut probe = Probe::new(bars, budget);
                        if std::env::var("AERIS_CHARTS_PROBE_FEATURE").as_deref() == Ok("footprint")
                        {
                            probe.enable_footprint();
                        }
                        probe.focus_handle = Some(cx.focus_handle());
                        probe
                    }))
                };
                cx.new(|_| root)
            },
        )
        .expect("the probe window opens");
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Typing mode is engine-owned in GPUI: a new text drawing opens the session, committed
    /// characters and editing keys reach it, shortcuts never type, and Enter commits.
    #[test]
    fn text_drawing_typing_mode_edits_through_the_engine_session() {
        use gpui::{Keystroke, Modifiers};

        let key = |key: &str, key_char: Option<&str>, modifiers: Modifiers| KeyDownEvent {
            keystroke: Keystroke {
                modifiers,
                key: key.into(),
                key_char: key_char.map(str::to_string),
            },
            is_held: false,
            prefer_character_input: false,
        };
        let mut probe = Probe::new(32, Some(1));
        let id = probe
            .engine
            .add_drawing(
                DrawingKind::Text,
                0,
                vec![DrawingPoint {
                    logical: 10.0,
                    price: 100.0,
                }],
                None,
            )
            .expect("text drawing");
        // Beginning an edit needs a settled chart: the editor layout converts the anchors.
        probe.rebuild_with_measure(
            1024.0,
            640.0,
            1.0,
            |text, _bold| text.chars().count() as f64 * 7.0,
            |text, _bold| text.chars().count() as f64 * 6.0,
        );
        probe.drawing_created(id);
        assert_eq!(probe.engine.editing_drawing(), Some(id));

        for c in ["H", "i", "!"] {
            probe.on_text_edit_key(&key(c, Some(c), Modifiers::default()));
        }
        probe.on_text_edit_key(&key("left", None, Modifiers::default()));
        probe.on_text_edit_key(&key("backspace", None, Modifiers::default()));
        let ctrl = Modifiers {
            control: true,
            ..Modifiers::default()
        };
        probe.on_text_edit_key(&key("z", Some("z"), ctrl));
        let mut alt_gr = key("q", Some("@"), Modifiers { alt: true, ..ctrl });
        alt_gr.prefer_character_input = true;
        probe.on_text_edit_key(&alt_gr);
        assert_eq!(probe.engine.drawing_text_edit(), Some((id, "H@!", 2)));

        // Shift+Home selects to the start; typing replaces the selection.
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        probe.on_text_edit_key(&key("home", None, shift));
        assert_eq!(probe.engine.drawing_text_edit_selection(), Some("H@"));
        probe.on_text_edit_key(&key("W", Some("W"), shift));
        assert_eq!(probe.engine.drawing_text_edit(), Some((id, "W!", 1)));
        // Select-all, then a word-delete clears the selection.
        probe.on_text_edit_key(&key("a", Some("a"), ctrl));
        assert_eq!(probe.engine.drawing_text_edit_selection(), Some("W!"));
        probe.on_text_edit_key(&key("right", None, Modifiers::default()));
        assert_eq!(probe.engine.drawing_text_edit(), Some((id, "W!", 2)));

        probe.on_text_edit_key(&key("enter", None, Modifiers::default()));
        assert_eq!(probe.engine.editing_drawing(), None);
        assert_eq!(probe.engine.drawing(id).unwrap().text, "W!");
    }

    /// Indicator panes share the content height with the primary pane. Adding one must not trip
    /// the layout self-check, and CVD must be a real trade-derived study aligned to the candles.
    #[test]
    fn indicator_panes_rebuild_and_cvd_follows_the_candle_times() {
        let mut probe = Probe::new(64, Some(1));
        let rebuild = |probe: &mut Probe| {
            probe.rebuild_with_measure(
                1024.0,
                640.0,
                1.25,
                |text, _| text.len() as f64 * 7.0,
                |text, _| text.len() as f64 * 6.0,
            );
        };
        rebuild(&mut probe);
        probe.toggle_rsi();
        rebuild(&mut probe);
        probe.toggle_cvd();
        rebuild(&mut probe);
        assert_eq!(probe.engine.panes.len(), 3, "{}", probe.click_status);

        let cvd = probe.cvd.expect("CVD installs from the demo trade tape");
        let (times, ..) = probe
            .engine
            .data_layer()
            .series_data(cvd.series_id)
            .unwrap();
        let candle_times = &probe.source_bars.times;
        assert_eq!(times.len(), candle_times.len());
        assert!(times
            .iter()
            .zip(candle_times)
            .all(|(cvd, candle)| (*cvd as f64 - candle).abs() < 1e-9));

        probe.toggle_cvd();
        probe.toggle_rsi();
        rebuild(&mut probe);
        assert_eq!(probe.engine.panes.len(), 1);
    }

    #[test]
    fn action_chip_hover_uses_click_cursor_and_control_input_target() {
        let mut probe = Probe::new(64, Some(1));
        probe.rebuild_with_measure(
            1024.0,
            640.0,
            1.0,
            |text, _| text.len() as f64 * 7.0,
            |text, _| text.len() as f64 * 6.0,
        );
        probe.engine.clear_drawings();
        let pane_x = probe.engine.pane_w - 9.5;
        let chart_x = probe.engine.pane_left + pane_x;
        let y = 200.0;
        probe.update_pointer_feedback(chart_x, pane_x, y);
        assert!(probe.engine.alert_create_hit_at(pane_x, y));
        assert_eq!(probe.cursor_style, CursorStyle::PointingHand);
        assert_eq!(
            probe.input_target_at(chart_x, pane_x, y),
            InputTarget::Alert
        );
        probe.engine.set_alert_create_button_visible(false);
        probe.update_pointer_feedback(chart_x, pane_x, y);
        assert_ne!(
            probe.input_target_at(chart_x, pane_x, y),
            InputTarget::Alert
        );
    }

    /// Issue #12: Wayland delivers per-HID-report pointer motion (~1000 Hz, often one axis per
    /// event). A captured drawing stream must forward at most one coalesced sample per painted
    /// frame — the newest one — or device-specific event cadence leaks into canonical geometry.
    #[test]
    fn drawing_capture_coalesces_pointer_samples_to_one_knot_per_frame() {
        let mut probe = Probe::new(32, Some(1));
        let measure = |text: &str, _bold: bool| text.chars().count() as f64 * 7.0;
        let countdown_measure = |text: &str, _bold: bool| text.chars().count() as f64 * 6.0;
        probe.rebuild_with_measure(1024.0, 640.0, 1.0, measure, countdown_measure);
        assert!(probe
            .engine
            .set_drawing_tool(Some(DrawingKind::Brush), None, None));
        assert!(
            probe
                .engine
                .drawing_tool_pointer_down(100.0, 100.0, DrawingModifiers::default())
                .pointer_capture
        );

        // A diagonal drag as Wayland reports it: one axis per event, far above frame cadence.
        for (x, y) in [
            (101.5, 100.0),
            (101.5, 101.5),
            (103.0, 101.5),
            (103.0, 103.0),
        ] {
            probe.pending_creation_point = Some((x, y, DrawingModifiers::default()));
        }
        probe.flush_pending_drawing_creation();
        assert!(probe.dirty, "a captured knot repaints the frame");
        assert_eq!(
            probe.pending_creation_point, None,
            "the pending sample is consumed by the frame"
        );

        // An idle frame with no pending sample captures nothing.
        probe.dirty = false;
        probe.flush_pending_drawing_creation();
        assert!(!probe.dirty);

        // Only the newest sample became a knot: start + one coalesced capture.
        let id = probe
            .engine
            .drawing_tool_pointer_up(103.0, 103.0, DrawingModifiers::default())
            .created
            .expect("captured drawing commits");
        let drawing = probe
            .engine
            .drawings()
            .iter()
            .find(|d| d.id == id)
            .expect("the committed brush exists");
        assert_eq!(
            drawing.points.len(),
            2,
            "intermediate staircase samples must not become knots"
        );
    }

    #[test]
    fn volume_profile_demo_owns_its_sources() {
        let mut probe = Probe::new_interactive(32);
        probe.toggle_volume_profile();
        let (id, volume) = probe.volume_profile.unwrap();
        probe.engine.time_scale.set_width(1024.0);
        probe.engine.fit_content();
        probe.engine.build_frame();
        assert!(
            probe
                .engine
                .volume_profile_indicator_snapshot(id)
                .unwrap()
                .profile
                .total_volume
                > 0.0
        );
        probe.toggle_volume_profile();
        assert!(probe.volume_profile.is_none());
        assert!(probe.engine.volume_profile_indicator_snapshot(id).is_none());
        assert!(!probe.engine.remove_series(volume));
    }

    #[test]
    fn resize_replaces_negotiated_pane_dimensions_at_fractional_dpr() {
        let mut probe = Probe::new(32, Some(1));
        let measure = |text: &str, _bold: bool| text.chars().count() as f64 * 7.0;
        let countdown_measure = |text: &str, _bold: bool| text.chars().count() as f64 * 6.0;
        probe.rebuild_with_measure(1024.0, 640.0, 1.5, measure, countdown_measure);
        let old_scissor = probe.frame.panes[0].scissor;

        probe.rebuild_with_measure(1536.0, 864.0, 1.5, measure, countdown_measure);

        let content_h = 864.0 - probe.engine.time_axis_height();
        let pane_w = 1536.0 - probe.engine.left_axis_w - probe.engine.axis_w;
        assert_eq!(probe.engine.pane_w, pane_w);
        assert_eq!(probe.engine.pane_h, content_h);
        assert_eq!(
            probe.frame.width,
            probe.engine.pane_left + probe.engine.pane_w
        );
        assert_eq!(probe.frame.height, content_h);
        assert_eq!(probe.frame.pixel_ratio, 1.5);
        assert_eq!(
            probe.frame.panes[0].scissor,
            [
                (probe.engine.pane_left * 1.5).round() as u32,
                0,
                (pane_w * 1.5).round() as u32,
                (content_h * 1.5).round() as u32,
            ]
        );
        assert_ne!(probe.frame.panes[0].scissor, old_scissor);
        assert!(probe
            .axis
            .iter()
            .any(|prim| matches!(prim, Prim::Text { .. })));
    }

    #[test]
    fn desktop_gesture_defaults_match_browser_host() {
        let config = GestureConfig::default();

        assert!(config.pan);
        assert!(config.wheel_scroll);
        assert!(config.wheel_zoom);
        assert!(config.axis_dblclick_reset_time);
        assert!(config.axis_dblclick_reset_price);
        assert!(config.axis_scale_price);
        assert!(config.axis_scale_time);
        assert!(config.panes_resize);
        assert!(!config.kinetic_mouse);
    }

    #[test]
    fn click_after_pan_preserves_position_and_next_pan_uses_current_snapshot() {
        let mut probe = Probe::new(64, Some(1));
        probe.rebuild_with_measure(
            1024.0,
            640.0,
            1.0,
            |text, _bold| text.chars().count() as f64 * 7.0,
            |text, _bold| text.chars().count() as f64 * 6.0,
        );
        probe.engine.scroll_to_position(0.0);
        let initial = probe.engine.scroll_position();

        probe.begin_mouse_pan(200.0);
        probe.engine.time_scale_scroll_to(160.0);
        probe.end_mouse_pan();
        let after_first_pan = probe.engine.scroll_position();
        let first_delta = after_first_pan - initial;
        assert!(first_delta > 0.0);

        // A press/release below click slop still enters the pane pan recognizer, but must close its
        // snapshot without restoring the position reached by the previous drag.
        probe.cancel_kinetic_scroll();
        probe.begin_mouse_pan(120.0);
        probe.end_mouse_pan();
        let after_click = probe.engine.scroll_position();
        assert!((after_click - after_first_pan).abs() < 1e-12);

        // A fresh 20px drag must start at `after_click`, rather than reusing the first drag's saved
        // state. It therefore moves half as far as the first 40px drag at unchanged bar spacing.
        probe.begin_mouse_pan(120.0);
        probe.engine.time_scale_scroll_to(100.0);
        probe.end_mouse_pan();
        let after_second_pan = probe.engine.scroll_position();
        let expected = after_click + first_delta / 2.0;
        assert!((after_second_pan - expected).abs() < 1e-12);
    }

    fn assert_theme(engine: &ChartEngine, theme: DemoTheme) {
        let options = engine.options.get();
        let background = theme.surface();
        let border = theme_border(theme);
        let text = theme_text(theme);
        assert_eq!(options.layout.background.color, background);
        assert_eq!(options.layout.text_color, text);
        assert_eq!(options.left_price_scale.border_color, border);
        assert_eq!(options.right_price_scale.border_color, border);
        assert_eq!(options.time_scale.border_color, border);
        assert_eq!(options.grid.vert_lines.color, border);
        assert_eq!(options.grid.horz_lines.color, border);
        assert_eq!(options.layout.panes.separator_color, border);
        assert_eq!(options.crosshair.vert_line.color, theme.crosshair());
        assert_eq!(
            options.crosshair.horz_line.label_background_color,
            theme.crosshair()
        );
    }

    #[test]
    fn gpui_package_themes_use_exact_tokens_on_every_axis() {
        let mut probe = Probe::new(32, Some(1));
        assert_theme(&probe.engine, DemoTheme::Light);
        probe.apply_theme(DemoTheme::Dark);
        assert_theme(&probe.engine, DemoTheme::Dark);
    }

    #[test]
    fn interactive_demo_uses_engine_defaults_except_hidden_grid() {
        let probe = Probe::new_interactive(32);
        let mut expected = aeris_charts_core::options::ChartOptions::default();
        expected.grid.vert_lines.visible = false;
        expected.grid.horz_lines.visible = false;
        assert_eq!(probe.engine.options.get(), &expected);

        let series = &probe.engine.series[0];
        assert!(series.title.is_empty());
        assert!(series.title_visible);
        assert!(series.countdown_visible);
        assert!(series.price_line_visible);
        assert_eq!(series.price_line_extent, PriceLineExtent::Partial);
    }

    #[test]
    fn toolbar_manifest_covers_the_full_native_demo_surface() {
        let manifest = TOOLBAR_FEATURE_MANIFEST.join("|");
        for required in [
            "candlestick",
            "baseline",
            "brushable-area",
            "footprint",
            "sma20",
            "rsi14",
            "split-horizontal",
            "resize",
            "brush",
            "path",
            "text-color",
            "crosshair",
            "price-line",
            "extent",
            "bid-ask",
            "separator",
            "mouse-kinetic",
            "vertical-line",
        ] {
            assert!(
                manifest.contains(required),
                "missing toolbar feature {required}"
            );
        }
        assert!(!GestureConfig::default().kinetic_mouse);
    }

    #[test]
    fn drawing_template_controls_compose_without_restarting_creation() {
        let mut probe = Probe::new(64, Some(1));
        probe.rebuild_with_measure(
            1024.0,
            640.0,
            1.0,
            |text, _bold| text.chars().count() as f64 * 7.0,
            |text, _bold| text.chars().count() as f64 * 6.0,
        );
        probe.engine.clear_drawings();
        probe.arm_drawing(DrawingKind::TrendLine);
        assert_eq!(
            probe.place_drawing_anchor(200.0, 180.0, DrawingModifiers::default()),
            -1
        );
        probe.update_drawing_template(|template| {
            template.color = "#ff9800".into();
            template.width = 4;
            template.text_italic = true;
        });
        assert!(probe.engine.drawing_create_active());
        let id = probe.place_drawing_anchor(500.0, 300.0, DrawingModifiers::default());
        assert!(id > 0, "changing style must not discard the first anchor");
        assert_eq!(probe.drawing_template.color, "#ff9800");
        assert_eq!(probe.drawing_template.width, 4);
        assert!(probe.drawing_template.text_italic);
    }

    /// Moving from a trend line onto its `+ Add text` prompt keeps the prompt hovered with the
    /// text cursor (the browser's engine-owned arbitration), so the label stays clickable.
    #[test]
    fn trend_label_prompt_stays_hovered_with_a_text_cursor() {
        let mut probe = Probe::new(64, Some(1));
        let rebuild = |probe: &mut Probe| {
            probe.rebuild_with_measure(
                1024.0,
                640.0,
                1.0,
                |text, _bold| text.chars().count() as f64 * 7.0,
                |text, _bold| text.chars().count() as f64 * 6.0,
            );
        };
        rebuild(&mut probe);
        probe.engine.clear_drawings();
        probe.arm_drawing(DrawingKind::TrendLine);
        probe.place_drawing_anchor(200.0, 300.0, DrawingModifiers::default());
        let id = probe.place_drawing_anchor(600.0, 300.0, DrawingModifiers::default()) as u32;
        rebuild(&mut probe);

        // Right/top template: the prompt sits above the line near its right end, off the body.
        let (x, y, _) = probe.engine.drawing_text_transform(id).unwrap();
        let (label_x, label_y) = (x - 20.0, y);
        assert!(probe.engine.hit_test_drawing(label_x, label_y).is_none());
        probe.update_crosshair(label_x, label_y);
        probe.update_cursor(label_x + probe.engine.pane_left, label_y);
        assert_eq!(probe.engine.hovered_text(), Some(id));
        assert_eq!(probe.cursor_style, CursorStyle::IBeam);
        rebuild(&mut probe);
        assert!(probe.frame.panes[0].main.iter().any(|prim| matches!(
            prim,
            Prim::RotatedText { text, .. } if text == "+ Add text"
        )));
    }

    /// The native template matches the browser toolbar: a new trend line has no label (so it
    /// shows the `+ Add text` prompt) and its label ink follows the line color; only the
    /// standalone Text tool takes template content, and live style patches never touch labels.
    #[test]
    fn drawing_template_matches_the_browser_defaults() {
        let template = DrawingTemplate::default();
        let trend: serde_json::Value =
            serde_json::from_str(&template.json(DrawingKind::TrendLine)).unwrap();
        assert!(trend.get("text").is_none());
        assert!(trend.get("text_color").is_none());
        assert_eq!(trend["color"], "#168ef7");
        assert_eq!(trend["text_size"], 14);

        let mut probe = Probe::new(64, Some(1));
        probe.rebuild_with_measure(
            1024.0,
            640.0,
            1.0,
            |text, _bold| text.chars().count() as f64 * 7.0,
            |text, _bold| text.chars().count() as f64 * 6.0,
        );
        probe.engine.clear_drawings();
        probe.arm_drawing(DrawingKind::TrendLine);
        probe.place_drawing_anchor(200.0, 180.0, DrawingModifiers::default());
        let id = probe.place_drawing_anchor(500.0, 300.0, DrawingModifiers::default());
        assert!(id > 0);
        let drawing = probe.engine.drawing(id as u32).unwrap();
        assert!(drawing.text.is_empty(), "a new trend line starts unlabeled");
        assert!(
            drawing.text_color.is_none(),
            "trend label ink follows the line"
        );

        // Typing a label, then restyling, keeps the label; resetting ink restores inheritance.
        probe.engine.set_selected_drawing(Some(id as u32));
        assert!(probe.engine.begin_drawing_text_edit(id as u32, true));
        assert!(probe.engine.drawing_text_edit_insert("breakout"));
        assert!(probe.engine.commit_drawing_text_edit());
        probe.update_drawing_template(|t| t.text_color = Some("#ab47bc".into()));
        probe.update_drawing_template(|t| t.width = 3);
        probe.update_drawing_template(|t| t.text_color = None);
        let drawing = probe.engine.drawing(id as u32).unwrap();
        assert_eq!(drawing.text, "breakout");
        assert_eq!(drawing.width, 3.0);
        assert!(drawing.text_color.is_none());
    }

    #[test]
    fn interactive_drawing_creation_commits_and_selects() {
        let mut probe = Probe::new(64, Some(1));
        probe.rebuild_with_measure(
            1024.0,
            640.0,
            1.0,
            |text, _bold| text.chars().count() as f64 * 7.0,
            |text, _bold| text.chars().count() as f64 * 6.0,
        );
        probe.engine.clear_drawings();
        probe.arm_drawing(DrawingKind::TrendLine);
        assert!(!probe.engine.drawing_create_active());
        assert_eq!(
            probe.place_drawing_anchor(200.0, 180.0, DrawingModifiers::default()),
            -1
        );
        let id = probe.place_drawing_anchor(500.0, 300.0, DrawingModifiers::default());
        assert!(id > 0);
        assert_eq!(probe.engine.drawings().len(), 1);
        assert_eq!(probe.engine.selected_drawing(), Some(id as u32));
    }

    #[test]
    fn native_path_creation_pops_and_finishes_as_one_drawing() {
        let mut probe = Probe::new(64, Some(1));
        probe.rebuild_with_measure(
            1024.0,
            640.0,
            1.0,
            |text, _bold| text.chars().count() as f64 * 7.0,
            |text, _bold| text.chars().count() as f64 * 6.0,
        );
        probe.engine.clear_drawings();
        probe.arm_drawing(DrawingKind::Path);
        for (x, y) in [(200.0, 180.0), (350.0, 260.0), (500.0, 200.0)] {
            assert_eq!(
                probe.place_drawing_anchor(x, y, DrawingModifiers::default()),
                -1
            );
        }
        assert!(probe.pop_drawing_anchor());
        assert!(probe.finish_drawing_creation());
        assert_eq!(probe.engine.drawings().len(), 1);
        assert_eq!(probe.engine.drawings()[0].kind, DrawingKind::Path);
        assert_eq!(probe.engine.drawings()[0].points.len(), 2);
        assert_eq!(probe.engine.active_drawing_tool(), None);
    }

    #[test]
    fn armed_ctrl_magnet_snaps_the_crosshair_without_a_preview_dot() {
        let mut probe = Probe::new(64, Some(1));
        probe.rebuild_with_measure(
            1024.0,
            640.0,
            1.0,
            |text, _bold| text.chars().count() as f64 * 7.0,
            |text, _bold| text.chars().count() as f64 * 6.0,
        );
        probe.engine.clear_drawings();
        probe.update_crosshair_modifier(true, false);
        assert!(!probe.engine.crosshair_ohlc_magnet);
        probe.arm_drawing(DrawingKind::TrendLine);
        assert!(!probe.engine.drawing_create_active());

        let x = probe.engine.time_scale.logical_to_coordinate(32.0);
        let y = 200.0;
        probe.update_crosshair(x, y);
        let free = probe.engine.build_frame();
        probe.update_crosshair_modifier(true, false);
        let snapped = probe.engine.build_frame();
        let crosshair_color =
            Color::parse_css(&probe.engine.options.get().crosshair.horz_line.color)
                .expect("the package crosshair color is valid");
        let crosshair_y = |frame: &ChartFrame| {
            frame.panes[0].main.iter().find_map(|prim| match prim {
                Prim::HLine { y, color, .. } if *color == crosshair_color => Some(*y),
                _ => None,
            })
        };

        assert_ne!(crosshair_y(&free), crosshair_y(&snapped));
        assert_eq!(
            snapped.panes[0]
                .main
                .iter()
                .filter(|prim| matches!(prim, Prim::Circle { .. }))
                .count(),
            0,
            "arming a tool must not create a pre-click anchor handle"
        );
    }
}

#[cfg(test)]
mod semantic_regressions {
    use super::*;

    #[test]
    fn pending_template_patch_reaches_committed_drawing() {
        let mut probe = Probe::new(64, Some(1));
        probe.rebuild_with_measure(
            1024.0,
            640.0,
            1.0,
            |text, _bold| text.chars().count() as f64 * 7.0,
            |text, _bold| text.chars().count() as f64 * 6.0,
        );
        probe.engine.clear_drawings();
        probe.arm_drawing(DrawingKind::TrendLine);
        assert_eq!(
            probe.place_drawing_anchor(200.0, 180.0, DrawingModifiers::default()),
            -1
        );
        probe.update_drawing_template(|template| {
            template.color = "#ff9800".into();
            template.width = 4;
            template.text_italic = true;
        });
        let id = probe.place_drawing_anchor(500.0, 300.0, DrawingModifiers::default());
        assert!(id > 0);
        let options = probe.engine.drawing_options_json(id as u32).unwrap();
        assert!(options.contains("\"color\":\"#ff9800\""));
        assert!(options.contains(r#""width":4.0"#));
        assert!(options.contains(r#""text_italic":true"#));
    }

    #[test]
    fn selected_template_patch_preserves_unrelated_options() {
        let mut probe = Probe::new(64, Some(1));
        let id = probe
            .engine
            .add_drawing(
                DrawingKind::TrendLine,
                0,
                vec![
                    DrawingPoint {
                        logical: 4.0,
                        price: 100.0,
                    },
                    DrawingPoint {
                        logical: 8.0,
                        price: 105.0,
                    },
                ],
                Some(r##"{"color":"#123456","width":7,"text":"keep me"}"##),
            )
            .unwrap();
        probe.engine.set_selected_drawing(Some(id));
        probe.update_drawing_template(|template| template.text_italic = true);
        let options = probe.engine.drawing_options_json(id).unwrap();
        assert!(options.contains("\"color\":\"#123456\""));
        assert!(options.contains(r#""width":7.0"#));
        assert!(options.contains(r#""text":"keep me""#));
        assert!(options.contains(r#""text_italic":true"#));
    }

    #[test]
    fn theme_changes_preserve_pinned_styles_and_follow_unpinned_styles() {
        let mut probe = Probe::new(32, Some(1));
        probe.toggle_axis_border_pin(DemoTheme::Light);
        probe.toggle_text_color_pin(DemoTheme::Light);
        probe.apply_theme(DemoTheme::Dark);
        assert_eq!(
            probe.engine.options.get().time_scale.border_color,
            "#2962ff"
        );
        assert_eq!(probe.engine.options.get().layout.text_color, "#ab47bc");

        probe.toggle_axis_border_pin(DemoTheme::Dark);
        probe.toggle_text_color_pin(DemoTheme::Dark);
        assert_eq!(
            probe.engine.options.get().time_scale.border_color,
            aeris_charts_core::style::DARK_BORDER_CSS
        );
        assert_eq!(
            probe.engine.options.get().layout.text_color,
            aeris_charts_core::style::DARK_AXIS_TEXT_CSS
        );
        probe.apply_theme(DemoTheme::Light);
        assert_eq!(
            probe.engine.options.get().time_scale.border_color,
            aeris_charts_core::style::LIGHT_BORDER_CSS
        );
        assert_eq!(
            probe.engine.options.get().layout.text_color,
            aeris_charts_core::style::LIGHT_AXIS_TEXT_CSS
        );
    }

    #[test]
    fn idle_interactive_probe_stops_requesting_frames() {
        let mut interactive = Probe::new(8, None);
        assert!(!interactive.needs_animation_frame());
        interactive.kinetic_active = true;
        assert!(interactive.needs_animation_frame());
        interactive.kinetic_active = false;
        interactive.begin_keyboard_pan_at(-1, 10.0, false, 1_000.0);
        assert!(interactive.needs_animation_frame());
        interactive.cancel_keyboard_pan();
        interactive.engine.start_scroll_animation(3.0, 160.0, 0.0);
        assert!(interactive.needs_animation_frame());
        assert!(Probe::new(8, Some(2)).needs_animation_frame());
    }

    #[test]
    fn brushable_area_uses_the_native_area_and_delta_tooltip_paths() {
        let mut probe = Probe::new(32, None);
        probe.enable_brushable_area();
        let state = probe.brushable_area.expect("brush interaction is attached");
        assert_eq!(probe.engine.series[0].kind, SeriesKind::Area);
        assert!(probe.engine.has_delta_tooltip());

        probe.engine.time_scale.set_width(probe.engine.pane_w);
        probe.engine.set_visible_logical_range(0.0, 31.0);
        let from = probe.engine.time_scale.index_to_coordinate(4);
        let to = probe.engine.time_scale.index_to_coordinate(12);
        assert!(probe
            .engine
            .set_delta_tooltip_points(state.tooltip_id, &[from, to]));
        let active_range = probe.engine.delta_tooltip_active_range(state.tooltip_id);
        assert!(
            active_range.is_some(),
            "expected active range for coordinates {from}..{to} in pane {}",
            probe.engine.pane_w
        );
        probe.sync_brushable_area();
        assert!(probe
            .brushable_area
            .is_some_and(|brush| brush.styled_range.is_some()));

        assert!(probe.clear_brushable_area_selection());
        assert_eq!(
            probe.engine.delta_tooltip_active_range(state.tooltip_id),
            None
        );
        assert!(probe
            .brushable_area
            .is_some_and(|brush| brush.styled_range.is_none()));

        probe.set_series_kind(SeriesKind::Line);
        assert_eq!(probe.engine.series[0].kind, SeriesKind::Line);
        assert!(probe.brushable_area.is_none());
        assert!(!probe.engine.has_delta_tooltip());
    }

    #[test]
    fn footprint_demo_uses_an_explicit_tick_tape_and_restores_the_base_series() {
        let mut probe = Probe::new_interactive(32);
        probe.engine.time_scale.set_width(probe.engine.pane_w);
        let previous_bar_spacing = probe.engine.bar_spacing();
        let previous_right_offset = probe.engine.right_offset();
        probe.enable_footprint();

        let state = probe.footprint.expect("footprint demo is active");
        let series = probe
            .engine
            .series
            .iter()
            .find(|series| series.id == state.series_id && !series.removed)
            .expect("footprint series is live");
        assert_eq!(series.kind, SeriesKind::Footprint);
        assert_eq!(series.title, "ORDER FLOW");
        assert!(series.price_line_visible);
        assert!(series.last_value_visible);
        assert!(series.countdown_visible);
        assert!(!probe.engine.series[0].visible);
        assert_eq!(
            probe.engine.footprint_bars(state.series_id).unwrap().len(),
            12
        );
        assert_eq!(probe.engine.bar_spacing(), 72.0);
        assert_eq!(probe.engine.right_offset(), 0.0);

        probe.set_series_kind(SeriesKind::Candlestick);
        assert!(probe.footprint.is_none());
        assert!(probe.engine.series[0].visible);
        assert_eq!(probe.engine.bar_spacing(), previous_bar_spacing);
        assert_eq!(probe.engine.right_offset(), previous_right_offset);
        assert!(probe
            .engine
            .series
            .iter()
            .all(|series| series.id != state.series_id || series.removed));
    }

    #[test]
    fn nested_workspace_split_extents_use_the_owning_browser_grid_space() {
        let horizontal = WorkspaceLayout::Split {
            direction: SplitDirection::Horizontal,
            ratio: 0.5,
            a: Box::new(WorkspaceLayout::Cell { id: 1 }),
            b: Box::new(WorkspaceLayout::Split {
                direction: SplitDirection::Horizontal,
                ratio: 0.5,
                a: Box::new(WorkspaceLayout::Cell { id: 2 }),
                b: Box::new(WorkspaceLayout::Cell { id: 3 }),
            }),
        };
        let width = |id, _| match id {
            1 => 100.0,
            2 => 200.0,
            _ => 300.0,
        };
        assert_eq!(
            owning_split_total_extent(&horizontal, 1, 2, SplitDirection::Horizontal, &width),
            Some(602.0)
        );
        assert_eq!(
            owning_split_total_extent(&horizontal, 2, 3, SplitDirection::Horizontal, &width),
            Some(501.0)
        );

        let vertical = WorkspaceLayout::Split {
            direction: SplitDirection::Vertical,
            ratio: 0.5,
            a: Box::new(WorkspaceLayout::Cell { id: 1 }),
            b: Box::new(WorkspaceLayout::Split {
                direction: SplitDirection::Vertical,
                ratio: 0.5,
                a: Box::new(WorkspaceLayout::Cell { id: 2 }),
                b: Box::new(WorkspaceLayout::Cell { id: 3 }),
            }),
        };
        let height = |id, _| match id {
            1 => 80.0,
            2 => 120.0,
            _ => 160.0,
        };
        assert_eq!(
            owning_split_total_extent(&vertical, 1, 2, SplitDirection::Vertical, &height),
            Some(362.0)
        );
        assert_eq!(
            owning_split_total_extent(&vertical, 2, 3, SplitDirection::Vertical, &height),
            Some(281.0)
        );
    }

    #[test]
    fn split_flex_ratios_preserve_the_model_ratio_after_the_fixed_divider() {
        let (first, second) = split_flex_ratios(0.35);
        assert!((first - 0.35).abs() < f32::EPSILON);
        assert!((second - 0.65).abs() < f32::EPSILON);
        let content_extent = 1_000.0 - WORKSPACE_DIVIDER_LAYOUT_PX;
        assert!(
            (content_extent * first / (content_extent * (first + second)) - 0.35).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn workspace_snapshot_helpers_follow_authoritative_layout() {
        let mut workspace = Workspace::new();
        let second = workspace.split(1, SplitDirection::Horizontal).unwrap();
        let third = workspace.split(second, SplitDirection::Vertical).unwrap();
        let layout = workspace.layout();
        assert_eq!(layout_first(&layout), 1);
        assert_eq!(layout_last(&layout), third);
        workspace.resize_between(second, third, 0.2).unwrap();
        workspace.remove(second).unwrap();
        let layout = workspace.layout();
        assert_eq!(layout_first(&layout), 1);
        assert_eq!(layout_last(&layout), third);
        assert_eq!(workspace.cell_ids(), [1, third]);
    }
}
