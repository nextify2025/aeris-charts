//! Timeline-mark lane geometry: tokens in the pane chrome layer, the hover ring and the dwell
//! tooltip in the overlay layer. Every token derives from constants (never measured text), so the
//! lane is pixel-identical on every executor; clustering happened in CSS media space inside the
//! shared `lane_layout`, and snapping to device pixels happens only here.

use super::*;
use crate::TimelineGlyphShape;
use crate::timeline_marks::{
    TIMELINE_GLYPH_TEXT_CSS, TIMELINE_LANE_BOTTOM_GAP_CSS, TIMELINE_LANE_HEIGHT_CSS,
};

/// Hover ring radius beyond the token box (CSS px).
const RING_INSET_CSS: f64 = 2.0;
/// Square token corner radius (CSS px).
const SQUARE_RADIUS_CSS: f64 = 2.0;

impl ChartEngine {
    /// Lane tokens for `pane_index` (only the primary series' pane carries the lane).
    pub(crate) fn build_timeline_marks_frame(
        &self,
        pane_index: usize,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        let layout = self.lane_layout();
        if layout.pane != Some(pane_index) {
            return;
        }
        let family = self.options.get().layout.font_family.clone();
        let text_color = self.primary_text_color();
        let surface = self.timeline_surface_color();
        let marks = &self.timeline_marks().marks;
        for token in &layout.tokens {
            let first = &marks[token.marks[0]];
            let size = token.size_css();
            let (fill, shape, border) = if token.mixed() {
                (surface, TimelineGlyphShape::Square, Some(text_color))
            } else {
                (
                    Color::parse_css(&first.glyph.color).unwrap_or(text_color),
                    first.glyph.shape,
                    None,
                )
            };
            let cx = (token.x * hpr).round() as f32;
            let cy = (layout.center_y * vpr).round() as f32;
            Self::push_timeline_glyph(
                out,
                shape,
                token.x,
                layout.center_y,
                size,
                fill,
                border.map(|color| (color, Self::timeline_border_width(vpr) as f32)),
                hpr,
                vpr,
            );
            let text = if token.count() > 1 {
                if token.count() >= 100 {
                    "99+".to_string()
                } else {
                    token.count().to_string()
                }
            } else {
                first.glyph.letter.clone()
            };
            if text.is_empty() {
                continue;
            }
            let color = if token.mixed() {
                text_color
            } else {
                fill.contrast_text()
            };
            // The pin's disc sits above its tip, so its glyph text rides the disc.
            let text_cy = if shape == TimelineGlyphShape::Pin {
                ((layout.center_y - size * 0.15) * vpr).round() as f32
            } else {
                cy
            };
            out.push(Prim::Text {
                x: cx,
                y: text_cy,
                text,
                color,
                size: (TIMELINE_GLYPH_TEXT_CSS * vpr) as f32,
                family: family.clone(),
                align: TextAlign::Center,
                weight: 600,
                italic: false,
            });
        }
    }

    /// The hovered token's ring and, once the host armed it after the dwell, the title tooltip.
    pub(crate) fn build_timeline_hover_frame(
        &self,
        pane_index: usize,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        let Some(token) = self.hovered_timeline_token() else {
            return;
        };
        let layout = self.lane_layout();
        if layout.pane != Some(pane_index) {
            return;
        }
        let center_y = layout.center_y;
        drop(layout);
        let text_color = self.primary_text_color();
        let size = token.size_css() + 2.0 * RING_INSET_CSS;
        let round = token.count() == 1
            && matches!(
                self.timeline_marks().marks[token.marks[0]].glyph.shape,
                TimelineGlyphShape::Circle | TimelineGlyphShape::Pin
            )
            && !token.mixed();
        let device = DeviceBox::snap(
            token.x - size / 2.0,
            center_y - size / 2.0,
            size,
            size,
            hpr,
            vpr,
        );
        let radius = if round {
            device.w.min(device.h) / 2.0
        } else {
            ((SQUARE_RADIUS_CSS + RING_INSET_CSS) * hpr.min(vpr)) as f32
        };
        out.push(Prim::RoundRect {
            x: device.x,
            y: device.y,
            w: device.w,
            h: device.h,
            radii: [radius; 4],
            fill: Color::rgba(0, 0, 0, 0),
            border_width: Self::timeline_border_width(vpr) as f32,
            border_color: text_color,
        });
        if !self.timeline_marks.tooltip_armed {
            return;
        }
        let hit = self.timeline_mark_hit_at(token.x, center_y);
        let Some(hit) = hit else {
            return;
        };
        let text = hit.tooltip_text();
        if text.is_empty() {
            return;
        }
        let font_size = self.options.get().layout.font_size;
        let height = Self::trading_tooltip_height(font_size);
        let pane_top = self.panes.get(pane_index).map_or(0.0, |pane| pane.top);
        let lane_top = center_y - TIMELINE_LANE_HEIGHT_CSS / 2.0;
        let above = lane_top - height - TIMELINE_LANE_BOTTOM_GAP_CSS;
        let y = if above >= pane_top + 2.0 {
            above
        } else {
            pane_top + 2.0
        };
        self.push_trading_tooltip_box(out, &text, token.x, y, hpr, vpr);
    }

    fn timeline_surface_color(&self) -> Color {
        let fallback = aeris_charts_core::style::DEFAULT_SURFACE_RGB;
        Color::parse_css(&self.options.get().layout.background.color)
            .unwrap_or(Color::rgb(fallback.0, fallback.1, fallback.2))
            .solid()
    }

    fn timeline_border_width(vpr: f64) -> f64 {
        aeris_charts_core::style::border_width_device_px(vpr)
    }

    /// One token glyph centered on `(x, y)` CSS px with side `size` CSS px.
    #[allow(clippy::too_many_arguments)]
    fn push_timeline_glyph(
        out: &mut Vec<Prim>,
        shape: TimelineGlyphShape,
        x: f64,
        y: f64,
        size: f64,
        fill: Color,
        border: Option<(Color, f32)>,
        hpr: f64,
        vpr: f64,
    ) {
        let cx = (x * hpr).round() as f32;
        let cy = (y * vpr).round() as f32;
        let half_x = (size / 2.0 * hpr) as f32;
        let half_y = (size / 2.0 * vpr) as f32;
        match shape {
            TimelineGlyphShape::Circle => out.push(Prim::Circle {
                cx,
                cy,
                radius: half_x.min(half_y),
                fill,
                stroke_width: border.map_or(0.0, |(_, width)| width),
                stroke: border.map_or(fill, |(color, _)| color),
            }),
            TimelineGlyphShape::Square => {
                let device = DeviceBox::snap(x - size / 2.0, y - size / 2.0, size, size, hpr, vpr);
                out.push(Prim::RoundRect {
                    x: device.x,
                    y: device.y,
                    w: device.w,
                    h: device.h,
                    radii: [(SQUARE_RADIUS_CSS * hpr.min(vpr)) as f32; 4],
                    fill,
                    border_width: border.map_or(0.0, |(_, width)| width),
                    border_color: border.map_or(fill, |(color, _)| color),
                });
            }
            TimelineGlyphShape::Diamond => {
                out.push(Prim::Triangle {
                    a: [cx, cy - half_y],
                    b: [cx - half_x, cy],
                    c: [cx + half_x, cy],
                    color: fill,
                });
                out.push(Prim::Triangle {
                    a: [cx - half_x, cy],
                    b: [cx + half_x, cy],
                    c: [cx, cy + half_y],
                    color: fill,
                });
            }
            TimelineGlyphShape::Pin => {
                let radius = (size * 0.34 * hpr.min(vpr)) as f32;
                let disc_cy = ((y - size * 0.15) * vpr).round() as f32;
                out.push(Prim::Circle {
                    cx,
                    cy: disc_cy,
                    radius,
                    fill,
                    stroke_width: 0.0,
                    stroke: fill,
                });
                let wing = radius * 0.85;
                out.push(Prim::Triangle {
                    a: [cx - wing, disc_cy + radius * 0.5],
                    b: [cx + wing, disc_cy + radius * 0.5],
                    c: [cx, cy + half_y],
                    color: fill,
                });
            }
        }
    }
}
