use super::*;
use crate::footprint::{
    FootprintAggregationOptions, FootprintBar, FootprintCellMode, footprint_row_price_bounds,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FootprintLod {
    Detailed,
    Cells,
    Summary,
}

#[derive(Clone, Copy)]
struct FootprintTextStyle<'a> {
    fallback_color: Color,
    explicit_text: bool,
    surface: Color,
    size: f32,
    /// Bar summaries stay at the configured scale even when cell numbers grow
    /// into tall rows — grown summaries would collide across adjacent bars.
    summary: f32,
    family: &'a str,
    pixel_ratio: f64,
}

impl ChartEngine {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_footprint_series_frame(
        &self,
        rs: ResolvedSeries,
        from: i64,
        to: i64,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
        scale: &PriceScaleCore,
    ) {
        let Some(state) = self
            .series_entry(rs.id)
            .and_then(|series| series.footprint.as_ref())
        else {
            return;
        };
        let Some(stream) = self.trade_stream(state.trade_stream_id) else {
            return;
        };
        let plot = self.data.plot(rs.id);
        let spacing = self.time_scale.bar_spacing();
        let font_size = state.visual.font_size;
        let layout = &self.options.get().layout;
        let family = layout.font_family.clone();
        let fallback_text = state
            .visual
            .text_color
            .or_else(|| Color::parse_css(&layout.text_color))
            .unwrap_or(Color::rgb(215, 219, 228));
        let explicit_text = state.visual.text_color.is_some();
        let surface = Color::parse_css(&layout.background.color).unwrap_or(Color::rgb(
            aeris_charts_core::style::DEFAULT_SURFACE_RGB.0,
            aeris_charts_core::style::DEFAULT_SURFACE_RGB.1,
            aeris_charts_core::style::DEFAULT_SURFACE_RGB.2,
        ));
        // Thin central spine so gapped levels still read as one bar (Sierra wick marker).
        let spine = fallback_text_to_spine(fallback_text);
        let text_style = FootprintTextStyle {
            fallback_color: fallback_text,
            explicit_text,
            surface,
            size: (font_size * vpr) as f32,
            summary: (font_size * vpr) as f32 * 0.85,
            family: &family,
            pixel_ratio: vpr,
        };
        for row in plot.visible_rows(from, to) {
            let Some(bar) = stream.bars().get(row) else {
                continue;
            };
            let Some(logical) = plot.index_at(row) else {
                continue;
            };
            let x = self.time_scale.index_to_coordinate(logical);
            let left = ((x - spacing * 0.48) * hpr).round() as i32;
            let right = ((x + spacing * 0.48) * hpr).round() as i32;
            let width = (right - left).max(1);
            let aggregation = stream.options();
            let representative_row_height = bar
                .levels
                .first()
                .map(|level| {
                    let (lower_price, upper_price) =
                        footprint_row_price_bounds(&aggregation, level.price, level.price);
                    let upper = scale.price_to_coordinate(upper_price, rs.base_value);
                    let lower = scale.price_to_coordinate(lower_price, rs.base_value);
                    (lower - upper).abs()
                })
                .unwrap_or(0.0);
            let lod = if spacing >= 64.0 && representative_row_height >= font_size + 3.0 {
                FootprintLod::Detailed
            } else if spacing >= 10.0 && representative_row_height * vpr >= 2.0 {
                FootprintLod::Cells
            } else {
                FootprintLod::Summary
            };
            match lod {
                FootprintLod::Detailed | FootprintLod::Cells => {
                    // Volume-profile heatmap: intensity is relative to this bar so one
                    // quiet bar never washes out beside one active bar.
                    let (max_bid, max_ask, max_total, max_abs_delta) = bar_volume_maxima(bar);
                    // Wick spine behind the cells.
                    let (low_price, high_price) =
                        footprint_row_price_bounds(&aggregation, bar.low, bar.high);
                    let high = scale.price_to_coordinate(high_price, rs.base_value);
                    let low = scale.price_to_coordinate(low_price, rs.base_value);
                    let spine_top = (high.min(low) * vpr).round() as i32;
                    let spine_bottom = (high.max(low) * vpr).round() as i32;
                    let spine_w = (1.0 * hpr).round().max(1.0) as i32;
                    let spine_x = left + (width - spine_w) / 2;
                    out.push(Prim::Rect {
                        rect: IRect {
                            x: spine_x,
                            y: spine_top,
                            w: spine_w,
                            h: (spine_bottom - spine_top).max(1),
                        },
                        color: spine,
                    });
                    let detailed = lod == FootprintLod::Detailed;
                    // Numbers fill tall rows the way professional footprints do: grow
                    // past the configured size up to ~55% of the row, capped by the
                    // column width so glyphs never spill sideways. Never shrinks —
                    // short rows are already gated out of Detailed.
                    let text_style = if detailed {
                        let row_px = (representative_row_height * vpr) as f32;
                        let span = match state.visual.cell_mode {
                            FootprintCellMode::BidAsk
                            | FootprintCellMode::HorizontalImbalance
                            | FootprintCellMode::BidAskHistogram => {
                                let split = left + width / 2;
                                let left_w = (split - left).max(1);
                                let right_w =
                                    (right - (split + i32::from(width > 3)).min(right)).max(1);
                                left_w.min(right_w) as f32
                            }
                            FootprintCellMode::Total
                            | FootprintCellMode::Delta
                            | FootprintCellMode::ProfileInBar
                            | FootprintCellMode::VolumeLadder => width as f32,
                        };
                        let mut chars = 1usize;
                        for level in &bar.levels {
                            match state.visual.cell_mode {
                                FootprintCellMode::BidAsk
                                | FootprintCellMode::HorizontalImbalance
                                | FootprintCellMode::BidAskHistogram => {
                                    if level.bid_volume > 0.0 {
                                        chars = chars.max(compact_volume(level.bid_volume).len());
                                    }
                                    if level.ask_volume > 0.0 {
                                        chars = chars.max(compact_volume(level.ask_volume).len());
                                    }
                                }
                                FootprintCellMode::Total
                                | FootprintCellMode::ProfileInBar
                                | FootprintCellMode::VolumeLadder => {
                                    chars = chars.max(compact_volume(level.total_volume).len());
                                }
                                FootprintCellMode::Delta => {
                                    if level.delta != 0.0 {
                                        chars = chars.max(compact_volume(level.delta).len());
                                    }
                                }
                            }
                        }
                        let grown = (row_px * 0.55).min(span * 0.94 / (0.58 * chars as f32));
                        FootprintTextStyle {
                            size: text_style.size.max(grown),
                            ..text_style
                        }
                    } else {
                        text_style
                    };
                    for level in &bar.levels {
                        let (lower_price, upper_price) =
                            footprint_row_price_bounds(&aggregation, level.price, level.price);
                        let upper = scale.price_to_coordinate(upper_price, rs.base_value);
                        let lower = scale.price_to_coordinate(lower_price, rs.base_value);
                        let top = (upper.min(lower) * vpr).round() as i32;
                        let bottom = (upper.max(lower) * vpr).round() as i32;
                        // Leave a 1px row gap so stacked levels read as distinct cells
                        // without spending an extra separator prim per level.
                        let full_height = (bottom - top).max(1);
                        let height = if full_height > 2 {
                            full_height - 1
                        } else {
                            full_height
                        };
                        let center_y = top as f32 + height as f32 / 2.0;
                        let is_poc = level.level == bar.poc_level;
                        match state.visual.cell_mode {
                            FootprintCellMode::BidAsk
                            | FootprintCellMode::HorizontalImbalance
                            | FootprintCellMode::BidAskHistogram => {
                                // 1px center gap doubles as the bid/ask divider.
                                let split = left + width / 2;
                                let left_w = (split - left).max(1);
                                let right_x = (split + i32::from(width > 3)).min(right);
                                let right_w = (right - right_x).max(1);
                                let bid_im = level.bid_imbalance || level.stacked_bid_imbalance;
                                let ask_im = level.ask_imbalance || level.stacked_ask_imbalance;
                                // POC edges always mark the row; the yellow wash applies
                                // only when the row carries no imbalance so one signal
                                // never erases the other.
                                let imbalanced = bid_im || ask_im;
                                let poc_wash = is_poc && !imbalanced;
                                // Bid half: faint track plus a center-anchored profile bar
                                // whose length is the volume shape professionals read.
                                let mut bid_text_bg = track_cell(state.visual.bid_color);
                                if bid_im {
                                    let bg = imbalance_cell(
                                        state.visual.stacked_bid_color,
                                        level.stacked_bid_imbalance,
                                    );
                                    out.push(Prim::Rect {
                                        rect: IRect {
                                            x: left,
                                            y: top,
                                            w: left_w,
                                            h: height,
                                        },
                                        color: bg,
                                    });
                                    bid_text_bg = bg;
                                } else {
                                    out.push(Prim::Rect {
                                        rect: IRect {
                                            x: left,
                                            y: top,
                                            w: left_w,
                                            h: height,
                                        },
                                        color: bid_text_bg,
                                    });
                                    let bar_w = profile_width(left_w, level.bid_volume, max_bid);
                                    if bar_w > 0 {
                                        let mut bar = profile_bar(
                                            state.visual.bid_color,
                                            level.bid_volume,
                                            max_bid,
                                        );
                                        if poc_wash {
                                            bar = poc_bar(bar, state.visual.poc_color);
                                        }
                                        out.push(Prim::Rect {
                                            rect: IRect {
                                                x: split - bar_w,
                                                y: top,
                                                w: bar_w,
                                                h: height,
                                            },
                                            color: bar,
                                        });
                                        if bar_w * 4 >= left_w {
                                            bid_text_bg = bar;
                                        }
                                    } else if poc_wash {
                                        bid_text_bg = poc_bar(bid_text_bg, state.visual.poc_color);
                                    }
                                }
                                // Ask half mirrors the bid half.
                                let mut ask_text_bg = track_cell(state.visual.ask_color);
                                if ask_im {
                                    let bg = imbalance_cell(
                                        state.visual.stacked_ask_color,
                                        level.stacked_ask_imbalance,
                                    );
                                    out.push(Prim::Rect {
                                        rect: IRect {
                                            x: right_x,
                                            y: top,
                                            w: right_w,
                                            h: height,
                                        },
                                        color: bg,
                                    });
                                    ask_text_bg = bg;
                                } else {
                                    out.push(Prim::Rect {
                                        rect: IRect {
                                            x: right_x,
                                            y: top,
                                            w: right_w,
                                            h: height,
                                        },
                                        color: ask_text_bg,
                                    });
                                    let bar_w = profile_width(right_w, level.ask_volume, max_ask);
                                    if bar_w > 0 {
                                        let mut bar = profile_bar(
                                            state.visual.ask_color,
                                            level.ask_volume,
                                            max_ask,
                                        );
                                        if poc_wash {
                                            bar = poc_bar(bar, state.visual.poc_color);
                                        }
                                        out.push(Prim::Rect {
                                            rect: IRect {
                                                x: right_x,
                                                y: top,
                                                w: bar_w,
                                                h: height,
                                            },
                                            color: bar,
                                        });
                                        if bar_w * 4 >= right_w {
                                            ask_text_bg = bar;
                                        }
                                    } else if poc_wash {
                                        ask_text_bg = poc_bar(ask_text_bg, state.visual.poc_color);
                                    }
                                }
                                if is_poc {
                                    push_poc_stripe(
                                        out,
                                        left,
                                        top,
                                        height,
                                        hpr,
                                        state.visual.poc_color,
                                    );
                                }
                                if detailed && height as f32 >= text_style.size * 1.05 {
                                    let bid_bold = is_poc || bid_im;
                                    let ask_bold = is_poc || ask_im;
                                    if level.bid_volume > 0.0 {
                                        push_cell_text(
                                            out,
                                            left as f32 + left_w as f32 / 2.0,
                                            center_y,
                                            compact_volume(level.bid_volume),
                                            &text_style,
                                            contrast_on(bid_text_bg, &text_style),
                                            bid_bold,
                                        );
                                    }
                                    if level.ask_volume > 0.0 {
                                        push_cell_text(
                                            out,
                                            right_x as f32 + right_w as f32 / 2.0,
                                            center_y,
                                            compact_volume(level.ask_volume),
                                            &text_style,
                                            contrast_on(ask_text_bg, &text_style),
                                            ask_bold,
                                        );
                                    }
                                }
                            }
                            FootprintCellMode::Total
                            | FootprintCellMode::Delta
                            | FootprintCellMode::ProfileInBar
                            | FootprintCellMode::VolumeLadder => {
                                let total_mode = matches!(
                                    state.visual.cell_mode,
                                    FootprintCellMode::Total
                                        | FootprintCellMode::ProfileInBar
                                        | FootprintCellMode::VolumeLadder
                                );
                                let (value, base, peak) = if total_mode {
                                    let base = if level.ask_volume >= level.bid_volume {
                                        state.visual.ask_color
                                    } else {
                                        state.visual.bid_color
                                    };
                                    (level.total_volume, base, max_total)
                                } else {
                                    let base = if level.delta >= 0.0 {
                                        state.visual.positive_delta_color
                                    } else {
                                        state.visual.negative_delta_color
                                    };
                                    (level.delta, base, max_abs_delta)
                                };
                                let imbalanced = level.bid_imbalance
                                    || level.ask_imbalance
                                    || level.stacked_bid_imbalance
                                    || level.stacked_ask_imbalance;
                                let poc_wash = is_poc && !imbalanced;
                                let mut text_bg = track_cell(base);
                                if imbalanced {
                                    let stacked =
                                        level.stacked_bid_imbalance || level.stacked_ask_imbalance;
                                    let side_color = if level.ask_volume >= level.bid_volume {
                                        state.visual.stacked_ask_color
                                    } else {
                                        state.visual.stacked_bid_color
                                    };
                                    text_bg = imbalance_cell(side_color, stacked);
                                    out.push(Prim::Rect {
                                        rect: IRect {
                                            x: left,
                                            y: top,
                                            w: width,
                                            h: height,
                                        },
                                        color: text_bg,
                                    });
                                } else {
                                    out.push(Prim::Rect {
                                        rect: IRect {
                                            x: left,
                                            y: top,
                                            w: width,
                                            h: height,
                                        },
                                        color: text_bg,
                                    });
                                    // Total grows left-anchored like a volume bar; delta
                                    // diverges from the bar center the way order-flow
                                    // traders read signed pressure.
                                    let magnitude = if total_mode {
                                        level.total_volume
                                    } else {
                                        value.abs()
                                    };
                                    let center = left + width / 2;
                                    let (bar_x, bar_w) = if total_mode {
                                        (left, profile_width(width, magnitude, peak))
                                    } else if value >= 0.0 {
                                        (
                                            center,
                                            profile_width(width - (center - left), magnitude, peak),
                                        )
                                    } else {
                                        let bar_w = profile_width(center - left, magnitude, peak);
                                        (center - bar_w, bar_w)
                                    };
                                    if bar_w > 0 {
                                        let mut bar = profile_bar(base, magnitude, peak);
                                        if poc_wash {
                                            bar = poc_bar(bar, state.visual.poc_color);
                                        }
                                        out.push(Prim::Rect {
                                            rect: IRect {
                                                x: bar_x,
                                                y: top,
                                                w: bar_w,
                                                h: height,
                                            },
                                            color: bar,
                                        });
                                        if bar_w * 4 >= width {
                                            text_bg = bar;
                                        }
                                    } else if poc_wash {
                                        text_bg = poc_bar(text_bg, state.visual.poc_color);
                                    }
                                }
                                if is_poc {
                                    push_poc_stripe(
                                        out,
                                        left,
                                        top,
                                        height,
                                        hpr,
                                        state.visual.poc_color,
                                    );
                                }
                                if detailed
                                    && value != 0.0
                                    && height as f32 >= text_style.size * 1.05
                                {
                                    push_cell_text(
                                        out,
                                        left as f32 + width as f32 / 2.0,
                                        center_y,
                                        compact_volume(value),
                                        &text_style,
                                        contrast_on(text_bg, &text_style),
                                        is_poc || imbalanced,
                                    );
                                }
                            }
                        }
                    }
                    if detailed
                        && state.visual.show_bar_summary
                        && summary_fits_bar(spacing, font_size)
                    {
                        push_bar_summary(
                            out,
                            bar,
                            left,
                            width,
                            scale,
                            rs.base_value,
                            &text_style,
                            state.visual.positive_delta_color.solid(),
                            state.visual.negative_delta_color.solid(),
                            &aggregation,
                        );
                    }
                }
                FootprintLod::Summary => {
                    let (low_price, high_price) =
                        footprint_row_price_bounds(&aggregation, bar.low, bar.high);
                    let high = scale.price_to_coordinate(high_price, rs.base_value);
                    let low = scale.price_to_coordinate(low_price, rs.base_value);
                    let top = (high.min(low) * vpr).round() as i32;
                    let bottom = (high.max(low) * vpr).round() as i32;
                    // Solid delta body so zoomed-out bars read as candles, not wash.
                    let body = if bar.delta >= 0.0 {
                        state.visual.positive_delta_color.solid()
                    } else {
                        state.visual.negative_delta_color.solid()
                    };
                    out.push(Prim::Rect {
                        rect: IRect {
                            x: left,
                            y: top,
                            w: width,
                            h: (bottom - top).max(1),
                        },
                        color: body,
                    });
                    let poc_y = (scale.price_to_coordinate(bar.poc_price, rs.base_value) * vpr)
                        .round() as i32;
                    out.push(Prim::Rect {
                        rect: IRect {
                            x: left,
                            y: poc_y - 1,
                            w: width,
                            h: (vpr.round().max(1.0) as i32 + 1).max(2),
                        },
                        color: state.visual.poc_color,
                    });
                }
            }
        }
    }
}

/// Whether the two-line bar summary fits its own bar without overprinting the
/// neighbors: ~17 glyphs at 85% of `font_size` need `spacing >= 9 * font_size`
/// (`17 × 0.85 × 0.6 / 0.96`). Narrower bars keep numbers, cells, POC, and
/// imbalance — only the summary drops out.
fn summary_fits_bar(spacing: f64, font_size: f64) -> bool {
    spacing >= font_size * 9.0
}

fn bar_volume_maxima(bar: &FootprintBar) -> (f64, f64, f64, f64) {
    let mut max_bid: f64 = 0.0;
    let mut max_ask: f64 = 0.0;
    let mut max_total: f64 = 0.0;
    let mut max_abs_delta: f64 = 0.0;
    for level in &bar.levels {
        max_bid = max_bid.max(level.bid_volume);
        max_ask = max_ask.max(level.ask_volume);
        max_total = max_total.max(level.total_volume);
        max_abs_delta = max_abs_delta.max(level.delta.abs());
    }
    (max_bid, max_ask, max_total, max_abs_delta)
}

/// Faint full-cell wash so empty rows keep their shape without competing with
/// prints. The hue hints the side; the profile bar carries the magnitude.
fn track_cell(base: Color) -> Color {
    Color::rgba(base.r(), base.g(), base.b(), 26)
}

/// Horizontal extent of a profile bar: linear in volume so the silhouette reads
/// as an honest volume profile. Returns 0 when there is nothing to draw.
fn profile_width(span: i32, volume: f64, peak: f64) -> i32 {
    if span <= 0 || peak <= 0.0 || volume <= 0.0 {
        return 0;
    }
    let fraction = (volume / peak).clamp(0.0, 1.0);
    ((span as f64 * fraction).round() as i32).clamp(1, span)
}

/// The profile bar itself: same hue, strong alpha ramp so heavy prints glow.
/// Gamma < 1 keeps small prints legible next to the bar peak.
fn profile_bar(base: Color, volume: f64, peak: f64) -> Color {
    if peak <= 0.0 || volume <= 0.0 {
        return track_cell(base);
    }
    let t = (volume / peak).clamp(0.0, 1.0).powf(0.6);
    let alpha = (110.0 + 145.0 * t).round().clamp(0.0, 255.0) as u8;
    Color::rgba(base.r(), base.g(), base.b(), alpha)
}

/// Imbalance keeps its configured hue at full strength: single imbalances are
/// strong, stacked runs are fully opaque so they pop at Cells density too.
fn imbalance_cell(color: Color, stacked: bool) -> Color {
    if stacked {
        color.solid()
    } else {
        Color::rgba(color.r(), color.g(), color.b(), 215)
    }
}

/// POC wash: pull the profile bar two-thirds toward the POC hue (reference
/// yellow row) while keeping the bar's own alpha, so the row reads as POC and
/// the volume silhouette survives underneath.
fn poc_bar(bar: Color, poc: Color) -> Color {
    let mix = |c: u8, p: u8| ((u16::from(c) + 2 * u16::from(p)) / 3) as u8;
    Color::rgba(
        mix(bar.r(), poc.r()),
        mix(bar.g(), poc.g()),
        mix(bar.b(), poc.b()),
        bar.a().max(200),
    )
}

fn push_poc_stripe(out: &mut Vec<Prim>, left: i32, top: i32, height: i32, hpr: f64, poc: Color) {
    // One solid stripe on the row's leading edge: reads at any row height,
    // never boxes the numbers in.
    let stripe = (2.0 * hpr).round().max(2.0) as i32;
    out.push(Prim::Rect {
        rect: IRect {
            x: left,
            y: top,
            w: stripe,
            h: height.max(1),
        },
        color: poc.solid(),
    });
}

/// Glyph color for a footprint cell. An explicit host text color stays authoritative.
/// On light surfaces the canonical layout foreground stays fixed so translucent cell
/// fills cannot flip numbers to white; dark surfaces still resolve contrast against
/// the composited cell.
fn contrast_on(cell: Color, style: &FootprintTextStyle<'_>) -> Color {
    if style.explicit_text || style.surface.luminance() > 160.0 {
        return style.fallback_color;
    }
    composite_over(cell, style.surface).contrast_text()
}

fn composite_over(foreground: Color, background: Color) -> Color {
    let alpha = u32::from(foreground.a());
    let inverse = 255 - alpha;
    let blend = |f: u8, b: u8| ((u32::from(f) * alpha + u32::from(b) * inverse + 127) / 255) as u8;
    Color::rgb(
        blend(foreground.r(), background.r()),
        blend(foreground.g(), background.g()),
        blend(foreground.b(), background.b()),
    )
}

fn fallback_text_to_spine(text: Color) -> Color {
    Color::rgba(text.r(), text.g(), text.b(), 90)
}

fn push_cell_text(
    out: &mut Vec<Prim>,
    x: f32,
    y: f32,
    text: String,
    style: &FootprintTextStyle<'_>,
    color: Color,
    bold: bool,
) {
    out.push(Prim::Text {
        x,
        y,
        text,
        color,
        size: style.size,
        family: style.family.to_string(),
        align: TextAlign::Center,
        weight: if bold { 700 } else { 500 },
        italic: false,
    });
}

#[allow(clippy::too_many_arguments)]
fn push_bar_summary(
    out: &mut Vec<Prim>,
    bar: &FootprintBar,
    left: i32,
    width: i32,
    scale: &PriceScaleCore,
    base_value: f64,
    style: &FootprintTextStyle<'_>,
    positive: Color,
    negative: Color,
    aggregation: &FootprintAggregationOptions,
) {
    // Delta is the signal: color it by sign at full strength. Volume stays in the
    // theme foreground so the two lines scan as signal + context. The block sits
    // below the bar's outer cell edge so it never overprints bottom-row numbers.
    let summary_size = style.summary;
    let (bottom_price, _) = footprint_row_price_bounds(aggregation, bar.low, bar.high);
    let y = (scale.price_to_coordinate(bottom_price, base_value) * style.pixel_ratio) as f32
        + summary_size * 0.9;
    let delta_text_color = if bar.delta >= 0.0 { positive } else { negative };
    out.push(Prim::Text {
        x: left as f32 + width as f32 / 2.0,
        y,
        text: format!(
            "Δ {}  H {}  L {}",
            compact_volume(bar.delta),
            compact_volume(bar.max_delta),
            compact_volume(bar.min_delta)
        ),
        color: delta_text_color,
        size: summary_size,
        family: style.family.to_string(),
        align: TextAlign::Center,
        weight: 700,
        italic: false,
    });
    out.push(Prim::Text {
        x: left as f32 + width as f32 / 2.0,
        y: y + summary_size * 1.05,
        text: format!(
            "V {}  B {}  A {}",
            compact_volume(bar.total_volume),
            compact_volume(bar.bid_volume),
            compact_volume(bar.ask_volume)
        ),
        color: style.fallback_color,
        size: summary_size,
        family: style.family.to_string(),
        align: TextAlign::Center,
        weight: 500,
        italic: false,
    });
}

fn compact_volume(value: f64) -> String {
    let absolute = value.abs();
    if absolute >= 1_000_000.0 {
        format!("{:.1}M", value / 1_000_000.0)
    } else if absolute >= 1_000.0 {
        format!("{:.1}K", value / 1_000.0)
    } else if value.fract().abs() < 1e-9 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}
