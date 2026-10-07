//! Backend-neutral composition of the final unscissored chart axis/top layer.
//!
//! Axis label selection and geometry already live in [`crate::AxisFrame`]. This module owns the
//! remaining chart policy that every host must execute identically: watermark placement, axis
//! chrome, tick stubs, pane separators, boxed-label attachment, and text primitive placement.
//! Hosts contribute the exact-font cap-center metric installed on the engine for all middle-
//! anchored chart text.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{IRect, Prim, TextAlign};

use crate::{
    AxisFrame, AxisLabel, AxisTextAlign, AxisTextMidpoint, ChartEngine, axis_metrics::AxisMetrics,
};

impl ChartEngine {
    /// Build the final unscissored axis/top primitive layer into `output`, retaining its capacity.
    ///
    /// The host's installed cap-center metric is sampled at each run's painted size, family,
    /// and weight. Without native metrics the correction is zero.
    pub fn build_axis_primitives_into(&self, axis_frame: &AxisFrame, output: &mut Vec<Prim>) {
        output.clear();
        let dpr = self.dpr;
        let bitmap_w = (self.css_width * dpr).round().max(1.0);
        let pane_left = self.pane_left;
        let pane_w = self.pane_w;
        let pane_h = self.pane_h;
        let options = self.options.get();
        let layout = &options.layout;
        let left_scale = &options.left_price_scale;
        let right_scale = &options.right_price_scale;
        let time_scale = &options.time_scale;
        let watermark = &options.watermark;
        // Project the canonical CSS border width onto the integer device-pixel draw list with
        // browser border semantics: whole device pixels, rounded down, never below one.
        let border_w = aeris_charts_core::style::border_width_device_px(dpr) as i32;
        let parse = |css: &str, fallback: Color| Color::parse_css(css).unwrap_or(fallback);
        let fallback = Color::rgb(
            aeris_charts_core::style::DEFAULT_BORDER_RGB.0,
            aeris_charts_core::style::DEFAULT_BORDER_RGB.1,
            aeris_charts_core::style::DEFAULT_BORDER_RGB.2,
        );
        let left_border = parse(&left_scale.border_color, fallback);
        let right_border = parse(&right_scale.border_color, fallback);
        let time_border = parse(&time_scale.border_color, fallback);

        // Watermark is below chrome and labels, matching the browser host's old overlay slot.
        if watermark.visible && !watermark.text.is_empty() {
            let default_text = aeris_charts_core::style::DEFAULT_AXIS_TEXT_RGB;
            let (x, align) = match watermark.horz_align.as_str() {
                "left" => (pane_left, TextAlign::Left),
                "right" => (pane_left + pane_w, TextAlign::Right),
                _ => (pane_left + pane_w / 2.0, TextAlign::Center),
            };
            let y = match watermark.vert_align.as_str() {
                "top" => watermark.font_size / 2.0,
                "bottom" => pane_h - watermark.font_size / 2.0,
                _ => pane_h / 2.0,
            };
            output.push(Prim::Text {
                x: (x * dpr) as f32,
                y: (y * dpr) as f32,
                text: watermark.text.clone(),
                color: parse(
                    &watermark.color,
                    Color::rgb(default_text.0, default_text.1, default_text.2),
                ),
                size: (watermark.font_size * dpr) as f32,
                family: watermark.font_family.clone(),
                align,
                weight: if watermark.font_style.contains("bold") {
                    700
                } else {
                    400
                },
                italic: watermark.font_style.contains("italic"),
            });
        }

        {
            let mut rect = |x: f64, y: f64, w: f64, h: f64, color: Color| {
                let x0 = x.round() as i32;
                let y0 = y.round() as i32;
                let x1 = (x + w).round() as i32;
                let y1 = (y + h).round() as i32;
                if x1 > x0 && y1 > y0 {
                    output.push(Prim::Rect {
                        rect: IRect {
                            x: x0,
                            y: y0,
                            w: x1 - x0,
                            h: y1 - y0,
                        },
                        color,
                    });
                }
            };

            for band in &axis_frame.bands {
                rect(
                    band.x * dpr,
                    band.y * dpr,
                    band.width * dpr,
                    band.height * dpr,
                    band.color,
                );
            }

            if self.left_axis_w > 0.0 && left_scale.border_visible {
                rect(
                    (pane_left * dpr).round() - f64::from(border_w),
                    0.0,
                    f64::from(border_w),
                    (pane_h * dpr).round(),
                    left_border,
                );
            }
            if self.axis_w > 0.0 && right_scale.border_visible {
                rect(
                    ((pane_left + pane_w) * dpr).round(),
                    0.0,
                    f64::from(border_w),
                    (pane_h * dpr).round(),
                    right_border,
                );
            }
            if time_scale.border_visible && self.time_axis_visible {
                rect(
                    0.0,
                    (pane_h * dpr).round(),
                    bitmap_w,
                    f64::from(border_w),
                    time_border,
                );
            }

            let tick_len = (AxisMetrics::TICK_LENGTH * dpr).round();
            let tick_off = (dpr * 0.5).floor();
            for tick in &axis_frame.price_ticks {
                let (enabled, color) = if tick.left {
                    (left_scale.border_visible, left_border)
                } else {
                    (right_scale.border_visible, right_border)
                };
                if enabled {
                    rect(
                        (tick.x * dpr).round(),
                        (tick.y * dpr).round() - tick_off,
                        tick_len,
                        f64::from(border_w),
                        color,
                    );
                }
            }
            if time_scale.border_visible && self.time_ticks_visible && self.time_axis_visible {
                let y0 = (pane_h * dpr).round();
                for x in &axis_frame.time_ticks {
                    rect(
                        (x * dpr).round() - tick_off,
                        y0,
                        f64::from(border_w),
                        tick_len,
                        time_border,
                    );
                }
            }

            // A pane boundary is a structural divider, not plot chrome: it spans the complete
            // chart width, crossing every visible left/right price-scale strip, so the resting
            // line describes the same full-width boundary as the hover band and the hit test.
            // The rule fills the whole `PANE_SEPARATOR` layout slot on the device grid, so it
            // never leaves an unpainted gap. Resolve its height once per DPR instead of snapping
            // both edges independently: at fractional DPRs edge snapping can otherwise make two
            // identical separators alternate between adjacent device-pixel thicknesses.
            let separator_color = parse(&layout.panes.separator_color, right_border);
            let separator_h = (crate::PANE_SEPARATOR * dpr).round().max(1.0);
            for separator in &axis_frame.separators {
                let y0 = (separator * dpr).round();
                rect(0.0, y0, bitmap_w, separator_h, separator_color);
            }
            if let Some(separator) = axis_frame
                .separator_hover
                .and_then(|index| axis_frame.separators.get(index))
            {
                // The hover band extends 4 CSS px beyond both sides of the separator rule.
                let y0 = ((separator - 4.0) * dpr).round();
                rect(
                    0.0,
                    y0,
                    bitmap_w,
                    ((crate::PANE_SEPARATOR + 8.0) * dpr).round().max(1.0),
                    parse(&layout.panes.separator_hover_color, separator_color),
                );
            }
        }

        let append_text = |label: &AxisLabel, output: &mut Vec<Prim>| {
            let size = layout.font_size * label.font_scale;
            let weight = if label.bold { 700 } else { 400 };
            let correction = if label.midpoint == AxisTextMidpoint::None || label.text.is_empty() {
                0.0
            } else {
                self.text_cap_center(size, &layout.font_family, weight, false)
            };
            output.push(Prim::Text {
                x: (label.x * dpr) as f32,
                y: ((label.y + correction) * dpr) as f32,
                text: label.text.clone(),
                color: label.color,
                size: (size * dpr) as f32,
                family: layout.font_family.clone(),
                align: match label.align {
                    AxisTextAlign::Left => TextAlign::Left,
                    AxisTextAlign::Right => TextAlign::Right,
                    AxisTextAlign::Center => TextAlign::Center,
                },
                weight,
                italic: false,
            });
        };
        for label in axis_frame
            .labels
            .iter()
            .filter(|label| label.background.is_none())
        {
            append_text(label, output);
        }
        for label in &axis_frame.rotated_labels {
            output.push(Prim::RotatedText {
                x: (label.x * dpr) as f32,
                y: (label.y * dpr) as f32,
                text: label.text.clone(),
                color: label.color,
                size: (layout.font_size * label.font_scale * dpr) as f32,
                family: layout.font_family.clone(),
                align: match label.align {
                    AxisTextAlign::Left => TextAlign::Left,
                    AxisTextAlign::Right => TextAlign::Right,
                    AxisTextAlign::Center => TextAlign::Center,
                },
                weight: if label.bold { 700 } else { 400 },
                italic: false,
                angle: label.angle as f32,
            });
        }

        let mut last_attach: Option<(u32, f64)> = None;
        for label in axis_frame
            .labels
            .iter()
            .filter(|label| label.background.is_some())
        {
            if let Some((x, y, w, h, color)) = label.background {
                let mut bx = (x * dpr).round();
                let mut by = match (label.attach_group, last_attach) {
                    (Some(group), Some((previous, bottom))) if group == previous => bottom,
                    _ => (y * dpr).round(),
                };
                let mut right = ((x + w) * dpr).round();
                let bottom = ((y + h) * dpr).round();
                match (label.align, label.midpoint) {
                    // The price box always starts one border thickness past the strip boundary,
                    // leaving a seam between it and the title chip that ends on the boundary.
                    // The reservation is unconditional: with the axis border visible the border
                    // paints INTO that seam (the line reads between the two chips), and with it
                    // hidden the chart surface shows through instead. Gating it on the border
                    // would butt the two chips into one solid bar whenever the border is off.
                    (AxisTextAlign::Left, AxisTextMidpoint::Label) => {
                        bx = bx.max(((pane_left + pane_w) * dpr).round() + f64::from(border_w));
                    }
                    (AxisTextAlign::Right, AxisTextMidpoint::Label) => {
                        right = right.min((pane_left * dpr).round() - f64::from(border_w));
                    }
                    (AxisTextAlign::Center, AxisTextMidpoint::StableTime)
                        if time_scale.border_visible =>
                    {
                        by = by.max((pane_h * dpr).round() + f64::from(border_w));
                    }
                    _ => {}
                }
                let bw = right - bx;
                let bh = bottom - by;
                last_attach = label.attach_group.map(|group| (group, by + bh));
                // A hollow chip's outline remains its own explicit semantic width; axis chrome
                // above independently uses the canonical design-system border token.
                let border = label
                    .border
                    .map(|(width, color)| ((width * dpr).floor().max(1.0), color));
                if let Some((border_width, border_color)) = border {
                    if label.background_corners.is_empty() {
                        output.push(Prim::Rect {
                            rect: IRect {
                                x: bx as i32,
                                y: by as i32,
                                w: bw as i32,
                                h: bh as i32,
                            },
                            color: border_color,
                        });
                        output.push(Prim::Rect {
                            rect: IRect {
                                x: (bx + border_width) as i32,
                                y: (by + border_width) as i32,
                                w: (bw - border_width * 2.0).max(0.0) as i32,
                                h: (bh - border_width * 2.0).max(0.0) as i32,
                            },
                            color,
                        });
                    } else {
                        let corners = label.background_corners;
                        let radius = (AxisMetrics::TAG_RADIUS * dpr) as f32;
                        output.push(Prim::RoundRect {
                            x: bx as f32,
                            y: by as f32,
                            w: bw as f32,
                            h: bh as f32,
                            radii: [
                                if corners.top_left { radius } else { 0.0 },
                                if corners.top_right { radius } else { 0.0 },
                                if corners.bottom_right { radius } else { 0.0 },
                                if corners.bottom_left { radius } else { 0.0 },
                            ],
                            fill: border_color,
                            border_width: 0.0,
                            border_color,
                        });
                        let inner_radius = (radius - border_width as f32).max(0.0);
                        output.push(Prim::RoundRect {
                            x: (bx + border_width) as f32,
                            y: (by + border_width) as f32,
                            w: (bw - border_width * 2.0).max(0.0) as f32,
                            h: (bh - border_width * 2.0).max(0.0) as f32,
                            radii: [
                                if corners.top_left { inner_radius } else { 0.0 },
                                if corners.top_right { inner_radius } else { 0.0 },
                                if corners.bottom_right {
                                    inner_radius
                                } else {
                                    0.0
                                },
                                if corners.bottom_left {
                                    inner_radius
                                } else {
                                    0.0
                                },
                            ],
                            fill: color,
                            border_width: 0.0,
                            border_color,
                        });
                    }
                } else if label.background_corners.is_empty() {
                    output.push(Prim::Rect {
                        rect: IRect {
                            x: bx as i32,
                            y: by as i32,
                            w: bw as i32,
                            h: bh as i32,
                        },
                        color,
                    });
                } else {
                    let corners = label.background_corners;
                    let radius = (AxisMetrics::TAG_RADIUS * dpr) as f32;
                    output.push(Prim::RoundRect {
                        x: bx as f32,
                        y: by as f32,
                        w: bw as f32,
                        h: bh as f32,
                        radii: [
                            if corners.top_left { radius } else { 0.0 },
                            if corners.top_right { radius } else { 0.0 },
                            if corners.bottom_right { radius } else { 0.0 },
                            if corners.bottom_left { radius } else { 0.0 },
                        ],
                        fill: color,
                        border_width: 0.0,
                        border_color: Color::rgba(0, 0, 0, 0),
                    });
                }
            } else {
                last_attach = None;
            }
            append_text(label, output);
        }
        // The same original SVG pixels paint above the chip on every backend. Snap the
        // destination to the pixel grid so the image sampler cannot blur the stroke.
        if let Some(icon) = &axis_frame.crosshair_action_icon {
            output.push(Prim::Image {
                image: icon.image.clone(),
                rect: [
                    (icon.x * dpr).round() as f32,
                    (icon.y * dpr).round() as f32,
                    (icon.side * dpr).round().max(1.0) as f32,
                    (icon.side * dpr).round().max(1.0) as f32,
                ],
                opacity: 1.0,
            });
        }
    }
}
