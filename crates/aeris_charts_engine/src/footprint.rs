//! Tick-truth aggregation for footprint / numbers-bar series.
//!
//! The retained trade tape is authoritative. Bars, price levels, delta extrema, POC, and
//! imbalances are derived state and can be rebuilt deterministically after a late event. Rendering
//! consumes [`FootprintBar`] values; it never attempts to infer order flow from OHLC rows.

use std::collections::{HashMap, VecDeque};

use aeris_charts_core::model::data_layer::{PointColorChannel, SeriesId, SeriesIdError};
use aeris_charts_core::model::data_validation::{MAX_SAFE_VALUE, MIN_SAFE_VALUE};
use aeris_charts_core::scale::exchange_time::ExchangeTime;
use aeris_charts_core::scale::session_slots::{
    OutOfSessionPolicy, SessionBarGrid, SessionSlotError, SessionWindow,
};
use aeris_charts_core::style::{MARKET_DOWN_RGB, MARKET_UP_RGB};
use aeris_charts_render::color::Color;

use crate::{
    marker_pos, marker_shape, ChartEngine, Marker, PriceFormatKind, SeriesKind, SeriesOwner,
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
    /// Lifetime tape trades folded into bubble markers. A live tip folds only its new trades.
    #[serde(default)]
    pub bubble_trades_scanned: u64,
    /// Lifetime bubble marker sizes computed. A tip sizes its new or merged bubbles and rescales
    /// every retained marker only when the peak bubble volume changes.
    #[serde(default)]
    pub bubble_markers_sized: u64,
}

/// Lifetime work telemetry of one stream's chart dependents (see [`TradeStreamStats`]).
#[derive(Clone, Copy, Debug, Default)]
struct TradeDependentWork {
    study_rows_computed: u64,
    bar_rows_projected: u64,
    bubble_trades_scanned: u64,
    bubble_markers_sized: u64,
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TradeBubbleOptions {
    pub minimum_volume: f64,
    pub max_markers: usize,
    pub aggregation_window_micros: i64,
}

impl Default for TradeBubbleOptions {
    fn default() -> Self {
        Self {
            minimum_volume: 0.0,
            max_markers: 2_048,
            aggregation_window_micros: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TradeBubbleDependent {
    pub series_id: SeriesId,
    pub options: TradeBubbleOptions,
    pub applied_revision: u64,
    /// Resumable fold behind the series markers; `None` forces the next refresh to refold.
    fold: Option<BubbleFold>,
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
    /// Exact integer row identity. `price == level * row_size()` is the row's lowest tick.
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

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
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
        if let Some(&(old_index, new_index)) = self.common_indices.get(upper) {
            if old_index as f64 == logical {
                return new_index as f64;
            }
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

    /// Whole-second open of the time bar a print joins, the time its bubble marker must carry:
    /// markers snap an off-grid time to the next bar, and session anchoring folds auction and
    /// lunch prints into bars they are not stamped in. `None` when the session policy leaves the
    /// print out of every bar. Other aggregations have no time grid and return the print's second.
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

    pub fn bars(&self) -> &[FootprintBar] {
        &self.bars
    }

    /// Logical bar positions and their full-resolution temporal bounds. This is the boundary that
    /// future non-time axes and drawing rebasing consume; the existing whole-second projection is
    /// deliberately kept separate until that axis is chart-integrated.
    pub fn bar_sequence(&self) -> BarSequence<'_> {
        BarSequence { bars: &self.bars }
    }

    pub fn trades(&self) -> impl ExactSizeIterator<Item = &FootprintTrade> {
        self.trades
            .range(..self.visible_trade_count())
            .map(|trade| &trade.event)
    }

    pub(crate) fn classified_trades(
        &self,
    ) -> impl ExactSizeIterator<Item = (&FootprintTrade, AggressorSide)> {
        self.trades
            .range(..self.visible_trade_count())
            .map(|trade| (&trade.event, trade.classified_side))
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
            + self.bars.capacity() * core::mem::size_of::<FootprintBar>()
            + self.replay_checkpoints.capacity() * core::mem::size_of::<ReplayCheckpoint>()
            + self
                .bars
                .iter()
                .map(|bar| bar.levels.capacity() * core::mem::size_of::<FootprintLevel>())
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

    /// Evict complete bars from the front until `keep` remain, together with exactly the trades
    /// they aggregated. The surviving bars, running state, and trade classifications are the ones
    /// a reconstruction from the retained tape and its rebuild seed produces, so nothing is
    /// reconstructed: the work is proportional to the evicted trades plus the retained bar count.
    /// Returns how many leading trades left the tape, or `None` when retention cleared the stream.
    pub(crate) fn retain_last_bars(&mut self, keep: usize) -> Option<usize> {
        if self.bars.len() <= keep {
            return Some(0);
        }
        if keep == 0 {
            self.trades.clear();
            self.trade_ids.clear();
            self.trade_id_base = 0;
            self.bars.clear();
            self.last_trade_price = None;
            self.last_classified_side = AggressorSide::Unknown;
            self.active_session = None;
            self.session_delta = 0.0;
            self.rebuild_seed = RebuildSeed::default();
            self.replay_checkpoints.clear();
            self.tail_bar_key = None;
            self.revision = self.revision.saturating_add(1);
            return None;
        }
        let first = self.bars.len() - keep;
        // Bars consume the visible tape in canonical order, so the evicted bars own exactly their
        // counted leading trades. A timestamp cutoff would misassign a trade sharing its
        // microsecond with the first retained bar's open, which trade, volume, and range bars
        // allow, and the retained bars would then differ from the unretained chart's.
        //
        // Counting also subsumes a session bar-key cutoff: a session-anchored bar counts the
        // prints it folds in, including an opening-auction print stamped before its open, so they
        // leave with that bar. Only prints the session policy excludes join no bar and are not
        // counted; the walk steps over them, evicting those stamped before the first retained
        // bar's first print, and they reach the rebuild seed like every evicted print.
        let owned = self.bars[..first]
            .iter()
            .map(|bar| bar.trade_count as usize)
            .sum::<usize>();
        let visible = self.visible_trade_count();
        let mut seed = self.rebuild_seed;
        let mut counted = 0;
        let mut trade_start = 0;
        while trade_start < visible {
            let stored = &self.trades[trade_start];
            let trade = &stored.event;
            let excluded = self.excluded_from_bars(trade.timestamp_micros);
            if !excluded {
                if counted == owned {
                    break;
                }
                counted += 1;
            }
            if let Some(trade_id) = trade.trade_id {
                if self.trade_ids.get(&trade_id) == Some(&(self.trade_id_base + trade_start)) {
                    self.trade_ids.remove(&trade_id);
                }
            }
            trade_start += 1;
            let side = stored.classified_side;
            seed.last_trade_price = Some(trade.price);
            if side != AggressorSide::Unknown {
                seed.last_classified_side = side;
            }
            if excluded {
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
        for bar in &self.bars[..first] {
            seed.cumulative_delta += bar.delta;
        }
        self.rebuild_seed = seed;
        self.trades.drain(..trade_start);
        self.trade_id_base += trade_start;
        self.bars.drain(..first);
        // A reconstruction numbers the retained bars from zero.
        for (index, bar) in self.bars.iter_mut().enumerate() {
            bar.logical_index = index as u64;
        }
        // Checkpoints inside the retained suffix stay exact snapshots of the state after their
        // trade count; re-address them so later backward replay seeks still start from bounded
        // checkpoints. Checkpoints inside the evicted prefix leave with it.
        self.replay_checkpoints.retain_mut(|checkpoint| {
            if checkpoint.trade_count <= trade_start || checkpoint.bars_len <= first {
                return false;
            }
            checkpoint.trade_count -= trade_start;
            checkpoint.bars_len -= first;
            if let Some(active) = checkpoint.active_bar.as_mut() {
                active.logical_index = checkpoint.bars_len as u64 - 1;
            }
            true
        });
        self.revision = self.revision.saturating_add(1);
        Some(trade_start)
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
        validate_trade_batch(self.options, &input)?;
        if input.is_empty() {
            return Ok(FootprintUpdateKind::Tip);
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
            return Ok(FootprintUpdateKind::Tip);
        }

        let mut next_input_order = self.next_input_order;
        for event in input {
            if let Some(position) = event
                .trade_id
                .and_then(|trade_id| self.trade_ids.get(&trade_id))
                .map(|&position| position - self.trade_id_base)
            {
                let input_order = self.trades[position].input_order;
                self.trades[position] = StoredTrade {
                    event,
                    input_order,
                    classified_side: AggressorSide::Unknown,
                };
            } else {
                self.trades.push_back(StoredTrade {
                    event,
                    input_order: next_input_order,
                    classified_side: AggressorSide::Unknown,
                });
                next_input_order = next_input_order.saturating_add(1);
            }
        }
        self.trades.make_contiguous().sort_by_key(trade_order_key);
        self.next_input_order = next_input_order;
        self.reindex_trade_ids();
        self.rebuild();
        self.refresh_tail_bar_key();
        self.revision = self.revision.saturating_add(1);
        Ok(FootprintUpdateKind::Historical)
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
                    .and_then(|trade_id| self.trade_ids.get(&trade_id))
                    .is_some_and(|&position| {
                        self.trades[position - self.trade_id_base]
                            .event
                            .timestamp_micros
                            <= clock
                    })
        })
    }

    fn historical_update_candidate(&self) -> Self {
        Self {
            options: self.options,
            trades: self.trades.clone(),
            trade_ids: self.trade_ids.clone(),
            trade_id_base: self.trade_id_base,
            bars: Vec::with_capacity(self.bars.len()),
            next_input_order: self.next_input_order,
            rebuild_seed: self.rebuild_seed,
            last_trade_price: self.last_trade_price,
            last_classified_side: self.last_classified_side,
            active_session: self.active_session,
            session_delta: self.session_delta,
            work: self.work,
            revision: self.revision,
            replay_clock_micros: self.replay_clock_micros,
            replay_checkpoints: self.replay_checkpoints.clone(),
            session_grid: self.session_grid.clone(),
            tail_bar_key: self.tail_bar_key,
            dependent_work: self.dependent_work,
        }
    }

    /// This stream re-aggregated under `options`: the same retained tape (prints the replay clock
    /// hides included), replay clock, retention seed, and session anchoring, re-placed on the new
    /// bar interval. Sessions need whole-second time bars, as [`Self::set_sessions`] does.
    fn with_options(&self, options: FootprintAggregationOptions) -> Result<Self, FootprintError> {
        validate_options(options)?;
        let mut next = self.historical_update_candidate();
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

    pub fn bar(&self, index: usize) -> Option<&FootprintBar> {
        self.bars.get(index)
    }

    fn rebuild(&mut self) {
        self.bars.clear();
        self.replay_checkpoints.clear();
        self.last_trade_price = self.rebuild_seed.last_trade_price;
        self.last_classified_side = self.rebuild_seed.last_classified_side;
        self.active_session = self.rebuild_seed.active_session;
        self.session_delta = self.rebuild_seed.session_delta;
        let trade_count = self.visible_trade_count();
        for index in 0..trade_count {
            // The event has no heap-owned fields. Copying one value avoids retaining a second tape
            // while mutable derived state is rebuilt.
            let trade = self.trades[index].event.clone();
            let side = classify_aggressor(&trade, self.last_trade_price, self.last_classified_side);
            self.trades[index].classified_side = side;
            self.apply_classified_trade(&trade, side);
            self.maybe_record_replay_checkpoint(index + 1);
        }
        self.work.historical_rebuilds += 1;
        self.work.rebuilt_ticks += trade_count;
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
            self.bars.truncate(checkpoint.bars_len);
            if let (Some(active), Some(current)) = (checkpoint.active_bar, self.bars.last_mut()) {
                *current = active;
            }
            self.last_trade_price = checkpoint.last_trade_price;
            self.last_classified_side = checkpoint.last_classified_side;
            self.active_session = checkpoint.active_session;
            self.session_delta = checkpoint.session_delta;
            checkpoint.trade_count
        } else {
            self.bars.clear();
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
        let level = self.options.row_level(trade.price);
        let start_new = self
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
                        price: level as f64 * self.options.row_size(),
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
            self.refresh_trade_dependents(stream_id)?;
            self.refresh_footprint_series_from_stream(stream_id, None)?;
        }
        Ok(stats)
    }

    pub fn trade_stream_stats(&self, stream_id: u64) -> Option<TradeStreamStats> {
        let stream = self.trade_stream(stream_id)?;
        let dependents = self.trade_dependents.get(&stream_id);
        let bar_dependents = self.trade_bar_dependents.get(&stream_id);
        let bubbles = self.trade_bubbles.get(&stream_id);
        Some(TradeStreamStats {
            revision: stream.revision(),
            stream_capacity_bytes: stream.capacity_bytes(),
            dependent_count: dependents.map_or(0, Vec::len)
                + bar_dependents.map_or(0, Vec::len)
                + bubbles.map_or(0, Vec::len),
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
            bubble_trades_scanned: stream.dependent_work.bubble_trades_scanned,
            bubble_markers_sized: stream.dependent_work.bubble_markers_sized,
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
        if self.series.iter().any(|series| {
            series
                .footprint
                .as_ref()
                .is_some_and(|state| state.trade_stream_id == stream_id && !series.removed)
        }) || self
            .trade_bar_dependents
            .get(&stream_id)
            .is_some_and(|dependents| !dependents.is_empty())
            || self
                .trade_dependents
                .get(&stream_id)
                .is_some_and(|dependents| !dependents.is_empty())
            || self
                .trade_bubbles
                .get(&stream_id)
                .is_some_and(|dependents| !dependents.is_empty())
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
        self.set_series_pane(id, pane_index, 1.0);
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
        self.set_series_pane(id, pane_index, 1.0);
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
    /// windows.
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
        self.trade_streams.insert(stream_id, next);
        self.refresh_trade_dependents(stream_id)?;
        self.refresh_footprint_series_from_stream(stream_id, None)
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

    pub fn add_trade_bubbles(
        &mut self,
        stream_id: u64,
        series_id: SeriesId,
        options: TradeBubbleOptions,
    ) -> Result<(), FootprintError> {
        self.trade_stream(stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
        self.validate_series_id(series_id).map_err(series_error)?;
        if !options.minimum_volume.is_finite()
            || options.minimum_volume < 0.0
            || options.max_markers == 0
            || options.max_markers > 4_096
            || options.aggregation_window_micros < 0
        {
            return Err(FootprintError::InvalidAggregation);
        }
        self.trade_bubbles
            .entry(stream_id)
            .or_default()
            .retain(|dependent| dependent.series_id != series_id);
        self.trade_bubbles
            .entry(stream_id)
            .or_default()
            .push(TradeBubbleDependent {
                series_id,
                options,
                applied_revision: 0,
                fold: None,
            });
        self.refresh_trade_bubbles(stream_id, false)
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
        if self
            .trade_stream(stream_id)
            .ok_or(FootprintError::UnknownSeries(id))?
            .options()
            == options.aggregation
        {
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
            + self.trade_bubbles.get(&stream_id).map_or(0, Vec::len);
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
        self.trade_streams.insert(stream_id, aggregator);
        self.reset_trade_dependent_folds(stream_id);
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

    pub fn footprint_bars(&self, id: SeriesId) -> Option<Vec<FootprintBar>> {
        let stream_id = self.series_entry(id)?.footprint.as_ref()?.trade_stream_id;
        Some(self.trade_stream(stream_id)?.bars().to_vec())
    }

    pub fn footprint_bar(&self, id: SeriesId, bar_index: usize) -> Option<FootprintBar> {
        let stream_id = self.series_entry(id)?.footprint.as_ref()?.trade_stream_id;
        self.trade_stream(stream_id)?.bar(bar_index).cloned()
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
            let mut next = stream.historical_update_candidate();
            let result = next.update_trades(trades)?;
            debug_assert_eq!(result, FootprintUpdateKind::Historical);
            if next.tape_bar_times_collide() {
                return Err(FootprintError::ProjectionTimeCollision);
            }
            self.trade_streams.insert(stream_id, next);
            if !batch_changes_visible_state {
                return Ok(FootprintUpdateKind::Historical);
            }
            self.refresh_trade_dependents(stream_id)?;
            self.refresh_footprint_series_from_stream(stream_id, None)?;
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
        // Closed bars are immutable on the tip path: only the previously active bar and the bars
        // this batch opened change. The footprint projection advances first because its
        // retention ceiling may evict bars from the stream front; every other dependent then
        // continues from the same stream bar on the evicted-adjusted index.
        let from = previous_bar_count.saturating_sub(1);
        let bars_before = self
            .trade_stream(stream_id)
            .map_or(0, |stream| stream.bars().len());
        self.refresh_footprint_series_from_stream(stream_id, Some(from))?;
        let evicted = bars_before.saturating_sub(
            self.trade_stream(stream_id)
                .map_or(0, |stream| stream.bars().len()),
        );
        self.refresh_trade_dependents_from(stream_id, from.checked_sub(evicted))?;
        Ok(result)
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
            + self
                .trade_bubbles
                .values()
                .flatten()
                .filter_map(|dependent| dependent.fold.as_ref())
                .map(BubbleFold::capacity_bytes)
                .sum::<usize>()
    }

    pub(crate) fn trim_footprint_rows_front(&mut self, id: SeriesId, keep: usize) {
        let Some(stream_id) = self
            .series_entry(id)
            .and_then(|series| series.footprint.as_ref())
            .map(|state| state.trade_stream_id)
        else {
            return;
        };
        // Anchored cumulative-delta studies keep a base the evicted bars established, so the
        // values of the bars that remain never change when older history leaves the chart.
        if let (Some(stream), Some(dependents)) = (
            self.trade_streams.get(&stream_id),
            self.trade_dependents.get_mut(&stream_id),
        ) {
            let evicted = &stream.bars()[..stream.bars().len().saturating_sub(keep)];
            for dependent in dependents {
                dependent.evict_front(evicted, stream.evicted_cumulative_delta(), keep == 0);
            }
        }
        let evicted_bars = self
            .trade_stream(stream_id)
            .map_or(0, |stream| stream.bars().len().saturating_sub(keep));
        let evicted_trades = self
            .trade_streams
            .get_mut(&stream_id)
            .and_then(|stream| stream.retain_last_bars(keep));
        // Every presentation leaves the data layer in one transaction, before the sidecar and
        // bubble eviction below read the first retained row key from it.
        let presentations = self.trim_stream_rows_front(id, stream_id, keep);
        let sequence_owner = self.trade_stream(stream_id).is_some_and(|stream| {
            !matches!(stream.options().bars, FootprintBarAggregation::Time { .. })
        });
        if sequence_owner {
            if let Some(points) = self.sequence_points.as_mut() {
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
        }
        // Bubbles drop the prints that left the tape and keep the rest, exactly as a refold of the
        // retained tape would; a fold that cannot evict in place refolds once.
        self.evict_trade_bubbles_front(stream_id, evicted_trades, evicted_bars);
        // The only failure is an unknown stream, and this one was just trimmed.
        let refreshed = self.refresh_trade_bubbles(stream_id, true);
        debug_assert!(refreshed.is_ok(), "retention trims a live stream");
        // A trimmed presentation lost rows outside its own write path, so the indicators and
        // resampled series reading it recompute from the retained rows, as a retention trim of
        // that series itself does. Otherwise a resampled tail refresh would keep bars aggregated
        // from evicted rows, and a full load, which installs every presentation before the
        // footprint trims them, would keep indicators computed over the evicted history. The rows
        // and the sequence sidecar are final here, so the time sync this runs sees exactly the
        // state the caller's own sync does.
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

    /// Re-address every bubble fold of a stream after retention evicted `trades` leading trades
    /// (`None`: the stream was cleared) and `bars` leading bars.
    fn evict_trade_bubbles_front(&mut self, stream_id: u64, trades: Option<usize>, bars: usize) {
        let key_base = self.sequence_key_base(stream_id);
        let count = self.trade_bubbles.get(&stream_id).map_or(0, Vec::len);
        for index in 0..count {
            let Some(dependent) = self
                .trade_bubbles
                .get_mut(&stream_id)
                .and_then(|dependents| dependents.get_mut(index))
            else {
                break;
            };
            let series_id = dependent.series_id;
            let Some(mut fold) = dependent.fold.take() else {
                continue;
            };
            let Some(series) = self.series_entry_mut(series_id) else {
                continue;
            };
            let evicted = trades.is_some_and(|trades| {
                fold.evict_front(&mut series.markers, trades, bars, key_base)
            });
            if evicted {
                self.invalidate_frame_series(series_id);
                if let Some(dependent) = self
                    .trade_bubbles
                    .get_mut(&stream_id)
                    .and_then(|dependents| dependents.get_mut(index))
                {
                    dependent.fold = Some(fold);
                }
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

    pub(crate) fn trade_stream(&self, stream_id: u64) -> Option<&FootprintAggregator> {
        self.trade_streams.get(&stream_id)
    }

    pub(crate) fn is_trade_bar_dependent(&self, series_id: SeriesId) -> bool {
        self.trade_bar_dependents
            .values()
            .flatten()
            .any(|dependent| dependent.series_id == series_id)
    }

    fn prune_trade_stream_if_unused(&mut self, stream_id: u64) {
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
            || self
                .trade_bubbles
                .get(&stream_id)
                .is_some_and(|dependents| !dependents.is_empty());
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
        self.refresh_trade_dependents_from(stream_id, None)
    }

    /// Refresh every study, trade-bound candle/bar, and bubble dependent of a stream.
    /// `Some(from)` is the live-tip path: stream bars before `from` are unchanged and the tape
    /// only grew at its tip, so each dependent works on `bars[from..]` and the new trades only.
    /// `None` rebuilds every dependent from the stream.
    fn refresh_trade_dependents_from(
        &mut self,
        stream_id: u64,
        incremental_from: Option<usize>,
    ) -> Result<(), FootprintError> {
        self.trade_stream(stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
        let studies = self.trade_dependents.get(&stream_id).map_or(0, Vec::len);
        for index in 0..studies {
            self.refresh_trade_study(stream_id, index, incremental_from)?;
        }
        self.refresh_trade_bar_dependents_from(stream_id, incremental_from)?;
        self.refresh_trade_bubbles(stream_id, incremental_from.is_some())
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
            {
                if stored.applied_revision != revision {
                    stored.applied_revision = revision;
                    if updated {
                        stored.incremental_updates = stored.incremental_updates.saturating_add(1);
                    } else {
                        stored.rebuilds = stored.rebuilds.saturating_add(1);
                    }
                }
            }
        }
        Ok(())
    }

    /// Refold or advance large-trade bubble markers. `incremental` resumes each dependent's fold
    /// over only the trades appended since its last refresh (live tips, and retention after
    /// `evict_trade_bubbles_front`); a dependent without a resumable fold refolds. Tape
    /// replacement, corrections, and replay seeks refold the visible tape once. A stream without
    /// bubbles does no work.
    pub(crate) fn refresh_trade_bubbles(
        &mut self,
        stream_id: u64,
        incremental: bool,
    ) -> Result<(), FootprintError> {
        let count = self.trade_bubbles.get(&stream_id).map_or(0, Vec::len);
        if count == 0 {
            return Ok(());
        }
        self.trade_stream(stream_id)
            .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
        let key_base = self.sequence_key_base(stream_id);
        for index in 0..count {
            let Some((series_id, options, fold)) = self
                .trade_bubbles
                .get_mut(&stream_id)
                .and_then(|dependents| dependents.get_mut(index))
                .map(|dependent| {
                    (
                        dependent.series_id,
                        dependent.options,
                        dependent.fold.take(),
                    )
                })
            else {
                break;
            };
            let Some(series) = self.series_entry_mut(series_id) else {
                continue;
            };
            let mut markers = std::mem::take(&mut series.markers);
            let stream = self
                .trade_streams
                .get(&stream_id)
                .expect("validated trade stream");
            let resumed =
                fold.filter(|fold| incremental && fold.resumes(stream, key_base, &markers));
            let resumed_fold = resumed.is_some();
            let mut fold = resumed.unwrap_or_else(|| {
                markers.clear();
                BubbleFold::new(key_base)
            });
            let work = fold.advance(&mut markers, stream, options);
            let revision = stream.revision();
            if let Some(series) = self.series_entry_mut(series_id) {
                series.markers = markers;
            }
            if !resumed_fold || work.trades_scanned > 0 {
                self.invalidate_frame_series(series_id);
            }
            if let Some(stored) = self
                .trade_bubbles
                .get_mut(&stream_id)
                .and_then(|dependents| dependents.get_mut(index))
            {
                stored.fold = Some(fold);
                stored.applied_revision = revision;
            }
            self.record_dependent_work(stream_id, |recorded| {
                recorded.bubble_trades_scanned += work.trades_scanned;
                recorded.bubble_markers_sized += work.markers_sized;
            });
            // The markers now belong to this fold; any other fold writing the same series must
            // refold before it resumes.
            self.invalidate_trade_bubble_folds(series_id, Some((stream_id, index)));
        }
        Ok(())
    }

    /// Forget every bubble fold that writes `series_id` (except `keep`): the series markers
    /// changed outside that fold, so its next refresh must refold instead of resuming.
    pub(crate) fn invalidate_trade_bubble_folds(
        &mut self,
        series_id: SeriesId,
        keep: Option<(u64, usize)>,
    ) {
        for (&stream_id, dependents) in &mut self.trade_bubbles {
            for (index, dependent) in dependents.iter_mut().enumerate() {
                if dependent.series_id == series_id && keep != Some((stream_id, index)) {
                    dependent.fold = None;
                }
            }
        }
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
        for dependent in self.trade_bubbles.get_mut(&stream_id).into_iter().flatten() {
            dependent.fold = None;
        }
    }

    fn record_dependent_work(
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
    fn stream_presentations(&self, stream_id: u64) -> impl Iterator<Item = SeriesId> + '_ {
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
    /// sequence-axis writer (projection, candles, studies, bubbles) continues from the first key
    /// the stream's presentations hold, including a presentation bound after a trim.
    fn sequence_key_base(&self, stream_id: u64) -> Option<i64> {
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

/// Smallest and largest bubble diameter as a multiple of the marker envelope.
const TRADE_BUBBLE_MIN_SIZE: f64 = 0.5;
const TRADE_BUBBLE_MAX_SIZE: f64 = 2.5;

/// Bubble area scales with volume relative to the largest retained bubble, so size compares
/// prints instead of saturating.
fn trade_bubble_size(volume: f64, peak_volume: f64) -> f64 {
    let ratio = if peak_volume > 0.0 {
        (volume / peak_volume).clamp(0.0, 1.0)
    } else {
        0.0
    };
    TRADE_BUBBLE_MIN_SIZE + (TRADE_BUBBLE_MAX_SIZE - TRADE_BUBBLE_MIN_SIZE) * ratio.sqrt()
}

/// One large print, or consecutive same-side prints merged at one price, in a bubble fold.
#[derive(Clone, Debug)]
struct TradeBubble {
    sequence: u64,
    /// Marker time: the open of the bar holding the first print on a time axis, else that bar's
    /// row key.
    time: i64,
    /// Second that names an id-less marker: the first print's own UTC second on a time axis
    /// (distinct prints folded into one session bar keep distinct names), else its bar row key.
    identity: i64,
    price: f64,
    volume: f64,
    aggressor: AggressorSide,
    last_timestamp_micros: i64,
    /// Provider id of the bubble's first print; it names the marker.
    trade_id: Option<u64>,
    /// Tape positions of the bubble's first and last print, re-addressed on retention.
    first_trade: usize,
    last_trade: usize,
}

impl TradeBubble {
    /// A translucent circle centred on the traded price and colored by the host aggressor side.
    /// The fold sizes it once the peak retained volume is known.
    fn marker(&self) -> Marker {
        let color = match self.aggressor {
            AggressorSide::Buy => Color::rgba(76, 175, 80, 150),
            AggressorSide::Sell => Color::rgba(239, 83, 80, 150),
            AggressorSide::Unknown => Color::rgba(158, 158, 158, 150),
        };
        let identity = self.identity;
        Marker {
            time: self.time,
            position: marker_pos::AT_PRICE_MIDDLE,
            shape: marker_shape::CIRCLE,
            color,
            text: String::new(),
            id: self
                .trade_id
                .map_or_else(|| format!("trade-{identity}"), |id| format!("trade-{id}")),
            size: 0.0,
            price: Some(self.price),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct BubbleWork {
    trades_scanned: u64,
    markers_sized: u64,
}

/// Resumable large-trade bubble fold over the visible canonical tape. A clean rebuild and a live
/// tip run the same steps, so the retained bubbles, marker ids, and sizes are identical. The
/// series markers mirror `bubbles` one to one, oldest first.
#[derive(Clone, Debug)]
struct BubbleFold {
    /// Visible tape trades already folded.
    trades: usize,
    /// Sequence-axis bar holding the next trade, and the trades before that bar.
    bar_cursor: usize,
    trades_before_cursor: usize,
    /// Row key of bar 0 on a non-time axis (bubbles anchor to bar rows); `None` on a time axis,
    /// where bubbles anchor to the open of the bar holding the print
    /// ([`FootprintAggregator::print_bar_time`]).
    key_base: Option<i64>,
    /// The newest `max_markers` bubbles.
    bubbles: VecDeque<TradeBubble>,
    /// Sliding-window maximum of the retained volumes: `(sequence, volume)` with non-increasing
    /// volumes, the newest bubble winning ties.
    peaks: VecDeque<(u64, f64)>,
    next_sequence: u64,
    /// Peak volume the current marker sizes were computed against (NaN before the first fold).
    sized_peak: f64,
}

impl BubbleFold {
    fn new(key_base: Option<i64>) -> Self {
        Self {
            trades: 0,
            bar_cursor: 0,
            trades_before_cursor: 0,
            key_base,
            bubbles: VecDeque::new(),
            peaks: VecDeque::new(),
            next_sequence: 0,
            sized_peak: f64::NAN,
        }
    }

    /// Whether this fold can continue over `stream`: the tape only grew since the last advance,
    /// the rows keep their key base, and the series still holds exactly this fold's markers.
    fn resumes(
        &self,
        stream: &FootprintAggregator,
        key_base: Option<i64>,
        markers: &[Marker],
    ) -> bool {
        self.key_base == key_base
            && self.trades <= stream.visible_trade_count()
            && markers.len() == self.bubbles.len()
    }

    /// Fold the visible trades after `self.trades` into `markers`. Only new or merged bubbles are
    /// sized, unless the peak retained volume changed, which rescales every retained marker.
    fn advance(
        &mut self,
        markers: &mut Vec<Marker>,
        stream: &FootprintAggregator,
        options: TradeBubbleOptions,
    ) -> BubbleWork {
        let bars = stream.bars();
        let visible = stream.visible_trade_count();
        let mut work = BubbleWork::default();
        // Bubbles this advance creates get a marker only if they are still retained at its end,
        // so a refold over a long tape never holds more than `max_markers` markers.
        let first_new = self.next_sequence;
        // Leading markers whose bubbles left the window during this advance.
        let mut evicted = 0;
        // Oldest bubble whose volume changed or that was created during this advance.
        let mut dirty = None;
        for (trade_index, stored) in
            (self.trades..visible).zip(stream.trades.range(self.trades..visible))
        {
            let trade = &stored.event;
            work.trades_scanned += 1;
            let bar_key = self.key_base.map(|key_base| {
                // Same bar assignment as a walk from bar 0: skip bars whose trades precede this
                // one, keeping any overflow on the last bar.
                while self.bar_cursor + 1 < bars.len()
                    && trade_index
                        >= self.trades_before_cursor + bars[self.bar_cursor].trade_count as usize
                {
                    self.trades_before_cursor += bars[self.bar_cursor].trade_count as usize;
                    self.bar_cursor += 1;
                }
                key_base + self.bar_cursor as i64
            });
            if trade.volume < options.minimum_volume {
                continue;
            }
            // A time-axis bubble sits on the bar holding its print: markers snap an off-grid time
            // to the next bar, and session anchoring folds auction and closing prints into bars
            // they are not stamped in. A print the session policy leaves out of every bar has no
            // bubble.
            let (time, identity) = match bar_key {
                Some(key) => (key, key),
                None => {
                    let Some(time) = stream.print_bar_time(trade.timestamp_micros) else {
                        continue;
                    };
                    (time, trade.timestamp_micros.div_euclid(MICROS_PER_SECOND))
                }
            };
            if options.aggregation_window_micros > 0 {
                if let Some(bubble) = self.bubbles.back_mut().filter(|bubble| {
                    bubble.time == time
                        && bubble.aggressor == trade.aggressor
                        && bubble.price.to_bits() == trade.price.to_bits()
                        && (trade.timestamp_micros - bubble.last_timestamp_micros).abs()
                            <= options.aggregation_window_micros
                }) {
                    bubble.volume += trade.volume;
                    bubble.last_timestamp_micros = trade.timestamp_micros;
                    bubble.last_trade = trade_index;
                    let (sequence, volume) = (bubble.sequence, bubble.volume);
                    // The newest bubble always ends the peak window; re-seat it at its new volume.
                    self.peaks.pop_back();
                    self.push_peak(sequence, volume);
                    dirty.get_or_insert(sequence);
                    continue;
                }
            }
            if self.bubbles.len() == options.max_markers {
                if let Some(oldest) = self.bubbles.pop_front() {
                    if self
                        .peaks
                        .front()
                        .is_some_and(|&(sequence, _)| sequence == oldest.sequence)
                    {
                        self.peaks.pop_front();
                    }
                    evicted += usize::from(oldest.sequence < first_new);
                }
            }
            let sequence = self.next_sequence;
            self.next_sequence += 1;
            self.bubbles.push_back(TradeBubble {
                sequence,
                time,
                identity,
                price: trade.price,
                volume: trade.volume,
                aggressor: trade.aggressor,
                last_timestamp_micros: trade.timestamp_micros,
                trade_id: trade.trade_id,
                first_trade: trade_index,
                last_trade: trade_index,
            });
            self.push_peak(sequence, trade.volume);
            dirty.get_or_insert(sequence);
        }
        self.trades = visible;
        // The markers mirror the bubbles: drop the evicted ones, then append the new survivors,
        // which follow every surviving older bubble.
        markers.drain(..evicted);
        let kept = markers.len();
        markers.extend(self.bubbles.iter().skip(kept).map(TradeBubble::marker));
        let peak = self.peaks.front().map_or(0.0, |&(_, volume)| volume);
        let first = if peak.to_bits() != self.sized_peak.to_bits() {
            self.sized_peak = peak;
            Some(0)
        } else {
            let front = self.bubbles.front().map_or(0, |bubble| bubble.sequence);
            dirty.map(|sequence: u64| sequence.saturating_sub(front) as usize)
        };
        if let Some(first) = first {
            for (marker, bubble) in markers.iter_mut().zip(&self.bubbles).skip(first) {
                marker.size = trade_bubble_size(bubble.volume, peak);
                work.markers_sized += 1;
            }
        }
        work
    }

    /// Retention evicted the tape's first `trades` trades and its first `bars` bars, and the rows
    /// now start at `key_base`. Drop the bubbles made only of evicted prints and re-address the
    /// rest, leaving exactly the state a refold of the retained tape reaches: bubbles are ordered
    /// by their prints, so the evicted ones form a prefix of the window, and every bubble older
    /// than the window is evicted too. Marker times are absolute (a bar-open UTC second, or a
    /// row key that retention does not change), so the survivors keep their markers; the next
    /// advance rescales them if the peak left. `false` when the fold cannot evict in place (it
    /// has not folded the evicted prints, the markers changed, the key base moved unexpectedly,
    /// or a merged bubble straddles the boundary) and must refold.
    fn evict_front(
        &mut self,
        markers: &mut Vec<Marker>,
        trades: usize,
        bars: usize,
        key_base: Option<i64>,
    ) -> bool {
        let rows_follow = match (self.key_base, key_base) {
            (None, None) => true,
            (Some(old), Some(new)) => {
                old + bars as i64 == new && (self.bar_cursor >= bars || self.trades == trades)
            }
            _ => false,
        };
        if !rows_follow || self.trades < trades || markers.len() != self.bubbles.len() {
            return false;
        }
        let evicted = self
            .bubbles
            .partition_point(|bubble| bubble.last_trade < trades);
        if self
            .bubbles
            .get(evicted)
            .is_some_and(|bubble| bubble.first_trade < trades)
        {
            return false;
        }
        self.bubbles.drain(..evicted);
        markers.drain(..evicted);
        let front = self
            .bubbles
            .front()
            .map_or(self.next_sequence, |bubble| bubble.sequence);
        while self
            .peaks
            .front()
            .is_some_and(|&(sequence, _)| sequence < front)
        {
            self.peaks.pop_front();
        }
        for bubble in &mut self.bubbles {
            bubble.first_trade -= trades;
            bubble.last_trade -= trades;
        }
        self.trades -= trades;
        if self.bar_cursor >= bars {
            self.bar_cursor -= bars;
            self.trades_before_cursor -= trades;
        } else {
            self.bar_cursor = 0;
            self.trades_before_cursor = 0;
        }
        self.key_base = key_base;
        true
    }

    /// Retained fold capacity; bounded by the dependent's `max_markers`.
    fn capacity_bytes(&self) -> usize {
        self.bubbles.capacity() * core::mem::size_of::<TradeBubble>()
            + self.peaks.capacity() * core::mem::size_of::<(u64, f64)>()
    }

    fn push_peak(&mut self, sequence: u64, volume: f64) {
        while self.peaks.back().is_some_and(|&(_, peak)| peak <= volume) {
            self.peaks.pop_back();
        }
        self.peaks.push_back((sequence, volume));
    }
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
    /// values keep their full-history base, as the stream seeds session and continuous delta;
    /// evicting every bar resets the stream, and the base with it.
    fn evict_front(&mut self, evicted: &[FootprintBar], cumulative: f64, clear: bool) {
        if evicted.is_empty() {
            return;
        }
        if clear {
            self.anchor_base = None;
            self.resume = None;
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
        if let Some(trade_id) = trade.trade_id {
            if ids.insert(trade_id, index).is_some() {
                return Err(FootprintError::DuplicateTradeId { trade_id });
            }
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
            }
        );

        let mut expected = FootprintAggregator::new(options).unwrap();
        expected.set_trades(history).unwrap();
        assert_eq!(aggregator.bars(), expected.bars());
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
    fn non_time_trade_bubbles_use_logical_bar_indices() {
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
        chart
            .add_trade_bubbles(stream_id, footprint, TradeBubbleOptions::default())
            .unwrap();
        assert_eq!(
            chart
                .series_entry(footprint)
                .unwrap()
                .markers
                .iter()
                .map(|marker| marker.time)
                .collect::<Vec<_>>(),
            vec![0, 0]
        );
        chart
            .update_footprint_trade(footprint, trade(1_000_003, 102.0, 1.0, AggressorSide::Buy))
            .unwrap();
        assert_eq!(
            chart
                .series_entry(footprint)
                .unwrap()
                .markers
                .iter()
                .map(|marker| marker.time)
                .collect::<Vec<_>>(),
            vec![0, 0, 1]
        );
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
        chart
            .add_trade_bubbles(stream, footprint, TradeBubbleOptions::default())
            .unwrap();
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
        assert_eq!(chart.series_entry(footprint).unwrap().markers.len(), 2);

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
        let backward = chart.set_replay_clock_micros(Some(1)).unwrap();
        assert_eq!(backward.visible_trades, 1);
        assert_eq!(chart.footprint_bars(footprint).unwrap().len(), 1);
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
        assert!(frame
            .panes
            .iter()
            .flat_map(|pane| &pane.main)
            .any(|primitive| matches!(primitive, Prim::Text { text, .. } if text == "Replay")));
        assert!(frame
            .panes
            .iter()
            .flat_map(|pane| &pane.main)
            .any(|primitive| matches!(
                primitive,
                Prim::VLine {
                    style: LineStyle::Dashed,
                    ..
                }
            )));

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
    fn cvd_and_delta_dependents_follow_late_corrections_and_report_rebuilds() {
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
        assert!(after.dependent_rebuilds > before.dependent_rebuilds);
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
    fn trade_bubbles_are_bounded_and_rebuilt_from_the_shared_tape() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let stream = chart
            .add_trade_stream("CME:RTY", FootprintAggregationOptions::default())
            .unwrap();
        let series = chart.add_series(SeriesKind::Footprint);
        chart
            .configure_footprint_series(series, FootprintSeriesOptions::default())
            .unwrap();
        chart
            .bind_footprint_series_to_stream(series, stream)
            .unwrap();
        chart
            .set_footprint_trades(
                series,
                vec![
                    trade(1_000_000, 100.0, 1.0, AggressorSide::Buy),
                    trade(1_100_000, 100.0, 3.0, AggressorSide::Buy),
                    trade(2_000_000, 101.0, 10.0, AggressorSide::Sell),
                ],
            )
            .unwrap();
        chart
            .add_trade_bubbles(
                stream,
                series,
                TradeBubbleOptions {
                    minimum_volume: 2.0,
                    max_markers: 2,
                    aggregation_window_micros: 200_000,
                },
            )
            .unwrap();
        let entry = chart.series_entry(series).unwrap();
        assert_eq!(entry.markers.len(), 2);
        let expected = TRADE_BUBBLE_MIN_SIZE
            + (TRADE_BUBBLE_MAX_SIZE - TRADE_BUBBLE_MIN_SIZE) * (3.0_f64 / 10.0).sqrt();
        assert!((entry.markers[0].size - expected).abs() < 1e-12);
        assert_eq!(entry.markers[1].size, TRADE_BUBBLE_MAX_SIZE);
        assert_eq!(chart.trade_stream_stats(stream).unwrap().dependent_count, 1);
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
        assert!(levels
            .iter()
            .any(|level| level.price == 105.0 && level.total_volume == 4.0));
        // Bar high/low stay exact trade prices; the row extent covers whole rows.
        assert_eq!((bars[0].low, bars[0].high), (100.0, 105.0));
        assert_eq!(
            footprint_row_price_bounds(&options, 100.0, 105.0),
            (99.5, 109.5)
        );
        // Trades are still validated against the instrument tick, not the row.
        assert!(chart
            .set_footprint_trades(
                series,
                vec![trade(1_400_000, 100.5, 1.0, AggressorSide::Buy)]
            )
            .is_err());
        assert!(validate_options(FootprintAggregationOptions {
            ticks_per_row: 0,
            ..options
        })
        .is_err());
    }

    #[test]
    fn trade_bubbles_are_price_centred_circles_that_keep_the_newest_prints() {
        let mut chart = ChartEngine::new(600.0, 400.0, 1.0);
        let stream = chart
            .add_trade_stream("CME:ES", FootprintAggregationOptions::default())
            .unwrap();
        let series = chart.add_series(SeriesKind::Footprint);
        chart
            .configure_footprint_series(series, FootprintSeriesOptions::default())
            .unwrap();
        chart
            .bind_footprint_series_to_stream(series, stream)
            .unwrap();
        chart
            .set_footprint_trades(
                series,
                vec![
                    trade(1_000_000, 100.0, 4.0, AggressorSide::Sell),
                    trade(2_000_000, 101.0, 2.0, AggressorSide::Buy),
                    // Merged into the previous buy: same side, price, bar and window.
                    trade(2_050_000, 101.0, 2.0, AggressorSide::Buy),
                    trade(3_000_000, 99.0, 1.0, AggressorSide::Sell),
                ],
            )
            .unwrap();
        chart
            .add_trade_bubbles(
                stream,
                series,
                TradeBubbleOptions {
                    minimum_volume: 0.0,
                    max_markers: 2,
                    aggregation_window_micros: 100_000,
                },
            )
            .unwrap();
        let markers = &chart.series_entry(series).unwrap().markers;
        assert_eq!(markers.len(), 2, "the oldest bubble is evicted first");
        assert_eq!(markers[0].price, Some(101.0));
        assert_eq!(markers[1].price, Some(99.0));
        for marker in markers {
            assert_eq!(marker.shape, marker_shape::CIRCLE);
            assert_eq!(marker.position, marker_pos::AT_PRICE_MIDDLE);
        }
        assert!(
            markers[0].color.g() > markers[0].color.r(),
            "buys are green"
        );
        assert!(markers[1].color.r() > markers[1].color.g(), "sells are red");
        assert_eq!(
            markers[0].size, TRADE_BUBBLE_MAX_SIZE,
            "merged volume sets the peak"
        );
        let quarter = TRADE_BUBBLE_MIN_SIZE + (TRADE_BUBBLE_MAX_SIZE - TRADE_BUBBLE_MIN_SIZE) * 0.5;
        assert!((markers[1].size - quarter).abs() < 1e-12);
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
        assert_eq!(chart.trade_stream_stats(stream).unwrap().revision, 3);
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
        assert!(prims.iter().any(|primitive| {
            matches!(primitive, Prim::Text { text, .. } if text.contains("Δ 70") && text.contains("H 70") && text.contains("L -10"))
        }));
        assert!(prims.iter().any(|primitive| {
            matches!(primitive, Prim::Text { text, .. } if text.contains("V 110") && text.contains("B 20") && text.contains("A 90"))
        }));
        assert!(prims.iter().any(|primitive| {
            matches!(primitive, Prim::Rect { color, .. } if *color == stacked_ask)
        }));
        // POC reads as a side stripe on its row, not a full outline.
        assert!(prims.iter().any(|primitive| {
            matches!(primitive, Prim::Rect { rect, color }
                if *color == poc.solid() && rect.w == 2 && rect.h > 1)
        }));
        // Imbalance glyphs are bold so the signal scans at a glance.
        assert!(prims
            .iter()
            .any(|primitive| { matches!(primitive, Prim::Text { weight, .. } if *weight == 700) }));

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
        assert!(!prims
            .iter()
            .any(|primitive| matches!(primitive, Prim::Text { .. })));
        assert!(prims.iter().any(|primitive| {
            matches!(primitive, Prim::Rect { rect, color }
                if *color == poc.solid() && rect.w == 2 && rect.h > 1)
        }));
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
        assert!(!prims
            .iter()
            .any(|primitive| matches!(primitive, Prim::Text { .. })));
        assert!(prims
            .iter()
            .any(|primitive| matches!(primitive, Prim::Rect { color, .. } if *color == poc)));
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
    fn footprint_numbers_grow_into_tall_rows() {
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
        // Three levels across a tall pane: rows are far taller than the
        // configured 9px, so numbers must grow instead of floating tiny.
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
        let largest = frame.panes[0].main[segment.start..segment.end]
            .iter()
            .filter_map(|primitive| match primitive {
                Prim::Text { size, .. } => Some(*size),
                _ => None,
            })
            .fold(0.0f32, f32::max);
        assert!(
            largest > 9.0,
            "tall rows must grow numbers past the configured 9px, got {largest}"
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
        // 9px type needs spacing >= 81; at 72 the summary would overprint neighbors.
        chart.set_bar_spacing(100.0);
        assert!(summaries(&mut chart) > 0);
        chart.set_bar_spacing(72.0);
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
                |primitive| matches!(primitive, Prim::Rect { color, .. } if *color == stacked_ask)
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
}
