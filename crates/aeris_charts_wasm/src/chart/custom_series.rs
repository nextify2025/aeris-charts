//! Custom series (plugin platform Phase C-c, design doc §4.3; reference api/chart-api.ts
//! `addCustomSeries`, model/icustom-series.ts, model/series/custom-pane-view.ts).
//!
//! A custom series is a user-defined series TYPE. The engine owns its time mapping: its
//! data-layer rows carry times only (whitespace-style), so the merged time scale, logical
//! ranges, coordinate mapping, and visible-range math work exactly like built-ins. The host
//! stores the raw plugin items (`{time, ...}` JS objects) aligned 1:1 with those rows —
//! [`crate::custom_align`] keeps the two views in lockstep through the same sort/dedupe/update
//! rules the built-ins follow.
//!
//! Per frame, BEFORE any layout pass: the host walks each custom series' visible
//! non-whitespace items through the plugin's `price_value_builder` and records a C-b autoscale
//! contribution (the owning series' scale, same visibility gating) plus the engine's custom
//! frame values (the scale anchor, the last-value label, and the built-in last-price line).
//! AFTER the engine frame build: the plugin's `render(ctx)` records the same backend-neutral
//! draw commands as the primitive passes over the visible items — each with its absolute
//! bitmap-px bar-center x — and the decoded prims splice into the pane's `main` layer at the
//! series' paint-order mark (frame/mod.rs `series_paint_marks`), so a custom series z-orders
//! between built-in series exactly like a built-in kind instead of always painting on top.

use super::primitives::PrimitiveConverters;
use super::*;
use crate::prim_decode::{decode_commands, pane_clip};
use aeris_charts_engine::{CustomSeriesFrameValues, CustomSeriesLastValue, SeriesEntry};

#[wasm_bindgen(inline_js = r#"
// One draw context per custom-series render call (Phase C-c). The draw fns are the same
// plain-object recorders as the primitive context (every draw call is one JSON-marshallable
// command); `items` are the visible non-whitespace items with their absolute bitmap-px
// bar-center x; `price_to_y` arrives ready-made, bound to the series' own scale.
export function build_custom_series_context(commands, fields, items, price_to_y, bar_spacing) {
    return {
        pane_width: fields.pane_width,
        pane_height: fields.pane_height,
        pane_left: fields.pane_left,
        pane_top: fields.pane_top,
        dpr: fields.dpr,
        items,
        bar_spacing,
        price_to_y,
        rect(x, y, w, h, color) { commands.push({ c: "rect", x, y, w, h, color }); },
        rect_frame(x, y, w, h, color, line_width) { commands.push({ c: "rect_frame", x, y, w, h, color, line_width }); },
        hline(y, x1, x2, color, width, style) { commands.push({ c: "hline", y, x1, x2, color, width, style }); },
        vline(x, y1, y2, color, width, style) { commands.push({ c: "vline", x, y1, y2, color, width, style }); },
        polyline(points, color, width, style) { commands.push({ c: "polyline", points: Array.from(points), color, width, style }); },
        area_fill(points, base_y, top_color, bottom_color) { commands.push({ c: "area_fill", points: Array.from(points), base_y, top_color, bottom_color }); },
        circle(x, y, r, fill_color, border_color, border_width) { commands.push({ c: "circle", x, y, r, fill_color, border_color, border_width }); },
        round_rect(x, y, w, h, r, color) { commands.push({ c: "round_rect", x, y, w, h, r, color }); },
        triangle(x1, y1, x2, y2, x3, y3, color) { commands.push({ c: "triangle", x1, y1, x2, y2, x3, y3, color }); },
        text(x, y, text, options) { commands.push({ c: "text", x, y, text, ...(options ?? {}) }); },
    };
}
"#)]
extern "C" {
    fn build_custom_series_context(
        commands: &js_sys::Array,
        fields: &js_sys::Object,
        items: &js_sys::Array,
        price_to_y: &js_sys::Function,
        bar_spacing: f64,
    ) -> js_sys::Object;
}

/// One registered custom series (Phase C-c): the owning engine series id, the retained
/// pane-view plugin object (reference `ICustomSeriesPaneView`, adapted to a plain JS object by the
/// TS package), and the raw items aligned with the engine's time-only rows (post-sanitize
/// order — `items[i]` is the row `times[i]`).
pub(super) struct CustomSeriesEntry {
    pub series: u32,
    pub view: js_sys::Object,
    pub items: Vec<JsValue>,
    pub times: Vec<i64>,
}

/// The result of walking one custom series' items for a frame: the autoscale union plus the
/// first/last value records the engine's built-in chrome consumes.
#[derive(Default)]
struct CustomSeriesWalk {
    min: Option<f64>,
    max: Option<f64>,
    first_value: Option<f64>,
    last: Option<CustomSeriesLastValue>,
    last_visible: Option<CustomSeriesLastValue>,
}

/// Read an optional function hook from the pane-view object (absent/non-function → `None`).
fn custom_hook(view: &js_sys::Object, name: &str) -> Option<js_sys::Function> {
    js_sys::Reflect::get(view, &name.into())
        .ok()
        .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
}

/// The plugin's whitespace verdict for one item: its `is_whitespace` hook when given (a throw
/// is contained and the item counts as data), else the contract default — the item has only
/// `time` (the reference's `{time}`-only whitespace item; a `color` or any value field marks data).
fn custom_is_whitespace(
    view: &js_sys::Object,
    check: Option<&js_sys::Function>,
    item: &JsValue,
) -> bool {
    if let Some(check) = check {
        return match check.call1(view, item) {
            Ok(result) => result.is_truthy(),
            Err(error) => {
                web_sys::console::warn_1(
                    &format!("aeris_charts: custom series `is_whitespace` threw — {error:?}")
                        .into(),
                );
                false
            }
        };
    }
    match item.dyn_ref::<js_sys::Object>() {
        Some(obj) => js_sys::Object::keys(obj).length() <= 1,
        None => false,
    }
}

/// One item's `price_value_builder` result as `(finite values, current)`: the finite values
/// feed the autoscale union; `current` is the raw LAST element (the reference's custom plot-row Close
/// slot, get-series-plot-row-creator.ts `value: [last, max, min, last]`) and may be
/// non-finite, in which case the item carries no chrome value. A throw or a non-array result
/// skips the item (a broken plugin must never take the frame down).
fn custom_price_values(
    builder: &js_sys::Function,
    view: &js_sys::Object,
    item: &JsValue,
) -> (Vec<f64>, Option<f64>) {
    let result = match builder.call1(view, item) {
        Ok(result) => result,
        Err(error) => {
            web_sys::console::warn_1(
                &format!("aeris_charts: custom series `price_value_builder` threw — {error:?}")
                    .into(),
            );
            return (Vec::new(), None);
        }
    };
    let Ok(array) = result.dyn_into::<js_sys::Array>() else {
        web_sys::console::warn_1(
            &"aeris_charts: custom series `price_value_builder` must return an array of numbers"
                .into(),
        );
        return (Vec::new(), None);
    };
    let current = array.iter().last().and_then(|v| v.as_f64());
    let values = array
        .iter()
        .filter_map(|v| v.as_f64())
        .filter(|v| v.is_finite())
        .collect();
    (values, current.filter(|v| v.is_finite()))
}

/// the reference's custom barColorer (series-bar-colorer.ts Custom arm): the data item's `color` wins,
/// then the series `color` option, then the reference's `customStyleDefaults.color`.
fn custom_bar_color(item: &JsValue, series: &SeriesEntry) -> Color {
    js_sys::Reflect::get(item, &"color".into())
        .ok()
        .and_then(|c| c.as_string())
        .and_then(|css| Color::parse_css(&css))
        .unwrap_or_else(|| {
            series
                .line_color
                .as_deref()
                .and_then(Color::parse_css)
                .unwrap_or(aeris_charts_engine::DEFAULT_LINE_COLOR)
        })
}

impl ChartInner {
    pub(super) fn destroy_all_custom_series(&mut self) {
        for entry in std::mem::take(&mut self.custom_series) {
            fire_custom_destroy(&entry);
        }
    }

    /// Add a custom series (reference `addCustomSeries`) and return its engine id. The pane view
    /// must carry `price_value_builder` and `render` functions (reference `ensure(customPaneView)`);
    /// `is_whitespace`/`default_options`/`destroy` are optional. `adopt_primary` converts the
    /// engine's construction-time series 0 instead of allocating a new one (the TS package's
    /// first-series adoption, mirroring `add_series`).
    pub(super) fn add_custom_series(&mut self, view: js_sys::Object, adopt_primary: bool) -> u32 {
        if custom_hook(&view, "price_value_builder").is_none()
            || custom_hook(&view, "render").is_none()
        {
            web_sys::console::warn_1(
                &"aeris_charts: add_custom_series rejected — the pane view needs `price_value_builder` and `render` functions".into(),
            );
            return u32::MAX;
        }
        let id = if adopt_primary {
            self.engine.convert_series_kind(0, SeriesKind::Custom);
            0
        } else {
            self.engine.add_series(SeriesKind::Custom)
        };
        self.custom_series.push(CustomSeriesEntry {
            series: id,
            view,
            items: Vec::new(),
            times: Vec::new(),
        });
        id
    }

    /// Replace a custom series' items (reference `ISeriesApi.setData`). Each item must carry a
    /// `time` (UTC seconds); the raw items are kept verbatim (for `render`/`data()`) and only
    /// their times cross into the engine, as whitespace-style rows. Invalid timestamps reject the
    /// complete batch; otherwise out-of-order input stably sorts and duplicates collapse last-wins.
    pub(super) fn set_custom_series_data(
        &mut self,
        id: u32,
        items: js_sys::Array,
    ) -> Option<String> {
        let Some(position) = self.custom_series.iter().position(|e| e.series == id) else {
            web_sys::console::warn_1(
                &format!("aeris_charts: set_custom_series_data for unknown custom series {id}")
                    .into(),
            );
            return Some(rejected_diagnostics_json("unknown custom series id"));
        };
        let raw: Vec<JsValue> = items.iter().collect();
        let times: Vec<f64> = raw
            .iter()
            .map(|item| {
                js_sys::Reflect::get(item, &"time".into())
                    .ok()
                    .and_then(|t| t.as_f64())
                    .unwrap_or(f64::NAN)
            })
            .collect();
        let (times, aligned, report) = match crate::custom_align::sanitize_items(&times, raw) {
            Ok(sanitized) => sanitized,
            Err(error) => {
                web_sys::console::warn_1(
                    &format!("aeris_charts: set_custom_series_data rejected — {error}").into(),
                );
                return Some(rejected_validation_diagnostics_json(error));
            }
        };
        if !report.is_clean() {
            web_sys::console::warn_1(
                &format!(
                    "aeris_charts: set_custom_series_data sanitized data — accepted {}, dropped {} invalid, {} duplicate{}",
                    report.accepted,
                    report.dropped_invalid,
                    report.dropped_duplicate,
                    if report.reordered { ", reordered" } else { "" },
                )
                .into(),
            );
        }
        let count = times.len();
        let entry = &mut self.custom_series[position];
        entry.times = times.clone();
        entry.items = aligned;
        self.engine.install_series_data(
            id as SeriesId,
            times,
            vec![f64::NAN; count],
            vec![f64::NAN; count],
            vec![f64::NAN; count],
            vec![f64::NAN; count],
        );
        validation_diagnostics_json(&report)
    }

    /// Streaming update of a custom series (reference `ISeriesApi.update`): append a new time or
    /// replace the item at an existing one (a mid-history change splices, like the data
    /// layer's rebuild case). An invalid time rejects the update with no mutation.
    pub(super) fn update_custom_series_item(&mut self, id: u32, item: JsValue) -> Option<String> {
        let Some(position) = self.custom_series.iter().position(|e| e.series == id) else {
            web_sys::console::warn_1(
                &format!("aeris_charts: update_custom_series_item for unknown custom series {id}")
                    .into(),
            );
            return Some(rejected_diagnostics_json("unknown custom series id"));
        };
        let time = js_sys::Reflect::get(&item, &"time".into())
            .ok()
            .and_then(|t| t.as_f64())
            .unwrap_or(f64::NAN);
        let time = match aeris_charts_core::model::data_validation::validate_timestamp(time) {
            Ok(time) => time,
            Err(error) => {
                web_sys::console::warn_1(
                    &format!("aeris_charts: update_custom_series_item rejected — {error}").into(),
                );
                return Some(rejected_validation_diagnostics_json(
                    aeris_charts_core::model::data_validation::ValidationError::InvalidTimestamp {
                        index: 0,
                        error,
                    },
                ));
            }
        };
        let entry = &mut self.custom_series[position];
        crate::custom_align::upsert_item(&mut entry.times, &mut entry.items, time, item);
        self.engine
            .update_series_bar(id as SeriesId, time as f64, [f64::NAN; 4]);
        None
    }

    /// The custom series' raw items aligned with the engine rows (post-sanitize order),
    /// backing the TS `series.data()`; `null` for an unknown id.
    pub(super) fn custom_series_data(&self, id: u32) -> JsValue {
        let Some(entry) = self.custom_series.iter().find(|e| e.series == id) else {
            return JsValue::NULL;
        };
        let out = js_sys::Array::new();
        for item in &entry.items {
            out.push(item);
        }
        out.into()
    }

    /// The custom item at a logical index (the engine plot's mismatch-direction search,
    /// backing the TS `series.data_by_index`); `null` off the data or for an unknown id.
    pub(super) fn custom_series_data_by_index(&self, id: u32, index: f64, mismatch: i8) -> JsValue {
        if !index.is_finite() || index.fract() != 0.0 {
            return JsValue::NULL;
        }
        let Some(entry) = self.custom_series.iter().find(|e| e.series == id) else {
            return JsValue::NULL;
        };
        self.engine
            .data_layer()
            .plot(id as SeriesId)
            .search(index as i64, mismatch_direction_from_i8(mismatch))
            .and_then(|row| entry.items.get(row).cloned())
            .unwrap_or(JsValue::NULL)
    }

    /// Drop every custom-series entry whose owning series is gone, firing the pane view's
    /// `destroy` hook (reference `ICustomSeriesPaneView.destroy` runs when the series leaves the
    /// chart). Called after any `remove_series`, mirroring `detach_orphaned_series_primitives`.
    pub(super) fn drop_orphaned_custom_series(&mut self) {
        if self.custom_series.is_empty() {
            return;
        }
        let entries = std::mem::take(&mut self.custom_series);
        for entry in entries {
            let live = self
                .series
                .iter()
                .any(|s| s.id == entry.series as SeriesId && !s.removed);
            if live {
                self.custom_series.push(entry);
                continue;
            }
            fire_custom_destroy(&entry);
        }
    }

    /// Walk one custom series' aligned rows once for a frame: skip whitespace items (the
    /// plugin's `is_whitespace` or the contract default), union the finite
    /// `price_value_builder` values over the visible range `[from, to]` (merged indices), and
    /// collect the first/last value records the engine's chrome consumes.
    fn walk_custom_series(
        &self,
        index: usize,
        view: &js_sys::Object,
        from: i64,
        to: i64,
    ) -> CustomSeriesWalk {
        let entry = &self.custom_series[index];
        let mut walk = CustomSeriesWalk::default();
        let Some(builder) = custom_hook(view, "price_value_builder") else {
            return walk;
        };
        let whitespace_check = custom_hook(view, "is_whitespace");
        let Some(series) = self
            .series
            .iter()
            .find(|series| series.id == entry.series as SeriesId && !series.removed)
        else {
            return walk;
        };
        let plot = self.engine.data_layer().plot(entry.series as SeriesId);
        for (row, item) in entry.items.iter().enumerate() {
            let Some(merged) = plot.index_at(row) else {
                break;
            };
            if custom_is_whitespace(view, whitespace_check.as_ref(), item) {
                continue;
            }
            let (values, current) = custom_price_values(&builder, view, item);
            let record = current.map(|value| CustomSeriesLastValue {
                value,
                color: custom_bar_color(item, series),
                time: entry.times[row],
            });
            if record.is_some() {
                walk.last = record;
            }
            if merged < from {
                continue;
            }
            if walk.first_value.is_none() {
                walk.first_value = current;
            }
            if merged > to {
                continue;
            }
            if record.is_some() {
                walk.last_visible = record;
            }
            for value in values {
                walk.min = Some(walk.min.map_or(value, |min: f64| min.min(value)));
                walk.max = Some(walk.max.map_or(value, |max: f64| max.max(value)));
            }
        }
        walk
    }

    /// Refresh each custom series' frame values (first/last values for the scale anchor and
    /// the built-in chrome) and record its autoscale contribution for this frame (Phase C-c;
    /// the C-b `add_autoscale_contribution` path, gated on the owning series' visibility and
    /// pane like any primitive's). Runs at the top of `render`, right after the series-
    /// primitive collection, so the axis-width negotiation, the axis frame, and the pane
    /// frame all see the merged ranges.
    pub(super) fn collect_custom_series_autoscale(&mut self) {
        if self.custom_series.is_empty() {
            return;
        }
        let Some((from, to)) = self.engine.visible_range() else {
            return;
        };
        for index in 0..self.custom_series.len() {
            let (series_id, view) = {
                let entry = &self.custom_series[index];
                (entry.series, entry.view.clone())
            };
            let Some(series) = self
                .series
                .iter()
                .find(|s| s.id == series_id as SeriesId && !s.removed)
            else {
                continue;
            };
            let (visible, pane, target) =
                (series.visible, series.pane_index, series.price_scale_target);
            // A hidden or pane-less series contributes nothing and its frame values clear (a
            // hidden series paints no last-value chrome either — the reference's visibility gate).
            if !visible || pane >= self.panes.len() {
                self.engine.set_custom_frame_values(
                    series_id as SeriesId,
                    CustomSeriesFrameValues::default(),
                );
                continue;
            }
            let walk = self.walk_custom_series(index, &view, from, to);
            self.engine.set_custom_frame_values(
                series_id as SeriesId,
                CustomSeriesFrameValues {
                    first_value: walk.first_value,
                    last: walk.last,
                    last_visible: walk.last_visible,
                },
            );
            let Some((min, max)) = walk.min.zip(walk.max) else {
                continue;
            };
            self.engine
                .add_autoscale_contribution(PrimitiveAutoscaleContribution {
                    series: series_id as SeriesId,
                    pane,
                    target,
                    min,
                    max,
                });
        }
    }

    /// Run every custom series' `render(ctx)` for this frame (Phase C-c): the visible
    /// non-whitespace items with their absolute bitmap-px bar-center x, `price_to_y` bound to
    /// the series' own scale (the C-b converter), and the same command recorders as the
    /// primitive passes. The decoded prims splice into the pane's `main` layer at the series'
    /// paint-order mark, so plugin output z-orders between the built-in series and is
    /// pixel-identical across both backends. Plugin JS failures are contained to the
    /// offending series — the chart frame itself always completes.
    pub(super) fn run_custom_series(&mut self) {
        if self.custom_series.is_empty() {
            return;
        }
        let Some((from, to)) = self.engine.visible_range() else {
            return;
        };
        let snapshot = self.primitive_scale_snapshot();
        // Resolve each live custom series to its pane paint mark, in chart paint order, then
        // render pane-by-pane in mark order so splicing is a single forward pass per pane
        // (stable sort keeps the chart's paint order between equal marks).
        struct PendingCustom {
            pane: usize,
            mark: usize,
            entry_index: usize,
            target: PriceScaleTarget,
            base: Option<f64>,
        }
        let mut pending: Vec<PendingCustom> = Vec::new();
        for id in self.engine.series_order().to_vec() {
            let Some(index) = self.custom_series.iter().position(|e| e.series == id) else {
                continue;
            };
            let Some(series) = self
                .series
                .iter()
                .find(|series| series.id == id && !series.removed)
            else {
                continue;
            };
            if !series.visible {
                continue;
            }
            let pane = series.pane_index;
            let Some(frame_pane) = self.frame.panes.get(pane) else {
                continue;
            };
            let Some(mark) = frame_pane
                .series_paint_marks
                .iter()
                .find(|(sid, _)| *sid == id)
                .map(|(_, mark)| *mark)
            else {
                continue;
            };
            let target = series.price_scale_target;
            // The custom first value anchors percentage/indexed modes (its geometry's own
            // base value, like the C-b series path).
            let base = series
                .custom_frame
                .first_value
                .filter(|value| value.is_finite());
            pending.push(PendingCustom {
                pane,
                mark,
                entry_index: index,
                target,
                base,
            });
        }
        pending.sort_by_key(|p| (p.pane, p.mark));
        let mut shifts = vec![0usize; self.frame.panes.len()];
        for p in pending {
            let view = self.custom_series[p.entry_index].view.clone();
            let Some(renderer) = custom_hook(&view, "render") else {
                continue;
            };
            let Some(pane_snap) = snapshot.panes.get(p.pane) else {
                continue;
            };
            let Some(scissor) = self.frame.panes.get(p.pane).map(|frame| frame.scissor) else {
                continue;
            };
            let converters =
                PrimitiveConverters::for_series(&snapshot, pane_snap, p.target, p.base, scissor[0]);
            let items = js_sys::Array::new();
            {
                let entry = &self.custom_series[p.entry_index];
                let plot = self.engine.data_layer().plot(entry.series as SeriesId);
                let whitespace_check = custom_hook(&view, "is_whitespace");
                for (row, item) in entry.items.iter().enumerate() {
                    let Some(merged) = plot.index_at(row) else {
                        break;
                    };
                    if merged < from || merged > to {
                        continue;
                    }
                    if custom_is_whitespace(&view, whitespace_check.as_ref(), item) {
                        continue;
                    }
                    // Bar-center x in absolute bitmap px — the draw context's space, matching
                    // the primitive converters' math exactly (time_to_x without the no-snap).
                    let x = snapshot.time_scale.index_to_coordinate(merged) * snapshot.hpr
                        + f64::from(scissor[0]);
                    let point = js_sys::Object::new();
                    let _ = js_sys::Reflect::set(&point, &"x".into(), &JsValue::from_f64(x));
                    let _ = js_sys::Reflect::set(&point, &"item".into(), item);
                    items.push(&point);
                }
            }
            let commands = js_sys::Array::new();
            let fields = js_sys::Object::new();
            let [pane_left_px, pane_top_px, pane_w_px, pane_h_px] = scissor;
            for (key, value) in [
                ("pane_width", f64::from(pane_w_px)),
                ("pane_height", f64::from(pane_h_px)),
                ("pane_left", f64::from(pane_left_px)),
                ("pane_top", f64::from(pane_top_px)),
                ("dpr", self.dpr),
            ] {
                let _ = js_sys::Reflect::set(&fields, &key.into(), &value.into());
            }
            let ctx = build_custom_series_context(
                &commands,
                &fields,
                &items,
                converters.price_to_y.as_ref().unchecked_ref(),
                self.engine.time_scale.bar_spacing(),
            );
            if let Err(error) = renderer.call1(&view, &ctx) {
                web_sys::console::warn_1(
                    &format!("aeris_charts: custom series `render` threw — {error:?}").into(),
                );
                continue;
            }
            // The draw context is single-use (its converter closure is dropped after the
            // synchronous render call), so a plugin stashing `ctx` cannot call into revoked fns.
            drop(ctx);
            let json = match js_sys::JSON::stringify(commands.as_ref()) {
                Ok(json) => String::from(json),
                Err(_) => continue,
            };
            let text_defaults = self.text_defaults();
            let frame_pane = &mut self.frame.panes[p.pane];
            let decoded = decode_commands(
                &json,
                &mut frame_pane.points,
                &text_defaults,
                pane_clip(scissor),
            );
            if !decoded.warnings.is_empty() {
                web_sys::console::warn_1(
                    &format!(
                        "aeris_charts: custom series skipped {} command(s) — {}",
                        decoded.warnings.len(),
                        decoded.warnings.join("; ")
                    )
                    .into(),
                );
            }
            // Splice at the series' paint mark (shifted by prims already spliced into this
            // pane), preserving the chart z-order between built-in series.
            let insert_at = (p.mark + shifts[p.pane]).min(frame_pane.main.len());
            shifts[p.pane] += decoded.prims.len();
            frame_pane.main.splice(insert_at..insert_at, decoded.prims);
        }
    }
}

fn fire_custom_destroy(entry: &CustomSeriesEntry) {
    if let Some(destroy) = custom_hook(&entry.view, "destroy") {
        if let Err(error) = destroy.call0(&entry.view) {
            web_sys::console::warn_1(
                &format!("aeris_charts: custom series `destroy` hook threw — {error:?}").into(),
            );
        }
    }
}
