//! Drawing-tool frame emission (model in drawings.rs): each pane's committed drawings in
//! z-order, the selected drawing's anchor handles, and the interactive-creation preview.
//!
//! Coordinates follow the frame build's conventions (frame/mod.rs): x is pane-local media px
//! scaled by the exact horizontal ratio (the trailing `translate_prims_x` shifts everything
//! when a left axis reserves space), y is chart-top-relative media px scaled by the vertical
//! ratio — the same space the price-line/series geometry uses, so a drawing's prims land
//! exactly on its converted anchors. Standalone drawing text uses `Prim::Text`; trend labels
//! use the backend-neutral `Prim::RotatedText` contract so every executor receives the same
//! aligned anchor and segment-normalized angle.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{IRect, LineStyle, LineType, Prim, TextAlign};

use super::series_geometry::{push_clipped_stroke, push_styled_stroke};
use super::{POSITION_ENTRY, PRIMARY};
use crate::drawings::handles::{handle_set, DrawingHandle, HandleShape};
use crate::drawings::{
    cap_radius, resolve_drawing_geometry, Drawing, DrawingBodyGeometry, DrawingGeometryOptions,
    DrawingHandleMode, DrawingId, DrawingKind, DrawingPart, DrawingParts, DrawingTextHAlign,
    DrawingTextLayout, PartContext, PositionGeometry, PositionZone, TEXT_CHROME_PAD, TEXT_PAD,
    TREND_TEXT_PLACEHOLDER,
};
use crate::ChartEngine;
use aeris_charts_core::model::plot_list::PlotValueIndex;

/// industry-standard drawing anchor handle: a theme-derived disc with the primary-token border
/// (the crosshair-marks disc idiom — the border is a larger filled disc underneath). Slightly
/// larger than the series selection anchors (2.5/1.5, series_geometry.rs) since these are
/// drag targets.
const ANCHOR_RADIUS: f64 = 4.0;
const ANCHOR_BORDER_WIDTH: f64 = 1.5;
const ANCHOR_BORDER: Color = PRIMARY;
const POSITION_ENTRY_WIDTH_CSS: f64 = 0.5;
const POSITION_ZONE_ALPHA: u8 = 70;
/// Progress must read as the emphasized portion of either semantic side on its own. Keep this
/// above the base-zone alpha instead of relying on a second lower-alpha pass to become visible
/// only through accidental compositing.
const POSITION_PROGRESS_ALPHA: u8 = 96;
/// The hover ring's dimmed variant of the focus border (the public reference shows the same border at
/// roughly half strength until the drawing is actually selected).
const HOVER_BORDER: Color = Color(PRIMARY.0 & 0xFFFF_FF00 | 0x73);
const TREND_TEXT_PLACEHOLDER_ALPHA: u8 = 0x99;

fn point_on_segment(a: (f64, f64), b: (f64, f64), t: f64) -> (f64, f64) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// A crisp line's `[from, to]` span (either order) clamped to `pane` in whole pixels, `None` when
/// it misses the pane. Executors dash a crisp line from its start, so a start clamped into the
/// pane moves back to a whole dash period from the unclamped start and keeps the pattern's phase;
/// the executors' dash loops stay bounded by the pane.
fn crisp_span(
    from: f64,
    to: f64,
    (low, high): (f64, f64),
    width: i32,
    style: LineStyle,
) -> Option<(i32, i32)> {
    let (start, end) = (from.min(to).round(), from.max(to).round());
    if !(start <= high && end >= low) {
        return None;
    }
    let period: f64 = style
        .dash_pattern(width as f32)
        .iter()
        .copied()
        .map(f64::from)
        .sum();
    let clamped = if start < low && period > 0.0 {
        low - (low - start).rem_euclid(period)
    } else {
        start.max(low)
    };
    Some((clamped.round() as i32, end.min(high) as i32))
}

/// A core segment stroke; a dashed or dotted one reaches executors as solid dash runs clipped to
/// `pane` ([`push_styled_stroke`]).
fn push_segment(
    a: (f64, f64),
    b: (f64, f64),
    (stroke, pane): ((f32, LineStyle, Color), aeris_charts_render::shape::Rect),
    out: &mut Vec<Prim>,
    points: &mut Vec<[f32; 2]>,
) {
    if (a.0 - b.0).abs() <= f64::EPSILON && (a.1 - b.1).abs() <= f64::EPSILON {
        return;
    }
    push_styled_stroke(out, points, &[a, b], LineType::Simple, stroke, pane);
}

fn push_drawing_cap(
    cap: crate::DrawingLineCap,
    endpoint: (f64, f64),
    toward: (f64, f64),
    width: f64,
    color: Color,
    out: &mut Vec<Prim>,
) {
    if cap == crate::DrawingLineCap::None {
        return;
    }
    let dx = toward.0 - endpoint.0;
    let dy = toward.1 - endpoint.1;
    let distance = dx.hypot(dy);
    if distance <= f64::EPSILON {
        return;
    }
    let ux = dx / distance;
    let uy = dy / distance;
    let radius = cap_radius(width);
    match cap {
        crate::DrawingLineCap::Circle => out.push(Prim::Circle {
            cx: endpoint.0 as f32,
            cy: endpoint.1 as f32,
            radius: radius as f32,
            fill: color,
            stroke_width: 0.0,
            stroke: color,
        }),
        crate::DrawingLineCap::Arrow => {
            let base_x = endpoint.0 + ux * radius * 2.0;
            let base_y = endpoint.1 + uy * radius * 2.0;
            let side_x = -uy * radius;
            let side_y = ux * radius;
            out.push(Prim::Triangle {
                a: [endpoint.0 as f32, endpoint.1 as f32],
                b: [(base_x + side_x) as f32, (base_y + side_y) as f32],
                c: [(base_x - side_x) as f32, (base_y - side_y) as f32],
                color,
            });
        }
        crate::DrawingLineCap::None => {}
    }
}

#[cfg(test)]
mod trend_label_tests {
    use super::*;
    use crate::drawings::{DrawingPoint, DrawingTextVAlign};

    #[test]
    fn all_nine_trend_label_positions_follow_the_segment() {
        let mut drawing = Drawing::new(
            1,
            DrawingKind::TrendLine,
            0,
            vec![
                DrawingPoint {
                    logical: 0.0,
                    price: 0.0,
                },
                DrawingPoint {
                    logical: 1.0,
                    price: 1.0,
                },
            ],
        );
        for line in [
            [(10.0, 50.0), (90.0, 50.0)],
            [(10.0, 80.0), (90.0, 20.0)],
            [(50.0, 90.0), (50.0, 10.0)],
            [(90.0, 20.0), (10.0, 80.0)],
        ] as [[(f64, f64); 2]; 4]
        {
            let mut start = line[0];
            let mut end = line[1];
            if end.0 < start.0 || ((end.0 - start.0).abs() <= f64::EPSILON && end.1 > start.1) {
                std::mem::swap(&mut start, &mut end);
            }
            let dx: f64 = end.0 - start.0;
            let dy: f64 = end.1 - start.1;
            let length = dx.hypot(dy);
            let (ux, uy) = (dx / length, dy / length);
            for (h_align, expected_distance) in [
                (DrawingTextHAlign::Left, 4.0),
                (DrawingTextHAlign::Center, length / 2.0),
                (DrawingTextHAlign::Right, length - 4.0),
            ] {
                drawing.text_h_align = h_align;
                for (v_align, expected_normal) in [
                    (DrawingTextVAlign::Top, 10.0),
                    (DrawingTextVAlign::Middle, 0.0),
                    (DrawingTextVAlign::Bottom, -10.0),
                ] {
                    drawing.text_v_align = v_align;
                    let (x, y, align, angle) = ChartEngine::drawing_text_placement(
                        &drawing, &line, 100.0, 0.0, 100.0, 12.0, 4.0,
                    );
                    assert_eq!(align, h_align);
                    let (from_x, from_y) = (x - start.0, y - start.1);
                    assert!((from_x * ux + from_y * uy - expected_distance).abs() < 1e-9);
                    assert!((from_x * uy - from_y * ux - expected_normal).abs() < 1e-9);
                    assert!((-std::f64::consts::FRAC_PI_2..=std::f64::consts::FRAC_PI_2)
                        .contains(&angle));
                }
            }
        }

        drawing.text_h_align = DrawingTextHAlign::Right;
        drawing.text_v_align = DrawingTextVAlign::Middle;
        let line = [(10.0, 80.0), (90.0, 20.0)];
        let reversed = [line[1], line[0]];
        let (x, y, _, angle) =
            ChartEngine::drawing_text_placement(&drawing, &reversed, 100.0, 0.0, 100.0, 12.0, 4.0);
        assert!((x - 86.8).abs() < 1e-9);
        assert!((y - 22.4).abs() < 1e-9);
        assert!((angle - (-0.6_f64).atan2(0.8)).abs() < 1e-9);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PositionRunSide {
    Reward,
    Risk,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PositionRunProgress {
    pub(super) start: crate::drawings::DrawingPoint,
    pub(super) point: crate::drawings::DrawingPoint,
    pub(super) side: PositionRunSide,
}

impl ChartEngine {
    fn drawing_frame_text<'a>(&self, drawing: &'a Drawing) -> Option<(&'a str, bool)> {
        if !drawing.text.is_empty() {
            return Some((drawing.display_text(), false));
        }
        (drawing.kind == DrawingKind::TrendLine
            && self.hovered_text == Some(drawing.id)
            && self.editing_drawing() != Some(drawing.id))
        .then_some((TREND_TEXT_PLACEHOLDER, true))
    }

    /// Width source for the middle-line cutout. Hover reserves the full prompt; once editing
    /// begins, an empty value uses the editor's one-em caret opening and measured text expands it.
    fn drawing_frame_gap_text<'a>(&self, drawing: &'a Drawing) -> Option<&'a str> {
        if drawing.kind == DrawingKind::TrendLine && self.editing_drawing() == Some(drawing.id) {
            return Some(drawing.display_text());
        }
        if !drawing.text.is_empty() {
            return Some(drawing.display_text());
        }
        (drawing.kind == DrawingKind::TrendLine && self.hovered_text == Some(drawing.id))
            .then_some(TREND_TEXT_PLACEHOLDER)
    }

    fn measure_drawing_frame_text(&self, drawing: &Drawing, text: &str, size: f64) -> f64 {
        let layout = &self.options.get().layout;
        self.measure_text_run(
            text,
            size,
            &layout.font_family,
            drawing.text_weight.unwrap_or(400),
            drawing.text_italic,
        )
    }

    #[cfg(test)]
    pub(crate) fn build_drawings_frame_reference(
        &self,
        pane_index: usize,
        pane_w_px: i32,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
    ) {
        for drawing in &self.drawings {
            if drawing.pane_index != pane_index
                || !drawing.visible
                || !drawing.interval_visibility.allows(self.drawing_interval)
                || !self.drawing_viewport_candidate_reference(drawing)
            {
                continue;
            }
            let Some(px) = self.drawing_px(drawing) else {
                continue;
            };
            let px = px
                .into_iter()
                .map(|(x, y)| (x * hpr, y * vpr))
                .collect::<Vec<_>>();
            self.build_drawing_prims(drawing, &px, pane_w_px, vpr, out, points);
            self.build_drawing_text(drawing, &px, pane_w_px, vpr, out);
            self.build_drawing_labels(drawing, &px, vpr, out);
        }
    }

    /// Segmented committed build for retained reassembly: stable z-order committed drawings,
    /// recording each emitted drawing's prim/point range in `parts` (stable z-order) and the
    /// trailing preview (brush + pending) start in `preview_start` (prim, point). Previews
    /// always trail committed so assembly can place them topmost among chart content.
    /// Ordering.rs reassembles these parts idle-below / active-above without rebuilding
    /// geometry on hover/selection. Drawings on stale panes draw nowhere.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_drawings_frame_segmented(
        &self,
        pane_index: usize,
        pane_w_px: i32,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
        parts: &mut Vec<super::RetainedDrawingPart>,
        preview_start: &mut (usize, usize),
    ) {
        parts.clear();
        if self.drawing_runtime.borrow().pane_count(pane_index) <= 20 {
            for drawing in &self.drawings {
                if drawing.pane_index != pane_index
                    || !drawing.visible
                    || !drawing.interval_visibility.allows(self.drawing_interval)
                {
                    continue;
                }
                let Some(px) = self.drawing_px(drawing) else {
                    continue;
                };
                let px = px
                    .into_iter()
                    .map(|(x, y)| (x * hpr, y * vpr))
                    .collect::<Vec<_>>();
                let prim_start = out.len();
                let point_start = points.len();
                self.build_drawing_prims(drawing, &px, pane_w_px, vpr, out, points);
                self.build_drawing_text(drawing, &px, pane_w_px, vpr, out);
                self.build_drawing_labels(drawing, &px, vpr, out);
                parts.push(super::RetainedDrawingPart {
                    id: drawing.id,
                    prim_start,
                    prim_end: out.len(),
                    point_start,
                    point_end: points.len(),
                });
            }
        } else {
            let candidates = self.take_drawing_candidates(pane_index, None);
            let mut runtime = self.drawing_runtime.borrow_mut();
            for &id in &candidates {
                let Some(position) = runtime.position(id) else {
                    continue;
                };
                let Some(drawing) = self.drawings.get(position) else {
                    continue;
                };
                if !drawing.visible || !drawing.interval_visibility.allows(self.drawing_interval) {
                    continue;
                }
                let Some(key) = self.drawing_coordinate_key(drawing) else {
                    continue;
                };
                let Some(px) = self.drawing_px_cached(drawing, &mut runtime, key) else {
                    continue;
                };
                let px = px
                    .iter()
                    .map(|&(x, y)| (x * hpr, y * vpr))
                    .collect::<Vec<_>>();
                let prim_start = out.len();
                let point_start = points.len();
                self.build_drawing_prims(drawing, &px, pane_w_px, vpr, out, points);
                self.build_drawing_text(drawing, &px, pane_w_px, vpr, out);
                self.build_drawing_labels(drawing, &px, vpr, out);
                parts.push(super::RetainedDrawingPart {
                    id: drawing.id,
                    prim_start,
                    prim_end: out.len(),
                    point_start,
                    point_end: points.len(),
                });
                runtime.record_visible();
            }
            drop(runtime);
            self.recycle_drawing_candidates(candidates);
        }
        *preview_start = (out.len(), points.len());
        // Live brush stroke: the decimated points so far paint as the same smooth curve the
        // commit will store, so what the user sees while dragging is what they get.
        if let Some(capture) = self.brush_capture() {
            if capture.pane_index == pane_index && capture.points.len() >= 2 {
                let px: Option<Vec<(f64, f64)>> = capture
                    .points
                    .iter()
                    .map(|&point| self.drawing_to_px(pane_index, point))
                    .collect();
                if let Some(px) = px {
                    let px: Vec<(f64, f64)> =
                        px.into_iter().map(|(x, y)| (x * hpr, y * vpr)).collect();
                    self.build_drawing_prims(&capture.options, &px, pane_w_px, vpr, out, points);
                }
            }
        }
        // Interactive creation: committed anchors plus the preview point render as a tentative
        // drawing, with handles on the committed anchors (the reference rectangle-drawing-tool's
        // PreviewRectangle — same geometry, shown while placing).
        if let Some(pending) = self.pending_drawing() {
            if pending.drawing.pane_index == pane_index {
                let mut anchors = pending.drawing.points.clone();
                let is_sequence = pending.drawing.kind.spec().placement.is_sequence();
                if let Some(preview) = pending.preview {
                    if is_sequence || anchors.len() < pending.drawing.kind.anchor_count() {
                        anchors.push(preview);
                    }
                }
                // Families that resolve partial anchors preview from the second anchor on.
                let partial = anchors.len() >= 2
                    && pending
                        .drawing
                        .kind
                        .spec()
                        .family
                        .is_some_and(|family| family.partial_preview);
                let ready = if is_sequence {
                    anchors.len() >= pending.drawing.kind.anchor_count()
                } else {
                    anchors.len() == pending.drawing.kind.anchor_count() || partial
                };
                if ready {
                    let px: Option<Vec<(f64, f64)>> = anchors
                        .iter()
                        .map(|&point| {
                            self.drawing_anchor_px(
                                pending.drawing.kind,
                                pane_index,
                                pending.drawing.price_scale,
                                point,
                            )
                        })
                        .collect();
                    if let Some(media) = px {
                        let px: Vec<(f64, f64)> =
                            media.iter().map(|&(x, y)| (x * hpr, y * vpr)).collect();
                        let mut preview_drawing = pending.drawing.clone();
                        // Family stats and angles measure the previewed geometry, not only the
                        // anchors placed so far.
                        preview_drawing.points.clone_from(&anchors);
                        if preview_drawing.kind.spec().handles == DrawingHandleMode::RectangleBounds
                        {
                            if let Some(fill) = preview_drawing.preview_fill_color.clone() {
                                preview_drawing.fill_color = Some(fill);
                            }
                        }
                        self.build_drawing_prims(
                            &preview_drawing,
                            &px,
                            pane_w_px,
                            vpr,
                            out,
                            points,
                        );
                        if pending.drawing.kind.spec().handles == DrawingHandleMode::RectangleBounds
                        {
                            // the public reference shows all eight anchors while the rectangle is being
                            // drawn (committed corner + live preview corner), not only after
                            // the commit.
                            let handles = handle_set(DrawingHandleMode::RectangleBounds, &px);
                            build_handles(&handles, vpr, self.anchor_fill(), out);
                        } else {
                            // The placed anchors' handles, where the family paints them on the
                            // previewed geometry (a regression's on its fitted line); derived
                            // handles wait for the committed drawing.
                            let placed = pending.drawing.points.len();
                            let handles: Vec<(f64, f64)> = self
                                .drawing_handle_set(&preview_drawing, &media)
                                .into_iter()
                                .filter(|handle| {
                                    matches!(
                                        handle.part,
                                        crate::DrawingDragPart::Anchor(index) if index < placed
                                    )
                                })
                                .map(|handle| (handle.point.0 * hpr, handle.point.1 * vpr))
                                .collect();
                            build_anchor_handles(&handles, vpr, self.anchor_fill(), out);
                        }
                    }
                } else if !anchors.is_empty() {
                    // Fewer anchors than the tool needs: a two-anchor kind before the preview
                    // resolves shows its first anchor as a handle alone; a tool of three or more
                    // anchors between clicks also runs a guide polyline in the drawing's stroke
                    // through its placed anchors to the pointer, so every click leaves visible
                    // ink.
                    let px: Option<Vec<(f64, f64)>> = anchors
                        .iter()
                        .map(|&point| {
                            self.drawing_anchor_px(
                                pending.drawing.kind,
                                pane_index,
                                pending.drawing.price_scale,
                                point,
                            )
                            .map(|(x, y)| (x * hpr, y * vpr))
                        })
                        .collect();
                    if let (Some(px), Some(pane)) = (px, self.panes.get(pane_index)) {
                        if px.len() >= 2 {
                            let clip = aeris_charts_render::shape::Rect {
                                left: 0.0,
                                top: pane.top * vpr,
                                right: f64::from(pane_w_px),
                                bottom: (pane.top + pane.height) * vpr,
                            };
                            push_clipped_stroke(
                                out,
                                points,
                                &px,
                                clip,
                                (
                                    (pending.drawing.width * vpr) as f32,
                                    pending.drawing.style,
                                    pending.drawing.stroke_color(),
                                ),
                                &mut Vec::new(),
                            );
                        }
                        let placed = pending.drawing.points.len().clamp(1, px.len());
                        build_anchor_handles(&px[..placed], vpr, self.anchor_fill(), out);
                    }
                }
            }
        }
    }

    /// The selected/hovered drawing's converted bitmap-px anchor points, or `None` when the
    /// id is stale, on another pane, or off-screen.
    fn overlay_drawing_px(
        &self,
        pane_index: usize,
        id: DrawingId,
        hpr: f64,
        vpr: f64,
    ) -> Option<Vec<(f64, f64)>> {
        let drawing = self.drawing(id)?;
        if drawing.pane_index != pane_index
            || !drawing.visible
            || !drawing.interval_visibility.allows(self.drawing_interval)
        {
            return None;
        }
        let key = self.drawing_coordinate_key(drawing)?;
        let mut runtime = self.drawing_runtime.borrow_mut();
        let px = self.drawing_px_cached(drawing, &mut runtime, key)?;
        Some(
            px.iter()
                .map(|&(x, y)| (x * hpr, y * vpr))
                .collect::<Vec<_>>(),
        )
    }

    /// Selection chrome is retained with the overlay, so selection-only changes do not
    /// invalidate or reconstruct unrelated drawing geometry. The text tool gets no anchor
    /// handles (the public reference: text has no drag points) — its selection affordance is the focus
    /// border alone. That border STAYS painted while the host typing-mode editor is open
    /// (the wrap is borderless; only the caret overlays), so entering/leaving edit cannot
    /// shift the outline.
    pub(super) fn build_selected_drawing_handles_frame(
        &self,
        pane_index: usize,
        pane_w_px: i32,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        let Some(id) = self.selected_drawing else {
            return;
        };
        let Some(drawing) = self.drawing(id) else {
            return;
        };
        if drawing.kind.spec().requests_text_editor {
            let Some(px) = self.overlay_drawing_px(pane_index, id, hpr, vpr) else {
                return;
            };
            self.push_text_chrome(drawing, &px, pane_w_px, vpr, ANCHOR_BORDER, out);
            return;
        }
        let Some(px) = self.overlay_drawing_px(pane_index, id, 1.0, 1.0) else {
            return;
        };
        let mut handles = self.drawing_handle_set(drawing, &px);
        for handle in &mut handles {
            handle.point = (handle.point.0 * hpr, handle.point.1 * vpr);
        }
        build_handles(&handles, vpr, self.anchor_fill(), out);
    }

    /// The hovered text drawing's focus border at hover opacity (the public reference's hover ring):
    /// the same chrome box as selection, dimmed. Suppressed while the drawing is selected
    /// (the full-strength border already paints, including during typing mode).
    pub(super) fn build_hovered_text_frame(
        &self,
        pane_index: usize,
        pane_w_px: i32,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        let Some(id) = self.hovered_text else {
            return;
        };
        if self.selected_drawing == Some(id) {
            return;
        }
        let Some(drawing) = self.drawing(id) else {
            return;
        };
        if drawing.kind != DrawingKind::Text {
            return;
        }
        let Some(px) = self.overlay_drawing_px(pane_index, id, hpr, vpr) else {
            return;
        };
        self.push_text_chrome(drawing, &px, pane_w_px, vpr, HOVER_BORDER, out);
    }

    /// One drawing's geometry prims at bitmap-px anchors `px`.
    fn build_drawing_prims(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        pane_w_px: i32,
        vpr: f64,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
    ) {
        if matches!(
            drawing.kind,
            DrawingKind::FixedRangeVolumeProfile
                | DrawingKind::AnchoredVolumeProfile
                | DrawingKind::AnchoredVwap
        ) && drawing.profile.is_some()
        {
            self.build_profile_drawing_prims(drawing, px, pane_w_px, vpr, out, points);
            return;
        }
        if drawing.kind.spec().family.is_some() {
            self.build_family_prims(drawing, px, pane_w_px, vpr, out, points);
            return;
        }
        let color = drawing.stroke_color();
        let crisp_width = (drawing.width * vpr).round().max(1.0) as i32;
        let Some(pane) = self.panes.get(drawing.pane_index) else {
            return;
        };
        // Dashed and dotted strokes lower to solid dash runs clipped to the pane.
        let stroke = (
            ((drawing.width * vpr) as f32, drawing.style, color),
            aeris_charts_render::shape::Rect {
                left: 0.0,
                top: pane.top * vpr,
                right: f64::from(pane_w_px),
                bottom: (pane.top + pane.height) * vpr,
            },
        );
        let Some(geometry) = resolve_drawing_geometry(
            drawing.kind,
            px,
            f64::from(pane_w_px),
            pane.top * vpr,
            pane.height * vpr,
            DrawingGeometryOptions {
                line_width: drawing.width,
                device_scale: vpr,
                extend_left: drawing.extend_left,
                extend_right: drawing.extend_right,
            },
        ) else {
            return;
        };
        match geometry.body {
            DrawingBodyGeometry::Segment { a, b } => {
                let label_gap = self.segment_label_gap(drawing, px, pane_w_px, vpr, a, b);
                if let Some((gap_start, gap_end)) = label_gap {
                    push_segment(a, point_on_segment(a, b, gap_start), stroke, out, points);
                    push_segment(point_on_segment(a, b, gap_end), b, stroke, out, points);
                } else {
                    push_segment(a, b, stroke, out, points);
                }
                push_drawing_cap(drawing.stroke_start, a, b, drawing.width * vpr, color, out);
                push_drawing_cap(drawing.stroke_end, b, a, drawing.width * vpr, color, out);
            }
            DrawingBodyGeometry::Horizontal { y, x0, x1 } => {
                let x0 = (x0.round() as i32).clamp(0, pane_w_px);
                let x1 = (x1.round() as i32).clamp(0, pane_w_px);
                if x0 != x1 {
                    out.push(Prim::HLine {
                        y: y.round() as i32,
                        x0: x0.min(x1),
                        x1: x0.max(x1),
                        width: crisp_width,
                        style: drawing.style,
                        color,
                    });
                }
            }
            DrawingBodyGeometry::Vertical { x, y0, y1 } => {
                out.push(Prim::VLine {
                    x: x.round() as i32,
                    y0: y0.round().max(0.0) as i32,
                    y1: y1.round().max(0.0) as i32,
                    width: crisp_width,
                    style: drawing.style,
                    color,
                });
            }
            DrawingBodyGeometry::Rectangle {
                left,
                right,
                top,
                bottom,
            } => {
                let left = left.round() as i32;
                let right = right.round() as i32;
                let top = top.round() as i32;
                let bottom = bottom.round() as i32;
                // Official `positionsBox`: both endpoint pixels belong to the box, so an
                // equal-point preview still occupies one bitmap pixel.
                let width = (right - left).abs() + 1;
                let height = (bottom - top).abs() + 1;
                // reference rectangle-drawing-tool default: the fill is the border color washed
                // out (its `previewFillColor`/`fillColor` alpha pattern) — 20% here.
                let fill = drawing.fill_or_wash(51);
                if drawing.fill_enabled {
                    out.push(Prim::Rect {
                        rect: IRect {
                            x: left,
                            y: top,
                            w: width,
                            h: height,
                        },
                        color: fill,
                    });
                }
                if !drawing.border_visible {
                    return;
                }
                if drawing.style == LineStyle::Solid {
                    out.push(Prim::RectFrame {
                        rect: IRect {
                            x: left,
                            y: top,
                            w: width,
                            h: height,
                        },
                        border: crisp_width,
                        color,
                    });
                } else {
                    // Dotted/dashed border: four crisp line prims sharing the dash pattern,
                    // centered on the frame's inner edge (where RectFrame paints).
                    let half = (crisp_width as f64 / 2.0) as i32;
                    for y in [top + half, top + height - half] {
                        out.push(Prim::HLine {
                            y,
                            x0: left,
                            x1: left + width,
                            width: crisp_width,
                            style: drawing.style,
                            color,
                        });
                    }
                    for x in [left + half, left + width - half] {
                        out.push(Prim::VLine {
                            x,
                            y0: top,
                            y1: top + height,
                            width: crisp_width,
                            style: drawing.style,
                            color,
                        });
                    }
                }
            }
            DrawingBodyGeometry::Position(position) => {
                let reward = Color::parse_css(aeris_charts_core::style::MARKET_UP_CSS)
                    .unwrap_or(Color::rgb(8, 153, 129));
                let risk = Color::parse_css(aeris_charts_core::style::MARKET_DOWN_CSS)
                    .unwrap_or(Color::rgb(247, 82, 95));
                push_position_zone(out, position.reward_zone(), reward);
                push_position_zone(out, position.risk_zone(), risk);

                let left_px = position.left.round() as i32;
                let right_px = position.right.round() as i32;
                if left_px != right_px {
                    let entry_path = [
                        [left_px as f32, position.entry_y as f32],
                        [right_px as f32, position.entry_y as f32],
                    ];
                    super::series_geometry::push_line_stroke(
                        out,
                        points,
                        &entry_path,
                        (POSITION_ENTRY_WIDTH_CSS * vpr) as f32,
                        drawing.style,
                        LineType::Simple,
                        POSITION_ENTRY,
                    );
                }
            }
            // The text tool's geometry is its label (emitted by `build_drawing_text`).
            DrawingBodyGeometry::Empty => {}
            DrawingBodyGeometry::Polyline {
                points: line_points,
                line_type,
                terminal,
            } => {
                push_styled_stroke(out, points, line_points, line_type, stroke.0, stroke.1);
                if let (Some(first), Some(last)) = (line_points.first(), line_points.last()) {
                    if line_points.len() >= 2 {
                        push_drawing_cap(
                            drawing.stroke_start,
                            *first,
                            line_points[1],
                            drawing.width * vpr,
                            color,
                            out,
                        );
                        push_drawing_cap(
                            drawing.stroke_end,
                            *last,
                            line_points[line_points.len() - 2],
                            drawing.width * vpr,
                            color,
                            out,
                        );
                    }
                }
                if let Some(terminal) = terminal {
                    let first_point = points.len() as u32;
                    for (x, y) in terminal {
                        points.push([x as f32, y as f32]);
                    }
                    out.push(Prim::Polyline {
                        first_point,
                        point_count: 3,
                        width: (drawing.width * vpr) as f32,
                        style: LineStyle::Solid,
                        line_type: LineType::Simple,
                        color,
                    });
                }
            }
        }
    }

    /// The `[start, end]` fraction of segment `a → b` a middle segment-layout label cuts out of
    /// the stroke: the measured run (the host editor's one-em minimum while editing) plus the
    /// text pad on each side, projected onto the segment.
    fn segment_label_gap(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        pane_w_px: i32,
        vpr: f64,
        a: (f64, f64),
        b: (f64, f64),
    ) -> Option<(f64, f64)> {
        self.drawing_frame_gap_text(drawing)
            .filter(|_| {
                drawing.kind.spec().text_layout == DrawingTextLayout::Segment
                    && drawing.text_v_align == crate::drawings::DrawingTextVAlign::Middle
            })
            .and_then(|text| {
                let (size, x, y, align, angle) =
                    self.text_run_geometry(drawing, px, pane_w_px, vpr);
                let mut width = self.measure_drawing_frame_text(drawing, text, size);
                if self.editing_drawing() == Some(drawing.id) {
                    // Match the host editor's one-em empty/minimum width. This leaves a
                    // compact caret slot and then grows from actual shaped advance.
                    width = width.max(size);
                }
                let gap = TEXT_PAD * vpr;
                let (local_start, local_end) = match align {
                    DrawingTextHAlign::Left => (-gap, width + gap),
                    DrawingTextHAlign::Center => (-width / 2.0 - gap, width / 2.0 + gap),
                    DrawingTextHAlign::Right => (-width - gap, gap),
                };
                let length_sq = (b.0 - a.0).powi(2) + (b.1 - a.1).powi(2);
                if length_sq <= f64::EPSILON {
                    return None;
                }
                let project = |distance: f64| {
                    let px = x + angle.cos() * distance;
                    let py = y + angle.sin() * distance;
                    ((px - a.0) * (b.0 - a.0) + (py - a.1) * (b.1 - a.1)) / length_sq
                };
                let first = project(local_start);
                let second = project(local_end);
                let start = first.min(second).clamp(0.0, 1.0);
                let end = first.max(second).clamp(0.0, 1.0);
                (start < end).then_some((start, end))
            })
    }

    /// The color every text run of `drawing` resolves to: the explicit `text_color`, the stroke
    /// for segment-layout labels, then the chart foreground.
    pub(crate) fn drawing_label_color(&self, drawing: &Drawing) -> Color {
        let layout = &self.options.get().layout;
        drawing
            .text_color
            .as_deref()
            .and_then(Color::parse_css)
            .or_else(|| {
                (drawing.kind.spec().text_layout == DrawingTextLayout::Segment)
                    .then(|| Color::parse_css(&drawing.color))
                    .flatten()
            })
            .or_else(|| Color::parse_css(&layout.text_color))
            .unwrap_or_else(|| {
                let fallback = aeris_charts_core::style::DEFAULT_FOREGROUND_RGB;
                Color::rgb(fallback.0, fallback.1, fallback.2)
            })
    }

    /// Lower a B8 family tool's shared parts (`drawings/parts.rs`) into the ordered frame. The
    /// family resolved them in bitmap px; this is the only place family geometry becomes `Prim`s.
    fn build_family_prims(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        pane_w_px: i32,
        vpr: f64,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
    ) {
        let (Some(family), Some(pane)) = (
            drawing.kind.spec().family,
            self.panes.get(drawing.pane_index),
        ) else {
            return;
        };
        // The frame's horizontal ratio: `pane_w_px` is the rounded bitmap width it derives from.
        let hpr = f64::from(pane_w_px) / self.pane_w.max(1.0);
        let context = PartContext {
            engine: self,
            drawing,
            px,
            pane: aeris_charts_render::shape::Rect {
                left: 0.0,
                top: pane.top * vpr,
                right: f64::from(pane_w_px),
                bottom: (pane.top + pane.height) * vpr,
            },
            scale: vpr,
            x_scale: hpr,
            text_editing: self.editing_drawing() == Some(drawing.id),
        };
        debug_assert!(
            drawing.points.len() != px.len()
                || drawing.points.iter().zip(px).all(|(&point, &anchor)| {
                    context.point_px(point).is_none_or(|mapped| {
                        (mapped.0 - anchor.0).abs() <= 1e-6 * anchor.0.abs().max(1.0)
                            && (mapped.1 - anchor.1).abs() <= 1e-6 * anchor.1.abs().max(1.0)
                    })
                }),
            "derived family points share the anchors' caller-px space"
        );
        let mut parts = DrawingParts::default();
        (family.build_parts)(&context, &mut parts);
        let mut scratch = Vec::new();
        let color = drawing.stroke_color();
        let layout = &self.options.get().layout;
        for part in &parts.items {
            match *part {
                DrawingPart::Stroke {
                    start,
                    end,
                    stroke,
                    label_gap,
                } => {
                    let line = &parts.points[start..end];
                    let width = (stroke.width_css(drawing) * vpr) as f32;
                    let style = stroke.line_style(drawing);
                    let stroke_color = stroke.color.unwrap_or(color);
                    let gap = if label_gap && line.len() == 2 {
                        self.segment_label_gap(drawing, px, pane_w_px, vpr, line[0], line[1])
                    } else {
                        None
                    };
                    // Dashed styles split into solid dash runs (the series' dash contract), so
                    // the WebGPU tessellator, which has no dash concept, paints Canvas2D's dashes.
                    let mut push = |run: &[(f64, f64)]| {
                        push_clipped_stroke(
                            out,
                            points,
                            run,
                            context.pane,
                            (width, style, stroke_color),
                            &mut scratch,
                        );
                    };
                    match gap {
                        Some((gap_start, gap_end)) => {
                            let (a, b) = (line[0], line[1]);
                            push(&[a, point_on_segment(a, b, gap_start)]);
                            push(&[point_on_segment(a, b, gap_end), b]);
                        }
                        None => push(line),
                    }
                }
                DrawingPart::HLine { y, x0, x1, stroke } => {
                    let width = (stroke.width_css(drawing) * vpr).round().max(1.0) as i32;
                    let style = stroke.line_style(drawing);
                    let span = crisp_span(x0, x1, (0.0, f64::from(pane_w_px)), width, style);
                    if let Some((x0, x1)) = span.filter(|(x0, x1)| x0 != x1) {
                        out.push(Prim::HLine {
                            y: y.round() as i32,
                            x0,
                            x1,
                            width,
                            style,
                            color: stroke.color.unwrap_or(color),
                        });
                    }
                }
                DrawingPart::VLine { x, y0, y1, stroke } => {
                    let width = (stroke.width_css(drawing) * vpr).round().max(1.0) as i32;
                    let style = stroke.line_style(drawing);
                    let pane_span = (context.pane.top.floor(), context.pane.bottom.ceil());
                    if let Some((y0, y1)) = crisp_span(y0, y1, pane_span, width, style) {
                        out.push(Prim::VLine {
                            x: x.round() as i32,
                            y0,
                            y1,
                            width,
                            style,
                            color: stroke.color.unwrap_or(color),
                        });
                    }
                }
                DrawingPart::Fill {
                    upper,
                    lower,
                    count,
                    color: fill,
                    ..
                } => {
                    let upper_first = points.len() as u32;
                    points.extend(
                        parts.points[upper..upper + count]
                            .iter()
                            .map(|&(x, y)| [x as f32, y as f32]),
                    );
                    let lower_first = points.len() as u32;
                    points.extend(
                        parts.points[lower..lower + count]
                            .iter()
                            .map(|&(x, y)| [x as f32, y as f32]),
                    );
                    out.push(Prim::BandFill {
                        upper_first,
                        lower_first,
                        point_count: count as u32,
                        line_type: LineType::Simple,
                        fill: fill.unwrap_or(color),
                    });
                }
                DrawingPart::Disc {
                    center,
                    radius,
                    color: fill,
                } => {
                    let fill = fill.unwrap_or(color);
                    out.push(Prim::Circle {
                        cx: center.0 as f32,
                        cy: center.1 as f32,
                        radius: radius as f32,
                        fill,
                        stroke_width: 0.0,
                        stroke: fill,
                    });
                }
                DrawingPart::Tube { start, end, stroke } => {
                    let line = &parts.points[start..end];
                    let width = stroke.width_css(drawing) * vpr;
                    let fill = stroke.color.unwrap_or(color);
                    let mut chains = Vec::new();
                    let count = DrawingParts::tube_region(line, width, context.pane, &mut chains);
                    if count > 0 {
                        let upper_first = points.len() as u32;
                        points.extend(chains.iter().map(|&(x, y)| [x as f32, y as f32]));
                        out.push(Prim::BandFill {
                            upper_first,
                            lower_first: upper_first + count as u32,
                            point_count: count as u32,
                            line_type: LineType::Simple,
                            fill,
                        });
                    } else {
                        // Beyond the fill bounds even when coarsened: a plain stroke, whose
                        // self-overlaps may blend twice on the GPU executors.
                        push_clipped_stroke(
                            out,
                            points,
                            line,
                            context.pane,
                            (width as f32, LineStyle::Solid, fill),
                            &mut scratch,
                        );
                    }
                }
                DrawingPart::Label { index } => {
                    let label = &parts.labels[index];
                    let box_layout = label.layout(|line| {
                        self.measure_text_run(
                            line,
                            label.size,
                            &layout.font_family,
                            label.weight,
                            label.italic,
                        )
                    });
                    // Boxes off the pane (level labels of far tines) emit nothing.
                    if !box_layout.rect.intersects(&context.pane) {
                        continue;
                    }
                    let rect = IRect {
                        x: box_layout.rect.left.round() as i32,
                        y: box_layout.rect.top.round() as i32,
                        w: (box_layout.rect.right - box_layout.rect.left)
                            .round()
                            .max(1.0) as i32,
                        h: (box_layout.rect.bottom - box_layout.rect.top)
                            .round()
                            .max(1.0) as i32,
                    };
                    if let Some(background) = label.background {
                        out.push(Prim::Rect {
                            rect,
                            color: background,
                        });
                    }
                    if let Some(border) = label.border {
                        out.push(Prim::RectFrame {
                            rect,
                            border: vpr.round().max(1.0) as i32,
                            color: border,
                        });
                    }
                    let text_color = label
                        .color
                        .unwrap_or_else(|| self.drawing_label_color(drawing));
                    for (line_index, text) in label.lines.iter().enumerate() {
                        out.push(Prim::Text {
                            x: box_layout.text_x as f32,
                            y: (box_layout.first_y + line_index as f64 * box_layout.line_height)
                                as f32,
                            text: text.clone(),
                            color: text_color,
                            size: label.size as f32,
                            family: layout.font_family.clone(),
                            align: TextAlign::Left,
                            weight: label.weight,
                            italic: label.italic,
                        });
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build_profile_drawing_prims(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        pane_w_px: i32,
        vpr: f64,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
    ) {
        let Ok(snapshot) = self.profile_drawing_snapshot(drawing.id) else {
            return;
        };
        match snapshot {
            crate::ProfileDrawingSnapshot::Volume(profile) => {
                let Some(options) = drawing.profile.as_ref() else {
                    return;
                };
                let left = px
                    .first()
                    .map_or(0.0, |point| point.0)
                    .clamp(0.0, f64::from(pane_w_px));
                let right = match drawing.kind {
                    DrawingKind::FixedRangeVolumeProfile => px
                        .get(1)
                        .map_or(left, |point| point.0)
                        .clamp(0.0, f64::from(pane_w_px)),
                    _ => f64::from(pane_w_px),
                };
                let range_left = left.min(right);
                let range_right = left.max(right);
                let available = (range_right - range_left).max(1.0) * options.width_percent / 100.0;
                let max_volume = profile
                    .rows
                    .iter()
                    .map(|row| row.total_volume)
                    .fold(0.0_f64, f64::max);
                if max_volume <= 0.0 {
                    return;
                }
                let bid = Color::rgba(247, 82, 95, 150);
                let ask = Color::rgba(8, 153, 129, 150);
                let unknown = Color::rgba(120, 130, 145, 130);
                for row in &profile.rows {
                    let Some((_, y0)) = self.drawing_to_px_for(
                        drawing.pane_index,
                        drawing.price_scale,
                        crate::DrawingPoint {
                            logical: drawing.points[0].logical,
                            price: row.low,
                        },
                    ) else {
                        continue;
                    };
                    let Some((_, y1)) = self.drawing_to_px_for(
                        drawing.pane_index,
                        drawing.price_scale,
                        crate::DrawingPoint {
                            logical: drawing.points[0].logical,
                            price: row.high,
                        },
                    ) else {
                        continue;
                    };
                    let width = available * row.total_volume / max_volume;
                    let x0 = range_right - width;
                    let height = ((y0 - y1).abs() * vpr).round().max(1.0) as i32;
                    let y = (y0.min(y1) * vpr).round() as i32;
                    let mut cursor = x0;
                    for (volume, color) in [
                        (row.bid_volume, bid),
                        (row.unknown_volume, unknown),
                        (row.ask_volume, ask),
                    ] {
                        if volume <= 0.0 {
                            continue;
                        }
                        let segment = width * volume / row.total_volume;
                        out.push(Prim::Rect {
                            rect: IRect {
                                x: (cursor * vpr).round() as i32,
                                y,
                                w: (segment * vpr).round().max(1.0) as i32,
                                h: height,
                            },
                            color,
                        });
                        cursor += segment;
                    }
                }
                if let Some(poc) = profile
                    .poc
                    .and_then(|price| {
                        self.drawing_to_px_for(
                            drawing.pane_index,
                            drawing.price_scale,
                            crate::DrawingPoint {
                                logical: drawing.points[0].logical,
                                price,
                            },
                        )
                    })
                    .map(|(_, y)| y * vpr)
                {
                    out.push(Prim::HLine {
                        y: poc.round() as i32,
                        x0: ((range_right - available) * vpr).round() as i32,
                        x1: (range_right * vpr).round() as i32,
                        width: vpr.round().max(1.0) as i32,
                        style: LineStyle::Solid,
                        color: Color::rgb(245, 166, 35),
                    });
                }
            }
            crate::ProfileDrawingSnapshot::Vwap(values) => {
                let mut center = Vec::with_capacity(values.len());
                let mut upper = Vec::with_capacity(values.len());
                let mut lower = Vec::with_capacity(values.len());
                for value in values {
                    let seconds = value.timestamp_micros.div_euclid(1_000_000) as f64;
                    let Some(logical) = self.time_to_index(seconds, true).map(|index| index as f64)
                    else {
                        continue;
                    };
                    for (price, target) in [
                        (value.vwap, &mut center),
                        (value.upper_band, &mut upper),
                        (value.lower_band, &mut lower),
                    ] {
                        if let Some((x, y)) = self.drawing_to_px_for(
                            drawing.pane_index,
                            drawing.price_scale,
                            crate::DrawingPoint { logical, price },
                        ) {
                            target.push([x as f32 * vpr as f32, y as f32 * vpr as f32]);
                        }
                    }
                }
                let color = drawing.stroke_color();
                super::series_geometry::push_line_stroke(
                    out,
                    points,
                    &center,
                    (drawing.width * vpr) as f32,
                    drawing.style,
                    LineType::Simple,
                    color,
                );
                let band = Color::rgba(color.r(), color.g(), color.b(), color.a().min(150));
                for path in [&upper, &lower] {
                    super::series_geometry::push_line_stroke(
                        out,
                        points,
                        path,
                        vpr.max(1.0) as f32,
                        LineStyle::Dashed,
                        LineType::Simple,
                        band,
                    );
                }
            }
        }
    }

    /// The text run's resolved glyph size (bitmap px, placeholder floor included), aligned
    /// anchor point, and horizontal alignment — shared by the label prim, the container box,
    /// and the focus/hover chrome so every consumer draws the same geometry.
    fn text_run_geometry(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        pane_w_px: i32,
        vpr: f64,
    ) -> (f64, f64, f64, DrawingTextHAlign, f64) {
        let layout = &self.options.get().layout;
        let size = drawing.resolved_text_size(layout.font_size) * vpr;
        let pane = &self.panes[drawing.pane_index];
        let (x, y, align, angle) = ChartEngine::drawing_text_placement(
            drawing,
            px,
            f64::from(pane_w_px),
            pane.top * vpr,
            pane.height * vpr,
            size,
            TEXT_PAD * vpr,
        );
        (size, x, y, align, angle)
    }

    /// The text tool's interaction chrome (hover ring, focus border): a crisp integer-snapped
    /// hollow frame on the SAME box the host's editing wrap draws — the label run (advance ×
    /// 1.2·size, the hit test's line-height convention) padded by the editing chrome's
    /// 2 px border + 4 px padding (drawings.rs `TEXT_CHROME_PAD`). Selection, hover, and
    /// typing mode land on one outline, so entering/leaving the editor moves nothing.
    fn push_text_chrome(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        pane_w_px: i32,
        vpr: f64,
        color: Color,
        out: &mut Vec<Prim>,
    ) {
        let (size, x, y, align, _) = self.text_run_geometry(drawing, px, pane_w_px, vpr);
        let width = self.measure_drawing_text(drawing, size);
        let height = size * 1.2;
        let pad = TEXT_CHROME_PAD * vpr;
        let left = match align {
            DrawingTextHAlign::Left => x,
            DrawingTextHAlign::Center => x - width / 2.0,
            DrawingTextHAlign::Right => x - width,
        };
        let rect = IRect {
            x: (left - pad).round() as i32,
            y: (y - height / 2.0 - pad).round() as i32,
            w: (width + 2.0 * pad).round().max(1.0) as i32,
            h: (height + 2.0 * pad).round().max(1.0) as i32,
        };
        out.push(Prim::RectFrame {
            rect,
            border: (2.0 * vpr).round().max(1.0) as i32,
            color,
        });
    }

    /// One drawing's text label (every tool can carry one): the placement resolves the 3×3
    /// alignment against the tool's reference box, except trend lines, whose slots follow the
    /// actual segment and whose middle slot opens a measured stroke gap. Trend labels emit
    /// `Prim::RotatedText`; other drawing text emits `Prim::Text`. In either contract x is the
    /// aligned edge and y is the vertical center (the IR's middle-baseline convention). Empty
    /// standalone text paints nothing; an empty hovered trend label paints its dedicated
    /// prompt at the canonical label transform. A text tool
    /// with a `box_color`/`box_border_color` gets its container (crisp integer-snapped
    /// `Rect`/`RectFrame` prims behind the run — the public reference's text-box background/border).
    fn build_drawing_text(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        pane_w_px: i32,
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        // Empty text paints nothing. While the host typing-mode editor is open the LABEL and
        // the focus border still paint — the editor wrap is borderless with transparent glyphs,
        // so entering edit cannot lift the text or shift the outline (the public reference's
        // overlay-caret model). Families that own their text lay it out in their parts.
        if drawing
            .kind
            .spec()
            .family
            .is_some_and(|family| family.owns_text)
        {
            return;
        }
        let Some((text, placeholder)) = self.drawing_frame_text(drawing) else {
            return;
        };
        let is_text_tool = drawing.kind == DrawingKind::Text;
        let (size, x, y, align, angle) = self.text_run_geometry(drawing, px, pane_w_px, vpr);
        let layout = &self.options.get().layout;
        let mut color = self.drawing_label_color(drawing);
        if placeholder {
            color = Color::rgba(
                color.r(),
                color.g(),
                color.b(),
                color.a().min(TREND_TEXT_PLACEHOLDER_ALPHA),
            );
        }

        // The container (text tool with a background/border): a box wrapping the run, emitted
        // as the rectangle tool's crisp integer-snapped prims (`Rect` fill + `RectFrame`
        // border) — strong-color thin geometry at fractional positions AA-phases differently
        // between the backends, so the box snaps to whole device px (the public reference's boxes are
        // crisp the same way).
        let box_fill = drawing.box_color.as_deref().and_then(Color::parse_css);
        let box_border = drawing
            .box_border_color
            .as_deref()
            .and_then(Color::parse_css);
        if is_text_tool && (box_fill.is_some() || box_border.is_some()) {
            let width = self.measure_drawing_text(drawing, size);
            let height = size * 1.2;
            let pad = 4.0 * vpr;
            let left = match align {
                DrawingTextHAlign::Left => x,
                DrawingTextHAlign::Center => x - width / 2.0,
                DrawingTextHAlign::Right => x - width,
            };
            let rect = IRect {
                x: (left - pad).round() as i32,
                y: (y - height / 2.0 - pad).round() as i32,
                w: (width + 2.0 * pad).round().max(1.0) as i32,
                h: (height + 2.0 * pad).round().max(1.0) as i32,
            };
            if let Some(fill) = box_fill {
                out.push(Prim::Rect { rect, color: fill });
            }
            if let Some(border) = box_border {
                out.push(Prim::RectFrame {
                    rect,
                    // Browser border semantics: whole device pixels, rounded down, at least one.
                    border: (drawing.box_border_width * vpr).floor().max(1.0) as i32,
                    color: border,
                });
            }
        }

        let text_prim = Prim::RotatedText {
            x: x as f32,
            y: y as f32,
            text: text.to_string(),
            color,
            size: size as f32,
            family: layout.font_family.clone(),
            align: match align {
                DrawingTextHAlign::Left => TextAlign::Left,
                DrawingTextHAlign::Center => TextAlign::Center,
                DrawingTextHAlign::Right => TextAlign::Right,
            },
            weight: drawing.text_weight.unwrap_or(400),
            italic: drawing.text_italic,
            angle: angle as f32,
        };
        if drawing.kind.spec().text_layout == DrawingTextLayout::Segment {
            out.push(text_prim);
        } else if let Prim::RotatedText {
            x,
            y,
            text,
            color,
            size,
            family,
            align,
            weight,
            italic,
            ..
        } = text_prim
        {
            out.push(Prim::Text {
                x,
                y,
                text,
                color,
                size,
                family,
                align,
                weight,
                italic,
            });
        }
    }

    fn build_drawing_labels(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        let Some(anchor) = px.first().copied() else {
            return;
        };
        if drawing.labels.is_empty()
            || drawing
                .kind
                .spec()
                .family
                .is_some_and(|family| family.owns_labels)
        {
            return;
        }
        let color = drawing
            .text_color
            .as_deref()
            .and_then(Color::parse_css)
            .or_else(|| Color::parse_css(&drawing.color))
            .unwrap_or_else(|| Color::rgb(255, 255, 255));
        let size = drawing.resolved_text_size(self.options.get().layout.font_size) * vpr;
        for (index, label) in drawing.labels.iter().enumerate() {
            if !label.visible {
                continue;
            }
            let value = label.text.clone().unwrap_or_else(|| {
                let first = drawing.points.first().map_or(0.0, |point| point.price);
                let second = drawing.points.get(1).map(|point| point.price);
                match label.metric {
                    crate::DrawingLabelMetric::Price => self.price_formatter.format(first),
                    crate::DrawingLabelMetric::PriceChange => second
                        .map(|value| self.price_formatter.format(value - first))
                        .unwrap_or_default(),
                    crate::DrawingLabelMetric::PercentChange => second
                        .filter(|_| first.abs() > f64::EPSILON)
                        .map(|value| format!("{:.2}%", (value - first) / first * 100.0))
                        .unwrap_or_default(),
                    crate::DrawingLabelMetric::Ticks => second
                        .map(|value| format!("{:.4}", value - first))
                        .unwrap_or_default(),
                    crate::DrawingLabelMetric::BarCount => drawing
                        .points
                        .get(1)
                        .map(|value| {
                            format!(
                                "{} bars",
                                (value.logical - drawing.points[0].logical).abs().round() as i64
                            )
                        })
                        .unwrap_or_default(),
                    crate::DrawingLabelMetric::DateTimeRange => "range".to_string(),
                    crate::DrawingLabelMetric::Duration => drawing
                        .points
                        .get(1)
                        .map(|value| {
                            format!(
                                "{:.2} bars",
                                (value.logical - drawing.points[0].logical).abs()
                            )
                        })
                        .unwrap_or_default(),
                    crate::DrawingLabelMetric::Angle => drawing
                        .points
                        .get(1)
                        .map(|value| {
                            let dx = value.logical - drawing.points[0].logical;
                            let dy = value.price - first;
                            format!("{:.1}°", dy.atan2(dx).to_degrees())
                        })
                        .unwrap_or_default(),
                    crate::DrawingLabelMetric::Distance => drawing
                        .points
                        .get(1)
                        .map(|value| {
                            format!(
                                "{:.2}",
                                (value.logical - drawing.points[0].logical)
                                    .hypot(value.price - first)
                            )
                        })
                        .unwrap_or_default(),
                    crate::DrawingLabelMetric::VolumeInRange => "volume".to_string(),
                }
            });
            if value.is_empty() {
                continue;
            }
            let offset = (index as f64 + 1.0) * size * 1.25;
            let y = match label.position {
                crate::DrawingLabelPosition::Above => anchor.1 - offset,
                crate::DrawingLabelPosition::Below => anchor.1 + offset,
                crate::DrawingLabelPosition::Inside | crate::DrawingLabelPosition::On => anchor.1,
                crate::DrawingLabelPosition::Outside => anchor.1 + offset,
            };
            out.push(Prim::Text {
                x: anchor.0 as f32,
                y: y as f32,
                text: value,
                color,
                size: size as f32,
                family: self.options.get().layout.font_family.clone(),
                align: TextAlign::Left,
                weight: drawing.text_weight.unwrap_or(400),
                italic: drawing.text_italic,
            });
        }
    }

    fn build_position_labels(
        &self,
        drawing: &Drawing,
        position: PositionGeometry,
        vpr: f64,
        reward: Color,
        risk: Color,
        out: &mut Vec<Prim>,
    ) {
        let (Some(entry), Some(target), Some(stop)) = (
            drawing.points.first(),
            drawing.points.get(1),
            drawing.points.get(2),
        ) else {
            return;
        };
        let reward_distance = (target.price - entry.price).abs();
        let risk_distance = (entry.price - stop.price).abs();
        let base = entry.price.abs();
        let reward_percent = if base > f64::EPSILON {
            reward_distance / base * 100.0
        } else {
            0.0
        };
        let risk_percent = if base > f64::EPSILON {
            risk_distance / base * 100.0
        } else {
            0.0
        };
        let target_text = format!(
            "Target: {} ({reward_percent:.2}%)",
            self.price_formatter.format(reward_distance)
        );
        let stop_text = format!(
            "Stop: {} ({risk_percent:.2}%)",
            self.price_formatter.format(risk_distance)
        );
        let center_x = (position.left + position.right) / 2.0;
        let label_offset = 14.0 * vpr;
        let target_label_y = if position.target_y < position.entry_y {
            position.target_y - label_offset
        } else {
            position.target_y + label_offset
        };
        let stop_label_y = if position.stop_y < position.entry_y {
            position.stop_y - label_offset
        } else {
            position.stop_y + label_offset
        };
        self.push_position_label_block(out, center_x, target_label_y, &[target_text], reward, vpr);
        self.push_position_label_block(out, center_x, stop_label_y, &[stop_text], risk, vpr);
    }

    /// Dynamic position progress belongs to pane chrome rather than retained drawing geometry:
    /// series updates already invalidate chrome, so the darker traversed fill and terminal-candle
    /// trend can follow data without rebuilding every drawing on each tick.
    pub(super) fn build_position_progress_frame(
        &self,
        pane_index: usize,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
        hpr: f64,
        vpr: f64,
    ) {
        for drawing in self.drawings.iter().filter(|drawing| {
            drawing.pane_index == pane_index
                && matches!(
                    drawing.kind,
                    DrawingKind::LongPosition | DrawingKind::ShortPosition
                )
                && drawing.points.len() == 3
        }) {
            let Some(run) = self.position_run_progress(drawing) else {
                continue;
            };
            let run_point = run.point;
            let run_start = run.start;
            let entry = drawing.points[0].price;
            if !run_point.price.is_finite() || !entry.is_finite() {
                continue;
            }

            let Some(px) = self.drawing_px(drawing) else {
                continue;
            };
            let px = px
                .into_iter()
                .map(|(x, y)| (x * hpr, y * vpr))
                .collect::<Vec<_>>();
            let Some(geometry) = resolve_drawing_geometry(
                drawing.kind,
                &px,
                self.pane_w * hpr,
                self.panes[pane_index].top * vpr,
                self.panes[pane_index].height * vpr,
                DrawingGeometryOptions {
                    line_width: drawing.width,
                    device_scale: vpr,
                    extend_left: drawing.extend_left,
                    extend_right: drawing.extend_right,
                },
            ) else {
                continue;
            };
            let DrawingBodyGeometry::Position(position) = geometry.body else {
                continue;
            };

            // The progress origin is the first post-placement candle that actually reaches/crosses
            // the entry. A position that has not filled emits no progress geometry at all.
            let Some((start_x, _)) =
                self.drawing_to_px_for(pane_index, drawing.price_scale, run_start)
            else {
                continue;
            };
            let Some((run_x, run_y)) =
                self.drawing_to_px_for(pane_index, drawing.price_scale, run_point)
            else {
                continue;
            };
            let start_x = (start_x * hpr).clamp(position.left, position.right);
            let run_x = (run_x * hpr).clamp(position.left, position.right);
            let run_y = run_y * vpr;
            let semantic = match run.side {
                PositionRunSide::Reward => {
                    Color::parse_css(aeris_charts_core::style::MARKET_UP_CSS)
                        .unwrap_or(Color::rgb(8, 153, 129))
                }
                PositionRunSide::Risk => {
                    Color::parse_css(aeris_charts_core::style::MARKET_DOWN_CSS)
                        .unwrap_or(Color::rgb(247, 82, 95))
                }
            };

            // Stronger opacity represents only the price/time space actually travelled since the
            // fill: first-fill x -> current/terminal x, entry y -> current/terminal y. It never
            // darkens the untouched remainder of either TP/SL zone.
            let travel_left = start_x.min(run_x);
            let travel_right = start_x.max(run_x);
            if travel_right > travel_left && (run_y - position.entry_y).abs() > f64::EPSILON {
                push_position_zone_with_alpha(
                    out,
                    PositionZone {
                        left: travel_left,
                        right: travel_right,
                        y0: position.entry_y,
                        y1: run_y,
                    },
                    semantic,
                    POSITION_PROGRESS_ALPHA,
                );
            }
            let progress_path = [
                [start_x as f32, position.entry_y as f32],
                [run_x as f32, run_y as f32],
            ];
            if progress_path[0] != progress_path[1] {
                super::series_geometry::push_line_stroke(
                    out,
                    points,
                    &progress_path,
                    vpr.max(1.0) as f32,
                    LineStyle::Dashed,
                    LineType::Simple,
                    POSITION_ENTRY,
                );
            }
        }
    }

    pub(super) fn position_run_progress(&self, drawing: &Drawing) -> Option<PositionRunProgress> {
        let target = match drawing.price_scale {
            crate::DrawingPriceScale::Right => crate::PriceScaleTarget::Right,
            crate::DrawingPriceScale::Left => crate::PriceScaleTarget::Left,
            crate::DrawingPriceScale::Overlay => crate::PriceScaleTarget::Overlay,
        };
        let series = self.series.iter().find(|series| {
            series.visible
                && !series.removed
                && series.pane_index == drawing.pane_index
                && super::series_scale_target(series) == target
        })?;
        // Custom-series frame values expose only their current value, not historical OHLC
        // extrema. Fabricating a "run" endpoint from that current value would violate the
        // position contract, so only canonical plot-backed series participate here.
        if series.kind == crate::SeriesKind::Custom {
            return None;
        }
        let plot = self.data.plot(series.id);
        let entry = drawing.points.first()?;
        let extent = drawing.points.get(1)?;
        let target_price = extent.price;
        let stop_price = drawing.points.get(2)?.price;
        if !entry.logical.is_finite()
            || !entry.price.is_finite()
            || !extent.logical.is_finite()
            || !target_price.is_finite()
            || !stop_price.is_finite()
            || extent.logical < entry.logical
        {
            return None;
        }
        let first_index = entry.logical.ceil();
        let last_index = extent.logical.floor();
        if first_index < i64::MIN as f64
            || first_index > i64::MAX as f64
            || last_index < i64::MIN as f64
            || last_index > i64::MAX as f64
            || first_index > last_index
        {
            return None;
        }
        let first_row = plot.first_non_whitespace_row(first_index as i64)?;
        let last_row = plot.last_non_whitespace_row(last_index as i64)?;
        if first_row > last_row {
            return None;
        }

        let first_high = plot.value_at(first_row, PlotValueIndex::High);
        let first_low = plot.value_at(first_row, PlotValueIndex::Low);
        if !first_high.is_finite() || !first_low.is_finite() {
            return None;
        }

        // Before fill, entry is approached from whichever side contains the first post-placement
        // candle. A candle already spanning entry fills immediately. Otherwise a one-sided extrema
        // predicate (High >= entry from below, Low <= entry from above) lets the LOD hierarchy find
        // the first touch/cross without scanning every historical candle. A gap across entry is a
        // deterministic OHLC "cross" and is anchored visually at the exact entry level.
        let starts_below = first_high < entry.price;
        let starts_above = first_low > entry.price;
        let fill_row = if !starts_below && !starts_above {
            first_row
        } else {
            let range_crosses_entry = |start: usize, end: usize| {
                let mut crossed = false;
                let mut inspect = |row: usize| {
                    if crossed || plot.is_whitespace_row(row) {
                        return;
                    }
                    let value_index = if starts_below {
                        PlotValueIndex::High
                    } else {
                        PlotValueIndex::Low
                    };
                    let value = plot.value_at(row, value_index);
                    crossed |= value.is_finite()
                        && if starts_below {
                            value >= entry.price
                        } else {
                            value <= entry.price
                        };
                };
                if let Some(lod) = plot.lod() {
                    let (rows, _) = lod.rows_on_range(start..end, usize::MAX);
                    for row in rows.iter() {
                        inspect(row);
                    }
                } else {
                    for row in start..end {
                        inspect(row);
                    }
                }
                crossed
            };
            if !range_crosses_entry(first_row, last_row + 1) {
                return None;
            }
            let mut lo = first_row;
            let mut hi = last_row;
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if range_crosses_entry(first_row, mid + 1) {
                    hi = mid;
                } else {
                    lo = mid + 1;
                }
            }
            lo
        };

        let range_hits = |start: usize, end: usize| {
            let mut target_hit = false;
            let mut stop_hit = false;
            let mut inspect = |row: usize| {
                if plot.is_whitespace_row(row) {
                    return;
                }
                let high = plot.value_at(row, PlotValueIndex::High);
                let low = plot.value_at(row, PlotValueIndex::Low);
                match drawing.kind {
                    DrawingKind::LongPosition => {
                        target_hit |= high.is_finite() && high >= target_price;
                        stop_hit |= low.is_finite() && low <= stop_price;
                    }
                    DrawingKind::ShortPosition => {
                        target_hit |= low.is_finite() && low <= target_price;
                        stop_hit |= high.is_finite() && high >= stop_price;
                    }
                    _ => {}
                }
            };
            if let Some(lod) = plot.lod() {
                let (rows, _) = lod.rows_on_range(start..end, usize::MAX);
                for row in rows.iter() {
                    inspect(row);
                }
            } else {
                for row in start..end {
                    inspect(row);
                }
            }
            (target_hit, stop_hit)
        };

        let (any_target, any_stop) = range_hits(fill_row, last_row + 1);
        let (terminal_row, side, closed) = if any_target || any_stop {
            // Prefix boundary-hit is monotonic, so binary search finds the first candle touching
            // either target or stop without rescanning a long-lived position on every frame.
            let mut lo = fill_row;
            let mut hi = last_row;
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                let (target_hit, stop_hit) = range_hits(fill_row, mid + 1);
                if target_hit || stop_hit {
                    hi = mid;
                } else {
                    lo = mid + 1;
                }
            }
            let (target_hit, stop_hit) = range_hits(lo, lo + 1);
            // OHLC cannot tell intrabar order when both boundaries are touched by one candle.
            // Resolve that ambiguity conservatively as stop-first.
            let side = if stop_hit {
                PositionRunSide::Risk
            } else if target_hit {
                PositionRunSide::Reward
            } else {
                return None;
            };
            (lo, side, true)
        } else {
            let current = plot.value_at(last_row, PlotValueIndex::Close);
            if !current.is_finite() {
                return None;
            }
            let current = current.clamp(target_price.min(stop_price), target_price.max(stop_price));
            let side = match drawing.kind {
                DrawingKind::LongPosition => {
                    if current >= entry.price {
                        PositionRunSide::Reward
                    } else {
                        PositionRunSide::Risk
                    }
                }
                DrawingKind::ShortPosition => {
                    if current <= entry.price {
                        PositionRunSide::Reward
                    } else {
                        PositionRunSide::Risk
                    }
                }
                _ => return None,
            };
            (last_row, side, false)
        };

        let logical = plot.index_at(terminal_row)?;
        let price = if closed {
            match side {
                PositionRunSide::Reward => target_price,
                PositionRunSide::Risk => stop_price,
            }
        } else {
            plot.value_at(terminal_row, PlotValueIndex::Close)
                .clamp(target_price.min(stop_price), target_price.max(stop_price))
        };
        let start_logical = plot.index_at(fill_row)?;
        price.is_finite().then_some(PositionRunProgress {
            start: crate::drawings::DrawingPoint {
                logical: start_logical as f64,
                price: entry.price,
            },
            point: crate::drawings::DrawingPoint {
                logical: logical as f64,
                price,
            },
            side,
        })
    }

    /// Position information labels paint in chrome after the dynamic run overlay. This keeps the
    /// dashed run line visually behind the label chips instead of striking through their text.
    pub(super) fn build_position_labels_frame(
        &self,
        pane_index: usize,
        out: &mut Vec<Prim>,
        hpr: f64,
        vpr: f64,
    ) {
        let reward = Color::parse_css(aeris_charts_core::style::MARKET_UP_CSS)
            .unwrap_or(Color::rgb(8, 153, 129));
        let risk = Color::parse_css(aeris_charts_core::style::MARKET_DOWN_CSS)
            .unwrap_or(Color::rgb(247, 82, 95));
        for drawing in self.drawings.iter().filter(|drawing| {
            drawing.pane_index == pane_index
                && matches!(
                    drawing.kind,
                    DrawingKind::LongPosition | DrawingKind::ShortPosition
                )
                && drawing.points.len() == 3
        }) {
            let Some(px) = self.drawing_px(drawing) else {
                continue;
            };
            let px = px
                .into_iter()
                .map(|(x, y)| (x * hpr, y * vpr))
                .collect::<Vec<_>>();
            let Some(geometry) = resolve_drawing_geometry(
                drawing.kind,
                &px,
                self.pane_w * hpr,
                self.panes[pane_index].top * vpr,
                self.panes[pane_index].height * vpr,
                DrawingGeometryOptions {
                    line_width: drawing.width,
                    device_scale: vpr,
                    extend_left: drawing.extend_left,
                    extend_right: drawing.extend_right,
                },
            ) else {
                continue;
            };
            let DrawingBodyGeometry::Position(position) = geometry.body else {
                continue;
            };
            self.build_position_labels(drawing, position, vpr, reward, risk, out);
        }
    }

    fn push_position_label_block(
        &self,
        out: &mut Vec<Prim>,
        x: f64,
        y: f64,
        lines: &[String],
        background: Color,
        vpr: f64,
    ) {
        if lines.is_empty() {
            return;
        }
        let layout = &self.options.get().layout;
        let size = self.drawing_stats_size() * vpr;
        let line_height = size * 1.25;
        let pad_x = 6.0 * vpr;
        let pad_y = 3.0 * vpr;
        let width = lines
            .iter()
            .map(|line| self.measure_text_run(line, size, &layout.font_family, 400, false))
            .fold(0.0_f64, f64::max)
            + 2.0 * pad_x;
        let height = lines.len() as f64 * line_height + 2.0 * pad_y;
        let rect = IRect {
            x: (x - width / 2.0).round() as i32,
            y: (y - height / 2.0).round() as i32,
            w: width.round().max(1.0) as i32,
            h: height.round().max(1.0) as i32,
        };
        out.push(Prim::Rect {
            rect,
            color: Color::rgba(background.r(), background.g(), background.b(), 224),
        });
        let text_color = if background.luminance() > 175.0 {
            Color::rgb(0, 0, 0)
        } else {
            Color::rgb(255, 255, 255)
        };
        let first_y = y - ((lines.len() as f64 - 1.0) * line_height) / 2.0;
        for (index, text) in lines.iter().enumerate() {
            out.push(Prim::Text {
                x: x as f32,
                y: (first_y + index as f64 * line_height) as f32,
                text: text.clone(),
                color: text_color,
                size: size as f32,
                family: layout.font_family.clone(),
                align: TextAlign::Center,
                weight: 400,
                italic: false,
            });
        }
    }

    /// The anchor-handle fill for the current theme (white on light backgrounds, black on dark —
    /// the series selection anchors' luminance rule, series_geometry.rs).
    fn anchor_fill(&self) -> Color {
        let fallback = aeris_charts_core::style::DEFAULT_SURFACE_RGB;
        let background = Color::parse_css(&self.options.get().layout.background.color)
            .unwrap_or(Color::rgb(fallback.0, fallback.1, fallback.2));
        if background.luminance() > 160.0 {
            Color::rgb(0xff, 0xff, 0xff)
        } else {
            Color::rgb(0, 0, 0)
        }
    }
}

fn push_position_zone(out: &mut Vec<Prim>, zone: PositionZone, color: Color) {
    push_position_zone_with_alpha(out, zone, color, POSITION_ZONE_ALPHA);
}

fn push_position_zone_with_alpha(out: &mut Vec<Prim>, zone: PositionZone, color: Color, alpha: u8) {
    let left = zone.left.round() as i32;
    let right = zone.right.round() as i32;
    let top = zone.y0.min(zone.y1).round() as i32;
    let bottom = zone.y0.max(zone.y1).round() as i32;
    let width = (right - left).abs() + 1;
    let height = (bottom - top).abs() + 1;
    let rect = IRect {
        x: left,
        y: top,
        w: width,
        h: height,
    };
    out.push(Prim::Rect {
        rect,
        color: Color::rgba(color.r(), color.g(), color.b(), alpha),
    });
}

/// One handle per anchor: the border disc underneath, the fill disc on top.
fn build_anchor_handles(px: &[(f64, f64)], vpr: f64, fill: Color, out: &mut Vec<Prim>) {
    for &(cx, cy) in px {
        out.push(Prim::Circle {
            cx: cx as f32,
            cy: cy as f32,
            radius: ((ANCHOR_RADIUS + ANCHOR_BORDER_WIDTH) * vpr) as f32,
            fill: ANCHOR_BORDER,
            stroke_width: 0.0,
            stroke: ANCHOR_BORDER,
        });
        out.push(Prim::Circle {
            cx: cx as f32,
            cy: cy as f32,
            radius: (ANCHOR_RADIUS * vpr) as f32,
            fill,
            stroke_width: 0.0,
            stroke: fill,
        });
    }
}

/// Paint a handle set (`drawings/handles.rs`) in its order: discs are the anchor disc pair,
/// squares the Long/Short Position control pair, and rounded squares the rectangle edge-midpoint
/// pair (2 px corner radius).
fn build_handles(handles: &[DrawingHandle], vpr: f64, fill: Color, out: &mut Vec<Prim>) {
    for handle in handles {
        let (cx, cy) = handle.point;
        match handle.shape {
            HandleShape::Disc => build_anchor_handles(&[handle.point], vpr, fill, out),
            HandleShape::Square => {
                let side = (2.0 * (ANCHOR_RADIUS + ANCHOR_BORDER_WIDTH) * vpr)
                    .round()
                    .max(1.0) as i32;
                let inner_side = (2.0 * ANCHOR_RADIUS * vpr).round().max(1.0) as i32;
                out.push(Prim::Rect {
                    rect: IRect {
                        x: (cx - f64::from(side) / 2.0).round() as i32,
                        y: (cy - f64::from(side) / 2.0).round() as i32,
                        w: side,
                        h: side,
                    },
                    color: ANCHOR_BORDER,
                });
                out.push(Prim::Rect {
                    rect: IRect {
                        x: (cx - f64::from(inner_side) / 2.0).round() as i32,
                        y: (cy - f64::from(inner_side) / 2.0).round() as i32,
                        w: inner_side,
                        h: inner_side,
                    },
                    color: fill,
                });
            }
            HandleShape::RoundedSquare => {
                let outer = ((ANCHOR_RADIUS + ANCHOR_BORDER_WIDTH) * vpr) as f32;
                let inner = (ANCHOR_RADIUS * vpr) as f32;
                let radii = [2.0 * vpr as f32; 4];
                out.push(Prim::RoundRect {
                    x: cx as f32 - outer,
                    y: cy as f32 - outer,
                    w: outer * 2.0,
                    h: outer * 2.0,
                    radii,
                    fill: ANCHOR_BORDER,
                    border_width: 0.0,
                    border_color: ANCHOR_BORDER,
                });
                out.push(Prim::RoundRect {
                    x: cx as f32 - inner,
                    y: cy as f32 - inner,
                    w: inner * 2.0,
                    h: inner * 2.0,
                    radii,
                    fill,
                    border_width: 0.0,
                    border_color: fill,
                });
            }
        }
    }
}
