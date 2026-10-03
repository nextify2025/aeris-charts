//! Pane-primitive draw pass (plugin platform Phase C-a, design doc §4.2 Option A).
//!
//! During [`ChartInner::render`], after the engine frame and axis frame are built and before
//! backend execution, every registered primitive gets its `pane_views()` called. Each view's
//! `renderer(ctx)` receives a host-built draw context whose converters (`price_to_y`,
//! `time_to_x`, `logical_to_x`) resolve against the settled post-layout scales — the same math
//! the engine's own geometry uses — and whose draw functions record plain-JS commands into a
//! per-view array. The array is JSON-stringified (one marshalling pass per view per frame) and
//! decoded into `Prim`s by the pure [`crate::prim_decode`] module, then appended to the pane's
//! `under`/`main`/`top` layer by the view's `z_order`. Both backends consume the same prims,
//! so plugin output is pixel-identical across WebGPU and Canvas2D by construction.
//!
//! Coordinate space: absolute bitmap px of the whole chart (x from the left edge, y from the
//! top — pane offsets included, matching the pane's scissor rect). This makes the converters
//! agree with engine geometry exactly: the engine rounds `(coord * ratio)` per axis and adds
//! the integer pane offset, and `round(a + k) == round(a) + k` for integer `k`.
//!
//! Reference library source: model/ipane-primitive.ts (`IPanePrimitiveBase`) and api/pane-api.ts
//! (`PaneApi.attachPrimitive`). Divergence: reference hands renderers a raw
//! `CanvasRenderingTarget2D`; Aeris renderers record backend-neutral commands instead (the
//! design's locked A-first hybrid). reference pane primitives carry no axis views (those exist only
//! on `ISeriesPrimitiveBase`); Aeris accepts `price_axis_views`/`time_axis_views` on pane
//! primitives as a deliberate extension, painted as boxed labels through the same `AxisLabel`
//! path the engine's price-line/crosshair labels use.

use super::inner_render::measure_text_ctx;
use super::*;
use crate::prim_decode::{decode_commands, pane_clip};
use aeris_charts_core::model::plot_list::PlotValueIndex;
use aeris_charts_core::scale::price_scale_core::PriceScaleCore;
use aeris_charts_core::style::{DARK_BORDER_RGB, DEFAULT_AXIS_TEXT_RGB};
use aeris_charts_engine::{ChartHover, HostPrimitiveHit, HostPrimitiveLayer};
use aeris_charts_render::draw_list::TextAlign;

/// Fallback background for a primitive axis label with no color given — the reference crosshair
/// label default (frame/mod.rs `PRIMITIVE_LABEL_BG`).
const PRIMITIVE_LABEL_BG: Color =
    Color::rgb(DARK_BORDER_RGB.0, DARK_BORDER_RGB.1, DARK_BORDER_RGB.2);

/// Parse the canvas-normalized subset every frame executor can represent. Unsupported font
/// variants are rejected instead of painting one font in the browser and another in native.
fn parse_primitive_font(spec: &str) -> Option<(f64, String, u16, bool)> {
    let (prefix, family) = spec.split_once("px ")?;
    let family = family.trim();
    if family.is_empty() {
        return None;
    }
    let mut tokens = prefix.split_whitespace().collect::<Vec<_>>();
    let size = tokens.pop()?.parse::<f64>().ok()?;
    if !size.is_finite() || size <= 0.0 {
        return None;
    }
    let mut weight = 400;
    let mut italic = false;
    for token in tokens {
        match token {
            "normal" => {}
            "italic" | "oblique" => italic = true,
            "bold" => weight = 700,
            _ => {
                let parsed = token.parse::<u16>().ok()?;
                if !(100..=900).contains(&parsed) {
                    return None;
                }
                weight = parsed;
            }
        }
    }
    Some((size, family.to_string(), weight, italic))
}

#[wasm_bindgen(inline_js = r#"
// One draw context per renderer call. The converter fns arrive ready-made from Rust (they close
// over the settled scale state); the draw fns are plain object recorders — every draw call is one
// JSON-marshallable command. Coordinates are absolute bitmap px (see the module docs).
export function build_primitive_draw_context(commands, fields, price_to_y, time_to_x, logical_to_x) {
    return {
        pane_width: fields.pane_width,
        pane_height: fields.pane_height,
        pane_left: fields.pane_left,
        pane_top: fields.pane_top,
        dpr: fields.dpr,
        price_to_y, time_to_x, logical_to_x,
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
    fn build_primitive_draw_context(
        commands: &js_sys::Array,
        fields: &js_sys::Object,
        price_to_y: &js_sys::Function,
        time_to_x: &js_sys::Function,
        logical_to_x: &js_sys::Function,
    ) -> js_sys::Object;
}

/// Settled scale state cloned out of the engine once per render, so the converter closures the
/// draw context hands to JS are `'static` (no borrows of the chart while JS runs).
pub(super) struct PrimitiveScaleSnapshot {
    pub(super) time_scale: aeris_charts_core::scale::time_scale_core::TimeScaleCore,
    /// Merged time points for `time_to_x` lookups (UTC seconds, ascending).
    times: Vec<i64>,
    /// Per-pane right/left scales plus their percentage-mode base values (reference anchors those to
    /// the pane's primary series' first visible value, like the tick formatter's source).
    pub(super) panes: Vec<PaneScaleSnapshot>,
    /// Horizontal/vertical bitmap ratios, recomputed with the frame build's exact formulas
    /// (aeris_charts_engine frame/mod.rs `build_frame_into`) so converter output matches engine geometry.
    pub(super) hpr: f64,
    vpr: f64,
}

pub(super) struct PaneScaleSnapshot {
    right: PriceScaleCore,
    left: PriceScaleCore,
    /// The pane's overlay scale (Phase C-b): series primitives bound to an overlay series
    /// resolve `price_to_y` against it. Unused by the pane-primitive pass.
    overlay: PriceScaleCore,
    named: Vec<(PriceScaleId, PriceScaleCore)>,
    base_right: Option<f64>,
    base_left: Option<f64>,
}

impl ChartInner {
    /// The pane's primary series' base value for percentage/indexed scale modes — the first
    /// visible, non-overlay series bound to the scale (mirrors `format_tick_value`'s primary
    /// rule and the engine's `series_base_value`). `None` when the pane has no such series.
    fn pane_scale_base_value(
        &self,
        pane_index: usize,
        target: PriceScaleTarget,
        from: i64,
    ) -> Option<f64> {
        let series = self
            .series
            .iter()
            .find(|s| s.visible && s.pane_index == pane_index && s.price_scale_target == target)?;
        let plot = self.engine.data_layer().plot(series.id);
        let row = plot.first_non_whitespace_row(from)?;
        let value = plot.value_at(row, PlotValueIndex::Close);
        // An explicit scale base (price-scale option `base_value`) replaces the first value.
        let explicit = self
            .engine
            .price_scale_for(pane_index, target)
            .and_then(|scale| scale.options().base_value);
        explicit.or_else(|| value.is_finite().then_some(value))
    }

    /// A series' own base value for percentage/indexed scale modes — its first visible close
    /// (the anchor the series' own geometry uses; mirrors the engine's `series_base_value`).
    fn series_scale_base_value(&self, id: SeriesId, from: i64) -> Option<f64> {
        let plot = self.engine.data_layer().plot(id);
        let row = plot.first_non_whitespace_row(from)?;
        let value = plot.value_at(row, PlotValueIndex::Close);
        // An explicit scale base (price-scale option `base_value`) replaces the first value.
        let explicit = self
            .engine
            .series_price_scale(id)
            .and_then(|(pane, target)| self.engine.price_scale_for(pane, target))
            .and_then(|scale| scale.options().base_value);
        explicit.or_else(|| value.is_finite().then_some(value))
    }

    pub(super) fn primitive_scale_snapshot(&self) -> PrimitiveScaleSnapshot {
        // Mirror the frame build's ratio math (frame/mod.rs): the per-pane bitmap/media ratio
        // can differ slightly from DPR when a fractional-DPR pane dimension rounds.
        let nominal_dpr = self.dpr.max(0.01);
        let hpr = (self.pane_w * nominal_dpr).round().max(1.0) / self.pane_w.max(1.0);
        let vpr = (self.pane_h * nominal_dpr).round().max(1.0) / self.pane_h.max(1.0);
        let visible_from = self.engine.visible_range().map(|(from, _)| from);
        let panes = self
            .panes
            .iter()
            .enumerate()
            .map(|(pi, pane)| {
                let named = self
                    .engine
                    .price_scales(pi)
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|info| {
                        let target = self.engine.price_scale_target_for_id(pi, &info.id)?;
                        let PriceScaleTarget::Named(id) = target else {
                            return None;
                        };
                        Some((id, self.engine.price_scale_for(pi, target)?.clone()))
                    })
                    .collect();
                PaneScaleSnapshot {
                    right: pane.price_scale.clone(),
                    left: pane.left_scale.clone(),
                    overlay: pane.overlay_scale.clone(),
                    named,
                    base_right: visible_from.and_then(|from| {
                        self.pane_scale_base_value(pi, PriceScaleTarget::Right, from)
                    }),
                    base_left: visible_from.and_then(|from| {
                        self.pane_scale_base_value(pi, PriceScaleTarget::Left, from)
                    }),
                }
            })
            .collect();
        PrimitiveScaleSnapshot {
            time_scale: self.engine.time_scale.clone(),
            times: self.engine.data_layer().merged_times().to_vec(),
            panes,
            hpr,
            vpr,
        }
    }

    /// Run every attached pane primitive for this frame: update views, record draw commands
    /// into the pane layers by z-order, and append boxed axis labels. Plugin JS failures are
    /// contained to the offending view — the chart frame itself always completes.
    pub(super) fn run_pane_primitives(&mut self) {
        if self.primitives.is_empty() {
            return;
        }
        let snapshot = self.primitive_scale_snapshot();
        for index in 0..self.primitives.len() {
            let (pane, obj) = {
                let entry = &self.primitives[index];
                (entry.pane as usize, entry.obj.clone())
            };
            // A stale pane index (the pane was removed after attach) draws nowhere, exactly
            // like a pane-less series (reference `removePane` orphaning).
            let Some(pane_snap) = snapshot.panes.get(pane) else {
                continue;
            };
            let Some(scissor) = self.frame.panes.get(pane).map(|frame| frame.scissor) else {
                continue;
            };

            if let Ok(hook) = js_sys::Reflect::get(&obj, &"update_all_views".into()) {
                if let Ok(hook) = hook.dyn_into::<js_sys::Function>() {
                    if let Err(error) = hook.call0(&obj) {
                        web_sys::console::warn_1(
                            &format!(
                                "aeris_charts: pane primitive `update_all_views` threw — {error:?}"
                            )
                            .into(),
                        );
                    }
                }
            }

            let views = js_sys::Reflect::get(&obj, &"pane_views".into())
                .ok()
                .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
                .and_then(|f| f.call0(&obj).ok());
            if let Some(views) = views {
                let views = js_sys::Array::from(&views);
                // One set of converter closures per primitive; each view shares them (they
                // depend only on the pane's settled scales, not on the view).
                let converters = PrimitiveConverters::new(&snapshot, pane_snap, scissor[0]);
                for view in views.iter() {
                    self.run_primitive_view(&view, &converters, pane, scissor);
                }
            }

            self.append_primitive_axis_labels(&obj, pane);
            self.append_primitive_text_views(&obj, pane, scissor, snapshot.hpr, snapshot.vpr);
        }
    }

    /// Hover hit testing (plugin platform Phase C-d; reference pane-hit-test.ts `hitTestPane`) at
    /// pane-relative media px `(x_css, y_css)` (x from the pane's left edge, y from the chart's
    /// top — the crosshair's space). Primitives are JS objects, so their best hit is gathered
    /// here; the engine arbitrates it against drawings and series
    /// (`ChartEngine::resolve_pointer_hover`) and refreshes hover promotion and the cursor.
    /// Returns JSON `{"series_id","object_id","cursor","general_hit"}`: a series-primitive hit
    /// reports its owning series too (the reference's source IS the series), a pane-primitive hit
    /// reports no series, and only primitive and drawing hits carry a cursor.
    pub(super) fn hover_at(&mut self, x_css: f64, y_css: f64) -> String {
        let primitive = self.primitive_hit_at(x_css, y_css);
        let hover = self.engine.resolve_pointer_hover(
            x_css,
            y_css,
            primitive.as_ref().map(PrimitiveHit::engine_hit),
        );
        self.hover_json(hover, primitive.as_ref())
    }

    /// The input controller's current hover as `hover_at` JSON. The controller already resolved
    /// every built-in candidate at its hover point, so arbitration re-runs only when a primitive
    /// is hit there.
    pub(super) fn controller_hover_json(&mut self) -> String {
        let Some((x, y)) = self.engine.crosshair else {
            return self.hover_json(ChartHover::None, None);
        };
        let primitive = if self.primitives.is_empty() && self.series_primitives.is_empty() {
            None
        } else {
            self.primitive_hit_at(x, y)
        };
        let hover = match &primitive {
            Some(hit) => self
                .engine
                .resolve_pointer_hover(x, y, Some(hit.engine_hit())),
            None => self.engine.input_hover(),
        };
        self.hover_json(hover, primitive.as_ref())
    }

    fn hover_json(&self, hover: ChartHover, primitive: Option<&PrimitiveHit>) -> String {
        let (series_id, object_id, cursor, general_hit) = match hover {
            ChartHover::None => (None, None, None, None),
            ChartHover::HostPrimitive(hit) => (
                hit.series,
                primitive.and_then(|primitive| primitive.external_id.clone()),
                primitive.and_then(|primitive| primitive.cursor.clone()),
                None,
            ),
            ChartHover::Drawing { id, cursor } => (
                None,
                Some(format!("drawing:{id}")),
                Some(cursor.to_string()),
                None,
            ),
            ChartHover::Series(id) => (Some(id), None, None, None),
            ChartHover::VolumeProfile(id) => {
                (None, Some(format!("volume_profile:{id}")), None, None)
            }
            ChartHover::General => (None, None, None, self.engine.general_hovered_hit()),
        };
        serde_json::json!({
            "series_id": series_id,
            "object_id": object_id,
            "cursor": cursor,
            "general_hit": general_hit.as_ref().map(super::general_charts::general_hit_value),
        })
        .to_string()
    }

    /// The best primitive hit at a hover point: the highest z-order (`top` > `normal` >
    /// `bottom`, first-come within a layer — the reference's priority/distance fields are not
    /// modeled), series in stable paint order topmost first, then the pane's own primitives (the
    /// pane is the lowest source — the reference's `[pane, ...orderedSources()].reverse()`).
    /// Primitives receive the same absolute bitmap-px coordinates their draw context uses.
    /// Hidden series' primitives are skipped (a hidden series paints nothing, so it captures no
    /// hover — reference gates the built-in hit on visibility but overlooks its primitive path).
    fn primitive_hit_at(&self, x_css: f64, y_css: f64) -> Option<PrimitiveHit> {
        let pane = self.engine.pane_at_y(y_css)?;
        let nominal_dpr = self.dpr.max(0.01);
        let hpr = (self.pane_w * nominal_dpr).round().max(1.0) / self.pane_w.max(1.0);
        let vpr = (self.pane_h * nominal_dpr).round().max(1.0) / self.pane_h.max(1.0);
        let pane_left_px = (self.pane_left * nominal_dpr).round().max(0.0);
        let (bx, by) = (x_css * hpr + pane_left_px, y_css * vpr);
        let mut best: Option<PrimitiveHit> = None;
        let mut offer = |hit: PrimitiveHit| {
            if best
                .as_ref()
                .is_none_or(|current| hit.layer() > current.layer())
            {
                best = Some(hit);
            }
        };
        for &id in self.engine.series_order().iter().rev() {
            let on_hit_pane = self
                .series
                .iter()
                .any(|s| s.id == id && !s.removed && s.visible && s.pane_index == pane);
            if !on_hit_pane {
                continue;
            }
            for entry in self.series_primitives.iter().filter(|e| e.series == id) {
                if let Some(hit) = call_hit_test(&entry.obj, bx, by) {
                    offer(hit.for_series(id));
                }
            }
        }
        for entry in self.primitives.iter().filter(|e| e.pane as usize == pane) {
            if let Some(hit) = call_hit_test(&entry.obj, bx, by) {
                offer(hit.for_pane());
            }
        }
        best
    }

    /// Refresh the engine's per-frame autoscale store from every series primitive's
    /// `autoscale_info(from, to)` hook (Phase C-b; reference `ISeriesPrimitiveBase.autoscaleInfo`).
    /// Runs at the top of `render`, before any layout/autoscale pass consumes the scale
    /// ranges, so axis-width negotiation, the axis frame, and the pane frame all see the
    /// merged ranges. `from`/`to` are the visible logical range, as in reference. The engine gates
    /// each contribution on the owning series' visibility and data at merge time; a pane-less
    /// series' contribution is dropped here (it scales nowhere, reference `removePane` orphaning).
    pub(super) fn collect_series_primitive_autoscale(&mut self) {
        self.engine.clear_autoscale_contributions();
        if self.series_primitives.is_empty() {
            return;
        }
        let Some((from, to)) = self.engine.visible_range() else {
            return;
        };
        for index in 0..self.series_primitives.len() {
            let (series_id, obj) = {
                let entry = &self.series_primitives[index];
                (entry.series, entry.obj.clone())
            };
            let hook = js_sys::Reflect::get(&obj, &"autoscale_info".into())
                .ok()
                .and_then(|f| f.dyn_into::<js_sys::Function>().ok());
            let Some(hook) = hook else {
                continue;
            };
            let result = match hook.call2(
                &obj,
                &JsValue::from_f64(from as f64),
                &JsValue::from_f64(to as f64),
            ) {
                Ok(result) => result,
                Err(error) => {
                    web_sys::console::warn_1(
                        &format!(
                            "aeris_charts: series primitive `autoscale_info` threw — {error:?}"
                        )
                        .into(),
                    );
                    continue;
                }
            };
            // reference: a `null` autoscale info carries no range. Non-finite bounds are rejected
            // by the engine (`add_autoscale_contribution`).
            if result.is_null() || result.is_undefined() {
                continue;
            }
            let min = js_sys::Reflect::get(&result, &"min".into())
                .ok()
                .and_then(|v| v.as_f64());
            let max = js_sys::Reflect::get(&result, &"max".into())
                .ok()
                .and_then(|v| v.as_f64());
            let (Some(min), Some(max)) = (min, max) else {
                continue;
            };
            let Some((pane, target)) = self.engine.series_price_scale(series_id as SeriesId) else {
                continue;
            };
            if pane >= self.panes.len() {
                continue;
            }
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

    /// Run every attached series primitive for this frame (Phase C-b): the pane-primitive
    /// pass's mechanics with the binding resolved through the owning series each frame (so
    /// views follow pane moves and scale rebinding), `price_to_y(price)` bound to the series'
    /// own scale, and price-axis labels routed to that scale's strip. A hidden or pane-less
    /// series paints nothing — reference empties a series' views while it is not visible
    /// (series-pane-view-base.ts) and scales a pane-less source nowhere.
    pub(super) fn run_series_primitives(&mut self) {
        if self.series_primitives.is_empty() {
            return;
        }
        let snapshot = self.primitive_scale_snapshot();
        let visible_from = self.engine.visible_range().map(|(from, _)| from);
        for index in 0..self.series_primitives.len() {
            let (series_id, obj) = {
                let entry = &self.series_primitives[index];
                (entry.series, entry.obj.clone())
            };
            let Some(series) = self
                .series
                .iter()
                .find(|s| s.id == series_id as SeriesId && !s.removed)
            else {
                continue;
            };
            if !series.visible {
                continue;
            }
            let target = series.price_scale_target;
            let pane = series.pane_index;
            let Some(pane_snap) = snapshot.panes.get(pane) else {
                continue;
            };
            let Some(scissor) = self.frame.panes.get(pane).map(|frame| frame.scissor) else {
                continue;
            };
            // The series' OWN base value anchors percentage/indexed modes (as its geometry
            // does), not the pane's primary series'.
            let base = visible_from
                .and_then(|from| self.series_scale_base_value(series_id as SeriesId, from));

            if let Ok(hook) = js_sys::Reflect::get(&obj, &"update_all_views".into()) {
                if let Ok(hook) = hook.dyn_into::<js_sys::Function>() {
                    if let Err(error) = hook.call0(&obj) {
                        web_sys::console::warn_1(
                            &format!(
                                "aeris_charts: series primitive `update_all_views` threw — {error:?}"
                            )
                            .into(),
                        );
                    }
                }
            }

            let views = js_sys::Reflect::get(&obj, &"pane_views".into())
                .ok()
                .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
                .and_then(|f| f.call0(&obj).ok());
            if let Some(views) = views {
                let views = js_sys::Array::from(&views);
                // One set of converter closures per primitive; each view shares them.
                let converters =
                    PrimitiveConverters::for_series(&snapshot, pane_snap, target, base, scissor[0]);
                for view in views.iter() {
                    self.run_primitive_view(&view, &converters, pane, scissor);
                }
            }

            self.append_series_primitive_axis_labels(&obj, pane, target, base);
            self.append_primitive_text_views(&obj, pane, scissor, snapshot.hpr, snapshot.vpr);
        }
    }

    /// The text defaults folded into decoded `text` commands (prim_decode): the layout font
    /// family and text color, and the layout font size scaled to the pane's bitmap px (the
    /// draw context's coordinate space) — the same sources the axis labels draw with.
    pub(super) fn text_defaults(&self) -> crate::prim_decode::TextDefaults {
        let layout = self.opts().layout;
        crate::prim_decode::TextDefaults {
            family: layout.font_family,
            size: (layout.font_size * self.dpr) as f32,
            color: Color::parse_css(&layout.text_color).unwrap_or(Color::rgb(
                DEFAULT_AXIS_TEXT_RGB.0,
                DEFAULT_AXIS_TEXT_RGB.1,
                DEFAULT_AXIS_TEXT_RGB.2,
            )),
        }
    }

    /// Record one pane view's commands and append the decoded prims to the pane's layer for
    /// the view's `z_order` (reference `PrimitivePaneViewZOrder`: bottom → `under`, normal → `main`,
    /// top → `top`).
    fn run_primitive_view(
        &mut self,
        view: &JsValue,
        converters: &PrimitiveConverters,
        pane: usize,
        scissor: [u32; 4],
    ) {
        let z_order = js_sys::Reflect::get(view, &"z_order".into())
            .ok()
            .and_then(|z| z.as_string())
            .unwrap_or_else(|| "normal".to_string());
        let renderer = js_sys::Reflect::get(view, &"renderer".into())
            .ok()
            .and_then(|f| f.dyn_into::<js_sys::Function>().ok());
        let Some(renderer) = renderer else {
            return;
        };
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
        let ctx = build_primitive_draw_context(
            &commands,
            &fields,
            converters.price_to_y.as_ref().unchecked_ref(),
            converters.time_to_x.as_ref().unchecked_ref(),
            converters.logical_to_x.as_ref().unchecked_ref(),
        );
        if let Err(error) = renderer.call1(view, &ctx) {
            web_sys::console::warn_1(
                &format!("aeris_charts: pane primitive renderer threw — {error:?}").into(),
            );
            return;
        }
        // The draw context is single-use: its converter closures are dropped after the
        // synchronous renderer call, so a plugin stashing `ctx` cannot call into revoked fns.
        drop(ctx);
        let json = match js_sys::JSON::stringify(commands.as_ref()) {
            Ok(json) => String::from(json),
            Err(_) => return,
        };
        let text_defaults = self.text_defaults();
        let Some(frame_pane) = self.frame.panes.get_mut(pane) else {
            return;
        };
        let decoded = decode_commands(
            &json,
            &mut frame_pane.points,
            &text_defaults,
            pane_clip(scissor),
        );
        if !decoded.warnings.is_empty() {
            web_sys::console::warn_1(
                &format!(
                    "aeris_charts: pane primitive raised {} command warning(s) — {}",
                    decoded.warnings.len(),
                    decoded.warnings.join("; ")
                )
                .into(),
            );
        }
        let layer = match z_order.as_str() {
            "bottom" => &mut frame_pane.under,
            "top" => &mut frame_pane.top_prims,
            _ => &mut frame_pane.main,
        };
        layer.extend(decoded.prims);
    }

    /// Lower a primitive's `text_views(info)` draws into the owning pane's ordered top layer.
    /// The pane scissor keeps text out of shared axis chrome on every backend.
    /// The hook receives the pane's bitmap dimensions, exact pixel ratios, and layout font — state
    /// a plugin cannot read mid-render otherwise (chart APIs are off-limits from render hooks).
    /// Descriptors carry absolute bitmap-px coordinates. Shared by pane and series passes.
    fn append_primitive_text_views(
        &mut self,
        obj: &js_sys::Object,
        pane: usize,
        scissor: [u32; 4],
        hpr: f64,
        vpr: f64,
    ) {
        let layout = self.opts().layout;
        let info = js_sys::Object::new();
        let [pane_left_px, pane_top_px, pane_w_px, pane_h_px] = scissor;
        for (key, value) in [
            ("pane_width", f64::from(pane_w_px)),
            ("pane_height", f64::from(pane_h_px)),
            ("pane_left", f64::from(pane_left_px)),
            ("pane_top", f64::from(pane_top_px)),
            ("dpr", self.dpr),
            ("hpr", hpr),
            ("vpr", vpr),
            ("font_size", layout.font_size),
        ] {
            let _ = js_sys::Reflect::set(&info, &key.into(), &value.into());
        }
        let _ = js_sys::Reflect::set(
            &info,
            &"font_family".into(),
            &layout.font_family.clone().into(),
        );
        let Some(descriptors) = call_view_array_with(obj, "text_views", Some(&info.into())) else {
            return;
        };
        for descriptor in descriptors.iter() {
            let get = |key: &str| js_sys::Reflect::get(&descriptor, &key.into()).ok();
            let Some(text) = get("text")
                .and_then(|t| t.as_string())
                .filter(|t| !t.is_empty())
            else {
                continue;
            };
            let finite = |key: &str| get(key).and_then(|v| v.as_f64()).filter(|v| v.is_finite());
            let (Some(x), Some(y)) = (finite("x"), finite("y")) else {
                continue;
            };
            let non_empty_string = |key: &str| {
                get(key)
                    .and_then(|v| v.as_string())
                    .filter(|s| !s.is_empty())
            };
            // An explicit `font` shorthand wins; otherwise compose from size/family/bold with
            // the layout font as the default, matching the engine's own axis labels.
            let font = non_empty_string("font").unwrap_or_else(|| {
                let size = finite("size")
                    .filter(|s| *s > 0.0)
                    .unwrap_or(layout.font_size);
                let family =
                    non_empty_string("font_family").unwrap_or_else(|| layout.font_family.clone());
                let bold = get("bold").and_then(|b| b.as_bool()).unwrap_or(false);
                format!("{}{}px {}", if bold { "bold " } else { "" }, size, family)
            });
            let keyword = |key: &str, allowed: &[&str], fallback: &str| {
                non_empty_string(key)
                    .filter(|v| allowed.contains(&v.as_str()))
                    .unwrap_or_else(|| fallback.to_string())
            };
            let align = match keyword("align", &["left", "center", "right"], "left").as_str() {
                "center" => TextAlign::Center,
                "right" => TextAlign::Right,
                _ => TextAlign::Left,
            };
            let baseline = keyword(
                "baseline",
                &["top", "middle", "bottom", "alphabetic"],
                "alphabetic",
            );
            let ctx = &self.axis_ctx;
            ctx.save();
            ctx.set_font("10px sans-serif");
            ctx.set_font(&font);
            let normalized_font = js_sys::Reflect::get(ctx.as_ref(), &"font".into())
                .ok()
                .and_then(|value| value.as_string());
            let parsed = normalized_font.as_deref().and_then(parse_primitive_font);
            ctx.set_text_baseline(&baseline);
            let target_ascent = ctx
                .measure_text(&text)
                .ok()
                .map(|m| m.actual_bounding_box_ascent());
            ctx.set_text_baseline("middle");
            let middle_ascent = ctx
                .measure_text(&text)
                .ok()
                .map(|m| m.actual_bounding_box_ascent());
            ctx.set_fill_style_str("#000000");
            ctx.set_fill_style_str(
                &non_empty_string("color").unwrap_or_else(|| layout.text_color.clone()),
            );
            let color = js_sys::Reflect::get(ctx.as_ref(), &"fillStyle".into())
                .ok()
                .and_then(|value| value.as_string())
                .and_then(|value| Color::parse_css(&value));
            ctx.restore();
            let (
                Some((size, family, weight, italic)),
                Some(color),
                Some(target_ascent),
                Some(middle_ascent),
            ) = (parsed, color, target_ascent, middle_ascent)
            else {
                web_sys::console::warn_1(
                    &"aeris_charts: unsupported primitive text font or color".into(),
                );
                continue;
            };
            let Some(frame_pane) = self.frame.panes.get_mut(pane) else {
                continue;
            };
            frame_pane.top_prims.push(Prim::Text {
                x: x as f32,
                y: (y + (middle_ascent - target_ascent) * vpr) as f32,
                text,
                color,
                size: (size * vpr) as f32,
                family,
                align,
                weight,
                italic,
            });
        }
    }

    /// Append the primitive's `price_axis_views`/`time_axis_views` as boxed `AxisLabel`s on the
    /// pane's right scale / the time strip, through the same IR + paint path the engine's own
    /// price-line and crosshair labels use (frame/axis.rs). Descriptor `coordinate` is media px
    /// from the pane's top (price) / the pane's left edge (time), matching the reference's
    /// `ISeriesPrimitiveAxisView.coordinate` semantics.
    fn append_primitive_axis_labels(&mut self, obj: &js_sys::Object, pane: usize) {
        self.append_primitive_price_axis_labels(obj, pane, PriceScaleTarget::Right, None);
        self.append_primitive_time_axis_labels(obj);
    }

    /// The series-primitive axis-label path (Phase C-b): price labels land on the OWNING
    /// series' scale strip — left or right as bound; an overlay series' labels go to the
    /// right strip per the engine's last-value rule (frame/axis.rs) — and descriptors may
    /// give a `price` instead of a raw `coordinate`, converted on the series' own scale.
    fn append_series_primitive_axis_labels(
        &mut self,
        obj: &js_sys::Object,
        pane: usize,
        target: PriceScaleTarget,
        base: Option<f64>,
    ) {
        let display_target = match target {
            PriceScaleTarget::Overlay => PriceScaleTarget::Right,
            target => target,
        };
        let Some(scale) = self.engine.price_scale_for(pane, target).cloned() else {
            return;
        };
        self.append_primitive_price_axis_labels(obj, pane, display_target, Some((scale, base)));
        self.append_primitive_time_axis_labels(obj);
    }

    /// Append `price_axis_views` as boxed labels on one axis strip (right or left, gated on
    /// that strip's visibility). A descriptor normally carries `coordinate` (media px from
    /// the pane's top); when `price_scale` is given (the series path), a `price` field takes
    /// precedence and is converted through that scale — a plugin cannot call chart APIs from
    /// a mid-render hook, and the draw context's bitmap-px converters don't yield the
    /// media-px coordinate the axis strip wants.
    fn append_primitive_price_axis_labels(
        &mut self,
        obj: &js_sys::Object,
        pane: usize,
        display_target: PriceScaleTarget,
        price_scale: Option<(PriceScaleCore, Option<f64>)>,
    ) {
        let Some((side, axis_x, axis_width)) =
            self.engine.price_scale_axis_geometry(pane, display_target)
        else {
            return;
        };
        let options = self.opts();
        let Some(labels) = call_view_array(obj, "price_axis_views") else {
            return;
        };
        let font_size = options.layout.font_size;
        let fallback = aeris_charts_core::style::DEFAULT_SURFACE_RGB;
        let surface = Color::parse_css(&options.layout.background.color)
            .unwrap_or(Color::rgb(fallback.0, fallback.1, fallback.2));
        let font_family = options.layout.font_family;
        let dpr = self.dpr;
        // Experimental host-owned chrome keeps its own layout-size metrics; only the shared
        // engine axis labels compact. Plugin labels never render bold.
        let measure = |text: &str| {
            measure_text_ctx(&self.axis_ctx, dpr, &font_family, font_size, false, text)
        };
        let Some(pane_state) = self.panes.get(pane) else {
            return;
        };
        let (pane_top, pane_bottom) = (pane_state.top, pane_state.top + pane_state.height);
        for label in labels.iter() {
            let Some(text) = js_sys::Reflect::get(&label, &"text".into())
                .ok()
                .and_then(|t| t.as_string())
            else {
                continue;
            };
            // A finite `price` (converted on the owning scale) wins over a raw coordinate.
            let price_y = js_sys::Reflect::get(&label, &"price".into())
                .ok()
                .and_then(|p| p.as_f64())
                .filter(|p| p.is_finite())
                .and_then(|price| {
                    price_scale.as_ref().and_then(|(scale, base)| {
                        if scale.is_empty() {
                            None
                        } else {
                            Some(scale.price_to_coordinate(price, base.unwrap_or(price)))
                        }
                    })
                });
            let y = match price_y {
                Some(y) => y,
                None => {
                    let Some(coordinate) = js_sys::Reflect::get(&label, &"coordinate".into())
                        .ok()
                        .and_then(|c| c.as_f64())
                        .filter(|c| c.is_finite())
                    else {
                        continue;
                    };
                    pane_top + coordinate
                }
            };
            if y < pane_top || y > pane_bottom {
                continue;
            }
            let width = 1.0 + 5.0 + 5.0 + 5.0 + measure(&text);
            let height = font_size + 2.5 * 2.0;
            // Colors mirror the price-line label resolution: the descriptor's
            // `background_color`/`color` picks the box and `text_color` the glyphs; an omitted
            // text color automatically contrasts with the effective background.
            let background = reflect_color(&label, "background_color")
                .or_else(|| reflect_color(&label, "color"))
                .unwrap_or(PRIMITIVE_LABEL_BG);
            let text_color = reflect_color(&label, "text_color")
                .unwrap_or_else(|| background.contrast_text_over(surface));
            // Placement mirrors the engine's price-line labels (frame/axis.rs): right-strip
            // labels left-align past the pane edge, left-strip labels right-align before it.
            let (x, align, background_x) = match side {
                PriceScaleSide::Left => (
                    axis_x + axis_width - 10.0,
                    AxisTextAlign::Right,
                    axis_x + axis_width - width,
                ),
                PriceScaleSide::Right => (axis_x + 10.0, AxisTextAlign::Left, axis_x),
            };
            self.axis_frame.labels.push(AxisLabel {
                text,
                x,
                y,
                color: text_color,
                align,
                midpoint: AxisTextMidpoint::Label,
                font_scale: 1.0,
                bold: false,
                background: Some((background_x, y - height / 2.0, width, height, background)),
                background_corners: AxisLabelCorners::for_align(align),
                measure_extra: 0.0,
                attach_group: None,
                border: None,
            });
        }
    }

    /// Append `time_axis_views` as boxed labels on the time strip: centered on the coordinate,
    /// clamped into the strip, on the same vertical slot as the crosshair time label
    /// (frame/axis.rs). Shared by the pane- and series-primitive paths.
    fn append_primitive_time_axis_labels(&mut self, obj: &js_sys::Object) {
        if !self.engine.time_axis_visible {
            return;
        }
        let Some(labels) = call_view_array(obj, "time_axis_views") else {
            return;
        };
        let options = self.opts();
        let font_size = options.layout.font_size;
        let fallback = aeris_charts_core::style::DEFAULT_SURFACE_RGB;
        let surface = Color::parse_css(&options.layout.background.color)
            .unwrap_or(Color::rgb(fallback.0, fallback.1, fallback.2));
        let font_family = options.layout.font_family;
        let dpr = self.dpr;
        // Same experimental-chrome boundary as the price-axis views above.
        let measure = |text: &str| {
            measure_text_ctx(&self.axis_ctx, dpr, &font_family, font_size, false, text)
        };
        for label in labels.iter() {
            let Some(text) = js_sys::Reflect::get(&label, &"text".into())
                .ok()
                .and_then(|t| t.as_string())
            else {
                continue;
            };
            let Some(coordinate) = js_sys::Reflect::get(&label, &"coordinate".into())
                .ok()
                .and_then(|c| c.as_f64())
                .filter(|c| c.is_finite())
            else {
                continue;
            };
            const BORDER: f64 = 1.0;
            const TICK: f64 = 5.0;
            const PADDING: f64 = 3.0;
            let width = measure(&text) + 9.0 * 2.0;
            let height = BORDER + TICK + PADDING + font_size + PADDING;
            let x = self.pane_left + coordinate;
            let box_x = (x - width / 2.0).clamp(
                self.pane_left,
                (self.pane_left + self.pane_w - width).max(self.pane_left),
            );
            let background = reflect_color(&label, "background_color")
                .or_else(|| reflect_color(&label, "color"))
                .unwrap_or(PRIMITIVE_LABEL_BG);
            let text_color = reflect_color(&label, "text_color")
                .unwrap_or_else(|| background.contrast_text_over(surface));
            self.axis_frame.labels.push(AxisLabel {
                text,
                x: box_x + width / 2.0,
                y: self.pane_h + BORDER + TICK + PADDING + font_size / 2.0,
                color: text_color,
                align: AxisTextAlign::Center,
                midpoint: AxisTextMidpoint::StableTime,
                font_scale: 1.0,
                bold: false,
                background: Some((box_x, self.pane_h, width, height, background)),
                background_corners: AxisLabelCorners::BOTTOM,
                measure_extra: 0.0,
                attach_group: None,
                border: None,
            });
        }
    }
}

/// The three converter fns handed to every view of one primitive, closed over the pane's
/// cloned scale state. Coordinates come out in absolute bitmap px (module docs above).
pub(super) struct PrimitiveConverters {
    pub(super) price_to_y: Closure<dyn FnMut(f64, JsValue) -> JsValue>,
    #[allow(dead_code)] // shared horizontal pair; the custom-series pass binds only price_to_y
    time_to_x: Closure<dyn FnMut(f64) -> JsValue>,
    #[allow(dead_code)] // shared horizontal pair; the custom-series pass binds only price_to_y
    logical_to_x: Closure<dyn FnMut(f64) -> JsValue>,
}

/// The horizontal converter pair (`time_to_x`, `logical_to_x`) every primitive flavor shares.
type HorizontalConverters = (
    Closure<dyn FnMut(f64) -> JsValue>,
    Closure<dyn FnMut(f64) -> JsValue>,
);

impl PrimitiveConverters {
    fn new(snapshot: &PrimitiveScaleSnapshot, pane: &PaneScaleSnapshot, pane_left_px: u32) -> Self {
        let vpr = snapshot.vpr;

        let (right, left) = (pane.right.clone(), pane.left.clone());
        let (base_right, base_left) = (pane.base_right, pane.base_left);
        // `price_to_y(price, target?)`: target `"left"` selects the pane's left scale; anything
        // else (including undefined) the right scale. `null` when the scale has no range.
        let price_to_y = Closure::wrap(Box::new(move |price: f64, target: JsValue| {
            let use_left = target.as_string().as_deref() == Some("left");
            let scale = if use_left { &left } else { &right };
            if scale.is_empty() {
                return JsValue::NULL;
            }
            let base = if use_left { base_left } else { base_right };
            JsValue::from_f64(scale.price_to_coordinate(price, base.unwrap_or(price)) * vpr)
        }) as Box<dyn FnMut(f64, JsValue) -> JsValue>);

        let (time_to_x, logical_to_x) = Self::horizontal(snapshot, pane_left_px);
        Self {
            price_to_y,
            time_to_x,
            logical_to_x,
        }
    }

    /// Series-bound converters (Phase C-b): `price_to_y(price)` (no target argument — the
    /// trailing slot is simply ignored when JS calls it with one) resolves on the OWNING
    /// series' scale — its pane's left/right/overlay scale as the series is bound — with the
    /// series' own base value anchoring percentage/indexed modes, like its geometry.
    pub(super) fn for_series(
        snapshot: &PrimitiveScaleSnapshot,
        pane: &PaneScaleSnapshot,
        target: PriceScaleTarget,
        base: Option<f64>,
        pane_left_px: u32,
    ) -> Self {
        let vpr = snapshot.vpr;
        let scale = match target {
            PriceScaleTarget::Right => pane.right.clone(),
            PriceScaleTarget::Left => pane.left.clone(),
            PriceScaleTarget::Overlay => pane.overlay.clone(),
            PriceScaleTarget::Named(id) => pane
                .named
                .iter()
                .find_map(|(candidate, scale)| (*candidate == id).then(|| scale.clone()))
                .unwrap_or_else(|| PriceScaleCore::new(Default::default())),
        };
        // `price_to_y(price)`: bitmap y on the owning series' scale; `null` when it has no range.
        let price_to_y = Closure::wrap(Box::new(move |price: f64, _target: JsValue| {
            if scale.is_empty() {
                return JsValue::NULL;
            }
            JsValue::from_f64(scale.price_to_coordinate(price, base.unwrap_or(price)) * vpr)
        }) as Box<dyn FnMut(f64, JsValue) -> JsValue>);

        let (time_to_x, logical_to_x) = Self::horizontal(snapshot, pane_left_px);
        Self {
            price_to_y,
            time_to_x,
            logical_to_x,
        }
    }

    /// The horizontal converters every primitive flavor shares (they depend only on the time
    /// scale and the pane's left edge, not on any price scale).
    fn horizontal(snapshot: &PrimitiveScaleSnapshot, pane_left_px: u32) -> HorizontalConverters {
        let hpr = snapshot.hpr;
        let left_px = f64::from(pane_left_px);

        let (time_scale, times) = (snapshot.time_scale.clone(), snapshot.times.clone());
        // `time_to_x(time)`: bitmap x for an exact UTC-seconds bar time, else `null` (reference
        // `timeToCoordinate` does not snap to the nearest bar).
        let time_to_x = Closure::wrap(Box::new(move |time: f64| {
            let Ok(time) = aeris_charts_core::model::data_validation::validate_timestamp(time)
            else {
                return JsValue::NULL;
            };
            if times.is_empty() {
                return JsValue::NULL;
            }
            let index = times.partition_point(|&point| point < time);
            if index >= times.len() || times[index] != time {
                return JsValue::NULL;
            }
            JsValue::from_f64(time_scale.index_to_coordinate(index as i64) * hpr + left_px)
        }) as Box<dyn FnMut(f64) -> JsValue>);

        let time_scale = snapshot.time_scale.clone();
        let empty = snapshot.times.is_empty();
        // `logical_to_x(index)`: bitmap x for a (possibly fractional) logical bar index.
        let logical_to_x = Closure::wrap(Box::new(move |logical: f64| {
            if !logical.is_finite() || empty {
                return JsValue::NULL;
            }
            JsValue::from_f64(time_scale.logical_to_coordinate(logical) * hpr + left_px)
        }) as Box<dyn FnMut(f64) -> JsValue>);

        (time_to_x, logical_to_x)
    }
}

/// Call one of the primitive's axis-view hooks and return its array, or `None` when the hook is
/// absent, not a function, threw, or did not return an array.
fn call_view_array(obj: &js_sys::Object, hook: &str) -> Option<js_sys::Array> {
    call_view_array_with(obj, hook, None)
}

/// [`call_view_array`] with an optional single argument passed to the hook (the `text_views`
/// overlay-text hook takes an `info` object; the axis-view hooks take none).
fn call_view_array_with(
    obj: &js_sys::Object,
    hook: &str,
    arg: Option<&JsValue>,
) -> Option<js_sys::Array> {
    let hook_fn = js_sys::Reflect::get(obj, &hook.into())
        .ok()?
        .dyn_into::<js_sys::Function>()
        .ok()?;
    let value = match arg {
        Some(arg) => hook_fn.call1(obj, arg),
        None => hook_fn.call0(obj),
    }
    .map_err(|error| {
        web_sys::console::warn_1(
            &format!("aeris_charts: pane primitive `{hook}` threw — {error:?}").into(),
        );
    })
    .ok()?;
    value.dyn_into::<js_sys::Array>().ok()
}

/// One primitive's non-null `hit_test` result (reference `PrimitiveHoveredItem`, reduced to Aeris's
/// arbitration model: the host owns z-ordering, so the plugin's `z_order` only says which
/// layer the hit belongs to; the reference's `distance`/`hitTestPriority`/`itemType`/`isBackground`
/// fields are not modeled).
struct PrimitiveHit {
    external_id: Option<String>,
    cursor: Option<String>,
    /// The owning series for a series primitive, `None` for a pane primitive (the reference's hit-test
    /// source: a series-primitive hit reports its series as `hoveredSeries`, a pane-primitive
    /// hit reports none).
    series: Option<SeriesId>,
    /// The result's `z_order` ("top"/"normal"/"bottom"; default "normal").
    z_order: String,
}

impl PrimitiveHit {
    fn for_series(mut self, id: SeriesId) -> Self {
        self.series = Some(id);
        self
    }

    fn for_pane(self) -> Self {
        self
    }

    fn layer(&self) -> HostPrimitiveLayer {
        match self.z_order.as_str() {
            "top" => HostPrimitiveLayer::Top,
            "bottom" => HostPrimitiveLayer::Bottom,
            _ => HostPrimitiveLayer::Normal,
        }
    }

    fn engine_hit(&self) -> HostPrimitiveHit {
        HostPrimitiveHit {
            series: self.series,
            layer: self.layer(),
            cursor: self.cursor.is_some(),
        }
    }
}

/// Call a primitive's `hit_test(x, y)` hook (Phase C-d) with absolute bitmap-px coordinates
/// and parse the result, or `None` when the hook is absent, not a function, threw, or
/// returned a non-object value.
fn call_hit_test(obj: &js_sys::Object, x: f64, y: f64) -> Option<PrimitiveHit> {
    let value = js_sys::Reflect::get(obj, &"hit_test".into())
        .ok()?
        .dyn_into::<js_sys::Function>()
        .ok()?
        .call2(obj, &JsValue::from_f64(x), &JsValue::from_f64(y))
        .map_err(|error| {
            web_sys::console::warn_1(
                &format!("aeris_charts: primitive `hit_test` threw — {error:?}").into(),
            );
        })
        .ok()?;
    if value.is_null() || value.is_undefined() || !value.is_object() {
        return None;
    }
    let string_prop = |key: &str| {
        js_sys::Reflect::get(&value, &key.into())
            .ok()
            .and_then(|v| v.as_string())
            .filter(|s| !s.is_empty())
    };
    Some(PrimitiveHit {
        external_id: string_prop("external_id"),
        cursor: string_prop("cursor_style"),
        series: None,
        z_order: string_prop("z_order").unwrap_or_else(|| "normal".to_string()),
    })
}

/// Read an optional CSS color property from a label descriptor.
fn reflect_color(value: &JsValue, key: &str) -> Option<Color> {
    js_sys::Reflect::get(value, &key.into())
        .ok()?
        .as_string()
        .and_then(|css| Color::parse_css(&css))
}
