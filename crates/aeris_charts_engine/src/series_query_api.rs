//! Series data/coordinate queries mirroring the reference series API (`price_to_coordinate`,
//! `data_by_index`, `bars_in_logical_range`, ...). Extracted from `lib.rs`.

use super::*;

const MAX_AREA_BRUSH_RANGES: usize = 64;

impl ChartEngine {
    /// Install transient brush styling on an ordinary Area series. Canonical data and all ordinary
    /// series behavior stay on the built-in Area path; an empty/absent interaction clears this state.
    pub fn set_area_brush_state(
        &mut self,
        id: SeriesId,
        outside: BrushStyle,
        ranges: Vec<BrushRange>,
    ) -> bool {
        let Some(series) = self
            .series
            .iter_mut()
            .find(|series| series.id == id && !series.removed && series.kind == SeriesKind::Area)
        else {
            return false;
        };
        if ranges.len() > MAX_AREA_BRUSH_RANGES
            || !outside.line_width.is_finite()
            || outside.line_width <= 0.0
            || ranges.iter().any(|range| {
                !range.from.is_finite()
                    || !range.to.is_finite()
                    || !range.style.line_width.is_finite()
                    || range.style.line_width <= 0.0
            })
        {
            return false;
        }
        series.area_brush = Some(AreaBrushState { outside, ranges });
        self.invalidate_frame_scene();
        true
    }

    /// Engine-owned default brush styles for an Area series, derived from its rendered stroke,
    /// its line width, and the canonical area-fill strength. Hosts override individual fields.
    pub fn area_brush_defaults(&self, id: SeriesId) -> Option<AreaBrushDefaults> {
        self.series
            .iter()
            .find(|series| series.id == id && !series.removed && series.kind == SeriesKind::Area)
            .map(crate::frame::area_brush_defaults)
    }

    pub fn clear_area_brush_state(&mut self, id: SeriesId) -> bool {
        let Some(series) = self
            .series
            .iter_mut()
            .find(|series| series.id == id && !series.removed)
        else {
            return false;
        };
        let changed = series.area_brush.take().is_some();
        if changed {
            self.invalidate_frame_scene();
        }
        true
    }

    /// A unified value snapshot for every live series. `None` selects each engine-owned series' own
    /// latest non-whitespace row; `Some(index)` performs an exact merged-logical lookup and never
    /// borrows a neighboring value. Custom-series values are host-produced, so only their last
    /// recorded frame value is available in latest mode. The query retains only its result and does
    /// not copy series history.
    pub fn value_snapshot(&self, logical_index: Option<i64>) -> Vec<SeriesValueSnapshot> {
        self.series_order
            .iter()
            .filter_map(|&id| self.series_value_snapshot(id, logical_index))
            .collect()
    }

    fn series_value_snapshot(
        &self,
        id: SeriesId,
        requested_index: Option<i64>,
    ) -> Option<SeriesValueSnapshot> {
        let series = self.series_entry(id)?;
        let feature_kind = self.feature_series_kind(id);
        let price_scale_id = self
            .price_scale_id_for_target(series.pane_index, series.price_scale_target)
            .unwrap_or("")
            .to_string();
        let mut snapshot = SeriesValueSnapshot {
            series_id: id,
            kind: series.kind,
            feature_kind,
            pane_index: series.pane_index,
            price_scale_id,
            logical_index: requested_index,
            time: requested_index
                .and_then(|index| usize::try_from(index).ok())
                .and_then(|index| self.axis_time_key_at(index)),
            open: None,
            high: None,
            low: None,
            close: None,
            value: None,
            previous_value: None,
            formatted_open: None,
            formatted_high: None,
            formatted_low: None,
            formatted_close: None,
            formatted_value: None,
            formatted_previous_value: None,
        };

        if series.kind == SeriesKind::Custom {
            if requested_index.is_none() {
                let latest = series
                    .custom_frame
                    .last
                    .filter(|last| last.value.is_finite());
                if let Some(latest) = latest {
                    snapshot.logical_index = self
                        .data
                        .merged_times()
                        .binary_search(&latest.time)
                        .ok()
                        .and_then(|index| i64::try_from(index).ok());
                    snapshot.time = Some(latest.time);
                    snapshot.value = Some(latest.value);
                    snapshot.formatted_value =
                        Some(self.format_series_resolved(series, latest.value));
                }
            }
            return Some(snapshot);
        }

        let plot = self.data.plot(id);
        let row = match requested_index {
            Some(index) => plot
                .search(index, MismatchDirection::None)
                .filter(|&row| !plot.is_whitespace_row(row)),
            None => plot.last_non_whitespace_row(TimePointIndex::MAX),
        };
        let Some(row) = row else {
            return Some(snapshot);
        };
        let index = plot.index_at(row)?;
        let time = self.axis_time_key_at(index as usize)?;
        snapshot.logical_index = Some(index);
        snapshot.time = Some(time);

        let format = |value: f64| {
            value
                .is_finite()
                .then(|| self.format_series_resolved(series, value))
        };
        let close = plot.value_at(row, PlotValueIndex::Close);
        if matches!(
            series.kind,
            SeriesKind::Candlestick | SeriesKind::Bar | SeriesKind::Footprint
        ) {
            let open = plot.value_at(row, PlotValueIndex::Open);
            let high = plot.value_at(row, PlotValueIndex::High);
            let low = plot.value_at(row, PlotValueIndex::Low);
            snapshot.open = open.is_finite().then_some(open);
            snapshot.high = high.is_finite().then_some(high);
            snapshot.low = low.is_finite().then_some(low);
            snapshot.close = close.is_finite().then_some(close);
            snapshot.formatted_open = format(open);
            snapshot.formatted_high = format(high);
            snapshot.formatted_low = format(low);
            snapshot.formatted_close = format(close);
        } else {
            snapshot.value = close.is_finite().then_some(close);
            snapshot.formatted_value = format(close);
        }
        if let Some(previous_row) = plot.last_non_whitespace_row_before(row) {
            let previous = plot.value_at(previous_row, PlotValueIndex::Close);
            snapshot.previous_value = previous.is_finite().then_some(previous);
            snapshot.formatted_previous_value = format(previous);
        }
        Some(snapshot)
    }

    pub fn series_price_to_coordinate(&self, id: SeriesId, price: f64) -> Option<f64> {
        if !price.is_finite() {
            return None;
        }
        let (pane, target) = self.series_price_scale(id)?;
        let scale = self.price_scale_for(pane, target)?;
        if scale.is_empty() {
            return None;
        }
        let base = self.visible_series_base_value(id)?;
        Some(scale.price_to_coordinate(price, base))
    }

    pub fn series_coordinate_to_price(&self, id: SeriesId, coordinate: f64) -> Option<f64> {
        if !coordinate.is_finite() {
            return None;
        }
        let (pane, target) = self.series_price_scale(id)?;
        let scale = self.price_scale_for(pane, target)?;
        if scale.is_empty() {
            return None;
        }
        let base = self.visible_series_base_value(id)?;
        Some(scale.coordinate_to_price(coordinate, base))
    }

    pub fn series_kind(&self, id: SeriesId) -> Option<SeriesKind> {
        self.series
            .iter()
            .find(|series| series.id == id && !series.removed)
            .map(|series| series.kind)
    }

    /// Apply a per-series `priceFormat` JSON patch (reference `series.applyOptions({ priceFormat })`):
    /// `{"type":"price"|"volume"|"percent"|"custom", "precision"?, "min_move"?, "tick_ladder"?}`
    /// (`minMove` accepted as an alias). Absent keys keep their current values (reference merge
    /// semantics), except that a built-in type naming `min_move` without `precision` derives the
    /// precision from it (reference `precisionByMinMove`). `tick_ladder` is an ascending array of
    /// `{from, min_move, precision?}` price bands (`null` clears it). Switching to a non-custom
    /// type clears any installed custom formatter fn; `{type:"custom"}` keeps the installed fn.
    /// Returns false for a malformed patch or ladder, an unknown type, or an unknown/removed id.
    pub fn series_apply_price_format_json(&mut self, id: SeriesId, json: &str) -> bool {
        let Ok(serde_json::Value::Object(patch)) = serde_json::from_str::<serde_json::Value>(json)
        else {
            return false;
        };
        let Some(s) = self.series.iter_mut().find(|s| s.id == id && !s.removed) else {
            return false;
        };
        let Some(kind) = patch.get("type").and_then(serde_json::Value::as_str) else {
            return false;
        };
        let kind = match kind {
            "price" => PriceFormatKind::Price,
            "volume" => PriceFormatKind::Volume,
            "percent" => PriceFormatKind::Percent,
            "custom" => PriceFormatKind::Custom,
            _ => return false,
        };
        // `tick_ladder`: `[{from, min_move, precision?}, ...]` price bands, or `null` to clear.
        // Validated before any field changes so a malformed ladder leaves the format intact.
        let tick_ladder = match patch.get("tick_ladder") {
            None => None,
            Some(serde_json::Value::Null) => Some(None),
            Some(value) => match parse_tick_ladder(value) {
                Some(ladder) => Some(Some(ladder)),
                None => return false,
            },
        };
        let precision = patch.get("precision").and_then(serde_json::Value::as_u64);
        if let Some(precision) = precision {
            // 10^precision is computed at format time; clamp to the exact f64 integer range.
            s.price_format.precision = precision.min(15) as u32;
        }
        if let Some(min_move) = patch
            .get("min_move")
            .or_else(|| patch.get("minMove"))
            .and_then(serde_json::Value::as_f64)
        {
            if min_move.is_finite() && min_move > 0.0 {
                s.price_format.min_move = min_move;
                // reference `precisionByMinMove` (chart-api.ts `patchPriceFormat`): a built-in
                // format that names `min_move` without `precision` derives its decimals from it,
                // so `{type:"price", min_move:0.0001}` labels with 4 decimals.
                if precision.is_none() && kind != PriceFormatKind::Custom {
                    s.price_format.precision =
                        aeris_charts_core::format::price_formatter::precision_by_min_move(min_move);
                }
            }
        }
        if let Some(tick_ladder) = tick_ladder {
            s.price_format.tick_ladder = tick_ladder;
        }
        s.price_format.kind = kind;
        if kind != PriceFormatKind::Custom {
            s.price_format.formatter = None;
        }
        // Formatter changes can alter scale tick spacing, autoscale minimum movement, axis width,
        // and therefore the pane/time-scale layout. Reference series.applyOptions uses fullUpdate.
        self.invalidate_frame_all();
        true
    }

    /// Install the host formatter fn for a custom price format (reference
    /// `priceFormat: {type:'custom', formatter}`): switches the series to
    /// [`PriceFormatKind::Custom`], keeping its precision/min_move. A `None` return from the
    /// callback falls back to the built-in price formatter. Returns false for an
    /// unknown/removed id.
    pub fn set_series_price_formatter(&mut self, id: SeriesId, f: PriceFormatterFn) -> bool {
        let Some(s) = self.series.iter_mut().find(|s| s.id == id && !s.removed) else {
            return false;
        };
        s.price_format.kind = PriceFormatKind::Custom;
        s.price_format.formatter = Some(f);
        self.invalidate_frame_all();
        true
    }

    /// Reconstruct the series' current options as a snake_case JSON object (TS
    /// `series_options` field names). Unset color overrides serialize as `""` — the
    /// follow-body/engine-default state the setters already accept. Every color slot is
    /// stored as a verbatim CSS string and returned exactly as applied (reference `options()`
    /// parity); unparseable strings fall back to their default at render time. `None` for
    /// an unknown or removed series.
    pub fn series_options_json(&self, id: SeriesId) -> Option<String> {
        let s = self.series.iter().find(|s| s.id == id && !s.removed)?;
        // Verbatim CSS color slots (reference stores the applied string): `""` when unset.
        let verbatim = |value: &Option<String>| value.clone().unwrap_or_default();
        let line_type = match s.line_type {
            LineType::WithSteps => "stepped",
            LineType::Curved => "curved",
            LineType::Simple => "simple",
        };
        let price_scale_id = self
            .panes
            .get(s.pane_index)
            .and_then(|pane| pane.public_id_for_target(s.price_scale_target))
            .unwrap_or("right");
        // reference PriceFormat wire form; a custom format's fn is not serializable (reference `options()`
        // returns it, but the JSON boundary carries only the declarative keys).
        let price_format = match s.price_format.kind {
            PriceFormatKind::Price => {
                let mut format = serde_json::json!({
                    "type": "price",
                    "precision": s.price_format.precision,
                    "min_move": s.price_format.min_move,
                });
                if let Some(ladder) = &s.price_format.tick_ladder {
                    format["tick_ladder"] = ladder
                        .bands()
                        .iter()
                        .map(|band| {
                            serde_json::json!({
                                "from": band.from,
                                "min_move": band.min_move,
                                "precision": band.precision,
                            })
                        })
                        .collect();
                }
                format
            }
            // the reference's `PriceFormatVolume` is exactly `{type: "volume"}` — precision is an accepted
            // apply-time superset (drives the volume formatter) but is not serialized back.
            PriceFormatKind::Volume => serde_json::json!({
                "type": "volume",
            }),
            PriceFormatKind::Percent => serde_json::json!({
                "type": "percent",
                "precision": s.price_format.precision,
            }),
            PriceFormatKind::Custom => serde_json::json!({
                "type": "custom",
                "min_move": s.price_format.min_move,
            }),
        };
        // Built imperatively: the field set outgrows the `json!` macro's recursion limit.
        let mut out = serde_json::Map::new();
        let mut insert = |key: &str, value: serde_json::Value| {
            out.insert(key.to_string(), value);
        };
        insert(
            "color",
            s.line_color
                .clone()
                // The color this series actually strokes (an unset Area reports its Area hue).
                .unwrap_or_else(|| crate::frame::series_stroke_color(s).to_css())
                .into(),
        );
        insert("up_color", verbatim(&s.up_color).into());
        insert("down_color", verbatim(&s.down_color).into());
        insert("wick_up_color", verbatim(&s.wick_up_color).into());
        insert("wick_down_color", verbatim(&s.wick_down_color).into());
        insert("border_up_color", verbatim(&s.border_up_color).into());
        insert("border_down_color", verbatim(&s.border_down_color).into());
        insert("wick_visible", s.wick_visible.unwrap_or(true).into());
        insert("border_visible", s.border_visible.unwrap_or(true).into());
        insert(
            "line_width",
            s.line_width.unwrap_or(crate::frame::LINE_WIDTH).into(),
        );
        insert("line_type", line_type.into());
        insert("line_style", s.line_style.into());
        insert("line_visible", s.line_visible.into());
        insert("area_top_color", verbatim(&s.area_top_color).into());
        insert("area_bottom_color", verbatim(&s.area_bottom_color).into());
        insert("invert_filled_area", s.invert_filled_area.into());
        insert("break_on_trading_day", s.break_on_trading_day.into());
        insert("histogram_updown", s.histogram_updown.into());
        insert(
            "histogram_updown_rule",
            s.histogram_updown_rule.as_str().into(),
        );
        insert("base", s.base.into());
        insert(
            "baseline_value",
            s.baseline.map_or(serde_json::Value::Null, Into::into),
        );
        insert("top_fill_color1", verbatim(&s.top_fill_color1).into());
        insert("top_fill_color2", verbatim(&s.top_fill_color2).into());
        insert("top_line_color", verbatim(&s.top_line_color).into());
        insert(
            "top_line_width",
            s.top_line_width.map_or(serde_json::Value::Null, Into::into),
        );
        insert("top_line_style", s.top_line_style.into());
        insert("bottom_fill_color1", verbatim(&s.bottom_fill_color1).into());
        insert("bottom_fill_color2", verbatim(&s.bottom_fill_color2).into());
        insert("bottom_line_color", verbatim(&s.bottom_line_color).into());
        insert(
            "bottom_line_width",
            s.bottom_line_width
                .map_or(serde_json::Value::Null, Into::into),
        );
        insert("bottom_line_style", s.bottom_line_style.into());
        insert("open_visible", s.open_visible.into());
        insert("close_visible", s.close_visible.into());
        insert("thin_bars", s.thin_bars.into());
        insert("heikin_ashi", s.heikin_ashi.into());
        insert("point_markers", s.point_markers.into());
        insert(
            "point_markers_radius",
            s.point_markers_radius
                .map_or(serde_json::Value::Null, Into::into),
        );
        insert(
            "crosshair_marker_visible",
            s.crosshair_marker_visible.into(),
        );
        insert("crosshair_marker_radius", s.crosshair_marker_radius.into());
        insert(
            "crosshair_marker_border_color",
            verbatim(&s.crosshair_marker_border_color).into(),
        );
        insert(
            "crosshair_marker_background_color",
            verbatim(&s.crosshair_marker_background_color).into(),
        );
        insert(
            "crosshair_marker_border_width",
            s.crosshair_marker_border_width.into(),
        );
        insert("last_value_visible", s.last_value_visible.into());
        insert("title", s.title.clone().into());
        insert("title_visible", s.title_visible.into());
        insert("countdown_visible", s.countdown_visible.into());
        insert("price_line_visible", s.price_line_visible.into());
        insert("price_line_source", s.price_line_source.into());
        insert("price_line_extent", s.price_line_extent.as_str().into());
        insert("price_line_width", s.price_line_width.into());
        insert("price_line_color", verbatim(&s.price_line_color).into());
        insert("price_line_style", s.price_line_style.into());
        insert("bid_ask_visible", s.bid_ask_visible.into());
        insert("bid_color", s.bid_color.clone().into());
        insert("ask_color", s.ask_color.clone().into());
        insert("bid_ask_line_width", s.bid_ask_line_width.into());
        insert("bid_ask_line_style", s.bid_ask_line_style.into());
        insert("bid", s.bid.map_or(serde_json::Value::Null, Into::into));
        insert("ask", s.ask.map_or(serde_json::Value::Null, Into::into));
        insert("last_price_animation", s.last_price_animation.into());
        insert("visible", s.visible.into());
        insert("price_scale_id", price_scale_id.into());
        insert("pane", s.pane_index.into());
        insert("price_format", price_format);
        let (time_alignment, max_staleness) = match self.data.time_alignment(id) {
            Some(TimeAlignment::AsOf { max_staleness }) => ("as_of", max_staleness),
            _ => ("union", None),
        };
        insert("time_alignment", time_alignment.into());
        insert(
            "as_of_max_staleness",
            max_staleness.map_or(serde_json::Value::Null, Into::into),
        );
        Some(serde_json::Value::Object(out).to_string())
    }

    /// Merge a snake_case JSON patch of series style options into the series (reference
    /// `ISeriesApi.applyOptions` semantics): only keys present in the patch are touched, keys
    /// with the wrong type are ignored, and unknown keys are skipped silently. Colors follow
    /// the keep/clear/pin contract of the candle part colors (`""` clears an override back to
    /// its follow state). Returns false for a malformed patch or an unknown/removed id.
    pub fn series_apply_options_json(&mut self, id: SeriesId, json: &str) -> bool {
        self.invalidate_frame_scene();
        let Ok(serde_json::Value::Object(patch)) = serde_json::from_str::<serde_json::Value>(json)
        else {
            return false;
        };
        let Some(s) = self.series.iter_mut().find(|s| s.id == id && !s.removed) else {
            return false;
        };
        // Verbatim CSS color slots (reference parity): any non-empty string is stored as-is —
        // including named colors the renderer cannot parse, which fall back to the default
        // at render time — so `options()` returns exactly what was applied. `""` clears.
        let color_string_slot = |slot: &mut Option<String>, value: &serde_json::Value| {
            if let Some(css) = value.as_str() {
                *slot = (!css.is_empty()).then(|| css.to_string());
            }
        };
        let finite = |value: &serde_json::Value| value.as_f64().filter(|v| v.is_finite());
        let positive = |value: &serde_json::Value| finite(value).filter(|&v| v > 0.0);
        let non_negative = |value: &serde_json::Value| finite(value).filter(|&v| v >= 0.0);
        let u8_bounded = |value: &serde_json::Value, max: u8| {
            value
                .as_u64()
                .and_then(|v| u8::try_from(v).ok())
                .filter(|&v| v <= max)
        };
        let optional_finite = |slot: &mut Option<f64>, value: &serde_json::Value| match value {
            serde_json::Value::Null => *slot = None,
            value => {
                if let Some(v) = positive(value) {
                    *slot = Some(v);
                }
            }
        };
        for (key, value) in &patch {
            match key.as_str() {
                "color" => color_string_slot(&mut s.line_color, value),
                "up_color" => color_string_slot(&mut s.up_color, value),
                "down_color" => color_string_slot(&mut s.down_color, value),
                "wick_up_color" => color_string_slot(&mut s.wick_up_color, value),
                "wick_down_color" => color_string_slot(&mut s.wick_down_color, value),
                "border_up_color" => color_string_slot(&mut s.border_up_color, value),
                "border_down_color" => color_string_slot(&mut s.border_down_color, value),
                "wick_visible" => {
                    if let Some(v) = value.as_bool() {
                        s.wick_visible = Some(v);
                    }
                }
                "border_visible" => {
                    if let Some(v) = value.as_bool() {
                        s.border_visible = Some(v);
                    }
                }
                "line_width" => {
                    if let Some(v) = positive(value) {
                        s.line_width = Some(v);
                    }
                }
                "area_top_color" => color_string_slot(&mut s.area_top_color, value),
                "area_bottom_color" => color_string_slot(&mut s.area_bottom_color, value),
                "last_value_visible" => {
                    if let Some(v) = value.as_bool() {
                        s.last_value_visible = v;
                    }
                }
                "title" => {
                    if let Some(v) = value.as_str() {
                        s.title = v.to_string();
                    }
                }
                "title_visible" => {
                    if let Some(v) = value.as_bool() {
                        s.title_visible = v;
                    }
                }
                "countdown_visible" => {
                    if let Some(v) = value.as_bool() {
                        s.countdown_visible = v;
                    }
                }
                "price_line_visible" => {
                    if let Some(v) = value.as_bool() {
                        s.price_line_visible = v;
                    }
                }
                // reference PriceLineSource (0 LastBar, 1 LastVisible).
                "price_line_source" => {
                    if let Some(v) = u8_bounded(value, 1) {
                        s.price_line_source = v;
                    }
                }
                "price_line_extent" => {
                    if let Some(v) = value.as_str().and_then(PriceLineExtent::parse) {
                        s.price_line_extent = v;
                    }
                }
                "price_line_width" => {
                    if let Some(v) = positive(value) {
                        s.price_line_width = v;
                    }
                }
                "price_line_color" => color_string_slot(&mut s.price_line_color, value),
                "price_line_style" => {
                    if let Some(v) = u8_bounded(value, 4) {
                        s.price_line_style = v;
                    }
                }
                "bid_ask_visible" => {
                    if let Some(v) = value.as_bool() {
                        s.bid_ask_visible = v;
                    }
                }
                // Verbatim CSS like the part colors: any string stored as-is (`""` restores
                // the default at render time); renderer parses, falling back when invalid.
                "bid_color" => {
                    if let Some(v) = value.as_str() {
                        s.bid_color = if v.is_empty() {
                            aeris_charts_core::style::DEFAULT_PRIMARY_CSS.to_string()
                        } else {
                            v.to_string()
                        };
                    }
                }
                "ask_color" => {
                    if let Some(v) = value.as_str() {
                        s.ask_color = if v.is_empty() {
                            aeris_charts_core::style::MARKET_DOWN_CSS.to_string()
                        } else {
                            v.to_string()
                        };
                    }
                }
                "bid_ask_line_width" => {
                    if let Some(v) = positive(value) {
                        s.bid_ask_line_width = v;
                    }
                }
                "bid_ask_line_style" => {
                    if let Some(v) = u8_bounded(value, 4) {
                        s.bid_ask_line_style = v;
                    }
                }
                "bid" => match value {
                    serde_json::Value::Null => s.bid = None,
                    value => {
                        if let Some(v) = finite(value) {
                            s.bid = Some(v);
                        }
                    }
                },
                "ask" => match value {
                    serde_json::Value::Null => s.ask = None,
                    value => {
                        if let Some(v) = finite(value) {
                            s.ask = Some(v);
                        }
                    }
                },
                "line_style" => {
                    if let Some(v) = u8_bounded(value, 4) {
                        s.line_style = v;
                    }
                }
                "line_visible" => {
                    if let Some(v) = value.as_bool() {
                        s.line_visible = v;
                    }
                }
                "point_markers_radius" => match value {
                    serde_json::Value::Null => s.point_markers_radius = None,
                    value => {
                        if let Some(v) = positive(value) {
                            s.point_markers_radius = Some(v);
                        }
                    }
                },
                "crosshair_marker_visible" => {
                    if let Some(v) = value.as_bool() {
                        s.crosshair_marker_visible = v;
                    }
                }
                "crosshair_marker_radius" => {
                    if let Some(v) = non_negative(value) {
                        s.crosshair_marker_radius = v;
                    }
                }
                "crosshair_marker_border_color" => {
                    color_string_slot(&mut s.crosshair_marker_border_color, value)
                }
                "crosshair_marker_background_color" => {
                    color_string_slot(&mut s.crosshair_marker_background_color, value)
                }
                "crosshair_marker_border_width" => {
                    if let Some(v) = non_negative(value) {
                        s.crosshair_marker_border_width = v;
                    }
                }
                "top_fill_color1" => color_string_slot(&mut s.top_fill_color1, value),
                "top_fill_color2" => color_string_slot(&mut s.top_fill_color2, value),
                "top_line_color" => color_string_slot(&mut s.top_line_color, value),
                "top_line_width" => optional_finite(&mut s.top_line_width, value),
                "top_line_style" => {
                    if let Some(v) = u8_bounded(value, 4) {
                        s.top_line_style = v;
                    }
                }
                "bottom_fill_color1" => color_string_slot(&mut s.bottom_fill_color1, value),
                "bottom_fill_color2" => color_string_slot(&mut s.bottom_fill_color2, value),
                "bottom_line_color" => color_string_slot(&mut s.bottom_line_color, value),
                "bottom_line_width" => optional_finite(&mut s.bottom_line_width, value),
                "bottom_line_style" => {
                    if let Some(v) = u8_bounded(value, 4) {
                        s.bottom_line_style = v;
                    }
                }
                "base" => {
                    if let Some(v) = finite(value) {
                        s.base = v;
                    }
                }
                "invert_filled_area" => {
                    if let Some(v) = value.as_bool() {
                        s.invert_filled_area = v;
                    }
                }
                "break_on_trading_day" => {
                    if let Some(v) = value.as_bool() {
                        s.break_on_trading_day = v;
                    }
                }
                "open_visible" => {
                    if let Some(v) = value.as_bool() {
                        s.open_visible = v;
                    }
                }
                "close_visible" => {
                    if let Some(v) = value.as_bool() {
                        s.close_visible = v;
                    }
                }
                "thin_bars" => {
                    if let Some(v) = value.as_bool() {
                        s.thin_bars = v;
                    }
                }
                "heikin_ashi" => {
                    if let Some(v) = value.as_bool() {
                        s.heikin_ashi = v;
                    }
                }
                "histogram_updown" => {
                    if let Some(v) = value.as_bool() {
                        s.histogram_updown = v;
                    }
                }
                "histogram_updown_rule" => {
                    if let Some(rule) = value.as_str().and_then(crate::HistogramUpDownRule::parse) {
                        s.histogram_updown_rule = rule;
                    }
                }
                // Unknown keys are ignored gracefully (reference applyOptions merge semantics).
                _ => {}
            }
        }
        if let Some(value) = patch.get("price_format") {
            // Nested object patch — routed to the dedicated price-format applier so the
            // declarative keys round-trip through `series_options_json`.
            self.series_apply_price_format_json(id, &value.to_string());
        }
        true
    }

    fn series_point_at_row(&self, id: SeriesId, row: usize) -> Option<SeriesDataPoint> {
        let plot = self.data.plot(id);
        let index = plot.index_at(row)?;
        // An as-of overlay point shows one of the series' own rows: report that row's time.
        let time = if plot.is_as_of() {
            *self.data.series_data(id)?.0.get(plot.source_row(row))?
        } else {
            self.axis_time_key_at(index as usize)?
        };
        Some(SeriesDataPoint {
            time,
            open: plot.value_at(row, PlotValueIndex::Open),
            high: plot.value_at(row, PlotValueIndex::High),
            low: plot.value_at(row, PlotValueIndex::Low),
            close: plot.value_at(row, PlotValueIndex::Close),
        })
    }

    pub fn series_data_by_index(
        &self,
        id: SeriesId,
        logical_index: i64,
        mismatch: MismatchDirection,
    ) -> Option<SeriesDataPoint> {
        let row = self.data.plot(id).search(logical_index, mismatch)?;
        self.series_point_at_row(id, row)
    }

    pub fn series_data(&self, id: SeriesId) -> Vec<SeriesDataPoint> {
        if self.data.plot(id).is_as_of() {
            // An as-of overlay's data is its own rows, not the time points that show them.
            let Some((times, columns)) = self.data.series_data(id) else {
                return Vec::new();
            };
            return times
                .iter()
                .enumerate()
                .map(|(row, &time)| SeriesDataPoint {
                    time,
                    open: columns[0][row],
                    high: columns[1][row],
                    low: columns[2][row],
                    close: columns[3][row],
                })
                .collect();
        }
        let size = self.data.plot(id).size();
        (0..size)
            .filter_map(|row| self.series_point_at_row(id, row))
            .collect()
    }

    /// Format one value with the series' resolved price format, backing reference
    /// `series.priceFormatter()` (series.ts `_recreateFormatter`): the custom fn when the
    /// format is `custom` (a `None`/declining return falls through), then the built-in for
    /// the format kind, then the chart-level `localization.priceFormatter`, and finally the
    /// plain built-in price formatter. `None` for an unknown/removed id or non-finite value.
    pub fn series_format_price(&self, id: SeriesId, value: f64) -> Option<String> {
        if !value.is_finite() {
            return None;
        }
        let series = self.series.iter().find(|s| s.id == id && !s.removed)?;
        Some(self.format_series_resolved(series, value))
    }

    /// The shared series price-format resolution chain (custom fn → built-ins → chart
    /// formatter → default built-in). Used by `series_format_price` and `last_value_data`.
    pub(crate) fn format_series_resolved(&self, series: &SeriesEntry, value: f64) -> String {
        if let Some(s) = self.format_with_price_format(&series.price_format, value) {
            return s;
        }
        if let Some(f) = &self.price_formatter_fn {
            if let Some(s) = f(value) {
                return s;
            }
        }
        self.price_formatter.format(value)
    }

    /// reference `ISeriesApi.lastValueData(globalLast)` (iseries-api.ts:321, series.ts:158-211):
    /// the last (global) or last VISIBLE non-whitespace bar's close, formatted with the
    /// series' price format, plus its UTC-seconds time. Whitespace bars are skipped exactly
    /// like the reference's whitespace-filtered plot list. `None` (serialized as "" at the boundary)
    /// when the series is unknown/removed, has no real bars, or (visible mode) no real bar
    /// at or left of the visible right edge.
    pub fn series_last_value_data(&self, id: SeriesId, global_last: bool) -> Option<String> {
        let series = self.series.iter().find(|s| s.id == id && !s.removed)?;
        // A custom series' last value is the host-recorded one (Phase C-c; the plugin's
        // current value of the last / last-visible non-whitespace item), formatted with the
        // series' price format like a built-in close.
        if series.kind == SeriesKind::Custom {
            let last = if global_last {
                series.custom_frame.last
            } else {
                series.custom_frame.last_visible
            }?;
            let formatted = self.format_series_resolved(series, last.value);
            return Some(
                serde_json::json!({
                    "value": last.value,
                    "formatted": formatted,
                    "time": last.time,
                })
                .to_string(),
            );
        }
        let plot = self.data.plot(id);
        if plot.is_empty() {
            return None;
        }
        let row = if global_last {
            plot.last_non_whitespace_row(TimePointIndex::MAX)
        } else {
            let to = self.time_scale.visible_strict_range()?.right();
            plot.last_non_whitespace_row(to)
        }?;
        let value = plot.value_at(row, PlotValueIndex::Close);
        if !value.is_finite() {
            return None;
        }
        let time = if plot.is_as_of() {
            // The as-of bar's own time, like `data()`.
            *self.data.series_data(id)?.0.get(plot.source_row(row))?
        } else {
            self.axis_time_key_at(plot.index_at(row)? as usize)?
        };
        let formatted = self.format_series_resolved(series, value);
        Some(
            serde_json::json!({
                "value": value,
                "formatted": formatted,
                "time": time,
            })
            .to_string(),
        )
    }

    /// reference `barsInLogicalRange`, including its gap behavior and fractional bars-before/after
    /// results. Times are source UTC seconds of the first/last series bars inside the range.
    pub fn series_bars_in_logical_range(
        &self,
        id: SeriesId,
        from: f64,
        to: f64,
    ) -> Option<BarsInLogicalRange> {
        if !from.is_finite() || !to.is_finite() || from > to {
            return None;
        }
        let plot = self.data.plot(id);
        let data_first = plot.first_index()?;
        let data_last = plot.last_index()?;
        let strict = LogicalRange::new(from, to).to_strict();
        let first_row = plot.search(strict.left(), MismatchDirection::NearestRight);
        let last_row = plot.search(strict.right(), MismatchDirection::NearestLeft);
        let first_index = first_row.and_then(|row| plot.index_at(row));
        let last_index = last_row.and_then(|row| plot.index_at(row));

        if first_index
            .zip(last_index)
            .is_some_and(|(first, last)| first > last)
        {
            return Some(BarsInLogicalRange {
                bars_before: from - data_first as f64,
                bars_after: data_last as f64 - to,
                from: None,
                to: None,
            });
        }

        let bars_before = match first_index {
            None => from - data_first as f64,
            Some(index) if index == data_first => from - data_first as f64,
            Some(index) => (index - data_first) as f64,
        };
        let bars_after = match last_index {
            None => data_last as f64 - to,
            Some(index) if index == data_last => data_last as f64 - to,
            Some(index) => (data_last - index) as f64,
        };
        let times = first_index.zip(last_index).and_then(|(first, last)| {
            Some((
                self.axis_time_key_at(first as usize)?,
                self.axis_time_key_at(last as usize)?,
            ))
        });
        Some(BarsInLogicalRange {
            bars_before,
            bars_after,
            from: times.map(|times| times.0),
            to: times.map(|times| times.1),
        })
    }

    /// reference series option `autoscaleInfoProvider`: install (or clear with `None`) a callback
    /// that REPLACES this series' autoscale contribution. It runs during every autoscale pass with
    /// the series' own info for the visible bars (data plus primitives and marker margins; `None`
    /// when the series has no data) and returns the range and margins to use instead (`None`
    /// removes the series from autoscale). Returns false for an unknown/removed id.
    pub fn set_series_autoscale_info_provider(
        &mut self,
        id: SeriesId,
        provider: Option<AutoscaleInfoProviderFn>,
    ) -> bool {
        let Some(series) = self.series.iter_mut().find(|s| s.id == id && !s.removed) else {
            return false;
        };
        series.autoscale_info_provider = provider;
        self.invalidate_frame_scene();
        true
    }

    /// Whether a series currently has an autoscale info provider installed.
    pub fn series_has_autoscale_info_provider(&self, id: SeriesId) -> bool {
        self.series_entry(id)
            .is_some_and(|series| series.autoscale_info_provider.is_some())
    }
}

/// Parse a `tick_ladder` JSON array (`[{from, min_move, precision?}, ...]`, `minMove` accepted as
/// an alias). `None` for any malformed band or a ladder [`PriceTickLadder::new`] rejects.
fn parse_tick_ladder(value: &serde_json::Value) -> Option<PriceTickLadder> {
    let bands = value.as_array()?;
    if bands.len() > MAX_PRICE_TICK_BANDS {
        return None;
    }
    let bands = bands
        .iter()
        .map(|band| {
            let band = band.as_object()?;
            let from = band.get("from")?.as_f64()?;
            let min_move = band
                .get("min_move")
                .or_else(|| band.get("minMove"))?
                .as_f64()?;
            let mut parsed = PriceTickBand::new(from, min_move);
            if let Some(precision) = band.get("precision").filter(|value| !value.is_null()) {
                parsed.precision = u32::try_from(precision.as_u64()?).ok()?;
            }
            Some(parsed)
        })
        .collect::<Option<Vec<_>>>()?;
    PriceTickLadder::new(bands).ok()
}
