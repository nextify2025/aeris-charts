//! Market data for the page: the types the chart and header consume, the [`MarketData`] seam a
//! real feed plugs into, and [`SimulatedMarket`], a deterministic feed that needs no account.
//!
//! Timestamps are Unix seconds holding the exchange's local wall-clock time (09:30 is stored as
//! 09:30 UTC), so the chart's time axis reads in exchange time without time-zone handling.

/// Seconds in a day.
pub const DAY: i64 = 86_400;
const MINUTE: i64 = 60;
/// Regular session: 09:30 to 16:00 exchange time, 390 one-minute bars.
const SESSION_OPEN: i64 = 9 * 3_600 + 30 * MINUTE;
const SESSION_MINUTES: i64 = 390;

/// A chart period, as the period tabs list them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Period {
    /// Today's session minute by minute, drawn as a price line with its average-price line.
    Intraday,
    Min1,
    Min5,
    Min15,
    Hour1,
    Day,
    Week,
    Month,
}

impl Period {
    pub const ALL: [Period; 8] = [
        Period::Intraday,
        Period::Min1,
        Period::Min5,
        Period::Min15,
        Period::Hour1,
        Period::Day,
        Period::Week,
        Period::Month,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Period::Intraday => "分时",
            Period::Min1 => "1分",
            Period::Min5 => "5分",
            Period::Min15 => "15分",
            Period::Hour1 => "1小时",
            Period::Day => "日K",
            Period::Week => "周K",
            Period::Month => "月K",
        }
    }

    /// Bar length for minute-based periods.
    fn minutes(self) -> Option<i64> {
        match self {
            Period::Intraday | Period::Min1 => Some(1),
            Period::Min5 => Some(5),
            Period::Min15 => Some(15),
            Period::Hour1 => Some(60),
            Period::Day | Period::Week | Period::Month => None,
        }
    }
}

/// One bar. `turnover` is the traded value (price × volume summed over the bar's trades).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candle {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub turnover: f64,
}

impl Candle {
    /// Fold a later bar into this one.
    fn merge(&mut self, next: &Candle) {
        self.high = self.high.max(next.high);
        self.low = self.low.min(next.low);
        self.close = next.close;
        self.volume += next.volume;
        self.turnover += next.turnover;
    }
}

/// The header's quote and key statistics.
#[derive(Clone, Debug)]
pub struct Quote {
    /// Time of the latest trade, in exchange wall-clock seconds.
    pub time: i64,
    /// Whether the session is still trading.
    pub trading: bool,
    pub symbol: String,
    pub name: String,
    pub exchange: String,
    pub currency: String,
    pub last: f64,
    pub prev_close: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub volume: f64,
    pub turnover: f64,
    pub week52_high: f64,
    pub week52_low: f64,
    pub shares_outstanding: f64,
    pub eps_ttm: f64,
    pub dividend_per_share: f64,
}

impl Quote {
    pub fn change(&self) -> f64 {
        self.last - self.prev_close
    }

    pub fn change_percent(&self) -> f64 {
        if self.prev_close == 0.0 {
            0.0
        } else {
            self.change() / self.prev_close * 100.0
        }
    }

    pub fn market_cap(&self) -> f64 {
        self.last * self.shares_outstanding
    }

    pub fn pe_ttm(&self) -> Option<f64> {
        (self.eps_ttm > 0.0).then(|| self.last / self.eps_ttm)
    }

    pub fn dividend_yield(&self) -> f64 {
        if self.last == 0.0 {
            0.0
        } else {
            self.dividend_per_share / self.last * 100.0
        }
    }

    /// Today's range as a percentage of the previous close.
    pub fn amplitude(&self) -> f64 {
        if self.prev_close == 0.0 {
            0.0
        } else {
            (self.high - self.low) / self.prev_close * 100.0
        }
    }
}

/// Where the page gets its data. [`SimulatedMarket`] is built in; a broker adapter such as the
/// Longbridge one sketched in `longbridge.rs` implements the same four calls.
pub trait MarketData {
    /// The latest quote.
    fn quote(&self) -> Quote;
    /// Every bar of `period`, oldest first.
    fn candles(&self, period: Period) -> Vec<Candle>;
    /// The newest bar of `period`, reflecting the latest trade.
    fn latest(&self, period: Period) -> Option<Candle>;
    /// Advance the live feed. Returns whether anything changed; a real feed would drain the
    /// trades pushed to it since the last call.
    fn poll(&mut self) -> bool;
}

/// A deterministic random walk: two years of daily bars and five sessions of minute bars, the last
/// session still trading. Every period is aggregated from the same bars, so they always agree.
pub struct SimulatedMarket {
    rng: Rng,
    days: Vec<Candle>,
    /// The last five sessions minute by minute; the last session is today.
    minutes: Vec<Candle>,
    /// Session start (00:00 exchange time) of each minute session, oldest first.
    sessions: Vec<i64>,
    /// Simulated seconds into the current minute, advanced by [`MarketData::poll`].
    tick_seconds: i64,
}

/// 2026-09-25, a Friday: the simulated "today".
const TODAY: i64 = 1_790_294_400;
const DAILY_SESSIONS: usize = 520;
const MINUTE_SESSIONS: usize = 5;
/// Today's session has traded this many minutes when the app starts (14:10).
const MINUTES_TRADED_TODAY: i64 = 280;

impl SimulatedMarket {
    pub fn new() -> Self {
        let mut rng = Rng(0x5eed_2026_0925);
        // Trading days, oldest first, skipping weekends.
        let mut dates = Vec::with_capacity(DAILY_SESSIONS);
        let mut date = TODAY;
        while dates.len() < DAILY_SESSIONS {
            let weekday = (date / DAY + 4).rem_euclid(7); // 1970-01-01 was a Thursday.
            if weekday != 0 && weekday != 6 {
                dates.push(date);
            }
            date -= DAY;
        }
        dates.reverse();

        // Daily bars: drifting regimes so trends, reversals, and ranges all show up.
        let mut days = Vec::with_capacity(DAILY_SESSIONS);
        let mut price = 86.0;
        for (index, &date) in dates.iter().enumerate() {
            let regime = match index * 8 / DAILY_SESSIONS {
                0 | 1 => 0.0022,
                2 => -0.0015,
                3 | 4 => 0.0028,
                5 => -0.0030,
                _ => 0.0018,
            };
            let open = price * (1.0 + rng.normal() * 0.004);
            let close = open * (1.0 + regime + rng.normal() * 0.017);
            let high = open.max(close) * (1.0 + rng.uniform() * 0.012);
            let low = open.min(close) * (1.0 - rng.uniform() * 0.012);
            let volume =
                (18.0e6 * (0.6 + rng.uniform()) * (1.0 + (close / open - 1.0).abs() * 25.0))
                    .round();
            days.push(Candle {
                time: date,
                open,
                high,
                low,
                close,
                volume,
                turnover: volume * (high + low + close) / 3.0,
            });
            price = close;
        }

        let mut market = Self {
            rng,
            days,
            minutes: Vec::new(),
            sessions: dates[DAILY_SESSIONS - MINUTE_SESSIONS..].to_vec(),
            tick_seconds: 0,
        };
        market.simulate_sessions();
        market
    }

    /// Minute bars for the last sessions, each a bridge from the session's open to its close;
    /// the daily bars of those sessions are then rebuilt from their minutes.
    fn simulate_sessions(&mut self) {
        let first_day = self.days.len() - MINUTE_SESSIONS;
        for (offset, &session) in self.sessions.clone().iter().enumerate() {
            let day_index = first_day + offset;
            let day = self.days[day_index];
            let minutes = if offset + 1 == MINUTE_SESSIONS {
                MINUTES_TRADED_TODAY
            } else {
                SESSION_MINUTES
            };
            let target = if offset + 1 == MINUTE_SESSIONS {
                day.open * (1.0 + self.rng.normal() * 0.006)
            } else {
                day.close
            };
            let mut price = day.open;
            for minute in 0..minutes {
                let remaining = (minutes - minute) as f64;
                let pull = (target - price) / remaining;
                let open = price;
                let close = (price + pull + self.rng.normal() * price * 0.0011).max(1.0);
                let high = open.max(close) * (1.0 + self.rng.uniform() * 0.0008);
                let low = open.min(close) * (1.0 - self.rng.uniform() * 0.0008);
                // U-shaped intraday volume: heavy at the open and into the close.
                let t = minute as f64 / SESSION_MINUTES as f64;
                let shape = 0.5 + 3.0 * (t - 0.5).powi(2) * 4.0 / 3.0;
                let volume =
                    (day.volume / SESSION_MINUTES as f64 * shape * (0.5 + self.rng.uniform()))
                        .round();
                self.minutes.push(Candle {
                    time: session + SESSION_OPEN + minute * MINUTE,
                    open,
                    high,
                    low,
                    close,
                    volume,
                    turnover: volume * (open + close) / 2.0,
                });
                price = close;
            }
            self.days[day_index] = self.session_bar(session).expect("session has minutes");
        }
    }

    /// The daily bar of one minute session.
    fn session_bar(&self, session: i64) -> Option<Candle> {
        let mut bars = self
            .minutes
            .iter()
            .filter(|bar| bar.time >= session && bar.time < session + DAY);
        let mut day = *bars.next()?;
        day.time = session;
        for bar in bars {
            day.merge(bar);
        }
        Some(day)
    }

    fn today(&self) -> i64 {
        *self.sessions.last().expect("at least one session")
    }

    /// Whether today's last minute has been fully traded.
    fn session_over(&self) -> bool {
        self.minutes.last().is_some_and(|last| {
            last.time >= self.today() + SESSION_OPEN + (SESSION_MINUTES - 1) * MINUTE
                && self.tick_seconds >= 50
        })
    }

    /// Group bars into buckets keyed by `key`, keeping the first bar's time for each bucket.
    fn aggregate(bars: &[Candle], key: impl Fn(i64) -> i64) -> Vec<Candle> {
        let mut out: Vec<Candle> = Vec::new();
        let mut current_key = None;
        for bar in bars {
            let bucket = key(bar.time);
            if current_key == Some(bucket) {
                out.last_mut().expect("open bucket").merge(bar);
            } else {
                let mut first = *bar;
                first.time = bucket;
                out.push(first);
                current_key = Some(bucket);
            }
        }
        out
    }

    fn minute_bucket(time: i64, minutes: i64) -> i64 {
        let session = time.div_euclid(DAY) * DAY;
        let since_open = time - session - SESSION_OPEN;
        session + SESSION_OPEN + since_open.div_euclid(minutes * MINUTE) * minutes * MINUTE
    }

    fn week_bucket(time: i64) -> i64 {
        // Monday of the week.
        let days = time.div_euclid(DAY);
        let weekday = (days + 3).rem_euclid(7); // 0 = Monday
        (days - weekday) * DAY
    }

    fn month_bucket(time: i64) -> i64 {
        let (year, month, _) = civil_from_days(time.div_euclid(DAY));
        days_from_civil(year, month, 1) * DAY
    }
}

impl Default for SimulatedMarket {
    fn default() -> Self {
        Self::new()
    }
}

impl MarketData for SimulatedMarket {
    fn quote(&self) -> Quote {
        let today = *self.days.last().expect("daily bars");
        let prev_close = self.days[self.days.len() - 2].close;
        let year = &self.days[self.days.len().saturating_sub(252)..];
        let last_minute = self.minutes.last().map_or(today.time, |bar| bar.time);
        Quote {
            time: last_minute + self.tick_seconds,
            trading: !self.session_over(),
            symbol: "DEMO".into(),
            name: "示例科技".into(),
            exchange: "模拟行情".into(),
            currency: "USD".into(),
            last: today.close,
            prev_close,
            open: today.open,
            high: today.high,
            low: today.low,
            volume: today.volume,
            turnover: today.turnover,
            week52_high: year.iter().map(|bar| bar.high).fold(f64::MIN, f64::max),
            week52_low: year.iter().map(|bar| bar.low).fold(f64::MAX, f64::min),
            shares_outstanding: 2.46e9,
            eps_ttm: 4.87,
            dividend_per_share: 0.96,
        }
    }

    fn candles(&self, period: Period) -> Vec<Candle> {
        match period {
            Period::Intraday => {
                let today = self.today();
                self.minutes
                    .iter()
                    .filter(|bar| bar.time >= today)
                    .copied()
                    .collect()
            }
            Period::Min1 => self.minutes.clone(),
            Period::Min5 | Period::Min15 | Period::Hour1 => {
                let minutes = period.minutes().expect("minute period");
                Self::aggregate(&self.minutes, |time| Self::minute_bucket(time, minutes))
            }
            Period::Day => self.days.clone(),
            Period::Week => Self::aggregate(&self.days, Self::week_bucket),
            Period::Month => Self::aggregate(&self.days, Self::month_bucket),
        }
    }

    fn latest(&self, period: Period) -> Option<Candle> {
        match period {
            Period::Intraday | Period::Min1 => self.minutes.last().copied(),
            Period::Min5 | Period::Min15 | Period::Hour1 => {
                let minutes = period.minutes().expect("minute period");
                let last = self.minutes.last()?;
                let bucket = Self::minute_bucket(last.time, minutes);
                let first = self.minutes.iter().position(|bar| bar.time >= bucket)?;
                Self::aggregate(&self.minutes[first..], |time| {
                    Self::minute_bucket(time, minutes)
                })
                .pop()
            }
            Period::Day => self.days.last().copied(),
            Period::Week | Period::Month => {
                let key: fn(i64) -> i64 = if period == Period::Week {
                    Self::week_bucket
                } else {
                    Self::month_bucket
                };
                let bucket = key(self.days.last()?.time);
                let first = self.days.iter().position(|bar| key(bar.time) == bucket)?;
                Self::aggregate(&self.days[first..], key).pop()
            }
        }
    }

    /// One tick is ten simulated seconds: the current minute's price moves, and every sixth tick
    /// opens the next minute until the session closes.
    fn poll(&mut self) -> bool {
        let today = self.today();
        let Some(last) = self.minutes.last().copied() else {
            return false;
        };
        if self.session_over() {
            return false;
        }
        self.tick_seconds += 10;
        let step = last.close * self.rng.normal() * 0.0006;
        let price = (last.close + step).max(1.0);
        let volume = (self.days.last().map_or(0.0, |day| day.volume) / SESSION_MINUTES as f64
            * (0.08 + self.rng.uniform() * 0.12))
            .round();
        if self.tick_seconds >= 60 {
            self.tick_seconds = 0;
            self.minutes.push(Candle {
                time: last.time + MINUTE,
                open: last.close,
                high: last.close.max(price),
                low: last.close.min(price),
                close: price,
                volume,
                turnover: volume * price,
            });
        } else {
            let bar = self.minutes.last_mut().expect("current minute");
            bar.high = bar.high.max(price);
            bar.low = bar.low.min(price);
            bar.close = price;
            bar.volume += volume;
            bar.turnover += volume * price;
        }
        let day_index = self.days.len() - 1;
        self.days[day_index] = self.session_bar(today).expect("today has minutes");
        true
    }
}

/// A small deterministic generator (SplitMix64) with uniform and normal draws.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Standard normal (Box–Muller).
    fn normal(&mut self) -> f64 {
        let u = self.uniform().max(f64::MIN_POSITIVE);
        let v = self.uniform();
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian (Howard Hinnant's algorithm).
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let month = i64::from(month);
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_round_trips() {
        assert_eq!(civil_from_days(TODAY / DAY), (2026, 9, 25));
        for days in [-1_000, 0, 19_000, 20_721] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
    }

    #[test]
    fn periods_agree_with_each_other() {
        let market = SimulatedMarket::new();
        let day = market.latest(Period::Day).unwrap();
        let intraday = market.candles(Period::Intraday);
        assert_eq!(intraday.len() as i64, MINUTES_TRADED_TODAY);
        assert_eq!(day.close, intraday.last().unwrap().close);
        let volume: f64 = intraday.iter().map(|bar| bar.volume).sum();
        assert!((day.volume - volume).abs() < 1e-6);
        let hour = market.latest(Period::Hour1).unwrap();
        assert_eq!(hour.close, day.close);
        assert_eq!(market.candles(Period::Hour1).last().unwrap(), &hour);
        let week = market.candles(Period::Week);
        assert_eq!(week.last().unwrap(), &market.latest(Period::Week).unwrap());
        assert!(market.candles(Period::Month).len() >= 24);
        assert_eq!(market.quote().last, day.close);
    }

    #[test]
    fn polling_moves_the_live_bar() {
        let mut market = SimulatedMarket::new();
        let before = market.candles(Period::Min1).len();
        for _ in 0..6 {
            assert!(market.poll());
        }
        assert_eq!(market.candles(Period::Min1).len(), before + 1);
        assert_eq!(
            market.latest(Period::Day).unwrap().close,
            market.latest(Period::Min1).unwrap().close
        );
    }
}
