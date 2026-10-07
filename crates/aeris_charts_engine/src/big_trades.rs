//! Engine-owned big-trades study: large aggressive orders drawn as volume bubbles over a price
//! series, derived from a chart trade stream.
//!
//! Exchanges report one aggressive order as several prints when it fills against several resting
//! orders or price levels. The study first rebuilds those orders (consecutive prints from the same
//! aggressor, each within the grouping window of the previous one, whose price never moves against
//! the aggressor), then filters the rebuilt orders, so an order filled as many small prints still
//! qualifies. The automatic filter keeps adapting as the tape grows: every completed order is
//! judged against a quantile of the recently completed orders.
//!
//! Orders belong to the bar they open in. A print the session policy leaves out of every bar
//! never starts or extends an order. Live tips fold only their new prints. Sealing a bar folds its
//! released prints into the state every raw-tape replay starts from, so bubbles in sealed history
//! survive; evicting sealed bars drops the orders that opened in them and keeps every other order
//! and the automatic filter's state, so retention never replays the raw tape or rewrites a
//! retained bubble.

use std::collections::VecDeque;

use aeris_charts_core::model::data_layer::SeriesId;
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{IRect, Prim, TextAlign};

use crate::footprint::{
    AggressorSide, FootprintAggregator, FootprintBar, FootprintError, FootprintTrade,
};
use crate::frame::{pane_scale, series_scale_target};
use crate::{ChartEngine, NativePrimitiveId, SeriesKind};

pub const MAX_BIG_TRADES_INDICATORS: usize = 16;
/// Newest qualifying orders kept per indicator; older bubbles are evicted first.
pub const MAX_BIG_TRADES_BUBBLES: usize = 4_096;
pub const MAX_BIG_TRADES_GROUPING_WINDOW_MICROS: i64 = 1_000_000;
/// Completed orders the automatic filter samples.
const AUTO_SAMPLE_ORDERS: usize = 4_096;
/// The automatic threshold is refreshed after this many completed orders, which also bounds the
/// warm-up before the first bubble can qualify.
const AUTO_REFRESH_ORDERS: u64 = 128;
const MAX_COLOR_BYTES: usize = 128;
const MICROS_PER_SECOND: i64 = 1_000_000;
const LABEL_PADDING: f64 = 4.0;
const LABEL_WEIGHT: u16 = 600;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BigTradesIntensity {
    /// More bubbles: orders above the recent 95th percentile.
    Weak,
    /// Orders above the recent 98th percentile.
    #[default]
    Medium,
    /// Fewest bubbles: orders above the recent 99.5th percentile.
    Strong,
}

impl BigTradesIntensity {
    fn quantile(self) -> f64 {
        match self {
            Self::Weak => 0.95,
            Self::Medium => 0.98,
            Self::Strong => 0.995,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum BigTradesFilter {
    /// Orders strictly larger than a quantile of the recently completed orders.
    Auto { intensity: BigTradesIntensity },
    /// Orders at least this large.
    Fixed { minimum_volume: f64 },
}

impl Default for BigTradesFilter {
    fn default() -> Self {
        Self::Auto {
            intensity: BigTradesIntensity::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BigTradesSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl BigTradesSize {
    /// Smallest and largest bubble diameter in CSS px.
    fn diameter_range(self) -> (f64, f64) {
        match self {
            Self::Small => (6.0, 24.0),
            Self::Medium => (8.0, 36.0),
            Self::Large => (10.0, 52.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BigTradesOptions {
    pub filter: BigTradesFilter,
    /// Largest gap between consecutive prints of one rebuilt order. Zero groups only prints that
    /// share an exact timestamp.
    pub grouping_window_micros: i64,
    pub size: BigTradesSize,
    /// Draw the compact order volume inside every bubble large enough to hold it.
    pub show_volume: bool,
    pub visible: bool,
    pub buy_color: String,
    pub sell_color: String,
    pub buy_border_color: String,
    pub sell_border_color: String,
    /// `None` follows the chart layout text color and retokenizes on theme changes.
    pub text_color: Option<String>,
}

impl Default for BigTradesOptions {
    fn default() -> Self {
        Self {
            filter: BigTradesFilter::default(),
            grouping_window_micros: 1_000,
            size: BigTradesSize::default(),
            show_volume: true,
            visible: true,
            buy_color: "rgba(8,153,129,0.35)".into(),
            sell_color: "rgba(247,82,95,0.35)".into(),
            buy_border_color: "#089981".into(),
            sell_border_color: "#f7525f".into(),
            text_color: None,
        }
    }
}

impl BigTradesOptions {
    pub(crate) fn valid(&self) -> bool {
        let valid_color =
            |color: &String| color.len() <= MAX_COLOR_BYTES && Color::parse_css(color).is_some();
        let valid_filter = match self.filter {
            BigTradesFilter::Auto { .. } => true,
            BigTradesFilter::Fixed { minimum_volume } => {
                minimum_volume.is_finite() && minimum_volume > 0.0
            }
        };
        valid_filter
            && (0..=MAX_BIG_TRADES_GROUPING_WINDOW_MICROS).contains(&self.grouping_window_micros)
            && [
                &self.buy_color,
                &self.sell_color,
                &self.buy_border_color,
                &self.sell_border_color,
            ]
            .into_iter()
            .all(valid_color)
            && self.text_color.as_ref().is_none_or(valid_color)
    }

    fn rebuild_required(&self, next: &Self) -> bool {
        self.filter != next.filter || self.grouping_window_micros != next.grouping_window_micros
    }
}

/// One rebuilt aggressive order.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct BigTrade {
    pub side: AggressorSide,
    pub volume: f64,
    pub prints: u32,
    /// Volume-weighted average fill price; the bubble is centred here.
    pub vwap: f64,
    pub low: f64,
    pub high: f64,
    pub start_timestamp_micros: i64,
    pub end_timestamp_micros: i64,
    /// Chart time key of the bar the order opened in: the bar's UTC-second open for time bars,
    /// the bar's logical row key for trade, volume, and range bars. Retention never re-keys it,
    /// except on a stream without presentations, whose orders carry stream bar positions.
    pub bar_time: i64,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct BigTradesSnapshot {
    /// The filter currently applied. `None` while the automatic filter is still sampling.
    pub threshold: Option<f64>,
    /// Qualifying orders oldest first, ending with the still-forming order when it qualifies.
    pub bubbles: Vec<BigTrade>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PrintKey {
    timestamp_micros: i64,
    price_bits: u64,
    volume_bits: u64,
    trade_id: Option<u64>,
}

impl PrintKey {
    fn of(trade: &FootprintTrade) -> Self {
        Self {
            timestamp_micros: trade.timestamp_micros,
            price_bits: trade.price.to_bits(),
            volume_bits: trade.volume.to_bits(),
            trade_id: trade.trade_id,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct OpenOrder {
    side: AggressorSide,
    session_id: Option<u64>,
    volume: f64,
    notional: f64,
    prints: u32,
    low: f64,
    high: f64,
    last_price: f64,
    start_timestamp_micros: i64,
    last_timestamp_micros: i64,
    bar_time: i64,
}

impl OpenOrder {
    fn new(trade: &FootprintTrade, side: AggressorSide, bar_time: i64) -> Self {
        Self {
            side,
            session_id: trade.session_id,
            volume: trade.volume,
            notional: trade.price * trade.volume,
            prints: 1,
            low: trade.price,
            high: trade.price,
            last_price: trade.price,
            start_timestamp_micros: trade.timestamp_micros,
            last_timestamp_micros: trade.timestamp_micros,
            bar_time,
        }
    }

    fn continues_with(&self, trade: &FootprintTrade, side: AggressorSide, window: i64) -> bool {
        let gap = trade
            .timestamp_micros
            .saturating_sub(self.last_timestamp_micros);
        let with_the_aggressor = match side {
            AggressorSide::Buy => trade.price >= self.last_price,
            AggressorSide::Sell => trade.price <= self.last_price,
            AggressorSide::Unknown => false,
        };
        side == self.side
            && trade.session_id == self.session_id
            && (0..=window).contains(&gap)
            && with_the_aggressor
    }

    fn add(&mut self, trade: &FootprintTrade) {
        self.volume += trade.volume;
        self.notional += trade.price * trade.volume;
        self.prints = self.prints.saturating_add(1);
        self.low = self.low.min(trade.price);
        self.high = self.high.max(trade.price);
        self.last_price = trade.price;
        self.last_timestamp_micros = trade.timestamp_micros;
    }

    fn order(&self) -> BigTrade {
        BigTrade {
            side: self.side,
            volume: self.volume,
            prints: self.prints,
            vwap: (self.notional / self.volume).clamp(self.low, self.high),
            low: self.low,
            high: self.high,
            start_timestamp_micros: self.start_timestamp_micros,
            end_timestamp_micros: self.last_timestamp_micros,
            bar_time: self.bar_time,
        }
    }
}

/// Work one builder pass did, recorded in the stream's [`crate::TradeStreamStats`].
#[derive(Clone, Copy, Debug, Default)]
struct BuilderWork {
    prints_scanned: u64,
    replays: u64,
}

impl BuilderWork {
    fn add(&mut self, other: Self) {
        self.prints_scanned += other.prints_scanned;
        self.replays += other.replays;
    }
}

/// Order reconstruction after some prefix of the tape.
#[derive(Clone, Debug, Default)]
struct OrderState {
    open: Option<OpenOrder>,
    /// Key of the oldest bar retention kept: an order opened before it opened in an evicted bar
    /// and is never shown, though it still completes and feeds the automatic filter exactly as
    /// it would had nothing been evicted.
    bar_floor: i64,
    recent_volumes: VecDeque<f64>,
    completed_orders: u64,
    threshold: Option<f64>,
    bubbles: VecDeque<BigTrade>,
}

/// Incremental order rebuild over the stream's raw tape. A tip append continues from the
/// still-open order; any other tape change replays the raw tape from the state at its start, so
/// the result is identical to a rebuild from scratch. Trades the stream is about to release by
/// sealing bars are first folded into that start state, so bubbles in sealed history survive.
/// Evicted, prepended, and re-keyed bars are followed in place in both states.
#[derive(Clone, Debug)]
struct OrderBuilder {
    state: OrderState,
    /// State after exactly the released trades: where every raw-tape replay starts.
    sealed: OrderState,
    consumed: usize,
    last_print: Option<PrintKey>,
    /// Row key of stream bar 0 on a non-time sequence axis, where orders carry their bar's row
    /// key; `None` on a time axis, where they carry their bar's open.
    key_base: Option<i64>,
    tape_epoch: u64,
    released_trades: u64,
    bar_origin: i64,
}

impl OrderBuilder {
    fn new(
        options: &BigTradesOptions,
        stream: &FootprintAggregator,
        key_base: Option<i64>,
    ) -> Self {
        let state = OrderState::new(options);
        Self {
            sealed: state.clone(),
            state,
            consumed: 0,
            last_print: None,
            key_base,
            tape_epoch: stream.tape_epoch(),
            released_trades: stream.released_trades(),
            bar_origin: stream.bar_origin(),
        }
    }

    /// Fold the visible raw prints after `consumed`, or replay the raw tape from the sealed
    /// state when the tape changed other than by a tip append (`tip` false) or was not the one
    /// this builder folded.
    fn refresh(
        &mut self,
        options: &BigTradesOptions,
        stream: &FootprintAggregator,
        tip: bool,
        key_base: Option<i64>,
    ) -> BuilderWork {
        self.follow(options, stream, key_base);
        let count = stream.trades().len();
        let continues = tip
            && self.consumed <= count
            && (self.consumed == 0
                || stream.trade_at(self.consumed - 1).map(PrintKey::of) == self.last_print);
        let mut work = BuilderWork::default();
        if !continues {
            self.state = self.sealed.clone();
            self.consumed = 0;
            work.replays = u64::from(count > 0);
        }
        let from = self.consumed;
        if from >= count {
            return work;
        }
        // Sequence-axis orders key their bar by position; the raw bars partition the visible raw
        // tape in order, so the suffix resolves by walking back from the newest bar.
        let positions = self
            .key_base
            .map(|_| bar_positions_from(stream.bars(), count, from));
        for (offset, (trade, side)) in stream.classified_trades_from(from).enumerate() {
            let bar_time = match (self.key_base, &positions) {
                (Some(key_base), Some(positions)) => Some(key_base + positions[offset] as i64),
                _ => stream.print_bar_time(trade.timestamp_micros),
            };
            // A print the session policy leaves out of every bar never starts or extends an order.
            if let Some(bar_time) = bar_time {
                self.state.push_print(options, trade, side, bar_time);
            }
        }
        work.prints_scanned = (count - from) as u64;
        self.consumed = count;
        self.last_print = stream.trade_at(count - 1).map(PrintKey::of);
        work
    }

    /// Track a replaced tape, released trades, and bars that were evicted, prepended, or
    /// re-keyed. Orders carry `key_base + position` on a sequence axis, so a bar renumbering by
    /// `shift` with the key base moving from `old` to `new` re-keys them by `old + shift - new`:
    /// nothing while presentations keep their row keys across retention, the evicted count on a
    /// stream without presentations, whose keys are positions.
    fn follow(
        &mut self,
        options: &BigTradesOptions,
        stream: &FootprintAggregator,
        key_base: Option<i64>,
    ) {
        if stream.tape_epoch() != self.tape_epoch
            || stream.released_trades() < self.released_trades
            || self.key_base.is_some() != key_base.is_some()
        {
            *self = Self::new(options, stream, key_base);
            return;
        }
        if stream.released_trades() != self.released_trades {
            // Engine seals absorb the released trades first. Should one ever not, the orders of
            // those trades are lost and the replay restarts at the new tape start.
            self.released_trades = stream.released_trades();
            self.consumed = 0;
        }
        let shift = stream.bar_origin() - self.bar_origin;
        let rekey = match (self.key_base, key_base) {
            (Some(old), Some(new)) => old + shift - new,
            _ => 0,
        };
        if shift == 0 && rekey == 0 {
            return;
        }
        // Evicted bars leave a floor below the first retained bar key; with no bar left, every
        // order known so far opened in an evicted bar.
        let floor = (shift > 0).then(|| match key_base {
            Some(key_base) => key_base,
            None => stream.bars().first().map_or_else(
                || {
                    self.state
                        .newest_bar_time()
                        .max(self.sealed.newest_bar_time())
                        + 1
                },
                |bar| bar.start_timestamp_micros.div_euclid(MICROS_PER_SECOND),
            ),
        });
        for state in [&mut self.state, &mut self.sealed] {
            state.rebase(rekey, floor);
        }
        self.key_base = key_base;
        self.bar_origin = stream.bar_origin();
    }

    /// Fold the oldest `count` raw trades, about to be released by sealing, into the replay
    /// start state. The bookkeeping is proportional to the released trades and is not counted
    /// as fold work: no shown order changes.
    fn absorb_sealed(
        &mut self,
        options: &BigTradesOptions,
        stream: &FootprintAggregator,
        count: usize,
        key_base: Option<i64>,
    ) -> BuilderWork {
        let work = self.refresh(options, stream, true, key_base);
        let bars = stream.bars();
        let mut bar = stream.sealed_bar_count();
        let mut left = bars.get(bar).map_or(0, |bar| bar.trade_count as usize);
        for (trade, side) in stream.classified_trades_from(0).take(count) {
            let bar_time = match self.key_base {
                Some(key_base) => {
                    while left == 0 && bar + 1 < bars.len() {
                        bar += 1;
                        left = bars[bar].trade_count as usize;
                    }
                    left = left.saturating_sub(1);
                    Some(key_base + bar as i64)
                }
                None => stream.print_bar_time(trade.timestamp_micros),
            };
            if let Some(bar_time) = bar_time {
                self.sealed.push_print(options, trade, side, bar_time);
            }
        }
        self.consumed = self.consumed.saturating_sub(count);
        self.released_trades = self.released_trades.saturating_add(count as u64);
        work
    }

    /// Orders of an older page the stream just joined in front of its sealed history. The page
    /// is reconstructed on its own, so an order spanning the join splits there.
    fn prepend_page(
        &mut self,
        options: &BigTradesOptions,
        stream: &FootprintAggregator,
        page: &FootprintAggregator,
        key_base: Option<i64>,
    ) {
        self.follow(options, stream, key_base);
        let mut orders = OrderState::new(options);
        // Page bars lead the joined history, so their positions are already final.
        let bars = page.bars();
        let count = page.trades().len();
        let positions = self.key_base.map(|_| bar_positions_from(bars, count, 0));
        for (offset, (trade, side)) in page.classified_trades_from(0).enumerate() {
            let bar_time = match (self.key_base, &positions) {
                (Some(key_base), Some(positions)) => Some(key_base + positions[offset] as i64),
                _ => page.print_bar_time(trade.timestamp_micros),
            };
            if let Some(bar_time) = bar_time {
                orders.push_print(options, trade, side, bar_time);
            }
        }
        if let Some(open) = orders.open.take() {
            orders.complete(options, open.order());
        }
        for state in [&mut self.state, &mut self.sealed] {
            let room = MAX_BIG_TRADES_BUBBLES - state.bubbles.len();
            for order in orders.bubbles.iter().rev().take(room) {
                state.bubbles.push_front(*order);
            }
        }
    }

    /// Retained capacity; bounded by [`MAX_BIG_TRADES_BUBBLES`] and the automatic sample.
    fn capacity_bytes(&self) -> usize {
        self.state.capacity_bytes() + self.sealed.capacity_bytes()
    }
}

impl OrderState {
    fn new(options: &BigTradesOptions) -> Self {
        Self {
            threshold: match options.filter {
                BigTradesFilter::Auto { .. } => None,
                BigTradesFilter::Fixed { minimum_volume } => Some(minimum_volume),
            },
            bar_floor: i64::MIN,
            ..Self::default()
        }
    }

    fn shown(&self, options: &BigTradesOptions, order: &BigTrade) -> bool {
        order.bar_time >= self.bar_floor && self.qualifies(options, order.volume)
    }

    fn qualifies(&self, options: &BigTradesOptions, volume: f64) -> bool {
        self.threshold
            .is_some_and(|threshold| match options.filter {
                BigTradesFilter::Auto { .. } => volume > threshold,
                BigTradesFilter::Fixed { .. } => volume >= threshold,
            })
    }

    /// Newest bar key an order of this state carries, or `i64::MIN` without orders.
    fn newest_bar_time(&self) -> i64 {
        self.bubbles
            .back()
            .map(|order| order.bar_time)
            .into_iter()
            .chain(self.open.map(|open| open.bar_time))
            .max()
            .unwrap_or(i64::MIN)
    }

    /// Follow bars re-keyed by `rekey`, then drop the orders that opened before `floor`, the key
    /// of the oldest retained bar. Bubbles are in bar order, so the evicted ones form a prefix. A
    /// still-forming order whose bar left keeps folding its prints unseen, as it would without
    /// retention.
    fn rebase(&mut self, rekey: i64, floor: Option<i64>) {
        if rekey != 0 {
            for order in self.bubbles.iter_mut() {
                order.bar_time -= rekey;
            }
            if let Some(open) = self.open.as_mut() {
                open.bar_time -= rekey;
            }
            self.bar_floor = self.bar_floor.saturating_sub(rekey);
        }
        if let Some(floor) = floor {
            let evicted = self.bubbles.partition_point(|order| order.bar_time < floor);
            self.bubbles.drain(..evicted);
            self.bar_floor = self.bar_floor.max(floor);
        }
    }

    /// Retained capacity; bounded by [`MAX_BIG_TRADES_BUBBLES`] and the automatic sample.
    fn capacity_bytes(&self) -> usize {
        self.bubbles.capacity() * core::mem::size_of::<BigTrade>()
            + self.recent_volumes.capacity() * core::mem::size_of::<f64>()
    }

    fn push_print(
        &mut self,
        options: &BigTradesOptions,
        trade: &FootprintTrade,
        side: AggressorSide,
        bar_time: i64,
    ) {
        if let Some(open) = self.open.as_mut()
            && open.continues_with(trade, side, options.grouping_window_micros)
        {
            open.add(trade);
            return;
        }
        if let Some(open) = self.open.take() {
            self.complete(options, open.order());
        }
        // A print without a known aggressor cannot belong to an aggressive order.
        if side != AggressorSide::Unknown {
            self.open = Some(OpenOrder::new(trade, side, bar_time));
        }
    }

    fn complete(&mut self, options: &BigTradesOptions, order: BigTrade) {
        if self.shown(options, &order) {
            if self.bubbles.len() == MAX_BIG_TRADES_BUBBLES {
                self.bubbles.pop_front();
            }
            self.bubbles.push_back(order);
        }
        let BigTradesFilter::Auto { intensity } = options.filter else {
            return;
        };
        if self.recent_volumes.len() == AUTO_SAMPLE_ORDERS {
            self.recent_volumes.pop_front();
        }
        self.recent_volumes.push_back(order.volume);
        self.completed_orders = self.completed_orders.saturating_add(1);
        if self.completed_orders.is_multiple_of(AUTO_REFRESH_ORDERS) {
            self.threshold = Some(quantile(&self.recent_volumes, intensity.quantile()));
        }
    }

    fn forming(&self, options: &BigTradesOptions) -> Option<BigTrade> {
        self.open
            .map(|open| open.order())
            .filter(|order| self.shown(options, order))
    }
}

/// Bar position of every print from `from` onward. The bars after sealed history partition the
/// visible raw tape in order, so a suffix is resolved by walking back from the newest bar.
fn bar_positions_from(bars: &[FootprintBar], trade_count: usize, from: usize) -> Vec<usize> {
    let mut positions = vec![0; trade_count.saturating_sub(from)];
    let mut end = trade_count;
    for (index, bar) in bars.iter().enumerate().rev() {
        if end <= from {
            break;
        }
        let start = end.saturating_sub(bar.trade_count as usize);
        for position in &mut positions[start.max(from) - from..end - from] {
            *position = index;
        }
        end = start;
    }
    positions
}

fn quantile(values: &VecDeque<f64>, quantile: f64) -> f64 {
    let mut sorted = values.iter().copied().collect::<Vec<_>>();
    let index = ((sorted.len() - 1) as f64 * quantile).round() as usize;
    *sorted.select_nth_unstable_by(index, f64::total_cmp).1
}

/// Compact volume label: `250`, `12.5`, `0.25`, `1.23k`, `4.5M`.
fn format_big_trade_volume(volume: f64) -> String {
    let (value, suffix) = if volume >= 1_000_000.0 {
        (volume / 1_000_000.0, "M")
    } else if volume >= 1_000.0 {
        (volume / 1_000.0, "k")
    } else {
        (volume, "")
    };
    let digits = if value >= 100.0 {
        0
    } else if value >= 10.0 {
        1
    } else {
        2
    };
    let text = format!("{value:.digits$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        &text
    };
    format!("{text}{suffix}")
}

#[derive(Clone, Debug)]
pub(crate) struct BigTradesIndicator {
    pub id: NativePrimitiveId,
    pub series_id: SeriesId,
    options: BigTradesOptions,
    builder: OrderBuilder,
}

impl ChartEngine {
    /// Draw the large aggressive orders of a chart trade stream as volume bubbles over any price
    /// series. The indicator keeps the stream alive and is removed with its series.
    pub fn add_big_trades(
        &mut self,
        stream_id: u64,
        series_id: SeriesId,
        options: BigTradesOptions,
    ) -> Result<NativePrimitiveId, FootprintError> {
        let stream = self
            .trade_streams
            .get(&stream_id)
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
            return Err(FootprintError::InvalidBigTradesOptions);
        }
        if self.big_trades.values().map(Vec::len).sum::<usize>() >= MAX_BIG_TRADES_INDICATORS {
            return Err(FootprintError::BigTradesCapacity);
        }
        let id = self.next_native_primitive_id;
        self.next_native_primitive_id =
            id.checked_add(1).ok_or(FootprintError::BigTradesCapacity)?;
        let key_base = self.sequence_key_base(stream_id);
        let mut builder = OrderBuilder::new(&options, stream, key_base);
        let work = builder.refresh(&options, stream, false, key_base);
        self.big_trades
            .entry(stream_id)
            .or_default()
            .push(BigTradesIndicator {
                id,
                series_id,
                options,
                builder,
            });
        self.record_big_trades_work(stream_id, work);
        self.invalidate_frame_series(series_id);
        Ok(id)
    }

    /// Restyle in place; a filter or grouping change replays the raw tape once. Sealed history
    /// has released its trades, so its bubbles do not survive such a change.
    pub fn set_big_trades_options(
        &mut self,
        id: NativePrimitiveId,
        options: BigTradesOptions,
    ) -> Result<(), FootprintError> {
        if !options.valid() {
            return Err(FootprintError::InvalidBigTradesOptions);
        }
        let stream_id = self
            .big_trades
            .iter()
            .find_map(|(stream_id, indicators)| {
                indicators
                    .iter()
                    .any(|indicator| indicator.id == id)
                    .then_some(*stream_id)
            })
            .ok_or(FootprintError::UnknownBigTrades(id))?;
        let key_base = self.sequence_key_base(stream_id);
        let (stream_id, indicator) = self
            .big_trades
            .iter_mut()
            .find_map(|(stream_id, indicators)| {
                indicators
                    .iter_mut()
                    .find(|indicator| indicator.id == id)
                    .map(|indicator| (*stream_id, indicator))
            })
            .ok_or(FootprintError::UnknownBigTrades(id))?;
        let mut work = BuilderWork::default();
        if indicator.options.rebuild_required(&options) {
            let stream = self
                .trade_streams
                .get(&stream_id)
                .ok_or(FootprintError::UnknownTradeStream(stream_id))?;
            indicator.builder = OrderBuilder::new(&options, stream, key_base);
            work = indicator.builder.refresh(&options, stream, false, key_base);
        }
        indicator.options = options;
        let series_id = indicator.series_id;
        self.record_big_trades_work(stream_id, work);
        self.invalidate_frame_series(series_id);
        Ok(())
    }

    pub fn big_trades_options(&self, id: NativePrimitiveId) -> Option<&BigTradesOptions> {
        self.big_trades_indicator(id)
            .map(|indicator| &indicator.options)
    }

    pub fn big_trades_snapshot(&self, id: NativePrimitiveId) -> Option<BigTradesSnapshot> {
        let indicator = self.big_trades_indicator(id)?;
        let builder = &indicator.builder.state;
        Some(BigTradesSnapshot {
            threshold: builder.threshold,
            bubbles: builder
                .bubbles
                .iter()
                .copied()
                .chain(builder.forming(&indicator.options))
                .collect(),
        })
    }

    pub fn remove_big_trades(&mut self, id: NativePrimitiveId) -> bool {
        let Some((stream_id, series_id)) =
            self.big_trades
                .iter_mut()
                .find_map(|(stream_id, indicators)| {
                    let index = indicators.iter().position(|indicator| indicator.id == id)?;
                    Some((*stream_id, indicators.remove(index).series_id))
                })
        else {
            return false;
        };
        self.big_trades
            .retain(|_, indicators| !indicators.is_empty());
        self.invalidate_frame_series(series_id);
        self.prune_trade_stream_if_unused(stream_id);
        true
    }

    fn big_trades_indicator(&self, id: NativePrimitiveId) -> Option<&BigTradesIndicator> {
        self.big_trades
            .values()
            .flatten()
            .find(|indicator| indicator.id == id)
    }

    pub(crate) fn big_trades_count(&self, stream_id: u64) -> usize {
        self.big_trades.get(&stream_id).map_or(0, Vec::len)
    }

    /// Retained capacity of every big-trades indicator.
    pub(crate) fn big_trades_capacity_bytes(&self) -> usize {
        self.big_trades
            .values()
            .flatten()
            .map(|indicator| indicator.builder.capacity_bytes())
            .sum()
    }

    /// Advance every indicator of a stream after its tape or bars changed. `tip` is true only
    /// for an append at the tape tip, sealing, and eviction; otherwise the raw tape is replayed
    /// from the sealed state.
    pub(crate) fn refresh_big_trades(&mut self, stream_id: u64, tip: bool) {
        let key_base = self.sequence_key_base(stream_id);
        let count = self.big_trades_count(stream_id);
        let mut work = BuilderWork::default();
        for index in 0..count {
            let (Some(stream), Some(indicator)) = (
                self.trade_streams.get(&stream_id),
                self.big_trades
                    .get_mut(&stream_id)
                    .and_then(|indicators| indicators.get_mut(index)),
            ) else {
                break;
            };
            work.add(
                indicator
                    .builder
                    .refresh(&indicator.options, stream, tip, key_base),
            );
            let series_id = indicator.series_id;
            self.invalidate_frame_series(series_id);
        }
        self.record_big_trades_work(stream_id, work);
    }

    /// Move an indicator to another price series, keeping its orders.
    pub(crate) fn rehost_big_trades(&mut self, id: NativePrimitiveId, series_id: SeriesId) {
        let Some(indicator) = self
            .big_trades
            .values_mut()
            .flatten()
            .find(|indicator| indicator.id == id)
        else {
            return;
        };
        let previous = std::mem::replace(&mut indicator.series_id, series_id);
        self.invalidate_frame_series(previous);
        self.invalidate_frame_series(series_id);
    }

    /// Fold the trades of the oldest `bars` raw bars into every indicator's replay start state
    /// before the stream seals those bars and releases their trades.
    pub(crate) fn absorb_sealed_trades_into_big_trades(&mut self, stream_id: u64, bars: usize) {
        let key_base = self.sequence_key_base(stream_id);
        let (Some(stream), Some(indicators)) = (
            self.trade_streams.get(&stream_id),
            self.big_trades.get_mut(&stream_id),
        ) else {
            return;
        };
        let count = stream.raw_trades_of_bars(bars);
        let mut work = BuilderWork::default();
        for indicator in indicators {
            work.add(
                indicator
                    .builder
                    .absorb_sealed(&indicator.options, stream, count, key_base),
            );
        }
        self.record_big_trades_work(stream_id, work);
    }

    /// Add the orders of an older page the stream just joined in front of its sealed history.
    pub(crate) fn prepend_big_trades_page(&mut self, stream_id: u64, page: &FootprintAggregator) {
        let key_base = self.sequence_key_base(stream_id);
        let (Some(stream), Some(indicators)) = (
            self.trade_streams.get(&stream_id),
            self.big_trades.get_mut(&stream_id),
        ) else {
            return;
        };
        for indicator in indicators {
            indicator
                .builder
                .prepend_page(&indicator.options, stream, page, key_base);
        }
    }

    fn record_big_trades_work(&mut self, stream_id: u64, work: BuilderWork) {
        self.record_dependent_work(stream_id, |recorded| {
            recorded.big_trades_prints_scanned += work.prints_scanned;
            recorded.big_trades_replays += work.replays;
        });
    }

    /// Bubbles for every visible indicator on `series_id`, largest first so smaller orders stay
    /// readable on top. Drawn above every series of the pane.
    pub(crate) fn build_big_trades_frame(
        &self,
        series_id: SeriesId,
        from: i64,
        to: i64,
        hpr: f64,
        vpr: f64,
        out: &mut Vec<Prim>,
    ) {
        if !self
            .big_trades
            .values()
            .flatten()
            .any(|indicator| indicator.series_id == series_id && indicator.options.visible)
        {
            return;
        }
        let Some(series) = self.series_entry(series_id) else {
            return;
        };
        if !series.visible {
            return;
        }
        let Some(pane) = self.panes.get(series.pane_index) else {
            return;
        };
        let scale = pane_scale(pane, series_scale_target(series));
        if scale.is_empty() {
            return;
        }
        let Some(base_value) = self.series_base_value(series_id, from) else {
            return;
        };
        // ponytail: every chrome rebuild allocates the visible-order list, its sort, one label
        // String per labelled bubble, and a font-family clone, bounded by 16 indicators x 4,096
        // orders. Reuse a scratch buffer only once the release perf_gate measures it.
        let layout = &self.options.get().layout;
        let family = layout.font_family.clone();
        let font_size = layout.font_size;
        for indicator in self
            .big_trades
            .values()
            .flatten()
            .filter(|indicator| indicator.series_id == series_id && indicator.options.visible)
        {
            let options = &indicator.options;
            let parse = |color: &str| Color::parse_css(color).expect("validated big-trades color");
            let (buy_fill, sell_fill) = (parse(&options.buy_color), parse(&options.sell_color));
            let (buy_border, sell_border) = (
                parse(&options.buy_border_color),
                parse(&options.sell_border_color),
            );
            let text_color = options
                .text_color
                .as_deref()
                .map_or_else(|| self.primary_text_color(), parse);
            let sequence_axis = indicator.builder.key_base.is_some();
            let builder = &indicator.builder.state;
            let forming = builder.forming(options);
            let peak = builder
                .bubbles
                .iter()
                .chain(forming.as_ref())
                .map(|order| order.volume)
                .fold(0.0_f64, f64::max);
            if peak <= 0.0 {
                continue;
            }
            let mut visible = builder
                .bubbles
                .iter()
                .chain(forming.as_ref())
                .filter_map(|order| {
                    // Sequence-axis orders carry their bar's row key, which retention never
                    // re-keys; the time scale indexes the chart's row keys.
                    let index = if sequence_axis {
                        self.data
                            .merged_times()
                            .binary_search(&order.bar_time)
                            .ok()
                            .map(|index| index as i64)
                    } else {
                        self.time_to_index(order.bar_time as f64, true)
                    }?;
                    (from..=to).contains(&index).then_some((index, order))
                })
                .collect::<Vec<_>>();
            visible.sort_by(|left, right| right.1.volume.total_cmp(&left.1.volume));
            let (min_diameter, max_diameter) = options.size.diameter_range();
            for (index, order) in visible {
                let (fill, border) = if order.side == AggressorSide::Buy {
                    (buy_fill, buy_border)
                } else {
                    (sell_fill, sell_border)
                };
                let diameter =
                    min_diameter + (max_diameter - min_diameter) * (order.volume / peak).sqrt();
                let x = self.time_scale.index_to_coordinate(index) * hpr;
                let y = scale.price_to_coordinate(order.vwap, base_value) * vpr;
                if order.high > order.low {
                    let top = scale.price_to_coordinate(order.high, base_value) * vpr;
                    let bottom = scale.price_to_coordinate(order.low, base_value) * vpr;
                    let width = hpr.round().max(1.0) as i32;
                    out.push(Prim::Rect {
                        rect: IRect {
                            x: x.round() as i32 - width / 2,
                            y: top.min(bottom).round() as i32,
                            w: width,
                            h: (bottom - top).abs().round().max(1.0) as i32,
                        },
                        color: border,
                    });
                }
                out.push(Prim::Circle {
                    cx: x as f32,
                    cy: y as f32,
                    radius: (diameter * 0.5 * hpr) as f32,
                    fill,
                    stroke_width: hpr.max(1.0) as f32,
                    stroke: border,
                });
                if !options.show_volume {
                    continue;
                }
                let label = format_big_trade_volume(order.volume);
                let label_size = (diameter * 0.36).clamp(8.0, font_size.max(8.0));
                let label_width =
                    self.measure_text_run(&label, label_size, &family, LABEL_WEIGHT, false);
                if label_width + LABEL_PADDING > diameter {
                    continue;
                }
                out.push(Prim::Text {
                    x: x as f32,
                    y: y as f32,
                    text: label,
                    color: text_color,
                    size: (label_size * vpr) as f32,
                    family: family.clone(),
                    align: TextAlign::Center,
                    weight: LABEL_WEIGHT,
                    italic: false,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FootprintAggregationOptions, OrderFlowPresentationOptions};

    fn print(
        timestamp_micros: i64,
        price: f64,
        volume: f64,
        aggressor: AggressorSide,
    ) -> FootprintTrade {
        FootprintTrade {
            timestamp_micros,
            price,
            volume,
            aggressor,
            bid: None,
            ask: None,
            sequence: None,
            trade_id: None,
            conditions: 0,
            session_id: Some(1),
        }
    }

    fn fixed(minimum_volume: f64) -> BigTradesOptions {
        BigTradesOptions {
            filter: BigTradesFilter::Fixed { minimum_volume },
            ..BigTradesOptions::default()
        }
    }

    fn chart_with_stream() -> (ChartEngine, u64) {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let stream = chart
            .add_trade_stream(
                "CME:ES",
                FootprintAggregationOptions {
                    tick_size: 0.25,
                    ..FootprintAggregationOptions::default()
                },
            )
            .unwrap();
        (chart, stream)
    }

    #[test]
    fn split_prints_rebuild_one_order_before_the_filter() {
        let (mut chart, stream) = chart_with_stream();
        chart
            .set_trade_stream_trades(
                stream,
                vec![
                    // One buy order sweeping three levels as five small prints.
                    print(1_000_000, 100.0, 2.0, AggressorSide::Buy),
                    print(1_000_000, 100.0, 2.0, AggressorSide::Buy),
                    print(1_000_000, 100.25, 2.0, AggressorSide::Buy),
                    print(1_000_400, 100.5, 2.0, AggressorSide::Buy),
                    print(1_000_900, 100.5, 2.0, AggressorSide::Buy),
                    // A buy below the last fill is a new order, not a continuation.
                    print(1_000_950, 100.25, 2.0, AggressorSide::Buy),
                    // Beyond the grouping window.
                    print(1_010_000, 100.5, 9.0, AggressorSide::Buy),
                ],
            )
            .unwrap();
        let id = chart.add_big_trades(stream, 0, fixed(8.0)).unwrap();
        let snapshot = chart.big_trades_snapshot(id).unwrap();
        assert_eq!(snapshot.threshold, Some(8.0));
        assert_eq!(snapshot.bubbles.len(), 2, "no single print reaches 8 lots");
        let sweep = snapshot.bubbles[0];
        assert_eq!(sweep.side, AggressorSide::Buy);
        assert_eq!((sweep.volume, sweep.prints), (10.0, 5));
        assert_eq!((sweep.low, sweep.high), (100.0, 100.5));
        assert!((sweep.vwap - 100.25).abs() < 1e-12);
        assert_eq!(sweep.bar_time, 0);
        assert_eq!(
            (sweep.start_timestamp_micros, sweep.end_timestamp_micros),
            (1_000_000, 1_000_900)
        );
        assert_eq!(
            (snapshot.bubbles[1].volume, snapshot.bubbles[1].prints),
            (9.0, 1)
        );
    }

    #[test]
    fn opposite_aggressors_never_merge_and_unclassified_prints_start_no_order() {
        let (mut chart, stream) = chart_with_stream();
        chart
            .set_trade_stream_trades(
                stream,
                vec![
                    // Nothing precedes it, so the stream cannot classify it.
                    print(1_000_000, 100.0, 50.0, AggressorSide::Unknown),
                    print(1_000_000, 100.0, 5.0, AggressorSide::Buy),
                    print(1_000_000, 100.0, 5.0, AggressorSide::Sell),
                    print(1_000_000, 100.0, 5.0, AggressorSide::Sell),
                    // The stream's tick rule classifies a zero-tick print as the previous side.
                    print(1_000_000, 100.0, 5.0, AggressorSide::Unknown),
                ],
            )
            .unwrap();
        let id = chart.add_big_trades(stream, 0, fixed(5.0)).unwrap();
        let orders = chart
            .big_trades_snapshot(id)
            .unwrap()
            .bubbles
            .iter()
            .map(|order| (order.side, order.volume))
            .collect::<Vec<_>>();
        assert_eq!(
            orders,
            vec![(AggressorSide::Buy, 5.0), (AggressorSide::Sell, 15.0)]
        );
    }

    #[test]
    fn automatic_filter_keeps_adapting_to_recent_orders() {
        let (mut chart, stream) = chart_with_stream();
        let id = chart
            .add_big_trades(stream, 0, BigTradesOptions::default())
            .unwrap();
        // Distinct seconds keep every print its own order.
        let mut next_second = 0;
        let mut orders = |volumes: &mut dyn Iterator<Item = f64>| {
            volumes
                .map(|volume| {
                    next_second += 1;
                    print(next_second * 1_000_000, 100.0, volume, AggressorSide::Buy)
                })
                .collect::<Vec<_>>()
        };
        // An order completes when the next one starts, so one extra print completes the sample.
        let warm_up = orders(&mut (0..=AUTO_REFRESH_ORDERS).map(|index| (index % 10 + 1) as f64));
        chart.update_trade_stream_trades(stream, warm_up).unwrap();
        assert_eq!(chart.big_trades_snapshot(id).unwrap().threshold, Some(10.0));
        let quiet_market_block = orders(&mut [50.0, 1.0].into_iter());
        chart
            .update_trade_stream_trades(stream, quiet_market_block)
            .unwrap();
        let snapshot = chart.big_trades_snapshot(id).unwrap();
        assert_eq!(
            snapshot
                .bubbles
                .iter()
                .map(|order| order.volume)
                .collect::<Vec<_>>(),
            vec![50.0],
            "a 50-lot stands out against 1-10 lot orders"
        );

        let busy_market = orders(&mut std::iter::repeat_n(100.0, AUTO_SAMPLE_ORDERS));
        chart
            .update_trade_stream_trades(stream, busy_market)
            .unwrap();
        assert_eq!(
            chart.big_trades_snapshot(id).unwrap().threshold,
            Some(100.0)
        );
        let after = orders(&mut [50.0, 150.0, 1.0].into_iter());
        chart.update_trade_stream_trades(stream, after).unwrap();
        let bubbles = chart.big_trades_snapshot(id).unwrap().bubbles;
        assert_eq!(bubbles.last().map(|order| order.volume), Some(150.0));
        assert_eq!(
            bubbles.iter().filter(|order| order.volume == 50.0).count(),
            1,
            "the same 50-lot no longer qualifies"
        );
    }

    #[test]
    fn tip_appends_match_a_full_replay_even_inside_an_open_order() {
        let tape = (0..600)
            .map(|index| {
                let side = if (index / 7) % 2 == 0 {
                    AggressorSide::Buy
                } else {
                    AggressorSide::Sell
                };
                let tick = f64::from(index % 5) * 0.25;
                let price = match side {
                    AggressorSide::Buy => 100.0 + tick,
                    _ => 102.0 - tick,
                };
                // One outsized print guarantees a bubble once the automatic filter is warm.
                let volume = if index == 450 {
                    500.0
                } else {
                    f64::from(index % 9 + 1)
                };
                print(1_000_000 + i64::from(index / 3) * 400, price, volume, side)
            })
            .collect::<Vec<_>>();
        let (mut live, live_stream) = chart_with_stream();
        let live_id = live
            .add_big_trades(live_stream, 0, BigTradesOptions::default())
            .unwrap();
        for batch in tape.chunks(13) {
            live.update_trade_stream_trades(live_stream, batch.to_vec())
                .unwrap();
        }
        let (mut replayed, replayed_stream) = chart_with_stream();
        replayed
            .set_trade_stream_trades(replayed_stream, tape)
            .unwrap();
        let replayed_id = replayed
            .add_big_trades(replayed_stream, 0, BigTradesOptions::default())
            .unwrap();
        let live_snapshot = live.big_trades_snapshot(live_id).unwrap();
        assert!(live_snapshot.threshold.is_some());
        assert!(!live_snapshot.bubbles.is_empty());
        assert_eq!(
            live_snapshot,
            replayed.big_trades_snapshot(replayed_id).unwrap()
        );
    }

    #[test]
    fn bubbles_draw_above_candles_with_volume_labels_on_large_orders() {
        let (mut chart, stream) = chart_with_stream();
        chart.install_series_data(
            0,
            vec![0, 60, 120],
            vec![100.0, 101.0, 102.0],
            vec![103.0, 104.0, 105.0],
            vec![99.0, 100.0, 101.0],
            vec![101.0, 102.0, 103.0],
        );
        chart
            .set_trade_stream_trades(
                stream,
                vec![
                    print(61_000_000, 101.0, 250.0, AggressorSide::Sell),
                    print(61_000_000, 100.75, 250.0, AggressorSide::Sell),
                    print(121_000_000, 102.0, 20.0, AggressorSide::Buy),
                ],
            )
            .unwrap();
        let options = fixed(10.0);
        let id = chart.add_big_trades(stream, 0, options.clone()).unwrap();
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        let frame = chart.build_frame();
        let main = &frame.panes[0].main;
        let circles = main
            .iter()
            .filter_map(|primitive| match primitive {
                Prim::Circle { radius, fill, .. } => Some((*radius, *fill)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let sell_fill = Color::parse_css(&options.sell_color).unwrap();
        let buy_fill = Color::parse_css(&options.buy_color).unwrap();
        assert_eq!(circles.len(), 2);
        assert_eq!(circles[0].1, sell_fill, "the largest order paints first");
        assert_eq!(circles[0].0, 18.0);
        assert_eq!(circles[1].1, buy_fill);
        assert!(
            main.iter()
                .any(|primitive| { matches!(primitive, Prim::Text { text, .. } if text == "500") })
        );
        let candle_end = chart
            .frame_series_segments(0)
            .iter()
            .find(|segment| segment.series_id == Some(0))
            .unwrap()
            .end;
        assert!(
            main.iter()
                .position(|primitive| matches!(primitive, Prim::Circle { .. }))
                .unwrap()
                >= candle_end,
            "bubbles paint above the series"
        );
        // The two-level sell sweep marks its traded range.
        assert!(main.iter().any(|primitive| matches!(
            primitive,
            Prim::Rect { color, rect } if *color == Color::parse_css(&options.sell_border_color).unwrap() && rect.h > 1
        )));

        chart
            .set_big_trades_options(
                id,
                BigTradesOptions {
                    show_volume: false,
                    ..options.clone()
                },
            )
            .unwrap();
        let frame = chart.build_frame();
        assert!(
            !frame.panes[0]
                .main
                .iter()
                .any(|primitive| matches!(primitive, Prim::Text { text, .. } if text == "500"))
        );
        chart
            .set_big_trades_options(
                id,
                BigTradesOptions {
                    visible: false,
                    ..options
                },
            )
            .unwrap();
        let frame = chart.build_frame();
        assert!(
            !frame.panes[0]
                .main
                .iter()
                .any(|primitive| matches!(primitive, Prim::Circle { .. }))
        );
    }

    #[test]
    fn lifecycle_validates_options_and_releases_the_stream() {
        let (mut chart, stream) = chart_with_stream();
        assert_eq!(
            chart.add_big_trades(stream, 0, fixed(0.0)),
            Err(FootprintError::InvalidBigTradesOptions)
        );
        assert_eq!(
            chart.add_big_trades(
                stream,
                0,
                BigTradesOptions {
                    buy_color: "not a color".into(),
                    ..BigTradesOptions::default()
                }
            ),
            Err(FootprintError::InvalidBigTradesOptions)
        );
        let histogram = chart.add_series(SeriesKind::Histogram);
        assert_eq!(
            chart.add_big_trades(stream, histogram, BigTradesOptions::default()),
            Err(FootprintError::UnsupportedBigTradesSeries(histogram))
        );
        assert_eq!(
            chart.add_big_trades(99, 0, BigTradesOptions::default()),
            Err(FootprintError::UnknownTradeStream(99))
        );

        let id = chart
            .add_big_trades(stream, 0, BigTradesOptions::default())
            .unwrap();
        assert_eq!(chart.trade_stream_stats(stream).unwrap().dependent_count, 1);
        assert_eq!(
            chart.remove_trade_stream(stream),
            Err(FootprintError::TradeStreamInUse(stream))
        );
        chart.set_big_trades_options(id, fixed(3.0)).unwrap();
        assert_eq!(chart.big_trades_options(id), Some(&fixed(3.0)));
        assert_eq!(chart.big_trades_snapshot(id).unwrap().threshold, Some(3.0));
        assert!(chart.remove_big_trades(id));
        assert!(!chart.remove_big_trades(id));
        assert_eq!(chart.big_trades_options(id), None);
        assert!(chart.remove_trade_stream(stream).is_ok());

        let line = chart.add_series(SeriesKind::Line);
        let stream = chart
            .add_trade_stream("CME:NQ", FootprintAggregationOptions::default())
            .unwrap();
        let id = chart
            .add_big_trades(stream, line, BigTradesOptions::default())
            .unwrap();
        assert!(chart.remove_series(line));
        assert_eq!(chart.big_trades_options(id), None);
        for index in 0..MAX_BIG_TRADES_INDICATORS {
            chart
                .add_big_trades(stream, 0, fixed(1.0 + index as f64))
                .unwrap();
        }
        assert_eq!(
            chart.add_big_trades(stream, 0, BigTradesOptions::default()),
            Err(FootprintError::BigTradesCapacity)
        );
    }

    #[test]
    fn order_flow_presentation_installs_big_trades_on_the_primary_candles() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let presentation = chart
            .add_order_flow_presentation(
                "CME:ES",
                0,
                OrderFlowPresentationOptions {
                    aggregation: FootprintAggregationOptions {
                        tick_size: 0.25,
                        ticks_per_row: 1,
                        ..FootprintAggregationOptions::default()
                    },
                    visual: crate::FootprintVisualOptions::default(),
                    show_footprint: false,
                    show_cumulative_delta: false,
                    show_delta_histogram: false,
                    big_trades: Some(fixed(5.0)),
                },
            )
            .unwrap();
        let id = presentation.big_trades().unwrap();
        chart
            .update_order_flow_presentation(
                presentation,
                vec![print(1_000_000, 100.0, 6.0, AggressorSide::Buy)],
                false,
            )
            .unwrap();
        assert_eq!(chart.big_trades_snapshot(id).unwrap().bubbles.len(), 1);
        assert!(chart.remove_order_flow_presentation(presentation));
        assert_eq!(chart.big_trades_options(id), None);
        assert!(chart.trade_stream(presentation.trade_stream()).is_none());
    }

    #[test]
    fn retained_bubbles_are_bounded_to_the_newest_orders() {
        let (mut chart, stream) = chart_with_stream();
        // Every print is its own order; one more completed order than the cap, plus the
        // still-forming tip order.
        let orders = MAX_BIG_TRADES_BUBBLES as i64 + 2;
        chart
            .set_trade_stream_trades(
                stream,
                (0..orders)
                    .map(|index| print(1_000_000 + index * 10_000, 100.0, 1.0, AggressorSide::Buy))
                    .collect(),
            )
            .unwrap();
        let id = chart.add_big_trades(stream, 0, fixed(1.0)).unwrap();
        let bubbles = chart.big_trades_snapshot(id).unwrap().bubbles;
        assert_eq!(bubbles.len(), MAX_BIG_TRADES_BUBBLES + 1);
        assert_eq!(
            bubbles[0].start_timestamp_micros, 1_010_000,
            "the oldest order is evicted"
        );
        assert_eq!(
            bubbles.last().unwrap().start_timestamp_micros,
            1_000_000 + (orders - 1) * 10_000
        );
    }

    /// The orders a builder shows: completed ones oldest first, then the forming one.
    fn shown(builder: &OrderBuilder, options: &BigTradesOptions) -> Vec<BigTrade> {
        builder
            .state
            .bubbles
            .iter()
            .copied()
            .chain(builder.state.forming(options))
            .collect()
    }

    fn crafted(index: usize) -> FootprintTrade {
        FootprintTrade {
            timestamp_micros: 1_700_000_000_000_000 + index as i64 * 150_000,
            price: 100.0 + (index / 3 % 2) as f64 * 0.25,
            volume: 1.0 + (index * 7 % 5) as f64,
            aggressor: if (index / 3).is_multiple_of(3) {
                AggressorSide::Sell
            } else {
                AggressorSide::Buy
            },
            trade_id: Some(index as u64),
            ..print(0, 100.0, 1.0, AggressorSide::Buy)
        }
    }

    #[test]
    fn suffix_bar_positions_and_labels_are_exact() {
        let bar = |trade_count| FootprintBar {
            trade_count,
            ..FootprintBar::default()
        };
        let bars = [bar(2), bar(0), bar(3)];
        assert_eq!(bar_positions_from(&bars, 5, 0), vec![0, 0, 2, 2, 2]);
        assert_eq!(bar_positions_from(&bars, 5, 3), vec![2, 2]);
        // Two prints per bar on a sequence axis whose rows start at key 7; every print is its own
        // order. Folding the tape in suffixes resolves the same bar keys as one pass (time-bar
        // orders key by the print's own bar time instead of a position walk).
        let mut stream = FootprintAggregator::new(FootprintAggregationOptions {
            tick_size: 0.25,
            bars: crate::FootprintBarAggregation::Trades { trades_per_bar: 2 },
            ..FootprintAggregationOptions::default()
        })
        .unwrap();
        let options = BigTradesOptions {
            grouping_window_micros: 0,
            ..fixed(0.5)
        };
        let tape = (0..5)
            .map(|index| print(1_000_000 + index * 10, 100.0, 1.0, AggressorSide::Buy))
            .collect::<Vec<_>>();
        stream.set_trades(tape.clone()).unwrap();
        let mut whole = OrderBuilder::new(&options, &stream, Some(7));
        whole.refresh(&options, &stream, false, Some(7));
        let keys = |builder: &OrderBuilder| {
            shown(builder, &options)
                .iter()
                .map(|order| order.bar_time)
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(&whole), vec![7, 7, 8, 8, 9]);
        let mut stream = FootprintAggregator::new(stream.options()).unwrap();
        let mut suffixes = OrderBuilder::new(&options, &stream, Some(7));
        for chunk in [&tape[..1], &tape[1..3], &tape[3..]] {
            stream.update_trades(chunk.to_vec()).unwrap();
            let work = suffixes.refresh(&options, &stream, true, Some(7));
            assert_eq!((work.prints_scanned, work.replays), (chunk.len() as u64, 0));
        }
        assert_eq!(keys(&suffixes), keys(&whole));
        assert_eq!(format_big_trade_volume(250.0), "250");
        assert_eq!(format_big_trade_volume(12.5), "12.5");
        assert_eq!(format_big_trade_volume(0.25), "0.25");
        assert_eq!(format_big_trade_volume(1_234.0), "1.23k");
        assert_eq!(format_big_trade_volume(4_500_000.0), "4.5M");
    }

    /// Retention seals and then evicts the oldest bars. It drops the orders that opened in evicted
    /// bars in place and changes nothing else: after the eviction and a further tip, the retained
    /// builder shows exactly the unretained builder's orders on the retained bars, with the same
    /// automatic threshold, on time and sequence axes alike, including orders that straddle the
    /// eviction boundary, and a replay of the raw tape rebuilds the same orders.
    #[test]
    fn eviction_in_place_keeps_exactly_the_unretained_orders_of_the_retained_bars() {
        // Time bars are whole chart seconds (chart streams reject sub-second intervals), so an
        // order's second key identifies its bar.
        let second = FootprintAggregationOptions {
            tick_size: 0.25,
            bars: crate::FootprintBarAggregation::Time {
                interval_micros: 1_000_000,
                anchor_micros: 0,
            },
            ..FootprintAggregationOptions::default()
        };
        let minute = FootprintAggregationOptions {
            tick_size: 0.25,
            ..FootprintAggregationOptions::default()
        };
        let trade_bars = FootprintAggregationOptions {
            tick_size: 0.25,
            bars: crate::FootprintBarAggregation::Trades { trades_per_bar: 3 },
            ..FootprintAggregationOptions::default()
        };
        let auto = BigTradesOptions {
            filter: BigTradesFilter::Auto {
                intensity: BigTradesIntensity::Weak,
            },
            grouping_window_micros: 400_000,
            ..BigTradesOptions::default()
        };
        let fixed = BigTradesOptions {
            grouping_window_micros: 400_000,
            ..fixed(2.0)
        };
        let (mut evictions, mut straddles) = (0, 0);
        for (aggregation, key_base) in [(second, None), (minute, None), (trade_bars, Some(7))] {
            for options in [&fixed, &auto] {
                let mut stream = FootprintAggregator::new(aggregation).unwrap();
                stream.set_trades((0..900).map(crafted).collect()).unwrap();
                let mut full = OrderBuilder::new(options, &stream, key_base);
                full.refresh(options, &stream, false, key_base);
                let tip = (900..930).map(crafted).collect::<Vec<_>>();
                let bars = stream.bars().len();
                for keep in (1..bars).step_by(bars / 40 + 1) {
                    let mut retained = stream.clone();
                    let mut builder = full.clone();
                    let evicted = bars - keep;
                    // Retention seals the evicted bars, which the builder absorbs first, then
                    // evicts them; following the eviction folds and replays nothing.
                    let released = retained.raw_trades_of_bars(evicted);
                    let work = builder.absorb_sealed(options, &retained, released, key_base);
                    assert_eq!((work.prints_scanned, work.replays), (0, 0));
                    assert_eq!(retained.seal_bars(evicted), released);
                    retained.evict_sealed_bars(evicted);
                    let moved = key_base.map(|base| base + evicted as i64);
                    let work = builder.refresh(options, &retained, true, moved);
                    assert_eq!((work.prints_scanned, work.replays), (0, 0));
                    let first_key = moved.unwrap_or_else(|| {
                        retained.bars()[0]
                            .start_timestamp_micros
                            .div_euclid(MICROS_PER_SECOND)
                    });
                    straddles += usize::from(
                        builder
                            .state
                            .open
                            .is_some_and(|open| open.bar_time < first_key)
                            || full.state.bubbles.iter().any(|order| {
                                order.bar_time < first_key
                                    && order.end_timestamp_micros
                                        >= retained.bars()[0].start_timestamp_micros
                            }),
                    );
                    let mut unretained = stream.clone();
                    let mut reference = full.clone();
                    retained.update_trades(tip.clone()).unwrap();
                    unretained.update_trades(tip.clone()).unwrap();
                    let work = builder.refresh(options, &retained, true, moved);
                    assert_eq!((work.prints_scanned, work.replays), (tip.len() as u64, 0));
                    reference.refresh(options, &unretained, true, key_base);
                    let tail = shown(&reference, options)
                        .into_iter()
                        .filter(|order| order.bar_time >= first_key)
                        .collect::<Vec<_>>();
                    let context = format!("{aggregation:?} {:?} keep {keep}", options.filter);
                    assert_eq!(shown(&builder, options), tail, "{context}");
                    assert_eq!(
                        builder.state.threshold, reference.state.threshold,
                        "{context}"
                    );
                    // A replay starts from the state after the released trades, so it rebuilds
                    // exactly what the in-place path kept, including the automatic filter.
                    let mut replayed = builder.clone();
                    let work = replayed.refresh(options, &retained, false, moved);
                    assert_eq!(work.replays, 1, "{context}");
                    assert_eq!(shown(&replayed, options), tail, "{context}");
                    assert_eq!(
                        replayed.state.threshold, reference.state.threshold,
                        "{context}"
                    );
                    evictions += 1;
                }
            }
        }
        assert!(evictions > 100 && straddles > 0, "{evictions} {straddles}");
    }

    /// The stream counts the prints big trades fold and the tape replays they run: a tip folds
    /// only its new prints, a retention trim replays nothing, and a filter change replays once.
    #[test]
    fn work_counters_bound_tips_and_retention() {
        let (mut chart, stream) = chart_with_stream();
        let footprint = chart
            .add_footprint_series(crate::FootprintSeriesOptions {
                aggregation: FootprintAggregationOptions {
                    tick_size: 0.25,
                    ..FootprintAggregationOptions::default()
                },
                ..crate::FootprintSeriesOptions::default()
            })
            .unwrap();
        chart
            .bind_footprint_series_to_stream(footprint, stream)
            .unwrap();
        let id = chart.add_big_trades(stream, footprint, fixed(2.0)).unwrap();
        assert!(chart.set_series_max_points(footprint, Some(12)));
        let minute = |index: usize| FootprintTrade {
            timestamp_micros: 1_700_000_000_000_000 + index as i64 * 20_000_000,
            ..crafted(index)
        };
        chart
            .set_trade_stream_trades(stream, (0..30).map(minute).collect())
            .unwrap();
        let counters = |chart: &ChartEngine| {
            let stats = chart.trade_stream_stats(stream).unwrap();
            (stats.big_trades_prints_scanned, stats.big_trades_replays)
        };
        let mut trims = 0;
        for index in 30..120 {
            let before = counters(&chart);
            let first = chart
                .footprint_bar(footprint, 0)
                .unwrap()
                .start_timestamp_micros;
            chart
                .update_trade_stream_trades(stream, vec![minute(index)])
                .unwrap();
            let after = counters(&chart);
            assert_eq!(
                (after.0 - before.0, after.1 - before.1),
                (1, 0),
                "tip {index}"
            );
            trims += usize::from(
                chart
                    .footprint_bar(footprint, 0)
                    .unwrap()
                    .start_timestamp_micros
                    != first,
            );
        }
        assert!(trims >= 3, "{trims}");
        let before = counters(&chart);
        chart.set_big_trades_options(id, fixed(3.0)).unwrap();
        let visible = chart.trade_stream(stream).unwrap().trades().len() as u64;
        let after = counters(&chart);
        assert_eq!((after.0 - before.0, after.1 - before.1), (visible, 1));
    }
}
