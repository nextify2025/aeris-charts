//! The K-line chart: an Aeris [`ChartEngine`] hosted in a GPUI view.
//!
//! The engine owns chart state and interaction math; this view feeds it bars, binds KLineChart
//! indicators, arms drawing tools, forwards pointer and keyboard input, rebuilds the frame when
//! something changed, and paints it through the Aeris GPUI executor. The host contract follows
//! `aeris_charts_render_gpui`'s `gpui_probe` example, trimmed to what a stock page needs.
//!
//! Per-pane legends in KLineChart's style ("MA5: 123.45 …") are ordinary GPUI text laid over the
//! canvas at each pane's top edge.

use std::collections::HashMap;
use std::time::Instant;

use aeris_charts_core::model::data_layer::SeriesId;
use aeris_charts_core::model::plot_list::MismatchDirection;
use aeris_charts_engine::klinechart::{Figure, Indicator, Placement};
use aeris_charts_engine::{
    wheel_zoom_scale, ChartEngine, ChartFrame, ChartTheme, DrawingKind, DrawingModifiers,
    DrawingPoint, GestureResolver, GestureUpdateKind, InputDevice, InputModifiers, InputTarget,
    PointerSample, PriceScaleTarget, SeriesKind, WheelBehavior, WheelDeltaMode, WheelIntent,
    WheelSample, KLINECHART_LINE_COLORS, WHEEL_SCROLL_PX_PER_DELTA,
};
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{LineStyle, Prim};
use aeris_charts_render_gpui::backend::{measure_text, to_hsla};
use aeris_charts_render_gpui::{
    AerisViewport, GpuiChartRenderer, GpuiFrameMetrics, PreparedAerisFrame,
};
use gpui_kit::*;

use crate::market::{civil_from_days, Candle, Period, DAY};

/// KLineChart's default bar spacing, used when a period opens.
const DEFAULT_BAR_SPACING: f64 = 8.0;
/// Minutes in a regular session, the width of the intraday view.
const SESSION_MINUTES: f64 = 390.0;
const CLICK_SLOP: f64 = 5.0;
const SEPARATOR_HIT: f64 = 4.0;
const WHEEL_LINE_HEIGHT: f32 = 32.0;
/// KLineChart's overlay color, a little heavier than its 1 px default so it reads on both themes.
const DRAWING_TEMPLATE: &str = r##"{"color":"#1677FF","width":1.5}"##;
/// The intraday price line and its fill.
const INTRADAY_LINE: &str = "#1677FF";
const INTRADAY_FILL_TOP: &str = "rgba(22,119,255,0.22)";
const INTRADAY_FILL_BOTTOM: &str = "rgba(22,119,255,0.01)";
/// The intraday average-price line, in the traditional yellow.
const AVERAGE_PRICE_COLOR: &str = "#F5A623";

/// KLineChart's rising and falling colors.
const GREEN: &str = "#2DC08E";
const RED: &str = "#F92855";

/// An indicator bound to the chart, with its output series in output order. `outputs` is empty
/// while a price-pane indicator is parked (the intraday view hides them).
#[derive(Clone, Debug)]
pub struct Study {
    pub indicator: Indicator,
    pub outputs: Vec<SeriesId>,
}

/// Chart chrome colors, as CSS, taken from the application theme so the chart matches the page.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartColors {
    pub background: String,
    pub text: String,
    pub grid: String,
    pub border: String,
    pub crosshair: String,
    pub dark: bool,
}

#[derive(Clone, Copy, Debug)]
enum Drag {
    Pan {
        price_pan: Option<(usize, PriceScaleTarget)>,
    },
    TimeAxis,
    PriceAxis {
        pane: usize,
        target: PriceScaleTarget,
    },
    Separator {
        index: usize,
        last_y: f64,
    },
    Drawing,
    DrawingCreation,
}

/// One text run of a legend line.
struct LegendRun {
    text: String,
    color: Hsla,
}

pub struct ChartView {
    engine: ChartEngine,
    renderer: GpuiChartRenderer,
    frame: ChartFrame,
    axis: Vec<Prim>,
    focus: FocusHandle,
    /// Size and DPR the frame was last built for.
    built_for: (f32, f32, f32),
    dirty: bool,
    plan_dirty: bool,
    metrics: Option<GpuiFrameMetrics>,
    /// Apply the period's default view on the next layout (the time scale needs its width first).
    reset_view: bool,
    /// A scripted crosshair (bar index, price), placed once layout knows the coordinates.
    pending_crosshair: Option<(usize, f64)>,
    viewport_offset: (f32, f32),
    input: GestureResolver,
    input_target: InputTarget,
    cursor: CursorStyle,
    press_start: Option<(f64, f64)>,
    press_moved: bool,
    drag: Option<Drag>,
    drag_started: bool,
    pending_creation_point: Option<(f64, f64, DrawingModifiers)>,
    creation_press_committed: bool,
    started: Instant,

    period: Period,
    candles: Vec<Candle>,
    volume: SeriesId,
    turnover: SeriesId,
    /// Price-pane indicators (MA, BOLL, …).
    main: Vec<Study>,
    /// Indicators in panes of their own, top to bottom.
    subs: Vec<Study>,
    /// The intraday average-price line.
    average_price: Option<Study>,
    /// The intraday previous-close line.
    prev_close_line: Option<u32>,
    prev_close: f64,
    /// Bar under the pointer, for the legends.
    hover: Option<usize>,
    /// Pane tops the legends were last placed at; a layout change re-renders them.
    legend_tops: Vec<f64>,
    colors: Option<ChartColors>,
    red_up: bool,
    /// Snap drawing anchors to bar prices without holding Ctrl/Cmd.
    magnet: bool,
}

impl ChartView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut engine = ChartEngine::new(1000.0, 600.0, 1.0);
        let volume = engine.add_series(SeriesKind::Histogram);
        engine.set_series_visible(volume, false);
        let turnover = engine.add_series(SeriesKind::Line);
        engine.set_series_visible(turnover, false);
        if let Some(candles) = engine.series.iter_mut().find(|s| s.id == 0) {
            // A candle-close countdown suits futures and crypto, not a stock page.
            candles.countdown_visible = false;
        }
        // The crosshair's price-tag action chip requests alerts, which this page does not offer.
        engine.set_alert_create_button_visible(false);
        Self {
            engine,
            renderer: GpuiChartRenderer::new(),
            frame: ChartFrame::default(),
            axis: Vec::new(),
            focus: cx.focus_handle(),
            built_for: (0.0, 0.0, 0.0),
            dirty: true,
            plan_dirty: true,
            metrics: None,
            reset_view: true,
            pending_crosshair: None,
            viewport_offset: (0.0, 0.0),
            input: GestureResolver::default(),
            input_target: InputTarget::Pane,
            cursor: CursorStyle::Crosshair,
            press_start: None,
            press_moved: false,
            drag: None,
            drag_started: false,
            pending_creation_point: None,
            creation_press_committed: false,
            started: Instant::now(),
            period: Period::Day,
            candles: Vec::new(),
            volume,
            turnover,
            main: Vec::new(),
            subs: Vec::new(),
            average_price: None,
            prev_close_line: None,
            prev_close: 0.0,
            hover: None,
            legend_tops: Vec::new(),
            colors: None,
            red_up: false,
            magnet: false,
        }
    }

    // ---- data -------------------------------------------------------------------------------

    /// Replace every bar, for a newly selected period. `prev_close` anchors the intraday view.
    pub fn set_candles(&mut self, period: Period, candles: Vec<Candle>, prev_close: f64) {
        let was_intraday = self.period == Period::Intraday;
        let intraday = period == Period::Intraday;
        self.period = period;
        self.candles = candles;
        self.prev_close = prev_close;
        let column = |f: fn(&Candle) -> f64| self.candles.iter().map(f).collect::<Vec<_>>();
        let times = column(|bar| bar.time as f64);
        let (open, high, low, close) = (
            column(|bar| bar.open),
            column(|bar| bar.high),
            column(|bar| bar.low),
            column(|bar| bar.close),
        );
        let (volume, turnover) = (column(|bar| bar.volume), column(|bar| bar.turnover));
        if intraday != was_intraday {
            self.engine.convert_series_kind(
                0,
                if intraday {
                    SeriesKind::Area
                } else {
                    SeriesKind::Candlestick
                },
            );
            self.style_price_series();
        }
        // Invalid rows cannot occur in generated data; a real feed would surface the report.
        let _ = self
            .engine
            .set_series_data(0, &times, &open, &high, &low, &close);
        let _ =
            self.engine
                .set_series_data(self.volume, &times, &volume, &volume, &volume, &volume);
        let _ = self.engine.set_series_data(
            self.turnover,
            &times,
            &turnover,
            &turnover,
            &turnover,
            &turnover,
        );
        self.sync_intraday_studies();
        self.install_time_formatters();
        self.hover = None;
        self.reset_view = true;
        self.dirty = true;
    }

    /// Apply the newest bar: replaces the live bar or appends the next one.
    pub fn update_latest(&mut self, bar: Candle) {
        match self.candles.last_mut() {
            Some(last) if last.time == bar.time => *last = bar,
            Some(last) if last.time > bar.time => return,
            _ => self.candles.push(bar),
        }
        let time = bar.time as f64;
        self.engine
            .update_series_bar(0, time, [bar.open, bar.high, bar.low, bar.close]);
        self.engine
            .update_series_bar(self.volume, time, [bar.volume; 4]);
        self.engine
            .update_series_bar(self.turnover, time, [bar.turnover; 4]);
        self.dirty = true;
    }

    pub fn period(&self) -> Period {
        self.period
    }

    pub fn bar_count(&self) -> usize {
        self.candles.len()
    }

    pub fn candle(&self, index: usize) -> Option<Candle> {
        self.candles.get(index).copied()
    }

    // ---- indicators -------------------------------------------------------------------------

    fn bind(&mut self, indicator: Indicator) -> Option<Study> {
        // AVP reads turnover as its source; the rest read the candles.
        let source = if matches!(indicator, Indicator::Avp) {
            self.turnover
        } else {
            0
        };
        let volume = indicator.needs_volume().then_some(self.volume);
        let outputs = self
            .engine
            .add_klinechart_indicator(source, indicator.clone(), volume);
        (!outputs.is_empty()).then_some(Study { indicator, outputs })
    }

    fn unbind(&mut self, study: &Study) {
        // Removing any output drops the whole binding and its pane.
        if let Some(&first) = study.outputs.first() {
            self.engine.remove_series(first);
        }
    }

    pub fn has_main(&self, name: &str) -> bool {
        self.main.iter().any(|study| study.indicator.name() == name)
    }

    pub fn has_sub(&self, name: &str) -> bool {
        self.subs.iter().any(|study| study.indicator.name() == name)
    }

    /// Add or remove a price-pane indicator by KLineChart name.
    pub fn toggle_main(&mut self, name: &str) {
        if let Some(index) = self.main.iter().position(|s| s.indicator.name() == name) {
            let study = self.main.remove(index);
            self.unbind(&study);
        } else if let Some(indicator) = Indicator::from_name(name) {
            if indicator.placement() != Placement::Price {
                return;
            }
            if self.period == Period::Intraday {
                // Parked until a candle period shows again; the choice is kept.
                self.main.push(Study {
                    indicator,
                    outputs: Vec::new(),
                });
            } else if let Some(study) = self.bind(indicator) {
                self.main.push(study);
            }
        }
        self.dirty = true;
    }

    /// Add or remove an indicator pane by KLineChart name.
    pub fn toggle_sub(&mut self, name: &str) {
        if let Some(index) = self.subs.iter().position(|s| s.indicator.name() == name) {
            let study = self.subs.remove(index);
            self.unbind(&study);
        } else if let Some(indicator) = Indicator::from_name(name) {
            if let Some(study) = self.bind(indicator) {
                self.subs.push(study);
            }
        }
        self.balance_panes();
        self.dirty = true;
    }

    /// The candle pane keeps about three fifths of the height however many panes are open, as
    /// KLineChart's fixed-height indicator panes do.
    fn balance_panes(&mut self) {
        let count = self.engine.panes.len();
        if count < 2 {
            return;
        }
        let subs = (count - 1) as f64;
        let sub_share = match count - 1 {
            1 => 0.26,
            2 => 0.2,
            _ => 0.44 / subs,
        };
        for (index, pane) in self.engine.panes.iter_mut().enumerate() {
            pane.stretch_factor = if index == 0 {
                1.0 - sub_share * subs
            } else {
                sub_share
            };
        }
    }

    /// The intraday view swaps price studies for the average-price and previous-close lines, as
    /// trading apps do.
    fn sync_intraday_studies(&mut self) {
        if let Some(line) = self.prev_close_line.take() {
            self.engine.remove_price_line(line);
        }
        if self.period == Period::Intraday {
            let main = std::mem::take(&mut self.main);
            for study in &main {
                self.unbind(study);
            }
            self.main = main
                .into_iter()
                .map(|study| Study {
                    indicator: study.indicator,
                    outputs: Vec::new(),
                })
                .collect();
            if self.average_price.is_none() {
                self.average_price = self.bind(Indicator::Avp);
                if let Some(study) = self.average_price.clone() {
                    for &output in &study.outputs {
                        self.style_series(output, AVERAGE_PRICE_COLOR, 1.0);
                    }
                }
            }
            let color = self
                .colors
                .as_ref()
                .and_then(|colors| Color::parse_css(&colors.text))
                .unwrap_or(Color::rgb(0x76, 0x80, 0x8f));
            self.prev_close_line = Some(self.engine.create_price_line(
                0,
                self.prev_close,
                color,
                1,
                LineStyle::Dashed,
                "昨收",
            ));
        } else {
            if let Some(study) = self.average_price.take() {
                self.unbind(&study);
            }
            let main = std::mem::take(&mut self.main);
            for study in main {
                if study.outputs.is_empty() {
                    if let Some(bound) = self.bind(study.indicator) {
                        self.main.push(bound);
                    }
                } else {
                    self.main.push(study);
                }
            }
        }
    }

    fn style_series(&mut self, id: SeriesId, color: &str, width: f64) {
        if let Some(series) = self
            .engine
            .series
            .iter_mut()
            .find(|s| s.id == id && !s.removed)
        {
            series.line_color = Some(color.to_owned());
            series.line_width = Some(width);
        }
        self.dirty = true;
    }

    /// Candles follow the market colors; the intraday area is a blue line over a soft fill.
    fn style_price_series(&mut self) {
        let intraday = self.period == Period::Intraday;
        if let Some(series) = self.engine.series.iter_mut().find(|s| s.id == 0) {
            if intraday {
                series.line_color = Some(INTRADAY_LINE.to_owned());
                series.line_width = Some(1.5);
                series.area_top_color = Some(INTRADAY_FILL_TOP.to_owned());
                series.area_bottom_color = Some(INTRADAY_FILL_BOTTOM.to_owned());
            } else {
                series.line_color = None;
                series.line_width = None;
                series.area_top_color = None;
                series.area_bottom_color = None;
            }
        }
    }

    /// Chinese-style axis and crosshair times for the current period.
    fn install_time_formatters(&mut self) {
        self.engine
            .set_tick_mark_formatter(Some(Box::new(|time, kind| {
                let (year, month, day) = civil_from_days(time.div_euclid(DAY));
                let seconds = time.rem_euclid(DAY);
                Some(match kind {
                    0 => format!("{year}"),
                    1 => format!("{year}-{month:02}"),
                    2 => format!("{month:02}-{day:02}"),
                    _ => format!("{:02}:{:02}", seconds / 3_600, seconds % 3_600 / 60),
                })
            })));
        let period = self.period;
        self.engine
            .set_time_formatter(Some(Box::new(move |time| Some(format_time(time, period)))));
    }

    // ---- drawings and style -----------------------------------------------------------------

    pub fn active_tool(&self) -> Option<DrawingKind> {
        self.engine.active_drawing_tool()
    }

    /// Arm a drawing tool (or disarm with `None`). Tools disarm after one drawing, like KLineChart.
    pub fn set_tool(&mut self, kind: Option<DrawingKind>) {
        self.pending_creation_point = None;
        self.creation_press_committed = false;
        self.engine
            .set_drawing_tool(kind, kind.map(|_| DRAWING_TEMPLATE), None);
        self.dirty = true;
    }

    pub fn clear_drawings(&mut self) {
        self.engine.cancel_drawing_tool();
        self.engine.clear_drawings();
        self.dirty = true;
    }

    pub fn drawing_count(&self) -> usize {
        self.engine.drawings().len()
    }

    pub fn magnet(&self) -> bool {
        self.magnet
    }

    pub fn set_magnet(&mut self, magnet: bool) {
        self.magnet = magnet;
    }

    /// Place a drawing from (bar index, price) anchors, for scripted scenes.
    pub fn add_drawing(&mut self, kind: DrawingKind, points: &[(f64, f64)], text: Option<&str>) {
        let points = points
            .iter()
            .map(|&(logical, price)| DrawingPoint { logical, price })
            .collect();
        let options = match text {
            Some(text) => format!(
                r##"{{"color":"#1677FF","width":1.5,"text":{}}}"##,
                json_string(text)
            ),
            None => DRAWING_TEMPLATE.to_owned(),
        };
        self.engine.add_drawing(kind, 0, points, Some(&options));
        self.dirty = true;
    }

    /// Put the crosshair on bar `index` at `price`, for scripted scenes.
    pub fn place_crosshair(&mut self, index: usize, price: f64) {
        self.pending_crosshair = Some((index, price));
        self.dirty = true;
    }

    pub fn set_colors(&mut self, colors: ChartColors) {
        if self.colors.as_ref() == Some(&colors) {
            return;
        }
        self.colors = Some(colors);
        self.apply_style();
    }

    pub fn red_up(&self) -> bool {
        self.red_up
    }

    /// Red for rising prices (the mainland China and Hong Kong convention) or green.
    pub fn set_red_up(&mut self, red_up: bool) {
        self.red_up = red_up;
        self.apply_style();
    }

    fn apply_style(&mut self) {
        let Some(colors) = self.colors.clone() else {
            return;
        };
        self.engine.set_theme(if colors.dark {
            ChartTheme::Dark
        } else {
            ChartTheme::Light
        });
        let (up, down) = market_colors(self.red_up);
        let ChartColors {
            background,
            text,
            grid,
            border,
            crosshair,
            ..
        } = &colors;
        let patch = format!(
            r##"{{
                "layout": {{
                    "background": {{"type": "solid", "color": "{background}"}},
                    "textColor": "{text}",
                    "bullishColor": "{up}",
                    "bearishColor": "{down}",
                    "fontSize": 11,
                    "panes": {{"separatorColor": "{border}"}}
                }},
                "grid": {{"vertLines": {{"color": "{grid}"}}, "horzLines": {{"color": "{grid}"}}}},
                "crosshair": {{
                    "vertLine": {{"color": "{crosshair}", "labelBackgroundColor": "{crosshair}"}},
                    "horzLine": {{"color": "{crosshair}", "labelBackgroundColor": "{crosshair}"}}
                }},
                "rightPriceScale": {{"borderColor": "{border}"}},
                "timeScale": {{"borderColor": "{border}"}}
            }}"##
        );
        self.engine
            .options
            .apply_str(&patch)
            .expect("the chart style patch is valid JSON");
        self.engine.refresh_klinechart_colors();
        if let Some(line) = self.prev_close_line {
            let _ = self
                .engine
                .price_line_apply_options(line, &format!(r#"{{"color":"{text}"}}"#));
        }
        self.renderer.invalidate_caches();
        self.dirty = true;
    }

    // ---- frame ------------------------------------------------------------------------------

    fn rebuild(&mut self, width: f32, height: f32, scale_factor: f32, window: &Window) {
        if let Some((x, y, modifiers)) = self.pending_creation_point.take() {
            if self
                .engine
                .drawing_tool_pointer_move(x, y, modifiers, true)
                .changed
            {
                self.dirty = true;
            }
        }
        let key = (width, height, scale_factor);
        if self.built_for == key && !self.dirty && !self.frame.panes.is_empty() {
            return;
        }
        let resized = self.built_for != key;
        if self.built_for.2 != 0.0 && self.built_for.2 != scale_factor {
            self.renderer.invalidate_caches();
        }
        self.built_for = key;
        self.dirty = false;
        self.engine.css_width = f64::from(width);
        self.engine.css_height = f64::from(height);
        self.engine.dpr = f64::from(scale_factor);

        // Drawing labels measure with the same shaper that paints them.
        let layout = self.engine.options.get().layout.clone();
        let mut widths = HashMap::new();
        for drawing in self.engine.drawings() {
            let text = drawing.display_text();
            if text.is_empty() {
                continue;
            }
            let size = drawing.resolved_text_size(layout.font_size);
            let weight = drawing.text_weight.unwrap_or(400);
            let key = format!(
                "{text}\u{0}{size}\u{0}{}\u{0}{weight}\u{0}{}",
                layout.font_family, drawing.text_italic
            );
            let width = measure_text(
                window,
                text,
                &layout.font_family,
                size as f32,
                weight,
                drawing.text_italic,
            )
            .width;
            widths.insert(key, f64::from(width));
        }
        self.engine
            .set_text_measure(Some(Box::new(move |text, size, family, weight, italic| {
                let key = format!("{text}\u{0}{size}\u{0}{family}\u{0}{weight}\u{0}{italic}");
                widths
                    .get(&key)
                    .copied()
                    .unwrap_or_else(|| text.chars().count() as f64 * size * 0.6)
            })));

        let axis_size = self.engine.axis_font_size() as f32;
        let countdown_size = self.engine.countdown_font_size() as f32;
        let family = layout.font_family.as_str();
        let axis_measure = |text: &str, bold: bool| {
            f64::from(
                measure_text(
                    window,
                    text,
                    family,
                    axis_size,
                    if bold { 700 } else { 400 },
                    false,
                )
                .width,
            )
        };
        let countdown_measure = |text: &str, bold: bool| {
            f64::from(
                measure_text(
                    window,
                    text,
                    family,
                    countdown_size,
                    if bold { 700 } else { 400 },
                    false,
                )
                .width,
            )
        };
        self.engine.recompute_layout_with_measure(
            resized || self.reset_view,
            axis_measure,
            countdown_measure,
        );
        if self.reset_view {
            self.reset_view = false;
            self.apply_default_view();
            self.engine
                .recompute_layout_with_measure(true, axis_measure, countdown_measure);
        }
        if let Some((index, price)) = self.pending_crosshair.take() {
            let x = self.engine.time_scale.index_to_coordinate(index as i64);
            let base = self.candles.first().map_or(price, |bar| bar.close);
            let y = self.engine.panes[0]
                .price_scale
                .price_to_coordinate(price, base);
            self.engine.crosshair = Some((x, y));
            self.hover = (!self.candles.is_empty()).then(|| index.min(self.candles.len() - 1));
        }
        let max_label_width = (self.engine.axis_font_size() + 4.0) * 5.0 / 8.0
            * f64::from(self.engine.tick_mark_max_character_length.max(1));
        let axis_frame =
            self.engine
                .build_axis_frame(max_label_width, axis_measure, countdown_measure);
        self.engine.build_frame_into(&mut self.frame);
        // GPUI converts a vertical text center into a baseline itself; no ink-box correction.
        self.engine
            .build_axis_primitives_into(&axis_frame, &mut self.axis, |_| 0.0);
        self.plan_dirty = true;
    }

    /// The whole session for the intraday line; otherwise the latest bars at KLineChart's
    /// spacing, or every bar when a short history would leave the left of the pane empty.
    fn apply_default_view(&mut self) {
        let bars = self.candles.len() as f64;
        if self.period == Period::Intraday {
            self.engine
                .set_visible_logical_range(-0.5, SESSION_MINUTES - 0.5);
        } else {
            let visible = (self.engine.pane_w / DEFAULT_BAR_SPACING).floor().max(10.0);
            let from = if bars < visible { -1.0 } else { bars - visible };
            self.engine.set_visible_logical_range(from, bars + 2.0);
        }
    }

    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let viewport = AerisViewport::from_bounds(
            bounds.origin.x.into(),
            bounds.origin.y.into(),
            bounds.size.width.into(),
            bounds.size.height.into(),
        );
        let scale_factor = window.scale_factor();
        let replan = self.plan_dirty || self.metrics.is_none();
        let cached = self.metrics.unwrap_or_default();
        let Self {
            engine,
            renderer,
            frame,
            axis,
            ..
        } = self;
        let prepared = PreparedAerisFrame::from_engine(frame, engine).with_axis(axis, &[]);
        let result = if replan {
            renderer.paint_frame(&prepared, viewport, scale_factor, window, cx)
        } else {
            renderer.paint_planned_frame(&prepared, viewport, scale_factor, window, cx, cached)
        };
        match result {
            Ok(metrics) => {
                self.plan_dirty = false;
                self.metrics = Some(metrics);
            }
            Err(error) => eprintln!("stock-detail: chart frame skipped: {error}"),
        }
    }

    // ---- legends ----------------------------------------------------------------------------

    fn pane_tops(&self) -> impl Iterator<Item = f64> + '_ {
        self.engine.panes.iter().map(|pane| pane.top)
    }

    fn legend_index(&self) -> Option<usize> {
        let last = self.candles.len().checked_sub(1)?;
        Some(self.hover.unwrap_or(last).min(last))
    }

    fn format_value(&self, output: SeriesId, index: usize) -> String {
        self.engine
            .series_data_by_index(output, index as i64, MismatchDirection::None)
            .map(|point| point.close)
            .filter(|value| value.is_finite())
            .and_then(|value| self.engine.series_format_price(output, value))
            .unwrap_or_else(|| "--".into())
    }

    fn study_legend(&self, study: &Study, index: usize, muted: Hsla, text: Hsla) -> Vec<LegendRun> {
        let mut runs = vec![LegendRun {
            text: study.indicator.title(),
            color: muted,
        }];
        let mut line = 0;
        for (output, (title, figure)) in study.outputs.iter().zip(
            study
                .indicator
                .output_titles()
                .into_iter()
                .zip(study.indicator.figures()),
        ) {
            let color = if figure == Figure::Line {
                let css = KLINECHART_LINE_COLORS[line % KLINECHART_LINE_COLORS.len()];
                line += 1;
                css_hsla(css)
            } else {
                text
            };
            runs.push(LegendRun {
                text: format!("{title}: {}", self.format_value(*output, index)),
                color,
            });
        }
        runs
    }

    fn average_price_legend(&self, study: &Study, index: usize) -> Vec<LegendRun> {
        let value = study
            .outputs
            .first()
            .map_or_else(|| "--".into(), |&output| self.format_value(output, index));
        vec![LegendRun {
            text: format!("均价: {value}"),
            color: css_hsla(AVERAGE_PRICE_COLOR),
        }]
    }

    fn candle_legend(&self, index: usize, muted: Hsla, text: Hsla) -> Vec<LegendRun> {
        let bar = self.candles[index];
        let reference = if self.period == Period::Intraday {
            self.prev_close
        } else {
            index
                .checked_sub(1)
                .map_or(bar.open, |previous| self.candles[previous].close)
        };
        let change = if reference == 0.0 {
            0.0
        } else {
            (bar.close - reference) / reference * 100.0
        };
        let (up, down) = market_colors(self.red_up);
        let direction = css_hsla(if bar.close >= reference { up } else { down });
        let run = |label: &str, value: String, color: Hsla| LegendRun {
            text: format!("{label} {value}"),
            color,
        };
        let mut runs = vec![LegendRun {
            text: format_time(bar.time, self.period),
            color: muted,
        }];
        if self.period == Period::Intraday {
            runs.push(run("价", format!("{:.2}", bar.close), direction));
        } else {
            runs.push(run("开", format!("{:.2}", bar.open), text));
            runs.push(run("高", format!("{:.2}", bar.high), text));
            runs.push(run("低", format!("{:.2}", bar.low), text));
            runs.push(run("收", format!("{:.2}", bar.close), direction));
        }
        runs.push(run("涨跌幅", format!("{change:+.2}%"), direction));
        runs.push(run("成交量", format_volume(bar.volume), text));
        runs.push(run("成交额", format_volume(bar.turnover), text));
        runs
    }

    fn render_legends(&self, cx: &App) -> Vec<AnyElement> {
        let Some(index) = self.legend_index() else {
            return Vec::new();
        };
        let theme = gpui_kit::component::ActiveTheme::theme(cx);
        let (muted, text) = (theme.muted_foreground, theme.foreground);
        let line = |runs: Vec<LegendRun>| {
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_x_3()
                .children(
                    runs.into_iter()
                        .map(|run| div().text_color(run.color).child(run.text)),
                )
                .into_any_element()
        };
        let mut panes: Vec<(usize, Vec<AnyElement>)> =
            vec![(0, vec![line(self.candle_legend(index, muted, text))])];
        let mut push = |pane: usize, element: AnyElement| {
            if let Some((_, lines)) = panes.iter_mut().find(|(p, _)| *p == pane) {
                lines.push(element);
            } else {
                panes.push((pane, vec![element]));
            }
        };
        if let Some(study) = &self.average_price {
            push(0, line(self.average_price_legend(study, index)));
        }
        for study in self.main.iter().filter(|study| !study.outputs.is_empty()) {
            push(0, line(self.study_legend(study, index, muted, text)));
        }
        for study in &self.subs {
            let Some(&first) = study.outputs.first() else {
                continue;
            };
            let Some((pane, _)) = self.engine.series_price_scale(first) else {
                continue;
            };
            push(pane, line(self.study_legend(study, index, muted, text)));
        }
        let right = (self.engine.axis_w + 8.0) as f32;
        panes
            .into_iter()
            .filter_map(|(pane, lines)| {
                let top = self.engine.panes.get(pane)?.top as f32;
                Some(
                    div()
                        .absolute()
                        .top(px(top + 4.0))
                        .left(px(8.0))
                        .right(px(right))
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .text_xs()
                        .children(lines)
                        .into_any_element(),
                )
            })
            .collect()
    }

    // ---- input ------------------------------------------------------------------------------

    fn now_ms(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1_000.0
    }

    fn local(&self, position: Point<Pixels>) -> (f64, f64, f64) {
        let x: f32 = position.x.into();
        let y: f32 = position.y.into();
        let chart_x = f64::from(x - self.viewport_offset.0);
        let y = f64::from(y - self.viewport_offset.1);
        (chart_x, chart_x - self.engine.pane_left, y)
    }

    fn separator_at(&self, y: f64) -> Option<usize> {
        self.engine
            .panes
            .iter()
            .skip(1)
            .position(|pane| (y - pane.top).abs() <= SEPARATOR_HIT)
    }

    fn in_plot(&self, chart_x: f64, y: f64) -> bool {
        chart_x >= self.engine.pane_left
            && chart_x <= self.engine.pane_left + self.engine.pane_w
            && (0.0..=self.engine.pane_h).contains(&y)
    }

    fn sample(&self, pane_x: f64, y: f64, modifiers: Modifiers) -> PointerSample {
        PointerSample {
            id: 1,
            device: InputDevice::Mouse,
            target: self.input_target,
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

    fn drawing_modifiers(&self, modifiers: Modifiers) -> DrawingModifiers {
        DrawingModifiers {
            magnet: self.magnet || modifiers.control || modifiers.platform,
            straighten: modifiers.shift,
        }
    }

    fn target_at(&self, chart_x: f64, pane_x: f64, y: f64) -> InputTarget {
        if self.separator_at(y).is_some() {
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

    fn clear_hover(&mut self) {
        self.engine.set_hovered_series(None);
        self.engine.set_hovered_text(None);
        self.engine.set_hovered_drawing(None);
    }

    fn update_pointer(&mut self, chart_x: f64, pane_x: f64, y: f64) {
        let separator = self.separator_at(y);
        let resizing = matches!(self.drag, Some(Drag::Separator { .. }));
        let hover = (!resizing).then_some(separator).flatten();
        if self.engine.separator_hover != hover {
            self.engine.set_separator_hover(hover);
        }
        let in_plot = self.in_plot(chart_x, y);
        if in_plot && separator.is_none() && !resizing {
            self.engine.crosshair = Some((pane_x, y));
            if let Some(hit) = self.engine.hit_test_drawing(pane_x, y) {
                self.engine.set_hovered_series(None);
                self.engine.set_hovered_text(Some(hit.id));
                self.engine.set_hovered_drawing(Some(hit.id));
            } else {
                self.engine.set_hovered_text(None);
                self.engine.set_hovered_drawing(None);
                let series = self.engine.hit_test_series(pane_x, y);
                self.engine.set_hovered_series(series);
            }
            let logical = (self.engine.time_scale.coordinate_to_float_index(pane_x) + 0.5).floor();
            self.hover = (logical >= 0.0 && !self.candles.is_empty())
                .then(|| (logical as usize).min(self.candles.len() - 1));
        } else {
            self.engine.crosshair = None;
            self.clear_hover();
            self.hover = None;
        }
        let drawing_cursor = (self.engine.active_drawing_tool().is_none() && in_plot)
            .then(|| self.engine.hit_test_drawing(pane_x, y))
            .flatten()
            .map(|hit| match hit.cursor {
                "pointer" => CursorStyle::PointingHand,
                "move" | "grab" | "grabbing" => CursorStyle::ClosedHand,
                "ns-resize" => CursorStyle::ResizeUpDown,
                "ew-resize" => CursorStyle::ResizeLeftRight,
                "nwse-resize" => CursorStyle::ResizeUpLeftDownRight,
                "nesw-resize" => CursorStyle::ResizeUpRightDownLeft,
                _ => CursorStyle::Crosshair,
            });
        self.cursor = if resizing || separator.is_some() {
            CursorStyle::ResizeRow
        } else if y > self.engine.pane_h {
            CursorStyle::ResizeLeftRight
        } else if !in_plot {
            CursorStyle::ResizeUpDown
        } else if let Some(cursor) = drawing_cursor {
            cursor
        } else {
            CursorStyle::Crosshair
        };
        self.dirty = true;
    }

    fn on_hover(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if !*hovered && self.drag.is_none() {
            self.engine.crosshair = None;
            self.clear_hover();
            self.hover = None;
            self.dirty = true;
            cx.notify();
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        self.engine.time_scale_end_scroll();
        self.engine.cancel_scroll_animation();
        let (chart_x, pane_x, y) = self.local(event.position);
        self.input_target = self.target_at(chart_x, pane_x, y);
        let sample = self.sample(pane_x, y, event.modifiers);
        self.input.pointer_down(sample);
        self.press_start = Some((pane_x, y));
        self.press_moved = false;
        let modifiers = self.drawing_modifiers(event.modifiers);

        if event.click_count >= 2 && self.engine.drawing_tool_sequence_active() {
            self.engine.drawing_tool_activate(pane_x, y, modifiers);
            self.engine.drawing_tool_finish();
            self.press_moved = true;
            self.dirty = true;
            cx.notify();
            return;
        }
        if self.engine.active_drawing_tool().is_some() {
            let update = self.engine.drawing_tool_pointer_down(pane_x, y, modifiers);
            self.creation_press_committed = update.created.is_some();
            if update.pointer_capture {
                self.drag = Some(Drag::DrawingCreation);
            }
            self.dirty = true;
            cx.notify();
            return;
        }
        let pane = self.engine.pane_index_at_y(y);
        if event.click_count >= 2 {
            // Double-click an axis to reset it, as in KLineChart.
            if y > self.engine.pane_h {
                self.engine.reset_time_scale();
            } else if let Some(target) = self.engine.price_axis_target_at(pane, pane_x) {
                self.engine.reset_price_scale(pane, target);
            }
            self.press_moved = true;
            self.dirty = true;
            cx.notify();
            return;
        }
        self.drag = if let Some(index) = self.separator_at(y) {
            self.engine.set_separator_hover(None);
            Some(Drag::Separator { index, last_y: y })
        } else if y > self.engine.pane_h {
            self.engine.time_axis_start_scale(pane_x);
            Some(Drag::TimeAxis)
        } else if chart_x < self.engine.pane_left
            || chart_x > self.engine.pane_left + self.engine.pane_w
        {
            self.engine
                .price_axis_target_at(pane, pane_x)
                .filter(|&target| self.engine.price_axis_scalable(pane, target))
                .map(|target| {
                    self.engine.price_axis_start_scale(pane, target, y);
                    Drag::PriceAxis { pane, target }
                })
        } else if self.engine.drawing_drag_start_at(pane_x, y) {
            Some(Drag::Drawing)
        } else {
            let price_pan = self
                .engine
                .price_pan_target_at(pane, pane_x, y)
                .map(|target| (pane, target));
            Some(Drag::Pan { price_pan })
        };
        self.update_pointer(chart_x, pane_x, y);
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (chart_x, pane_x, y) = self.local(event.position);
        let sample = self.sample(pane_x, y, event.modifiers);
        let update = self.input.pointer_move(sample);
        if event.dragging() {
            if let Some((sx, sy)) = self.press_start {
                self.press_moved |= (pane_x - sx).abs() + (y - sy).abs() >= CLICK_SLOP;
            }
        }
        let modifiers = self.drawing_modifiers(event.modifiers);
        let started = update.kind == GestureUpdateKind::DragStarted;
        let moving = matches!(
            update.kind,
            GestureUpdateKind::DragStarted | GestureUpdateKind::DragMoved
        );
        match self.drag {
            Some(Drag::Pan { price_pan }) if event.dragging() && started => {
                self.engine.time_scale_end_scroll();
                self.engine.time_scale_start_scroll(pane_x);
                if let Some((pane, target)) = price_pan {
                    self.engine.price_axis_start_scroll(pane, target, y);
                }
                self.drag_started = true;
            }
            Some(Drag::Pan { price_pan })
                if event.dragging()
                    && self.drag_started
                    && update.kind == GestureUpdateKind::DragMoved =>
            {
                self.engine.time_scale_scroll_to(pane_x);
                if let Some((pane, target)) = price_pan {
                    self.engine.price_axis_scroll_to(pane, target, y);
                }
            }
            Some(Drag::TimeAxis) if event.dragging() && moving => {
                self.engine.time_axis_scale_to(pane_x);
                self.drag_started = true;
            }
            Some(Drag::PriceAxis { pane, target }) if event.dragging() && moving => {
                self.engine.price_axis_scale_to(pane, target, y);
                self.drag_started = true;
            }
            Some(Drag::Separator { index, last_y }) if event.dragging() && moving => {
                self.engine.drag_pane_separator(index, y - last_y);
                self.drag = Some(Drag::Separator { index, last_y: y });
                self.drag_started = true;
            }
            Some(Drag::Drawing) if event.dragging() => {
                self.engine.drawing_drag_to(pane_x, y, modifiers);
            }
            Some(Drag::DrawingCreation) if event.dragging() => {
                // Captured tools (the brush) sample at most once per frame.
                self.pending_creation_point = Some((pane_x, y, modifiers));
            }
            _ => {
                if self.engine.active_drawing_tool().is_some() {
                    self.engine
                        .drawing_tool_pointer_move(pane_x, y, modifiers, event.dragging());
                }
            }
        }
        self.update_pointer(chart_x, pane_x, y);
        cx.notify();
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (chart_x, pane_x, y) = self.local(event.position);
        let sample = self.sample(pane_x, y, event.modifiers);
        self.input.pointer_up(sample);
        let moved = self.press_moved;
        let committed_on_press = std::mem::take(&mut self.creation_press_committed);
        self.press_start = None;
        self.press_moved = false;
        let modifiers = self.drawing_modifiers(event.modifiers);
        let click = match self.drag.take() {
            Some(Drag::Pan { price_pan }) => {
                if let Some((pane, target)) = price_pan {
                    self.engine.price_axis_end_scroll(pane, target);
                }
                if self.drag_started {
                    self.engine.time_scale_end_scroll();
                }
                !moved
            }
            Some(Drag::TimeAxis) => {
                self.engine.time_axis_end_scale();
                false
            }
            Some(Drag::PriceAxis { pane, target }) => {
                self.engine.price_axis_end_scale(pane, target);
                false
            }
            Some(Drag::Separator { .. }) => false,
            Some(Drag::Drawing) => {
                self.engine.drawing_drag_end();
                false
            }
            Some(Drag::DrawingCreation) => {
                if let Some((x, y, modifiers)) = self.pending_creation_point.take() {
                    self.engine.drawing_tool_pointer_move(x, y, modifiers, true);
                }
                self.engine.drawing_tool_pointer_up(pane_x, y, modifiers);
                false
            }
            None if committed_on_press => false,
            None if self.engine.active_drawing_tool().is_some() => {
                if !moved {
                    self.engine.drawing_tool_activate(pane_x, y, modifiers);
                }
                false
            }
            None => !moved,
        };
        self.drag_started = false;
        if click {
            self.engine.select_drawing_at(pane_x, y);
        }
        self.update_pointer(chart_x, pane_x, y);
        cx.notify();
    }

    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (chart_x, pane_x, y) = self.local(event.position);
        let delta = event.delta.pixel_delta(px(WHEEL_LINE_HEIGHT));
        let dx: f32 = delta.x.into();
        let dy: f32 = delta.y.into();
        let (nx, ny) = (f64::from(dx) / 100.0, f64::from(dy) / 100.0);
        let intent = WheelSample {
            x: pane_x,
            y,
            delta_x: nx,
            delta_y: ny,
            delta_mode: if matches!(event.delta, ScrollDelta::Pixels(_)) {
                WheelDeltaMode::Pixel
            } else {
                WheelDeltaMode::Line
            },
            modifiers: InputModifiers {
                shift: event.modifiers.shift,
                control: event.modifiers.control,
                alt: event.modifiers.alt,
                meta: event.modifiers.platform,
            },
            timestamp_ms: self.now_ms(),
        }
        .intent(WheelBehavior::Auto);
        if matches!(intent, WheelIntent::Zoom | WheelIntent::PanAndZoom) && ny != 0.0 {
            self.engine.time_scale_zoom(pane_x, wheel_zoom_scale(ny));
        }
        let pan = if nx.abs() >= ny.abs() { nx } else { -ny };
        if matches!(intent, WheelIntent::Pan | WheelIntent::PanAndZoom) && pan != 0.0 {
            self.engine.time_scale_start_scroll(0.0);
            self.engine
                .time_scale_scroll_to(WHEEL_SCROLL_PX_PER_DELTA * pan);
            self.engine.time_scale_end_scroll();
        }
        self.update_pointer(chart_x, pane_x, y);
        cx.stop_propagation();
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let center = self.engine.pane_w / 2.0;
        let handled = match event.keystroke.key.as_str() {
            "+" | "=" => {
                self.engine.time_scale_zoom(center, 0.5);
                true
            }
            "-" => {
                self.engine.time_scale_zoom(center, -0.5);
                true
            }
            "home" => {
                self.reset_view = true;
                true
            }
            "enter" => self.engine.drawing_tool_finish().created.is_some(),
            "delete" | "backspace" => self.engine.remove_selected_drawing(),
            "escape" => {
                self.engine.cancel_drawing_tool();
                self.pending_creation_point = None;
                self.engine.set_selected_drawing(None);
                true
            }
            _ => false,
        };
        if handled {
            self.dirty = true;
            cx.stop_propagation();
            cx.notify();
        }
    }
}

impl Focusable for ChartView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ChartView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let prepaint_entity = entity.clone();
        let background = css_hsla(&self.engine.options.get().layout.background.color);
        let legends = self.render_legends(cx);
        self.legend_tops = self.pane_tops().collect();
        div()
            .id("kline-chart")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(background)
            .cursor(self.cursor)
            .track_focus(&self.focus)
            .on_hover(cx.listener(Self::on_hover))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .on_key_down(cx.listener(Self::on_key_down))
            .child(
                canvas(
                    move |bounds: Bounds<Pixels>, window, cx| {
                        let width: f32 = bounds.size.width.into();
                        let height: f32 = bounds.size.height.into();
                        let offset = (bounds.origin.x.into(), bounds.origin.y.into());
                        let scale_factor = window.scale_factor();
                        prepaint_entity.update(cx, |view: &mut ChartView, cx| {
                            view.viewport_offset = offset;
                            view.rebuild(width, height, scale_factor, window);
                            // Legends are laid out before the chart; follow a pane change.
                            if !view.legend_tops.iter().copied().eq(view.pane_tops()) {
                                cx.notify();
                            }
                        });
                        bounds
                    },
                    move |_, bounds, window, cx| {
                        entity.update(cx, |view: &mut ChartView, cx| {
                            view.paint(bounds, window, cx);
                        });
                    },
                )
                .size_full(),
            )
            .children(legends)
    }
}

/// Up and down colors for the chosen convention.
pub fn market_colors(red_up: bool) -> (&'static str, &'static str) {
    if red_up {
        (RED, GREEN)
    } else {
        (GREEN, RED)
    }
}

pub fn css_hsla(css: &str) -> Hsla {
    to_hsla(Color::parse_css(css).unwrap_or(Color::rgb(0x76, 0x80, 0x8f)))
}

/// A GPUI color as CSS `rgba(…)`, for the chart options.
pub fn hsla_css(color: Hsla) -> String {
    let rgba = color.to_rgb();
    format!(
        "rgba({},{},{},{:.3})",
        (rgba.r * 255.0).round(),
        (rgba.g * 255.0).round(),
        (rgba.b * 255.0).round(),
        rgba.a
    )
}

/// Day names by days since 1970-01-01 (a Thursday) modulo 7.
const WEEKDAYS: [&str; 7] = ["周四", "周五", "周六", "周日", "周一", "周二", "周三"];

pub fn format_time(time: i64, period: Period) -> String {
    let days = time.div_euclid(DAY);
    let (year, month, day) = civil_from_days(days);
    let seconds = time.rem_euclid(DAY);
    match period {
        Period::Intraday | Period::Min1 | Period::Min5 | Period::Min15 | Period::Hour1 => format!(
            "{year}-{month:02}-{day:02} {:02}:{:02}",
            seconds / 3_600,
            seconds % 3_600 / 60
        ),
        Period::Day | Period::Week => format!(
            "{year}-{month:02}-{day:02} {}",
            WEEKDAYS[days.rem_euclid(7) as usize]
        ),
        Period::Month => format!("{year}-{month:02}"),
    }
}

/// `1.23万` / `4.56亿` style counts, as Chinese trading apps show them.
pub fn format_volume(volume: f64) -> String {
    if volume.abs() >= 1.0e8 {
        format!("{:.2}亿", volume / 1.0e8)
    } else if volume.abs() >= 1.0e4 {
        format!("{:.2}万", volume / 1.0e4)
    } else {
        format!("{volume:.0}")
    }
}

/// A JSON string literal, for drawing text in option patches.
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    // Named imports: `super::*` would bring GPUI's `test` attribute in through `gpui_kit::*`.
    use super::{format_time, format_volume, json_string, market_colors, GREEN, RED};
    use crate::market::Period;

    #[test]
    fn times_read_like_chinese_trading_apps() {
        // 2026-09-25 14:09 exchange time, a Friday.
        let time = 1_790_294_400 + 14 * 3_600 + 9 * 60;
        assert_eq!(format_time(time, Period::Min5), "2026-09-25 14:09");
        assert_eq!(format_time(time, Period::Day), "2026-09-25 周五");
        assert_eq!(format_time(time, Period::Month), "2026-09");
    }

    #[test]
    fn counts_use_wan_and_yi() {
        assert_eq!(format_volume(9_999.0), "9999");
        assert_eq!(format_volume(14_311_500.0), "1431.15万");
        assert_eq!(format_volume(2_720_000_000.0), "27.20亿");
    }

    #[test]
    fn drawing_text_is_escaped_for_json() {
        assert_eq!(json_string("目标\"价\"\n"), r#""目标\"价\"\n""#);
    }

    #[test]
    fn market_colors_swap_with_the_convention() {
        assert_eq!(market_colors(false), (GREEN, RED));
        assert_eq!(market_colors(true), (RED, GREEN));
    }
}
