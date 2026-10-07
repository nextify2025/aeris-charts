//! Tick-truth aggregation for footprint / numbers-bar series.
//!
//! The retained trade tape is authoritative. Bars, price levels, delta extrema, POC, and
//! imbalances are derived state and can be rebuilt deterministically after a late event. Rendering
//! consumes [`FootprintBar`] values; it never attempts to infer order flow from OHLC rows.

use std::collections::{HashMap, HashSet, VecDeque};

use aeris_charts_core::model::data_layer::{PointColorChannel, SeriesId, SeriesIdError};
use aeris_charts_core::model::data_validation::{MAX_SAFE_VALUE, MIN_SAFE_VALUE};
use aeris_charts_core::scale::exchange_time::ExchangeTime;
use aeris_charts_core::scale::session_slots::{
    OutOfSessionPolicy, SessionBarGrid, SessionSlotError, SessionWindow,
};
use aeris_charts_core::style::{MARKET_DOWN_RGB, MARKET_UP_RGB};
use aeris_charts_render::color::Color;

use crate::{
    ChartEngine, PriceFormatKind, SEPARATE_INDICATOR_PANE_STRETCH, SeriesKind, SeriesOwner,
    SeriesPriceFormat,
};

const MICROS_PER_SECOND: i64 = 1_000_000;
const REPLAY_CHECKPOINT_INTERVAL: usize = 1_024;
const MAX_REPLAY_CHECKPOINTS: usize = 64;
const MIN_TIMESTAMP_MICROS: i64 = -62_167_219_200 * MICROS_PER_SECOND;
const MAX_TIMESTAMP_MICROS: i64 = 253_402_300_799 * MICROS_PER_SECOND + 999_999;
pub const MAX_TRADE_STREAMS: usize = 64;
pub const MAX_TRADE_STREAM_KEY_BYTES: usize = 128;
pub const MAX_TIME_AND_SALES_ROWS: usize = 4_096;
/// Hard ceiling on the raw tape an order-flow presentation retains. Past it the oldest whole
/// bars are sealed with the shared retention hysteresis: their trades are released and the
/// finished bars stay as history, so completed footprints outlive any raw window.
pub const ORDER_FLOW_MAX_RETAINED_TRADES: usize = 262_144;
/// Trading sessions of sealed history an order-flow stream keeps. Older sessions are evicted.
pub const ORDER_FLOW_MAX_RETAINED_SESSIONS: usize = 5;
/// Memory ceiling of one order-flow stream: raw tape plus sealed history. The oldest sealed bars
/// are evicted with the shared retention hysteresis once it is exceeded.
pub const ORDER_FLOW_MAX_STREAM_BYTES: usize = 384 * 1024 * 1024;
/// Footprint bar spacing in CSS px that fits `bid x ask` numbers at the default font size.
pub const FOOTPRINT_BAR_SPACING: f64 = 104.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TradeStudyKind {
    CumulativeDelta,
    DeltaHistogram,
    /// Total traded volume of each derived bar.
    Volume,
}

/// Session-anchored time bars for a trade stream: exchange-local windows restart the time-bar
/// grid at every window open (see [`SessionBarGrid`]), in the chart's exchange time zone and
/// session start. `outside` decides what happens to prints outside every window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TradeSessionOptions {
    pub windows: Vec<SessionWindow>,
    pub outside: OutOfSessionPolicy,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CumulativeDeltaReset {
    #[default]
    Session,
    Continuous,
    Anchored,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TradeStudyOptions {
    pub cumulative_delta_reset: CumulativeDeltaReset,
    pub anchor_timestamp_micros: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TradeStreamStats {
    pub revision: u64,
    pub stream_capacity_bytes: usize,
    pub dependent_count: usize,
    pub dependent_rebuilds: u64,
    pub dependent_incremental_updates: u64,
    /// Lifetime CVD/delta rows computed. A live tip steps only the changed bar suffix.
    #[serde(default)]
    pub dependent_rows_computed: u64,
    /// Lifetime footprint and ordinary candle/bar rows projected from the stream's bars.
    #[serde(default)]
    pub bar_rows_projected: u64,
    /// Lifetime tape prints big-trades indicators folded into rebuilt orders. A live tip folds
    /// only its new prints; a retention trim evicts orders in place and folds none.
    #[serde(default)]
    pub big_trades_prints_scanned: u64,
    /// Lifetime big-trades replays of the whole visible tape: tape replacement, corrections,
    /// replay seeks, and filter or grouping changes. Live tips and retention trims replay nothing.
    #[serde(default)]
    pub big_trades_replays: u64,
}

/// Lifetime work telemetry of one stream's chart dependents (see [`TradeStreamStats`]).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TradeDependentWork {
    study_rows_computed: u64,
    bar_rows_projected: u64,
    pub(crate) big_trades_prints_scanned: u64,
    pub(crate) big_trades_replays: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct ReplaySeekStats {
    pub previous_clock_micros: Option<i64>,
    pub clock_micros: Option<i64>,
    pub visible_trades: usize,
    pub rebuilt_trades: usize,
    pub incremental_trades: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct ReplayClockStats {
    pub previous_clock_micros: Option<i64>,
    pub clock_micros: Option<i64>,
    pub stream_count: usize,
    pub depth_stream_count: usize,
    pub visible_trades: usize,
    pub rebuilt_trades: usize,
    pub incremental_trades: usize,
    pub visible_depth_events: usize,
    pub rebuilt_depth_events: usize,
}

/// Which side initiated a trade. Unknown trades remain in total volume but never manufacture bid
/// or ask volume.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggressorSide {
    Buy,
    Sell,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct TimeAndSalesOptions {
    pub minimum_volume: f64,
    pub side: Option<AggressorSide>,
    pub max_rows: usize,
}

impl Default for TimeAndSalesOptions {
    fn default() -> Self {
        Self {
            minimum_volume: 0.0,
            side: None,
            max_rows: 500,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct TimeAndSalesRow {
    pub timestamp_micros: i64,
    pub price: f64,
    pub volume: f64,
    pub aggressor: AggressorSide,
    pub trade_id: Option<u64>,
    pub conditions: u32,
}

/// One raw trade event. `timestamp_micros` is signed Unix time at microsecond resolution. The
/// optional quote is the market state at the trade and is used only when `aggressor` is unknown.
#[derive(Clone, Debug, PartialEq)]
pub struct FootprintTrade {
    pub timestamp_micros: i64,
    pub price: f64,
    pub volume: f64,
    pub aggressor: AggressorSide,
    pub bid: Option<f64>,
    pub ask: Option<f64>,
    /// Feed ordering within an equal timestamp. Missing sequences retain input order.
    pub sequence: Option<u64>,
    /// Stable provider identity used for idempotent correction/replay when available.
    pub trade_id: Option<u64>,
    /// Opaque feed condition bits retained for later microstructure extensions.
    pub conditions: u32,
    /// Host-defined session identity. A change starts a new bar and resets session delta.
    pub session_id: Option<u64>,
}

/// The bar-building policy. Time bars are aligned to `anchor_micros`; trade and volume bars start
/// at their first event. A trade is never split between volume bars.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FootprintBarAggregation {
    Time {
        interval_micros: u64,
        anchor_micros: i64,
    },
    Trades {
        trades_per_bar: u32,
    },
    Volume {
        volume_per_bar: f64,
    },
    /// Close a bar once its high-low span reaches this many integer ticks. A trade is never
    /// split; the event that reaches the threshold remains in the closing bar.
    Range {
        range_ticks: u32,
    },
}

impl Default for FootprintBarAggregation {
    fn default() -> Self {
        Self::Time {
            interval_micros: 60_000_000,
            anchor_micros: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FootprintImbalanceOptions {
    /// Dominant volume divided by the diagonally adjacent opposite-side volume.
    pub ratio: f64,
    /// Minimum dominant-side volume required before a level can be imbalanced.
    pub minimum_volume: f64,
    /// Adjacent imbalanced levels required to mark the complete run as stacked.
    pub consecutive_levels: u32,
}

impl Default for FootprintImbalanceOptions {
    fn default() -> Self {
        Self {
            ratio: 3.0,
            minimum_volume: 10.0,
            consecutive_levels: 3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FootprintAggregationOptions {
    /// Instrument price increment. Every trade must lie on this grid.
    pub tick_size: f64,
    /// Ticks grouped into one footprint row. `1` keeps one row per tick; larger values
    /// aggregate adjacent ticks so dense instruments stay legible.
    pub ticks_per_row: u32,
    pub bars: FootprintBarAggregation,
    pub imbalance: FootprintImbalanceOptions,
}

impl FootprintAggregationOptions {
    /// Price height of one footprint row.
    #[must_use]
    pub fn row_size(&self) -> f64 {
        self.tick_size * f64::from(self.ticks_per_row)
    }

    /// Row identity containing `price`; the row spans `ticks_per_row` ticks starting at
    /// `row * row_size()`.
    pub(crate) fn row_level(&self, price: f64) -> i64 {
        price_level(price, self.tick_size).div_euclid(i64::from(self.ticks_per_row.max(1)))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FootprintCellMode {
    #[default]
    BidAsk,
    Total,
    Delta,
    ProfileInBar,
    VolumeLadder,
    HorizontalImbalance,
    BidAskHistogram,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FootprintVisualOptions {
    pub cell_mode: FootprintCellMode,
    pub font_size: f64,
    pub bid_color: Color,
    pub ask_color: Color,
    pub positive_delta_color: Color,
    pub negative_delta_color: Color,
    /// `None` follows the chart layout foreground and retokenizes on theme changes.
    pub text_color: Option<Color>,
    pub poc_color: Color,
    pub stacked_bid_color: Color,
    pub stacked_ask_color: Color,
    pub show_bar_summary: bool,
    /// Merge stored rows into display rows that stay legible at the current zoom.
    pub adaptive_rows: bool,
}

impl Default for FootprintVisualOptions {
    fn default() -> Self {
        Self {
            cell_mode: FootprintCellMode::BidAsk,
            font_size: 11.0,
            bid_color: Color::rgba(239, 83, 80, 70),
            ask_color: Color::rgba(MARKET_UP_RGB.0, MARKET_UP_RGB.1, MARKET_UP_RGB.2, 70),
            positive_delta_color: Color::rgba(
                MARKET_UP_RGB.0,
                MARKET_UP_RGB.1,
                MARKET_UP_RGB.2,
                110,
            ),
            negative_delta_color: Color::rgba(239, 83, 80, 110),
            text_color: None,
            poc_color: Color::rgb(255, 193, 7),
            stacked_bid_color: Color::rgb(255, 82, 82),
            stacked_ask_color: Color::rgb(MARKET_UP_RGB.0, MARKET_UP_RGB.1, MARKET_UP_RGB.2),
            show_bar_summary: true,
            adaptive_rows: false,
        }
    }
}

impl FootprintVisualOptions {
    pub(crate) fn reset_style_to_defaults(&mut self) {
        let defaults = Self::default();
        self.font_size = defaults.font_size;
        self.bid_color = defaults.bid_color;
        self.ask_color = defaults.ask_color;
        self.positive_delta_color = defaults.positive_delta_color;
        self.negative_delta_color = defaults.negative_delta_color;
        self.text_color = defaults.text_color;
        self.poc_color = defaults.poc_color;
        self.stacked_bid_color = defaults.stacked_bid_color;
        self.stacked_ask_color = defaults.stacked_ask_color;
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct FootprintSeriesOptions {
    pub aggregation: FootprintAggregationOptions,
    pub visual: FootprintVisualOptions,
}

/// One atomic chart presentation derived from a shared canonical trade stream.
#[derive(Clone, Debug, PartialEq)]
pub struct OrderFlowPresentationOptions {
    pub aggregation: FootprintAggregationOptions,
    pub visual: FootprintVisualOptions,
    pub show_footprint: bool,
    pub show_cumulative_delta: bool,
    pub show_delta_histogram: bool,
    /// Big-trades bubbles over the primary price series.
    pub big_trades: Option<crate::BigTradesOptions>,
}

/// Engine-issued identities for one order-flow presentation graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderFlowPresentation {
    trade_stream: u64,
    footprint_series: Option<SeriesId>,
    cumulative_delta_series: Option<SeriesId>,
    delta_series: Option<SeriesId>,
    big_trades: Option<crate::NativePrimitiveId>,
    ticks_per_row: u32,
    primary_series: SeriesId,
}

impl OrderFlowPresentation {
    #[must_use]
    pub const fn trade_stream(self) -> u64 {
        self.trade_stream
    }

    #[must_use]
    pub const fn footprint_series(self) -> Option<SeriesId> {
        self.footprint_series
    }

    #[must_use]
    pub const fn cumulative_delta_series(self) -> Option<SeriesId> {
        self.cumulative_delta_series
    }

    #[must_use]
    pub const fn delta_series(self) -> Option<SeriesId> {
        self.delta_series
    }

    #[must_use]
    pub const fn big_trades(self) -> Option<crate::NativePrimitiveId> {
        self.big_trades
    }

    #[must_use]
    pub const fn ticks_per_row(self) -> u32 {
        self.ticks_per_row
    }
}

impl Default for FootprintAggregationOptions {
    fn default() -> Self {
        Self {
            tick_size: 0.25,
            ticks_per_row: 1,
            bars: FootprintBarAggregation::default(),
            imbalance: FootprintImbalanceOptions::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub struct FootprintLevel {
    /// Exact integer row identity; `price == level * row size` is the row's lowest tick. Stored
    /// bars keep one row per instrument tick; presented bars group `ticks_per_row` of them.
    pub level: i64,
    pub price: f64,
    pub bid_volume: f64,
    pub ask_volume: f64,
    pub unknown_volume: f64,
    pub total_volume: f64,
    pub delta: f64,
    pub bid_imbalance: bool,
    pub ask_imbalance: bool,
    pub stacked_bid_imbalance: bool,
    pub stacked_ask_imbalance: bool,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct FootprintBar {
    /// Deterministic logical position within the canonical bar sequence. Unlike the display
    /// timestamp, this position remains distinct when several non-time bars open within one second.
    pub logical_index: u64,
    pub start_timestamp_micros: i64,
    pub end_timestamp_micros: i64,
    pub session_id: Option<u64>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub bid_volume: f64,
    pub ask_volume: f64,
    pub unknown_volume: f64,
    pub total_volume: f64,
    pub delta: f64,
    pub delta_percent: f64,
    /// Highest running bar delta observed after applying each trade, with zero as the initial
    /// state. This deliberately cannot be reconstructed from final `delta`.
    pub max_delta: f64,
    /// Lowest running bar delta observed after applying each trade, with zero as the initial state.
    pub min_delta: f64,
    /// Session cumulative delta at the end of this bar.
    pub session_delta: f64,
    pub trade_count: u32,
    pub poc_level: i64,
    pub poc_price: f64,
    /// Sorted ascending by integer price level.
    pub levels: Vec<FootprintLevel>,
}

impl FootprintBar {
    fn clone_without_levels(&self) -> Self {
        Self {
            levels: Vec::new(),
            ..*self
        }
    }
}

/// Read-only view of the canonical logical bar domain produced by a trade aggregator. The view
/// intentionally keeps open/close microsecond times beside the logical index; callers must not
/// derive a non-time bar's horizontal identity by truncating either timestamp to whole seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct BarSequencePoint {
    pub logical_index: u64,
    pub open_timestamp_micros: i64,
    pub close_timestamp_micros: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct BarSequence<'a> {
    bars: &'a [FootprintBar],
}

impl<'a> BarSequence<'a> {
    pub fn len(self) -> usize {
        self.bars.len()
    }

    pub fn is_empty(self) -> bool {
        self.bars.is_empty()
    }

    pub fn get(self, position: usize) -> Option<BarSequencePoint> {
        self.bars.get(position).map(|bar| BarSequencePoint {
            logical_index: bar.logical_index,
            open_timestamp_micros: bar.start_timestamp_micros,
            close_timestamp_micros: bar.end_timestamp_micros,
        })
    }

    pub fn iter(self) -> impl ExactSizeIterator<Item = BarSequencePoint> + 'a {
        self.bars.iter().map(|bar| BarSequencePoint {
            logical_index: bar.logical_index,
            open_timestamp_micros: bar.start_timestamp_micros,
            close_timestamp_micros: bar.end_timestamp_micros,
        })
    }
}

/// Mapping from a prior logical bar sequence to a rebuilt sequence. Common bars are matched by
/// their full-resolution open/close times in order, so duplicate whole-second labels do not
/// collapse into one identity. Anchors between common bars are mapped by the piecewise sequence
/// position and anchors outside the common extent extrapolate the nearest shift.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BarSequenceMapping {
    common_indices: Vec<(u64, u64)>,
}

impl BarSequenceMapping {
    pub fn between(old: BarSequence<'_>, new: BarSequence<'_>) -> Self {
        Self::between_iter(old.iter(), new.iter())
    }

    pub fn between_points(old: &[BarSequencePoint], new: &[BarSequencePoint]) -> Self {
        Self::between_iter(old.iter().copied(), new.iter().copied())
    }

    fn between_iter<I, J>(old: I, new: J) -> Self
    where
        I: IntoIterator<Item = BarSequencePoint>,
        J: IntoIterator<Item = BarSequencePoint>,
    {
        let mut common_indices: Vec<(u64, u64)> = Vec::new();
        let mut old = old.into_iter().peekable();
        let mut new = new.into_iter().peekable();
        while let (Some(&old_point), Some(&new_point)) = (old.peek(), new.peek()) {
            match (
                old_point.open_timestamp_micros,
                old_point.close_timestamp_micros,
            )
                .cmp(&(
                    new_point.open_timestamp_micros,
                    new_point.close_timestamp_micros,
                )) {
                std::cmp::Ordering::Less => {
                    old.next();
                }
                std::cmp::Ordering::Greater => {
                    new.next();
                }
                std::cmp::Ordering::Equal => {
                    let current = (old_point.logical_index, new_point.logical_index);
                    if common_indices.len() >= 2 {
                        let a = common_indices[common_indices.len() - 2];
                        let b = common_indices[common_indices.len() - 1];
                        let ab_old = (b.0 - a.0) as u128;
                        let ab_new = (b.1 - a.1) as u128;
                        let bc_old = (current.0 - b.0) as u128;
                        let bc_new = (current.1 - b.1) as u128;
                        if ab_old * bc_new == ab_new * bc_old {
                            if let Some(last) = common_indices.last_mut() {
                                *last = current;
                            }
                        } else {
                            common_indices.push(current);
                        }
                    } else {
                        common_indices.push(current);
                    }
                    old.next();
                    new.next();
                }
            }
        }
        Self { common_indices }
    }

    pub fn is_empty(&self) -> bool {
        self.common_indices.is_empty()
    }

    pub fn map_logical_index(&self, logical_index: u64) -> Option<u64> {
        let first = *self.common_indices.first()?;
        let last = *self.common_indices.last()?;
        if logical_index <= first.0 {
            return Some(
                first
                    .1
                    .saturating_sub(first.0.saturating_sub(logical_index)),
            );
        }
        if logical_index >= last.0 {
            return Some(last.1.saturating_add(logical_index.saturating_sub(last.0)));
        }
        let upper = self
            .common_indices
            .partition_point(|&(old_index, _)| old_index < logical_index);
        let (old_left, new_left) = self.common_indices[upper - 1];
        let (old_right, new_right) = self.common_indices[upper];
        let old_span = (old_right - old_left) as u128;
        let new_span = (new_right - new_left) as u128;
        let offset = (logical_index - old_left) as u128;
        let mapped = (new_left as u128).saturating_add(offset * new_span / old_span);
        u64::try_from(mapped).ok()
    }

    /// Map a fractional logical anchor through the same piecewise sequence mapping used for
    /// integer bar identities. Fractional anchors remain in the interpolation space between
    /// their neighboring bars instead of being rounded to a row.
    pub fn map_logical(&self, logical: f64) -> f64 {
        if !logical.is_finite() || self.common_indices.is_empty() {
            return logical;
        }
        let upper = self
            .common_indices
            .partition_point(|&(old_index, _)| (old_index as f64) < logical);
        if let Some(&(old_index, new_index)) = self.common_indices.get(upper)
            && old_index as f64 == logical
        {
            return new_index as f64;
        }
        if upper == 0 {
            let (old_index, new_index) = self.common_indices[0];
            return new_index as f64 + logical - old_index as f64;
        }
        if upper == self.common_indices.len() {
            let (old_index, new_index) = self.common_indices[upper - 1];
            return new_index as f64 + logical - old_index as f64;
        }
        let (old_left, new_left) = self.common_indices[upper - 1];
        let (old_right, new_right) = self.common_indices[upper];
        let fraction = (logical - old_left as f64) / (old_right - old_left) as f64;
        new_left as f64 + fraction * (new_right - new_left) as f64
    }

    pub fn rebase_anchor(
        &self,
        anchor: BarSequencePoint,
        new: BarSequence<'_>,
    ) -> Option<BarSequencePoint> {
        let logical_index = self.map_logical_index(anchor.logical_index)?;
        new.iter()
            .find(|point| point.logical_index == logical_index)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FootprintWorkStats {
    pub incremental_ticks: usize,
    pub historical_rebuilds: usize,
    pub rebuilt_ticks: usize,
    /// Late prints and corrections that fell inside sealed history and were not applied.
    pub skipped_sealed_trades: usize,
}

/// Outcome of joining an older tape page to the front of a stream's history.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HistoryPrefixStats {
    pub accepted_trades: usize,
    /// Prints at or after the start of the retained history, or repeats of retained trade ids.
    pub skipped_trades: usize,
    /// The history accepts no older trades: its oldest bars were evicted, earlier or because
    /// this page reached the session or memory budget. Hosts stop paging further back.
    pub history_full: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FootprintError {
    InvalidTradeStreamKey,
    TradeStreamCapacity,
    UnknownTradeStream(u64),
    TradeStreamInUse(u64),
    InvalidTickSize,
    InvalidAggregation,
    InvalidImbalance,
    InvalidVisualOptions,
    InvalidTimeAndSalesOptions,
    InvalidTimestamp { index: usize },
    InvalidPrice { index: usize },
    OffGridPrice { index: usize },
    InvalidVolume { index: usize },
    InvalidQuote { index: usize },
    DuplicateTradeId { trade_id: u64 },
    UnsupportedChartAggregation,
    SequenceDomainInUse,
    ProjectionTimeCollision,
    UnsupportedTradeBarSeries(SeriesId),
    SeriesOwned(SeriesId),
    UnsupportedBigTradesSeries(SeriesId),
    InvalidBigTradesOptions,
    BigTradesCapacity,
    UnknownBigTrades(crate::NativePrimitiveId),
    InvalidAuctionMarkerOptions,
    AuctionMarkerCapacity,
    UnknownAuctionMarkers(crate::NativePrimitiveId),
    UnknownSeries(SeriesId),
    StaleSeries(SeriesId),
    Depth(crate::DepthError),
    InvalidSessions(SessionSlotError),
}

impl core::fmt::Display for FootprintError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidTradeStreamKey => write!(f, "trade stream key is empty or too long"),
            Self::TradeStreamCapacity => write!(f, "trade stream capacity is exhausted"),
            Self::UnknownTradeStream(id) => write!(f, "unknown trade stream {id}"),
            Self::TradeStreamInUse(id) => write!(f, "trade stream {id} still has dependents"),
            Self::InvalidTickSize => write!(f, "tick_size must be finite and greater than zero"),
            Self::InvalidAggregation => write!(f, "footprint bar aggregation is invalid"),
            Self::InvalidImbalance => write!(f, "footprint imbalance options are invalid"),
            Self::InvalidVisualOptions => write!(f, "footprint visual options are invalid"),
            Self::InvalidTimeAndSalesOptions => write!(f, "time-and-sales options are invalid"),
            Self::InvalidTimestamp { index } => write!(f, "trade {index} has an invalid timestamp"),
            Self::InvalidPrice { index } => write!(f, "trade {index} has an invalid price"),
            Self::OffGridPrice { index } => {
                write!(f, "trade {index} price is not aligned to tick_size")
            }
            Self::InvalidVolume { index } => write!(f, "trade {index} has an invalid volume"),
            Self::InvalidQuote { index } => write!(f, "trade {index} has an invalid bid/ask quote"),
            Self::DuplicateTradeId { trade_id } => {
                write!(f, "trade_id {trade_id} appears more than once")
            }
            Self::UnsupportedChartAggregation => write!(
                f,
                "chart footprint series currently require whole-second aligned time bars"
            ),
            Self::SequenceDomainInUse => write!(
                f,
                "a chart can own only one independent non-time bar sequence, and none beside \
                 time-resampled series"
            ),
            Self::ProjectionTimeCollision => write!(
                f,
                "two footprint bars resolve to the same canonical chart second"
            ),
            Self::UnsupportedTradeBarSeries(id) => write!(
                f,
                "series {id} must be a candlestick or OHLC bar presentation"
            ),
            Self::SeriesOwned(id) => {
                write!(
                    f,
                    "series {id} is already written by another engine feature"
                )
            }
            Self::UnsupportedBigTradesSeries(id) => write!(
                f,
                "series {id} must be a candlestick, bar, line, area, baseline, or footprint series"
            ),
            Self::InvalidBigTradesOptions => write!(f, "big-trades options are invalid"),
            Self::BigTradesCapacity => write!(f, "big-trades indicator capacity is exhausted"),
            Self::UnknownBigTrades(id) => write!(f, "unknown big-trades indicator {id}"),
            Self::InvalidAuctionMarkerOptions => write!(f, "auction-marker options are invalid"),
            Self::AuctionMarkerCapacity => write!(f, "auction-marker capacity is exhausted"),
            Self::UnknownAuctionMarkers(id) => write!(f, "unknown auction markers {id}"),
            Self::UnknownSeries(id) => write!(f, "unknown series id {id}"),
            Self::StaleSeries(id) => write!(f, "stale series id {id}"),
            Self::Depth(error) => write!(f, "depth replay failed: {error}"),
            Self::InvalidSessions(error) => write!(f, "invalid trade sessions: {error}"),
        }
    }
}

impl std::error::Error for FootprintError {}

#[derive(Clone, Debug)]
struct StoredTrade {
    event: FootprintTrade,
    input_order: u64,
    classified_side: AggressorSide,
}

#[derive(Clone, Copy, Debug, Default)]
struct RebuildSeed {
    last_trade_price: Option<f64>,
    last_classified_side: AggressorSide,
    active_session: Option<Option<u64>>,
    session_delta: f64,
    /// Running sum of the evicted bars' final deltas, in bar order: continuous cumulative delta
    /// resumes from it exactly as a fold over the full history would.
    cumulative_delta: f64,
}

#[derive(Clone, Debug)]
struct ReplayCheckpoint {
    trade_count: usize,
    bars_len: usize,
    active_bar: Option<FootprintBar>,
    last_trade_price: Option<f64>,
    last_classified_side: AggressorSide,
    active_session: Option<Option<u64>>,
    session_delta: f64,
}

/// Authoritative tick tape plus its reusable derived footprint bars.
///
/// Bars form two tiers. The leading `sealed_bars` are final history whose raw trades were
/// released; every later bar is derived from the retained raw tape and can be rebuilt from it.
/// Levels are stored at the instrument tick, so `ticks_per_row` only groups rows for display.
#[derive(Clone, Debug)]
pub struct FootprintAggregator {
    options: FootprintAggregationOptions,
    /// Canonical tape. A deque so retention evicts from the front in proportion to the evicted
    /// trades instead of moving every surviving trade.
    trades: VecDeque<StoredTrade>,
    /// Provider trade id to absolute tape position; `trades[i]` sits at `trade_id_base + i`.
    trade_ids: HashMap<u64, usize>,
    /// Absolute position of `trades[0]`. Retention advances it instead of reindexing the
    /// surviving trades, so evicting history costs work proportional to the evicted trades.
    trade_id_base: usize,
    bars: Vec<FootprintBar>,
    next_input_order: u64,
    rebuild_seed: RebuildSeed,
    last_trade_price: Option<f64>,
    last_classified_side: AggressorSide,
    active_session: Option<Option<u64>>,
    session_delta: f64,
    work: FootprintWorkStats,
    revision: u64,
    replay_clock_micros: Option<i64>,
    replay_checkpoints: Vec<ReplayCheckpoint>,
    /// Exchange-session anchoring of time bars; `None` keeps the plain `anchor_micros` grid.
    session_grid: Option<SessionBarGrid>,
    /// Bar key of the last print on the whole tape, hidden prints included, that joins a time bar
    /// (`None` for other aggregations or before any such print). A tip is checked against it, so
    /// a print the replay clock still hides cannot collide with one appended later.
    tail_bar_key: Option<TimeBarKey>,
    /// Chart-dependent work telemetry; carried across tape replacement and corrections.
    dependent_work: TradeDependentWork,
    sealed_bars: usize,
    /// Bytes held by the sealed bars, maintained as bars are sealed, joined and evicted.
    sealed_bytes: usize,
    /// Canonical order key of the first raw trade when bars were last sealed. Events sorting
    /// before it belong to sealed history and are skipped instead of rewriting final bars.
    sealed_floor: Option<(i64, u64, u64)>,
    /// Earliest trade time covered by sealed history, so an older page never overlaps it.
    sealed_start_micros: Option<i64>,
    /// Oldest bars were evicted, so the history can no longer be extended further back.
    history_truncated: bool,
    /// Order key bounding the prints a grid change released without bars (see
    /// `discard_sealed_history`), kept until the refill's first page joins or the history is
    /// truncated. A later seal moves `sealed_floor` and the history start but never this bound.
    refill_floor: Option<(i64, u64, u64)>,
    /// Ids of prints tying `refill_floor` on time and sequence that a seal released after the
    /// grid change, so a refill page repeating them is deduplicated like the raw tape's prints.
    refill_released_ids: HashSet<u64>,
    /// Raw trades released by sealing since the tape was last replaced.
    released_trades: u64,
    /// Bar-zero position offset: grows as bars are evicted, shrinks as history is prepended.
    bar_origin: i64,
    /// Incremented whenever the whole tape is replaced or its bars are re-placed on another grid
    /// (sessions, exchange time, interval, or tick size), so tape-replaying dependents start over.
    tape_epoch: u64,
}

fn bar_bytes(bar: &FootprintBar) -> usize {
    core::mem::size_of::<FootprintBar>()
        + bar.levels.capacity() * core::mem::size_of::<FootprintLevel>()
}

/// Open (microseconds) and session of the time bar a print joins. A session change forces a new
/// bar, so two prints with one open but different sessions would give two bars one open time.
type TimeBarKey = (i64, Option<u64>);

#[derive(Clone, Debug)]
pub(crate) struct FootprintSeriesState {
    pub trade_stream_id: u64,
    pub visual: FootprintVisualOptions,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TradeStudyDependent {
    pub series_id: SeriesId,
    pub kind: TradeStudyKind,
    pub options: TradeStudyOptions,
    pub applied_revision: u64,
    pub rebuilds: u64,
    pub incremental_updates: u64,
    /// Anchored cumulative-delta base established by bars retention already evicted. With the
    /// stream's evicted cumulative delta it seeds the fold, so evicting history never rewrites
    /// the values of the bars that remain.
    anchor_base: Option<f64>,
    /// Cumulative-delta running state before `bars[row]`, valid while that bar prefix is unchanged.
    resume: Option<(usize, CumulativeDeltaFold)>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TradeBarDependent {
    pub series_id: SeriesId,
    pub applied_revision: u64,
    pub rebuilds: u64,
    pub incremental_updates: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FootprintUpdateKind {
    Tip,
    Historical,
}

impl FootprintAggregator {
    pub fn new(options: FootprintAggregationOptions) -> Result<Self, FootprintError> {
        validate_options(options)?;
        Ok(Self {
            options,
            trades: VecDeque::new(),
            trade_ids: HashMap::new(),
            trade_id_base: 0,
            bars: Vec::new(),
            next_input_order: 0,
            rebuild_seed: RebuildSeed::default(),
            last_trade_price: None,
            last_classified_side: AggressorSide::Unknown,
            active_session: None,
            session_delta: 0.0,
            work: FootprintWorkStats::default(),
            revision: 1,
            replay_clock_micros: None,
            replay_checkpoints: Vec::new(),
            session_grid: None,
            tail_bar_key: None,
            dependent_work: TradeDependentWork::default(),
            sealed_bars: 0,
            sealed_bytes: 0,
            sealed_floor: None,
            sealed_start_micros: None,
            history_truncated: false,
            refill_floor: None,
            refill_released_ids: HashSet::new(),
            released_trades: 0,
            bar_origin: 0,
            tape_epoch: 0,
        })
    }

    /// Anchor time bars to exchange-local session windows placed in `exchange` time (`None`
    /// restores the plain `anchor_micros` grid), then rebuild the derived bars from the tape.
    /// Each window restarts the bar grid at its open; prints outside every window follow
    /// `sessions.outside`. Requires whole-second time bars of at most one day.
    pub fn set_sessions(
        &mut self,
        sessions: Option<&TradeSessionOptions>,
        exchange: &ExchangeTime,
    ) -> Result<(), FootprintError> {
        let grid = match sessions {
            None => None,
            Some(sessions) => {
                let FootprintBarAggregation::Time {
                    interval_micros, ..
                } = self.options.bars
                else {
                    return Err(FootprintError::UnsupportedChartAggregation);
                };
                let seconds = interval_micros / MICROS_PER_SECOND as u64;
                if seconds * MICROS_PER_SECOND as u64 != interval_micros {
                    return Err(FootprintError::UnsupportedChartAggregation);
                }
                let seconds = u32::try_from(seconds).unwrap_or(u32::MAX);
                Some(
                    SessionBarGrid::new(
                        sessions.windows.clone(),
                        seconds,
                        exchange,
                        sessions.outside,
                    )
                    .map_err(FootprintError::InvalidSessions)?,
                )
            }
        };
        if grid == self.session_grid {
            return Ok(());
        }
        self.session_grid = grid;
        self.discard_sealed_history();
        self.rebuild();
        self.refresh_tail_bar_key();
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }

    /// The session anchoring of time bars, if any.
    pub fn session_grid(&self) -> Option<&SessionBarGrid> {
        self.session_grid.as_ref()
    }

    /// Open (microseconds) of the time bar a print at `timestamp_micros` joins, or `None` when
    /// the session policy leaves it out of every bar.
    fn time_bar_start(
        &self,
        timestamp_micros: i64,
        interval_micros: u64,
        anchor_micros: i64,
    ) -> Option<i64> {
        let anchored =
            || aligned_bucket_start(timestamp_micros, interval_micros as i64, anchor_micros);
        let Some(grid) = &self.session_grid else {
            return Some(anchored());
        };
        match grid.bar_open(timestamp_micros.div_euclid(MICROS_PER_SECOND)) {
            Ok(open) => open.map(|open| open * MICROS_PER_SECOND),
            // Windows that cannot be placed on this trading day (a DST transition collapsed
            // one) keep the plain anchor grid for it rather than dropping the print.
            Err(_) => Some(anchored()),
        }
    }

    /// Whole-second open of the time bar a print joins, the bar a big-trades order opening on it
    /// belongs to: session anchoring folds auction and lunch prints into bars they are not
    /// stamped in. `None` when the session policy leaves the print out of every bar. Other
    /// aggregations have no time grid and return the print's second.
    pub(crate) fn print_bar_time(&self, timestamp_micros: i64) -> Option<i64> {
        match self.options.bars {
            FootprintBarAggregation::Time {
                interval_micros,
                anchor_micros,
            } => self
                .time_bar_start(timestamp_micros, interval_micros, anchor_micros)
                .map(|open| open.div_euclid(MICROS_PER_SECOND)),
            FootprintBarAggregation::Trades { .. }
            | FootprintBarAggregation::Volume { .. }
            | FootprintBarAggregation::Range { .. } => {
                Some(timestamp_micros.div_euclid(MICROS_PER_SECOND))
            }
        }
    }

    /// Whether session anchoring leaves this print out of every bar.
    fn excluded_from_bars(&self, timestamp_micros: i64) -> bool {
        match (self.options.bars, &self.session_grid) {
            (
                FootprintBarAggregation::Time {
                    interval_micros,
                    anchor_micros,
                },
                Some(_),
            ) => self
                .time_bar_start(timestamp_micros, interval_micros, anchor_micros)
                .is_none(),
            _ => false,
        }
    }

    /// Key of the time bar a print joins; `None` for other aggregations or an excluded print.
    fn time_bar_key(&self, trade: &FootprintTrade) -> Option<TimeBarKey> {
        let FootprintBarAggregation::Time {
            interval_micros,
            anchor_micros,
        } = self.options.bars
        else {
            return None;
        };
        self.time_bar_start(trade.timestamp_micros, interval_micros, anchor_micros)
            .map(|open| (open, trade.session_id))
    }

    /// Re-derive the tape's last time-bar key after the tape or its bar grid changed wholesale.
    fn refresh_tail_bar_key(&mut self) {
        self.tail_bar_key = if matches!(self.options.bars, FootprintBarAggregation::Time { .. }) {
            self.trades
                .iter()
                .rev()
                .find_map(|stored| self.time_bar_key(&stored.event))
        } else {
            None
        };
    }

    /// Whether `trades`, following a print with bar key `previous`, open two time bars at one
    /// open time. Bar opens never decrease along the canonical order, so only neighbouring prints
    /// that join bars can collide.
    fn bar_times_collide<'a>(
        &self,
        mut previous: Option<TimeBarKey>,
        trades: impl IntoIterator<Item = &'a FootprintTrade>,
    ) -> bool {
        for trade in trades {
            let Some(key) = self.time_bar_key(trade) else {
                continue;
            };
            if previous.is_some_and(|(open, session)| open == key.0 && session != key.1) {
                return true;
            }
            previous = Some(key);
        }
        false
    }

    /// Whether the whole tape, prints the replay clock hides included, opens two time bars at
    /// one open time. Chart presentations key rows by bar open, so a chart stream never holds
    /// such a tape: a clock move could not reveal it without breaking every dependent.
    pub(crate) fn tape_bar_times_collide(&self) -> bool {
        matches!(self.options.bars, FootprintBarAggregation::Time { .. })
            && self.bar_times_collide(None, self.trades.iter().map(|stored| &stored.event))
    }

    /// [`Self::tape_bar_times_collide`] for a tip batch appended to this tape, in O(batch).
    pub(crate) fn tip_bar_times_collide(&self, input: &[FootprintTrade]) -> bool {
        self.bar_times_collide(self.tail_bar_key, input)
    }

    pub fn options(&self) -> FootprintAggregationOptions {
        self.options
    }

    /// Stored bars visible at the replay clock. Levels are at the instrument tick; see
    /// [`Self::presented_bar`] for rows grouped by `ticks_per_row`.
    pub fn bars(&self) -> &[FootprintBar] {
        &self.bars[..self.visible_bar_count()]
    }

    /// Time bars restart at the bucket containing the changed timestamp. Non-time bars can
    /// split equal-timestamp prints across many bars; the last bar starting strictly before
    /// the change may also contain one, even when later bars start at that same timestamp.
    /// Earlier bars cannot contain changed prints because trades are ordered by timestamp.
    fn auction_repair_from(&self, earliest_trade: i64) -> usize {
        if matches!(self.options.bars, FootprintBarAggregation::Time { .. }) {
            self.bars()
                .partition_point(|bar| bar.start_timestamp_micros <= earliest_trade)
                .saturating_sub(1)
        } else {
            self.bars()
                .partition_point(|bar| bar.start_timestamp_micros < earliest_trade)
                .saturating_sub(1)
        }
    }

    /// One stored bar with its levels grouped into `ticks_per_row` display rows.
    pub fn presented_bar<'a>(&self, bar: &'a FootprintBar) -> std::borrow::Cow<'a, FootprintBar> {
        if self.options.ticks_per_row > 1 {
            std::borrow::Cow::Owned(merged_footprint_bar(
                bar,
                self.options.ticks_per_row,
                self.options.imbalance,
                self.options.row_size(),
            ))
        } else {
            std::borrow::Cow::Borrowed(bar)
        }
    }

    /// Leading bars that are final history: their raw trades were released.
    pub fn sealed_bar_count(&self) -> usize {
        self.sealed_bars.min(self.bars().len())
    }

    /// Stored bars hidden because the replay clock sits inside sealed history. Sealed bars
    /// carry no trades, so replay reveals them bar by bar as the clock passes their close.
    fn hidden_bar_count(&self) -> usize {
        self.bars.len() - self.visible_bar_count()
    }

    fn visible_bar_count(&self) -> usize {
        match self.replay_clock_micros {
            Some(clock) if self.bars.len() == self.sealed_bars => self
                .bars
                .partition_point(|bar| bar.end_timestamp_micros <= clock),
            _ => self.bars.len(),
        }
    }

    /// Time of the oldest trade the history covers, sealed or raw. Older backfill pages join in
    /// front of it through [`ChartEngine::prepend_order_flow_history`].
    pub fn history_start_micros(&self) -> Option<i64> {
        if self.sealed_bars > 0 {
            self.sealed_start_micros
        } else {
            self.trades
                .front()
                .map(|trade| trade.event.timestamp_micros)
        }
    }

    /// Whether older history can still be joined in front; false once the oldest bars were
    /// evicted to keep the session or memory budget.
    pub fn accepts_older_history(&self) -> bool {
        !self.history_truncated
    }

    pub(crate) fn released_trades(&self) -> u64 {
        self.released_trades
    }

    pub(crate) fn bar_origin(&self) -> i64 {
        self.bar_origin
    }

    pub(crate) fn tape_epoch(&self) -> u64 {
        self.tape_epoch
    }

    /// Logical bar positions and their full-resolution temporal bounds. This is the boundary that
    /// future non-time axes and drawing rebasing consume; the existing whole-second projection is
    /// deliberately kept separate until that axis is chart-integrated.
    pub fn bar_sequence(&self) -> BarSequence<'_> {
        BarSequence { bars: self.bars() }
    }

    pub fn trades(&self) -> impl ExactSizeIterator<Item = &FootprintTrade> {
        self.trades
            .range(..self.visible_trade_count())
            .map(|trade| &trade.event)
    }

    pub(crate) fn classified_trades(
        &self,
    ) -> impl ExactSizeIterator<Item = (&FootprintTrade, AggressorSide)> {
        self.classified_trades_from(0)
    }

    pub(crate) fn classified_trades_from(
        &self,
        from: usize,
    ) -> impl ExactSizeIterator<Item = (&FootprintTrade, AggressorSide)> {
        let end = self.visible_trade_count();
        self.trades
            .range(from.min(end)..end)
            .map(|trade| (&trade.event, trade.classified_side))
    }

    pub(crate) fn trade_at(&self, index: usize) -> Option<&FootprintTrade> {
        (index < self.visible_trade_count()).then(|| &self.trades[index].event)
    }

    pub fn work_stats(&self) -> FootprintWorkStats {
        self.work
    }

    /// Continuous cumulative delta before the first retained bar (zero until retention evicts).
    pub(crate) fn evicted_cumulative_delta(&self) -> f64 {
        self.rebuild_seed.cumulative_delta
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn replay_clock_micros(&self) -> Option<i64> {
        self.replay_clock_micros
    }

    pub fn set_replay_clock_micros(
        &mut self,
        clock_micros: Option<i64>,
    ) -> Result<ReplaySeekStats, FootprintError> {
        if clock_micros
            .is_some_and(|clock| !(MIN_TIMESTAMP_MICROS..=MAX_TIMESTAMP_MICROS).contains(&clock))
        {
            return Err(FootprintError::InvalidTimestamp { index: 0 });
        }
        let previous_clock_micros = self.replay_clock_micros;
        if previous_clock_micros == clock_micros {
            return Ok(ReplaySeekStats {
                previous_clock_micros,
                clock_micros,
                visible_trades: self.visible_trade_count(),
                rebuilt_trades: 0,
                incremental_trades: 0,
            });
        }
        let previously_visible = self.visible_trade_count();
        self.replay_clock_micros = clock_micros;
        let visible_trades = self.visible_trade_count();
        if previous_clock_micros.is_some() && visible_trades >= previously_visible {
            for index in previously_visible..visible_trades {
                // Classify at reveal, in canonical order: a trade ingested or reconstructed while
                // the clock hid it has no classified predecessor yet, so its stored side is stale.
                let event = self.trades[index].event.clone();
                let side =
                    classify_aggressor(&event, self.last_trade_price, self.last_classified_side);
                self.trades[index].classified_side = side;
                self.apply_classified_trade(&event, side);
                self.maybe_record_replay_checkpoint(index + 1);
            }
            let incremental_trades = visible_trades - previously_visible;
            self.work.incremental_ticks = self
                .work
                .incremental_ticks
                .saturating_add(incremental_trades);
            self.revision = self.revision.saturating_add(1);
            return Ok(ReplaySeekStats {
                previous_clock_micros,
                clock_micros,
                visible_trades,
                rebuilt_trades: 0,
                incremental_trades,
            });
        }
        let rebuilt_trades = self.rebuild_visible_from_checkpoint(visible_trades);
        self.revision = self.revision.saturating_add(1);
        Ok(ReplaySeekStats {
            previous_clock_micros,
            clock_micros,
            visible_trades,
            rebuilt_trades,
            incremental_trades: 0,
        })
    }

    fn visible_trade_count(&self) -> usize {
        self.replay_clock_micros.map_or(self.trades.len(), |clock| {
            self.trades
                .partition_point(|trade| trade.event.timestamp_micros <= clock)
        })
    }

    pub fn reset_work_stats(&mut self) {
        self.work = FootprintWorkStats::default();
    }

    pub fn capacity_bytes(&self) -> usize {
        self.trades.capacity() * core::mem::size_of::<StoredTrade>()
            + self.trade_ids.capacity() * core::mem::size_of::<(u64, usize)>()
            + self.refill_released_ids.capacity() * core::mem::size_of::<u64>()
            + (self.bars.capacity() - self.bars.len()) * core::mem::size_of::<FootprintBar>()
            + self.replay_checkpoints.capacity() * core::mem::size_of::<ReplayCheckpoint>()
            + self.sealed_bytes
            + self.bars[self.sealed_bars..]
                .iter()
                .map(bar_bytes)
                .sum::<usize>()
            + self
                .replay_checkpoints
                .iter()
                .filter_map(|checkpoint| checkpoint.active_bar.as_ref())
                .map(|bar| bar.levels.capacity() * core::mem::size_of::<FootprintLevel>())
                .sum::<usize>()
            + self.session_grid.as_ref().map_or(0, |grid| {
                core::mem::size_of_val(grid.windows()) + grid.exchange_time().capacity_bytes()
            })
    }

    /// Oldest raw bars to seal so the raw tape fits `ceiling` less the retention hysteresis
    /// margin; zero when the tape is within the ceiling. The forming bar is never sealed.
    pub(crate) fn bars_to_seal(&self, ceiling: usize) -> usize {
        if self.trades.len() <= ceiling {
            return 0;
        }
        let budget = ceiling - ceiling / crate::CAP_TRIM_MARGIN_DIVISOR;
        // Trades masked by a replay clock follow the last bar and are never sealed. Prints the
        // session policy excludes join no bar; they leave with the sealed bars they precede (see
        // `raw_trades_of_bars`). Every one of them is counted as retained, so excluded prints
        // before or between the kept bars never carry the raw tape past the ceiling; the cost is
        // sealing up to their count more bars when they precede the sealed ones.
        // ponytail: excluded prints after the newest bar (a closed session's tail, or a tape
        // without bars) cannot leave by sealing a front bar and stay until a later bar opens;
        // bounding them needs eviction of bar-less prints, deferred until a host feeds
        // out-of-session tape at that volume.
        let raw = &self.bars[self.sealed_bars..];
        let mut retained = self.trades.len()
            - raw
                .iter()
                .map(|bar| bar.trade_count as usize)
                .sum::<usize>();
        let mut keep = 0;
        for bar in raw.iter().rev() {
            retained = retained.saturating_add(bar.trade_count as usize);
            if keep > 0 && retained > budget {
                break;
            }
            keep += 1;
        }
        raw.len() - keep
    }

    /// Leading raw-tape trades the oldest `count` raw bars own. Bars consume the visible tape in
    /// canonical order, so they own exactly their counted leading trades; a timestamp cutoff would
    /// misassign a trade sharing its microsecond with the next bar's open, which trade, volume,
    /// and range bars allow. Counting also subsumes a session bar-key cutoff: a session-anchored
    /// bar counts the prints it folds in, including an opening-auction print stamped before its
    /// open. Only prints the session policy excludes join no bar and are not counted; the walk
    /// steps over them and releases those stamped before the first print a later bar keeps.
    pub(crate) fn raw_trades_of_bars(&self, count: usize) -> usize {
        let start = self.sealed_bars.min(self.bars.len());
        let end = start.saturating_add(count).min(self.bars.len());
        let owned = self.bars[start..end]
            .iter()
            .map(|bar| bar.trade_count as usize)
            .sum::<usize>();
        if self.session_grid.is_none() {
            return owned;
        }
        let visible = self.visible_trade_count();
        let mut counted = 0;
        let mut released = 0;
        while released < visible {
            if !self.excluded_from_bars(self.trades[released].event.timestamp_micros) {
                if counted == owned {
                    break;
                }
                counted += 1;
            }
            released += 1;
        }
        released
    }

    /// Newest bars to keep so sealed history spans at most `max_sessions` sessions and the
    /// stream fits `max_bytes` less the retention hysteresis margin, or `None` when nothing has
    /// to go. Only sealed bars are evicted, and none while replay hides part of the history.
    pub(crate) fn bars_within_history_budget(
        &self,
        max_sessions: usize,
        max_bytes: usize,
    ) -> Option<usize> {
        if self.sealed_bars == 0 || self.hidden_bar_count() > 0 {
            return None;
        }
        let mut evict = 0;
        let mut sessions = 0;
        let mut current = None;
        for (index, bar) in self.bars.iter().enumerate().rev() {
            if current != Some(bar.session_id) {
                current = Some(bar.session_id);
                sessions += 1;
                if sessions > max_sessions {
                    evict = index + 1;
                    break;
                }
            }
        }
        let mut evict = evict.min(self.sealed_bars);
        let total = self.capacity_bytes();
        if total > max_bytes {
            let target = max_bytes - max_bytes / crate::CAP_TRIM_MARGIN_DIVISOR;
            // Only level storage is certainly released; the bar slots may stay as capacity.
            let level_bytes =
                |bar: &FootprintBar| bar.levels.capacity() * core::mem::size_of::<FootprintLevel>();
            let mut freed = self.bars[..evict].iter().map(level_bytes).sum::<usize>();
            while evict < self.sealed_bars && total - freed > target {
                freed += level_bytes(&self.bars[evict]);
                evict += 1;
            }
        }
        (evict > 0).then(|| self.bars.len() - evict)
    }

    /// Release the raw trades of the oldest `count` raw bars and keep those bars as final
    /// history. The released trades reach the rebuild seed, their ids leave the index and the
    /// tape's absolute base advances past them, so sealing costs work proportional to the
    /// released trades: no surviving trade is moved or re-indexed and no bar is rebuilt.
    /// Returns the number of trades released.
    pub(crate) fn seal_bars(&mut self, count: usize) -> usize {
        let count = count.min(self.bars.len() - self.sealed_bars);
        if count == 0 {
            return 0;
        }
        let end = self.sealed_bars + count;
        let trade_start = self.raw_trades_of_bars(count);
        if self.sealed_bars == 0 {
            self.sealed_start_micros = self
                .trades
                .front()
                .map(|trade| trade.event.timestamp_micros);
        }
        let mut seed = self.rebuild_seed;
        for index in 0..trade_start {
            let stored = &self.trades[index];
            let trade = &stored.event;
            if let Some(trade_id) = trade.trade_id
                && self.trade_ids.get(&trade_id) == Some(&(self.trade_id_base + index))
            {
                self.trade_ids.remove(&trade_id);
            }
            if let (Some((timestamp, sequence, _)), Some(trade_id)) =
                (self.refill_floor, trade.trade_id)
                && (trade.timestamp_micros, trade.sequence.unwrap_or(u64::MAX))
                    == (timestamp, sequence)
            {
                self.refill_released_ids.insert(trade_id);
            }
            let side = stored.classified_side;
            seed.last_trade_price = Some(trade.price);
            if side != AggressorSide::Unknown {
                seed.last_classified_side = side;
            }
            // An excluded print classifies the prints after it but joins no session delta.
            if self.excluded_from_bars(trade.timestamp_micros) {
                continue;
            }
            if seed.active_session != Some(trade.session_id) {
                seed.active_session = Some(trade.session_id);
                seed.session_delta = 0.0;
            }
            seed.session_delta += match side {
                AggressorSide::Buy => trade.volume,
                AggressorSide::Sell => -trade.volume,
                AggressorSide::Unknown => 0.0,
            };
        }
        self.rebuild_seed = seed;
        let last_released = trade_start
            .checked_sub(1)
            .map(|last| trade_order_key(&self.trades[last]));
        self.trades.drain(..trade_start);
        self.trade_id_base += trade_start;
        // Appends past a power-of-two tape double its capacity. Keep twice the retention
        // hysteresis as headroom so steady trims reuse the allocation instead of holding the
        // doubled one; the shrink is a no-op once the capacity already fits.
        let headroom = self.trades.len() / crate::CAP_TRIM_MARGIN_DIVISOR * 2;
        self.trades.shrink_to(self.trades.len() + headroom);
        self.sealed_bytes += self.bars[self.sealed_bars..end]
            .iter()
            .map(bar_bytes)
            .sum::<usize>();
        self.sealed_bars = end;
        // With the raw tape released entirely, the last released trade bounds sealed history.
        self.sealed_floor = self
            .trades
            .front()
            .map(trade_order_key)
            .or(last_released)
            .or(self.sealed_floor);
        // Checkpoints inside the raw suffix stay valid once rebased onto the new tape start, so
        // later late events and backward replay seeks keep their bounded restart points.
        self.replay_checkpoints.retain_mut(|checkpoint| {
            if checkpoint.trade_count < trade_start {
                return false;
            }
            checkpoint.trade_count -= trade_start;
            true
        });
        self.released_trades = self.released_trades.saturating_add(trade_start as u64);
        self.revision = self.revision.saturating_add(1);
        trade_start
    }

    /// Drop the oldest `count` sealed bars. The raw tape and session state are untouched; their
    /// final deltas join the rebuild seed, so continuous cumulative delta resumes exactly as a
    /// fold over the full history would.
    pub(crate) fn evict_sealed_bars(&mut self, count: usize) {
        let count = count.min(self.sealed_bars);
        if count == 0 {
            return;
        }
        for bar in &self.bars[..count] {
            self.rebuild_seed.cumulative_delta += bar.delta;
        }
        self.sealed_bytes -= self.bars[..count].iter().map(bar_bytes).sum::<usize>();
        self.sealed_bars -= count;
        self.bars.drain(..count);
        let headroom = self.bars.len() / crate::CAP_TRIM_MARGIN_DIVISOR * 2;
        self.bars.shrink_to(self.bars.len() + headroom);
        for (index, bar) in self.bars.iter_mut().enumerate() {
            bar.logical_index = index as u64;
        }
        let evicted = count as u64;
        self.replay_checkpoints.retain_mut(|checkpoint| {
            if checkpoint.bars_len < count {
                return false;
            }
            checkpoint.bars_len -= count;
            if let Some(active) = &mut checkpoint.active_bar {
                active.logical_index = active.logical_index.saturating_sub(evicted);
            }
            true
        });
        self.bar_origin = self.bar_origin.saturating_add(count as i64);
        // The released trades of the remaining sealed bars are gone, so their opening time is
        // the closest bound left on the history start.
        self.sealed_start_micros = self.bars.first().map(|bar| bar.start_timestamp_micros);
        self.history_truncated = true;
        self.clear_refill_floor();
        self.revision = self.revision.saturating_add(1);
    }

    /// Change the display rows and imbalance rules. Levels are stored per tick, so only the
    /// derived imbalance flags are recomputed, in place, without replaying any trade.
    pub(crate) fn set_row_options(
        &mut self,
        ticks_per_row: u32,
        imbalance: FootprintImbalanceOptions,
    ) -> Result<bool, FootprintError> {
        let next = FootprintAggregationOptions {
            ticks_per_row,
            imbalance,
            ..self.options
        };
        validate_options(next)?;
        if next == self.options {
            return Ok(false);
        }
        let imbalance_changed = next.imbalance != self.options.imbalance;
        self.options = next;
        if imbalance_changed {
            let bars = self.bars.iter_mut().chain(
                self.replay_checkpoints
                    .iter_mut()
                    .filter_map(|checkpoint| checkpoint.active_bar.as_mut()),
            );
            for bar in bars {
                recompute_bar_derived(bar, imbalance);
            }
        }
        self.revision = self.revision.saturating_add(1);
        Ok(true)
    }

    /// Join an older tape page to the front of the history. While no bar is sealed the page
    /// joins the raw tape, which is rebuilt exactly. Once history is sealed, the page is
    /// aggregated on its own, on the stream's session grid, and its bars become sealed history:
    /// the boundary time bar is joined exactly, the first session's cumulative delta is carried
    /// forward, and an unknown-aggressor print at the old front keeps the classification it
    /// already had. A page joined to sealed history is returned with its own trades and bars, for
    /// studies that derive from trades. A page whose bars would open two time bars at one open
    /// time is refused with `ProjectionTimeCollision` before anything changes.
    pub(crate) fn prepend_history(
        &mut self,
        input: Vec<FootprintTrade>,
    ) -> Result<(HistoryPrefixStats, Option<FootprintAggregator>), FootprintError> {
        validate_trade_batch(self.options, &input)?;
        let total = input.len();
        if self.history_truncated {
            let stats = HistoryPrefixStats {
                accepted_trades: 0,
                skipped_trades: total,
                history_full: true,
            };
            return Ok((stats, None));
        }
        let start = self.history_start_micros();
        // The refill floor bounds the prints released before a grid change dropped their bars
        // (see `discard_sealed_history`). On trade, volume and range bars sealing can release a
        // print of the raw tape's first microsecond: one that sorts before the floor by
        // sequence, or ties it and carries a trade id (deduplicated below against the raw tape
        // and the tied prints a later seal released), refills before the tape, as one load of
        // the whole tape orders it. Live appends that seal again before the refill arrives move
        // the history start to that microsecond but leave the floor, so both branches below
        // receive the released prints.
        let refill_floor = self.refill_floor;
        let prefix = input
            .into_iter()
            .filter(|trade| {
                (start.is_none_or(|start| trade.timestamp_micros < start)
                    || refill_floor.is_some_and(|(timestamp, sequence, _)| {
                        let key = (trade.timestamp_micros, trade.sequence.unwrap_or(u64::MAX));
                        key < (timestamp, sequence)
                            || key == (timestamp, sequence) && trade.trade_id.is_some()
                    }))
                    && trade.trade_id.is_none_or(|trade_id| {
                        !self.trade_ids.contains_key(&trade_id)
                            && !self.refill_released_ids.contains(&trade_id)
                    })
            })
            .collect::<Vec<_>>();
        let mut stats = HistoryPrefixStats {
            accepted_trades: prefix.len(),
            skipped_trades: total - prefix.len(),
            history_full: false,
        };
        if prefix.is_empty() {
            return Ok((stats, None));
        }
        let mut sealed_page = None;
        if self.sealed_bars == 0 {
            let base = self.next_input_order;
            let mut trades = prefix
                .into_iter()
                .enumerate()
                .map(|(index, event)| StoredTrade {
                    event,
                    input_order: base.saturating_add(index as u64),
                    classified_side: AggressorSide::Unknown,
                })
                .collect::<Vec<_>>();
            trades.sort_by_key(trade_order_key);
            // The page sorts strictly before the tape, so only the page and its joint with the
            // tape's first print that joins a time bar can collide: O(page), checked before
            // anything changes.
            let first_keyed = self
                .trades
                .iter()
                .map(|stored| &stored.event)
                .find(|event| self.time_bar_key(event).is_some());
            if self.bar_times_collide(
                None,
                trades.iter().map(|stored| &stored.event).chain(first_keyed),
            ) {
                return Err(FootprintError::ProjectionTimeCollision);
            }
            let mut tape = VecDeque::from(trades);
            tape.append(&mut self.trades);
            // The page sorts before the tape by time and sequence; renumbering the arrival order
            // along the joined tape keeps a refilled print that ties the old front on both
            // before it, so later events merge in the order one load gives.
            for (index, stored) in tape.iter_mut().enumerate() {
                stored.input_order = base.saturating_add(index as u64);
            }
            self.next_input_order = base.saturating_add(tape.len() as u64);
            self.trades = tape;
            self.reindex_trade_ids();
            // The joined tape starts the history, as one load of it would. A floor left by sealed
            // bars a grid change dropped (see `discard_sealed_history`) bounded history the page
            // now supplies, so it no longer turns corrections or windows inside the page away.
            self.sealed_floor = None;
            self.clear_refill_floor();
            self.rebuild();
            // An empty tape had no tail bar key, so the page's last bar must bound later tips.
            self.refresh_tail_bar_key();
        } else {
            let (dropped, page) = self.prepend_sealed_history(prefix)?;
            stats.accepted_trades -= dropped;
            stats.skipped_trades += dropped;
            sealed_page = Some(page);
            // The page now starts the history, so older pages are bounded by its start alone.
            self.clear_refill_floor();
        }
        self.revision = self.revision.saturating_add(1);
        Ok((stats, sealed_page))
    }

    /// Returns the prefix trades dropped because their bar collides with a different session,
    /// and the page aggregated on its own.
    fn prepend_sealed_history(
        &mut self,
        prefix: Vec<FootprintTrade>,
    ) -> Result<(usize, FootprintAggregator), FootprintError> {
        let mut page = FootprintAggregator::new(self.options)?;
        // The page lands on the stream's own bar grid, so its bars and the boundary bar it joins
        // are the bars one aggregation of the whole tape builds.
        page.session_grid = self.session_grid.clone();
        page.set_trades(prefix)?;
        if page.tape_bar_times_collide() {
            return Err(FootprintError::ProjectionTimeCollision);
        }
        let page_start = page
            .trades
            .front()
            .map(|trade| trade.event.timestamp_micros);
        let mut bars = page.bars.clone();
        let mut dropped = 0;
        if let Some(session) = page.active_session {
            self.shift_leading_session_delta(session, page.session_delta);
        }
        let time_bars = matches!(self.options.bars, FootprintBarAggregation::Time { .. });
        if time_bars
            && let Some(first) = self.bars.first()
            && bars
                .last()
                .is_some_and(|last| last.start_timestamp_micros == first.start_timestamp_micros)
            && let Some(last) = bars.pop()
        {
            if last.session_id == first.session_id {
                let before = bar_bytes(first);
                let joined = joined_footprint_bar(&last, first, self.options.imbalance);
                self.sealed_bytes = self.sealed_bytes - before + bar_bytes(&joined);
                self.bars[0] = joined;
            } else {
                dropped = last.trade_count as usize;
            }
        }
        let added = bars.len();
        self.sealed_bytes += bars.iter().map(bar_bytes).sum::<usize>();
        bars.append(&mut self.bars);
        self.bars = bars;
        for (index, bar) in self.bars.iter_mut().enumerate() {
            bar.logical_index = index as u64;
        }
        for checkpoint in &mut self.replay_checkpoints {
            checkpoint.bars_len += added;
            if let Some(active) = &mut checkpoint.active_bar {
                active.logical_index = active.logical_index.saturating_add(added as u64);
            }
        }
        self.sealed_bars += added;
        self.bar_origin = self.bar_origin.saturating_sub(added as i64);
        self.sealed_start_micros = page_start;
        Ok((dropped, page))
    }

    /// Carry an older page's closing session delta into the leading bars of the same session,
    /// and into every replay state positioned inside that run.
    fn shift_leading_session_delta(&mut self, session: Option<u64>, shift: f64) {
        if shift == 0.0
            || self
                .bars
                .first()
                .is_none_or(|bar| bar.session_id != session)
        {
            return;
        }
        let run = self
            .bars
            .iter()
            .take_while(|bar| bar.session_id == session)
            .count();
        for bar in &mut self.bars[..run] {
            bar.session_delta += shift;
        }
        if self.sealed_bars <= run && self.rebuild_seed.active_session == Some(session) {
            self.rebuild_seed.session_delta += shift;
        }
        for checkpoint in &mut self.replay_checkpoints {
            if checkpoint.bars_len <= run && checkpoint.active_session == Some(session) {
                checkpoint.session_delta += shift;
                if let Some(active) = &mut checkpoint.active_bar {
                    active.session_delta += shift;
                }
            }
        }
        if self.bars.len() <= run && self.active_session == Some(session) {
            self.session_delta += shift;
        }
    }

    /// Atomically replace the tape, sort it by feed order, and reconstruct every derived bar.
    pub fn set_trades(&mut self, input: Vec<FootprintTrade>) -> Result<(), FootprintError> {
        validate_trade_batch(self.options, &input)?;
        let base = self.next_input_order;
        let mut trades = input
            .into_iter()
            .enumerate()
            .map(|(index, event)| StoredTrade {
                event,
                input_order: base.saturating_add(index as u64),
                classified_side: AggressorSide::Unknown,
            })
            .collect::<Vec<_>>();
        trades.sort_by_key(trade_order_key);
        self.next_input_order = base.saturating_add(trades.len() as u64);
        self.trades = VecDeque::from(trades);
        self.reindex_trade_ids();
        self.rebuild_seed = RebuildSeed::default();
        self.sealed_bars = 0;
        self.sealed_bytes = 0;
        self.sealed_floor = None;
        self.sealed_start_micros = None;
        self.history_truncated = false;
        self.clear_refill_floor();
        self.released_trades = 0;
        self.bar_origin = 0;
        self.tape_epoch = self.tape_epoch.wrapping_add(1);
        self.rebuild();
        self.refresh_tail_bar_key();
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }

    /// Append a live event in O(current-bar levels) when it is at the tape tip. A late event or a
    /// provider correction is inserted in canonical order and triggers deterministic historical
    /// reconstruction instead of corrupting Max/Min Delta paths.
    pub fn update_trade(
        &mut self,
        event: FootprintTrade,
    ) -> Result<FootprintUpdateKind, FootprintError> {
        self.update_trades(vec![event])
    }

    /// Atomically apply a provider batch. Monotonic new events retain the incremental path;
    /// corrections or late events merge into the final canonical tape and rebuild exactly once.
    pub fn update_trades(
        &mut self,
        input: Vec<FootprintTrade>,
    ) -> Result<FootprintUpdateKind, FootprintError> {
        self.apply_trades(input).map(|(kind, _)| kind)
    }

    /// [`Self::update_trades`] plus the first bar index whose derived state may have changed.
    /// A late event or correction rebuilds from the newest replay checkpoint preceding it, so
    /// its cost is bounded by its distance from the tip rather than by the retained tape.
    pub(crate) fn apply_trades(
        &mut self,
        input: Vec<FootprintTrade>,
    ) -> Result<(FootprintUpdateKind, usize), FootprintError> {
        self.merge_trades(input, false)
    }

    /// [`Self::apply_trades`]. With `refuse_bar_time_collisions`, as for a chart stream whose
    /// presentations key time-bar rows by bar open, a late merge that would open two time bars at
    /// one open time is refused with `ProjectionTimeCollision` before anything changes.
    pub(crate) fn merge_trades(
        &mut self,
        input: Vec<FootprintTrade>,
        refuse_bar_time_collisions: bool,
    ) -> Result<(FootprintUpdateKind, usize), FootprintError> {
        validate_trade_batch(self.options, &input)?;
        let tip_bar = self.bars.len().saturating_sub(1);
        if input.is_empty() {
            return Ok((FootprintUpdateKind::Tip, tip_bar));
        }
        if self.batch_is_tip(&input) {
            for event in input {
                let classified_side =
                    classify_aggressor(&event, self.last_trade_price, self.last_classified_side);
                let stored = StoredTrade {
                    event,
                    input_order: self.next_input_order,
                    classified_side,
                };
                self.next_input_order = self.next_input_order.saturating_add(1);
                if self
                    .replay_clock_micros
                    .is_none_or(|clock| stored.event.timestamp_micros <= clock)
                {
                    self.apply_classified_trade(&stored.event, stored.classified_side);
                    self.maybe_record_replay_checkpoint(self.trades.len() + 1);
                }
                if let Some(trade_id) = stored.event.trade_id {
                    self.trade_ids
                        .insert(trade_id, self.trade_id_base + self.trades.len());
                }
                if let Some(key) = self.time_bar_key(&stored.event) {
                    self.tail_bar_key = Some(key);
                }
                self.trades.push_back(stored);
                self.work.incremental_ticks += 1;
            }
            self.revision = self.revision.saturating_add(1);
            return Ok((FootprintUpdateKind::Tip, tip_bar));
        }

        // Every retained trade before the earliest canonical position touched by the batch keeps
        // its index and order, so only the tail from there is re-sorted and re-indexed.
        let mut first_changed = self.trades.len();
        let mut next_input_order = self.next_input_order;
        let mut stored = Vec::with_capacity(input.len());
        for event in input {
            let replaced = event
                .trade_id
                .and_then(|trade_id| self.trade_ids.get(&trade_id))
                .map(|&position| position - self.trade_id_base);
            let input_order = replaced.map_or(next_input_order, |position| {
                self.trades[position].input_order
            });
            let key = (
                event.timestamp_micros,
                event.sequence.unwrap_or(u64::MAX),
                input_order,
            );
            // Sealed bars are final: a print or correction that sorts into them is not applied.
            if self.sealed_floor.is_some_and(|floor| key < floor) {
                self.work.skipped_sealed_trades += 1;
                continue;
            }
            match replaced {
                Some(position) => first_changed = first_changed.min(position),
                None => next_input_order = next_input_order.saturating_add(1),
            }
            first_changed = first_changed.min(
                self.trades
                    .partition_point(|trade| trade_order_key(trade) < key),
            );
            stored.push((
                replaced,
                StoredTrade {
                    event,
                    input_order,
                    classified_side: AggressorSide::Unknown,
                },
            ));
        }
        if stored.is_empty() {
            return Ok((FootprintUpdateKind::Historical, tip_bar));
        }
        // Merge the changed suffix aside and check it before anything changes, so a bar-time
        // collision anywhere in it (hidden prints included) is refused atomically in O(suffix).
        let mut suffix = self
            .trades
            .range(first_changed..)
            .cloned()
            .collect::<Vec<_>>();
        for (replaced, trade) in stored {
            match replaced {
                Some(position) => suffix[position - first_changed] = trade,
                None => suffix.push(trade),
            }
        }
        suffix.sort_by_key(trade_order_key);
        if refuse_bar_time_collisions && self.suffix_bar_times_collide(first_changed, &suffix) {
            return Err(FootprintError::ProjectionTimeCollision);
        }
        self.replace_tape_suffix(first_changed, suffix);
        self.next_input_order = next_input_order;
        let first_bar = self.rebuild_from(first_changed);
        self.refresh_tail_bar_key();
        self.revision = self.revision.saturating_add(1);
        Ok((FootprintUpdateKind::Historical, first_bar))
    }

    /// Whether `suffix`, replacing the raw tape from position `from`, opens two time bars at one
    /// open time. Bar opens never decrease along the canonical order, so the suffix is checked
    /// against the last bar key before it: a retained print's, or the newest sealed bar's.
    fn suffix_bar_times_collide(&self, from: usize, suffix: &[StoredTrade]) -> bool {
        if !matches!(self.options.bars, FootprintBarAggregation::Time { .. }) {
            return false;
        }
        let previous = self
            .trades
            .range(..from)
            .rev()
            .find_map(|stored| self.time_bar_key(&stored.event))
            .or_else(|| {
                self.bars[..self.sealed_bars]
                    .last()
                    .map(|bar| (bar.start_timestamp_micros, bar.session_id))
            });
        self.bar_times_collide(previous, suffix.iter().map(|stored| &stored.event))
    }

    /// Replace the raw tape from position `from` with `suffix`, re-indexing only the trade ids
    /// the old and new suffixes hold.
    fn replace_tape_suffix(&mut self, from: usize, suffix: Vec<StoredTrade>) {
        for (index, stored) in self.trades.range(from..).enumerate() {
            if let Some(trade_id) = stored.event.trade_id
                && self.trade_ids.get(&trade_id) == Some(&(self.trade_id_base + from + index))
            {
                self.trade_ids.remove(&trade_id);
            }
        }
        self.trades.truncate(from);
        self.trades.extend(suffix);
        for (index, stored) in self.trades.range(from..).enumerate() {
            if let Some(trade_id) = stored.event.trade_id {
                self.trade_ids
                    .insert(trade_id, self.trade_id_base + from + index);
            }
        }
    }

    /// Replace every retained print at or after the batch's earliest timestamp with the batch and
    /// keep everything older. Returns the first bar index whose derived state may have changed,
    /// or `None` for an empty batch, which covers no span. Prints sorting into sealed history are
    /// skipped, as for [`Self::update_trades`].
    pub(crate) fn replace_trades_from_window(
        &mut self,
        input: Vec<FootprintTrade>,
    ) -> Result<Option<usize>, FootprintError> {
        validate_trade_batch(self.options, &input)?;
        let Some(from) = input.iter().map(|trade| trade.timestamp_micros).min() else {
            return Ok(None);
        };
        let cut = self
            .trades
            .partition_point(|trade| trade.event.timestamp_micros < from);
        let mut next_input_order = self.next_input_order;
        let mut window = Vec::with_capacity(input.len());
        for event in input {
            let input_order = next_input_order;
            next_input_order = next_input_order.saturating_add(1);
            let key = (
                event.timestamp_micros,
                event.sequence.unwrap_or(u64::MAX),
                input_order,
            );
            if self.sealed_floor.is_some_and(|floor| key < floor) {
                self.work.skipped_sealed_trades += 1;
                continue;
            }
            window.push(StoredTrade {
                event,
                input_order,
                classified_side: AggressorSide::Unknown,
            });
        }
        window.sort_by_key(trade_order_key);
        if self.suffix_bar_times_collide(cut, &window) {
            return Err(FootprintError::ProjectionTimeCollision);
        }
        self.replace_tape_suffix(cut, window);
        self.next_input_order = next_input_order;
        let first_bar = self.rebuild_from(cut);
        self.refresh_tail_bar_key();
        self.revision = self.revision.saturating_add(1);
        Ok(Some(first_bar))
    }

    pub(crate) fn batch_is_tip(&self, input: &[FootprintTrade]) -> bool {
        let mut previous = self.trades.back().map(trade_order_key);
        for (index, event) in input.iter().enumerate() {
            if event
                .trade_id
                .is_some_and(|trade_id| self.trade_ids.contains_key(&trade_id))
            {
                return false;
            }
            let key = (
                event.timestamp_micros,
                event.sequence.unwrap_or(u64::MAX),
                self.next_input_order.saturating_add(index as u64),
            );
            if previous.is_some_and(|previous| previous > key) {
                return false;
            }
            previous = Some(key);
        }
        true
    }

    /// Whether applying `input` can change what the replay clock reveals: an event at or before
    /// the clock, or a correction of a trade the clock already revealed, which a later timestamp
    /// hides. Every other batch changes only future source truth.
    pub(crate) fn batch_changes_visible_tape(&self, input: &[FootprintTrade]) -> bool {
        let Some(clock) = self.replay_clock_micros else {
            return true;
        };
        input.iter().any(|event| {
            event.timestamp_micros <= clock
                || event
                    .trade_id
                    .and_then(|trade_id| self.stored_trade_timestamp(trade_id))
                    .is_some_and(|timestamp| timestamp <= clock)
        })
    }

    /// Timestamp of the retained print a provider `trade_id` names. Positions in `trade_ids` are
    /// absolute, so the deque index subtracts `trade_id_base` (advanced by retention).
    fn stored_trade_timestamp(&self, trade_id: u64) -> Option<i64> {
        let position = self.trade_ids.get(&trade_id)?;
        let trade = self.trades.get(position.checked_sub(self.trade_id_base)?)?;
        Some(trade.event.timestamp_micros)
    }

    /// This stream re-aggregated under `options`: the same retained tape (prints the replay clock
    /// hides included), replay clock, retention seed, and session anchoring, re-placed on the new
    /// bar interval. Sessions need whole-second time bars, as [`Self::set_sessions`] does. Sealed
    /// bars were built on the grid being replaced and their trades are released, so they are
    /// dropped (see [`Self::discard_sealed_history`]).
    fn with_options(&self, options: FootprintAggregationOptions) -> Result<Self, FootprintError> {
        validate_options(options)?;
        let mut next = self.clone();
        next.discard_sealed_history();
        next.options = options;
        next.session_grid = None;
        match &self.session_grid {
            Some(grid) => {
                let sessions = TradeSessionOptions {
                    windows: grid.windows().to_vec(),
                    outside: grid.policy(),
                };
                next.set_sessions(Some(&sessions), grid.exchange_time())?;
            }
            None => {
                next.rebuild();
                next.refresh_tail_bar_key();
                next.revision = next.revision.saturating_add(1);
            }
        }
        Ok(next)
    }

    /// Drop every sealed bar before the bar grid changes. Sealed bars keep no trades, so they
    /// cannot be re-placed on a new session grid, exchange time, interval, or tick size; keeping
    /// them would leave one stream with two bar grids. The history then starts at the raw tape,
    /// as a fresh load of it would, and a host refills it on the new grid through
    /// `prepend_order_flow_history`, so nothing of the released trades is seeded: neither their
    /// session delta and tick-rule state nor any cumulative delta. History the budgets already
    /// evicted cannot be refilled: there the sealed bars leave as evicted history, their deltas
    /// joining the rebuild seed. A refillable discard records the sealed floor as the refill
    /// floor, which bounds the refill until its first page joins even if bars are sealed again
    /// first. Either way the bars are re-placed, so tape-replaying dependents start over.
    fn discard_sealed_history(&mut self) {
        let refillable = !self.history_truncated;
        self.evict_sealed_bars(self.sealed_bars);
        if refillable {
            self.history_truncated = false;
            self.rebuild_seed = RebuildSeed::default();
            // The current floor bounds every print released so far, including any a seal after
            // an earlier grid change released, so it replaces an earlier refill floor.
            self.refill_floor = self.sealed_floor;
        }
        self.tape_epoch = self.tape_epoch.wrapping_add(1);
    }

    fn clear_refill_floor(&mut self) {
        self.refill_floor = None;
        self.refill_released_ids = HashSet::new();
    }

    pub fn bar(&self, index: usize) -> Option<&FootprintBar> {
        self.bars().get(index)
    }

    /// Re-derive every raw bar from the retained tape; sealed history is final and kept.
    fn rebuild(&mut self) {
        self.bars.truncate(self.sealed_bars);
        self.replay_checkpoints.clear();
        self.last_trade_price = self.rebuild_seed.last_trade_price;
        self.last_classified_side = self.rebuild_seed.last_classified_side;
        self.active_session = self.rebuild_seed.active_session;
        self.session_delta = self.rebuild_seed.session_delta;
        self.reclassify_from(0);
    }

    /// Rebuild the derived bars after the trades from `first_changed` changed, restoring the
    /// newest replay checkpoint that precedes them. Returns the first bar index that was rebuilt.
    /// The result is identical to [`Self::rebuild`]; only the unchanged prefix is skipped.
    fn rebuild_from(&mut self, first_changed: usize) -> usize {
        let Some(checkpoint) = self
            .replay_checkpoints
            .iter()
            .rev()
            .find(|checkpoint| {
                checkpoint.trade_count <= first_changed && checkpoint.bars_len <= self.bars.len()
            })
            .cloned()
        else {
            self.rebuild();
            return self.sealed_bars;
        };
        self.replay_checkpoints
            .retain(|retained| retained.trade_count <= checkpoint.trade_count);
        self.restore_checkpoint(&checkpoint);
        self.reclassify_from(checkpoint.trade_count);
        checkpoint.bars_len.saturating_sub(1).max(self.sealed_bars)
    }

    fn restore_checkpoint(&mut self, checkpoint: &ReplayCheckpoint) {
        self.bars.truncate(checkpoint.bars_len);
        // A checkpoint at the seal boundary points at a sealed bar, which is already final.
        if checkpoint.bars_len > self.sealed_bars
            && let (Some(active), Some(current)) =
                (checkpoint.active_bar.clone(), self.bars.last_mut())
        {
            *current = active;
        }
        self.last_trade_price = checkpoint.last_trade_price;
        self.last_classified_side = checkpoint.last_classified_side;
        self.active_session = checkpoint.active_session;
        self.session_delta = checkpoint.session_delta;
    }

    /// Classify and aggregate the visible trades from `start` onto the current derived state.
    fn reclassify_from(&mut self, start: usize) {
        let trade_count = self.visible_trade_count();
        for index in start..trade_count {
            // The event has no heap-owned fields. Copying one value avoids retaining a second tape
            // while mutable derived state is rebuilt.
            let trade = self.trades[index].event.clone();
            let side = classify_aggressor(&trade, self.last_trade_price, self.last_classified_side);
            self.trades[index].classified_side = side;
            self.apply_classified_trade(&trade, side);
            self.maybe_record_replay_checkpoint(index + 1);
        }
        self.work.historical_rebuilds += 1;
        self.work.rebuilt_ticks += trade_count.saturating_sub(start);
    }

    fn maybe_record_replay_checkpoint(&mut self, trade_count: usize) {
        if trade_count == 0 || !trade_count.is_multiple_of(REPLAY_CHECKPOINT_INTERVAL) {
            return;
        }
        if self
            .replay_checkpoints
            .iter()
            .any(|checkpoint| checkpoint.trade_count == trade_count)
        {
            return;
        }
        if self.replay_checkpoints.len() == MAX_REPLAY_CHECKPOINTS {
            self.replay_checkpoints.remove(0);
        }
        self.replay_checkpoints.push(ReplayCheckpoint {
            trade_count,
            bars_len: self.bars.len(),
            active_bar: self.bars.last().cloned(),
            last_trade_price: self.last_trade_price,
            last_classified_side: self.last_classified_side,
            active_session: self.active_session,
            session_delta: self.session_delta,
        });
    }

    fn rebuild_visible_from_checkpoint(&mut self, visible_trades: usize) -> usize {
        let checkpoint = self
            .replay_checkpoints
            .iter()
            .rev()
            .find(|checkpoint| {
                checkpoint.trade_count <= visible_trades && checkpoint.bars_len <= self.bars.len()
            })
            .cloned();
        let start = if let Some(checkpoint) = checkpoint {
            self.restore_checkpoint(&checkpoint);
            checkpoint.trade_count
        } else {
            self.bars.truncate(self.sealed_bars);
            self.last_trade_price = self.rebuild_seed.last_trade_price;
            self.last_classified_side = self.rebuild_seed.last_classified_side;
            self.active_session = self.rebuild_seed.active_session;
            self.session_delta = self.rebuild_seed.session_delta;
            0
        };
        for index in start..visible_trades {
            let stored = self.trades[index].clone();
            self.apply_classified_trade(&stored.event, stored.classified_side);
            self.maybe_record_replay_checkpoint(index + 1);
        }
        let rebuilt_trades = visible_trades.saturating_sub(start);
        self.work.historical_rebuilds = self.work.historical_rebuilds.saturating_add(1);
        self.work.rebuilt_ticks = self.work.rebuilt_ticks.saturating_add(rebuilt_trades);
        rebuilt_trades
    }

    fn reindex_trade_ids(&mut self) {
        self.trade_id_base = 0;
        self.trade_ids.clear();
        self.trade_ids.reserve(
            self.trades
                .iter()
                .filter(|trade| trade.event.trade_id.is_some())
                .count(),
        );
        for (index, trade) in self.trades.iter().enumerate() {
            if let Some(trade_id) = trade.event.trade_id {
                self.trade_ids.insert(trade_id, index);
            }
        }
    }

    fn apply_classified_trade(&mut self, trade: &FootprintTrade, side: AggressorSide) {
        let time_start = match self.options.bars {
            FootprintBarAggregation::Time {
                interval_micros,
                anchor_micros,
            } => {
                let start =
                    self.time_bar_start(trade.timestamp_micros, interval_micros, anchor_micros);
                if start.is_none() {
                    // Left out of every bar by the session policy; it still classifies the
                    // prints after it.
                    self.last_trade_price = Some(trade.price);
                    if side != AggressorSide::Unknown {
                        self.last_classified_side = side;
                    }
                    return;
                }
                start
            }
            FootprintBarAggregation::Trades { .. }
            | FootprintBarAggregation::Volume { .. }
            | FootprintBarAggregation::Range { .. } => None,
        };
        if self.active_session != Some(trade.session_id) {
            self.active_session = Some(trade.session_id);
            self.session_delta = 0.0;
        }
        self.last_trade_price = Some(trade.price);
        if side != AggressorSide::Unknown {
            self.last_classified_side = side;
        }
        let level = price_level(trade.price, self.options.tick_size);
        // The first raw trade always opens a bar: it did so when the bar before it was sealed.
        let start_new = self.bars.len() == self.sealed_bars
            || self
                .bars
                .last()
                .is_none_or(|bar| self.must_start_bar(bar, trade, time_start));
        if start_new {
            // Time bars open at their (session-anchored) bucket; other bars at their first print.
            let start = time_start.unwrap_or(trade.timestamp_micros);
            self.bars.push(FootprintBar {
                logical_index: self
                    .bars
                    .last()
                    .map_or(0, |bar| bar.logical_index.saturating_add(1)),
                start_timestamp_micros: start,
                end_timestamp_micros: trade.timestamp_micros,
                session_id: trade.session_id,
                open: trade.price,
                high: trade.price,
                low: trade.price,
                close: trade.price,
                bid_volume: 0.0,
                ask_volume: 0.0,
                unknown_volume: 0.0,
                total_volume: 0.0,
                delta: 0.0,
                delta_percent: 0.0,
                max_delta: 0.0,
                min_delta: 0.0,
                session_delta: self.session_delta,
                trade_count: 0,
                poc_level: level,
                poc_price: trade.price,
                levels: Vec::new(),
            });
        }
        let bar = self.bars.last_mut().expect("a trade always owns a bar");
        bar.end_timestamp_micros = trade.timestamp_micros;
        bar.high = bar.high.max(trade.price);
        bar.low = bar.low.min(trade.price);
        bar.close = trade.price;
        bar.trade_count = bar.trade_count.saturating_add(1);
        bar.total_volume += trade.volume;
        let delta = match side {
            AggressorSide::Buy => {
                bar.ask_volume += trade.volume;
                trade.volume
            }
            AggressorSide::Sell => {
                bar.bid_volume += trade.volume;
                -trade.volume
            }
            AggressorSide::Unknown => {
                bar.unknown_volume += trade.volume;
                0.0
            }
        };
        bar.delta += delta;
        bar.max_delta = bar.max_delta.max(bar.delta);
        bar.min_delta = bar.min_delta.min(bar.delta);
        self.session_delta += delta;
        bar.session_delta = self.session_delta;

        let level_position = bar
            .levels
            .binary_search_by_key(&level, |entry| entry.level)
            .unwrap_or_else(|position| {
                bar.levels.insert(
                    position,
                    FootprintLevel {
                        level,
                        price: level as f64 * self.options.tick_size,
                        ..FootprintLevel::default()
                    },
                );
                position
            });
        let cell = &mut bar.levels[level_position];
        cell.total_volume += trade.volume;
        match side {
            AggressorSide::Buy => cell.ask_volume += trade.volume,
            AggressorSide::Sell => cell.bid_volume += trade.volume,
            AggressorSide::Unknown => cell.unknown_volume += trade.volume,
        }
        cell.delta = cell.ask_volume - cell.bid_volume;
        recompute_bar_derived(bar, self.options.imbalance);
    }

    fn must_start_bar(
        &self,
        bar: &FootprintBar,
        trade: &FootprintTrade,
        time_start: Option<i64>,
    ) -> bool {
        if bar.session_id != trade.session_id {
            return true;
        }
        match self.options.bars {
            FootprintBarAggregation::Time { .. } => time_start != Some(bar.start_timestamp_micros),
            FootprintBarAggregation::Trades { trades_per_bar } => bar.trade_count >= trades_per_bar,
            FootprintBarAggregation::Volume { volume_per_bar } => {
                bar.total_volume >= volume_per_bar
            }
            FootprintBarAggregation::Range { range_ticks } => {
                price_level(bar.high, self.options.tick_size)
                    .saturating_sub(price_level(bar.low, self.options.tick_size))
                    >= i64::from(range_ticks)
            }
        }
    }
}

impl ChartEngine {
    pub(crate) fn apply_indicator_chrome_to_trade_studies(
        &mut self,
        options: crate::IndicatorChromeOptions,
    ) -> bool {
        let ids = self
            .trade_dependents
            .values()
            .flatten()
            .map(|dependent| dependent.series_id)
            .collect::<Vec<_>>();
        let mut changed = false;
        for id in ids {
            if let Some(series) = self.series_entry_mut(id) {
                changed |= series.title_visible != options.name_labels_visible
                    || series.last_value_visible != options.value_labels_visible
                    || series.price_line_visible != options.price_lines_visible;
                series.title_visible = options.name_labels_visible;
                series.last_value_visible = options.value_labels_visible;
                series.price_line_visible = options.price_lines_visible;
            }
        }
        changed
    }

    pub fn replay_clock_micros(&self) -> Option<i64> {
        self.replay_clock_micros
    }

    pub(crate) fn replay_cutoff_seconds(&self) -> Option<i64> {
        self.replay_clock_micros
            .map(|clock| clock.div_euclid(MICROS_PER_SECOND))
    }

    pub(crate) fn replay_time_is_visible(&self, time: i64) -> bool {
        self.replay_cutoff_seconds()
            .is_none_or(|cutoff| time <= cutoff)
    }

    /// Apply one host clock to the chart's ordinary time-domain rows and every canonical trade
    /// stream. Source rows and future trades stay retained; only the visible projections change.
    pub fn set_replay_clock_micros(
        &mut self,
        clock_micros: Option<i64>,
    ) -> Result<ReplayClockStats, FootprintError> {
        if clock_micros
            .is_some_and(|clock| !(MIN_TIMESTAMP_MICROS..=MAX_TIMESTAMP_MICROS).contains(&clock))
        {
            return Err(FootprintError::InvalidTimestamp { index: 0 });
        }
        let previous_clock_micros = self.replay_clock_micros;
        if previous_clock_micros == clock_micros {
            return Ok(ReplayClockStats {
                previous_clock_micros,
                clock_micros,
                stream_count: self.trade_streams.len(),
                depth_stream_count: self.depth_streams.len(),
                ..ReplayClockStats::default()
            });
        }

        let mut stream_ids = self.trade_streams.keys().copied().collect::<Vec<_>>();
        stream_ids.sort_unstable();
        let mut stats = ReplayClockStats {
            previous_clock_micros,
            clock_micros,
            stream_count: stream_ids.len(),
            depth_stream_count: self.depth_streams.len(),
            ..ReplayClockStats::default()
        };
        for stream_id in &stream_ids {
            let seek = self
                .trade_streams
                .get_mut(stream_id)
                .expect("collected trade stream remains live")
                .set_replay_clock_micros(clock_micros)?;
            stats.visible_trades = stats.visible_trades.saturating_add(seek.visible_trades);
            stats.rebuilt_trades = stats.rebuilt_trades.saturating_add(seek.rebuilt_trades);
            stats.incremental_trades = stats
                .incremental_trades
                .saturating_add(seek.incremental_trades);
        }
        // Resampled targets refresh from the earlier of the old and new cutoffs below, and
        // indicators from their dependencies' visible lengths; read both before the early
        // exposure changes them.
        let previous_cutoff = self.data.time_cutoff();
        let indicator_extents = self.indicator_dependency_extents();
        // Footprint projections reinstall below and enforce their retention ceilings against the
        // rows the data layer exposes. Expose the new clock's rows first, so a forward seek trims
        // to the same ceiling a clean rebuild does instead of counting against the old clock.
        if self.sequence_points.is_none() {
            self.data
                .set_time_cutoff(clock_micros.map(|clock| clock.div_euclid(MICROS_PER_SECOND)));
        }
        for stream_id in stream_ids {
            self.refresh_trade_dependents(stream_id)?;
            self.refresh_footprint_series_from_stream(stream_id, None)?;
        }
        for book in self.depth_streams.values_mut() {
            let depth = book
                .set_replay_clock_micros(clock_micros)
                .map_err(FootprintError::Depth)?;
            stats.visible_depth_events = stats
                .visible_depth_events
                .saturating_add(depth.visible_events);
            stats.rebuilt_depth_events = stats
                .rebuilt_depth_events
                .saturating_add(depth.rebuilt_events);
        }
        self.refresh_all_depth_heatmaps()
            .map_err(FootprintError::Depth)?;
        self.replay_clock_micros = clock_micros;
        self.refresh_synthetic_replay_projections()
            .map_err(|_| FootprintError::InvalidAggregation)?;
        let cutoff_seconds = self
            .sequence_points
            .is_none()
            .then(|| clock_micros.map(|clock| clock.div_euclid(MICROS_PER_SECOND)))
            .flatten();
        self.data.set_time_cutoff(cutoff_seconds);
        // Revealed rows count against retention ceilings: trim to what a clean load to this
        // clock with the same caps holds.
        let capped = self
            .series
            .iter()
            .filter(|series| !series.removed && series.max_points.is_some())
            .map(|series| series.id)
            .collect::<Vec<_>>();
        for id in capped {
            if self.enforce_series_cap(id) {
                self.sync_time_points();
                self.recompute_indicators_for(id);
            }
        }
        // Resampled bars aggregate only the visible source prefix, like studies, and both
        // refresh from where the move changed that prefix rather than from row 0.
        self.refresh_resampled_after_cutoff(previous_cutoff, cutoff_seconds);
        self.refresh_indicators_after_cutoff(&indicator_extents);
        self.invalidate_frame_scene();
        Ok(stats)
    }

    /// Create a bounded chart-level trade stream keyed by the host instrument identity. The
    /// stream owns canonical ordering, classification, corrections and retention; dependent
    /// footprint/study series refer to it by the returned opaque id.
    pub fn add_trade_stream(
        &mut self,
        key: &str,
        options: FootprintAggregationOptions,
    ) -> Result<u64, FootprintError> {
        if !matches!(options.bars, FootprintBarAggregation::Time { .. })
            && (!self.synthetic_series.is_empty() || !self.resampled_series.is_empty())
        {
            return Err(FootprintError::SequenceDomainInUse);
        }
        if key.is_empty() || key.len() > MAX_TRADE_STREAM_KEY_BYTES {
            return Err(FootprintError::InvalidTradeStreamKey);
        }
        if let Some(&stream_id) = self.trade_stream_keys.get(key) {
            let existing = self
                .trade_stream(stream_id)
                .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
            if existing.options() != options {
                return Err(FootprintError::InvalidAggregation);
            }
            return Ok(stream_id);
        }
        if self.trade_streams.len() >= MAX_TRADE_STREAMS {
            return Err(FootprintError::TradeStreamCapacity);
        }
        let stream_id = self.next_trade_stream_id;
        self.next_trade_stream_id = self.next_trade_stream_id.saturating_add(1).max(1);
        self.trade_streams
            .insert(stream_id, FootprintAggregator::new(options)?);
        self.trade_stream_keys.insert(key.to_string(), stream_id);
        Ok(stream_id)
    }

    pub fn trade_stream_id(&self, key: &str) -> Option<u64> {
        self.trade_stream_keys.get(key).copied()
    }

    pub fn trade_stream_revision(&self, stream_id: u64) -> Option<u64> {
        self.trade_stream(stream_id)
            .map(FootprintAggregator::revision)
    }

    pub fn trade_stream_replay_clock_micros(&self, stream_id: u64) -> Option<Option<i64>> {
        Some(self.trade_stream(stream_id)?.replay_clock_micros())
    }

    pub fn set_trade_stream_replay_clock_micros(
        &mut self,
        stream_id: u64,
        clock_micros: Option<i64>,
    ) -> Result<ReplaySeekStats, FootprintError> {
        let stats = self
            .trade_streams
            .get_mut(&stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?
            .set_replay_clock_micros(clock_micros)?;
        if stats.previous_clock_micros != stats.clock_micros {
            self.invalidate_profile_drawings_using_stream(stream_id);
            self.refresh_trade_dependents(stream_id)?;
            self.refresh_footprint_series_from_stream(stream_id, None)?;
        }
        Ok(stats)
    }

    pub fn trade_stream_stats(&self, stream_id: u64) -> Option<TradeStreamStats> {
        let stream = self.trade_stream(stream_id)?;
        let dependents = self.trade_dependents.get(&stream_id);
        let bar_dependents = self.trade_bar_dependents.get(&stream_id);
        Some(TradeStreamStats {
            revision: stream.revision(),
            stream_capacity_bytes: stream.capacity_bytes(),
            dependent_count: dependents.map_or(0, Vec::len)
                + bar_dependents.map_or(0, Vec::len)
                + self.big_trades_count(stream_id)
                + self.auction_markers_count(stream_id),
            dependent_rebuilds: dependents
                .into_iter()
                .flatten()
                .map(|dependent| dependent.rebuilds)
                .sum::<u64>()
                + bar_dependents
                    .into_iter()
                    .flatten()
                    .map(|dependent| dependent.rebuilds)
                    .sum::<u64>(),
            dependent_incremental_updates: dependents
                .into_iter()
                .flatten()
                .map(|dependent| dependent.incremental_updates)
                .sum::<u64>()
                + bar_dependents
                    .into_iter()
                    .flatten()
                    .map(|dependent| dependent.incremental_updates)
                    .sum::<u64>(),
            dependent_rows_computed: stream.dependent_work.study_rows_computed,
            bar_rows_projected: stream.dependent_work.bar_rows_projected,
            big_trades_prints_scanned: stream.dependent_work.big_trades_prints_scanned,
            big_trades_replays: stream.dependent_work.big_trades_replays,
        })
    }

    /// Return a bounded, newest-first time-and-sales projection of the canonical classified tape.
    pub fn time_and_sales(
        &self,
        stream_id: u64,
        options: TimeAndSalesOptions,
    ) -> Result<Vec<TimeAndSalesRow>, FootprintError> {
        if !options.minimum_volume.is_finite()
            || options.minimum_volume < 0.0
            || options.max_rows > MAX_TIME_AND_SALES_ROWS
        {
            return Err(FootprintError::InvalidTimeAndSalesOptions);
        }
        let stream = self
            .trade_stream(stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
        Ok(stream
            .trades
            .range(..stream.visible_trade_count())
            .rev()
            .filter(|stored| {
                stored.event.volume >= options.minimum_volume
                    && options
                        .side
                        .is_none_or(|side| side == stored.classified_side)
            })
            .take(options.max_rows)
            .map(|stored| TimeAndSalesRow {
                timestamp_micros: stored.event.timestamp_micros,
                price: stored.event.price,
                volume: stored.event.volume,
                aggressor: stored.classified_side,
                trade_id: stored.event.trade_id,
                conditions: stored.event.conditions,
            })
            .collect())
    }

    pub fn remove_trade_stream(&mut self, stream_id: u64) -> Result<(), FootprintError> {
        if !self.trade_streams.contains_key(&stream_id) {
            return Err(FootprintError::UnknownTradeStream(stream_id));
        }
        if self.profile_drawings_use_stream(stream_id)
            || self.series.iter().any(|series| {
                series
                    .footprint
                    .as_ref()
                    .is_some_and(|state| state.trade_stream_id == stream_id && !series.removed)
            })
            || self
                .trade_bar_dependents
                .get(&stream_id)
                .is_some_and(|dependents| !dependents.is_empty())
            || self
                .trade_dependents
                .get(&stream_id)
                .is_some_and(|dependents| !dependents.is_empty())
            || self.big_trades_count(stream_id) > 0
            || self.auction_markers_count(stream_id) > 0
        {
            return Err(FootprintError::TradeStreamInUse(stream_id));
        }
        self.trade_streams.remove(&stream_id);
        self.trade_stream_keys.retain(|_, id| *id != stream_id);
        Ok(())
    }

    fn install_footprint_bars_projection(
        &mut self,
        id: SeriesId,
        stream_id: u64,
        projection: BarProjection,
    ) -> Result<(), FootprintError> {
        let installed = match projection {
            BarProjection::Time((times, open, high, low, close)) => {
                self.install_footprint_projection(id, times, open, high, low, close)
            }
            BarProjection::Sequence(projection) => {
                let key_base = self.sequence_install_key_base(stream_id);
                self.install_footprint_sequence_projection(id, key_base, projection)
            }
        };
        installed
            .then_some(())
            .ok_or(FootprintError::UnknownSeries(id))
    }

    /// Replace the footprint rows from stream bar `from` on. `false` when the rows no longer line
    /// up with the stream, in which case the caller installs the complete projection.
    fn update_footprint_bars_projection(
        &mut self,
        id: SeriesId,
        from: usize,
        projection: BarProjection,
    ) -> bool {
        match projection {
            BarProjection::Time((times, open, high, low, close)) => {
                self.update_footprint_projection_bars(id, times, open, high, low, close) > 0
            }
            BarProjection::Sequence(projection) => {
                self.update_footprint_sequence_projection_bars(id, from, projection) > 0
            }
        }
    }

    fn install_trade_bar_projection(
        &mut self,
        id: SeriesId,
        stream_id: u64,
        projection: BarProjection,
    ) -> Result<(), FootprintError> {
        let installed = match projection {
            BarProjection::Time((times, open, high, low, close)) => {
                self.install_series_data_inner(id, times, open, high, low, close)
            }
            BarProjection::Sequence(projection) => {
                let key_base = self.sequence_install_key_base(stream_id);
                self.install_trade_bar_sequence_projection(id, key_base, projection)
            }
        };
        installed
            .then_some(())
            .ok_or(FootprintError::UnknownSeries(id))
    }

    fn update_trade_bar_projection(
        &mut self,
        id: SeriesId,
        from: usize,
        projection: BarProjection,
    ) -> bool {
        match projection {
            BarProjection::Time((times, open, high, low, close)) => {
                self.update_series_bars_sanitized_inner(id, times, open, high, low, close) > 0
            }
            BarProjection::Sequence(projection) => {
                self.update_trade_bar_sequence_projection_bars(id, from, projection) > 0
            }
        }
    }

    /// Rebind a footprint series to a canonical chart stream. The stream's aggregation policy is
    /// authoritative; the visual options remain series-local.
    pub fn bind_footprint_series_to_stream(
        &mut self,
        id: SeriesId,
        stream_id: u64,
    ) -> Result<(), FootprintError> {
        self.validate_series_id(id).map_err(series_error)?;
        let stream = self
            .trade_stream(stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
        let projection = BarProjection::of(stream.options(), stream.bars())?;
        let visual = self
            .series_entry(id)
            .and_then(|series| series.footprint.as_ref())
            .map(|state| state.visual.clone())
            .ok_or(FootprintError::UnknownSeries(id))?;
        let previous_stream_id = self
            .series_entry(id)
            .and_then(|series| series.footprint.as_ref())
            .map(|state| state.trade_stream_id)
            .ok_or(FootprintError::UnknownSeries(id))?;
        self.series_entry_mut(id)
            .and_then(|series| series.footprint.as_mut())
            .ok_or(FootprintError::UnknownSeries(id))?
            .clone_from(&FootprintSeriesState {
                trade_stream_id: stream_id,
                visual,
            });
        self.record_dependent_work(stream_id, |work| {
            work.bar_rows_projected += projection.len() as u64;
        });
        self.install_footprint_bars_projection(id, stream_id, projection)?;
        self.invalidate_frame_series(id);
        if previous_stream_id != stream_id {
            self.prune_trade_stream_if_unused(previous_stream_id);
        }
        Ok(())
    }

    /// Bind an ordinary candlestick or OHLC bar presentation to the canonical bars derived from
    /// a chart-level trade stream. The series retains presentation options only; trade ordering,
    /// corrections, aggregation, and logical non-time bar identity remain stream-owned.
    ///
    /// The kind check runs first: a footprint or scalar series is `UnsupportedTradeBarSeries`, and
    /// a series with a `max_points` cap is `InvalidAggregation`. A candlestick or bar that another
    /// engine feature writes (a resampled or synthetic-bar target, or a study converted to a
    /// candle) is `SeriesOwned`; rebinding a bound candle to another stream is allowed. Every
    /// refusal happens before anything changes.
    pub fn bind_trade_bar_series_to_stream(
        &mut self,
        id: SeriesId,
        stream_id: u64,
    ) -> Result<(), FootprintError> {
        self.validate_series_id(id).map_err(series_error)?;
        if !self
            .series_entry(id)
            .is_some_and(|series| matches!(series.kind, SeriesKind::Candlestick | SeriesKind::Bar))
        {
            return Err(FootprintError::UnsupportedTradeBarSeries(id));
        }
        if self.series_max_points(id).is_some() {
            return Err(FootprintError::InvalidAggregation);
        }
        // One writer per series: only an unowned series, or one already bound to a stream (a
        // rebind), may become a trade-bound presentation. The trade writers below install through
        // the unguarded internals, so this is the only thing keeping a resampled or synthetic
        // target, or a study, from having two writers.
        if !matches!(self.series_owner(id), None | Some(SeriesOwner::TradeBars)) {
            return Err(FootprintError::SeriesOwned(id));
        }
        self.trade_stream(stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
        // Trade-bound bars take the stream's time domain; an as-of overlay rejoins the union.
        self.rejoin_time_union(id);
        for dependents in self.trade_bar_dependents.values_mut() {
            dependents.retain(|dependent| dependent.series_id != id);
        }
        self.trade_bar_dependents
            .entry(stream_id)
            .or_default()
            .push(TradeBarDependent {
                series_id: id,
                applied_revision: 0,
                rebuilds: 0,
                incremental_updates: 0,
            });
        let refreshed = self.refresh_trade_bar_dependents_from(stream_id, None);
        if refreshed.is_err() {
            // A failed first projection must not leave a registration that fails every tip.
            if let Some(dependents) = self.trade_bar_dependents.get_mut(&stream_id) {
                dependents.retain(|dependent| dependent.series_id != id);
                if dependents.is_empty() {
                    self.trade_bar_dependents.remove(&stream_id);
                }
            }
        }
        refreshed
    }

    pub fn add_cvd_series(
        &mut self,
        stream_id: u64,
        pane_index: usize,
        options: TradeStudyOptions,
    ) -> Result<SeriesId, FootprintError> {
        self.trade_stream(stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
        if options.cumulative_delta_reset == CumulativeDeltaReset::Anchored
            && options.anchor_timestamp_micros.is_none()
        {
            return Err(FootprintError::InvalidAggregation);
        }
        let id = self.add_series(SeriesKind::Line);
        self.set_series_pane(id, pane_index, SEPARATE_INDICATOR_PANE_STRETCH);
        if let Some(series) = self.series_entry_mut(id) {
            series.title = "CVD".to_string();
        }
        self.register_trade_dependent(stream_id, TradeStudyKind::CumulativeDelta, id, options);
        if let Err(error) = self.refresh_trade_dependents(stream_id) {
            self.remove_series(id);
            return Err(error);
        }
        Ok(id)
    }

    pub fn add_delta_series(
        &mut self,
        stream_id: u64,
        pane_index: usize,
    ) -> Result<SeriesId, FootprintError> {
        self.trade_stream(stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
        let id = self.add_series(SeriesKind::Histogram);
        self.set_series_pane(id, pane_index, SEPARATE_INDICATOR_PANE_STRETCH);
        if let Some(series) = self.series_entry_mut(id) {
            series.title = "Delta".to_string();
        }
        self.register_trade_dependent(
            stream_id,
            TradeStudyKind::DeltaHistogram,
            id,
            TradeStudyOptions::default(),
        );
        if let Err(error) = self.refresh_trade_dependents(stream_id) {
            self.remove_series(id);
            return Err(error);
        }
        Ok(id)
    }

    /// Add a volume histogram derived from the same canonical bars as the stream's candles: one
    /// column of total traded volume per bar, updated with every live, corrected, replayed, or
    /// retained tape change. Columns take the up/down volume tint of the chart's primary price
    /// series (`histogram_updown`), which hosts may restyle like any histogram.
    pub fn add_trade_volume_series(
        &mut self,
        stream_id: u64,
        pane_index: usize,
    ) -> Result<SeriesId, FootprintError> {
        self.trade_stream(stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
        let id = self.add_series(SeriesKind::Histogram);
        self.set_series_pane(id, pane_index, 1.0);
        if let Some(series) = self.series_entry_mut(id) {
            series.title = "Volume".to_string();
            series.histogram_updown = true;
            series.price_format.kind = PriceFormatKind::Volume;
        }
        self.register_trade_dependent(
            stream_id,
            TradeStudyKind::Volume,
            id,
            TradeStudyOptions::default(),
        );
        if let Err(error) = self.refresh_trade_dependents(stream_id) {
            self.remove_series(id);
            return Err(error);
        }
        Ok(id)
    }

    /// Anchor a time-bar trade stream to exchange-local session windows placed in the chart's
    /// exchange time zone and session start: each window restarts the bar grid at its open (a
    /// 60-minute A-share bar opens at 09:30, 10:30, 13:00 and 14:00), and `outside` decides what
    /// happens to auction, lunch, and after-hours prints. `None` restores the plain
    /// `anchor_micros` grid. The stream rebuilds its bars once and every dependent follows; a
    /// rejected configuration changes nothing. Changing the chart's exchange time re-places the
    /// windows. Sealed order-flow history cannot be re-placed, so a grid change drops it: the
    /// history then starts at the raw tape and the host refills it on the new grid through
    /// [`Self::prepend_order_flow_history`].
    pub fn set_trade_stream_sessions(
        &mut self,
        stream_id: u64,
        sessions: Option<TradeSessionOptions>,
    ) -> Result<(), FootprintError> {
        let current = self
            .trade_stream(stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
        if sessions.is_none() && current.session_grid().is_none() {
            return Ok(());
        }
        let mut next = current.clone();
        next.set_sessions(sessions.as_ref(), &self.exchange_time)?;
        if next.revision() == current.revision() {
            return Ok(());
        }
        // Reject a bar-time collision anywhere on the tape before anything changes.
        if next.tape_bar_times_collide() {
            return Err(FootprintError::ProjectionTimeCollision);
        }
        self.install_regridded_trade_stream(stream_id, next);
        self.refresh_trade_dependents(stream_id)?;
        self.refresh_footprint_series_from_stream(stream_id, None)
    }

    /// Install `next`, the stream re-aggregated on another bar grid. Sealed history it discarded
    /// from an already truncated history leaves every dependent as evicted history does:
    /// anchored cumulative-delta bases advance over it (see
    /// `FootprintAggregator::discard_sealed_history`). The caller then re-projects every
    /// dependent once from the new bars.
    fn install_regridded_trade_stream(&mut self, stream_id: u64, next: FootprintAggregator) {
        let evicted = self
            .trade_stream(stream_id)
            .filter(|current| current.history_truncated)
            .map_or(0, |current| {
                usize::try_from(next.bar_origin() - current.bar_origin()).unwrap_or(0)
            });
        self.evict_trade_dependents_front(stream_id, evicted);
        self.trade_streams.insert(stream_id, next);
        self.invalidate_profile_drawings_using_stream(stream_id);
    }

    /// Re-place every session-anchored trade stream after the chart's exchange time changed. A
    /// stream whose bars would collide in the new placement keeps its previous one.
    pub(crate) fn refresh_trade_stream_sessions(&mut self) {
        let mut exchange = self.exchange_time.clone();
        exchange.set_calendar_dates(false);
        let mut stale = self
            .trade_streams
            .iter()
            .filter_map(|(&id, stream)| {
                let grid = stream.session_grid()?;
                (grid.exchange_time() != &exchange).then(|| {
                    (
                        id,
                        TradeSessionOptions {
                            windows: grid.windows().to_vec(),
                            outside: grid.policy(),
                        },
                    )
                })
            })
            .collect::<Vec<_>>();
        stale.sort_unstable_by_key(|(id, _)| *id);
        for (id, sessions) in stale {
            let _ = self.set_trade_stream_sessions(id, Some(sessions));
        }
    }

    /// Create the complete series/pane graph for one shared order-flow stream.
    ///
    /// `ticks_per_row == 0` selects automatic rows: levels are kept at the instrument tick and
    /// merged into legible display rows for the current zoom. The footprint series owns the
    /// last-value label, price line, and countdown; the primary series only supplies the bar
    /// grid, so a host installs it as whitespace while the footprint is shown.
    ///
    /// Any failure rolls back every series, pane dependency, and stream created by this call.
    pub fn add_order_flow_presentation(
        &mut self,
        stream_key: &str,
        primary_series: SeriesId,
        mut options: OrderFlowPresentationOptions,
    ) -> Result<OrderFlowPresentation, FootprintError> {
        self.validate_series_id(primary_series)
            .map_err(series_error)?;
        if options.aggregation.ticks_per_row == 0 {
            options.aggregation.ticks_per_row = 1;
            options.visual.adaptive_rows = true;
        }
        validate_chart_projection(options.aggregation)?;
        validate_visual_options(&options.visual)?;

        let stream = self.add_trade_stream(stream_key, options.aggregation)?;
        let mut presentation = OrderFlowPresentation {
            trade_stream: stream,
            footprint_series: None,
            cumulative_delta_series: None,
            delta_series: None,
            big_trades: None,
            ticks_per_row: options.aggregation.ticks_per_row,
            primary_series,
        };
        let result = (|| {
            if options.show_footprint {
                let series = self.add_footprint_series(FootprintSeriesOptions {
                    aggregation: options.aggregation,
                    visual: options.visual,
                })?;
                self.bind_footprint_series_to_stream(series, stream)?;
                if let Some(entry) = self.series_entry_mut(series) {
                    entry.countdown_visible = true;
                }
                presentation.footprint_series = Some(series);
            }
            if options.show_cumulative_delta {
                presentation.cumulative_delta_series =
                    Some(self.add_order_flow_study(stream, TradeStudyKind::CumulativeDelta)?);
            }
            if options.show_delta_histogram {
                presentation.delta_series =
                    Some(self.add_order_flow_study(stream, TradeStudyKind::DeltaHistogram)?);
            }
            if let Some(big_trades) = options.big_trades.take() {
                let host = presentation.footprint_series.unwrap_or(primary_series);
                presentation.big_trades = Some(self.add_big_trades(stream, host, big_trades)?);
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.remove_order_flow_presentation(presentation);
            return Err(error);
        }
        Ok(presentation)
    }

    /// Bring an existing order-flow graph to `options` in place, keeping its stream and every
    /// bar of its history. Rows, imbalance rules, cell display, the shown series and the
    /// big-trades indicator change without replaying the tape. The tick size and bar
    /// aggregation define the stream, so changing either needs a new presentation.
    ///
    /// On error `presentation` still describes exactly the graph that exists.
    pub fn reconfigure_order_flow_presentation(
        &mut self,
        presentation: &mut OrderFlowPresentation,
        mut options: OrderFlowPresentationOptions,
    ) -> Result<(), FootprintError> {
        let stream_id = presentation.trade_stream;
        if options.aggregation.ticks_per_row == 0 {
            options.aggregation.ticks_per_row = 1;
            options.visual.adaptive_rows = true;
        }
        validate_chart_projection(options.aggregation)?;
        validate_visual_options(&options.visual)?;
        if options.big_trades.as_ref().is_some_and(|big| !big.valid()) {
            return Err(FootprintError::InvalidBigTradesOptions);
        }
        let current = self
            .trade_stream(stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?
            .options();
        if current.tick_size != options.aggregation.tick_size
            || current.bars != options.aggregation.bars
        {
            return Err(FootprintError::InvalidAggregation);
        }
        self.trade_streams
            .get_mut(&stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?
            .set_row_options(
                options.aggregation.ticks_per_row,
                options.aggregation.imbalance,
            )?;
        presentation.ticks_per_row = options.aggregation.ticks_per_row;

        match (presentation.footprint_series, options.show_footprint) {
            (Some(series), true) => {
                self.series_entry_mut(series)
                    .and_then(|entry| entry.footprint.as_mut())
                    .ok_or(FootprintError::UnknownSeries(series))?
                    .visual = options.visual;
                self.invalidate_frame_series(series);
            }
            (Some(series), false) => {
                // The footprint's bubbles move to the price series instead of leaving with it.
                if let Some(id) = presentation.big_trades {
                    self.rehost_big_trades(id, presentation.primary_series);
                }
                self.remove_series(series);
                presentation.footprint_series = None;
            }
            (None, true) => {
                let series = self.add_footprint_series(FootprintSeriesOptions {
                    aggregation: options.aggregation,
                    visual: options.visual,
                })?;
                if let Err(error) = self.bind_footprint_series_to_stream(series, stream_id) {
                    self.remove_series(series);
                    return Err(error);
                }
                if let Some(entry) = self.series_entry_mut(series) {
                    entry.countdown_visible = true;
                }
                presentation.footprint_series = Some(series);
                if let Some(id) = presentation.big_trades {
                    self.rehost_big_trades(id, series);
                }
            }
            (None, false) => {}
        }

        for (kind, show) in [
            (
                TradeStudyKind::CumulativeDelta,
                options.show_cumulative_delta,
            ),
            (TradeStudyKind::DeltaHistogram, options.show_delta_histogram),
        ] {
            let slot = match kind {
                TradeStudyKind::CumulativeDelta => &mut presentation.cumulative_delta_series,
                TradeStudyKind::DeltaHistogram => &mut presentation.delta_series,
                // An order-flow graph shows cumulative delta and delta studies; tick-built volume
                // belongs to trade-bound candles, not to the presentation.
                TradeStudyKind::Volume => continue,
            };
            match (*slot, show) {
                (Some(series), false) => {
                    *slot = None;
                    self.remove_series(series);
                }
                (None, true) => *slot = Some(self.add_order_flow_study(stream_id, kind)?),
                _ => {}
            }
        }

        let host = presentation
            .footprint_series
            .unwrap_or(presentation.primary_series);
        match (presentation.big_trades, options.big_trades) {
            (Some(id), Some(next)) => self.set_big_trades_options(id, next)?,
            (Some(id), None) => {
                presentation.big_trades = None;
                self.remove_big_trades(id);
            }
            (None, Some(next)) => {
                presentation.big_trades = Some(self.add_big_trades(stream_id, host, next)?);
            }
            (None, None) => {}
        }
        Ok(())
    }

    /// One tape study in its own indicator pane, labelled with the chart's indicator chrome.
    fn add_order_flow_study(
        &mut self,
        stream_id: u64,
        kind: TradeStudyKind,
    ) -> Result<SeriesId, FootprintError> {
        let pane = self
            .add_pane(false)
            .ok_or(FootprintError::InvalidAggregation)?;
        self.panes[pane].stretch_factor = SEPARATE_INDICATOR_PANE_STRETCH;
        let id = match kind {
            TradeStudyKind::CumulativeDelta => {
                self.add_cvd_series(stream_id, pane, TradeStudyOptions::default())
            }
            TradeStudyKind::DeltaHistogram => self.add_delta_series(stream_id, pane),
            TradeStudyKind::Volume => self.add_trade_volume_series(stream_id, pane),
        }?;
        let chrome = self.indicator_chrome;
        if let Some(series) = self.series_entry_mut(id) {
            series.title_visible = chrome.name_labels_visible;
            series.last_value_visible = chrome.value_labels_visible;
            series.price_line_visible = chrome.price_lines_visible;
        }
        Ok(id)
    }

    /// Replace or append canonical tape data and atomically advance every dependent presentation.
    ///
    /// Appending accumulates the host's new suffix onto the retained tape, so bars built from
    /// trades the host has since evicted are kept. Past [`ORDER_FLOW_MAX_RETAINED_TRADES`] the
    /// oldest bars are sealed into history, which spans at most
    /// [`ORDER_FLOW_MAX_RETAINED_SESSIONS`] sessions and [`ORDER_FLOW_MAX_STREAM_BYTES`].
    /// Replacing the tape discards that history.
    pub fn update_order_flow_presentation(
        &mut self,
        presentation: OrderFlowPresentation,
        trades: Vec<FootprintTrade>,
        append: bool,
    ) -> Result<FootprintUpdateKind, FootprintError> {
        let update = if append {
            self.update_trade_stream_trades(presentation.trade_stream, trades)?
        } else {
            self.set_trade_stream_trades(presentation.trade_stream, trades)?;
            FootprintUpdateKind::Historical
        };
        self.enforce_order_flow_retention(presentation);
        Ok(update)
    }

    /// Install a host's rewritten bounded tape window without discarding older history.
    ///
    /// The window is authoritative only for the span it covers: every print at or after its
    /// earliest timestamp is replaced by the window, so corrections, cancellations, backfilled
    /// prints and a restarted tape take effect, while bars built before that span (including
    /// sealed history) are kept. An empty window covers nothing and changes nothing.
    pub fn replace_order_flow_window(
        &mut self,
        presentation: OrderFlowPresentation,
        trades: Vec<FootprintTrade>,
    ) -> Result<(), FootprintError> {
        let stream_id = presentation.trade_stream;
        let stream = self
            .trade_stream(stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
        let previous_bar_count = stream.bars().len();
        // Auction detection reads finished bars, not tape checkpoints. Include the bar whose
        // start precedes the window even when its last old print precedes the window too.
        let auction_from = trades
            .iter()
            .map(|trade| trade.timestamp_micros)
            .min()
            .map(|from| stream.auction_repair_from(from));
        let Some(first_bar) = self
            .trade_streams
            .get_mut(&stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?
            .replace_trades_from_window(trades)?
        else {
            return Ok(());
        };
        // A suffix projection only upserts rows, so a window that removed bars re-projects all.
        let incremental_from = self
            .trade_stream(stream_id)
            .is_some_and(|stream| stream.bars().len() >= previous_bar_count)
            .then_some(first_bar);
        self.invalidate_profile_drawings_using_stream(stream_id);
        // The bar projector may need a full install when the window shrinks, but
        // auction marks retain their unchanged prefix and only replay the rewritten bars.
        self.refresh_stream_presentations_from(stream_id, incremental_from, false, auction_from)?;
        self.enforce_order_flow_retention(presentation);
        Ok(())
    }

    /// Join an older tape page to the front of an order-flow history, typically one page of a
    /// host's backfill paged backward from the oldest retained trade. Prints at or after the
    /// start of the retained history are skipped, except the prints a grid change released at
    /// that start's microsecond, even when live appends sealed bars again before the refill (see
    /// `FootprintAggregator::prepend_history`). Every dependent is re-projected once.
    pub fn prepend_order_flow_history(
        &mut self,
        presentation: OrderFlowPresentation,
        trades: Vec<FootprintTrade>,
    ) -> Result<HistoryPrefixStats, FootprintError> {
        let stream_id = presentation.trade_stream;
        let (mut stats, sealed_page) = self
            .trade_streams
            .get_mut(&stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?
            .prepend_history(trades)?;
        if let Some(page) = &sealed_page {
            self.prepend_big_trades_page(stream_id, page);
        }
        if stats.accepted_trades > 0 {
            self.invalidate_profile_drawings_using_stream(stream_id);
            self.refresh_stream_presentations_from(stream_id, None, false, None)?;
            self.enforce_order_flow_retention(presentation);
        }
        stats.history_full |= self
            .trade_stream(stream_id)
            .is_some_and(|stream| stream.history_truncated);
        Ok(stats)
    }

    /// Seal raw bars past the raw-tape ceiling, then evict sealed history past the session and
    /// memory budgets. Sealing changes no bar, so only eviction touches dependent rows.
    fn enforce_order_flow_retention(&mut self, presentation: OrderFlowPresentation) {
        let stream_id = presentation.trade_stream;
        let Some(seal) = self
            .trade_stream(stream_id)
            .map(|stream| stream.bars_to_seal(ORDER_FLOW_MAX_RETAINED_TRADES))
        else {
            return;
        };
        self.seal_trade_stream_bars(stream_id, seal);
        let Some(keep) = self.trade_stream(stream_id).and_then(|stream| {
            stream.bars_within_history_budget(
                ORDER_FLOW_MAX_RETAINED_SESSIONS,
                ORDER_FLOW_MAX_STREAM_BYTES,
            )
        }) else {
            return;
        };
        // Every presentation of the stream holds one row per bar under the same keys, so any of
        // them anchors the trim; the footprint does when the graph shows one.
        let anchor = presentation
            .footprint_series
            .or_else(|| self.stream_presentations(stream_id).next());
        self.trim_trade_stream_front(stream_id, anchor, keep);
        match anchor {
            Some(anchor) => self.recompute_indicators_for(anchor),
            None => self.sync_time_points(),
        }
    }

    /// Seal the oldest `count` raw bars of a stream. Big-trades indicators first absorb the
    /// released trades, so their bubbles survive the release and later replays.
    fn seal_trade_stream_bars(&mut self, stream_id: u64, count: usize) {
        if count == 0 {
            return;
        }
        self.absorb_sealed_trades_into_big_trades(stream_id, count);
        let Some(stream) = self.trade_streams.get_mut(&stream_id) else {
            return;
        };
        if stream.seal_bars(count) == 0 {
            return;
        }
        self.invalidate_profile_drawings_using_stream(stream_id);
        self.refresh_big_trades(stream_id, true);
    }

    /// Tear down a complete order-flow graph and release its fixed aggregation stream.
    pub fn remove_order_flow_presentation(&mut self, presentation: OrderFlowPresentation) -> bool {
        let mut changed = presentation
            .big_trades
            .is_some_and(|id| self.remove_big_trades(id));
        for series in [
            presentation.cumulative_delta_series,
            presentation.delta_series,
            presentation.footprint_series,
        ]
        .into_iter()
        .flatten()
        {
            changed |= self.remove_series(series);
        }
        if self.remove_trade_stream(presentation.trade_stream).is_ok() {
            changed = true;
        }
        changed
    }

    /// The footprint an order-flow presentation draws over `primary`: the first visible footprint
    /// in its pane bound to a shared chart trade stream, as `add_order_flow_presentation` builds
    /// it. Under the presentation contract the primary is whitespace and this footprint presents
    /// its bars, so engine readouts default to it.
    pub(crate) fn order_flow_footprint_over(&self, primary: SeriesId) -> Option<SeriesId> {
        let pane = self.series_entry(primary)?.pane_index;
        self.series
            .iter()
            .find(|series| {
                !series.removed
                    && series.visible
                    && series.pane_index == pane
                    && series.footprint.as_ref().is_some_and(|state| {
                        self.trade_stream_keys
                            .values()
                            .any(|&stream_id| stream_id == state.trade_stream_id)
                    })
            })
            .map(|series| series.id)
    }

    /// Open the footprint viewport: bars wide enough for `bid x ask` numbers, anchored at the
    /// real-time edge. Hosts call it when the user enters footprint mode, never on rebuilds,
    /// so the user's own zoom survives tape gaps and settings changes.
    pub fn fit_footprint_viewport(&mut self) {
        // A chart that has not been laid out yet has a zero-width time scale, which would clamp
        // the spacing to nothing. Seed it with the CSS width; the first layout narrows it by the
        // axis widths without changing the spacing.
        if self.time_scale.width() <= 0.0 {
            self.time_scale.set_width(self.css_width);
        }
        self.set_bar_spacing(FOOTPRINT_BAR_SPACING);
        self.scroll_to_real_time();
    }

    /// Add a first-class tick-driven footprint series. Time bars use the chart's UTC-second
    /// projection; trade-count, volume, and range bars use the chart-owned logical sequence axis
    /// with full-resolution open/close times retained in its sidecar.
    pub fn add_footprint_series(
        &mut self,
        options: FootprintSeriesOptions,
    ) -> Result<SeriesId, FootprintError> {
        let id = self.add_series(SeriesKind::Footprint);
        if let Err(error) = self.configure_footprint_series(id, options) {
            self.remove_series(id);
            return Err(error);
        }
        Ok(id)
    }

    /// Make `id` a footprint series. A series that a trade stream, study, resampler, or synthetic
    /// bars already write is `SeriesOwned`, refused before anything changes.
    pub fn configure_footprint_series(
        &mut self,
        id: SeriesId,
        options: FootprintSeriesOptions,
    ) -> Result<(), FootprintError> {
        validate_chart_projection(options.aggregation)?;
        validate_visual_options(&options.visual)?;
        self.validate_series_id(id).map_err(series_error)?;
        // One writer per series: a trade-bound candle, a study, or a resampled or synthetic
        // target cannot become a footprint.
        if !matches!(self.series_owner(id), None | Some(SeriesOwner::Footprint)) {
            return Err(FootprintError::SeriesOwned(id));
        }
        let aggregator = FootprintAggregator::new(options.aggregation)?;
        // A footprint owns its trade-derived rows; an as-of overlay rejoins the union.
        self.rejoin_time_union(id);
        let stream_id = self
            .series_entry(id)
            .and_then(|series| series.footprint.as_ref())
            .map(|state| state.trade_stream_id)
            .unwrap_or_else(|| {
                let stream_id = self.next_trade_stream_id;
                self.next_trade_stream_id = self.next_trade_stream_id.saturating_add(1).max(1);
                stream_id
            });
        self.trade_streams.insert(stream_id, aggregator);
        self.reset_trade_dependent_folds(stream_id);
        let had_data = !self.data_layer().plot(id).is_empty();
        let series = self
            .series_entry_mut(id)
            .ok_or(FootprintError::UnknownSeries(id))?;
        series.kind = SeriesKind::Footprint;
        series.feature = None;
        series.footprint = Some(FootprintSeriesState {
            trade_stream_id: stream_id,
            visual: options.visual,
        });
        series.price_format = footprint_price_format(options.aggregation.tick_size);
        series.custom_frame = Default::default();
        self.data.set_rows_count_as_data(id, true);
        if had_data {
            let cleared = self.install_footprint_projection(
                id,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            );
            debug_assert!(cleared);
        }
        self.invalidate_frame_series(id);
        Ok(())
    }

    pub fn footprint_series_options(&self, id: SeriesId) -> Option<FootprintSeriesOptions> {
        let state = self.series_entry(id)?.footprint.as_ref()?;
        Some(FootprintSeriesOptions {
            aggregation: self.trade_stream(state.trade_stream_id)?.options(),
            visual: state.visual.clone(),
        })
    }

    pub fn apply_footprint_series_options(
        &mut self,
        id: SeriesId,
        options: FootprintSeriesOptions,
    ) -> Result<(), FootprintError> {
        validate_chart_projection(options.aggregation)?;
        validate_visual_options(&options.visual)?;
        self.validate_series_id(id).map_err(series_error)?;
        let stream_id = self
            .series_entry(id)
            .and_then(|series| series.footprint.as_ref())
            .map(|state| state.trade_stream_id)
            .ok_or(FootprintError::UnknownSeries(id))?;
        let current = self
            .trade_stream(stream_id)
            .ok_or(FootprintError::UnknownSeries(id))?
            .options();
        if current.tick_size == options.aggregation.tick_size
            && current.bars == options.aggregation.bars
        {
            // Rows and imbalance rules re-derive in place from the per-tick levels, so the
            // stream's sealed history survives them.
            self.trade_streams
                .get_mut(&stream_id)
                .ok_or(FootprintError::UnknownSeries(id))?
                .set_row_options(
                    options.aggregation.ticks_per_row,
                    options.aggregation.imbalance,
                )?;
            self.series_entry_mut(id)
                .and_then(|series| series.footprint.as_mut())
                .expect("validated footprint series")
                .visual = options.visual;
            self.invalidate_frame_series(id);
            return Ok(());
        }
        let dependent_count = self
            .series
            .iter()
            .filter(|series| {
                !series.removed
                    && series
                        .footprint
                        .as_ref()
                        .is_some_and(|state| state.trade_stream_id == stream_id)
            })
            .count()
            + self.trade_dependents.get(&stream_id).map_or(0, Vec::len)
            + self.big_trades_count(stream_id)
            + self.auction_markers_count(stream_id);
        if dependent_count > 1 {
            return Err(FootprintError::TradeStreamInUse(stream_id));
        }
        // A non-time sequence axis never shares the chart with resampling or synthetic bars, as
        // `add_trade_stream` enforces.
        if !matches!(
            options.aggregation.bars,
            FootprintBarAggregation::Time { .. }
        ) && (!self.synthetic_series.is_empty() || !self.resampled_series.is_empty())
        {
            return Err(FootprintError::SequenceDomainInUse);
        }
        let aggregator = self
            .trade_stream(stream_id)
            .ok_or(FootprintError::UnknownSeries(id))?
            .with_options(options.aggregation)?;
        if aggregator.tape_bar_times_collide() {
            return Err(FootprintError::ProjectionTimeCollision);
        }
        self.install_regridded_trade_stream(stream_id, aggregator);
        let series = self
            .series_entry_mut(id)
            .expect("validated footprint series");
        series
            .footprint
            .as_mut()
            .expect("validated footprint series")
            .clone_from(&FootprintSeriesState {
                trade_stream_id: stream_id,
                visual: options.visual,
            });
        series.price_format = footprint_price_format(options.aggregation.tick_size);
        // Trade-bound candles/bars present the same stream bars, so they follow the change too.
        // The footprint installs first: after a switch onto the sequence axis the other
        // presentations continue from its row keys, never from their old time keys.
        self.refresh_footprint_series_from_stream(stream_id, None)?;
        self.refresh_trade_dependents(stream_id)
    }

    /// Footprint bars with levels grouped into the configured `ticks_per_row` rows.
    pub fn footprint_bars(&self, id: SeriesId) -> Option<Vec<FootprintBar>> {
        let stream_id = self.series_entry(id)?.footprint.as_ref()?.trade_stream_id;
        let stream = self.trade_stream(stream_id)?;
        Some(
            stream
                .bars()
                .iter()
                .map(|bar| stream.presented_bar(bar).into_owned())
                .collect(),
        )
    }

    pub fn footprint_bar(&self, id: SeriesId, bar_index: usize) -> Option<FootprintBar> {
        let stream_id = self.series_entry(id)?.footprint.as_ref()?.trade_stream_id;
        let stream = self.trade_stream(stream_id)?;
        Some(stream.presented_bar(stream.bar(bar_index)?).into_owned())
    }

    pub fn footprint_work_stats(&self, id: SeriesId) -> Option<FootprintWorkStats> {
        Some(
            self.trade_stream(self.series_entry(id)?.footprint.as_ref()?.trade_stream_id)?
                .work_stats(),
        )
    }

    pub fn set_footprint_trades(
        &mut self,
        id: SeriesId,
        trades: Vec<FootprintTrade>,
    ) -> Result<(), FootprintError> {
        self.validate_series_id(id).map_err(series_error)?;
        let stream_id = self
            .series_entry(id)
            .and_then(|series| series.footprint.as_ref())
            .map(|state| state.trade_stream_id)
            .ok_or(FootprintError::UnknownSeries(id))?;
        self.set_trade_stream_trades(stream_id, trades)
    }

    /// Atomically replace the canonical tape owned by a chart-level trade stream and refresh all
    /// footprint, candle/bar, study, and marker dependents from the resulting bar sequence.
    pub fn set_trade_stream_trades(
        &mut self,
        stream_id: u64,
        trades: Vec<FootprintTrade>,
    ) -> Result<(), FootprintError> {
        let mut next = self
            .trade_stream(stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?
            .clone();
        next.set_trades(trades)?;
        if next.tape_bar_times_collide() {
            return Err(FootprintError::ProjectionTimeCollision);
        }
        self.trade_streams.insert(stream_id, next);
        self.reset_trade_dependent_folds(stream_id);
        self.invalidate_profile_drawings_using_stream(stream_id);
        self.refresh_trade_dependents(stream_id)?;
        self.refresh_footprint_series_from_stream(stream_id, None)
    }

    pub fn update_footprint_trade(
        &mut self,
        id: SeriesId,
        trade: FootprintTrade,
    ) -> Result<FootprintUpdateKind, FootprintError> {
        self.update_footprint_trades(id, vec![trade])
    }

    /// Apply a live trade batch while synchronizing the shared time/scale projection once. A batch
    /// containing only tip events merges the active/recent bars in one data-layer operation;
    /// corrections or late events reconstruct once after the final canonical tape is known.
    pub fn update_footprint_trades(
        &mut self,
        id: SeriesId,
        trades: Vec<FootprintTrade>,
    ) -> Result<FootprintUpdateKind, FootprintError> {
        self.validate_series_id(id).map_err(series_error)?;
        if trades.is_empty() {
            return Ok(FootprintUpdateKind::Tip);
        }
        let stream_id = self
            .series_entry(id)
            .and_then(|series| series.footprint.as_ref())
            .map(|state| state.trade_stream_id)
            .ok_or(FootprintError::UnknownSeries(id))?;
        self.update_trade_stream_trades(stream_id, trades)
    }

    pub fn update_trade_stream_trade(
        &mut self,
        stream_id: u64,
        trade: FootprintTrade,
    ) -> Result<FootprintUpdateKind, FootprintError> {
        self.update_trade_stream_trades(stream_id, vec![trade])
    }

    /// Apply a live trade batch to the canonical stream once, then incrementally advance every
    /// dependent presentation from the same derived bars. Late events and corrections rebuild
    /// one candidate tape before any visible state is changed.
    pub fn update_trade_stream_trades(
        &mut self,
        stream_id: u64,
        trades: Vec<FootprintTrade>,
    ) -> Result<FootprintUpdateKind, FootprintError> {
        if trades.is_empty() {
            self.trade_stream(stream_id)
                .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
            return Ok(FootprintUpdateKind::Tip);
        }
        let stream = self
            .trade_stream(stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
        let options = stream.options();
        let batch_changes_visible_state = stream.batch_changes_visible_tape(&trades);
        validate_trade_batch(options, &trades)?;
        let historical = !stream.batch_is_tip(&trades);
        let previous_bar_count = stream.bars().len();
        if historical {
            // Auction marks re-detect from the bar of the earliest print the batch touches: a new
            // print or, for a corrected `trade_id`, the retained print it replaces. They are read
            // from the pre-merge bars; the merge below refuses a bar-time collision before it
            // changes anything, so it is applied completely or not at all.
            let earliest_trade = trades
                .iter()
                .flat_map(|trade| {
                    let original = trade
                        .trade_id
                        .and_then(|id| stream.stored_trade_timestamp(id));
                    [Some(trade.timestamp_micros), original]
                        .into_iter()
                        .flatten()
                })
                .min()
                .unwrap();
            let auction_from = stream.auction_repair_from(earliest_trade);
            let (result, first_bar) = self
                .trade_streams
                .get_mut(&stream_id)
                .ok_or(FootprintError::UnknownTradeStream(stream_id))?
                .merge_trades(trades, true)?;
            debug_assert_eq!(result, FootprintUpdateKind::Historical);
            if !batch_changes_visible_state {
                return Ok(FootprintUpdateKind::Historical);
            }
            // A suffix projection only upserts rows, so a correction that removed a bar
            // re-projects everything.
            let incremental_from = self
                .trade_stream(stream_id)
                .is_some_and(|stream| stream.bars().len() >= previous_bar_count)
                .then_some(first_bar);
            self.invalidate_profile_drawings_using_stream(stream_id);
            self.refresh_stream_presentations_from(
                stream_id,
                incremental_from,
                false,
                Some(auction_from),
            )?;
            return Ok(FootprintUpdateKind::Historical);
        }

        if stream.tip_bar_times_collide(&trades) {
            return Err(FootprintError::ProjectionTimeCollision);
        }
        let result = self
            .trade_streams
            .get_mut(&stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?
            .update_trades(trades)?;
        debug_assert_eq!(result, FootprintUpdateKind::Tip);
        if !batch_changes_visible_state {
            return Ok(result);
        }
        self.invalidate_profile_drawings_using_stream(stream_id);
        // Closed bars are immutable on the tip path: only the previously active bar and the bars
        // this batch opened change.
        let from = previous_bar_count.saturating_sub(1);
        self.refresh_stream_presentations_from(stream_id, Some(from), true, Some(from))?;
        Ok(result)
    }

    /// Re-project every presentation of a stream whose bars changed from `incremental_from` on
    /// (every bar when `None`). The footprint projection advances first because its retention
    /// ceiling may evict bars from the stream front; every other dependent then continues from
    /// the same stream bar on the evicted-adjusted index. `tip_append` is true only when the
    /// tape grew at its tip, which lets big trades continue instead of replaying the raw tape.
    /// Auction marks re-detect from stream bar `auction_from` (every bar when `None`); retention
    /// already dropped the marks of the evicted bars, so they continue on the adjusted index too.
    fn refresh_stream_presentations_from(
        &mut self,
        stream_id: u64,
        incremental_from: Option<usize>,
        tip_append: bool,
        auction_from: Option<usize>,
    ) -> Result<(), FootprintError> {
        let bars_before = self
            .trade_stream(stream_id)
            .map_or(0, |stream| stream.bars().len());
        self.refresh_footprint_series_from_stream(stream_id, incremental_from)?;
        let evicted = bars_before.saturating_sub(
            self.trade_stream(stream_id)
                .map_or(0, |stream| stream.bars().len()),
        );
        self.refresh_trade_dependents_from(
            stream_id,
            incremental_from.and_then(|from| from.checked_sub(evicted)),
            tip_append,
            auction_from.and_then(|from| from.checked_sub(evicted)),
        )
    }

    /// Advance every footprint series bound to `stream_id`. `Some(from)` is the live-tip path:
    /// stream bars before `from` are unchanged, so only `bars[from..]` is projected into each
    /// series, including under a retention ceiling. A trim inside one series' update evicts bars
    /// from the stream front, so later series continue from the evicted-adjusted index. `None`
    /// installs every complete projection.
    fn refresh_footprint_series_from_stream(
        &mut self,
        stream_id: u64,
        incremental_from: Option<usize>,
    ) -> Result<(), FootprintError> {
        let bars_at_start = self
            .trade_stream(stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?
            .bars()
            .len();
        let series_ids = self
            .series
            .iter()
            .filter(|series| !series.removed)
            .filter_map(|series| {
                series
                    .footprint
                    .as_ref()
                    .is_some_and(|state| state.trade_stream_id == stream_id)
                    .then_some(series.id)
            })
            .collect::<Vec<_>>();
        for id in series_ids {
            let stream = self
                .trade_stream(stream_id)
                .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
            let evicted = bars_at_start.saturating_sub(stream.bars().len());
            let from = incremental_from
                .and_then(|from| from.checked_sub(evicted))
                .map(|from| from.min(stream.bars().len()));
            let mut projected = 0;
            let mut updated = false;
            if let Some(from) = from {
                let projection = BarProjection::of(stream.options(), &stream.bars()[from..])?;
                projected += projection.len();
                updated = self.update_footprint_bars_projection(id, from, projection);
            }
            if !updated {
                let stream = self
                    .trade_stream(stream_id)
                    .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
                let projection = BarProjection::of(stream.options(), stream.bars())?;
                projected += projection.len();
                self.install_footprint_bars_projection(id, stream_id, projection)?;
            }
            self.record_dependent_work(stream_id, |work| {
                work.bar_rows_projected += projected as u64;
            });
            self.invalidate_frame_series(id);
        }
        Ok(())
    }

    pub(crate) fn footprint_capacity_bytes(&self) -> usize {
        self.series
            .iter()
            .filter_map(|series| series.footprint.as_ref())
            .filter_map(|state| self.trade_stream(state.trade_stream_id))
            .map(FootprintAggregator::capacity_bytes)
            .sum::<usize>()
            + self.big_trades_capacity_bytes()
    }

    pub(crate) fn trim_footprint_rows_front(&mut self, id: SeriesId, keep: usize) {
        let Some(stream_id) = self
            .series_entry(id)
            .and_then(|series| series.footprint.as_ref())
            .map(|state| state.trade_stream_id)
        else {
            return;
        };
        self.trim_trade_stream_front(stream_id, Some(id), keep);
    }

    /// Keep a trade stream's newest `keep` visible bars and trim every presentation of the stream
    /// to the same bar boundary in one data-layer transaction (`trim_stream_rows_front`). Raw
    /// bars inside the evicted range are sealed first, so big trades absorb their prints and the
    /// eviction itself releases no trade. `anchor` is the presentation whose rows `keep` counts:
    /// the footprint whose retention cap fired, or the first presentation of an order-flow graph;
    /// `None` trims only the stream and its big-trades indicators. Shared by series retention and
    /// the order-flow history budget.
    pub(crate) fn trim_trade_stream_front(
        &mut self,
        stream_id: u64,
        anchor: Option<SeriesId>,
        keep: usize,
    ) {
        let Some(stream) = self.trade_stream(stream_id) else {
            return;
        };
        let evict = stream
            .bars
            .len()
            .saturating_sub(keep.saturating_add(stream.hidden_bar_count()));
        let unsealed = evict.saturating_sub(stream.sealed_bars);
        self.seal_trade_stream_bars(stream_id, unsealed);
        self.evict_trade_dependents_front(stream_id, evict);
        if let Some(stream) = self.trade_streams.get_mut(&stream_id) {
            stream.evict_sealed_bars(evict);
        }
        // Every presentation leaves the data layer in one transaction, before the sidecar and
        // big-trades eviction below read the first retained row key from it.
        let presentations = anchor.map_or_else(Vec::new, |anchor| {
            self.trim_stream_rows_front(anchor, stream_id, keep)
        });
        let sequence_owner = self.trade_stream(stream_id).is_some_and(|stream| {
            !matches!(stream.options().bars, FootprintBarAggregation::Time { .. })
        });
        if sequence_owner && let Some(points) = self.sequence_points.as_mut() {
            if points.len() > keep {
                points.drain(..points.len() - keep);
            }
            for (index, point) in points.iter_mut().enumerate() {
                point.logical_index = index as u64;
            }
            // Every retention caller runs `sync_time_points` next, which reads this
            // sidecar and applies the exact viewport compensation for the trim. Syncing
            // the scale here as well would move the base index first and compensate twice.
        }
        // Big trades follow the evicted bars: they drop the orders that opened in them and keep
        // every other order and the automatic filter's state, without replaying the raw tape.
        // Auction marks drop the evicted bars' marks and keep the rest, without re-detecting.
        if evict > 0 {
            self.refresh_big_trades(stream_id, true);
            self.evict_auction_markers_front(stream_id, evict);
        }
        // A trimmed presentation lost rows outside its own write path, so the indicators and
        // resampled series reading it recompute from the retained rows, as a retention trim of
        // that series itself does. Otherwise a resampled tail refresh would keep bars aggregated
        // from evicted rows, and a full load, which installs every presentation before the
        // footprint trims them, would keep indicators computed over the evicted history. The rows
        // and the sequence sidecar are final here, so the time sync this runs sees exactly the
        // state the caller's own sync does.
        // ponytail: each consumed presentation recomputes (and time-syncs) on its own, where the
        // data-layer trim above runs once for all of them. One propagation over every trimmed
        // source would make this pass independent of the presentation count; deferred until a
        // stream with many consumed presentations measures slow.
        for series_id in presentations {
            let consumed = self.indicators.iter().any(|binding| {
                binding.source == series_id
                    || binding.volume_source == Some(series_id)
                    || binding.amount_source == Some(series_id)
            }) || self.resampled_series.values().any(|binding| {
                binding.source == series_id || binding.volume_source == Some(series_id)
            });
            if consumed {
                self.recompute_indicators_for(series_id);
            }
        }
    }

    /// Retention is about to evict the oldest `evicted` bars of a stream. Anchored
    /// cumulative-delta studies keep the base those bars established, so the values of the bars
    /// that remain never change when older history leaves the chart.
    fn evict_trade_dependents_front(&mut self, stream_id: u64, evicted: usize) {
        if evicted == 0 {
            return;
        }
        if let (Some(stream), Some(dependents)) = (
            self.trade_streams.get(&stream_id),
            self.trade_dependents.get_mut(&stream_id),
        ) {
            let evicted = &stream.bars()[..evicted.min(stream.bars().len())];
            for dependent in dependents {
                dependent.evict_front(evicted, stream.evicted_cumulative_delta());
            }
        }
    }

    /// Drop the footprint `id`'s oldest rows down to `keep` and, from every other presentation of
    /// its stream, the rows keyed before the footprint's first retained row, as one data-layer
    /// transaction: the shared time axis and every plot index rebuild once, not once per
    /// presentation. A live tip advances the projection before its studies and candles, which may
    /// therefore still lack the bars the tip appends; trimming by key keeps them aligned where a
    /// row count would not. Every other footprint bound to the stream drops the same rows, because
    /// footprint geometry reads stream bar `i` for row `i`. Returns the presentations that lost
    /// rows.
    ///
    /// Counts come from the rows each series exposes (up to the replay clock), so rows past the
    /// clock survive; the footprint's own trim does not depend on its stream existing.
    fn trim_stream_rows_front(
        &mut self,
        id: SeriesId,
        stream_id: u64,
        keep: usize,
    ) -> Vec<SeriesId> {
        let drop = self
            .data
            .series_rows(id)
            .map_or(0, |rows| rows.saturating_sub(keep));
        // The first row the footprint exposes after the trim (`None`: none is left to expose).
        let first_key = self
            .data
            .series_data(id)
            .and_then(|(times, _)| times.get(drop).copied());
        let mut presentations = self
            .stream_presentations(stream_id)
            .filter(|&series_id| series_id != id)
            .collect::<Vec<_>>();
        let mut trims = Vec::with_capacity(presentations.len() + 1);
        trims.push((id, keep));
        presentations.retain(|&series_id| {
            let Some((times, _)) = self.data.series_data(series_id) else {
                return false;
            };
            let evicted =
                first_key.map_or(times.len(), |key| times.partition_point(|&time| time < key));
            if evicted > 0 {
                let rows = self.data.series_rows(series_id).unwrap_or(0);
                trims.push((series_id, rows.saturating_sub(evicted)));
            }
            evicted > 0
        });
        self.data.trim_fronts(&trims);
        for &series_id in &presentations {
            self.invalidate_frame_series(series_id);
        }
        presentations
    }

    /// Read-only view of a chart trade stream: its raw tape, bars and retained history.
    pub fn trade_stream(&self, stream_id: u64) -> Option<&FootprintAggregator> {
        self.trade_streams.get(&stream_id)
    }

    pub(crate) fn is_trade_bar_dependent(&self, series_id: SeriesId) -> bool {
        self.trade_bar_dependents
            .values()
            .flatten()
            .any(|dependent| dependent.series_id == series_id)
    }

    pub(crate) fn prune_trade_stream_if_unused(&mut self, stream_id: u64) {
        let used = self.series.iter().any(|series| {
            !series.removed
                && series
                    .footprint
                    .as_ref()
                    .is_some_and(|state| state.trade_stream_id == stream_id)
        }) || self.trade_stream_keys.values().any(|&id| id == stream_id)
            || self
                .trade_bar_dependents
                .get(&stream_id)
                .is_some_and(|dependents| !dependents.is_empty())
            || self
                .trade_dependents
                .get(&stream_id)
                .is_some_and(|dependents| !dependents.is_empty())
            || self.big_trades_count(stream_id) > 0
            || self.auction_markers_count(stream_id) > 0;
        if !used {
            self.trade_streams.remove(&stream_id);
        }
    }

    fn register_trade_dependent(
        &mut self,
        stream_id: u64,
        kind: TradeStudyKind,
        series_id: SeriesId,
        options: TradeStudyOptions,
    ) {
        let dependents = self.trade_dependents.entry(stream_id).or_default();
        dependents.retain(|dependent| dependent.series_id != series_id);
        dependents.push(TradeStudyDependent {
            series_id,
            kind,
            options,
            applied_revision: 0,
            rebuilds: 0,
            incremental_updates: 0,
            anchor_base: None,
            resume: None,
        });
    }

    fn refresh_trade_dependents(&mut self, stream_id: u64) -> Result<(), FootprintError> {
        self.refresh_trade_dependents_from(stream_id, None, false, None)
    }

    /// Refresh every study, trade-bound candle/bar, big-trades and auction-marker dependent of a
    /// stream. `Some(from)`: stream bars before `from` are unchanged, so each study and candle
    /// works on `bars[from..]` only. `None` rebuilds every dependent from the stream. `tip_append`
    /// is true only when the tape grew at its tip, which lets big trades fold the new prints
    /// instead of replaying the raw tape. Auction marks re-detect from bar `auction_from` (every
    /// bar when `None`), which can precede `from` (a rewritten window or a corrected print).
    fn refresh_trade_dependents_from(
        &mut self,
        stream_id: u64,
        incremental_from: Option<usize>,
        tip_append: bool,
        auction_from: Option<usize>,
    ) -> Result<(), FootprintError> {
        self.trade_stream(stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
        let studies = self.trade_dependents.get(&stream_id).map_or(0, Vec::len);
        for index in 0..studies {
            self.refresh_trade_study(stream_id, index, incremental_from)?;
        }
        self.refresh_trade_bar_dependents_from(stream_id, incremental_from)?;
        self.refresh_big_trades(stream_id, tip_append);
        self.refresh_auction_markers(stream_id, auction_from);
        Ok(())
    }

    /// Recompute one CVD, delta, or volume study. The incremental path writes only `bars[from..]`
    /// through the tail update path; cumulative delta resumes its cached running state, falling
    /// back to the retention seed, so both paths step exactly the fold a clean rebuild steps.
    fn refresh_trade_study(
        &mut self,
        stream_id: u64,
        index: usize,
        incremental_from: Option<usize>,
    ) -> Result<(), FootprintError> {
        let Some(dependent) = self
            .trade_dependents
            .get(&stream_id)
            .and_then(|dependents| dependents.get(index))
            .copied()
        else {
            return Ok(());
        };
        let key_base = self.sequence_key_base(stream_id);
        let stream = self
            .trade_stream(stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
        let bars = stream.bars();
        let revision = stream.revision();
        let from = incremental_from.map_or(0, |from| from.min(bars.len()));
        let suffix = &bars[from..];
        let times = key_base.map_or_else(
            || {
                suffix
                    .iter()
                    .map(|bar| bar.start_timestamp_micros.div_euclid(MICROS_PER_SECOND))
                    .collect::<Vec<_>>()
            },
            |base| (from..bars.len()).map(|row| base + row as i64).collect(),
        );
        let mut computed = suffix.len();
        let mut resume = None;
        let values = match dependent.kind {
            TradeStudyKind::CumulativeDelta => {
                let seed = CumulativeDeltaFold {
                    cumulative: stream.evicted_cumulative_delta(),
                    anchor_base: dependent.anchor_base,
                };
                let (start, mut fold) = dependent
                    .resume
                    .filter(|&(row, _)| incremental_from.is_some() && row <= from)
                    .unwrap_or((0, seed));
                for bar in &bars[start..from] {
                    fold.step(bar, dependent.options);
                }
                computed += from - start;
                // The last bar stays active; cache the state before it for the next tip.
                let snapshot = bars.len().saturating_sub(1).max(from);
                resume = Some((from, fold));
                let mut values = Vec::with_capacity(suffix.len());
                for (row, bar) in (from..).zip(suffix) {
                    if row == snapshot {
                        resume = Some((row, fold));
                    }
                    values.push(fold.step(bar, dependent.options));
                }
                values
            }
            TradeStudyKind::DeltaHistogram => suffix.iter().map(|bar| bar.delta).collect(),
            TradeStudyKind::Volume => suffix.iter().map(|bar| bar.total_volume).collect(),
        };
        let (open, high, low, colors) = match dependent.kind {
            TradeStudyKind::CumulativeDelta => {
                (values.clone(), values.clone(), values.clone(), None)
            }
            TradeStudyKind::DeltaHistogram | TradeStudyKind::Volume => (
                vec![0.0; values.len()],
                values.iter().map(|value| value.max(0.0)).collect(),
                values.iter().map(|value| value.min(0.0)).collect(),
                // Volume columns take their up/down tint from the series' `histogram_updown`.
                matches!(dependent.kind, TradeStudyKind::DeltaHistogram).then(|| {
                    values
                        .iter()
                        .copied()
                        .map(delta_body_color)
                        .collect::<Vec<_>>()
                }),
            ),
        };
        let series_id = dependent.series_id;
        let rows = values.len();
        let incremental = incremental_from.is_some();
        if incremental {
            if self.update_series_bars_sanitized_inner(series_id, times, open, high, low, values)
                == 0
            {
                // The study rows no longer line up with the stream: rebuild this study.
                return self.refresh_trade_study(stream_id, index, None);
            }
            if let Some(colors) = colors {
                // The replaced and appended rows are the study's newest rows.
                let total = self
                    .data
                    .series_memory_usage(series_id)
                    .map_or(0, |usage| usage.rows);
                for (row, color) in (total.saturating_sub(rows)..).zip(colors) {
                    self.data
                        .set_point_color(series_id, PointColorChannel::Body, row, color);
                }
            }
        } else {
            if !self.install_series_data_inner(series_id, times, open, high, low, values) {
                return Err(FootprintError::UnknownSeries(series_id));
            }
            if let Some(mut colors) = colors {
                // Delta is an engine-owned source series, so install its sign palette through the
                // same internal data owner immediately after the data mutation that resets colors.
                // A host retention cap on the study keeps only the newest rows.
                let rows = self
                    .data
                    .series_memory_usage(series_id)
                    .map_or(0, |usage| usage.rows);
                colors.drain(..colors.len().saturating_sub(rows));
                let installed = self
                    .data
                    .set_point_colors(series_id, [Some(colors), None, None]);
                debug_assert!(installed, "one sign color per retained delta row");
            }
        }
        if let Some(stored) = self
            .trade_dependents
            .get_mut(&stream_id)
            .and_then(|dependents| dependents.get_mut(index))
        {
            stored.resume = resume;
            if stored.applied_revision != revision {
                stored.applied_revision = revision;
                if incremental {
                    stored.incremental_updates = stored.incremental_updates.saturating_add(1);
                } else {
                    stored.rebuilds = stored.rebuilds.saturating_add(1);
                }
            }
        }
        self.record_dependent_work(stream_id, |work| {
            work.study_rows_computed += computed as u64;
        });
        Ok(())
    }

    /// Project the stream's bars into every trade-bound candlestick/OHLC-bar series. The
    /// incremental path projects only `bars[from..]`; a rejected tail reinstalls that series.
    fn refresh_trade_bar_dependents_from(
        &mut self,
        stream_id: u64,
        incremental_from: Option<usize>,
    ) -> Result<(), FootprintError> {
        let count = self
            .trade_bar_dependents
            .get(&stream_id)
            .map_or(0, Vec::len);
        for index in 0..count {
            let Some(series_id) = self
                .trade_bar_dependents
                .get(&stream_id)
                .and_then(|dependents| dependents.get(index))
                .map(|dependent| dependent.series_id)
            else {
                break;
            };
            let stream = self
                .trade_stream(stream_id)
                .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
            let revision = stream.revision();
            let mut projected = 0;
            let mut updated = false;
            if let Some(from) = incremental_from.map(|from| from.min(stream.bars().len())) {
                let projection = BarProjection::of(stream.options(), &stream.bars()[from..])?;
                projected += projection.len();
                updated = self.update_trade_bar_projection(series_id, from, projection);
            }
            if !updated {
                let stream = self
                    .trade_stream(stream_id)
                    .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
                let projection = BarProjection::of(stream.options(), stream.bars())?;
                projected += projection.len();
                self.install_trade_bar_projection(series_id, stream_id, projection)?;
            }
            self.record_dependent_work(stream_id, |work| {
                work.bar_rows_projected += projected as u64;
            });
            if let Some(stored) = self
                .trade_bar_dependents
                .get_mut(&stream_id)
                .and_then(|dependents| dependents.get_mut(index))
                && stored.applied_revision != revision
            {
                stored.applied_revision = revision;
                if updated {
                    stored.incremental_updates = stored.incremental_updates.saturating_add(1);
                } else {
                    stored.rebuilds = stored.rebuilds.saturating_add(1);
                }
            }
        }
        Ok(())
    }

    /// The stream's tape was replaced: anchored bases and every resumable fold restart.
    fn reset_trade_dependent_folds(&mut self, stream_id: u64) {
        for dependent in self
            .trade_dependents
            .get_mut(&stream_id)
            .into_iter()
            .flatten()
        {
            dependent.anchor_base = None;
            dependent.resume = None;
        }
    }

    pub(crate) fn record_dependent_work(
        &mut self,
        stream_id: u64,
        record: impl FnOnce(&mut TradeDependentWork),
    ) {
        if let Some(stream) = self.trade_streams.get_mut(&stream_id) {
            record(&mut stream.dependent_work);
        }
    }

    /// Every live series presenting a stream: bound footprints, trade-bound candles/bars, and
    /// CVD/delta studies. All of them hold one row per stream bar under the same keys.
    pub(crate) fn stream_presentations(
        &self,
        stream_id: u64,
    ) -> impl Iterator<Item = SeriesId> + '_ {
        self.series
            .iter()
            .filter(move |series| {
                !series.removed
                    && series
                        .footprint
                        .as_ref()
                        .is_some_and(|state| state.trade_stream_id == stream_id)
            })
            .map(|series| series.id)
            .chain(
                self.trade_bar_dependents
                    .get(&stream_id)
                    .into_iter()
                    .flatten()
                    .map(|dependent| dependent.series_id),
            )
            .chain(
                self.trade_dependents
                    .get(&stream_id)
                    .into_iter()
                    .flatten()
                    .map(|dependent| dependent.series_id),
            )
    }

    /// Row key of stream bar 0 on a non-time sequence axis, `None` on a time axis. Rows are keyed
    /// contiguously and a retention trim drops keys from the front without re-keying, so every
    /// sequence-axis writer (projection, candles, studies, big trades) continues from the first key
    /// the stream's presentations hold, including a presentation bound after a trim.
    pub(crate) fn sequence_key_base(&self, stream_id: u64) -> Option<i64> {
        let stream = self.trade_stream(stream_id)?;
        if matches!(stream.options().bars, FootprintBarAggregation::Time { .. }) {
            return None;
        }
        Some(
            self.stream_presentations(stream_id)
                .find_map(|id| {
                    self.data
                        .series_data(id)
                        .and_then(|(times, _)| times.first().copied())
                })
                .unwrap_or(0),
        )
    }

    /// Key base a complete sequence-axis install of a stream presentation uses. While the chart's
    /// sequence axis is live, a presentation installed after retention trims (a late binding or
    /// a rebind) continues from the stream's retained key; a fresh axis starts at zero.
    fn sequence_install_key_base(&self, stream_id: u64) -> i64 {
        self.sequence_points()
            .and_then(|_| self.sequence_key_base(stream_id))
            .unwrap_or(0)
    }
}

const MAXIMUM_ROW_MERGE: u32 = 1_000_000_000;

/// Smallest 1-2-5 multiple of the stored row that is at least `minimum_px` tall when one stored
/// row is `row_px` tall.
#[must_use]
pub fn footprint_row_merge(row_px: f64, minimum_px: f64) -> u32 {
    if !row_px.is_finite() || row_px <= 0.0 || row_px >= minimum_px {
        return 1;
    }
    let wanted = minimum_px / row_px;
    let mut decade = 1_u32;
    while decade <= MAXIMUM_ROW_MERGE / 5 {
        for step in [1, 2, 5] {
            let candidate = decade * step;
            if f64::from(candidate) >= wanted {
                return candidate;
            }
        }
        decade *= 10;
    }
    MAXIMUM_ROW_MERGE
}

/// One bar's levels regrouped into rows of `merge` stored rows, with imbalances, stacks, and POC
/// recomputed on the display rows so every highlight describes what is drawn.
pub(crate) fn merged_footprint_bar(
    bar: &FootprintBar,
    merge: u32,
    imbalance: FootprintImbalanceOptions,
    display_row_size: f64,
) -> FootprintBar {
    let merge = i64::from(merge.max(1));
    let mut levels: Vec<FootprintLevel> = Vec::new();
    for level in &bar.levels {
        let row = level.level.div_euclid(merge);
        match levels.last_mut() {
            Some(last) if last.level == row => {
                last.bid_volume += level.bid_volume;
                last.ask_volume += level.ask_volume;
                last.unknown_volume += level.unknown_volume;
                last.total_volume += level.total_volume;
                last.delta += level.delta;
            }
            _ => levels.push(FootprintLevel {
                level: row,
                price: row as f64 * display_row_size,
                bid_volume: level.bid_volume,
                ask_volume: level.ask_volume,
                unknown_volume: level.unknown_volume,
                total_volume: level.total_volume,
                delta: level.delta,
                ..FootprintLevel::default()
            }),
        }
    }
    let mut merged = FootprintBar {
        levels,
        ..bar.clone_without_levels()
    };
    recompute_bar_derived(&mut merged, imbalance);
    merged
}

/// The POC price [`merged_footprint_bar`] computes for the same rows, without building them.
pub(crate) fn merged_poc_price(bar: &FootprintBar, merge: u32, display_row_size: f64) -> f64 {
    let merge = i64::from(merge.max(1));
    if merge == 1 {
        return bar.poc_price;
    }
    let distance = |row: i64| (row as f64 * display_row_size - bar.close).abs();
    bar.levels
        .chunk_by(|left, right| left.level.div_euclid(merge) == right.level.div_euclid(merge))
        .map(|levels| {
            let total = levels
                .iter()
                .fold(0.0, |sum, level| sum + level.total_volume);
            (levels[0].level.div_euclid(merge), total)
        })
        .max_by(|(left_row, left_total), (right_row, right_total)| {
            left_total
                .total_cmp(right_total)
                .then_with(|| distance(*right_row).total_cmp(&distance(*left_row)))
                .then_with(|| right_row.cmp(left_row))
        })
        .map_or(bar.poc_price, |(row, _)| row as f64 * display_row_size)
}

/// One bar built from the trades of `earlier` followed by those of `later`, exactly as if they
/// had been aggregated together. The delta path continues from `earlier`'s close, and the
/// closing session delta is `later`'s, which already includes `earlier`.
fn joined_footprint_bar(
    earlier: &FootprintBar,
    later: &FootprintBar,
    imbalance: FootprintImbalanceOptions,
) -> FootprintBar {
    let mut levels = Vec::with_capacity(earlier.levels.len() + later.levels.len());
    let (mut left, mut right) = (
        earlier.levels.iter().peekable(),
        later.levels.iter().peekable(),
    );
    loop {
        let next = match (left.peek().copied(), right.peek().copied()) {
            (Some(a), Some(b)) if a.level == b.level => {
                left.next();
                right.next();
                FootprintLevel {
                    bid_volume: a.bid_volume + b.bid_volume,
                    ask_volume: a.ask_volume + b.ask_volume,
                    unknown_volume: a.unknown_volume + b.unknown_volume,
                    total_volume: a.total_volume + b.total_volume,
                    ..*a
                }
            }
            (Some(a), Some(b)) if a.level < b.level => {
                left.next();
                *a
            }
            (Some(a), None) => {
                left.next();
                *a
            }
            (_, Some(b)) => {
                right.next();
                *b
            }
            (None, None) => break,
        };
        levels.push(FootprintLevel {
            delta: next.ask_volume - next.bid_volume,
            ..next
        });
    }
    let mut joined = FootprintBar {
        start_timestamp_micros: earlier.start_timestamp_micros,
        open: earlier.open,
        high: earlier.high.max(later.high),
        low: earlier.low.min(later.low),
        bid_volume: earlier.bid_volume + later.bid_volume,
        ask_volume: earlier.ask_volume + later.ask_volume,
        unknown_volume: earlier.unknown_volume + later.unknown_volume,
        total_volume: earlier.total_volume + later.total_volume,
        delta: earlier.delta + later.delta,
        max_delta: earlier.max_delta.max(earlier.delta + later.max_delta),
        min_delta: earlier.min_delta.min(earlier.delta + later.min_delta),
        trade_count: earlier.trade_count.saturating_add(later.trade_count),
        levels,
        ..later.clone_without_levels()
    };
    recompute_bar_derived(&mut joined, imbalance);
    joined
}

/// Running cumulative-delta state after a prefix of bars. Clean rebuilds and live tips step the
/// same fold, so their floating-point results are identical.
#[derive(Clone, Copy, Debug, Default)]
struct CumulativeDeltaFold {
    cumulative: f64,
    anchor_base: Option<f64>,
}

impl CumulativeDeltaFold {
    fn step(&mut self, bar: &FootprintBar, options: TradeStudyOptions) -> f64 {
        self.cumulative += bar.delta;
        match options.cumulative_delta_reset {
            CumulativeDeltaReset::Session => bar.session_delta,
            CumulativeDeltaReset::Continuous => self.cumulative,
            CumulativeDeltaReset::Anchored => {
                if options
                    .anchor_timestamp_micros
                    .is_some_and(|anchor| bar.end_timestamp_micros < anchor)
                {
                    0.0
                } else {
                    let base = *self.anchor_base.get_or_insert(self.cumulative - bar.delta);
                    self.cumulative - base
                }
            }
        }
    }
}

impl TradeStudyDependent {
    /// Retention is evicting `evicted` from the stream front, whose evicted cumulative delta is
    /// `cumulative` before them. Carry an anchored base those bars establish so the retained
    /// values keep their full-history base, as the stream seeds session and continuous delta.
    /// Evicting every bar keeps that seed too: bars built later continue the same history.
    fn evict_front(&mut self, evicted: &[FootprintBar], cumulative: f64) {
        if evicted.is_empty() {
            return;
        }
        let mut fold = CumulativeDeltaFold {
            cumulative,
            anchor_base: self.anchor_base,
        };
        for bar in evicted {
            fold.step(bar, self.options);
        }
        self.anchor_base = fold.anchor_base;
        self.resume = self
            .resume
            .and_then(|(row, fold)| row.checked_sub(evicted.len()).map(|row| (row, fold)));
    }
}

/// Data-layer columns for one contiguous run of stream bars.
enum BarProjection {
    /// Time bars keyed by their UTC-second open.
    Time(ProjectionColumns),
    /// Non-time bars carrying their full-resolution sequence points.
    Sequence(SequenceProjectionColumns),
}

impl BarProjection {
    fn of(
        options: FootprintAggregationOptions,
        bars: &[FootprintBar],
    ) -> Result<Self, FootprintError> {
        if matches!(options.bars, FootprintBarAggregation::Time { .. }) {
            projection_columns(bars).map(Self::Time)
        } else {
            Ok(Self::Sequence(sequence_projection_columns(bars)))
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Time((times, ..)) => times.len(),
            Self::Sequence((points, ..)) => points.len(),
        }
    }
}

/// Delta histogram body color: market up for non-negative delta, market down otherwise.
fn delta_body_color(value: f64) -> u32 {
    let rgb = if value >= 0.0 {
        MARKET_UP_RGB
    } else {
        MARKET_DOWN_RGB
    };
    Color::rgb(rgb.0, rgb.1, rgb.2).0
}

fn validate_chart_projection(options: FootprintAggregationOptions) -> Result<(), FootprintError> {
    match options.bars {
        FootprintBarAggregation::Time {
            interval_micros,
            anchor_micros,
        } if interval_micros >= MICROS_PER_SECOND as u64
            && interval_micros.is_multiple_of(MICROS_PER_SECOND as u64)
            && anchor_micros % MICROS_PER_SECOND == 0 =>
        {
            Ok(())
        }
        FootprintBarAggregation::Trades { trades_per_bar } => (trades_per_bar > 0)
            .then_some(())
            .ok_or(FootprintError::UnsupportedChartAggregation),
        FootprintBarAggregation::Volume { volume_per_bar } => (volume_per_bar.is_finite()
            && volume_per_bar > 0.0)
            .then_some(())
            .ok_or(FootprintError::UnsupportedChartAggregation),
        FootprintBarAggregation::Range { range_ticks } => (range_ticks > 0)
            .then_some(())
            .ok_or(FootprintError::UnsupportedChartAggregation),
        FootprintBarAggregation::Time { .. } => Err(FootprintError::UnsupportedChartAggregation),
    }
}

fn validate_visual_options(options: &FootprintVisualOptions) -> Result<(), FootprintError> {
    if options.font_size.is_finite() && (6.0..=48.0).contains(&options.font_size) {
        Ok(())
    } else {
        Err(FootprintError::InvalidVisualOptions)
    }
}

fn footprint_price_format(tick_size: f64) -> SeriesPriceFormat {
    let precision = (0..=15)
        .find(|precision| {
            let scaled = tick_size * 10_f64.powi(*precision as i32);
            (scaled - scaled.round()).abs() <= scaled.abs().max(1.0) * 1e-10
        })
        .unwrap_or(15);
    SeriesPriceFormat {
        kind: PriceFormatKind::Price,
        precision,
        min_move: tick_size,
        formatter: None,
        tick_ladder: None,
    }
}

/// Price extent of the rows containing `low` and `high`, padded half a tick so the lowest
/// and highest ticks sit inside their rows.
pub(crate) fn footprint_row_price_bounds(
    options: &FootprintAggregationOptions,
    low: f64,
    high: f64,
) -> (f64, f64) {
    let row_size = options.row_size();
    let half_tick = options.tick_size / 2.0;
    (
        options.row_level(low) as f64 * row_size - half_tick,
        options.row_level(high) as f64 * row_size + row_size - half_tick,
    )
}

fn series_error(error: SeriesIdError) -> FootprintError {
    match error {
        SeriesIdError::Unknown(id) => FootprintError::UnknownSeries(id),
        SeriesIdError::Stale(id) => FootprintError::StaleSeries(id),
    }
}

type ProjectionColumns = (Vec<i64>, Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>);
type SequenceProjectionColumns = (
    Vec<BarSequencePoint>,
    Vec<f64>,
    Vec<f64>,
    Vec<f64>,
    Vec<f64>,
);

fn projection_columns(bars: &[FootprintBar]) -> Result<ProjectionColumns, FootprintError> {
    let mut times = Vec::with_capacity(bars.len());
    let mut open = Vec::with_capacity(bars.len());
    let mut high = Vec::with_capacity(bars.len());
    let mut low = Vec::with_capacity(bars.len());
    let mut close = Vec::with_capacity(bars.len());
    for bar in bars {
        let time = bar.start_timestamp_micros.div_euclid(MICROS_PER_SECOND);
        if times.last() == Some(&time) {
            return Err(FootprintError::ProjectionTimeCollision);
        }
        times.push(time);
        open.push(bar.open);
        high.push(bar.high);
        low.push(bar.low);
        close.push(bar.close);
    }
    Ok((times, open, high, low, close))
}

fn sequence_projection_columns(bars: &[FootprintBar]) -> SequenceProjectionColumns {
    let mut points = Vec::with_capacity(bars.len());
    let mut open = Vec::with_capacity(bars.len());
    let mut high = Vec::with_capacity(bars.len());
    let mut low = Vec::with_capacity(bars.len());
    let mut close = Vec::with_capacity(bars.len());
    for bar in bars {
        points.push(BarSequencePoint {
            logical_index: bar.logical_index,
            open_timestamp_micros: bar.start_timestamp_micros,
            close_timestamp_micros: bar.end_timestamp_micros,
        });
        open.push(bar.open);
        high.push(bar.high);
        low.push(bar.low);
        close.push(bar.close);
    }
    (points, open, high, low, close)
}

fn validate_options(options: FootprintAggregationOptions) -> Result<(), FootprintError> {
    if !options.tick_size.is_finite() || options.tick_size <= 0.0 {
        return Err(FootprintError::InvalidTickSize);
    }
    if options.ticks_per_row == 0 || !options.row_size().is_finite() {
        return Err(FootprintError::InvalidAggregation);
    }
    let valid_bars = match options.bars {
        FootprintBarAggregation::Time {
            interval_micros,
            anchor_micros,
        } => {
            let timestamp_span = (MAX_TIMESTAMP_MICROS - MIN_TIMESTAMP_MICROS) as u64;
            interval_micros > 0
                && interval_micros <= timestamp_span
                && (MIN_TIMESTAMP_MICROS..=MAX_TIMESTAMP_MICROS).contains(&anchor_micros)
        }
        FootprintBarAggregation::Trades { trades_per_bar } => trades_per_bar > 0,
        FootprintBarAggregation::Volume { volume_per_bar } => {
            volume_per_bar.is_finite() && volume_per_bar > 0.0
        }
        FootprintBarAggregation::Range { range_ticks } => range_ticks > 0,
    };
    if !valid_bars {
        return Err(FootprintError::InvalidAggregation);
    }
    let imbalance = options.imbalance;
    if !imbalance.ratio.is_finite()
        || imbalance.ratio < 1.0
        || !imbalance.minimum_volume.is_finite()
        || imbalance.minimum_volume < 0.0
        || imbalance.consecutive_levels == 0
    {
        return Err(FootprintError::InvalidImbalance);
    }
    Ok(())
}

fn validate_trade_batch(
    options: FootprintAggregationOptions,
    input: &[FootprintTrade],
) -> Result<(), FootprintError> {
    validate_options(options)?;
    let mut ids = HashMap::with_capacity(input.len());
    for (index, trade) in input.iter().enumerate() {
        validate_trade(options, trade, index)?;
        if let Some(trade_id) = trade.trade_id
            && ids.insert(trade_id, index).is_some()
        {
            return Err(FootprintError::DuplicateTradeId { trade_id });
        }
    }
    Ok(())
}

fn validate_trade(
    options: FootprintAggregationOptions,
    trade: &FootprintTrade,
    index: usize,
) -> Result<(), FootprintError> {
    if !(MIN_TIMESTAMP_MICROS..=MAX_TIMESTAMP_MICROS).contains(&trade.timestamp_micros) {
        return Err(FootprintError::InvalidTimestamp { index });
    }
    if !trade.price.is_finite() || !(MIN_SAFE_VALUE..=MAX_SAFE_VALUE).contains(&trade.price) {
        return Err(FootprintError::InvalidPrice { index });
    }
    let scaled_price = trade.price / options.tick_size;
    if !scaled_price.is_finite() || scaled_price < i64::MIN as f64 || scaled_price > i64::MAX as f64
    {
        return Err(FootprintError::InvalidPrice { index });
    }
    let level = price_level(trade.price, options.tick_size);
    let snapped = level as f64 * options.tick_size;
    let tolerance =
        options.tick_size.abs() * 1e-9 + f64::EPSILON * trade.price.abs().max(1.0) * 4.0;
    if (snapped - trade.price).abs() > tolerance {
        return Err(FootprintError::OffGridPrice { index });
    }
    if !trade.volume.is_finite() || trade.volume <= 0.0 || trade.volume > MAX_SAFE_VALUE {
        return Err(FootprintError::InvalidVolume { index });
    }
    if trade.bid.is_some_and(|price| {
        !price.is_finite() || !(MIN_SAFE_VALUE..=MAX_SAFE_VALUE).contains(&price)
    }) || trade.ask.is_some_and(|price| {
        !price.is_finite() || !(MIN_SAFE_VALUE..=MAX_SAFE_VALUE).contains(&price)
    }) || matches!((trade.bid, trade.ask), (Some(bid), Some(ask)) if bid > ask)
    {
        return Err(FootprintError::InvalidQuote { index });
    }
    Ok(())
}

fn trade_order_key(trade: &StoredTrade) -> (i64, u64, u64) {
    (
        trade.event.timestamp_micros,
        trade.event.sequence.unwrap_or(u64::MAX),
        trade.input_order,
    )
}

fn classify_aggressor(
    trade: &FootprintTrade,
    previous_price: Option<f64>,
    previous_side: AggressorSide,
) -> AggressorSide {
    if trade.aggressor != AggressorSide::Unknown {
        return trade.aggressor;
    }
    if trade.ask.is_some_and(|ask| trade.price >= ask) {
        return AggressorSide::Buy;
    }
    if trade.bid.is_some_and(|bid| trade.price <= bid) {
        return AggressorSide::Sell;
    }
    match previous_price.and_then(|previous| trade.price.partial_cmp(&previous)) {
        Some(core::cmp::Ordering::Greater) => AggressorSide::Buy,
        Some(core::cmp::Ordering::Less) => AggressorSide::Sell,
        Some(core::cmp::Ordering::Equal) => previous_side,
        None => AggressorSide::Unknown,
    }
}

fn price_level(price: f64, tick_size: f64) -> i64 {
    (price / tick_size).round() as i64
}

fn aligned_bucket_start(timestamp: i64, interval: i64, anchor: i64) -> i64 {
    anchor + (timestamp - anchor).div_euclid(interval) * interval
}

fn recompute_bar_derived(bar: &mut FootprintBar, options: FootprintImbalanceOptions) {
    bar.delta_percent = if bar.total_volume > 0.0 {
        bar.delta / bar.total_volume * 100.0
    } else {
        0.0
    };
    for level in &mut bar.levels {
        level.bid_imbalance = false;
        level.ask_imbalance = false;
        level.stacked_bid_imbalance = false;
        level.stacked_ask_imbalance = false;
    }
    for index in 0..bar.levels.len() {
        let level = bar.levels[index].level;
        let lower_bid = index
            .checked_sub(1)
            .and_then(|lower| {
                (bar.levels[lower].level == level - 1).then_some(bar.levels[lower].bid_volume)
            })
            .unwrap_or(0.0);
        let upper_ask = bar
            .levels
            .get(index + 1)
            .filter(|upper| upper.level == level + 1)
            .map_or(0.0, |upper| upper.ask_volume);
        let ask = bar.levels[index].ask_volume;
        let bid = bar.levels[index].bid_volume;
        bar.levels[index].ask_imbalance = dominant(ask, lower_bid, options);
        bar.levels[index].bid_imbalance = dominant(bid, upper_ask, options);
    }
    mark_stacks(&mut bar.levels, options.consecutive_levels as usize, true);
    mark_stacks(&mut bar.levels, options.consecutive_levels as usize, false);

    if let Some(poc) = bar.levels.iter().max_by(|left, right| {
        left.total_volume
            .total_cmp(&right.total_volume)
            .then_with(|| {
                let left_distance = (left.price - bar.close).abs();
                let right_distance = (right.price - bar.close).abs();
                right_distance.total_cmp(&left_distance)
            })
            .then_with(|| right.level.cmp(&left.level))
    }) {
        bar.poc_level = poc.level;
        bar.poc_price = poc.price;
    }
}

fn dominant(value: f64, opposite: f64, options: FootprintImbalanceOptions) -> bool {
    value >= options.minimum_volume && (opposite == 0.0 || value / opposite >= options.ratio)
}

fn mark_stacks(levels: &mut [FootprintLevel], minimum: usize, ask: bool) {
    let mut start = 0;
    while start < levels.len() {
        let imbalanced = if ask {
            levels[start].ask_imbalance
        } else {
            levels[start].bid_imbalance
        };
        if !imbalanced {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < levels.len()
            && levels[end].level == levels[end - 1].level + 1
            && if ask {
                levels[end].ask_imbalance
            } else {
                levels[end].bid_imbalance
            }
        {
            end += 1;
        }
        if end - start >= minimum {
            for level in &mut levels[start..end] {
                if ask {
                    level.stacked_ask_imbalance = true;
                } else {
                    level.stacked_bid_imbalance = true;
                }
            }
        }
        start = end;
    }
}

#[cfg(test)]
mod tip_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawingKind, DrawingPoint};
    use aeris_charts_render::draw_list::{LineStyle, Prim};

    fn trade(
        timestamp_micros: i64,
        price: f64,
        volume: f64,
        side: AggressorSide,
    ) -> FootprintTrade {
        FootprintTrade {
            timestamp_micros,
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

    #[test]
    fn default_positive_footprint_roles_share_the_market_up_hue() {
        let visual = FootprintVisualOptions::default();
        let market_up = (MARKET_UP_RGB.0, MARKET_UP_RGB.1, MARKET_UP_RGB.2);
        for color in [
            visual.ask_color,
            visual.positive_delta_color,
            visual.stacked_ask_color,
        ] {
            assert_eq!((color.r(), color.g(), color.b()), market_up);
        }
        assert_eq!(visual.ask_color.a(), 70);
        assert_eq!(visual.positive_delta_color.a(), 110);
        assert_eq!(visual.stacked_ask_color.a(), 255);
    }

    #[test]
    fn tick_truth_drives_levels_poc_and_non_final_delta_extrema() {
        let mut aggregator = FootprintAggregator::new(FootprintAggregationOptions {
            tick_size: 1.0,
            ticks_per_row: 1,
            bars: FootprintBarAggregation::Time {
                interval_micros: 60_000_000,
                anchor_micros: 0,
            },
            imbalance: FootprintImbalanceOptions {
                ratio: 10.0,
                minimum_volume: 1_000.0,
                consecutive_levels: 3,
            },
        })
        .unwrap();
        aggregator
            .set_trades(vec![
                trade(1_000_000, 100.0, 10.0, AggressorSide::Buy),
                trade(2_000_000, 101.0, 7.0, AggressorSide::Sell),
                trade(3_000_000, 101.0, 8.0, AggressorSide::Sell),
                trade(4_000_000, 100.0, 6.0, AggressorSide::Buy),
            ])
            .unwrap();

        let bar = &aggregator.bars()[0];
        assert_eq!(
            (bar.bid_volume, bar.ask_volume, bar.total_volume),
            (15.0, 16.0, 31.0)
        );
        assert_eq!((bar.delta, bar.max_delta, bar.min_delta), (1.0, 10.0, -5.0));
        assert_eq!(
            (bar.open, bar.high, bar.low, bar.close),
            (100.0, 101.0, 100.0, 100.0)
        );
        assert_eq!((bar.poc_price, bar.session_delta), (100.0, 1.0));
        assert_eq!(bar.levels.len(), 2);
        assert_eq!(
            (bar.levels[0].bid_volume, bar.levels[0].ask_volume),
            (0.0, 16.0)
        );
        assert_eq!(
            (bar.levels[1].bid_volume, bar.levels[1].ask_volume),
            (15.0, 0.0)
        );
    }

    #[test]
    fn stacked_imbalances_mark_complete_bid_and_ask_runs() {
        let options = FootprintAggregationOptions {
            tick_size: 1.0,
            ticks_per_row: 1,
            bars: FootprintBarAggregation::Time {
                interval_micros: 60_000_000,
                anchor_micros: 0,
            },
            imbalance: FootprintImbalanceOptions {
                ratio: 3.0,
                minimum_volume: 20.0,
                consecutive_levels: 2,
            },
        };
        let mut ask = FootprintAggregator::new(options).unwrap();
        ask.set_trades(vec![
            trade(1, 100.0, 10.0, AggressorSide::Sell),
            trade(2, 101.0, 40.0, AggressorSide::Buy),
            trade(3, 101.0, 10.0, AggressorSide::Sell),
            trade(4, 102.0, 50.0, AggressorSide::Buy),
        ])
        .unwrap();
        let levels = &ask.bars()[0].levels;
        assert!(!levels[0].stacked_ask_imbalance);
        assert!(levels[1].ask_imbalance && levels[1].stacked_ask_imbalance);
        assert!(levels[2].ask_imbalance && levels[2].stacked_ask_imbalance);

        let mut bid = FootprintAggregator::new(options).unwrap();
        bid.set_trades(vec![
            trade(1, 100.0, 50.0, AggressorSide::Sell),
            trade(2, 101.0, 10.0, AggressorSide::Buy),
            trade(3, 101.0, 40.0, AggressorSide::Sell),
            trade(4, 102.0, 10.0, AggressorSide::Buy),
        ])
        .unwrap();
        let levels = &bid.bars()[0].levels;
        assert!(levels[0].bid_imbalance && levels[0].stacked_bid_imbalance);
        assert!(levels[1].bid_imbalance && levels[1].stacked_bid_imbalance);
        assert!(!levels[2].stacked_bid_imbalance);
    }

    #[test]
    fn host_side_quote_rule_tick_rule_and_ambiguity_are_deterministic() {
        let mut aggregator = FootprintAggregator::new(FootprintAggregationOptions {
            tick_size: 1.0,
            ticks_per_row: 1,
            ..FootprintAggregationOptions::default()
        })
        .unwrap();
        let mut first = trade(1, 100.0, 2.0, AggressorSide::Unknown);
        first.bid = Some(99.0);
        first.ask = Some(100.0);
        let equal = trade(2, 100.0, 3.0, AggressorSide::Unknown);
        let lower = trade(3, 99.0, 5.0, AggressorSide::Unknown);
        aggregator.set_trades(vec![first, equal, lower]).unwrap();
        let bar = &aggregator.bars()[0];
        assert_eq!(
            (bar.ask_volume, bar.bid_volume, bar.unknown_volume),
            (5.0, 5.0, 0.0)
        );

        let mut ambiguous = FootprintAggregator::new(FootprintAggregationOptions {
            tick_size: 1.0,
            ticks_per_row: 1,
            ..FootprintAggregationOptions::default()
        })
        .unwrap();
        ambiguous
            .set_trades(vec![trade(1, 100.0, 4.0, AggressorSide::Unknown)])
            .unwrap();
        assert_eq!(ambiguous.bars()[0].unknown_volume, 4.0);
    }

    #[test]
    fn late_event_rebuild_matches_sorted_history_and_live_tip_is_incremental() {
        let options = FootprintAggregationOptions {
            tick_size: 1.0,
            ticks_per_row: 1,
            ..FootprintAggregationOptions::default()
        };
        let first = trade(1, 100.0, 8.0, AggressorSide::Buy);
        let late = trade(2, 100.0, 20.0, AggressorSide::Sell);
        let last = trade(3, 100.0, 15.0, AggressorSide::Buy);
        let mut streamed = FootprintAggregator::new(options).unwrap();
        streamed.update_trade(first.clone()).unwrap();
        streamed.update_trade(last.clone()).unwrap();
        assert_eq!(streamed.work_stats().incremental_ticks, 2);
        streamed.update_trade(late.clone()).unwrap();
        assert_eq!(streamed.work_stats().historical_rebuilds, 1);

        let mut historical = FootprintAggregator::new(options).unwrap();
        historical.set_trades(vec![first, late, last]).unwrap();
        assert_eq!(streamed.bars(), historical.bars());
        assert_eq!(
            (streamed.bars()[0].max_delta, streamed.bars()[0].min_delta),
            (8.0, -12.0)
        );
    }

    #[test]
    fn historical_batch_merges_final_tape_with_one_rebuild() {
        let options = FootprintAggregationOptions {
            tick_size: 1.0,
            ticks_per_row: 1,
            ..FootprintAggregationOptions::default()
        };
        let mut aggregator = FootprintAggregator::new(options).unwrap();
        let mut history = (0..100)
            .map(|index| {
                let mut event = trade(
                    index * 1_000,
                    100.0 + (index % 3) as f64,
                    1.0,
                    AggressorSide::Buy,
                );
                event.trade_id = Some(index as u64);
                event
            })
            .collect::<Vec<_>>();
        aggregator.set_trades(history.clone()).unwrap();
        aggregator.reset_work_stats();

        let corrections = [10, 30, 70]
            .into_iter()
            .map(|index| {
                let mut event = history[index].clone();
                event.volume = 5.0;
                event.aggressor = AggressorSide::Sell;
                history[index] = event.clone();
                event
            })
            .collect();
        assert_eq!(
            aggregator.update_trades(corrections).unwrap(),
            FootprintUpdateKind::Historical
        );
        assert_eq!(
            aggregator.work_stats(),
            FootprintWorkStats {
                incremental_ticks: 0,
                historical_rebuilds: 1,
                rebuilt_ticks: 100,
                skipped_sealed_trades: 0,
            }
        );

        let mut expected = FootprintAggregator::new(options).unwrap();
        expected.set_trades(history).unwrap();
        assert_eq!(aggregator.bars(), expected.bars());
    }

    /// A live-shaped tape: second-spaced prints over 10-second bars, every third print without a
    /// provider aggressor so tick-rule classification carries state across the tape.
    fn late_test_tape(count: i64) -> Vec<FootprintTrade> {
        (1..=count)
            .map(|index| {
                let side = match index % 3 {
                    0 => AggressorSide::Unknown,
                    1 => AggressorSide::Buy,
                    _ => AggressorSide::Sell,
                };
                let mut print = trade(
                    index * 1_000_000,
                    100.0 + ((index * 7) % 11) as f64,
                    (index % 5 + 1) as f64,
                    side,
                );
                print.sequence = Some(index as u64);
                print.trade_id = Some(index as u64);
                print
            })
            .collect()
    }

    fn late_test_options() -> FootprintAggregationOptions {
        FootprintAggregationOptions {
            tick_size: 1.0,
            ticks_per_row: 1,
            bars: FootprintBarAggregation::Time {
                interval_micros: 10_000_000,
                anchor_micros: 0,
            },
            ..FootprintAggregationOptions::default()
        }
    }

    #[test]
    fn late_prints_and_corrections_rebuild_only_from_the_nearest_checkpoint() {
        let tape = late_test_tape(20_000);
        let mut live = FootprintAggregator::new(late_test_options()).unwrap();
        live.set_trades(tape.clone()).unwrap();
        let before = live.work_stats();

        // One print a bar and a half behind the tip, plus a correction of a recent print.
        let mut late = trade(19_985_500_000, 104.0, 9.0, AggressorSide::Unknown);
        late.sequence = Some(19_985);
        late.trade_id = Some(1_000_000);
        let mut correction = tape[19_990].clone();
        correction.volume += 3.0;
        correction.aggressor = AggressorSide::Sell;
        let (kind, first_bar) = live
            .apply_trades(vec![late.clone(), correction.clone()])
            .unwrap();
        assert_eq!(kind, FootprintUpdateKind::Historical);
        let rebuilt = live.work_stats().rebuilt_ticks - before.rebuilt_ticks;
        assert!(
            rebuilt <= REPLAY_CHECKPOINT_INTERVAL + 32,
            "a late print near the tip replays at most one checkpoint interval, not {rebuilt}"
        );
        // Ten prints a bar: the rebuilt suffix spans at most one checkpoint interval of bars.
        assert!(live.bars().len() - first_bar <= REPLAY_CHECKPOINT_INTERVAL / 10 + 2);

        let mut expected_tape = tape;
        expected_tape[19_990] = correction;
        expected_tape.push(late);
        let mut expected = FootprintAggregator::new(late_test_options()).unwrap();
        expected.set_trades(expected_tape).unwrap();
        assert_eq!(live.bars(), expected.bars());
        assert_eq!(live.bars()[..first_bar], expected.bars()[..first_bar]);
        assert_eq!(
            live.classified_trades()
                .map(|(trade, side)| (trade.timestamp_micros, trade.trade_id, side))
                .collect::<Vec<_>>(),
            expected
                .classified_trades()
                .map(|(trade, side)| (trade.timestamp_micros, trade.trade_id, side))
                .collect::<Vec<_>>()
        );

        // Corrections still resolve through the incrementally re-indexed trade ids.
        let mut recorrected = expected.trade_at(19_995).unwrap().clone();
        recorrected.volume = 1.0;
        live.update_trades(vec![recorrected.clone()]).unwrap();
        expected.update_trades(vec![recorrected]).unwrap();
        assert_eq!(live.bars(), expected.bars());
    }

    #[test]
    fn late_print_projects_dependents_like_a_full_reinstall() {
        let presentation = |chart: &mut ChartEngine| {
            chart
                .add_order_flow_presentation(
                    "CME:ES",
                    0,
                    OrderFlowPresentationOptions {
                        aggregation: late_test_options(),
                        visual: FootprintVisualOptions::default(),
                        show_footprint: true,
                        show_cumulative_delta: true,
                        show_delta_histogram: true,
                        big_trades: Some(crate::BigTradesOptions::default()),
                    },
                )
                .unwrap()
        };
        let tape = late_test_tape(5_000);
        let mut late = trade(4_975_500_000, 101.0, 40.0, AggressorSide::Sell);
        late.sequence = Some(4_975);
        late.trade_id = Some(9_999_999);

        let mut live = ChartEngine::new(800.0, 400.0, 1.0);
        let live_flow = presentation(&mut live);
        live.update_order_flow_presentation(live_flow, tape.clone(), false)
            .unwrap();
        let kind = live
            .update_order_flow_presentation(live_flow, vec![late.clone()], true)
            .unwrap();
        assert_eq!(kind, FootprintUpdateKind::Historical);

        let mut reinstalled = ChartEngine::new(800.0, 400.0, 1.0);
        let full_flow = presentation(&mut reinstalled);
        let mut full_tape = tape;
        full_tape.push(late);
        reinstalled
            .update_order_flow_presentation(full_flow, full_tape, false)
            .unwrap();

        let series = |flow: OrderFlowPresentation| {
            [
                flow.footprint_series().unwrap(),
                flow.cumulative_delta_series().unwrap(),
                flow.delta_series().unwrap(),
            ]
        };
        assert_eq!(
            live.footprint_bars(live_flow.footprint_series().unwrap()),
            reinstalled.footprint_bars(full_flow.footprint_series().unwrap())
        );
        for (live_id, full_id) in series(live_flow).into_iter().zip(series(full_flow)) {
            let (live_times, live_values) = live.data_layer().series_data(live_id).unwrap();
            let (full_times, full_values) = reinstalled.data_layer().series_data(full_id).unwrap();
            assert_eq!(live_times, full_times);
            assert_eq!(live_values, full_values);
        }
        let delta = live_flow.delta_series().unwrap();
        let rows = live.data_layer().series_data(delta).unwrap().0.len();
        for row in 0..rows {
            assert_eq!(
                live.data.point_color(
                    delta,
                    aeris_charts_core::model::data_layer::PointColorChannel::Body,
                    row,
                ),
                reinstalled.data.point_color(
                    full_flow.delta_series().unwrap(),
                    aeris_charts_core::model::data_layer::PointColorChannel::Body,
                    row,
                )
            );
        }
        assert_eq!(
            live.big_trades_snapshot(live_flow.big_trades().unwrap()),
            reinstalled.big_trades_snapshot(full_flow.big_trades().unwrap())
        );
    }

    #[test]
    fn sealing_and_eviction_keep_bars_and_checkpoints_without_rebuilding() {
        let mut live = FootprintAggregator::new(late_test_options()).unwrap();
        live.set_trades(late_test_tape(20_000)).unwrap();
        let before = live.work_stats();
        let original = live.bars().to_vec();
        let released = live.raw_trades_of_bars(600);
        assert_eq!(live.seal_bars(600), released);
        assert_eq!(live.sealed_bar_count(), 600);
        assert_eq!(live.trades().len(), 20_000 - released);
        assert_eq!(live.bars(), original.as_slice(), "sealing changes no bar");
        assert_eq!(live.work_stats().rebuilt_ticks, before.rebuilt_ticks);

        live.evict_sealed_bars(500);
        assert_eq!(live.work_stats().rebuilt_ticks, before.rebuilt_ticks);
        assert_eq!(live.sealed_bar_count(), 100);
        let retained = &original[500..];
        let mut rebuilt = live.clone();
        rebuilt.rebuild();
        assert_eq!(
            live.bars(),
            rebuilt.bars(),
            "raw bars rebuild exactly after sealed ones"
        );
        assert_eq!(live.bars().len(), retained.len());
        for (index, (bar, original)) in live.bars().iter().zip(retained).enumerate() {
            assert_eq!(bar.logical_index, index as u64);
            assert_eq!(
                FootprintBar {
                    logical_index: original.logical_index - 500,
                    ..original.clone()
                },
                *bar
            );
        }

        // Rebased checkpoints keep a late print after retention bounded and exact.
        let mut late = trade(19_995_500_000, 103.0, 2.0, AggressorSide::Buy);
        late.sequence = Some(19_995);
        late.trade_id = Some(2_000_000);
        let before_late = live.work_stats();
        live.update_trades(vec![late.clone()]).unwrap();
        rebuilt.update_trades(vec![late]).unwrap();
        rebuilt.rebuild();
        assert!(
            live.work_stats().rebuilt_ticks - before_late.rebuilt_ticks
                <= REPLAY_CHECKPOINT_INTERVAL + 16
        );
        assert_eq!(live.bars(), rebuilt.bars());
    }

    #[test]
    fn backward_replay_seek_restores_nearest_bounded_checkpoint() {
        let options = FootprintAggregationOptions {
            tick_size: 1.0,
            ticks_per_row: 1,
            bars: FootprintBarAggregation::Trades { trades_per_bar: 10 },
            ..FootprintAggregationOptions::default()
        };
        let tape = (1..=3_000)
            .map(|timestamp| {
                trade(
                    timestamp,
                    100.0 + (timestamp % 5) as f64,
                    1.0,
                    AggressorSide::Buy,
                )
            })
            .collect::<Vec<_>>();
        let mut replay = FootprintAggregator::new(options).unwrap();
        replay.set_trades(tape.clone()).unwrap();

        let seek = replay.set_replay_clock_micros(Some(2_300)).unwrap();
        assert_eq!(seek.visible_trades, 2_300);
        assert_eq!(seek.rebuilt_trades, 2_300 - 2_048);
        assert!(replay.replay_checkpoints.len() <= MAX_REPLAY_CHECKPOINTS);

        let mut expected = FootprintAggregator::new(options).unwrap();
        expected.set_trades(tape[..2_300].to_vec()).unwrap();
        assert_eq!(replay.bars(), expected.bars());

        let checkpoint_seek = replay.set_replay_clock_micros(Some(2_048)).unwrap();
        assert_eq!(checkpoint_seek.rebuilt_trades, 0);
        let mut expected = FootprintAggregator::new(options).unwrap();
        expected.set_trades(tape[..2_048].to_vec()).unwrap();
        assert_eq!(replay.bars(), expected.bars());
    }

    #[test]
    fn session_change_resets_cumulative_delta_and_forces_a_bar_boundary() {
        let mut aggregator = FootprintAggregator::new(FootprintAggregationOptions {
            tick_size: 1.0,
            ticks_per_row: 1,
            ..FootprintAggregationOptions::default()
        })
        .unwrap();
        let first = trade(1, 100.0, 7.0, AggressorSide::Buy);
        let mut second = trade(2, 100.0, 3.0, AggressorSide::Sell);
        second.session_id = Some(2);
        aggregator.set_trades(vec![first, second]).unwrap();
        assert_eq!(aggregator.bars().len(), 2);
        assert_eq!(aggregator.bars()[0].session_delta, 7.0);
        assert_eq!(aggregator.bars()[1].session_delta, -3.0);
    }

    #[test]
    fn time_trade_and_volume_bar_modes_have_stable_boundaries() {
        let tape = vec![
            trade(1, 100.0, 4.0, AggressorSide::Buy),
            trade(2, 100.0, 4.0, AggressorSide::Buy),
            trade(3, 100.0, 4.0, AggressorSide::Buy),
            trade(4, 100.0, 4.0, AggressorSide::Buy),
            trade(5, 100.0, 4.0, AggressorSide::Buy),
        ];
        let mut trades = FootprintAggregator::new(FootprintAggregationOptions {
            tick_size: 1.0,
            ticks_per_row: 1,
            bars: FootprintBarAggregation::Trades { trades_per_bar: 2 },
            ..FootprintAggregationOptions::default()
        })
        .unwrap();
        trades.set_trades(tape.clone()).unwrap();
        assert_eq!(
            trades
                .bars()
                .iter()
                .map(|bar| bar.trade_count)
                .collect::<Vec<_>>(),
            vec![2, 2, 1]
        );

        let mut volume = FootprintAggregator::new(FootprintAggregationOptions {
            tick_size: 1.0,
            ticks_per_row: 1,
            bars: FootprintBarAggregation::Volume {
                volume_per_bar: 10.0,
            },
            ..FootprintAggregationOptions::default()
        })
        .unwrap();
        volume.set_trades(tape).unwrap();
        assert_eq!(
            volume
                .bars()
                .iter()
                .map(|bar| bar.total_volume)
                .collect::<Vec<_>>(),
            vec![12.0, 8.0]
        );
    }

    #[test]
    fn range_bar_mode_closes_on_tick_span_without_splitting_the_trigger_trade() {
        let mut range = FootprintAggregator::new(FootprintAggregationOptions {
            tick_size: 0.25,
            ticks_per_row: 1,
            bars: FootprintBarAggregation::Range { range_ticks: 4 },
            ..FootprintAggregationOptions::default()
        })
        .unwrap();
        range
            .set_trades(vec![
                trade(1, 100.00, 1.0, AggressorSide::Buy),
                trade(2, 100.75, 2.0, AggressorSide::Buy),
                trade(3, 101.00, 3.0, AggressorSide::Buy),
                trade(4, 101.25, 4.0, AggressorSide::Buy),
            ])
            .unwrap();
        assert_eq!(range.bars().len(), 2);
        assert_eq!(range.bars()[0].trade_count, 3);
        assert_eq!(range.bars()[0].high, 101.0);
        assert_eq!(range.bars()[1].trade_count, 1);
        assert_eq!(range.bars()[1].open, 101.25);
    }

    #[test]
    fn logical_bar_sequence_keeps_subsecond_bars_and_gap_times_distinct() {
        let mut aggregator = FootprintAggregator::new(FootprintAggregationOptions {
            tick_size: 1.0,
            ticks_per_row: 1,
            bars: FootprintBarAggregation::Trades { trades_per_bar: 1 },
            ..FootprintAggregationOptions::default()
        })
        .unwrap();
        aggregator
            .set_trades(vec![
                trade(1_000_001, 100.0, 1.0, AggressorSide::Buy),
                trade(1_000_002, 101.0, 1.0, AggressorSide::Buy),
                trade(9_000_000, 102.0, 1.0, AggressorSide::Buy),
            ])
            .unwrap();
        let sequence = aggregator.bar_sequence();
        assert_eq!(sequence.len(), 3);
        assert_eq!(
            sequence.iter().collect::<Vec<_>>(),
            vec![
                BarSequencePoint {
                    logical_index: 0,
                    open_timestamp_micros: 1_000_001,
                    close_timestamp_micros: 1_000_001,
                },
                BarSequencePoint {
                    logical_index: 1,
                    open_timestamp_micros: 1_000_002,
                    close_timestamp_micros: 1_000_002,
                },
                BarSequencePoint {
                    logical_index: 2,
                    open_timestamp_micros: 9_000_000,
                    close_timestamp_micros: 9_000_000,
                },
            ]
        );
        assert_eq!(aggregator.bars()[0].start_timestamp_micros / 1_000_000, 1);
        assert_eq!(aggregator.bars()[1].start_timestamp_micros / 1_000_000, 1);
        assert_eq!(aggregator.bars()[2].start_timestamp_micros / 1_000_000, 9);
    }

    #[test]
    fn logical_bar_sequence_mapping_rebases_prepend_without_merging_same_second_bars() {
        let options = FootprintAggregationOptions {
            tick_size: 1.0,
            ticks_per_row: 1,
            bars: FootprintBarAggregation::Trades { trades_per_bar: 1 },
            ..FootprintAggregationOptions::default()
        };
        let mut old = FootprintAggregator::new(options).unwrap();
        old.set_trades(vec![
            trade(1_000_001, 100.0, 1.0, AggressorSide::Buy),
            trade(1_000_002, 101.0, 1.0, AggressorSide::Buy),
            trade(9_000_000, 102.0, 1.0, AggressorSide::Buy),
        ])
        .unwrap();
        let old_anchor = old.bar_sequence().get(1).unwrap();

        let mut rebuilt = FootprintAggregator::new(options).unwrap();
        rebuilt
            .set_trades(vec![
                trade(500_000, 99.0, 1.0, AggressorSide::Buy),
                trade(1_000_001, 100.0, 1.0, AggressorSide::Buy),
                trade(1_000_002, 101.0, 1.0, AggressorSide::Buy),
                trade(9_000_000, 102.0, 1.0, AggressorSide::Buy),
            ])
            .unwrap();
        let mapping = BarSequenceMapping::between(old.bar_sequence(), rebuilt.bar_sequence());
        assert_eq!(mapping.map_logical_index(old_anchor.logical_index), Some(2));
        assert_eq!(
            mapping.rebase_anchor(old_anchor, rebuilt.bar_sequence()),
            rebuilt.bar_sequence().get(2)
        );
        assert!(!mapping.is_empty());
    }

    #[test]
    fn non_time_sequence_rebuild_rebases_drawing_anchors_by_full_resolution_bar_identity() {
        let mut chart = ChartEngine::new(800.0, 420.0, 1.0);
        let id = chart
            .add_footprint_series(FootprintSeriesOptions {
                aggregation: FootprintAggregationOptions {
                    tick_size: 1.0,
                    ticks_per_row: 1,
                    bars: FootprintBarAggregation::Trades { trades_per_bar: 1 },
                    ..FootprintAggregationOptions::default()
                },
                ..FootprintSeriesOptions::default()
            })
            .unwrap();
        chart
            .set_footprint_trades(
                id,
                vec![
                    trade(2_000_001, 100.0, 1.0, AggressorSide::Buy),
                    trade(3_000_001, 101.0, 1.0, AggressorSide::Buy),
                ],
            )
            .unwrap();
        let drawing = chart
            .add_drawing(
                DrawingKind::TrendLine,
                0,
                vec![
                    DrawingPoint {
                        logical: 0.5,
                        price: 100.0,
                    },
                    DrawingPoint {
                        logical: 1.0,
                        price: 101.0,
                    },
                ],
                None,
            )
            .unwrap();

        chart
            .update_footprint_trade(id, trade(1_000_001, 99.0, 1.0, AggressorSide::Buy))
            .unwrap();

        let points = &chart.drawing(drawing).unwrap().points;
        assert_eq!(points[0].logical, 1.5);
        assert_eq!(points[1].logical, 2.0);
    }

    #[test]
    fn invalid_or_off_grid_batches_are_atomic() {
        let mut aggregator = FootprintAggregator::new(FootprintAggregationOptions {
            tick_size: 0.25,
            ticks_per_row: 1,
            ..FootprintAggregationOptions::default()
        })
        .unwrap();
        aggregator
            .set_trades(vec![trade(1, 100.25, 1.0, AggressorSide::Buy)])
            .unwrap();
        let before = aggregator.bars().to_vec();
        let error = aggregator
            .set_trades(vec![trade(2, 100.30, 1.0, AggressorSide::Buy)])
            .unwrap_err();
        assert_eq!(error, FootprintError::OffGridPrice { index: 0 });
        assert_eq!(aggregator.bars(), before);
    }

    #[test]
    fn trade_id_correction_replaces_source_truth_and_rebuilds_delta_path() {
        let mut aggregator = FootprintAggregator::new(FootprintAggregationOptions {
            tick_size: 1.0,
            ticks_per_row: 1,
            ..FootprintAggregationOptions::default()
        })
        .unwrap();
        let mut original = trade(1, 100.0, 12.0, AggressorSide::Buy);
        original.trade_id = Some(77);
        aggregator.set_trades(vec![original]).unwrap();

        let mut correction = trade(1, 100.0, 5.0, AggressorSide::Sell);
        correction.trade_id = Some(77);
        assert_eq!(
            aggregator.update_trade(correction).unwrap(),
            FootprintUpdateKind::Historical
        );
        let bar = &aggregator.bars()[0];
        assert_eq!(
            (bar.bid_volume, bar.ask_volume, bar.delta),
            (5.0, 0.0, -5.0)
        );
        assert_eq!((bar.max_delta, bar.min_delta), (0.0, -5.0));
        assert_eq!(aggregator.trades().len(), 1);
    }

    #[test]
    fn session_correction_validates_final_tape_and_collision_failure_is_atomic() {
        let mut chart = ChartEngine::new(800.0, 420.0, 1.0);
        chart
            .configure_footprint_series(
                0,
                FootprintSeriesOptions {
                    aggregation: FootprintAggregationOptions {
                        tick_size: 1.0,
                        ticks_per_row: 1,
                        ..FootprintAggregationOptions::default()
                    },
                    visual: FootprintVisualOptions::default(),
                },
            )
            .unwrap();
        let mut original = trade(1_000_000, 100.0, 4.0, AggressorSide::Buy);
        original.trade_id = Some(7);
        chart
            .set_footprint_trades(0, vec![original.clone()])
            .unwrap();

        let mut correction = original;
        correction.session_id = Some(2);
        assert_eq!(
            chart.update_footprint_trade(0, correction).unwrap(),
            FootprintUpdateKind::Historical
        );
        assert_eq!(chart.footprint_bars(0).unwrap()[0].session_id, Some(2));

        let before_bars = chart.footprint_bars(0).unwrap().to_vec();
        let before_stats = chart.footprint_work_stats(0).unwrap();
        let mut collision = trade(2_000_000, 101.0, 2.0, AggressorSide::Sell);
        collision.session_id = Some(3);
        assert_eq!(
            chart.update_footprint_trade(0, collision).unwrap_err(),
            FootprintError::ProjectionTimeCollision
        );
        assert_eq!(chart.footprint_bars(0).unwrap(), before_bars);
        assert_eq!(chart.footprint_work_stats(0).unwrap(), before_stats);
    }

    /// Chart presentations key time-bar rows by bar open, so a tape where a session change falls
    /// inside one bar interval is rejected before anything changes, on every tape path, and also
    /// when the clashing print is still hidden by the replay clock: revealing it later could not
    /// be refused.
    #[test]
    fn bar_time_collisions_are_rejected_atomically_including_hidden_prints() {
        let mut chart = ChartEngine::new(800.0, 420.0, 1.0);
        let stream = chart
            .add_trade_stream(
                "TEST:COLLIDE",
                FootprintAggregationOptions {
                    tick_size: 1.0,
                    ticks_per_row: 1,
                    ..FootprintAggregationOptions::default()
                },
            )
            .unwrap();
        let candles = chart.add_series(SeriesKind::Candlestick);
        chart
            .bind_trade_bar_series_to_stream(candles, stream)
            .unwrap();
        let delta = chart.add_delta_series(stream, 1).unwrap();
        let print = |second: i64, session: u64, id: u64| FootprintTrade {
            session_id: Some(session),
            trade_id: Some(id),
            ..trade(
                second * 1_000_000,
                100.0 + id as f64,
                1.0,
                AggressorSide::Buy,
            )
        };
        let state = |chart: &ChartEngine| {
            let aggregator = chart.trade_stream(stream).unwrap();
            (
                aggregator.revision(),
                aggregator
                    .trades
                    .iter()
                    .map(|stored| stored.event.clone())
                    .collect::<Vec<_>>(),
                aggregator.bars().to_vec(),
                chart.data_layer().series_data(candles).unwrap().0.to_vec(),
                chart.data_layer().series_data(delta).unwrap().0.to_vec(),
            )
        };
        let collision = Err(FootprintError::ProjectionTimeCollision);

        // A replacement tape with two sessions in the first minute.
        let empty = state(&chart);
        assert_eq!(
            chart.set_trade_stream_trades(stream, vec![print(0, 1, 1), print(30, 2, 2)]),
            collision
        );
        assert_eq!(state(&chart), empty);
        chart
            .set_trade_stream_trades(
                stream,
                vec![print(0, 1, 1), print(60, 1, 2), print(120, 1, 3)],
            )
            .unwrap();

        // A correction that moves a print into another session's minute.
        let loaded = state(&chart);
        assert_eq!(
            chart
                .update_trade_stream_trades(stream, vec![print(1, 2, 2)])
                .map(|_| ()),
            collision
        );
        assert_eq!(state(&chart), loaded);

        // Behind the clock: a hidden print opens minute 180; a tip or a correction that puts
        // another session in that minute is rejected although no visible bar holds it.
        chart.set_replay_clock_micros(Some(150_000_000)).unwrap();
        assert_eq!(
            chart.update_trade_stream_trades(stream, vec![print(180, 1, 4)]),
            Ok(FootprintUpdateKind::Tip)
        );
        let hidden = state(&chart);
        assert_eq!(
            chart
                .update_trade_stream_trades(stream, vec![print(190, 2, 5)])
                .map(|_| ()),
            collision
        );
        assert_eq!(
            chart
                .update_trade_stream_trades(stream, vec![print(200, 1, 5), print(185, 2, 4)])
                .map(|_| ()),
            collision
        );
        assert_eq!(state(&chart), hidden);
        assert_eq!(
            chart.update_trade_stream_trades(stream, vec![print(200, 1, 5)]),
            Ok(FootprintUpdateKind::Tip)
        );

        // Revealing the hidden minute keeps every presentation keyed by one row per bar.
        chart.set_replay_clock_micros(None).unwrap();
        let (_, _, bars, candle_times, delta_times) = state(&chart);
        let opens = bars
            .iter()
            .map(|bar| bar.start_timestamp_micros / 1_000_000)
            .collect::<Vec<_>>();
        assert_eq!(opens, [0, 60, 120, 180]);
        assert_eq!(candle_times, opens);
        assert_eq!(delta_times, opens);
    }

    #[test]
    fn tick_size_owns_named_scale_format_autoscale_and_outer_cell_hit_bounds() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.5);
        let target = chart
            .add_price_scale(
                0,
                "footprint-dedicated",
                crate::PriceScaleSide::Right,
                None,
                true,
            )
            .unwrap();
        assert!(chart.try_set_series_pane_and_scale(0, 0, 1.0, "footprint-dedicated"));
        chart
            .configure_footprint_series(
                0,
                FootprintSeriesOptions {
                    aggregation: FootprintAggregationOptions {
                        tick_size: 1.0,
                        ticks_per_row: 1,
                        ..FootprintAggregationOptions::default()
                    },
                    visual: FootprintVisualOptions::default(),
                },
            )
            .unwrap();
        chart
            .set_footprint_trades(0, vec![trade(1_000_000, 100.0, 4.0, AggressorSide::Buy)])
            .unwrap();
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        chart.build_frame();

        assert_eq!(
            chart.price_scale_id_for_target(0, target),
            Some("footprint-dedicated")
        );
        let options: serde_json::Value =
            serde_json::from_str(&chart.series_options_json(0).unwrap()).unwrap();
        assert_eq!(options["price_format"]["precision"], 0);
        assert_eq!(options["price_format"]["min_move"], 1.0);
        let range = chart.price_scale_visible_range_for(0, target).unwrap();
        assert!(range.0 <= 99.5 && range.1 >= 100.5, "range was {range:?}");

        let x = chart.logical_to_coordinate(0.0).unwrap();
        let outer_cell_y = chart.series_price_to_coordinate(0, 100.45).unwrap();
        assert!(chart.hit_test_one_series(0, x, outer_cell_y).is_some());
    }

    #[test]
    fn chart_series_projects_scale_rows_but_keeps_tick_derived_queries() {
        let mut chart = ChartEngine::new(800.0, 420.0, 1.0);
        chart
            .configure_footprint_series(
                0,
                FootprintSeriesOptions {
                    aggregation: FootprintAggregationOptions {
                        tick_size: 1.0,
                        ticks_per_row: 1,
                        bars: FootprintBarAggregation::Time {
                            interval_micros: 60_000_000,
                            anchor_micros: 0,
                        },
                        imbalance: FootprintImbalanceOptions::default(),
                    },
                    visual: FootprintVisualOptions::default(),
                },
            )
            .unwrap();
        chart
            .set_footprint_trades(
                0,
                vec![
                    trade(1_000_000, 100.0, 10.0, AggressorSide::Buy),
                    trade(2_000_000, 100.0, 18.0, AggressorSide::Sell),
                    trade(61_000_000, 101.0, 7.0, AggressorSide::Buy),
                ],
            )
            .unwrap();

        assert_eq!(chart.series_kind(0), Some(SeriesKind::Footprint));
        assert_eq!(chart.data_layer().series_data(0).unwrap().0.len(), 2);
        assert_eq!(chart.footprint_bars(0).unwrap().len(), 2);
        assert_eq!(chart.footprint_bar(0, 0).unwrap().delta, -8.0);
        assert_eq!(chart.footprint_bar(0, 0).unwrap().max_delta, 10.0);
        assert_eq!(chart.footprint_bar(0, 0).unwrap().min_delta, -8.0);

        let update = chart
            .update_footprint_trade(0, trade(62_000_000, 102.0, 4.0, AggressorSide::Sell))
            .unwrap();
        assert_eq!(update, FootprintUpdateKind::Tip);
        assert_eq!(chart.data_layer().series_data(0).unwrap().0.len(), 2);
        assert_eq!(chart.footprint_bar(0, 1).unwrap().close, 102.0);
        assert!(chart.footprint_work_stats(0).unwrap().incremental_ticks > 0);

        let historical = chart
            .update_footprint_trade(0, trade(1_500_000, 100.0, 20.0, AggressorSide::Buy))
            .unwrap();
        assert_eq!(historical, FootprintUpdateKind::Historical);
        assert_eq!(chart.data_layer().series_data(0).unwrap().0.len(), 2);
        assert_eq!(chart.footprint_bar(0, 0).unwrap().delta, 12.0);
    }

    #[test]
    fn retained_time_footprint_appends_live_bars_like_a_full_reinstall() {
        let options = FootprintSeriesOptions {
            aggregation: FootprintAggregationOptions {
                tick_size: 1.0,
                ticks_per_row: 1,
                bars: FootprintBarAggregation::Time {
                    interval_micros: 1_000_000,
                    anchor_micros: 0,
                },
                imbalance: FootprintImbalanceOptions::default(),
            },
            visual: FootprintVisualOptions::default(),
        };
        let history = (1..=4)
            .map(|second| {
                trade(
                    second * 1_000_000,
                    100.0 + second as f64,
                    1.0,
                    AggressorSide::Buy,
                )
            })
            .collect::<Vec<_>>();
        let live = vec![
            trade(4_500_000, 90.0, 2.0, AggressorSide::Sell),
            trade(5_000_000, 106.0, 3.0, AggressorSide::Buy),
            trade(6_000_000, 107.0, 1.0, AggressorSide::Buy),
        ];

        // A retention cap must not force time bars through a full reinstall per live batch:
        // timestamps key the suffix, so the appended result matches a fresh projection.
        let mut live_chart = ChartEngine::new(600.0, 400.0, 1.0);
        live_chart
            .configure_footprint_series(0, options.clone())
            .unwrap();
        live_chart.set_footprint_trades(0, history.clone()).unwrap();
        assert!(live_chart.set_series_max_points(0, Some(3)));
        let update = live_chart.update_footprint_trades(0, live.clone()).unwrap();
        assert_eq!(update, FootprintUpdateKind::Tip);

        let mut fresh = ChartEngine::new(600.0, 400.0, 1.0);
        fresh.configure_footprint_series(0, options).unwrap();
        fresh
            .set_footprint_trades(0, history.into_iter().chain(live).collect())
            .unwrap();
        assert!(fresh.set_series_max_points(0, Some(3)));

        assert_eq!(live_chart.footprint_bars(0), fresh.footprint_bars(0));
        assert_eq!(
            live_chart.data_layer().series_data(0),
            fresh.data_layer().series_data(0)
        );
    }

    #[test]
    fn non_time_footprint_projection_uses_sequence_labels_without_timestamp_collisions() {
        let mut chart = ChartEngine::new(800.0, 420.0, 1.0);
        let id = chart
            .add_footprint_series(FootprintSeriesOptions {
                aggregation: FootprintAggregationOptions {
                    tick_size: 1.0,
                    ticks_per_row: 1,
                    bars: FootprintBarAggregation::Trades { trades_per_bar: 1 },
                    ..FootprintAggregationOptions::default()
                },
                ..FootprintSeriesOptions::default()
            })
            .unwrap();
        chart
            .set_footprint_trades(
                id,
                vec![
                    trade(1_000_001, 100.0, 1.0, AggressorSide::Buy),
                    trade(1_000_002, 101.0, 1.0, AggressorSide::Buy),
                    trade(9_000_000, 102.0, 1.0, AggressorSide::Buy),
                ],
            )
            .unwrap();
        assert_eq!(chart.data_layer().merged_times(), &[0, 1, 2]);
        assert_eq!(chart.time_to_index(1.000002, false), Some(1));
        chart.time_scale.set_width(800.0);
        chart.build_frame();
        let gap_x = chart.time_scale.index_to_coordinate(2);
        assert_eq!(chart.coordinate_to_time(gap_x), Some(9.0));
        assert_eq!(chart.series_data(id)[0].time, 1);
        assert_eq!(
            chart
                .value_snapshot(Some(2))
                .into_iter()
                .find(|snapshot| snapshot.series_id == id)
                .and_then(|snapshot| snapshot.time),
            Some(9)
        );
        assert!(chart.set_crosshair_position(101.0, 1.000002, id));
        assert_eq!(chart.crosshair_sync_position().unwrap().time, 1.000002);
    }

    #[test]
    fn removing_the_last_non_time_series_retires_the_sequence_axis() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let footprint = chart
            .add_footprint_series(FootprintSeriesOptions {
                aggregation: FootprintAggregationOptions {
                    tick_size: 1.0,
                    ticks_per_row: 1,
                    bars: FootprintBarAggregation::Trades { trades_per_bar: 1 },
                    ..FootprintAggregationOptions::default()
                },
                ..FootprintSeriesOptions::default()
            })
            .unwrap();
        chart
            .set_footprint_trades(
                footprint,
                vec![trade(1_000_001, 100.0, 1.0, AggressorSide::Buy)],
            )
            .unwrap();
        assert!(chart.sequence_points().is_some());
        assert!(chart.remove_series(footprint));
        assert!(chart.sequence_points().is_none());
    }

    #[test]
    fn non_time_tip_updates_reuse_sequence_rows_for_derived_studies() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let footprint = chart
            .add_footprint_series(FootprintSeriesOptions {
                aggregation: FootprintAggregationOptions {
                    tick_size: 1.0,
                    ticks_per_row: 1,
                    bars: FootprintBarAggregation::Trades { trades_per_bar: 2 },
                    ..FootprintAggregationOptions::default()
                },
                ..FootprintSeriesOptions::default()
            })
            .unwrap();
        chart
            .set_footprint_trades(
                footprint,
                vec![
                    trade(1_000_001, 100.0, 1.0, AggressorSide::Buy),
                    trade(1_000_002, 101.0, 1.0, AggressorSide::Sell),
                ],
            )
            .unwrap();
        let stream_id = chart
            .series_entry(footprint)
            .unwrap()
            .footprint
            .as_ref()
            .unwrap()
            .trade_stream_id;
        let delta = chart.add_delta_series(stream_id, 0).unwrap();
        assert_eq!(chart.data_layer().series_data(delta).unwrap().0, &[0]);

        assert_eq!(
            chart
                .update_footprint_trade(footprint, trade(1_000_003, 102.0, 1.0, AggressorSide::Buy))
                .unwrap(),
            FootprintUpdateKind::Tip
        );
        assert_eq!(
            chart.data_layer().series_data(footprint).unwrap().0,
            &[0, 1]
        );
        assert_eq!(chart.data_layer().series_data(delta).unwrap().0, &[0, 1]);
        assert_eq!(chart.sequence_points().unwrap().len(), 2);
    }

    #[test]
    fn non_time_sequence_indices_remain_chart_local_after_retention_trims_absolute_bars() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let id = chart
            .add_footprint_series(FootprintSeriesOptions {
                aggregation: FootprintAggregationOptions {
                    tick_size: 1.0,
                    ticks_per_row: 1,
                    bars: FootprintBarAggregation::Trades { trades_per_bar: 1 },
                    ..FootprintAggregationOptions::default()
                },
                ..FootprintSeriesOptions::default()
            })
            .unwrap();
        chart
            .set_footprint_trades(
                id,
                vec![
                    trade(1_000_001, 100.0, 1.0, AggressorSide::Buy),
                    trade(2_000_001, 101.0, 1.0, AggressorSide::Buy),
                    trade(3_000_001, 102.0, 1.0, AggressorSide::Buy),
                ],
            )
            .unwrap();
        assert!(chart.set_series_max_points(id, Some(2)));

        let sequence = chart.sequence_points().unwrap();
        assert_eq!(
            sequence
                .iter()
                .map(|point| point.logical_index)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(sequence[0].open_timestamp_micros, 2_000_001);
        assert_eq!(sequence[1].open_timestamp_micros, 3_000_001);
    }

    #[test]
    fn non_time_big_trades_use_logical_bar_indices() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let footprint = chart
            .add_footprint_series(FootprintSeriesOptions {
                aggregation: FootprintAggregationOptions {
                    tick_size: 1.0,
                    ticks_per_row: 1,
                    bars: FootprintBarAggregation::Trades { trades_per_bar: 2 },
                    ..FootprintAggregationOptions::default()
                },
                ..FootprintSeriesOptions::default()
            })
            .unwrap();
        chart
            .set_footprint_trades(
                footprint,
                vec![
                    trade(1_000_001, 100.0, 1.0, AggressorSide::Buy),
                    trade(1_000_002, 101.0, 1.0, AggressorSide::Sell),
                ],
            )
            .unwrap();
        let stream_id = chart
            .series_entry(footprint)
            .unwrap()
            .footprint
            .as_ref()
            .unwrap()
            .trade_stream_id;
        let big_trades = chart
            .add_big_trades(
                stream_id,
                footprint,
                crate::BigTradesOptions {
                    filter: crate::BigTradesFilter::Fixed {
                        minimum_volume: 1.0,
                    },
                    ..crate::BigTradesOptions::default()
                },
            )
            .unwrap();
        let bar_times = |chart: &ChartEngine| {
            chart
                .big_trades_snapshot(big_trades)
                .unwrap()
                .bubbles
                .iter()
                .map(|order| order.bar_time)
                .collect::<Vec<_>>()
        };
        assert_eq!(bar_times(&chart), vec![0, 0]);
        chart
            .update_footprint_trade(footprint, trade(1_000_003, 102.0, 1.0, AggressorSide::Buy))
            .unwrap();
        assert_eq!(bar_times(&chart), vec![0, 0, 1]);
    }

    #[test]
    fn chart_trade_stream_is_shared_by_bound_footprint_dependents() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let stream = chart
            .add_trade_stream(
                "CME:ES",
                FootprintAggregationOptions {
                    tick_size: 1.0,
                    ticks_per_row: 1,
                    ..FootprintAggregationOptions::default()
                },
            )
            .unwrap();
        assert_eq!(
            chart.add_trade_stream("CME:ES", FootprintAggregationOptions::default()),
            Err(FootprintError::InvalidAggregation)
        );
        chart
            .configure_footprint_series(0, FootprintSeriesOptions::default())
            .unwrap();
        let second = chart.add_series(SeriesKind::Footprint);
        chart
            .configure_footprint_series(second, FootprintSeriesOptions::default())
            .unwrap();
        chart.bind_footprint_series_to_stream(0, stream).unwrap();
        chart
            .bind_footprint_series_to_stream(second, stream)
            .unwrap();
        chart
            .set_footprint_trades(0, vec![trade(1, 100.0, 2.0, AggressorSide::Buy)])
            .unwrap();
        assert_eq!(chart.footprint_bars(0), chart.footprint_bars(second));
        let revision = chart.trade_stream_revision(stream).unwrap();
        chart
            .update_footprint_trade(second, trade(2, 101.0, 1.0, AggressorSide::Sell))
            .unwrap();
        assert!(chart.trade_stream_revision(stream).unwrap() > revision);
        assert_eq!(chart.footprint_bars(0), chart.footprint_bars(second));
    }

    #[test]
    fn time_and_sales_is_bounded_filtered_and_uses_canonical_classification() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let stream = chart
            .add_trade_stream("tape", FootprintAggregationOptions::default())
            .unwrap();
        let mut inferred_buy = trade(2, 101.0, 3.0, AggressorSide::Unknown);
        inferred_buy.bid = Some(100.0);
        inferred_buy.ask = Some(101.0);
        chart
            .set_trade_stream_trades(
                stream,
                vec![
                    trade(1, 100.0, 1.0, AggressorSide::Sell),
                    inferred_buy,
                    trade(3, 102.0, 5.0, AggressorSide::Buy),
                ],
            )
            .unwrap();
        let rows = chart
            .time_and_sales(
                stream,
                TimeAndSalesOptions {
                    minimum_volume: 2.0,
                    side: Some(AggressorSide::Buy),
                    max_rows: 2,
                },
            )
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].timestamp_micros, 3);
        assert_eq!(rows[1].timestamp_micros, 2);
        assert_eq!(rows[1].aggressor, AggressorSide::Buy);
        assert_eq!(
            chart.time_and_sales(
                stream,
                TimeAndSalesOptions {
                    max_rows: MAX_TIME_AND_SALES_ROWS + 1,
                    ..TimeAndSalesOptions::default()
                }
            ),
            Err(FootprintError::InvalidTimeAndSalesOptions)
        );
    }

    #[test]
    fn ordinary_candles_share_non_time_trade_stream_bars_and_sequence_identity() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let footprint = chart
            .add_footprint_series(FootprintSeriesOptions {
                aggregation: FootprintAggregationOptions {
                    tick_size: 1.0,
                    ticks_per_row: 1,
                    bars: FootprintBarAggregation::Trades { trades_per_bar: 2 },
                    ..FootprintAggregationOptions::default()
                },
                ..FootprintSeriesOptions::default()
            })
            .unwrap();
        let stream_id = chart
            .series_entry(footprint)
            .unwrap()
            .footprint
            .as_ref()
            .unwrap()
            .trade_stream_id;
        let candles = chart.add_series(SeriesKind::Candlestick);
        chart
            .bind_trade_bar_series_to_stream(candles, stream_id)
            .unwrap();

        chart
            .set_footprint_trades(
                footprint,
                vec![
                    trade(1_000_001, 100.0, 1.0, AggressorSide::Buy),
                    trade(1_000_002, 102.0, 1.0, AggressorSide::Sell),
                    trade(1_000_003, 101.0, 1.0, AggressorSide::Buy),
                ],
            )
            .unwrap();
        let (times, columns) = chart.data_layer().series_data(candles).unwrap();
        assert_eq!(times, &[0, 1]);
        assert_eq!(columns[0], [100.0, 101.0]);
        assert_eq!(columns[1], [102.0, 101.0]);
        assert_eq!(columns[2], [100.0, 101.0]);
        assert_eq!(columns[3], [102.0, 101.0]);
        assert_eq!(chart.sequence_points().unwrap().len(), 2);

        chart
            .update_footprint_trade(footprint, trade(1_000_004, 103.0, 1.0, AggressorSide::Buy))
            .unwrap();
        let (_, columns) = chart.data_layer().series_data(candles).unwrap();
        assert_eq!(columns[0], [100.0, 101.0]);
        assert_eq!(columns[1], [102.0, 103.0]);
        assert_eq!(columns[2], [100.0, 101.0]);
        assert_eq!(columns[3], [102.0, 103.0]);
        let stats = chart.trade_stream_stats(stream_id).unwrap();
        assert_eq!(stats.dependent_count, 1);
        assert_eq!(stats.dependent_rebuilds, 2);
        assert_eq!(stats.dependent_incremental_updates, 1);
    }

    #[test]
    fn trade_bar_binding_rejects_scalar_series_and_releases_stream_dependency_on_remove() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let stream = chart
            .add_trade_stream(
                "CME:ES:volume",
                FootprintAggregationOptions {
                    bars: FootprintBarAggregation::Volume {
                        volume_per_bar: 2.0,
                    },
                    ..FootprintAggregationOptions::default()
                },
            )
            .unwrap();
        let line = chart.add_series(SeriesKind::Line);
        assert_eq!(
            chart.bind_trade_bar_series_to_stream(line, stream),
            Err(FootprintError::UnsupportedTradeBarSeries(line))
        );
        let bars = chart.add_series(SeriesKind::Bar);
        chart.bind_trade_bar_series_to_stream(bars, stream).unwrap();
        assert!(!chart.set_series_max_points(bars, Some(1)));
        chart
            .set_trade_stream_trades(
                stream,
                vec![
                    trade(1_000_001, 100.0, 1.0, AggressorSide::Buy),
                    trade(1_000_002, 101.0, 1.0, AggressorSide::Sell),
                ],
            )
            .unwrap();
        assert_eq!(chart.data_layer().series_data(bars).unwrap().0, &[0]);
        chart
            .update_trade_stream_trade(stream, trade(1_000_003, 102.0, 1.0, AggressorSide::Buy))
            .unwrap();
        assert_eq!(chart.data_layer().series_data(bars).unwrap().0, &[0, 1]);
        assert_eq!(chart.sequence_points().unwrap().len(), 2);
        assert_eq!(chart.trade_stream_stats(stream).unwrap().dependent_count, 1);
        assert!(chart.remove_series(bars));
        assert_eq!(chart.trade_stream_stats(stream).unwrap().dependent_count, 0);
        chart.remove_trade_stream(stream).unwrap();
    }

    /// A series has one engine writer. Every attach path refuses a series another feature already
    /// writes, before it changes anything, so two writers never fight over one series' rows.
    #[test]
    fn single_owner_attach_checks() {
        use crate::{ResampleBoundary, ResampleOptions, SyntheticBarOptions};

        let dependents =
            |chart: &ChartEngine, stream| chart.trade_stream_stats(stream).unwrap().dependent_count;
        let print = |micros| trade(micros, 100.0, 1.0, AggressorSide::Buy);
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let stream = chart
            .add_trade_stream("K:1m", FootprintAggregationOptions::default())
            .unwrap();
        let other = chart
            .add_trade_stream(
                "K:5m",
                FootprintAggregationOptions {
                    bars: FootprintBarAggregation::Time {
                        interval_micros: 300_000_000,
                        anchor_micros: 0,
                    },
                    ..FootprintAggregationOptions::default()
                },
            )
            .unwrap();

        // A resampled target is written by its binding.
        let source = chart.add_series(SeriesKind::Candlestick);
        let target = chart.add_series(SeriesKind::Candlestick);
        chart
            .configure_resampled_series(
                source,
                None,
                target,
                None,
                ResampleOptions {
                    interval_seconds: 300,
                    boundaries: vec![ResampleBoundary {
                        start_time: 0,
                        end_time: 30 * 86_400,
                        session_id: 1,
                    }],
                },
            )
            .unwrap();
        assert_eq!(
            chart.bind_trade_bar_series_to_stream(target, stream),
            Err(FootprintError::SeriesOwned(target))
        );
        assert_eq!(dependents(&chart, stream), 0);
        // The refused bind left nothing registered, so the stream keeps accepting trades.
        chart
            .update_trade_stream_trades(stream, vec![print(60_000_001)])
            .unwrap();

        // A study is a scalar series until it is converted, so the candle-kind check refuses it
        // first. Once converted to a candle it keeps its study registration: the study still
        // writes it.
        let cvd = chart
            .add_cvd_series(stream, 1, TradeStudyOptions::default())
            .unwrap();
        assert_eq!(
            chart.bind_trade_bar_series_to_stream(cvd, stream),
            Err(FootprintError::UnsupportedTradeBarSeries(cvd))
        );
        assert_eq!(dependents(&chart, stream), 1);
        chart.convert_series_kind(cvd, SeriesKind::Candlestick);
        assert_eq!(
            chart.bind_trade_bar_series_to_stream(cvd, stream),
            Err(FootprintError::SeriesOwned(cvd))
        );
        assert_eq!(dependents(&chart, stream), 1);
        chart
            .update_trade_stream_trades(stream, vec![print(60_000_002)])
            .unwrap();

        // A footprint is not a candle presentation at all.
        let footprint = chart
            .add_footprint_series(FootprintSeriesOptions::default())
            .unwrap();
        assert_eq!(
            chart.bind_trade_bar_series_to_stream(footprint, stream),
            Err(FootprintError::UnsupportedTradeBarSeries(footprint))
        );
        assert_eq!(dependents(&chart, stream), 1);

        // Candles bound to a stream, or studies of it, cannot become a footprint. Reconfiguring a
        // footprint, and rebinding a bound candle to another stream, stay allowed.
        let candles = chart.add_series(SeriesKind::Candlestick);
        chart
            .bind_trade_bar_series_to_stream(candles, stream)
            .unwrap();
        chart
            .bind_trade_bar_series_to_stream(candles, stream)
            .unwrap();
        let delta = chart.add_delta_series(stream, 2).unwrap();
        let volume = chart.add_trade_volume_series(stream, 2).unwrap();
        assert_eq!(dependents(&chart, stream), 4);
        for id in [candles, cvd, delta, volume] {
            let kind = chart.series_kind(id);
            assert_eq!(
                chart.configure_footprint_series(id, FootprintSeriesOptions::default()),
                Err(FootprintError::SeriesOwned(id))
            );
            assert_eq!(chart.series_kind(id), kind);
        }
        assert_eq!(dependents(&chart, stream), 4);
        chart
            .configure_footprint_series(footprint, FootprintSeriesOptions::default())
            .unwrap();
        chart
            .bind_trade_bar_series_to_stream(candles, other)
            .unwrap();
        assert_eq!(dependents(&chart, stream), 3);
        assert_eq!(dependents(&chart, other), 1);
        chart
            .update_trade_stream_trades(stream, vec![print(60_000_003)])
            .unwrap();
        chart
            .update_trade_stream_trades(other, vec![print(60_000_004)])
            .unwrap();

        // A first refresh that fails leaves no registration behind: this stream's half-second bars
        // share one chart second.
        let half_seconds = chart
            .add_trade_stream(
                "K:500ms",
                FootprintAggregationOptions {
                    bars: FootprintBarAggregation::Time {
                        interval_micros: 500_000,
                        anchor_micros: 0,
                    },
                    ..FootprintAggregationOptions::default()
                },
            )
            .unwrap();
        chart
            .set_trade_stream_trades(half_seconds, vec![print(1_000_000), print(1_500_000)])
            .unwrap();
        let unbound = chart.add_series(SeriesKind::Candlestick);
        assert_eq!(
            chart.bind_trade_bar_series_to_stream(unbound, half_seconds),
            Err(FootprintError::ProjectionTimeCollision)
        );
        assert_eq!(dependents(&chart, half_seconds), 0);
        assert!(!chart.series_is_source_owned(unbound));
        chart
            .update_trade_stream_trades(half_seconds, vec![print(2_000_000)])
            .unwrap();

        // Synthetic bars own their series, and no trade stream binds it.
        let mut synthetic = ChartEngine::new(600.0, 400.0, 1.0);
        let stream = synthetic
            .add_trade_stream("K:1m", FootprintAggregationOptions::default())
            .unwrap();
        let renko = synthetic.add_series(SeriesKind::Candlestick);
        synthetic
            .configure_synthetic_bar_series(
                renko,
                SyntheticBarOptions::RenkoFixed { box_size: 1.0 },
            )
            .unwrap();
        assert_eq!(
            synthetic.bind_trade_bar_series_to_stream(renko, stream),
            Err(FootprintError::SeriesOwned(renko))
        );
        assert_eq!(dependents(&synthetic, stream), 0);
        synthetic
            .update_trade_stream_trades(stream, vec![print(60_000_001)])
            .unwrap();
    }

    #[test]
    fn replay_clock_masks_future_tape_from_every_trade_stream_dependent() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let stream = chart
            .add_trade_stream(
                "CME:ES:replay",
                FootprintAggregationOptions {
                    tick_size: 1.0,
                    ticks_per_row: 1,
                    bars: FootprintBarAggregation::Trades { trades_per_bar: 1 },
                    ..FootprintAggregationOptions::default()
                },
            )
            .unwrap();
        let footprint = chart.add_series(SeriesKind::Footprint);
        chart
            .configure_footprint_series(footprint, FootprintSeriesOptions::default())
            .unwrap();
        chart
            .bind_footprint_series_to_stream(footprint, stream)
            .unwrap();
        let candles = chart.add_series(SeriesKind::Candlestick);
        chart
            .bind_trade_bar_series_to_stream(candles, stream)
            .unwrap();
        let delta = chart.add_delta_series(stream, 1).unwrap();
        let big_trades = chart
            .add_big_trades(
                stream,
                footprint,
                crate::BigTradesOptions {
                    filter: crate::BigTradesFilter::Fixed {
                        minimum_volume: 1.0,
                    },
                    grouping_window_micros: 0,
                    ..crate::BigTradesOptions::default()
                },
            )
            .unwrap();
        let bubbles =
            |chart: &ChartEngine| chart.big_trades_snapshot(big_trades).unwrap().bubbles.len();
        chart
            .set_trade_stream_trades(
                stream,
                (1..=3)
                    .map(|timestamp| {
                        trade(timestamp, 99.0 + timestamp as f64, 1.0, AggressorSide::Buy)
                    })
                    .collect(),
            )
            .unwrap();

        let seek = chart.set_replay_clock_micros(Some(2)).unwrap();
        assert_eq!(seek.stream_count, 1);
        assert_eq!(seek.visible_trades, 2);
        assert_eq!(seek.rebuilt_trades, 2);
        assert_eq!(chart.footprint_bars(footprint).unwrap().len(), 2);
        assert_eq!(chart.data_layer().series_data(candles).unwrap().0.len(), 2);
        assert_eq!(chart.data_layer().series_data(delta).unwrap().0.len(), 2);
        assert_eq!(bubbles(&chart), 2);

        let before_future = chart.trade_stream_stats(stream).unwrap();
        chart
            .update_trade_stream_trade(stream, trade(4, 103.0, 1.0, AggressorSide::Buy))
            .unwrap();
        assert_eq!(chart.footprint_bars(footprint).unwrap().len(), 2);
        let after_future = chart.trade_stream_stats(stream).unwrap();
        assert_eq!(
            after_future.dependent_rebuilds,
            before_future.dependent_rebuilds
        );
        assert_eq!(
            after_future.dependent_incremental_updates,
            before_future.dependent_incremental_updates
        );
        let forward = chart.set_replay_clock_micros(Some(4)).unwrap();
        assert_eq!(forward.visible_trades, 4);
        assert_eq!(chart.footprint_bars(footprint).unwrap().len(), 4);
        assert_eq!(bubbles(&chart), 4);
        let backward = chart.set_replay_clock_micros(Some(1)).unwrap();
        assert_eq!(backward.visible_trades, 1);
        assert_eq!(chart.footprint_bars(footprint).unwrap().len(), 1);
        assert_eq!(bubbles(&chart), 1);
        chart.set_replay_clock_micros(None).unwrap();
        assert_eq!(chart.footprint_bars(footprint).unwrap().len(), 4);
    }

    #[test]
    fn chart_replay_clock_masks_ordinary_rows_without_discarding_future_updates() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        chart
            .set_series_data(
                0,
                &[1.0, 2.0, 3.0],
                &[10.0, 20.0, 30.0],
                &[11.0, 21.0, 31.0],
                &[9.0, 19.0, 29.0],
                &[10.5, 20.5, 30.5],
            )
            .unwrap();
        let sma = chart.add_sma(0, 2).unwrap();

        let stats = chart.set_replay_clock_micros(Some(2_500_000)).unwrap();
        assert_eq!(stats.stream_count, 0);
        assert_eq!(chart.replay_clock_micros(), Some(2_500_000));
        assert_eq!(chart.data_layer().merged_times(), &[1, 2]);
        assert_eq!(chart.data_layer().series_data(0).unwrap().0, &[1, 2]);
        assert_eq!(chart.data_layer().series_data(sma).unwrap().0, &[2]);
        chart.fit_content();
        let frame = chart.build_frame();
        assert!(
            frame
                .panes
                .iter()
                .flat_map(|pane| &pane.main)
                .any(|primitive| matches!(primitive, Prim::Text { text, .. } if text == "Replay"))
        );
        assert!(
            frame
                .panes
                .iter()
                .flat_map(|pane| &pane.main)
                .any(|primitive| matches!(
                    primitive,
                    Prim::VLine {
                        style: LineStyle::Dashed,
                        ..
                    }
                ))
        );

        assert!(chart.update_series_bar(0, 4.0, [40.0, 41.0, 39.0, 40.5]));
        assert_eq!(chart.data_layer().merged_times(), &[1, 2]);
        chart.set_replay_clock_micros(Some(4_500_000)).unwrap();
        assert_eq!(chart.data_layer().merged_times(), &[1, 2, 3, 4]);
        assert_eq!(chart.data_layer().series_data(0).unwrap().0, &[1, 2, 3, 4]);
        assert_eq!(chart.data_layer().series_data(sma).unwrap().0, &[2, 3, 4]);
        chart.set_replay_clock_micros(None).unwrap();
        assert_eq!(chart.data_layer().merged_times(), &[1, 2, 3, 4]);
    }

    #[test]
    fn cvd_and_delta_dependents_follow_late_corrections_and_report_the_update() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let stream = chart
            .add_trade_stream("CME:NQ", FootprintAggregationOptions::default())
            .unwrap();
        let footprint = chart.add_series(SeriesKind::Footprint);
        chart
            .configure_footprint_series(footprint, FootprintSeriesOptions::default())
            .unwrap();
        chart
            .bind_footprint_series_to_stream(footprint, stream)
            .unwrap();
        let cvd = chart
            .add_cvd_series(stream, 1, TradeStudyOptions::default())
            .unwrap();
        let delta = chart.add_delta_series(stream, 1).unwrap();
        let mut initial_sell = trade(2_000_000, 100.0, 3.0, AggressorSide::Sell);
        initial_sell.trade_id = Some(2);
        chart
            .set_footprint_trades(
                footprint,
                vec![
                    trade(1_000_000, 100.0, 4.0, AggressorSide::Buy),
                    initial_sell,
                ],
            )
            .unwrap();
        let cvd_values = chart.data_layer().series_data(cvd).unwrap().1[3];
        let delta_values = chart.data_layer().series_data(delta).unwrap().1[3];
        assert_eq!(cvd_values.last().copied(), Some(1.0));
        assert_eq!(delta_values.last().copied(), Some(1.0));
        assert_eq!(
            chart.data.point_color(
                delta,
                aeris_charts_core::model::data_layer::PointColorChannel::Body,
                0,
            ),
            Some(Color::rgb(MARKET_UP_RGB.0, MARKET_UP_RGB.1, MARKET_UP_RGB.2).0),
            "positive delta uses the market-up color"
        );
        let before = chart.trade_stream_stats(stream).unwrap();
        let mut correction = trade(2_000_000, 100.0, 8.0, AggressorSide::Sell);
        correction.trade_id = Some(2);
        chart.update_footprint_trade(footprint, correction).unwrap();
        let after = chart.trade_stream_stats(stream).unwrap();
        assert!(after.revision > before.revision);
        // A correction re-projects dependents from its first affected bar, not from scratch.
        assert!(after.dependent_incremental_updates > before.dependent_incremental_updates);
        assert_eq!(after.dependent_rebuilds, before.dependent_rebuilds);
        assert_eq!(
            chart.data_layer().series_data(cvd).unwrap().1[3]
                .last()
                .copied(),
            Some(-4.0)
        );
        assert_eq!(
            chart.data_layer().series_data(delta).unwrap().1[3]
                .last()
                .copied(),
            Some(-4.0)
        );
        assert_eq!(
            chart.data.point_color(
                delta,
                aeris_charts_core::model::data_layer::PointColorChannel::Body,
                0,
            ),
            Some(Color::rgb(MARKET_DOWN_RGB.0, MARKET_DOWN_RGB.1, MARKET_DOWN_RGB.2).0),
            "corrected negative delta uses the market-down color"
        );
    }

    #[test]
    fn ticks_per_row_groups_adjacent_ticks_into_one_row() {
        let options = FootprintAggregationOptions {
            tick_size: 1.0,
            ticks_per_row: 5,
            ..FootprintAggregationOptions::default()
        };
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let series = chart.add_series(SeriesKind::Footprint);
        chart
            .configure_footprint_series(
                series,
                FootprintSeriesOptions {
                    aggregation: options,
                    ..FootprintSeriesOptions::default()
                },
            )
            .unwrap();
        chart
            .set_footprint_trades(
                series,
                vec![
                    trade(1_000_000, 100.0, 1.0, AggressorSide::Buy),
                    trade(1_100_000, 101.0, 2.0, AggressorSide::Sell),
                    trade(1_200_000, 104.0, 3.0, AggressorSide::Buy),
                    trade(1_300_000, 105.0, 4.0, AggressorSide::Buy),
                ],
            )
            .unwrap();
        let bars = chart.footprint_bars(series).unwrap();
        assert_eq!(bars.len(), 1);
        let levels = &bars[0].levels;
        assert_eq!(
            levels.len(),
            2,
            "100-104 share one row; 105 starts the next"
        );
        let lower = levels.iter().find(|level| level.price == 100.0).unwrap();
        assert_eq!(lower.total_volume, 6.0);
        assert_eq!(lower.bid_volume, 2.0);
        assert_eq!(lower.ask_volume, 4.0);
        assert!(
            levels
                .iter()
                .any(|level| level.price == 105.0 && level.total_volume == 4.0)
        );
        // Bar high/low stay exact trade prices; the row extent covers whole rows.
        assert_eq!((bars[0].low, bars[0].high), (100.0, 105.0));
        assert_eq!(
            footprint_row_price_bounds(&options, 100.0, 105.0),
            (99.5, 109.5)
        );
        // Trades are still validated against the instrument tick, not the row.
        assert!(
            chart
                .set_footprint_trades(
                    series,
                    vec![trade(1_400_000, 100.5, 1.0, AggressorSide::Buy)]
                )
                .is_err()
        );
        assert!(
            validate_options(FootprintAggregationOptions {
                ticks_per_row: 0,
                ..options
            })
            .is_err()
        );
    }

    #[test]
    fn footprint_retention_evicts_shared_studies_with_the_same_bar_boundary() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let stream = chart
            .add_trade_stream("CME:YM", FootprintAggregationOptions::default())
            .unwrap();
        let footprint = chart.add_series(SeriesKind::Footprint);
        chart
            .configure_footprint_series(footprint, FootprintSeriesOptions::default())
            .unwrap();
        chart
            .bind_footprint_series_to_stream(footprint, stream)
            .unwrap();
        let cvd = chart
            .add_cvd_series(stream, 1, TradeStudyOptions::default())
            .unwrap();
        let candles = chart.add_series(SeriesKind::Candlestick);
        chart
            .bind_trade_bar_series_to_stream(candles, stream)
            .unwrap();
        chart.set_series_max_points(footprint, Some(1));
        chart
            .set_footprint_trades(
                footprint,
                vec![
                    trade(1_000_000, 100.0, 1.0, AggressorSide::Buy),
                    trade(61_000_000, 101.0, 2.0, AggressorSide::Buy),
                ],
            )
            .unwrap();
        assert_eq!(chart.footprint_bars(footprint).unwrap().len(), 1);
        assert_eq!(chart.data_layer().series_data(cvd).unwrap().0.len(), 1);
        assert_eq!(chart.data_layer().series_data(candles).unwrap().0.len(), 1);
        // Install, then sealing the evicted bar's trades, then evicting the sealed bar.
        assert_eq!(chart.trade_stream_stats(stream).unwrap().revision, 4);
    }

    #[test]
    fn generic_ohlc_mutations_cannot_desynchronize_footprint_source_truth() {
        let mut chart = ChartEngine::new(800.0, 420.0, 1.0);
        let id = chart.add_series(SeriesKind::Footprint);
        assert!(chart.footprint_bars(id).is_some());
        chart
            .set_footprint_trades(id, vec![trade(1, 100.0, 3.0, AggressorSide::Buy)])
            .unwrap();
        let source_bar = chart.footprint_bar(id, 0).unwrap().clone();

        assert!(!chart.update_series_bar(id, 60.0, [1.0, 2.0, 0.0, 1.0]));
        assert_eq!(chart.series_pop(id, 1), None);
        assert_eq!(
            chart
                .set_series_data(id, &[60.0], &[1.0], &[2.0], &[0.0], &[1.0])
                .unwrap_err(),
            aeris_charts_core::model::data_validation::ValidationError::UnsupportedSeriesData(id)
        );
        assert_eq!(chart.footprint_bar(id, 0), Some(source_bar));
        assert_eq!(chart.data_layer().series_data(id).unwrap().0.len(), 1);

        chart.convert_series_kind(id, SeriesKind::Candlestick);
        assert_eq!(chart.series_kind(id), Some(SeriesKind::Candlestick));
        chart.convert_series_kind(id, SeriesKind::Footprint);
        assert_eq!(chart.series_kind(id), Some(SeriesKind::Candlestick));

        let removable = chart
            .add_footprint_series(FootprintSeriesOptions::default())
            .unwrap();
        chart
            .set_footprint_trades(removable, vec![trade(2, 100.0, 3.0, AggressorSide::Buy)])
            .unwrap();
        assert!(chart.memory_usage().footprint_capacity_bytes > 0);
        assert!(chart.remove_series(removable));
        assert_eq!(chart.memory_usage().footprint_capacity_bytes, 0);
    }

    #[test]
    fn footprint_frame_owns_detail_summary_poc_and_stacked_highlights() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let visual = FootprintVisualOptions {
            font_size: 9.0,
            ..FootprintVisualOptions::default()
        };
        let stacked_ask = visual.stacked_ask_color;
        let poc = visual.poc_color;
        chart
            .configure_footprint_series(
                0,
                FootprintSeriesOptions {
                    aggregation: FootprintAggregationOptions {
                        tick_size: 1.0,
                        ticks_per_row: 1,
                        bars: FootprintBarAggregation::Time {
                            interval_micros: 60_000_000,
                            anchor_micros: 0,
                        },
                        imbalance: FootprintImbalanceOptions {
                            ratio: 3.0,
                            minimum_volume: 20.0,
                            consecutive_levels: 2,
                        },
                    },
                    visual,
                },
            )
            .unwrap();
        chart
            .set_footprint_trades(
                0,
                vec![
                    trade(1, 100.0, 10.0, AggressorSide::Sell),
                    trade(2, 101.0, 40.0, AggressorSide::Buy),
                    trade(3, 101.0, 10.0, AggressorSide::Sell),
                    trade(4, 102.0, 50.0, AggressorSide::Buy),
                ],
            )
            .unwrap();
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        chart.set_bar_spacing(100.0);
        let detailed = chart.build_frame();
        let segment = chart
            .frame_series_segments(0)
            .iter()
            .find(|segment| segment.series_id == Some(0))
            .unwrap();
        let prims = &detailed.panes[0].main[segment.start..segment.end];
        let texts = prims
            .iter()
            .filter_map(|primitive| match primitive {
                Prim::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            texts.contains(&"Δ 70") && texts.contains(&"V 110"),
            "{texts:?}"
        );
        // bid x ask cells print both sides of every row.
        assert!(texts.contains(&"10") && texts.contains(&"40") && texts.contains(&"50"));
        assert!(prims.iter().any(|primitive| {
            matches!(primitive, Prim::Rect { color, .. } if *color == stacked_ask.solid())
        }));
        // POC outlines its row without covering the numbers.
        let poc_outline = |prims: &[Prim]| {
            prims.iter().any(|primitive| {
                matches!(primitive, Prim::RoundRect { fill, border_color, border_width, .. }
                    if fill.a() == 0 && *border_color == poc.solid() && *border_width >= 1.0)
            })
        };
        assert!(poc_outline(prims));
        // The traded range sits on the cluster's left edge in the direction color.
        let up = FootprintVisualOptions::default().ask_color.solid();
        assert!(prims.iter().any(|primitive| {
            matches!(primitive, Prim::Rect { rect, color } if *color == up && rect.w == 2 && rect.h > 1)
        }));
        // Imbalance glyphs are bold so the signal scans at a glance.
        assert!(
            prims.iter().any(|primitive| {
                matches!(primitive, Prim::Text { weight, .. } if *weight == 700)
            })
        );

        chart.set_theme(crate::ChartTheme::Light);
        let expected_text = Color::parse_css(&chart.options.get().layout.text_color).unwrap();
        let themed = chart.build_frame();
        let segment = chart
            .frame_series_segments(0)
            .iter()
            .find(|segment| segment.series_id == Some(0))
            .unwrap();
        let cell_text_colors = themed.panes[0].main[segment.start..segment.end]
            .iter()
            .filter_map(|primitive| match primitive {
                Prim::Text { text, color, .. }
                    if text
                        .chars()
                        .all(|character| character.is_ascii_digit() || character == '-') =>
                {
                    Some(*color)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(!cell_text_colors.is_empty());
        assert!(cell_text_colors.iter().all(|color| *color == expected_text));

        chart.set_bar_spacing(20.0);
        let cells = chart.build_frame();
        let segment = chart
            .frame_series_segments(0)
            .iter()
            .find(|segment| segment.series_id == Some(0))
            .unwrap();
        let prims = &cells.panes[0].main[segment.start..segment.end];
        assert!(
            !prims
                .iter()
                .any(|primitive| matches!(primitive, Prim::Text { .. }))
        );
        assert!(poc_outline(prims));
        assert!(
            prims
                .iter()
                .filter(|primitive| matches!(primitive, Prim::Rect { .. }))
                .count()
                > 2
        );

        chart.set_bar_spacing(3.0);
        let summary = chart.build_frame();
        let segment = chart
            .frame_series_segments(0)
            .iter()
            .find(|segment| segment.series_id == Some(0))
            .unwrap();
        let prims = &summary.panes[0].main[segment.start..segment.end];
        assert!(
            !prims
                .iter()
                .any(|primitive| matches!(primitive, Prim::Text { .. }))
        );
        assert!(prims.iter().any(
            |primitive| matches!(primitive, Prim::Rect { color, .. } if *color == poc.solid())
        ));
    }

    #[test]
    fn footprint_volume_profile_scales_cell_intensity() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let visual = FootprintVisualOptions {
            font_size: 9.0,
            ..FootprintVisualOptions::default()
        };
        let bid = visual.bid_color;
        chart
            .configure_footprint_series(
                0,
                FootprintSeriesOptions {
                    aggregation: FootprintAggregationOptions {
                        tick_size: 1.0,
                        ticks_per_row: 1,
                        bars: FootprintBarAggregation::Time {
                            interval_micros: 60_000_000,
                            anchor_micros: 0,
                        },
                        // Disable imbalance so raw volume heat is directly comparable.
                        imbalance: FootprintImbalanceOptions {
                            ratio: 3.0,
                            minimum_volume: 1_000_000.0,
                            consecutive_levels: 3,
                        },
                    },
                    visual,
                },
            )
            .unwrap();
        chart
            .set_footprint_trades(
                0,
                vec![
                    trade(1, 100.0, 5.0, AggressorSide::Sell),
                    trade(2, 101.0, 50.0, AggressorSide::Sell),
                    // POC settles here so the two compared rows keep raw profile bars.
                    trade(3, 102.0, 200.0, AggressorSide::Sell),
                ],
            )
            .unwrap();
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        chart.set_bar_spacing(100.0);
        let frame = chart.build_frame();
        let segment = chart
            .frame_series_segments(0)
            .iter()
            .find(|segment| segment.series_id == Some(0))
            .unwrap();
        let prims = &frame.panes[0].main[segment.start..segment.end];
        let mut alphas = prims
            .iter()
            .filter_map(|primitive| match primitive {
                Prim::Rect { rect: _, color }
                    if color.r() == bid.r() && color.g() == bid.g() && color.b() == bid.b() =>
                {
                    Some(color.a())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        alphas.sort_unstable();
        alphas.dedup();
        assert!(
            alphas.len() >= 2,
            "quiet and heavy prints must differ in intensity, got {alphas:?}"
        );
        assert!(
            alphas.iter().all(|alpha| *alpha >= 26),
            "faint cells must keep their shape, got {alphas:?}"
        );
        // Profile silhouette: the heavy print's bar must extend further than the
        // quiet print's bar within the same half-width.
        let mut bar_widths = prims
            .iter()
            .filter_map(|primitive| match primitive {
                Prim::Rect { rect, color }
                    if color.r() == bid.r()
                        && color.g() == bid.g()
                        && color.b() == bid.b()
                        && color.a() > 26 =>
                {
                    Some(rect.w)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        bar_widths.sort_unstable();
        bar_widths.dedup();
        assert!(
            bar_widths.len() >= 2,
            "profile bars must grow with volume, got {bar_widths:?}"
        );
    }

    #[test]
    fn footprint_numbers_keep_the_configured_size_in_tall_rows() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart
            .configure_footprint_series(
                0,
                FootprintSeriesOptions {
                    aggregation: FootprintAggregationOptions {
                        tick_size: 1.0,
                        ticks_per_row: 1,
                        bars: FootprintBarAggregation::Time {
                            interval_micros: 60_000_000,
                            anchor_micros: 0,
                        },
                        imbalance: FootprintImbalanceOptions {
                            ratio: 3.0,
                            minimum_volume: 1_000_000.0,
                            consecutive_levels: 3,
                        },
                    },
                    visual: FootprintVisualOptions {
                        font_size: 9.0,
                        ..FootprintVisualOptions::default()
                    },
                },
            )
            .unwrap();
        // Three levels across a tall pane: rows are far taller than 9px, but numbers stay at the
        // configured size so zooming in never balloons the text.
        chart
            .set_footprint_trades(
                0,
                vec![
                    trade(1, 100.0, 5.0, AggressorSide::Sell),
                    trade(2, 101.0, 50.0, AggressorSide::Sell),
                    trade(3, 102.0, 200.0, AggressorSide::Sell),
                ],
            )
            .unwrap();
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        chart.set_bar_spacing(100.0);
        let frame = chart.build_frame();
        let segment = chart
            .frame_series_segments(0)
            .iter()
            .find(|segment| segment.series_id == Some(0))
            .unwrap();
        let sizes = frame.panes[0].main[segment.start..segment.end]
            .iter()
            .filter_map(|primitive| match primitive {
                Prim::Text { size, .. } => Some(*size),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(!sizes.is_empty());
        assert!(
            sizes.iter().all(|size| *size <= 9.0),
            "numbers must stay at the configured 9px, got {sizes:?}"
        );
    }

    #[test]
    fn footprint_summary_drops_out_when_the_bar_cannot_fit_it() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart
            .configure_footprint_series(
                0,
                FootprintSeriesOptions {
                    aggregation: FootprintAggregationOptions {
                        tick_size: 1.0,
                        ticks_per_row: 1,
                        bars: FootprintBarAggregation::Time {
                            interval_micros: 60_000_000,
                            anchor_micros: 0,
                        },
                        ..FootprintAggregationOptions::default()
                    },
                    visual: FootprintVisualOptions {
                        font_size: 9.0,
                        ..FootprintVisualOptions::default()
                    },
                },
            )
            .unwrap();
        chart
            .set_footprint_trades(
                0,
                vec![
                    trade(1, 100.0, 5.0, AggressorSide::Sell),
                    trade(2, 101.0, 50.0, AggressorSide::Sell),
                    trade(3, 102.0, 200.0, AggressorSide::Sell),
                ],
            )
            .unwrap();
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        let summaries = |chart: &mut ChartEngine| {
            chart.build_frame().panes[0]
                .main
                .iter()
                .filter(
                    |primitive| matches!(primitive, Prim::Text { text, .. } if text.contains("Δ")),
                )
                .count()
        };
        chart.set_bar_spacing(100.0);
        assert!(summaries(&mut chart) > 0);
        // Below the numbers threshold the summary drops out with the cell numbers.
        chart.set_bar_spacing(40.0);
        assert_eq!(summaries(&mut chart), 0);
    }

    #[test]
    fn footprint_single_imbalance_highlights_without_stack() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let visual = FootprintVisualOptions {
            font_size: 9.0,
            ..FootprintVisualOptions::default()
        };
        let stacked_ask = visual.stacked_ask_color;
        let single = Color::rgba(stacked_ask.r(), stacked_ask.g(), stacked_ask.b(), 215);
        chart
            .configure_footprint_series(
                0,
                FootprintSeriesOptions {
                    aggregation: FootprintAggregationOptions {
                        tick_size: 1.0,
                        ticks_per_row: 1,
                        bars: FootprintBarAggregation::Time {
                            interval_micros: 60_000_000,
                            anchor_micros: 0,
                        },
                        imbalance: FootprintImbalanceOptions {
                            ratio: 3.0,
                            minimum_volume: 20.0,
                            consecutive_levels: 3,
                        },
                    },
                    visual,
                },
            )
            .unwrap();
        chart
            .set_footprint_trades(
                0,
                vec![
                    trade(1, 100.0, 30.0, AggressorSide::Sell),
                    trade(2, 101.0, 100.0, AggressorSide::Buy),
                    trade(3, 102.0, 5.0, AggressorSide::Buy),
                ],
            )
            .unwrap();
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        chart.set_bar_spacing(100.0);
        let frame = chart.build_frame();
        let segment = chart
            .frame_series_segments(0)
            .iter()
            .find(|segment| segment.series_id == Some(0))
            .unwrap();
        let prims = &frame.panes[0].main[segment.start..segment.end];
        assert!(
            prims
                .iter()
                .any(|primitive| matches!(primitive, Prim::Rect { color, .. } if *color == single)),
            "a lone diagonal imbalance must still highlight its cell"
        );
        assert!(
            !prims.iter().any(
                // The 2px range line shares the up color; only cells count here.
                |primitive| matches!(primitive, Prim::Rect { rect, color }
                    if *color == stacked_ask.solid() && rect.w > 2)
            ),
            "a run shorter than the stacked threshold must not use the stacked treatment"
        );
    }

    #[test]
    fn live_batch_updates_many_ticks_with_one_projection_generation() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        chart
            .configure_footprint_series(
                0,
                FootprintSeriesOptions {
                    aggregation: FootprintAggregationOptions {
                        tick_size: 1.0,
                        ticks_per_row: 1,
                        ..FootprintAggregationOptions::default()
                    },
                    visual: FootprintVisualOptions::default(),
                },
            )
            .unwrap();
        chart
            .set_footprint_trades(0, vec![trade(1, 100.0, 1.0, AggressorSide::Buy)])
            .unwrap();
        let update = chart
            .update_footprint_trades(
                0,
                vec![
                    trade(2, 101.0, 2.0, AggressorSide::Buy),
                    trade(60_000_001, 102.0, 3.0, AggressorSide::Sell),
                    trade(60_000_002, 101.0, 4.0, AggressorSide::Buy),
                ],
            )
            .unwrap();
        assert_eq!(update, FootprintUpdateKind::Tip);
        assert_eq!(chart.data_layer().series_data(0).unwrap().0.len(), 2);
        assert_eq!(chart.footprint_bars(0).unwrap().len(), 2);
        assert_eq!(chart.footprint_work_stats(0).unwrap().incremental_ticks, 3);

        let update = chart
            .update_footprint_trades(
                0,
                vec![
                    trade(3, 100.0, 5.0, AggressorSide::Sell),
                    trade(60_000_003, 102.0, 1.0, AggressorSide::Buy),
                ],
            )
            .unwrap();
        assert_eq!(update, FootprintUpdateKind::Historical);
        assert_eq!(chart.footprint_bars(0).unwrap()[0].delta, -2.0);
    }

    #[test]
    fn retention_evicts_tape_and_bars_together_without_losing_session_delta_seed() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        chart
            .configure_footprint_series(
                0,
                FootprintSeriesOptions {
                    aggregation: FootprintAggregationOptions {
                        tick_size: 1.0,
                        ticks_per_row: 1,
                        ..FootprintAggregationOptions::default()
                    },
                    visual: FootprintVisualOptions::default(),
                },
            )
            .unwrap();
        chart
            .set_footprint_trades(
                0,
                vec![
                    trade(1, 100.0, 10.0, AggressorSide::Buy),
                    trade(60_000_001, 101.0, 5.0, AggressorSide::Sell),
                    trade(120_000_001, 102.0, 2.0, AggressorSide::Buy),
                ],
            )
            .unwrap();
        assert!(chart.set_series_max_points(0, Some(2)));
        let bars = chart.footprint_bars(0).unwrap();
        assert_eq!(bars.len(), 2);
        assert_eq!((bars[0].session_delta, bars[1].session_delta), (5.0, 7.0));
        assert_eq!(
            chart
                .trade_stream(
                    chart
                        .series_entry(0)
                        .unwrap()
                        .footprint
                        .as_ref()
                        .unwrap()
                        .trade_stream_id,
                )
                .unwrap()
                .trades()
                .len(),
            2
        );

        assert_eq!(
            chart
                .update_footprint_trade(0, trade(60_000_002, 101.0, 1.0, AggressorSide::Buy),)
                .unwrap(),
            FootprintUpdateKind::Historical
        );
        let bars = chart.footprint_bars(0).unwrap();
        assert_eq!((bars[0].session_delta, bars[1].session_delta), (6.0, 8.0));
        assert_eq!(chart.data_layer().series_data(0).unwrap().0.len(), 2);
    }

    #[test]
    fn trade_studies_open_at_the_same_small_height_as_other_indicators() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let stream = chart
            .add_trade_stream("pane-sizing", FootprintAggregationOptions::default())
            .unwrap();
        let cvd = chart
            .add_cvd_series(stream, 1, TradeStudyOptions::default())
            .unwrap();
        let delta = chart.add_delta_series(stream, 2).unwrap();
        assert_eq!(chart.series_entry(cvd).unwrap().pane_index, 1);
        assert_eq!(chart.series_entry(delta).unwrap().pane_index, 2);
        assert_eq!(chart.panes[1].stretch_factor, 0.3);
        assert_eq!(chart.panes[2].stretch_factor, 0.3);

        chart.layout_panes(400.0);
        assert_eq!(chart.panes[1].height, chart.panes[2].height);
        assert!(chart.panes[1].height < chart.panes[0].height);

        let custom_pane = chart.add_pane(true).unwrap();
        chart.panes[custom_pane].stretch_factor = 0.8;
        chart
            .add_cvd_series(stream, custom_pane, TradeStudyOptions::default())
            .unwrap();
        assert_eq!(chart.panes[custom_pane].stretch_factor, 0.8);
    }

    #[test]
    fn order_flow_presentation_is_atomic_and_the_footprint_owns_the_price_chrome() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let presentation = chart
            .add_order_flow_presentation(
                "BTC:provider-generation-7",
                0,
                OrderFlowPresentationOptions {
                    aggregation: FootprintAggregationOptions {
                        tick_size: 0.25,
                        ticks_per_row: 0,
                        ..FootprintAggregationOptions::default()
                    },
                    visual: FootprintVisualOptions::default(),
                    show_footprint: true,
                    show_cumulative_delta: true,
                    show_delta_histogram: true,
                    big_trades: Some(crate::BigTradesOptions::default()),
                },
            )
            .unwrap();
        // Automatic rows keep tick truth and merge for display.
        assert_eq!(presentation.ticks_per_row(), 1);
        let footprint = presentation.footprint_series().unwrap();
        assert!(
            chart
                .footprint_series_options(footprint)
                .unwrap()
                .visual
                .adaptive_rows
        );
        assert!(presentation.cumulative_delta_series().is_some());
        assert!(presentation.delta_series().is_some());
        let big_trades = presentation.big_trades().unwrap();
        assert_eq!(chart.panes[1].stretch_factor, 0.3);
        assert_eq!(chart.panes[2].stretch_factor, 0.3);
        let entry = chart.series_entry(footprint).unwrap();
        assert!(entry.last_value_visible && entry.price_line_visible && entry.countdown_visible);
        // The primary is never cut over: the host supplies it as whitespace instead.
        assert_eq!(chart.series_entry(0).unwrap().render_before_time, None);

        chart
            .update_order_flow_presentation(
                presentation,
                vec![trade(60_000_000, 100.0, 2.0, AggressorSide::Buy)],
                false,
            )
            .unwrap();
        assert!(chart.remove_order_flow_presentation(presentation));
        assert_eq!(chart.big_trades_options(big_trades), None);
        assert!(chart.trade_stream(presentation.trade_stream()).is_none());
    }

    fn order_flow_options(show_footprint: bool) -> OrderFlowPresentationOptions {
        OrderFlowPresentationOptions {
            aggregation: FootprintAggregationOptions {
                tick_size: 1.0,
                ticks_per_row: 1,
                ..FootprintAggregationOptions::default()
            },
            visual: FootprintVisualOptions::default(),
            show_footprint,
            show_cumulative_delta: true,
            show_delta_histogram: false,
            big_trades: None,
        }
    }

    #[test]
    fn order_flow_studies_without_a_footprint_keep_the_primary_candles() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let presentation = chart
            .add_order_flow_presentation("CME:ES", 0, order_flow_options(false))
            .unwrap();
        chart
            .update_order_flow_presentation(
                presentation,
                vec![trade(60_000_000, 100.0, 2.0, AggressorSide::Buy)],
                false,
            )
            .unwrap();
        assert_eq!(chart.series_entry(0).unwrap().render_before_time, None);
    }

    #[test]
    fn appended_suffixes_keep_bars_whose_trades_the_host_evicted() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let presentation = chart
            .add_order_flow_presentation("CME:ES", 0, order_flow_options(true))
            .unwrap();
        let footprint = presentation.footprint_series().unwrap();
        chart
            .update_order_flow_presentation(
                presentation,
                vec![
                    trade(1_000_000, 100.0, 3.0, AggressorSide::Buy),
                    trade(61_000_000, 101.0, 1.0, AggressorSide::Sell),
                ],
                false,
            )
            .unwrap();
        // The host's sliding window has since dropped both trades and sends only its new suffix.
        let update = chart
            .update_order_flow_presentation(
                presentation,
                vec![trade(121_000_000, 102.0, 2.0, AggressorSide::Buy)],
                true,
            )
            .unwrap();
        assert_eq!(update, FootprintUpdateKind::Tip);
        let bars = chart.footprint_bars(footprint).unwrap();
        assert_eq!(bars.len(), 3);
        assert_eq!(bars[0].start_timestamp_micros, 0);
        assert_eq!(
            (bars[0].delta, bars[1].delta, bars[2].session_delta),
            (3.0, -1.0, 4.0)
        );
        assert_eq!(
            chart.data_layer().series_data(footprint).unwrap().0.len(),
            3
        );
    }

    #[test]
    fn a_rewritten_window_replaces_only_the_span_it_covers() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let presentation = chart
            .add_order_flow_presentation("CME:ES", 0, order_flow_options(true))
            .unwrap();
        let footprint = presentation.footprint_series().unwrap();
        let cvd = presentation.cumulative_delta_series().unwrap();
        chart
            .update_order_flow_presentation(
                presentation,
                vec![
                    trade(1_000_000, 100.0, 3.0, AggressorSide::Buy),
                    trade(61_000_000, 101.0, 1.0, AggressorSide::Sell),
                    trade(121_000_000, 102.0, 2.0, AggressorSide::Buy),
                    trade(125_000_000, 102.0, 4.0, AggressorSide::Buy),
                ],
                false,
            )
            .unwrap();
        // The host's window no longer holds the first two bars' trades. It corrected the print
        // at 121 s, cancelled the one at 125 s and gained a new bar.
        chart
            .replace_order_flow_window(
                presentation,
                vec![
                    trade(121_000_000, 102.0, 5.0, AggressorSide::Sell),
                    trade(181_000_000, 103.0, 2.0, AggressorSide::Buy),
                ],
            )
            .unwrap();
        let bars = chart.footprint_bars(footprint).unwrap();
        let deltas = bars.iter().map(|bar| bar.delta).collect::<Vec<_>>();
        assert_eq!(deltas, [3.0, -1.0, -5.0, 2.0]);
        assert_eq!(bars[3].session_delta, -1.0);
        assert_eq!(
            chart.data_layer().series_data(footprint).unwrap().0.len(),
            4
        );
        assert_eq!(
            chart.data_layer().series_data(cvd).unwrap().1[3]
                .last()
                .copied(),
            Some(-1.0)
        );

        // A restarted window that removes the newest bars shrinks every dependent with them.
        chart
            .replace_order_flow_window(
                presentation,
                vec![trade(61_000_000, 101.0, 2.0, AggressorSide::Buy)],
            )
            .unwrap();
        let bars = chart.footprint_bars(footprint).unwrap();
        let deltas = bars.iter().map(|bar| bar.delta).collect::<Vec<_>>();
        assert_eq!(deltas, [3.0, 2.0]);
        assert_eq!(
            chart.data_layer().series_data(footprint).unwrap().0.len(),
            2
        );
        assert_eq!(chart.data_layer().series_data(cvd).unwrap().0.len(), 2);

        // An empty window covers no span, so it keeps every bar.
        chart
            .replace_order_flow_window(presentation, Vec::new())
            .unwrap();
        assert_eq!(chart.footprint_bars(footprint).unwrap().len(), 2);
    }

    #[test]
    fn order_flow_raw_tape_is_bounded_by_sealing_whole_oldest_bars() {
        const TRADES_PER_BAR: usize = 16_384;
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let presentation = chart
            .add_order_flow_presentation("CME:ES", 0, order_flow_options(true))
            .unwrap();
        let footprint = presentation.footprint_series().unwrap();
        let cvd = presentation.cumulative_delta_series().unwrap();
        // Every bar is one rapid buy sweep, so each retained bar holds exactly one big order.
        let big_trades = chart
            .add_big_trades(
                presentation.trade_stream(),
                0,
                crate::BigTradesOptions {
                    filter: crate::BigTradesFilter::Fixed {
                        minimum_volume: 1.0,
                    },
                    ..crate::BigTradesOptions::default()
                },
            )
            .unwrap();
        let bar_trades = |bar: usize| {
            (0..TRADES_PER_BAR).map(move |index| {
                trade(
                    (bar as i64) * 60_000_000 + index as i64,
                    100.0,
                    1.0,
                    AggressorSide::Buy,
                )
            })
        };
        let full_bars = ORDER_FLOW_MAX_RETAINED_TRADES / TRADES_PER_BAR;
        chart
            .update_order_flow_presentation(
                presentation,
                (0..full_bars).flat_map(bar_trades).collect(),
                false,
            )
            .unwrap();
        assert_eq!(chart.footprint_bars(footprint).unwrap().len(), full_bars);

        chart
            .update_order_flow_presentation(presentation, bar_trades(full_bars).collect(), true)
            .unwrap();
        let stream = chart.trade_stream(presentation.trade_stream()).unwrap();
        let budget = ORDER_FLOW_MAX_RETAINED_TRADES
            - ORDER_FLOW_MAX_RETAINED_TRADES / crate::CAP_TRIM_MARGIN_DIVISOR;
        assert!(stream.trades().len() <= budget);
        let bars = chart.footprint_bars(footprint).unwrap();
        assert_eq!(bars.len(), full_bars + 1, "sealed bars stay as history");
        let sealed = stream.sealed_bar_count();
        assert!(sealed > 0);
        assert_eq!(
            (bars.len() - sealed) * TRADES_PER_BAR,
            stream.trades().len()
        );
        assert_eq!(bars[0].start_timestamp_micros, 0);
        assert_eq!(
            bars.last().unwrap().start_timestamp_micros,
            full_bars as i64 * 60_000_000
        );
        assert_eq!(
            chart.data_layer().series_data(footprint).unwrap().0.len(),
            bars.len()
        );
        assert_eq!(
            chart.data_layer().series_data(cvd).unwrap().0.len(),
            bars.len()
        );
        assert_eq!(
            bars.last().unwrap().session_delta,
            ((full_bars + 1) * TRADES_PER_BAR) as f64
        );
        let bubbles = chart.big_trades_snapshot(big_trades).unwrap().bubbles;
        assert_eq!(bubbles.len(), bars.len(), "sealed bars keep their orders");
        assert_eq!(bubbles[0].bar_time, 0);
        assert_eq!(bubbles[0].volume, TRADES_PER_BAR as f64);

        // A late print inside the raw tape replays from the sealed state, keeping sealed orders.
        let late = trade(
            full_bars as i64 * 60_000_000 - 1_000_000,
            100.0,
            1.0,
            AggressorSide::Sell,
        );
        assert_eq!(
            chart
                .update_order_flow_presentation(presentation, vec![late], true)
                .unwrap(),
            FootprintUpdateKind::Historical
        );
        let bubbles = chart.big_trades_snapshot(big_trades).unwrap().bubbles;
        assert_eq!(
            bubbles.len(),
            bars.len() + 1,
            "the late sell is one more order"
        );
        assert_eq!(bubbles[0].bar_time, 0);

        // A print older than the raw tape would rewrite final history and is skipped.
        let skipped = trade(30_000_000, 100.0, 1.0, AggressorSide::Sell);
        chart
            .update_order_flow_presentation(presentation, vec![skipped], true)
            .unwrap();
        let stream = chart.trade_stream(presentation.trade_stream()).unwrap();
        assert_eq!(stream.work_stats().skipped_sealed_trades, 1);
        assert_eq!(chart.footprint_bars(footprint).unwrap()[0], bars[0]);
    }

    /// Consecutive one-minute bars; session `i` spans `sessions[i].0` minutes of
    /// `sessions[i].1` trades each.
    fn session_tape(sessions: &[(usize, usize)]) -> Vec<FootprintTrade> {
        let mut minute = 0_i64;
        let mut tape = Vec::new();
        for (session, &(minutes, minute_trades)) in sessions.iter().enumerate() {
            for _ in 0..minutes {
                for index in 0..minute_trades {
                    let mut print = trade(
                        minute * 60_000_000 + index as i64,
                        100.0 + (index % 4) as f64,
                        1.0,
                        if index % 2 == 0 {
                            AggressorSide::Buy
                        } else {
                            AggressorSide::Sell
                        },
                    );
                    print.session_id = Some(session as u64);
                    tape.push(print);
                }
                minute += 1;
            }
        }
        tape
    }

    /// The order-flow tape budget evicts like series retention: on time and sequence-axis bars,
    /// with or without studies to anchor the row trim, big trades drop the evicted bars' orders in
    /// place, fold only the appended prints, and never replay the retained tape.
    #[test]
    fn order_flow_budget_evicts_big_trades_in_place_with_or_without_presentations() {
        const TRADES_PER_BAR: usize = 16_384;
        for (sequence_axis, show_cumulative_delta) in
            [(false, true), (false, false), (true, true), (true, false)]
        {
            let mut options = order_flow_options(false);
            if sequence_axis {
                options.aggregation.bars = FootprintBarAggregation::Trades {
                    trades_per_bar: TRADES_PER_BAR as u32,
                };
            }
            let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
            let presentation = chart
                .add_order_flow_presentation(
                    "CME:ES",
                    0,
                    OrderFlowPresentationOptions {
                        show_cumulative_delta,
                        big_trades: Some(crate::BigTradesOptions {
                            filter: crate::BigTradesFilter::Fixed {
                                minimum_volume: 1.0,
                            },
                            ..crate::BigTradesOptions::default()
                        }),
                        ..options
                    },
                )
                .unwrap();
            let stream_id = presentation.trade_stream();
            let big_trades = presentation.big_trades().unwrap();
            let bar_trades = |bar: usize| {
                (0..TRADES_PER_BAR).map(move |index| {
                    trade(
                        (bar as i64) * 60_000_000 + index as i64,
                        100.0,
                        1.0,
                        AggressorSide::Buy,
                    )
                })
            };
            let full_bars = ORDER_FLOW_MAX_RETAINED_TRADES / TRADES_PER_BAR;
            chart
                .update_order_flow_presentation(
                    presentation,
                    (0..full_bars).flat_map(bar_trades).collect(),
                    false,
                )
                .unwrap();
            let before = chart.trade_stream_stats(stream_id).unwrap();
            chart
                .update_order_flow_presentation(presentation, bar_trades(full_bars).collect(), true)
                .unwrap();
            let after = chart.trade_stream_stats(stream_id).unwrap();
            let context =
                format!("sequence axis: {sequence_axis}, studies shown: {show_cumulative_delta}");
            assert_eq!(
                (
                    after.big_trades_prints_scanned - before.big_trades_prints_scanned,
                    after.big_trades_replays - before.big_trades_replays,
                ),
                (TRADES_PER_BAR as u64, 0),
                "{context}"
            );
            let stream = chart.trade_stream(stream_id).unwrap();
            let budget = ORDER_FLOW_MAX_RETAINED_TRADES
                - ORDER_FLOW_MAX_RETAINED_TRADES / crate::CAP_TRIM_MARGIN_DIVISOR;
            assert!(stream.trades().len() <= budget, "{context}");
            let bars = stream.bars().to_vec();
            if let Some(cvd) = presentation.cumulative_delta_series() {
                assert_eq!(
                    chart.data_layer().series_data(cvd).unwrap().0.len(),
                    bars.len()
                );
            }
            let bubbles = chart.big_trades_snapshot(big_trades).unwrap().bubbles;
            assert_eq!(bubbles.len(), bars.len(), "{context}");
            // Orders key the first retained bar by its row key on a sequence axis: the
            // studies' first retained row, or position 0 of a stream without presentations.
            let first_key = match (sequence_axis, presentation.cumulative_delta_series()) {
                (false, _) => bars[0].start_timestamp_micros / MICROS_PER_SECOND,
                (true, Some(cvd)) => chart.data_layer().series_data(cvd).unwrap().0[0],
                (true, None) => 0,
            };
            assert_eq!(bubbles[0].bar_time, first_key, "{context}");
            assert!(
                bubbles
                    .iter()
                    .zip(bubbles.iter().skip(1))
                    .all(|(a, b)| b.bar_time == a.bar_time + i64::from(sequence_axis)
                        || !sequence_axis),
                "{context}"
            );
        }
    }

    #[test]
    fn sealed_history_keeps_at_most_the_session_budget() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let presentation = chart
            .add_order_flow_presentation("CME:ES", 0, order_flow_options(true))
            .unwrap();
        let footprint = presentation.footprint_series().unwrap();
        let cvd = presentation.cumulative_delta_series().unwrap();
        // Six small sessions, then a seventh large enough to push them all out of the raw tape.
        let large = ORDER_FLOW_MAX_RETAINED_TRADES / 50_000 + 1;
        let mut sessions = vec![(1, 10); 6];
        sessions.push((large, 50_000));
        let tape = session_tape(&sessions);
        chart
            .update_order_flow_presentation(presentation, tape, false)
            .unwrap();
        // Seven bars were sealed: the six small sessions and the large session's first minute.
        // The two oldest sessions then exceed the session budget.
        let stream = chart.trade_stream(presentation.trade_stream()).unwrap();
        assert_eq!(stream.sealed_bar_count(), 5);
        let bars = chart.footprint_bars(footprint).unwrap();
        let sessions = bars
            .iter()
            .map(|bar| bar.session_id)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(sessions.len(), ORDER_FLOW_MAX_RETAINED_SESSIONS);
        assert_eq!(
            bars[0].session_id,
            Some(2),
            "the two oldest sessions were evicted"
        );
        assert_eq!(bars.len(), 4 + large);
        assert_eq!(bars[0].logical_index, 0);
        assert_eq!(
            chart.data_layer().series_data(footprint).unwrap().0.len(),
            bars.len()
        );
        assert_eq!(
            chart.data_layer().series_data(cvd).unwrap().0.len(),
            bars.len()
        );
        // Evicted history cannot be extended further back.
        let stats = chart
            .prepend_order_flow_history(
                presentation,
                vec![trade(0, 100.0, 1.0, AggressorSide::Buy)],
            )
            .unwrap();
        assert_eq!(
            stats,
            HistoryPrefixStats {
                accepted_trades: 0,
                skipped_trades: 1,
                history_full: true,
            }
        );
    }

    #[test]
    fn sealed_history_stays_within_the_memory_budget() {
        let mut aggregator = FootprintAggregator::new(late_test_options()).unwrap();
        aggregator.set_trades(late_test_tape(20_000)).unwrap();
        aggregator.seal_bars(aggregator.bars().len() - 1);
        let total = aggregator.capacity_bytes();
        assert_eq!(aggregator.bars_within_history_budget(5, total), None);
        let keep = aggregator
            .bars_within_history_budget(5, total / 2)
            .expect("over budget");
        let target = total / 2 - total / 2 / crate::CAP_TRIM_MARGIN_DIVISOR;
        aggregator.evict_sealed_bars(aggregator.bars().len() - keep);
        assert!(aggregator.capacity_bytes() <= target);
        assert_eq!(aggregator.bars().len(), keep);
    }

    #[test]
    fn replay_inside_sealed_history_reveals_bars_by_their_close() {
        let mut aggregator = FootprintAggregator::new(late_test_options()).unwrap();
        aggregator.set_trades(late_test_tape(2_000)).unwrap();
        let original = aggregator.bars().to_vec();
        aggregator.seal_bars(100);
        let clock = original[50].end_timestamp_micros;
        aggregator.set_replay_clock_micros(Some(clock)).unwrap();
        assert_eq!(aggregator.bars(), &original[..51]);
        assert_eq!(aggregator.trades().len(), 0);
        // Retention never evicts history the replay clock hides.
        assert_eq!(aggregator.bars_within_history_budget(1, 0), None);
        let inside_raw = original[150].end_timestamp_micros;
        aggregator
            .set_replay_clock_micros(Some(inside_raw))
            .unwrap();
        assert_eq!(aggregator.bars(), &original[..151]);
        aggregator.set_replay_clock_micros(None).unwrap();
        assert_eq!(aggregator.bars(), original.as_slice());
    }

    /// `late_test_tape` without unknown aggressors, so classification never depends on
    /// trades outside a page.
    fn classified_tape(count: i64) -> Vec<FootprintTrade> {
        late_test_tape(count)
            .into_iter()
            .map(|mut print| {
                if print.aggressor == AggressorSide::Unknown {
                    print.aggressor = AggressorSide::Buy;
                }
                print
            })
            .collect()
    }

    #[test]
    fn prepended_history_matches_one_aggregation_of_the_whole_tape() {
        let tape = classified_tape(4_000);
        let mut whole = FootprintAggregator::new(late_test_options()).unwrap();
        whole.set_trades(tape.clone()).unwrap();

        // Into the raw tape: rebuilt exactly.
        let mut raw = FootprintAggregator::new(late_test_options()).unwrap();
        raw.set_trades(tape[1_500..].to_vec()).unwrap();
        let (stats, page) = raw.prepend_history(tape.clone()).unwrap();
        assert!(page.is_none());
        assert_eq!(
            stats,
            HistoryPrefixStats {
                accepted_trades: 1_500,
                skipped_trades: 2_500,
                history_full: false,
            }
        );
        assert_eq!(raw.bars(), whole.bars());

        // Into sealed history, across a time bar the page shares with the old front.
        let mut sealed = FootprintAggregator::new(late_test_options()).unwrap();
        sealed.set_trades(tape[1_505..].to_vec()).unwrap();
        sealed.seal_bars(100);
        let (stats, page) = sealed.prepend_history(tape[..1_505].to_vec()).unwrap();
        assert_eq!(stats.accepted_trades, 1_505);
        assert!(page.is_some());
        assert_eq!(sealed.bars().len(), whole.bars().len());
        for (joined, expected) in sealed.bars().iter().zip(whole.bars()) {
            assert_eq!(joined.levels, expected.levels);
            assert_eq!(
                (joined.delta, joined.session_delta, joined.trade_count),
                (expected.delta, expected.session_delta, expected.trade_count)
            );
            assert_eq!(
                (joined.max_delta, joined.min_delta, joined.logical_index),
                (
                    expected.max_delta,
                    expected.min_delta,
                    expected.logical_index
                )
            );
        }
        // The raw tape keeps aggregating exactly after the joined history.
        let next = classified_tape(4_010)[4_000..].to_vec();
        sealed.update_trades(next.clone()).unwrap();
        whole.update_trades(next).unwrap();
        assert_eq!(sealed.bars().last(), whole.bars().last());
    }

    #[test]
    fn prepended_pages_add_their_big_orders_in_front_of_sealed_history() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let mut options = order_flow_options(true);
        options.big_trades = Some(crate::BigTradesOptions {
            filter: crate::BigTradesFilter::Fixed {
                minimum_volume: 40.0,
            },
            ..crate::BigTradesOptions::default()
        });
        let presentation = chart
            .add_order_flow_presentation("CME:ES", 0, options)
            .unwrap();
        let big_trades = presentation.big_trades().unwrap();
        let block = |minute: i64| trade(minute * 60_000_000, 100.0, 50.0, AggressorSide::Buy);
        chart
            .update_order_flow_presentation(presentation, vec![block(10), block(11)], false)
            .unwrap();
        let stream_id = presentation.trade_stream();
        chart.seal_trade_stream_bars(stream_id, 1);
        assert_eq!(chart.trade_stream(stream_id).unwrap().sealed_bar_count(), 1);
        let stats = chart
            .prepend_order_flow_history(presentation, vec![block(2), block(5)])
            .unwrap();
        assert_eq!(stats.accepted_trades, 2);
        let footprint = presentation.footprint_series().unwrap();
        let bars = chart.footprint_bars(footprint).unwrap();
        assert_eq!(bars.len(), 4);
        assert_eq!(bars[0].start_timestamp_micros, 120_000_000);
        let times = chart
            .big_trades_snapshot(big_trades)
            .unwrap()
            .bubbles
            .iter()
            .map(|order| order.bar_time)
            .collect::<Vec<_>>();
        assert_eq!(times, vec![120, 300, 600, 660]);
    }

    #[test]
    fn evicting_sequence_bars_renumbers_their_big_orders() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let stream = chart
            .add_trade_stream(
                "CME:ES",
                FootprintAggregationOptions {
                    tick_size: 1.0,
                    bars: FootprintBarAggregation::Trades { trades_per_bar: 1 },
                    ..FootprintAggregationOptions::default()
                },
            )
            .unwrap();
        let id = chart
            .add_big_trades(
                stream,
                0,
                crate::BigTradesOptions {
                    filter: crate::BigTradesFilter::Fixed {
                        minimum_volume: 1.0,
                    },
                    ..crate::BigTradesOptions::default()
                },
            )
            .unwrap();
        chart
            .set_trade_stream_trades(
                stream,
                (0..5)
                    .map(|second| trade(second * 1_000_000, 100.0, 1.0, AggressorSide::Buy))
                    .collect(),
            )
            .unwrap();
        let positions = |chart: &ChartEngine| {
            chart
                .big_trades_snapshot(id)
                .unwrap()
                .bubbles
                .iter()
                .map(|order| (order.bar_time, order.start_timestamp_micros))
                .collect::<Vec<_>>()
        };
        assert_eq!(positions(&chart)[0], (0, 0));
        chart.trim_trade_stream_front(stream, None, 3);
        assert_eq!(
            positions(&chart),
            vec![(0, 2_000_000), (1, 3_000_000), (2, 4_000_000)]
        );
        assert_eq!(chart.trade_stream(stream).unwrap().bars().len(), 3);

        // With a presentation the orders carry its row keys, which retention never re-keys: the
        // retained orders keep the keys their bars keep, past the evicted ones.
        let candles = chart.add_series(SeriesKind::Candlestick);
        chart
            .bind_trade_bar_series_to_stream(candles, stream)
            .unwrap();
        chart
            .set_trade_stream_trades(
                stream,
                (0..5)
                    .map(|second| trade(second * 1_000_000, 100.0, 1.0, AggressorSide::Buy))
                    .collect(),
            )
            .unwrap();
        chart.trim_trade_stream_front(stream, Some(candles), 3);
        let keys = chart.data_layer().series_data(candles).unwrap().0.to_vec();
        assert_eq!(keys, [2, 3, 4]);
        assert_eq!(
            positions(&chart),
            vec![(2, 2_000_000), (3, 3_000_000), (4, 4_000_000)]
        );
    }

    #[test]
    fn row_and_imbalance_changes_regroup_sealed_history_in_place() {
        let tape = classified_tape(3_000);
        let mut live = FootprintAggregator::new(late_test_options()).unwrap();
        live.set_trades(tape.clone()).unwrap();
        live.seal_bars(200);
        let rebuilt_before = live.work_stats().rebuilt_ticks;
        let imbalance = FootprintImbalanceOptions {
            ratio: 2.0,
            minimum_volume: 1.0,
            consecutive_levels: 2,
        };
        assert!(live.set_row_options(2, imbalance).unwrap());
        assert_eq!(live.work_stats().rebuilt_ticks, rebuilt_before);
        assert_eq!(live.sealed_bar_count(), 200);

        let mut expected = FootprintAggregator::new(FootprintAggregationOptions {
            ticks_per_row: 2,
            imbalance,
            ..late_test_options()
        })
        .unwrap();
        expected.set_trades(tape).unwrap();
        for (bar, fresh) in live.bars().iter().zip(expected.bars()) {
            assert_eq!(
                live.presented_bar(bar).as_ref(),
                expected.presented_bar(fresh).as_ref()
            );
        }
        assert_eq!(
            merged_poc_price(&live.bars()[7], 2, 2.0),
            live.presented_bar(&live.bars()[7]).poc_price
        );
    }

    #[test]
    fn reconfiguring_a_presentation_keeps_its_stream_history_and_orders() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let mut options = order_flow_options(false);
        options.show_cumulative_delta = false;
        let mut presentation = chart
            .add_order_flow_presentation("CME:ES", 0, options.clone())
            .unwrap();
        let stream_id = presentation.trade_stream();
        let block = |minute: i64| trade(minute * 60_000_000, 100.0, 50.0, AggressorSide::Buy);
        chart
            .update_order_flow_presentation(presentation, vec![block(1), block(2)], false)
            .unwrap();
        let revision = chart.trade_stream(stream_id).unwrap().revision();

        let mut next = options.clone();
        next.show_footprint = true;
        next.show_cumulative_delta = true;
        next.show_delta_histogram = true;
        next.aggregation.ticks_per_row = 4;
        next.big_trades = Some(crate::BigTradesOptions {
            filter: crate::BigTradesFilter::Fixed {
                minimum_volume: 40.0,
            },
            ..crate::BigTradesOptions::default()
        });
        chart
            .reconfigure_order_flow_presentation(&mut presentation, next.clone())
            .unwrap();
        assert_eq!(presentation.trade_stream(), stream_id);
        assert_eq!(presentation.ticks_per_row(), 4);
        let footprint = presentation.footprint_series().unwrap();
        assert_eq!(chart.footprint_bars(footprint).unwrap().len(), 2);
        let cvd = presentation.cumulative_delta_series().unwrap();
        assert_eq!(chart.data_layer().series_data(cvd).unwrap().0.len(), 2);
        assert!(presentation.delta_series().is_some());
        let big_trades = presentation.big_trades().unwrap();
        assert_eq!(
            chart.big_trades_snapshot(big_trades).unwrap().bubbles.len(),
            2
        );
        assert_eq!(
            chart.trade_stream(stream_id).unwrap().revision(),
            revision + 1,
            "only the row change touched the stream"
        );

        // Hiding the footprint moves its orders to the price series instead of dropping them.
        let mut hidden = next.clone();
        hidden.show_footprint = false;
        hidden.show_delta_histogram = false;
        chart
            .reconfigure_order_flow_presentation(&mut presentation, hidden)
            .unwrap();
        assert_eq!(presentation.footprint_series(), None);
        assert_eq!(presentation.delta_series(), None);
        assert_eq!(presentation.big_trades(), Some(big_trades));
        assert_eq!(
            chart.big_trades_snapshot(big_trades).unwrap().bubbles.len(),
            2
        );
        assert!(
            !chart
                .series_entry(footprint)
                .is_some_and(|entry| !entry.removed)
        );

        let mut regridded = next;
        regridded.aggregation.tick_size = 0.5;
        assert_eq!(
            chart.reconfigure_order_flow_presentation(&mut presentation, regridded),
            Err(FootprintError::InvalidAggregation)
        );
        assert!(chart.remove_order_flow_presentation(presentation));
        assert!(chart.trade_stream(stream_id).is_none());
    }

    #[test]
    fn whitespace_primary_draws_only_tape_covered_footprints_at_a_legible_zoom() {
        let mut chart = ChartEngine::new(1200.0, 600.0, 1.0);
        // Ten one-minute slots; the tape only covers the last two.
        let times = (0..10)
            .map(|index| f64::from(index) * 60.0)
            .collect::<Vec<_>>();
        let nan = vec![f64::NAN; times.len()];
        chart
            .set_series_data(0, &times, &nan, &nan, &nan, &nan)
            .unwrap();
        let presentation = chart
            .add_order_flow_presentation(
                "BTC",
                0,
                OrderFlowPresentationOptions {
                    aggregation: FootprintAggregationOptions {
                        tick_size: 1.0,
                        ticks_per_row: 0,
                        ..FootprintAggregationOptions::default()
                    },
                    visual: FootprintVisualOptions::default(),
                    show_footprint: true,
                    show_cumulative_delta: false,
                    show_delta_histogram: false,
                    big_trades: None,
                },
            )
            .unwrap();
        let footprint = presentation.footprint_series().unwrap();
        let tape = (0..200)
            .map(|index| {
                let minute = 8 + index / 100;
                let side = if index % 3 == 0 {
                    AggressorSide::Sell
                } else {
                    AggressorSide::Buy
                };
                trade(
                    i64::from(minute) * 60_000_000 + i64::from(index),
                    100.0 + f64::from(index % 40),
                    1.0,
                    side,
                )
            })
            .collect();
        chart
            .update_order_flow_presentation(presentation, tape, false)
            .unwrap();
        chart.fit_footprint_viewport();
        assert_eq!(chart.time_scale.bar_spacing(), FOOTPRINT_BAR_SPACING);

        let frame = chart.build_frame();
        let primary = chart
            .frame_series_segments(0)
            .iter()
            .filter(|segment| segment.series_id == Some(0))
            .map(|segment| segment.end - segment.start)
            .sum::<usize>();
        assert_eq!(primary, 0, "a whitespace primary draws no candles");
        let segment = chart
            .frame_series_segments(0)
            .iter()
            .find(|segment| segment.series_id == Some(footprint))
            .unwrap();
        let prims = &frame.panes[0].main[segment.start..segment.end];
        let summaries = prims
            .iter()
            .filter(
                |primitive| matches!(primitive, Prim::Text { text, .. } if text.starts_with('Δ')),
            )
            .count();
        assert_eq!(summaries, 2, "only the two tape-covered bars draw clusters");
        let font = FootprintVisualOptions::default().font_size;
        let row_heights = prims.iter().filter_map(|primitive| match primitive {
            Prim::RoundRect { h, .. } => Some(*h),
            _ => None,
        });
        for height in row_heights {
            assert!(
                f64::from(height) + 1.0 >= font + 5.0,
                "adaptive rows stay legible, got {height}px"
            );
        }
    }

    #[test]
    fn display_rows_merge_in_one_two_five_steps_until_legible() {
        assert_eq!(footprint_row_merge(20.0, 16.0), 1);
        assert_eq!(footprint_row_merge(9.0, 16.0), 2);
        assert_eq!(footprint_row_merge(4.0, 16.0), 5);
        assert_eq!(footprint_row_merge(0.1, 16.0), 200);
        assert_eq!(footprint_row_merge(f64::NAN, 16.0), 1);
        assert_eq!(footprint_row_merge(1e-12, 16.0), MAXIMUM_ROW_MERGE);
    }

    #[test]
    fn merged_display_rows_sum_volumes_and_recompute_imbalance_and_poc() {
        let mut aggregator = FootprintAggregator::new(FootprintAggregationOptions {
            tick_size: 1.0,
            ticks_per_row: 1,
            bars: FootprintBarAggregation::Time {
                interval_micros: 60_000_000,
                anchor_micros: 0,
            },
            imbalance: FootprintImbalanceOptions {
                ratio: 3.0,
                minimum_volume: 1.0,
                consecutive_levels: 2,
            },
        })
        .unwrap();
        aggregator
            .set_trades(vec![
                trade(1, 100.0, 1.0, AggressorSide::Sell),
                trade(2, 101.0, 1.0, AggressorSide::Sell),
                trade(3, 102.0, 4.0, AggressorSide::Buy),
                trade(4, 103.0, 4.0, AggressorSide::Buy),
                trade(5, 104.0, 9.0, AggressorSide::Buy),
            ])
            .unwrap();
        let bar = &aggregator.bars()[0];
        let merged = merged_footprint_bar(bar, 2, aggregator.options().imbalance, 2.0);
        let rows = merged
            .levels
            .iter()
            .map(|level| (level.level, level.price, level.bid_volume, level.ask_volume))
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            [
                (50, 100.0, 2.0, 0.0),
                (51, 102.0, 0.0, 8.0),
                (52, 104.0, 0.0, 9.0)
            ]
        );
        assert!(merged.levels[1].ask_imbalance, "8 asks over 2 bids below");
        assert!(merged.levels[2].stacked_ask_imbalance);
        assert_eq!(merged.poc_level, 52);
        assert_eq!(merged.total_volume, bar.total_volume);
    }

    #[test]
    fn correcting_a_retained_trade_after_sealing_repairs_auction_marks_like_a_fresh_build() {
        // Fork guarantee (X17): retention seals the oldest bars and advances `trade_id_base`, so
        // the auction repair reads a corrected print's original timestamp at its deque index
        // (`position - trade_id_base`), never at the absolute position, which names a later print.
        let bars = FootprintBarAggregation::Time {
            interval_micros: 60_000_000,
            anchor_micros: 0,
        };
        let new_chart = || {
            let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
            let stream = chart
                .add_trade_stream(
                    "auction",
                    FootprintAggregationOptions {
                        tick_size: 1.0,
                        bars,
                        ..FootprintAggregationOptions::default()
                    },
                )
                .unwrap();
            chart
                .configure_footprint_series(0, FootprintSeriesOptions::default())
                .unwrap();
            chart.bind_footprint_series_to_stream(0, stream).unwrap();
            (chart, stream)
        };
        let two_sided = |minute: i64| {
            let time = minute * 60_000_000 + 1;
            let id = minute as u64 * 2;
            let mut buy = trade(time, 100.0, 3.0, AggressorSide::Buy);
            buy.trade_id = Some(id);
            let mut sell = trade(time + 1, 100.0, 4.0, AggressorSide::Sell);
            sell.trade_id = Some(id + 1);
            [buy, sell]
        };
        let mut tape = (0..20).flat_map(two_sided).collect::<Vec<_>>();
        let (mut chart, stream) = new_chart();
        chart.set_trade_stream_trades(stream, tape.clone()).unwrap();
        let options = crate::AuctionMarkerOptions::default();
        let id = chart
            .add_auction_markers(stream, 0, options.clone())
            .unwrap();
        assert!(chart.set_series_max_points(0, Some(18)));
        let retained = chart.trade_stream(stream).unwrap();
        assert!(retained.trade_id_base > 0, "retention must seal raw trades");
        let first_time = retained.bars()[0].start_timestamp_micros / 1_000_000;
        assert!(
            chart
                .auction_markers_snapshot(id)
                .unwrap()
                .iter()
                .any(|mark| mark.bar_time == 600),
            "minute 10 must carry an auction mark the repair has to remove"
        );

        // Move minute 10's sell (still retained) into minute 11: minute 10 loses its unfinished
        // auction. Its absolute tape position is a later print's deque index.
        let mut corrected = tape[21].clone();
        corrected.timestamp_micros = 11 * 60_000_000 + 3;
        assert_eq!(
            chart
                .update_trade_stream_trade(stream, corrected.clone())
                .unwrap(),
            FootprintUpdateKind::Historical
        );
        tape[21] = corrected;
        tape.sort_by_key(|print| print.timestamp_micros);

        let (mut fresh, fresh_stream) = new_chart();
        fresh.set_trade_stream_trades(fresh_stream, tape).unwrap();
        let fresh_id = fresh.add_auction_markers(fresh_stream, 0, options).unwrap();
        let expected = fresh
            .auction_markers_snapshot(fresh_id)
            .unwrap()
            .into_iter()
            .filter(|mark| mark.bar_time >= first_time)
            .collect::<Vec<_>>();
        assert!(!expected.iter().any(|mark| mark.bar_time == 600));
        assert!(expected.iter().any(|mark| mark.bar_time == 660));
        assert_eq!(chart.auction_markers_snapshot(id).unwrap(), expected);
    }
}
