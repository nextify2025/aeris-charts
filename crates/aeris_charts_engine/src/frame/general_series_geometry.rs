use aeris_charts_core::scale::general_scale::{BandScale, LinearScale, PointScale};
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{Gradient, IRect, LineStyle, LineType, Prim, TextAlign};
use aeris_charts_render::line::push_styled_stroke;
use aeris_charts_render::shape::Rect;

use crate::general_axes::NumericAxisScale;
use crate::{
    AxisDimension, ChartEngine, DEFAULT_LINE_COLOR, GeneralAxisDomain, GeneralPointSymbol,
    GeneralReferenceOptions, GeneralReferenceValue, GeneralScaleType, GeneralSeriesKind,
};

use super::PRIMARY;

const GENERAL_HOVER: Color = Color(PRIMARY.0 & 0xFFFF_FF00 | 0x73);
const GENERAL_BRUSH: Color = Color(PRIMARY.0 & 0xFFFF_FF00 | 0x28);
const GENERAL_BRUSH_EDGE: Color = Color(PRIMARY.0 & 0xFFFF_FF00 | 0x8F);
const GENERAL_REFERENCE_REGION: Color = Color(PRIMARY.0 & 0xFFFF_FF00 | 0x20);
const GENERAL_REFERENCE_MARK: Color = Color(PRIMARY.0 & 0xFFFF_FF00 | 0xCC);
const GENERAL_LINE_WIDTH_CSS: f64 = 2.0;
const MAX_GENERAL_DATA_LABELS_PER_PANE: usize = 512;
const MAX_GENERAL_DATA_LABEL_ATTEMPTS_PER_PANE: usize = 4_096;

fn opacity_alpha(opacity: f64) -> u8 {
    (opacity.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// A series stroke over the pooled points `[first, first + count)` (shared with its area or band
/// fill). A solid style strokes them as they are; a dashed or dotted one reaches executors as
/// solid dash runs clipped to the pane, like every engine-owned dashed stroke, so the dash count
/// follows the visible length.
fn push_general_stroke(
    out: &mut Vec<Prim>,
    points: &mut Vec<[f32; 2]>,
    (first, count): (u32, u32),
    line_type: LineType,
    (width, style, color): (f32, LineStyle, Color),
    pane: Rect,
) {
    if style == LineStyle::Solid {
        out.push(Prim::Polyline {
            first_point: first,
            point_count: count,
            width,
            style,
            line_type,
            color,
        });
        return;
    }
    let run: Vec<(f64, f64)> = points[first as usize..(first + count) as usize]
        .iter()
        .map(|point| (f64::from(point[0]), f64::from(point[1])))
        .collect();
    push_styled_stroke(out, points, &run, line_type, (width, style, color), pane);
}

fn push_general_point_symbol(
    out: &mut Vec<Prim>,
    symbol: GeneralPointSymbol,
    x: f64,
    y: f64,
    radius: f64,
    color: Color,
) {
    let (x, y, radius) = (x as f32, y as f32, radius as f32);
    match symbol {
        GeneralPointSymbol::Circle => out.push(Prim::Circle {
            cx: x,
            cy: y,
            radius,
            fill: color,
            stroke_width: 0.0,
            stroke: color,
        }),
        GeneralPointSymbol::Square => out.push(Prim::RoundRect {
            x: x - radius,
            y: y - radius,
            w: radius * 2.0,
            h: radius * 2.0,
            radii: [0.0; 4],
            fill: color,
            border_width: 0.0,
            border_color: color,
        }),
        GeneralPointSymbol::Diamond => {
            let top = [x, y - radius];
            let right = [x + radius, y];
            let bottom = [x, y + radius];
            let left = [x - radius, y];
            out.push(Prim::Triangle {
                a: top,
                b: right,
                c: bottom,
                color,
            });
            out.push(Prim::Triangle {
                a: top,
                b: bottom,
                c: left,
                color,
            });
        }
        GeneralPointSymbol::Triangle => out.push(Prim::Triangle {
            a: [x, y - radius],
            b: [x + radius, y + radius],
            c: [x - radius, y + radius],
            color,
        }),
    }
}

impl ChartEngine {
    pub(super) fn build_general_series_frame(
        &self,
        pane_index: usize,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
    ) -> [Option<Prim>; 2] {
        let mut interaction = [None, None];
        let Some(pane_id) = self.pane_stable_id(pane_index) else {
            return interaction;
        };
        let Some(plot) = self.general_plot_rect(pane_index) else {
            return interaction;
        };
        let Some(pane) = self.panes.get(pane_index) else {
            return interaction;
        };
        // Series strokes clip to the pane when they lower dashes (pane-local x).
        let pane_clip = Rect {
            left: 0.0,
            top: pane.top * vpr,
            right: plot.width * hpr,
            bottom: (pane.top + pane.height) * vpr,
        };
        let layout = &self.options.get().layout;
        let label_size = layout.font_size.max(1.0);
        let label_color = self.primary_text_color();
        let mut occupied_labels: Vec<[f64; 4]> = Vec::new();
        let mut label_primitives = Vec::new();
        let mut label_attempts = 0;
        self.push_general_reference_regions(pane_id, plot, hpr, vpr, out);
        if let Some((dimension, from, to)) = self.general_brush_axis_bounds(pane_index) {
            match dimension {
                AxisDimension::X => {
                    let left = (from * hpr).round() as i32;
                    let right = (to * hpr).round() as i32;
                    let top = (plot.y * vpr).round() as i32;
                    let bottom = ((plot.y + plot.height) * vpr).round() as i32;
                    out.push(Prim::Rect {
                        rect: IRect {
                            x: left,
                            y: top,
                            w: (right - left).max(1),
                            h: (bottom - top).max(1),
                        },
                        color: GENERAL_BRUSH,
                    });
                    for x in [from, to] {
                        let x = (x * hpr).round() as i32;
                        out.push(Prim::VLine {
                            x,
                            y0: top,
                            y1: bottom,
                            width: hpr.min(vpr).round().max(1.0) as i32,
                            style: LineStyle::Solid,
                            color: GENERAL_BRUSH_EDGE,
                        });
                    }
                }
                AxisDimension::Y => {
                    let top = (from * vpr).round() as i32;
                    let bottom = (to * vpr).round() as i32;
                    let right = (plot.width * hpr).round() as i32;
                    out.push(Prim::Rect {
                        rect: IRect {
                            x: 0,
                            y: top,
                            w: right.max(1),
                            h: (bottom - top).max(1),
                        },
                        color: GENERAL_BRUSH,
                    });
                    for y in [from, to] {
                        let y = (y * vpr).round() as i32;
                        out.push(Prim::HLine {
                            y,
                            x0: 0,
                            x1: right,
                            width: hpr.min(vpr).round().max(1.0) as i32,
                            style: LineStyle::Solid,
                            color: GENERAL_BRUSH_EDGE,
                        });
                    }
                }
                AxisDimension::Angle | AxisDimension::Radius => {}
            }
        }
        for series in self
            .general_series_iter()
            .filter(|series| series.visible() && series.pane_id() == pane_id)
        {
            let Some(dataset) = self.general_dataset(series.dataset()) else {
                continue;
            };
            let mut push_label = |row: usize, x: f64, above: f64, below: f64, width_limit: f64| {
                if !series.data_labels()
                    || occupied_labels.len() >= MAX_GENERAL_DATA_LABELS_PER_PANE
                    || label_attempts >= MAX_GENERAL_DATA_LABEL_ATTEMPTS_PER_PANE
                {
                    return;
                }
                label_attempts += 1;
                let text = dataset
                    .row_label(row)
                    .map(str::to_owned)
                    .unwrap_or_else(|| dataset.y()[row].to_string());
                let width =
                    self.measure_text_run(&text, label_size, &layout.font_family, 400, false);
                if !width.is_finite() || width <= 0.0 || width > width_limit {
                    return;
                }
                let left = x - width * 0.5;
                let right = x + width * 0.5;
                if left < 2.0 || right > plot.width - 2.0 {
                    return;
                }
                for y in [above, below] {
                    let top = y - label_size * 0.55;
                    let bottom = y + label_size * 0.55;
                    if top < plot.y + 2.0 || bottom > plot.y + plot.height - 2.0 {
                        continue;
                    }
                    let rect = [left, top, right, bottom];
                    if occupied_labels.iter().any(|other| {
                        rect[0] < other[2] + 2.0
                            && rect[2] > other[0] - 2.0
                            && rect[1] < other[3] + 2.0
                            && rect[3] > other[1] - 2.0
                    }) {
                        continue;
                    }
                    occupied_labels.push(rect);
                    label_primitives.push(Prim::Text {
                        x: (x * hpr) as f32,
                        y: (y * vpr) as f32,
                        text,
                        color: label_color,
                        size: (label_size * vpr) as f32,
                        family: layout.font_family.clone(),
                        align: TextAlign::Center,
                        weight: 400,
                        italic: false,
                    });
                    break;
                }
            };
            match series.kind() {
                GeneralSeriesKind::XyLine | GeneralSeriesKind::XyArea => {
                    let color = series
                        .color()
                        .and_then(Color::parse_css)
                        .unwrap_or(DEFAULT_LINE_COLOR);
                    let is_area = series.kind() == GeneralSeriesKind::XyArea;
                    if is_area && series.stack_id().is_some() {
                        let fill = Color::rgba(
                            color.r(),
                            color.g(),
                            color.b(),
                            opacity_alpha(series.fill_opacity() * (7.0 / 9.0)),
                        );
                        let mut upper = Vec::<[f32; 2]>::new();
                        let mut lower = Vec::<[f32; 2]>::new();
                        let mut markers = Vec::new();
                        let mut flush_run =
                            |upper: &mut Vec<[f32; 2]>, lower: &mut Vec<[f32; 2]>| {
                                if upper.len() >= 2 && upper.len() == lower.len() {
                                    let upper_first = points.len() as u32;
                                    let point_count = upper.len() as u32;
                                    points.append(upper);
                                    let lower_first = points.len() as u32;
                                    points.append(lower);
                                    out.push(Prim::BandFill {
                                        upper_first,
                                        lower_first,
                                        point_count,
                                        line_type: series.interpolation().render_type(),
                                        fill,
                                    });
                                    push_general_stroke(
                                        out,
                                        points,
                                        (upper_first, point_count),
                                        series.interpolation().render_type(),
                                        (
                                            (series.line_width() * vpr) as f32,
                                            series.line_style().render_style(),
                                            color,
                                        ),
                                        pane_clip,
                                    );
                                } else {
                                    upper.clear();
                                    lower.clear();
                                }
                            };
                        self.visit_general_stacked_area_points(series, |geometry| {
                            if geometry.starts_new_run {
                                flush_run(&mut upper, &mut lower);
                            }
                            upper.push([(geometry.x * hpr) as f32, (geometry.high_y * vpr) as f32]);
                            lower.push([(geometry.x * hpr) as f32, (geometry.low_y * vpr) as f32]);
                            if series.point_markers() {
                                push_general_point_symbol(
                                    &mut markers,
                                    series.point_symbol(),
                                    geometry.x * hpr,
                                    geometry.high_y * vpr,
                                    series.point_radius() * vpr,
                                    color,
                                );
                            }
                            push_label(
                                geometry.row,
                                geometry.x,
                                geometry.low_y.min(geometry.high_y) - label_size * 0.65 - 4.0,
                                geometry.low_y.max(geometry.high_y) + label_size * 0.65 + 4.0,
                                plot.width - 4.0,
                            );
                            let (hovered, selected) =
                                self.general_row_interaction(series.id(), geometry.row);
                            if hovered || selected {
                                let stroke = if selected { PRIMARY } else { GENERAL_HOVER };
                                interaction[usize::from(selected)] = Some(Prim::Circle {
                                    cx: (geometry.x * hpr) as f32,
                                    cy: (geometry.high_y * vpr) as f32,
                                    radius: ((if selected { 5.0 } else { 4.0 }) * vpr) as f32,
                                    fill: Color::rgba(0, 0, 0, 0),
                                    stroke_width: (if selected { 2.0 } else { 1.0 }) * vpr as f32,
                                    stroke,
                                });
                            }
                        });
                        flush_run(&mut upper, &mut lower);
                        out.append(&mut markers);
                        continue;
                    }
                    let baseline_y = is_area
                        .then(|| self.general_path_baseline_y(series))
                        .flatten();
                    let mut run = Vec::<[f32; 2]>::new();
                    let mut markers = Vec::new();
                    let mut flush_run = |run: &mut Vec<[f32; 2]>| {
                        if run.len() >= 2 {
                            let first_point = points.len() as u32;
                            let point_count = run.len() as u32;
                            points.append(run);
                            if let Some(base_y) = baseline_y {
                                out.push(Prim::AreaFill {
                                    first_point,
                                    point_count,
                                    base_y: (base_y * vpr) as f32,
                                    line_type: series.interpolation().render_type(),
                                    gradient: Gradient {
                                        top: Color::rgba(
                                            color.r(),
                                            color.g(),
                                            color.b(),
                                            opacity_alpha(series.fill_opacity()),
                                        ),
                                        bottom: Color::rgba(
                                            color.r(),
                                            color.g(),
                                            color.b(),
                                            opacity_alpha(series.fill_opacity() / 3.0),
                                        ),
                                    },
                                });
                            }
                            push_general_stroke(
                                out,
                                points,
                                (first_point, point_count),
                                series.interpolation().render_type(),
                                (
                                    (series.line_width() * vpr) as f32,
                                    series.line_style().render_style(),
                                    color,
                                ),
                                pane_clip,
                            );
                        } else {
                            run.clear();
                        }
                    };
                    self.visit_general_path_points(series, |geometry| {
                        if geometry.starts_new_run {
                            flush_run(&mut run);
                        }
                        run.push([(geometry.x * hpr) as f32, (geometry.y * vpr) as f32]);
                        if series.point_markers() {
                            push_general_point_symbol(
                                &mut markers,
                                series.point_symbol(),
                                geometry.x * hpr,
                                geometry.y * vpr,
                                series.point_radius() * vpr,
                                color,
                            );
                        }
                        push_label(
                            geometry.row,
                            geometry.x,
                            geometry.y - label_size * 0.65 - 4.0,
                            geometry.y + label_size * 0.65 + 4.0,
                            plot.width - 4.0,
                        );
                        let (hovered, selected) =
                            self.general_row_interaction(series.id(), geometry.row);
                        if hovered || selected {
                            let stroke = if selected { PRIMARY } else { GENERAL_HOVER };
                            interaction[usize::from(selected)] = Some(Prim::Circle {
                                cx: (geometry.x * hpr) as f32,
                                cy: (geometry.y * vpr) as f32,
                                radius: ((if selected { 5.0 } else { 4.0 }) * vpr) as f32,
                                fill: Color::rgba(0, 0, 0, 0),
                                stroke_width: (if selected { 2.0 } else { 1.0 }) * vpr as f32,
                                stroke,
                            });
                        }
                    });
                    flush_run(&mut run);
                    out.append(&mut markers);
                }
                GeneralSeriesKind::RangeBar => {
                    let color = series
                        .color()
                        .and_then(Color::parse_css)
                        .unwrap_or(DEFAULT_LINE_COLOR);
                    self.visit_general_range_bars(series, |geometry| {
                        let left = (geometry.left * hpr).round() as i32;
                        let right = (geometry.right * hpr).round() as i32;
                        let top = (geometry.top * vpr).round() as i32;
                        let bottom = (geometry.bottom * vpr).round() as i32;
                        if right <= left || bottom <= top {
                            return;
                        }
                        out.push(Prim::Rect {
                            rect: IRect {
                                x: left,
                                y: top,
                                w: right - left,
                                h: bottom - top,
                            },
                            color,
                        });
                        let (hovered, selected) =
                            self.general_row_interaction(series.id(), geometry.row);
                        if hovered || selected {
                            interaction[usize::from(selected)] = Some(Prim::RectFrame {
                                rect: IRect {
                                    x: left,
                                    y: top,
                                    w: right - left,
                                    h: bottom - top,
                                },
                                border: ((if selected { 2.0 } else { 1.0 }) * hpr.min(vpr))
                                    .round()
                                    .max(1.0) as i32,
                                color: if selected { PRIMARY } else { GENERAL_HOVER },
                            });
                        }
                    });
                }
                GeneralSeriesKind::RangeArea => {
                    let color = series
                        .color()
                        .and_then(Color::parse_css)
                        .unwrap_or(DEFAULT_LINE_COLOR);
                    let fill = Color::rgba(
                        color.r(),
                        color.g(),
                        color.b(),
                        opacity_alpha(series.fill_opacity() * (7.0 / 9.0)),
                    );
                    let mut upper = Vec::<[f32; 2]>::new();
                    let mut lower = Vec::<[f32; 2]>::new();
                    let mut markers = Vec::new();
                    let mut flush_run = |upper: &mut Vec<[f32; 2]>, lower: &mut Vec<[f32; 2]>| {
                        if upper.len() >= 2 && upper.len() == lower.len() {
                            let upper_first = points.len() as u32;
                            let point_count = upper.len() as u32;
                            points.append(upper);
                            let lower_first = points.len() as u32;
                            points.append(lower);
                            out.push(Prim::BandFill {
                                upper_first,
                                lower_first,
                                point_count,
                                line_type: series.interpolation().render_type(),
                                fill,
                            });
                            push_general_stroke(
                                out,
                                points,
                                (upper_first, point_count),
                                series.interpolation().render_type(),
                                (
                                    (series.line_width() * vpr) as f32,
                                    series.line_style().render_style(),
                                    color,
                                ),
                                pane_clip,
                            );
                            push_general_stroke(
                                out,
                                points,
                                (lower_first, point_count),
                                series.interpolation().render_type(),
                                (
                                    (series.line_width() * vpr) as f32,
                                    series.line_style().render_style(),
                                    color,
                                ),
                                pane_clip,
                            );
                        } else {
                            upper.clear();
                            lower.clear();
                        }
                    };
                    self.visit_general_range_points(series, |geometry| {
                        if geometry.starts_new_run {
                            flush_run(&mut upper, &mut lower);
                        }
                        upper.push([(geometry.x * hpr) as f32, (geometry.high_y * vpr) as f32]);
                        lower.push([(geometry.x * hpr) as f32, (geometry.low_y * vpr) as f32]);
                        if series.point_markers() {
                            for y in [geometry.low_y, geometry.high_y] {
                                push_general_point_symbol(
                                    &mut markers,
                                    series.point_symbol(),
                                    geometry.x * hpr,
                                    y * vpr,
                                    series.point_radius() * vpr,
                                    color,
                                );
                            }
                        }
                        push_label(
                            geometry.row,
                            geometry.x,
                            geometry.high_y - label_size * 0.65 - 4.0,
                            geometry.low_y + label_size * 0.65 + 4.0,
                            plot.width - 4.0,
                        );
                        let (hovered, selected) =
                            self.general_row_interaction(series.id(), geometry.row);
                        if hovered || selected {
                            let stroke = if selected { PRIMARY } else { GENERAL_HOVER };
                            interaction[usize::from(selected)] = Some(Prim::Circle {
                                cx: (geometry.x * hpr) as f32,
                                cy: (((geometry.low_y + geometry.high_y) * 0.5) * vpr) as f32,
                                radius: ((if selected { 5.0 } else { 4.0 }) * vpr) as f32,
                                fill: Color::rgba(0, 0, 0, 0),
                                stroke_width: (if selected { 2.0 } else { 1.0 }) * vpr as f32,
                                stroke,
                            });
                        }
                    });
                    flush_run(&mut upper, &mut lower);
                    out.append(&mut markers);
                }
                GeneralSeriesKind::ErrorBar => {
                    let color = series
                        .color()
                        .and_then(Color::parse_css)
                        .unwrap_or(DEFAULT_LINE_COLOR);
                    let line_width = hpr.min(vpr).round().max(1.0) as i32;
                    self.visit_general_error_bars(series, |geometry| {
                        let x = (geometry.x * hpr).round() as i32;
                        let y = (geometry.y * vpr).round() as i32;
                        let cap_x = (geometry.cap_half_size * hpr).round().max(1.0) as i32;
                        let cap_y = (geometry.cap_half_size * vpr).round().max(1.0) as i32;
                        let x_low = geometry.x_low.map(|value| (value * hpr).round() as i32);
                        let x_high = geometry.x_high.map(|value| (value * hpr).round() as i32);
                        let y_low = geometry.y_low.map(|value| (value * vpr).round() as i32);
                        let y_high = geometry.y_high.map(|value| (value * vpr).round() as i32);
                        if x_low.is_some() || x_high.is_some() {
                            let from = x_low.unwrap_or(x);
                            let to = x_high.unwrap_or(x);
                            out.push(Prim::HLine {
                                y,
                                x0: from.min(to),
                                x1: from.max(to),
                                width: line_width,
                                style: LineStyle::Solid,
                                color,
                            });
                            for bound in [x_low, x_high].into_iter().flatten() {
                                out.push(Prim::VLine {
                                    x: bound,
                                    y0: y - cap_y,
                                    y1: y + cap_y,
                                    width: line_width,
                                    style: LineStyle::Solid,
                                    color,
                                });
                            }
                        }
                        if y_low.is_some() || y_high.is_some() {
                            let from = y_low.unwrap_or(y);
                            let to = y_high.unwrap_or(y);
                            out.push(Prim::VLine {
                                x,
                                y0: from.min(to),
                                y1: from.max(to),
                                width: line_width,
                                style: LineStyle::Solid,
                                color,
                            });
                            for bound in [y_low, y_high].into_iter().flatten() {
                                out.push(Prim::HLine {
                                    y: bound,
                                    x0: x - cap_x,
                                    x1: x + cap_x,
                                    width: line_width,
                                    style: LineStyle::Solid,
                                    color,
                                });
                            }
                        }
                        out.push(Prim::Circle {
                            cx: (geometry.x * hpr) as f32,
                            cy: (geometry.y * vpr) as f32,
                            radius: (2.0 * vpr) as f32,
                            fill: color,
                            stroke_width: 0.0,
                            stroke: color,
                        });
                        let top = [Some(geometry.y), geometry.y_low, geometry.y_high]
                            .into_iter()
                            .flatten()
                            .fold(geometry.y, f64::min);
                        let bottom = [Some(geometry.y), geometry.y_low, geometry.y_high]
                            .into_iter()
                            .flatten()
                            .fold(geometry.y, f64::max);
                        push_label(
                            geometry.row,
                            geometry.x,
                            top - label_size * 0.65 - 4.0,
                            bottom + label_size * 0.65 + 4.0,
                            plot.width - 4.0,
                        );
                        let (hovered, selected) =
                            self.general_row_interaction(series.id(), geometry.row);
                        if hovered || selected {
                            interaction[usize::from(selected)] = Some(Prim::Circle {
                                cx: (geometry.x * hpr) as f32,
                                cy: (geometry.y * vpr) as f32,
                                radius: ((geometry.cap_half_size
                                    + if selected { 3.0 } else { 2.0 })
                                    * vpr) as f32,
                                fill: Color::rgba(0, 0, 0, 0),
                                stroke_width: (if selected { 2.0 } else { 1.0 }) * vpr as f32,
                                stroke: if selected { PRIMARY } else { GENERAL_HOVER },
                            });
                        }
                    });
                }
                GeneralSeriesKind::Column => {
                    let color = series
                        .color()
                        .and_then(Color::parse_css)
                        .unwrap_or(DEFAULT_LINE_COLOR);
                    self.visit_general_columns(series, |geometry| {
                        let left = (geometry.left * hpr).round() as i32;
                        let right = (geometry.right * hpr).round() as i32;
                        let top = (geometry.top * vpr).round() as i32;
                        let bottom = (geometry.bottom * vpr).round() as i32;
                        let width = (right - left).max(1);
                        let height = bottom - top;
                        if height <= 0 {
                            return;
                        }
                        out.push(Prim::Rect {
                            rect: IRect {
                                x: left,
                                y: top,
                                w: width,
                                h: height,
                            },
                            color,
                        });
                        let positive_above = (dataset.y()[geometry.row] >= 0.0)
                            != self
                                .general_axis(series.y_axis_id())
                                .is_some_and(|axis| axis.reverse());
                        let (above, below) = if positive_above {
                            (
                                geometry.top - label_size * 0.65 - 2.0,
                                geometry.bottom + label_size * 0.65 + 2.0,
                            )
                        } else {
                            (
                                geometry.bottom + label_size * 0.65 + 2.0,
                                geometry.top - label_size * 0.65 - 2.0,
                            )
                        };
                        push_label(
                            geometry.row,
                            (geometry.left + geometry.right) * 0.5,
                            above,
                            below,
                            geometry.right - geometry.left - 4.0,
                        );
                        let (hovered, selected) =
                            self.general_row_interaction(series.id(), geometry.row);
                        if hovered || selected {
                            interaction[usize::from(selected)] = Some(Prim::RectFrame {
                                rect: IRect {
                                    x: left,
                                    y: top,
                                    w: width,
                                    h: height,
                                },
                                border: ((if selected { 2.0 } else { 1.0 }) * hpr.min(vpr))
                                    .round()
                                    .max(1.0) as i32,
                                color: if selected { PRIMARY } else { GENERAL_HOVER },
                            });
                        }
                    });
                }
                GeneralSeriesKind::HorizontalBar => {
                    let color = series
                        .color()
                        .and_then(Color::parse_css)
                        .unwrap_or(DEFAULT_LINE_COLOR);
                    self.visit_general_horizontal_bars(series, |geometry| {
                        let left = (geometry.left * hpr).round() as i32;
                        let right = (geometry.right * hpr).round() as i32;
                        let top = (geometry.top * vpr).round() as i32;
                        let bottom = (geometry.bottom * vpr).round() as i32;
                        let width = right - left;
                        let height = (bottom - top).max(1);
                        if width <= 0 {
                            return;
                        }
                        out.push(Prim::Rect {
                            rect: IRect {
                                x: left,
                                y: top,
                                w: width,
                                h: height,
                            },
                            color,
                        });
                        push_label(
                            geometry.row,
                            (geometry.left + geometry.right) * 0.5,
                            geometry.top - label_size * 0.65 - 2.0,
                            geometry.bottom + label_size * 0.65 + 2.0,
                            geometry.right - geometry.left - 4.0,
                        );
                        let (hovered, selected) =
                            self.general_row_interaction(series.id(), geometry.row);
                        if hovered || selected {
                            interaction[usize::from(selected)] = Some(Prim::RectFrame {
                                rect: IRect {
                                    x: left,
                                    y: top,
                                    w: width,
                                    h: height,
                                },
                                border: ((if selected { 2.0 } else { 1.0 }) * hpr.min(vpr))
                                    .round()
                                    .max(1.0) as i32,
                                color: if selected { PRIMARY } else { GENERAL_HOVER },
                            });
                        }
                    });
                }
                GeneralSeriesKind::BoxPlot => {
                    let color = series
                        .color()
                        .and_then(Color::parse_css)
                        .unwrap_or(DEFAULT_LINE_COLOR);
                    let fill = Color::rgba(color.r(), color.g(), color.b(), 48);
                    let line_width =
                        (GENERAL_LINE_WIDTH_CSS * hpr.min(vpr)).round().max(1.0) as i32;
                    self.visit_general_box_plots(series, |geometry| {
                        let left = (geometry.left * hpr).round() as i32;
                        let right = (geometry.right * hpr).round() as i32;
                        let q1 = (geometry.q1_y * vpr).round() as i32;
                        let q3 = (geometry.q3_y * vpr).round() as i32;
                        let top = q1.min(q3);
                        let bottom = q1.max(q3);
                        if right <= left || bottom <= top {
                            return;
                        }
                        out.push(Prim::Rect {
                            rect: IRect {
                                x: left,
                                y: top,
                                w: (right - left).max(1),
                                h: (bottom - top).max(1),
                            },
                            color: fill,
                        });
                        let x0 = (geometry.left * hpr).round() as i32;
                        let x1 = (geometry.right * hpr).round() as i32;
                        let cx = (geometry.center_x * hpr).round() as i32;
                        let min_y = (geometry.min_y * vpr).round() as i32;
                        let q1_y = (geometry.q1_y * vpr).round() as i32;
                        let median_y = (geometry.median_y * vpr).round() as i32;
                        let q3_y = (geometry.q3_y * vpr).round() as i32;
                        let max_y = (geometry.max_y * vpr).round() as i32;
                        for y in [q1_y, median_y, q3_y, min_y, max_y] {
                            out.push(Prim::HLine {
                                y,
                                x0,
                                x1,
                                width: line_width,
                                style: LineStyle::Solid,
                                color,
                            });
                        }
                        for x in [x0, x1] {
                            out.push(Prim::VLine {
                                x,
                                y0: q1_y.min(q3_y),
                                y1: q1_y.max(q3_y),
                                width: line_width,
                                style: LineStyle::Solid,
                                color,
                            });
                        }
                        for (from, to) in [(min_y, q1_y), (q3_y, max_y)] {
                            out.push(Prim::VLine {
                                x: cx,
                                y0: from.min(to),
                                y1: from.max(to),
                                width: line_width,
                                style: LineStyle::Solid,
                                color,
                            });
                        }
                        push_label(
                            geometry.row,
                            geometry.center_x,
                            geometry.min_y.min(geometry.max_y) - label_size * 0.65 - 4.0,
                            geometry.min_y.max(geometry.max_y) + label_size * 0.65 + 4.0,
                            geometry.right - geometry.left - 4.0,
                        );
                        let (hovered, selected) =
                            self.general_row_interaction(series.id(), geometry.row);
                        if hovered || selected {
                            interaction[usize::from(selected)] = Some(Prim::RectFrame {
                                rect: IRect {
                                    x: left,
                                    y: (geometry.min_y.min(geometry.max_y) * vpr).round() as i32,
                                    w: (right - left).max(1),
                                    h: ((geometry.min_y.max(geometry.max_y)
                                        - geometry.min_y.min(geometry.max_y))
                                        * vpr)
                                        .round()
                                        .max(1.0) as i32,
                                },
                                border: ((if selected { 2.0 } else { 1.0 }) * hpr.min(vpr))
                                    .round()
                                    .max(1.0) as i32,
                                color: if selected { PRIMARY } else { GENERAL_HOVER },
                            });
                        }
                    });
                }
                GeneralSeriesKind::HeatmapGrid => {
                    let base_color = series.color().and_then(Color::parse_css);
                    self.visit_general_heatmap_cells(series, |geometry| {
                        let left = (geometry.left * hpr).round() as i32;
                        let right = (geometry.right * hpr).round() as i32;
                        let top = (geometry.top * vpr).round() as i32;
                        let bottom = (geometry.bottom * vpr).round() as i32;
                        let width = (right - left).max(1);
                        let height = (bottom - top).max(1);
                        let intensity = geometry.intensity.clamp(0.0, 1.0);
                        let color = base_color.map_or_else(
                            || {
                                Color::rgba(
                                    0,
                                    (100.0 + 155.0 * intensity).round() as u8,
                                    (100.0 * intensity).round() as u8,
                                    (51.0 + 204.0 * intensity).round() as u8,
                                )
                            },
                            |base| {
                                Color::rgba(
                                    base.r(),
                                    base.g(),
                                    base.b(),
                                    ((base.a() as f64) * (0.2 + 0.8 * intensity))
                                        .round()
                                        .clamp(0.0, 255.0)
                                        as u8,
                                )
                            },
                        );
                        out.push(Prim::Rect {
                            rect: IRect {
                                x: left,
                                y: top,
                                w: width,
                                h: height,
                            },
                            color,
                        });
                        push_label(
                            geometry.row,
                            (geometry.left + geometry.right) * 0.5,
                            geometry.top - label_size * 0.65,
                            geometry.bottom + label_size * 0.65,
                            geometry.right - geometry.left - 4.0,
                        );
                        let (hovered, selected) =
                            self.general_row_interaction(series.id(), geometry.row);
                        if hovered || selected {
                            interaction[usize::from(selected)] = Some(Prim::RectFrame {
                                rect: IRect {
                                    x: left,
                                    y: top,
                                    w: width,
                                    h: height,
                                },
                                border: ((if selected { 2.0 } else { 1.0 }) * hpr.min(vpr))
                                    .round()
                                    .max(1.0) as i32,
                                color: if selected { PRIMARY } else { GENERAL_HOVER },
                            });
                        }
                    });
                }
                GeneralSeriesKind::Scatter | GeneralSeriesKind::Bubble => {
                    let color = series
                        .color()
                        .and_then(Color::parse_css)
                        .unwrap_or(DEFAULT_LINE_COLOR);
                    self.visit_general_scatter_points(series, |geometry| {
                        push_general_point_symbol(
                            out,
                            if series.kind() == GeneralSeriesKind::Bubble {
                                GeneralPointSymbol::Circle
                            } else {
                                series.point_symbol()
                            },
                            geometry.x * hpr,
                            geometry.y * vpr,
                            geometry.radius * vpr,
                            color,
                        );
                        push_label(
                            geometry.row,
                            geometry.x,
                            geometry.y - geometry.radius - label_size * 0.65 - 2.0,
                            geometry.y + geometry.radius + label_size * 0.65 + 2.0,
                            plot.width - 4.0,
                        );
                        let (hovered, selected) =
                            self.general_row_interaction(series.id(), geometry.row);
                        if hovered || selected {
                            let stroke = if selected { PRIMARY } else { GENERAL_HOVER };
                            interaction[usize::from(selected)] = Some(Prim::Circle {
                                cx: (geometry.x * hpr) as f32,
                                cy: (geometry.y * vpr) as f32,
                                radius: ((geometry.radius + if selected { 3.0 } else { 2.0 }) * vpr)
                                    as f32,
                                fill: Color::rgba(0, 0, 0, 0),
                                stroke_width: (if selected { 2.0 } else { 1.0 }) * vpr as f32,
                                stroke,
                            });
                        }
                    });
                }
            }
        }
        self.push_general_reference_marks(pane_id, plot, hpr, vpr, out);
        out.extend(label_primitives);
        interaction
    }

    fn push_general_reference_regions(
        &self,
        pane_id: crate::PaneId,
        plot: crate::general_axes::GeneralPlotRect,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        for reference in self
            .general_reference_iter()
            .filter(|reference| reference.pane_id() == pane_id)
        {
            let GeneralReferenceOptions::Region {
                x_axis_id,
                y_axis_id,
                x_from,
                x_to,
                y_from,
                y_to,
                fill_color,
                ..
            } = reference.options()
            else {
                continue;
            };
            let (Some(x0), Some(x1), Some(y0), Some(y1)) = (
                self.general_reference_coordinate(x_axis_id, x_from, plot),
                self.general_reference_coordinate(x_axis_id, x_to, plot),
                self.general_reference_coordinate(y_axis_id, y_from, plot),
                self.general_reference_coordinate(y_axis_id, y_to, plot),
            ) else {
                continue;
            };
            let left = (x0.min(x1) * hpr).round() as i32;
            let right = (x0.max(x1) * hpr).round() as i32;
            let top = (y0.min(y1) * vpr).round() as i32;
            let bottom = (y0.max(y1) * vpr).round() as i32;
            if right <= left || bottom <= top {
                continue;
            }
            out.push(Prim::Rect {
                rect: IRect {
                    x: left,
                    y: top,
                    w: right - left,
                    h: bottom - top,
                },
                color: fill_color
                    .as_deref()
                    .and_then(Color::parse_css)
                    .unwrap_or(GENERAL_REFERENCE_REGION),
            });
        }
    }

    fn push_general_reference_marks(
        &self,
        pane_id: crate::PaneId,
        plot: crate::general_axes::GeneralPlotRect,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        let plot_top = (plot.y * vpr).round() as i32;
        let plot_bottom = ((plot.y + plot.height) * vpr).round() as i32;
        let plot_right = (plot.width * hpr).round() as i32;
        for reference in self
            .general_reference_iter()
            .filter(|reference| reference.pane_id() == pane_id)
        {
            match reference.options() {
                GeneralReferenceOptions::Line {
                    axis_id,
                    value,
                    color,
                    line_width,
                    ..
                } => {
                    let Some(axis) = self.general_axis(axis_id) else {
                        continue;
                    };
                    let Some(coordinate) = self.general_reference_coordinate(axis_id, value, plot)
                    else {
                        continue;
                    };
                    let color = color
                        .as_deref()
                        .and_then(Color::parse_css)
                        .unwrap_or(GENERAL_REFERENCE_MARK);
                    let width = (*line_width * hpr.min(vpr)).round().max(1.0) as i32;
                    match axis.dimension() {
                        AxisDimension::X => out.push(Prim::VLine {
                            x: (coordinate * hpr).round() as i32,
                            y0: plot_top,
                            y1: plot_bottom,
                            width,
                            style: LineStyle::Solid,
                            color,
                        }),
                        AxisDimension::Y => out.push(Prim::HLine {
                            y: (coordinate * vpr).round() as i32,
                            x0: 0,
                            x1: plot_right,
                            width,
                            style: LineStyle::Solid,
                            color,
                        }),
                        AxisDimension::Angle | AxisDimension::Radius => {}
                    }
                }
                GeneralReferenceOptions::Dot {
                    x_axis_id,
                    y_axis_id,
                    x,
                    y,
                    color,
                    radius,
                    ..
                } => {
                    let (Some(x), Some(y)) = (
                        self.general_reference_coordinate(x_axis_id, x, plot),
                        self.general_reference_coordinate(y_axis_id, y, plot),
                    ) else {
                        continue;
                    };
                    let color = color
                        .as_deref()
                        .and_then(Color::parse_css)
                        .unwrap_or(GENERAL_REFERENCE_MARK);
                    out.push(Prim::Circle {
                        cx: (x * hpr) as f32,
                        cy: (y * vpr) as f32,
                        radius: (*radius * vpr) as f32,
                        fill: color,
                        stroke_width: 0.0,
                        stroke: color,
                    });
                }
                GeneralReferenceOptions::Region { .. } => {}
            }
        }
    }

    fn general_reference_coordinate(
        &self,
        axis_id: &str,
        value: &GeneralReferenceValue,
        plot: crate::general_axes::GeneralPlotRect,
    ) -> Option<f64> {
        let axis = self.general_axis(axis_id)?;
        let domain = self.effective_general_axis_domain(axis)?;
        let (from, to, low, high) = match axis.dimension() {
            AxisDimension::X => {
                let range = if axis.reverse() {
                    (plot.width, 0.0)
                } else {
                    (0.0, plot.width)
                };
                (range.0, range.1, 0.0, plot.width)
            }
            AxisDimension::Y => {
                let bottom = plot.y + plot.height;
                let range = if axis.reverse() {
                    (plot.y, bottom)
                } else {
                    (bottom, plot.y)
                };
                (range.0, range.1, plot.y, bottom)
            }
            AxisDimension::Angle | AxisDimension::Radius => return None,
        };
        let coordinate = match (value, domain) {
            (GeneralReferenceValue::Numeric(value), GeneralAxisDomain::Numeric(domain)) => {
                NumericAxisScale::new(axis.scale(), domain, from, to)?.coordinate(*value)
            }
            (GeneralReferenceValue::Temporal(value), GeneralAxisDomain::Temporal([start, end])) => {
                LinearScale::new(start as f64, end as f64, from, to)
                    .ok()?
                    .coordinate(*value as f64)
            }
            (GeneralReferenceValue::Category(value), GeneralAxisDomain::Category(categories)) => {
                let index = categories.iter().position(|category| category == value)?;
                match axis.scale() {
                    GeneralScaleType::Band => {
                        let scale = BandScale::new(
                            categories.len(),
                            from,
                            to,
                            axis.band_padding_inner(),
                            axis.band_padding_outer(),
                            0.5,
                        )
                        .ok()?;
                        scale.bounds(index).map(|(from, to)| (from + to) * 0.5)
                    }
                    GeneralScaleType::Point => {
                        PointScale::new(categories.len(), from, to, axis.band_padding_outer(), 0.5)
                            .ok()?
                            .coordinate(index)
                    }
                    _ => None,
                }
            }
            _ => None,
        }?;
        coordinate
            .is_finite()
            .then_some(coordinate)
            .filter(|coordinate| *coordinate >= low.min(high) && *coordinate <= high.max(low))
    }
}
