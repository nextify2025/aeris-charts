use super::feature_geometry::{positions_box, positions_line};
use super::*;
use crate::native_primitives::{
    AnchoredTextHorizontalAlign, AnchoredTextVerticalAlign, NativePanePrimitiveKind,
    NativeSeriesPrimitiveKind, OverlayPriceScaleSide,
};
use aeris_charts_core::format::time_formatter::{
    format_date_pattern, format_tick_label_in, TickMarkType,
};
use aeris_charts_core::scale::exchange_time::ExchangeTime;
use aeris_charts_render::draw_list::TextAlign;

/// Session color of one source bar: the optional `[start_hour, end_hour)` gate and the weekend
/// test both use exchange wall-clock time (fractional hours, minute precision and finer).
fn session_color(
    time: i64,
    options: crate::SessionHighlightingOptions,
    exchange: &ExchangeTime,
) -> Option<Color> {
    if let (Some(start), Some(end)) = (options.start_hour, options.end_hour) {
        let seconds = exchange.local_seconds_of_day(time);
        let start = (start * 3_600.0).round() as i64;
        let end = (end * 3_600.0).round() as i64;
        let inside = if start <= end {
            seconds >= start && seconds < end
        } else {
            seconds >= start || seconds < end
        };
        if !inside {
            return None;
        }
    }
    Some(if exchange.local_weekday(time) >= 5 {
        options.weekend_color
    } else {
        options.weekday_color
    })
}

fn surface_contrast_line_color(explicit: Option<Color>, surface_css: &str) -> Color {
    explicit.unwrap_or_else(|| {
        let fallback = aeris_charts_core::style::DEFAULT_SURFACE_RGB;
        let surface =
            Color::parse_css(surface_css).unwrap_or(Color::rgb(fallback.0, fallback.1, fallback.2));
        if surface.luminance() > 160.0 {
            Color::rgba(0, 0, 0, 51)
        } else {
            Color::rgba(255, 255, 255, 31)
        }
    })
}

impl ChartEngine {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_native_series_background_primitives_frame(
        &self,
        rs: ResolvedSeries,
        from: i64,
        to: i64,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
        scale: &PriceScaleCore,
    ) {
        let Some(series) = self.series_entry(rs.id) else {
            return;
        };
        if series.native_primitives.is_empty() || scale.is_empty() {
            return;
        }
        let plot = self.data.plot(rs.id);
        let rows = plot
            .visible_rows(from, to)
            .filter(|&row| !plot.is_whitespace_row(row))
            .collect::<Vec<_>>();
        if rows.is_empty() {
            return;
        }
        for primitive in &series.native_primitives {
            let NativeSeriesPrimitiveKind::BandsIndicator(options) = &primitive.kind else {
                continue;
            };
            let upper_first = points.len() as u32;
            points.extend(rows.iter().filter_map(|&row| {
                let index = plot.index_at(row)?;
                let price = plot.value_at(row, PlotValueIndex::Close);
                price.is_finite().then(|| {
                    [
                        (self.time_scale.index_to_coordinate(index) * hpr) as f32,
                        (scale.price_to_coordinate(price * 1.1, rs.base_value) * vpr) as f32,
                    ]
                })
            }));
            let count = points.len() as u32 - upper_first;
            if count == 0 {
                continue;
            }
            let lower_first = points.len() as u32;
            points.extend(rows.iter().filter_map(|&row| {
                let index = plot.index_at(row)?;
                let price = plot.value_at(row, PlotValueIndex::Close);
                price.is_finite().then(|| {
                    [
                        (self.time_scale.index_to_coordinate(index) * hpr) as f32,
                        (scale.price_to_coordinate(price * 0.9, rs.base_value) * vpr) as f32,
                    ]
                })
            }));
            if points.len() as u32 - lower_first != count {
                points.truncate(upper_first as usize);
                continue;
            }
            let width = (options.line_width * vpr) as f32;
            out.push(Prim::Polyline {
                first_point: upper_first,
                point_count: count,
                width,
                style: LineStyle::Solid,
                line_type: LineType::Simple,
                color: options.line_color,
            });
            out.push(Prim::Polyline {
                first_point: lower_first,
                point_count: count,
                width,
                style: LineStyle::Solid,
                line_type: LineType::Simple,
                color: options.line_color,
            });
            // The official renderer strokes `lines` before filling `region`.
            out.push(Prim::BandFill {
                line_type: LineType::Simple,
                upper_first,
                lower_first,
                point_count: count,
                fill: options.fill_color,
            });
        }
    }

    pub(super) fn build_native_delta_tooltip_frame(
        &self,
        pane_index: usize,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        const INLINE_PADDING: f64 = 10.0;
        const BLOCK_PADDING: f64 = 5.0;
        let pane = &self.panes[pane_index];
        let chart_options = self.options.get();
        let layout = &chart_options.layout;
        let family = layout.font_family.as_str();
        let foreground =
            Color::parse_css(&layout.text_color).unwrap_or(Color::rgb(0x13, 0x17, 0x22));
        let muted_foreground =
            Color::parse_css(&layout.muted_text_color).unwrap_or(Color::rgb(0x78, 0x7b, 0x86));
        let border =
            Color::parse_css(&chart_options.right_price_scale.border_color).unwrap_or(Color::rgb(
                aeris_charts_core::style::DEFAULT_BORDER_RGB.0,
                aeris_charts_core::style::DEFAULT_BORDER_RGB.1,
                aeris_charts_core::style::DEFAULT_BORDER_RGB.2,
            ));
        let background = Color::parse_css(&layout.background.color)
            .or_else(|| Color::parse_css(&layout.background.top_color))
            .unwrap_or(Color::rgb(255, 255, 255));
        let secondary_size = layout.font_size.max(1.0);
        let primary_size = secondary_size + 2.0;
        for series in self
            .series
            .iter()
            .filter(|series| series.visible && series.pane_index == pane_index)
        {
            for primitive in &series.native_primitives {
                let NativeSeriesPrimitiveKind::DeltaTooltip(state) = &primitive.kind else {
                    continue;
                };
                let visible_points = state.visible_points();
                if visible_points.is_empty() {
                    continue;
                }
                let plot = self.data.plot(series.id);
                let Some((times, _)) = self.data.series_data(series.id) else {
                    continue;
                };
                let mut items = Vec::with_capacity(2);
                for point in visible_points {
                    let Some(row) = plot.search(
                        point.index,
                        aeris_charts_core::model::plot_list::MismatchDirection::None,
                    ) else {
                        continue;
                    };
                    let price = plot.value_at(
                        row,
                        aeris_charts_core::model::plot_list::PlotValueIndex::Close,
                    );
                    let Some(time) = times.get(plot.source_row(row)).copied() else {
                        continue;
                    };
                    items.push((
                        self.time_scale.index_to_coordinate(point.index),
                        point.index,
                        price,
                        time,
                    ));
                }
                if items.is_empty() {
                    continue;
                }
                let marker_color = series
                    .line_color
                    .as_deref()
                    .and_then(Color::parse_css)
                    .unwrap_or(Color::rgb(0x88, 0x88, 0x88));
                let line_color =
                    surface_contrast_line_color(state.options.line_color, &layout.background.color);
                let top = pane.top + state.options.top_offset;
                for (x, _, price, _) in &items {
                    let y = self
                        .series_price_to_coordinate(series.id, *price)
                        .unwrap_or(-1_000.0);
                    out.push(Prim::VLine {
                        x: (*x * hpr).round() as i32,
                        y0: (top * vpr).round() as i32,
                        y1: ((pane.top + pane.height) * vpr).round() as i32,
                        width: hpr.round().max(1.0) as i32,
                        style: LineStyle::Solid,
                        color: line_color,
                    });
                    if y.is_finite() {
                        out.push(Prim::Circle {
                            cx: (*x * hpr) as f32,
                            cy: (y * vpr) as f32,
                            radius: (6.0 * hpr.min(vpr)) as f32,
                            fill: background,
                            stroke_width: 0.0,
                            stroke: background,
                        });
                        out.push(Prim::Circle {
                            cx: (*x * hpr) as f32,
                            cy: (y * vpr) as f32,
                            radius: (4.0 * hpr.min(vpr)) as f32,
                            fill: marker_color,
                            stroke_width: 0.0,
                            stroke: marker_color,
                        });
                    }
                }

                items.sort_by_key(|item| item.1);
                let tooltip_lines = |item: &(f64, i64, f64, i64)| {
                    let mut lines = vec![self.format_series_plain_value(series, item.2)];
                    // A host `timeFormatter` owns the whole time text; otherwise the built-in
                    // date and time lines use exchange wall-clock time. Both print the bar's
                    // label time.
                    let printed = self.bar_label_time(item.3);
                    if let Some(text) = self.host_time_label(printed) {
                        lines.push(text);
                        return lines;
                    }
                    lines.push(format_date_pattern(
                        self.exchange_time.local_seconds(printed),
                        "dd MMM yyyy",
                        &self.month_names,
                    ));
                    if state.options.show_time {
                        lines.push(format_tick_label_in(
                            printed,
                            TickMarkType::Time,
                            &self.month_names,
                            &self.exchange_time,
                        ));
                    }
                    lines
                };
                let lines: Vec<Vec<String>> = items.iter().map(tooltip_lines).collect();
                let section_width = |content: &[String], sizes: &[f64], weights: &[u16]| {
                    content
                        .iter()
                        .enumerate()
                        .map(|(index, text)| {
                            self.measure_text_run(text, sizes[index], family, weights[index], false)
                        })
                        .fold(0.0_f64, f64::max)
                        + INLINE_PADDING * 2.0
                };
                let section_height = |count: usize, line_heights: &[f64]| {
                    BLOCK_PADDING * 1.5 + line_heights.iter().take(count).sum::<f64>()
                };
                let tooltip_sizes = [primary_size, secondary_size, secondary_size];
                let tooltip_weights = [590, 400, 400];
                let tooltip_heights = [
                    primary_size + 4.0,
                    secondary_size + 4.0,
                    secondary_size + 4.0,
                ];
                let mut positions: Vec<(f64, f64)> = items
                    .iter()
                    .zip(&lines)
                    .map(|(item, lines)| {
                        let width = section_width(lines, &tooltip_sizes, &tooltip_weights);
                        (
                            (item.0 - width * 0.5).clamp(0.0, self.pane_w - width),
                            width,
                        )
                    })
                    .collect();
                let mut delta_top = String::new();
                let mut delta_bottom = String::new();
                let mut delta_bg = Color::rgb(255, 255, 255);
                let mut delta_text = Color::rgb(0x13, 0x17, 0x22);
                let mut delta_width = 0.0;
                if items.len() == 2 {
                    let change = items[1].2 - items[0].2;
                    let percent = 100.0 * change / items[0].2;
                    let positive = change >= 0.0;
                    let change_text = self.format_series_plain_value(series, change);
                    delta_top = if positive {
                        format!("+{change_text}")
                    } else {
                        change_text
                    };
                    delta_bottom = format!("{}{percent:.2}%", if positive { "+" } else { "" });
                    delta_bg = if positive {
                        Color::rgba(MARKET_UP_RGB.0, MARKET_UP_RGB.1, MARKET_UP_RGB.2, 51)
                    } else {
                        Color::rgba(MARKET_DOWN_RGB.0, MARKET_DOWN_RGB.1, MARKET_DOWN_RGB.2, 51)
                    };
                    delta_text = if positive { UP } else { DOWN };
                    let min_delta = section_width(
                        &[delta_top.clone(), delta_bottom.clone()],
                        &[14.0, 12.0],
                        &[590, 400],
                    );
                    let overlap = min_delta + positions[0].0 + positions[0].1 - positions[1].0;
                    if overlap > 0.0 {
                        let half = overlap * 0.5;
                        let left_space = positions[0].0;
                        let right_space = self.pane_w - positions[1].0 - positions[1].1;
                        if left_space >= half && right_space >= half {
                            positions[0].0 -= half;
                            positions[1].0 += half;
                        } else if left_space < right_space {
                            positions[0].0 = 0.0;
                            positions[1].0 += overlap - left_space;
                        } else {
                            positions[0].0 = (positions[0].0 - (overlap - right_space)).max(0.0);
                            positions[1].0 += right_space;
                        }
                    }
                    delta_width = (positions[1].0 - positions[0].0 - positions[0].1).round();
                }
                let tooltip_height = lines
                    .iter()
                    .map(|lines| section_height(lines.len(), &tooltip_heights))
                    .fold(0.0_f64, f64::max);
                let delta_height = if items.len() == 2 {
                    section_height(2, &[primary_size + 4.0, secondary_size + 4.0])
                } else {
                    0.0
                };
                let main_height = tooltip_height.max(delta_height);
                let main_x = positions[0].0.round();
                let main_width = if items.len() == 2 {
                    (positions[1].0 + positions[1].1 - positions[0].0).round()
                } else {
                    positions[0].1.round()
                };
                out.push(Prim::RoundRect {
                    x: (main_x * hpr) as f32,
                    y: (top * vpr) as f32,
                    w: (main_width * hpr) as f32,
                    h: (main_height * vpr) as f32,
                    radii: [(6.0 * hpr.min(vpr)) as f32; 4],
                    fill: background,
                    border_width: aeris_charts_core::style::border_width_device_px(hpr.min(vpr))
                        as f32,
                    border_color: border,
                });
                if items.len() == 2 && delta_width > 0.0 {
                    out.push(Prim::Rect {
                        rect: IRect {
                            x: ((positions[0].0 + positions[0].1) * hpr).round() as i32,
                            y: (top * vpr).round() as i32,
                            w: (delta_width * hpr).round() as i32,
                            h: (main_height * vpr).round() as i32,
                        },
                        color: delta_bg,
                    });
                }
                for (index, content) in lines.iter().enumerate() {
                    let center_x = positions[index].0 + positions[index].1 * 0.5;
                    let content_height = section_height(content.len(), &tooltip_heights);
                    let mut y = top + (main_height - content_height) * 0.5 + BLOCK_PADDING;
                    for (line_index, text) in content.iter().enumerate() {
                        out.push(Prim::Text {
                            x: (center_x * hpr) as f32,
                            y: ((y + tooltip_sizes[line_index] * 0.5) * vpr) as f32,
                            text: text.clone(),
                            color: if line_index == 0 {
                                foreground
                            } else {
                                muted_foreground
                            },
                            size: (tooltip_sizes[line_index] * vpr) as f32,
                            family: family.into(),
                            align: TextAlign::Center,
                            weight: tooltip_weights[line_index],
                            italic: false,
                        });
                        y += tooltip_heights[line_index];
                    }
                }
                if items.len() == 2 {
                    let delta_center = positions[1].0 - delta_width * 0.5;
                    let content_height =
                        section_height(2, &[primary_size + 4.0, secondary_size + 4.0]);
                    let mut y = top + (main_height - content_height) * 0.5 + BLOCK_PADDING;
                    for (index, text) in [delta_top, delta_bottom].into_iter().enumerate() {
                        let size = [primary_size, secondary_size][index];
                        out.push(Prim::Text {
                            x: (delta_center * hpr) as f32,
                            y: ((y + size * 0.5) * vpr) as f32,
                            text,
                            color: delta_text,
                            size: (size * vpr) as f32,
                            family: family.into(),
                            align: TextAlign::Center,
                            weight: [590, 400][index],
                            italic: false,
                        });
                        y += [primary_size + 4.0, secondary_size + 4.0][index];
                    }
                }
            }
        }
    }

    /// Official anchored-text placement: viewport-relative alignment with fixed 20px/10px
    /// horizontal/vertical margins. Text stays in the shared pane frame and is independent of
    /// time/price coordinates or source data visibility.
    pub(super) fn build_native_anchored_text_frame(
        &self,
        pane_index: usize,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        let Some(pane) = self.panes.get(pane_index) else {
            return;
        };
        for primitive in self
            .series
            .iter()
            .filter(|series| series.visible && series.pane_index == pane_index)
            .flat_map(|series| &series.native_primitives)
        {
            let NativeSeriesPrimitiveKind::AnchoredText(options) = &primitive.kind else {
                continue;
            };
            let (x, align) = match options.horizontal_align {
                AnchoredTextHorizontalAlign::Left => (20.0, TextAlign::Left),
                AnchoredTextHorizontalAlign::Middle => (self.pane_w * 0.5, TextAlign::Center),
                AnchoredTextHorizontalAlign::Right => {
                    ((self.pane_w - 20.0).max(0.0), TextAlign::Right)
                }
            };
            // Prim::Text uses a middle baseline. Convert the reference renderer's alphabetic
            // baseline placement using its declared line-height box.
            let y = match options.vertical_align {
                AnchoredTextVerticalAlign::Top => 10.0 + options.line_height * 0.5,
                AnchoredTextVerticalAlign::Middle => pane.height * 0.5,
                AnchoredTextVerticalAlign::Bottom => {
                    (pane.height - 10.0 - options.line_height * 0.5).max(0.0)
                }
            };
            out.push(Prim::Text {
                x: (x * hpr) as f32,
                y: ((pane.top + y) * vpr) as f32,
                text: options.text.clone(),
                color: options.color,
                size: (options.font_size * vpr) as f32,
                family: options.font_family.clone(),
                align,
                weight: options.font_weight,
                italic: options.italic,
            });
        }
    }

    pub(super) fn build_native_text_watermark_frame(
        &self,
        pane_index: usize,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        let Some(pane) = self.panes.get(pane_index) else {
            return;
        };
        let Some(pane_id) = pane.stable_id() else {
            return;
        };
        for primitive in self
            .native_pane_primitives
            .iter()
            .filter(|primitive| primitive.pane_id == pane_id)
        {
            let NativePanePrimitiveKind::TextWatermark(options) = &primitive.kind;
            if !options.visible {
                continue;
            }
            let laid = options
                .lines
                .iter()
                .filter(|line| !line.text.is_empty())
                .map(|line| {
                    let width = self.measure_text_run(
                        &line.text,
                        line.font_size,
                        &line.font_family,
                        line.font_weight,
                        line.italic,
                    );
                    let zoom = if width > self.pane_w && width > 0.0 {
                        self.pane_w / width
                    } else {
                        1.0
                    };
                    (line, zoom)
                })
                .collect::<Vec<_>>();
            let text_height = laid
                .iter()
                .map(|(line, zoom)| line.line_height * zoom)
                .sum::<f64>();
            let mut y = match options.vertical_align {
                AnchoredTextVerticalAlign::Top => 0.0,
                AnchoredTextVerticalAlign::Middle => ((pane.height - text_height) * 0.5).max(0.0),
                AnchoredTextVerticalAlign::Bottom => (pane.height - text_height).max(0.0),
            };
            for (line, zoom) in laid {
                let (x, align) = match options.horizontal_align {
                    AnchoredTextHorizontalAlign::Left => (line.line_height * 0.5, TextAlign::Left),
                    AnchoredTextHorizontalAlign::Middle => (self.pane_w * 0.5, TextAlign::Center),
                    AnchoredTextHorizontalAlign::Right => (
                        (self.pane_w - 1.0 - line.line_height * 0.5).max(0.0),
                        TextAlign::Right,
                    ),
                };
                let size = line.font_size * zoom;
                out.push(Prim::Text {
                    x: (x * hpr) as f32,
                    y: ((pane.top + y + size * 0.5) * vpr) as f32,
                    text: line.text.clone(),
                    color: line.color,
                    size: (size * vpr) as f32,
                    family: line.font_family.clone(),
                    align,
                    weight: line.font_weight,
                    italic: line.italic,
                });
                y += line.line_height * zoom;
            }
        }
    }

    /// Official image-watermark placement: center the source aspect ratio in the plot area after
    /// padding and optional maximum dimensions. The image is the attached series' bottom view.
    pub(super) fn build_native_image_watermark_frame(
        &self,
        pane_index: usize,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        let Some(pane) = self.panes.get(pane_index) else {
            return;
        };
        for primitive in self
            .series
            .iter()
            .filter(|series| series.visible && series.pane_index == pane_index)
            .flat_map(|series| &series.native_primitives)
        {
            let NativeSeriesPrimitiveKind::ImageWatermark { image, options } = &primitive.kind
            else {
                continue;
            };
            let mut available_width = self.pane_w - options.padding * 2.0;
            let mut available_height = pane.height - options.padding * 2.0;
            if let Some(max_width) = options.max_width {
                available_width = available_width.min(max_width);
            }
            if let Some(max_height) = options.max_height {
                available_height = available_height.min(max_height);
            }
            if available_width <= 0.0 || available_height <= 0.0 || options.alpha <= 0.0 {
                continue;
            }
            let scale = (available_width / f64::from(image.width))
                .min(available_height / f64::from(image.height));
            if !scale.is_finite() || scale <= 0.0 {
                continue;
            }
            let width = f64::from(image.width) * scale;
            let height = f64::from(image.height) * scale;
            out.push(Prim::Image {
                image: image.clone(),
                rect: [
                    ((self.pane_w - width) * 0.5 * hpr) as f32,
                    ((pane.top + (pane.height - height) * 0.5) * vpr) as f32,
                    (width * hpr) as f32,
                    (height * vpr) as f32,
                ],
                opacity: options.alpha as f32,
            });
        }
    }

    /// Official session-highlighting semantics: color every source bar, derive the slot width from
    /// the first two source coordinates, clip in bitmap space, and paint below all series. An
    /// as-of overlay colors each visible point, one bar spacing wide, as the row it shows.
    pub(super) fn build_native_session_highlighting_frame(
        &self,
        pane_index: usize,
        from: i64,
        to: i64,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        let pane = &self.panes[pane_index];
        let pane_width = (self.pane_w * hpr).round() as i32;
        let y = (pane.top * vpr).round() as i32;
        let height = (pane.height * vpr).round().max(1.0) as i32;
        for series in self
            .series
            .iter()
            .filter(|series| series.visible && series.pane_index == pane_index)
        {
            let Some((times, _)) = self.data.series_data(series.id) else {
                continue;
            };
            let plot = self.data.plot(series.id);
            let bar_width = if plot.is_as_of() {
                // As-of points sit on consecutive chart slots, not on the series' own times.
                self.time_scale.bar_spacing()
            } else if times.len() > 1 {
                let Some(first) = self.time_to_coordinate(times[0] as f64) else {
                    continue;
                };
                let Some(second) = self.time_to_coordinate(times[1] as f64) else {
                    continue;
                };
                second - first
            } else {
                6.0
            };
            let half_width = hpr * bar_width / 2.0;
            for primitive in &series.native_primitives {
                let NativeSeriesPrimitiveKind::SessionHighlighting(state) = &primitive.kind else {
                    continue;
                };
                let mut pending: Option<(IRect, Color)> = None;
                {
                    let mut append = |logical: i64, color: Color| {
                        if logical < from || logical > to {
                            return;
                        }
                        let x = self.time_scale.index_to_coordinate(logical) * hpr;
                        let left = (x - half_width).round().max(0.0) as i32;
                        let right = (x + half_width).round().min(pane_width as f64) as i32;
                        if right <= left {
                            return;
                        }
                        let rect = IRect {
                            x: left,
                            y,
                            w: right - left,
                            h: height,
                        };
                        match pending.as_mut() {
                            Some((previous, previous_color))
                                if *previous_color == color
                                    && rect.x <= previous.x.saturating_add(previous.w) =>
                            {
                                previous.w = previous.w.max(rect.x + rect.w - previous.x);
                            }
                            Some(_) => {
                                let (rect, color) = pending.replace((rect, color)).unwrap();
                                out.push(Prim::Rect { rect, color });
                            }
                            None => pending = Some((rect, color)),
                        }
                    };
                    let at = |time: i64| self.time_to_index(time as f64, false);
                    if plot.is_as_of() {
                        // Each visible point is highlighted as the row it shows.
                        for row in plot.visible_rows(from, to) {
                            let (Some(logical), Some(&time)) =
                                (plot.index_at(row), times.get(plot.source_row(row)))
                            else {
                                continue;
                            };
                            let color = match &state.highlights {
                                Some(highlights) => highlights
                                    .binary_search_by_key(&time, |highlight| highlight.time)
                                    .ok()
                                    .map(|index| highlights[index].color),
                                None => session_color(time, state.options, &self.exchange_time),
                            };
                            if let Some(color) = color {
                                append(logical, color);
                            }
                        }
                    } else if let Some(highlights) = &state.highlights {
                        for highlight in highlights {
                            if let Some(logical) = at(highlight.time) {
                                append(logical, highlight.color);
                            }
                        }
                    } else {
                        for &time in times {
                            if let Some(color) =
                                session_color(time, state.options, &self.exchange_time)
                            {
                                if let Some(logical) = at(time) {
                                    append(logical, color);
                                }
                            }
                        }
                    }
                }
                if let Some((rect, color)) = pending {
                    out.push(Prim::Rect { rect, color });
                }
            }
        }
    }

    /// Cursor highlight is retained separately from the static underlay so movement rebuilds one
    /// rectangle instead of the grid and every series.
    pub(super) fn build_native_crosshair_highlight_frame(
        &self,
        pane_index: usize,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        let Some((x, _)) = self.clamped_crosshair() else {
            return;
        };
        let x = self
            .time_scale
            .index_to_coordinate(self.snapped_crosshair_index(x));
        let pane = &self.panes[pane_index];
        let (left, width) = positions_line(x, hpr, self.time_scale.bar_spacing());
        for series in self
            .series
            .iter()
            .filter(|series| series.visible && series.pane_index == pane_index)
        {
            for primitive in &series.native_primitives {
                let NativeSeriesPrimitiveKind::HighlightBarCrosshair { color } = primitive.kind
                else {
                    continue;
                };
                let color =
                    surface_contrast_line_color(color, &self.options.get().layout.background.color);
                out.push(Prim::Rect {
                    rect: IRect {
                        x: left,
                        y: (pane.top * vpr).round() as i32,
                        w: width,
                        h: (pane.height * vpr).round().max(1.0) as i32,
                    },
                    color,
                });
            }
        }
    }

    /// Visible accessibility focus ring. Keyboard and ARIA semantics live at the host boundary;
    /// the selected source time, exact value lookup, device-pixel geometry, and backend parity are
    /// owned by the shared engine.
    pub(super) fn build_native_accessibility_focus_frame(
        &self,
        pane_index: usize,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        let transparent = Color::rgba(0, 0, 0, 0);
        for series in self
            .series
            .iter()
            .filter(|series| series.visible && series.pane_index == pane_index)
        {
            for primitive in &series.native_primitives {
                let NativeSeriesPrimitiveKind::AccessibilityFocus(state) = primitive.kind else {
                    continue;
                };
                let Some(time) = state.time else {
                    continue;
                };
                let plot = self.data.plot(series.id);
                let row = if plot.is_as_of() {
                    // An overlay row is focused on the first point showing it; a row collapsed
                    // into a later one or waiting past the last point has no point.
                    self.data
                        .series_data(series.id)
                        .and_then(|(times, _)| times.binary_search(&time).ok())
                        .and_then(|row| plot.row_for_source(row))
                } else {
                    self.time_to_index(time as f64, false).and_then(|logical| {
                        plot.search(
                            logical,
                            aeris_charts_core::model::plot_list::MismatchDirection::None,
                        )
                    })
                };
                let Some((row, logical)) =
                    row.and_then(|row| plot.index_at(row).map(|logical| (row, logical)))
                else {
                    continue;
                };
                if plot.is_whitespace_row(row) {
                    continue;
                }
                let price = plot.value_at(
                    row,
                    aeris_charts_core::model::plot_list::PlotValueIndex::Close,
                );
                let Some(y) = self.series_price_to_coordinate(series.id, price) else {
                    continue;
                };
                let x = self.time_scale.index_to_coordinate(logical);
                if x < 0.0 || x > self.pane_w {
                    continue;
                }
                let cx = (x * hpr) as f32;
                let cy = (y * vpr) as f32;
                let radius = (state.options.size * 0.5 * hpr.min(vpr)) as f32;
                if state.options.high_contrast {
                    out.push(Prim::Circle {
                        cx,
                        cy,
                        radius: radius + (3.0 * hpr.min(vpr)) as f32,
                        fill: transparent,
                        stroke_width: (4.0 * hpr.min(vpr)) as f32,
                        stroke: Color::rgb(0, 0, 0),
                    });
                    out.push(Prim::Circle {
                        cx,
                        cy,
                        radius: radius + hpr.min(vpr) as f32,
                        fill: transparent,
                        stroke_width: (4.0 * hpr.min(vpr)) as f32,
                        stroke: Color::rgb(255, 255, 255),
                    });
                } else {
                    out.push(Prim::Circle {
                        cx,
                        cy,
                        radius: radius + hpr.min(vpr) as f32,
                        fill: transparent,
                        stroke_width: (4.0 * hpr.min(vpr)) as f32,
                        stroke: Color::rgba(255, 255, 255, 230),
                    });
                }
                out.push(Prim::Circle {
                    cx,
                    cy,
                    radius,
                    fill: transparent,
                    stroke_width: ((if state.options.high_contrast {
                        3.0
                    } else {
                        2.0
                    }) * hpr.min(vpr)) as f32,
                    stroke: state.options.color,
                });
            }
        }
    }

    /// Official tooltip primitive's bottom-layer vertical guide. Pointer normalization and exact
    /// source-row lookup are shared with [`ChartEngine::tooltip_snapshot`]; the DOM tooltip is
    /// only presentation chrome at the browser boundary.
    pub(super) fn build_native_tooltip_crosshair_frame(
        &self,
        pane_index: usize,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        let pane = &self.panes[pane_index];
        for series in self
            .series
            .iter()
            .filter(|series| series.visible && series.pane_index == pane_index)
        {
            for primitive in &series.native_primitives {
                let NativeSeriesPrimitiveKind::Tooltip(options) = primitive.kind else {
                    continue;
                };
                let Some(snapshot) = self.tooltip_snapshot(primitive.id) else {
                    continue;
                };
                let (x, width) = positions_line(snapshot.x, hpr, 1.0);
                let top = pane.top + options.top_margin;
                let bottom = pane.top + pane.height;
                if top >= bottom {
                    continue;
                }
                out.push(Prim::Rect {
                    rect: IRect {
                        x,
                        y: (top * vpr).round() as i32,
                        w: width,
                        h: ((bottom - top) * vpr).round().max(1.0) as i32,
                    },
                    color: surface_contrast_line_color(
                        options.line_color,
                        &self.options.get().layout.background.color,
                    ),
                });
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_native_series_primitives_frame(
        &self,
        rs: ResolvedSeries,
        from: i64,
        to: i64,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
        points: &mut Vec<[f32; 2]>,
        scale: &PriceScaleCore,
    ) {
        let Some(series) = self.series_entry(rs.id) else {
            return;
        };
        if series.native_primitives.is_empty() || scale.is_empty() {
            return;
        }
        let Some(base_value) = self.series_base_value(rs.id, from) else {
            return;
        };
        for primitive in &series.native_primitives {
            match &primitive.kind {
                NativeSeriesPrimitiveKind::AnchoredText(_)
                | NativeSeriesPrimitiveKind::ImageWatermark { .. } => {}
                NativeSeriesPrimitiveKind::BandsIndicator(_) => {}
                NativeSeriesPrimitiveKind::OverlayPriceScale(options) => {
                    const TICK_SPACING: f64 = 40.0;
                    const HALF_TICK: f64 = 10.0;
                    const SIDE_MARGIN: f64 = 10.0;
                    const FONT_SIZE: f64 = 12.0;
                    const FAMILY: &str = "-apple-system, BlinkMacSystemFont, 'Trebuchet MS', Roboto, Ubuntu, sans-serif";
                    let pane = &self.panes[series.pane_index];
                    let mut labels = Vec::new();
                    let mut position = HALF_TICK;
                    while position <= pane.height - HALF_TICK {
                        let coordinate = pane.top + position;
                        let price = scale.coordinate_to_price(coordinate, base_value);
                        let logical = scale.price_to_logical_value(price, base_value);
                        labels.push((position, self.format_series_value(series, scale, logical)));
                        position += TICK_SPACING;
                    }
                    let max_label_length = labels
                        .iter()
                        .map(|(_, label)| label.chars().count())
                        .max()
                        .unwrap_or(0);
                    if max_label_length == 0 {
                        continue;
                    }
                    let test_label = "0".repeat(max_label_length);
                    let width = self.measure_text_run(&test_label, FONT_SIZE, FAMILY, 400, false);
                    let text_x = match options.side {
                        OverlayPriceScaleSide::Left => SIDE_MARGIN + width / 2.0,
                        OverlayPriceScaleSide::Right => self.pane_w - SIDE_MARGIN - width / 2.0,
                    };
                    let text_color = options
                        .text_color
                        .unwrap_or_else(|| self.primary_text_color());
                    for (position, label) in labels {
                        out.push(Prim::Text {
                            x: (text_x * hpr) as f32,
                            y: ((pane.top + position) * vpr) as f32,
                            text: label,
                            color: text_color,
                            size: (FONT_SIZE * vpr) as f32,
                            family: FAMILY.into(),
                            align: TextAlign::Center,
                            weight: 400,
                            italic: false,
                        });
                    }
                }
                NativeSeriesPrimitiveKind::VolumeProfileIndicator(state) => {
                    if !state.options.visible {
                        continue;
                    }
                    let profile = &state.snapshot.profile;
                    let Some(poc) = profile.poc_index else {
                        continue;
                    };
                    let max_volume = profile.rows[poc].volume;
                    if max_volume <= 0.0 {
                        continue;
                    }
                    let right = self.pane_w.max(0.0);
                    let width = right * state.options.width_percent / 100.0;
                    let up_color =
                        Color::parse_css(&state.options.up_color).expect("validated up color");
                    let down_color =
                        Color::parse_css(&state.options.down_color).expect("validated down color");
                    let area_up_color = Color::parse_css(&state.options.value_area_up_color)
                        .expect("validated value-area up color");
                    let area_down_color = Color::parse_css(&state.options.value_area_down_color)
                        .expect("validated value-area down color");
                    let poc_color =
                        Color::parse_css(&state.options.poc_color).expect("validated POC color");
                    for (index, row) in profile.rows.iter().enumerate() {
                        if row.volume <= 0.0 {
                            continue;
                        }
                        let right_px = (right * hpr).round() as i32;
                        let x = ((right - width * row.volume / max_volume) * hpr).round() as i32;
                        let w = (right_px - x).max(1);
                        let (y, h) = positions_box(
                            scale.price_to_coordinate(row.low, base_value),
                            scale.price_to_coordinate(row.high, base_value),
                            vpr,
                        );
                        let in_area = state.options.show_value_area
                            && profile.value_area_low_index.is_some_and(|low| index >= low)
                            && profile
                                .value_area_high_index
                                .is_some_and(|high| index <= high);
                        let up_width = ((f64::from(w) * row.up_volume / row.volume).round() as i32)
                            .clamp(0, w);
                        if up_width > 0 {
                            out.push(Prim::Rect {
                                rect: IRect {
                                    x,
                                    y,
                                    w: up_width,
                                    h: h.max(1),
                                },
                                color: if in_area { area_up_color } else { up_color },
                            });
                        }
                        if w > up_width {
                            out.push(Prim::Rect {
                                rect: IRect {
                                    x: x + up_width,
                                    y,
                                    w: w - up_width,
                                    h: h.max(1),
                                },
                                color: if in_area { area_down_color } else { down_color },
                            });
                        }
                    }
                    if state.options.show_poc {
                        let price = profile.rows[poc].low
                            + (profile.rows[poc].high - profile.rows[poc].low) * 0.5;
                        out.push(Prim::HLine {
                            y: (scale.price_to_coordinate(price, base_value) * vpr).round() as i32,
                            x0: ((right - width) * hpr).round() as i32,
                            x1: (right * hpr).round() as i32,
                            width: vpr.floor().max(1.0) as i32,
                            style: LineStyle::Solid,
                            color: poc_color,
                        });
                    }
                }
                NativeSeriesPrimitiveKind::VolumeProfile { data, options } => {
                    let Some(logical) = self.time_to_index(data.time as f64, false) else {
                        continue;
                    };
                    if to < logical || from as f64 > logical as f64 + data.width {
                        continue;
                    }
                    let x = self.time_scale.index_to_coordinate(logical);
                    let width = self.time_scale.bar_spacing() * data.width;
                    let y1 = scale.price_to_coordinate(data.profile[0].price, base_value);
                    let y2 = scale.price_to_coordinate(data.profile[1].price, base_value);
                    let column_height = (y1 - y2).max(1.0);
                    let max_volume = data
                        .profile
                        .iter()
                        .map(|point| point.volume)
                        .fold(0.0_f64, f64::max);
                    let (background_x, background_width) = positions_box(x, x + width, hpr);
                    let (background_y, background_height) =
                        positions_box(y1, y1 - column_height * data.profile.len() as f64, vpr);
                    out.push(Prim::Rect {
                        rect: IRect {
                            x: background_x,
                            y: background_y,
                            w: background_width,
                            h: background_height,
                        },
                        color: options.background_color,
                    });
                    for point in &data.profile {
                        if point.volume <= 0.0 {
                            continue;
                        }
                        let row_y = scale.price_to_coordinate(point.price, base_value);
                        let row_width = width * point.volume / max_volume;
                        let (row_x, row_width) = positions_box(x, x + row_width, hpr);
                        let (row_y, row_height) = positions_box(row_y, row_y - column_height, vpr);
                        out.push(Prim::Rect {
                            rect: IRect {
                                x: row_x,
                                y: row_y,
                                w: row_width,
                                h: (row_height - 2).max(1),
                            },
                            color: options.row_color,
                        });
                    }
                }
                NativeSeriesPrimitiveKind::VerticalLine { time, options } => {
                    let Some(x) = self.time_to_coordinate(*time as f64) else {
                        continue;
                    };
                    let pane = &self.panes[series.pane_index];
                    let (x, width) = positions_line(x, hpr, options.width);
                    out.push(Prim::Rect {
                        rect: IRect {
                            x,
                            y: (pane.top * vpr).round() as i32,
                            w: width,
                            h: (pane.height * vpr).round().max(1.0) as i32,
                        },
                        color: options.color,
                    });
                }
                NativeSeriesPrimitiveKind::TrendLine {
                    first_time,
                    first_price,
                    second_time,
                    second_price,
                    options,
                } => {
                    let (Some(first_x), Some(second_x)) = (
                        self.time_to_coordinate(*first_time as f64),
                        self.time_to_coordinate(*second_time as f64),
                    ) else {
                        continue;
                    };
                    let first_y = scale.price_to_coordinate(*first_price, base_value);
                    let second_y = scale.price_to_coordinate(*second_price, base_value);
                    let first = [
                        (first_x * hpr).round() as f32,
                        (first_y * vpr).round() as f32,
                    ];
                    let second = [
                        (second_x * hpr).round() as f32,
                        (second_y * vpr).round() as f32,
                    ];
                    let first_point = points.len() as u32;
                    points.extend([first, second]);
                    out.push(Prim::Polyline {
                        first_point,
                        point_count: 2,
                        width: options.width as f32,
                        style: LineStyle::Solid,
                        line_type: LineType::Simple,
                        color: options.line_color,
                    });
                    if options.show_labels {
                        let first_text = format!("{first_price:.1}");
                        let second_text = format!("{second_price:.1}");
                        self.build_native_trend_label(first, &first_text, true, hpr, options, out);
                        self.build_native_trend_label(
                            second,
                            &second_text,
                            false,
                            hpr,
                            options,
                            out,
                        );
                    }
                }
                NativeSeriesPrimitiveKind::AccessibilityFocus(_)
                | NativeSeriesPrimitiveKind::SessionHighlighting(_)
                | NativeSeriesPrimitiveKind::HighlightBarCrosshair { .. }
                | NativeSeriesPrimitiveKind::Tooltip(_)
                | NativeSeriesPrimitiveKind::DeltaTooltip(_) => {}
            }
        }
    }

    fn build_native_trend_label(
        &self,
        point: [f32; 2],
        text: &str,
        left: bool,
        hpr: f64,
        options: &crate::TrendLineOptions,
        out: &mut Vec<Prim>,
    ) {
        const SIZE: f64 = 24.0;
        let offset = 5.0 * hpr;
        let text_width = self.measure_text_run(text, SIZE, "Arial", 400, false);
        let left_adjustment = if left { text_width + offset * 4.0 } else { 0.0 };
        let x = f64::from(point[0]);
        let y = f64::from(point[1]);
        out.push(Prim::RoundRect {
            x: (x + offset - left_adjustment) as f32,
            y: (y - SIZE) as f32,
            w: (text_width + offset * 2.0) as f32,
            h: (SIZE + offset) as f32,
            radii: [5.0; 4],
            fill: options.label_background_color,
            border_width: 0.0,
            border_color: options.label_background_color,
        });
        out.push(Prim::Text {
            x: (x + offset * 2.0 - left_adjustment) as f32,
            y: (y - SIZE * 0.5) as f32,
            text: text.to_string(),
            color: options.label_text_color,
            size: SIZE as f32,
            family: "Arial".into(),
            align: TextAlign::Left,
            weight: 400,
            italic: false,
        });
    }
}
