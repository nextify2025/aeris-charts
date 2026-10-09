use std::borrow::Cow;

use super::*;
use crate::footprint::{
    FootprintAggregationOptions, FootprintBar, FootprintCellMode, FootprintLevel,
    footprint_row_merge, footprint_row_price_bounds, horizontal_imbalance_bar,
    merged_footprint_bar, merged_poc_price,
};

/// Bars at least this wide (CSS px) print cell numbers.
const DETAIL_MIN_SPACING: f64 = 48.0;
/// Bars at least this wide draw volume cells; narrower bars draw a range summary.
const CELLS_MIN_SPACING: f64 = 8.0;
/// Vertical room (CSS px) a printed row keeps around its glyphs.
const DETAIL_ROW_PADDING: f64 = 5.0;
/// Display rows shorter than this (CSS px) merge when only cells draw.
const CELLS_MIN_ROW: f64 = 3.0;
/// Average advance of a compact volume glyph relative to the font size.
const GLYPH_ADVANCE: f32 = 0.62;
/// Share of the bar slot the cluster occupies; the rest separates neighboring bars.
const CLUSTER_WIDTH: f64 = 0.9;

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
    summary: f32,
    family: &'a str,
    pixel_ratio: f64,
}

/// Device-px column of one cluster: the range line on the left edge, then the cells.
#[derive(Clone, Copy)]
struct ClusterColumn {
    left: i32,
    right: i32,
    range_width: i32,
    cells_left: i32,
}

impl ClusterColumn {
    fn new(x: f64, spacing: f64, hpr: f64) -> Self {
        let half = spacing * CLUSTER_WIDTH / 2.0;
        let left = ((x - half) * hpr).round() as i32;
        let right = (((x + half) * hpr).round() as i32).max(left + 1);
        let range_width = (2.0 * hpr).round().max(1.0) as i32;
        let gap = hpr.round().max(1.0) as i32;
        let cells_left = (left + range_width + gap).min(right - 1);
        Self {
            left,
            right,
            range_width,
            cells_left,
        }
    }

    const fn cells_width(self) -> i32 {
        self.right - self.cells_left
    }
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
        let visual = &state.visual;
        let plot = self.data.plot(rs.id);
        let spacing = self.time_scale.bar_spacing();
        let font_size = visual.font_size;
        let stored = stream.options();
        let rows = plot.visible_rows(from, to).collect::<Vec<_>>();
        // One merge factor per frame keeps rows aligned across every visible bar.
        let stored_row_px =
            rows.last()
                .and_then(|row| stream.bars().get(*row))
                .map_or(0.0, |bar| {
                    let lower = scale.price_to_coordinate(bar.close, rs.base_value);
                    let upper =
                        scale.price_to_coordinate(bar.close + stored.row_size(), rs.base_value);
                    (lower - upper).abs()
                });
        let wide = spacing >= DETAIL_MIN_SPACING;
        let merge = if visual.adaptive_rows {
            let minimum = if wide {
                font_size + DETAIL_ROW_PADDING
            } else {
                CELLS_MIN_ROW
            };
            footprint_row_merge(stored_row_px, minimum)
        } else {
            1
        };
        let aggregation = FootprintAggregationOptions {
            ticks_per_row: stored.ticks_per_row.saturating_mul(merge),
            ..stored
        };
        let row_px = stored_row_px * f64::from(merge);
        let lod = if wide && row_px >= font_size + 3.0 {
            FootprintLod::Detailed
        } else if spacing >= CELLS_MIN_SPACING && row_px * vpr >= 2.0 {
            FootprintLod::Cells
        } else {
            FootprintLod::Summary
        };

        let layout = &self.options.get().layout;
        let family = layout.font_family.clone();
        let fallback_text = visual
            .text_color
            .or_else(|| Color::parse_css(&layout.text_color))
            .unwrap_or(Color::rgb(215, 219, 228));
        let surface = Color::parse_css(&layout.background.color).unwrap_or(Color::rgb(
            aeris_charts_core::style::DEFAULT_SURFACE_RGB.0,
            aeris_charts_core::style::DEFAULT_SURFACE_RGB.1,
            aeris_charts_core::style::DEFAULT_SURFACE_RGB.2,
        ));
        let text_style = FootprintTextStyle {
            fallback_color: fallback_text,
            explicit_text: visual.text_color.is_some(),
            surface,
            size: (font_size * vpr) as f32,
            summary: (font_size * vpr) as f32 * 0.9,
            family: &family,
            pixel_ratio: vpr,
        };
        for row in rows {
            let Some(source) = stream.bars().get(row) else {
                continue;
            };
            let Some(logical) = plot.index_at(row) else {
                continue;
            };
            let x = self.time_scale.index_to_coordinate(logical);
            let column = ClusterColumn::new(x, spacing, hpr);
            if lod == FootprintLod::Summary {
                let poc_price = merged_poc_price(source, stored.ticks_per_row, stored.row_size());
                push_range_summary(
                    out,
                    source,
                    poc_price,
                    column,
                    scale,
                    rs.base_value,
                    vpr,
                    visual,
                    &stored,
                );
                continue;
            }
            // Stored levels are per tick; display rows group `ticks_per_row` of them.
            let mut bar = if aggregation.ticks_per_row > 1 {
                Cow::Owned(merged_footprint_bar(
                    source,
                    aggregation.ticks_per_row,
                    stored.imbalance,
                    aggregation.row_size(),
                ))
            } else {
                Cow::Borrowed(source)
            };
            if visual.cell_mode == FootprintCellMode::HorizontalImbalance {
                bar = Cow::Owned(horizontal_imbalance_bar(&bar, stored.imbalance));
            }
            let detailed = lod == FootprintLod::Detailed;
            push_range_line(
                out,
                &bar,
                column,
                scale,
                rs.base_value,
                vpr,
                visual,
                &aggregation,
            );
            let maxima = bar_volume_maxima(&bar);
            for level in &bar.levels {
                let (lower_price, upper_price) =
                    footprint_row_price_bounds(&aggregation, level.price, level.price);
                let upper = scale.price_to_coordinate(upper_price, rs.base_value);
                let lower = scale.price_to_coordinate(lower_price, rs.base_value);
                let top = (upper.min(lower) * vpr).round() as i32;
                let bottom = (upper.max(lower) * vpr).round() as i32;
                // A 1px gap separates stacked rows without a separator primitive per row.
                let full_height = (bottom - top).max(1);
                let height = if full_height > 2 {
                    full_height - 1
                } else {
                    full_height
                };
                let cell = CellRect { top, height };
                let is_poc = level.level == bar.poc_level;
                let text = detailed.then_some(&text_style);
                match visual.cell_mode {
                    FootprintCellMode::BidAsk | FootprintCellMode::HorizontalImbalance => {
                        push_bid_ask_row(out, level, column, cell, 0, maxima, visual, text);
                    }
                    FootprintCellMode::VolumeLadder => push_bid_ask_row(
                        out,
                        level,
                        column,
                        cell,
                        full_height / 2,
                        maxima,
                        visual,
                        text,
                    ),
                    FootprintCellMode::BidAskHistogram => {
                        push_bid_ask_histogram_row(out, level, column, cell, maxima, visual, text);
                    }
                    FootprintCellMode::ProfileInBar => push_profile_row(
                        out,
                        level,
                        column,
                        cell,
                        maxima,
                        visual,
                        text_style.fallback_color,
                        text,
                    ),
                    FootprintCellMode::Total | FootprintCellMode::Delta => {
                        push_single_value_row(out, level, column, cell, maxima, visual, text);
                    }
                }
                if is_poc {
                    push_poc_outline(out, column, cell, hpr, visual.poc_color);
                }
            }
            if detailed && visual.show_bar_summary {
                push_bar_summary(
                    out,
                    &bar,
                    column,
                    scale,
                    rs.base_value,
                    &text_style,
                    visual.positive_delta_color.solid(),
                    visual.negative_delta_color.solid(),
                    &aggregation,
                );
            }
        }
    }
}

#[derive(Clone, Copy)]
struct CellRect {
    top: i32,
    height: i32,
}

#[derive(Clone, Copy)]
struct VolumeMaxima {
    bid: f64,
    ask: f64,
    total: f64,
    abs_delta: f64,
}

fn bar_volume_maxima(bar: &FootprintBar) -> VolumeMaxima {
    let mut maxima = VolumeMaxima {
        bid: 0.0,
        ask: 0.0,
        total: 0.0,
        abs_delta: 0.0,
    };
    for level in &bar.levels {
        maxima.bid = maxima.bid.max(level.bid_volume);
        maxima.ask = maxima.ask.max(level.ask_volume);
        maxima.total = maxima.total.max(level.total_volume);
        maxima.abs_delta = maxima.abs_delta.max(level.delta.abs());
    }
    maxima
}

/// The bar's traded range as a thin line on the cluster's left edge, colored by direction.
#[allow(clippy::too_many_arguments)]
fn push_range_line(
    out: &mut Vec<Prim>,
    bar: &FootprintBar,
    column: ClusterColumn,
    scale: &PriceScaleCore,
    base_value: f64,
    vpr: f64,
    visual: &crate::FootprintVisualOptions,
    aggregation: &FootprintAggregationOptions,
) {
    let (low_price, high_price) = footprint_row_price_bounds(aggregation, bar.low, bar.high);
    let high = scale.price_to_coordinate(high_price, base_value);
    let low = scale.price_to_coordinate(low_price, base_value);
    let top = (high.min(low) * vpr).round() as i32;
    let bottom = (high.max(low) * vpr).round() as i32;
    let color = if bar.close >= bar.open {
        visual.ask_color.solid()
    } else {
        visual.bid_color.solid()
    };
    out.push(Prim::Rect {
        rect: IRect {
            x: column.left,
            y: top,
            w: column.range_width,
            h: (bottom - top).max(1),
        },
        color,
    });
}

/// Zoomed-out bar: the traded range in the bar-delta color with its POC marked.
#[allow(clippy::too_many_arguments)]
fn push_range_summary(
    out: &mut Vec<Prim>,
    bar: &FootprintBar,
    poc_price: f64,
    column: ClusterColumn,
    scale: &PriceScaleCore,
    base_value: f64,
    vpr: f64,
    visual: &crate::FootprintVisualOptions,
    aggregation: &FootprintAggregationOptions,
) {
    let (low_price, high_price) = footprint_row_price_bounds(aggregation, bar.low, bar.high);
    let high = scale.price_to_coordinate(high_price, base_value);
    let low = scale.price_to_coordinate(low_price, base_value);
    let top = (high.min(low) * vpr).round() as i32;
    let bottom = (high.max(low) * vpr).round() as i32;
    let body = if bar.delta >= 0.0 {
        visual.positive_delta_color.solid()
    } else {
        visual.negative_delta_color.solid()
    };
    let width = (column.right - column.left).max(1);
    out.push(Prim::Rect {
        rect: IRect {
            x: column.left,
            y: top,
            w: width,
            h: (bottom - top).max(1),
        },
        color: body,
    });
    let poc_y = (scale.price_to_coordinate(poc_price, base_value) * vpr).round() as i32;
    out.push(Prim::Rect {
        rect: IRect {
            x: column.left,
            y: poc_y - 1,
            w: width,
            h: (vpr.round().max(1.0) as i32 + 1).max(2),
        },
        color: visual.poc_color.solid(),
    });
}

/// One `bid x ask` row: each half carries a faint side track, a volume bar growing from the
/// center divider, and its number. Imbalanced halves switch to the imbalance color. The bid half
/// is drawn `bid_raise` device px above the row (the staggered volume ladder); zero keeps it level.
#[allow(clippy::too_many_arguments)]
fn push_bid_ask_row(
    out: &mut Vec<Prim>,
    level: &FootprintLevel,
    column: ClusterColumn,
    cell: CellRect,
    bid_raise: i32,
    maxima: VolumeMaxima,
    visual: &crate::FootprintVisualOptions,
    text: Option<&FootprintTextStyle<'_>>,
) {
    let width = column.cells_width().max(2);
    let split = column.cells_left + width / 2;
    let bid_left = column.cells_left;
    let bid_width = (split - bid_left).max(1);
    let ask_left = (split + i32::from(width > 3)).min(column.right);
    let ask_width = (column.right - ask_left).max(1);
    let bid_imbalanced = level.bid_imbalance || level.stacked_bid_imbalance;
    let ask_imbalanced = level.ask_imbalance || level.stacked_ask_imbalance;
    let bid_cell = CellRect {
        top: cell.top - bid_raise,
        height: cell.height,
    };

    let bid_background = push_half(
        out,
        HalfCell {
            left: bid_left,
            width: bid_width,
            grows_left: true,
        },
        bid_cell,
        level.bid_volume,
        maxima.bid,
        visual.bid_color,
        bid_imbalanced
            .then(|| imbalance_cell(visual.stacked_bid_color, level.stacked_bid_imbalance)),
    );
    let ask_background = push_half(
        out,
        HalfCell {
            left: ask_left,
            width: ask_width,
            grows_left: false,
        },
        cell,
        level.ask_volume,
        maxima.ask,
        visual.ask_color,
        ask_imbalanced
            .then(|| imbalance_cell(visual.stacked_ask_color, level.stacked_ask_imbalance)),
    );
    let Some(style) = text else {
        return;
    };
    let center_y = cell.top as f32 + cell.height as f32 / 2.0;
    if level.bid_volume > 0.0 {
        push_cell_text(
            out,
            bid_left as f32 + bid_width as f32 / 2.0,
            center_y - bid_raise as f32,
            bid_width,
            cell.height,
            compact_volume(level.bid_volume),
            style,
            contrast_on(bid_background, style),
            bid_imbalanced,
        );
    }
    if level.ask_volume > 0.0 {
        push_cell_text(
            out,
            ask_left as f32 + ask_width as f32 / 2.0,
            center_y,
            ask_width,
            cell.height,
            compact_volume(level.ask_volume),
            style,
            contrast_on(ask_background, style),
            ask_imbalanced,
        );
    }
}

#[derive(Clone, Copy)]
struct HalfCell {
    left: i32,
    width: i32,
    /// The volume bar grows from the right edge (bid half) instead of the left edge.
    grows_left: bool,
}

/// Draws one half cell and returns the color its number sits on.
#[allow(clippy::too_many_arguments)]
fn push_half(
    out: &mut Vec<Prim>,
    half: HalfCell,
    cell: CellRect,
    volume: f64,
    peak: f64,
    side_color: Color,
    imbalance: Option<Color>,
) -> Color {
    if let Some(color) = imbalance {
        out.push(Prim::Rect {
            rect: IRect {
                x: half.left,
                y: cell.top,
                w: half.width,
                h: cell.height,
            },
            color,
        });
        return color;
    }
    let track = track_cell(side_color);
    out.push(Prim::Rect {
        rect: IRect {
            x: half.left,
            y: cell.top,
            w: half.width,
            h: cell.height,
        },
        color: track,
    });
    let bar_width = profile_width(half.width, volume, peak);
    if bar_width == 0 {
        return track;
    }
    let bar = profile_bar(side_color, volume, peak);
    out.push(Prim::Rect {
        rect: IRect {
            x: if half.grows_left {
                half.left + half.width - bar_width
            } else {
                half.left
            },
            y: cell.top,
            w: bar_width,
            h: cell.height,
        },
        color: bar,
    });
    if bar_width * 4 >= half.width {
        bar
    } else {
        track
    }
}

/// One total or delta row spanning the full cell width.
fn push_single_value_row(
    out: &mut Vec<Prim>,
    level: &FootprintLevel,
    column: ClusterColumn,
    cell: CellRect,
    maxima: VolumeMaxima,
    visual: &crate::FootprintVisualOptions,
    text: Option<&FootprintTextStyle<'_>>,
) {
    let left = column.cells_left;
    let width = column.cells_width().max(1);
    let total_mode = !matches!(visual.cell_mode, FootprintCellMode::Delta);
    let (value, base, peak) = if total_mode {
        let base = if level.ask_volume >= level.bid_volume {
            visual.ask_color
        } else {
            visual.bid_color
        };
        (level.total_volume, base, maxima.total)
    } else {
        let base = if level.delta >= 0.0 {
            visual.positive_delta_color
        } else {
            visual.negative_delta_color
        };
        (level.delta, base, maxima.abs_delta)
    };
    let imbalanced = level.bid_imbalance
        || level.ask_imbalance
        || level.stacked_bid_imbalance
        || level.stacked_ask_imbalance;
    let mut background = track_cell(base);
    if imbalanced {
        let stacked = level.stacked_bid_imbalance || level.stacked_ask_imbalance;
        let side_color = if level.ask_volume >= level.bid_volume {
            visual.stacked_ask_color
        } else {
            visual.stacked_bid_color
        };
        background = imbalance_cell(side_color, stacked);
        out.push(Prim::Rect {
            rect: IRect {
                x: left,
                y: cell.top,
                w: width,
                h: cell.height,
            },
            color: background,
        });
    } else {
        out.push(Prim::Rect {
            rect: IRect {
                x: left,
                y: cell.top,
                w: width,
                h: cell.height,
            },
            color: background,
        });
        // Total grows from the left like a volume profile; delta diverges from the center.
        let magnitude = value.abs();
        let center = left + width / 2;
        let (bar_x, bar_width) = if total_mode {
            (left, profile_width(width, magnitude, peak))
        } else if value >= 0.0 {
            (
                center,
                profile_width(width - (center - left), magnitude, peak),
            )
        } else {
            let bar_width = profile_width(center - left, magnitude, peak);
            (center - bar_width, bar_width)
        };
        if bar_width > 0 {
            let bar = profile_bar(base, magnitude, peak);
            out.push(Prim::Rect {
                rect: IRect {
                    x: bar_x,
                    y: cell.top,
                    w: bar_width,
                    h: cell.height,
                },
                color: bar,
            });
            if bar_width * 4 >= width {
                background = bar;
            }
        }
    }
    if let Some(style) = text
        && value != 0.0
    {
        push_cell_text(
            out,
            left as f32 + width as f32 / 2.0,
            cell.top as f32 + cell.height as f32 / 2.0,
            width,
            cell.height,
            compact_volume(value),
            style,
            contrast_on(background, style),
            imbalanced,
        );
    }
}

/// Profile-in-bar row: one total-volume bar from the cluster's left edge, scaled to the bar's
/// peak row and split into bid, ask, and unclassified segments in that order. No track wash is
/// drawn, so the empty space reads as the profile's shape. Imbalanced rows print in bold.
#[allow(clippy::too_many_arguments)]
fn push_profile_row(
    out: &mut Vec<Prim>,
    level: &FootprintLevel,
    column: ClusterColumn,
    cell: CellRect,
    maxima: VolumeMaxima,
    visual: &crate::FootprintVisualOptions,
    unknown_color: Color,
    text: Option<&FootprintTextStyle<'_>>,
) {
    let left = column.cells_left;
    let width = column.cells_width().max(1);
    let total = level.total_volume;
    let bar_width = profile_width(width, total, maxima.total);
    let mut background = Color::rgba(0, 0, 0, 0);
    if bar_width > 0 {
        let segment = |volume: f64| -> i32 {
            ((f64::from(bar_width) * (volume / total).clamp(0.0, 1.0)).round() as i32)
                .clamp(0, bar_width)
        };
        let bid_width = segment(level.bid_volume);
        let ask_width = segment(level.ask_volume).min(bar_width - bid_width);
        let unknown_width = bar_width - bid_width - ask_width;
        let mut x = left;
        let mut widest = (0, background);
        for (segment_width, base) in [
            (bid_width, visual.bid_color),
            (ask_width, visual.ask_color),
            (unknown_width, unknown_color),
        ] {
            if segment_width == 0 {
                continue;
            }
            let color = profile_bar(base, total, maxima.total);
            out.push(Prim::Rect {
                rect: IRect {
                    x,
                    y: cell.top,
                    w: segment_width,
                    h: cell.height,
                },
                color,
            });
            if segment_width > widest.0 {
                widest = (segment_width, color);
            }
            x += segment_width;
        }
        if bar_width * 4 >= width {
            background = widest.1;
        }
    }
    let imbalanced = level.bid_imbalance
        || level.ask_imbalance
        || level.stacked_bid_imbalance
        || level.stacked_ask_imbalance;
    if let Some(style) = text
        && total != 0.0
    {
        push_cell_text(
            out,
            left as f32 + width as f32 / 2.0,
            cell.top as f32 + cell.height as f32 / 2.0,
            width,
            cell.height,
            compact_volume(total),
            style,
            contrast_on(background, style),
            imbalanced,
        );
    }
}

/// Bid/ask histogram row: the ask bar fills the upper half of the row and the bid bar the lower
/// half, both from the cluster's left edge on one scale (the larger side's peak) so their lengths
/// compare directly. Imbalanced sides use the imbalance color; numbers print as `bid x ask`.
fn push_bid_ask_histogram_row(
    out: &mut Vec<Prim>,
    level: &FootprintLevel,
    column: ClusterColumn,
    cell: CellRect,
    maxima: VolumeMaxima,
    visual: &crate::FootprintVisualOptions,
    text: Option<&FootprintTextStyle<'_>>,
) {
    let left = column.cells_left;
    let width = column.cells_width().max(1);
    let peak = maxima.bid.max(maxima.ask);
    let ask_height = (cell.height / 2).max(1);
    let ask_cell = CellRect {
        top: cell.top,
        height: ask_height,
    };
    let bid_cell = CellRect {
        top: cell.top + ask_height,
        height: (cell.height - ask_height).max(1),
    };
    let sides = [
        (
            ask_cell,
            level.ask_volume,
            visual.ask_color,
            (level.ask_imbalance || level.stacked_ask_imbalance)
                .then(|| imbalance_cell(visual.stacked_ask_color, level.stacked_ask_imbalance)),
        ),
        (
            bid_cell,
            level.bid_volume,
            visual.bid_color,
            (level.bid_imbalance || level.stacked_bid_imbalance)
                .then(|| imbalance_cell(visual.stacked_bid_color, level.stacked_bid_imbalance)),
        ),
    ];
    for (side_cell, volume, base, imbalance) in sides {
        let bar_width = profile_width(width, volume, peak);
        if bar_width == 0 {
            continue;
        }
        out.push(Prim::Rect {
            rect: IRect {
                x: left,
                y: side_cell.top,
                w: bar_width,
                h: side_cell.height,
            },
            color: imbalance.unwrap_or_else(|| profile_bar(base, volume, peak)),
        });
    }
    let Some(style) = text else {
        return;
    };
    if level.bid_volume == 0.0 && level.ask_volume == 0.0 {
        return;
    }
    let imbalanced = level.bid_imbalance
        || level.ask_imbalance
        || level.stacked_bid_imbalance
        || level.stacked_ask_imbalance;
    push_cell_text(
        out,
        left as f32 + width as f32 / 2.0,
        cell.top as f32 + cell.height as f32 / 2.0,
        width,
        cell.height,
        format!(
            "{} x {}",
            compact_volume(level.bid_volume),
            compact_volume(level.ask_volume)
        ),
        style,
        contrast_on(Color::rgba(0, 0, 0, 0), style),
        imbalanced,
    );
}

/// Faint full-cell wash so empty rows keep their shape without competing with prints.
fn track_cell(base: Color) -> Color {
    Color::rgba(base.r(), base.g(), base.b(), 26)
}

/// Horizontal extent of a profile bar, linear in volume. Returns 0 when nothing draws.
fn profile_width(span: i32, volume: f64, peak: f64) -> i32 {
    if span <= 0 || peak <= 0.0 || volume <= 0.0 {
        return 0;
    }
    let fraction = (volume / peak).clamp(0.0, 1.0);
    ((f64::from(span) * fraction).round() as i32).clamp(1, span)
}

/// Profile bar color: the side hue with an alpha ramp so heavy prints stand out. Gamma below 1
/// keeps small prints visible next to the bar peak.
fn profile_bar(base: Color, volume: f64, peak: f64) -> Color {
    if peak <= 0.0 || volume <= 0.0 {
        return track_cell(base);
    }
    let t = (volume / peak).clamp(0.0, 1.0).powf(0.6);
    let alpha = (110.0 + 145.0 * t).round().clamp(0.0, 255.0) as u8;
    Color::rgba(base.r(), base.g(), base.b(), alpha)
}

/// Single imbalances are strong; stacked runs are opaque so they read at every density.
fn imbalance_cell(color: Color, stacked: bool) -> Color {
    if stacked {
        color.solid()
    } else {
        Color::rgba(color.r(), color.g(), color.b(), 215)
    }
}

/// Outlines the point-of-control row across the cells without covering its numbers.
fn push_poc_outline(
    out: &mut Vec<Prim>,
    column: ClusterColumn,
    cell: CellRect,
    hpr: f64,
    poc: Color,
) {
    let border = hpr.round().max(1.0) as f32;
    out.push(Prim::RoundRect {
        x: column.cells_left as f32,
        y: cell.top as f32,
        w: column.cells_width().max(1) as f32,
        h: cell.height.max(1) as f32,
        radii: [0.0; 4],
        fill: Color::rgba(0, 0, 0, 0),
        border_width: border,
        border_color: poc.solid(),
    });
}

/// Glyph color for a cell. An explicit host text color stays authoritative. On light surfaces
/// the layout foreground stays fixed so translucent fills cannot flip numbers to white; dark
/// surfaces resolve contrast against the composited cell.
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

/// Prints a cell number at the configured size when it fits its cell; a number that would
/// overflow is left out rather than shrunk or clipped.
#[allow(clippy::too_many_arguments)]
fn push_cell_text(
    out: &mut Vec<Prim>,
    x: f32,
    y: f32,
    available_width: i32,
    available_height: i32,
    text: String,
    style: &FootprintTextStyle<'_>,
    color: Color,
    bold: bool,
) {
    let text_width = text.chars().count() as f32 * GLYPH_ADVANCE * style.size;
    if text_width + 2.0 > available_width as f32 || (available_height as f32) < style.size * 1.05 {
        return;
    }
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

/// Bar statistics under the cluster: delta in its sign color, then total volume.
#[allow(clippy::too_many_arguments)]
fn push_bar_summary(
    out: &mut Vec<Prim>,
    bar: &FootprintBar,
    column: ClusterColumn,
    scale: &PriceScaleCore,
    base_value: f64,
    style: &FootprintTextStyle<'_>,
    positive: Color,
    negative: Color,
    aggregation: &FootprintAggregationOptions,
) {
    let size = style.summary;
    let width = (column.right - column.left) as f32;
    let delta = format!("Δ {}", compact_volume(bar.delta));
    let volume = format!("V {}", compact_volume(bar.total_volume));
    let widest = delta.chars().count().max(volume.chars().count()) as f32;
    if widest * GLYPH_ADVANCE * size + 2.0 > width {
        return;
    }
    let (bottom_price, _) = footprint_row_price_bounds(aggregation, bar.low, bar.high);
    let y = (scale.price_to_coordinate(bottom_price, base_value) * style.pixel_ratio) as f32
        + size * 0.95;
    let x = column.left as f32 + width / 2.0;
    out.push(Prim::Text {
        x,
        y,
        text: delta,
        color: if bar.delta >= 0.0 { positive } else { negative },
        size,
        family: style.family.to_string(),
        align: TextAlign::Center,
        weight: 700,
        italic: false,
    });
    out.push(Prim::Text {
        x,
        y: y + size * 1.15,
        text: volume,
        color: style.fallback_color,
        size,
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
    } else if absolute >= 10.0 {
        format!("{value:.1}")
    } else if absolute >= 0.01 {
        format!("{value:.2}")
    } else {
        // Fractional crypto sizes: two significant digits, so a traded level never reads "0.00".
        let decimals = (1.0 - absolute.log10().floor()).clamp(2.0, 8.0) as usize;
        let text = format!("{value:.decimals$}");
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::compact_volume;

    #[test]
    fn compact_volume_never_rounds_a_traded_size_to_zero() {
        assert_eq!(compact_volume(0.003), "0.003");
        assert_eq!(compact_volume(0.00042), "0.00042");
        assert_eq!(compact_volume(-0.0051), "-0.0051");
        assert_eq!(compact_volume(0.16), "0.16");
        assert_eq!(compact_volume(4.01), "4.01");
        assert_eq!(compact_volume(26.4), "26.4");
        assert_eq!(compact_volume(70.0), "70");
        assert_eq!(compact_volume(1_500.0), "1.5K");
    }
}
