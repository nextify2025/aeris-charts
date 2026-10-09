//! Auction-marker rules over canonical footprint levels.

use aeris_charts_core::model::data_layer::SeriesId;
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{IRect, LineStyle, Prim, TextAlign};

use crate::footprint::{FootprintBar, FootprintBarAggregation, FootprintError};
use crate::frame::{pane_scale, series_scale_target};
use crate::{ChartEngine, NativePrimitiveId, SeriesKind};

pub const MAX_AUCTION_MARKERS: usize = 16;
const MICROS_PER_SECOND: i64 = 1_000_000;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuctionMarkerOptions {
    pub min_side_volume: f64,
    pub exhaustion_max_volume: f64,
    pub exhaustion_levels: usize,
    pub absorption_min_volume: f64,
    pub absorption_ratio: f64,
    pub extreme_levels: usize,
    pub min_rejection_rows: usize,
    pub extend_until_revisited: bool,
    pub include_forming_bar: bool,
    pub visible: bool,
}

impl Default for AuctionMarkerOptions {
    fn default() -> Self {
        Self {
            min_side_volume: 0.0,
            exhaustion_max_volume: 10.0,
            exhaustion_levels: 3,
            absorption_min_volume: 100.0,
            absorption_ratio: 3.0,
            extreme_levels: 2,
            min_rejection_rows: 1,
            extend_until_revisited: false,
            include_forming_bar: false,
            visible: true,
        }
    }
}

impl AuctionMarkerOptions {
    pub fn valid(&self) -> bool {
        [
            self.min_side_volume,
            self.exhaustion_max_volume,
            self.absorption_min_volume,
            self.absorption_ratio,
        ]
        .iter()
        .all(|value| value.is_finite() && *value >= 0.0)
            && (2..=8).contains(&self.exhaustion_levels)
            && self.extreme_levels > 0
            && self.extreme_levels <= 8
            && self.min_rejection_rows <= 8
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuctionMarkKind {
    UnfinishedAuction,
    Exhaustion,
    Absorption,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuctionSide {
    High,
    Low,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct AuctionMark {
    pub bar_time: i64,
    pub kind: AuctionMarkKind,
    pub side: AuctionSide,
    pub price: f64,
    pub volume: f64,
}

// ponytail: each detection allocates one small `Vec` (capacity 6) per bar, a tip one or two;
// reuse a scratch buffer only if the release perf_gate (Targets D and U) measures it.
fn detect(bar: &FootprintBar, options: &AuctionMarkerOptions) -> Vec<AuctionMark> {
    let levels = &bar.levels;
    let mut marks = Vec::with_capacity(6);
    if levels.is_empty() {
        return marks;
    }
    for side in [AuctionSide::Low, AuctionSide::High] {
        let edge = if side == AuctionSide::Low {
            &levels[0]
        } else {
            levels.last().unwrap()
        };
        let (volume, other) = if side == AuctionSide::Low {
            (edge.bid_volume, edge.ask_volume)
        } else {
            (edge.ask_volume, edge.bid_volume)
        };
        // A strictly positive side is required even at the default zero threshold.
        if volume > 0.0
            && other > 0.0
            && volume >= options.min_side_volume
            && other >= options.min_side_volume
        {
            marks.push(AuctionMark {
                bar_time: 0,
                kind: AuctionMarkKind::UnfinishedAuction,
                side,
                price: edge.price,
                volume: volume + other,
            });
        }
        if levels.len() >= options.exhaustion_levels
            && volume > 0.0
            && volume <= options.exhaustion_max_volume
            && (1..options.exhaustion_levels).all(|offset| {
                let index = if side == AuctionSide::Low {
                    offset
                } else {
                    levels.len() - 1 - offset
                };
                let previous = if side == AuctionSide::Low {
                    levels[index - 1].bid_volume
                } else {
                    levels[index + 1].ask_volume
                };
                let next = if side == AuctionSide::Low {
                    levels[index].bid_volume
                } else {
                    levels[index].ask_volume
                };
                next > previous
            })
        {
            marks.push(AuctionMark {
                bar_time: 0,
                kind: AuctionMarkKind::Exhaustion,
                side,
                price: edge.price,
                volume,
            });
        }
        let candidates = (0..options.extreme_levels.min(levels.len()))
            .filter_map(|distance| {
                let index = if side == AuctionSide::Low {
                    distance
                } else {
                    levels.len() - 1 - distance
                };
                let level = &levels[index];
                let (aggressor, opposite) = if side == AuctionSide::Low {
                    (level.bid_volume, level.ask_volume)
                } else {
                    (level.ask_volume, level.bid_volume)
                };
                let rejected = if side == AuctionSide::Low {
                    levels
                        .iter()
                        .filter(|row| row.price > level.price && row.price <= bar.close)
                        .count()
                        >= options.min_rejection_rows
                } else {
                    levels
                        .iter()
                        .filter(|row| row.price < level.price && row.price >= bar.close)
                        .count()
                        >= options.min_rejection_rows
                };
                (aggressor > 0.0
                    && aggressor >= options.absorption_min_volume
                    && aggressor >= options.absorption_ratio * opposite
                    && rejected)
                    .then_some((distance, level.price, aggressor))
            })
            .max_by(|a, b| a.2.total_cmp(&b.2).then_with(|| b.0.cmp(&a.0)));
        if let Some((_, price, volume)) = candidates {
            marks.push(AuctionMark {
                bar_time: 0,
                kind: AuctionMarkKind::Absorption,
                side,
                price,
                volume,
            });
        }
    }
    marks
}

#[derive(Clone, Debug)]
struct BarMarks {
    start_micros: i64,
    time: i64,
    marks: Vec<AuctionMark>,
}

#[derive(Clone, Debug)]
pub(crate) struct AuctionMarkerIndicator {
    id: NativePrimitiveId,
    pub(crate) series_id: SeriesId,
    options: AuctionMarkerOptions,
    bars: Vec<BarMarks>,
    /// Number of bars evaluated in the most recent refresh, for bounded-work tests.
    #[cfg(test)]
    refreshed_bars: usize,
    /// Bars evaluated over the indicator's lifetime, for bounded-work tests that span several
    /// refreshes of one mutation (a retention trim and the tip that caused it).
    #[cfg(test)]
    detected_bars: usize,
}

/// Mark time of the stream bar at `position`: its start in UTC seconds on a time axis, or its row
/// key on a non-time sequence axis (`key_base` is the stream's [`ChartEngine::sequence_key_base`]),
/// the same key the footprint rows, studies and big trades use. Retention never re-keys a
/// presentation row, so a mark keeps its key when older bars are evicted; without presentation
/// rows the keys are bar positions and [`AuctionMarkerIndicator::evict_front`] shifts them.
fn bar_time(bar: &FootprintBar, key_base: Option<i64>, position: usize) -> i64 {
    key_base.map_or_else(
        || bar.start_timestamp_micros.div_euclid(MICROS_PER_SECOND),
        |base| base + position as i64,
    )
}

impl AuctionMarkerIndicator {
    /// Drop the marks of the `count` oldest bars that retention evicted from the stream, keeping
    /// the rest aligned with the stream bars without re-detecting them. `key_base` is the stream's
    /// sequence key base after the trim. A presentation keeps its retained row keys, so the
    /// retained marks already match it; a sequence stream without presentation rows keys its bars
    /// by position, so the retained marks shift down with them, as big trades do.
    fn evict_front(&mut self, count: usize, key_base: Option<i64>) {
        self.bars.drain(..count.min(self.bars.len()));
        let Some(base) = key_base else {
            return;
        };
        if self.bars.first().is_none_or(|bar| bar.time == base) {
            return;
        }
        for (position, bar) in self.bars.iter_mut().enumerate() {
            bar.time = base + position as i64;
            for mark in &mut bar.marks {
                mark.bar_time = bar.time;
            }
        }
    }

    fn refresh(&mut self, bars: &[FootprintBar], key_base: Option<i64>, from: Option<usize>) {
        // A newly closed former tip needs evaluation. Historical corrections invalidate only
        // their suffix; retention drops the evicted bars' marks through `evict_front` before this
        // runs, so the retained marks stay aligned with `bars`.
        let start = from
            .map_or(0, |index| {
                if bars.len() > self.bars.len() {
                    index.min(self.bars.len().saturating_sub(1))
                } else {
                    index
                }
            })
            .min(self.bars.len())
            .min(bars.len());
        self.bars.truncate(start);
        for (index, bar) in bars.iter().enumerate().skip(start) {
            let mut marks = if index + 1 == bars.len() && !self.options.include_forming_bar {
                Vec::new()
            } else {
                detect(bar, &self.options)
            };
            let time = bar_time(bar, key_base, index);
            for mark in &mut marks {
                mark.bar_time = time;
            }
            self.bars.push(BarMarks {
                start_micros: bar.start_timestamp_micros,
                time,
                marks,
            });
        }
        #[cfg(test)]
        {
            self.refreshed_bars = bars.len() - start;
            self.detected_bars += self.refreshed_bars;
        }
    }
}

impl ChartEngine {
    pub fn add_auction_markers(
        &mut self,
        stream_id: u64,
        series_id: SeriesId,
        options: AuctionMarkerOptions,
    ) -> Result<NativePrimitiveId, FootprintError> {
        let stream = self
            .trade_stream(stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
        let series = self
            .series_entry(series_id)
            .ok_or(FootprintError::UnknownSeries(series_id))?;
        if !matches!(
            series.kind,
            SeriesKind::Candlestick
                | SeriesKind::Bar
                | SeriesKind::Line
                | SeriesKind::Area
                | SeriesKind::Baseline
                | SeriesKind::Footprint
        ) {
            return Err(FootprintError::UnsupportedBigTradesSeries(series_id));
        }
        if !options.valid() {
            return Err(FootprintError::InvalidAuctionMarkerOptions);
        }
        if self.auction_markers.values().map(Vec::len).sum::<usize>() >= MAX_AUCTION_MARKERS {
            return Err(FootprintError::AuctionMarkerCapacity);
        }
        let mut indicator = AuctionMarkerIndicator {
            id: self.next_native_primitive_id,
            series_id,
            options,
            bars: Vec::new(),
            #[cfg(test)]
            refreshed_bars: 0,
            #[cfg(test)]
            detected_bars: 0,
        };
        indicator.refresh(stream.bars(), self.sequence_key_base(stream_id), None);
        self.next_native_primitive_id = self
            .next_native_primitive_id
            .checked_add(1)
            .ok_or(FootprintError::AuctionMarkerCapacity)?;
        let id = indicator.id;
        self.auction_markers
            .entry(stream_id)
            .or_default()
            .push(indicator);
        self.invalidate_frame_series(series_id);
        Ok(id)
    }

    pub fn set_auction_marker_options(
        &mut self,
        id: NativePrimitiveId,
        options: AuctionMarkerOptions,
    ) -> Result<(), FootprintError> {
        if !options.valid() {
            return Err(FootprintError::InvalidAuctionMarkerOptions);
        }
        let key_base = self
            .auction_markers
            .iter()
            .find(|(_, entries)| entries.iter().any(|entry| entry.id == id))
            .and_then(|(stream_id, _)| self.sequence_key_base(*stream_id));
        let (stream_id, indicator) = self
            .auction_markers
            .iter_mut()
            .find_map(|(stream, entries)| {
                entries
                    .iter_mut()
                    .find(|entry| entry.id == id)
                    .map(|entry| (*stream, entry))
            })
            .ok_or(FootprintError::UnknownAuctionMarkers(id))?;
        if indicator.options != options {
            let stream = self
                .trade_streams
                .get(&stream_id)
                .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
            indicator.options = options;
            indicator.refresh(stream.bars(), key_base, None);
        }
        let series_id = indicator.series_id;
        self.invalidate_frame_series(series_id);
        Ok(())
    }

    pub fn auction_marker_options(&self, id: NativePrimitiveId) -> Option<&AuctionMarkerOptions> {
        self.auction_markers
            .values()
            .flatten()
            .find(|entry| entry.id == id)
            .map(|entry| &entry.options)
    }

    pub fn auction_markers_snapshot(
        &self,
        id: NativePrimitiveId,
    ) -> Result<Vec<AuctionMark>, FootprintError> {
        let indicator = self
            .auction_markers
            .values()
            .flatten()
            .find(|entry| entry.id == id)
            .ok_or(FootprintError::UnknownAuctionMarkers(id))?;
        Ok(indicator
            .bars
            .iter()
            .flat_map(|bar| bar.marks.iter().copied())
            .collect())
    }

    pub fn remove_auction_markers(&mut self, id: NativePrimitiveId) -> Result<(), FootprintError> {
        let (stream_id, series_id) = self
            .auction_markers
            .iter_mut()
            .find_map(|(stream, entries)| {
                let index = entries.iter().position(|entry| entry.id == id)?;
                Some((*stream, entries.remove(index).series_id))
            })
            .ok_or(FootprintError::UnknownAuctionMarkers(id))?;
        self.auction_markers
            .retain(|_, entries| !entries.is_empty());
        self.invalidate_frame_series(series_id);
        self.prune_trade_stream_if_unused(stream_id);
        Ok(())
    }

    pub(crate) fn auction_markers_count(&self, stream_id: u64) -> usize {
        self.auction_markers.get(&stream_id).map_or(0, Vec::len)
    }

    pub(crate) fn refresh_auction_markers(&mut self, stream_id: u64, from: Option<usize>) {
        let key_base = self.sequence_key_base(stream_id);
        let (Some(stream), Some(entries)) = (
            self.trade_streams.get(&stream_id),
            self.auction_markers.get_mut(&stream_id),
        ) else {
            return;
        };
        for entry in entries.iter_mut() {
            entry.refresh(stream.bars(), key_base, from);
        }
        self.invalidate_auction_markers(stream_id);
    }

    /// Retention evicted the `count` oldest bars of a stream: drop their marks in every marker set
    /// of the stream. The surviving marks follow their bars' row keys, so nothing is re-detected.
    pub(crate) fn evict_auction_markers_front(&mut self, stream_id: u64, count: usize) {
        let key_base = self.sequence_key_base(stream_id);
        let Some(entries) = self.auction_markers.get_mut(&stream_id) else {
            return;
        };
        for entry in entries.iter_mut() {
            entry.evict_front(count, key_base);
        }
        self.invalidate_auction_markers(stream_id);
    }

    fn invalidate_auction_markers(&mut self, stream_id: u64) {
        for index in 0..self.auction_markers_count(stream_id) {
            let series_id = self.auction_markers[&stream_id][index].series_id;
            self.invalidate_frame_series(series_id);
        }
    }

    /// Paint the visible marks of every marker set bound to `series_id` into the pane chrome.
    /// Marks have no hit target. ponytail: hover or selection of a mark, when a product needs
    /// it, belongs in the engine input controller, not in a host.
    pub(crate) fn build_auction_markers_frame(
        &self,
        series_id: SeriesId,
        from: i64,
        to: i64,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        let Some(series) = self.series_entry(series_id).filter(|series| series.visible) else {
            return;
        };
        let Some(pane) = self.panes.get(series.pane_index) else {
            return;
        };
        let scale = pane_scale(pane, series_scale_target(series));
        if scale.is_empty() {
            return;
        }
        let Some(base) = self.series_base_value(series_id, from) else {
            return;
        };
        let font = &self.options.get().layout;
        let color = Color::rgb(230, 170, 60);
        // Streams iterate in id order (a `BTreeMap`), so the primitive order is deterministic.
        for (stream_id, entries) in &self.auction_markers {
            let Some(stream) = self.trade_stream(*stream_id) else {
                continue;
            };
            let sequence = !matches!(stream.options().bars, FootprintBarAggregation::Time { .. });
            // Sequence-axis marks carry their bar's row key, which retention never re-keys; the
            // time scale indexes the chart's row keys (as big trades do).
            let to_index = |time: i64| {
                if sequence {
                    self.data
                        .merged_times()
                        .binary_search(&time)
                        .ok()
                        .map(|index| index as i64)
                } else {
                    self.time_to_index(time as f64, true)
                }
            };
            for entry in entries
                .iter()
                .filter(|entry| entry.series_id == series_id && entry.options.visible)
            {
                let first = entry
                    .bars
                    .partition_point(|bar| to_index(bar.time).is_none_or(|index| index < from));
                let last = entry
                    .bars
                    .partition_point(|bar| to_index(bar.time).is_some_and(|index| index <= to));
                for bar in &entry.bars[first..last.max(first)] {
                    for mark in &bar.marks {
                        let index = to_index(bar.time);
                        let Some(index) = index.filter(|index| (from..=to).contains(index)) else {
                            continue;
                        };
                        let x = (self.time_scale.index_to_coordinate(index) + 6.0) * hpr;
                        let y = scale.price_to_coordinate(mark.price, base) * vpr;
                        let side = if mark.side == AuctionSide::High {
                            -1.0
                        } else {
                            1.0
                        };
                        match mark.kind {
                            AuctionMarkKind::UnfinishedAuction => {
                                out.push(Prim::Triangle {
                                    a: [x as f32, (y + side * 5.0 * vpr) as f32],
                                    b: [(x - 4.0 * hpr) as f32, (y - side * 3.0 * vpr) as f32],
                                    c: [(x + 4.0 * hpr) as f32, (y - side * 3.0 * vpr) as f32],
                                    color,
                                });
                                // ponytail: a ray scans the visible bars after its mark on
                                // every frame, O(visible marks x visible bars). Target U's ray
                                // variant measured update + frame p99 0.18-0.32 ms and worst
                                // 0.24-0.40 ms over two runs (budgets 4 ms / 16.67 ms; M8 merge,
                                // Linux, 4 CPUs, release).
                                // Cache first-revisit indexes at refresh time only if a
                                // measurement shows the cost.
                                if entry.options.extend_until_revisited {
                                    let bars = stream.bars();
                                    let after = bars.partition_point(|candidate| {
                                        candidate.start_timestamp_micros <= bar.start_micros
                                    });
                                    // A ray beyond the viewport ends at the pane edge. Do not
                                    // scan invisible future history to construct this frame.
                                    // The marks stay aligned with the stream bars, so the
                                    // visible end of the marks is the visible end of the bars.
                                    let visible_end = last.min(bars.len());
                                    let end = (after..visible_end.max(after)).find(|&position| {
                                        bars[position].low <= mark.price
                                            && mark.price <= bars[position].high
                                    });
                                    let end_x = end
                                        .and_then(|position| to_index(entry.bars[position].time))
                                        .map(|index| {
                                            if index > to {
                                                self.pane_w * hpr
                                            } else {
                                                self.time_scale.index_to_coordinate(index) * hpr
                                            }
                                        })
                                        .unwrap_or(self.pane_w * hpr);
                                    out.push(Prim::HLine {
                                        y: y.round() as i32,
                                        x0: x.round() as i32,
                                        x1: end_x.round().max(x.round() + 1.0) as i32,
                                        width: hpr.round().max(1.0) as i32,
                                        style: LineStyle::Dashed,
                                        color,
                                    });
                                }
                            }
                            AuctionMarkKind::Exhaustion => out.push(Prim::Circle {
                                cx: x as f32,
                                cy: y as f32,
                                radius: (3.0 * hpr) as f32,
                                fill: color,
                                stroke_width: 0.0,
                                stroke: color,
                            }),
                            AuctionMarkKind::Absorption => {
                                out.push(Prim::RectFrame {
                                    rect: IRect {
                                        x: (x - 4.0 * hpr).round() as i32,
                                        y: (y - 4.0 * vpr).round() as i32,
                                        w: (8.0 * hpr).round().max(1.0) as i32,
                                        h: (8.0 * vpr).round().max(1.0) as i32,
                                    },
                                    border: hpr.round().max(1.0) as i32,
                                    color,
                                });
                                if self.time_scale.bar_spacing() >= 6.0 {
                                    out.push(Prim::Text {
                                        x: (x + 6.0 * hpr) as f32,
                                        y: y as f32,
                                        text: "ABS".into(),
                                        color,
                                        size: (font.font_size * vpr) as f32,
                                        family: font.font_family.clone(),
                                        align: TextAlign::Left,
                                        weight: 600,
                                        italic: false,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::footprint::{
        AggressorSide, FootprintImbalanceOptions, FootprintLevel, FootprintTrade,
        merged_footprint_bar,
    };
    use crate::{
        BigTradesFilter, BigTradesOptions, FootprintAggregationOptions, FootprintSeriesOptions,
        FootprintVisualOptions, OrderFlowPresentationOptions,
    };

    fn level(price: f64, bid: f64, ask: f64) -> FootprintLevel {
        FootprintLevel {
            level: price as i64,
            price,
            bid_volume: bid,
            ask_volume: ask,
            ..FootprintLevel::default()
        }
    }

    fn mark(kind: AuctionMarkKind, side: AuctionSide, price: f64, volume: f64) -> AuctionMark {
        AuctionMark {
            bar_time: 0,
            kind,
            side,
            price,
            volume,
        }
    }

    #[test]
    fn unfinished_auction_requires_both_sides_at_the_canonical_extreme() {
        let mut bar = FootprintBar {
            close: 101.0,
            levels: vec![level(100.0, 0.0, 5.0), level(101.0, 5.0, 0.0)],
            ..FootprintBar::default()
        };
        assert_eq!(detect(&bar, &AuctionMarkerOptions::default()), vec![]);
        bar.levels = vec![level(100.0, 2.0, 3.0), level(101.0, 3.0, 2.0)];
        assert_eq!(
            detect(&bar, &AuctionMarkerOptions::default()),
            vec![
                mark(
                    AuctionMarkKind::UnfinishedAuction,
                    AuctionSide::Low,
                    100.0,
                    5.0
                ),
                mark(
                    AuctionMarkKind::UnfinishedAuction,
                    AuctionSide::High,
                    101.0,
                    5.0
                ),
            ]
        );
        assert_eq!(
            detect(
                &bar,
                &AuctionMarkerOptions {
                    min_side_volume: 4.0,
                    ..AuctionMarkerOptions::default()
                }
            ),
            vec![]
        );
    }

    #[test]
    fn exhaustion_is_strict_on_both_sides_and_plateaus_do_not_qualify() {
        let mut high = FootprintBar {
            close: 103.0,
            levels: vec![
                level(100.0, 0.0, 0.0),
                level(101.0, 0.0, 8.0),
                level(102.0, 0.0, 4.0),
                level(103.0, 0.0, 2.0),
            ],
            ..FootprintBar::default()
        };
        assert_eq!(
            detect(&high, &AuctionMarkerOptions::default()),
            vec![mark(
                AuctionMarkKind::Exhaustion,
                AuctionSide::High,
                103.0,
                2.0
            )]
        );
        high.levels[2].ask_volume = 2.0;
        assert_eq!(detect(&high, &AuctionMarkerOptions::default()), vec![]);

        let low = FootprintBar {
            close: 103.0,
            levels: vec![
                level(100.0, 2.0, 0.0),
                level(101.0, 4.0, 0.0),
                level(102.0, 8.0, 0.0),
                level(103.0, 0.0, 0.0),
            ],
            ..FootprintBar::default()
        };
        assert_eq!(
            detect(&low, &AuctionMarkerOptions::default()),
            vec![mark(
                AuctionMarkKind::Exhaustion,
                AuctionSide::Low,
                100.0,
                2.0
            )]
        );
    }

    #[test]
    fn absorption_chooses_largest_then_nearest_and_requires_rejection_rows() {
        let low = FootprintBar {
            close: 103.0,
            levels: vec![
                level(100.0, 120.0, 20.0),
                level(101.0, 120.0, 20.0),
                level(102.0, 0.0, 0.0),
                level(103.0, 0.0, 0.0),
            ],
            ..FootprintBar::default()
        };
        assert_eq!(
            detect(&low, &AuctionMarkerOptions::default()),
            vec![
                mark(
                    AuctionMarkKind::UnfinishedAuction,
                    AuctionSide::Low,
                    100.0,
                    140.0
                ),
                mark(AuctionMarkKind::Absorption, AuctionSide::Low, 100.0, 120.0),
            ]
        );
        let mut larger_low = low.clone();
        larger_low.levels[1].bid_volume = 150.0;
        assert_eq!(
            detect(&larger_low, &AuctionMarkerOptions::default()),
            vec![
                mark(
                    AuctionMarkKind::UnfinishedAuction,
                    AuctionSide::Low,
                    100.0,
                    140.0
                ),
                mark(AuctionMarkKind::Absorption, AuctionSide::Low, 101.0, 150.0),
            ]
        );
        let high = FootprintBar {
            close: 100.0,
            levels: vec![
                level(100.0, 0.0, 0.0),
                level(101.0, 0.0, 0.0),
                level(102.0, 20.0, 120.0),
                level(103.0, 20.0, 120.0),
            ],
            ..FootprintBar::default()
        };
        assert_eq!(
            detect(&high, &AuctionMarkerOptions::default()),
            vec![
                mark(
                    AuctionMarkKind::UnfinishedAuction,
                    AuctionSide::High,
                    103.0,
                    140.0
                ),
                mark(AuctionMarkKind::Absorption, AuctionSide::High, 103.0, 120.0),
            ]
        );
        let mut larger_high = high.clone();
        larger_high.levels[2].ask_volume = 150.0;
        assert_eq!(
            detect(&larger_high, &AuctionMarkerOptions::default()),
            vec![
                mark(
                    AuctionMarkKind::UnfinishedAuction,
                    AuctionSide::High,
                    103.0,
                    140.0
                ),
                mark(AuctionMarkKind::Absorption, AuctionSide::High, 102.0, 150.0),
            ]
        );
        let mut not_rejected = high.clone();
        not_rejected.close = 102.0;
        assert_eq!(
            detect(
                &not_rejected,
                &AuctionMarkerOptions {
                    min_rejection_rows: 2,
                    ..AuctionMarkerOptions::default()
                }
            ),
            vec![mark(
                AuctionMarkKind::UnfinishedAuction,
                AuctionSide::High,
                103.0,
                140.0
            )]
        );
    }

    #[test]
    fn auction_detection_ignores_display_row_merging() {
        let (mut chart, stream) = setup();
        chart
            .set_trade_stream_trades(
                stream,
                vec![
                    trade(1_000_000, 100.0, 5.0, AggressorSide::Buy),
                    trade(1_000_001, 101.0, 5.0, AggressorSide::Sell),
                    trade(61_000_000, 102.0, 1.0, AggressorSide::Buy),
                ],
            )
            .unwrap();
        let canonical = &chart.trade_stream(stream).unwrap().bars()[0];
        assert_eq!(canonical.levels.len(), 2);
        assert_eq!(detect(canonical, &AuctionMarkerOptions::default()), vec![]);
        let merged = merged_footprint_bar(canonical, 2, FootprintImbalanceOptions::default(), 2.0);
        assert_eq!(
            detect(&merged, &AuctionMarkerOptions::default()),
            vec![
                mark(
                    AuctionMarkKind::UnfinishedAuction,
                    AuctionSide::Low,
                    100.0,
                    10.0
                ),
                mark(
                    AuctionMarkKind::UnfinishedAuction,
                    AuctionSide::High,
                    100.0,
                    10.0
                ),
            ]
        );
        let id = chart
            .add_auction_markers(stream, 0, AuctionMarkerOptions::default())
            .unwrap();
        assert_eq!(chart.auction_markers_snapshot(id).unwrap(), vec![]);
    }

    fn trade(time: i64, price: f64, volume: f64, side: AggressorSide) -> FootprintTrade {
        FootprintTrade {
            timestamp_micros: time,
            price,
            volume,
            aggressor: side,
            bid: None,
            ask: None,
            sequence: None,
            trade_id: None,
            conditions: 0,
            session_id: Some(1),
        }
    }

    fn setup() -> (ChartEngine, u64) {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let stream = chart
            .add_trade_stream(
                "auction",
                FootprintAggregationOptions {
                    tick_size: 1.0,
                    ..FootprintAggregationOptions::default()
                },
            )
            .unwrap();
        chart
            .configure_footprint_series(0, FootprintSeriesOptions::default())
            .unwrap();
        chart.bind_footprint_series_to_stream(0, stream).unwrap();
        (chart, stream)
    }

    fn presentation_options(rows: u32, footprint: bool) -> OrderFlowPresentationOptions {
        OrderFlowPresentationOptions {
            aggregation: FootprintAggregationOptions {
                tick_size: 1.0,
                ticks_per_row: rows,
                ..FootprintAggregationOptions::default()
            },
            visual: FootprintVisualOptions::default(),
            show_footprint: footprint,
            show_cumulative_delta: false,
            show_delta_histogram: false,
            big_trades: None,
        }
    }

    #[test]
    fn reconfiguring_order_flow_keeps_canonical_auction_marks_and_history() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let mut presentation = chart
            .add_order_flow_presentation("auction", 0, presentation_options(1, true))
            .unwrap();
        let stream = presentation.trade_stream();
        let tape = (0..12)
            .flat_map(|row| {
                let time = row * 60_000_000 + 1;
                [
                    trade(time, 100.0, 3.0, AggressorSide::Buy),
                    trade(time + 1, 100.0, 4.0, AggressorSide::Sell),
                ]
            })
            .collect::<Vec<_>>();
        chart
            .update_order_flow_presentation(presentation, tape.clone(), false)
            .unwrap();
        let id = chart
            .add_auction_markers(stream, 0, AuctionMarkerOptions::default())
            .unwrap();
        let initial = chart.auction_markers_snapshot(id).unwrap();
        assert_eq!(initial.len(), 22);
        chart
            .reconfigure_order_flow_presentation(&mut presentation, presentation_options(4, false))
            .unwrap();
        assert_eq!(chart.auction_markers_snapshot(id).unwrap(), initial);
        chart
            .reconfigure_order_flow_presentation(&mut presentation, presentation_options(2, true))
            .unwrap();
        assert_eq!(
            chart
                .trade_stream(stream)
                .unwrap()
                .trades()
                .cloned()
                .collect::<Vec<_>>(),
            tape
        );
        assert_eq!(chart.auction_markers_snapshot(id).unwrap(), initial);

        let mut fresh = ChartEngine::new(800.0, 500.0, 1.0);
        let fresh_presentation = fresh
            .add_order_flow_presentation("auction", 0, presentation_options(2, true))
            .unwrap();
        fresh
            .update_order_flow_presentation(fresh_presentation, tape, false)
            .unwrap();
        let fresh_id = fresh
            .add_auction_markers(
                fresh_presentation.trade_stream(),
                0,
                AuctionMarkerOptions::default(),
            )
            .unwrap();
        assert_eq!(
            chart.auction_markers_snapshot(id),
            fresh.auction_markers_snapshot(fresh_id)
        );
    }

    fn assert_window_auction_matches_fresh(
        bars: FootprintBarAggregation,
        mut tape: Vec<FootprintTrade>,
        replacement: Vec<FootprintTrade>,
        options: AuctionMarkerOptions,
        changed_bar_time: i64,
        expected_volume: f64,
    ) {
        let mut presentation_config = presentation_options(1, true);
        presentation_config.aggregation.bars = bars;
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let presentation = chart
            .add_order_flow_presentation("auction", 0, presentation_config.clone())
            .unwrap();
        chart
            .update_order_flow_presentation(presentation, tape.clone(), false)
            .unwrap();
        let stream = presentation.trade_stream();
        let id = chart
            .add_auction_markers(stream, 0, options.clone())
            .unwrap();
        let from = replacement
            .iter()
            .map(|print| print.timestamp_micros)
            .min()
            .unwrap();
        // The fork keys non-time marks by presentation row (`sequence_key_base` plus position).
        let key_base = chart.sequence_key_base(stream);
        assert_eq!(
            key_base.is_some(),
            !matches!(bars, FootprintBarAggregation::Time { .. })
        );
        let (_, changed_bar) = chart
            .trade_stream(stream)
            .unwrap()
            .bars()
            .iter()
            .enumerate()
            .find(|(position, bar)| bar_time(bar, key_base, *position) == changed_bar_time)
            .unwrap();
        assert!(changed_bar.start_timestamp_micros < from);
        assert!(changed_bar.end_timestamp_micros < from);

        chart
            .replace_order_flow_window(presentation, replacement.clone())
            .unwrap();
        tape.retain(|print| print.timestamp_micros < from);
        tape.extend(replacement);

        let mut fresh = ChartEngine::new(800.0, 500.0, 1.0);
        let fresh_presentation = fresh
            .add_order_flow_presentation("auction", 0, presentation_config)
            .unwrap();
        fresh
            .update_order_flow_presentation(fresh_presentation, tape, false)
            .unwrap();
        let fresh_id = fresh
            .add_auction_markers(fresh_presentation.trade_stream(), 0, options)
            .unwrap();
        let fresh_marks = fresh.auction_markers_snapshot(fresh_id).unwrap();
        assert!(fresh_marks.iter().any(|mark| {
            mark.bar_time == changed_bar_time
                && mark.kind == AuctionMarkKind::UnfinishedAuction
                && mark.volume == expected_volume
        }));
        assert_eq!(chart.auction_markers_snapshot(id).unwrap(), fresh_marks);
    }

    #[test]
    fn rewritten_window_inside_time_bar_after_last_print_repairs_auction_marks() {
        let tape = [0, 60, 120]
            .into_iter()
            .flat_map(|second| {
                [
                    trade(second * 1_000_000 + 1, 100.0, 3.0, AggressorSide::Buy),
                    trade(second * 1_000_000 + 2, 100.0, 4.0, AggressorSide::Sell),
                ]
            })
            .collect();
        assert_window_auction_matches_fresh(
            FootprintBarAggregation::Time {
                interval_micros: 60_000_000,
                anchor_micros: 0,
            },
            tape,
            vec![
                trade(60_000_003, 100.0, 5.0, AggressorSide::Buy),
                trade(120_000_001, 100.0, 3.0, AggressorSide::Buy),
                trade(120_000_002, 100.0, 4.0, AggressorSide::Sell),
            ],
            AuctionMarkerOptions::default(),
            60,
            12.0,
        );
    }

    #[test]
    fn rewritten_window_inside_non_time_bar_after_last_print_repairs_auction_marks() {
        assert_window_auction_matches_fresh(
            FootprintBarAggregation::Trades { trades_per_bar: 3 },
            vec![
                trade(1, 100.0, 3.0, AggressorSide::Buy),
                trade(2, 100.0, 4.0, AggressorSide::Sell),
                trade(3, 100.0, 2.0, AggressorSide::Buy),
                trade(11, 100.0, 3.0, AggressorSide::Buy),
                trade(12, 100.0, 4.0, AggressorSide::Sell),
            ],
            vec![trade(13, 100.0, 5.0, AggressorSide::Buy)],
            AuctionMarkerOptions {
                include_forming_bar: true,
                ..AuctionMarkerOptions::default()
            },
            1,
            12.0,
        );
    }

    #[test]
    fn rewritten_window_inside_forming_bar_after_last_print_repairs_auction_marks() {
        assert_window_auction_matches_fresh(
            FootprintBarAggregation::Time {
                interval_micros: 60_000_000,
                anchor_micros: 0,
            },
            vec![
                trade(1, 100.0, 3.0, AggressorSide::Buy),
                trade(2, 100.0, 4.0, AggressorSide::Sell),
                trade(60_000_001, 100.0, 3.0, AggressorSide::Buy),
                trade(60_000_002, 100.0, 4.0, AggressorSide::Sell),
            ],
            vec![trade(60_000_003, 100.0, 5.0, AggressorSide::Buy)],
            AuctionMarkerOptions {
                include_forming_bar: true,
                ..AuctionMarkerOptions::default()
            },
            60,
            12.0,
        );
    }

    fn duplicate_start_tape() -> Vec<FootprintTrade> {
        vec![
            trade(1, 100.0, 2.0, AggressorSide::Buy),
            trade(5, 100.0, 3.0, AggressorSide::Sell),
            trade(5, 100.0, 4.0, AggressorSide::Buy),
            trade(5, 100.0, 5.0, AggressorSide::Sell),
            trade(5, 100.0, 6.0, AggressorSide::Buy),
            trade(5, 100.0, 7.0, AggressorSide::Sell),
        ]
    }

    fn duplicate_start_options() -> AuctionMarkerOptions {
        AuctionMarkerOptions {
            include_forming_bar: true,
            ..AuctionMarkerOptions::default()
        }
    }

    #[test]
    fn rewritten_window_at_repeated_non_time_bar_starts_repairs_previous_bar() {
        let mut config = presentation_options(1, true);
        config.aggregation.bars = FootprintBarAggregation::Trades { trades_per_bar: 2 };
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let presentation = chart
            .add_order_flow_presentation("auction", 0, config.clone())
            .unwrap();
        chart
            .update_order_flow_presentation(presentation, duplicate_start_tape(), false)
            .unwrap();
        let stream = presentation.trade_stream();
        assert_eq!(
            chart
                .trade_stream(stream)
                .unwrap()
                .bars()
                .iter()
                .map(|bar| bar.start_timestamp_micros)
                .collect::<Vec<_>>(),
            vec![1, 5, 5]
        );
        let options = duplicate_start_options();
        let id = chart
            .add_auction_markers(stream, 0, options.clone())
            .unwrap();
        assert!(
            chart
                .auction_markers_snapshot(id)
                .unwrap()
                .iter()
                .any(|mark| mark.bar_time == 0 && mark.kind == AuctionMarkKind::UnfinishedAuction)
        );
        let replacement = (0..5)
            .map(|_| trade(5, 100.0, 2.0, AggressorSide::Buy))
            .collect::<Vec<_>>();
        chart
            .replace_order_flow_window(presentation, replacement.clone())
            .unwrap();

        let mut fresh = ChartEngine::new(800.0, 500.0, 1.0);
        let fresh_presentation = fresh
            .add_order_flow_presentation("auction", 0, config)
            .unwrap();
        let mut final_tape = vec![duplicate_start_tape()[0].clone()];
        final_tape.extend(replacement);
        fresh
            .update_order_flow_presentation(fresh_presentation, final_tape, false)
            .unwrap();
        let fresh_id = fresh
            .add_auction_markers(fresh_presentation.trade_stream(), 0, options)
            .unwrap();
        assert_eq!(
            chart.auction_markers_snapshot(id),
            fresh.auction_markers_snapshot(fresh_id)
        );
    }

    #[test]
    fn sequenced_late_print_at_repeated_non_time_bar_starts_repairs_previous_bar() {
        let options_bars = FootprintAggregationOptions {
            tick_size: 1.0,
            bars: FootprintBarAggregation::Trades { trades_per_bar: 2 },
            ..FootprintAggregationOptions::default()
        };
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let stream = chart.add_trade_stream("auction", options_bars).unwrap();
        chart
            .configure_footprint_series(0, FootprintSeriesOptions::default())
            .unwrap();
        chart.bind_footprint_series_to_stream(0, stream).unwrap();
        let mut tape = duplicate_start_tape();
        chart.set_trade_stream_trades(stream, tape.clone()).unwrap();
        let options = duplicate_start_options();
        let id = chart
            .add_auction_markers(stream, 0, options.clone())
            .unwrap();
        assert!(
            chart
                .auction_markers_snapshot(id)
                .unwrap()
                .iter()
                .any(|mark| mark.bar_time == 0 && mark.kind == AuctionMarkKind::UnfinishedAuction)
        );
        let mut late = trade(5, 101.0, 2.0, AggressorSide::Buy);
        late.sequence = Some(0);
        chart
            .update_trade_stream_trade(stream, late.clone())
            .unwrap();
        tape.push(late);

        let mut fresh = ChartEngine::new(800.0, 500.0, 1.0);
        let fresh_stream = fresh.add_trade_stream("auction", options_bars).unwrap();
        fresh
            .configure_footprint_series(0, FootprintSeriesOptions::default())
            .unwrap();
        fresh
            .bind_footprint_series_to_stream(0, fresh_stream)
            .unwrap();
        fresh.set_trade_stream_trades(fresh_stream, tape).unwrap();
        let fresh_id = fresh.add_auction_markers(fresh_stream, 0, options).unwrap();
        assert_eq!(
            chart.auction_markers_snapshot(id),
            fresh.auction_markers_snapshot(fresh_id)
        );
    }

    #[test]
    fn seeded_auction_repairs_match_fresh_build_across_bar_kinds() {
        // Small fixed-seed LCG: repeatable duplicate timestamps and varied canonical
        // sequence positions without a property-test dependency or a large tape.
        fn next(state: &mut u64) -> u64 {
            *state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            *state >> 32
        }
        fn random_trade(state: &mut u64, time: i64) -> FootprintTrade {
            let mut print = trade(
                time,
                100.0 + (next(state) % 3) as f64,
                2.0 + (next(state) % 4) as f64,
                if next(state).is_multiple_of(2) {
                    AggressorSide::Buy
                } else {
                    AggressorSide::Sell
                },
            );
            print.sequence = Some(next(state) % 3);
            print
        }
        for seed in [0x0BAD_5EED_u64, 0x00A0_C710_u64] {
            for bars in [
                FootprintBarAggregation::Time {
                    interval_micros: 60_000_000,
                    anchor_micros: 0,
                },
                FootprintBarAggregation::Trades { trades_per_bar: 2 },
                FootprintBarAggregation::Volume {
                    volume_per_bar: 6.0,
                },
                FootprintBarAggregation::Range { range_ticks: 1 },
            ] {
                for include_forming_bar in [false, true] {
                    let mut state = seed;
                    let mut config = presentation_options(1, true);
                    config.aggregation.bars = bars;
                    let options = AuctionMarkerOptions {
                        include_forming_bar,
                        exhaustion_levels: 2,
                        absorption_min_volume: 3.0,
                        ..AuctionMarkerOptions::default()
                    };
                    let mut tape = (0..20)
                        .map(|index| random_trade(&mut state, 1 + (index / 4) * 60_000_000))
                        .collect::<Vec<_>>();
                    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
                    let presentation = chart
                        .add_order_flow_presentation("auction", 0, config.clone())
                        .unwrap();
                    chart
                        .update_order_flow_presentation(presentation, tape.clone(), false)
                        .unwrap();
                    let id = chart
                        .add_auction_markers(presentation.trade_stream(), 0, options.clone())
                        .unwrap();
                    for step in 0..16 {
                        if step % 2 == 0 {
                            let from = 1 + (next(&mut state) % 7) as i64 * 60_000_000;
                            let replacement = (0..(3 + next(&mut state) % 7))
                                .map(|index| random_trade(&mut state, from + (index / 3) as i64))
                                .collect::<Vec<_>>();
                            chart
                                .replace_order_flow_window(presentation, replacement.clone())
                                .unwrap();
                            tape.retain(|print| print.timestamp_micros < from);
                            tape.extend(replacement);
                        } else {
                            let time = 1 + (next(&mut state) % 7) as i64 * 60_000_000;
                            let mut late = random_trade(&mut state, time);
                            late.sequence = Some(0);
                            chart
                                .update_trade_stream_trade(
                                    presentation.trade_stream(),
                                    late.clone(),
                                )
                                .unwrap();
                            tape.push(late);
                        }
                        let mut fresh = ChartEngine::new(800.0, 500.0, 1.0);
                        let fresh_presentation = fresh
                            .add_order_flow_presentation("auction", 0, config.clone())
                            .unwrap();
                        fresh
                            .update_order_flow_presentation(fresh_presentation, tape.clone(), false)
                            .unwrap();
                        let fresh_id = fresh
                            .add_auction_markers(
                                fresh_presentation.trade_stream(),
                                0,
                                options.clone(),
                            )
                            .unwrap();
                        assert_eq!(
                            chart.auction_markers_snapshot(id).unwrap(),
                            fresh.auction_markers_snapshot(fresh_id).unwrap(),
                            "seed={seed:x}, bars={bars:?}, include_forming_bar={include_forming_bar}, step={step}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn rewritten_window_repairs_only_auction_suffix_even_when_it_shrinks() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let presentation = chart
            .add_order_flow_presentation("auction", 0, presentation_options(1, true))
            .unwrap();
        let stream = presentation.trade_stream();
        let mut tape = (0..80)
            .flat_map(|row| {
                let time = row * 60_000_000 + 1;
                [
                    trade(time, 100.0, 3.0, AggressorSide::Buy),
                    trade(time + 1, 100.0, 4.0, AggressorSide::Sell),
                ]
            })
            .collect::<Vec<_>>();
        chart
            .update_order_flow_presentation(presentation, tape.clone(), false)
            .unwrap();
        let id = chart
            .add_auction_markers(stream, 0, AuctionMarkerOptions::default())
            .unwrap();
        let prefix = chart.auction_markers_snapshot(id).unwrap();
        let replacement = vec![
            trade(73 * 60_000_000 + 1, 100.0, 5.0, AggressorSide::Sell),
            trade(74 * 60_000_000 + 1, 101.0, 8.0, AggressorSide::Buy),
            trade(74 * 60_000_000 + 2, 101.0, 3.0, AggressorSide::Sell),
            trade(75 * 60_000_000 + 1, 102.0, 2.0, AggressorSide::Buy),
        ];
        chart
            .replace_order_flow_window(presentation, replacement.clone())
            .unwrap();
        tape.retain(|print| print.timestamp_micros < replacement[0].timestamp_micros);
        tape.extend(replacement);
        assert!(
            chart.auction_markers[&stream][0].refreshed_bars <= 7,
            "repair must not visit the preserved 73-bar prefix"
        );
        assert_eq!(
            &chart.auction_markers_snapshot(id).unwrap()[..146],
            &prefix[..146]
        );
        let mut fresh = ChartEngine::new(800.0, 500.0, 1.0);
        let fresh_presentation = fresh
            .add_order_flow_presentation("auction", 0, presentation_options(1, true))
            .unwrap();
        fresh
            .update_order_flow_presentation(fresh_presentation, tape, false)
            .unwrap();
        let fresh_id = fresh
            .add_auction_markers(
                fresh_presentation.trade_stream(),
                0,
                AuctionMarkerOptions::default(),
            )
            .unwrap();
        assert_eq!(
            chart.auction_markers_snapshot(id),
            fresh.auction_markers_snapshot(fresh_id)
        );
        assert_eq!(
            chart.footprint_bars(presentation.footprint_series().unwrap()),
            fresh.footprint_bars(fresh_presentation.footprint_series().unwrap())
        );
    }

    #[test]
    fn lifecycle_capacity_validation_and_runtime_only_export() {
        let (mut chart, stream) = setup();
        assert_eq!(
            chart.add_auction_markers(stream + 1, 0, AuctionMarkerOptions::default()),
            Err(FootprintError::UnknownTradeStream(stream + 1))
        );
        let invalid = AuctionMarkerOptions {
            exhaustion_levels: 1,
            ..AuctionMarkerOptions::default()
        };
        assert_eq!(
            chart.add_auction_markers(stream, 0, invalid.clone()),
            Err(FootprintError::InvalidAuctionMarkerOptions)
        );
        let ids = (0..MAX_AUCTION_MARKERS)
            .map(|_| {
                chart
                    .add_auction_markers(stream, 0, AuctionMarkerOptions::default())
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            chart.add_auction_markers(stream, 0, AuctionMarkerOptions::default()),
            Err(FootprintError::AuctionMarkerCapacity)
        );
        assert_eq!(
            chart.set_auction_marker_options(ids[0], invalid),
            Err(FootprintError::InvalidAuctionMarkerOptions)
        );
        assert_eq!(
            chart.auction_marker_options(ids[0]),
            Some(&AuctionMarkerOptions::default())
        );
        let export = chart.export_state_json().unwrap();
        assert!(!export.contains("auction_marker"));
        chart.remove_auction_markers(ids[0]).unwrap();
        assert_eq!(
            chart.auction_markers_snapshot(ids[0]),
            Err(FootprintError::UnknownAuctionMarkers(ids[0]))
        );
        chart
            .add_auction_markers(stream, 0, AuctionMarkerOptions::default())
            .unwrap();
    }

    #[test]
    fn invalid_options_leave_prior_options_and_removed_ids_report_errors() {
        let (mut chart, stream) = setup();
        let original = AuctionMarkerOptions {
            min_side_volume: 2.0,
            include_forming_bar: true,
            ..AuctionMarkerOptions::default()
        };
        let id = chart
            .add_auction_markers(stream, 0, original.clone())
            .unwrap();
        let invalid = [
            AuctionMarkerOptions {
                exhaustion_levels: 9,
                ..original.clone()
            },
            AuctionMarkerOptions {
                extreme_levels: 0,
                ..original.clone()
            },
            AuctionMarkerOptions {
                min_side_volume: -1.0,
                ..original.clone()
            },
            AuctionMarkerOptions {
                min_side_volume: f64::INFINITY,
                ..original.clone()
            },
            AuctionMarkerOptions {
                min_side_volume: f64::NAN,
                ..original.clone()
            },
            AuctionMarkerOptions {
                exhaustion_max_volume: -1.0,
                ..original.clone()
            },
            AuctionMarkerOptions {
                exhaustion_max_volume: f64::NAN,
                ..original.clone()
            },
            AuctionMarkerOptions {
                exhaustion_max_volume: f64::INFINITY,
                ..original.clone()
            },
            AuctionMarkerOptions {
                absorption_min_volume: -1.0,
                ..original.clone()
            },
            AuctionMarkerOptions {
                absorption_min_volume: f64::NEG_INFINITY,
                ..original.clone()
            },
            AuctionMarkerOptions {
                absorption_min_volume: f64::NAN,
                ..original.clone()
            },
            AuctionMarkerOptions {
                absorption_ratio: -1.0,
                ..original.clone()
            },
            AuctionMarkerOptions {
                absorption_ratio: f64::NAN,
                ..original.clone()
            },
            AuctionMarkerOptions {
                absorption_ratio: f64::INFINITY,
                ..original.clone()
            },
        ];
        for candidate in invalid {
            assert_eq!(
                chart.set_auction_marker_options(id, candidate.clone()),
                Err(FootprintError::InvalidAuctionMarkerOptions),
                "{candidate:?}"
            );
            assert_eq!(chart.auction_marker_options(id), Some(&original));
            assert_eq!(
                chart.add_auction_markers(stream, 0, candidate.clone()),
                Err(FootprintError::InvalidAuctionMarkerOptions),
                "{candidate:?}"
            );
            assert_eq!(chart.auction_marker_options(id), Some(&original));
        }
        chart.remove_auction_markers(id).unwrap();
        assert_eq!(chart.auction_marker_options(id), None);
        assert_eq!(
            chart.set_auction_marker_options(id, original),
            Err(FootprintError::UnknownAuctionMarkers(id))
        );
        assert_eq!(
            chart.auction_markers_snapshot(id),
            Err(FootprintError::UnknownAuctionMarkers(id))
        );
        assert_eq!(
            chart.remove_auction_markers(id),
            Err(FootprintError::UnknownAuctionMarkers(id))
        );
    }

    #[test]
    fn forming_late_print_replay_and_shared_dependents() {
        let (mut chart, stream) = setup();
        let tape = vec![
            trade(1_000_000, 100.0, 2.0, AggressorSide::Buy),
            trade(1_000_001, 100.0, 2.0, AggressorSide::Sell),
            trade(61_000_000, 101.0, 2.0, AggressorSide::Buy),
            trade(61_000_001, 101.0, 2.0, AggressorSide::Sell),
            trade(121_000_000, 102.0, 2.0, AggressorSide::Buy),
        ];
        chart.set_trade_stream_trades(stream, tape.clone()).unwrap();
        let markers = chart
            .add_auction_markers(stream, 0, AuctionMarkerOptions::default())
            .unwrap();
        let big = chart
            .add_big_trades(
                stream,
                0,
                BigTradesOptions {
                    filter: BigTradesFilter::Fixed {
                        minimum_volume: 1.0,
                    },
                    ..BigTradesOptions::default()
                },
            )
            .unwrap();
        assert_eq!(
            chart
                .auction_markers_snapshot(markers)
                .unwrap()
                .iter()
                .filter(|mark| mark.kind == AuctionMarkKind::UnfinishedAuction)
                .count(),
            4
        );
        assert!(!chart.big_trades_snapshot(big).unwrap().bubbles.is_empty());
        chart
            .update_trade_stream_trade(stream, trade(61_000_002, 101.0, 5.0, AggressorSide::Buy))
            .unwrap();
        let entry = &chart.auction_markers[&stream][0];
        assert!(
            entry.refreshed_bars <= 2,
            "late print refreshed {} bars",
            entry.refreshed_bars
        );
        let mut fresh = setup().0;
        let fresh_stream = fresh.trade_stream_id("auction").unwrap();
        let mut final_tape = tape;
        final_tape.push(trade(61_000_002, 101.0, 5.0, AggressorSide::Buy));
        fresh
            .set_trade_stream_trades(fresh_stream, final_tape.clone())
            .unwrap();
        let fresh_id = fresh
            .add_auction_markers(fresh_stream, 0, AuctionMarkerOptions::default())
            .unwrap();
        let fresh_big = fresh
            .add_big_trades(
                fresh_stream,
                0,
                BigTradesOptions {
                    filter: BigTradesFilter::Fixed {
                        minimum_volume: 1.0,
                    },
                    ..BigTradesOptions::default()
                },
            )
            .unwrap();
        assert_eq!(chart.footprint_bars(0), fresh.footprint_bars(0));
        assert_eq!(
            chart.big_trades_snapshot(big),
            fresh.big_trades_snapshot(fresh_big)
        );
        assert_eq!(
            chart.auction_markers_snapshot(markers),
            fresh.auction_markers_snapshot(fresh_id)
        );
        chart
            .set_trade_stream_replay_clock_micros(stream, Some(61_000_002))
            .unwrap();
        assert!(
            chart
                .auction_markers_snapshot(markers)
                .unwrap()
                .iter()
                .all(|mark| mark.bar_time <= 61)
        );
        let (mut prefix, prefix_stream) = setup();
        prefix
            .set_trade_stream_trades(
                prefix_stream,
                final_tape
                    .into_iter()
                    .filter(|trade| trade.timestamp_micros <= 61_000_002)
                    .collect(),
            )
            .unwrap();
        let prefix_id = prefix
            .add_auction_markers(prefix_stream, 0, AuctionMarkerOptions::default())
            .unwrap();
        assert_eq!(
            chart.auction_markers_snapshot(markers),
            prefix.auction_markers_snapshot(prefix_id)
        );
        chart
            .set_trade_stream_replay_clock_micros(stream, None)
            .unwrap();
        assert_eq!(
            chart.auction_markers_snapshot(markers),
            fresh.auction_markers_snapshot(fresh_id)
        );
    }

    #[test]
    fn frame_marks_use_shared_chrome_and_visible_bars_only() {
        let (mut chart, stream) = setup();
        chart
            .set_trade_stream_trades(
                stream,
                vec![
                    trade(1_000_000, 100.0, 120.0, AggressorSide::Sell),
                    trade(1_000_001, 100.0, 1.0, AggressorSide::Buy),
                    trade(1_000_002, 101.0, 20.0, AggressorSide::Buy),
                    trade(1_000_003, 102.0, 5.0, AggressorSide::Buy),
                    trade(1_000_004, 103.0, 2.0, AggressorSide::Buy),
                    trade(61_000_000, 101.0, 1.0, AggressorSide::Buy),
                    trade(121_000_000, 102.0, 1.0, AggressorSide::Buy),
                ],
            )
            .unwrap();
        let id = chart
            .add_auction_markers(
                stream,
                0,
                AuctionMarkerOptions {
                    extend_until_revisited: true,
                    ..AuctionMarkerOptions::default()
                },
            )
            .unwrap();
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        chart.build_frame();
        let mut prims = Vec::new();
        chart.build_auction_markers_frame(0, 0, 0, 1.0, 1.0, &mut prims);
        assert!(
            prims
                .iter()
                .any(|prim| matches!(prim, Prim::Triangle { .. }))
        );
        assert!(prims.iter().any(|prim| matches!(prim, Prim::Circle { .. })));
        assert!(
            prims
                .iter()
                .any(|prim| matches!(prim, Prim::RectFrame { .. }))
        );
        assert!(
            prims
                .iter()
                .any(|prim| matches!(prim, Prim::Text { text, .. } if text == "ABS"))
        );
        assert!(prims.iter().any(|prim| matches!(
            prim,
            Prim::HLine {
                style: LineStyle::Dashed,
                ..
            }
        )));
        prims.clear();
        chart.build_auction_markers_frame(0, 3, 3, 1.0, 1.0, &mut prims);
        assert!(prims.is_empty(), "off-screen bars must emit no chrome");
        chart
            .set_auction_marker_options(
                id,
                AuctionMarkerOptions {
                    visible: false,
                    ..AuctionMarkerOptions::default()
                },
            )
            .unwrap();
        prims.clear();
        chart.build_auction_markers_frame(0, 0, 0, 1.0, 1.0, &mut prims);
        assert!(prims.is_empty());
    }

    #[test]
    fn unfinished_rays_end_on_first_revisiting_bar_and_disabled_option_emits_none() {
        let (mut chart, stream) = setup();
        chart
            .set_trade_stream_trades(
                stream,
                vec![
                    trade(1_000_000, 100.0, 2.0, AggressorSide::Buy),
                    trade(1_000_001, 100.0, 3.0, AggressorSide::Sell),
                    trade(61_000_000, 101.0, 1.0, AggressorSide::Buy),
                    trade(121_000_000, 100.0, 1.0, AggressorSide::Buy),
                    trade(181_000_000, 100.0, 1.0, AggressorSide::Buy),
                ],
            )
            .unwrap();
        let id = chart
            .add_auction_markers(
                stream,
                0,
                AuctionMarkerOptions {
                    extend_until_revisited: true,
                    ..AuctionMarkerOptions::default()
                },
            )
            .unwrap();
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        chart.build_frame();
        let mut prims = Vec::new();
        chart.build_auction_markers_frame(0, 0, 3, 1.0, 1.0, &mut prims);
        let start_x = (chart.time_scale.index_to_coordinate(0) + 6.0).round() as i32;
        let revisit_x = chart.time_scale.index_to_coordinate(2).round() as i32;
        let rays = prims
            .iter()
            .filter_map(|prim| match prim {
                Prim::HLine {
                    x0,
                    x1,
                    style: LineStyle::Dashed,
                    ..
                } => Some((*x0, *x1)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(rays, vec![(start_x, revisit_x); 2]);
        assert_eq!(
            prims
                .iter()
                .filter(|prim| matches!(prim, Prim::Triangle { .. }))
                .count(),
            2
        );
        chart
            .set_auction_marker_options(id, AuctionMarkerOptions::default())
            .unwrap();
        prims.clear();
        chart.build_auction_markers_frame(0, 0, 3, 1.0, 1.0, &mut prims);
        assert_eq!(
            prims
                .iter()
                .filter(|prim| matches!(prim, Prim::Triangle { .. }))
                .count(),
            2
        );
        assert_eq!(
            prims
                .iter()
                .filter(|prim| matches!(prim, Prim::HLine { .. }))
                .count(),
            0
        );
    }

    #[test]
    fn absorption_frame_suppresses_text_below_six_pixel_bar_spacing() {
        let (mut chart, stream) = setup();
        chart
            .set_trade_stream_trades(
                stream,
                vec![
                    trade(1_000_000, 100.0, 120.0, AggressorSide::Sell),
                    trade(1_000_001, 100.0, 20.0, AggressorSide::Buy),
                    trade(1_000_002, 101.0, 1.0, AggressorSide::Buy),
                    trade(61_000_000, 102.0, 1.0, AggressorSide::Buy),
                ],
            )
            .unwrap();
        let id = chart
            .add_auction_markers(stream, 0, AuctionMarkerOptions::default())
            .unwrap();
        assert_eq!(
            chart.auction_markers_snapshot(id).unwrap(),
            vec![
                mark(
                    AuctionMarkKind::UnfinishedAuction,
                    AuctionSide::Low,
                    100.0,
                    140.0
                ),
                mark(AuctionMarkKind::Absorption, AuctionSide::Low, 100.0, 120.0),
            ]
        );
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        for (spacing, expected_text) in [(5.0, 0), (6.0, 1)] {
            chart.set_bar_spacing(spacing);
            chart.build_frame();
            let mut prims = Vec::new();
            chart.build_auction_markers_frame(0, 0, 0, 1.0, 1.0, &mut prims);
            assert_eq!(
                prims
                    .iter()
                    .filter(|prim| matches!(prim, Prim::RectFrame { .. }))
                    .count(),
                1
            );
            assert_eq!(
                prims
                    .iter()
                    .filter(|prim| matches!(prim, Prim::Text { text, .. } if text == "ABS"))
                    .count(),
                expected_text,
                "bar spacing {spacing}"
            );
        }
    }

    #[test]
    fn forming_option_and_front_retention_keep_mark_times_aligned() {
        let (mut chart, stream) = setup();
        let tape = (0..6)
            .flat_map(|index| {
                let time = index * 60_000_000 + 1;
                [
                    trade(time, 100.0, 3.0, AggressorSide::Buy),
                    trade(time + 1, 100.0, 4.0, AggressorSide::Sell),
                ]
            })
            .collect();
        chart.set_trade_stream_trades(stream, tape).unwrap();
        let id = chart
            .add_auction_markers(stream, 0, AuctionMarkerOptions::default())
            .unwrap();
        assert_eq!(chart.auction_markers_snapshot(id).unwrap().len(), 10);
        chart
            .set_auction_marker_options(
                id,
                AuctionMarkerOptions {
                    include_forming_bar: true,
                    ..AuctionMarkerOptions::default()
                },
            )
            .unwrap();
        assert_eq!(chart.auction_markers_snapshot(id).unwrap().len(), 12);
        let big = chart
            .add_big_trades(
                stream,
                0,
                BigTradesOptions {
                    filter: BigTradesFilter::Fixed {
                        minimum_volume: 1.0,
                    },
                    ..BigTradesOptions::default()
                },
            )
            .unwrap();
        assert!(chart.set_series_max_points(0, Some(3)));
        assert_eq!(chart.footprint_bars(0).unwrap().len(), 3);
        assert!(
            chart
                .big_trades_snapshot(big)
                .unwrap()
                .bubbles
                .iter()
                .all(|order| order.bar_time >= 180)
        );
        let snapshot = chart.auction_markers_snapshot(id).unwrap();
        assert_eq!(snapshot.len(), 6);
        assert_eq!(
            snapshot
                .iter()
                .map(|mark| mark.bar_time)
                .collect::<Vec<_>>(),
            vec![180, 180, 240, 240, 300, 300]
        );
        assert_eq!(chart.auction_markers[&stream][0].bars.len(), 3);
    }

    #[test]
    fn late_tail_repair_does_not_visit_retained_prefix() {
        let (mut chart, stream) = setup();
        let tape = (0..256)
            .flat_map(|row| {
                let time = row * 60_000_000 + 1;
                [
                    trade(time, 100.0, 3.0, AggressorSide::Buy),
                    trade(time + 1, 100.0, 4.0, AggressorSide::Sell),
                ]
            })
            .collect();
        chart.set_trade_stream_trades(stream, tape).unwrap();
        let id = chart
            .add_auction_markers(
                stream,
                0,
                AuctionMarkerOptions {
                    include_forming_bar: true,
                    ..AuctionMarkerOptions::default()
                },
            )
            .unwrap();
        let before = chart.auction_markers_snapshot(id).unwrap();
        chart
            .update_trade_stream_trade(
                stream,
                trade(250 * 60_000_000 + 3, 100.0, 4.0, AggressorSide::Sell),
            )
            .unwrap();
        assert_eq!(
            &chart.auction_markers_snapshot(id).unwrap()[..500],
            &before[..500],
            "unchanged prefix marks must stay identical"
        );
        assert!(
            chart.auction_markers[&stream][0].refreshed_bars <= 6,
            "late print should repair at most its six-bar suffix"
        );
    }

    /// One closed bar at `minute`: a buy and a sell at the same price, so every bar carries an
    /// unfinished auction on both sides.
    fn two_sided_bar(minute: i64) -> [FootprintTrade; 2] {
        let time = minute * 60_000_000 + 1;
        [
            trade(time, 100.0, 3.0, AggressorSide::Buy),
            trade(time + 1, 100.0, 4.0, AggressorSide::Sell),
        ]
    }

    #[test]
    fn live_tips_under_a_max_points_ceiling_evict_marks_without_redetection() {
        // Fork guarantee (X14, Target D): retention drops the evicted bars' marks in step with
        // the stream and the tip re-detects only the bars it changed, never the retained history.
        let (mut chart, stream) = setup();
        let tape = (0..40).flat_map(two_sided_bar).collect::<Vec<_>>();
        chart.set_trade_stream_trades(stream, tape).unwrap();
        let id = chart
            .add_auction_markers(stream, 0, AuctionMarkerOptions::default())
            .unwrap();
        assert!(chart.set_series_max_points(0, Some(16)));
        let mut trims = 0;
        for minute in 40..60 {
            let first = chart.trade_stream(stream).unwrap().bars()[0].start_timestamp_micros;
            let detected = chart.auction_markers[&stream][0].detected_bars;
            chart
                .update_trade_stream_trades(stream, two_sided_bar(minute).to_vec())
                .unwrap();
            let bars = chart.trade_stream(stream).unwrap().bars();
            trims += usize::from(bars[0].start_timestamp_micros != first);
            let entry = &chart.auction_markers[&stream][0];
            assert!(
                entry.refreshed_bars <= 2 && entry.detected_bars - detected <= 2,
                "tip {minute} re-detected {} bars",
                entry.detected_bars - detected
            );
            assert_eq!(entry.bars.len(), bars.len(), "tip {minute}");
            assert!(
                entry
                    .bars
                    .iter()
                    .zip(bars)
                    .all(|(marks, bar)| marks.start_micros == bar.start_timestamp_micros),
                "tip {minute}: marks are misaligned with the stream bars"
            );
        }
        assert!(trims >= 1, "the ceiling must evict during the tips");
        // A fresh build over the whole tape, restricted to the retained bars.
        let (mut fresh, fresh_stream) = setup();
        fresh
            .set_trade_stream_trades(fresh_stream, (0..60).flat_map(two_sided_bar).collect())
            .unwrap();
        let fresh_id = fresh
            .add_auction_markers(fresh_stream, 0, AuctionMarkerOptions::default())
            .unwrap();
        let first_time =
            chart.trade_stream(stream).unwrap().bars()[0].start_timestamp_micros / 1_000_000;
        let expected = fresh
            .auction_markers_snapshot(fresh_id)
            .unwrap()
            .into_iter()
            .filter(|mark| mark.bar_time >= first_time)
            .collect::<Vec<_>>();
        assert_eq!(chart.auction_markers_snapshot(id).unwrap(), expected);
    }

    #[test]
    fn non_time_mark_keys_follow_footprint_and_big_trades_rows_through_prepends_and_trims() {
        // Fork guarantee (X15, owner decision 1): on a sequence axis a mark's `bar_time` is its
        // bar's row key, the key the footprint rows and big-trades orders carry, which a trim
        // never re-keys and a prepend extends in front.
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let options = OrderFlowPresentationOptions {
            aggregation: FootprintAggregationOptions {
                tick_size: 1.0,
                ticks_per_row: 1,
                bars: FootprintBarAggregation::Trades { trades_per_bar: 2 },
                ..FootprintAggregationOptions::default()
            },
            big_trades: Some(BigTradesOptions {
                filter: BigTradesFilter::Fixed {
                    minimum_volume: 1.0,
                },
                ..BigTradesOptions::default()
            }),
            ..presentation_options(1, true)
        };
        let presentation = chart
            .add_order_flow_presentation("auction", 0, options)
            .unwrap();
        let stream = presentation.trade_stream();
        let footprint = presentation.footprint_series().unwrap();
        let big = presentation.big_trades().unwrap();
        chart
            .update_order_flow_presentation(
                presentation,
                (20..40).flat_map(two_sided_bar).collect(),
                false,
            )
            .unwrap();
        let id = chart
            .add_auction_markers(
                stream,
                footprint,
                AuctionMarkerOptions {
                    include_forming_bar: true,
                    ..AuctionMarkerOptions::default()
                },
            )
            .unwrap();
        let check = |chart: &ChartEngine, context: &str| {
            let rows = chart
                .data_layer()
                .series_data(footprint)
                .unwrap()
                .0
                .to_vec();
            assert_eq!(
                rows.len(),
                chart.trade_stream(stream).unwrap().bars().len(),
                "{context}"
            );
            let marks = chart
                .auction_markers_snapshot(id)
                .unwrap()
                .iter()
                .map(|mark| mark.bar_time)
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(marks.into_iter().collect::<Vec<_>>(), rows, "{context}");
            let orders = chart
                .big_trades_snapshot(big)
                .unwrap()
                .bubbles
                .iter()
                .map(|order| order.bar_time)
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(orders.into_iter().collect::<Vec<_>>(), rows, "{context}");
        };
        check(&chart, "install");
        let stats = chart
            .prepend_order_flow_history(presentation, (10..20).flat_map(two_sided_bar).collect())
            .unwrap();
        assert_eq!(stats.accepted_trades, 20);
        check(&chart, "prepend");
        assert!(chart.set_series_max_points(footprint, Some(12)));
        let first_key = chart.data_layer().series_data(footprint).unwrap().0[0];
        assert!(first_key > 0, "the trim keeps the retained rows' keys");
        check(&chart, "trim");
        for minute in 40..44 {
            chart
                .update_order_flow_presentation(presentation, two_sided_bar(minute).to_vec(), true)
                .unwrap();
            check(&chart, &format!("tip {minute}"));
        }
    }

    #[test]
    fn presentation_less_sequence_trims_shift_mark_keys_with_big_trades() {
        // Without presentation rows a sequence stream keys its bars by position, so a front trim
        // shifts the retained keys down. Marks follow, as big trades do (owner decision 1), and
        // equal a fresh build over the retained tape.
        let options = FootprintAggregationOptions {
            tick_size: 1.0,
            bars: FootprintBarAggregation::Trades { trades_per_bar: 2 },
            ..FootprintAggregationOptions::default()
        };
        let markers = AuctionMarkerOptions {
            include_forming_bar: true,
            ..AuctionMarkerOptions::default()
        };
        let big_trades = BigTradesOptions {
            filter: BigTradesFilter::Fixed {
                minimum_volume: 1.0,
            },
            ..BigTradesOptions::default()
        };
        let build = |minutes: std::ops::Range<i64>| {
            let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
            let stream = chart.add_trade_stream("auction", options).unwrap();
            let big = chart.add_big_trades(stream, 0, big_trades.clone()).unwrap();
            let id = chart
                .add_auction_markers(stream, 0, markers.clone())
                .unwrap();
            chart
                .set_trade_stream_trades(stream, minutes.flat_map(two_sided_bar).collect())
                .unwrap();
            (chart, stream, big, id)
        };
        let keys = |chart: &ChartEngine, big, id| {
            let marks = chart
                .auction_markers_snapshot(id)
                .unwrap()
                .iter()
                .map(|mark| mark.bar_time)
                .collect::<std::collections::BTreeSet<_>>();
            let orders = chart
                .big_trades_snapshot(big)
                .unwrap()
                .bubbles
                .iter()
                .map(|order| order.bar_time)
                .collect::<std::collections::BTreeSet<_>>();
            (marks, orders)
        };
        let (mut chart, stream, big, id) = build(0..6);
        assert_eq!(chart.sequence_key_base(stream), Some(0));
        chart.trim_trade_stream_front(stream, None, 3);
        assert_eq!(chart.trade_stream(stream).unwrap().bars().len(), 3);
        let (marks, orders) = keys(&chart, big, id);
        assert_eq!(marks, (0..3).collect());
        assert_eq!(marks, orders);
        let (fresh, _, _, fresh_id) = build(3..6);
        assert_eq!(
            chart.auction_markers_snapshot(id).unwrap(),
            fresh.auction_markers_snapshot(fresh_id).unwrap()
        );
        // The next tip keys its bar after the shifted ones.
        chart
            .update_trade_stream_trades(stream, two_sided_bar(6).to_vec())
            .unwrap();
        let (marks, orders) = keys(&chart, big, id);
        assert_eq!(marks, (0..4).collect());
        assert_eq!(marks, orders);
    }

    #[test]
    fn marker_only_streams_count_one_dependent_and_refuse_removal_or_regrid() {
        // Fork guarantee (X16): auction markers are counted at every in-use site, so a stream
        // they read is neither removed, pruned, nor re-aggregated under them.
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let options = FootprintAggregationOptions {
            tick_size: 1.0,
            ..FootprintAggregationOptions::default()
        };
        let stream = chart.add_trade_stream("auction", options).unwrap();
        chart
            .set_trade_stream_trades(stream, (0..4).flat_map(two_sided_bar).collect())
            .unwrap();
        let id = chart
            .add_auction_markers(stream, 0, AuctionMarkerOptions::default())
            .unwrap();
        assert_eq!(chart.trade_stream_stats(stream).unwrap().dependent_count, 1);
        assert_eq!(
            chart.remove_trade_stream(stream),
            Err(FootprintError::TradeStreamInUse(stream))
        );
        // A footprint bound next to the markers is not the stream's single owner.
        let footprint = chart.add_series(SeriesKind::Candlestick);
        let footprint_options = FootprintSeriesOptions {
            aggregation: options,
            ..FootprintSeriesOptions::default()
        };
        chart
            .configure_footprint_series(footprint, footprint_options.clone())
            .unwrap();
        chart
            .bind_footprint_series_to_stream(footprint, stream)
            .unwrap();
        let regrid = FootprintSeriesOptions {
            aggregation: FootprintAggregationOptions {
                tick_size: 2.0,
                ..options
            },
            ..footprint_options
        };
        assert_eq!(
            chart.apply_footprint_series_options(footprint, regrid.clone()),
            Err(FootprintError::TradeStreamInUse(stream))
        );
        assert!(!chart.auction_markers_snapshot(id).unwrap().is_empty());
        // Removing the markers' host series sweeps the markers with it.
        assert!(chart.remove_series(0));
        assert_eq!(
            chart.auction_markers_snapshot(id),
            Err(FootprintError::UnknownAuctionMarkers(id))
        );
        assert_eq!(chart.trade_stream_stats(stream).unwrap().dependent_count, 0);
        // With the markers gone the footprint owns the stream alone and may re-aggregate it.
        chart
            .apply_footprint_series_options(footprint, regrid)
            .unwrap();
    }

    #[test]
    fn marker_frames_are_identical_across_engine_instances() {
        // Fork guarantee (X18): marker sets iterate in stream order, so two engines given the
        // same state emit the same primitives in the same order.
        let build = || {
            let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
            for index in 0..6 {
                let stream = chart
                    .add_trade_stream(
                        &format!("auction-{index}"),
                        FootprintAggregationOptions {
                            tick_size: 1.0,
                            ..FootprintAggregationOptions::default()
                        },
                    )
                    .unwrap();
                let tape = (0..4)
                    .flat_map(|minute| {
                        let time = minute * 60_000_000 + 1;
                        let price = 100.0 + index as f64;
                        [
                            trade(time, price, 3.0, AggressorSide::Buy),
                            trade(time + 1, price, 4.0, AggressorSide::Sell),
                        ]
                    })
                    .collect();
                chart.set_trade_stream_trades(stream, tape).unwrap();
                chart
                    .add_auction_markers(stream, 0, AuctionMarkerOptions::default())
                    .unwrap();
            }
            let values = [100.0, 101.0, 102.0, 103.0];
            chart
                .set_series_data(
                    0,
                    &[0.0, 60.0, 120.0, 180.0],
                    &values,
                    &values,
                    &values,
                    &values,
                )
                .unwrap();
            chart.time_scale.set_width(800.0);
            chart.fit_content();
            chart.build_frame();
            let mut prims = Vec::new();
            chart.build_auction_markers_frame(0, 0, 3, 1.0, 1.0, &mut prims);
            prims
        };
        let first = build();
        assert!(!first.is_empty());
        for _ in 0..8 {
            assert_eq!(build(), first);
        }
    }
}
